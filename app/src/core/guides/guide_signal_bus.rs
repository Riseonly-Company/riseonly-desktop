use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use futures::channel::oneshot;
use gpui::{App, AsyncApp};
use rise_canon::{RiseGuideCanon, RiseGuideSignal};

use crate::core::guides::guide_trace::GuideTrace;

/// one shot bridge from a callback to await
/// shared by the signal bus and the screen observer
/// event or timeout or cancel can settle it in any order
#[derive(Clone, Default)]
pub struct GuideEventAwaiter {
    state: Rc<RefCell<GuideEventAwaiterState>>,
}

#[derive(Default)]
struct GuideEventAwaiterState {
    continuation: Option<oneshot::Sender<bool>>,
    result: bool,
    is_settled: bool,
}

impl GuideEventAwaiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_settled(&self) -> bool {
        self.state.borrow().is_settled
    }

    pub fn attach(&self, continuation: oneshot::Sender<bool>) {
        let mut state = self.state.borrow_mut();
        if state.is_settled {
            let _ = continuation.send(state.result);
            return;
        }
        state.continuation = Some(continuation);
    }

    pub fn settle(&self, value: bool) -> bool {
        let continuation = {
            let mut state = self.state.borrow_mut();
            if state.is_settled {
                return false;
            }
            state.is_settled = true;
            state.result = value;
            state.continuation.take()
        };
        if let Some(continuation) = continuation {
            let _ = continuation.send(value);
        }
        true
    }

    /// same awaiter, not an equal one
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.state, &other.state)
    }

    /// identity of the awaiter
    pub fn object_identifier(&self) -> usize {
        Rc::as_ptr(&self.state) as usize
    }
}

pub type GuideEventRelease = Rc<dyn Fn(&GuideEventAwaiter)>;

pub enum GuideEventWait {}

impl GuideEventWait {
    /// gives up after timeout_ms or on cancellation
    /// release drops the callers registration and must be idempotent
    /// the event may have removed it already
    pub async fn run(
        awaiter: GuideEventAwaiter,
        timeout_ms: i64,
        release: impl Fn(&GuideEventAwaiter) + 'static,
        cx: &AsyncApp,
    ) -> bool {
        let release: GuideEventRelease = Rc::new(release);
        let timer = cx
            .background_executor()
            .timer(Duration::from_millis(u64::try_from(timeout_ms).unwrap_or(0)));
        let timeout = {
            let awaiter = awaiter.clone();
            let release = release.clone();
            cx.spawn(async move |_| {
                timer.await;
                release(&awaiter);
                awaiter.settle(false);
            })
        };

        let mut on_cancel = GuideEventWaitCancellation {
            armed: Some((awaiter.clone(), release)),
            cx: cx.clone(),
        };
        let (continuation, delivery) = oneshot::channel();
        awaiter.attach(continuation);
        let delivered = delivery.await.unwrap_or(false);
        on_cancel.armed = None;

        drop(timeout);
        delivered
    }
}

struct GuideEventWaitCancellation {
    armed: Option<(GuideEventAwaiter, GuideEventRelease)>,
    cx: AsyncApp,
}

impl Drop for GuideEventWaitCancellation {
    fn drop(&mut self) {
        let Some((awaiter, release)) = self.armed.take() else {
            return;
        };
        self.cx
            .spawn(async move |_| {
                release(&awaiter);
                awaiter.settle(false);
            })
            .detach();
    }
}

type GuideSignalTrigger = Rc<dyn Fn(RiseGuideSignal, &mut App)>;

#[derive(Default)]
struct GuideSignalBusState {
    waiters: HashMap<RiseGuideSignal, Vec<GuideEventAwaiter>>,
    signals_by_awaiter: HashMap<usize, RiseGuideSignal>,
    has_waiters: bool,
    trigger_events: HashSet<RiseGuideSignal>,
    has_trigger_events: bool,
    on_trigger_event: Option<GuideSignalTrigger>,
    app: Option<AsyncApp>,
    emissions: HashMap<RiseGuideSignal, u64>,
}

thread_local! {
    static GUIDE_SIGNAL_BUS: RefCell<GuideSignalBusState> = RefCell::default();
}

/// where the app reports that the user did an action
/// a step with advance signal waits for feature code to say it, nothing is queued so arm the wait first
pub enum GuideSignalBus {}

impl GuideSignalBus {
    pub fn install_triggers(
        events: HashSet<RiseGuideSignal>,
        on_emit: impl Fn(RiseGuideSignal, &mut App) + 'static,
        cx: &mut App,
    ) {
        let app = cx.to_async();
        GUIDE_SIGNAL_BUS.with_borrow_mut(|bus| {
            bus.has_trigger_events = !events.is_empty();
            bus.on_trigger_event = if events.is_empty() {
                None
            } else {
                Some(Rc::new(on_emit) as GuideSignalTrigger)
            };
            bus.trigger_events = events;
            bus.app = Some(app);
        });
    }

    /// safe to call from anywhere on the main thread, even inside an optimistic mutation
    pub fn emit(signal: RiseGuideSignal) {
        #[cfg(debug_assertions)]
        {
            GuideSignalInventory::note_emission(signal);
            debug_assert!(
                !signal.is_state(),
                "{} is a state in the canon; report it with GuideFacts::set",
                signal.raw_value()
            );
        }
        let (trigger, pending, app) = GUIDE_SIGNAL_BUS.with_borrow_mut(|bus| {
            *bus.emissions.entry(signal).or_default() += 1;
            let trigger = if bus.has_trigger_events && bus.trigger_events.contains(&signal) {
                bus.on_trigger_event.clone()
            } else {
                None
            };
            if !bus.has_waiters {
                return (trigger, Vec::new(), bus.app.clone());
            }
            let Some(pending) = bus.waiters.remove(&signal) else {
                return (trigger, Vec::new(), bus.app.clone());
            };
            bus.has_waiters = !bus.waiters.is_empty();
            for awaiter in &pending {
                bus.signals_by_awaiter.remove(&awaiter.object_identifier());
            }
            (trigger, pending, bus.app.clone())
        });
        let waiters = pending.len();
        if let Some(app) = app
            && (trigger.is_some() || waiters > 0)
        {
            app.spawn(async move |cx| {
                cx.update(|cx| {
                    if let Some(trigger) = trigger {
                        trigger(signal, cx);
                    }
                    GuideTrace::event(signal, waiters as i64, cx);
                });
            })
            .detach();
        }
        for awaiter in pending {
            awaiter.settle(true);
        }
    }

    /// returns false on timeout or cancel, which the runtime reports as step_timeout
    /// any number of waiters can hold the same signal and one emission frees all
    pub async fn wait_for(signal: RiseGuideSignal, timeout_ms: i64, cx: &AsyncApp) -> bool {
        #[cfg(debug_assertions)]
        GuideSignalInventory::verify_declarations_once();
        if timeout_ms <= 0 {
            return false;
        }

        let awaiter = GuideEventAwaiter::new();
        GUIDE_SIGNAL_BUS.with_borrow_mut(|bus| {
            bus.waiters.entry(signal).or_default().push(awaiter.clone());
            bus.signals_by_awaiter.insert(awaiter.object_identifier(), signal);
            bus.has_waiters = true;
            bus.app = Some(cx.clone());
        });

        GuideEventWait::run(awaiter, timeout_ms, Self::release, cx).await
    }

    /// how many times the signal was emitted so far
    /// a step compares two readings to see if its event fired while it looked for its anchor
    pub fn emission_count(signal: RiseGuideSignal) -> u64 {
        GUIDE_SIGNAL_BUS.with_borrow(|bus| bus.emissions.get(&signal).copied().unwrap_or(0))
    }

    pub async fn wait_for_default(signal: RiseGuideSignal, cx: &AsyncApp) -> bool {
        Self::wait_for(signal, RiseGuideCanon::TIMINGS_STEP_TIMEOUT_DEFAULT_MS, cx).await
    }

    fn release(awaiter: &GuideEventAwaiter) {
        GUIDE_SIGNAL_BUS.with_borrow_mut(|bus| {
            let Some(signal) = bus.signals_by_awaiter.remove(&awaiter.object_identifier()) else {
                return;
            };
            let Some(pending) = bus.waiters.get_mut(&signal) else {
                return;
            };
            pending.retain(|other| !other.ptr_eq(awaiter));
            if pending.is_empty() {
                bus.waiters.remove(&signal);
            }
            bus.has_waiters = !bus.waiters.is_empty();
        });
    }
}

/// signal to the file that must emit it
/// a signal with no emission site is worse than a missing one because it looks working
#[cfg(debug_assertions)]
pub enum GuideSignalInventory {}

#[cfg(debug_assertions)]
#[derive(Default)]
struct GuideSignalInventoryState {
    observed: HashSet<RiseGuideSignal>,
    did_verify: bool,
}

#[cfg(debug_assertions)]
thread_local! {
    static GUIDE_SIGNAL_INVENTORY: RefCell<GuideSignalInventoryState> = RefCell::default();
}

#[cfg(debug_assertions)]
impl GuideSignalInventory {
    pub fn emission_sites() -> &'static HashMap<RiseGuideSignal, &'static str> {
        use std::sync::LazyLock;
        static SITES: LazyLock<HashMap<RiseGuideSignal, &'static str>> = LazyLock::new(|| {
            use RiseGuideSignal as S;
            HashMap::from([
                (S::MessageSent, "modules/chat/pages/Chats/Chat/Chat.swift"),
                (
                    S::ReactionAdded,
                    "modules/chat/stores/message/message-interactions/message-reactions-interactions.swift",
                ),
                (S::PostCreated, "modules/post/stores/post-interactions/post-interactions.swift"),
                (S::StoryCreated, "modules/story/stores/story/story-services/story-services.swift"),
                (S::CommentCreated, "modules/comment/stores/comment-interactions/comment-interactions.swift"),
                (S::PlaylistCreated, "modules/music/stores/music-interactions/music-interactions.swift"),
                (S::TrackAddedToPlaylist, "modules/music/stores/music-interactions/music-interactions.swift"),
                (S::TrackPlayed, "modules/music/stores/music-interactions/music-interactions.swift"),
                (S::FollowCreated, "modules/user/stores/user/user-interactions/user-interactions.swift"),
                (S::SearchSubmitted, "modules/search/stores/search-interactions/search-interactions.swift"),
                (S::ProfileSaved, "modules/user/stores/user/user-interactions/user-interactions.swift"),
                (S::MusicEffectsSheetClosed, "modules/music/shared/utils/music/MusicPlayerState.swift"),
                (S::InviteLinkCopied, "modules/music/stores/music-interactions/music-interactions.swift"),
                (S::JoinRequestApproved, "modules/music/stores/music-interactions/music-interactions.swift"),
                (S::TopicCreated, "modules/chat/stores/chats/chat-interactions/chat-topics-interactions.swift"),
                (S::SectionCreated, "modules/chat/stores/chats/chat-interactions/chat-topics-interactions.swift"),
                (S::StageSpeakerPromoted, "modules/rtc/stores/rtc/rtc-interactions/rtc-interactions.swift"),
                (S::VoiceParticipantMuted, "modules/rtc/stores/rtc/rtc-interactions/rtc-interactions.swift"),
                (S::VoiceRoomLeft, "modules/rtc/stores/rtc/rtc-interactions/rtc-interactions.swift"),
                (S::MapFriendAdded, "modules/map/stores/map-interactions/map-interactions.swift"),
                (S::MapFriendRequestSent, "modules/map/stores/map-interactions/map-interactions.swift"),
                (S::MapLinesToggled, "modules/map/stores/map-interactions/map-interactions.swift"),
                (S::MapPlaceSaved, "modules/map/stores/map-interactions/map-place-interactions.swift"),
                (S::MapReactionBurst, "modules/map/stores/map-interactions/map-interactions.swift"),
                (S::MapReactionSent, "modules/map/stores/map-interactions/map-interactions.swift"),
                (S::MapSettingsChanged, "modules/map/stores/map-interactions/map-interactions.swift"),
                (S::MapTrackPeriodChanged, "modules/map/stores/map-interactions/map-interactions.swift"),
                (S::MapTracksErased, "modules/map/stores/map-interactions/map-interactions.swift"),
                (S::RoleSaved, "modules/chat/stores/chats/chat-actions/chat-roles-actions.swift"),
                (S::RoleReordered, "modules/chat/stores/chats/chat-actions/chat-roles-actions.swift"),
                (S::MemberRoleAssigned, "modules/chat/stores/chats/chat-actions/chat-roles-actions.swift"),
                (
                    S::MemberRightsSaved,
                    "modules/chat/pages/Chats/Chat/ChatSettings/ChatSettingsMembers/MemberRightsSheet.swift",
                ),
                (S::RoleMentionInserted, "modules/chat/pages/Chats/Chat/Chat.swift"),
                (S::UserBlockSet, "modules/user/stores/user/user-interactions/user-contacts-interactions.swift"),
                (S::ChatWallpaperSet, "modules/chat/stores/chats/chat-interactions/chat-interactions.swift"),
                (
                    S::ChatDiscussionLinked,
                    "modules/chat/stores/chats/chat-interactions/chat-settings-interactions.swift",
                ),
                (S::FolderSaved, "modules/chat/components/Folders/ChatFolderEditorStore.swift"),
                (
                    S::MessageMediaReplaced,
                    "modules/chat/stores/message/message-interactions/message-edit-interactions.swift",
                ),
                (
                    S::CloseFriendsSaved,
                    "modules/story/stores/story/story-interactions/story-close-friends-interactions.swift",
                ),
                (S::HighlightCreated, "modules/story/stores/story/story-interactions/story-interactions.swift"),
                (S::HighlightSaved, "modules/story/stores/story/story-interactions/story-interactions.swift"),
                (
                    S::StoriesArchived,
                    "modules/story/stores/story/story-interactions/story-profile-interactions.swift",
                ),
                (S::CoauthorAccepted, "modules/guides/shared/GuideSandboxPosts.swift"),
                (S::CoauthorGridChanged, "modules/guides/shared/GuideSandboxPosts.swift"),
                (S::CoauthorLeft, "modules/guides/shared/GuideSandboxPosts.swift"),
                (S::EnergyThresholdChanged, "Core/Energy/RiseEnergySaving.swift"),
                (S::EnergyFeatureToggled, "Core/Energy/RiseEnergySaving.swift"),
                (S::SessionTerminated, "modules/guides/shared/GuideSandboxSettings.swift"),
                (S::GoalAdded, "modules/user/stores/user/user-services/user-services.swift"),
                (S::GoalProgressChanged, "modules/user/stores/user/user-services/user-services.swift"),
                (S::PlanAdded, "modules/user/stores/user/user-services/user-services.swift"),
                (
                    S::BugReportSubmitted,
                    "modules/diagnostics/stores/diagnostics/diagnostics-interactions/bug-report-interactions.swift",
                ),
                (S::PersonNotificationsChanged, "modules/guides/shared/GuideSandboxSettings.swift"),
                (S::AchievementVisibilityChanged, "modules/guides/shared/GuideSandboxSettings.swift"),
                (
                    S::AchievementShareStarted,
                    "modules/user/stores/user-achievements/user-achievements-interactions/user-achievements-interactions.swift",
                ),
                (
                    S::ThemeAppearanceChanged,
                    "modules/user/stores/user/user-interactions/user-theme-settings-interactions.swift",
                ),
                (
                    S::ThemeColorChanged,
                    "modules/user/pages/Profile/Settings/CustomizationSettings/Settings/ColorPickerScreen.swift",
                ),
                (
                    S::ThemeReset,
                    "modules/user/pages/Profile/Settings/CustomizationSettings/CustomizationSettings.swift",
                ),
                (S::WallpaperGradientChanged, "modules/chat/components/Chat/Backdrop/ChatWallpaperStore.swift"),
                (S::WallpaperPaletteChanged, "modules/chat/components/Chat/Backdrop/ChatWallpaperStore.swift"),
                (
                    S::WallpaperLookChanged,
                    "modules/user/pages/Profile/Settings/CustomizationSettings/Settings/ChatWallpapersSettings/WallpaperLookEditorSheet.swift",
                ),
                (S::AppWallpaperAdded, "modules/guides/shared/GuideSandboxSettings.swift"),
                (S::AppWallpaperSet, "modules/guides/shared/GuideSandboxSettings.swift"),
                (S::ProfileBadgeChanged, "modules/user/stores/user/user-interactions/user-interactions.swift"),
                (S::ProfileBannerChanged, "modules/guides/shared/GuideSandboxSettings.swift"),
                (S::StickerPackCreated, "modules/guides/shared/GuideSandboxStickers.swift"),
                (S::StickerAdded, "modules/guides/shared/GuideSandboxStickers.swift"),
                (S::StickerFavouriteChanged, "modules/guides/shared/GuideSandboxStickers.swift"),
                (S::PlaylistMemberRightsSaved, "modules/guides/shared/GuideSandbox.swift"),
                (S::PlaylistMemberRemoved, "modules/guides/shared/GuideSandbox.swift"),
                (S::TrackAddedToProfile, "modules/music/stores/music-interactions/music-interactions.swift"),
                (S::ResumeSaved, "modules/guides/shared/GuideSandboxJobs.swift"),
                (S::VacancyApplied, "modules/guides/shared/GuideSandboxJobs.swift"),
                (S::VacancyPublished, "modules/guides/shared/GuideSandboxJobs.swift"),
                (S::BotCreated, "modules/guides/shared/GuideSandboxBots.swift"),
                (S::BotCommandsSaved, "modules/guides/shared/GuideSandboxBots.swift"),
                (S::BotWebhookSaved, "modules/guides/shared/GuideSandboxBots.swift"),
                (S::BotMiniAppCreated, "modules/guides/shared/GuideSandboxBots.swift"),
                (S::MapPhotosAdded, "modules/guides/shared/GuideSandboxMapPhotos.swift"),
                (S::MapGridSaved, "modules/guides/shared/GuideSandboxMapPhotos.swift"),
                (S::CallEnded, "modules/guides/shared/GuideSandboxCall.swift"),
            ])
        });
        &SITES
    }

    pub fn observed() -> HashSet<RiseGuideSignal> {
        GUIDE_SIGNAL_INVENTORY.with_borrow(|inventory| inventory.observed.clone())
    }

    pub fn note_emission(signal: RiseGuideSignal) {
        GUIDE_SIGNAL_INVENTORY.with_borrow_mut(|inventory| {
            inventory.observed.insert(signal);
        });
    }

    /// event signals canon defines and this build declares no emission site for
    /// states are excluded, unreported_states checks those
    pub fn undeclared() -> Vec<RiseGuideSignal> {
        let sites = Self::emission_sites();
        RiseGuideSignal::ALL
            .into_iter()
            .filter(|signal| !signal.is_state() && !sites.contains_key(signal))
            .collect()
    }

    pub fn never_observed() -> Vec<RiseGuideSignal> {
        GUIDE_SIGNAL_INVENTORY.with_borrow(|inventory| {
            RiseGuideSignal::ALL
                .into_iter()
                .filter(|signal| !inventory.observed.contains(signal))
                .collect()
        })
    }

    pub fn verify_declarations_once() {
        let first = GUIDE_SIGNAL_INVENTORY.with_borrow_mut(|inventory| {
            !std::mem::replace(&mut inventory.did_verify, true)
        });
        if !first {
            return;
        }
        let missing = Self::undeclared();
        debug_assert!(
            missing.is_empty(),
            "guide signals with no declared emission site: {}",
            Self::joined(&missing)
        );
    }

    pub fn summary() -> String {
        let pending = Self::never_observed();
        let undeclared = Self::undeclared();
        let observed = GUIDE_SIGNAL_INVENTORY.with_borrow(|inventory| inventory.observed.len());
        format!(
            "guide signals: {}/{} emitted this run\nnever emitted: {}\nundeclared: {}",
            observed,
            RiseGuideSignal::ALL.len(),
            if pending.is_empty() { "none".to_owned() } else { Self::joined(&pending) },
            if undeclared.is_empty() { "none".to_owned() } else { Self::joined(&undeclared) },
        )
    }

    fn joined(signals: &[RiseGuideSignal]) -> String {
        signals.iter().map(|signal| signal.raw_value()).collect::<Vec<_>>().join(", ")
    }
}
