use gpui::{IntoElement, Pixels, Svg};
use rise_theme::{AppTheme, BadgeColors};

/// The marks that sit beside a name: verified, and premium. Both take an
/// explicit size, not an [`crate::IconSize`] step — they are sized against the
/// type they follow.
pub struct BadgeUi;

impl BadgeUi {
    /// A filled seal in the product's own red.
    pub fn official(size: Pixels) -> Option<Svg> {
        crate::IconUi::sized("checkmark.seal.fill", size, BadgeColors::official(), true)
    }

    /// The premium mark, drawn rather than animated: gpui has no damage regions,
    /// so a looping badge on every visible row repaints the whole window.
    pub fn premium(size: Pixels) -> Option<Svg> {
        crate::IconUi::sized("sparkles", size, BadgeColors::premium(), true)
    }

    /// Both marks, in the reference's order, for an author line.
    pub fn for_author(
        _theme: &AppTheme,
        size: Pixels,
        is_official: bool,
        is_premium: bool,
    ) -> Vec<impl IntoElement + use<>> {
        let mut badges = Vec::new();
        if is_official && let Some(badge) = Self::official(size) {
            badges.push(badge);
        }
        if is_premium && let Some(badge) = Self::premium(size) {
            badges.push(badge);
        }
        badges
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn both_marks_have_a_glyph_the_bundle_carries() {
        assert!(BadgeUi::official(px(18.0)).is_some());
        assert!(BadgeUi::premium(px(18.0)).is_some());
    }

    #[test]
    fn an_author_with_neither_mark_draws_nothing_beside_their_name() {
        let theme = AppTheme::dark();
        assert!(BadgeUi::for_author(&theme, px(18.0), false, false).is_empty());
    }

    #[test]
    fn the_marks_appear_in_the_reference_order() {
        let theme = AppTheme::dark();
        assert_eq!(BadgeUi::for_author(&theme, px(18.0), true, false).len(), 1);
        assert_eq!(BadgeUi::for_author(&theme, px(18.0), false, true).len(), 1);
        assert_eq!(
            BadgeUi::for_author(&theme, px(18.0), true, true).len(),
            2,
            "official first, then premium, the way the author line reads"
        );
    }
}
