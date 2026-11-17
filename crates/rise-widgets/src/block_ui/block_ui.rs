use gpui::{App, Context, Div, Pixels, Window, prelude::*};
use rise_platform::HostOs;
use rise_platform::materials::Material;
use rise_platform::window_chrome;
use rise_theme::AppTheme;

use crate::glass_panel::GlassPanel;

/// the app frame, one opaque slab of flush columns
/// left to right the section bar and the centre with its screen (chat list beside it when split)
/// and the right panel when the screen has a context for it
pub struct BlockUi;

impl BlockUi {
    /// the app and the window, now the same rectangle
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

    /// column inside the window, full height and square and transparent
    /// no top band of its own, whoever fills it adds window_controls_band
    pub fn column(theme: &AppTheme) -> Div {
        let _ = theme;
        gpui::div().h_full().overflow_hidden()
    }

    /// centre column, takes what the bar and list and panel leave
    pub fn centre(theme: &AppTheme) -> Div {
        Self::column(theme)
            .flex_1()
            .min_w_0()
            .flex()
            .flex_row()
            .justify_center()
    }

    /// screen inside the centre column, a feed passes the width it keeps
    /// the cap sits on a row child because taffy measures column height before max_w
    pub fn centre_content(content_width: Pixels) -> Div {
        gpui::div().h_full().w_full().max_w(content_width).overflow_hidden()
    }

    /// hairline between two flush columns
    pub fn divider(theme: &AppTheme) -> Div {
        gpui::div()
            .h_full()
            .flex_shrink_0()
            .w(theme.shell.column_divider)
            .bg(theme.border._100)
    }

    /// surface floating over the app like a popover, the only user of Material
    /// region must be stable and unique per surface, the glass host commits rectangles under it
    pub fn surface<V: 'static>(
        region: &'static str,
        material: Material,
        theme: &AppTheme,
        cx: &mut Context<V>,
    ) -> Div {
        GlassPanel::surface_rounded(region, material, f32::from(theme.shell.block_radius), cx)
            .rounded(theme.shell.block_radius)
            .overflow_hidden()
    }
}

/// room a screen leaves atop its column for the shell strip
/// a screen in flow gets it as top padding, a full bleed canvas paints under it
/// and moves only its controls down by this much
pub fn window_controls_band(window: &Window, cx: &App) -> Pixels {
    let strip = f32::from(rise_ui::theme(cx).shell.window_drag_height);
    gpui::px(window_chrome::controls_band_height(
        HostOs::current(),
        window_chrome::granted_decorations(window),
        strip,
    ))
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
