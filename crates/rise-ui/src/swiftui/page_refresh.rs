use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, Entity, FocusHandle, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent, SharedString, Window};

use crate::keyboard_dismiss::ScreenFocus;
use crate::swiftui::ButtonAction;
use crate::swiftui::prelude::*;

const REFRESH_KEY: &str = "r";

pub const PAGE_REFRESH_BINDING: &str = "secondary-r";

/// true if the key press is the refresh shortcut, for pages whose key handler does more than refresh
pub fn is_page_refresh_keystroke(keystroke: &Keystroke) -> bool {
    keystroke.key == REFRESH_KEY && keystroke.modifiers.secondary()
}

/// desktop version of the status bar tap, home or cmd or ctrl up takes a page to the top
pub fn is_page_scroll_to_top_keystroke(keystroke: &Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    let plain = !modifiers.control && !modifiers.alt && !modifiers.shift && !modifiers.platform && !modifiers.function;
    (keystroke.key == "home" && plain)
        || (keystroke.key == "up" && modifiers.secondary() && !modifiers.alt && !modifiers.shift)
}

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// keyboard focus of a page, taken on appear so keys work before the first click
/// handed back on leave so the page below keeps its keys
pub struct PageFocus {
    pub handle: FocusHandle,
    previous: Option<FocusHandle>,
}

impl PageFocus {
    pub fn keyed(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Entity<Self> {
        window.use_keyed_state(id.into(), cx, |_, cx| Self::with_handle(cx.focus_handle()))
    }

    /// for a page that already owns its focus node
    pub fn keyed_with(
        id: impl Into<ElementId>,
        handle: FocusHandle,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        window.use_keyed_state(id.into(), cx, move |_, _| Self::with_handle(handle))
    }

    pub fn with_handle(handle: FocusHandle) -> Self {
        Self { handle, previous: None }
    }

    /// on a push the focus belongs to the page it covers, but a field keeps it
    /// so a feed under a search field doesnt steal the typing
    pub fn did_appear(this: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let previous = window.focused(cx);
        let handle = this.update(cx, |focus, _| {
            focus.previous = previous;
            focus.handle.clone()
        });
        ScreenFocus::claim(&handle, window, cx);
    }

    /// hands the focus back only if its still inside this page
    /// a page node may be gone from the frame already, then gpui dispatches keys from the window root
    pub fn will_disappear(this: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let (handle, previous) = this.update(cx, |focus, _| (focus.handle.clone(), focus.previous.take()));
        if let Some(previous) = &previous {
            ScreenFocus::hand_back_screen(&handle, previous, window, cx);
        }
        let holds_focus = handle.contains_focused(window, cx)
            || window.focused(cx).is_some_and(|focused| !focused.is_in_rendered_frame(window));
        if let Some(previous) = previous
            && holds_focus
        {
            window.focus(&previous, cx);
        }
    }
}

/// cmd r or ctrl r over a page
#[derive(IntoElement)]
pub struct PageRefreshKeys {
    id: ElementId,
    on_refresh: ButtonAction,
    on_scroll_to_top: Option<ButtonAction>,
    content: AnyElement,
}

impl PageRefreshKeys {
    pub fn new(
        id: impl Into<ElementId>,
        on_refresh: impl Fn(&mut Window, &mut App) + 'static,
        content: impl IntoElement,
    ) -> Self {
        Self { id: id.into(), on_refresh: Rc::new(on_refresh), on_scroll_to_top: None, content: content.into_any_element() }
    }

    /// home and cmd or ctrl up over the page, like the phone status bar tap
    /// a focused text field keeps these keys for its caret
    pub fn on_scroll_to_top(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_scroll_to_top = Some(Rc::new(action));
        self
    }
}

impl RenderOnce for PageRefreshKeys {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let page_focus = PageFocus::keyed(child_id(&id, "focus"), window, cx);
        let focus = page_focus.read(cx).handle.clone();
        let leaving = page_focus.clone();
        let on_refresh = self.on_refresh;
        let on_scroll_to_top = self.on_scroll_to_top;
        ScreenFocus::adopt_if_vacant(&focus, window, cx);
        let pressed = focus.clone();
        div()
            .id(child_id(&id, "keys"))
            .size_full()
            .track_focus(&focus)
            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| {
                if ScreenFocus::is_idle(window, cx) {
                    ScreenFocus::claim(&pressed, window, cx);
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if let Some(on_scroll_to_top) = &on_scroll_to_top
                    && is_page_scroll_to_top_keystroke(&event.keystroke)
                    && !ScreenFocus::text_field_is_focused(window, cx)
                {
                    cx.stop_propagation();
                    on_scroll_to_top(window, cx);
                    return;
                }
                if !is_page_refresh_keystroke(&event.keystroke) {
                    return;
                }
                cx.stop_propagation();
                on_refresh(window, cx);
            })
            .child(self.content)
            .on_appear(child_id(&id, "focus-appear"), move |window, cx| {
                PageFocus::did_appear(&page_focus, window, cx);
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving, window, cx);
                }
            })
    }
}
