use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use gpui::{
    AnyElement, App, Bounds, Div, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, ParentElement, Pixels, Point, Styled, TransformationMatrix, Window,
    div, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;

use crate::swiftui::animated::{Animated, MotionTime};
use crate::swiftui::environment::LayoutDirection;
use crate::swiftui::measure::measure;
use crate::swiftui::motion::MotionSpec;

pub const DEFAULT_TRANSITION_SCALE: f32 = 0.0;

/// edge, leading and trailing follow the layout direction
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    Top,
    Leading,
    Bottom,
    Trailing,
}

impl Edge {
    /// unit vector towards this edge in screen coords, y grows down
    pub fn direction(self, layout_direction: LayoutDirection) -> Point<f32> {
        let towards_end = match layout_direction {
            LayoutDirection::LeftToRight => 1.0,
            LayoutDirection::RightToLeft => -1.0,
        };
        match self {
            Edge::Top => point(0.0, -1.0),
            Edge::Bottom => point(0.0, 1.0),
            Edge::Leading => point(-towards_end, 0.0),
            Edge::Trailing => point(towards_end, 0.0),
        }
    }
}

/// point in a view own bounds, 0 is leading or top and 1 is trailing or bottom
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitPoint {
    pub x: f32,
    pub y: f32,
}

impl UnitPoint {
    pub const ZERO: Self = Self::new(0.0, 0.0);
    pub const CENTER: Self = Self::new(0.5, 0.5);
    pub const LEADING: Self = Self::new(0.0, 0.5);
    pub const TRAILING: Self = Self::new(1.0, 0.5);
    pub const TOP: Self = Self::new(0.5, 0.0);
    pub const BOTTOM: Self = Self::new(0.5, 1.0);
    pub const TOP_LEADING: Self = Self::new(0.0, 0.0);
    pub const TOP_TRAILING: Self = Self::new(1.0, 0.0);
    pub const BOTTOM_LEADING: Self = Self::new(0.0, 1.0);
    pub const BOTTOM_TRAILING: Self = Self::new(1.0, 1.0);

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// point in screen coords, leading is the right side in right to left layout
    pub fn resolved(self, layout_direction: LayoutDirection) -> Self {
        match layout_direction {
            LayoutDirection::LeftToRight => self,
            LayoutDirection::RightToLeft => Self::new(1.0 - self.x, self.y),
        }
    }
}

/// which way a view is passing, picks the insertion or removal half of an asymmetric transition
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransitionPhase {
    Insertion,
    Removal,
}

/// look of a view part way through a transition
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionEffect {
    pub opacity: f32,
    pub offset: Point<f32>,
    pub scale: f32,
    pub anchor: UnitPoint,
}

impl TransitionEffect {
    pub const IDENTITY: Self = Self {
        opacity: 1.0,
        offset: Point { x: 0.0, y: 0.0 },
        scale: 1.0,
        anchor: UnitPoint::CENTER,
    };

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// opacity the clipped fallback used before the transform patch, not used for drawing now
    pub fn drawn_opacity(&self) -> f32 {
        self.opacity * self.scale.clamp(0.0, 1.0)
    }

    /// part of bounds a view scaled about its anchor covers, growing past 1 keeps the full bounds
    /// not used for drawing now, paint transforms instead of clipping
    pub fn scaled_bounds(&self, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        let scale = self.scale.clamp(0.0, 1.0);
        if scale >= 1.0 {
            return bounds;
        }
        let anchor_x = bounds.origin.x + bounds.size.width * self.anchor.x;
        let anchor_y = bounds.origin.y + bounds.size.height * self.anchor.y;
        Bounds {
            origin: point(
                anchor_x - (anchor_x - bounds.origin.x) * scale,
                anchor_y - (anchor_y - bounds.origin.y) * scale,
            ),
            size: size(bounds.size.width * scale, bounds.size.height * scale),
        }
    }

    /// both modifiers on the same view
    fn combined(self, other: Self) -> Self {
        let anchor = if other.scale != 1.0 { other.anchor } else { self.anchor };
        Self {
            opacity: self.opacity * other.opacity,
            offset: point(self.offset.x + other.offset.x, self.offset.y + other.offset.y),
            scale: self.scale * other.scale,
            anchor,
        }
    }
}

/// transition, any of identity or opacity or move or scale and combos
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Transition {
    Identity,
    #[default]
    Opacity,
    Move(Edge),
    Scale { scale: f32, anchor: UnitPoint },
    Combined(Box<Transition>, Box<Transition>),
    Asymmetric { insertion: Box<Transition>, removal: Box<Transition> },
    Animated { transition: Box<Transition>, animation: MotionSpec },
}

impl Transition {
    /// identity transition, does nothing
    pub fn identity() -> Self {
        Self::Identity
    }

    pub fn opacity() -> Self {
        Self::Opacity
    }

    /// the view travels by its own size towards the edge
    pub fn move_edge(edge: Edge) -> Self {
        Self::Move(edge)
    }

    /// scale about the center
    pub fn scale(scale: f32) -> Self {
        Self::scale_anchored(scale, UnitPoint::CENTER)
    }

    /// default scale, no args
    pub fn scale_default() -> Self {
        Self::scale(DEFAULT_TRANSITION_SCALE)
    }

    pub fn scale_anchored(scale: f32, anchor: UnitPoint) -> Self {
        Self::Scale { scale, anchor }
    }

    pub fn combined(self, other: Transition) -> Self {
        Self::Combined(Box::new(self), Box::new(other))
    }

    /// different transitions for insertion and removal
    pub fn asymmetric(insertion: Transition, removal: Transition) -> Self {
        Self::Asymmetric {
            insertion: Box::new(insertion),
            removal: Box::new(removal),
        }
    }

    /// this transition runs with its own animation instead of the transaction one
    pub fn animation(self, animation: MotionSpec) -> Self {
        Self::Animated {
            transition: Box::new(self),
            animation,
        }
    }

    /// animation the transition carries for the phase, if any
    /// outermost wins
    pub fn animation_for(&self, phase: TransitionPhase) -> Option<MotionSpec> {
        match self {
            Self::Animated { animation, .. } => Some(*animation),
            Self::Asymmetric { insertion, removal } => match phase {
                TransitionPhase::Insertion => insertion.animation_for(phase),
                TransitionPhase::Removal => removal.animation_for(phase),
            },
            Self::Combined(first, second) => first
                .animation_for(phase)
                .or_else(|| second.animation_for(phase)),
            Self::Identity | Self::Opacity | Self::Move(_) | Self::Scale { .. } => None,
        }
    }

    /// effect at amount of the active state, 0 is the view in place and 1 fully inserted or removed
    pub fn effect(
        &self,
        phase: TransitionPhase,
        amount: f32,
        layout_direction: LayoutDirection,
    ) -> TransitionEffect {
        match self {
            Self::Identity => TransitionEffect::IDENTITY,
            Self::Opacity => TransitionEffect {
                opacity: (1.0 - amount).clamp(0.0, 1.0),
                ..TransitionEffect::IDENTITY
            },
            Self::Move(edge) => {
                let direction = edge.direction(layout_direction);
                TransitionEffect {
                    offset: point(direction.x * amount, direction.y * amount),
                    ..TransitionEffect::IDENTITY
                }
            }
            Self::Scale { scale, anchor } => TransitionEffect {
                scale: 1.0 + (scale - 1.0) * amount,
                anchor: anchor.resolved(layout_direction),
                ..TransitionEffect::IDENTITY
            },
            Self::Combined(first, second) => first
                .effect(phase, amount, layout_direction)
                .combined(second.effect(phase, amount, layout_direction)),
            Self::Asymmetric { insertion, removal } => match phase {
                TransitionPhase::Insertion => insertion.effect(phase, amount, layout_direction),
                TransitionPhase::Removal => removal.effect(phase, amount, layout_direction),
            },
            Self::Animated { transition, .. } => transition.effect(phase, amount, layout_direction),
        }
    }
}

/// where a presence is, which way it passes and how far from the active state
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionState {
    pub phase: TransitionPhase,
    pub progress: f32,
}

/// conditional view with its transition, keep it in the view that owns the condition
/// call set where the optional gets assigned inside an animation
#[derive(Clone, Debug)]
pub struct Presence<T> {
    value: Option<T>,
    present: bool,
    phase: TransitionPhase,
    transition: Transition,
    progress: Animated<f32>,
    departures: Vec<Departure<T>>,
}

/// value a new key replaced, on its way out from wherever it stood
#[derive(Clone, Debug)]
struct Departure<T> {
    value: T,
    progress: Animated<f32>,
}

impl<T> Presence<T> {
    /// starting value is on screen at once, first appearance is not animated
    pub fn new(value: Option<T>) -> Self {
        let present = value.is_some();
        Self {
            value,
            present,
            phase: TransitionPhase::Insertion,
            transition: Transition::default(),
            progress: Animated::new(if present { 1.0 } else { 0.0 }),
            departures: Vec::new(),
        }
    }

    /// transition for the conditional content
    pub fn with_transition(mut self, transition: Transition) -> Self {
        self.transition = transition;
        self
    }

    pub fn set_transition(&mut self, transition: Transition) {
        self.transition = transition;
    }

    pub fn transition(&self) -> &Transition {
        &self.transition
    }

    /// sets the value with a spec, respects reduce motion
    pub fn set_in(
        &mut self,
        value: Option<T>,
        spec: impl Into<Option<MotionSpec>>,
        now: MotionTime,
        cx: &App,
    ) {
        self.set(value, spec, now, cx.reduce_motion());
    }

    /// inserts or replaces or removes the value, a removal keeps the old value until it has run
    /// an insertion during a removal starts from the progress on screen
    /// no spec or reduce motion lands at once
    pub fn set(
        &mut self,
        value: Option<T>,
        spec: impl Into<Option<MotionSpec>>,
        now: MotionTime,
        reduce_motion: bool,
    ) {
        let spec = spec.into();
        self.departures.retain(|departure| departure.progress.is_running(now));
        match value {
            Some(value) => {
                self.value = Some(value);
                if !self.present {
                    self.present = true;
                    self.phase = TransitionPhase::Insertion;
                    let spec = self.transition.animation_for(self.phase).or(spec);
                    self.progress.set(1.0, spec, now, reduce_motion);
                }
            }
            None => {
                if !self.present {
                    return;
                }
                self.present = false;
                self.phase = TransitionPhase::Removal;
                let spec = self.transition.animation_for(self.phase).or(spec);
                self.progress.set(0.0, spec, now, reduce_motion);
                if !self.progress.is_running(now) {
                    self.value = None;
                }
            }
        }
    }

    /// set_keyed that respects reduce motion
    pub fn set_keyed_in<K: PartialEq>(
        &mut self,
        value: Option<T>,
        key: impl Fn(&T) -> K,
        spec: impl Into<Option<MotionSpec>>,
        now: MotionTime,
        cx: &App,
    ) {
        self.set_keyed(value, key, spec, now, cx.reduce_motion());
    }

    /// same as set under one key
    /// a new key sends the old value out with the removal while the new one enters
    /// draw the old ones with the departing render
    pub fn set_keyed<K: PartialEq>(
        &mut self,
        value: Option<T>,
        key: impl Fn(&T) -> K,
        spec: impl Into<Option<MotionSpec>>,
        now: MotionTime,
        reduce_motion: bool,
    ) {
        let spec = spec.into();
        let is_new_key = match (self.value.as_ref(), value.as_ref()) {
            (Some(old), Some(new)) => key(old) != key(new),
            _ => false,
        };
        if is_new_key {
            self.depart_current(spec, now, reduce_motion);
        }
        self.set(value, spec, now, reduce_motion);
    }

    fn depart_current(&mut self, spec: Option<MotionSpec>, now: MotionTime, reduce_motion: bool) {
        let mut progress = std::mem::replace(&mut self.progress, Animated::new(0.0));
        let value = self.value.take();
        if self.present {
            self.present = false;
            let spec = self.transition.animation_for(TransitionPhase::Removal).or(spec);
            progress.set(0.0, spec, now, reduce_motion);
        }
        if let Some(value) = value.filter(|_| progress.is_running(now)) {
            self.departures.push(Departure { value, progress });
        }
    }

    /// true if the model holds a value, a departing value doesnt count
    pub fn is_present(&self) -> bool {
        self.present
    }

    /// true if a removed value is still on screen at now
    pub fn is_leaving(&self, now: MotionTime) -> bool {
        !self.present && self.value.is_some() && self.progress.is_running(now)
    }

    /// true while frames are still needed at now
    pub fn is_running(&self, now: MotionTime) -> bool {
        self.progress.is_running(now)
            || self.departures.iter().any(|departure| departure.progress.is_running(now))
    }

    /// value to draw at now, the current one or the departing one while it leaves
    pub fn value(&self, now: MotionTime) -> Option<&T> {
        if self.present || self.is_leaving(now) {
            self.value.as_ref()
        } else {
            None
        }
    }

    pub fn state(&self, now: MotionTime) -> TransitionState {
        TransitionState {
            phase: self.phase,
            progress: self.progress.value(now),
        }
    }

    /// transition effect at now for the installed layout direction
    pub fn effect(&self, now: MotionTime) -> TransitionEffect {
        self.effect_in(now, LayoutDirection::current())
    }

    pub fn effect_in(&self, now: MotionTime, layout_direction: LayoutDirection) -> TransitionEffect {
        let state = self.state(now);
        self.transition
            .effect(state.phase, 1.0 - state.progress, layout_direction)
    }

    #[track_caller]
    pub fn render<E>(
        &self,
        now: MotionTime,
        window: &Window,
        content: impl FnOnce(&T, TransitionState) -> E,
    ) -> Option<E> {
        if self.is_running(now) {
            window.request_animation_frame();
        }
        self.value(now).map(|value| content(value, self.state(now)))
    }

    /// render with the transition applied to the content
    #[track_caller]
    pub fn render_transitioned<E: Styled + IntoElement>(
        &self,
        now: MotionTime,
        window: &Window,
        content: impl FnOnce(&T) -> E,
    ) -> Option<Transitioned> {
        let effect = self.effect(now);
        self.render(now, window, |value, _| content(value).transition_effect(effect))
    }

    /// render_transitioned for a content height container
    /// a leaving value is marked so it stops counting toward the height at once
    #[track_caller]
    pub fn render_reveal<E: Styled + IntoElement>(
        &self,
        now: MotionTime,
        window: &Window,
        content: impl FnOnce(&T) -> E,
    ) -> Option<RevealItem> {
        let leaving = self.is_leaving(now);
        self.render_transitioned(now, window, content)
            .map(|child| RevealItem { child, leaving })
    }

    /// values a new key replaced that are still on screen at now, oldest first
    /// each with its removal effect
    pub fn departing_in(
        &self,
        now: MotionTime,
        layout_direction: LayoutDirection,
    ) -> impl Iterator<Item = (&T, TransitionEffect)> {
        self.departures
            .iter()
            .filter(move |departure| departure.progress.is_running(now))
            .map(move |departure| {
                let amount = 1.0 - departure.progress.value(now);
                let effect = self.transition.effect(TransitionPhase::Removal, amount, layout_direction);
                (&departure.value, effect)
            })
    }

    /// copies set_keyed sent out, stack them over the current content absolutely positioned
    /// content with element state needs the key in its element id
    #[track_caller]
    pub fn render_departing_transitioned<E: Styled + IntoElement>(
        &self,
        now: MotionTime,
        window: &Window,
        mut content: impl FnMut(&T) -> E,
    ) -> Vec<Transitioned> {
        if self.is_running(now) {
            window.request_animation_frame();
        }
        self.departing_in(now, LayoutDirection::current())
            .map(|(value, effect)| content(value).transition_effect(effect))
            .collect()
    }
}

pub struct RevealItem {
    child: Transitioned,
    leaving: bool,
}

/// height a content height container shows
/// gpui learns the new natural height only after layout
/// so it stays old one frame, then animates to measured
#[derive(Clone, Debug)]
pub struct ContentHeightMotion {
    height: Animated<f32>,
    branch: Option<bool>,
    has_measure: bool,
    swapping: bool,
}

impl Default for ContentHeightMotion {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentHeightMotion {
    pub fn new() -> Self {
        Self {
            height: Animated::new(0.0),
            branch: None,
            has_measure: false,
            swapping: false,
        }
    }

    /// branch this frame lays out, a changed branch makes the next measurement animate
    pub fn frame(&mut self, branch: bool) {
        if self.branch.is_some_and(|last| last != branch) {
            self.swapping = true;
        }
        self.branch = Some(branch);
    }

    /// height to hold the container at, none while it just follows its content
    pub fn clamp(&self, now: MotionTime) -> Option<f32> {
        (self.has_measure && (self.swapping || self.height.is_running(now)))
            .then(|| self.height.value(now).max(0.0))
    }

    /// true while frames are still needed at now
    pub fn is_running(&self, now: MotionTime) -> bool {
        self.height.is_running(now)
    }

    /// natural height after layout, only a branch swap animates
    /// wrapping or loaded text changes land at once, returns whether another frame is needed
    pub fn measured(&mut self, natural: f32, spec: MotionSpec, now: MotionTime, reduce_motion: bool) -> bool {
        let swapping = std::mem::take(&mut self.swapping);
        if !self.has_measure {
            self.has_measure = true;
            self.height.jump(natural);
            return false;
        }
        if (natural - *self.height.target()).abs() < 0.01 {
            return false;
        }
        if swapping || self.height.is_running(now) {
            self.height.set(natural, spec, now, reduce_motion);
            return true;
        }
        self.height.jump(natural);
        false
    }
}

/// height container for an if branch swap, branches fade through presence
/// so neighbours below slide instead of jumping
/// clips while moving since gpui cant overflow, dont ask
#[track_caller]
pub fn content_height_motion(
    id: impl Into<ElementId>,
    branch: bool,
    parent_gap: f32,
    spec: MotionSpec,
    items: impl IntoIterator<Item = RevealItem>,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let id = Arc::new(id.into());
    let motion = window.use_keyed_state(
        ElementId::NamedChild(id.clone(), "content-height".into()),
        cx,
        |_, _| ContentHeightMotion::new(),
    );
    let now = RiseClock::monotonic();
    motion.update(cx, |motion, _| motion.frame(branch));
    let clamp = motion.read(cx).clamp(now);
    if motion.read(cx).is_running(now) {
        window.request_animation_frame();
    }

    let measure_part = if branch { "measure-on" } else { "measure-off" };
    let reported = motion.clone();
    let measurer = measure(ElementId::NamedChild(id, measure_part.into()), move |bounds, _, cx| {
        let natural = f32::from(bounds.size.height);
        reported.update(cx, |motion, cx| {
            if motion.measured(natural, spec, RiseClock::monotonic(), cx.reduce_motion()) {
                cx.notify();
            }
        });
    });

    let mut natural = div().flex().flex_col().relative().w_full().flex_shrink_0();
    let mut leaving = Vec::new();
    for item in items {
        let row = div().w_full().pt(px(parent_gap)).child(item.child);
        if item.leaving {
            leaving.push(row.absolute().top(px(0.)).left(px(0.)));
        } else {
            natural = natural.child(row);
        }
    }

    let container = div()
        .relative()
        .w_full()
        .flex_shrink_0()
        .mt(px(-parent_gap))
        .child(natural.child(measurer))
        .children(leaving);
    match clamp {
        Some(height) => container.h(px(height)).overflow_hidden(),
        None => container,
    }
}

/// transition applied by hand for a state a view animates itself
pub trait TransitionModifiers: Styled + IntoElement + Sized {
    /// applies the effect, opacity multiplies the own one
    /// offset moves by a fraction of its size without moving its layout slot
    /// scale transforms the painted content
    fn transition_effect(mut self, effect: TransitionEffect) -> Transitioned {
        multiply_opacity(&mut self, effect.opacity);
        Transitioned {
            child: self.into_any_element(),
            effect,
            keys_cached_paint: false,
        }
    }

    /// scale about the center
    fn scale_effect(self, scale: f32) -> Transitioned {
        self.scale_effect_anchored(scale, UnitPoint::CENTER)
    }

    /// scale about an anchor
    /// every scale goes through here so the renderer patch switches them all to a real transform
    fn scale_effect_anchored(self, scale: f32, anchor: UnitPoint) -> Transitioned {
        self.transition_effect(TransitionEffect {
            scale,
            anchor: anchor.resolved(LayoutDirection::current()),
            ..TransitionEffect::IDENTITY
        })
    }
}

impl<E: Styled + IntoElement> TransitionModifiers for E {}

/// a faded view inside a faded transition is fainter still, opacities nest
pub(crate) fn multiply_opacity<E: Styled>(element: &mut E, factor: f32) {
    let own = element.style().opacity.unwrap_or(1.0);
    element.style().opacity = Some(own * factor);
}

/// element from transition_effect, no layout node
pub struct Transitioned {
    child: AnyElement,
    effect: TransitionEffect,
    keys_cached_paint: bool,
}

impl Transitioned {
    pub fn effect(&self) -> TransitionEffect {
        self.effect
    }

    /// cached views below miss their cache while the effect runs
    /// so none replays a frame from an earlier opacity or scale
    /// for short entrances with few cached views (eg a modal), list row transitions keep the replay
    pub fn keys_cached_paint(mut self) -> Self {
        self.keys_cached_paint = true;
        self
    }
}

fn effect_fingerprint(outer: u64, effect: &TransitionEffect) -> u64 {
    let mut hasher = DefaultHasher::new();
    outer.hash(&mut hasher);
    for value in [effect.opacity, effect.offset.x, effect.offset.y, effect.scale, effect.anchor.x, effect.anchor.y] {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

impl IntoElement for Transitioned {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Transitioned {
    type RequestLayoutState = ();
    type PrepaintState = Point<Pixels>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let offset = point(
            bounds.size.width * self.effect.offset.x,
            bounds.size.height * self.effect.offset.y,
        );
        let child = &mut self.child;
        let rekeys = self.keys_cached_paint && !self.effect.is_identity();
        let outer = rekeys.then(|| {
            let outer = window.env_fingerprint();
            window.set_env_fingerprint(effect_fingerprint(outer, &self.effect));
            outer
        });
        window.with_element_offset(offset, |window| child.prepaint(window, cx));
        if let Some(outer) = outer {
            window.set_env_fingerprint(outer);
        }
        offset
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        offset: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let scale = self.effect.scale.max(0.0);
        if scale == 1.0 {
            self.child.paint(window, cx);
            return;
        }
        if scale <= f32::EPSILON {
            return;
        }
        let anchor = point(
            bounds.origin.x + offset.x + bounds.size.width * self.effect.anchor.x,
            bounds.origin.y + offset.y + bounds.size.height * self.effect.anchor.y,
        );
        let scale_factor = window.scale_factor();
        let matrix = TransformationMatrix::unit()
            .translate(anchor.scale(scale_factor))
            .scale(size(scale, scale))
            .translate(anchor.scale(-scale_factor));
        let child = &mut self.child;
        window.with_transformation(matrix, |window| child.paint(window, cx));
    }
}
