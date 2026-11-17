use std::collections::HashSet;
use std::sync::LazyLock;

use rise_canon::RiseLimits;
use rise_canon::rise_proto::map::ProtoProfileResponse;
use rise_core::RiseIsolated;
use rise_engine::CancellationError;
use rise_i18n::tr;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::map::engine::core::rise_map_rpc::RiseMapRPC;
use crate::modules::map::engine::rise_map_engine_models::RiseMapError;
use crate::modules::map::stores::map_actions::map_photos_types::MapPhotoGrid;
use crate::modules::map::stores::map_actions::map_photos_wire_types::ProtoMapPhotoGridExt;
use crate::modules::map::stores::map_actions::map_types::{MapFriend, MapUser};
use crate::modules::map::stores::map_actions::map_wire_types::{MapNonEmpty, ProtoMapFriendExt, ProtoMapUserExt};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapProfile {
    pub user: MapUser,
    pub friends_count: i64,
    pub is_friend: bool,
    pub outgoing_request_id: Option<String>,
    pub can_view_tracks: bool,
    pub can_view_media: bool,
    pub can_view_friends: bool,
}

pub trait ProtoProfileResponseExt {
    fn model(&self) -> Result<MapProfile, RiseMapError>;
}

impl ProtoProfileResponseExt for ProtoProfileResponse {
    fn model(&self) -> Result<MapProfile, RiseMapError> {
        let Some(profile) = &self.profile else {
            return Err(RiseMapError::InvalidResponse);
        };
        let Some(user) = profile.user.as_ref().and_then(|user| user.model()) else {
            return Err(RiseMapError::InvalidResponse);
        };
        Ok(MapProfile {
            user,
            friends_count: profile.friends_count.max(0),
            is_friend: profile.is_friend,
            outgoing_request_id: profile.outgoing_request_id.map_non_empty(),
            can_view_tracks: profile.can_view_tracks,
            can_view_media: profile.can_view_media,
            can_view_friends: profile.can_view_friends,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MapProfileTab {
    Statistics,
    Media,
    Friends,
}

impl MapProfileTab {
    pub const ALL_CASES: [MapProfileTab; 3] = [MapProfileTab::Statistics, MapProfileTab::Media, MapProfileTab::Friends];

    pub const fn raw_value(self) -> &'static str {
        match self {
            MapProfileTab::Statistics => "statistics",
            MapProfileTab::Media => "media",
            MapProfileTab::Friends => "friends",
        }
    }

    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|tab| tab.raw_value() == raw_value)
    }

    pub const fn id(self) -> &'static str {
        self.raw_value()
    }

    pub fn title(self) -> String {
        match self {
            MapProfileTab::Statistics => tr("chat_settings_statistics"),
            MapProfileTab::Media => tr("map_media_library"),
            MapProfileTab::Friends => tr("map_friends_section"),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct MapProfileSnapshot {
    pub profile: Option<MapProfile>,
    pub grids: Vec<MapPhotoGrid>,
    pub friends: Vec<MapFriend>,
    pub has_more_grids: bool,
    pub has_more_friends: bool,
    pub loaded_tabs: HashSet<MapProfileTab>,
    pub failed: bool,
    pub page_revision: u64,
}

impl MapProfileSnapshot {
    fn failed() -> Self {
        Self { failed: true, ..Self::default() }
    }
}

struct RiseMapProfilesRepositoryState {
    owner_id: Option<String>,
    snapshot: MapProfileSnapshot,
    grid_cursor: Option<String>,
    friend_cursor: Option<String>,
    generation: u64,
    is_shutdown: bool,
    loading: HashSet<MapProfileTab>,
}

impl RiseMapProfilesRepositoryState {
    fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.owner_id = None;
        self.snapshot = MapProfileSnapshot::default();
        self.grid_cursor = None;
        self.friend_cursor = None;
        self.loading.clear();
    }

    fn update_grid(&mut self, grid: MapPhotoGrid) {
        if self.is_shutdown
            || self.owner_id.as_deref() != Some(grid.owner_id.as_str())
            || !self.snapshot.loaded_tabs.contains(&MapProfileTab::Media)
        {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.loading.clear();
        if let Some(index) = self.snapshot.grids.iter().position(|candidate| candidate.id == grid.id) {
            self.snapshot.grids[index] = grid;
        } else if grid.created_at >= self.snapshot.grids.first().map_or(0, |first| first.created_at) {
            self.snapshot.grids.insert(0, grid);
        }
        self.snapshot.page_revision = self.snapshot.page_revision.wrapping_add(1);
    }
}

static PAGE_SIZE: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_default_list_limit"));

struct RiseMapProfilesLoadingReset<'a> {
    state: &'a RiseIsolated<RiseMapProfilesRepositoryState>,
    expected: u64,
    tab: MapProfileTab,
}

impl Drop for RiseMapProfilesLoadingReset<'_> {
    fn drop(&mut self) {
        let expected = self.expected;
        let tab = self.tab;
        self.state.with(|s| {
            if expected == s.generation {
                s.loading.remove(&tab);
            }
        });
    }
}

/// one active profile kept only in account memory across sheets and temporary navigation
pub struct RiseMapProfilesRepository {
    rpc: RiseMapRPC,
    state: RiseIsolated<RiseMapProfilesRepositoryState>,
}

impl RiseMapProfilesRepository {
    fn page_size() -> i64 {
        *PAGE_SIZE
    }

    pub fn new(rpc: RiseMapRPC) -> Self {
        Self {
            rpc,
            state: RiseIsolated::new(RiseMapProfilesRepositoryState {
                owner_id: None,
                snapshot: MapProfileSnapshot::default(),
                grid_cursor: None,
                friend_cursor: None,
                generation: 0,
                is_shutdown: false,
                loading: HashSet::new(),
            }),
        }
    }

    pub async fn peer(&self, id: &str) -> anyhow::Result<MapProfile> {
        if self.state.with(|s| s.is_shutdown) {
            return Err(CancellationError.into());
        }
        let result = self.rpc.get_profile(id).await?;
        if self.state.with(|s| s.is_shutdown) {
            return Err(CancellationError.into());
        }
        Ok(result)
    }

    pub async fn open(&self, id: &str) -> MapProfileSnapshot {
        let expected = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            if s.owner_id.as_deref() != Some(id) {
                s.clear();
                s.owner_id = Some(id.to_owned());
            }
            Some(s.generation)
        });
        let Some(expected) = expected else {
            return MapProfileSnapshot::failed();
        };
        let result = self.peer(id).await;
        self.state.with(|s| {
            match result {
                Ok(profile) => {
                    if expected != s.generation || s.is_shutdown {
                        return MapProfileSnapshot::failed();
                    }
                    let can_view_media = profile.can_view_media;
                    let can_view_friends = profile.can_view_friends;
                    s.snapshot.profile = Some(profile);
                    s.snapshot.failed = false;
                    if !can_view_media {
                        s.snapshot.grids.clear();
                        s.snapshot.has_more_grids = false;
                        s.grid_cursor = None;
                        s.snapshot.loaded_tabs.remove(&MapProfileTab::Media);
                    }
                    if !can_view_friends {
                        s.snapshot.friends.clear();
                        s.snapshot.has_more_friends = false;
                        s.friend_cursor = None;
                        s.snapshot.loaded_tabs.remove(&MapProfileTab::Friends);
                    }
                }
                Err(_) => {
                    if expected == s.generation {
                        s.snapshot = MapProfileSnapshot::failed();
                    }
                }
            }
            s.snapshot.clone()
        })
    }

    pub async fn page(&self, id: &str, tab: MapProfileTab, more: bool) -> MapProfileSnapshot {
        let started = self.state.with(|s| {
            let Some(profile) = s.snapshot.profile.clone() else {
                return Err(s.snapshot.clone());
            };
            if s.is_shutdown || s.owner_id.as_deref() != Some(id) || s.loading.contains(&tab) {
                return Err(s.snapshot.clone());
            }
            if s.snapshot.loaded_tabs.contains(&tab) && !more {
                return Err(s.snapshot.clone());
            }
            s.loading.insert(tab);
            Ok((profile, s.generation))
        });
        let (profile, expected) = match started {
            Ok(started) => started,
            Err(snapshot) => return snapshot,
        };
        let _reset = RiseMapProfilesLoadingReset { state: &self.state, expected, tab };
        let outcome: anyhow::Result<Option<MapProfileSnapshot>> = async {
            match tab {
                MapProfileTab::Statistics => {}
                MapProfileTab::Media => {
                    let (has_more, cursor) =
                        self.state.with(|s| (s.snapshot.has_more_grids, if more { s.grid_cursor.clone() } else { None }));
                    if !profile.can_view_media || (more && !has_more) {
                        return Ok(Some(self.state.with(|s| s.snapshot.clone())));
                    }
                    let page = self.rpc.get_photo_grids(Some(id), Self::page_size(), cursor.as_deref()).await?;
                    let finished = self.state.with(|s| {
                        if expected != s.generation || s.is_shutdown {
                            return Some(s.snapshot.clone());
                        }
                        let known: HashSet<String> = s.snapshot.grids.iter().map(|grid| grid.id.clone()).collect();
                        let fresh: Vec<MapPhotoGrid> = page
                            .grids
                            .iter()
                            .filter_map(|grid| grid.model())
                            .filter(|grid| !known.contains(&grid.id))
                            .collect();
                        s.snapshot.grids.extend(fresh);
                        s.snapshot.has_more_grids = page.has_more
                            && page.next_cursor.is_some()
                            && page.next_cursor != s.grid_cursor;
                        s.grid_cursor = page.next_cursor.clone();
                        None
                    });
                    if finished.is_some() {
                        return Ok(finished);
                    }
                }
                MapProfileTab::Friends => {
                    let (has_more, cursor) = self
                        .state
                        .with(|s| (s.snapshot.has_more_friends, if more { s.friend_cursor.clone() } else { None }));
                    if !profile.can_view_friends || (more && !has_more) {
                        return Ok(Some(self.state.with(|s| s.snapshot.clone())));
                    }
                    let page = self.rpc.get_profile_friends(id, Self::page_size(), cursor.as_deref()).await?;
                    let finished = self.state.with(|s| {
                        if expected != s.generation || s.is_shutdown {
                            return Some(s.snapshot.clone());
                        }
                        let maximum_friends = usize::try_from(RiseLimits::service_value("map_max_friends")).unwrap_or(0);
                        let known: HashSet<String> =
                            s.snapshot.friends.iter().map(|friend| friend.id().to_owned()).collect();
                        let room = maximum_friends.saturating_sub(s.snapshot.friends.len());
                        let fresh: Vec<MapFriend> = page
                            .friends
                            .iter()
                            .filter_map(|friend| friend.model())
                            .filter(|friend| !known.contains(friend.id()))
                            .take(room)
                            .collect();
                        s.snapshot.friends.extend(fresh);
                        s.snapshot.has_more_friends = page.has_more
                            && page.next_cursor.is_some()
                            && page.next_cursor != s.friend_cursor
                            && s.snapshot.friends.len() < maximum_friends;
                        s.friend_cursor = page.next_cursor.clone();
                        None
                    });
                    if finished.is_some() {
                        return Ok(finished);
                    }
                }
            }
            Ok(None)
        }
        .await;
        self.state.with(|s| {
            match outcome {
                Ok(Some(snapshot)) => return snapshot,
                Ok(None) => {
                    s.snapshot.page_revision = s.snapshot.page_revision.wrapping_add(1);
                    s.snapshot.loaded_tabs.insert(tab);
                    s.snapshot.failed = false;
                }
                Err(_) => {
                    if expected == s.generation && more {
                        s.snapshot.failed = true;
                    } else if expected == s.generation {
                        s.snapshot.failed = true;
                        s.snapshot.loaded_tabs.remove(&tab);
                        if tab == MapProfileTab::Media {
                            s.snapshot.grids.clear();
                            s.snapshot.has_more_grids = false;
                            s.grid_cursor = None;
                        }
                        if tab == MapProfileTab::Friends {
                            s.snapshot.friends.clear();
                            s.snapshot.has_more_friends = false;
                            s.friend_cursor = None;
                        }
                    }
                }
            }
            s.snapshot.clone()
        })
    }

    pub async fn clear(&self) {
        self.state.with(RiseMapProfilesRepositoryState::clear);
    }

    /// mutations keep loaded pages and scroll positions
    /// pages in flight from before a removal must not put the deleted row back
    pub async fn remove_grid(&self, id: &str) {
        self.state.with(|s| {
            if s.is_shutdown {
                return;
            }
            s.generation = s.generation.wrapping_add(1);
            s.loading.clear();
            s.snapshot.grids.retain(|grid| grid.id != id);
            s.snapshot.page_revision = s.snapshot.page_revision.wrapping_add(1);
        });
    }

    pub async fn remove_friend(&self, id: &str, owner: &str) {
        self.state.with(|s| {
            if s.is_shutdown || s.owner_id.as_deref() != Some(owner) {
                return;
            }
            s.generation = s.generation.wrapping_add(1);
            s.loading.clear();
            s.snapshot.friends.retain(|friend| friend.id() != id);
            s.snapshot.page_revision = s.snapshot.page_revision.wrapping_add(1);
        });
    }

    pub async fn update_grid(&self, grid: MapPhotoGrid) {
        self.state.with(|s| s.update_grid(grid));
    }

    pub async fn synchronize_grids(&self, grids: Vec<MapPhotoGrid>) {
        let mut grids = grids;
        grids.sort_by_key(|grid| grid.created_at);
        self.state.with(|s| {
            for grid in grids {
                s.update_grid(grid);
            }
        });
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            s.clear();
        });
    }
}
