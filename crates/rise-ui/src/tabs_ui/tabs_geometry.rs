use gpui::Pixels;
use rise_theme::TabsMetrics;

/// How the bar decided to lay its tabs out.
#[derive(Clone, PartialEq, Debug)]
pub struct TabsLayout {
    /// One width per tab, in order.
    pub widths: Vec<Pixels>,
    /// Whether the bar shared its width out evenly, or fell back to each tab's
    /// own width and a horizontal scroll.
    pub distributes: bool,
}

impl TabsLayout {
    pub fn total(&self) -> Pixels {
        self.widths
            .iter()
            .copied()
            .fold(Pixels::ZERO, |sum, width| sum + width)
    }

    /// Where each tab starts, in order. The indicator rides on these.
    pub fn offsets(&self) -> Vec<Pixels> {
        let mut offsets = Vec::with_capacity(self.widths.len());
        let mut x = Pixels::ZERO;
        for width in &self.widths {
            offsets.push(x);
            x += *width;
        }
        offsets
    }
}

/// Where the indicator sits, and how wide it is.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct IndicatorGeometry {
    pub x: Pixels,
    pub width: Pixels,
}

/// Below this a drag has not started; a resting finger reports a few thousandths
/// of a page and interpolating against that makes the pill jitter.
const DRAG_EPSILON: f32 = 0.001;

/// Decides between an evenly shared bar and one that keeps each tab's own width
/// and scrolls. `force_scrollable` picks the latter even when the tabs fit.
pub fn layout(
    intrinsic: &[Pixels],
    available: Pixels,
    metrics: &TabsMetrics,
    force_scrollable: bool,
) -> TabsLayout {
    if intrinsic.is_empty() {
        return TabsLayout {
            widths: Vec::new(),
            distributes: false,
        };
    }

    let inner = available - metrics.bar_inner_padding * 2.0;
    let total = intrinsic
        .iter()
        .copied()
        .fold(Pixels::ZERO, |sum, width| sum + width);

    let distributes = !force_scrollable && total > Pixels::ZERO && total <= inner;

    if !distributes {
        return TabsLayout {
            widths: intrinsic.to_vec(),
            distributes: false,
        };
    }

    let each = (inner / intrinsic.len() as f32).max(metrics.tab_min_width);
    TabsLayout {
        widths: vec![each; intrinsic.len()],
        distributes: true,
    }
}

/// The indicator's rectangle for an active tab and a drag in flight. `progress`
/// is signed — negative towards the previous tab — and measured in pages.
pub fn indicator(layout: &TabsLayout, active: usize, progress: f32) -> Option<IndicatorGeometry> {
    let widths = &layout.widths;
    if widths.is_empty() || active >= widths.len() {
        return None;
    }

    let offsets = layout.offsets();
    let current = IndicatorGeometry {
        x: offsets[active],
        width: widths[active],
    };

    if !progress.is_finite() {
        return Some(current);
    }

    let neighbour = if progress < -DRAG_EPSILON {
        active.checked_sub(1)
    } else if progress > DRAG_EPSILON {
        Some(active + 1).filter(|index| *index < widths.len())
    } else {
        None
    };

    // No neighbour that way: the pill stays put rather than sliding off the bar.
    let Some(neighbour) = neighbour else {
        return Some(current);
    };

    let travelled = progress.abs().min(1.0);
    Some(IndicatorGeometry {
        x: current.x + (offsets[neighbour] - current.x) * travelled,
        width: current.width + (widths[neighbour] - current.width) * travelled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    fn metrics() -> TabsMetrics {
        TabsMetrics::default()
    }

    fn scrollable(widths: &[f32]) -> TabsLayout {
        TabsLayout {
            widths: widths.iter().copied().map(px).collect(),
            distributes: false,
        }
    }

    #[test]
    fn a_bar_whose_tabs_fit_shares_its_width_out_evenly() {
        let laid = layout(
            &[px(50.0), px(50.0), px(50.0)],
            px(400.0),
            &metrics(),
            false,
        );

        assert!(laid.distributes);
        assert_eq!(laid.widths.len(), 3);
        assert!(laid.widths.iter().all(|width| *width == laid.widths[0]));
        assert_eq!(laid.total(), px(400.0) - metrics().bar_inner_padding * 2.0);
    }

    #[test]
    fn a_bar_whose_tabs_do_not_fit_keeps_their_own_widths_and_scrolls() {
        let laid = layout(
            &[px(200.0), px(200.0), px(200.0)],
            px(300.0),
            &metrics(),
            false,
        );

        assert!(!laid.distributes);
        assert_eq!(laid.widths, vec![px(200.0), px(200.0), px(200.0)]);
    }

    #[test]
    fn the_feed_forces_the_scrolling_layout_even_though_its_tabs_would_fit() {
        let laid = layout(&[px(40.0), px(60.0), px(50.0)], px(222.0), &metrics(), true);

        assert!(!laid.distributes);
        assert_eq!(laid.widths, vec![px(40.0), px(60.0), px(50.0)]);
    }

    /// The minimum-width clamp wins over the even share, so the row overflows
    /// the bar and scrolls.
    #[test]
    fn a_shared_tab_never_shrinks_below_a_target_worth_aiming_at() {
        let intrinsic = vec![px(5.0); 20];
        let laid = layout(&intrinsic, px(200.0), &metrics(), false);

        assert!(laid.distributes, "100pt of tabs fit inside a 200pt bar");
        assert!(laid.widths.iter().all(|w| *w == metrics().tab_min_width));
        assert!(
            laid.total() > px(200.0),
            "the clamp overflows the bar, which is what the horizontal scroll is for"
        );
    }

    #[test]
    fn an_empty_bar_lays_nothing_out_rather_than_dividing_by_zero() {
        let laid = layout(&[], px(400.0), &metrics(), false);
        assert!(laid.widths.is_empty());
        assert!(!laid.distributes);
        assert_eq!(indicator(&laid, 0, 0.0), None);
    }

    #[test]
    fn offsets_are_the_running_sum_of_the_widths() {
        let laid = scrollable(&[40.0, 60.0, 50.0]);
        assert_eq!(laid.offsets(), vec![px(0.0), px(40.0), px(100.0)]);
    }

    #[test]
    fn at_rest_the_indicator_is_exactly_the_active_tab() {
        let laid = scrollable(&[40.0, 60.0, 50.0]);
        let geometry = indicator(&laid, 1, 0.0).unwrap();

        assert_eq!(geometry.x, px(40.0));
        assert_eq!(geometry.width, px(60.0));
    }

    #[test]
    fn a_half_finished_drag_puts_the_indicator_halfway_between_two_tabs() {
        let laid = scrollable(&[40.0, 60.0, 50.0]);
        let geometry = indicator(&laid, 1, 0.5).unwrap();

        assert_eq!(geometry.x, px(40.0 + (100.0 - 40.0) * 0.5));
        assert_eq!(geometry.width, px(60.0 + (50.0 - 60.0) * 0.5));
    }

    #[test]
    fn dragging_backwards_interpolates_towards_the_previous_tab() {
        let laid = scrollable(&[40.0, 60.0, 50.0]);
        let geometry = indicator(&laid, 1, -0.5).unwrap();

        assert_eq!(geometry.x, px(40.0 + (0.0 - 40.0) * 0.5));
        assert_eq!(geometry.width, px(60.0 + (40.0 - 60.0) * 0.5));
    }

    #[test]
    fn a_drag_past_the_last_tab_leaves_the_indicator_where_it_is() {
        let laid = scrollable(&[40.0, 60.0, 50.0]);

        assert_eq!(
            indicator(&laid, 2, 0.9).unwrap(),
            IndicatorGeometry {
                x: px(100.0),
                width: px(50.0)
            }
        );
        assert_eq!(
            indicator(&laid, 0, -0.9).unwrap(),
            IndicatorGeometry {
                x: px(0.0),
                width: px(40.0)
            }
        );
    }

    #[test]
    fn a_finger_resting_on_the_glass_does_not_move_the_indicator() {
        let laid = scrollable(&[40.0, 60.0, 50.0]);
        let at_rest = indicator(&laid, 1, 0.0).unwrap();

        assert_eq!(indicator(&laid, 1, 0.0005).unwrap(), at_rest);
        assert_eq!(indicator(&laid, 1, -0.0005).unwrap(), at_rest);
    }

    #[test]
    fn an_overshooting_drag_stops_at_the_neighbour_rather_than_past_it() {
        let laid = scrollable(&[40.0, 60.0, 50.0]);
        let geometry = indicator(&laid, 1, 2.5).unwrap();

        assert_eq!(geometry.x, px(100.0));
        assert_eq!(geometry.width, px(50.0));
    }

    #[test]
    fn a_broken_progress_value_never_reaches_the_layout() {
        let laid = scrollable(&[40.0, 60.0, 50.0]);
        let at_rest = indicator(&laid, 1, 0.0).unwrap();

        assert_eq!(indicator(&laid, 1, f32::NAN).unwrap(), at_rest);
        assert_eq!(indicator(&laid, 1, f32::INFINITY).unwrap(), at_rest);
    }

    #[test]
    fn an_active_index_past_the_end_draws_nothing_rather_than_panicking() {
        let laid = scrollable(&[40.0, 60.0]);
        assert_eq!(indicator(&laid, 5, 0.0), None);
    }
}
