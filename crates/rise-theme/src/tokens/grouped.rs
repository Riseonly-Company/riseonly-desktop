use gpui::Pixels;

use crate::tokens::density::Density;

/// Lengths and type sizes for a grouped settings list.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GroupedMetrics {
    /// The card's corner, and the inset from the screen's edge to the card.
    pub card_radius: Pixels,
    pub card_padding_x: Pixels,
    /// The inset a wrapperless group keeps instead, so a selected row's pill
    /// does not touch the panel it sits in.
    pub bare_padding_x: Pixels,
    /// The corner of a selected or hovered row with no card around it: exactly
    /// half [`Self::row_height`], so a one-line row is a true pill.
    pub selection_radius: Pixels,
    /// Between the section heading, the card and the footnote.
    pub header_gap: Pixels,
    /// The heading is inset further than the card.
    pub header_padding_x: Pixels,
    pub row_padding_x: Pixels,
    pub row_padding_y: Pixels,
    /// Between the icon, the text column and whatever trails them.
    pub row_gap: Pixels,
    /// Between a pretitle, a title and a subtitle.
    pub text_gap: Pixels,
    /// The least space a trailing value keeps from the label.
    pub trailing_gap: Pixels,
    pub icon_side: Pixels,
    pub icon_radius: Pixels,
    pub icon_glyph: Pixels,
    pub chevron: Pixels,
    pub separator_height: Pixels,
    /// How far a separator starts from the row's leading edge; must equal
    /// [`Self::label_offset_x`], or the hairline runs under the icon.
    pub separator_leading_padding: Pixels,
    pub end_padding_y: Pixels,
    pub end_gap: Pixels,
    pub end_icon: Pixels,
    /// The footnote under a group.
    pub footer_padding_left: Pixels,
    pub footer_padding_top: Pixels,
    pub title_size: f32,
    pub pretitle_size: f32,
    pub text_size: f32,
    pub subtitle_size: f32,
    pub trailing_size: f32,
    pub end_title_size: f32,
    pub footer_size: f32,
}

impl GroupedMetrics {
    /// How far the hairline between rows recedes from `border._200`.
    pub const SEPARATOR_ALPHA: f32 = 0.55;

    /// The hairline above a destructive end action: multiplies
    /// [`Self::SEPARATOR_ALPHA`] rather than replacing it.
    pub const END_SEPARATOR_ALPHA: f32 = 0.82;

    pub const CHEVRON_ALPHA: f32 = 0.75;

    /// The default chip behind an icon: the accent, very slightly softened.
    pub const ICON_BACKGROUND_ALPHA: f32 = 0.88;
    /// A destructive row's chip, softer still.
    pub const DESTRUCTIVE_ICON_BACKGROUND_ALPHA: f32 = 0.45;

    /// A disabled row stays legible rather than disappearing.
    pub const DISABLED_OPACITY: f32 = 0.42;

    /// How far the secondary text and the chevron recede once a row is filled
    /// with the accent.
    pub const ON_SELECTION_SECONDARY_ALPHA: f32 = 0.75;

    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            card_radius: l(25.0),
            card_padding_x: l(12.0),
            bare_padding_x: l(8.0),
            selection_radius: l(20.0),
            header_gap: l(8.0),
            header_padding_x: l(16.0),
            row_padding_x: l(16.0),
            row_padding_y: l(7.0),
            row_gap: l(12.0),
            text_gap: l(2.0),
            trailing_gap: l(4.0),
            icon_side: l(26.0),
            icon_radius: l(7.0),
            icon_glyph: l(13.0),
            chevron: l(12.0),
            separator_height: l(0.5),
            separator_leading_padding: l(54.0),
            end_padding_y: l(8.0),
            end_gap: l(10.0),
            end_icon: l(17.0),
            footer_padding_left: l(10.0),
            footer_padding_top: l(3.0),
            title_size: 12.0,
            pretitle_size: 11.0,
            text_size: 15.0,
            subtitle_size: 12.0,
            trailing_size: 14.0,
            end_title_size: 15.0,
            footer_size: 14.0,
        }
    }
}

impl GroupedMetrics {
    /// A one-line row, which is every row without a subtitle.
    pub fn row_height(&self) -> Pixels {
        self.icon_side + self.row_padding_y * 2.0
    }

    /// Where the label starts, which is also where a hairline starts.
    pub fn label_offset_x(&self) -> Pixels {
        self.row_padding_x + self.icon_side + self.row_gap
    }
}

impl Default for GroupedMetrics {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn the_horizontal_rhythm_and_the_type_are_still_the_references() {
        let grouped = GroupedMetrics::default();

        assert_eq!(grouped.card_padding_x, px(12.0));
        assert_eq!(grouped.row_padding_x, px(16.0));
        assert_eq!(grouped.row_gap, px(12.0));
        assert_eq!(grouped.header_padding_x, px(16.0));
        assert_eq!(grouped.separator_height, px(0.5));
        assert_eq!(grouped.text_size, 15.0);
        assert_eq!(grouped.subtitle_size, 12.0);
        assert_eq!(grouped.pretitle_size, 11.0);
    }

    #[test]
    fn a_row_is_the_forty_points_telegram_draws_rather_than_the_phones_fifty_three() {
        let grouped = GroupedMetrics::default();

        assert_eq!(grouped.icon_side, px(26.0));
        assert_eq!(grouped.row_padding_y, px(7.0));
        assert_eq!(
            grouped.row_height(),
            px(40.0),
            "the reference's 53 clears a 44pt TOUCH target; a pointer needs no such thing"
        );
    }

    #[test]
    fn a_selected_row_is_a_true_pill_and_not_merely_a_rounded_box() {
        let grouped = GroupedMetrics::default();

        assert_eq!(grouped.selection_radius * 2.0, grouped.row_height());
    }

    #[test]
    fn a_hairline_starts_exactly_where_the_label_does() {
        let grouped = GroupedMetrics::default();

        assert_eq!(grouped.separator_leading_padding, grouped.label_offset_x());
        assert!(
            grouped.separator_leading_padding > grouped.row_padding_x + grouped.icon_side,
            "the hairline would run under the icon chip"
        );
    }

    #[test]
    fn the_card_corner_is_generous_against_the_row_it_wraps() {
        let grouped = GroupedMetrics::default();

        assert_eq!(grouped.card_radius, px(25.0));
        assert!(
            grouped.card_radius > grouped.selection_radius,
            "the card must round more than a row inside it, or its corner reads as an accident"
        );
    }

    #[test]
    fn the_wrapperless_inset_still_clears_the_selection_pill_from_the_edge() {
        let grouped = GroupedMetrics::default();

        assert!(grouped.bare_padding_x > Pixels::ZERO);
        assert!(
            grouped.bare_padding_x < grouped.card_padding_x,
            "a group with no card should sit closer to its panel's edge, not further"
        );
    }

    #[test]
    fn density_scales_every_length_and_leaves_the_type_sizes_to_typography() {
        let dense = GroupedMetrics::new(Density::new(1.25));

        assert_eq!(dense.card_radius, px(25.0 * 1.25));
        assert_eq!(dense.icon_side, px(26.0 * 1.25));
        assert_eq!(dense.row_height(), px(40.0 * 1.25));
        assert_eq!(
            dense.text_size,
            GroupedMetrics::default().text_size,
            "a point size is scaled by Typography::style, not here, or it scales twice"
        );
    }
}
