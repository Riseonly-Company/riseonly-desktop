use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, ParentElement, Styled,
    Window, actions, div,
};

use crate::swiftui::environment::{EnvKey, env, env_scope};

pub const LAYER_KEY_CONTEXT: &str = "RiseLayer";
pub const FORM_KEY_CONTEXT: &str = "RiseForm";

const ESCAPE_KEY: &str = "escape";

actions!(rise_layer, [SubmitForm]);

struct KeyBindingsInstalled;

impl gpui::Global for KeyBindingsInstalled {}

/// safe to call twice, the composition root calls it with the other kit bindings
pub fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<KeyBindingsInstalled>() {
        return;
    }
    cx.set_global(KeyBindingsInstalled);
    let context = Some(FORM_KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("secondary-enter", SubmitForm, context),
        KeyBinding::new("alt-enter", SubmitForm, context),
    ]);
}

pub type LayerHandler = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

pub fn focus_is_inside_layer(window: &Window) -> bool {
    window.context_stack().iter().any(|context| context.contains(LAYER_KEY_CONTEXT))
}

/// what a layer root does with escape
#[derive(Clone)]
pub enum LayerCancel {
    Dismiss(LayerHandler),
    Blocked,
}

impl LayerCancel {
    pub fn dismiss(handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self::Dismiss(Rc::new(handler))
    }

    fn run(&self, window: &mut Window, cx: &mut App) {
        if let Self::Dismiss(handler) = self {
            handler(window, cx);
        }
    }
}

/// true for escape with no modifier, escape with a modifier is someone elses chord
pub fn is_plain_escape(event: &KeyDownEvent) -> bool {
    event.keystroke.key == ESCAPE_KEY && !event.keystroke.modifiers.modified()
}

pub trait LayerKeysExt: InteractiveElement + Sized {
    /// marks the root of a surface over the window, takes the element key context
    /// so an element with its own context needs this on a wrapper
    fn layer_keys(self, cancel: LayerCancel) -> Self {
        self.key_context(LAYER_KEY_CONTEXT).on_key_down(move |event: &KeyDownEvent, window, cx| {
            if is_plain_escape(event) {
                cx.stop_propagation();
                cancel.run(window, cx);
            }
        })
    }

    /// marks a form, cmd enter and alt enter from any field press on_submit while enabled
    /// a disabled submit still takes the chord so it never falls through
    fn form_primary(self, enabled: bool, on_submit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.key_context(FORM_KEY_CONTEXT).on_action(move |_: &SubmitForm, window, cx| {
            if enabled {
                on_submit(window, cx);
            }
        })
    }

    /// one element that is both a layer and a form, eg a dialog with a field and a main button
    fn layer_form_keys(
        self,
        cancel: LayerCancel,
        enabled: bool,
        on_submit: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        let context = format!("{LAYER_KEY_CONTEXT} {FORM_KEY_CONTEXT}");
        self.key_context(context.as_str())
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if is_plain_escape(event) {
                    cx.stop_propagation();
                    cancel.run(window, cx);
                }
            })
            .on_action(move |_: &SubmitForm, window, cx| {
                if enabled {
                    on_submit(window, cx);
                }
            })
    }
}

impl<E: InteractiveElement + Sized> LayerKeysExt for E {}

/// main action a form button declares while rendering
/// for forms whose button is drawn by a child the scope never sees
#[derive(Clone, Default)]
pub struct FormPrimarySlot(Rc<RefCell<Option<(bool, LayerHandler)>>>);

impl FormPrimarySlot {
    fn submit(&self, window: &mut Window, cx: &mut App) {
        let entry = self.0.borrow().clone();
        if let Some((true, handler)) = entry {
            handler(window, cx);
        }
    }
}

pub struct FormPrimaryKey;

impl EnvKey for FormPrimaryKey {
    type Value = Option<FormPrimarySlot>;

    fn default_value() -> Self::Value {
        None
    }
}

/// wraps a form
/// cmd enter and alt enter from any field inside press the button that declared itself while
/// this scope rendered
pub fn form_scope(content: impl IntoElement) -> AnyElement {
    let slot = FormPrimarySlot::default();
    let submit = slot.clone();
    div()
        .size_full()
        .key_context(FORM_KEY_CONTEXT)
        .on_action(move |_: &SubmitForm, window, cx| submit.submit(window, cx))
        .child(env_scope::<FormPrimaryKey>(Some(slot)).child(content))
        .into_any_element()
}

/// called from the render of a form main button (not cancel)
/// says what the chord presses and whether it may, outside a form_scope it does nothing
pub fn declare_form_primary(enabled: bool, handler: LayerHandler, cx: &App) {
    if let Some(slot) = env::<FormPrimaryKey>(cx) {
        *slot.0.borrow_mut() = Some((enabled, handler));
    }
}
