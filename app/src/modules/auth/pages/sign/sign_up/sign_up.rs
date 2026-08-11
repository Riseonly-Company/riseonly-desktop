use gpui::{AppContext, Context, IntoElement, Render, Window};

use crate::modules::auth::pages::sign::auth_step_flow::{AuthFlowEvent, AuthMode, AuthStepFlow};
use crate::modules::auth::stores::auth_interactions::AuthInteractionsStore;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignUpEvent {
    Dismissed,
}

pub struct SignUp {
    flow: gpui::Entity<AuthStepFlow>,
    _subscription: gpui::Subscription,
}

impl gpui::EventEmitter<SignUpEvent> for SignUp {}

impl SignUp {
    pub fn new(
        interactions: AuthInteractionsStore,
        can_dismiss: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let flow =
            cx.new(|cx| AuthStepFlow::new(AuthMode::SignUp, can_dismiss, interactions, window, cx));
        let subscription = cx.subscribe(&flow, |_, _, event, cx| match event {
            AuthFlowEvent::Dismissed => cx.emit(SignUpEvent::Dismissed),
        });

        Self {
            flow,
            _subscription: subscription,
        }
    }
}

impl Render for SignUp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.flow.clone()
    }
}
