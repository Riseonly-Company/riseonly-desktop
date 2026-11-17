use std::collections::HashMap;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::guides::engine::rise_guides_models::{
    RiseGuide, RiseGuideReceipt, RiseGuidesCacheSource,
};

#[derive(Clone, Debug, PartialEq)]
pub struct RiseGuidesSnapshot {
    pub revision: u64,
    pub guides: Vec<RiseGuide>,
    pub catalogue: Vec<RiseGuide>,
    pub receipts: HashMap<i64, RiseGuideReceipt>,
    pub auto_start: bool,
    pub cache_source: RiseGuidesCacheSource,
    pub is_refreshing: bool,
}

impl RiseGuidesSnapshot {
    pub fn empty() -> Self {
        Self {
            revision: 0,
            guides: Vec::new(),
            catalogue: Vec::new(),
            receipts: HashMap::new(),
            auto_start: true,
            cache_source: RiseGuidesCacheSource::Miss,
            is_refreshing: false,
        }
    }

    /// lookup by key, the canon empty_state trigger is the feature own initiative
    pub fn guide(&self, key: &str) -> Option<&RiseGuide> {
        let key = trimming_whitespaces_and_newlines(key);
        if key.is_empty() {
            return None;
        }
        self.catalogue
            .iter()
            .find(|guide| guide.key == key)
            .or_else(|| self.guides.iter().find(|guide| guide.key == key))
    }
}

/// immutable ui snapshot of the guides waiting for this account
/// published as a whole value and gated on account, epoch and session so a stale callback
/// cant paint over it
pub struct RiseGuidesPresentation {
    snapshot: RiseGuidesSnapshot,
    generation: u64,
    has_guides: bool,
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
}

impl RiseGuidesPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: RiseGuidesSnapshot::empty(),
            generation: 0,
            has_guides: false,
            account_id: None,
            account_epoch: None,
            session_id: None,
        }
    }

    pub fn snapshot(&self) -> &RiseGuidesSnapshot {
        &self.snapshot
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn has_guides(&self) -> bool {
        self.has_guides
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
        self.install(RiseGuidesSnapshot::empty(), cx);
    }

    pub fn publish(
        &mut self,
        next: RiseGuidesSnapshot,
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
        self.install(next, cx);
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
        self.install(RiseGuidesSnapshot::empty(), cx);
    }

    fn install(&mut self, next: RiseGuidesSnapshot, cx: &mut Context<Self>) {
        self.snapshot = next;
        self.has_guides = !self.snapshot.guides.is_empty();
        cx.notify();
    }
}
