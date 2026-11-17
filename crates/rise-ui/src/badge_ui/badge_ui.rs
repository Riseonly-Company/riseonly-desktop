use gpui::{AnyElement, IntoElement, Pixels, Svg, rgb};
use rise_theme::AppTheme;

use crate::official_badge::OfficialBadge;

const PREMIUM_PLACEHOLDER_SYMBOL: &str = "sparkles";
const PREMIUM_PLACEHOLDER_COLOR: u32 = 0x4FC3F7;

/// marks beside a name in order, official badge then the premium mark
pub struct BadgeUi;

impl BadgeUi {
    /// the verified seal at size
    pub fn official(size: Pixels) -> OfficialBadge {
        OfficialBadge::new().size(size)
    }

    pub fn premium(size: Pixels) -> Option<Svg> {
        crate::IconUi::sized(PREMIUM_PLACEHOLDER_SYMBOL, size, rgb(PREMIUM_PLACEHOLDER_COLOR).into(), true)
    }

    /// both marks in order, for an author line
    pub fn for_author(_theme: &AppTheme, size: Pixels, is_official: bool, is_premium: bool) -> Vec<AnyElement> {
        let mut badges = Vec::new();
        if is_official {
            badges.push(Self::official(size).into_any_element());
        }
        if is_premium && let Some(badge) = Self::premium(size) {
            badges.push(badge.into_any_element());
        }
        badges
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn the_premium_stand_in_has_a_glyph_the_bundle_carries() {
        assert!(BadgeUi::premium(px(18.0)).is_some());
    }

    #[test]
    fn the_official_mark_is_the_official_badge_twin_at_the_line_size() {
        assert_eq!(BadgeUi::official(px(18.0)), OfficialBadge::new().size(px(18.0)));
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
        assert_eq!(BadgeUi::for_author(&theme, px(18.0), true, true).len(), 2);
    }
}
