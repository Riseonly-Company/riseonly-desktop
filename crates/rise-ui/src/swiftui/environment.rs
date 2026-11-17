use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, Global, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, ParentElement, Pixels, Style, Window,
};
use smallvec::SmallVec;

use super::environment_values::ColorScheme;
use crate::expandable_text_ui::EdgeInsets;
use crate::liquid_chrome::LiquidGlassContrast;

/// one environment key, empty struct with a value type and a default
pub trait EnvKey: 'static {
    type Value: Clone + 'static;

    fn default_value() -> Self::Value;

    /// fingerprint of a value for cached views below the scope, a changed one renders them fresh
    /// none adds nothing so a key holding non plain data (entity or handle or callback)
    /// must stay the same for the life of its view or override this
    fn fingerprint(value: &Self::Value) -> Option<u64> {
        data_fingerprint(value)
    }
}

fn hash_f32(value: f32, hasher: &mut DefaultHasher) {
    value.to_bits().hash(hasher);
}

fn hash_hsla(color: &Hsla, hasher: &mut DefaultHasher) {
    for channel in [color.h, color.s, color.l, color.a] {
        hash_f32(channel, hasher);
    }
}

fn hash_bounds(bounds: &Bounds<Pixels>, hasher: &mut DefaultHasher) {
    for edge in [
        bounds.origin.x,
        bounds.origin.y,
        bounds.size.width,
        bounds.size.height,
    ] {
        hash_f32(f32::from(edge), hasher);
    }
}

/// fingerprint of the plain data types env keys carry, none for other types
pub fn data_fingerprint<T: 'static>(value: &T) -> Option<u64> {
    let value = value as &dyn Any;
    let mut hasher = DefaultHasher::new();
    if let Some(flag) = value.downcast_ref::<bool>() {
        flag.hash(&mut hasher);
    } else if let Some(number) = value.downcast_ref::<f32>() {
        hash_f32(*number, &mut hasher);
    } else if let Some(number) = value.downcast_ref::<u32>() {
        number.hash(&mut hasher);
    } else if let Some(number) = value.downcast_ref::<usize>() {
        number.hash(&mut hasher);
    } else if let Some(length) = value.downcast_ref::<Pixels>() {
        hash_f32(f32::from(*length), &mut hasher);
    } else if let Some(color) = value.downcast_ref::<Hsla>() {
        hash_hsla(color, &mut hasher);
    } else if let Some(color) = value.downcast_ref::<Option<Hsla>>() {
        color.is_some().hash(&mut hasher);
        if let Some(color) = color {
            hash_hsla(color, &mut hasher);
        }
    } else if let Some(text) = value.downcast_ref::<String>() {
        text.hash(&mut hasher);
    } else if let Some(text) = value.downcast_ref::<Option<String>>() {
        text.hash(&mut hasher);
    } else if let Some(texts) = value.downcast_ref::<Vec<String>>() {
        texts.hash(&mut hasher);
    } else if let Some(id) = value.downcast_ref::<Option<ElementId>>() {
        id.hash(&mut hasher);
    } else if let Some(bounds) = value.downcast_ref::<Option<Bounds<Pixels>>>() {
        bounds.is_some().hash(&mut hasher);
        if let Some(bounds) = bounds {
            hash_bounds(bounds, &mut hasher);
        }
    } else if let Some(insets) = value.downcast_ref::<EdgeInsets>() {
        for edge in [insets.top, insets.leading, insets.bottom, insets.trailing] {
            hash_f32(edge, &mut hasher);
        }
    } else if let Some(direction) = value.downcast_ref::<LayoutDirection>() {
        direction.hash(&mut hasher);
    } else if let Some(phase) = value.downcast_ref::<Option<ScenePhase>>() {
        phase.hash(&mut hasher);
    } else if let Some(scheme) = value.downcast_ref::<Option<ColorScheme>>() {
        scheme.hash(&mut hasher);
    } else if let Some(contrast) = value.downcast_ref::<LiquidGlassContrast>() {
        contrast.hash(&mut hasher);
    } else {
        return None;
    }
    Some(hasher.finish())
}

#[derive(Default)]
struct EnvStacks {
    stacks: HashMap<TypeId, Vec<Rc<dyn Any>>>,
    snapshot: Option<Rc<dyn Any>>,
}

impl Global for EnvStacks {}

type EnvSnapshot = HashMap<TypeId, Vec<Rc<dyn Any>>>;

fn capture_env(cx: &mut App) -> Rc<dyn Any> {
    let globals = cx.default_global::<EnvStacks>();
    if let Some(snapshot) = globals.snapshot.as_ref() {
        return snapshot.clone();
    }
    let snapshot: Rc<dyn Any> = Rc::new(globals.stacks.clone());
    globals.snapshot = Some(snapshot.clone());
    snapshot
}

fn swap_env(cx: &mut App, snapshot: Rc<dyn Any>) -> Rc<dyn Any> {
    let globals = cx.default_global::<EnvStacks>();
    let stacks = snapshot
        .downcast_ref::<EnvSnapshot>()
        .cloned()
        .unwrap_or_default();
    let previous = std::mem::replace(&mut globals.stacks, stacks);
    globals.snapshot = Some(snapshot);
    Rc::new(previous)
}

const ENV_SNAPSHOT_HOOKS: gpui::EnvSnapshotHooks = gpui::EnvSnapshotHooks {
    capture: capture_env,
    swap: swap_env,
};

fn push_value<K: EnvKey>(value: &K::Value, cx: &mut App) {
    gpui::set_env_snapshot_hooks(ENV_SNAPSHOT_HOOKS);
    let globals = cx.default_global::<EnvStacks>();
    globals.snapshot = None;
    globals
        .stacks
        .entry(TypeId::of::<K>())
        .or_default()
        .push(Rc::new(value.clone()));
}

fn pop_value<K: EnvKey>(cx: &mut App) {
    let globals = cx.default_global::<EnvStacks>();
    globals.snapshot = None;
    let stacks = &mut globals.stacks;
    if let Some(stack) = stacks.get_mut(&TypeId::of::<K>()) {
        stack.pop();
        if stack.is_empty() {
            stacks.remove(&TypeId::of::<K>());
        }
    }
}

/// value of the nearest env_scope above, or the key default
pub fn env<K: EnvKey>(cx: &App) -> K::Value {
    cx.try_global::<EnvStacks>()
        .and_then(|globals| globals.stacks.get(&TypeId::of::<K>()))
        .and_then(|stack| stack.last())
        .and_then(|top| top.downcast_ref::<K::Value>())
        .cloned()
        .unwrap_or_else(K::default_value)
}

/// sets an env value for the children added with child
pub fn env_scope<K: EnvKey>(value: K::Value) -> EnvScope<K> {
    EnvScope {
        value,
        children: SmallVec::new(),
    }
}

/// element from env_scope, one child adds no layout node
/// several share one unstyled box
pub struct EnvScope<K: EnvKey> {
    value: K::Value,
    children: SmallVec<[AnyElement; 1]>,
}

impl<K: EnvKey> ParentElement for EnvScope<K> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl<K: EnvKey> IntoElement for EnvScope<K> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<K: EnvKey> Element for EnvScope<K> {
    type RequestLayoutState = ();
    type PrepaintState = ();

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
        push_value::<K>(&self.value, cx);
        let layout_id = if self.children.len() == 1 {
            self.children[0].request_layout(window, cx)
        } else {
            let child_ids: SmallVec<[LayoutId; 4]> = self
                .children
                .iter_mut()
                .map(|child| child.request_layout(window, cx))
                .collect();
            window.request_layout(Style::default(), child_ids, cx)
        };
        pop_value::<K>(cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let outer = window.env_fingerprint();
        let fingerprint = K::fingerprint(&self.value);
        if let Some(fingerprint) = fingerprint {
            let mut hasher = DefaultHasher::new();
            outer.hash(&mut hasher);
            TypeId::of::<K>().hash(&mut hasher);
            fingerprint.hash(&mut hasher);
            window.set_env_fingerprint(hasher.finish());
            let name = std::any::type_name::<K>();
            window.push_env_trace(name.rsplit("::").next().unwrap_or(name), fingerprint);
        }
        push_value::<K>(&self.value, cx);
        for child in &mut self.children {
            child.prepaint(window, cx);
        }
        pop_value::<K>(cx);
        if fingerprint.is_some() {
            window.pop_env_trace();
        }
        window.set_env_fingerprint(outer);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        push_value::<K>(&self.value, cx);
        for child in &mut self.children {
            child.paint(window, cx);
        }
        pop_value::<K>(cx);
    }
}

/// layout direction, left to right or right to left
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayoutDirection {
    #[default]
    LeftToRight,
    RightToLeft,
}

/// layout direction key, the root sets it from the language store
/// a subtree may pin its own, eg the bottom sheet deck pins left to right
pub struct LayoutDirectionKey;

impl EnvKey for LayoutDirectionKey {
    type Value = LayoutDirection;

    fn default_value() -> Self::Value {
        LayoutDirection::current()
    }
}

pub fn layout_direction(cx: &App) -> LayoutDirection {
    env::<LayoutDirectionKey>(cx)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScenePhase {
    #[default]
    Active,
    Inactive,
    Background,
}

/// phase the app shell published last
/// app wide because focus and minimizing of all windows map to one scene
#[derive(Clone, Copy, Default)]
struct CurrentScenePhase(ScenePhase);

impl Global for CurrentScenePhase {}

/// called by the app shell when focus or minimizing changes the phase
pub fn set_scene_phase(phase: ScenePhase, cx: &mut App) {
    cx.set_global(CurrentScenePhase(phase));
}

/// scene phase key for a subtree that reports its own phase
pub struct ScenePhaseKey;

impl EnvKey for ScenePhaseKey {
    type Value = Option<ScenePhase>;

    fn default_value() -> Self::Value {
        None
    }
}

pub fn scene_phase(cx: &App) -> ScenePhase {
    env::<ScenePhaseKey>(cx)
        .or_else(|| cx.try_global::<CurrentScenePhase>().map(|current| current.0))
        .unwrap_or_default()
}

/// true if reduce motion is on, gpui already tracks the system setting
pub fn reduce_motion(cx: &App) -> bool {
    cx.reduce_motion()
}
