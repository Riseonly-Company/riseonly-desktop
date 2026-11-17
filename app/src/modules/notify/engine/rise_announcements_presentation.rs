use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::notify::engine::core::rise_announcements_resource_repository::RiseAnnouncementsCacheSource;
use crate::modules::notify::engine::core::rise_announcements_wire_models::RiseAnnouncement;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseAnnouncementsSnapshot {
    pub pending: Vec<RiseAnnouncement>,
    pub current: Option<RiseAnnouncement>,
    pub cache_source: RiseAnnouncementsCacheSource,
    pub is_refreshing: bool,
}

impl RiseAnnouncementsSnapshot {
    pub const EMPTY: RiseAnnouncementsSnapshot = RiseAnnouncementsSnapshot {
        pending: Vec::new(),
        current: None,
        cache_source: RiseAnnouncementsCacheSource::Miss,
        is_refreshing: false,
    };
}

/// immutable ui snapshot of the broadcasts waiting for this account
/// published as a whole value, account and session gated
/// so a signed out callback cant paint over the current one
pub struct RiseAnnouncementsPresentation {
    snapshot: RiseAnnouncementsSnapshot,
    generation: u64,
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
}

impl RiseAnnouncementsPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: RiseAnnouncementsSnapshot::EMPTY,
            generation: 0,
            account_id: None,
            account_epoch: None,
            session_id: None,
        }
    }

    pub fn snapshot(&self) -> &RiseAnnouncementsSnapshot {
        &self.snapshot
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn activate(
        &mut self,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        if self.account_id.as_deref() == Some(account_id)
            && self.account_epoch.as_deref() == Some(account_epoch)
            && self.session_id == Some(session_id)
        {
            return;
        }
        self.account_id = Some(account_id.to_owned());
        self.account_epoch = Some(account_epoch.to_owned());
        self.session_id = Some(session_id);
        self.generation = self.generation.wrapping_add(1);
        self.snapshot = RiseAnnouncementsSnapshot::EMPTY;
        cx.notify();
    }

    pub fn publish(
        &mut self,
        next: RiseAnnouncementsSnapshot,
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
        self.generation = self.generation.wrapping_add(1);
        self.snapshot = RiseAnnouncementsSnapshot::EMPTY;
        cx.notify();
    }
}
