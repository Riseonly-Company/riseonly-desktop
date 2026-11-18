use std::collections::HashMap;

use gpui::App;
use rise_canon::RisePushType;
use rise_canon::rise_key::{events, push, push_route, user as user_key};
use rise_canon::rise_service_text::music_service;
use rise_core::foundation::foundation_compare::{CompareOptions, range_of};
use rise_core::foundation::foundation_text::{
    character_count, drop_first_characters, trimming_whitespaces_and_newlines,
};
use rise_chat::ChatType;
use rise_i18n::{FormatArg, tr_args};
use rise_notify::{NotificationScope, PushPayload, Route, RouteKind};
use serde_json::{Map, Value};

use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{
    SaiActiveScreen, SaiNavigationBridge,
};
use crate::core::push_notifications::admin_console_link::AdminConsoleLink;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::widgets::notifier::notifier_store::NotifierAuthor;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::music::shared::utils::music::music_models::MusicDefaultLocalPlaylist;
use crate::modules::notify::stores::notify_actions::notify_types::{
    NotificationKind, NotifyCopyCanon,
};

pub enum PushNotificationRouteParser {}

impl PushNotificationRouteParser {
    /// parses a push payload into a route with the canon table
    /// so a new server type routes without editing this file
    pub fn route(user_info: &Map<String, Value>) -> Option<PushDeepLink> {
        if let Some(admin_url) =
            PushNotificationPayload::string(user_info, &[AdminConsoleLink::PAYLOAD_KEY])
        {
            return AdminConsoleLink::url(&admin_url).map(|url| PushDeepLink::AdminConsole { url });
        }

        let payload = NormalizedPushPayload::new(user_info);
        let type_ = rise_notify::payload_type_of(&payload)?;

        if let Some(announcement) = Self::announcement_link(type_, user_info) {
            return Some(announcement);
        }
        if let Some(bug_report) = Self::bug_report_link(type_, user_info) {
            return Some(bug_report);
        }
        if let Some(moderation) = Self::moderation_link(type_, user_info) {
            return Some(moderation);
        }
        if let Some(sparks) = Self::sparks_link(type_) {
            return Some(sparks);
        }
        if let Some(achievement) = Self::achievement_link(type_, user_info) {
            return Some(achievement);
        }

        let route = rise_notify::parse_route(&payload).ok()?;
        Self::link(&route)
    }

    fn announcement_link(type_: &str, user_info: &Map<String, Value>) -> Option<PushDeepLink> {
        if type_ != NotificationKind::Announcement.raw_value() {
            return None;
        }
        let announcement_id = PushNotificationPayload::string(user_info, &[push::ENTITY_ID])?;
        Some(PushDeepLink::Announcement { announcement_id })
    }

    fn bug_report_link(type_: &str, user_info: &Map<String, Value>) -> Option<PushDeepLink> {
        if type_ != RisePushType::BUG_REPORT_STATUS && type_ != RisePushType::FEEDBACK_STATUS {
            return None;
        }
        let report_id = PushNotificationPayload::string(user_info, &[push::REPORT_ID])?;
        Some(if type_ == RisePushType::FEEDBACK_STATUS {
            PushDeepLink::Feedback { report_id }
        } else {
            PushDeepLink::BugReport { report_id }
        })
    }

    fn moderation_link(type_: &str, user_info: &Map<String, Value>) -> Option<PushDeepLink> {
        match type_ {
            RisePushType::REPORT_VERDICT => {
                let report_id = PushNotificationPayload::string(user_info, &[push::REPORT_ID])?;
                Some(PushDeepLink::ModerationReport { report_id })
            }
            RisePushType::MODERATION_SANCTION => {
                let report_id = PushNotificationPayload::string(user_info, &[push::REPORT_ID])?;
                Some(PushDeepLink::ModerationSanction { report_id })
            }
            RisePushType::APPEAL_VERDICT => {
                let appeal_id = PushNotificationPayload::string(user_info, &[push::APPEAL_ID])?;
                Some(PushDeepLink::AppealVerdict { appeal_id })
            }
            _ if type_ == NotificationKind::Moderation.raw_value() => {
                match PushNotificationPayload::string(user_info, &[push::KIND]).as_deref() {
                    Some(music_service::MODERATION_KIND_TRACK) => {
                        let track_id = PushNotificationPayload::string(
                            user_info,
                            &[push_route::TRACK_ID, push::ENTITY_ID],
                        )?;
                        Some(PushDeepLink::StudioTrack { track_id })
                    }
                    Some(music_service::MODERATION_KIND_ALBUM_REQUEST) => {
                        Some(PushDeepLink::MyAlbumRequests)
                    }
                    Some(music_service::MODERATION_KIND_ARTIST_REQUEST) => {
                        Some(PushDeepLink::MyArtistRequest)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn sparks_link(type_: &str) -> Option<PushDeepLink> {
        (type_ == RisePushType::SPARKS_GRANTED).then_some(PushDeepLink::SparksWallet)
    }

    fn achievement_link(type_: &str, user_info: &Map<String, Value>) -> Option<PushDeepLink> {
        if type_ != rise_events::name::ACHIEVEMENT_EARNED {
            return None;
        }
        let raw = PushNotificationPayload::string(user_info, &[user_key::ACHIEVEMENT_ID])?;
        let achievement_id = trimming_whitespaces_and_newlines(&raw).parse::<i64>().ok().filter(|id| *id > 0)?;
        Some(PushDeepLink::Achievement { achievement_id })
    }

    /// maps a canon route kind to an app screen, field names come from the route table
    fn link(route: &Route) -> Option<PushDeepLink> {
        let field = |name: &str| route.field(name).map(str::to_owned);
        match route.kind() {
            RouteKind::Chat => Some(PushDeepLink::Chat {
                chat_id: field(push_route::CHAT_ID)?,
            }),
            RouteKind::ChatTopic => Some(PushDeepLink::ChatTopic {
                chat_id: field(push_route::CHAT_ID)?,
                topic_id: field(push_route::TOPIC_ID)?,
            }),
            RouteKind::Post => Some(PushDeepLink::Post {
                post_id: field(push_route::POST_ID)?,
            }),
            RouteKind::PostComment => {
                let post_id = field(push_route::POST_ID)?;
                let comment_id = field(push_route::COMMENT_ID)?;
                Some(PushDeepLink::PostComment {
                    post_id,
                    comment_id: Some(comment_id),
                })
            }
            RouteKind::Profile => Some(PushDeepLink::Profile {
                user_id: field(push_route::USER_ID),
                tag: field(push_route::TAG),
            }),
            RouteKind::ContactJoinInbox => Some(PushDeepLink::ContactJoinInbox),
            RouteKind::ContactPhotoSuggestions => Some(PushDeepLink::ContactPhotoSuggestions),
            RouteKind::Story => Some(PushDeepLink::Story {
                story_id: field(push_route::STORY_ID)?,
            }),
            RouteKind::Music => Some(PushDeepLink::Music),
            RouteKind::MusicTrack => Some(PushDeepLink::MusicTrack {
                track_id: field(push_route::TRACK_ID)?,
            }),
            RouteKind::MusicReleaseTrack => Some(PushDeepLink::MusicReleaseTrack {
                track_id: field(push_route::TRACK_ID)?,
            }),
            RouteKind::MusicAlbum => Some(PushDeepLink::MusicAlbum {
                album_id: field(push_route::ALBUM_ID)?,
            }),
            RouteKind::Playlist => Some(PushDeepLink::Playlist {
                playlist_id: field(push_route::PLAYLIST_ID)?,
            }),
            RouteKind::PlaylistJoinRequests => Some(PushDeepLink::PlaylistJoinRequests {
                playlist_id: field(push_route::PLAYLIST_ID)?,
            }),
            RouteKind::PlaylistSession => Some(PushDeepLink::PlaylistSession {
                playlist_id: field(push_route::PLAYLIST_ID)?,
            }),
            RouteKind::StickerPackJoinRequests => Some(PushDeepLink::StickerPackJoinRequests {
                pack_id: field(push_route::PACK_ID)?,
            }),
            RouteKind::StickerPackDetail => Some(PushDeepLink::StickerPackDetail {
                pack_id: field(push_route::PACK_ID)?,
                pack_kind: field(push_route::PACK_KIND),
            }),
            RouteKind::Map => Some(PushDeepLink::Map {
                user_id: field(push_route::USER_ID),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushForegroundNotificationPresentation {
    pub title: String,
    pub body: Option<String>,
    pub avatar_url: Option<String>,
    pub avatar_name: Option<String>,
    pub avatar_user_id: Option<String>,
    pub route: Option<PushDeepLink>,
    pub author: Option<NotifierAuthor>,
}

/// what local caches know about a community message the push names
/// topic title and author username and avatar, which the payload leaves out
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PushForegroundNotificationPresentationCommunityDetails {
    pub topic_title: Option<String>,
    pub author_tag: Option<String>,
    pub author_avatar_url: Option<String>,
    pub speaks_as_chat: bool,
}

impl PushForegroundNotificationPresentation {
    /// in process banner for the screen the person has open now
    pub fn make(
        user_info: &Map<String, Value>,
        fallback_title: &str,
        fallback_body: &str,
        cx: &mut App,
    ) -> Option<PushForegroundNotificationPresentation> {
        let visible = SaiNavigationBridge::visible_screens(cx);
        if PushNotificationRouteParser::route(user_info)
            .is_some_and(|route| visible.iter().any(|screen| route.silences_banner(Some(screen))))
        {
            return None;
        }
        let active_screen = SaiNavigationBridge::current_screen(cx);
        let community = Self::cached_community_details(user_info, cx);
        Self::make_with_community(
            user_info,
            fallback_title,
            fallback_body,
            active_screen.as_ref(),
            community.as_ref(),
        )
    }

    fn cached_community_details(
        user_info: &Map<String, Value>,
        cx: &mut App,
    ) -> Option<PushForegroundNotificationPresentationCommunityDetails> {
        Self::community_kind(user_info)?;
        let chat_id = PushNotificationPayload::string(user_info, &[push_route::CHAT_ID])?;
        let mut details = PushForegroundNotificationPresentationCommunityDetails::default();
        if let Some(topic_id) = PushNotificationPayload::string(user_info, &[push::TOPIC_ID]) {
            details.topic_title = ChatServicesStore::shared(cx)
                .update(cx, |store, cx| store.topic(&chat_id, &topic_id, cx))
                .and_then(|topic| {
                    if topic.is_general { None } else { PushNotificationPayload::normalized_text(&topic.title) }
                });
        }
        let Some(sender_id) = PushNotificationPayload::string(user_info, &[push_route::SENDER_ID]) else {
            return Some(details);
        };
        let message_id = PushNotificationPayload::string(user_info, &[push::MESSAGE_ID]);
        let messages = MessageServicesStore::shared(cx)
            .update(cx, |store, cx| store.messages_in_memory(&chat_id, cx));
        let named = message_id
            .as_deref()
            .and_then(|id| messages.iter().find(|message| message.matches_message_id(id)));
        if sender_id == chat_id
            || named.is_some_and(|message| {
                message
                    .author_signature
                    .as_deref()
                    .and_then(PushNotificationPayload::normalized_text)
                    .is_some()
            })
        {
            details.speaks_as_chat = true;
            return Some(details);
        }
        let message = named
            .or_else(|| {
                messages
                    .iter()
                    .rev()
                    .find(|message| message.sender_id == sender_id && !message.sender_tag.is_empty())
            });
        if let Some(message) = message
            && message.sender_id == sender_id
        {
            details.author_tag = PushNotificationPayload::normalized_text(&message.sender_tag);
            details.author_avatar_url =
                message.sender_logo.as_deref().and_then(PushNotificationPayload::normalized_text);
        }
        if (details.author_tag.is_none() || details.author_avatar_url.is_none())
            && let Some(member) =
                ChatServicesStore::shared(cx).update(cx, |store, cx| store.member(&chat_id, &sender_id, cx))
        {
            if details.author_tag.is_none() {
                details.author_tag = PushNotificationPayload::normalized_text(&member.tag);
            }
            if details.author_avatar_url.is_none() {
                details.author_avatar_url = PushNotificationPayload::normalized_text(&member.avatar_url);
            }
        }
        Some(details)
    }

    /// group or channel, the push names the community and not the person
    fn community_kind(user_info: &Map<String, Value>) -> Option<ChatType> {
        let payload = NormalizedPushPayload::new(user_info);
        if rise_notify::payload_type_of(&payload) != Some(RisePushType::NEW_MESSAGE) {
            return None;
        }
        let raw = PushNotificationPayload::string(user_info, &[push::CHAT_TYPE])?;
        ChatType::matching(&raw).filter(|kind| kind.is_community())
    }

    pub fn make_with_active_screen(
        user_info: &Map<String, Value>,
        fallback_title: &str,
        fallback_body: &str,
        active_screen: Option<&SaiActiveScreen>,
    ) -> Option<PushForegroundNotificationPresentation> {
        Self::make_with_community(user_info, fallback_title, fallback_body, active_screen, None)
    }

    pub fn make_with_community(
        user_info: &Map<String, Value>,
        fallback_title: &str,
        fallback_body: &str,
        active_screen: Option<&SaiActiveScreen>,
        community: Option<&PushForegroundNotificationPresentationCommunityDetails>,
    ) -> Option<PushForegroundNotificationPresentation> {
        let payload = NormalizedPushPayload::new(user_info);
        let type_ = rise_notify::payload_type_of(&payload).map(str::to_owned);
        let route = PushNotificationRouteParser::route(user_info);
        if route
            .as_ref()
            .is_some_and(|route| route.silences_banner(active_screen))
        {
            return None;
        }

        let sender_name = PushNotificationPayload::string(
            user_info,
            &[push::SENDER_NAME, push::ACTOR_NAME, push::USER_NAME],
        );
        let payload_title = PushNotificationPayload::string(user_info, &[push::TITLE]);
        let normalized_fallback_title = PushNotificationPayload::normalized_text(fallback_title);
        let is_new_message = type_.as_deref() == Some(RisePushType::NEW_MESSAGE);

        let preferred_title = if is_new_message {
            sender_name
                .clone()
                .or_else(|| payload_title.clone())
                .or(normalized_fallback_title)
        } else {
            payload_title
                .clone()
                .or_else(|| sender_name.clone())
                .or(normalized_fallback_title)
        };

        let payload_body = PushNotificationPayload::string(user_info, &[push::MESSAGE_PREVIEW]);
        let normalized_fallback_body = PushNotificationPayload::normalized_text(fallback_body);
        let mut preferred_body = payload_body.clone().or(normalized_fallback_body);

        if let Some(type_) = type_.as_deref()
            && sender_name.is_some()
            && let Some(row_copy) = Self::playlist_track_row_copy(type_, user_info)
        {
            preferred_body = Some(row_copy);
        }

        if is_new_message
            && payload_body.is_none()
            && let Some(sender_name) = sender_name.as_deref()
            && let Some(fallback_body) = preferred_body.as_deref()
        {
            preferred_body = Some(Self::removing_sender_prefix(sender_name, fallback_body));
        }

        if preferred_title.is_none() && preferred_body.is_none() {
            return None;
        }
        if preferred_body.as_deref().is_some_and(Self::is_creation_notice) {
            return None;
        }

        let title = preferred_title
            .clone()
            .or_else(|| preferred_body.clone())
            .unwrap_or_default();
        let body = if preferred_title.is_none() {
            None
        } else {
            preferred_body.clone()
        };
        let avatar_url = PushNotificationPayload::string(
            user_info,
            &[
                push::AVATAR_URL,
                push::SENDER_AVATAR,
                push::ACTOR_AVATAR,
                push::USER_LOGO,
                push::camel::USER_LOGO,
            ],
        );
        let avatar_user_id = PushNotificationPayload::string(
            user_info,
            &[
                push_route::SENDER_ID,
                push_route::ACTOR_ID,
                push_route::USER_ID,
                push::FOLLOWER_ID,
            ],
        );

        if let Some(kind) = Self::community_kind(user_info)
            && let Some(chat_title) = PushNotificationPayload::string(user_info, &[push::CHAT_TITLE])
        {
            let topic_title = community.and_then(|community| community.topic_title.clone());
            let author_tag = PushNotificationPayload::string(user_info, &[push::ACTOR_TAG])
                .or_else(|| community.and_then(|community| community.author_tag.clone()));
            let community_title = match topic_title {
                Some(topic_title) => format!("{chat_title} \u{203A} {topic_title}"),
                None => chat_title.clone(),
            };
            let is_system_notice = matches!(
                PushNotificationPayload::string(user_info, &[push::NOTIFICATION_SCOPE])
                    .and_then(|raw| NotificationScope::parse(&raw)),
                Some(NotificationScope::Events)
            );
            let speaks_as_chat = community.is_some_and(|community| community.speaks_as_chat)
                || avatar_user_id.as_deref().is_some_and(|sender_id| {
                    PushNotificationPayload::string(user_info, &[push_route::CHAT_ID]).as_deref() == Some(sender_id)
                });
            let author = if kind.is_broadcast() || is_system_notice || speaks_as_chat {
                None
            } else {
                sender_name.map(|name| NotifierAuthor {
                    name,
                    tag: author_tag,
                    avatar_url: community.and_then(|community| community.author_avatar_url.clone()),
                    user_id: avatar_user_id.clone(),
                })
            };
            return Some(PushForegroundNotificationPresentation {
                title: community_title,
                body: preferred_body,
                avatar_url,
                avatar_name: Some(chat_title),
                avatar_user_id: PushNotificationPayload::string(user_info, &[push_route::CHAT_ID]),
                route,
                author,
            });
        }

        Some(PushForegroundNotificationPresentation {
            title,
            body,
            avatar_url,
            avatar_name: sender_name.or(preferred_title),
            avatar_user_id,
            route,
            author: None,
        })
    }

    fn playlist_track_row_copy(type_: &str, user_info: &Map<String, Value>) -> Option<String> {
        if type_ != RisePushType::PLAYLIST_TRACK_ADDED && type_ != RisePushType::PLAYLIST_TRACK_REMOVED
        {
            return None;
        }
        let key = NotifyCopyCanon::client_key(type_, false)?;
        let is_default_local =
            PushNotificationPayload::string(user_info, &[push::PLAYLIST_IS_DEFAULT_LOCAL])
                .as_deref()
                == Some("true");
        let playlist_title = PushNotificationPayload::string(user_info, &[push::PLAYLIST_TITLE]);
        let values: HashMap<String, String> = HashMap::from([
            (
                events::TITLE.to_owned(),
                PushNotificationPayload::string(user_info, &[push::TRACK_TITLE])
                    .unwrap_or_default(),
            ),
            (
                push::PLAYLIST_TITLE.to_owned(),
                MusicDefaultLocalPlaylist::resolved_title(
                    playlist_title.as_deref(),
                    Some(is_default_local),
                ),
            ),
        ]);
        let arguments = NotifyCopyCanon::slot_arguments(type_, &values)?;
        let arguments: Vec<FormatArg<'_>> = arguments.iter().map(FormatArg::from).collect();
        Some(tr_args(&key, &arguments))
    }

    fn is_creation_notice(body: &str) -> bool {
        let trimmed = trimming_whitespaces_and_newlines(body);
        trimmed.ends_with("' was created")
            && ["Chat '", "Group '", "Channel '", "Supergroup '"].iter().any(|prefix| trimmed.starts_with(prefix))
    }

    fn removing_sender_prefix(sender_name: &str, body: &str) -> String {
        let prefix = format!("{sender_name}:");
        let anchored_case_insensitive =
            CompareOptions::ANCHORED.union(CompareOptions::CASE_INSENSITIVE);
        if range_of(body, &prefix, anchored_case_insensitive).is_none() {
            return body.to_owned();
        }
        let remainder = trimming_whitespaces_and_newlines(drop_first_characters(
            body,
            character_count(&prefix),
        ));
        if remainder.is_empty() {
            body.to_owned()
        } else {
            remainder.to_owned()
        }
    }
}

enum PushNotificationPayload {}

impl PushNotificationPayload {
    fn string(user_info: &Map<String, Value>, keys: &[&str]) -> Option<String> {
        keys.iter()
            .find_map(|key| Self::normalized(user_info.get(*key)))
    }

    /// payload value read as a string or number or integer
    /// a bool is 1 or 0 and anything else is absent
    fn normalized(value: Option<&Value>) -> Option<String> {
        let raw = match value? {
            Value::String(string) => string.clone(),
            Value::Bool(flag) => (if *flag { "1" } else { "0" }).to_owned(),
            Value::Number(number) => Self::number_string_value(number),
            Value::Null | Value::Array(_) | Value::Object(_) => return None,
        };
        Self::normalized_text(&raw)
    }

    fn normalized_text(raw: &str) -> Option<String> {
        let result = trimming_whitespaces_and_newlines(raw);
        (!result.is_empty()).then(|| result.to_owned())
    }

    fn number_string_value(number: &serde_json::Number) -> String {
        if let Some(integer) = number.as_i64() {
            return integer.to_string();
        }
        if let Some(integer) = number.as_u64() {
            return integer.to_string();
        }
        match number.as_f64() {
            Some(double) if double.is_finite() && double.fract() == 0.0 && double.abs() < 1e15 => {
                format!("{double:.0}")
            }
            Some(double) => double.to_string(),
            None => number.to_string(),
        }
    }
}

/// payload as rise_notify reads it, every value already coerced
struct NormalizedPushPayload {
    values: HashMap<String, String>,
}

impl NormalizedPushPayload {
    fn new(user_info: &Map<String, Value>) -> Self {
        let values = user_info
            .iter()
            .filter_map(|(key, value)| {
                PushNotificationPayload::normalized(Some(value)).map(|text| (key.clone(), text))
            })
            .collect();
        Self { values }
    }
}

impl PushPayload for NormalizedPushPayload {
    fn raw(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
}
