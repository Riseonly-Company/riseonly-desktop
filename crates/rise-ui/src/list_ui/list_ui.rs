use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnyElement, App, Context, Entity, List, ListAlignment, ListScrollEvent, ListState, Pixels,
    Styled, Window, list,
};
use rise_theme::AppTheme;

use super::pagination::{self, EdgeState, PaginationEdge, ScrollProbe};

/// A long list over gpui's own, with distance-based edge pagination.
///
/// Row identity is the caller's job and must be a stable server id: gpui keys
/// rows by index inside the tree, so a splice at the wrong place reuses the
/// wrong row's state.
pub struct ListUi {
    state: ListState,
    velocity: Rc<RefCell<VelocityTracker>>,
}

impl ListUi {
    pub fn new(theme: &AppTheme, item_count: usize, alignment: ListAlignment) -> Self {
        Self {
            state: ListState::new(item_count, alignment, theme.list.overdraw),
            velocity: Rc::new(RefCell::new(VelocityTracker::default())),
        }
    }

    pub fn state(&self) -> &ListState {
        &self.state
    }

    pub fn item_count(&self) -> usize {
        self.state.item_count()
    }

    /// Rows arrived at the end. The viewport does not move.
    pub fn appended(&self, count: usize) {
        if count == 0 {
            return;
        }
        let end = self.state.item_count();
        self.state.splice(end..end, count);
    }

    /// Rows arrived at the start. The row in view stays under the user's eyes.
    pub fn prepended(&self, count: usize) {
        if count == 0 {
            return;
        }
        self.state.splice(0..0, count);
    }

    pub fn removed(&self, range: std::ops::Range<usize>) {
        if range.is_empty() {
            return;
        }
        self.state.splice(range, 0);
    }

    /// A different list entirely — another feed tab, another account.
    ///
    /// Unlike the splice calls above, this DOES move the viewport to the start.
    pub fn replaced(&self, count: usize) {
        self.state.reset(count);
    }

    /// Where the list is, in pixels. `None` before the first layout, when every
    /// distance would read as zero and fire both edges at once.
    pub fn probe(&self) -> Option<ScrollProbe> {
        let viewport = self.state.viewport_bounds().size.height;
        if viewport <= Pixels::ZERO {
            return None;
        }

        let max = self.state.max_offset_for_scrollbar().y;
        // gpui reports the offset as a negative y.
        let scrolled = -self.state.scroll_px_offset_for_scrollbar().y;

        Some(ScrollProbe {
            viewport_height: viewport,
            distance_to_top: scrolled.max(Pixels::ZERO),
            distance_to_bottom: (max - scrolled).max(Pixels::ZERO),
            velocity: self.velocity.borrow().current(),
        })
    }

    /// Arms the edges. `reached` runs on the scroll path, so it must dispatch an
    /// intent and nothing else — never a fetch, a decode or a sort.
    pub fn on_edge<V: 'static>(
        &self,
        view: &Entity<V>,
        edges: impl Fn(&V, &App) -> EdgeState + 'static,
        reached: impl Fn(&mut V, PaginationEdge, &mut Context<V>) + 'static,
    ) {
        let view = view.downgrade();
        let velocity = Rc::clone(&self.velocity);
        let state = self.state.clone();
        let edges = Rc::new(edges);
        let reached = Rc::new(reached);
        let scheduled = Rc::new(Cell::new(false));

        self.state
            .set_scroll_handler(move |_: &ListScrollEvent, _window, cx| {
                // gpui holds the list's RefCell across this whole callback, so
                // asking the list where it is from here aborts the process: the
                // panic lands in an Objective-C frame that cannot unwind. `defer`
                // is the only safe place — `on_next_frame` runs its callbacks
                // inside an update of the ROOT view, which is a second borrow.
                if scheduled.replace(true) {
                    return;
                }

                let view = view.clone();
                let velocity = Rc::clone(&velocity);
                let state = state.clone();
                let edges = Rc::clone(&edges);
                let reached = Rc::clone(&reached);
                let scheduled = Rc::clone(&scheduled);

                cx.defer(move |cx| {
                    scheduled.set(false);

                    let scrolled = -state.scroll_px_offset_for_scrollbar().y;
                    velocity.borrow_mut().sample(scrolled, Instant::now());

                    let viewport = state.viewport_bounds().size.height;
                    if viewport <= Pixels::ZERO {
                        return;
                    }

                    let probe = ScrollProbe {
                        viewport_height: viewport,
                        distance_to_top: scrolled.max(Pixels::ZERO),
                        distance_to_bottom: (state.max_offset_for_scrollbar().y - scrolled)
                            .max(Pixels::ZERO),
                        velocity: velocity.borrow().current(),
                    };

                    let Some(view) = view.upgrade() else {
                        return;
                    };

                    let metrics = crate::theme(cx).list;
                    let state_of_edges = edges(view.read(cx), cx);
                    let Some(edge) = pagination::evaluate(&metrics, &probe, &state_of_edges) else {
                        return;
                    };

                    view.update(cx, |view, cx| reached(view, edge, cx));
                });
            });
    }

    /// The list fills its container.
    ///
    /// gpui's default sizing behaviour asks taffy for `height: auto`, and the
    /// list keeps its rows out of the layout tree — so an unsized one measures
    /// to nothing and paints nothing, however many rows it holds.
    pub fn element(
        &self,
        render_item: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> List {
        list(self.state.clone(), render_item).size_full()
    }
}

#[derive(Debug)]
struct VelocityTracker {
    last: Option<(Pixels, Instant)>,
    velocity: f32,
}

impl Default for VelocityTracker {
    fn default() -> Self {
        Self {
            last: None,
            velocity: 0.0,
        }
    }
}

impl VelocityTracker {
    /// Seconds; a near-zero gap between samples would report an absurd velocity.
    const MIN_INTERVAL: f32 = 0.004;
    const SMOOTHING: f32 = 0.6;
    /// Seconds without scrolling after which the list counts as at rest.
    const IDLE: f32 = 0.25;

    fn sample(&mut self, offset: Pixels, now: Instant) {
        let Some((previous_offset, previous_at)) = self.last else {
            self.last = Some((offset, now));
            return;
        };

        let elapsed = now.duration_since(previous_at).as_secs_f32();
        if elapsed >= Self::IDLE {
            self.velocity = 0.0;
            self.last = Some((offset, now));
            return;
        }

        let interval = elapsed.max(Self::MIN_INTERVAL);
        let instant = f32::from(offset - previous_offset) / interval;

        self.velocity = self.velocity * Self::SMOOTHING + instant * (1.0 - Self::SMOOTHING);
        self.last = Some((offset, now));
    }

    fn current(&self) -> f32 {
        if self.velocity.is_finite() {
            self.velocity
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;
    use std::time::Duration;

    #[test]
    fn one_sample_is_not_a_velocity() {
        let mut tracker = VelocityTracker::default();
        tracker.sample(px(0.0), Instant::now());
        assert_eq!(tracker.current(), 0.0);
    }

    #[test]
    fn a_steady_scroll_converges_on_its_real_speed() {
        let mut tracker = VelocityTracker::default();
        let mut now = Instant::now();

        let mut offset = 0.0;
        for _ in 0..60 {
            offset += 16.0;
            now += Duration::from_millis(16);
            tracker.sample(px(offset), now);
        }

        assert!(
            (tracker.current() - 1_000.0).abs() < 60.0,
            "converged on {} rather than about 1000",
            tracker.current()
        );
    }

    #[test]
    fn scrolling_upwards_reports_a_negative_velocity() {
        let mut tracker = VelocityTracker::default();
        let mut now = Instant::now();

        let mut offset = 2_000.0;
        for _ in 0..30 {
            offset -= 16.0;
            now += Duration::from_millis(16);
            tracker.sample(px(offset), now);
        }

        assert!(tracker.current() < -500.0);
    }

    #[test]
    fn two_samples_in_the_same_instant_do_not_report_an_absurd_speed() {
        let mut tracker = VelocityTracker::default();
        let now = Instant::now();

        tracker.sample(px(0.0), now);
        tracker.sample(px(40.0), now);

        assert!(
            tracker.current().abs() <= 40.0 / VelocityTracker::MIN_INTERVAL,
            "a zero interval would divide by nothing and pin the threshold at its ceiling"
        );
        assert!(tracker.current().is_finite());
    }

    #[test]
    fn a_list_left_alone_comes_back_to_rest() {
        let mut tracker = VelocityTracker::default();
        let mut now = Instant::now();

        let mut offset = 0.0;
        for _ in 0..30 {
            offset += 32.0;
            now += Duration::from_millis(16);
            tracker.sample(px(offset), now);
        }
        assert!(tracker.current() > 500.0);

        now += Duration::from_secs(2);
        tracker.sample(px(offset), now);
        assert_eq!(
            tracker.current(),
            0.0,
            "a scroll that stopped two seconds ago must not still be widening the trigger"
        );
    }

    #[gpui::test]
    fn a_list_before_its_first_layout_reports_nothing_rather_than_zero_distances(
        cx: &mut gpui::TestAppContext,
    ) {
        let theme = AppTheme::dark();
        let list = cx.update(|_| ListUi::new(&theme, 100, ListAlignment::Top));

        assert!(
            list.probe().is_none(),
            "unmeasured distances read as zero, which fires both edges at once"
        );
    }

    #[gpui::test]
    fn splicing_at_an_end_keeps_the_rows_that_were_already_there(cx: &mut gpui::TestAppContext) {
        let theme = AppTheme::dark();
        let list = cx.update(|_| ListUi::new(&theme, 20, ListAlignment::Top));

        list.appended(10);
        assert_eq!(list.item_count(), 30);

        list.prepended(5);
        assert_eq!(list.item_count(), 35);

        list.removed(0..5);
        assert_eq!(list.item_count(), 30);

        list.replaced(3);
        assert_eq!(list.item_count(), 3);
    }

    #[gpui::test]
    fn splicing_nothing_is_a_no_op_rather_than_a_reset(cx: &mut gpui::TestAppContext) {
        let theme = AppTheme::dark();
        let list = cx.update(|_| ListUi::new(&theme, 20, ListAlignment::Top));

        list.appended(0);
        list.prepended(0);
        list.removed(5..5);
        assert_eq!(list.item_count(), 20);
    }
}
