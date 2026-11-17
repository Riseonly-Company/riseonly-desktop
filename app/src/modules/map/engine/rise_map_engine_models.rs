use std::fmt;

use crate::modules::map::stores::map_actions::map_types::{
    MapFriend, MapFriendRequest, MapFriendRequestOutcome, MapInvite, MapInvitePreview, MapLiveFriend,
    MapMergePreview, MapState,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RiseMapLoadState {
    Idle,
    Loading,
    Loaded,
    Failed,
}

impl RiseMapLoadState {
    pub const fn raw_value(self) -> &'static str {
        match self {
            RiseMapLoadState::Idle => "idle",
            RiseMapLoadState::Loading => "loading",
            RiseMapLoadState::Loaded => "loaded",
            RiseMapLoadState::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RiseMapCacheSource {
    Miss,
    Memory,
    PersistentStore,
    Network,
}

impl RiseMapCacheSource {
    pub const fn raw_value(self) -> &'static str {
        match self {
            RiseMapCacheSource::Miss => "miss",
            RiseMapCacheSource::Memory => "memory",
            RiseMapCacheSource::PersistentStore => "persistent-store",
            RiseMapCacheSource::Network => "network",
        }
    }
}

/// membership and settings
/// state none means neither store nor server answered yet so the screen shows its skeleton
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseMapStateSnapshot {
    pub state: Option<MapState>,
    pub is_joining: bool,
    pub load_state: RiseMapLoadState,
    pub error_text: Option<String>,
    pub cache_source: RiseMapCacheSource,
    pub revision: u64,
}

impl RiseMapStateSnapshot {
    pub const EMPTY: RiseMapStateSnapshot = RiseMapStateSnapshot {
        state: None,
        is_joining: false,
        load_state: RiseMapLoadState::Idle,
        error_text: None,
        cache_source: RiseMapCacheSource::Miss,
        revision: 0,
    };

    pub fn is_member(&self) -> bool {
        self.state.as_ref().is_some_and(|state| state.is_member)
    }

    pub fn shows_initial_loader(&self) -> bool {
        self.state.is_none() && matches!(self.load_state, RiseMapLoadState::Idle | RiseMapLoadState::Loading)
    }

    pub fn shows_blocking_error(&self) -> bool {
        self.state.is_none() && self.load_state == RiseMapLoadState::Failed
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseMapFriendsSnapshot {
    pub friends: Vec<MapFriend>,
    pub incoming: Vec<MapFriendRequest>,
    pub outgoing: Vec<MapFriendRequest>,
    pub has_more_friends: bool,
    pub has_more_incoming: bool,
    pub has_more_outgoing: bool,
    pub is_loading_more: bool,
    pub invite: Option<MapInvite>,
    pub load_state: RiseMapLoadState,
    pub error_text: Option<String>,
    pub cache_source: RiseMapCacheSource,
    pub revision: u64,
}

impl RiseMapFriendsSnapshot {
    pub const EMPTY: RiseMapFriendsSnapshot = RiseMapFriendsSnapshot {
        friends: Vec::new(),
        incoming: Vec::new(),
        outgoing: Vec::new(),
        has_more_friends: false,
        has_more_incoming: false,
        has_more_outgoing: false,
        is_loading_more: false,
        invite: None,
        load_state: RiseMapLoadState::Idle,
        error_text: None,
        cache_source: RiseMapCacheSource::Miss,
        revision: 0,
    };

    pub fn is_empty(&self) -> bool {
        self.friends.is_empty() && self.incoming.is_empty() && self.outgoing.is_empty()
    }

    pub fn shows_initial_loader(&self) -> bool {
        self.is_empty() && matches!(self.load_state, RiseMapLoadState::Idle | RiseMapLoadState::Loading)
    }

    pub fn shows_blocking_error(&self) -> bool {
        self.is_empty() && self.load_state == RiseMapLoadState::Failed
    }
}

/// stable order, the order the session first listed them with newcomers appended
/// a pin keeps its slot while it moves so the layer diffs by id and never reorders
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseMapLiveSnapshot {
    pub friends: Vec<MapLiveFriend>,
    pub is_session_open: bool,
    pub cache_source: RiseMapCacheSource,
    pub revision: u64,
}

impl RiseMapLiveSnapshot {
    pub const EMPTY: RiseMapLiveSnapshot =
        RiseMapLiveSnapshot { friends: Vec::new(), is_session_open: false, cache_source: RiseMapCacheSource::Miss, revision: 0 };
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RiseMapError {
    MissingActiveSession,
    StaleAccount,
    NotMember,
    InvalidResponse,
}

impl RiseMapError {
    pub fn error_description(&self) -> Option<String> {
        let text = match self {
            RiseMapError::MissingActiveSession => "No active account session.",
            RiseMapError::StaleAccount => "The account changed while the request was running.",
            RiseMapError::NotMember => "The account has not joined the map.",
            RiseMapError::InvalidResponse => "The map answered with data that does not decode.",
        };
        Some(text.to_owned())
    }
}

impl fmt::Display for RiseMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for RiseMapError {}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RiseMapMutationOutcome {
    Done,
    Failed(Option<String>),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RiseMapReactionOutcome {
    Delivered,
    Paced,
    Lost,
}

#[derive(Clone, Debug)]
pub enum RiseMapFriendRequestResult {
    Sent(MapFriendRequestOutcome),
    Failed(Option<String>),
}

#[derive(Clone, Debug)]
pub enum RiseMapInvitePreviewResult {
    Loaded(MapInvitePreview),
    Failed(Option<String>),
}

#[derive(Clone, Debug)]
pub enum RiseMapMergePreviewResult {
    Loaded(MapMergePreview),
    Failed(Option<String>),
}
