use std::collections::HashSet;
use std::sync::LazyLock;

use parking_lot::Mutex;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseInboundOwner {
    Legacy,
    RiseEngine,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseInboundAccountScope {
    pub account_id: String,
    pub account_epoch: String,
}

impl RiseInboundAccountScope {
    pub fn new(account_id: Option<&str>, account_epoch: Option<&str>) -> Option<Self> {
        Some(Self {
            account_id: Self::normalized(account_id)?,
            account_epoch: Self::normalized(account_epoch)?,
        })
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(value?);
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }
}

/// pure routing policy kept apart from the lock backed router
/// so ownership can be unit tested without engine or socket
pub enum RiseInboundOwnershipPolicy {}

impl RiseInboundOwnershipPolicy {
    pub fn owner(
        for_push_event_type: Option<&str>,
        inbound_account: Option<&RiseInboundAccountScope>,
        rise_owner_account: Option<&RiseInboundAccountScope>,
        rise_owned_push_event_types: &HashSet<String>,
    ) -> RiseInboundOwner {
        let Some(inbound_account) = inbound_account else {
            return RiseInboundOwner::Legacy;
        };
        if rise_owner_account != Some(inbound_account) {
            return RiseInboundOwner::Legacy;
        }
        let Some(event_type) = Self::normalized(for_push_event_type) else {
            return RiseInboundOwner::Legacy;
        };
        if !rise_owned_push_event_types.contains(&event_type) {
            return RiseInboundOwner::Legacy;
        }
        RiseInboundOwner::RiseEngine
    }

    pub fn normalized(event_type: Option<&str>) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(event_type?);
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }

    pub fn normalized_set(event_types: &HashSet<String>) -> HashSet<String> {
        event_types
            .iter()
            .filter_map(|event_type| Self::normalized(Some(event_type)))
            .collect()
    }
}

#[derive(Default)]
struct RiseInboundOwnershipRouterState {
    owner_id: Option<Uuid>,
    owner_account: Option<RiseInboundAccountScope>,
    rise_owned_push_event_types: HashSet<String>,
}

pub struct RiseInboundOwnershipRouter {
    state: Mutex<RiseInboundOwnershipRouterState>,
}

static SHARED: LazyLock<RiseInboundOwnershipRouter> = LazyLock::new(|| {
    RiseInboundOwnershipRouter { state: Mutex::new(RiseInboundOwnershipRouterState::default()) }
});

impl RiseInboundOwnershipRouter {
    pub fn shared() -> &'static RiseInboundOwnershipRouter {
        &SHARED
    }

    pub fn owner(
        &self,
        for_push_event_type: Option<&str>,
        account_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> RiseInboundOwner {
        let inbound_account = RiseInboundAccountScope::new(account_id, account_epoch);
        let state = self.state.lock();
        RiseInboundOwnershipPolicy::owner(
            for_push_event_type,
            inbound_account.as_ref(),
            state.owner_account.as_ref(),
            &state.rise_owned_push_event_types,
        )
    }

    pub fn install(
        &self,
        owner_id: Uuid,
        account_id: &str,
        account_epoch: &str,
        rise_owned_push_event_types: &HashSet<String>,
    ) {
        let Some(owner_account) =
            RiseInboundAccountScope::new(Some(account_id), Some(account_epoch))
        else {
            return;
        };
        let normalized_types =
            RiseInboundOwnershipPolicy::normalized_set(rise_owned_push_event_types);
        let mut state = self.state.lock();
        state.owner_id = Some(owner_id);
        state.owner_account = Some(owner_account);
        state.rise_owned_push_event_types = normalized_types;
    }

    pub fn relinquish(&self, owner_id: Uuid) {
        let mut state = self.state.lock();
        if state.owner_id != Some(owner_id) {
            return;
        }
        state.owner_id = None;
        state.owner_account = None;
        state.rise_owned_push_event_types = HashSet::new();
    }

    pub fn rise_owned_push_event_types(&self) -> HashSet<String> {
        self.state.lock().rise_owned_push_event_types.clone()
    }
}
