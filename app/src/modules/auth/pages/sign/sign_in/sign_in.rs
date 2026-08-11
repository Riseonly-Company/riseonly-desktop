use gpui::{AppContext, Context, IntoElement, Render, Window};

use crate::modules::auth::pages::sign::auth_step_flow::{AuthFlowEvent, AuthMode, AuthStepFlow};
use crate::modules::auth::stores::auth_interactions::AuthInteractionsStore;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignInEvent {
    Dismissed,
}

pub struct SignIn {
    flow: gpui::Entity<AuthStepFlow>,
    _subscription: gpui::Subscription,
}

impl gpui::EventEmitter<SignInEvent> for SignIn {}

impl SignIn {
    pub fn new(
        interactions: AuthInteractionsStore,
        can_dismiss: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let flow =
            cx.new(|cx| AuthStepFlow::new(AuthMode::SignIn, can_dismiss, interactions, window, cx));
        let subscription = cx.subscribe(&flow, |_, _, event, cx| match event {
            AuthFlowEvent::Dismissed => cx.emit(SignInEvent::Dismissed),
        });

        Self {
            flow,
            _subscription: subscription,
        }
    }
}

impl Render for SignIn {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.flow.clone()
    }
}
