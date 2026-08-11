use gpui::{App, Context, Div, prelude::*};
use rise_platform::materials::Material;
use rise_theme::AppTheme;

use crate::glass_panel::GlassPanel;

/// The shape of the whole frame: the app background IS the window, one opaque
/// slab, with flush content columns inside it.
///
/// The navigation rail is the deliberate Telegram-style exception: it floats
/// five points inside the window and mirrors the native corner radius. Content
/// columns do not inherit that inset; lists and media may still reach the window
/// edge. [`BlockUi::window`] and [`BlockUi::column`] build that edge-to-edge
/// content plate, while the app shell owns the separate rail surface.
///
/// **THE CORNER IS THE WINDOW'S, NOT THIS ELEMENT'S.** It is cut out of the
/// window itself by `rise_platform::window_chrome::round_window_corner`, at the
/// radius macOS gives every window — the same corner Telegram gets. Painting our
/// own rounded slab inside the window instead, with a ring of inset around it,
/// was an earlier attempt at this, and it can only ever approximate that radius
/// while the window's shadow goes on tracing the window rather than the slab.
///
/// So the window element itself remains a square that fills its window, and the
/// native window is what clips its corners. Do not add an inner radius here.
///
/// One rule still follows, for a different reason than it used to:
/// **only the outermost element paints a background.** gpui's `ContentMask` is
/// a plain rectangle — `overflow_hidden` does NOT clip to a corner radius — so
/// a fill inside a rounded element never gets rounded by it. Here AppKit saves
/// the corners regardless, but a column repainting the app colour is pure
/// overdraw, so columns stay transparent.
pub struct BlockUi;

impl BlockUi {
    /// The app and the window, which are now the same rectangle.
    ///
    /// `relative` because overlays anchor inside it, and `overflow_hidden` so a
    /// column cannot spill past the window it is drawn in.
    pub fn window(theme: &AppTheme) -> Div {
        gpui::div()
            .size_full()
            .relative()
            .bg(theme.bg._100)
            .text_color(theme.text.primary)
            .overflow_hidden()
            .flex()
            .flex_row()
    }

    /// A column inside the window: full height, square, and TRANSPARENT.
    ///
    /// It draws no fill of its own — see the type-level note. What separates it
    /// from its neighbour is [`BlockUi::divider`], not a gap and not a shade.
    pub fn column(theme: &AppTheme) -> Div {
        let _ = theme;
        gpui::div().h_full().overflow_hidden()
    }

    /// The hairline between two flush columns.
    pub fn divider(theme: &AppTheme) -> Div {
        gpui::div()
            .h_full()
            .flex_shrink_0()
            .w(theme.shell.column_divider)
            .bg(theme.border._100)
    }

    /// A surface that floats OVER the app rather than being part of it: a
    /// popover, a resizable side panel. Rounded on every corner, and the only
    /// place a [`Material`] still means anything.
    ///
    /// `region` is the glass region key, which must be stable and unique per
    /// surface: it is what the glass host commits its rectangles under.
    pub fn surface<V: 'static>(
        region: &'static str,
        material: Material,
        theme: &AppTheme,
        cx: &mut Context<V>,
    ) -> Div {
        // The radius travels WITH the region. A native glass region is an
        // AppKit view under the Metal layer, so rounding the element around it
        // does nothing at all.
        GlassPanel::surface_rounded(region, material, f32::from(theme.shell.block_radius), cx)
            .rounded(theme.shell.block_radius)
            .overflow_hidden()
    }
}

/// The room the window buttons need at the top of the column that holds them.
///
/// A free function rather than a token because it is a DERIVED height: the
/// buttons' own inset, plus their size, plus the same inset under them. A screen
/// that draws into it collides with AppKit.
pub fn window_controls_band(cx: &App) -> gpui::Pixels {
    rise_ui::theme(cx).shell.window_drag_height
}

#[cfg(test)]
mod tests {
    use rise_theme::ShellMetrics;

    #[test]
    fn a_floating_panel_is_rounded_at_all() {
        let metrics = ShellMetrics::default();
        assert!(
            metrics.block_radius > gpui::px(0.0),
            "a popover with square corners does not read as floating over the app"
        );
    }

    #[test]
    fn two_flush_columns_are_separated_by_a_line_rather_than_a_gap() {
        let metrics = ShellMetrics::default();
        assert!(
            metrics.column_divider > gpui::px(0.0),
            "columns that touch with nothing between them are one column"
        );
        assert!(
            metrics.column_divider < metrics.rail_padding,
            "a divider wider than the rail's own padding reads as a gap, not a line"
        );
    }

    #[test]
    fn the_band_reserved_for_the_window_buttons_clears_them() {
        let metrics = ShellMetrics::default();
        let bottom = metrics.traffic_light_inset * 2.0 + gpui::px(12.0);

        assert!(
            metrics.window_drag_height >= bottom,
            "content drawn at {:?} would collide with a button ending at {bottom:?}",
            metrics.window_drag_height
        );
    }
}
