use std::rc::Rc;

use gpui::{App, Window};
use rise_core::foundation::foundation_url::Url;

use super::environment::{EnvKey, env};

/// result of an open url action
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OpenURLActionResult {
    Handled,
    Discarded,
    SystemAction,
    SystemActionUrl(Url),
}

type Handler = Rc<dyn Fn(&Url, &mut Window, &mut App) -> OpenURLActionResult>;

/// url handler, without one it asks the system
#[derive(Clone, Default)]
pub struct OpenURLAction {
    handler: Option<Handler>,
}

impl OpenURLAction {
    pub fn new(
        handler: impl Fn(&Url, &mut Window, &mut App) -> OpenURLActionResult + 'static,
    ) -> Self {
        Self {
            handler: Some(Rc::new(handler)),
        }
    }

    pub fn call_as_function(&self, url: &Url, window: &mut Window, cx: &mut App) {
        self.call_as_function_completion(url, window, cx, |_| {});
    }

    /// opens the url and tells if it was accepted
    pub fn call_as_function_completion(
        &self,
        url: &Url,
        window: &mut Window,
        cx: &mut App,
        completion: impl FnOnce(bool),
    ) {
        let result = match self.handler.clone() {
            Some(handler) => handler(url, window, cx),
            None => OpenURLActionResult::SystemAction,
        };
        let accepted = match result {
            OpenURLActionResult::Handled => true,
            OpenURLActionResult::Discarded => false,
            OpenURLActionResult::SystemAction => rise_platform::open_url::open_url(url, cx),
            OpenURLActionResult::SystemActionUrl(other) => {
                rise_platform::open_url::open_url(&other, cx)
            }
        };
        completion(accepted);
    }
}

/// open url key, a host wraps its content in an env_scope with the action
pub struct OpenURLKey;

impl EnvKey for OpenURLKey {
    type Value = OpenURLAction;

    fn default_value() -> Self::Value {
        OpenURLAction::default()
    }
}

/// open url action from the environment
pub fn open_url(cx: &App) -> OpenURLAction {
    env::<OpenURLKey>(cx)
}
