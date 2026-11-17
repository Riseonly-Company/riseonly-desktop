use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::rise_store::rise_store_models::RiseStoreNamespace;

/// direction of one part of a byte sortable view key
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RisePositionKeyOrder {
    Ascending,
    Descending,
}

/// opaque key whose byte order is the logical order of a view
/// build it with the key builder, feature code must not decode or assemble persisted keys
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RisePositionKey {
    raw_value: Vec<u8>,
}

impl RisePositionKey {
    pub fn new(raw_value: Vec<u8>) -> Self {
        assert!(!raw_value.is_empty() && raw_value.len() <= 8_192);
        Self { raw_value }
    }

    pub fn raw_value(&self) -> &[u8] {
        &self.raw_value
    }

    pub fn make(body: impl FnOnce(&mut RisePositionKeyBuilder)) -> Self {
        let mut builder = RisePositionKeyBuilder::new();
        body(&mut builder);
        builder.build()
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct RisePositionKeyBuilder {
    bytes: Vec<u8>,
}

impl RisePositionKeyBuilder {
    pub fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub fn append_signed_int64(&mut self, value: i64) {
        self.append_signed_int64_with_order(value, RisePositionKeyOrder::Ascending);
    }

    pub fn append_signed_int64_with_order(&mut self, value: i64, order: RisePositionKeyOrder) {
        self.bytes.push(match order {
            RisePositionKeyOrder::Ascending => 0x11,
            RisePositionKeyOrder::Descending => 0x12,
        });
        let sortable = (value as u64) ^ 0x8000_0000_0000_0000;
        let raw_bytes = sortable.to_be_bytes();
        match order {
            RisePositionKeyOrder::Ascending => self.bytes.extend_from_slice(&raw_bytes),
            RisePositionKeyOrder::Descending => {
                self.bytes.extend(raw_bytes.iter().map(|byte| !byte));
            }
        }
    }

    pub fn append_string(&mut self, value: &str) {
        self.append_string_with_order(value, RisePositionKeyOrder::Ascending);
    }

    pub fn append_string_with_order(&mut self, value: &str, order: RisePositionKeyOrder) {
        assert!(value.len() <= 4_096);
        self.bytes.push(match order {
            RisePositionKeyOrder::Ascending => 0x21,
            RisePositionKeyOrder::Descending => 0x22,
        });

        for byte in value.bytes() {
            if byte == 0 {
                self.append_string_byte(0, order);
                self.append_string_byte(0xff, order);
            } else {
                self.append_string_byte(byte, order);
            }
        }
        self.append_string_byte(0, order);
        self.append_string_byte(0, order);
    }

    pub fn appending_signed_int64(&self, value: i64) -> Self {
        self.appending_signed_int64_with_order(value, RisePositionKeyOrder::Ascending)
    }

    pub fn appending_signed_int64_with_order(
        &self,
        value: i64,
        order: RisePositionKeyOrder,
    ) -> Self {
        let mut copy = self.clone();
        copy.append_signed_int64_with_order(value, order);
        copy
    }

    pub fn appending_string(&self, value: &str) -> Self {
        self.appending_string_with_order(value, RisePositionKeyOrder::Ascending)
    }

    pub fn appending_string_with_order(&self, value: &str, order: RisePositionKeyOrder) -> Self {
        let mut copy = self.clone();
        copy.append_string_with_order(value, order);
        copy
    }

    pub fn build(&self) -> RisePositionKey {
        RisePositionKey::new(self.bytes.clone())
    }

    fn append_string_byte(&mut self, byte: u8, order: RisePositionKeyOrder) {
        self.bytes.push(match order {
            RisePositionKeyOrder::Ascending => byte,
            RisePositionKeyOrder::Descending => !byte,
        });
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RiseEntityAliasKind {
    raw_value: Cow<'static, str>,
}

impl RiseEntityAliasKind {
    pub fn new(raw_value: impl Into<String>) -> Self {
        let raw_value = raw_value.into();
        assert!(!raw_value.is_empty() && raw_value.len() <= 256);
        Self {
            raw_value: Cow::Owned(raw_value),
        }
    }

    pub fn raw_value(&self) -> &str {
        &self.raw_value
    }

    pub fn server_id() -> Self {
        Self {
            raw_value: Cow::Borrowed("server"),
        }
    }

    pub fn client_id() -> Self {
        Self {
            raw_value: Cow::Borrowed("client"),
        }
    }

    pub fn temporary_id() -> Self {
        Self {
            raw_value: Cow::Borrowed("temporary"),
        }
    }

    /// marks a device only row that trim_entity_view must never drop, value is the local id
    pub fn retained() -> Self {
        Self {
            raw_value: Cow::Borrowed("retained"),
        }
    }
}

/// stable alternate identity of a canonical local entity
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct RiseEntityAlias {
    pub kind: RiseEntityAliasKind,
    pub value: String,
}

impl RiseEntityAlias {
    pub fn new(kind: RiseEntityAliasKind, value: impl Into<String>) -> Self {
        let value = value.into();
        assert!(!value.is_empty() && value.len() <= 4_096);
        Self { kind, value }
    }
}

/// the canonical encrypted entity, views and aliases point to local_id and dont copy data
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseStoreEntity {
    pub local_id: String,
    pub data: Vec<u8>,
    pub version: i64,
    pub updated_at_milliseconds: i64,
}

impl RiseStoreEntity {
    pub fn new(
        local_id: impl Into<String>,
        data: Vec<u8>,
        version: i64,
        updated_at_milliseconds: i64,
    ) -> Self {
        let local_id = local_id.into();
        assert!(!local_id.is_empty() && local_id.len() <= 4_096);
        Self {
            local_id,
            data,
            version,
            updated_at_milliseconds,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RiseEntityWritePolicy {
    RequireIdenticalReplay,
    ServerAuthoritative,
    IgnoreIfNotNewer,
    CompareAndSwap { expected: RiseStoreEntity },
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RiseEntityKey {
    pub namespace: RiseStoreNamespace,
    pub local_id: String,
}

impl RiseEntityKey {
    pub fn new(namespace: RiseStoreNamespace, local_id: impl Into<String>) -> Self {
        let local_id = local_id.into();
        assert!(!local_id.is_empty() && local_id.len() <= 4_096);
        Self {
            namespace,
            local_id,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RiseEntityAliasLookup {
    pub namespace: RiseStoreNamespace,
    pub alias: RiseEntityAlias,
}

impl RiseEntityAliasLookup {
    pub fn new(namespace: RiseStoreNamespace, alias: RiseEntityAlias) -> Self {
        Self { namespace, alias }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RiseEntityViewKey {
    pub namespace: RiseStoreNamespace,
    pub view: String,
}

impl RiseEntityViewKey {
    pub fn new(namespace: RiseStoreNamespace, view: impl Into<String>) -> Self {
        let view = view.into();
        assert!(!view.is_empty() && view.len() <= 4_096);
        Self { namespace, view }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseEntityViewMembership {
    pub local_id: String,
    pub position_key: RisePositionKey,
    pub version: i64,
}

impl RiseEntityViewMembership {
    pub fn new(local_id: impl Into<String>, position_key: RisePositionKey, version: i64) -> Self {
        let local_id = local_id.into();
        assert!(!local_id.is_empty() && local_id.len() <= 4_096);
        Self {
            local_id,
            position_key,
            version,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RiseEntityViewRetention {
    First(usize),
    Latest(usize),
}

impl RiseEntityViewRetention {
    pub(crate) fn limit(&self) -> usize {
        match self {
            Self::First(limit) | Self::Latest(limit) => *limit,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RiseEntityViewPosition {
    pub position_key: RisePositionKey,
    pub local_id: String,
}

impl RiseEntityViewPosition {
    pub fn new(position_key: RisePositionKey, local_id: impl Into<String>) -> Self {
        let local_id = local_id.into();
        assert!(!local_id.is_empty() && local_id.len() <= 4_096);
        Self {
            position_key,
            local_id,
        }
    }
}

/// results come in the view ascending order, Latest picks the highest keys and First the lowest
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RiseEntityWindow {
    First {
        limit: usize,
    },
    Latest {
        limit: usize,
    },
    Before {
        anchor: RiseEntityViewPosition,
        limit: usize,
    },
    After {
        anchor: RiseEntityViewPosition,
        limit: usize,
    },
    Around {
        anchor: RiseEntityViewPosition,
        before_limit: usize,
        after_limit: usize,
    },
}

impl RiseEntityWindow {
    pub(crate) fn requested_item_count(&self) -> usize {
        match self {
            Self::First { limit }
            | Self::Latest { limit }
            | Self::Before { limit, .. }
            | Self::After { limit, .. } => *limit,
            Self::Around {
                before_limit,
                after_limit,
                ..
            } => before_limit
                .checked_add(*after_limit)
                .and_then(|sum| sum.checked_add(1))
                .unwrap_or(usize::MAX),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RiseEntityWindowQuery {
    pub key: RiseEntityViewKey,
    pub window: RiseEntityWindow,
}

impl RiseEntityWindowQuery {
    pub fn new(
        namespace: RiseStoreNamespace,
        view: impl Into<String>,
        window: RiseEntityWindow,
    ) -> Self {
        Self {
            key: RiseEntityViewKey::new(namespace, view),
            window,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseEntityViewItem {
    pub entity: RiseStoreEntity,
    pub position_key: RisePositionKey,
    pub membership_version: i64,
}

impl RiseEntityViewItem {
    pub fn new(
        entity: RiseStoreEntity,
        position_key: RisePositionKey,
        membership_version: i64,
    ) -> Self {
        Self {
            entity,
            position_key,
            membership_version,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiseEntityViewLoadState {
    Idle,
    Loading,
    Loaded,
    Failed,
}

impl RiseEntityViewLoadState {
    pub const ALL_CASES: [Self; 4] = [Self::Idle, Self::Loading, Self::Loaded, Self::Failed];

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Loading => "loading",
            Self::Loaded => "loaded",
            Self::Failed => "failed",
        }
    }

    pub fn from_raw(raw_value: &str) -> Option<Self> {
        Self::ALL_CASES
            .into_iter()
            .find(|state| state.raw_value() == raw_value)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseEntityViewState {
    pub revision: i64,
    pub before_cursor: Option<Vec<u8>>,
    pub after_cursor: Option<Vec<u8>>,
    pub has_more_before: bool,
    pub has_more_after: bool,
    pub load_state: RiseEntityViewLoadState,
    pub last_error_code: Option<String>,
    pub updated_at_milliseconds: i64,
}

impl RiseEntityViewState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        revision: i64,
        before_cursor: Option<Vec<u8>>,
        after_cursor: Option<Vec<u8>>,
        has_more_before: bool,
        has_more_after: bool,
        load_state: RiseEntityViewLoadState,
        last_error_code: Option<String>,
        updated_at_milliseconds: i64,
    ) -> Self {
        Self {
            revision,
            before_cursor,
            after_cursor,
            has_more_before,
            has_more_after,
            load_state,
            last_error_code,
            updated_at_milliseconds,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseEntityWindowResult {
    pub items: Vec<RiseEntityViewItem>,
    pub state: Option<RiseEntityViewState>,
    pub total_count: usize,
    pub has_cached_before: bool,
    pub has_cached_after: bool,
    pub anchor_index: Option<usize>,
    pub resolved_anchor: Option<RiseEntityViewPosition>,
}

impl RiseEntityWindowResult {
    pub fn new(
        items: Vec<RiseEntityViewItem>,
        state: Option<RiseEntityViewState>,
        total_count: usize,
        has_cached_before: bool,
        has_cached_after: bool,
        anchor_index: Option<usize>,
        resolved_anchor: Option<RiseEntityViewPosition>,
    ) -> Self {
        Self {
            items,
            state,
            total_count,
            has_cached_before,
            has_cached_after,
            anchor_index,
            resolved_anchor,
        }
    }
}
