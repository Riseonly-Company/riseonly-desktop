use std::rc::Rc;

use gpui::{App, Window};

use super::environment::{EnvKey, env};

type DismissHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Default)]
pub struct DismissAction {
    action: Option<DismissHandler>,
}

impl DismissAction {
    pub fn new(action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            action: Some(Rc::new(action)),
        }
    }

    /// dismisses the presentation, does nothing outside one
    pub fn call_as_function(&self, window: &mut Window, cx: &mut App) {
        if let Some(action) = &self.action {
            action(window, cx);
        }
    }
}

/// env key for dismiss, the host wraps its content with the dismiss action
pub struct DismissKey;

impl EnvKey for DismissKey {
    type Value = DismissAction;

    fn default_value() -> Self::Value {
        DismissAction::default()
    }
}

pub fn dismiss(cx: &App) -> DismissAction {
    env::<DismissKey>(cx)
}
