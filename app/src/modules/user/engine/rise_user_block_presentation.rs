use std::collections::{HashMap, HashSet};

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;

/// load state lives in the snapshot not in a transport object in the view
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseUserBlockLoadState {
    Idle,
    Loading,
    Loaded,
    Failed,
}

/// one row of the blocked list, logo is drawn since the block list has to be readable
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseBlockedUser {
    pub user_id: String,
    pub name: String,
    pub tag: String,
    pub logo: String,
    pub is_premium: bool,
    pub is_deleted: bool,
    pub scope: RiseBlockScope,
    pub blocked_at_ms: i64,
}

impl RiseBlockedUser {
    pub fn id(&self) -> &str {
        &self.user_id
    }
}

/// immutable bounded ui state for blocks
/// scopes is wider than entries, a profile read teaches a tier alone
/// no target means not blocked as far as known
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserBlockSnapshot {
    pub entries: Vec<RiseBlockedUser>,
    pub scopes: HashMap<String, RiseBlockScope>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
    pub load_state: RiseUserBlockLoadState,
    pub pending: HashSet<String>,
    pub error: Option<String>,
    pub is_loading_more: bool,
}

impl RiseUserBlockSnapshot {
    pub fn empty() -> RiseUserBlockSnapshot {
        RiseUserBlockSnapshot {
            entries: Vec::new(),
            scopes: HashMap::new(),
            has_more: false,
            next_cursor: None,
            load_state: RiseUserBlockLoadState::Idle,
            pending: HashSet::new(),
            error: None,
            is_loading_more: false,
        }
    }

    pub fn scope(&self, user_id: &str) -> Option<RiseBlockScope> {
        self.scopes.get(user_id).copied()
    }

    pub fn is_blocked(&self, user_id: &str) -> bool {
        self.scopes.contains_key(user_id)
    }

    pub fn is_totally_blocked(&self, user_id: &str) -> bool {
        self.scopes.get(user_id) == Some(&RiseBlockScope::Total)
    }
}

/// publish gate, a snapshot with a stale account
/// epoch or session is dropped so a reply after logout cant repaint
pub struct RiseUserBlockPresentation {
    snapshot: RiseUserBlockSnapshot,
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
}

impl RiseUserBlockPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: RiseUserBlockSnapshot::empty(),
            account_id: None,
            account_epoch: None,
            session_id: None,
        }
    }

    pub fn snapshot(&self) -> &RiseUserBlockSnapshot {
        &self.snapshot
    }

    pub fn activate(
        &mut self,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        self.account_id = Some(account_id.to_owned());
        self.account_epoch = Some(account_epoch.to_owned());
        self.session_id = Some(session_id);
        self.snapshot = RiseUserBlockSnapshot::empty();
        cx.notify();
    }

    pub fn publish(
        &mut self,
        next: RiseUserBlockSnapshot,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.account_id.as_deref() != Some(account_id)
            || self.account_epoch.as_deref() != Some(account_epoch)
            || self.session_id != Some(session_id)
        {
            return false;
        }
        if next == self.snapshot {
            return true;
        }
        self.snapshot = next;
        cx.notify();
        true
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(account_id) || self.session_id != Some(session_id) {
            return;
        }
        self.account_id = None;
        self.account_epoch = None;
        self.session_id = None;
        self.snapshot = RiseUserBlockSnapshot::empty();
        cx.notify();
    }
}
