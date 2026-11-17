use std::collections::HashMap;
use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use rise_canon::{RiseBugReportSurface, RiseFeedbackKind};
use rise_catalog::DIAGNOSTICS_FEEDBACK_KINDS_DEFAULT;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_types::{
    BugReportDraftKind, BugReportSheetPage,
};

const LOGICAL_DISK_CACHE_KEY: &str = "bugReportDrafts";
const CHECKPOINT_DELAY: Duration = Duration::from_millis(1500);
const ACCOUNT_SCOPE_PREFIX: &str = "account:";
const FORMAT_VERSION: i64 = 1;

const VERSION_KEY: &str = "version";
const PAGE_KEY: &str = "page";
const ACTIVE_KIND_KEY: &str = "activeKind";
const DRAFTS_KEY: &str = "drafts";
const CLIENT_REPORT_ID_KEY: &str = "clientReportId";
const FEEDBACK_KIND_KEY: &str = "feedbackKind";
const MESSAGE_KEY: &str = "message";
const COMPONENT_KEY: &str = "component";
const PAGES_KEY: &str = "pages";
const SEEDED_PAGES_KEY: &str = "seededPages";

/// what survives a relaunch of the report sheet: text, areas feedback kind idempotency key page
/// one record per draft kind under the account scope, attachments are not persisted
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BugReportPersistedDraft {
    pub client_report_id: String,
    pub kind: BugReportDraftKind,
    pub feedback_kind: RiseFeedbackKind,
    pub message: String,
    pub component: String,
    pub pages: Vec<RiseBugReportSurface>,
    pub seeded_pages: Vec<RiseBugReportSurface>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BugReportPersistedState {
    pub drafts: HashMap<BugReportDraftKind, BugReportPersistedDraft>,
    pub page: BugReportSheetPage,
    pub active_kind: BugReportDraftKind,
}

impl Default for BugReportPersistedState {
    fn default() -> Self {
        Self { drafts: HashMap::new(), page: BugReportSheetPage::Root, active_kind: BugReportDraftKind::Bug }
    }
}

/// written through the disk cache off the main thread and keyed per account
pub struct BugReportDraftPersistence {
    loaded_disk_cache_key: Option<String>,
    pending: Option<BugReportPersistedState>,
    checkpoint: Option<Task<()>>,
}

impl BugReportDraftPersistence {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { loaded_disk_cache_key: None, pending: None, checkpoint: None }
    }

    /// state saved for the active account
    /// none when nothing was saved or no account is active yet, the caller then starts from an
    /// empty draft
    pub fn load(&mut self) -> Option<BugReportPersistedState> {
        let cache_key = SaiAccountCacheScope::scoped_key(LOGICAL_DISK_CACHE_KEY);
        if !cache_key.starts_with(ACCOUNT_SCOPE_PREFIX) {
            return None;
        }
        let raw = SaiWsDiskCache::load(&cache_key);
        self.loaded_disk_cache_key = Some(cache_key);
        raw.as_ref().and_then(Value::as_object).and_then(Self::decode)
    }

    /// schedules a write, coalesced so the last state in the window is the one that lands
    pub fn save(&mut self, state: BugReportPersistedState, cx: &mut Context<Self>) {
        let cache_key = SaiAccountCacheScope::scoped_key(LOGICAL_DISK_CACHE_KEY);
        if !cache_key.starts_with(ACCOUNT_SCOPE_PREFIX) {
            return;
        }
        self.loaded_disk_cache_key = Some(cache_key);
        self.pending = Some(state);
        self.checkpoint = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CHECKPOINT_DELAY).await;
            let _ = this.update(cx, |this, cx| this.flush(cx));
        }));
    }

    pub fn flush(&mut self, cx: &mut App) {
        self.checkpoint = None;
        let (Some(pending), Some(loaded_disk_cache_key)) = (self.pending.as_ref(), self.loaded_disk_cache_key.clone())
        else {
            return;
        };
        let value = Self::encode(pending);
        self.pending = None;
        SaiWsDiskCache::save_asynchronously(loaded_disk_cache_key, value, |_, _| {}, cx);
    }

    /// drops the saved state of the active account so a sent or discarded draft cant come back
    /// on next launch
    pub fn clear(&mut self) {
        self.checkpoint = None;
        self.pending = None;
        let cache_key = SaiAccountCacheScope::scoped_key(LOGICAL_DISK_CACHE_KEY);
        if !cache_key.starts_with(ACCOUNT_SCOPE_PREFIX) {
            return;
        }
        SaiWsDiskCache::remove(&cache_key);
    }

    pub fn reset_account_state(&mut self) {
        self.checkpoint = None;
        self.pending = None;
        self.loaded_disk_cache_key = None;
    }

    fn encode(state: &BugReportPersistedState) -> Value {
        let mut drafts = Map::new();
        for (kind, draft) in &state.drafts {
            let mut dictionary = Map::new();
            dictionary.insert(CLIENT_REPORT_ID_KEY.to_owned(), Value::from(draft.client_report_id.clone()));
            dictionary.insert(FEEDBACK_KIND_KEY.to_owned(), Value::from(draft.feedback_kind.wire()));
            dictionary.insert(MESSAGE_KEY.to_owned(), Value::from(draft.message.clone()));
            dictionary.insert(COMPONENT_KEY.to_owned(), Value::from(draft.component.clone()));
            dictionary.insert(PAGES_KEY.to_owned(), Self::encode_pages(&draft.pages));
            dictionary.insert(SEEDED_PAGES_KEY.to_owned(), Self::encode_pages(&draft.seeded_pages));
            drafts.insert(kind.raw_value().to_owned(), Value::Object(dictionary));
        }
        let mut raw = Map::new();
        raw.insert(VERSION_KEY.to_owned(), Value::from(FORMAT_VERSION));
        raw.insert(PAGE_KEY.to_owned(), Value::from(state.page.raw_value()));
        raw.insert(ACTIVE_KIND_KEY.to_owned(), Value::from(state.active_kind.raw_value()));
        raw.insert(DRAFTS_KEY.to_owned(), Value::Object(drafts));
        Value::Object(raw)
    }

    fn encode_pages(pages: &[RiseBugReportSurface]) -> Value {
        Value::Array(pages.iter().map(|page| Value::from(page.wire())).collect())
    }

    fn decode(raw: &Map<String, Value>) -> Option<BugReportPersistedState> {
        if raw.get(VERSION_KEY).and_then(Value::as_i64) != Some(FORMAT_VERSION) {
            return None;
        }
        let mut state = BugReportPersistedState {
            page: raw
                .get(PAGE_KEY)
                .and_then(Value::as_str)
                .and_then(BugReportSheetPage::from_raw_value)
                .unwrap_or(BugReportSheetPage::Root),
            active_kind: raw
                .get(ACTIVE_KIND_KEY)
                .and_then(Value::as_str)
                .and_then(BugReportDraftKind::from_raw_value)
                .unwrap_or(BugReportDraftKind::Bug),
            ..BugReportPersistedState::default()
        };
        let Some(drafts) = raw.get(DRAFTS_KEY).and_then(Value::as_object) else {
            return Some(state);
        };
        for (key, value) in drafts {
            let Some(kind) = BugReportDraftKind::from_raw_value(key) else {
                continue;
            };
            let Some(dictionary) = value.as_object() else {
                continue;
            };
            let Some(client_report_id) = dictionary
                .get(CLIENT_REPORT_ID_KEY)
                .and_then(Value::as_str)
                .filter(|client_report_id| !client_report_id.is_empty())
            else {
                continue;
            };
            state.drafts.insert(
                kind,
                BugReportPersistedDraft {
                    client_report_id: client_report_id.to_owned(),
                    kind,
                    feedback_kind: dictionary
                        .get(FEEDBACK_KIND_KEY)
                        .and_then(Value::as_str)
                        .and_then(RiseFeedbackKind::from_wire)
                        .unwrap_or(DIAGNOSTICS_FEEDBACK_KINDS_DEFAULT),
                    message: Self::decode_text(dictionary, MESSAGE_KEY),
                    component: Self::decode_text(dictionary, COMPONENT_KEY),
                    pages: Self::decode_pages(dictionary, PAGES_KEY),
                    seeded_pages: Self::decode_pages(dictionary, SEEDED_PAGES_KEY),
                },
            );
        }
        Some(state)
    }

    fn decode_text(dictionary: &Map<String, Value>, key: &str) -> String {
        dictionary.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
    }

    fn decode_pages(dictionary: &Map<String, Value>, key: &str) -> Vec<RiseBugReportSurface> {
        dictionary
            .get(key)
            .and_then(Value::as_array)
            .map(|pages| {
                pages.iter().filter_map(Value::as_str).filter_map(RiseBugReportSurface::from_wire).collect()
            })
            .unwrap_or_default()
    }
}
