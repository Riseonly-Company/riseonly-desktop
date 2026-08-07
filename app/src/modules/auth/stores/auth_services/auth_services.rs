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
    /// Mirrors the domain's snapshot onto the GPUI thread.
    ///
    /// GPUI does not track dependencies, so the notify is explicit — and the
    /// presentation refuses to republish an unchanged snapshot, which is what
    /// stops a socket heartbeat from invalidating the shell every twenty-five
    /// seconds.
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

    pub fn tag_availability(&self) -> TagAvailability {
        self.snapshot.flow.tag_availability
    }

    /// The `https://t.me/<bot>?start=<phone>` the Telegram step opens.
    ///
    /// Composed here rather than in the view: the payload is digits only and the
    /// bot name comes from the server, so a view that assembled it would be one
    /// place the link could be built wrongly.
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

/// The process-wide handle. Auth is not account-scoped: it is what decides which
/// account there is, so it outlives every `AccountScope`.
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
