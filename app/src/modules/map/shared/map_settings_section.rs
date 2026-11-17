use rise_canon::RiseGuideAnchor;

use crate::modules::map::stores::map_actions::map_privacy_types::MapPrivacyKind;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MapSettingsSection {
    Location,
    Tracks,
    Merge,
    Home,
    Privacy,
    Notifications,
    PresencePrivacy,
    TracksPrivacy,
    MediaPrivacy,
    FriendsPrivacy,
}

impl MapSettingsSection {
    pub const ALL: [MapSettingsSection; 10] = [
        MapSettingsSection::Location,
        MapSettingsSection::Tracks,
        MapSettingsSection::Merge,
        MapSettingsSection::Home,
        MapSettingsSection::Privacy,
        MapSettingsSection::Notifications,
        MapSettingsSection::PresencePrivacy,
        MapSettingsSection::TracksPrivacy,
        MapSettingsSection::MediaPrivacy,
        MapSettingsSection::FriendsPrivacy,
    ];

    pub const RECORDING_SECTIONS: [MapSettingsSection; 4] =
        [MapSettingsSection::Location, MapSettingsSection::Tracks, MapSettingsSection::Merge, MapSettingsSection::Home];

    pub const SHARING_SECTIONS: [MapSettingsSection; 2] = [MapSettingsSection::Privacy, MapSettingsSection::Notifications];

    pub const PRIVACY_SECTIONS: [MapSettingsSection; 4] = [
        MapSettingsSection::PresencePrivacy,
        MapSettingsSection::TracksPrivacy,
        MapSettingsSection::MediaPrivacy,
        MapSettingsSection::FriendsPrivacy,
    ];

    pub const fn raw_value(self) -> &'static str {
        match self {
            MapSettingsSection::Location => "location",
            MapSettingsSection::Tracks => "tracks",
            MapSettingsSection::Merge => "merge",
            MapSettingsSection::Home => "home",
            MapSettingsSection::Privacy => "privacy",
            MapSettingsSection::Notifications => "notifications",
            MapSettingsSection::PresencePrivacy => "presencePrivacy",
            MapSettingsSection::TracksPrivacy => "tracksPrivacy",
            MapSettingsSection::MediaPrivacy => "mediaPrivacy",
            MapSettingsSection::FriendsPrivacy => "friendsPrivacy",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.raw_value() == raw)
    }

    /// the rows a guide points at on the settings page
    pub const fn guide_anchor(self) -> Option<RiseGuideAnchor> {
        match self {
            MapSettingsSection::Home => Some(RiseGuideAnchor::MapSettingsHome),
            MapSettingsSection::Tracks => Some(RiseGuideAnchor::MapSettingsTracks),
            _ => None,
        }
    }

    pub const fn title_key(self) -> &'static str {
        match self {
            MapSettingsSection::Location => "map_settings_position_section",
            MapSettingsSection::Tracks => "map_tracks_title",
            MapSettingsSection::Merge => "map_merge_title",
            MapSettingsSection::Home => "map_home_title",
            MapSettingsSection::Privacy => "settings_privacy_title",
            MapSettingsSection::Notifications => "map_settings_notifications_section",
            MapSettingsSection::PresencePrivacy => "map_privacy_presence_title",
            MapSettingsSection::TracksPrivacy => "map_privacy_tracks_title",
            MapSettingsSection::MediaPrivacy => "map_media_grids",
            MapSettingsSection::FriendsPrivacy => "map_friends_title",
        }
    }

    pub const fn privacy_kind(self) -> Option<MapPrivacyKind> {
        match self {
            MapSettingsSection::PresencePrivacy => Some(MapPrivacyKind::Presence),
            MapSettingsSection::TracksPrivacy => Some(MapPrivacyKind::Tracks),
            MapSettingsSection::MediaPrivacy => Some(MapPrivacyKind::Media),
            MapSettingsSection::FriendsPrivacy => Some(MapPrivacyKind::Friends),
            _ => None,
        }
    }
}
