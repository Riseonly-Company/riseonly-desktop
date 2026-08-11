use gpui::Pixels;
use rise_theme::PanelMetrics;

/// What letting go of the resize grip means.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ResizeOutcome {
    /// Settle at this width, already clamped into the allowed range.
    Settle(Pixels),
    /// The user dragged it away: close, without animating back through the
    /// widths they just dragged through.
    Dismiss,
}

/// The width to draw mid-drag. Allowed below `min_width` on purpose — the floor
/// is applied on release, so the panel can visibly approach nothing.
pub fn width_during_drag(metrics: &PanelMetrics, start: Pixels, delta_x: Pixels) -> Pixels {
    (start + delta_x).clamp(Pixels::ZERO, metrics.max_width)
}

/// What that width means once the pointer comes up.
pub fn release(metrics: &PanelMetrics, width: Pixels) -> ResizeOutcome {
    if width < metrics.dismiss_width() {
        ResizeOutcome::Dismiss
    } else {
        ResizeOutcome::Settle(width.clamp(metrics.min_width, metrics.max_width))
    }
}

/// Whether a horizontal swipe should pop the panel's inner page: a distance or a
/// flick. `translation` is positive rightwards; `velocity` is points per second
/// in the same sense.
pub fn should_pop(metrics: &PanelMetrics, translation: Pixels, velocity: f32) -> bool {
    if translation <= Pixels::ZERO {
        return false;
    }

    if translation >= metrics.swipe_back_distance {
        return true;
    }

    let flicked = velocity.is_finite() && velocity >= metrics.swipe_back_velocity;
    flicked && translation >= metrics.swipe_back_distance / 3.0
}

/// A stack of pages inside the panel, with a root that can never be popped.
#[derive(Clone, Debug)]
pub struct PanelPages<T> {
    root: T,
    pushed: Vec<T>,
}

impl<T> PanelPages<T> {
    pub fn new(root: T) -> Self {
        Self {
            root,
            pushed: Vec::new(),
        }
    }

    pub fn root(&self) -> &T {
        &self.root
    }

    pub fn top(&self) -> &T {
        self.pushed.last().unwrap_or(&self.root)
    }

    pub fn top_mut(&mut self) -> &mut T {
        self.pushed.last_mut().unwrap_or(&mut self.root)
    }

    /// How many pages are above the root. Zero means the root is showing.
    pub fn depth(&self) -> usize {
        self.pushed.len()
    }

    pub fn is_at_root(&self) -> bool {
        self.pushed.is_empty()
    }

    pub fn push(&mut self, page: T) {
        self.pushed.push(page);
    }

    /// Pops one page, or reports that there was nothing to pop — Esc pops a page
    /// if there is one and closes the whole panel if there is not.
    pub fn pop(&mut self) -> Option<T> {
        self.pushed.pop()
    }

    pub fn reset(&mut self) -> Vec<T> {
        std::mem::take(&mut self.pushed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    fn metrics() -> PanelMetrics {
        PanelMetrics::default()
    }

    #[test]
    fn dragging_the_grip_widens_and_narrows_the_panel() {
        let metrics = metrics();
        assert_eq!(
            width_during_drag(&metrics, metrics.default_width, px(80.0)),
            metrics.default_width + px(80.0)
        );
        assert_eq!(
            width_during_drag(&metrics, metrics.default_width, px(-80.0)),
            metrics.default_width - px(80.0)
        );
    }

    #[test]
    fn a_drag_may_go_below_the_floor_so_the_dismissal_is_visible() {
        let metrics = metrics();
        let narrow = width_during_drag(&metrics, metrics.default_width, px(-300.0));

        assert!(
            narrow < metrics.min_width,
            "clamping during the drag would make a dismissal happen with no warning"
        );
        assert!(narrow >= Pixels::ZERO, "and never past nothing");
    }

    #[test]
    fn a_drag_stops_at_the_ceiling_however_far_it_goes() {
        let metrics = metrics();
        assert_eq!(
            width_during_drag(&metrics, metrics.default_width, px(5_000.0)),
            metrics.max_width
        );
    }

    #[test]
    fn letting_go_past_half_closes_the_panel() {
        let metrics = metrics();
        assert_eq!(
            release(&metrics, metrics.dismiss_width() - px(1.0)),
            ResizeOutcome::Dismiss
        );
    }

    #[test]
    fn letting_go_short_of_half_snaps_back_to_the_floor() {
        let metrics = metrics();
        assert_eq!(
            release(&metrics, metrics.dismiss_width() + px(1.0)),
            ResizeOutcome::Settle(metrics.min_width),
            "between the dismiss threshold and the floor, the panel returns to its narrowest"
        );
    }

    #[test]
    fn letting_go_inside_the_range_keeps_the_width_the_user_chose() {
        let metrics = metrics();
        let chosen = metrics.default_width + px(40.0);
        assert_eq!(release(&metrics, chosen), ResizeOutcome::Settle(chosen));
    }

    #[test]
    fn a_swipe_the_wrong_way_never_pops() {
        let metrics = metrics();
        assert!(!should_pop(&metrics, px(-200.0), 5_000.0));
        assert!(!should_pop(&metrics, Pixels::ZERO, 5_000.0));
    }

    #[test]
    fn a_long_slow_swipe_pops() {
        let metrics = metrics();
        assert!(should_pop(&metrics, metrics.swipe_back_distance, 0.0));
    }

    #[test]
    fn a_short_fast_flick_pops_too() {
        let metrics = metrics();
        assert!(should_pop(
            &metrics,
            metrics.swipe_back_distance / 2.0,
            metrics.swipe_back_velocity + 1.0
        ));
    }

    #[test]
    fn a_short_slow_nudge_does_not() {
        let metrics = metrics();
        assert!(!should_pop(
            &metrics,
            metrics.swipe_back_distance / 2.0,
            10.0
        ));
    }

    #[test]
    fn a_flick_that_has_barely_moved_does_not_pop_either() {
        let metrics = metrics();
        assert!(
            !should_pop(&metrics, px(4.0), 50_000.0),
            "a fast twitch is not a gesture"
        );
    }

    #[test]
    fn a_broken_velocity_falls_back_to_the_distance_rule() {
        let metrics = metrics();
        assert!(!should_pop(
            &metrics,
            metrics.swipe_back_distance / 2.0,
            f32::NAN
        ));
        assert!(should_pop(&metrics, metrics.swipe_back_distance, f32::NAN));
    }

    #[test]
    fn the_root_page_can_never_be_popped_away() {
        let mut pages = PanelPages::new("comments");
        assert!(pages.is_at_root());
        assert_eq!(pages.pop(), None);
        assert_eq!(*pages.top(), "comments");

        pages.push("replies");
        assert_eq!(pages.depth(), 1);
        assert_eq!(*pages.top(), "replies");

        assert_eq!(pages.pop(), Some("replies"));
        assert!(pages.is_at_root());
        assert_eq!(*pages.top(), "comments");
    }

    #[test]
    fn resetting_returns_every_page_it_dropped() {
        let mut pages = PanelPages::new("comments");
        pages.push("replies");
        pages.push("deeper");

        assert_eq!(pages.reset(), vec!["replies", "deeper"]);
        assert!(pages.is_at_root());
    }
}
