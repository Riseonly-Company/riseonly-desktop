use gpui::{AppContext, Context, IntoElement, Render, Window};

use crate::modules::auth::pages::sign::auth_step_flow::{AuthMode, AuthStepFlow};
use crate::modules::auth::stores::auth_interactions::AuthInteractionsStore;

/// Sign-up is the same step flow in its registration mode. The phone step is
/// shared; the mode is what decides whether the next step asks for a password or
/// starts collecting a profile.
pub struct SignUp {
    flow: gpui::Entity<AuthStepFlow>,
}

impl SignUp {
    pub fn new(
        interactions: AuthInteractionsStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            flow: cx.new(|cx| AuthStepFlow::new(AuthMode::SignUp, interactions, window, cx)),
        }
    }
}

impl Render for SignUp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.flow.clone()
    }
}
