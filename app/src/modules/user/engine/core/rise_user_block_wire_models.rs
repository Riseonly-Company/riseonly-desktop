use std::sync::LazyLock;

use rise_canon::RiseEvent;
use rise_engine::RiseWirePushEnvelope;
use rise_privacy::BlockScope;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// tiers of a personal block. its one-way and the tier belongs to who placed it,
/// missing key or empty string both mean no block
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseBlockScope {
    Standard,
    Total,
}

impl RiseBlockScope {
    pub const ALL_CASES: [RiseBlockScope; 2] = [RiseBlockScope::Standard, RiseBlockScope::Total];

    pub fn raw_value(self) -> &'static str {
        self.canon().wire()
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        BlockScope::from_wire(raw).map(|scope| match scope {
            BlockScope::Standard => RiseBlockScope::Standard,
            BlockScope::Total => RiseBlockScope::Total,
        })
    }

    fn canon(self) -> BlockScope {
        match self {
            RiseBlockScope::Standard => BlockScope::Standard,
            RiseBlockScope::Total => BlockScope::Total,
        }
    }
}

impl Serialize for RiseBlockScope {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.raw_value())
    }
}

impl<'de> Deserialize<'de> for RiseBlockScope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        static VARIANTS: LazyLock<[&str; 2]> = LazyLock::new(|| [BlockScope::Standard.wire(), BlockScope::Total.wire()]);
        let raw = String::deserialize(deserializer)?;
        Self::from_raw_value(&raw).ok_or_else(|| serde::de::Error::unknown_variant(&raw, &*VARIANTS))
    }
}

/// one user_block_changed frame,
/// sent only to the blockers own devices so it never reveals that someone blocked you
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserBlockChange {
    pub target_user_id: String,
    pub is_blocked: bool,
    pub scope: Option<RiseBlockScope>,
    pub blocked_at_ms: i64,
}

pub enum RiseUserBlockRealtime {}

impl RiseUserBlockRealtime {
    pub const EVENT_TYPE: &'static str = rise_events::name::USER_BLOCK_CHANGED;

    /// must be total, a failing reducer makes sync demand a full resync so anything malformed returns none
    pub fn decode(envelope: &RiseWirePushEnvelope) -> Option<RiseUserBlockChange> {
        let data = envelope.data.data(false).ok()?;
        let RiseEvent::UserBlockChanged(change) = RiseEvent::decode(&envelope.r#type, &data).ok()? else {
            return None;
        };
        if change.target_user_id.is_empty() {
            return None;
        }
        Some(RiseUserBlockChange {
            target_user_id: change.target_user_id.clone(),
            is_blocked: change.is_blocked,
            scope: change
                .is_blocked
                .then(|| RiseBlockScope::from_raw_value(&change.block_scope).unwrap_or(RiseBlockScope::Standard)),
            blocked_at_ms: change.blocked_at_ms,
        })
    }
}
