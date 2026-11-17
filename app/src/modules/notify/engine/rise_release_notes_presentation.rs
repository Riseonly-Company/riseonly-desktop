use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::notify::engine::core::rise_release_notes_resource_repository::RiseReleaseNotesCacheSource;
use crate::modules::notify::engine::core::rise_release_notes_wire_models::RiseReleaseNote;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseReleaseNotesSnapshot {
    pub pending: Vec<RiseReleaseNote>,
    pub cache_source: RiseReleaseNotesCacheSource,
    pub is_refreshing: bool,
    pub did_refresh: bool,
}

impl RiseReleaseNotesSnapshot {
    pub const EMPTY: RiseReleaseNotesSnapshot = RiseReleaseNotesSnapshot {
        pending: Vec::new(),
        cache_source: RiseReleaseNotesCacheSource::Miss,
        is_refreshing: false,
        did_refresh: false,
    };

    /// one modal at a time even if a device skipped several versions
    /// the rest are marked seen without being shown
    pub fn current(&self) -> Option<&RiseReleaseNote> {
        self.pending.first()
    }

    pub fn is_blocking(&self) -> bool {
        self.current().is_some_and(|note| note.is_blocking)
    }
}

/// published as a whole value, account and session gated so a signed out account cant paint
/// over the current one
pub struct RiseReleaseNotesPresentation {
    snapshot: RiseReleaseNotesSnapshot,
    is_blocking: bool,
    did_refresh: bool,
    generation: u64,
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
}

impl RiseReleaseNotesPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: RiseReleaseNotesSnapshot::EMPTY,
            is_blocking: false,
            did_refresh: false,
            generation: 0,
            account_id: None,
            account_epoch: None,
            session_id: None,
        }
    }

    pub fn snapshot(&self) -> &RiseReleaseNotesSnapshot {
        &self.snapshot
    }

    pub fn is_blocking(&self) -> bool {
        self.is_blocking
    }

    pub fn did_refresh(&self) -> bool {
        self.did_refresh
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
        self.install(RiseReleaseNotesSnapshot::EMPTY);
        cx.notify();
    }

    pub fn publish(
        &mut self,
        next: RiseReleaseNotesSnapshot,
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
        self.install(next);
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
        self.install(RiseReleaseNotesSnapshot::EMPTY);
        cx.notify();
    }

    fn install(&mut self, next: RiseReleaseNotesSnapshot) {
        let next_is_blocking = next.is_blocking();
        if self.did_refresh != next.did_refresh {
            self.did_refresh = next.did_refresh;
        }
        self.snapshot = next;
        if self.is_blocking == next_is_blocking {
            return;
        }
        self.is_blocking = next_is_blocking;
    }
}
