use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::character_prefix;
use rise_limits::limit::{DIAGNOSTICS_BREADCRUMB_BYTES, DIAGNOSTICS_BREADCRUMBS};

/// rough origin of a breadcrumb so triage can filter without parsing the message
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseBreadcrumbsChannel {
    Lifecycle,
    Session,
    Socket,
    Rpc,
    Media,
    Screen,
    Failure,
}

impl RiseBreadcrumbsChannel {
    pub const fn raw_value(&self) -> &'static str {
        match self {
            Self::Lifecycle => "lifecycle",
            Self::Session => "session",
            Self::Socket => "socket",
            Self::Rpc => "rpc",
            Self::Media => "media",
            Self::Screen => "screen",
            Self::Failure => "failure",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        match raw {
            "lifecycle" => Some(Self::Lifecycle),
            "session" => Some(Self::Session),
            "socket" => Some(Self::Socket),
            "rpc" => Some(Self::Rpc),
            "media" => Some(Self::Media),
            "screen" => Some(Self::Screen),
            "failure" => Some(Self::Failure),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseBreadcrumbsEntry {
    pub at_ms: i64,
    pub channel: RiseBreadcrumbsChannel,
    pub message: String,
}

const MAXIMUM_ENTRIES: usize = DIAGNOSTICS_BREADCRUMBS as usize;
const MAXIMUM_BYTES: usize = DIAGNOSTICS_BREADCRUMB_BYTES as usize;

const MAXIMUM_MESSAGE_CHARACTERS: usize = 200;

const ENTRY_OVERHEAD_BYTES: usize = 16;

struct Ring {
    entries: VecDeque<RiseBreadcrumbsEntry>,
    stored_bytes: usize,
}

static RING: Mutex<Ring> = Mutex::new(Ring {
    entries: VecDeque::new(),
    stored_bytes: 0,
});

fn ring() -> std::sync::MutexGuard<'static, Ring> {
    RING.lock().unwrap_or_else(PoisonError::into_inner)
}

fn cost(message: &str) -> usize {
    message.len() + ENTRY_OVERHEAD_BYTES
}

pub enum RiseBreadcrumbs {}

impl RiseBreadcrumbs {
    /// safe from any thread even on crash adjacent paths, no disk
    /// message is lazy so turning the ring off later stays a one line change
    pub fn record(channel: RiseBreadcrumbsChannel, message: impl FnOnce() -> String) {
        let full = message();
        let text = character_prefix(&full, MAXIMUM_MESSAGE_CHARACTERS).to_owned();
        let entry_cost = cost(&text);
        let entry = RiseBreadcrumbsEntry {
            at_ms: Date::now().ms(),
            channel,
            message: text,
        };

        let mut ring = ring();
        ring.entries.push_back(entry);
        ring.stored_bytes += entry_cost;

        while ring.entries.len() > MAXIMUM_ENTRIES
            || (ring.stored_bytes > MAXIMUM_BYTES && !ring.entries.is_empty())
        {
            let Some(removed) = ring.entries.pop_front() else {
                break;
            };
            ring.stored_bytes = ring.stored_bytes.saturating_sub(cost(&removed.message));
        }
    }

    /// entries oldest first
    pub fn snapshot() -> Vec<RiseBreadcrumbsEntry> {
        ring().entries.iter().cloned().collect()
    }

    /// clears the ring on account switch and sign out
    /// reports from the second account must not carry the first one
    pub fn reset() {
        let mut ring = ring();
        ring.entries.clear();
        ring.stored_bytes = 0;
    }
}
