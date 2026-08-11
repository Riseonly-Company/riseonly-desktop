use gpui::{Hsla, Pixels};
use rise_theme::{AppTheme, GroupedMetrics, SettingsIconPalette, alpha};

/// Whether a group draws a card around its rows, or nothing at all. Bare drops
/// the card and the hairlines, and is the only surface where a row's own fill
/// shows, so a selection there reads as a pill.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GroupSurface {
    #[default]
    Card,
    Bare,
}

impl GroupSurface {
    pub fn is_card(self) -> bool {
        self == Self::Card
    }
}

/// Where a row sits in its group. A first or last row has to round its own
/// corners: a gpui `ContentMask` carries no radius, so `overflow_hidden()` on
/// the card does not round what is drawn inside it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RowPosition {
    /// The only row: both ends are the card's.
    Only,
    First,
    #[default]
    Middle,
    Last,
}

impl RowPosition {
    /// With `has_end_action`, the end action closes the card and the last row
    /// gets a hairline under it rather than a corner.
    pub fn of(index: usize, count: usize, has_end_action: bool) -> Self {
        let first = index == 0;
        let last = !has_end_action && index + 1 == count;

        match (first, last) {
            (true, true) => Self::Only,
            (true, false) => Self::First,
            (false, true) => Self::Last,
            (false, false) => Self::Middle,
        }
    }

    pub fn opens_the_card(self) -> bool {
        matches!(self, Self::First | Self::Only)
    }

    pub fn closes_the_card(self) -> bool {
        matches!(self, Self::Last | Self::Only)
    }
}

/// The corners a row rounds; left and right always agree.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct RowCorners {
    pub top: Pixels,
    pub bottom: Pixels,
}

/// Everything about a row that changes how it is painted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RowState {
    pub surface: GroupSurface,
    pub position: RowPosition,
    /// The current destination, in a group being used for navigation.
    /// Independent of [`GroupSurface`]: a card group can mark a selection too.
    pub selected: bool,
    pub destructive: bool,
    pub disabled: bool,
}

/// The four colours a call site may override; resolved only at render.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct GroupColors {
    pub surface: Option<Hsla>,
    pub primary_text: Option<Hsla>,
    pub secondary_text: Option<Hsla>,
    pub separator: Option<Hsla>,
}

/// A row's paint, resolved by [`row_chrome`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RowChrome {
    /// `None` is transparent — the ordinary case in both surfaces, since only a
    /// selection fills and the card paints its surface once, itself.
    pub background: Option<Hsla>,
    /// `None` when the pointer must change nothing — a disabled row, or one that
    /// is already filled because it is selected.
    pub hover_background: Option<Hsla>,
    pub label: Hsla,
    pub secondary: Hsla,
    pub chevron: Hsla,
    pub corners: RowCorners,
    pub opacity: f32,
}

/// The chip behind an icon, and the glyph on it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct IconChrome {
    pub background: Hsla,
    pub glyph: Hsla,
}

/// The corners a row rounds, given where it sits. A bare row is always a pill; a
/// card row borrows the card's corner only at the end that touches it.
pub fn row_corners(theme: &AppTheme, state: RowState) -> RowCorners {
    let metrics = theme.grouped;

    match state.surface {
        GroupSurface::Bare => RowCorners {
            top: metrics.selection_radius,
            bottom: metrics.selection_radius,
        },
        GroupSurface::Card => RowCorners {
            top: if state.position.opens_the_card() {
                metrics.card_radius
            } else {
                Pixels::ZERO
            },
            bottom: if state.position.closes_the_card() {
                metrics.card_radius
            } else {
                Pixels::ZERO
            },
        },
    }
}

pub fn row_chrome(theme: &AppTheme, colors: GroupColors, state: RowState) -> RowChrome {
    let secondary = colors.secondary_text.unwrap_or(theme.text.secondary);

    let label = if state.destructive {
        theme.semantic.error_100
    } else {
        colors.primary_text.unwrap_or(theme.text.primary)
    };

    let corners = row_corners(theme, state);

    let opacity = if state.disabled {
        GroupedMetrics::DISABLED_OPACITY
    } else {
        1.0
    };

    if state.selected {
        let on_selection = theme.bg._000;

        return RowChrome {
            background: Some(theme.primary._100),
            hover_background: None,
            label: on_selection,
            secondary: alpha(on_selection, GroupedMetrics::ON_SELECTION_SECONDARY_ALPHA),
            chevron: alpha(on_selection, GroupedMetrics::ON_SELECTION_SECONDARY_ALPHA),
            corners,
            opacity,
        };
    }

    RowChrome {
        background: None,
        hover_background: (!state.disabled).then_some(theme.bg._300),
        label,
        secondary,
        chevron: alpha(secondary, GroupedMetrics::CHEVRON_ALPHA),
        corners,
        opacity,
    }
}

/// What the card itself is painted with, when there is a card.
pub fn card_background(theme: &AppTheme, colors: GroupColors) -> Hsla {
    colors.surface.unwrap_or(theme.bg._200)
}

/// The icon chip, with whatever the item asked for taking precedence. A selected
/// row inverts it: the item's own colour was picked to sit on a neutral card.
pub fn icon_chrome(
    theme: &AppTheme,
    state: RowState,
    background: Option<Hsla>,
    glyph: Option<Hsla>,
) -> IconChrome {
    if state.selected {
        return IconChrome {
            background: theme.bg._000,
            glyph: theme.primary._100,
        };
    }

    let default_background = if state.destructive {
        alpha(
            theme.semantic.error_100,
            GroupedMetrics::DESTRUCTIVE_ICON_BACKGROUND_ALPHA,
        )
    } else {
        alpha(theme.primary._100, GroupedMetrics::ICON_BACKGROUND_ALPHA)
    };

    IconChrome {
        background: background.unwrap_or(default_background),
        glyph: glyph.unwrap_or_else(SettingsIconPalette::glyph),
    }
}

pub fn separator_color(theme: &AppTheme, colors: GroupColors) -> Hsla {
    colors
        .separator
        .unwrap_or_else(|| alpha(theme.border._200, GroupedMetrics::SEPARATOR_ALPHA))
}

/// The hairline above a destructive end action, dimmer than the ones between
/// rows.
pub fn end_separator_color(theme: &AppTheme, colors: GroupColors) -> Hsla {
    let separator = separator_color(theme, colors);

    alpha(separator, separator.a * GroupedMetrics::END_SEPARATOR_ALPHA)
}

/// Whether a hairline is drawn under the row at `index`. Never after the last
/// row, and never at all without the card.
pub fn draws_separator_after(surface: GroupSurface, index: usize, count: usize) -> bool {
    surface.is_card() && index + 1 < count
}

/// How far the group sits from the edge it is laid out in.
pub fn group_padding_x(metrics: &GroupedMetrics, surface: GroupSurface) -> Pixels {
    match surface {
        GroupSurface::Card => metrics.card_padding_x,
        GroupSurface::Bare => metrics.bare_padding_x,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(surface: GroupSurface) -> RowState {
        RowState {
            surface,
            ..RowState::default()
        }
    }

    fn at(surface: GroupSurface, position: RowPosition) -> RowState {
        RowState {
            surface,
            position,
            ..RowState::default()
        }
    }

    #[test]
    fn no_row_paints_its_own_background_until_it_is_selected() {
        let theme = AppTheme::dark();

        for surface in [GroupSurface::Card, GroupSurface::Bare] {
            assert_eq!(
                row_chrome(&theme, GroupColors::default(), state(surface)).background,
                None,
                "the reference paints the surface per row and clips the stack; gpui's \
                 content mask has no radius, so doing that here squares the card's corners"
            );
        }

        assert_eq!(
            card_background(&theme, GroupColors::default()),
            theme.bg._200,
            "the card is what carries the surface instead"
        );
    }

    #[test]
    fn a_card_row_borrows_the_cards_corner_only_at_the_end_that_touches_it() {
        let theme = AppTheme::dark();
        let radius = theme.grouped.card_radius;

        let first = row_corners(&theme, at(GroupSurface::Card, RowPosition::First));
        assert_eq!(first.top, radius);
        assert_eq!(first.bottom, Pixels::ZERO);

        let middle = row_corners(&theme, at(GroupSurface::Card, RowPosition::Middle));
        assert_eq!(middle.top, Pixels::ZERO);
        assert_eq!(middle.bottom, Pixels::ZERO);

        let last = row_corners(&theme, at(GroupSurface::Card, RowPosition::Last));
        assert_eq!(last.top, Pixels::ZERO);
        assert_eq!(last.bottom, radius);

        let only = row_corners(&theme, at(GroupSurface::Card, RowPosition::Only));
        assert_eq!(only.top, radius);
        assert_eq!(only.bottom, radius);
    }

    #[test]
    fn a_wrapperless_row_is_a_pill_wherever_it_sits() {
        let theme = AppTheme::dark();
        let radius = theme.grouped.selection_radius;

        for position in [
            RowPosition::Only,
            RowPosition::First,
            RowPosition::Middle,
            RowPosition::Last,
        ] {
            let corners = row_corners(&theme, at(GroupSurface::Bare, position));
            assert_eq!(corners.top, radius);
            assert_eq!(corners.bottom, radius);
        }
    }

    #[test]
    fn an_end_action_takes_over_being_the_bottom_of_the_card() {
        assert_eq!(RowPosition::of(2, 3, false), RowPosition::Last);
        assert_eq!(
            RowPosition::of(2, 3, true),
            RowPosition::Middle,
            "with an end action below it, the last ROW no longer closes the card"
        );
        assert_eq!(RowPosition::of(0, 1, false), RowPosition::Only);
        assert_eq!(RowPosition::of(0, 1, true), RowPosition::First);
        assert_eq!(RowPosition::of(0, 3, false), RowPosition::First);
        assert_eq!(RowPosition::of(1, 3, false), RowPosition::Middle);
    }

    #[test]
    fn a_selection_reads_as_the_accent_in_both_surfaces() {
        let theme = AppTheme::dark();

        for surface in [GroupSurface::Card, GroupSurface::Bare] {
            let selected = RowState {
                surface,
                selected: true,
                ..RowState::default()
            };
            let chrome = row_chrome(&theme, GroupColors::default(), selected);

            assert_eq!(chrome.background, Some(theme.primary._100));
            assert_eq!(chrome.label, theme.bg._000);
            assert_ne!(
                chrome.label, theme.text.primary,
                "the label has to invert, or it draws itself on its own accent"
            );
            assert_eq!(
                chrome.hover_background, None,
                "a filled row has nothing left for a hover to say"
            );
        }
    }

    #[test]
    fn a_destructive_row_is_red_until_it_is_selected() {
        let theme = AppTheme::dark();

        let destructive = RowState {
            destructive: true,
            ..RowState::default()
        };
        assert_eq!(
            row_chrome(&theme, GroupColors::default(), destructive).label,
            theme.semantic.error_100
        );

        let both = RowState {
            selected: true,
            ..destructive
        };
        assert_eq!(
            row_chrome(&theme, GroupColors::default(), both).label,
            theme.bg._000,
            "red on the accent fill is unreadable; the selection wins"
        );
    }

    #[test]
    fn a_disabled_row_dims_and_stops_answering_the_pointer() {
        let theme = AppTheme::dark();

        let disabled = RowState {
            disabled: true,
            ..RowState::default()
        };
        let chrome = row_chrome(&theme, GroupColors::default(), disabled);

        assert_eq!(chrome.opacity, GroupedMetrics::DISABLED_OPACITY);
        assert_eq!(chrome.hover_background, None);
    }

    #[test]
    fn an_override_replaces_the_theme_rather_than_blending_with_it() {
        let theme = AppTheme::dark();
        let colors = GroupColors {
            surface: Some(theme.bg._500),
            primary_text: Some(theme.primary._200),
            secondary_text: Some(theme.primary._300),
            separator: Some(theme.border._600),
        };

        let chrome = row_chrome(&theme, colors, state(GroupSurface::Card));

        assert_eq!(card_background(&theme, colors), theme.bg._500);
        assert_eq!(chrome.label, theme.primary._200);
        assert_eq!(chrome.secondary, theme.primary._300);
        assert_eq!(separator_color(&theme, colors), theme.border._600);
    }

    #[test]
    fn the_chevron_recedes_from_the_secondary_text_it_sits_beside() {
        let theme = AppTheme::dark();
        let chrome = row_chrome(&theme, GroupColors::default(), state(GroupSurface::Card));

        assert!(chrome.chevron.a < chrome.secondary.a);
    }

    #[test]
    fn an_items_own_chip_colour_wins_until_the_row_is_selected() {
        let theme = AppTheme::dark();
        let asked = SettingsIconPalette::sessions();

        let plain = icon_chrome(&theme, state(GroupSurface::Card), Some(asked), None);
        assert_eq!(plain.background, asked);
        assert_eq!(plain.glyph, SettingsIconPalette::glyph());

        let selected = icon_chrome(
            &theme,
            RowState {
                selected: true,
                ..state(GroupSurface::Card)
            },
            Some(asked),
            None,
        );
        assert_eq!(selected.background, theme.bg._000);
        assert_eq!(selected.glyph, theme.primary._100);
    }

    #[test]
    fn a_row_with_no_chip_colour_falls_back_to_the_accent_or_to_red() {
        let theme = AppTheme::dark();

        let plain = icon_chrome(&theme, state(GroupSurface::Card), None, None);
        assert_eq!(plain.background.h, theme.primary._100.h);
        assert_eq!(plain.background.a, GroupedMetrics::ICON_BACKGROUND_ALPHA);

        let destructive = icon_chrome(
            &theme,
            RowState {
                destructive: true,
                ..state(GroupSurface::Card)
            },
            None,
            None,
        );
        assert_eq!(destructive.background.h, theme.semantic.error_100.h);
        assert!(
            destructive.background.a < plain.background.a,
            "a red row should not carry two full-strength reds"
        );
    }

    #[test]
    fn separators_close_between_rows_and_never_after_the_last_one() {
        assert!(draws_separator_after(GroupSurface::Card, 0, 3));
        assert!(draws_separator_after(GroupSurface::Card, 1, 3));
        assert!(!draws_separator_after(GroupSurface::Card, 2, 3));
        assert!(!draws_separator_after(GroupSurface::Card, 0, 1));
    }

    #[test]
    fn a_wrapperless_group_draws_no_hairlines_at_all() {
        for index in 0..4 {
            assert!(!draws_separator_after(GroupSurface::Bare, index, 4));
        }
    }

    #[test]
    fn the_end_actions_hairline_is_dimmer_than_the_ones_above_it() {
        let theme = AppTheme::dark();

        let between = separator_color(&theme, GroupColors::default());
        let above_end = end_separator_color(&theme, GroupColors::default());

        assert!(above_end.a < between.a);
        assert_eq!(above_end.h, between.h);
    }

    #[test]
    fn a_wrapperless_group_sits_closer_to_its_edge() {
        let metrics = GroupedMetrics::default();

        assert_eq!(
            group_padding_x(&metrics, GroupSurface::Card),
            metrics.card_padding_x
        );
        assert_eq!(
            group_padding_x(&metrics, GroupSurface::Bare),
            metrics.bare_padding_x
        );
    }
}
