use gpui::{Hsla, rgb};

/// The two marks that sit beside a name. Neither is themed, which is why they
/// live here rather than in [`crate::ThemePalette`].
pub struct BadgeColors;

impl BadgeColors {
    const OFFICIAL: u32 = 0xFF5A5A;
    const PREMIUM: u32 = 0x4FC3F7;

    pub fn official() -> Hsla {
        rgb(Self::OFFICIAL).into()
    }

    pub fn premium() -> Hsla {
        rgb(Self::PREMIUM).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_official_mark_is_the_references_own_red() {
        let expected: Hsla = rgb(0xFF5A5A).into();
        assert_eq!(BadgeColors::official(), expected);
    }

    #[test]
    fn the_two_marks_are_distinguishable_from_each_other() {
        assert_ne!(BadgeColors::official(), BadgeColors::premium());
    }
}
