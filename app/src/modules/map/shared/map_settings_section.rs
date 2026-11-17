use crate::modules::map::stores::map_actions::map_privacy_types::MapPrivacyKind;

pub use crate::app::router::map_settings_section_hooks::MapSettingsSection;

/// the privacy list a section edits
pub trait MapSettingsSectionPrivacy {
    fn privacy_kind(self) -> Option<MapPrivacyKind>;
}

impl MapSettingsSectionPrivacy for MapSettingsSection {
    fn privacy_kind(self) -> Option<MapPrivacyKind> {
        match self {
            MapSettingsSection::PresencePrivacy => Some(MapPrivacyKind::Presence),
            MapSettingsSection::TracksPrivacy => Some(MapPrivacyKind::Tracks),
            MapSettingsSection::MediaPrivacy => Some(MapPrivacyKind::Media),
            MapSettingsSection::FriendsPrivacy => Some(MapPrivacyKind::Friends),
            _ => None,
        }
    }
}
