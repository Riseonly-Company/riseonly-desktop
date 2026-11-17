use std::collections::HashMap;

use gpui::{AnyElement, App, ElementId, Global, IntoElement, ParentElement, RenderOnce, Window, WindowId};
use rise_platform::capture_protection::set_window_excluded_from_capture;

use crate::swiftui::lifecycle::LifecycleModifiers;
use crate::swiftui::shapes::pass_through;

/// content that screenshots and screen recordings must not capture
/// a desktop compositor only excludes whole windows so the window is left out
/// if the host cant do that it stays inline
#[derive(IntoElement)]
pub struct ScreenCaptureProtectedUi {
    id: ElementId,
    is_protected: bool,
    content: AnyElement,
}

impl ScreenCaptureProtectedUi {
    pub fn new(id: impl Into<ElementId>, is_protected: bool, content: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            is_protected,
            content: content.into_any_element(),
        }
    }
}

impl RenderOnce for ScreenCaptureProtectedUi {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        if !self.is_protected {
            return self.content;
        }
        pass_through()
            .child(self.content)
            .on_appear(self.id, |window, cx| {
                ScreenCaptureProtection::claim(window, cx);
            })
            .and_on_disappear(|window, cx| {
                if let Some(window) = window {
                    ScreenCaptureProtection::release(window, cx);
                }
            })
            .into_any_element()
    }
}

/// how many protected views each window shows, the window stays out of captures while any is
#[derive(Default)]
struct ScreenCaptureProtection {
    claims: HashMap<WindowId, usize>,
}

impl Global for ScreenCaptureProtection {}

impl ScreenCaptureProtection {
    fn claim(window: &mut Window, cx: &mut App) {
        let id = window.window_handle().window_id();
        let registry = cx.default_global::<Self>();
        let count = registry.claims.entry(id).or_insert(0);
        *count += 1;
        if *count == 1 {
            set_window_excluded_from_capture(window, true);
        }
    }

    fn release(window: &mut Window, cx: &mut App) {
        let id = window.window_handle().window_id();
        let registry = cx.default_global::<Self>();
        let Some(count) = registry.claims.get_mut(&id) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            registry.claims.remove(&id);
            set_window_excluded_from_capture(window, false);
        }
    }
}
