use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::comment::engine::rise_comment_engine_models::{RiseCommentListSnapshot, RiseCommentScope};
use crate::modules::user::shared::rise_blocked_content::RiseBlockedContent;

/// published state of one list, one box per list so a like inside one sheet re-renders only
/// that list
pub struct RiseCommentListBox {
    snapshot: Arc<RiseCommentListSnapshot>,
}

impl RiseCommentListBox {
    fn new(scope: RiseCommentScope) -> Self {
        Self { snapshot: Arc::new(RiseCommentListSnapshot::empty(scope)) }
    }

    pub fn snapshot(&self) -> &Arc<RiseCommentListSnapshot> {
        &self.snapshot
    }
}

/// main thread end of the comment domain
/// immutable snapshots accepted only from the session activated here
pub struct RiseCommentPresentation {
    boxes: HashMap<RiseCommentScope, Entity<RiseCommentListBox>>,
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
}

impl RiseCommentPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self {
            boxes: HashMap::new(),
            account_id: None,
            account_epoch: None,
            session_id: None,
        })
    }

    fn retained_boxes() -> i64 {
        RiseLimits::client_value("comment_lists_memory")
    }

    pub fn activate(&mut self, account_id: &str, account_epoch: &str, session_id: Uuid, cx: &mut Context<Self>) {
        let is_same_session = self.account_id.as_deref() == Some(account_id)
            && self.account_epoch.as_deref() == Some(account_epoch)
            && self.session_id == Some(session_id);
        if is_same_session {
            return;
        }
        self.account_id = Some(account_id.to_owned());
        self.account_epoch = Some(account_epoch.to_owned());
        self.session_id = Some(session_id);
        self.reset_boxes(cx);
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(account_id) || self.session_id != Some(session_id) {
            return;
        }
        self.account_id = None;
        self.account_epoch = None;
        self.session_id = None;
        self.reset_boxes(cx);
    }

    /// box is created once and then only its snapshot moves so reading it doesnt invalidate observers
    pub fn box_for(&mut self, scope: &RiseCommentScope, cx: &mut Context<Self>) -> Entity<RiseCommentListBox> {
        if let Some(existing) = self.boxes.get(scope) {
            return existing.clone();
        }
        let created = cx.new(|_| RiseCommentListBox::new(scope.clone()));
        self.boxes.insert(scope.clone(), created.clone());
        self.prune_if_needed(scope, cx);
        created
    }

    /// comments of people this account blocked are left out of every list read here
    pub fn snapshot(&mut self, scope: &RiseCommentScope, cx: &mut Context<Self>) -> Arc<RiseCommentListSnapshot> {
        let snapshot = self.box_for(scope, cx).read(cx).snapshot.clone();
        RiseBlockedContent::visible_comment_snapshot(snapshot, cx)
    }

    pub fn publish(
        &mut self,
        snapshots: Vec<RiseCommentListSnapshot>,
        evicted: Vec<RiseCommentScope>,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.account_id.as_deref() != Some(account_id)
            || self.account_epoch.as_deref() != Some(account_epoch)
            || self.session_id != Some(session_id)
        {
            tracing::warn!(
                target: "riseonly::X2027",
                "publish refused scopes={} account_matches={} epoch_matches={} session_matches={}",
                snapshots.len(),
                self.account_id.as_deref() == Some(account_id),
                self.account_epoch.as_deref() == Some(account_epoch),
                self.session_id == Some(session_id)
            );
            return false;
        }

        for snapshot in snapshots {
            let target = self.box_for(&snapshot.scope, cx);
            target.update(cx, |list, cx| {
                if snapshot.revision < list.snapshot.revision || *list.snapshot == snapshot {
                    return;
                }
                list.snapshot = Arc::new(snapshot);
                cx.notify();
            });
        }
        for scope in evicted {
            let Some(target) = self.boxes.get(&scope) else {
                continue;
            };
            target.update(cx, |list, cx| Self::empty_box(list, scope, cx));
        }
        true
    }

    fn reset_boxes(&mut self, cx: &mut Context<Self>) {
        for (scope, target) in &self.boxes {
            let scope = scope.clone();
            target.update(cx, |list, cx| Self::empty_box(list, scope, cx));
        }
    }

    fn empty_box(list: &mut RiseCommentListBox, scope: RiseCommentScope, cx: &mut Context<RiseCommentListBox>) {
        let empty = RiseCommentListSnapshot::empty(scope);
        if *list.snapshot == empty {
            return;
        }
        list.snapshot = Arc::new(empty);
        cx.notify();
    }

    fn prune_if_needed(&mut self, protecting: &RiseCommentScope, cx: &mut Context<Self>) {
        if (self.boxes.len() as i64) <= Self::retained_boxes() * 2 {
            return;
        }
        self.boxes.retain(|key, list| {
            key == protecting || *list.read(cx).snapshot != RiseCommentListSnapshot::empty(key.clone())
        });
    }
}
