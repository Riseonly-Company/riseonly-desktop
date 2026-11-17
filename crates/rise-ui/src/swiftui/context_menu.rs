use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Bounds, Context, Div, ElementId, Entity, FocusHandle, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, Keystroke, KeystrokeEvent, MouseButton, MouseDownEvent, ParentElement,
    Pixels, Point, RenderOnce, SharedString, Styled, Subscription, Window, anchored, canvas, deferred,
    div, px,
};
use rise_theme::ThemeStore;

use crate::context_menu_ui::{ContextMenuItem, DeferredLayerFloorKey};
use crate::hold_context_menu_ui::hold_context_menu_action_list::{HoldContextMenuActionList, HoldContextMenuInkKey};
use crate::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use crate::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use crate::swiftui::environment::{env, env_scope};

pub(crate) const MENU_WINDOW_MARGIN: f32 = 8.0;

pub trait ContextMenuModifiers: IntoElement + Sized {
    /// attaches a context menu, no items means no menu
    fn context_menu(self, id: impl Into<ElementId>, items: Vec<ContextMenuItem>) -> ContextMenu {
        ContextMenu {
            id: id.into(),
            items,
            content: self.into_any_element(),
            on_presented_change: None,
            opens_on_double_click: false,
        }
    }
}

impl<E: IntoElement> ContextMenuModifiers for E {}

/// true if the key asks for the context menu, menu key alone or shift f10
pub fn is_context_menu_keystroke(keystroke: &Keystroke) -> bool {
    let modifiers = keystroke.modifiers;
    match keystroke.key.as_str() {
        "menu" => !modifiers.modified(),
        "f10" => modifiers.shift && modifiers.number_of_modifiers() == 1,
        _ => false,
    }
}

/// what a menu row or the closing of a menu runs
pub(crate) type ContextMenuRowAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// what a key does to an open menu
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContextMenuKeyAction {
    Dismiss,
    Highlight(usize),
    Activate(usize),
    Swallow,
}

/// how an open menu answers a keystroke, none lets the key through
/// enabled has one entry per row, a disabled row is never highlighted or picked
pub(crate) fn context_menu_key_action(
    keystroke: &Keystroke,
    highlighted: Option<usize>,
    enabled: &[bool],
) -> Option<ContextMenuKeyAction> {
    if keystroke.modifiers.modified() {
        return None;
    }
    let is_enabled = |index: &usize| enabled[*index];
    let target = match keystroke.key.as_str() {
        "escape" => return Some(ContextMenuKeyAction::Dismiss),
        "enter" => {
            return Some(match highlighted {
                Some(index) if enabled.get(index).copied().unwrap_or(false) => {
                    ContextMenuKeyAction::Activate(index)
                }
                _ => ContextMenuKeyAction::Swallow,
            });
        }
        "down" => {
            let start = highlighted.map_or(0, |index| index + 1);
            (start..enabled.len()).find(is_enabled)
        }
        "up" => {
            let end = highlighted.unwrap_or(enabled.len()).min(enabled.len());
            (0..end).rev().find(is_enabled)
        }
        "home" => (0..enabled.len()).find(is_enabled),
        "end" => (0..enabled.len()).rev().find(is_enabled),
        _ => return None,
    };
    Some(target.map_or(ContextMenuKeyAction::Swallow, ContextMenuKeyAction::Highlight))
}

/// keyboard state of one menu, alive while it is open
#[derive(Default)]
struct ContextMenuKeys {
    highlighted: Option<usize>,
    rows: Vec<Option<ContextMenuRowAction>>,
    dismiss: Option<ContextMenuRowAction>,
    interceptor: Option<Subscription>,
}

fn handle_context_menu_key(
    keys: &Entity<ContextMenuKeys>,
    keystroke: &Keystroke,
    window: &mut Window,
    cx: &mut App,
) {
    let action = {
        let state = keys.read(cx);
        let enabled = state.rows.iter().map(Option::is_some).collect::<Vec<_>>();
        context_menu_key_action(keystroke, state.highlighted, &enabled)
    };
    let Some(action) = action else {
        return;
    };
    cx.stop_propagation();
    match action {
        ContextMenuKeyAction::Dismiss => {
            let dismiss = keys.read(cx).dismiss.clone();
            if let Some(dismiss) = dismiss {
                dismiss(window, cx);
            }
        }
        ContextMenuKeyAction::Highlight(index) => {
            keys.update(cx, |state: &mut ContextMenuKeys, cx: &mut Context<ContextMenuKeys>| {
                state.highlighted = Some(index);
                cx.notify();
            });
        }
        ContextMenuKeyAction::Activate(index) => {
            let (dismiss, row) = {
                let state = keys.read(cx);
                (state.dismiss.clone(), state.rows.get(index).cloned().flatten())
            };
            if let Some(dismiss) = dismiss {
                dismiss(window, cx);
            }
            if let Some(row) = row {
                row(window, cx);
            }
        }
        ContextMenuKeyAction::Swallow => {}
    }
}

/// keyboard state of one menu given to its panel, so pointer and arrows move the same highlight
#[derive(Clone)]
pub(crate) struct ContextMenuKeysHandle(Entity<ContextMenuKeys>);

impl ContextMenuKeysHandle {
    fn highlight(&self, index: Option<usize>, cx: &mut App) {
        self.0.update(cx, |state: &mut ContextMenuKeys, cx: &mut Context<ContextMenuKeys>| {
            if state.highlighted != index {
                state.highlighted = index;
                cx.notify();
            }
        });
    }
}

/// keeps the menu keyboard in step with this frame and returns the highlighted row
/// open holds the rows and the close action of an open menu
pub(crate) fn sync_context_menu_keys(
    id: &ElementId,
    open: Option<(Vec<Option<ContextMenuRowAction>>, ContextMenuRowAction)>,
    window: &mut Window,
    cx: &mut App,
) -> (Option<usize>, ContextMenuKeysHandle) {
    let keys = window.use_keyed_state(
        ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static("keys")),
        cx,
        |_, _| ContextMenuKeys::default(),
    );
    let Some((rows, dismiss)) = open else {
        if keys.read(cx).interceptor.is_some() {
            keys.update(cx, |state: &mut ContextMenuKeys, _: &mut Context<ContextMenuKeys>| {
                *state = ContextMenuKeys::default();
            });
        }
        return (None, ContextMenuKeysHandle(keys));
    };
    let key_window = window.window_handle();
    let weak_keys = keys.downgrade();
    keys.update(cx, |state: &mut ContextMenuKeys, cx: &mut Context<ContextMenuKeys>| {
        state.rows = rows;
        state.dismiss = Some(dismiss);
        if state.interceptor.is_none() {
            state.interceptor = Some(cx.intercept_keystrokes(
                move |event: &KeystrokeEvent, window: &mut Window, cx: &mut App| {
                    if window.window_handle() != key_window {
                        return;
                    }
                    if let Some(keys) = weak_keys.upgrade() {
                        handle_context_menu_key(&keys, &event.keystroke, window, cx);
                    }
                },
            ));
        }
    });
    let highlighted = keys.read(cx).highlighted;
    (highlighted, ContextMenuKeysHandle(keys))
}

/// panel of an open menu, same action list as the chat hold menu
/// row ids are item indices, a pick closes the menu first and then runs the row
pub(crate) fn context_menu_panel(
    id: &ElementId,
    items: &[ContextMenuItem],
    selected: Option<&str>,
    highlighted: Option<usize>,
    keys: ContextMenuKeysHandle,
    dismiss: ContextMenuRowAction,
    cx: &mut App,
) -> AnyElement {
    let ink = ThemeStore::shared(cx).read(cx).current_theme().text.primary;
    let actions = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let mut action = HoldContextMenuAction::new(index.to_string(), item.label.clone())
                .destructive(item.destructive)
                .disabled(item.disabled)
                .section_break(item.begins_section)
                .checked(selected == Some(item.key.as_str()));
            action.system_image = item.icon.clone();
            action
        })
        .collect::<Vec<_>>();
    let callbacks = Rc::new(items.iter().map(|item| item.callback.clone()).collect::<Vec<_>>());
    let on_select: Rc<dyn Fn(String, &mut Window, &mut App)> =
        Rc::new(move |row_id: String, window: &mut Window, cx: &mut App| {
            let Some(callback) = row_id.parse::<usize>().ok().and_then(|index| callbacks.get(index)).cloned() else {
                return;
            };
            dismiss(window, cx);
            callback(window, cx);
        });
    let on_highlight: Rc<dyn Fn(Option<String>, &mut Window, &mut App)> =
        Rc::new(move |row_id: Option<String>, _: &mut Window, cx: &mut App| {
            keys.highlight(row_id.and_then(|row_id| row_id.parse::<usize>().ok()), cx);
        });
    let list = HoldContextMenuActionList::new(
        ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static("actions")),
        actions,
        highlighted.map(|index| index.to_string()),
        on_select,
    )
    .max_width(HoldContextMenuMetrics::MENU_TITLE_MAX_WIDTH)
    .row_line_limit(2)
    .on_highlight(on_highlight);
    env_scope::<HoldContextMenuInkKey>(ink).child(list).into_any_element()
}

/// outline for a menu source that got focus from the keyboard
/// draws over the source, no room and no hitbox
pub(crate) fn context_menu_focus_ring(focus: &FocusHandle, color: Hsla, window: &Window) -> Option<Div> {
    (focus.is_focused(window) && window.last_input_was_keyboard())
        .then(|| div().absolute().inset_0().border_2().border_color(color))
}

#[derive(IntoElement)]
pub struct ContextMenu {
    id: ElementId,
    items: Vec<ContextMenuItem>,
    content: AnyElement,
    on_presented_change: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,
    opens_on_double_click: bool,
}

impl ContextMenu {
    /// called when the menu opens or closes
    /// a native view under the window hides gpui drawing so its owner steps aside while open
    pub fn on_presented_change(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_presented_change = Some(Rc::new(f));
        self
    }

    /// double click opens the menu too
    pub fn opens_on_double_click(mut self) -> Self {
        self.opens_on_double_click = true;
        self
    }
}

/// menu state, none while closed
/// plus last painted box that a keyboard opened menu hangs from
struct ContextMenuState {
    anchor: Option<Point<Pixels>>,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    reported_open: bool,
    focus: FocusHandle,
}

fn set_anchor(state: &Entity<ContextMenuState>, anchor: Option<Point<Pixels>>, cx: &mut App) {
    state.update(cx, |state: &mut ContextMenuState, cx: &mut Context<ContextMenuState>| {
        if state.anchor != anchor {
            state.anchor = anchor;
            cx.notify();
        }
    });
}

impl RenderOnce for ContextMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.items.is_empty() {
            return self.content;
        }

        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| ContextMenuState {
            anchor: None,
            bounds: Rc::default(),
            reported_open: false,
            focus: cx.focus_handle().tab_stop(true),
        });
        let anchor = state.read(cx).anchor;
        let measured = state.read(cx).bounds.clone();
        let focus = state.read(cx).focus.clone();

        let is_open = anchor.is_some();
        if state.read(cx).reported_open != is_open {
            state.update(cx, |state: &mut ContextMenuState, _: &mut Context<ContextMenuState>| {
                state.reported_open = is_open;
            });
            if let Some(on_presented_change) = self.on_presented_change.clone() {
                window.defer(cx, move |window, cx| on_presented_change(is_open, window, cx));
            }
        }

        let focus_color = {
            let store = ThemeStore::shared(cx);
            store.read(cx).current_theme().primary._100
        };
        let layer_floor = env::<DeferredLayerFloorKey>(cx);

        let (highlighted, keys) = sync_context_menu_keys(
            &self.id,
            is_open.then(|| {
                let rows = self
                    .items
                    .iter()
                    .map(|item| (!item.disabled).then(|| item.callback.clone()))
                    .collect();
                let close_state = state.clone();
                let dismiss: ContextMenuRowAction =
                    Rc::new(move |_: &mut Window, cx: &mut App| set_anchor(&close_state, None, cx));
                (rows, dismiss)
            }),
            window,
            cx,
        );
        let focus_ring = context_menu_focus_ring(&focus, focus_color, window);

        let open_on_secondary_click = {
            let state = state.clone();
            move |event: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                cx.stop_propagation();
                set_anchor(&state, Some(event.position), cx);
            }
        };
        let open_on_double_click = self.opens_on_double_click.then(|| {
            let state = state.clone();
            move |event: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                if event.button == MouseButton::Left && event.click_count >= 2 {
                    set_anchor(&state, Some(event.position), cx);
                }
            }
        });
        let open_on_key = {
            let state = state.clone();
            let measured = measured.clone();
            move |event: &KeyDownEvent, _: &mut Window, cx: &mut App| {
                if !is_context_menu_keystroke(&event.keystroke) {
                    return;
                }
                let Some(bounds) = measured.get() else {
                    return;
                };
                cx.stop_propagation();
                set_anchor(&state, Some(bounds.bottom_left()), cx);
            }
        };
        let probe = canvas(
            move |bounds: Bounds<Pixels>, _: &mut Window, _: &mut App| measured.set(Some(bounds)),
            |_: Bounds<Pixels>, (), _: &mut Window, _: &mut App| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let menu = anchor.map(|position| {
            let close_state = state.clone();
            let dismiss: ContextMenuRowAction =
                Rc::new(move |_: &mut Window, cx: &mut App| set_anchor(&close_state, None, cx));
            let panel = context_menu_panel(&self.id, &self.items, None, highlighted, keys, dismiss, cx);
            let close_state = state.clone();
            deferred(
                anchored()
                    .position(position)
                    .snap_to_window_with_margin(px(MENU_WINDOW_MARGIN))
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-context-menu", self.id)))
                            .occlude()
                            .child(panel)
                            .on_mouse_down_out(move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                                set_anchor(&close_state, None, cx);
                            }),
                    ),
            )
            .with_priority(layer_floor + 1)
        });

        div()
            .id(self.id.clone())
            .relative()
            .child(self.content)
            .child(probe)
            .child(div().absolute().size_0().track_focus(&focus))
            .children(focus_ring)
            .children(menu)
            .on_mouse_down(MouseButton::Right, open_on_secondary_click)
            .when_some(open_on_double_click, |menu, open| menu.capture_any_mouse_down(open))
            .on_key_down(open_on_key)
            .into_any_element()
    }
}
