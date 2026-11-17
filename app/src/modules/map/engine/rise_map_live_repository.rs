use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::{BoxFuture, Shared};
use gpui::{App, AsyncApp};
use rise_canon::RiseLimits;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use serde::{Deserialize, Serialize};
use tokio::task::AbortHandle;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::map::engine::core::rise_map_resource_repository::{
    RiseMapFailure, RiseMapResourceRepository, RiseMapResourceRepositoryResource,
};
use crate::modules::map::engine::core::rise_map_rpc::RiseMapRPC;
use crate::modules::map::engine::rise_map_engine_models::{
    RiseMapCacheSource, RiseMapLiveSnapshot, RiseMapReactionOutcome,
};
use crate::modules::map::engine::rise_map_presentation::{RiseMapPresentation, RiseMapSessionIdentity};
use crate::modules::map::engine::rise_map_reactions_repository::RiseMapReactionsRepository;
use crate::modules::map::stores::map_actions::map_types::{
    MapFriendPosition, MapLiveFriend, MapReaction,
};
use crate::modules::map::stores::map_actions::map_wire_types::ProtoLiveFriendExt;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseMapLiveRepositoryPersisted {
    friends: Vec<MapLiveFriend>,
}

struct RiseMapLiveRepositoryState {
    order: Vec<String>,
    by_id: HashMap<String, MapLiveFriend>,
    is_session_open: bool,
    cache_source: RiseMapCacheSource,
    revision: u64,
    is_watching: bool,
    watching_sequence: u64,
    did_read_store: bool,
    heartbeat_interval_ms: i64,
    session_ttl_ms: i64,
    last_beat_ms: i64,
    session_generation: u64,
    heartbeat_task: Option<AbortHandle>,
    session_command_tail: Option<Shared<BoxFuture<'static, ()>>>,
    is_publishing: bool,
    needs_publish: bool,
    is_shutdown: bool,
}

static MAXIMUM_FRIENDS: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::service_value("map_max_friends")).unwrap_or(0));
static FALLBACK_HEARTBEAT_MS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("map_heartbeat_interval_ms"));
static FALLBACK_SESSION_TTL_MS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("map_viewer_session_ttl_ms"));
static UNWATCHED_UPDATE_MS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("map_unwatched_update_interval_ms"));

/// live layer, who the viewer may see right now and where
/// server fans movement out only to viewers with a session
/// so the session lives while a screen shows the map
pub struct RiseMapLiveRepository {
    reactions: Arc<RiseMapReactionsRepository>,
    rpc: RiseMapRPC,
    resources: Arc<RiseMapResourceRepository>,
    identity: RiseMapSessionIdentity,
    state: RiseIsolated<RiseMapLiveRepositoryState>,
}

struct RiseMapLivePublishingReset<'a> {
    state: &'a RiseIsolated<RiseMapLiveRepositoryState>,
}

impl Drop for RiseMapLivePublishingReset<'_> {
    fn drop(&mut self) {
        self.state.with(|s| s.is_publishing = false);
    }
}

impl RiseMapLiveRepository {
    pub fn new(
        rpc: RiseMapRPC,
        resources: Arc<RiseMapResourceRepository>,
        identity: RiseMapSessionIdentity,
        reactions: Arc<RiseMapReactionsRepository>,
    ) -> Self {
        Self {
            reactions,
            rpc,
            resources,
            identity,
            state: RiseIsolated::new(RiseMapLiveRepositoryState {
                order: Vec::new(),
                by_id: HashMap::new(),
                is_session_open: false,
                cache_source: RiseMapCacheSource::Miss,
                revision: 0,
                is_watching: false,
                watching_sequence: 0,
                did_read_store: false,
                heartbeat_interval_ms: *FALLBACK_HEARTBEAT_MS,
                session_ttl_ms: *FALLBACK_SESSION_TTL_MS,
                last_beat_ms: 0,
                session_generation: 0,
                heartbeat_task: None,
                session_command_tail: None,
                is_publishing: false,
                needs_publish: false,
                is_shutdown: false,
            }),
        }
    }

    pub fn unwatched_update_ms() -> i64 {
        *UNWATCHED_UPDATE_MS
    }

    pub async fn set_watching(self: &Arc<Self>, watching: bool, sequence: u64) {
        let step = self.state.with(|s| {
            if s.is_shutdown || sequence <= s.watching_sequence {
                return None;
            }
            s.watching_sequence = sequence;
            if watching == s.is_watching {
                return None;
            }
            s.is_watching = watching;
            Some(watching)
        });
        match step {
            Some(true) => self.begin_watching().await,
            Some(false) => self.end_watching().await,
            None => {}
        }
    }

    async fn begin_watching(self: &Arc<Self>) {
        let reads_store = self.state.with(|s| {
            if s.did_read_store {
                return false;
            }
            s.did_read_store = true;
            s.order.is_empty()
        });
        if reads_store
            && let Some(cached) = self
                .resources
                .cached::<RiseMapLiveRepositoryPersisted>(RiseMapResourceRepositoryResource::Live)
                .await
        {
            let applied = self.state.with(|s| {
                if s.is_shutdown || !s.is_watching {
                    return None;
                }
                if !s.order.is_empty() {
                    return Some(false);
                }
                s.replace(cached.friends);
                s.cache_source = RiseMapCacheSource::PersistentStore;
                Some(true)
            });
            match applied {
                None => return,
                Some(true) => self.publish().await,
                Some(false) => {}
            }
        }
        self.open_session().await;
    }

    async fn end_watching(self: &Arc<Self>) {
        self.state.with(|s| {
            s.session_generation = s.session_generation.wrapping_add(1);
            if let Some(heartbeat_task) = s.heartbeat_task.take() {
                heartbeat_task.abort();
            }
            s.is_session_open = false;
            self.enqueue_session_close(s);
        });
        self.persist().await;
        self.publish().await;
    }

    fn enqueue_session_close(self: &Arc<Self>, s: &mut RiseMapLiveRepositoryState) {
        let previous = s.session_command_tail.take();
        let generation = s.session_generation;
        let rpc = self.rpc.clone();
        let weak = Arc::downgrade(self);
        let handle = RiseAccountRuntime::current().spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            let Some(this) = weak.upgrade() else {
                return;
            };
            if !this.should_close_session(generation) {
                return;
            }
            let _ = rpc.close_session().await;
        });
        s.session_command_tail = Some(
            async move {
                let _ = handle.await;
            }
            .boxed()
            .shared(),
        );
    }

    fn should_close_session(&self, generation: u64) -> bool {
        self.state.with(|s| !s.is_shutdown && !s.is_watching && generation == s.session_generation)
    }

    fn should_open_session(&self, generation: u64) -> bool {
        self.state.with(|s| s.owns_session(generation))
    }

    /// reconnect and foreground return
    /// session may have lapsed and a heartbeat after a lapse reports everyone added and nobody removed
    /// so the whole picture is taken again
    pub async fn resume(self: &Arc<Self>) {
        if self.state.with(|s| s.is_shutdown || !s.is_watching) {
            return;
        }
        self.open_session().await;
    }

    pub async fn apply_remote_moved(&self, position: MapFriendPosition) {
        let changed = self.state.with(|s| {
            if s.is_shutdown || !s.is_watching {
                return false;
            }
            let Some(friend) = s.by_id.get_mut(&position.user_id) else {
                return false;
            };
            if let Some(known) = friend.position.as_ref()
                && known.updated_at() > position.updated_at()
            {
                return false;
            }
            if friend.position.as_ref() == Some(&position) {
                return false;
            }
            friend.position = Some(position);
            true
        });
        if changed {
            self.publish_coalesced().await;
        }
    }

    pub async fn apply_remote_hidden(&self, user_id: &str) {
        let removed = self.state.with(|s| !s.is_shutdown && s.remove(user_id));
        if !removed {
            return;
        }
        self.persist().await;
        self.publish().await;
    }

    pub async fn apply_remote_reaction(&self, _reaction: MapReaction) {
        if self.state.with(|s| s.is_shutdown || !s.is_watching) {
            return;
        }
        self.reactions.schedule_refresh();
    }

    pub async fn send_reaction(&self, target_user_id: &str, emoji: &str, count: i64) -> RiseMapReactionOutcome {
        if self.is_shutdown() {
            return RiseMapReactionOutcome::Lost;
        }
        match self.rpc.send_reaction(target_user_id, emoji, count).await {
            Ok(_) => RiseMapReactionOutcome::Delivered,
            Err(error) if RiseMapFailure::is_reaction_paced(&error) => RiseMapReactionOutcome::Paced,
            Err(_) => RiseMapReactionOutcome::Lost,
        }
    }

    pub async fn reset(&self) {
        let reset = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.session_generation = s.session_generation.wrapping_add(1);
            if let Some(heartbeat_task) = s.heartbeat_task.take() {
                heartbeat_task.abort();
            }
            s.is_session_open = false;
            s.order.clear();
            s.by_id.clear();
            s.cache_source = RiseMapCacheSource::Miss;
            true
        });
        if reset {
            self.publish().await;
        }
    }

    pub fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            s.session_generation = s.session_generation.wrapping_add(1);
            if let Some(heartbeat_task) = s.heartbeat_task.take() {
                heartbeat_task.abort();
            }
        });
    }

    fn is_shutdown(&self) -> bool {
        self.state.with(|s| s.is_shutdown)
    }

    async fn open_session(self: &Arc<Self>) {
        let (sender, receiver) = oneshot::channel::<anyhow::Result<rise_canon::rise_response::map::OpenSession>>();
        let generation = self.state.with(|s| {
            s.session_generation = s.session_generation.wrapping_add(1);
            let generation = s.session_generation;
            if let Some(heartbeat_task) = s.heartbeat_task.take() {
                heartbeat_task.abort();
            }

            let previous = s.session_command_tail.take();
            let rpc = self.rpc.clone();
            let weak = Arc::downgrade(self);
            let request = RiseAccountRuntime::current().spawn(async move {
                if let Some(previous) = previous {
                    previous.await;
                }
                let opens = weak.upgrade().is_some_and(|this| this.should_open_session(generation));
                let result = if opens {
                    rpc.open_session().await
                } else {
                    Err(anyhow::Error::new(CancellationError))
                };
                let _ = sender.send(result);
            });
            s.session_command_tail = Some(
                async move {
                    let _ = request.await;
                }
                .boxed()
                .shared(),
            );
            generation
        });

        let answer = receiver.await.unwrap_or_else(|_| Err(anyhow::Error::new(CancellationError)));
        match answer {
            Ok(snapshot) => {
                let friends: Vec<MapLiveFriend> =
                    snapshot.friends.iter().filter_map(ProtoLiveFriendExt::model).collect();
                let now = Self::now_milliseconds();
                let applied = self.state.with(|s| {
                    if !s.owns_session(generation) {
                        return false;
                    }
                    s.replace(friends);
                    s.is_session_open = true;
                    s.cache_source = RiseMapCacheSource::Network;
                    s.adopt(snapshot.heartbeat_interval_ms, snapshot.session_ttl_ms);
                    s.last_beat_ms = now;
                    true
                });
                if !applied {
                    return;
                }
                self.persist().await;
                self.publish().await;
            }
            Err(error) => {
                let closed = self.state.with(|s| {
                    if !s.owns_session(generation) {
                        return false;
                    }
                    s.is_session_open = false;
                    true
                });
                if !closed {
                    return;
                }
                self.publish().await;
                if RiseMapFailure::is_not_member(&error) {
                    return;
                }
            }
        }
        if !self.should_open_session(generation) {
            return;
        }
        self.reactions.schedule_refresh();
        self.start_heartbeat(generation);
    }

    fn start_heartbeat(self: &Arc<Self>, generation: u64) {
        let weak = Arc::downgrade(self);
        self.state.with(|s| {
            if !s.owns_session(generation) {
                return;
            }
            if let Some(previous) = s.heartbeat_task.take() {
                previous.abort();
            }
            let handle = RiseAccountRuntime::current().spawn(async move {
                loop {
                    let Some(interval) = weak.upgrade().map(|this| this.state.with(|s| s.heartbeat_interval_ms)) else {
                        return;
                    };
                    tokio::time::sleep(Duration::from_millis(u64::try_from(interval).unwrap_or(0))).await;
                    let Some(this) = weak.upgrade() else {
                        return;
                    };
                    if !this.beat(generation).await {
                        return;
                    }
                }
            });
            s.heartbeat_task = Some(handle.abort_handle());
        });
    }

    /// false ends the loop that asked
    async fn beat(self: &Arc<Self>, generation: u64) -> bool {
        let now = Self::now_milliseconds();
        let reopens = self.state.with(|s| {
            if !s.owns_session(generation) {
                return None;
            }
            let lapsed = now - s.last_beat_ms > s.session_ttl_ms;
            Some(!s.is_session_open || lapsed)
        });
        match reopens {
            None => return false,
            Some(true) => {
                let this = self.clone();
                RiseAccountRuntime::current().spawn(async move {
                    this.open_session().await;
                });
                return false;
            }
            Some(false) => {}
        }

        match self.rpc.heartbeat().await {
            Ok(delta) => {
                let added: Vec<MapLiveFriend> = delta.added.iter().filter_map(ProtoLiveFriendExt::model).collect();
                let now = Self::now_milliseconds();
                let changed = self.state.with(|s| {
                    if !s.owns_session(generation) {
                        return None;
                    }
                    s.last_beat_ms = now;
                    s.adopt(delta.heartbeat_interval_ms, delta.session_ttl_ms);
                    let mut changed = false;
                    for user_id in &delta.removed_user_ids {
                        changed = s.remove(user_id) || changed;
                    }
                    for friend in added {
                        changed = s.upsert(friend) || changed;
                    }
                    Some(changed)
                });
                let Some(changed) = changed else {
                    return false;
                };
                if changed {
                    self.persist().await;
                    self.publish().await;
                }
                self.reactions.schedule_refresh();
                true
            }
            Err(error) => {
                let is_not_member = RiseMapFailure::is_not_member(&error);
                let step = self.state.with(|s| {
                    if s.is_shutdown || generation != s.session_generation {
                        return None;
                    }
                    if is_not_member {
                        s.is_session_open = false;
                    }
                    Some(is_not_member)
                });
                match step {
                    None => false,
                    Some(true) => {
                        self.publish().await;
                        false
                    }
                    Some(false) => true,
                }
            }
        }
    }

    async fn persist(&self) {
        let friends = self.state.with(|s| s.ordered_friends());
        self.resources
            .store(RiseMapResourceRepositoryResource::Live, RiseMapLiveRepositoryPersisted { friends })
            .await;
    }

    async fn publish_coalesced(&self) {
        let starts = self.state.with(|s| {
            s.needs_publish = true;
            if s.is_publishing {
                return false;
            }
            s.is_publishing = true;
            true
        });
        if !starts {
            return;
        }
        let _publishing = RiseMapLivePublishingReset { state: &self.state };
        loop {
            let proceeds = self.state.with(|s| {
                if !s.needs_publish || s.is_shutdown {
                    return false;
                }
                s.needs_publish = false;
                true
            });
            if !proceeds {
                break;
            }
            self.persist().await;
            self.publish().await;
        }
    }

    async fn publish(&self) {
        let snapshot = self.state.with(RiseMapLiveRepositoryState::next_snapshot);
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

    fn publish_on_main(snapshot: RiseMapLiveSnapshot, identity: &RiseMapSessionIdentity, cx: &mut App) {
        RiseMapPresentation::shared(cx).update(cx, |presentation, cx| {
            presentation.publish_live(snapshot, identity, cx);
        });
    }

    fn now_milliseconds() -> i64 {
        RiseClock::now().ms()
    }
}

impl RiseMapLiveRepositoryState {
    fn owns_session(&self, generation: u64) -> bool {
        !self.is_shutdown && generation == self.session_generation && self.is_watching
    }

    fn adopt(&mut self, heartbeat_ms: i64, ttl_ms: i64) {
        if heartbeat_ms > 0 {
            self.heartbeat_interval_ms = heartbeat_ms;
        }
        if ttl_ms > 0 {
            self.session_ttl_ms = ttl_ms;
        }
    }

    /// friends that stay keep their slot, newcomers go to the end
    fn replace(&mut self, friends: Vec<MapLiveFriend>) {
        let mut next: HashMap<String, MapLiveFriend> = HashMap::new();
        for friend in friends.iter().take(*MAXIMUM_FRIENDS) {
            if next.contains_key(friend.id()) {
                continue;
            }
            next.insert(friend.id().to_owned(), self.preserving_newer_position(friend.clone()));
        }
        let mut next_order: Vec<String> = self.order.iter().filter(|id| next.contains_key(*id)).cloned().collect();
        let mut placed: HashSet<String> = next_order.iter().cloned().collect();
        for friend in &friends {
            if next.contains_key(friend.id()) && placed.insert(friend.id().to_owned()) {
                next_order.push(friend.id().to_owned());
            }
        }
        self.order = next_order;
        self.by_id = next;
    }

    fn upsert(&mut self, incoming: MapLiveFriend) -> bool {
        let friend = self.preserving_newer_position(incoming);
        if let Some(known) = self.by_id.get(friend.id()) {
            if *known == friend {
                return false;
            }
            self.by_id.insert(friend.id().to_owned(), friend);
            return true;
        }
        if self.order.len() >= *MAXIMUM_FRIENDS {
            return false;
        }
        self.order.push(friend.id().to_owned());
        self.by_id.insert(friend.id().to_owned(), friend);
        true
    }

    fn preserving_newer_position(&self, incoming: MapLiveFriend) -> MapLiveFriend {
        let Some(known) = self.by_id.get(incoming.id()).and_then(|friend| friend.position.as_ref()) else {
            return incoming;
        };
        let Some(position) = incoming.position.as_ref() else {
            return incoming;
        };
        if known.updated_at() <= position.updated_at() {
            return incoming;
        }
        let mut merged = incoming;
        merged.position = Some(known.clone());
        merged
    }

    fn remove(&mut self, user_id: &str) -> bool {
        if self.by_id.remove(user_id).is_none() {
            return false;
        }
        self.order.retain(|id| id != user_id);
        true
    }

    fn ordered_friends(&self) -> Vec<MapLiveFriend> {
        self.order.iter().filter_map(|id| self.by_id.get(id).cloned()).collect()
    }

    fn next_snapshot(&mut self) -> RiseMapLiveSnapshot {
        self.revision = self.revision.wrapping_add(1);
        RiseMapLiveSnapshot {
            friends: self.ordered_friends(),
            is_session_open: self.is_session_open,
            cache_source: self.cache_source,
            revision: self.revision,
        }
    }
}
