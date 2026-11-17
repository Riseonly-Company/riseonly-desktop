use std::collections::HashSet;

use rise_canon::rise_key::chat as key;
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct ChatArchiveMutationSnapshot {
    pub token: Uuid,
    pub chat_id: String,
    pub moves_to_archive: bool,
    pub source_row: Map<String, Value>,
    pub optimistic_row: Map<String, Value>,
    pub source_was_cached: bool,
    pub destination_row: Option<Map<String, Value>>,
    pub archived_count_delta: i64,
    pub archived_unread_delta: i64,
    pub archived_unread_chats_delta: i64,
    pub source_needs_refresh_after_commit: bool,
    pub confirmed_by_realtime: bool,
}

pub enum ChatArchiveMutationFailurePolicy {}

impl ChatArchiveMutationFailurePolicy {
    pub fn should_revert(confirmed_by_realtime: bool) -> bool {
        !confirmed_by_realtime
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatArchiveRealtimeResolution {
    NoPendingMutation,
    ConfirmsPendingMutation,
    ConflictsWithPendingMutation,
}

impl ChatArchiveRealtimeResolution {
    pub fn resolve(pending_moves_to_archive: Option<bool>, event_moves_to_archive: bool) -> Self {
        let Some(pending_moves_to_archive) = pending_moves_to_archive else {
            return Self::NoPendingMutation;
        };
        if pending_moves_to_archive == event_moves_to_archive {
            Self::ConfirmsPendingMutation
        } else {
            Self::ConflictsWithPendingMutation
        }
    }

    pub fn discards_pending_mutation(&self) -> bool {
        *self == Self::ConflictsWithPendingMutation
    }
}

pub enum ChatArchiveOptimisticRowPolicy {}

impl ChatArchiveOptimisticRowPolicy {
    pub fn destination_row(source_row: &Map<String, Value>, moves_to_archive: bool) -> Map<String, Value> {
        if !moves_to_archive {
            return source_row.clone();
        }
        let mut archived_row = source_row.clone();
        archived_row.insert(key::IS_PINNED.to_owned(), Value::Bool(false));
        archived_row.insert(key::PIN_POSITION.to_owned(), Value::Null);
        archived_row
    }
}

pub enum ChatArchiveCursorPolicy {}

impl ChatArchiveCursorPolicy {
    pub fn repaired_cursor(
        current_cursor: Option<&str>,
        removed_chat_ids: &HashSet<String>,
        remaining_chat_ids: &[String],
    ) -> Option<String> {
        let current_cursor = current_cursor?;
        if !removed_chat_ids.contains(current_cursor) && remaining_chat_ids.iter().any(|id| id == current_cursor) {
            return Some(current_cursor.to_owned());
        }
        remaining_chat_ids.last().cloned()
    }
}
