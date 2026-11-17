use gpui::{AnyWindowHandle, App};

/// the window the picker and editor are shown from
pub enum MediaPickerPresenterResolver {}

impl MediaPickerPresenterResolver {
    /// the active window or else the first open one
    /// a gpui sheet lives inside its window so theres no presented chain to walk
    pub fn top_presenter(cx: &App) -> Option<AnyWindowHandle> {
        cx.active_window().or_else(|| cx.windows().into_iter().next())
    }
}
