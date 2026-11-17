use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity, Subscription, Task};
use rise_canon::RiseLimits;
use rise_energy::rise_energy_saving::{
    RiseEnergySavingDidChange, RiseEnergySavingFeature, RiseEnergySavingSnapshot, RiseEnergySavingStore,
};
use rise_platform::process_info::{ThermalState, thermal_state};
use rise_ui::swiftui::environment::reduce_motion;

use crate::core::desktop_platform::rise_app_events::{
    RiseAppEvents, RiseNotificationName, RiseSubscription, UIAccessibility, UIApplication,
};
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::lottie_sticker::lottie_sticker_loader::LottieStickerLoader;
use crate::core::ui::lottie_sticker::rise_lottie_frame_cache::RiseLottieFrameCache;
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::RiseLottieFirstFrameCache;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnimatedMediaPlaybackKind {
    Lottie,
    AnimatedEmoji,
    AnimatedImage,
    VideoSticker,
    Gift,
    OpenedAnimation,
}

impl AnimatedMediaPlaybackKind {
    pub fn cost(self) -> i64 {
        match self {
            Self::Lottie | Self::AnimatedEmoji => 2,
            Self::AnimatedImage | Self::OpenedAnimation => 2,
            Self::VideoSticker => 4,
            Self::Gift => 2,
        }
    }

    /// power saving switch that freezes this kind, none when power saving leaves it playing
    pub fn energy_feature(self) -> Option<RiseEnergySavingFeature> {
        match self {
            Self::Lottie | Self::VideoSticker => Some(RiseEnergySavingFeature::AnimatedStickers),
            Self::AnimatedEmoji => Some(RiseEnergySavingFeature::AnimatedEmoji),
            Self::AnimatedImage => Some(RiseEnergySavingFeature::AnimatedGifs),
            Self::Gift | Self::OpenedAnimation => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AnimatedMediaThermalLevel {
    #[default]
    Nominal,
    Fair,
    Serious,
    Critical,
}

impl AnimatedMediaThermalLevel {
    pub fn current() -> Self {
        match thermal_state() {
            ThermalState::Nominal => Self::Nominal,
            ThermalState::Fair => Self::Fair,
            ThermalState::Serious => Self::Serious,
            ThermalState::Critical => Self::Critical,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimatedMediaPlaybackConditions {
    pub is_application_active: bool,
    pub is_reduce_motion_enabled: bool,
    pub energy_saving: RiseEnergySavingSnapshot,
}

impl AnimatedMediaPlaybackConditions {
    /// constructor with the default of no power saving
    pub fn new(is_application_active: bool, is_reduce_motion_enabled: bool) -> Self {
        Self {
            is_application_active,
            is_reduce_motion_enabled,
            energy_saving: RiseEnergySavingSnapshot::NONE,
        }
    }
}

pub enum AnimatedMediaPlaybackPolicy {}

pub static NOMINAL_BUDGET: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("animated_media_playback_budget"));

impl AnimatedMediaPlaybackPolicy {
    pub fn nominal_budget() -> i64 {
        *NOMINAL_BUDGET
    }

    pub fn budget(conditions: &AnimatedMediaPlaybackConditions) -> i64 {
        if !conditions.is_application_active || conditions.is_reduce_motion_enabled {
            return 0;
        }
        Self::nominal_budget()
    }

    /// power saving pauses decorative playback only
    /// not poster loading or functional audio and video players
    /// revoking keeps the frame and queue ownership so content resumes when conditions recover
    pub fn allows_playback(
        kind: AnimatedMediaPlaybackKind,
        conditions: &AnimatedMediaPlaybackConditions,
    ) -> bool {
        let Some(feature) = kind.energy_feature() else {
            return true;
        };
        !conditions.energy_saving.reduces(feature)
    }
}

/// identity of one player, each takes one with next and keeps it for life
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AnimatedMediaParticipantId(u64);

static NEXT_PARTICIPANT_ID: AtomicU64 = AtomicU64::new(1);

impl AnimatedMediaParticipantId {
    pub fn next() -> Self {
        Self(NEXT_PARTICIPANT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// callback that gets the grant id
pub type AnimatedMediaPlaybackCallback = Rc<dyn Fn(u64, &mut App)>;

#[derive(Clone)]
struct Participant {
    id: AnimatedMediaParticipantId,
    grant_id: u64,
    kind: AnimatedMediaPlaybackKind,
    on_granted: AnimatedMediaPlaybackCallback,
    on_revoked: AnimatedMediaPlaybackCallback,
}

impl Participant {
    fn cost(&self) -> i64 {
        self.kind.cost()
    }
}

enum Callback {
    Granted(Participant),
    Revoked(Participant),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoordinatorSignal {
    ReduceMotionStatusDidChange,
    WillEnterForeground,
    DidEnterBackground,
    DidReceiveMemoryWarning,
}

pub struct AnimatedMediaPlaybackCoordinator {
    active: HashMap<AnimatedMediaParticipantId, Participant>,
    active_order: Vec<AnimatedMediaParticipantId>,
    waiting: HashMap<AnimatedMediaParticipantId, Participant>,
    waiting_order: Vec<AnimatedMediaParticipantId>,
    next_grant_id: u64,
    is_application_active: bool,
    conditions: AnimatedMediaPlaybackConditions,
    budget: i64,
    testing_budget_override: Option<i64>,
    _observers: Vec<RiseSubscription>,
    _energy_saving_observer: Subscription,
    _signals: Task<()>,
}

impl AnimatedMediaPlaybackCoordinator {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let is_application_active = RiseAppLifecycle::scene_phase(cx) != ScenePhase::Background;
        let energy_store = RiseEnergySavingStore::shared(cx);
        let conditions = Self::conditions(is_application_active, cx);
        let budget = AnimatedMediaPlaybackPolicy::budget(&conditions);

        let energy_saving_observer = cx.subscribe(
            &energy_store,
            |this: &mut Self, _, _: &RiseEnergySavingDidChange, cx: &mut Context<Self>| {
                this.refresh_budget(cx);
            },
        );

        let (sender, mut inbox) = mpsc::unbounded::<CoordinatorSignal>();
        let center = RiseAppEvents::default_center();
        let observe = |name: RiseNotificationName, signal: CoordinatorSignal| {
            let sender = sender.clone();
            center.observe(name, move |_| {
                let _ = sender.unbounded_send(signal);
            })
        };
        let observers = vec![
            observe(
                UIAccessibility::REDUCE_MOTION_STATUS_DID_CHANGE_NOTIFICATION,
                CoordinatorSignal::ReduceMotionStatusDidChange,
            ),
            observe(UIApplication::WILL_ENTER_FOREGROUND_NOTIFICATION, CoordinatorSignal::WillEnterForeground),
            observe(UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION, CoordinatorSignal::DidEnterBackground),
            observe(
                UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION,
                CoordinatorSignal::DidReceiveMemoryWarning,
            ),
        ];

        let signals = cx.spawn(async move |this, cx| {
            while let Some(signal) = inbox.next().await {
                if signal == CoordinatorSignal::DidReceiveMemoryWarning {
                    RiseLottieFirstFrameCache::reset_memory();
                    RiseLottieFrameCache::shared().release_decoded_frames();
                    LottieStickerLoader::shared().release_memory_caches().await;
                    continue;
                }
                let updated = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                    match signal {
                        CoordinatorSignal::WillEnterForeground => this.is_application_active = true,
                        CoordinatorSignal::DidEnterBackground => this.is_application_active = false,
                        CoordinatorSignal::ReduceMotionStatusDidChange
                        | CoordinatorSignal::DidReceiveMemoryWarning => {}
                    }
                    this.refresh_budget(cx);
                });
                if updated.is_err() {
                    break;
                }
            }
        });

        Self {
            active: HashMap::new(),
            active_order: Vec::new(),
            waiting: HashMap::new(),
            waiting_order: Vec::new(),
            next_grant_id: 0,
            is_application_active,
            conditions,
            budget,
            testing_budget_override: None,
            _observers: observers,
            _energy_saving_observer: energy_saving_observer,
            _signals: signals,
        }
    }

    pub fn request(
        &mut self,
        id: AnimatedMediaParticipantId,
        kind: AnimatedMediaPlaybackKind,
        on_granted: impl Fn(u64, &mut App) + 'static,
        on_revoked: impl Fn(u64, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> u64 {
        let on_granted: AnimatedMediaPlaybackCallback = Rc::new(on_granted);
        let on_revoked: AnimatedMediaPlaybackCallback = Rc::new(on_revoked);

        if let Some(existing_grant_id) = self.active.get(&id).map(|existing| existing.grant_id) {
            let participant = Participant {
                id,
                grant_id: existing_grant_id,
                kind,
                on_granted,
                on_revoked,
            };
            self.active.insert(id, participant.clone());
            self.active_order.retain(|candidate| *candidate != id);
            self.active_order.push(id);
            Self::deliver(vec![Callback::Granted(participant)], cx);
            return existing_grant_id;
        }

        self.next_grant_id = self.next_grant_id.wrapping_add(1);
        let participant = Participant {
            id,
            grant_id: self.next_grant_id,
            kind,
            on_granted,
            on_revoked,
        };
        self.waiting.remove(&id);
        self.waiting_order.retain(|candidate| *candidate != id);

        let mut callbacks = Vec::new();
        let current_budget = self.effective_budget();
        let grant_id = participant.grant_id;

        if self.allows_playback(kind)
            && participant.cost() <= current_budget
            && self.active_cost() + participant.cost() <= current_budget
        {
            self.active.insert(id, participant.clone());
            self.active_order.push(id);
            callbacks.push(Callback::Granted(participant));
        } else {
            tracing::warn!(
                target: "riseonly::X960",
                ?kind,
                allows = self.allows_playback(kind),
                active_cost = self.active_cost(),
                active = self.active.len(),
                budget = current_budget,
                app_active = self.conditions.is_application_active,
                reduce_motion = self.conditions.is_reduce_motion_enabled,
                energy = ?self.conditions.energy_saving,
                "playback request waits"
            );
            self.waiting.insert(id, participant);
            self.waiting_order.push(id);
        }
        Self::deliver(callbacks, cx);
        grant_id
    }

    /// grant_id none releases whatever grant id holds
    pub fn release(
        &mut self,
        id: AnimatedMediaParticipantId,
        grant_id: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if let Some(active_participant) = self.active.get(&id)
            && grant_id.is_none_or(|grant_id| active_participant.grant_id == grant_id)
        {
            let _ = self.remove_active(id);
        }
        if let Some(waiting_participant) = self.waiting.get(&id)
            && grant_id.is_none_or(|grant_id| waiting_participant.grant_id == grant_id)
        {
            self.waiting.remove(&id);
            self.waiting_order.retain(|candidate| *candidate != id);
        }
        let callbacks = self.fill_available_slots();
        Self::deliver(callbacks, cx);
    }

    pub fn reset_for_testing(&mut self) {
        self.active.clear();
        self.active_order.clear();
        self.waiting.clear();
        self.waiting_order.clear();
        self.testing_budget_override = Some(AnimatedMediaPlaybackPolicy::nominal_budget());
    }

    pub fn active_cost_for_testing(&self) -> i64 {
        self.active_cost()
    }

    pub fn waiting_count_for_testing(&self) -> usize {
        self.waiting.len()
    }

    pub fn set_budget_for_testing(&mut self, value: i64, cx: &mut Context<Self>) {
        self.testing_budget_override = Some(value.max(0));
        self.rebalance_for_budget_change(cx);
    }

    fn effective_budget(&self) -> i64 {
        self.testing_budget_override.unwrap_or(self.budget)
    }

    fn active_cost(&self) -> i64 {
        self.active.values().map(Participant::cost).sum()
    }

    fn allows_playback(&self, kind: AnimatedMediaPlaybackKind) -> bool {
        AnimatedMediaPlaybackPolicy::allows_playback(kind, &self.conditions)
    }

    fn refresh_budget(&mut self, cx: &mut Context<Self>) {
        self.conditions = Self::conditions(self.is_application_active, cx);
        self.budget = AnimatedMediaPlaybackPolicy::budget(&self.conditions);
        if self.testing_budget_override.is_some() {
            return;
        }
        self.rebalance_for_budget_change(cx);
    }

    fn rebalance_for_budget_change(&mut self, cx: &mut Context<Self>) {
        let mut callbacks = Vec::new();
        for id in self.active_order.clone() {
            let kind = self
                .active
                .get(&id)
                .map_or(AnimatedMediaPlaybackKind::Lottie, |participant| participant.kind);
            if self.allows_playback(kind) {
                continue;
            }
            let Some(revoked) = self.remove_active(id) else {
                continue;
            };
            self.insert_revoked_into_waiting(revoked.clone());
            callbacks.push(Callback::Revoked(revoked));
        }
        while self.active_cost() > self.effective_budget() {
            let Some(oldest_id) = self.active_order.first().copied() else {
                break;
            };
            let Some(revoked) = self.remove_active(oldest_id) else {
                break;
            };
            self.insert_revoked_into_waiting(revoked.clone());
            callbacks.push(Callback::Revoked(revoked));
        }
        callbacks.extend(self.fill_available_slots());
        Self::deliver(callbacks, cx);
    }

    fn fill_available_slots(&mut self) -> Vec<Callback> {
        let mut callbacks = Vec::new();
        loop {
            let effective_budget = self.effective_budget();
            let active_cost = self.active_cost();
            let Some(index) = self.waiting_order.iter().rposition(|id| {
                self.waiting.get(id).is_some_and(|participant| {
                    self.allows_playback(participant.kind)
                        && active_cost + participant.cost() <= effective_budget
                })
            }) else {
                break;
            };
            let id = self.waiting_order.remove(index);
            let Some(participant) = self.waiting.remove(&id) else {
                continue;
            };
            self.active.insert(id, participant.clone());
            self.active_order.push(id);
            callbacks.push(Callback::Granted(participant));
        }
        callbacks
    }

    fn remove_active(&mut self, id: AnimatedMediaParticipantId) -> Option<Participant> {
        self.active_order.retain(|candidate| *candidate != id);
        self.active.remove(&id)
    }

    fn insert_revoked_into_waiting(&mut self, participant: Participant) {
        let id = participant.id;
        self.waiting.insert(id, participant);
        self.waiting_order.retain(|candidate| *candidate != id);
        self.waiting_order.insert(0, id);
    }

    fn deliver(callbacks: Vec<Callback>, cx: &mut Context<Self>) {
        if callbacks.is_empty() {
            return;
        }
        cx.defer(move |cx| {
            for callback in callbacks {
                match callback {
                    Callback::Granted(participant) => (participant.on_granted)(participant.grant_id, cx),
                    Callback::Revoked(participant) => (participant.on_revoked)(participant.grant_id, cx),
                }
            }
        });
    }

    fn conditions(is_application_active: bool, cx: &App) -> AnimatedMediaPlaybackConditions {
        AnimatedMediaPlaybackConditions {
            is_application_active,
            is_reduce_motion_enabled: reduce_motion(cx),
            energy_saving: RiseEnergySavingStore::snapshot(),
        }
    }
}
