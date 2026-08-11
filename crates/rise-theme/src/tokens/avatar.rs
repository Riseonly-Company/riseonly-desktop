use gpui::{Hsla, Pixels, rgb};

use crate::tokens::density::Density;

/// The sixteen colours a letter avatar is drawn on, and the hash that picks one.
///
/// Appearance-independent and not adjustable: `file-service`'s
/// `GenerateLetterAvatar` replicates this list and this hash, so a value changed
/// here without changing it there gives one user two colours.
pub struct AvatarPalette;

impl AvatarPalette {
    pub const COLORS: [u32; 16] = [
        0xFF4141, 0xFF8C00, 0xFFC800, 0xC8DC00, 0x64C864, 0x00C896, 0x00B4DC, 0x3296FF, 0x6464FF,
        0x9664FF, 0xC864FF, 0xFF64C8, 0xFF6496, 0xC89664, 0x969696, 0x64C8C8,
    ];

    /// `h = h * 31 + c` over UNICODE SCALARS, not UTF-16 code units — iterating
    /// UTF-16 agrees on ASCII and disagrees on every name with an emoji.
    pub fn simple_hash(value: &str) -> u32 {
        let mut hash: i32 = 0;
        for character in value.chars() {
            hash = (hash << 5)
                .wrapping_sub(hash)
                .wrapping_add(character as u32 as i32);
        }

        // Swift's `abs` traps on Int32.min; this wraps instead — the one input where they differ.
        hash.unsigned_abs()
    }

    const LETTER: u32 = 0xFFFFFF;

    pub fn color_for_key(key: &str) -> Hsla {
        let index = Self::simple_hash(key) as usize % Self::COLORS.len();
        rgb(Self::COLORS[index]).into()
    }

    pub fn letter_color() -> Hsla {
        rgb(Self::LETTER).into()
    }

    /// The colour for a user, keyed on the id first so it survives a rename, and
    /// on the name only when the id is missing.
    pub fn color_for_user(user_id: Option<&str>, name: Option<&str>) -> Hsla {
        let key = [user_id, name]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|value| !value.is_empty())
            .unwrap_or("none");

        Self::color_for_key(key)
    }

    /// The single character a letter avatar draws.
    pub fn display_letter(name: Option<&str>) -> String {
        name.map(str::trim)
            .and_then(|value| value.chars().next())
            .map(|first| first.to_uppercase().to_string())
            .unwrap_or_else(|| "?".to_owned())
    }

    /// Whether a `logo` field is something an image element can actually load.
    ///
    /// The wire uses an empty string for "no avatar"; handing that to an image
    /// element fails asynchronously, a frame after the placeholder was needed.
    pub fn resolved_logo_url(source: Option<&str>) -> Option<String> {
        let value = source.map(str::trim).filter(|value| !value.is_empty())?;
        let lower = value.to_ascii_lowercase();

        if lower.starts_with("http") || lower.starts_with("file://") || value.starts_with('/') {
            Some(value.to_owned())
        } else {
            None
        }
    }
}

/// Avatar sizes, one per place an avatar is drawn.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AvatarMetrics {
    /// A post header.
    pub post_header: Pixels,
    /// A comment row.
    pub comment: Pixels,
    /// One of the three faces in a post's "liked by" line, and how far each is
    /// offset from the one under it.
    pub likes_summary: Pixels,
    pub likes_summary_step: Pixels,
    /// The story tray's face, the ring drawn around it, and the cell that holds
    /// both.
    pub story: Pixels,
    pub story_ring: Pixels,
    pub story_cell: Pixels,
    /// The ring's stroke, and how far outside the face it sits.
    pub ring_width: Pixels,
    /// The toolbar avatar in a page header, sized to the header's own button.
    pub toolbar: Pixels,
    /// The hairline around a face, so a light avatar on a light card still has
    /// an edge.
    pub border_width: Pixels,
}

impl AvatarMetrics {
    /// How large the letter inside a placeholder is, relative to the face.
    pub const LETTER_RATIO: f32 = 0.46;
    pub const LETTER_MIN: f32 = 11.0;

    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            post_header: l(40.0),
            comment: l(36.0),
            likes_summary: l(23.0),
            likes_summary_step: l(15.0),
            story: l(62.0),
            story_ring: l(68.0),
            story_cell: l(74.0),
            ring_width: l(2.5),
            toolbar: l(45.0),
            border_width: l(1.0),
        }
    }

    /// The letter's point size for a face of this diameter.
    pub fn letter_size(diameter: Pixels) -> f32 {
        (f32::from(diameter) * Self::LETTER_RATIO).max(Self::LETTER_MIN)
    }
}

impl Default for AvatarMetrics {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn the_hash_is_the_references_own() {
        // h * 31 + c by hand, so a rewrite of the loop cannot change which colour a user gets.
        let expected = |value: &str| {
            let mut hash: i32 = 0;
            for character in value.chars() {
                hash = hash.wrapping_mul(31).wrapping_add(character as u32 as i32);
            }
            hash.unsigned_abs()
        };

        for key in ["1", "42", "aianov", "Дулат", "none", ""] {
            assert_eq!(AvatarPalette::simple_hash(key), expected(key), "{key}");
        }
    }

    #[test]
    fn the_same_user_always_gets_the_same_colour() {
        let first = AvatarPalette::color_for_user(Some("u1"), Some("Alice"));
        let renamed = AvatarPalette::color_for_user(Some("u1"), Some("Bob"));

        assert_eq!(
            first, renamed,
            "the id is the key, so a rename must not recolour the avatar"
        );
    }

    #[test]
    fn an_anonymous_face_still_gets_a_colour_rather_than_a_panic() {
        assert_eq!(
            AvatarPalette::color_for_user(None, None),
            AvatarPalette::color_for_key("none")
        );
        assert_eq!(
            AvatarPalette::color_for_user(Some("   "), None),
            AvatarPalette::color_for_key("none"),
            "a blank id is not an id"
        );
    }

    #[test]
    fn every_hash_lands_inside_the_palette() {
        for index in 0..2000u32 {
            let key = index.to_string();
            let slot = AvatarPalette::simple_hash(&key) as usize % AvatarPalette::COLORS.len();
            assert!(slot < AvatarPalette::COLORS.len());
        }
    }

    #[test]
    fn the_palette_is_the_sixteen_the_server_replicates() {
        assert_eq!(AvatarPalette::COLORS.len(), 16);
        assert_eq!(AvatarPalette::COLORS[0], 0xFF4141);
        assert_eq!(AvatarPalette::COLORS[15], 0x64C8C8);
    }

    #[test]
    fn a_letter_is_one_uppercase_character_or_a_question_mark() {
        assert_eq!(AvatarPalette::display_letter(Some("aianov")), "A");
        assert_eq!(AvatarPalette::display_letter(Some("  дулат")), "Д");
        assert_eq!(AvatarPalette::display_letter(Some("   ")), "?");
        assert_eq!(AvatarPalette::display_letter(None), "?");
    }

    #[test]
    fn an_empty_logo_is_not_a_url_to_try_loading() {
        assert_eq!(AvatarPalette::resolved_logo_url(Some("")), None);
        assert_eq!(AvatarPalette::resolved_logo_url(Some("   ")), None);
        assert_eq!(AvatarPalette::resolved_logo_url(None), None);
        assert_eq!(
            AvatarPalette::resolved_logo_url(Some("avatars/1.png")),
            None,
            "a bare relative path has no host to fetch it from"
        );
        assert_eq!(
            AvatarPalette::resolved_logo_url(Some(" https://cdn/a.png ")),
            Some("https://cdn/a.png".to_owned())
        );
        assert_eq!(
            AvatarPalette::resolved_logo_url(Some("/tmp/a.png")),
            Some("/tmp/a.png".to_owned())
        );
    }

    #[test]
    fn the_letter_never_shrinks_below_legibility() {
        assert_eq!(AvatarMetrics::letter_size(px(40.0)), 40.0 * 0.46);
        assert_eq!(
            AvatarMetrics::letter_size(px(20.0)),
            AvatarMetrics::LETTER_MIN,
            "a 20pt face would otherwise draw a 9pt letter"
        );
    }

    #[test]
    fn density_scales_every_face() {
        assert_eq!(AvatarMetrics::new(Density::new(1.25)).post_header, px(50.0));
    }
}
