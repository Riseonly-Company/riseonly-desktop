use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use gpui::{Hsla, white};
use rise_canon::rise_key::{events, music, push, user};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{FormatArg, format_printf, tr};
use rise_notify::{CopySpec, NOTIFICATION_COPY, NOTIFICATION_TYPES, NotificationTypeSpec};
use rise_theme::{AppTheme, color};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::core::push_notifications::admin_console_link::AdminConsoleLink;
use crate::modules::music::shared::utils::music::music_models::MusicDefaultLocalPlaylist;
use crate::modules::notify::components::notification_avatar_stack::NotificationAvatarGlyph;

/// notification copy catalog from canon
/// the event to key mapping was a hand written switch per client and drifted
/// so the table lives only in canon
pub enum NotifyCopyCanon {}

static TYPES_BY_WIRE: LazyLock<HashMap<&'static str, NotificationTypeSpec>> = LazyLock::new(|| {
    let mut map = HashMap::with_capacity(NOTIFICATION_TYPES.len());
    for spec in NOTIFICATION_TYPES {
        map.entry(spec.wire).or_insert(spec);
    }
    map
});

static COPY_BY_EVENT: LazyLock<HashMap<&'static str, &'static CopySpec>> = LazyLock::new(|| {
    let mut map = HashMap::with_capacity(NOTIFICATION_COPY.len());
    for spec in NOTIFICATION_COPY.iter() {
        map.entry(spec.event).or_insert(spec);
    }
    map
});

static GENERIC_ROW_KEY: LazyLock<&'static str> =
    LazyLock::new(|| match COPY_BY_EVENT.get("aggregate_generic").and_then(|copy| copy.client_key) {
        Some(key) => key,
        None => panic!("Canon copy aggregate_generic has no client key"),
    });

impl NotifyCopyCanon {
    /// feed placeholder
    /// canon keeps it as the generic aggregate row and says the client then shows server text
    /// instead of its own
    pub fn generic_row_key() -> &'static str {
        *GENERIC_ROW_KEY
    }

    pub fn r#type(wire: &str) -> Option<NotificationTypeSpec> {
        TYPES_BY_WIRE.get(wire).copied()
    }

    /// a key missing from canon must fail on the first screen not print raw to the user
    pub fn chrome_key(event: &str) -> &'static str {
        match COPY_BY_EVENT.get(event).and_then(|copy| copy.client_key) {
            Some(key) => key,
            None => panic!("Canon copy {event} has no client key"),
        }
    }

    /// none means the type is absent from the canon enum
    pub fn row_key(wire: &str, many: bool) -> Option<&'static str> {
        let spec = TYPES_BY_WIRE.get(wire)?;
        Some(if many { spec.row_i18n_key_many } else { spec.row_i18n_key_one })
    }

    /// none means the event has no client copy
    pub fn client_key(event: &str, many: bool) -> Option<&'static str> {
        let copy = COPY_BY_EVENT.get(event)?;
        if many { copy.client_key_many.or(copy.client_key) } else { copy.client_key }
    }

    /// used when the placeholder substitutions couldnt be assembled
    pub fn fallback_key(event: &str) -> Option<&'static str> {
        COPY_BY_EVENT.get(event)?.client_key_fallback
    }

    pub fn prefers_server_body(event: &str) -> bool {
        COPY_BY_EVENT.get(event).is_some_and(|copy| copy.client_prefers_server_body)
    }

    pub fn omits_server_context(event: &str) -> bool {
        COPY_BY_EVENT.get(event).is_some_and(|copy| {
            copy.placeholders
                .iter()
                .any(|placeholder| placeholder.in_server && placeholder.client_source == "omitted")
        })
    }

    /// format arguments in the slot order canon declares
    /// none means a slot is missing so use the fallback key
    pub fn slot_arguments(event: &str, values: &HashMap<String, String>) -> Option<Vec<String>> {
        let copy = COPY_BY_EVENT.get(event)?;
        let mut slots: Vec<(u8, &str)> = copy
            .placeholders
            .iter()
            .filter_map(|placeholder| placeholder.client_slot.map(|slot| (slot, placeholder.name)))
            .collect();
        slots.sort_by_key(|(slot, _)| *slot);

        let mut arguments = Vec::with_capacity(slots.len());
        for (_, name) in slots {
            let value = values.get(name).filter(|value| !value.is_empty())?;
            arguments.push(value.clone());
        }
        Some(arguments)
    }
}

/// type as it arrives in notificationType, mirrors the canon enum
/// last four cases arent canon types
/// three come only in the push payload data.type, the fourth takes anything unknown
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationKind {
    PostLike,
    CommentLike,
    CommentReply,
    Subscription,
    Mention,
    System,
    NewLogin,
    NewFollower,
    Moderation,
    NewPost,
    NewStory,
    NewStoryLike,
    VerificationApproved,
    VerificationRejected,
    PostRepost,
    Music,
    Announcement,
    SamePostLike,
    RepostLike,
    CoauthorInvite,
    CoauthorAccepted,
    PostTagged,
    SparksReceived,
    PostComment,
    Follow,
    Chat,
    Other,
}

impl NotificationKind {
    pub const ALL_CASES: [NotificationKind; 27] = [
        Self::PostLike,
        Self::CommentLike,
        Self::CommentReply,
        Self::Subscription,
        Self::Mention,
        Self::System,
        Self::NewLogin,
        Self::NewFollower,
        Self::Moderation,
        Self::NewPost,
        Self::NewStory,
        Self::NewStoryLike,
        Self::VerificationApproved,
        Self::VerificationRejected,
        Self::PostRepost,
        Self::Music,
        Self::Announcement,
        Self::SamePostLike,
        Self::RepostLike,
        Self::CoauthorInvite,
        Self::CoauthorAccepted,
        Self::PostTagged,
        Self::SparksReceived,
        Self::PostComment,
        Self::Follow,
        Self::Chat,
        Self::Other,
    ];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::PostLike => "post_like",
            Self::CommentLike => "comment_like",
            Self::CommentReply => "comment_reply",
            Self::Subscription => "subscription",
            Self::Mention => "mention",
            Self::System => "system",
            Self::NewLogin => "new_login",
            Self::NewFollower => "new_follower",
            Self::Moderation => "moderation_result",
            Self::NewPost => "new_post",
            Self::NewStory => "new_story",
            Self::NewStoryLike => "new_story_like",
            Self::VerificationApproved => "verification_approved",
            Self::VerificationRejected => "verification_rejected",
            Self::PostRepost => "post_repost",
            Self::Music => "music",
            Self::Announcement => "announcement",
            Self::SamePostLike => "same_post_like",
            Self::RepostLike => "repost_like",
            Self::CoauthorInvite => "coauthor_invite",
            Self::CoauthorAccepted => "coauthor_accepted",
            Self::PostTagged => "post_tagged",
            Self::SparksReceived => "sparks_received",
            Self::PostComment => "post_comment",
            Self::Follow => "follow",
            Self::Chat => "chat_message",
            Self::Other => "other",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|kind| kind.raw_value() == raw)
    }

    pub fn from_raw_string(raw: &str) -> Self {
        Self::from_raw_value(raw).unwrap_or(Self::Other)
    }

    pub fn is_follow(self) -> bool {
        matches!(self, Self::Subscription | Self::NewFollower | Self::Follow)
    }

    pub fn is_like(self) -> bool {
        self == Self::PostLike
    }

    pub fn is_comment_like(self) -> bool {
        self == Self::CommentLike
    }

    pub fn is_music(self) -> bool {
        self == Self::Music
    }

    pub fn is_comment_related(self) -> bool {
        matches!(self, Self::CommentReply | Self::PostComment | Self::Mention | Self::CommentLike)
    }

    pub fn canon_row_wire(self) -> Option<&'static str> {
        match self {
            Self::Follow => Some(Self::NewFollower.raw_value()),
            Self::PostComment | Self::Chat | Self::Other => None,
            _ => Some(self.raw_value()),
        }
    }

    pub fn canon_badge_wire(self) -> &'static str {
        match self {
            Self::Follow => Self::NewFollower.raw_value(),
            Self::PostComment | Self::Chat => Self::CommentReply.raw_value(),
            Self::Other => Self::System.raw_value(),
            _ => self.raw_value(),
        }
    }
}

impl Serialize for NotificationKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.raw_value())
    }
}

impl<'de> Deserialize<'de> for NotificationKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::from_raw_string(&raw))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationActorItem {
    pub id: i64,
    pub actor_id: String,
    pub actor_name: String,
    pub actor_tag: String,
    pub actor_avatar: Option<String>,
}

impl NotificationActorItem {
    /// translate resolves the anonymous actor chrome key
    /// prepare passes its frozen table, a main thread caller passes tr
    pub fn display_name_with(&self, translate: &dyn Fn(&str) -> String) -> String {
        if !self.actor_name.is_empty() {
            return self.actor_name.clone();
        }
        if !self.actor_tag.is_empty() {
            return format!("@{}", self.actor_tag);
        }
        translate(NotifyCopyCanon::chrome_key("actor_fallback"))
    }

    pub fn display_name(&self) -> String {
        self.display_name_with(&tr)
    }
}

/// feed row as the engine keeps it
/// parsed once from the wire, persisted as is
/// drawn by the snapshot builder
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationItem {
    pub id: String,
    pub kind: NotificationKind,
    pub title: String,
    pub body: String,
    pub data: BTreeMap<String, String>,
    pub is_read: bool,
    pub created_at_ms: i64,
    pub group_count: i64,
    pub actors: Vec<NotificationActorItem>,
}

impl NotificationItem {
    pub fn primary_actor(&self) -> Option<&NotificationActorItem> {
        self.actors.first()
    }

    pub fn total_count(&self) -> i64 {
        let actors = i64::try_from(self.actors.len()).unwrap_or(i64::MAX);
        self.group_count.max(actors).max(1)
    }

    pub fn with_read(&self, read: bool) -> NotificationItem {
        NotificationItem { is_read: read, ..self.clone() }
    }

    fn value(&self, key: &str) -> Option<String> {
        self.data.get(key).cloned()
    }

    pub fn post_id(&self) -> Option<String> {
        self.value(events::POST_ID)
    }

    pub fn comment_id(&self) -> Option<String> {
        self.value(events::COMMENT_ID)
    }

    pub fn parent_id(&self) -> Option<String> {
        self.value(events::PARENT_ID)
    }

    pub fn chat_id(&self) -> Option<String> {
        self.value(events::CHAT_ID)
    }

    pub fn track_id(&self) -> Option<String> {
        self.value(events::TRACK_ID)
    }

    pub fn album_id(&self) -> Option<String> {
        self.value(music::ALBUM_ID)
    }

    pub fn playlist_id(&self) -> Option<String> {
        self.value(events::PLAYLIST_ID)
    }

    pub fn entity_id(&self) -> String {
        self.value(user::ENTITY_ID).unwrap_or_default()
    }

    pub fn event_type(&self) -> String {
        self.value(events::TYPE).unwrap_or_default()
    }

    /// id of the achievement an earned achievement row names
    pub fn achievement_id(&self) -> Option<i64> {
        if self.event_type() != rise_events::name::ACHIEVEMENT_EARNED {
            return None;
        }
        let raw = self.value(user::ACHIEVEMENT_ID)?;
        trimming_whitespaces_and_newlines(&raw).parse::<i64>().ok().filter(|id| *id > 0)
    }

    pub fn action(&self) -> String {
        self.value(events::ACTION).unwrap_or_default()
    }

    /// operational alert for admins
    /// server heading and body under a system tile
    pub fn is_admin_alert(&self) -> bool {
        let event_type = self.event_type();
        event_type == AdminConsoleLink::ALERT_TYPE || event_type == AdminConsoleLink::INCIDENT_TYPE
    }

    /// web admin page the row names, as received
    /// tap opens it only once AdminConsoleLink accepts it, a refused one opens nothing
    pub fn admin_url(&self) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(self.data.get(AdminConsoleLink::PAYLOAD_KEY)?);
        if value.is_empty() { None } else { Some(value.to_owned()) }
    }

    pub fn source_user_id(&self) -> Option<String> {
        self.value(events::USER_ID)
            .or_else(|| self.value(push::FOLLOWER_ID))
            .or_else(|| self.primary_actor().map(|actor| actor.actor_id.clone()))
    }

    pub fn source_user_tag(&self) -> Option<String> {
        self.value(push::USER_TAG)
            .or_else(|| self.value(events::TAG))
            .or_else(|| self.primary_actor().map(|actor| actor.actor_tag.clone()))
    }

    pub fn facepile_actors(&self) -> Vec<NotificationActorItem> {
        if !self.actors.is_empty() {
            return self.actors.iter().take(3).cloned().collect();
        }
        let name = self.value(push::USER_NAME).unwrap_or_default();
        let logo = self.value(push::USER_LOGO).or_else(|| self.value(events::AVATAR_URL));
        let uid = self.source_user_id().unwrap_or_default();
        if name.is_empty() && logo.as_deref().unwrap_or_default().is_empty() && uid.is_empty() {
            return Vec::new();
        }
        vec![NotificationActorItem {
            id: 0,
            actor_id: uid,
            actor_name: name,
            actor_tag: self.source_user_tag().unwrap_or_default(),
            actor_avatar: logo.filter(|logo| !logo.is_empty()),
        }]
    }

    /// the bold names run
    /// one name, two joined
    /// or two and a remainder
    pub fn names_string(&self, translate: &dyn Fn(&str) -> String) -> String {
        let names: Vec<String> =
            self.facepile_actors().iter().take(2).map(|actor| actor.display_name_with(translate)).collect();
        let Some(first) = names.first() else {
            return String::new();
        };
        let total = self.total_count();
        let second = match names.get(1) {
            Some(second) if total > 1 => second,
            _ => return first.clone(),
        };
        if total == 2 {
            return format_printf(
                &translate(NotifyCopyCanon::chrome_key("feed_names_two")),
                &[FormatArg::from(first), FormatArg::from(second)],
            );
        }
        format_printf(
            &translate(NotifyCopyCanon::chrome_key("feed_names_more")),
            &[FormatArg::from(first), FormatArg::from(second), FormatArg::from(total - 2)],
        )
    }

    /// regular run after the names, the action copy of the rows kind
    pub fn action_suffix(&self, translate: &dyn Fn(&str) -> String) -> String {
        let many = self.total_count() > 1;

        if self.kind == NotificationKind::Music {
            return self.music_action_suffix(translate);
        }

        if self.kind == NotificationKind::PostComment
            && let Some(key) = NotifyCopyCanon::client_key(
                if self.parent_id().is_none() { "post_commented" } else { "comment_reply" },
                many,
            )
        {
            return translate(key);
        }

        if self.facepile_actors().is_empty()
            && NotifyCopyCanon::omits_server_context(&self.event_type())
            && !trimming_whitespaces_and_newlines(&self.body).is_empty()
        {
            return self.body.clone();
        }

        if let Some(wire) = self.kind.canon_row_wire()
            && let Some(key) = NotifyCopyCanon::row_key(wire, many)
            && key != NotifyCopyCanon::generic_row_key()
        {
            return translate(key);
        }

        self.body_or_generic(translate)
    }

    fn body_or_generic(&self, translate: &dyn Fn(&str) -> String) -> String {
        if self.body.is_empty() { translate(NotifyCopyCanon::generic_row_key()) } else { self.body.clone() }
    }

    fn music_placeholder_values(&self, translate: &dyn Fn(&str) -> String) -> HashMap<String, String> {
        let track_title = self.value(push::TRACK_TITLE).unwrap_or_default();
        let album_title = self.value(push::ALBUM_TITLE).unwrap_or_default();
        let title = if self.event_type() == "artist_album_released" { album_title } else { track_title };
        let playlist_title = MusicDefaultLocalPlaylist::resolved_title_with(
            self.data.get(push::PLAYLIST_TITLE).map(String::as_str),
            Some(self.data.get(push::PLAYLIST_IS_DEFAULT_LOCAL).map(String::as_str) == Some("true")),
            translate,
        );
        HashMap::from([(events::TITLE.to_owned(), title), (push::PLAYLIST_TITLE.to_owned(), playlist_title)])
    }

    fn music_action_suffix(&self, translate: &dyn Fn(&str) -> String) -> String {
        let event = self.event_type();

        let Some(key) = NotifyCopyCanon::client_key(&event, false) else {
            return self.body_or_generic(translate);
        };

        if NotifyCopyCanon::prefers_server_body(&event) && !self.body.is_empty() {
            return self.body.clone();
        }

        if self.facepile_actors().is_empty()
            && NotifyCopyCanon::omits_server_context(&event)
            && !trimming_whitespaces_and_newlines(&self.body).is_empty()
        {
            return self.body.clone();
        }

        let Some(arguments) = NotifyCopyCanon::slot_arguments(&event, &self.music_placeholder_values(translate))
        else {
            let Some(fallback) = NotifyCopyCanon::fallback_key(&event) else {
                return self.body_or_generic(translate);
            };
            return if self.body.is_empty() { translate(fallback) } else { self.body.clone() };
        };

        if arguments.is_empty() {
            return translate(key);
        }
        let arguments: Vec<FormatArg<'_>> = arguments.iter().map(FormatArg::from).collect();
        format_printf(&translate(key), &arguments)
    }

    /// icon stays local, canon doesnt know sf symbols
    pub fn badge_symbol(&self) -> &'static str {
        if self.is_admin_alert() {
            return "exclamationmark.triangle.fill";
        }
        if self.achievement_id().is_some() {
            return "trophy.fill";
        }
        match self.kind {
            NotificationKind::PostLike
            | NotificationKind::CommentLike
            | NotificationKind::NewStoryLike
            | NotificationKind::SamePostLike
            | NotificationKind::RepostLike => "heart.fill",
            NotificationKind::CommentReply | NotificationKind::PostComment | NotificationKind::Chat => {
                "bubble.left.fill"
            }
            NotificationKind::Mention => "at",
            NotificationKind::Subscription | NotificationKind::NewFollower | NotificationKind::Follow => {
                "person.fill.badge.plus"
            }
            NotificationKind::Moderation | NotificationKind::Music => "music.note",
            NotificationKind::NewPost => "doc.text.fill",
            NotificationKind::PostRepost => "arrow.2.squarepath",
            NotificationKind::CoauthorInvite | NotificationKind::CoauthorAccepted => "person.2.fill",
            NotificationKind::PostTagged => "person.crop.square",
            NotificationKind::SparksReceived => "sparkles",
            NotificationKind::NewStory => "circle.dashed",
            NotificationKind::VerificationApproved => "checkmark.seal.fill",
            NotificationKind::VerificationRejected => "xmark.seal.fill",
            NotificationKind::Announcement => "megaphone.fill",
            NotificationKind::System | NotificationKind::NewLogin | NotificationKind::Other => "bell.fill",
        }
    }

    pub fn empty_avatar_glyph(&self, theme: &AppTheme) -> Option<NotificationAvatarGlyph> {
        let product_written = self.kind == NotificationKind::Announcement
            || self.achievement_id().is_some()
            || (self.kind == NotificationKind::SparksReceived && self.event_type() == "sparks_granted")
            || (self.kind == NotificationKind::Music && self.facepile_actors().is_empty())
            || self.is_admin_alert();
        if !product_written {
            return None;
        }
        Some(NotificationAvatarGlyph {
            symbol: self.badge_symbol().to_owned(),
            background: self.badge_color(theme),
            foreground: white(),
        })
    }

    /// canon declares the badge color as a theme token name not a color
    /// palette stays local, the type to token mapping is shared
    pub fn badge_color(&self, theme: &AppTheme) -> Hsla {
        let Some(spec) = NotifyCopyCanon::r#type(self.kind.canon_badge_wire()) else {
            return theme.text.secondary;
        };
        if let Some(hex) = spec.badge_hex {
            return color(hex);
        }
        match spec.badge_theme_token {
            Some("primary_100") => theme.primary._100,
            Some("success_100") => theme.semantic.success_100,
            Some("error_100") => theme.semantic.error_100,
            _ => theme.text.secondary,
        }
    }
}
