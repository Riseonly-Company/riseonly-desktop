use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use rise_canon::rise_key::{bot, events, ledger, map, music, stories, theme, user};
use rise_canon::rise_ledger::{PREMIUM_PERIODS, SPARKS_PACKS};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{
    Url, UrlComponents, UrlQueryItem, removing_percent_encoding,
};
use rise_deeplink::{
    Environment, LITERAL_SEGMENT_CASE_INSENSITIVE, ParameterKind, ParameterKindSpec, RouteKind,
    SegmentSpec, TOKEN_CASE_INSENSITIVE,
};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::universal_link_parser::RiseonlyUniversalLink;

const PUBLIC_SITE_URL: Option<&str> = option_env!("PUBLIC_SITE_URL");
const APP_URL_SCHEME: Option<&str> = option_env!("APP_URL_SCHEME");

impl AppEnvironment {
    /// canon deep link environment, schemes and hosts are
    /// declared per environment name so no host table lives here
    pub(crate) fn canon_deep_link_environment(self) -> &'static Environment {
        self.canon()
    }

    pub fn public_site_url(self) -> Url {
        let canon = self.canon_deep_link_environment();
        let fallback = Url::from_string(&format!("{}://{}", canon.web_scheme, canon.web_host))
            .expect("canon deep-link host is a URL");
        let Some(configured) = Self::configured_url(PUBLIC_SITE_URL) else {
            return fallback;
        };
        let path = configured.path();
        if self != Self::current()
            || configured.scheme().map(str::to_lowercase).as_deref() != Some(canon.web_scheme)
            || configured.host().map(|host| host.to_lowercase()).as_deref() != Some(canon.web_host)
            || configured.user().is_some()
            || configured.password().is_some()
            || configured.port().is_some()
            || !(path.is_empty() || path == "/")
            || configured.query().is_some()
            || configured.fragment().is_some()
        {
            return fallback;
        }
        configured
    }

    pub fn app_url_scheme(self) -> String {
        let fallback = self.canon_deep_link_environment().url_scheme;
        match Self::configured_string(APP_URL_SCHEME).map(|value| value.to_lowercase()) {
            Some(configured) if self == Self::current() && configured == fallback => configured,
            _ => fallback.to_owned(),
        }
    }

    pub fn public_host(self) -> String {
        self.public_site_url()
            .host()
            .unwrap_or_else(|| self.canon_deep_link_environment().web_host.to_owned())
    }

    pub fn accepts_public_host(self, host: Option<&str>) -> bool {
        let Some(host) = host.map(str::to_lowercase) else {
            return false;
        };
        if host == self.public_host() {
            return true;
        }
        self.canon_deep_link_environment()
            .host_aliases
            .iter()
            .any(|alias| *alias == host)
    }

    fn configured_url(raw: Option<&str>) -> Option<Url> {
        let value = Self::configured_string(raw)?;
        Url::from_string(value)
    }

    fn configured_string(raw: Option<&str>) -> Option<&str> {
        let value = trimming_whitespaces_and_newlines(raw?);
        if value.is_empty() || value.contains("$(") {
            return None;
        }
        Some(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseonlyPublicLink {
    Profile { tag: String },
    PublicTag { tag: String },
    Post { id: String },
    PostComment { post_id: String, comment_id: String },
    Story { id: String },
    Highlight { id: String },
    Chat { id: String },
    ChatTopic { chat_id: String, topic_id: String },
    ChatMessage { chat_id: String, message_id: String, topic_id: Option<String> },
    Album { id: String },
    StickerPack { slug: String },
    Theme { id: String },
    Track { id: String, playlist_id: Option<String> },
    Playlist { reference: String },
    Join { token: String },
    WebLogin { token: String },
    ManagedLogin { code: String },
    PremiumPromo { code: String },
    MapInvite { token: String },
    StickerPackInvite { token: String },
    PurchaseSparks { amount: i64, shortfall: Option<i64> },
    PurchaseGift { user_id: String, period: String },
    PurchasePremium { period: String },
}

struct NormalizedRoute {
    segments: Vec<String>,
    query_items: Option<Vec<UrlQueryItem>>,
}

impl RiseonlyPublicLink {
    pub fn web_url(&self, environment: AppEnvironment) -> Option<Url> {
        let route = self.normalized_route()?;
        let mut components = UrlComponents::from_url(&environment.public_site_url(), false)?;
        components.set_path(&format!("/{}", route.segments.join("/")));
        components.set_query_items(route.query_items);
        components.url()
    }

    pub fn app_url(&self, environment: AppEnvironment) -> Option<Url> {
        let route = self.normalized_route()?;
        let section = route.segments.first()?;
        let mut components = UrlComponents::new();
        components.set_scheme(Some(environment.app_url_scheme().as_str()));
        components.set_host(Some(section.as_str()));
        if route.segments.len() > 1 {
            components.set_path(&format!("/{}", route.segments[1..].join("/")));
        }
        components.set_query_items(route.query_items);
        components.url()
    }

    pub fn deep_link(&self) -> Option<PushDeepLink> {
        self.normalized_route()?;
        let link = match self {
            Self::Profile { tag } => {
                PushDeepLink::Profile { user_id: None, tag: Self::normalized_tag(tag) }
            }
            Self::PublicTag { tag } => PushDeepLink::PublicTag { tag: Self::normalized_tag(tag)? },
            Self::Post { id } => PushDeepLink::Post { post_id: Self::normalized_id(id)? },
            Self::PostComment { post_id, comment_id } => PushDeepLink::PostComment {
                post_id: Self::normalized_id(post_id)?,
                comment_id: Some(Self::normalized_id(comment_id)?),
            },
            Self::Story { id } => PushDeepLink::Story { story_id: Self::normalized_id(id)? },
            Self::Highlight { id } => {
                PushDeepLink::Highlight { highlight_id: Self::normalized_id(id)? }
            }
            Self::Chat { id } => PushDeepLink::Chat { chat_id: Self::normalized_id(id)? },
            Self::ChatTopic { chat_id, topic_id } => PushDeepLink::ChatTopic {
                chat_id: Self::normalized_id(chat_id)?,
                topic_id: Self::normalized_id(topic_id)?,
            },
            Self::ChatMessage { chat_id, message_id, topic_id } => PushDeepLink::ChatMessage {
                chat_id: Self::normalized_id(chat_id)?,
                message_id: Self::normalized_id(message_id)?,
                topic_id: topic_id.as_deref().and_then(Self::normalized_id),
            },
            Self::Album { id } => PushDeepLink::MusicAlbum { album_id: Self::normalized_id(id)? },
            Self::StickerPack { slug } => {
                PushDeepLink::AddStickerPack { link: Self::normalized_slug(slug)? }
            }
            Self::Theme { id } => {
                PushDeepLink::AddTheme { theme_id: RiseonlyUniversalLink::canonical_resource_id(id)? }
            }
            Self::Track { id, playlist_id } => {
                let track_id = RiseonlyUniversalLink::canonical_resource_id(id)?;
                match playlist_id {
                    Some(playlist_id) => PushDeepLink::PlaylistTrack {
                        playlist_id: RiseonlyUniversalLink::canonical_resource_id(playlist_id)?,
                        track_id,
                    },
                    None => PushDeepLink::MusicTrack { track_id },
                }
            }
            Self::Playlist { reference } => {
                match RiseonlyUniversalLink::canonical_resource_id(reference) {
                    Some(playlist_id) => PushDeepLink::Playlist { playlist_id },
                    None => PushDeepLink::PlaylistInvite { token: reference.clone() },
                }
            }
            Self::Join { token } => {
                PushDeepLink::JoinChat { token: Self::normalized_join_token(token)? }
            }
            Self::WebLogin { token } => {
                PushDeepLink::WebLogin { login_token: Self::normalized_web_login_token(token)? }
            }
            Self::ManagedLogin { code } => {
                PushDeepLink::ManagedLogin { code: Self::normalized_login_code(code)? }
            }
            Self::PremiumPromo { code } => {
                PushDeepLink::PremiumPromo { code: Self::normalized_promo_code(code)? }
            }
            Self::MapInvite { token } => {
                PushDeepLink::MapInvite { token: Self::normalized_join_token(token)? }
            }
            Self::StickerPackInvite { token } => {
                PushDeepLink::StickerPackInvite { token: Self::normalized_join_token(token)? }
            }
            Self::PurchaseSparks { .. } | Self::PurchaseGift { .. } | Self::PurchasePremium { .. } => {
                return None;
            }
        };
        Some(link)
    }

    pub fn join_token(raw_value: &str) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw_value);
        if let Some(token) = Self::normalized_join_token(value) {
            return Some(token);
        }
        let components = UrlComponents::from_string(value)?;
        if components.user().is_some()
            || components.password().is_some()
            || components.port().is_some()
        {
            return None;
        }
        let path = components.percent_encoded_path();
        if path.is_empty() || path.ends_with('/') || path.contains("//") {
            return None;
        }
        let encoded_segments: Vec<&str> =
            path.split('/').filter(|segment| !segment.is_empty()).collect();
        if encoded_segments.len() != 2
            || !DeepLinkForms::matches_token(encoded_segments[0], RouteKind::Join)
        {
            return None;
        }
        let decoded = removing_percent_encoding(encoded_segments[1])?;
        Self::normalized_join_token(&decoded)
    }

    pub fn join_url(raw_value: &str, environment: AppEnvironment) -> Option<Url> {
        let token = Self::join_token(raw_value)?;
        RiseonlyPublicLink::Join { token }.web_url(environment)
    }

    pub fn normalized_id(raw: &str) -> Option<String> {
        Self::normalized(raw, ParameterKind::ResourceId)
    }

    pub fn normalized_tag(raw: &str) -> Option<String> {
        Self::normalized(raw, ParameterKind::Tag)
    }

    pub fn normalized_slug(raw: &str) -> Option<String> {
        Self::normalized_id(raw)
    }

    /// invite token is not trimmed and need not start with a letter
    /// nobody types it, it arrives whole inside the link
    pub fn normalized_join_token(raw: &str) -> Option<String> {
        let spec = DeepLinkForms::parameter_kind(ParameterKind::JoinToken);
        if !(spec.minimum_utf8_bytes..=spec.maximum_utf8_bytes).contains(&raw.len())
            || !raw.chars().all(|scalar| DeepLinkForms::is_allowed(scalar, spec.alphabet))
        {
            return None;
        }
        Some(raw.to_owned())
    }

    pub fn normalized_web_login_token(raw: &str) -> Option<String> {
        Self::normalized_slug(raw)
    }

    /// promo code as the admin panel prints it, lowercase letters and digits and dash and underscore
    /// server revalidates so the client only keeps the link shape sane
    pub fn normalized_promo_code(raw: &str) -> Option<String> {
        let value = Self::normalized(&raw.to_lowercase(), ParameterKind::ResourceId)?;
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase() || byte == b'-' || byte == b'_')
        {
            return None;
        }
        Some(value)
    }

    /// one time code of the bot, lowercase hex inside the canon length window
    pub fn normalized_login_code(raw: &str) -> Option<String> {
        let value = Self::normalized(&raw.to_lowercase(), ParameterKind::LoginCode)?;
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return None;
        }
        Some(value)
    }

    pub fn is_valid_playlist_invite_token(raw: &str) -> bool {
        if raw.len() > DeepLinkForms::parameter_kind(ParameterKind::PlaylistReference).maximum_utf8_bytes {
            return false;
        }
        let parts: Vec<&str> = raw.split('.').collect();
        if parts.len() != 3 || RiseonlyUniversalLink::canonical_resource_id(parts[0]).is_none() {
            return false;
        }
        let version = parts[1];
        if !(1..=10).contains(&version.len())
            || !version.bytes().next().is_some_and(|first| (b'1'..=b'9').contains(&first))
            || !version.bytes().all(|byte| byte.is_ascii_digit())
            || !version.parse::<i32>().is_ok_and(|value| value > 0)
            || parts[2].len() != 64
        {
            return false;
        }
        parts[2]
            .chars()
            .all(|scalar| matches!(scalar as u32, 48..=57 | 97..=102))
    }

    fn canon_route(&self) -> Option<(RouteKind, HashMap<&'static str, String>)> {
        match self {
            Self::Profile { tag } => {
                let tag = Self::normalized_tag(tag)?;
                Some((RouteKind::Profile, HashMap::from([(events::TAG, tag)])))
            }
            Self::PublicTag { tag } => {
                let tag = Self::normalized_tag(tag)?;
                Some((RouteKind::PublicTag, HashMap::from([(events::TAG, tag)])))
            }
            Self::Post { id } => {
                let id = Self::normalized_id(id)?;
                Some((RouteKind::Post, HashMap::from([(events::POST_ID, id)])))
            }
            Self::PostComment { post_id, comment_id } => {
                let post_id = Self::normalized_id(post_id)?;
                let comment_id = Self::normalized_id(comment_id)?;
                Some((
                    RouteKind::PostComment,
                    HashMap::from([(events::POST_ID, post_id), (events::COMMENT_ID, comment_id)]),
                ))
            }
            Self::Story { id } => {
                let id = Self::normalized_id(id)?;
                Some((RouteKind::Story, HashMap::from([(events::STORY_ID, id)])))
            }
            Self::Highlight { id } => {
                let id = Self::normalized_id(id)?;
                Some((RouteKind::Highlight, HashMap::from([(stories::HIGHLIGHT_ID, id)])))
            }
            Self::Chat { id } => {
                let id = Self::normalized_id(id)?;
                Some((RouteKind::Chat, HashMap::from([(events::CHAT_ID, id)])))
            }
            Self::ChatTopic { chat_id, topic_id } => {
                let chat_id = Self::normalized_id(chat_id)?;
                let topic_id = Self::normalized_id(topic_id)?;
                Some((
                    RouteKind::ChatTopic,
                    HashMap::from([(events::CHAT_ID, chat_id), (events::TOPIC_ID, topic_id)]),
                ))
            }
            Self::ChatMessage { chat_id, message_id, topic_id } => {
                let chat_id = Self::normalized_id(chat_id)?;
                let message_id = Self::normalized_id(message_id)?;
                let mut values =
                    HashMap::from([(events::CHAT_ID, chat_id), (events::MESSAGE_ID, message_id)]);
                if let Some(topic_id) = topic_id {
                    values.insert(events::TOPIC_ID, Self::normalized_id(topic_id)?);
                }
                Some((RouteKind::ChatMessage, values))
            }
            Self::Album { id } => {
                let id = Self::normalized_id(id)?;
                Some((RouteKind::Album, HashMap::from([(music::ALBUM_ID, id)])))
            }
            Self::StickerPack { slug } => {
                let slug = Self::normalized_slug(slug)?;
                Some((RouteKind::StickerPack, HashMap::from([(bot::SLUG, slug)])))
            }
            Self::Theme { id } => {
                let id = RiseonlyUniversalLink::canonical_resource_id(id)?;
                Some((RouteKind::Theme, HashMap::from([(theme::THEME_ID, id)])))
            }
            Self::Track { id, playlist_id } => {
                let id = RiseonlyUniversalLink::canonical_resource_id(id)?;
                let mut values = HashMap::from([(events::TRACK_ID, id)]);
                if let Some(playlist_id) = playlist_id {
                    values.insert(
                        events::PLAYLIST_ID,
                        RiseonlyUniversalLink::canonical_resource_id(playlist_id)?,
                    );
                }
                Some((RouteKind::Track, values))
            }
            Self::Playlist { reference } => {
                if RiseonlyUniversalLink::canonical_resource_id(reference).is_none()
                    && !Self::is_valid_playlist_invite_token(reference)
                {
                    return None;
                }
                Some((RouteKind::Playlist, HashMap::from([(events::REFERENCE, reference.clone())])))
            }
            Self::Join { token } => {
                let token = Self::normalized_join_token(token)?;
                Some((RouteKind::Join, HashMap::from([(bot::TOKEN, token)])))
            }
            Self::WebLogin { token } => {
                let token = Self::normalized_web_login_token(token)?;
                Some((RouteKind::WebLogin, HashMap::from([(bot::TOKEN, token)])))
            }
            Self::ManagedLogin { code } => {
                let code = Self::normalized_login_code(code)?;
                Some((RouteKind::ManagedLogin, HashMap::from([(user::CODE, code)])))
            }
            Self::PremiumPromo { code } => {
                let code = Self::normalized_promo_code(code)?;
                Some((RouteKind::PremiumPromo, HashMap::from([(user::CODE, code)])))
            }
            Self::MapInvite { token } => {
                let token = Self::normalized_join_token(token)?;
                Some((RouteKind::MapInvite, HashMap::from([(bot::TOKEN, token)])))
            }
            Self::StickerPackInvite { token } => {
                let token = Self::normalized_join_token(token)?;
                Some((RouteKind::StickerPackInvite, HashMap::from([(bot::TOKEN, token)])))
            }
            Self::PurchaseSparks { amount, shortfall } => {
                let amount = Self::normalized_pack_amount(&amount.to_string())?;
                let mut values = HashMap::from([(ledger::AMOUNT, amount.to_string())]);
                if let Some(shortfall) = shortfall {
                    let shortfall = Self::normalized_count(&shortfall.to_string())?;
                    values.insert(Self::sparks_shortfall_parameter()?, shortfall.to_string());
                }
                Some((RouteKind::PurchaseSparks, values))
            }
            Self::PurchaseGift { user_id, period } => {
                let user_id = Self::normalized_id(user_id)?;
                let period = Self::normalized_premium_period(period)?;
                Some((
                    RouteKind::PurchaseGift,
                    HashMap::from([(ledger::USER_ID, user_id), (map::PERIOD, period)]),
                ))
            }
            Self::PurchasePremium { period } => {
                let period = Self::normalized_premium_period(period)?;
                Some((RouteKind::PurchasePremium, HashMap::from([(map::PERIOD, period)])))
            }
        }
    }

    pub fn normalized_pack_amount(raw: &str) -> Option<i64> {
        let value = Self::normalized_count(raw)?;
        SPARKS_PACKS.iter().any(|pack| i64::from(pack.amount) == value).then_some(value)
    }

    /// positive whole number in ascii digits, inside the canon length window
    pub fn normalized_count(raw: &str) -> Option<i64> {
        let value = Self::normalized(raw, ParameterKind::Count)?;
        value.parse::<i64>().ok().filter(|number| *number > 0)
    }

    /// premium period token like 1m or 3m or 6m or 12m
    /// every plan and gift product has that suffix so one link picks the same period in every store
    pub fn normalized_premium_period(raw: &str) -> Option<String> {
        let value = Self::normalized(&raw.to_lowercase(), ParameterKind::PremiumPeriod)?;
        PREMIUM_PERIODS.iter().any(|period| period.token == value).then_some(value)
    }

    fn sparks_shortfall_parameter() -> Option<&'static str> {
        RouteKind::PurchaseSparks.build_form().query.first().map(|key| key.parameter)
    }

    fn normalized_route(&self) -> Option<NormalizedRoute> {
        let (route, values) = self.canon_route()?;
        let segments = DeepLinkForms::segments(route, &values)?;
        Some(NormalizedRoute { segments, query_items: DeepLinkForms::query_items(route, &values) })
    }

    fn normalized(raw: &str, kind: ParameterKind) -> Option<String> {
        let spec = DeepLinkForms::parameter_kind(kind);
        let value = trimming_whitespaces_and_newlines(raw);
        if value.is_empty()
            || value.len() < spec.minimum_utf8_bytes
            || value.len() > spec.maximum_utf8_bytes
            || !value.chars().next().is_some_and(DeepLinkForms::is_ascii_alphanumeric)
            || !value.chars().all(|scalar| DeepLinkForms::is_allowed(scalar, spec.alphabet))
        {
            return None;
        }
        Some(value.to_owned())
    }
}

/// reads the canon link table, tokens and literals and value bounds
/// the old string constants now come from rise_deeplink
pub enum DeepLinkForms {}

static ROUTES_BY_TOKEN: LazyLock<HashMap<String, Vec<RouteKind>>> = LazyLock::new(|| {
    let mut result: HashMap<String, Vec<RouteKind>> = HashMap::new();
    for route in RouteKind::ALL {
        let spec = route.spec();
        let Some(token) = spec.token else {
            continue;
        };
        for name in std::iter::once(token).chain(spec.aliases.iter().copied()) {
            result
                .entry(DeepLinkForms::token_key(name))
                .or_default()
                .push(route);
        }
    }
    result
});

impl DeepLinkForms {
    fn token_key(name: &str) -> String {
        if TOKEN_CASE_INSENSITIVE { name.to_lowercase() } else { name.to_owned() }
    }

    pub fn routes(token: &str) -> &'static [RouteKind] {
        ROUTES_BY_TOKEN
            .get(&Self::token_key(token))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn matches_token(segment: &str, route: RouteKind) -> bool {
        Self::routes(segment).contains(&route)
    }

    pub fn token_of(route: RouteKind) -> &'static str {
        route
            .spec()
            .token
            .expect("canon deep-link route used for a resource path has a token")
    }

    /// whether the route form tolerates a tail after #
    /// per form and user visible, on a bare tag a fragment means its not a tag
    /// on a post #comments is an anchor inside the page and opens in app
    pub fn tolerates(fragment: Option<&str>, route: RouteKind) -> bool {
        if fragment.is_none() {
            return true;
        }
        route.build_form().fragment_allowed
    }

    /// literal form segment, matched case insensitive as canon requires
    pub fn matches_literal(segment: &str, route: RouteKind, index: usize) -> bool {
        let Some(SegmentSpec::Literal(literal)) = route.build_form().segments.get(index) else {
            return false;
        };
        if !LITERAL_SEGMENT_CASE_INSENSITIVE {
            return segment == *literal;
        }
        segment.to_lowercase() == literal.to_lowercase()
    }

    /// query key names of every form of the route, aliases included
    pub fn query_key_names(route: RouteKind, parameter: &str) -> HashSet<String> {
        let mut names = HashSet::new();
        for form in route.forms() {
            for key in form.query.iter().filter(|key| key.parameter == parameter) {
                names.insert(key.key.to_lowercase());
                for alias in key.aliases {
                    names.insert(alias.to_lowercase());
                }
            }
        }
        names
    }

    pub fn parameter_kind(kind: ParameterKind) -> &'static ParameterKindSpec {
        kind.spec()
    }

    pub fn is_allowed(scalar: char, alphabet: &str) -> bool {
        match alphabet {
            "ascii_alphanumeric_dot_dash_underscore" => {
                matches!(scalar as u32, 45..=46 | 48..=57 | 65..=90 | 95 | 97..=122)
            }
            "ascii_alphanumeric_dot_dash_underscore_tilde" => {
                matches!(scalar as u32, 45..=46 | 48..=57 | 65..=90 | 95 | 97..=122 | 126)
            }
            _ => false,
        }
    }

    pub fn is_ascii_alphanumeric(scalar: char) -> bool {
        matches!(scalar as u32, 48..=57 | 65..=90 | 97..=122)
    }

    /// path segments for the build form of the route
    pub fn segments(route: RouteKind, values: &HashMap<&'static str, String>) -> Option<Vec<String>> {
        let mut result = Vec::new();
        for segment in route.build_form().segments {
            match segment {
                SegmentSpec::Literal(literal) => result.push((*literal).to_owned()),
                SegmentSpec::Parameter(name) => result.push(values.get(name)?.clone()),
            }
        }
        Some(result)
    }

    /// build form query keys filled with whichever values are present
    pub fn query_items(
        route: RouteKind,
        values: &HashMap<&'static str, String>,
    ) -> Option<Vec<UrlQueryItem>> {
        let items: Vec<UrlQueryItem> = route
            .build_form()
            .query
            .iter()
            .filter_map(|key| {
                let value = values.get(key.parameter)?;
                Some(UrlQueryItem::new(key.key, Some(value.clone())))
            })
            .collect();
        if items.is_empty() { None } else { Some(items) }
    }
}
