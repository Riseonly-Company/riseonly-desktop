use gpui::Pixels;

use crate::tokens::density::Density;

/// The desktop frame's own geometry: how wide each column is.
///
/// `rise-navigation`'s `PanePolicy` decides how many columns a width affords,
/// and must be built from these same numbers via `PanePolicy::from_shell_metrics`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShellMetrics {
    pub rail_width: Pixels,
    /// The Telegram-style breathing room around the rail only. Content columns
    /// remain edge-to-edge with the window.
    pub rail_outer_inset: Pixels,
    pub rail_item_size: Pixels,
    pub rail_item_gap: Pixels,
    pub rail_padding: Pixels,
    pub rail_icon_size: Pixels,
    pub sidebar_width: Pixels,
    pub aside_width: Pixels,
    pub content_min_width: Pixels,
    /// What the window opens at the first time; wide enough for all three
    /// columns plus the rail.
    pub window_default_width: Pixels,
    pub window_default_height: Pixels,
    /// How far a resize must travel past a column boundary before the layout is
    /// allowed to flip, or a window dragged along a threshold strobes.
    pub pane_hysteresis: Pixels,
    /// How far a second window opens down and to the right of the last one.
    pub window_cascade_offset: Pixels,
    /// How close an overlay may come to the window edge. gpui has no native
    /// popups on X11 or Windows, so every menu is drawn inside the window.
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
    /// How far in from the window's top-left the macOS traffic lights sit, which
    /// is exactly what AppKit is told — see [`Self::traffic_light_origin`].
    pub traffic_light_inset: Pixels,
    /// The band across the top of the window that belongs to the window controls
    /// and to dragging, on every platform, and that no screen may draw into.
    pub window_drag_height: Pixels,

    /// A floating panel's corner — a popover, a resizable side panel. NOT the
    /// window's, which is AppKit's and is not a value we hold.
    pub block_radius: Pixels,
    /// The hairline between two flush columns, which is what separates them now
    /// that there is no gap.
    pub column_divider: Pixels,
}

impl ShellMetrics {
    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            // Floor, because density scales the rail but not the AppKit-drawn buttons.
            rail_width: l(90.0).max(gpui::px(
                Self::TRAFFIC_LIGHT_SPAN + Self::TRAFFIC_LIGHT_INSET * 2.0,
            )),
            rail_outer_inset: gpui::px(5.0),
            rail_item_size: l(40.0),
            rail_item_gap: l(6.0),
            rail_padding: l(6.0),
            rail_icon_size: l(22.0),
            sidebar_width: l(320.0),
            aside_width: l(300.0),
            content_min_width: l(460.0),
            window_default_width: l(1196.0),
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
            // Not density-scaled: AppKit draws the traffic lights at a fixed size.
            traffic_light_inset: gpui::px(Self::TRAFFIC_LIGHT_INSET),
            window_drag_height: gpui::px(46.0),

            block_radius: l(10.0),
            column_divider: gpui::px(1.0),
        }
    }
}

impl ShellMetrics {
    /// Where AppKit is told to put the window buttons, measured from the
    /// window's top-left. The rail itself starts after [`Self::rail_outer_inset`],
    /// then keeps the same inset around the fixed-size AppKit buttons.
    pub fn traffic_light_origin(&self) -> gpui::Point<Pixels> {
        let origin = self.rail_outer_inset + self.traffic_light_inset;
        gpui::point(origin, origin)
    }

    /// How wide the three window buttons and their gaps are, together. Fixed by
    /// AppKit; the rail must be at least this wide plus its insets.
    pub const TRAFFIC_LIGHT_SPAN: f32 = 14.0 * 3.0 + 9.0 * 2.0;
    const TRAFFIC_LIGHT_INSET: f32 = 15.0;
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

    /// The window buttons sit inside the rail, so it must hold them with
    /// their inset on both sides.
    #[test]
    fn the_rail_is_wide_enough_to_hold_the_window_buttons() {
        for density in [Density::COMPACT, Density::NORMAL, Density::COMFORTABLE] {
            let metrics = ShellMetrics::new(density);
            let needed = px(ShellMetrics::TRAFFIC_LIGHT_SPAN) + metrics.traffic_light_inset * 2.0;

            assert!(
                metrics.rail_width >= needed,
                "at {density:?} a {:?} rail cannot hold {needed:?} of window buttons",
                metrics.rail_width
            );
        }
    }

    /// AppKit is handed a window-space origin. The rail alone floats five
    /// points inside the window, so that outer inset must be added exactly once.
    #[test]
    fn the_buttons_are_placed_inside_the_inset_rail() {
        let metrics = ShellMetrics::default();
        let origin = metrics.traffic_light_origin();

        assert_eq!(
            origin.x,
            metrics.rail_outer_inset + metrics.traffic_light_inset
        );
        assert_eq!(
            origin.y, origin.x,
            "the buttons are inset equally on both axes"
        );
    }

    /// AppKit rounds the window itself and we hold no radius for it. The buttons
    /// still have to clear that corner, and the OS radius is not a number this
    /// crate knows — so the check is against a generous upper bound on it.
    #[test]
    fn the_close_button_clears_any_plausible_macos_window_corner() {
        const WIDEST_PLAUSIBLE_OS_RADIUS: f32 = 26.0;

        for density in [Density::COMPACT, Density::NORMAL, Density::COMFORTABLE] {
            let metrics = ShellMetrics::new(density);
            let origin = f32::from(metrics.traffic_light_origin().x);
            let centre = WIDEST_PLAUSIBLE_OS_RADIUS;

            let offset = (centre - origin).max(0.0);
            assert!(
                offset * offset * 2.0 <= centre * centre,
                "at {density:?} the close button would ride on the window's rounding"
            );
        }
    }

    #[test]
    fn the_default_shell_is_the_documented_geometry() {
        let metrics = ShellMetrics::default();

        assert_eq!(metrics.rail_width, px(90.0));
        assert_eq!(metrics.rail_outer_inset, px(5.0));
        assert_eq!(metrics.block_radius, px(10.0));
        assert_eq!(metrics.column_divider, px(1.0));
        assert_eq!(metrics.traffic_light_inset, px(15.0));
        assert_eq!(metrics.sidebar_width, px(320.0));
        assert_eq!(metrics.aside_width, px(300.0));
    }

    #[test]
    fn density_scales_every_column() {
        let dense = ShellMetrics::new(Density::new(1.25));

        assert_eq!(dense.rail_width, px(90.0 * 1.25));
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

    /// An overlay's asked-for width is sane at every density; clamping it is the
    /// placement code's job.
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
                + metrics.rail_outer_inset * 2.0
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
