use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::{Rc, Weak};

use gpui::{
    AnyElement, App, AppContext, Context, Entity, IntoElement, ParentElement, RenderOnce,
    Styled, WeakEntity, Window, div,
};
use rise_ui::swiftui::environment::{EnvKey, env};
use rise_ui::window_screen_frame_reader::WindowScreenFrameReader;

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementKey, SharedElementPart, SharedElementSurface,
};

const MINIMUM_PURGE_THRESHOLD: usize = 64;

pub struct SharedElementLiftBoard {
    slots: HashMap<SharedElementKey, WeakEntity<SharedElementLiftSlot>>,
    lifted: HashMap<SharedElementKey, Entity<SharedElementLiftSlot>>,
    purge_threshold: usize,
}

impl SharedElementLiftBoard {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { slots: HashMap::new(), lifted: HashMap::new(), purge_threshold: MINIMUM_PURGE_THRESHOLD }
    }

    pub fn slot(&mut self, key: &SharedElementKey, cx: &mut Context<Self>) -> Entity<SharedElementLiftSlot> {
        if let Some(slot) = self.lifted.get(key) {
            return slot.clone();
        }
        if let Some(slot) = self.slots.get(key).and_then(WeakEntity::upgrade) {
            return slot;
        }
        let slot = cx.new(|_| SharedElementLiftSlot::new());
        self.slots.insert(key.clone(), slot.downgrade());
        self.purge_if_needed();
        slot
    }

    pub fn lift(&mut self, keys: HashSet<SharedElementKey>, epoch: u64, cx: &mut Context<Self>) {
        for key in keys {
            let slot = self.slot(&key, cx);
            self.lifted.insert(key, slot.clone());
            slot.update(cx, |slot, cx| {
                slot.epoch = epoch;
                slot.set_hidden(true, cx);
            });
        }
    }

    /// only for a landing mismatch, reveals one key of epoch before the rest land
    pub fn unlift(&mut self, key: &SharedElementKey, epoch: u64, cx: &mut Context<Self>) {
        let Some(slot) = self.lifted.get(key).cloned() else { return };
        if slot.read(cx).epoch != epoch {
            return;
        }
        self.lifted.remove(key);
        slot.update(cx, |slot, cx| slot.set_hidden(false, cx));
    }

    /// clears only slots lifted by this epoch, so a late landing never reveals a newer flight
    pub fn land(&mut self, epoch: u64, cx: &mut Context<Self>) {
        if self.lifted.is_empty() {
            return;
        }
        let landing: Vec<SharedElementKey> = self
            .lifted
            .iter()
            .filter(|(_, slot)| slot.read(cx).epoch == epoch)
            .map(|(key, _)| key.clone())
            .collect();
        for key in landing {
            if let Some(slot) = self.lifted.remove(&key) {
                slot.update(cx, |slot, cx| slot.set_hidden(false, cx));
            }
        }
    }

    pub fn clear_all(&mut self, cx: &mut Context<Self>) {
        let all: Vec<Entity<SharedElementLiftSlot>> = self.lifted.drain().map(|(_, slot)| slot).collect();
        for slot in all {
            slot.update(cx, |slot, cx| slot.set_hidden(false, cx));
        }
    }

    fn purge_if_needed(&mut self) {
        if self.slots.len() < self.purge_threshold {
            return;
        }
        self.slots.retain(|_, slot| slot.upgrade().is_some());
        self.purge_threshold = MINIMUM_PURGE_THRESHOLD.max(self.slots.len() * 2);
    }
}

pub type SharedElementLiftObserver = Rc<dyn Fn(bool, &mut App)>;

struct SharedElementLiftObserverEntry {
    observer: SharedElementLiftObserver,
    token: Weak<()>,
}

pub struct SharedElementLiftSlot {
    is_hidden: bool,
    epoch: u64,
    observers: BTreeMap<u64, SharedElementLiftObserverEntry>,
    next_observer_id: u64,
}

impl SharedElementLiftSlot {
    fn new() -> Self {
        Self { is_hidden: false, epoch: 0, observers: BTreeMap::new(), next_observer_id: 0 }
    }

    pub fn is_hidden(&self) -> bool {
        self.is_hidden
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// observer runs right after each real change of is_hidden, once the write has returned
    /// not called for the current value so read is_hidden when attaching
    /// slot keeps it until the returned token is cancelled
    pub fn add_observer(
        &mut self,
        observer: impl Fn(bool, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> SharedElementLiftObservation {
        self.next_observer_id = self.next_observer_id.wrapping_add(1);
        let token = Rc::new(());
        self.observers.insert(
            self.next_observer_id,
            SharedElementLiftObserverEntry { observer: Rc::new(observer), token: Rc::downgrade(&token) },
        );
        SharedElementLiftObservation { slot: Some(cx.entity().downgrade()), id: self.next_observer_id, _token: token }
    }

    fn set_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
        if self.is_hidden == hidden {
            return;
        }
        self.is_hidden = hidden;
        cx.notify();
        self.observers.retain(|_, entry| entry.token.strong_count() > 0);
        if self.observers.is_empty() {
            return;
        }
        let slot = cx.entity();
        cx.defer(move |cx| Self::deliver(&slot, hidden, cx));
    }

    fn deliver(slot: &Entity<Self>, hidden: bool, cx: &mut App) {
        let ids: Vec<u64> = slot.read(cx).observers.keys().copied().collect();
        for id in ids {
            if slot.read(cx).is_hidden != hidden {
                return;
            }
            let observer = slot
                .read(cx)
                .observers
                .get(&id)
                .filter(|entry| entry.token.strong_count() > 0)
                .map(|entry| entry.observer.clone());
            if let Some(observer) = observer {
                observer(hidden, cx);
            }
        }
    }

    fn remove_observer(&mut self, id: u64) {
        self.observers.remove(&id);
    }
}

pub struct SharedElementLiftObservation {
    slot: Option<WeakEntity<SharedElementLiftSlot>>,
    id: u64,
    _token: Rc<()>,
}

impl SharedElementLiftObservation {
    pub fn cancel(&mut self, cx: &mut App) {
        if let Some(slot) = self.slot.take().and_then(|slot| slot.upgrade()) {
            let id = self.id;
            slot.update(cx, |slot, _| slot.remove_observer(id));
        }
    }
}

/// environment key for the shared element surface
pub struct SharedElementSurfaceKey;

impl EnvKey for SharedElementSurfaceKey {
    /// none outside a shared element surface, where anchors cost nothing
    type Value = Option<SharedElementSurface>;

    fn default_value() -> Self::Value {
        None
    }
}

#[derive(IntoElement)]
pub struct SharedElementAnchorModifier {
    pub item_id: String,
    pub part: SharedElementPart,
    content: AnyElement,
}

impl RenderOnce for SharedElementAnchorModifier {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(surface) = env::<SharedElementSurfaceKey>(cx) else {
            return self.content;
        };
        let key = SharedElementKey::new(surface, self.item_id, self.part);
        let slot = SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.slot(&key, cx));
        SharedElementLiftedAnchorModifier { key, slot, content: self.content }.into_any_element()
    }
}

#[derive(IntoElement)]
struct SharedElementLiftedAnchorModifier {
    key: SharedElementKey,
    slot: Entity<SharedElementLiftSlot>,
    content: AnyElement,
}

impl RenderOnce for SharedElementLiftedAnchorModifier {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_hidden = self.slot.read(cx).is_hidden();
        div()
            .relative()
            .opacity(if is_hidden { 0.0 } else { 1.0 })
            .child(self.content)
            .child(WindowScreenFrameReader::new(self.key.frame_registry_key()))
    }
}

#[derive(IntoElement)]
pub struct SharedElementLiftModifier {
    pub item_id: String,
    pub part: SharedElementPart,
    content: AnyElement,
}

impl RenderOnce for SharedElementLiftModifier {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(surface) = env::<SharedElementSurfaceKey>(cx) else {
            return self.content;
        };
        let key = SharedElementKey::new(surface, self.item_id, self.part);
        let slot = SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.slot(&key, cx));
        SharedElementLiftOnlyModifier { slot, content: self.content }.into_any_element()
    }
}

#[derive(IntoElement)]
struct SharedElementLiftOnlyModifier {
    slot: Entity<SharedElementLiftSlot>,
    content: AnyElement,
}

impl RenderOnce for SharedElementLiftOnlyModifier {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_hidden = self.slot.read(cx).is_hidden();
        div().opacity(if is_hidden { 0.0 } else { 1.0 }).child(self.content)
    }
}

pub trait SharedElementLiftModifiers: IntoElement + Sized {
    fn shared_element_anchor(self, item_id: impl Into<String>, part: SharedElementPart) -> SharedElementAnchorModifier {
        SharedElementAnchorModifier { item_id: item_id.into(), part, content: self.into_any_element() }
    }

    fn shared_element_lift(self, item_id: impl Into<String>, part: SharedElementPart) -> SharedElementLiftModifier {
        SharedElementLiftModifier { item_id: item_id.into(), part, content: self.into_any_element() }
    }
}

impl<E: IntoElement> SharedElementLiftModifiers for E {}
