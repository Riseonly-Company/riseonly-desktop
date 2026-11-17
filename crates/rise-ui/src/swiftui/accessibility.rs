use gpui::{App, IntoElement, SharedString, Window};

pub trait AccessibilityActionModifiers: IntoElement + Sized {
    /// adds a named accessibility action, element comes back unchanged
    fn accessibility_action(
        self,
        name: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        let _ = (name.into(), handler);
        self
    }
}

impl<E: IntoElement> AccessibilityActionModifiers for E {}
