use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gpui::{App, Bounds, ElementId, IntoElement, Pixels, RenderOnce, Styled, Window, canvas};

/// compat key for the tab bar menu, which owns its own gesture and frame consumer
/// hold overlay uses a live registry and publishes no shared frames
pub struct HoldContextMenuActionFramesKey;

impl HoldContextMenuActionFramesKey {
    pub fn default_value() -> HashMap<String, Bounds<Pixels>> {
        HashMap::new()
    }

    pub fn reduce(value: &mut HashMap<String, Bounds<Pixels>>, next_value: HashMap<String, Bounds<Pixels>>) {
        value.extend(next_value);
    }
}

/// row rects in the action list own space
pub struct HoldContextMenuRowLocalFramesKey;

impl HoldContextMenuRowLocalFramesKey {
    pub fn default_value() -> HashMap<String, Bounds<Pixels>> {
        HashMap::new()
    }

    pub fn reduce(value: &mut HashMap<String, Bounds<Pixels>>, next_value: HashMap<String, Bounds<Pixels>>) {
        value.extend(next_value);
    }
}

fn next_serial() -> u64 {
    thread_local! {
        static SERIAL: Cell<u64> = const { Cell::new(0) };
    }
    SERIAL.with(|serial| {
        let next = serial.get() + 1;
        serial.set(next);
        next
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HoldContextMenuHitTargetsKey {
    Action(String),
    Emoji(String),
}

struct HoldContextMenuHitTargetsRecord {
    serial: u64,
    frame: Bounds<Pixels>,
}

#[derive(Default)]
struct HoldContextMenuHitTargetsInner {
    targets: HashMap<HoldContextMenuHitTargetsKey, HoldContextMenuHitTargetsRecord>,
}

thread_local! {
    static LIVE: RefCell<Weak<RefCell<HoldContextMenuHitTargetsInner>>> = RefCell::new(Weak::new());
}

/// one per open menu, clones share the registry
#[derive(Clone, Default)]
pub struct HoldContextMenuHitTargets {
    inner: Rc<RefCell<HoldContextMenuHitTargetsInner>>,
}

impl HoldContextMenuHitTargets {
    pub fn new() -> Self {
        Self::default()
    }

    fn set(&self, key: HoldContextMenuHitTargetsKey, view: &TargetView, frame: Bounds<Pixels>) {
        self.inner
            .borrow_mut()
            .targets
            .insert(key, HoldContextMenuHitTargetsRecord { serial: view.serial, frame });
        LIVE.with(|live| *live.borrow_mut() = Rc::downgrade(&self.inner));
    }

    /// action panel on screen in window coordinates when rect lies on it
    /// a guide step on a menu row keeps the whole panel clear
    pub fn lifted_actions_frame(containing_rect: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
        let live = LIVE.with(|live| live.borrow().upgrade())?;
        let live = live.borrow();
        let mut rows = live.targets.iter().filter_map(|(key, record)| match key {
            HoldContextMenuHitTargetsKey::Action(_) if record.frame.size.width > Pixels::ZERO
                && record.frame.size.height > Pixels::ZERO =>
            {
                Some(record.frame)
            }
            _ => None,
        });
        let first = rows.next()?;
        let panel = rows.fold(first, |panel, row| panel.union(&row));
        panel.contains(&containing_rect.center()).then_some(panel)
    }

    pub fn actions_frame_beside(rect: Bounds<Pixels>, gap: Pixels) -> Option<Bounds<Pixels>> {
        Self::live_actions_panel().filter(|panel| panel.dilate(gap).intersects(&rect))
    }

    /// action panel of the last mounted menu as painted, none with no menu on screen
    pub fn live_actions_panel() -> Option<Bounds<Pixels>> {
        let live = LIVE.with(|live| live.borrow().upgrade())?;
        let live = live.borrow();
        let mut rows = live.targets.iter().filter_map(|(key, record)| match key {
            HoldContextMenuHitTargetsKey::Action(_) if record.frame.size.width > Pixels::ZERO
                && record.frame.size.height > Pixels::ZERO =>
            {
                Some(record.frame)
            }
            _ => None,
        });
        let first = rows.next()?;
        Some(rows.fold(first, |panel, row| panel.union(&row)))
    }

    fn remove(&self, key: &HoldContextMenuHitTargetsKey, view: &TargetView) {
        let mut inner = self.inner.borrow_mut();
        if inner.targets.get(key).is_some_and(|record| record.serial == view.serial) {
            inner.targets.remove(key);
        }
    }

    /// frame of the target in window coordinates as painted last
    /// read at press time, publishing into state would feed every animation frame into the overlay render
    pub fn frame(&self, for_key: &HoldContextMenuHitTargetsKey) -> Option<Bounds<Pixels>> {
        let inner = self.inner.borrow();
        let record = inner.targets.get(for_key)?;
        (record.frame.size.width > Pixels::ZERO && record.frame.size.height > Pixels::ZERO).then_some(record.frame)
    }
}

/// registered while painted and removed once gpui drops it
struct TargetView {
    serial: u64,
    registry: Weak<RefCell<HoldContextMenuHitTargetsInner>>,
    key: HoldContextMenuHitTargetsKey,
}

impl TargetView {
    fn new(registry: &HoldContextMenuHitTargets, key: HoldContextMenuHitTargetsKey) -> Self {
        Self { serial: next_serial(), registry: Rc::downgrade(&registry.inner), key }
    }

    /// a new registry or key moves the registration over
    fn update(&mut self, registry: &HoldContextMenuHitTargets, key: &HoldContextMenuHitTargetsKey) {
        if !std::ptr::eq(self.registry.as_ptr(), Rc::as_ptr(&registry.inner)) || &self.key != key {
            self.unregister();
            *self = Self::new(registry, key.clone());
        }
    }

    fn did_move_to_window(&self, frame: Option<Bounds<Pixels>>) {
        let Some(inner) = self.registry.upgrade() else { return };
        let registry = HoldContextMenuHitTargets { inner };
        match frame {
            Some(frame) => registry.set(self.key.clone(), self, frame),
            None => registry.remove(&self.key, self),
        }
    }

    fn unregister(&self) {
        let Some(inner) = self.registry.upgrade() else { return };
        HoldContextMenuHitTargets { inner }.remove(&self.key, self);
    }
}

impl Drop for TargetView {
    fn drop(&mut self) {
        self.unregister();
    }
}

/// transparent layer over its parent that registers the parent painted frame
/// no hitbox so it takes no clicks
#[derive(IntoElement)]
pub struct HoldContextMenuHitTarget {
    registry: HoldContextMenuHitTargets,
    key: HoldContextMenuHitTargetsKey,
}

impl HoldContextMenuHitTarget {
    pub fn new(registry: HoldContextMenuHitTargets, key: HoldContextMenuHitTargetsKey) -> Self {
        Self { registry, key }
    }
}

impl RenderOnce for HoldContextMenuHitTarget {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let id = ElementId::Name(
            match &self.key {
                HoldContextMenuHitTargetsKey::Action(id) => format!("hold-hit-action-{id}"),
                HoldContextMenuHitTargetsKey::Emoji(id) => format!("hold-hit-emoji-{id}"),
            }
            .into(),
        );
        canvas(
            move |bounds, window, _cx| {
                window.with_global_id(id, |global_id, window| {
                    window.with_element_state::<TargetView, _>(global_id, |view, _| {
                        let mut view =
                            view.unwrap_or_else(|| TargetView::new(&self.registry, self.key.clone()));
                        view.update(&self.registry, &self.key);
                        view.did_move_to_window(Some(bounds));
                        ((), view)
                    })
                })
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
}
