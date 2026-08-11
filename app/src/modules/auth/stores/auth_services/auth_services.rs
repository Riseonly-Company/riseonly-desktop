use std::sync::Arc;

use gpui::{App, Context, Entity};

use crate::modules::auth::engine::rise_auth_domain::RiseAuthDomain;
use crate::modules::auth::engine::rise_auth_engine_models::{AccountSummary, AuthUser};
use crate::modules::auth::engine::rise_auth_presentation::{
    AuthFlowState, AuthSnapshot, TagAvailability,
};

pub struct AuthServicesStore {
    snapshot: Arc<AuthSnapshot>,
}

impl AuthServicesStore {
    pub fn new(domain: Arc<RiseAuthDomain>, cx: &mut Context<Self>) -> Self {
        let mut updates = domain.subscribe();

        cx.spawn(async move |this, cx| {
            while updates.changed().await.is_ok() {
                let next = Arc::clone(&updates.borrow_and_update());
                if this
                    .update(cx, |store, cx| {
                        store.snapshot = next;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        Self {
            snapshot: domain.snapshot(),
        }
    }

    pub fn snapshot(&self) -> &Arc<AuthSnapshot> {
        &self.snapshot
    }

    pub fn is_authenticated(&self) -> bool {
        self.snapshot.is_authenticated
    }

    pub fn is_restoring(&self) -> bool {
        self.snapshot.is_restoring
    }

    pub fn profile(&self) -> Option<&AuthUser> {
        self.snapshot.active.as_ref()
    }

    pub fn accounts(&self) -> &[AccountSummary] {
        &self.snapshot.accounts
    }

    pub fn flow(&self) -> &AuthFlowState {
        &self.snapshot.flow
    }

    pub fn is_busy(&self) -> bool {
        self.snapshot.flow.is_busy
    }

    pub fn error_key(&self) -> Option<&'static str> {
        self.snapshot.flow.error_key
    }

    #[cfg(test)]
    pub fn force_error(&mut self, key: Option<&'static str>, cx: &mut Context<Self>) {
        self.patch_flow(|flow| flow.error_key = key, cx);
    }

    #[cfg(test)]
    pub fn force_telegram_redirect(&mut self, bot: &str, cx: &mut Context<Self>) {
        let bot = bot.to_owned();
        self.patch_flow(
            move |flow| {
                flow.telegram_redirect_active = true;
                flow.bot_username = Some(bot);
                flow.phone_payload = Some("77075803272".to_owned());
            },
            cx,
        );
    }

    #[cfg(test)]
    fn patch_flow(&mut self, edit: impl FnOnce(&mut AuthFlowState), cx: &mut Context<Self>) {
        let mut snapshot = (*self.snapshot).clone();
        edit(&mut snapshot.flow);
        self.snapshot = Arc::new(snapshot);
        cx.notify();
    }

    pub fn tag_availability(&self) -> TagAvailability {
        self.snapshot.flow.tag_availability
    }

    #[cfg(test)]
    pub fn force_tag_availability(
        &mut self,
        availability: TagAvailability,
        cx: &mut Context<Self>,
    ) {
        self.patch_flow(|flow| flow.tag_availability = availability, cx);
    }

    pub fn tag_problem_key(&self) -> Option<&'static str> {
        self.snapshot.flow.tag_problem_key
    }

    pub fn tag_is_confirmed_free(&self) -> bool {
        self.tag_availability() == TagAvailability::Available
    }

    pub fn telegram_link(&self) -> Option<String> {
        let flow = &self.snapshot.flow;
        if !flow.telegram_redirect_active {
            return None;
        }
        let bot = flow.bot_username.as_deref()?;
        let payload = flow.phone_payload.as_deref().unwrap_or_default();
        Some(format!("https://t.me/{bot}?start={payload}"))
    }

    pub fn is_current_user(&self, user_id: &str) -> bool {
        self.snapshot
            .active
            .as_ref()
            .is_some_and(|active| active.id == user_id)
    }
}

pub struct AuthStores {
    pub services: Entity<AuthServicesStore>,
    pub actions: super::super::auth_actions::AuthActionsStore,
}

impl gpui::Global for AuthStores {}

pub fn auth_services(cx: &App) -> &Entity<AuthServicesStore> {
    &cx.global::<AuthStores>().services
}

pub fn try_auth_services(cx: &App) -> Option<&Entity<AuthServicesStore>> {
    cx.try_global::<AuthStores>().map(|stores| &stores.services)
}
