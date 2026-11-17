use std::fmt;

use rise_canon::{RiseLimits, RiseTimingCanon};
use rise_core::foundation::foundation_text::{characters, trimming_whitespaces_and_newlines};
use rise_i18n::tr;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;

pub enum UserContactLimits {}

impl UserContactLimits {
    pub const MAXIMUM_SYNC_RETRY_ATTEMPTS: i64 = 8;
    pub const INITIAL_SYNC_RETRY_DELAY_MILLISECONDS: i64 = 1_000;
    pub const MAXIMUM_SYNC_RETRY_DELAY_MILLISECONDS: i64 = 30_000;

    pub fn name() -> i64 {
        RiseLimits::service_value("max_contact_name_chars")
    }

    pub fn note() -> i64 {
        RiseLimits::service_value("max_contact_note_chars")
    }

    pub fn list_page() -> i64 {
        RiseLimits::service_value("max_contact_list_limit")
    }

    pub fn fallback_sync_batch() -> i64 {
        RiseTimingCanon::batch("contacts_sync_batch").client_batch_items
    }

    pub fn maximum_sync_batches() -> i64 {
        RiseTimingCanon::batch("contacts_sync_batches").client_batch_items
    }

    pub fn maximum_automatic_saved_contact_pages() -> i64 {
        RiseLimits::client_value("user_contact_limits_maximum_automatic_saved_contact_pages")
    }

    pub fn maximum_automatic_saved_contact_ids() -> i64 {
        RiseLimits::client_value("user_contact_limits_maximum_automatic_saved_contact_i_ds")
    }

    pub fn maximum_visible_saved_contact_hydrations() -> i64 {
        RiseLimits::client_value("user_contact_limits_maximum_visible_saved_contact_hydrations")
    }

    pub fn maximum_automatic_inbox_pages() -> i64 {
        RiseLimits::client_value("user_contact_limits_maximum_automatic_inbox_pages")
    }

    pub fn maximum_join_virtual_dialogs() -> i64 {
        RiseLimits::client_value("user_contact_limits_maximum_join_virtual_dialogs")
    }

    pub fn maximum_join_toasts_per_refresh() -> i64 {
        RiseLimits::client_value("user_contact_limits_maximum_join_toasts_per_refresh")
    }

    pub fn maximum_join_ack_batch() -> i64 {
        RiseLimits::client_value("user_contact_limits_maximum_join_ack_batch")
    }

    pub fn clip(text: &str, limit: i64) -> String {
        let mut used: i64 = 0;
        let mut end = 0;
        for character in characters(text) {
            used += i64::try_from(character.chars().count()).unwrap_or(i64::MAX);
            if used > limit {
                break;
            }
            end += character.len();
        }
        text[..end].to_owned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserContactDraft {
    pub target_user_id: String,
    pub first_name: String,
    pub last_name: String,
    pub note: String,
    pub share_phone_number: bool,
    pub sync_to_phone: bool,
}

impl UserContactDraft {
    pub fn normalized_first_name(&self) -> String {
        UserContactLimits::clip(trimming_whitespaces_and_newlines(&self.first_name), UserContactLimits::name())
    }

    pub fn normalized_last_name(&self) -> String {
        UserContactLimits::clip(trimming_whitespaces_and_newlines(&self.last_name), UserContactLimits::name())
    }

    pub fn normalized_note(&self) -> String {
        UserContactLimits::clip(trimming_whitespaces_and_newlines(&self.note), UserContactLimits::note())
    }

    pub fn is_valid(&self) -> bool {
        !trimming_whitespaces_and_newlines(&self.target_user_id).is_empty() && !self.normalized_first_name().is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserContactBlockFact {
    Unknown,
    Resolved(Option<RiseBlockScope>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserContactPeer {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub logo: String,
    pub is_premium: bool,
    pub is_online: bool,
    pub last_seen: Option<i64>,
    pub is_bot: bool,
    pub phone: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserContactMutationError {
    InvalidContact,
    InvalidResponse,
    SyncLimitExceeded,
    PhotoUploadResponse,
    PhoneUnavailable,
    ContactsPermissionUnavailable,
    PhotoSettlementTimeout,
}

impl UserContactMutationError {
    pub fn error_description(&self) -> Option<String> {
        Some(match self {
            Self::InvalidContact => tr("contact_error_invalid"),
            Self::InvalidResponse => tr("contact_error_invalid_response"),
            Self::SyncLimitExceeded => tr("contact_sync_too_many"),
            Self::PhotoUploadResponse => tr("contact_photo_upload_failed"),
            Self::PhoneUnavailable => tr("contact_phone_unavailable"),
            Self::ContactsPermissionUnavailable => tr("contact_sync_status_denied"),
            Self::PhotoSettlementTimeout => tr("contact_photo_settlement_timeout"),
        })
    }
}

impl fmt::Display for UserContactMutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for UserContactMutationError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserContactPhotoUpload {
    pub asset_id: String,
    pub stable_file_url: String,
}
