use gpui::{Hsla, rgb};

/// The colour behind each settings icon.
///
/// Appearance-independent, and deliberately outside [`crate::ThemePalette`],
/// which is serde-compatible with `theme-service`'s `palette_json`.
pub struct SettingsIconPalette;

impl SettingsIconPalette {
    const GLYPH: u32 = 0xFFFFFF;

    pub fn glyph() -> Hsla {
        rgb(Self::GLYPH).into()
    }

    pub fn profile() -> Hsla {
        rgb(0x008DE5).into()
    }

    pub fn privacy() -> Hsla {
        rgb(0x0074DF).into()
    }

    pub fn sessions() -> Hsla {
        rgb(0x00C781).into()
    }

    pub fn blocks() -> Hsla {
        rgb(0xFF4D4F).into()
    }

    pub fn goals_plans() -> Hsla {
        rgb(0xF19A1A).into()
    }

    pub fn notifications() -> Hsla {
        rgb(0xEC3900).into()
    }

    pub fn quick_reaction() -> Hsla {
        rgb(0xFF9500).into()
    }

    pub fn subscription() -> Hsla {
        rgb(0xE9C300).into()
    }

    pub fn customization() -> Hsla {
        rgb(0x00A9BB).into()
    }

    pub fn memory() -> Hsla {
        rgb(0x00D776).into()
    }

    pub fn moderation() -> Hsla {
        rgb(0x009A9A).into()
    }

    pub fn language() -> Hsla {
        rgb(0xE8AA00).into()
    }

    pub fn reset() -> Hsla {
        rgb(0xFF9500).into()
    }

    pub fn stickers() -> Hsla {
        rgb(0x7E57F7).into()
    }

    pub fn chat_folders() -> Hsla {
        rgb(0x3478F6).into()
    }

    pub fn logout() -> Hsla {
        rgb(0xFF3B30).into()
    }

    pub fn privacy_phone() -> Hsla {
        rgb(0x5AC8FA).into()
    }

    pub fn privacy_avatar() -> Hsla {
        rgb(0xF19A1A).into()
    }

    pub fn privacy_description() -> Hsla {
        rgb(0x7E57F7).into()
    }

    pub fn privacy_birthday() -> Hsla {
        rgb(0xFF6482).into()
    }

    pub fn privacy_friends() -> Hsla {
        rgb(0x0083E0).into()
    }

    pub fn privacy_plans() -> Hsla {
        rgb(0x00A9BB).into()
    }

    pub fn privacy_goals() -> Hsla {
        rgb(0x00C781).into()
    }

    pub fn privacy_messages() -> Hsla {
        rgb(0x34C759).into()
    }

    pub fn privacy_last_seen() -> Hsla {
        rgb(0xFF9500).into()
    }

    pub fn privacy_online_status() -> Hsla {
        rgb(0x30D158).into()
    }

    pub fn privacy_in_chat() -> Hsla {
        rgb(0xFF3B30).into()
    }

    pub fn moderation_become() -> Hsla {
        rgb(0x08A500).into()
    }

    pub fn moderation_appeals() -> Hsla {
        rgb(0xE0820A).into()
    }

    pub fn memory_videos() -> Hsla {
        rgb(0xFF3B30).into()
    }

    pub fn memory_images() -> Hsla {
        rgb(0xFF9500).into()
    }

    pub fn memory_photos() -> Hsla {
        rgb(0xFFCC00).into()
    }

    pub fn memory_files() -> Hsla {
        rgb(0x34C759).into()
    }

    pub fn memory_stories() -> Hsla {
        rgb(0x5AC8FA).into()
    }

    pub fn memory_audio() -> Hsla {
        rgb(0x007AFF).into()
    }

    pub fn memory_other() -> Hsla {
        rgb(0x8E8E93).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colours_are_the_references_own() {
        assert_eq!(SettingsIconPalette::profile(), rgb(0x008DE5).into());
        assert_eq!(SettingsIconPalette::logout(), rgb(0xFF3B30).into());
        assert_eq!(SettingsIconPalette::memory_other(), rgb(0x8E8E93).into());
    }

    #[test]
    fn a_chip_and_its_glyph_are_not_the_same_colour() {
        let glyph = SettingsIconPalette::glyph();

        for chip in [
            SettingsIconPalette::profile(),
            SettingsIconPalette::subscription(),
            SettingsIconPalette::memory_photos(),
            SettingsIconPalette::memory_other(),
        ] {
            assert_ne!(chip, glyph);
        }
    }
}
