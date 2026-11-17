use std::rc::Rc;

use gpui::prelude::*;
use gpui::{App, Window};

use crate::modules::auth::pages::sign::auth_step_flow::{
    AuthStepFlow, AuthStepFlowAction, AuthStepFlowModelPurpose,
};

#[derive(IntoElement, Default)]
pub struct SignUp {
    pub on_go_to_sign_in: Option<AuthStepFlowAction>,
    pub on_auth: Option<AuthStepFlowAction>,
    pub on_leave: Option<AuthStepFlowAction>,
}

impl RenderOnce for SignUp {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let on_auth: AuthStepFlowAction = match self.on_auth {
            Some(on_auth) => on_auth,
            None => Rc::new(|_: &mut Window, _: &mut App| {}),
        };
        let on_leave = self.on_leave;
        window.use_keyed_state("sign-up-auth-step-flow", cx, move |window, cx| {
            AuthStepFlow::new(AuthStepFlowModelPurpose::Registration, on_auth, None, window, cx).with_on_leave(on_leave)
        })
    }
}
