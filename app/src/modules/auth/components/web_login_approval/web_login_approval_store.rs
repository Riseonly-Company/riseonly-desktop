use gpui::{App, Context, Entity, Task};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::auth::engine::rise_web_login_approval_rpc::RiseWebLoginApprovalRefusal;
use crate::modules::auth::stores::auth_actions::auth_actions::AuthActionsStore;
use crate::modules::auth::stores::auth_actions::profile_model_hooks::ProfileModel;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

pub struct WebLoginApprovalStore {
    is_presented: bool,
    login_token: Option<String>,
    is_approving: bool,
    is_denying: bool,
    error_text: Option<String>,
    approval_task: Option<Task<()>>,
    request_generation: Uuid,
}

impl WebLoginApprovalStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            is_presented: false,
            login_token: None,
            is_approving: false,
            is_denying: false,
            error_text: None,
            approval_task: None,
            request_generation: Uuid::new_v4(),
        }
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    pub fn login_token(&self) -> Option<&str> {
        self.login_token.as_deref()
    }

    pub fn is_approving(&self) -> bool {
        self.is_approving
    }

    pub fn is_denying(&self) -> bool {
        self.is_denying
    }

    pub fn error_text(&self) -> Option<&str> {
        self.error_text.as_deref()
    }

    pub fn profile(&self, cx: &mut App) -> Option<ProfileModel> {
        AuthServicesStore::shared(cx).read(cx).profile().cloned()
    }

    pub fn present(&mut self, login_token: &str, cx: &mut Context<Self>) {
        let token = trimming_whitespaces_and_newlines(login_token);
        if token.is_empty() {
            return;
        }
        self.reset_account_state(cx);
        self.login_token = Some(token.to_owned());
        self.is_approving = false;
        self.is_denying = false;
        self.error_text = None;
        self.is_presented = true;
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.reset_account_state(cx);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.request_generation = Uuid::new_v4();
        self.approval_task = None;
        self.is_presented = false;
        self.login_token = None;
        self.is_approving = false;
        self.is_denying = false;
        self.error_text = None;
        cx.notify();
    }

    pub fn approve(&mut self, cx: &mut Context<Self>) {
        self.send(true, cx);
    }

    pub fn deny(&mut self, cx: &mut Context<Self>) {
        self.send(false, cx);
    }

    fn send(&mut self, approve: bool, cx: &mut Context<Self>) {
        if !self.is_presented || self.is_approving || self.is_denying {
            return;
        }
        let Some(token) = self.login_token.clone() else {
            return;
        };
        let Some(account_id) = self.profile(cx).map(|profile| profile.id).filter(|id| !id.is_empty()) else {
            return;
        };
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let generation = Uuid::new_v4();
        self.request_generation = generation;
        self.error_text = None;
        if approve {
            self.is_approving = true;
        } else {
            self.is_denying = true;
        }
        cx.notify();

        let request = AuthActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.approve_web_login_action(&token, approve, &account_id, account_epoch.as_deref(), cx)
        });
        self.approval_task = Some(cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |store, cx| {
                if !store.owns_request(generation, &account_id, account_epoch.as_deref()) {
                    return;
                }
                let error = match result {
                    Ok(()) => {
                        store.reset_account_state(cx);
                        return;
                    }
                    Err(error) => error,
                };
                if !approve && error.downcast_ref::<RiseWebLoginApprovalRefusal>().is_some() {
                    store.reset_account_state(cx);
                    return;
                }
                store.is_approving = false;
                store.is_denying = false;
                store.approval_task = None;
                store.error_text = Some(Self::failure_text(&error));
                cx.notify();
            })
            .ok();
        }));
    }

    fn failure_text(error: &anyhow::Error) -> String {
        match error.downcast_ref::<RiseWebLoginApprovalRefusal>() {
            Some(RiseWebLoginApprovalRefusal::Expired) => tr("weblogin_expired"),
            Some(RiseWebLoginApprovalRefusal::Refused) => tr("retry_later_error"),
            None => CanonErrors::load_failure_text(error),
        }
    }

    fn owns_request(&self, generation: Uuid, account_id: &str, account_epoch: Option<&str>) -> bool {
        self.is_presented
            && self.request_generation == generation
            && SaiAccountCacheScope::is_current_account_epoch(Some(account_id), account_epoch)
    }
}
