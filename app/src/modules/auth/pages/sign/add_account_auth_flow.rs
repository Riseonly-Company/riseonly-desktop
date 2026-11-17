use std::rc::Rc;

use gpui::prelude::*;
use gpui::{App, Context, Window, div};

use crate::core::widgets::notifier::notifier_host::NotifierHost;
use crate::modules::auth::pages::sign::sign_in::sign_in::SignIn;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

#[derive(IntoElement, Default)]
pub struct AddAccountAuthFlow;

impl RenderOnce for AddAccountAuthFlow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let notifier = window.use_keyed_state("add-account-notifier-host", cx, |_, cx| NotifierHost::new(cx));
        div()
            .relative()
            .size_full()
            .child(SignIn {
                on_close: Some(Rc::new(|_: &mut Window, cx: &mut App| {
                    AuthServicesStore::shared(cx).update(cx, |auth, cx| auth.cancel_adding_account(cx));
                })),
                ..SignIn::default()
            })
            .child(div().absolute().inset_0().child(notifier))
    }
}

/// add account cover as its own view so its notifies stay off the root columns
pub struct AddAccountAuthFlowView;

impl Render for AddAccountAuthFlowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        AddAccountAuthFlow
    }
}
