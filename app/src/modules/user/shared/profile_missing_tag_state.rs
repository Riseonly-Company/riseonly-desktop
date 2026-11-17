use std::collections::HashSet;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::lib::canon_errors::RiseFailure;

pub struct ProfileMissingTagState {
    pending: Option<ProfileMissingTagStateRequest>,
    missing: Option<ProfileMissingTagStateRequest>,
    failed: Option<ProfileMissingTagStateRequest>,
    deleted_user_ids: HashSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileMissingTagStateRequest {
    pub id: Uuid,
    pub tag: String,
    pub account_id: String,
    pub account_epoch: String,
}

impl ProfileMissingTagStateRequest {
    pub fn matches(&self, tag: &str, account_id: Option<&str>, account_epoch: Option<&str>) -> bool {
        self.tag == trimming_whitespaces_and_newlines(tag).to_lowercase()
            && Some(self.account_id.as_str()) == account_id
            && Some(self.account_epoch.as_str()) == account_epoch
    }
}

const USER_NOT_FOUND_MESSAGE: &str = "User not found";

impl ProfileMissingTagState {
    /// key of a profile opened by user id, same slot as a tag but kept apart by its prefix
    pub fn user_id_key(user_id: &str) -> String {
        format!("id:{}", trimming_whitespaces_and_newlines(user_id))
    }

    pub fn mark_deleted_user(user_id: &str, cx: &mut App) {
        let user_id = trimming_whitespaces_and_newlines(user_id).to_owned();
        if user_id.is_empty() {
            return;
        }
        Self::shared(cx).update(cx, |state, cx| {
            if state.deleted_user_ids.insert(user_id) {
                cx.notify();
            }
        });
    }

    pub fn is_deleted_user(&self, user_id: &str) -> bool {
        self.deleted_user_ids.contains(trimming_whitespaces_and_newlines(user_id))
    }

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { pending: None, missing: None, failed: None, deleted_user_ids: HashSet::new() }
    }

    pub fn begin(
        &mut self,
        tag: &str,
        account_id: Option<&str>,
        account_epoch: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<ProfileMissingTagStateRequest> {
        let account_id = account_id.filter(|value| !value.is_empty())?;
        let account_epoch = account_epoch.filter(|value| !value.is_empty())?;
        let tag = trimming_whitespaces_and_newlines(tag).to_lowercase();
        if tag.is_empty() {
            return None;
        }
        if let Some(pending) = &self.pending
            && pending.matches(&tag, Some(account_id), Some(account_epoch))
        {
            return Some(pending.clone());
        }
        let request = ProfileMissingTagStateRequest {
            id: Uuid::new_v4(),
            tag,
            account_id: account_id.to_owned(),
            account_epoch: account_epoch.to_owned(),
        };
        self.pending = Some(request.clone());
        self.missing = None;
        self.failed = None;
        cx.notify();
        Some(request)
    }

    pub fn finish(
        &mut self,
        request: Option<&ProfileMissingTagStateRequest>,
        failure: Option<&RiseFailure>,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = request else {
            return;
        };
        if self.pending.as_ref() != Some(request) {
            return;
        }
        self.pending = None;
        let is_missing = failure.is_some_and(|failure| match failure.code.as_deref() {
            Some(code) if !rise_errors::is_absent_code(code) => {
                rise_errors::ErrorCode::from_wire_or_alias(code) == Some(rise_errors::ErrorCode::UserAccountNotFound)
            }
            _ => failure.message == USER_NOT_FOUND_MESSAGE,
        });
        self.missing = is_missing.then(|| request.clone());
        self.failed = (failure.is_some() && self.missing.is_none()).then(|| request.clone());
        cx.notify();
    }

    pub fn is_failed(&self, tag: &str, account_id: Option<&str>, account_epoch: Option<&str>) -> bool {
        self.failed
            .as_ref()
            .is_some_and(|failed| failed.matches(tag, account_id, account_epoch))
    }

    pub fn is_missing(&self, tag: &str, account_id: Option<&str>, account_epoch: Option<&str>) -> bool {
        self.missing
            .as_ref()
            .is_some_and(|missing| missing.matches(tag, account_id, account_epoch))
    }
}
