use std::sync::Arc;

use gpui::Context;

use crate::modules::session::engine::rise_session_domain::RiseSessionDomain;
use crate::modules::session::engine::rise_session_engine_models::SessionEntry;
use crate::modules::session::engine::rise_session_repository::SessionSnapshot;

pub struct SessionServicesStore {
    snapshot: Arc<SessionSnapshot>,
}

impl SessionServicesStore {
    pub fn new(domain: Arc<RiseSessionDomain>, cx: &mut Context<Self>) -> Self {
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

    pub fn sessions(&self) -> &[SessionEntry] {
        &self.snapshot.rows
    }

    pub fn current_session(&self) -> Option<&SessionEntry> {
        self.snapshot
            .rows
            .iter()
            .find(|row| row.is_current)
            .or_else(|| self.snapshot.rows.first())
    }

    pub fn other_sessions(&self) -> Vec<&SessionEntry> {
        let current = self.current_session().map(|row| row.id.as_str());
        self.snapshot
            .rows
            .iter()
            .filter(|row| Some(row.id.as_str()) != current)
            .collect()
    }

    pub fn did_load(&self) -> bool {
        self.snapshot.did_load
    }

    pub fn is_loading(&self) -> bool {
        self.snapshot.is_loading
    }

    pub fn is_mutating(&self) -> bool {
        self.snapshot.is_mutating
    }

    pub fn has_more(&self) -> bool {
        self.snapshot.has_more
    }

    pub fn error_key(&self) -> Option<&'static str> {
        self.snapshot.error_key
    }

    /// Whether the server has ended the session this client is running on.
    ///
    /// Reported rather than acted on: signing out belongs to auth, and a session
    /// store that could clear an account would be a second place account state
    /// is owned.
    pub fn should_sign_out(&self) -> bool {
        self.snapshot.should_sign_out
    }
}

/// Where the session list lives for the life of the process.
pub struct SessionStores {
    pub interactions: super::super::session_interactions::SessionInteractionsStore,
}

impl gpui::Global for SessionStores {}

pub fn sessions(cx: &gpui::App) -> Option<&SessionStores> {
    cx.try_global::<SessionStores>()
}
