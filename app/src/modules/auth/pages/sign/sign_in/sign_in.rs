use gpui::{AppContext, Context, IntoElement, Render, Window};

use crate::modules::auth::pages::sign::auth_step_flow::{AuthMode, AuthStepFlow};
use crate::modules::auth::stores::auth_interactions::AuthInteractionsStore;

/// Sign-in is the step flow in its login mode, exactly as in the reference,
/// where `SignIn.swift` is eleven lines around `AuthStepFlow(mode: .login)`.
pub struct SignIn {
    flow: gpui::Entity<AuthStepFlow>,
}

impl SignIn {
    pub fn new(
        interactions: AuthInteractionsStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            flow: cx.new(|cx| AuthStepFlow::new(AuthMode::SignIn, interactions, window, cx)),
        }
    }
}

impl Render for SignIn {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.flow.clone()
    }
}
