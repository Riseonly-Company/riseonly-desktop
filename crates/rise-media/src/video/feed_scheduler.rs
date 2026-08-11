//! Which videos in a scrolling feed are decoding, and which are not.
//!
//! Hardware decode sessions are a small fixed resource, so the ceiling is
//! enforced here rather than by the feed. The policy is pure: viewport in,
//! transitions out.

use std::collections::HashMap;

use super::super::texture::external_texture::{BudgetError, BudgetTracker};

/// A stream's identity in the feed. Must be a stable server id, never an index:
/// a prepended page would otherwise re-open every live decoder.
pub type StreamId = u64;

/// How many hardware decode sessions may exist at once.
pub const MAX_LIVE_DECODERS: usize = 5;

/// How many streams ahead of the viewport are opened before they are needed.
pub const PRELOAD_AHEAD: usize = 2;

/// How many already-passed streams stay open for a backwards swipe.
pub const PRELOAD_BEHIND: usize = 1;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StreamState {
    /// Decoding and presenting.
    Playing,
    /// Decoder open and producing frames, but not visible.
    Warm,
    /// No decoder, no GPU memory. A poster frame is all the UI has.
    Closed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transition {
    Open(StreamId),
    Play(StreamId),
    Pause(StreamId),
    Close(StreamId),
}

/// The window of the feed the user can see, plus what is adjacent to it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Viewport {
    /// Fully or partly on screen, in feed order.
    pub visible: Vec<StreamId>,
    /// After the last visible item, in feed order.
    pub after: Vec<StreamId>,
    /// Before the first visible item, in reverse feed order — nearest first.
    pub before: Vec<StreamId>,
}

impl Viewport {
    pub fn new(visible: Vec<StreamId>, after: Vec<StreamId>, before: Vec<StreamId>) -> Self {
        Self {
            visible,
            after,
            before,
        }
    }

    fn desired(&self) -> Vec<(StreamId, StreamState)> {
        let mut desired: Vec<(StreamId, StreamState)> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Visible first and unconditionally: a preload must never take a decoder from a visible video.
        for id in &self.visible {
            if seen.insert(*id) {
                desired.push((*id, StreamState::Playing));
            }
        }

        for id in self.after.iter().take(PRELOAD_AHEAD) {
            if seen.insert(*id) {
                desired.push((*id, StreamState::Warm));
            }
        }

        for id in self.before.iter().take(PRELOAD_BEHIND) {
            if seen.insert(*id) {
                desired.push((*id, StreamState::Warm));
            }
        }

        desired
    }
}

/// Why a stream the policy wanted did not get a decoder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Denied {
    DecoderPoolFull,
    Budget(BudgetError),
}

#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub transitions: Vec<Transition>,
    pub denied: Vec<(StreamId, Denied)>,
}

impl Plan {
    pub fn opens(&self) -> impl Iterator<Item = StreamId> + '_ {
        self.transitions.iter().filter_map(|t| match t {
            Transition::Open(id) => Some(*id),
            _ => None,
        })
    }

    pub fn closes(&self) -> impl Iterator<Item = StreamId> + '_ {
        self.transitions.iter().filter_map(|t| match t {
            Transition::Close(id) => Some(*id),
            _ => None,
        })
    }
}

/// The decoder pool and its policy.
#[derive(Debug)]
pub struct FeedScheduler {
    states: HashMap<StreamId, StreamState>,
    order: Vec<StreamId>,
    max_live: usize,
    tracker: BudgetTracker,
}

impl FeedScheduler {
    pub fn new(tracker: BudgetTracker) -> Self {
        Self::with_capacity(tracker, MAX_LIVE_DECODERS)
    }

    pub fn with_capacity(tracker: BudgetTracker, max_live: usize) -> Self {
        Self {
            states: HashMap::new(),
            order: Vec::new(),
            max_live,
            tracker,
        }
    }

    pub fn state(&self, id: StreamId) -> StreamState {
        self.states.get(&id).copied().unwrap_or(StreamState::Closed)
    }

    pub fn live_count(&self) -> usize {
        self.states
            .values()
            .filter(|state| **state != StreamState::Closed)
            .count()
    }

    pub fn budget(&self) -> &BudgetTracker {
        &self.tracker
    }

    /// Reconcile the pool against a viewport, returning what must happen.
    ///
    /// Closes are emitted before opens and must be applied in that order: a
    /// session's frame buffers are only released on teardown.
    pub fn reconcile(&mut self, viewport: &Viewport) -> Plan {
        let desired = viewport.desired();
        let wanted: HashMap<StreamId, StreamState> = desired.iter().copied().collect();

        let mut plan = Plan::default();

        for id in self.order.clone() {
            if self.state(id) == StreamState::Closed {
                continue;
            }
            if !wanted.contains_key(&id) {
                self.close(id);
                plan.transitions.push(Transition::Close(id));
            }
        }

        // Evict the streams furthest from the viewport rather than refusing newcomers, so a visible video never loses to a warm one.
        let deficit = desired.len().saturating_sub(self.max_live);
        if deficit > 0 {
            for id in self.eviction_candidates(&wanted).into_iter().take(deficit) {
                self.close(id);
                plan.transitions.push(Transition::Close(id));
            }
        }

        for (id, target) in desired {
            let current = self.state(id);

            if current == StreamState::Closed {
                if self.live_count() >= self.max_live {
                    plan.denied.push((id, Denied::DecoderPoolFull));
                    continue;
                }
                self.states.insert(id, target);
                if !self.order.contains(&id) {
                    self.order.push(id);
                }
                plan.transitions.push(Transition::Open(id));
                if target == StreamState::Playing {
                    plan.transitions.push(Transition::Play(id));
                }
                continue;
            }

            if current != target {
                self.states.insert(id, target);
                plan.transitions.push(match target {
                    StreamState::Playing => Transition::Play(id),
                    StreamState::Warm => Transition::Pause(id),
                    StreamState::Closed => Transition::Close(id),
                });
            }
        }

        plan
    }

    /// A playing stream is never a candidate.
    fn eviction_candidates(&self, wanted: &HashMap<StreamId, StreamState>) -> Vec<StreamId> {
        let mut candidates: Vec<StreamId> = self
            .order
            .iter()
            .copied()
            .filter(|id| self.state(*id) == StreamState::Warm)
            .filter(|id| wanted.get(id) != Some(&StreamState::Playing))
            .collect();

        candidates.sort_by_key(|id| wanted.contains_key(id));
        candidates
    }

    fn close(&mut self, id: StreamId) {
        self.states.insert(id, StreamState::Closed);
        self.order.retain(|other| *other != id);
    }

    /// Tear every decoder down.
    pub fn close_all(&mut self) -> Plan {
        let mut plan = Plan::default();

        for id in self.order.clone() {
            self.close(id);
            plan.transitions.push(Transition::Close(id));
        }

        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scheduler() -> FeedScheduler {
        FeedScheduler::new(BudgetTracker::default())
    }

    #[test]
    fn the_visible_stream_plays_and_the_next_two_go_warm() {
        let mut scheduler = scheduler();
        let plan = scheduler.reconcile(&Viewport::new(vec![1], vec![2, 3, 4], vec![]));

        assert_eq!(scheduler.state(1), StreamState::Playing);
        assert_eq!(scheduler.state(2), StreamState::Warm);
        assert_eq!(scheduler.state(3), StreamState::Warm);
        assert_eq!(
            scheduler.state(4),
            StreamState::Closed,
            "preload is bounded at two"
        );
        assert!(plan.denied.is_empty());
    }

    #[test]
    fn one_stream_behind_stays_open_for_a_backwards_swipe() {
        let mut scheduler = scheduler();
        scheduler.reconcile(&Viewport::new(vec![5], vec![6, 7], vec![4, 3, 2]));

        assert_eq!(scheduler.state(4), StreamState::Warm);
        assert_eq!(scheduler.state(3), StreamState::Closed);
    }

    #[test]
    fn scrolling_forward_promotes_the_warm_stream_without_reopening_it() {
        let mut scheduler = scheduler();
        scheduler.reconcile(&Viewport::new(vec![1], vec![2, 3], vec![]));

        let plan = scheduler.reconcile(&Viewport::new(vec![2], vec![3, 4], vec![1]));

        assert!(
            !plan.opens().any(|id| id == 2),
            "the stream the user swiped to was already warm; reopening it is the visible stall"
        );
        assert!(plan.transitions.contains(&Transition::Play(2)));
        assert!(plan.transitions.contains(&Transition::Pause(1)));
        assert_eq!(scheduler.state(2), StreamState::Playing);
    }

    #[test]
    fn a_stream_scrolled_past_is_torn_down_not_left_decoding() {
        let mut scheduler = scheduler();
        scheduler.reconcile(&Viewport::new(vec![1], vec![2, 3], vec![]));

        scheduler.reconcile(&Viewport::new(vec![4], vec![5, 6], vec![3, 2, 1]));

        assert_eq!(scheduler.state(1), StreamState::Closed);
        assert_eq!(scheduler.state(2), StreamState::Closed);
    }

    #[test]
    fn the_decoder_pool_cap_is_never_exceeded_however_the_feed_scrolls() {
        let mut scheduler = scheduler();

        for position in 0..200u64 {
            let before: Vec<u64> = (0..position).rev().take(4).collect();
            let viewport = Viewport::new(
                vec![position],
                (position + 1..position + 5).collect(),
                before,
            );

            scheduler.reconcile(&viewport);

            assert!(
                scheduler.live_count() <= MAX_LIVE_DECODERS,
                "at position {position} the pool held {} decoders",
                scheduler.live_count()
            );
        }
    }

    #[test]
    fn several_visible_streams_all_play_even_when_that_uses_the_whole_pool() {
        let mut scheduler = scheduler();
        let plan = scheduler.reconcile(&Viewport::new(vec![1, 2, 3], vec![4, 5], vec![]));

        for id in [1, 2, 3] {
            assert_eq!(scheduler.state(id), StreamState::Playing, "stream {id}");
        }
        assert!(plan.denied.is_empty());
        assert!(scheduler.live_count() <= MAX_LIVE_DECODERS);
    }

    #[test]
    fn a_visible_stream_is_never_denied_in_favour_of_a_preload() {
        let mut scheduler = FeedScheduler::with_capacity(BudgetTracker::default(), 3);
        scheduler.reconcile(&Viewport::new(vec![1], vec![2, 3], vec![]));

        let plan = scheduler.reconcile(&Viewport::new(vec![10, 11], vec![12, 13], vec![]));

        for id in [10, 11] {
            assert_eq!(scheduler.state(id), StreamState::Playing, "stream {id}");
            assert!(!plan.denied.iter().any(|(denied, _)| *denied == id));
        }
    }

    #[test]
    fn closes_are_emitted_before_opens() {
        let mut scheduler = FeedScheduler::with_capacity(BudgetTracker::default(), 3);
        scheduler.reconcile(&Viewport::new(vec![1], vec![2, 3], vec![]));

        let plan = scheduler.reconcile(&Viewport::new(vec![20], vec![21, 22], vec![]));

        let first_open = plan
            .transitions
            .iter()
            .position(|t| matches!(t, Transition::Open(_)));
        let last_close = plan
            .transitions
            .iter()
            .rposition(|t| matches!(t, Transition::Close(_)));

        if let (Some(open), Some(close)) = (first_open, last_close) {
            assert!(
                close < open,
                "opening before closing needs both sessions at once, and fails on fast scroll"
            );
        }
    }

    #[test]
    fn an_idle_viewport_reconciles_to_no_work_at_all() {
        let mut scheduler = scheduler();
        let viewport = Viewport::new(vec![1], vec![2, 3], vec![]);
        scheduler.reconcile(&viewport);

        let plan = scheduler.reconcile(&viewport);

        assert!(
            plan.transitions.is_empty(),
            "a re-layout that changes nothing must not churn decoders"
        );
    }

    #[test]
    fn leaving_the_feed_releases_every_decoder() {
        let mut scheduler = scheduler();
        scheduler.reconcile(&Viewport::new(vec![1, 2], vec![3, 4], vec![]));

        let plan = scheduler.close_all();

        assert_eq!(scheduler.live_count(), 0);
        assert_eq!(plan.closes().count(), 4);
    }

    #[test]
    fn an_empty_viewport_closes_everything() {
        let mut scheduler = scheduler();
        scheduler.reconcile(&Viewport::new(vec![1], vec![2, 3], vec![]));

        scheduler.reconcile(&Viewport::new(vec![], vec![], vec![]));

        assert_eq!(scheduler.live_count(), 0);
    }

    #[test]
    fn a_duplicated_id_in_the_viewport_opens_one_decoder() {
        let mut scheduler = scheduler();
        let plan = scheduler.reconcile(&Viewport::new(vec![1], vec![1, 2], vec![1]));

        assert_eq!(plan.opens().filter(|id| *id == 1).count(), 1);
        assert_eq!(scheduler.state(1), StreamState::Playing);
    }

    #[test]
    fn a_prepended_page_does_not_disturb_the_live_decoders() {
        let mut scheduler = scheduler();
        scheduler.reconcile(&Viewport::new(vec![100], vec![101, 102], vec![]));

        let plan = scheduler.reconcile(&Viewport::new(vec![100], vec![101, 102], vec![99, 98]));

        assert!(
            !plan.closes().any(|id| id == 100),
            "using indices instead of stable ids is what makes a prepend restart playback"
        );
        assert_eq!(scheduler.state(100), StreamState::Playing);
    }
}
