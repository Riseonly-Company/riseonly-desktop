use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, Context, Entity};

use super::rise_pack_collaboration_repository::{
    RisePackCoauthor, RisePackInviteLinksPage, RisePackJoinRequest, RisePackPage,
};

/// what the coauthor screens read, lists the repository holds published whole after every change
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RisePackCollaborationSnapshot {
    pub account_id: String,
    pub revision: u64,
    pub links: HashMap<String, RisePackInviteLinksPage>,
    pub coauthors: HashMap<String, RisePackPage<RisePackCoauthor>>,
    pub requests: HashMap<String, RisePackPage<RisePackJoinRequest>>,
    pub joiners: HashMap<String, RisePackPage<RisePackCoauthor>>,
}

impl RisePackCollaborationSnapshot {
    fn new(account_id: String) -> Self {
        Self { account_id, ..Self::default() }
    }
}

pub struct RisePackCollaborationPresentation {
    snapshot: Arc<RisePackCollaborationSnapshot>,
}

impl RisePackCollaborationPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { snapshot: Arc::new(RisePackCollaborationSnapshot::default()) }
    }

    pub fn snapshot(&self) -> &Arc<RisePackCollaborationSnapshot> {
        &self.snapshot
    }

    pub fn activate(&mut self, account_id: &str, cx: &mut Context<Self>) {
        if self.snapshot.account_id == account_id {
            return;
        }
        self.snapshot = Arc::new(RisePackCollaborationSnapshot::new(account_id.to_owned()));
        cx.notify();
    }

    pub fn publish(&mut self, value: RisePackCollaborationSnapshot, cx: &mut Context<Self>) {
        if value.account_id != self.snapshot.account_id || value.revision <= self.snapshot.revision {
            return;
        }
        self.snapshot = Arc::new(value);
        cx.notify();
    }

    pub fn deactivate(&mut self, account_id: &str, cx: &mut Context<Self>) {
        if self.snapshot.account_id != account_id {
            return;
        }
        self.snapshot = Arc::new(RisePackCollaborationSnapshot::default());
        cx.notify();
    }
}
