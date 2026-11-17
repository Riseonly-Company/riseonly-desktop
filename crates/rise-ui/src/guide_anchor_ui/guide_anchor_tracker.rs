use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{App, Bounds, Pixels, Task, Window};
use rise_canon::{RiseGuideAnchor, RiseGuideCanon};

use crate::guide_anchor_ui::guide_anchor_registry::{
    GuideAnchorRegistry, GuideAnchorSighting, monotonic_ms,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GuideAnchorTrackerReport {
    Moved(Bounds<Pixels>),
    Settled(Bounds<Pixels>),
    Lost(GuideAnchorSighting),
}

type OnReport = Rc<dyn Fn(GuideAnchorTrackerReport, &mut App)>;

#[derive(Default)]
struct TrackerState {
    link: u64,
    is_linked: bool,
    anchor: Option<RiseGuideAnchor>,
    on_report: Option<OnReport>,
    known: Option<Bounds<Pixels>>,
    stable_since: i64,
    announced_settled: bool,
    change_since: Option<i64>,
    last_sighting: Option<GuideAnchorSighting>,
    ticks_since_probe: u32,
    traced_kind: Option<std::mem::Discriminant<GuideAnchorSighting>>,
    probes_since_trace: u32,
    last_tick_ms: i64,
    fallback: Option<Task<()>>,
}

/// one tracker per pointing overlay, clones share it
#[derive(Clone, Default)]
pub struct GuideAnchorTracker {
    state: Rc<RefCell<TrackerState>>,
}

impl GuideAnchorTracker {
    const CONFIRMATION_MS: i64 = 50;

    const SETTLE_MS: i64 = RiseGuideCanon::TIMINGS_ANCHOR_SETTLE_MS;

    const PROBE_EVERY_TICKS: u32 = 3;

    pub fn new() -> Self {
        Self::default()
    }

    /// starts following the anchor from the frame the step was shown with
    /// start frame is passed in so the first report is a real change and not a fake move
    pub fn follow(
        &self,
        anchor: RiseGuideAnchor,
        frame: Bounds<Pixels>,
        on_report: impl Fn(GuideAnchorTrackerReport, &mut App) + 'static,
        cx: &mut App,
    ) {
        self.stop();
        let link = {
            let mut state = self.state.borrow_mut();
            state.anchor = Some(anchor);
            state.on_report = Some(Rc::new(on_report));
            state.known = Some(frame);
            state.stable_since = monotonic_ms();
            state.announced_settled = true;
            state.change_since = None;
            state.last_sighting = Some(GuideAnchorSighting::Visible(frame));
            state.ticks_since_probe = Self::PROBE_EVERY_TICKS;
            state.traced_kind = None;
            state.last_tick_ms = monotonic_ms();
            state.is_linked = true;
            state.link
        };
        let fallback = Self::poll(Rc::downgrade(&self.state), link, cx);
        self.state.borrow_mut().fallback = Some(fallback);

        let Some(window) = GuideAnchorRegistry::window_level(anchor).or_else(|| cx.active_window())
        else {
            tracing::warn!(target: "riseonly::X1738", anchor = anchor.raw_value(), "tracker follow: no window");
            return;
        };
        tracing::warn!(target: "riseonly::X1738", anchor = anchor.raw_value(), window = ?window.window_id(), "tracker follow");
        let state = Rc::downgrade(&self.state);
        cx.defer(move |cx| {
            window
                .update(cx, |_, window, _| Self::schedule(state, link, window))
                .ok();
        });
    }

    pub fn stop(&self) {
        let mut state = self.state.borrow_mut();
        state.link = state.link.wrapping_add(1);
        state.is_linked = false;
        state.anchor = None;
        state.on_report = None;
        state.fallback = None;
    }

    /// samples on the follow interval when no frame did it in time
    /// frames win while they come, then the poll just wakes
    fn poll(state: Weak<RefCell<TrackerState>>, link: u64, cx: &mut App) -> Task<()> {
        let interval = GuideAnchorRegistry::follow_interval_ms();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(interval.unsigned_abs()))
                    .await;
                let Some(strong) = state.upgrade() else { return };
                let is_due = {
                    let current = strong.borrow();
                    if !current.is_linked || current.link != link {
                        return;
                    }
                    monotonic_ms() - current.last_tick_ms >= interval
                };
                if is_due {
                    cx.update(|cx| GuideAnchorTracker { state: strong }.tick(cx));
                }
            }
        })
    }

    /// one tick per displayed frame until stop or the tracker is dropped
    fn schedule(state: Weak<RefCell<TrackerState>>, link: u64, window: &Window) {
        window.on_next_frame(move |window, cx| {
            let Some(strong) = state.upgrade() else { return };
            {
                let current = strong.borrow();
                if !current.is_linked || current.link != link {
                    return;
                }
            }
            GuideAnchorTracker { state: strong }.tick(cx);
            Self::schedule(state, link, window);
        });
    }

    fn tick(&self, cx: &mut App) {
        let (report, absent_anchor) = {
            let mut state = self.state.borrow_mut();
            let Some(anchor) = state.anchor else { return };
            if state.on_report.is_none() {
                return;
            }
            state.last_tick_ms = monotonic_ms();
            let report = Self::step(&mut state, anchor, monotonic_ms());
            let absent = matches!(state.last_sighting, Some(GuideAnchorSighting::Absent));
            (report, absent.then_some(anchor))
        };
        if let Some(anchor) = absent_anchor {
            GuideAnchorRegistry::refresh_hidden(anchor, cx);
        }
        let Some(report) = report else { return };
        let on_report = self.state.borrow().on_report.clone();
        if let Some(on_report) = on_report {
            on_report(report, cx);
        }
    }

    /// one sample, returns the report it made if any
    fn step(
        state: &mut TrackerState,
        anchor: RiseGuideAnchor,
        now: i64,
    ) -> Option<GuideAnchorTrackerReport> {
        state.ticks_since_probe += 1;
        let sighting = if state.ticks_since_probe >= Self::PROBE_EVERY_TICKS
            || state.last_sighting.is_none()
        {
            state.ticks_since_probe = 0;
            let probed = GuideAnchorRegistry::sighting(anchor);
            Self::trace_probe(state, anchor, probed);
            probed
        } else if let (Some(frame), Some(GuideAnchorSighting::Visible(_))) =
            (GuideAnchorRegistry::frame(anchor), state.last_sighting)
        {
            GuideAnchorSighting::Visible(frame)
        } else {
            GuideAnchorRegistry::sighting(anchor)
        };
        state.last_sighting = Some(sighting);

        match sighting {
            GuideAnchorSighting::Visible(measured) => {
                if state.known.is_none() {
                    if !Self::confirmed(state, now) {
                        return None;
                    }
                    state.known = Some(measured);
                    state.stable_since = now;
                    state.announced_settled = false;
                    return Some(GuideAnchorTrackerReport::Moved(measured));
                }
                state.change_since = None;
                if GuideAnchorRegistry::are_same_frame(state.known, Some(measured)) {
                    if state.announced_settled || now - state.stable_since < Self::SETTLE_MS {
                        return None;
                    }
                    state.announced_settled = true;
                    Some(GuideAnchorTrackerReport::Settled(measured))
                } else {
                    state.known = Some(measured);
                    state.stable_since = now;
                    state.announced_settled = false;
                    Some(GuideAnchorTrackerReport::Moved(measured))
                }
            }
            GuideAnchorSighting::Covered | GuideAnchorSighting::Absent => {
                if state.known.is_none() {
                    state.change_since = None;
                    return None;
                }
                if !Self::confirmed(state, now) {
                    return None;
                }
                state.known = None;
                state.announced_settled = true;
                Some(GuideAnchorTrackerReport::Lost(sighting))
            }
        }
    }

    fn trace_probe(state: &mut TrackerState, anchor: RiseGuideAnchor, sighting: GuideAnchorSighting) {
        let kind = std::mem::discriminant(&sighting);
        state.probes_since_trace += 1;
        if state.traced_kind == Some(kind) && state.probes_since_trace < 100 {
            return;
        }
        state.traced_kind = Some(kind);
        state.probes_since_trace = 0;
        tracing::warn!(
            target: "riseonly::X1738",
            anchor = anchor.raw_value(),
            sighting = ?sighting,
            known = ?state.known,
            detail = %GuideAnchorRegistry::describe_sighting(anchor),
            "tracker probe"
        );
    }

    /// true when the state seen now held long enough to trust
    fn confirmed(state: &mut TrackerState, now: i64) -> bool {
        let Some(since) = state.change_since else {
            state.change_since = Some(now);
            return false;
        };
        if now - since < Self::CONFIRMATION_MS {
            return false;
        }
        state.change_since = None;
        true
    }
}
