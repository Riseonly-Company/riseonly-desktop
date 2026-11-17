use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, AsyncApp};
use rise_canon::RiseGuideSignal;

use crate::core::guides::guide_signal_bus::{GuideEventAwaiter, GuideEventWait, GuideSignalBus};
use crate::core::guides::guide_trace::GuideTrace;

type GuideFactTrigger = Rc<dyn Fn(RiseGuideSignal, &mut App)>;

#[derive(Default)]
struct GuideFactsState {
    truths: HashSet<RiseGuideSignal>,
    waiters: HashMap<RiseGuideSignal, Vec<GuideEventAwaiter>>,
    trigger_facts: HashSet<RiseGuideSignal>,
    on_trigger_fact: Option<GuideFactTrigger>,
    #[cfg(debug_assertions)]
    ever_reported: HashSet<RiseGuideSignal>,
}

thread_local! {
    static GUIDE_FACTS: RefCell<GuideFactsState> = RefCell::default();
}

/// what is true about the app right now, for conditions a guide waits on
/// a step waits on a fact not a gesture, asked at once and again on every change
pub enum GuideFacts {}

impl GuideFacts {
    pub fn install_triggers(
        facts: HashSet<RiseGuideSignal>,
        on_became_true: impl Fn(RiseGuideSignal, &mut App) + 'static,
    ) {
        GUIDE_FACTS.with_borrow_mut(|state| {
            state.on_trigger_fact = if facts.is_empty() {
                None
            } else {
                Some(Rc::new(on_became_true) as GuideFactTrigger)
            };
            state.trigger_facts = facts;
        });
    }

    /// idempotent, the common call re-sets a value it already holds
    /// call it where the state is decided and with false on the way out
    pub fn set(fact: RiseGuideSignal, value: bool, cx: &mut App) {
        Self::assert_is_state(fact);
        let changed = GUIDE_FACTS.with_borrow_mut(|state| {
            let was = state.truths.contains(&fact);
            if was == value {
                return false;
            }
            if value {
                state.truths.insert(fact);
            } else {
                state.truths.remove(&fact);
            }
            true
        });
        if !changed {
            return;
        }
        GuideTrace::fact(fact, value, cx);
        if !value {
            return;
        }
        Self::release(fact);
        let trigger = GUIDE_FACTS.with_borrow(|state| {
            if state.trigger_facts.contains(&fact) {
                state.on_trigger_fact.clone()
            } else {
                None
            }
        });
        if let Some(trigger) = trigger {
            trigger(fact, cx);
        }
    }

    pub fn is_true(fact: RiseGuideSignal) -> bool {
        GUIDE_FACTS.with_borrow(|state| state.truths.contains(&fact))
    }

    /// returns at once when the fact already holds
    /// timeout exists only because the caller has a deadline type
    /// the runtime passes longer than any session
    pub async fn wait_until_true(fact: RiseGuideSignal, timeout_ms: i64, cx: &AsyncApp) -> bool {
        Self::assert_is_state(fact);
        if Self::is_true(fact) {
            cx.update(|cx| GuideTrace::condition_already_true(fact, cx));
            return true;
        }
        let awaiter = GuideEventAwaiter::new();
        GUIDE_FACTS.with_borrow_mut(|state| {
            state.waiters.entry(fact).or_default().push(awaiter.clone());
        });
        GuideEventWait::run(awaiter, timeout_ms, Self::release_awaiter, cx).await
    }

    fn release(fact: RiseGuideSignal) {
        let pending = GUIDE_FACTS.with_borrow_mut(|state| state.waiters.remove(&fact));
        let Some(pending) = pending else {
            return;
        };
        for awaiter in pending {
            awaiter.settle(true);
        }
    }

    fn release_awaiter(awaiter: &GuideEventAwaiter) {
        GUIDE_FACTS.with_borrow_mut(|state| {
            state.waiters.retain(|_, list| {
                list.retain(|other| !other.ptr_eq(awaiter));
                !list.is_empty()
            });
        });
    }

    fn assert_is_state(fact: RiseGuideSignal) {
        debug_assert!(
            fact.is_state(),
            "{} is an event in the canon; report it with GuideSignalBus::emit",
            fact.raw_value()
        );
    }

    /// which facts this build reports
    /// the inventory check compares it with canon states
    /// a state nobody writes is a step that waits forever
    #[cfg(debug_assertions)]
    pub fn ever_reported() -> HashSet<RiseGuideSignal> {
        GUIDE_FACTS.with_borrow(|state| state.ever_reported.clone())
    }

    #[cfg(debug_assertions)]
    pub fn note_reported(fact: RiseGuideSignal) {
        GUIDE_FACTS.with_borrow_mut(|state| {
            state.ever_reported.insert(fact);
        });
    }

    #[cfg(debug_assertions)]
    pub fn unreported_states() -> Vec<RiseGuideSignal> {
        GUIDE_FACTS.with_borrow(|state| {
            RiseGuideSignal::ALL
                .into_iter()
                .filter(|signal| signal.is_state() && !state.ever_reported.contains(signal))
                .collect()
        })
    }
}

/// single entry point for waiting on a named signal so the runtime never branches on is_state itself
/// canon decided that already
pub enum GuideConditions {}

impl GuideConditions {
    pub async fn wait(signal: RiseGuideSignal, timeout_ms: i64, cx: &AsyncApp) -> bool {
        if signal.is_state() {
            return GuideFacts::wait_until_true(signal, timeout_ms, cx).await;
        }
        GuideSignalBus::wait_for(signal, timeout_ms, cx).await
    }
}
