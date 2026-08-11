//! Which tracks have a decoder open, and when the next one gets one.

/// A track's identity: a stable server id, never a queue index — shuffle and
/// drag-reorder would otherwise tear down the decoder that is playing.
pub type TrackId = u64;

/// How far before the end of a track its successor is opened.
pub const PRELOAD_LEAD_MILLIS: u64 = 2_000;

/// Decoders alive at once: the one playing and the one about to.
pub const MAX_LIVE_DECODERS: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TrackState {
    Playing,
    /// Decoder open and buffering, waiting for the current track to end.
    Preloaded,
    Closed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transition {
    Open(TrackId),
    Play(TrackId),
    Close(TrackId),
}

/// What the queue says should happen next. `next` is `None` at the end of a
/// queue with repeat off.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Upcoming {
    pub current: Option<TrackId>,
    pub next: Option<TrackId>,
}

#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub transitions: Vec<Transition>,
}

impl Plan {
    pub fn opens(&self) -> impl Iterator<Item = TrackId> + '_ {
        self.transitions
            .iter()
            .filter_map(|transition| match transition {
                Transition::Open(id) => Some(*id),
                _ => None,
            })
    }

    pub fn closes(&self) -> impl Iterator<Item = TrackId> + '_ {
        self.transitions
            .iter()
            .filter_map(|transition| match transition {
                Transition::Close(id) => Some(*id),
                _ => None,
            })
    }
}

#[derive(Debug, Default)]
pub struct PlaybackScheduler {
    states: Vec<(TrackId, TrackState)>,
}

impl PlaybackScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self, track: TrackId) -> TrackState {
        self.states
            .iter()
            .find(|(id, _)| *id == track)
            .map(|(_, state)| *state)
            .unwrap_or(TrackState::Closed)
    }

    pub fn live_count(&self) -> usize {
        self.states
            .iter()
            .filter(|(_, state)| *state != TrackState::Closed)
            .count()
    }

    /// Reconcile against the queue.
    ///
    /// `remaining_millis` is how much of the current track is left, or `None`
    /// when the container did not state a length; unknown preloads nothing.
    /// Closes are emitted before opens.
    pub fn reconcile(&mut self, upcoming: &Upcoming, remaining_millis: Option<u64>) -> Plan {
        let mut plan = Plan::default();

        let preload = upcoming
            .next
            .filter(|_| remaining_millis.is_some_and(|left| left <= PRELOAD_LEAD_MILLIS));

        for (id, state) in self.states.clone() {
            if state == TrackState::Closed {
                continue;
            }
            if Some(id) != upcoming.current && Some(id) != preload {
                self.set(id, TrackState::Closed);
                plan.transitions.push(Transition::Close(id));
            }
        }

        if let Some(current) = upcoming.current {
            match self.state(current) {
                TrackState::Playing => {}
                TrackState::Preloaded => {
                    self.set(current, TrackState::Playing);
                    plan.transitions.push(Transition::Play(current));
                }
                TrackState::Closed => {
                    self.set(current, TrackState::Playing);
                    plan.transitions.push(Transition::Open(current));
                    plan.transitions.push(Transition::Play(current));
                }
            }
        }

        if let Some(next) = preload
            && self.state(next) == TrackState::Closed
            && self.live_count() < MAX_LIVE_DECODERS
        {
            self.set(next, TrackState::Preloaded);
            plan.transitions.push(Transition::Open(next));
        }

        plan
    }

    /// Release every decoder.
    pub fn close_all(&mut self) -> Plan {
        let mut plan = Plan::default();

        for (id, state) in self.states.clone() {
            if state != TrackState::Closed {
                self.set(id, TrackState::Closed);
                plan.transitions.push(Transition::Close(id));
            }
        }

        plan
    }

    fn set(&mut self, track: TrackId, state: TrackState) {
        if state == TrackState::Closed {
            self.states.retain(|(id, _)| *id != track);
            return;
        }

        match self.states.iter_mut().find(|(id, _)| *id == track) {
            Some(entry) => entry.1 = state,
            None => self.states.push((track, state)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upcoming(current: u64, next: Option<u64>) -> Upcoming {
        Upcoming {
            current: Some(current),
            next,
        }
    }

    #[test]
    fn pressing_play_opens_exactly_one_decoder() {
        let mut scheduler = PlaybackScheduler::new();

        let plan = scheduler.reconcile(&upcoming(1, Some(2)), Some(180_000));

        assert_eq!(scheduler.state(1), TrackState::Playing);
        assert_eq!(scheduler.state(2), TrackState::Closed);
        assert_eq!(plan.opens().count(), 1);
    }

    #[test]
    fn the_next_track_opens_before_the_current_one_ends() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.reconcile(&upcoming(1, Some(2)), Some(180_000));

        let plan = scheduler.reconcile(&upcoming(1, Some(2)), Some(PRELOAD_LEAD_MILLIS - 1));

        assert_eq!(scheduler.state(2), TrackState::Preloaded);
        assert!(plan.opens().any(|id| id == 2));
    }

    #[test]
    fn the_preloaded_decoder_is_promoted_rather_than_reopened() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.reconcile(&upcoming(1, Some(2)), Some(180_000));
        scheduler.reconcile(&upcoming(1, Some(2)), Some(500));

        let plan = scheduler.reconcile(&upcoming(2, Some(3)), Some(200_000));

        assert!(
            !plan.opens().any(|id| id == 2),
            "reopening the track that was preloaded is the gap the preload exists to remove"
        );
        assert_eq!(scheduler.state(2), TrackState::Playing);
        assert!(plan.closes().any(|id| id == 1));
    }

    #[test]
    fn a_track_of_unknown_length_does_not_hold_a_second_decoder_open() {
        let mut scheduler = PlaybackScheduler::new();

        scheduler.reconcile(&upcoming(1, Some(2)), None);

        assert_eq!(scheduler.state(2), TrackState::Closed);
        assert_eq!(scheduler.live_count(), 1);
    }

    #[test]
    fn the_end_of_a_queue_preloads_nothing() {
        let mut scheduler = PlaybackScheduler::new();

        scheduler.reconcile(&upcoming(1, None), Some(100));

        assert_eq!(scheduler.live_count(), 1);
    }

    #[test]
    fn skipping_forward_closes_the_track_that_was_playing() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.reconcile(&upcoming(1, Some(2)), Some(180_000));

        let plan = scheduler.reconcile(&upcoming(9, Some(10)), Some(180_000));

        assert_eq!(scheduler.state(1), TrackState::Closed);
        assert_eq!(scheduler.state(9), TrackState::Playing);
        assert!(plan.closes().any(|id| id == 1));
    }

    #[test]
    fn closes_are_emitted_before_opens() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.reconcile(&upcoming(1, Some(2)), Some(500));

        let plan = scheduler.reconcile(&upcoming(7, Some(8)), Some(500));

        let first_open = plan
            .transitions
            .iter()
            .position(|t| matches!(t, Transition::Open(_)));
        let last_close = plan
            .transitions
            .iter()
            .rposition(|t| matches!(t, Transition::Close(_)));

        if let (Some(open), Some(close)) = (first_open, last_close) {
            assert!(close < open);
        }
    }

    #[test]
    fn a_whole_album_never_holds_more_than_two_decoders() {
        let mut scheduler = PlaybackScheduler::new();

        for track in 1..=40u64 {
            for remaining in [180_000, 60_000, 3_000, 1_500, 200] {
                scheduler.reconcile(&upcoming(track, Some(track + 1)), Some(remaining));
                assert!(
                    scheduler.live_count() <= MAX_LIVE_DECODERS,
                    "track {track} at {remaining} ms held {} decoders",
                    scheduler.live_count()
                );
            }
        }
    }

    #[test]
    fn reordering_the_queue_below_the_current_track_disturbs_nothing() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.reconcile(&upcoming(100, Some(101)), Some(180_000));

        // Rows moved, but the ids of current and next did not.
        let plan = scheduler.reconcile(&upcoming(100, Some(101)), Some(179_000));

        assert!(plan.transitions.is_empty());
        assert_eq!(scheduler.state(100), TrackState::Playing);
    }

    #[test]
    fn changing_what_is_next_during_a_preload_closes_the_wrong_one() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.reconcile(&upcoming(1, Some(2)), Some(500));
        assert_eq!(scheduler.state(2), TrackState::Preloaded);

        let plan = scheduler.reconcile(&upcoming(1, Some(3)), Some(400));

        assert!(plan.closes().any(|id| id == 2));
        assert_eq!(scheduler.state(3), TrackState::Preloaded);
    }

    #[test]
    fn leaving_the_player_releases_every_decoder() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.reconcile(&upcoming(1, Some(2)), Some(500));

        let plan = scheduler.close_all();

        assert_eq!(scheduler.live_count(), 0);
        assert_eq!(plan.closes().count(), 2);
    }

    #[test]
    fn stopping_playback_entirely_closes_everything() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.reconcile(&upcoming(1, Some(2)), Some(500));

        scheduler.reconcile(&Upcoming::default(), None);

        assert_eq!(scheduler.live_count(), 0);
    }
}
