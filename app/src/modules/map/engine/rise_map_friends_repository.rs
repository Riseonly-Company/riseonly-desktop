use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use gpui::{App, AsyncApp};
use rise_canon::{RiseLimits, RiseTimingCanon, rise_response};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use serde::{Deserialize, Serialize};
use tokio::task::AbortHandle;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::map::engine::core::rise_map_resource_repository::{
    RiseMapFailure, RiseMapResourceRepository, RiseMapResourceRepositoryResource,
};
use crate::modules::map::engine::core::rise_map_rpc::RiseMapRPC;
use crate::modules::map::engine::rise_map_engine_models::{
    RiseMapCacheSource, RiseMapFriendRequestResult, RiseMapFriendsSnapshot, RiseMapInvitePreviewResult,
    RiseMapLoadState, RiseMapMutationOutcome,
};
use crate::modules::map::engine::rise_map_presentation::{RiseMapPresentation, RiseMapSessionIdentity};
use crate::modules::map::stores::map_actions::map_types::{
    MapFriend, MapFriendRequest, MapFriendRequestOutcome, MapInvite, MapRequestDirection, MapRequestStatus,
};
use crate::modules::map::stores::map_actions::map_wire_types::{ProtoMapFriendExt, ProtoMapFriendRequestExt};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RiseMapFriendsRepositoryList {
    Friends,
    Incoming,
    Outgoing,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseMapFriendsRepositoryPersisted {
    friends: Vec<MapFriend>,
    incoming: Vec<MapFriendRequest>,
    outgoing: Vec<MapFriendRequest>,
    has_more_friends: bool,
    has_more_incoming: bool,
    has_more_outgoing: bool,
    invite: Option<MapInvite>,
}

#[derive(Clone, Debug)]
struct RiseMapFriendsRepositoryPage<Item> {
    items: Vec<Item>,
    cursor: Option<String>,
    has_more: bool,
    is_loading_more: bool,
}

impl<Item> Default for RiseMapFriendsRepositoryPage<Item> {
    fn default() -> Self {
        Self { items: Vec::new(), cursor: None, has_more: false, is_loading_more: false }
    }
}

impl<Item> RiseMapFriendsRepositoryPage<Item> {
    fn new(items: Vec<Item>, cursor: Option<String>, has_more: bool) -> Self {
        Self { items, cursor, has_more, is_loading_more: false }
    }
}

/// the running head refresh
/// done resolves when it ends without cancelling it
/// abort cancels it
struct RiseMapFriendsRefreshTask {
    token: u64,
    abort: AbortHandle,
    done: Shared<BoxFuture<'static, ()>>,
}

struct RiseMapFriendsRepositoryState {
    friends: RiseMapFriendsRepositoryPage<MapFriend>,
    incoming: RiseMapFriendsRepositoryPage<MapFriendRequest>,
    outgoing: RiseMapFriendsRepositoryPage<MapFriendRequest>,
    invite: Option<MapInvite>,
    load_state: RiseMapLoadState,
    error_text: Option<String>,
    cache_source: RiseMapCacheSource,
    revision: u64,
    did_read_store: bool,
    last_refresh_ms: i64,
    refresh_task: Option<RiseMapFriendsRefreshTask>,
    refresh_token: u64,
    head_generation: u64,
    is_shutdown: bool,
}

static REFRESH_INTERVAL_MS: LazyLock<i64> = LazyLock::new(|| RiseTimingCanon::freshness_ms("map_friends"));
static PAGE_SIZE: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_default_list_limit"));
static MAXIMUM_FRIENDS: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::service_value("map_max_friends")).unwrap_or(0));
static MAXIMUM_REQUESTS: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::client_value("map_request_list_memory_items")).unwrap_or(0));

/// the map's friend graph
/// friends and requests both ways and the personal invite link
/// bounded lists and cache first and optimistic removals
pub struct RiseMapFriendsRepository {
    rpc: RiseMapRPC,
    resources: Arc<RiseMapResourceRepository>,
    identity: RiseMapSessionIdentity,
    state: RiseIsolated<RiseMapFriendsRepositoryState>,
}

/// rollback for an optimistic step
/// a dropped call puts the state back and posts the snapshot since drop cant await
struct RiseMapFriendsRollback<'a> {
    repository: &'a RiseMapFriendsRepository,
    rollback: Option<Box<dyn FnOnce(&mut RiseMapFriendsRepositoryState) + Send + 'a>>,
}

impl<'a> RiseMapFriendsRollback<'a> {
    fn new(
        repository: &'a RiseMapFriendsRepository,
        rollback: impl FnOnce(&mut RiseMapFriendsRepositoryState) + Send + 'a,
    ) -> Self {
        Self { repository, rollback: Some(Box::new(rollback)) }
    }

    fn take(&mut self) -> Option<Box<dyn FnOnce(&mut RiseMapFriendsRepositoryState) + Send + 'a>> {
        self.rollback.take()
    }
}

impl Drop for RiseMapFriendsRollback<'_> {
    fn drop(&mut self) {
        let Some(rollback) = self.rollback.take() else {
            return;
        };
        let snapshot = self.repository.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            rollback(s);
            Some(s.next_snapshot())
        });
        if let Some(snapshot) = snapshot {
            RiseMapFriendsRepository::post_snapshot(snapshot, self.repository.identity.clone());
        }
    }
}

impl RiseMapFriendsRepository {
    pub fn new(
        rpc: RiseMapRPC,
        resources: Arc<RiseMapResourceRepository>,
        identity: RiseMapSessionIdentity,
    ) -> Self {
        Self {
            rpc,
            resources,
            identity,
            state: RiseIsolated::new(RiseMapFriendsRepositoryState {
                friends: RiseMapFriendsRepositoryPage::default(),
                incoming: RiseMapFriendsRepositoryPage::default(),
                outgoing: RiseMapFriendsRepositoryPage::default(),
                invite: None,
                load_state: RiseMapLoadState::Idle,
                error_text: None,
                cache_source: RiseMapCacheSource::Miss,
                revision: 0,
                did_read_store: false,
                last_refresh_ms: 0,
                refresh_task: None,
                refresh_token: 0,
                head_generation: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn open(self: &Arc<Self>, force: bool) {
        if !self.hydrate_from_store().await {
            return;
        }

        let now = Self::now_milliseconds();
        let proceeds = self.state.with(|s| {
            let is_stale = now - s.last_refresh_ms >= *REFRESH_INTERVAL_MS;
            force || is_stale || s.load_state != RiseMapLoadState::Loaded
        });
        if !proceeds {
            return;
        }
        self.refresh().await;
    }

    /// saved lists before any network
    /// called ahead of the map state refresh so an offline open shows stored friends at once
    /// once per session and false after shutdown
    pub async fn hydrate_from_store(self: &Arc<Self>) -> bool {
        let reads_store = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            if s.did_read_store {
                return Some(false);
            }
            s.did_read_store = true;
            Some(s.is_empty())
        });
        let Some(reads_store) = reads_store else {
            return false;
        };
        if reads_store
            && let Some(cached) = self
                .resources
                .cached::<RiseMapFriendsRepositoryPersisted>(RiseMapResourceRepositoryResource::Friends)
                .await
        {
            let applied = self.state.with(|s| {
                if s.is_shutdown {
                    return None;
                }
                if !s.is_empty() {
                    return Some(false);
                }
                s.friends = RiseMapFriendsRepositoryPage::new(cached.friends, None, cached.has_more_friends);
                s.incoming = RiseMapFriendsRepositoryPage::new(cached.incoming, None, cached.has_more_incoming);
                s.outgoing = RiseMapFriendsRepositoryPage::new(cached.outgoing, None, cached.has_more_outgoing);
                s.invite = cached.invite;
                s.cache_source = RiseMapCacheSource::PersistentStore;
                s.load_state = RiseMapLoadState::Loaded;
                Some(true)
            });
            match applied {
                None => return false,
                Some(true) => self.publish().await,
                Some(false) => {}
            }
        }
        true
    }

    pub async fn refresh(self: &Arc<Self>) {
        let this = self.clone();
        let step = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            if let Some(refresh_task) = s.refresh_task.as_ref() {
                return Some((None, refresh_task.done.clone()));
            }
            s.refresh_token = s.refresh_token.wrapping_add(1);
            let token = s.refresh_token;
            let handle = RiseAccountRuntime::current().spawn(async move {
                this.perform_refresh().await;
            });
            let abort = handle.abort_handle();
            let done: Shared<BoxFuture<'static, ()>> = async move {
                let _ = handle.await;
            }
            .boxed()
            .shared();
            s.refresh_task = Some(RiseMapFriendsRefreshTask { token, abort, done: done.clone() });
            Some((Some(token), done))
        });
        let Some((started, done)) = step else {
            return;
        };
        done.await;
        if let Some(token) = started {
            self.state.with(|s| {
                if s.refresh_task.as_ref().is_some_and(|task| task.token == token) {
                    s.refresh_task = None;
                }
            });
        }
    }

    pub async fn load_more(&self, list: RiseMapFriendsRepositoryList) {
        if self.state.with(|s| s.is_shutdown || s.load_state != RiseMapLoadState::Loaded) {
            return;
        }
        match list {
            RiseMapFriendsRepositoryList::Friends => {
                let start = self.state.with(|s| {
                    if !s.friends.has_more || s.friends.is_loading_more || s.friends.items.len() >= *MAXIMUM_FRIENDS {
                        return None;
                    }
                    let cursor = s.friends.cursor.clone()?;
                    s.friends.is_loading_more = true;
                    Some((cursor, s.head_generation))
                });
                let Some((cursor, generation)) = start else {
                    return;
                };
                let mut loading = RiseMapFriendsRollback::new(self, |s| s.friends.is_loading_more = false);
                self.publish().await;
                let page = self.rpc.get_friends(*PAGE_SIZE, Some(cursor.as_str())).await.ok();
                loading.take();
                let applied = self.state.with(|s| {
                    if s.is_shutdown {
                        return false;
                    }
                    s.friends.is_loading_more = false;
                    if generation == s.head_generation
                        && let Some(page) = page
                    {
                        let fresh: Vec<MapFriend> =
                            page.friends.iter().filter_map(ProtoMapFriendExt::model).collect();
                        s.friends.items = RiseMapFriendsRepositoryState::merged(&s.friends.items, fresh);
                        s.friends.has_more = page.has_more && page.next_cursor.is_some();
                        s.friends.cursor = page.next_cursor;
                    }
                    true
                });
                if !applied {
                    return;
                }
            }
            RiseMapFriendsRepositoryList::Incoming => self.load_more_requests(MapRequestDirection::Incoming).await,
            RiseMapFriendsRepositoryList::Outgoing => self.load_more_requests(MapRequestDirection::Outgoing).await,
        }
        self.publish().await;
    }

    pub async fn send_request(&self, target_user_id: &str) -> RiseMapFriendRequestResult {
        if self.is_shutdown() {
            return RiseMapFriendRequestResult::Failed(None);
        }
        match self.rpc.send_friend_request(target_user_id).await {
            Ok(outcome) => {
                if self.is_shutdown() {
                    return RiseMapFriendRequestResult::Failed(None);
                }
                self.absorb(&outcome).await;
                RiseMapFriendRequestResult::Sent(outcome)
            }
            Err(error) => RiseMapFriendRequestResult::Failed(RiseMapFailure::text(&error)),
        }
    }

    pub async fn use_invite(&self, token: &str) -> RiseMapFriendRequestResult {
        if self.is_shutdown() {
            return RiseMapFriendRequestResult::Failed(None);
        }
        match self.rpc.use_invite(token).await {
            Ok(outcome) => {
                if self.is_shutdown() {
                    return RiseMapFriendRequestResult::Failed(None);
                }
                self.absorb(&outcome).await;
                RiseMapFriendRequestResult::Sent(outcome)
            }
            Err(error) => RiseMapFriendRequestResult::Failed(RiseMapFailure::text(&error)),
        }
    }

    pub async fn preview_invite(&self, token: &str) -> RiseMapInvitePreviewResult {
        if self.is_shutdown() {
            return RiseMapInvitePreviewResult::Failed(None);
        }
        match self.rpc.preview_invite(token).await {
            Ok(preview) => RiseMapInvitePreviewResult::Loaded(preview),
            Err(error) => RiseMapInvitePreviewResult::Failed(RiseMapFailure::text(&error)),
        }
    }

    pub async fn respond(&self, request_id: &str, accept: bool) -> RiseMapMutationOutcome {
        let removed = self.state.with(|s| {
            if s.is_shutdown {
                return Err(());
            }
            Ok(s.remove(request_id, MapRequestDirection::Incoming))
        });
        let Ok(removed) = removed else {
            return RiseMapMutationOutcome::Failed(None);
        };
        let had_removed = removed.is_some();
        let mut rollback = RiseMapFriendsRollback::new(self, move |s| {
            if let Some(removed) = removed {
                s.restore(removed, MapRequestDirection::Incoming);
            }
        });
        if had_removed {
            self.publish().await;
        }

        match self.rpc.respond_friend_request(request_id, accept).await {
            Ok(outcome) => {
                rollback.take();
                if self.is_shutdown() {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.absorb(&outcome).await;
                if outcome.friend.is_none() {
                    self.persist().await;
                }
                RiseMapMutationOutcome::Done
            }
            Err(error) => {
                let restore = rollback.take();
                if !self.apply_rollback(restore) {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.publish().await;
                RiseMapMutationOutcome::Failed(RiseMapFailure::text(&error))
            }
        }
    }

    pub async fn cancel(&self, request_id: &str) -> RiseMapMutationOutcome {
        let removed = self.state.with(|s| {
            if s.is_shutdown {
                return Err(());
            }
            Ok(s.remove(request_id, MapRequestDirection::Outgoing))
        });
        let Ok(removed) = removed else {
            return RiseMapMutationOutcome::Failed(None);
        };
        let had_removed = removed.is_some();
        let mut rollback = RiseMapFriendsRollback::new(self, move |s| {
            if let Some(removed) = removed {
                s.restore(removed, MapRequestDirection::Outgoing);
            }
        });
        if had_removed {
            self.publish().await;
        }

        match self.rpc.cancel_friend_request(request_id).await {
            Ok(()) => {
                rollback.take();
                if self.is_shutdown() {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.persist().await;
                RiseMapMutationOutcome::Done
            }
            Err(error) => {
                let restore = rollback.take();
                if !self.apply_rollback(restore) {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.publish().await;
                RiseMapMutationOutcome::Failed(RiseMapFailure::text(&error))
            }
        }
    }

    pub async fn remove_friend(&self, user_id: &str) -> RiseMapMutationOutcome {
        let removed = self.state.with(|s| {
            if s.is_shutdown {
                return Err(());
            }
            let index = s.friends.items.iter().position(|friend| friend.id() == user_id);
            Ok(index.map(|index| (s.friends.items.remove(index), index)))
        });
        let Ok(removed) = removed else {
            return RiseMapMutationOutcome::Failed(None);
        };
        let had_removed = removed.is_some();
        let owned_user_id = user_id.to_owned();
        let mut rollback = RiseMapFriendsRollback::new(self, move |s| {
            if let Some((removed, index)) = removed
                && !s.friends.items.iter().any(|friend| friend.id() == owned_user_id)
            {
                let at = index.min(s.friends.items.len());
                s.friends.items.insert(at, removed);
            }
        });
        if had_removed {
            self.publish().await;
        }

        match self.rpc.remove_friend(user_id).await {
            Ok(()) => {
                rollback.take();
                if self.is_shutdown() {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.persist().await;
                RiseMapMutationOutcome::Done
            }
            Err(error) => {
                let restore = rollback.take();
                if !self.apply_rollback(restore) {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.publish().await;
                RiseMapMutationOutcome::Failed(RiseMapFailure::text(&error))
            }
        }
    }

    pub async fn load_invite(&self, force: bool) -> Option<MapInvite> {
        let current = self.state.with(|s| if s.is_shutdown { Err(()) } else { Ok(s.invite.clone()) });
        let Ok(current) = current else {
            return None;
        };
        if let Some(invite) = current
            && !force
        {
            return Some(invite);
        }
        let fresh = self.rpc.get_invite().await.ok();
        let fresh = self.state.with(|s| match fresh {
            Some(fresh) if !s.is_shutdown => Ok(fresh),
            _ => Err(s.invite.clone()),
        });
        match fresh {
            Ok(fresh) => {
                self.set_invite(fresh.clone()).await;
                Some(fresh)
            }
            Err(invite) => invite,
        }
    }

    pub async fn reset_invite(&self) -> RiseMapMutationOutcome {
        if self.is_shutdown() {
            return RiseMapMutationOutcome::Failed(None);
        }
        match self.rpc.reset_invite().await {
            Ok(fresh) => {
                if self.is_shutdown() {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.set_invite(fresh).await;
                RiseMapMutationOutcome::Done
            }
            Err(error) => RiseMapMutationOutcome::Failed(RiseMapFailure::text(&error)),
        }
    }

    pub async fn apply_remote_request(&self, request: MapFriendRequest) {
        let account_id = self.identity.account_id.as_str();
        let changed = self.state.with(|s| {
            if s.is_shutdown || request.status != MapRequestStatus::Pending {
                return false;
            }
            if request.to_user.id == account_id {
                s.insert_request(request, MapRequestDirection::Incoming)
            } else if request.from_user.id == account_id {
                s.insert_request(request, MapRequestDirection::Outgoing)
            } else {
                false
            }
        });
        if !changed {
            return;
        }
        self.persist().await;
        self.publish().await;
    }

    pub async fn apply_remote_settled(&self, request_id: &str) {
        let changed = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            let from_incoming = s.remove(request_id, MapRequestDirection::Incoming).is_some();
            let from_outgoing = s.remove(request_id, MapRequestDirection::Outgoing).is_some();
            from_incoming || from_outgoing
        });
        if !changed {
            return;
        }
        self.persist().await;
        self.publish().await;
    }

    pub async fn apply_remote_accepted(&self, friend: MapFriend) {
        let changed = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            let friend_id = friend.id().to_owned();
            let inserted = s.insert(friend);
            let settled = s.drop_requests(&friend_id);
            inserted || settled
        });
        if !changed {
            return;
        }
        self.persist().await;
        self.publish().await;
    }

    pub async fn apply_remote_removed(&self, user_id: &str) {
        let changed = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            let Some(index) = s.friends.items.iter().position(|friend| friend.id() == user_id) else {
                return false;
            };
            s.friends.items.remove(index);
            true
        });
        if !changed {
            return;
        }
        self.persist().await;
        self.publish().await;
    }

    /// leaving the map empties the graph on this device, the server keeps it for a return
    pub async fn reset(&self) {
        let reset = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.head_generation = s.head_generation.wrapping_add(1);
            s.friends = RiseMapFriendsRepositoryPage::default();
            s.incoming = RiseMapFriendsRepositoryPage::default();
            s.outgoing = RiseMapFriendsRepositoryPage::default();
            s.invite = None;
            s.load_state = RiseMapLoadState::Idle;
            s.error_text = None;
            s.cache_source = RiseMapCacheSource::Miss;
            s.last_refresh_ms = 0;
            true
        });
        if reset {
            self.publish().await;
        }
    }

    pub fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            if let Some(refresh_task) = s.refresh_task.take() {
                refresh_task.abort.abort();
            }
        });
    }

    fn is_shutdown(&self) -> bool {
        self.state.with(|s| s.is_shutdown)
    }

    /// undoes an optimistic step unless the repository shut down meanwhile
    /// false means it did and the caller returns at once
    fn apply_rollback(
        &self,
        rollback: Option<Box<dyn FnOnce(&mut RiseMapFriendsRepositoryState) + Send + '_>>,
    ) -> bool {
        self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            if let Some(rollback) = rollback {
                rollback(s);
            }
            true
        })
    }

    async fn perform_refresh(&self) {
        let announces = self.state.with(|s| {
            if s.is_empty() && s.load_state != RiseMapLoadState::Loaded {
                s.load_state = RiseMapLoadState::Loading;
                return true;
            }
            false
        });
        if announces {
            self.publish().await;
        }

        let fetched = futures::try_join!(
            self.rpc.get_friends(*PAGE_SIZE, None),
            self.rpc.get_friend_requests(MapRequestDirection::Incoming, *PAGE_SIZE, None),
            self.rpc.get_friend_requests(MapRequestDirection::Outgoing, *PAGE_SIZE, None),
        );
        match fetched {
            Ok((fresh_friends, fresh_incoming, fresh_outgoing)) => {
                let now = Self::now_milliseconds();
                let applied = self.state.with(|s| {
                    if s.is_shutdown {
                        return false;
                    }
                    s.head_generation = s.head_generation.wrapping_add(1);
                    let friends: Vec<MapFriend> =
                        fresh_friends.friends.iter().filter_map(ProtoMapFriendExt::model).collect();
                    s.friends = RiseMapFriendsRepositoryPage::new(
                        RiseMapFriendsRepositoryState::merged(&[], friends),
                        fresh_friends.next_cursor.clone(),
                        fresh_friends.has_more && fresh_friends.next_cursor.is_some(),
                    );
                    s.incoming = RiseMapFriendsRepositoryState::request_page(&fresh_incoming);
                    s.outgoing = RiseMapFriendsRepositoryState::request_page(&fresh_outgoing);
                    s.last_refresh_ms = now;
                    s.load_state = RiseMapLoadState::Loaded;
                    s.error_text = None;
                    s.cache_source = RiseMapCacheSource::Network;
                    true
                });
                if !applied {
                    return;
                }
                self.persist().await;
                self.publish().await;
            }
            Err(error) => {
                let error_text = RiseMapFailure::text(&error);
                let failed = self.state.with(|s| {
                    if s.is_shutdown || s.load_state == RiseMapLoadState::Loaded {
                        return false;
                    }
                    s.load_state = RiseMapLoadState::Failed;
                    s.error_text = error_text;
                    true
                });
                if failed {
                    self.publish().await;
                }
            }
        }
    }

    async fn load_more_requests(&self, direction: MapRequestDirection) {
        let start = self.state.with(|s| {
            let current = s.requests(direction);
            if !current.has_more || current.is_loading_more || current.items.len() >= *MAXIMUM_REQUESTS {
                return None;
            }
            let cursor = current.cursor.clone()?;
            s.set_loading_more(true, direction);
            Some((cursor, s.head_generation))
        });
        let Some((cursor, generation)) = start else {
            return;
        };
        let mut loading = RiseMapFriendsRollback::new(self, move |s| s.set_loading_more(false, direction));
        self.publish().await;

        let fresh = self
            .rpc
            .get_friend_requests(direction, *PAGE_SIZE, Some(cursor.as_str()))
            .await
            .ok();
        loading.take();
        self.state.with(|s| {
            if s.is_shutdown {
                return;
            }
            s.set_loading_more(false, direction);
            let Some(fresh) = fresh.filter(|_| generation == s.head_generation) else {
                return;
            };
            let mut page = s.requests(direction).clone();
            let known: HashSet<String> = page.items.iter().map(|request| request.id.clone()).collect();
            let room = MAXIMUM_REQUESTS.saturating_sub(page.items.len());
            let added = fresh
                .requests
                .iter()
                .filter_map(ProtoMapFriendRequestExt::model)
                .filter(|request| request.status == MapRequestStatus::Pending && !known.contains(&request.id))
                .take(room);
            page.items.extend(added);
            page.has_more = fresh.has_more && fresh.next_cursor.is_some();
            page.cursor = fresh.next_cursor;
            s.set_requests(page, direction);
        });
    }

    async fn absorb(&self, outcome: &MapFriendRequestOutcome) {
        let account_id = self.identity.account_id.as_str();
        let changed = self.state.with(|s| {
            let mut changed = false;
            if let Some(friend) = outcome.friend.as_ref() {
                changed = s.insert(friend.clone()) || changed;
                changed = s.drop_requests(friend.id()) || changed;
            } else if outcome.request.status == MapRequestStatus::Pending {
                if outcome.request.from_user.id == account_id {
                    changed = s.insert_request(outcome.request.clone(), MapRequestDirection::Outgoing);
                } else if outcome.request.to_user.id == account_id {
                    changed = s.insert_request(outcome.request.clone(), MapRequestDirection::Incoming);
                }
            } else {
                changed = s.remove(&outcome.request.id, MapRequestDirection::Incoming).is_some() || changed;
                changed = s.remove(&outcome.request.id, MapRequestDirection::Outgoing).is_some() || changed;
            }
            changed
        });
        if !changed {
            return;
        }
        self.persist().await;
        self.publish().await;
    }

    async fn set_invite(&self, fresh: MapInvite) {
        let changed = self.state.with(|s| {
            if s.invite.as_ref() == Some(&fresh) {
                return false;
            }
            s.invite = Some(fresh);
            true
        });
        if !changed {
            return;
        }
        self.persist().await;
        self.publish().await;
    }

    async fn persist(&self) {
        let persisted = self.state.with(|s| {
            let head = usize::try_from(*PAGE_SIZE).unwrap_or(0);
            RiseMapFriendsRepositoryPersisted {
                friends: s.friends.items.iter().take(head).cloned().collect(),
                incoming: s.incoming.items.iter().take(head).cloned().collect(),
                outgoing: s.outgoing.items.iter().take(head).cloned().collect(),
                has_more_friends: s.friends.has_more || s.friends.items.len() > head,
                has_more_incoming: s.incoming.has_more || s.incoming.items.len() > head,
                has_more_outgoing: s.outgoing.has_more || s.outgoing.items.len() > head,
                invite: s.invite.clone(),
            }
        });
        self.resources.store(RiseMapResourceRepositoryResource::Friends, persisted).await;
    }

    async fn publish(&self) {
        let snapshot = self.state.with(RiseMapFriendsRepositoryState::next_snapshot);
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let identity = self.identity.clone();
        let _ = hop
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| Self::publish_on_main(snapshot, &identity, cx));
                    Ok::<_, anyhow::Error>(())
                }
                .boxed_local()
            })
            .await;
    }

    fn post_snapshot(snapshot: RiseMapFriendsSnapshot, identity: RiseMapSessionIdentity) {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        hop.post(move |cx: &mut App| Self::publish_on_main(snapshot, &identity, cx));
    }

    fn publish_on_main(snapshot: RiseMapFriendsSnapshot, identity: &RiseMapSessionIdentity, cx: &mut App) {
        RiseMapPresentation::shared(cx).update(cx, |presentation, cx| {
            presentation.publish_friends(snapshot, identity, cx);
        });
    }

    fn now_milliseconds() -> i64 {
        RiseClock::now().ms()
    }
}

impl RiseMapFriendsRepositoryState {
    fn is_empty(&self) -> bool {
        self.friends.items.is_empty() && self.incoming.items.is_empty() && self.outgoing.items.is_empty()
    }

    fn set_loading_more(&mut self, value: bool, direction: MapRequestDirection) {
        match direction {
            MapRequestDirection::Incoming => self.incoming.is_loading_more = value,
            MapRequestDirection::Outgoing => self.outgoing.is_loading_more = value,
        }
    }

    fn insert(&mut self, friend: MapFriend) -> bool {
        if self.friends.items.iter().any(|known| known.id() == friend.id()) {
            return false;
        }
        self.friends.items.insert(0, friend);
        if self.friends.items.len() > *MAXIMUM_FRIENDS {
            self.friends.items.truncate(*MAXIMUM_FRIENDS);
            self.friends.has_more = true;
        }
        true
    }

    fn insert_request(&mut self, request: MapFriendRequest, direction: MapRequestDirection) -> bool {
        let page = self.requests_mut(direction);
        if page.items.iter().any(|known| known.id == request.id) {
            return false;
        }
        page.items.insert(0, request);
        if page.items.len() > *MAXIMUM_REQUESTS {
            page.items.truncate(*MAXIMUM_REQUESTS);
            page.has_more = true;
        }
        true
    }

    fn remove(&mut self, request_id: &str, direction: MapRequestDirection) -> Option<(MapFriendRequest, usize)> {
        let page = self.requests_mut(direction);
        let index = page.items.iter().position(|request| request.id == request_id)?;
        let request = page.items.remove(index);
        Some((request, index))
    }

    fn restore(&mut self, removed: (MapFriendRequest, usize), direction: MapRequestDirection) {
        let (request, index) = removed;
        let page = self.requests_mut(direction);
        if page.items.iter().any(|known| known.id == request.id) {
            return;
        }
        let at = index.min(page.items.len());
        page.items.insert(at, request);
    }

    fn drop_requests(&mut self, user_id: &str) -> bool {
        let before = self.incoming.items.len() + self.outgoing.items.len();
        self.incoming.items.retain(|request| request.from_user.id != user_id);
        self.outgoing.items.retain(|request| request.to_user.id != user_id);
        self.incoming.items.len() + self.outgoing.items.len() != before
    }

    fn requests(&self, direction: MapRequestDirection) -> &RiseMapFriendsRepositoryPage<MapFriendRequest> {
        match direction {
            MapRequestDirection::Incoming => &self.incoming,
            MapRequestDirection::Outgoing => &self.outgoing,
        }
    }

    fn requests_mut(&mut self, direction: MapRequestDirection) -> &mut RiseMapFriendsRepositoryPage<MapFriendRequest> {
        match direction {
            MapRequestDirection::Incoming => &mut self.incoming,
            MapRequestDirection::Outgoing => &mut self.outgoing,
        }
    }

    fn set_requests(&mut self, page: RiseMapFriendsRepositoryPage<MapFriendRequest>, direction: MapRequestDirection) {
        *self.requests_mut(direction) = page;
    }

    fn next_snapshot(&mut self) -> RiseMapFriendsSnapshot {
        self.revision = self.revision.wrapping_add(1);
        RiseMapFriendsSnapshot {
            friends: self.friends.items.clone(),
            incoming: self.incoming.items.clone(),
            outgoing: self.outgoing.items.clone(),
            has_more_friends: self.friends.has_more,
            has_more_incoming: self.incoming.has_more,
            has_more_outgoing: self.outgoing.has_more,
            is_loading_more: self.friends.is_loading_more
                || self.incoming.is_loading_more
                || self.outgoing.is_loading_more,
            invite: self.invite.clone(),
            load_state: self.load_state,
            error_text: self.error_text.clone(),
            cache_source: self.cache_source,
            revision: self.revision,
        }
    }

    fn request_page(response: &rise_response::map::GetFriendRequests) -> RiseMapFriendsRepositoryPage<MapFriendRequest> {
        let mut seen = HashSet::new();
        let items: Vec<MapFriendRequest> = response
            .requests
            .iter()
            .filter_map(ProtoMapFriendRequestExt::model)
            .filter(|request| request.status == MapRequestStatus::Pending && seen.insert(request.id.clone()))
            .take(*MAXIMUM_REQUESTS)
            .collect();
        RiseMapFriendsRepositoryPage::new(
            items,
            response.next_cursor.clone(),
            response.has_more && response.next_cursor.is_some(),
        )
    }

    fn merged(existing: &[MapFriend], page: Vec<MapFriend>) -> Vec<MapFriend> {
        let mut seen: HashSet<String> = existing.iter().map(|friend| friend.id().to_owned()).collect();
        let mut result = existing.to_vec();
        for friend in page {
            if !seen.insert(friend.id().to_owned()) {
                continue;
            }
            if result.len() >= *MAXIMUM_FRIENDS {
                break;
            }
            result.push(friend);
        }
        result
    }
}
