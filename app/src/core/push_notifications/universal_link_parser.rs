use std::collections::HashSet;

use rise_canon::rise_key::{bot, events, user};
use rise_core::foundation::foundation_url::{Url, UrlComponents, removing_percent_encoding};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_deeplink::{
    EMPTY_SEGMENT_ALLOWED, ParameterKind, RouteKind, SINGLE_SEGMENT_FALLBACK_ROUTE,
    TRAILING_SLASH_ALLOWED,
};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::riseonly_public_link::{DeepLinkForms, RiseonlyPublicLink};
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotTagRules;

pub enum UniversalLinkParser {}

impl UniversalLinkParser {
    pub fn parse(url: &Url, environment: AppEnvironment) -> Option<PushDeepLink> {
        let components = UrlComponents::from_url(url, false)?;
        if components.user().is_some()
            || components.password().is_some()
            || components.port().is_some()
        {
            return None;
        }
        let scheme = components.scheme()?.to_lowercase();

        let is_app_url = scheme == environment.app_url_scheme();
        let is_web_url = scheme == environment.canon_deep_link_environment().web_scheme
            && environment.accepts_public_host(components.host().as_deref());
        if !is_app_url && !is_web_url {
            return None;
        }
        let segments = Self::route_segments(&components, is_app_url)?;
        let section = segments.first()?.to_lowercase();

        if segments.len() == 1
            && SINGLE_SEGMENT_FALLBACK_ROUTE == RouteKind::PublicTag
            && components.percent_encoded_query().is_none()
            && DeepLinkForms::tolerates(components.percent_encoded_fragment(), RouteKind::PublicTag)
            && let Some(tag) = RiseonlyPublicLink::normalized_tag(&segments[0])
        {
            return RiseonlyPublicLink::PublicTag { tag }.deep_link();
        }

        let routes = DeepLinkForms::routes(&section);

        let is_bot_tag =
            segments.len() == 2 && BotTagRules::is_valid(&segments[1].to_lowercase());
        if segments.len() == 2
            && (routes.contains(&RouteKind::BotChat)
                || (routes.contains(&RouteKind::Chat) && is_bot_tag))
        {
            let tag = RiseonlyPublicLink::normalized_tag(&segments[1])?;
            if !(routes.contains(&RouteKind::BotChat) || BotTagRules::is_valid(&tag))
                || components.percent_encoded_fragment().is_some()
            {
                return None;
            }
            let query = components.query_items().unwrap_or_default();
            let route =
                if routes.contains(&RouteKind::Chat) { RouteKind::Chat } else { RouteKind::BotChat };
            let keys = DeepLinkForms::query_key_names(route, bot::START_PARAM);
            if query.len() > 1 || !query.iter().all(|item| keys.contains(&item.name.to_lowercase())) {
                return None;
            }
            if let Some(item) = query.first() {
                return Some(PushDeepLink::BotChat {
                    tag: tag.to_lowercase(),
                    start_param: Some(item.value.clone().unwrap_or_default()),
                });
            }
            return Some(PushDeepLink::BotChat { tag: tag.to_lowercase(), start_param: None });
        }

        if routes.contains(&RouteKind::BotMiniApp) {
            return Self::bot_mini_app_link(&segments, &components);
        }

        Self::public_link(routes, &segments, &components, environment, is_app_url)?.deep_link()
    }

    fn public_link(
        routes: &[RouteKind],
        segments: &[String],
        components: &UrlComponents,
        environment: AppEnvironment,
        is_app_url: bool,
    ) -> Option<RiseonlyPublicLink> {
        if routes.contains(&RouteKind::Profile) {
            if segments.len() != 2 {
                return None;
            }
            let tag = RiseonlyPublicLink::normalized_tag(&segments[1])?;
            return Some(RiseonlyPublicLink::Profile { tag });
        }

        if routes.contains(&RouteKind::Post) {
            if segments.len() == 2
                && let Some(post_id) = RiseonlyPublicLink::normalized_id(&segments[1])
            {
                return Some(RiseonlyPublicLink::Post { id: post_id });
            }
            if segments.len() != 4
                || !DeepLinkForms::matches_literal(&segments[2], RouteKind::PostComment, 2)
            {
                return None;
            }
            let post_id = RiseonlyPublicLink::normalized_id(&segments[1])?;
            let comment_id = RiseonlyPublicLink::normalized_id(&segments[3])?;
            return Some(RiseonlyPublicLink::PostComment { post_id, comment_id });
        }

        if routes.contains(&RouteKind::Story) {
            if segments.len() != 2 {
                return None;
            }
            let story_id = RiseonlyPublicLink::normalized_id(&segments[1])?;
            return Some(RiseonlyPublicLink::Story { id: story_id });
        }

        if routes.contains(&RouteKind::Highlight) {
            if segments.len() != 2 {
                return None;
            }
            let highlight_id = RiseonlyPublicLink::normalized_id(&segments[1])?;
            return Some(RiseonlyPublicLink::Highlight { id: highlight_id });
        }

        if routes.contains(&RouteKind::Chat) {
            if segments.len() == 2
                && let Some(chat_id) = RiseonlyPublicLink::normalized_id(&segments[1])
            {
                return Some(RiseonlyPublicLink::Chat { id: chat_id });
            }
            if segments.len() == 4
                && routes.contains(&RouteKind::ChatMessage)
                && DeepLinkForms::matches_literal(&segments[2], RouteKind::ChatMessage, 2)
            {
                let chat_id = RiseonlyPublicLink::normalized_id(&segments[1])?;
                let message_id = RiseonlyPublicLink::normalized_id(&segments[3])?;
                let topic_keys =
                    DeepLinkForms::query_key_names(RouteKind::ChatMessage, events::TOPIC_ID);
                let topic_id = components
                    .query_items()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|item| topic_keys.contains(&item.name.to_lowercase()))
                    .and_then(|item| item.value)
                    .and_then(|value| RiseonlyPublicLink::normalized_id(&value));
                return Some(RiseonlyPublicLink::ChatMessage { chat_id, message_id, topic_id });
            }
            if segments.len() != 4
                || !DeepLinkForms::matches_literal(&segments[2], RouteKind::ChatTopic, 2)
            {
                return None;
            }
            let chat_id = RiseonlyPublicLink::normalized_id(&segments[1])?;
            let topic_id = RiseonlyPublicLink::normalized_id(&segments[3])?;
            return Some(RiseonlyPublicLink::ChatTopic { chat_id, topic_id });
        }

        if routes.contains(&RouteKind::Album) {
            if segments.len() != 2 {
                return None;
            }
            let album_id = RiseonlyPublicLink::normalized_id(&segments[1])?;
            return Some(RiseonlyPublicLink::Album { id: album_id });
        }

        if routes.contains(&RouteKind::StickerPack) {
            if segments.len() != 2 {
                return None;
            }
            let slug = RiseonlyPublicLink::normalized_slug(&segments[1])?;
            return Some(RiseonlyPublicLink::StickerPack { slug });
        }

        if routes.contains(&RouteKind::Theme) {
            if segments.len() != 2 {
                return None;
            }
            let id = RiseonlyUniversalLink::canonical_resource_id(&segments[1])?;
            return Some(RiseonlyPublicLink::Theme { id });
        }

        if routes.contains(&RouteKind::Track) {
            if segments.len() != 2 {
                return None;
            }
            let track_id = RiseonlyUniversalLink::canonical_resource_id(&segments[1])?;
            if !Self::has_exact_resource_path(
                components,
                environment,
                is_app_url,
                RouteKind::Track,
                &track_id,
            ) {
                return None;
            }
            let playlist_id = Self::playlist_context_id(components)?;
            return Some(RiseonlyPublicLink::Track { id: track_id, playlist_id });
        }

        if routes.contains(&RouteKind::Playlist) {
            if segments.len() != 2
                || !Self::has_exact_resource_path(
                    components,
                    environment,
                    is_app_url,
                    RouteKind::Playlist,
                    &segments[1],
                )
                || components.percent_encoded_query().is_some()
            {
                return None;
            }
            return Some(RiseonlyPublicLink::Playlist { reference: segments[1].clone() });
        }

        if routes.contains(&RouteKind::Join) {
            let host_is_public = components
                .host()
                .is_some_and(|host| host.to_lowercase() == environment.public_host());
            if !(is_app_url || host_is_public) {
                return None;
            }
            let token = Self::join_token(segments, components)?;
            return Some(RiseonlyPublicLink::Join { token });
        }

        if routes.contains(&RouteKind::WebLogin) {
            if segments.len() != 1 {
                return None;
            }
            let token = Self::single_query_token(
                components,
                &DeepLinkForms::query_key_names(RouteKind::WebLogin, bot::TOKEN),
            )?;
            let token = RiseonlyPublicLink::normalized_web_login_token(&token)?;
            return Some(RiseonlyPublicLink::WebLogin { token });
        }

        if routes.contains(&RouteKind::PremiumPromo) {
            if segments.len() != 2 {
                return None;
            }
            let code = RiseonlyPublicLink::normalized_promo_code(&segments[1])?;
            return Some(RiseonlyPublicLink::PremiumPromo { code });
        }

        if routes.contains(&RouteKind::MapInvite) {
            if segments.len() != 2 {
                return None;
            }
            let token = RiseonlyPublicLink::normalized_join_token(&segments[1])?;
            return Some(RiseonlyPublicLink::MapInvite { token });
        }

        if routes.contains(&RouteKind::StickerPackInvite) {
            if segments.len() != 2 {
                return None;
            }
            let token = RiseonlyPublicLink::normalized_join_token(&segments[1])?;
            return Some(RiseonlyPublicLink::StickerPackInvite { token });
        }

        if routes.contains(&RouteKind::ManagedLogin) {
            if segments.len() != 1 {
                return None;
            }
            let code = Self::single_query_token(
                components,
                &DeepLinkForms::query_key_names(RouteKind::ManagedLogin, user::CODE),
            )?;
            let code = RiseonlyPublicLink::normalized_login_code(&code)?;
            return Some(RiseonlyPublicLink::ManagedLogin { code });
        }

        if segments.len() != 1 || components.percent_encoded_query().is_some() {
            return None;
        }
        let tag = RiseonlyPublicLink::normalized_tag(&segments[0])?;
        Some(RiseonlyPublicLink::PublicTag { tag })
    }

    pub fn malformed_music_link(url: &Url, environment: AppEnvironment) -> Option<PushDeepLink> {
        if Self::parse(url, environment).is_some() {
            return None;
        }
        let components = UrlComponents::from_url(url, false)?;
        if components.user().is_some()
            || components.password().is_some()
            || components.port().is_some()
        {
            return None;
        }
        let scheme = components.scheme()?.to_lowercase();
        let is_app_url = scheme == environment.app_url_scheme();
        let is_web_url = scheme == environment.canon_deep_link_environment().web_scheme
            && environment.accepts_public_host(components.host().as_deref());
        if !is_app_url && !is_web_url {
            return None;
        }
        let segments = Self::route_segments(&components, is_app_url)?;
        if segments.len() != 2 {
            return None;
        }
        let section = segments[0].to_lowercase();
        let reference = &segments[1];
        if RiseonlyUniversalLink::canonical_resource_id(reference).is_some() {
            return None;
        }
        let routes = DeepLinkForms::routes(&section);
        if routes.contains(&RouteKind::Track) {
            if !Self::has_exact_resource_path(
                &components,
                environment,
                is_app_url,
                RouteKind::Track,
                reference,
            ) || !DeepLinkForms::tolerates(components.percent_encoded_fragment(), RouteKind::Track)
            {
                return None;
            }
            let playlist_id = Self::playlist_context_id(&components)?;
            if let Some(playlist_id) = playlist_id {
                return Some(PushDeepLink::PlaylistTrack {
                    playlist_id,
                    track_id: reference.clone(),
                });
            }
            return Some(PushDeepLink::MusicTrack { track_id: reference.clone() });
        }
        if routes.contains(&RouteKind::Playlist)
            && !RiseonlyPublicLink::is_valid_playlist_invite_token(reference)
        {
            if !Self::has_exact_resource_path(
                &components,
                environment,
                is_app_url,
                RouteKind::Playlist,
                reference,
            ) || components.percent_encoded_query().is_some()
                || !DeepLinkForms::tolerates(
                    components.percent_encoded_fragment(),
                    RouteKind::Playlist,
                )
            {
                return None;
            }
            return Some(PushDeepLink::Playlist { playlist_id: reference.clone() });
        }
        None
    }

    fn route_segments(components: &UrlComponents, is_app_url: bool) -> Option<Vec<String>> {
        let encoded_path = components.percent_encoded_path();
        if !encoded_path.is_empty()
            && (!encoded_path.starts_with('/')
                || (!TRAILING_SLASH_ALLOWED && encoded_path.ends_with('/'))
                || encoded_path.contains("//"))
        {
            return None;
        }

        let mut segments = Vec::new();
        if is_app_url {
            let host = components.host()?.to_lowercase();
            if host.is_empty() {
                return None;
            }
            segments.push(host);
        }

        if !encoded_path.is_empty() {
            for encoded in encoded_path[1..].split('/') {
                if !EMPTY_SEGMENT_ALLOWED && encoded.is_empty() {
                    return None;
                }
                let decoded = removing_percent_encoding(encoded)?;
                if decoded.is_empty() {
                    return None;
                }
                segments.push(decoded);
            }
        }
        Some(segments)
    }

    fn bot_mini_app_link(segments: &[String], components: &UrlComponents) -> Option<PushDeepLink> {
        if segments.len() != 3 {
            return None;
        }
        let tag = RiseonlyPublicLink::normalized_tag(&segments[1])?;
        let slug = RiseonlyPublicLink::normalized_slug(&segments[2])?;
        let names = DeepLinkForms::query_key_names(RouteKind::BotMiniApp, bot::START_PARAM);
        let start_param = components
            .query_items()
            .and_then(|items| items.into_iter().find(|item| names.contains(&item.name.to_lowercase())))
            .and_then(|item| item.value);
        Some(PushDeepLink::BotMiniApp {
            tag: tag.to_lowercase(),
            slug: slug.to_lowercase(),
            start_param,
        })
    }

    fn has_exact_resource_path(
        components: &UrlComponents,
        environment: AppEnvironment,
        is_app_url: bool,
        route: RouteKind,
        resource: &str,
    ) -> bool {
        if is_app_url {
            return components
                .percent_encoded_host()
                .is_some_and(|host| DeepLinkForms::matches_token(host, route))
                && components.percent_encoded_path() == format!("/{resource}");
        }
        let is_web_scheme = components
            .scheme()
            .is_some_and(|scheme| {
                scheme.to_lowercase() == environment.canon_deep_link_environment().web_scheme
            });
        if !is_web_scheme || !environment.accepts_public_host(components.host().as_deref()) {
            return false;
        }
        let path = components.percent_encoded_path();
        path.strip_prefix('/')
            .and_then(|path| path.split_once('/'))
            .is_some_and(|(section, tail)| {
                tail == resource && DeepLinkForms::matches_token(section, route)
            })
    }

    /// none rejects the link, some none is a track with no playlist context
    fn playlist_context_id(components: &UrlComponents) -> Option<Option<String>> {
        let Some(query) = components.percent_encoded_query() else {
            return Some(None);
        };
        let names = DeepLinkForms::query_key_names(RouteKind::Track, events::PLAYLIST_ID);
        let name = names
            .iter()
            .find(|name| query.starts_with(&format!("{name}=")))?;
        let raw_value = &query[name.len() + 1..];
        let playlist_id = RiseonlyUniversalLink::canonical_resource_id(raw_value)?;
        if query != format!("{name}={playlist_id}") {
            return None;
        }
        Some(Some(playlist_id))
    }

    fn join_token(segments: &[String], components: &UrlComponents) -> Option<String> {
        if segments.len() == 2
            && components.percent_encoded_query().is_none()
            && let Some(token) = RiseonlyPublicLink::normalized_join_token(&segments[1])
        {
            return Some(token);
        }
        if segments.len() != 1 {
            return None;
        }
        let token = Self::single_query_token(
            components,
            &DeepLinkForms::query_key_names(RouteKind::Join, bot::TOKEN),
        )?;
        RiseonlyPublicLink::normalized_join_token(&token)
    }

    fn single_query_token(components: &UrlComponents, accepted_names: &HashSet<String>) -> Option<String> {
        let items = components.query_items()?;
        if items.len() != 1 {
            return None;
        }
        let item = items.into_iter().next()?;
        if !accepted_names.contains(&item.name.to_lowercase()) {
            return None;
        }
        item.value
    }
}

pub enum RiseonlyUniversalLink {}

impl RiseonlyUniversalLink {
    pub fn track(
        track_id: &str,
        playlist_id: Option<&str>,
        environment: AppEnvironment,
    ) -> Option<Url> {
        RiseonlyPublicLink::Track {
            id: track_id.to_owned(),
            playlist_id: playlist_id.map(str::to_owned),
        }
        .web_url(environment)
    }

    pub fn playlist(playlist_id: &str, environment: AppEnvironment) -> Option<Url> {
        RiseonlyPublicLink::Playlist { reference: playlist_id.to_owned() }.web_url(environment)
    }

    /// lowercase uuid resource id, canon wants exactly 36 bytes and lower case
    /// parsing as Uuid isnt enough so the spelling is checked too
    pub fn canonical_resource_id(raw: &str) -> Option<String> {
        let spec = DeepLinkForms::parameter_kind(ParameterKind::UuidLower);
        if raw.len() != spec.maximum_utf8_bytes || raw.len() != spec.minimum_utf8_bytes {
            return None;
        }
        let value = Uuid::from_uuid_string(raw)?;
        if value.uuid_string().to_lowercase() != raw {
            return None;
        }
        let bytes = raw.as_bytes();
        if !b"12345678".contains(&bytes[14]) || !b"89ab".contains(&bytes[19]) {
            return None;
        }
        Some(raw.to_owned())
    }
}
