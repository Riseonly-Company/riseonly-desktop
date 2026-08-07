//! Who is allowed to animate, when everything on screen wants to.
//!
//! Ported from `AnimatedMediaPlaybackCoordinator`. The budget exists to bound
//! the decode pool, not to ration animation: playing a warm sticker is an LZ4
//! decode on a worker plus one texture write per presentation, so it is sized so
//! that everything that can be on screen at once — a full emoji picker grid is
//! the worst case — animates.
//!
//! Deliberately not degraded by thermal state or a battery saver. Freezing
//! stickers because the machine got warm treats the symptom; the cost is held
//! down by the frame cache instead, so there is nothing to trade away.
//!
//! DESKTOP CHANGE. The reference gates on `isApplicationActive`, because a
//! phone app that is not active is not visible. A desktop window is routinely
//! visible and unfocused — half the screen each, the user typing in the other
//! half — and freezing every sticker because focus moved to a browser is a
//! regression the user watches happen. The gate here is window VISIBILITY, and
//! the host is expected to report occlusion, not focus.

pub type PlayerId = u64;

/// Each grant carries an identity so a late callback from a revoked grant
/// cannot install itself over the player's current state. Same reason every
/// async result in this codebase carries a generation.
pub type GrantId = u64;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PlaybackKind {
    Lottie,
    AnimatedImage,
    /// A WEBM or MP4 sticker: a real decode session, not a rasterised sequence,
    /// and priced accordingly.
    VideoSticker,
}

impl PlaybackKind {
    pub const fn cost(self) -> usize {
        match self {
            Self::Lottie | Self::AnimatedImage => 2,
            Self::VideoSticker => 4,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlaybackConditions {
    pub window_visible: bool,
    pub reduce_motion: bool,
}

impl Default for PlaybackConditions {
    fn default() -> Self {
        Self {
            window_visible: true,
            reduce_motion: false,
        }
    }
}

/// Sized so a full picker grid animates. See the module note.
pub const NOMINAL_BUDGET: usize = 128;

pub const fn budget(conditions: PlaybackConditions) -> usize {
    if !conditions.window_visible || conditions.reduce_motion {
        return 0;
    }
    NOMINAL_BUDGET
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transition {
    Granted { player: PlayerId, grant: GrantId },
    Revoked { player: PlayerId, grant: GrantId },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Participant {
    player: PlayerId,
    grant: GrantId,
    kind: PlaybackKind,
}

#[derive(Debug)]
pub struct PlaybackHub {
    active: Vec<Participant>,
    waiting: Vec<Participant>,
    conditions: PlaybackConditions,
    budget: usize,
    next_grant: GrantId,
}

impl PlaybackHub {
    pub fn new(conditions: PlaybackConditions) -> Self {
        Self {
            active: Vec::new(),
            waiting: Vec::new(),
            conditions,
            budget: budget(conditions),
            next_grant: 0,
        }
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    pub fn active_cost(&self) -> usize {
        self.active.iter().map(|p| p.kind.cost()).sum()
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    pub fn waiting_count(&self) -> usize {
        self.waiting.len()
    }

    pub fn is_active(&self, player: PlayerId) -> bool {
        self.active.iter().any(|p| p.player == player)
    }

    /// Ask for a playback slot.
    ///
    /// A newcomer never evicts a player that is still on screen. Evicting the
    /// oldest on every request turned scrolling into continuous slot rotation:
    /// each newly visible sticker revoked a playing one, which tore down its
    /// registration and re-registered it moments later. Slots are released as
    /// their owners leave the viewport, and handed to whoever is waiting, so
    /// the set of playing items stays stable instead of churning once per
    /// scroll tick.
    pub fn request(&mut self, player: PlayerId, kind: PlaybackKind) -> Vec<Transition> {
        if let Some(existing) = self.active.iter_mut().find(|p| p.player == player) {
            existing.kind = kind;
            let grant = existing.grant;
            return vec![Transition::Granted { player, grant }];
        }

        self.waiting.retain(|p| p.player != player);

        self.next_grant += 1;
        let participant = Participant {
            player,
            grant: self.next_grant,
            kind,
        };

        if self.active_cost() + kind.cost() <= self.budget {
            self.active.push(participant);
            return vec![Transition::Granted {
                player,
                grant: participant.grant,
            }];
        }

        self.waiting.push(participant);
        Vec::new()
    }

    /// Give a slot back. Anything waiting that fits takes it.
    pub fn release(&mut self, player: PlayerId) -> Vec<Transition> {
        self.active.retain(|p| p.player != player);
        self.waiting.retain(|p| p.player != player);
        self.fill_available_slots()
    }

    pub fn set_conditions(&mut self, conditions: PlaybackConditions) -> Vec<Transition> {
        self.conditions = conditions;
        self.budget = budget(conditions);
        self.rebalance()
    }

    pub fn conditions(&self) -> PlaybackConditions {
        self.conditions
    }

    fn rebalance(&mut self) -> Vec<Transition> {
        let mut transitions = Vec::new();

        while self.active_cost() > self.budget && !self.active.is_empty() {
            let revoked = self.active.remove(0);
            // Back to the front of the queue: it was playing a moment ago, so
            // it is the likeliest thing to be visible when the budget returns.
            self.waiting.insert(0, revoked);
            transitions.push(Transition::Revoked {
                player: revoked.player,
                grant: revoked.grant,
            });
        }

        transitions.extend(self.fill_available_slots());
        transitions
    }

    fn fill_available_slots(&mut self) -> Vec<Transition> {
        let mut transitions = Vec::new();

        loop {
            let remaining = self.budget.saturating_sub(self.active_cost());
            let Some(index) = self
                .waiting
                .iter()
                .position(|p| p.kind.cost() <= remaining && p.kind.cost() > 0)
            else {
                break;
            };

            let promoted = self.waiting.remove(index);
            self.active.push(promoted);
            transitions.push(Transition::Granted {
                player: promoted.player,
                grant: promoted.grant,
            });
        }

        transitions
    }
}

impl Default for PlaybackHub {
    fn default() -> Self {
        Self::new(PlaybackConditions::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub_with_budget(cost_slots: usize) -> PlaybackHub {
        PlaybackHub {
            budget: cost_slots,
            ..PlaybackHub::default()
        }
    }

    #[test]
    fn a_full_picker_grid_all_animates() {
        let mut hub = PlaybackHub::default();

        for player in 0..NOMINAL_BUDGET as u64 / PlaybackKind::Lottie.cost() as u64 {
            hub.request(player, PlaybackKind::Lottie);
        }

        assert_eq!(hub.active_count(), 64);
        assert_eq!(hub.waiting_count(), 0);
    }

    #[test]
    fn a_video_sticker_costs_twice_a_lottie() {
        let mut hub = hub_with_budget(4);

        hub.request(1, PlaybackKind::VideoSticker);
        hub.request(2, PlaybackKind::Lottie);

        assert!(hub.is_active(1));
        assert!(!hub.is_active(2));
        assert_eq!(hub.waiting_count(), 1);
    }

    #[test]
    fn a_newcomer_never_evicts_a_sticker_that_is_still_on_screen() {
        let mut hub = hub_with_budget(2);
        hub.request(1, PlaybackKind::Lottie);

        let transitions = hub.request(2, PlaybackKind::Lottie);

        assert!(transitions.is_empty(), "scroll churn starts exactly here");
        assert!(hub.is_active(1));
        assert!(!hub.is_active(2));
    }

    #[test]
    fn a_released_slot_goes_to_whoever_was_waiting() {
        let mut hub = hub_with_budget(2);
        hub.request(1, PlaybackKind::Lottie);
        hub.request(2, PlaybackKind::Lottie);

        let transitions = hub.release(1);

        assert_eq!(
            transitions,
            vec![Transition::Granted {
                player: 2,
                grant: 2
            }]
        );
        assert!(hub.is_active(2));
    }

    #[test]
    fn an_occluded_window_stops_every_animation() {
        let mut hub = PlaybackHub::default();
        for player in 0..10 {
            hub.request(player, PlaybackKind::Lottie);
        }

        let transitions = hub.set_conditions(PlaybackConditions {
            window_visible: false,
            reduce_motion: false,
        });

        assert_eq!(transitions.len(), 10);
        assert!(
            transitions
                .iter()
                .all(|t| matches!(t, Transition::Revoked { .. }))
        );
        assert_eq!(hub.active_count(), 0);
    }

    #[test]
    fn everything_resumes_when_the_window_comes_back() {
        let mut hub = PlaybackHub::default();
        for player in 0..10 {
            hub.request(player, PlaybackKind::Lottie);
        }
        hub.set_conditions(PlaybackConditions {
            window_visible: false,
            reduce_motion: false,
        });

        let transitions = hub.set_conditions(PlaybackConditions::default());

        assert_eq!(hub.active_count(), 10);
        assert!(
            transitions
                .iter()
                .all(|t| matches!(t, Transition::Granted { .. }))
        );
    }

    #[test]
    fn reduce_motion_is_honoured_even_with_the_window_in_front() {
        let mut hub = PlaybackHub::default();
        hub.request(1, PlaybackKind::Lottie);

        hub.set_conditions(PlaybackConditions {
            window_visible: true,
            reduce_motion: true,
        });

        assert_eq!(hub.budget(), 0);
        assert_eq!(hub.active_count(), 0);
    }

    #[test]
    fn a_repeat_request_keeps_the_original_grant() {
        let mut hub = PlaybackHub::default();
        let first = hub.request(1, PlaybackKind::Lottie);

        let second = hub.request(1, PlaybackKind::Lottie);

        assert_eq!(first, second);
        assert_eq!(hub.active_count(), 1);
    }

    #[test]
    fn a_grant_id_is_never_reused_so_a_late_callback_can_be_rejected() {
        let mut hub = hub_with_budget(2);
        hub.request(1, PlaybackKind::Lottie);
        hub.release(1);
        let second = hub.request(1, PlaybackKind::Lottie);

        assert_eq!(
            second,
            vec![Transition::Granted {
                player: 1,
                grant: 2
            }]
        );
    }

    #[test]
    fn scrolling_a_full_screen_does_not_churn_the_playing_set() {
        let mut hub = hub_with_budget(20);
        for player in 0..10 {
            hub.request(player, PlaybackKind::Lottie);
        }

        let mut churn = 0;
        for step in 0..100u64 {
            churn += hub.request(100 + step, PlaybackKind::Lottie).len();
        }

        assert_eq!(
            churn, 0,
            "a hundred stickers scrolling past must not touch the ten that are visible"
        );
        assert_eq!(hub.active_count(), 10);
    }
}
