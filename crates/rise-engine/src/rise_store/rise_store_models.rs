use std::fmt;

use serde::{Deserialize, Serialize};

/// Account-scoped storage partition. Versioned so a shape change becomes a new
/// namespace rather than a migration of live rows.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Namespace(String);

impl Namespace {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Namespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LocalId(String);

impl LocalId {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ViewId(String);

impl ViewId {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Byte-sortable ordering key for a row inside a materialized view.
///
/// Opaque on purpose: the server decides ordering, and comparing raw bytes lets
/// a page merge without the client re-deriving a sort. Never build one from a
/// float or a formatted timestamp.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct PositionKey(Vec<u8>);

impl PositionKey {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Option<Self> {
        let bytes = bytes.into();
        (!bytes.is_empty()).then_some(Self(bytes))
    }

    // Biased into unsigned space so negative ordinals still sort before positive
    // ones under a plain byte comparison.
    pub fn from_ordinal(ordinal: i64) -> Self {
        let biased = (ordinal as i128 - i64::MIN as i128) as u64;
        Self(biased.to_be_bytes().to_vec())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LoadState {
    #[default]
    Idle,
    Loading,
    Loaded,
    Failed,
}

impl LoadState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Loading => "loading",
            Self::Loaded => "loaded",
            Self::Failed => "failed",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "idle" => Some(Self::Idle),
            "loading" => Some(Self::Loading),
            "loaded" => Some(Self::Loaded),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Where a value came from, recorded so the debugger can prove a cache hit did
/// not touch the network.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CacheSource {
    Memory,
    PersistentStore,
    Miss,
    Network,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ViewState {
    pub revision: i64,
    pub before_cursor: Option<Vec<u8>>,
    pub after_cursor: Option<Vec<u8>>,
    pub has_more_before: bool,
    pub has_more_after: bool,
    pub load_state: LoadState,
    pub last_error_code: Option<String>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            revision: 0,
            before_cursor: None,
            after_cursor: None,
            has_more_before: false,
            has_more_after: false,
            load_state: LoadState::Idle,
            last_error_code: None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StoredEntity {
    pub local_id: LocalId,
    pub payload: Vec<u8>,
    pub version: i64,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ViewRow {
    pub local_id: LocalId,
    pub position: PositionKey,
    pub payload: Vec<u8>,
    pub version: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_key_must_carry_bytes() {
        assert!(PositionKey::new(Vec::new()).is_none());
        assert!(PositionKey::new(vec![1]).is_some());
    }

    #[test]
    fn ordinal_keys_sort_in_ordinal_order_as_raw_bytes() {
        let mut keys: Vec<PositionKey> = [5i64, -3, 0, i64::MIN, i64::MAX, -1]
            .into_iter()
            .map(PositionKey::from_ordinal)
            .collect();
        keys.sort();

        let expected: Vec<PositionKey> = [i64::MIN, -3, -1, 0, 5, i64::MAX]
            .into_iter()
            .map(PositionKey::from_ordinal)
            .collect();

        assert_eq!(
            keys, expected,
            "byte order must match ordinal order or page merges reorder rows"
        );
    }

    #[test]
    fn load_state_round_trips_through_its_wire_form() {
        for state in [
            LoadState::Idle,
            LoadState::Loading,
            LoadState::Loaded,
            LoadState::Failed,
        ] {
            assert_eq!(LoadState::parse(state.as_str()), Some(state));
        }
        assert_eq!(LoadState::parse("nonsense"), None);
    }

    #[test]
    fn a_fresh_view_state_claims_nothing() {
        let state = ViewState::default();
        assert_eq!(state.revision, 0);
        assert_eq!(state.load_state, LoadState::Idle);
        assert!(!state.has_more_before && !state.has_more_after);
        assert!(state.before_cursor.is_none() && state.after_cursor.is_none());
    }
}
