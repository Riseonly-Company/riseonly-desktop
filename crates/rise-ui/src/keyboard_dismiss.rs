use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, DispatchPhase, Element, ElementId, Entity, GlobalElementId, InspectorElementId, LayoutId, FocusHandle, Global, InteractiveElement, IntoElement,
    KeyDownEvent, MouseDownEvent, ParentElement, Pixels, Point, RenderOnce, Styled, WeakFocusHandle, Window, WindowId,
    div,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::rise_app_events::{RiseAppEvents, RiseNotificationName};

use crate::swiftui::environment::{EnvKey, env};

const MARKER_MIN_SIDE: f32 = 0.5;
const RETURN_KEY: &str = "enter";

/// takes keyboard focus away from whatever text field holds it
pub struct KeyboardDismiss;

impl KeyboardDismiss {
    /// resigns the first responder of the key window
    pub fn perform(cx: &mut App) {
        let Some(window) = cx.active_window() else {
            return;
        };
        cx.defer(move |cx| {
            window.update(cx, |_, window, cx| ScreenFocus::release_text_field(window, cx)).ok();
        });
    }

    pub fn release_field_focus(sheet_focus: Option<&FocusHandle>, window: &mut Window, cx: &mut App) {
        match sheet_focus {
            Some(sheet_focus) => window.focus(sheet_focus, cx),
            None => ScreenFocus::release_text_field(window, cx),
        }
    }
}

#[derive(Default)]
struct ScreenFocusRegistry {
    window_roots: HashMap<WindowId, WeakFocusHandle>,
    last_screens: HashMap<WindowId, WeakFocusHandle>,
    text_fields: Vec<WeakFocusHandle>,
    sheets: Vec<RegisteredSheet>,
}

struct RegisteredSheet {
    handle: WeakFocusHandle,
    presented_at: Duration,
}

impl Global for ScreenFocusRegistry {}

/// gives a screen the key path
/// gpui sends keys only from the focused node up to the root and RootView focuses itself when
/// nothing is
pub struct ScreenFocus;

impl ScreenFocus {
    /// names the node the window falls back to, counts as holding no real focus
    pub fn set_window_root(root: &FocusHandle, window: &Window, cx: &mut App) {
        let open_windows: Vec<WindowId> = cx.windows().iter().map(|handle| handle.window_id()).collect();
        let registry = cx.default_global::<ScreenFocusRegistry>();
        registry.window_roots.retain(|window_id, _| open_windows.contains(window_id));
        registry.last_screens.retain(|window_id, _| open_windows.contains(window_id));
        registry.window_roots.insert(window.window_handle().window_id(), root.downgrade());
    }

    /// marks a focus node as a text field, the one kind of focus a screen never takes away
    pub fn register_text_field(field: &FocusHandle, cx: &mut App) {
        let registry = cx.default_global::<ScreenFocusRegistry>();
        registry.text_fields.retain(|known| known.upgrade().is_some());
        registry.text_fields.push(field.downgrade());
    }

    /// marks the focus node of a presented sheet
    /// a field inside hands focus to the sheet so the screen under it never gets keys
    /// later registrations are inner sheets
    pub fn register_sheet(sheet: &FocusHandle, cx: &mut App) {
        let registry = cx.default_global::<ScreenFocusRegistry>();
        let presented_at = match registry.sheets.last() {
            Some(last) if last.handle == *sheet => last.presented_at,
            _ => RiseClock::monotonic(),
        };
        registry.sheets.retain(|known| known.handle.upgrade().is_some() && known.handle != *sheet);
        registry.sheets.push(RegisteredSheet { handle: sheet.downgrade(), presented_at });
    }

    /// when the innermost sheet on screen in this window was presented, none with no sheet up
    pub fn top_sheet_presented_at(window: &Window, cx: &App) -> Option<Duration> {
        cx.try_global::<ScreenFocusRegistry>()?
            .sheets
            .iter()
            .rev()
            .find(|sheet| sheet.handle.upgrade().is_some_and(|handle| handle.is_in_rendered_frame(window)))
            .map(|sheet| sheet.presented_at)
    }

    /// focus node of the innermost sheet on screen in this window, none with no sheet up
    pub fn top_sheet(window: &Window, cx: &App) -> Option<FocusHandle> {
        cx.try_global::<ScreenFocusRegistry>()?
            .sheets
            .iter()
            .rev()
            .filter_map(|sheet| sheet.handle.upgrade())
            .find(|handle| handle.is_in_rendered_frame(window))
    }

    /// desktop stand in for the keyboard being up
    pub fn text_field_is_focused(window: &Window, cx: &App) -> bool {
        let Some(focused) = window.focused(cx) else {
            return false;
        };
        cx.try_global::<ScreenFocusRegistry>()
            .is_some_and(|registry| registry.text_fields.iter().any(|field| *field == focused))
    }

    /// true when nothing holds focus or only the window root does
    pub fn is_idle(window: &Window, cx: &App) -> bool {
        let Some(focused) = window.focused(cx) else {
            return true;
        };
        let window_id = window.window_handle().window_id();
        cx.try_global::<ScreenFocusRegistry>()
            .and_then(|registry| registry.window_roots.get(&window_id))
            .is_some_and(|root| *root == focused)
    }

    /// screen that last took the keys in this window, while it is still on screen
    pub fn last_screen(window: &Window, cx: &App) -> Option<FocusHandle> {
        let window_id = window.window_handle().window_id();
        cx.try_global::<ScreenFocusRegistry>()?
            .last_screens
            .get(&window_id)?
            .upgrade()
            .filter(|screen| screen.is_in_rendered_frame(window))
    }

    pub fn keep_text_field_on_press(bounds: Bounds<Pixels>, window: &Window, cx: &mut App) {
        let window_id = window.window_handle().window_id();
        let coordinator = KeyboardDismissCoordinator::shared(cx);
        if !coordinator.installed_windows.contains(&window_id) {
            return;
        }
        coordinator.markers.push(KeyboardDismissExcludeMarkerView { bounds, is_visible: true });
    }

    /// for a screen appearing or a surface being pressed
    /// takes focus from the root and the screen under it, leaving it with a text field
    pub fn claim(handle: &FocusHandle, window: &mut Window, cx: &mut App) {
        Self::remember_screen(handle, window, cx);
        if !Self::text_field_is_focused(window, cx) {
            window.focus(handle, cx);
        }
    }

    /// only a text field gives the focus up
    /// any other holder keeps it or screen keys would die on every click
    /// focus goes to the sheet around the field, else to the last screen
    pub fn release_text_field(window: &mut Window, cx: &mut App) {
        if !Self::text_field_is_focused(window, cx) {
            return;
        }
        let window_id = window.window_handle().window_id();
        let target = {
            let shown: &Window = window;
            let app: &App = cx;
            app.try_global::<ScreenFocusRegistry>().and_then(|registry| {
                registry
                    .sheets
                    .iter()
                    .rev()
                    .filter_map(|sheet| sheet.handle.upgrade())
                    .find(|sheet| sheet.contains_focused(shown, app))
                    .or_else(|| {
                        registry
                            .last_screens
                            .get(&window_id)
                            .and_then(|screen| screen.upgrade())
                            .filter(|screen| screen.is_in_rendered_frame(shown))
                    })
            })
        };
        match target {
            Some(target) => window.focus(&target, cx),
            None => window.blur(),
        }
    }

    /// a leaving page gives the last screen place back to whatever held focus before it
    pub fn hand_back_screen(leaving: &FocusHandle, previous: &FocusHandle, window: &Window, cx: &mut App) {
        let window_id = window.window_handle().window_id();
        let is_last_screen = cx
            .try_global::<ScreenFocusRegistry>()
            .and_then(|registry| registry.last_screens.get(&window_id))
            .is_some_and(|screen| *screen == *leaving);
        if is_last_screen {
            Self::remember_screen(previous, window, cx);
        }
    }

    pub fn return_to_last_screen(leaving: &FocusHandle, window: &mut Window, cx: &mut App) {
        if !layer_holds_focus(leaving, window, cx) {
            return;
        }
        let window_id = window.window_handle().window_id();
        let screen = {
            let shown: &Window = window;
            cx.try_global::<ScreenFocusRegistry>()
                .and_then(|registry| registry.last_screens.get(&window_id))
                .and_then(|screen| screen.upgrade())
                .filter(|screen| screen.is_in_rendered_frame(shown))
        };
        match screen {
            Some(screen) => window.focus(&screen, cx),
            None => window.blur(),
        }
    }

    pub fn adopt_if_vacant(handle: &FocusHandle, window: &Window, cx: &mut App) {
        if Self::last_screen(window, cx).is_some_and(|screen| !Self::is_window_root(&screen, window, cx)) {
            return;
        }
        Self::remember_screen(handle, window, cx);
    }

    fn is_window_root(handle: &FocusHandle, window: &Window, cx: &App) -> bool {
        let window_id = window.window_handle().window_id();
        cx.try_global::<ScreenFocusRegistry>()
            .and_then(|registry| registry.window_roots.get(&window_id))
            .is_some_and(|root| *root == *handle)
    }

    fn remember_screen(handle: &FocusHandle, window: &Window, cx: &mut App) {
        if Self::is_window_root(handle, window, cx) {
            return;
        }
        let window_id = window.window_handle().window_id();
        let registry = cx.default_global::<ScreenFocusRegistry>();
        if registry.text_fields.iter().any(|field| *field == *handle) {
            return;
        }
        registry.last_screens.insert(window_id, handle.downgrade());
    }
}

/// node of a leaving layer may already be out of the frame where contains_focused is false
/// so a focus outside the frame counts as the layer
fn layer_holds_focus(leaving: &FocusHandle, window: &Window, cx: &App) -> bool {
    leaving.contains_focused(window, cx)
        || window.focused(cx).is_some_and(|focused| !focused.is_in_rendered_frame(window))
}

/// what a modal layer hands keyboard focus back to
/// unlike a page a layer takes focus from a text field too, its escape has to work at once
#[derive(Default)]
pub struct LayerFocus {
    previous: Option<FocusHandle>,
}

impl LayerFocus {
    pub fn keyed(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Entity<Self> {
        window.use_keyed_state(id.into(), cx, |_, _| Self::default())
    }

    pub fn did_appear(this: &Entity<Self>, handle: &FocusHandle, window: &mut Window, cx: &mut App) {
        let previous = window.focused(cx).filter(|focused| focused != handle);
        this.update(cx, |focus, _| focus.previous = previous);
        window.focus(handle, cx);
    }

    /// only a focus still on the layer moves, its action may have opened a screen that already claimed it
    /// with nothing to return to RootView takes focus on its next render
    pub fn will_disappear(this: &Entity<Self>, handle: &FocusHandle, window: &mut Window, cx: &mut App) {
        let previous = this.update(cx, |focus, _| focus.previous.take());
        if !layer_holds_focus(handle, window, cx) {
            return;
        }
        match previous.filter(|previous| previous.is_in_rendered_frame(window)) {
            Some(previous) => window.focus(&previous, cx),
            None => window.blur(),
        }
    }
}

/// focus node of the sheet around this content, set by the sheet hosts
pub struct SheetFocusKey;

impl EnvKey for SheetFocusKey {
    type Value = Option<FocusHandle>;

    fn default_value() -> Self::Value {
        None
    }
}

/// content of a sheet that a keyboard press sends
/// secondary modifier sends from anywhere inside, bare Return only with focus on this surface
#[derive(IntoElement)]
pub struct SheetConfirmSurface {
    id: ElementId,
    content: AnyElement,
    on_confirm: Box<dyn Fn(&mut Window, &mut App)>,
}

impl SheetConfirmSurface {
    pub fn new(
        id: impl Into<ElementId>,
        content: impl IntoElement,
        on_confirm: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            content: content.into_any_element(),
            on_confirm: Box::new(on_confirm),
        }
    }
}

impl RenderOnce for SheetConfirmSurface {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let surface: Entity<FocusHandle> = window.use_keyed_state(self.id, cx, |_, cx| cx.focus_handle());
        let surface = surface.read(cx).clone();
        if env::<SheetFocusKey>(cx).is_some_and(|sheet| sheet.is_focused(window)) {
            window.focus(&surface, cx);
        }
        let on_confirm = self.on_confirm;
        let key_surface = surface.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&surface)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let modifiers = &event.keystroke.modifiers;
                if event.keystroke.key != RETURN_KEY
                    || !(modifiers.secondary() || (!modifiers.modified() && key_surface.is_focused(window)))
                {
                    return;
                }
                cx.stop_propagation();
                on_confirm(window, cx);
            })
            .child(self.content)
    }
}

pub const RISE_INPUT_PICKER_DISMISS_REQUESTED: RiseNotificationName =
    RiseNotificationName::new("riseonly.inputPickerDismissRequested");

static EDIT_MENU_IS_VISIBLE: AtomicBool = AtomicBool::new(false);

/// true while a text edit menu is on screen, the click that closes it must not also dismiss
pub struct EditMenuPresentation;

impl EditMenuPresentation {
    pub fn is_visible() -> bool {
        EDIT_MENU_IS_VISIBLE.load(Ordering::Relaxed)
    }

    pub fn set_is_visible(is_visible: bool) {
        EDIT_MENU_IS_VISIBLE.store(is_visible, Ordering::Relaxed);
    }
}

/// one opted out region as the click saw it, window coordinates
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyboardDismissExcludeMarkerView {
    pub bounds: Bounds<Pixels>,
    pub is_visible: bool,
}

/// whether a click landed inside an opted out region
pub struct KeyboardDismissExclusionHitTest;

impl KeyboardDismissExclusionHitTest {
    pub fn contains(point: Point<Pixels>, markers: &[KeyboardDismissExcludeMarkerView]) -> bool {
        markers.iter().any(|marker| {
            marker.is_visible
                && f32::from(marker.bounds.size.width) > MARKER_MIN_SIDE
                && f32::from(marker.bounds.size.height) > MARKER_MIN_SIDE
                && marker.bounds.contains(&point)
        })
    }
}

/// window level click state
/// windows that installed it and the opted out regions the current click reported
#[derive(Default)]
struct KeyboardDismissCoordinator {
    installed_windows: HashSet<WindowId>,
    markers: Vec<KeyboardDismissExcludeMarkerView>,
}

impl Global for KeyboardDismissCoordinator {}

impl KeyboardDismissCoordinator {
    fn shared(cx: &mut App) -> &mut Self {
        cx.default_global::<Self>()
    }

    fn handle_tap(window: &mut Window, cx: &mut App) {
        ScreenFocus::release_text_field(window, cx);
        RiseAppEvents::default_center().post(RISE_INPUT_PICKER_DISMISS_REQUESTED);
    }

    fn gesture_recognizer(
        point: Point<Pixels>,
        markers: &[KeyboardDismissExcludeMarkerView],
        focus_moved_into_text_field: bool,
    ) -> bool {
        if EditMenuPresentation::is_visible() || focus_moved_into_text_field {
            return false;
        }
        !KeyboardDismissExclusionHitTest::contains(point, markers)
    }
}

pub trait KeyboardDismissModifiers: IntoElement + Sized {
    fn keyboard_dismiss_excluded(self) -> KeyboardDismissExcluded {
        KeyboardDismissExcluded {
            content: self.into_any_element(),
        }
    }

    /// put once on the root view of a window
    fn install_keyboard_dismiss_on_tap(self) -> KeyboardDismissInstaller {
        KeyboardDismissInstaller {
            content: div().size_full().child(self).into_any_element(),
        }
    }
}

impl<E: IntoElement> KeyboardDismissModifiers for E {}

/// the element keyboard_dismiss_excluded returns, adds no layout box
/// a wrapper div would hug content in a centering stack and squeeze a full width row
pub struct KeyboardDismissExcluded {
    content: AnyElement,
}

impl IntoElement for KeyboardDismissExcluded {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for KeyboardDismissExcluded {
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
        (self.content.request_layout(window, cx), ())
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
        self.content.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let window_id = window.window_handle().window_id();
        if KeyboardDismissCoordinator::shared(cx).installed_windows.contains(&window_id) {
            window.on_mouse_event(move |_: &MouseDownEvent, phase, _, cx| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                KeyboardDismissCoordinator::shared(cx)
                    .markers
                    .push(KeyboardDismissExcludeMarkerView { bounds, is_visible: true });
            });
        }
        self.content.paint(window, cx);
    }
}

/// the element install_keyboard_dismiss_on_tap returns
/// a click anywhere not on a text field or opted out region drops text field focus and folds
/// the input pickers
pub struct KeyboardDismissInstaller {
    content: AnyElement,
}

impl IntoElement for KeyboardDismissInstaller {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for KeyboardDismissInstaller {
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
        let window_id = window.window_handle().window_id();
        KeyboardDismissCoordinator::shared(cx)
            .installed_windows
            .insert(window_id);
        (self.content.request_layout(window, cx), ())
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
        self.content.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture || !bounds.contains(&event.position) {
                return;
            }
            let point = event.position;
            let focused_before = window.focused(cx);
            window.defer(cx, move |window, cx| {
                let markers = std::mem::take(&mut KeyboardDismissCoordinator::shared(cx).markers);
                let focus_moved_into_text_field =
                    window.focused(cx) != focused_before && ScreenFocus::text_field_is_focused(window, cx);
                if KeyboardDismissCoordinator::gesture_recognizer(point, &markers, focus_moved_into_text_field) {
                    KeyboardDismissCoordinator::handle_tap(window, cx);
                }
            });
        });
        self.content.paint(window, cx);
    }
}
