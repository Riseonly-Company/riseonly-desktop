use std::collections::HashMap;
use std::sync::LazyLock;

use rise_canon::RiseLimits;
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RiseMessageScope {
    #[serde(rename = "chatID")]
    pub chat_id: String,
    #[serde(rename = "topicID", default, skip_serializing_if = "Option::is_none")]
    pub topic_id: Option<String>,
    #[serde(rename = "threadRootID", default, skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<String>,
}

impl RiseMessageScope {
    pub fn new(chat_id: String, topic_id: Option<String>, thread_root_id: Option<String>) -> Self {
        assert!(!chat_id.is_empty());
        Self { chat_id, topic_id, thread_root_id }
    }

    /// matches the scroll and cache identity and keeps nil topic apart from the empty general topic id
    pub fn storage_key(&self) -> String {
        if let Some(thread_root_id) = &self.thread_root_id {
            return format!("{}::th::{}", self.chat_id, thread_root_id);
        }
        if let Some(topic_id) = &self.topic_id {
            return format!("{}::t::{}", self.chat_id, topic_id);
        }
        self.chat_id.clone()
    }

    pub fn parse(storage_key: &str) -> Option<Self> {
        if let Some((chat_id, thread)) = storage_key.split_once("::th::") {
            if chat_id.is_empty() {
                return None;
            }
            return Some(Self::new(chat_id.to_owned(), None, Some(thread.to_owned())));
        }
        if let Some((chat_id, topic)) = storage_key.split_once("::t::") {
            if chat_id.is_empty() {
                return None;
            }
            return Some(Self::new(chat_id.to_owned(), Some(topic.to_owned()), None));
        }
        if storage_key.is_empty() {
            return None;
        }
        Some(Self::new(storage_key.to_owned(), None, None))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RiseMessageAnchor {
    Latest,
    LocalID(String),
    ServerID(String),
    SavedServerID(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseMessagePageDirection {
    Older,
    Newer,
}

impl RiseMessagePageDirection {
    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::Older => "older",
            Self::Newer => "newer",
        }
    }

    pub const fn wire_up(self) -> bool {
        matches!(self, Self::Older)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseMessageWindowShift {
    Older,
    Newer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseMessagePageLoadOutcome {
    Loaded { count: i64 },
    StaleResponseDropped,
    CursorStalled,
    EndReached,
    AlreadyLoading,
}

/// superseded isnt a failure, a newer jump or release or account change took the window
/// only failed is a dead end the reader must be told about
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseMessageJumpOutcome {
    Installed,
    Superseded,
    Failed,
}

impl RiseMessageJumpOutcome {
    pub const fn is_installed(self) -> bool {
        matches!(self, Self::Installed)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseMessagePageLoadReceipt {
    pub outcome: RiseMessagePageLoadOutcome,
    pub store_revision: Option<i64>,
    pub entity_versions_by_local_id: HashMap<String, i64>,
    pub used_latest_fallback: bool,
    pub anchor_resolution_failed_authoritatively: bool,
    pub resolved_anchor_id: Option<String>,
}

impl RiseMessagePageLoadReceipt {
    pub fn new(outcome: RiseMessagePageLoadOutcome, store_revision: Option<i64>) -> Self {
        Self {
            outcome,
            store_revision,
            entity_versions_by_local_id: HashMap::new(),
            used_latest_fallback: false,
            anchor_resolution_failed_authoritatively: false,
            resolved_anchor_id: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RiseMessageWindowConfiguration {
    pub before: i64,
    pub after: i64,
}

impl Default for RiseMessageWindowConfiguration {
    fn default() -> Self {
        Self::new(160, 159)
    }
}

static INITIAL_VIEWPORT: LazyLock<RiseMessageWindowConfiguration> = LazyLock::new(|| {
    let count = RiseLimits::client_value("message_initial_presentation_rows");
    RiseMessageWindowConfiguration::new(count / 2, count - count / 2 - 1)
});

impl RiseMessageWindowConfiguration {
    pub fn new(before: i64, after: i64) -> Self {
        assert!(before >= 0 && after >= 0 && before + after < 320);
        Self { before, after }
    }

    pub fn presentation() -> Self {
        Self::default()
    }

    pub fn initial_viewport() -> Self {
        *INITIAL_VIEWPORT
    }

    pub fn older_shift() -> Self {
        Self::new(256, 63)
    }

    pub fn newer_shift() -> Self {
        Self::new(64, 255)
    }

    pub fn shifted(direction: RiseMessageWindowShift) -> Self {
        match direction {
            RiseMessageWindowShift::Older => Self::older_shift(),
            RiseMessageWindowShift::Newer => Self::newer_shift(),
        }
    }

    pub fn shifted_anchor_offset_in_window(
        direction: RiseMessageWindowShift,
        anchor_offset_in_window: Option<i64>,
        stride: i64,
        window_size: Option<i64>,
        growth: i64,
    ) -> Self {
        let Some(anchor_offset_in_window) = anchor_offset_in_window.filter(|_| stride > 0) else {
            return Self::shifted(direction);
        };
        let presentation = Self::presentation();
        let maximum_span = presentation.before + presentation.after + 1;
        let span = maximum_span.min(window_size.unwrap_or(maximum_span).max(1));
        let offset = (span - 1).min(anchor_offset_in_window).max(0);
        let target = match direction {
            RiseMessageWindowShift::Older => offset + stride,
            RiseMessageWindowShift::Newer => offset - (stride - growth).max(0),
        };
        let before = (span - 1).min(target).max(0);
        Self::new(before, span - 1 - before)
    }
}
