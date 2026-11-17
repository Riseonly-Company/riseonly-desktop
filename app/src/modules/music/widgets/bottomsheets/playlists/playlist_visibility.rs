use gpui::Hsla;
use rise_i18n::tr;
use rise_music::PlaylistVisibility as CanonPlaylistVisibility;

use crate::core::ui::settings_icon_palette::SettingsIconPalette;

/// playlist visibility kept in its own file since the playlist sheets and the guides sandbox read it
/// order is local then private then public like the sheet lists them
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaylistVisibility {
    Local,
    Private,
    Public,
}

impl PlaylistVisibility {
    pub const ALL: [PlaylistVisibility; 3] =
        [PlaylistVisibility::Local, PlaylistVisibility::Private, PlaylistVisibility::Public];

    fn canon(self) -> CanonPlaylistVisibility {
        match self {
            PlaylistVisibility::Local => CanonPlaylistVisibility::LocalPlaylist,
            PlaylistVisibility::Private => CanonPlaylistVisibility::PrivatePlaylist,
            PlaylistVisibility::Public => CanonPlaylistVisibility::PublicPlaylist,
        }
    }

    pub fn raw_value(self) -> &'static str {
        self.canon().wire()
    }

    pub fn from_raw(raw_value: &str) -> Option<PlaylistVisibility> {
        PlaylistVisibility::ALL.into_iter().find(|visibility| visibility.raw_value() == raw_value)
    }

    pub fn title(self) -> String {
        tr(self.canon().i18n_key())
    }

    pub fn description(self) -> String {
        tr(self.canon().description_i18n_key())
    }

    /// sf symbol name, canon doesnt know them so the icon stays local
    pub fn icon(self) -> &'static str {
        match self {
            PlaylistVisibility::Local => "iphone",
            PlaylistVisibility::Private => "lock.fill",
            PlaylistVisibility::Public => "globe",
        }
    }

    /// icon background
    /// three types differ in hue and hues come from the shared app table
    /// the glyph is always white
    pub fn icon_background(self) -> Hsla {
        match self {
            PlaylistVisibility::Local => SettingsIconPalette::memory(),
            PlaylistVisibility::Private => SettingsIconPalette::goals_plans(),
            PlaylistVisibility::Public => SettingsIconPalette::chat_folders(),
        }
    }
}
