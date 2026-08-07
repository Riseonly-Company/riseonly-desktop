use gpui::Pixels;

use crate::tokens::density::Density;

/// The desktop frame's own geometry — the one part of the design that is not the
/// phone's.
///
/// `docs/PLATFORM_GLASS.md` fixes the shape: rail 56, list 320, content
/// flexible, aside 300. Those numbers live here rather than in the shell because
/// they are lengths, and a length outside `rise-theme` is a length density can
/// never reach.
///
/// `rise-navigation`'s `PanePolicy` decides how many columns a width affords;
/// this decides how wide each column is. The two must be built from the same
/// numbers, which is what `PanePolicy::from_shell_metrics` is for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShellMetrics {
    pub rail_width: Pixels,
    pub rail_item_size: Pixels,
    pub rail_item_gap: Pixels,
    pub rail_padding: Pixels,
    pub rail_icon_size: Pixels,
    pub sidebar_width: Pixels,
    pub aside_width: Pixels,
    pub content_min_width: Pixels,
    /// What the window opens at the first time.
    ///
    /// Wide enough for all three columns plus the rail, so a first run shows the
    /// shell the design is about rather than the phone-shaped stack it collapses
    /// to. It scales with density for the same reason everything else does: a
    /// comfortable-density user wants a bigger window, not a more crowded one.
    pub window_default_width: Pixels,
    pub window_default_height: Pixels,
    /// How far a resize must travel past a column boundary before the layout is
    /// allowed to flip. Without it a window dragged along a threshold strobes
    /// between one and two columns.
    pub pane_hysteresis: Pixels,
    /// How far a second window opens down and to the right of the last one, so
    /// two windows onto the same account are not one window as far as the eye
    /// can tell.
    pub window_cascade_offset: Pixels,
    /// How close an overlay may come to the window edge.
    ///
    /// gpui has no native popups on X11 or Windows, so every menu, dropdown and
    /// tooltip is drawn inside the window. These are the numbers that keep one
    /// from touching the frame, and they are here rather than in the widget for
    /// the same reason every other length is: a widget that spells a number is a
    /// widget density cannot reach.
    pub overlay_margin: Pixels,
    /// Between an overlay and the rectangle it is anchored to.
    pub overlay_gap: Pixels,
    pub menu_min_width: Pixels,
    pub menu_max_width: Pixels,
    pub menu_row_height: Pixels,
    pub palette_width: Pixels,
    /// How far below the window's top edge the command palette opens.
    pub palette_top_inset: Pixels,
    pub palette_max_height: Pixels,
    pub palette_row_height: Pixels,
    /// How far in from the window's top-left the macOS traffic lights sit.
    ///
    /// The window has no titlebar strip — content runs to the top edge, the way
    /// Telegram for macOS and Zed do it — so the buttons are placed over the
    /// content and everything else has to keep out of their way.
    pub traffic_light_inset: Pixels,
    /// The band across the top of the window that belongs to the window
    /// controls and to dragging, and that no screen may put anything in.
    ///
    /// It exists on every platform, not only where the traffic lights are: a
    /// frameless window still needs somewhere the user can grab it.
    pub window_drag_height: Pixels,
}

impl ShellMetrics {
    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            rail_width: l(56.0),
            rail_item_size: l(40.0),
            rail_item_gap: l(6.0),
            rail_padding: l(6.0),
            rail_icon_size: l(22.0),
            sidebar_width: l(320.0),
            aside_width: l(300.0),
            content_min_width: l(460.0),
            window_default_width: l(1180.0),
            window_default_height: l(760.0),
            pane_hysteresis: l(24.0),
            window_cascade_offset: l(28.0),
            overlay_margin: l(12.0),
            overlay_gap: l(4.0),
            menu_min_width: l(180.0),
            menu_max_width: l(320.0),
            menu_row_height: l(28.0),
            palette_width: l(560.0),
            palette_top_inset: l(96.0),
            palette_max_height: l(420.0),
            palette_row_height: l(40.0),
            // Not scaled by density. The traffic lights are drawn by AppKit at a
            // fixed size, so an inset that grew with density would stop lining
            // up with them.
            traffic_light_inset: gpui::px(19.0),
            window_drag_height: gpui::px(38.0),
        }
    }
}

impl Default for ShellMetrics {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn the_window_controls_fit_inside_the_band_reserved_for_them() {
        for density in [Density::COMPACT, Density::NORMAL, Density::COMFORTABLE] {
            let metrics = ShellMetrics::new(density);
            assert!(
                metrics.traffic_light_inset < metrics.window_drag_height,
                "at {density:?} the traffic lights sit below the band that keeps content out"
            );
        }
    }

    #[test]
    fn the_traffic_light_inset_does_not_scale_with_density() {
        assert_eq!(
            ShellMetrics::new(Density::COMFORTABLE).traffic_light_inset,
            ShellMetrics::new(Density::COMPACT).traffic_light_inset,
            "AppKit draws the buttons at a fixed size; a scaled inset stops lining up"
        );
    }

    #[test]
    fn the_default_shell_is_the_documented_geometry() {
        let metrics = ShellMetrics::default();

        assert_eq!(metrics.rail_width, px(56.0));
        assert_eq!(metrics.sidebar_width, px(320.0));
        assert_eq!(metrics.aside_width, px(300.0));
    }

    #[test]
    fn density_scales_every_column() {
        let dense = ShellMetrics::new(Density::new(1.25));

        assert_eq!(dense.rail_width, px(56.0 * 1.25));
        assert_eq!(dense.sidebar_width, px(320.0 * 1.25));
        assert_eq!(dense.aside_width, px(300.0 * 1.25));
    }

    #[test]
    fn a_rail_item_fits_inside_the_rail_with_its_padding() {
        for density in [Density::COMPACT, Density::NORMAL, Density::COMFORTABLE] {
            let metrics = ShellMetrics::new(density);
            assert!(
                metrics.rail_item_size + metrics.rail_padding * 2.0 <= metrics.rail_width,
                "the rail item overflows the rail at {density:?}"
            );
        }
    }

    #[test]
    fn the_icon_fits_inside_the_item_it_sits_in() {
        let metrics = ShellMetrics::default();
        assert!(metrics.rail_icon_size < metrics.rail_item_size);
    }

    /// `palette_width` and `menu_max_width` are what an overlay asks for, not
    /// what it gets: a 400px window clamps both, which is the placement code's
    /// job. What the tokens owe is that the ask is sane at every density — it
    /// fits the window the app opens at, and a menu fits the narrowest column
    /// the shell will ever draw.
    #[test]
    fn what_an_overlay_asks_for_fits_the_window_the_app_opens_at() {
        for density in [Density::COMPACT, Density::NORMAL, Density::COMFORTABLE] {
            let metrics = ShellMetrics::new(density);

            assert!(
                metrics.palette_width + metrics.overlay_margin * 2.0
                    <= metrics.window_default_width,
                "the palette at {density:?} is clamped on a first run"
            );
            assert!(
                metrics.menu_max_width + metrics.overlay_margin * 2.0 <= metrics.content_min_width,
                "a menu at {density:?} cannot open inside the narrowest content column"
            );
            assert!(metrics.menu_min_width <= metrics.menu_max_width);
            assert!(metrics.overlay_gap < metrics.overlay_margin);
        }
    }

    #[test]
    fn the_palette_leaves_room_below_itself_in_a_default_window() {
        let metrics = ShellMetrics::default();
        assert!(
            metrics.palette_top_inset + metrics.palette_max_height + metrics.overlay_margin
                <= metrics.window_default_height
        );
        assert!(metrics.palette_row_height > metrics.menu_row_height);
    }

    #[test]
    fn the_first_window_is_wide_enough_for_the_shell_it_is_showing() {
        for density in [Density::COMPACT, Density::NORMAL, Density::COMFORTABLE] {
            let metrics = ShellMetrics::new(density);
            let three_columns = metrics.rail_width
                + metrics.sidebar_width
                + metrics.content_min_width
                + metrics.aside_width;

            assert!(
                metrics.window_default_width >= three_columns,
                "a first run at {density:?} opens collapsed to a phone-shaped stack"
            );
        }
    }
}
