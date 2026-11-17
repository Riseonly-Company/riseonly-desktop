use std::cell::Cell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui::{
    Anchor, AnyElement, App, Bounds, ClickEvent, Context, ElementId, Entity, FocusHandle, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, anchored, canvas, deferred, div, px,
};
use rise_theme::ThemeStore;

use crate::guide_anchor_ui::guide_anchor_registry::{GuideAnchorCoverKind, GuideAnchorCoverModifiers};
use crate::swiftui::context_menu::{
    ContextMenuRowAction, MENU_WINDOW_MARGIN, context_menu_focus_ring, context_menu_panel, is_context_menu_keystroke,
    sync_context_menu_keys,
};
use crate::swiftui::environment::{EnvKey, EnvScope, env, env_scope};

/// what tint a native menu gets, it colors the row glyphs and never the label
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeMenuTint {
    Primary,
    SystemRed,
}

/// deferred priority of the window layer the subtree draws in
/// a menu floats one above it
/// a fixed low priority drew a menu opened in a higher layer (media editor) under it
pub struct DeferredLayerFloorKey;

impl EnvKey for DeferredLayerFloorKey {
    type Value = usize;

    fn default_value() -> Self::Value {
        0
    }
}

/// tint as a native menu inside the subtree reads it, None is the app accent
pub struct NativeMenuTintKey;

impl EnvKey for NativeMenuTintKey {
    type Value = Option<NativeMenuTint>;

    fn default_value() -> Self::Value {
        None
    }
}

pub trait ContextMenuUiModifiers: IntoElement + Sized {
    fn native_menu_icons_match_label(self) -> EnvScope<NativeMenuTintKey> {
        env_scope::<NativeMenuTintKey>(Some(NativeMenuTint::Primary)).child(self)
    }

    fn native_menu_destructive_icon_matches_label(self) -> EnvScope<NativeMenuTintKey> {
        env_scope::<NativeMenuTintKey>(Some(NativeMenuTint::SystemRed)).child(self)
    }

    fn native_menu_label_follows_appearance(self) -> Self {
        self
    }
}

impl<E: IntoElement> ContextMenuUiModifiers for E {}

/// one row of a ContextMenuUi, equal by id and key since the callback cant be compared
#[derive(Clone)]
pub struct ContextMenuItem {
    pub id: i64,
    pub label: String,
    pub icon: Option<String>,
    pub key: String,
    pub callback: Rc<dyn Fn(&mut Window, &mut App)>,
    pub destructive: bool,
    pub icon_tint: Option<NativeMenuTint>,
    pub disabled: bool,
    pub begins_section: bool,
}

impl ContextMenuItem {
    pub fn new(
        id: i64,
        label: impl Into<String>,
        icon: Option<String>,
        key: impl Into<String>,
        callback: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id,
            label: label.into(),
            icon,
            key: key.into(),
            callback: Rc::new(callback),
            destructive: false,
            icon_tint: None,
            disabled: false,
            begins_section: false,
        }
    }

    pub fn begins_section(mut self, begins_section: bool) -> Self {
        self.begins_section = begins_section;
        self
    }

    pub fn destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn native_menu_destructive_icon_matches_label(mut self) -> Self {
        self.icon_tint = Some(NativeMenuTint::SystemRed);
        self
    }
}

impl PartialEq for ContextMenuItem {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.key == other.key
    }
}

impl Eq for ContextMenuItem {}

impl Hash for ContextMenuItem {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.key.hash(state);
    }
}

type PresentedChange = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// where the open menu stands, None while closed
#[derive(Default)]
struct ContextMenuPresentation {
    anchor: Option<(Point<Pixels>, Anchor)>,
    label_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

/// a label that opens an item list, the row whose key equals selected gets a checkmark
/// on desktop a click or right click opens it at the cursor, a click outside closes it
/// same panel as the chat hold menu
#[derive(IntoElement)]
pub struct ContextMenuUi {
    id: ElementId,
    items: Vec<ContextMenuItem>,
    selected: Option<String>,
    label: AnyElement,
    on_presented_change: Option<PresentedChange>,
}

impl ContextMenuUi {
    pub fn new(id: impl Into<ElementId>, items: Vec<ContextMenuItem>, label: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            items,
            selected: None,
            label: label.into_any_element(),
            on_presented_change: None,
        }
    }

    pub fn selected(mut self, selected: impl Into<Option<String>>) -> Self {
        self.selected = selected.into();
        self
    }

    /// draws each row icon
    /// the chat panel always draws them so this is the default and changes nothing
    pub fn shows_item_icons(self) -> Self {
        self
    }

    /// a tap outside an open menu closes it and also reaches the view under
    /// a screen that must not act on that tap watches the menu here
    pub fn on_presented_change(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_presented_change = Some(Rc::new(f));
        self
    }
}

/// where a list opened from the keyboard hangs, under the label
/// growing away from the nearer window edge
fn label_anchor(bounds: Bounds<Pixels>, window: &Window) -> (Point<Pixels>, Anchor) {
    let half_width = px(f32::from(window.viewport_size().width) * 0.5);
    if bounds.center().x > half_width {
        (bounds.bottom_right(), Anchor::TopRight)
    } else {
        (bounds.bottom_left(), Anchor::TopLeft)
    }
}

fn present(
    presentation: &Entity<ContextMenuPresentation>,
    anchor: Option<(Point<Pixels>, Anchor)>,
    on_presented_change: Option<&PresentedChange>,
    window: &mut Window,
    cx: &mut App,
) {
    let was_open = presentation.read(cx).anchor.is_some();
    presentation.update(cx, |state: &mut ContextMenuPresentation, cx: &mut Context<ContextMenuPresentation>| {
        state.anchor = anchor;
        cx.notify();
    });
    let is_open = anchor.is_some();
    if was_open != is_open
        && let Some(on_presented_change) = on_presented_change
    {
        on_presented_change(is_open, window, cx);
    }
}

impl RenderOnce for ContextMenuUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let presentation = window.use_keyed_state(self.id.clone(), cx, |_, _| {
            ContextMenuPresentation::default()
        });
        let anchor = presentation.read(cx).anchor;
        let label_bounds = presentation.read(cx).label_bounds.clone();
        let focus = window
            .use_keyed_state::<FocusHandle>(
                ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), SharedString::new_static("focus")),
                cx,
                |_, cx| cx.focus_handle().tab_stop(true),
            )
            .read(cx)
            .clone();

        let accent = {
            let store = ThemeStore::shared(cx);
            store.read(cx).current_theme().primary._100
        };
        let layer_floor = env::<DeferredLayerFloorKey>(cx);

        let dismiss: ContextMenuRowAction = {
            let presentation = presentation.clone();
            let on_presented_change = self.on_presented_change.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                present(&presentation, None, on_presented_change.as_ref(), window, cx);
            })
        };
        let (highlighted, keys) = sync_context_menu_keys(
            &self.id,
            anchor.is_some().then(|| {
                let rows = self
                    .items
                    .iter()
                    .map(|item| (!item.disabled).then(|| item.callback.clone()))
                    .collect();
                (rows, dismiss.clone())
            }),
            window,
            cx,
        );
        let focus_ring = context_menu_focus_ring(&focus, accent, window);

        let open_on_click = {
            let presentation = presentation.clone();
            let on_presented_change = self.on_presented_change.clone();
            let label_bounds = label_bounds.clone();
            move |event: &ClickEvent, window: &mut Window, cx: &mut App| {
                let anchor = presentation.read(cx).anchor.is_none().then(|| match label_bounds.get() {
                    Some(bounds) if event.is_keyboard() => label_anchor(bounds, window),
                    _ => (event.position(), Anchor::TopLeft),
                });
                present(
                    &presentation,
                    anchor,
                    on_presented_change.as_ref(),
                    window,
                    cx,
                );
            }
        };
        let open_on_right_click = {
            let presentation = presentation.clone();
            let on_presented_change = self.on_presented_change.clone();
            move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                cx.stop_propagation();
                present(
                    &presentation,
                    Some((event.position, Anchor::TopLeft)),
                    on_presented_change.as_ref(),
                    window,
                    cx,
                );
            }
        };

        let open_on_key = {
            let presentation = presentation.clone();
            let on_presented_change = self.on_presented_change.clone();
            let label_bounds = label_bounds.clone();
            move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                if !is_context_menu_keystroke(&event.keystroke) {
                    return;
                }
                let Some(bounds) = label_bounds.get() else {
                    return;
                };
                cx.stop_propagation();
                present(
                    &presentation,
                    Some(label_anchor(bounds, window)),
                    on_presented_change.as_ref(),
                    window,
                    cx,
                );
            }
        };

        let close_label_bounds = label_bounds.clone();
        let label = div()
            .id(self.id.clone())
            .cursor_pointer()
            .track_focus(&focus)
            .on_any_mouse_down(|_: &MouseDownEvent, window: &mut Window, _: &mut App| window.prevent_default())
            .child(self.label)
            .child(
                canvas(
                    move |bounds: Bounds<Pixels>, _: &mut Window, _: &mut App| label_bounds.set(Some(bounds)),
                    |_: Bounds<Pixels>, (), _: &mut Window, _: &mut App| {},
                )
                .absolute()
                .size_full(),
            )
            .children(focus_ring)
            .on_key_down(open_on_key)
            .on_click(open_on_click)
            .on_mouse_down(MouseButton::Right, open_on_right_click);

        let menu = anchor.map(|(position, corner)| {
            let panel = context_menu_panel(
                &self.id,
                &self.items,
                self.selected.as_deref(),
                highlighted,
                keys,
                dismiss,
                cx,
            );
            let close_presentation = presentation.clone();
            let close_on_presented_change = self.on_presented_change.clone();
            deferred(
                anchored()
                    .position(position)
                    .anchor(corner)
                    .snap_to_window_with_margin(px(MENU_WINDOW_MARGIN))
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-menu", self.id)))
                            .occlude()
                            .child(panel)
                            .on_mouse_down_out(move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                                if close_label_bounds.get().is_some_and(|bounds| bounds.contains(&event.position)) {
                                    return;
                                }
                                present(
                                    &close_presentation,
                                    None,
                                    close_on_presented_change.as_ref(),
                                    window,
                                    cx,
                                );
                            })
                            .guide_anchor_cover(
                                SharedString::from(format!("{}-guide-menu", self.id)),
                                GuideAnchorCoverKind::Menu,
                            ),
                    ),
            )
            .with_priority(layer_floor + 1)
        });

        div().child(label).children(menu)
    }
}
