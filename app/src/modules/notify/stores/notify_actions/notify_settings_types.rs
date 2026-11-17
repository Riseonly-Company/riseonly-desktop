use std::sync::LazyLock;

use rise_canon::rise_key::notify;
use rise_notify::{SETTINGS_FLAGS, SettingsFlagSpec};

pub type NotificationSettingsModel = rise_notify::NotificationSettings;

/// a missing flag must fail the build
/// a silently dropped toggle is a setting the user turned off while the server keeps sending it
pub fn canon_notification_flag(wire: &str) -> SettingsFlagSpec {
    match SETTINGS_FLAGS.iter().find(|spec| spec.wire == wire) {
        Some(spec) => *spec,
        None => panic!("Canon notification flag {wire} is missing"),
    }
}

pub trait NotificationSettingsField: Copy {
    fn value_in(self, model: &NotificationSettingsModel) -> bool;
}

impl NotificationSettingsField for NotificationCategory {
    fn value_in(self, model: &NotificationSettingsModel) -> bool {
        match self {
            NotificationCategory::Messages => model.messages_enabled,
            NotificationCategory::Mentions => model.mentions_enabled,
            NotificationCategory::Replies => model.replies_enabled,
            NotificationCategory::Comments => model.comments_enabled,
            NotificationCategory::PostLikes => model.post_likes_enabled,
            NotificationCategory::CommentLikes => model.comment_likes_enabled,
            NotificationCategory::NewFollowers => model.new_followers_enabled,
            NotificationCategory::NewPosts => model.new_posts_enabled,
            NotificationCategory::Stories => model.stories_enabled,
            NotificationCategory::Coauthors => model.coauthors_enabled,
            NotificationCategory::Tags => model.tags_enabled,
            NotificationCategory::MapReactions => model.map_reactions_enabled,
            NotificationCategory::Sparks => model.sparks_enabled,
            NotificationCategory::Announcements => model.announcements_enabled,
        }
    }
}

impl NotificationSettingsField for MusicNotificationSetting {
    fn value_in(self, model: &NotificationSettingsModel) -> bool {
        match self {
            MusicNotificationSetting::SessionJoined => model.music_session_joined_enabled,
            MusicNotificationSetting::SessionLeft => model.music_session_left_enabled,
            MusicNotificationSetting::SessionEnded => model.music_session_ended_enabled,
            MusicNotificationSetting::SessionPermissionRequests => model.music_session_permission_requests_enabled,
            MusicNotificationSetting::SessionPermissionUpdates => model.music_session_permission_updates_enabled,
            MusicNotificationSetting::PlaylistTrackAdded => model.music_playlist_track_added_enabled,
            MusicNotificationSetting::PlaylistTrackRemoved => model.music_playlist_track_removed_enabled,
            MusicNotificationSetting::ArtistReleases => model.music_artist_releases_enabled,
            MusicNotificationSetting::PlaylistAccess => model.music_playlist_access_enabled,
            MusicNotificationSetting::PlaylistListeningStarted => model.music_playlist_listening_started_enabled,
        }
    }
}

/// extension to read a notification settings value by key
pub trait NotificationSettingsModelValues {
    fn value(&self, field: impl NotificationSettingsField) -> bool;
}

impl NotificationSettingsModelValues for NotificationSettingsModel {
    fn value(&self, field: impl NotificationSettingsField) -> bool {
        field.value_in(self)
    }
}

/// wire_key is the canon flag so the i18n key and badge color come from canon
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationCategory {
    Messages,
    Mentions,
    Replies,
    Comments,
    PostLikes,
    CommentLikes,
    NewFollowers,
    NewPosts,
    Stories,
    Coauthors,
    Tags,
    MapReactions,
    Sparks,
    Announcements,
}

impl NotificationCategory {
    pub const ALL_CASES: [NotificationCategory; 14] = [
        Self::Messages,
        Self::Mentions,
        Self::Replies,
        Self::Comments,
        Self::PostLikes,
        Self::CommentLikes,
        Self::NewFollowers,
        Self::NewPosts,
        Self::Stories,
        Self::Coauthors,
        Self::Tags,
        Self::MapReactions,
        Self::Sparks,
        Self::Announcements,
    ];

    /// the canon notification flag this row toggles
    pub fn wire_key(self) -> &'static str {
        match self {
            Self::Messages => notify::MESSAGES_ENABLED,
            Self::Mentions => notify::MENTIONS_ENABLED,
            Self::Replies => notify::REPLIES_ENABLED,
            Self::Comments => notify::COMMENTS_ENABLED,
            Self::PostLikes => notify::POST_LIKES_ENABLED,
            Self::CommentLikes => notify::COMMENT_LIKES_ENABLED,
            Self::NewFollowers => notify::NEW_FOLLOWERS_ENABLED,
            Self::NewPosts => notify::NEW_POSTS_ENABLED,
            Self::Stories => notify::STORIES_ENABLED,
            Self::Coauthors => notify::COAUTHORS_ENABLED,
            Self::Tags => notify::TAGS_ENABLED,
            Self::MapReactions => notify::MAP_REACTIONS_ENABLED,
            Self::Sparks => notify::SPARKS_ENABLED,
            Self::Announcements => notify::ANNOUNCEMENTS_ENABLED,
        }
    }

    pub fn from_wire_key(wire_key: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|category| category.wire_key() == wire_key)
    }

    pub fn id(self) -> &'static str {
        self.wire_key()
    }

    pub fn title_key(self) -> &'static str {
        canon_notification_flag(self.wire_key()).i18n_key
    }

    pub fn icon_hex(self) -> &'static str {
        canon_notification_flag(self.wire_key()).icon_hex
    }

    /// icon stays local, canon doesnt know sf symbols
    pub fn icon(self) -> &'static str {
        match self {
            Self::Messages => "bubble.left.and.bubble.right",
            Self::Mentions => "at",
            Self::Replies => "arrowshape.turn.up.left",
            Self::Comments => "text.bubble",
            Self::PostLikes => "heart",
            Self::CommentLikes => "hand.thumbsup",
            Self::NewFollowers => "person.badge.plus",
            Self::NewPosts => "doc.text",
            Self::Stories => "circle.dashed",
            Self::Coauthors => "person.2.fill",
            Self::Tags => "person.crop.square",
            Self::MapReactions => "map.fill",
            Self::Sparks => "sparkles",
            Self::Announcements => "megaphone",
        }
    }
}

/// wire_key is the canon flag
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MusicNotificationSetting {
    SessionJoined,
    SessionLeft,
    SessionEnded,
    SessionPermissionRequests,
    SessionPermissionUpdates,
    PlaylistTrackAdded,
    PlaylistTrackRemoved,
    ArtistReleases,
    PlaylistAccess,
    PlaylistListeningStarted,
}

static SESSION_CASES: LazyLock<Vec<MusicNotificationSetting>> =
    LazyLock::new(|| MusicNotificationSetting::cases("music_session"));
static GENERAL_CASES: LazyLock<Vec<MusicNotificationSetting>> =
    LazyLock::new(|| MusicNotificationSetting::cases("music_general"));

impl MusicNotificationSetting {
    pub const ALL_CASES: [MusicNotificationSetting; 10] = [
        Self::SessionJoined,
        Self::SessionLeft,
        Self::SessionEnded,
        Self::SessionPermissionRequests,
        Self::SessionPermissionUpdates,
        Self::PlaylistTrackAdded,
        Self::PlaylistTrackRemoved,
        Self::ArtistReleases,
        Self::PlaylistAccess,
        Self::PlaylistListeningStarted,
    ];

    /// the canon notification flag this row toggles
    pub fn wire_key(self) -> &'static str {
        match self {
            Self::SessionJoined => notify::MUSIC_SESSION_JOINED_ENABLED,
            Self::SessionLeft => notify::MUSIC_SESSION_LEFT_ENABLED,
            Self::SessionEnded => notify::MUSIC_SESSION_ENDED_ENABLED,
            Self::SessionPermissionRequests => notify::MUSIC_SESSION_PERMISSION_REQUESTS_ENABLED,
            Self::SessionPermissionUpdates => notify::MUSIC_SESSION_PERMISSION_UPDATES_ENABLED,
            Self::PlaylistTrackAdded => notify::MUSIC_PLAYLIST_TRACK_ADDED_ENABLED,
            Self::PlaylistTrackRemoved => notify::MUSIC_PLAYLIST_TRACK_REMOVED_ENABLED,
            Self::ArtistReleases => notify::MUSIC_ARTIST_RELEASES_ENABLED,
            Self::PlaylistAccess => notify::MUSIC_PLAYLIST_ACCESS_ENABLED,
            Self::PlaylistListeningStarted => notify::MUSIC_PLAYLIST_LISTENING_STARTED_ENABLED,
        }
    }

    pub fn from_wire_key(wire_key: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|setting| setting.wire_key() == wire_key)
    }

    pub fn session_cases() -> &'static [MusicNotificationSetting] {
        &SESSION_CASES
    }

    pub fn general_cases() -> &'static [MusicNotificationSetting] {
        &GENERAL_CASES
    }

    fn cases(group: &str) -> Vec<MusicNotificationSetting> {
        SETTINGS_FLAGS
            .iter()
            .filter(|flag| flag.group == group)
            .map(|flag| match Self::from_wire_key(flag.wire) {
                Some(setting) => setting,
                None => panic!("Canon music flag {} has no screen row", flag.wire),
            })
            .collect()
    }

    pub fn id(self) -> &'static str {
        self.wire_key()
    }

    pub fn title_key(self) -> &'static str {
        canon_notification_flag(self.wire_key()).i18n_key
    }

    pub fn icon_hex(self) -> &'static str {
        canon_notification_flag(self.wire_key()).icon_hex
    }

    /// icon stays local, canon doesnt know sf symbols
    pub fn icon(self) -> &'static str {
        match self {
            Self::SessionJoined => "person.badge.plus",
            Self::SessionLeft => "person.badge.minus",
            Self::SessionEnded => "stop.circle.fill",
            Self::SessionPermissionRequests => "hand.raised.fill",
            Self::SessionPermissionUpdates => "slider.horizontal.3",
            Self::PlaylistTrackAdded => "text.badge.plus",
            Self::PlaylistTrackRemoved => "text.badge.minus",
            Self::ArtistReleases => "sparkles",
            Self::PlaylistAccess => "person.2.fill",
            Self::PlaylistListeningStarted => "play.circle.fill",
        }
    }
}

pub enum NotificationSettingsDebounce {}

impl NotificationSettingsDebounce {
    pub const GROUP_KEY: &'static str = "notification_settings";
    pub const ID: &'static str = "notification_settings";
}
