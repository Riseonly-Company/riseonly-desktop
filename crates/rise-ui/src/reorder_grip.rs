use std::cell::Cell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

use gpui::{
    App, Context, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement, RenderOnce, Styled, Window,
    div,
};
use rise_core::foundation::foundation_date::RiseClock;

use crate::swiftui::animated::{Animated, MotionTime};
use crate::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use crate::swiftui::motion::spring;

pub type ReorderGripOnChange = Rc<dyn Fn(f32, &mut Window, &mut App)>;
pub type ReorderGripOnEnd = Rc<dyn Fn(&mut Window, &mut App)>;

/// reorder grip of a settings row, only marks where a drag may start
/// on_change gets the vertical travel since the press began
#[derive(IntoElement)]
pub struct ReorderGripHandle {
    id: ElementId,
    pub on_change: ReorderGripOnChange,
    pub on_end: ReorderGripOnEnd,
}

impl ReorderGripHandle {
    pub fn new(id: impl Into<ElementId>, on_change: ReorderGripOnChange, on_end: ReorderGripOnEnd) -> Self {
        Self {
            id: id.into(),
            on_change,
            on_end,
        }
    }
}

/// whether the press has traveled far enough to lift the row
#[derive(Default)]
struct ReorderGripDriver {
    moved: Rc<Cell<bool>>,
}

impl ReorderGripDriver {
    const LIFT_THRESHOLD: f32 = 2.0;
}

impl RenderOnce for ReorderGripHandle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let driver = window.use_keyed_state(
            ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "reorder-grip-driver".into()),
            cx,
            |_: &mut Window, _: &mut Context<ReorderGripDriver>| ReorderGripDriver::default(),
        );
        let moved = driver.read(cx).moved.clone();
        let changed_moved = moved.clone();
        let on_change = self.on_change.clone();
        let on_end = self.on_end.clone();
        let gesture = DragGesture::new(0.0)
            .on_changed(move |value: &DragGestureValue, window: &mut Window, cx: &mut App| {
                let translation = f32::from(value.translation.height);
                if !changed_moved.get() && translation.abs() < ReorderGripDriver::LIFT_THRESHOLD {
                    return;
                }
                changed_moved.set(true);
                on_change(translation, window, cx);
            })
            .on_ended(move |_: &DragGestureValue, window: &mut Window, cx: &mut App| {
                if moved.replace(false) {
                    on_end(window, cx);
                }
            });
        div()
            .id(self.id.clone())
            .size_full()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .gesture(self.id, gesture)
            .size_full()
    }
}

/// drag to reorder in a laid out list, shared by every list that reorders by hand
/// the dragged item follows the pointer, a displaced one springs from where it stood into its new slot
/// a drop glides the item home and the lift fades, frames run only while something moves
pub struct ReorderMotion<K> {
    dragging: Option<K>,
    order: Vec<K>,
    translation: f32,
    accumulated: f32,
    dropped: Option<K>,
    drop: Animated<f32>,
    lifted: Option<K>,
    lift: Animated<f32>,
    shifts: HashMap<K, Animated<f32>>,
    landed: Vec<K>,
}

impl<K: Clone + Eq + Hash> Default for ReorderMotion<K> {
    fn default() -> Self {
        Self {
            dragging: None,
            order: Vec::new(),
            translation: 0.0,
            accumulated: 0.0,
            dropped: None,
            drop: Animated::new(0.0),
            lifted: None,
            lift: Animated::new(0.0),
            shifts: HashMap::new(),
            landed: Vec::new(),
        }
    }
}

impl<K: Clone + Eq + Hash> ReorderMotion<K> {
    pub const SWAP_THRESHOLD: f32 = 0.5;
    const SWAP_SPRING: (f64, f64) = (0.28, 0.86);
    const DROP_SPRING: (f64, f64) = (0.3, 0.86);
    const LIFT_SPRING: (f64, f64) = (0.22, 0.9);

    pub fn new() -> Self {
        Self::default()
    }

    pub fn dragging(&self) -> Option<&K> {
        self.dragging.as_ref()
    }

    /// order to lay out, the working one while a drag runs
    pub fn shown_order(&self, base: &[K]) -> Vec<K> {
        if self.dragging.is_some() { self.order.clone() } else { base.to_vec() }
    }

    /// pointer travel since the press, stride is the distance from an item origin to the next one
    /// true when an item changed its slot
    pub fn drag(
        &mut self,
        key: &K,
        translation: f32,
        base: &[K],
        stride: impl Fn(&K) -> f32,
        now: MotionTime,
        reduce_motion: bool,
    ) -> bool {
        if !self.begin(key, base, now, reduce_motion) {
            return false;
        }
        self.translation = translation;
        let dragged_stride = stride(key);
        let mut swapped = false;
        loop {
            let Some(index) = self.order.iter().position(|candidate| candidate == key) else {
                break;
            };
            let offset = self.translation - self.accumulated;
            let neighbour = if offset > 0.0 {
                self.order.get(index + 1).cloned().map(|next| (index + 1, next, 1.0))
            } else if offset < 0.0 && index > 0 {
                self.order.get(index - 1).cloned().map(|previous| (index - 1, previous, -1.0))
            } else {
                None
            };
            let Some((neighbour_index, neighbour, direction)) = neighbour else {
                break;
            };
            let neighbour_stride = stride(&neighbour);
            if offset.abs() <= neighbour_stride * Self::SWAP_THRESHOLD {
                break;
            }
            self.order.swap(index, neighbour_index);
            self.accumulated += neighbour_stride * direction;
            self.spring_from(&neighbour, dragged_stride * direction, now, reduce_motion);
            swapped = true;
        }
        swapped
    }

    pub fn drag_slots(
        &mut self,
        key: &K,
        translation: f32,
        base: &[K],
        slot: impl Fn(usize) -> f32,
        now: MotionTime,
        reduce_motion: bool,
    ) -> bool {
        if !self.begin(key, base, now, reduce_motion) {
            return false;
        }
        self.translation = translation;
        let mut swapped = false;
        loop {
            let Some(index) = self.order.iter().position(|candidate| candidate == key) else {
                break;
            };
            let offset = self.translation - self.accumulated;
            let target = if offset > 0.0 && index + 1 < self.order.len() {
                index + 1
            } else if offset < 0.0 && index > 0 {
                index - 1
            } else {
                break;
            };
            let distance = slot(target) - slot(index);
            if offset.abs() <= distance.abs() * Self::SWAP_THRESHOLD {
                break;
            }
            let neighbour = self.order[target].clone();
            self.order.swap(index, target);
            self.accumulated += distance;
            self.spring_from(&neighbour, distance, now, reduce_motion);
            swapped = true;
        }
        swapped
    }

    fn begin(&mut self, key: &K, base: &[K], now: MotionTime, reduce_motion: bool) -> bool {
        if self.dragging.as_ref() == Some(key) {
            return true;
        }
        if !base.contains(key) {
            return false;
        }
        self.dragging = Some(key.clone());
        self.order = base.to_vec();
        self.accumulated = 0.0;
        self.dropped = None;
        self.drop.jump(0.0);
        self.lifted = Some(key.clone());
        self.lift.set(1.0, spring(Self::LIFT_SPRING.0, Self::LIFT_SPRING.1), now, reduce_motion);
        true
    }

    /// the order the drop lands, none when no drag ran
    pub fn end(&mut self, now: MotionTime, reduce_motion: bool) -> Option<Vec<K>> {
        let key = self.dragging.take()?;
        let offset = self.translation - self.accumulated;
        self.drop.jump(offset);
        self.drop.set(0.0, spring(Self::DROP_SPRING.0, Self::DROP_SPRING.1), now, reduce_motion);
        self.dropped = Some(key);
        self.lift.set(0.0, spring(Self::LIFT_SPRING.0, Self::LIFT_SPRING.1), now, reduce_motion);
        self.translation = 0.0;
        self.accumulated = 0.0;
        self.shifts.retain(|_, shift| shift.is_running(now));
        let order = std::mem::take(&mut self.order);
        self.landed = order.clone();
        Some(order)
    }

    /// the list now lays out the order the drop handed back, offsets go on from the new slots
    pub fn settle(&mut self) {
        self.landed.clear();
    }

    /// offset of key from the slot a list that keeps its own order until the drop lands gives it
    pub fn offset_from_base_at(&self, key: &K, base: &[K], stride: impl Fn(&K) -> f32, now: MotionTime) -> f32 {
        let order = if self.dragging.is_some() { &self.order } else { &self.landed };
        let origin = |list: &[K]| -> Option<f32> {
            let index = list.iter().position(|candidate| candidate == key)?;
            Some(list[..index].iter().map(&stride).sum())
        };
        let delta = match (origin(order), origin(base)) {
            (Some(laid), Some(kept)) => laid - kept,
            _ => 0.0,
        };
        delta + self.offset_at(key, now)
    }

    /// offset_from_base_at for a render, asks for the next frame while anything still moves
    pub fn offset_from_base(&self, key: &K, base: &[K], stride: impl Fn(&K) -> f32, window: &Window) -> f32 {
        let now = RiseClock::monotonic();
        if self.springs(now) {
            window.request_animation_frame();
        }
        self.offset_from_base_at(key, base, stride, now)
    }

    pub fn offset_from_slots_at(&self, key: &K, base: &[K], slot: impl Fn(usize) -> f32, now: MotionTime) -> f32 {
        let order = if self.dragging.is_some() { &self.order } else { &self.landed };
        let delta = match (
            order.iter().position(|candidate| candidate == key),
            base.iter().position(|candidate| candidate == key),
        ) {
            (Some(laid), Some(kept)) => slot(laid) - slot(kept),
            _ => 0.0,
        };
        delta + self.offset_at(key, now)
    }

    pub fn offset_from_slots(&self, key: &K, base: &[K], slot: impl Fn(usize) -> f32, window: &Window) -> f32 {
        let now = RiseClock::monotonic();
        if self.springs(now) {
            window.request_animation_frame();
        }
        self.offset_from_slots_at(key, base, slot, now)
    }

    /// drops every offset at once, for a list rebuilt from elsewhere mid motion
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// vertical offset to draw key at, from its laid out slot
    pub fn offset_at(&self, key: &K, now: MotionTime) -> f32 {
        if self.dragging.as_ref() == Some(key) {
            return self.translation - self.accumulated;
        }
        let shift = self.shifts.get(key).map_or(0.0, |shift| shift.value(now));
        let drop = if self.dropped.as_ref() == Some(key) { self.drop.value(now) } else { 0.0 };
        shift + drop
    }

    /// lift of key from 0 to 1, shadow and raise follow it
    pub fn lift_at(&self, key: &K, now: MotionTime) -> f32 {
        if self.lifted.as_ref() == Some(key) { self.lift.value(now).clamp(0.0, 1.0) } else { 0.0 }
    }

    pub fn is_running(&self, now: MotionTime) -> bool {
        self.dragging.is_some() || self.springs(now)
    }

    fn springs(&self, now: MotionTime) -> bool {
        self.drop.is_running(now) || self.lift.is_running(now) || self.shifts.values().any(|shift| shift.is_running(now))
    }

    /// offset for a render, asks for the next frame while anything still moves
    pub fn offset(&self, key: &K, window: &Window) -> f32 {
        let now = RiseClock::monotonic();
        if self.springs(now) {
            window.request_animation_frame();
        }
        self.offset_at(key, now)
    }

    pub fn lift(&self, key: &K, window: &Window) -> f32 {
        let now = RiseClock::monotonic();
        if self.springs(now) {
            window.request_animation_frame();
        }
        self.lift_at(key, now)
    }

    fn spring_from(&mut self, key: &K, delta: f32, now: MotionTime, reduce_motion: bool) {
        let shift = self.shifts.entry(key.clone()).or_insert_with(|| Animated::new(0.0));
        let from = shift.value(now) + delta;
        shift.jump(from);
        shift.set(0.0, spring(Self::SWAP_SPRING.0, Self::SWAP_SPRING.1), now, reduce_motion);
    }
}
