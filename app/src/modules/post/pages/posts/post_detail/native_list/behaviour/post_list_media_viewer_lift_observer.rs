use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::{Rc, Weak};

use gpui::{App, Entity, Subscription};

use crate::core::media_viewer::media_viewer_presentation_state::MediaViewerPresentationState;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PostListMediaViewerLiftObserverSnapshot {
    pub is_presented: bool,
    pub hero_lift_id: Option<String>,
    pub bubble_cell_hidden: bool,
}

impl PostListMediaViewerLiftObserverSnapshot {
    pub fn idle() -> Self {
        Self::default()
    }

    pub fn lifted_registry_key(&self) -> Option<&str> {
        if self.is_presented && self.bubble_cell_hidden { self.hero_lift_id.as_deref() } else { None }
    }
}

pub type PostListMediaViewerLiftHandler = Rc<dyn Fn(bool, &mut App)>;
pub type PostListMediaViewerHeroTargetHandler = Rc<dyn Fn(&str, &mut App)>;

pub struct PostListMediaViewerLiftObserver {
    this: Weak<Self>,
    snapshot: RefCell<PostListMediaViewerLiftObserverSnapshot>,
    lift_observers: RefCell<HashMap<String, BTreeMap<u64, PostListMediaViewerLiftHandler>>>,
    hero_target_observers: RefCell<BTreeMap<u64, PostListMediaViewerHeroTargetHandler>>,
    next_observer_id: Cell<u64>,
    is_invalidated: Cell<bool>,
    subscription: RefCell<Option<Subscription>>,
}

impl PostListMediaViewerLiftObserver {
    /// pass the app state, never a private default
    pub fn new(state: &Entity<MediaViewerPresentationState>, cx: &mut App) -> Rc<Self> {
        let observer = Rc::new_cyclic(|this| Self {
            this: this.clone(),
            snapshot: RefCell::new(PostListMediaViewerLiftObserverSnapshot::idle()),
            lift_observers: RefCell::new(HashMap::new()),
            hero_target_observers: RefCell::new(BTreeMap::new()),
            next_observer_id: Cell::new(0),
            is_invalidated: Cell::new(false),
            subscription: RefCell::new(None),
        });
        *observer.snapshot.borrow_mut() = Self::read_snapshot(state, cx);
        let weak = observer.this.clone();
        let subscription = cx.observe(state, move |state, cx| {
            if let Some(observer) = weak.upgrade() {
                observer.state_did_change(&state, cx);
            }
        });
        *observer.subscription.borrow_mut() = Some(subscription);
        observer
    }

    pub fn snapshot(&self) -> PostListMediaViewerLiftObserverSnapshot {
        self.snapshot.borrow().clone()
    }

    /// whether registry_key is hidden under the viewer
    pub fn is_lifted(&self, registry_key: &str) -> bool {
        self.snapshot.borrow().lifted_registry_key() == Some(registry_key)
    }

    pub fn observe_lift(
        &self,
        registry_key: &str,
        handler: impl Fn(bool, &mut App) + 'static,
    ) -> PostListMediaViewerLiftObservation {
        let id = self.next_id();
        self.lift_observers
            .borrow_mut()
            .entry(registry_key.to_owned())
            .or_default()
            .insert(id, Rc::new(handler));
        PostListMediaViewerLiftObservation::new(self.this.clone(), id, Some(registry_key.to_owned()))
    }

    pub fn observe_hero_target(
        &self,
        handler: impl Fn(&str, &mut App) + 'static,
    ) -> PostListMediaViewerLiftObservation {
        let id = self.next_id();
        self.hero_target_observers.borrow_mut().insert(id, Rc::new(handler));
        PostListMediaViewerLiftObservation::new(self.this.clone(), id, None)
    }

    /// stops tracking and drops every handler, the hub calls it when its state is gone
    pub fn invalidate(&self) {
        self.is_invalidated.set(true);
        *self.subscription.borrow_mut() = None;
        self.lift_observers.borrow_mut().clear();
        self.hero_target_observers.borrow_mut().clear();
    }

    fn next_id(&self) -> u64 {
        let id = self.next_observer_id.get().wrapping_add(1);
        self.next_observer_id.set(id);
        id
    }

    fn remove_observer(&self, id: u64, registry_key: Option<&str>) {
        let Some(registry_key) = registry_key else {
            self.hero_target_observers.borrow_mut().remove(&id);
            return;
        };
        let mut lift_observers = self.lift_observers.borrow_mut();
        if let Some(observers) = lift_observers.get_mut(registry_key) {
            observers.remove(&id);
            if observers.is_empty() {
                lift_observers.remove(registry_key);
            }
        }
    }

    fn read_snapshot(state: &Entity<MediaViewerPresentationState>, cx: &App) -> PostListMediaViewerLiftObserverSnapshot {
        let state = state.read(cx);
        PostListMediaViewerLiftObserverSnapshot {
            is_presented: state.is_presented(),
            hero_lift_id: state.hero_lift_id().map(str::to_owned),
            bubble_cell_hidden: state.bubble_cell_hidden(),
        }
    }

    fn state_did_change(&self, state: &Entity<MediaViewerPresentationState>, cx: &mut App) {
        if self.is_invalidated.get() {
            return;
        }
        let current = Self::read_snapshot(state, cx);
        let previous = self.snapshot.replace(current.clone());
        if current == previous {
            return;
        }

        if let Some(hero) = current.hero_lift_id.as_deref()
            && current.hero_lift_id != previous.hero_lift_id
        {
            let ids: Vec<u64> = self.hero_target_observers.borrow().keys().copied().collect();
            for id in ids {
                if !self.is_current(&current) {
                    return;
                }
                let handler = self.hero_target_observers.borrow().get(&id).cloned();
                if let Some(handler) = handler {
                    handler(hero, cx);
                }
            }
        }
        if previous.lifted_registry_key() == current.lifted_registry_key() {
            return;
        }
        if let Some(revealed) = previous.lifted_registry_key() {
            self.notify_lift(revealed, false, &current, cx);
        }
        if let Some(hidden) = current.lifted_registry_key() {
            self.notify_lift(hidden, true, &current, cx);
        }
    }

    fn is_current(&self, current: &PostListMediaViewerLiftObserverSnapshot) -> bool {
        !self.is_invalidated.get() && *self.snapshot.borrow() == *current
    }

    fn notify_lift(
        &self,
        registry_key: &str,
        is_hidden: bool,
        current: &PostListMediaViewerLiftObserverSnapshot,
        cx: &mut App,
    ) {
        let ids: Vec<u64> = match self.lift_observers.borrow().get(registry_key) {
            Some(observers) => observers.keys().copied().collect(),
            None => return,
        };
        for id in ids {
            if !self.is_current(current) {
                return;
            }
            let handler = self.lift_observers.borrow().get(registry_key).and_then(|observers| observers.get(&id).cloned());
            if let Some(handler) = handler {
                handler(is_hidden, cx);
            }
        }
    }
}

/// token of one lift observer registration
/// removes it on cancel or drop
pub struct PostListMediaViewerLiftObservation {
    owner: RefCell<Weak<PostListMediaViewerLiftObserver>>,
    id: u64,
    registry_key: Option<String>,
}

impl PostListMediaViewerLiftObservation {
    fn new(owner: Weak<PostListMediaViewerLiftObserver>, id: u64, registry_key: Option<String>) -> Self {
        Self { owner: RefCell::new(owner), id, registry_key }
    }

    pub fn cancel(&self) {
        if let Some(owner) = self.owner.replace(Weak::new()).upgrade() {
            owner.remove_observer(self.id, self.registry_key.as_deref());
        }
    }
}

impl Drop for PostListMediaViewerLiftObservation {
    fn drop(&mut self) {
        self.cancel();
    }
}
