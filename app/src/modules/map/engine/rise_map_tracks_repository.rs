use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Weak};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use gpui::{App, AsyncApp};
use rise_canon::{RiseLimits, RiseTimingCanon};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch, RiseStoredValue,
    RiseStoredValueWritePolicy,
};
use rise_engine::{RiseEngine, RiseWireRemoteError};
use rise_errors::ErrorCode;
use rise_i18n::tr;
use serde::{Deserialize, Serialize};
use tokio::task::AbortHandle;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::map::engine::core::map_track_tile_store::{MapDecodedTile, MapTrackTileStore};
use crate::modules::map::engine::core::rise_map_resource_repository::{
    RiseMapFailure, RiseMapResourceRepository, RiseMapResourceRepositoryResource,
};
use crate::modules::map::engine::core::rise_map_rpc::{RiseMapEndpoints, RiseMapRPC};
use crate::modules::map::engine::map_track_erasure_geometry::MapEraseStroke;
use crate::modules::map::engine::rise_map_engine_models::{
    RiseMapCacheSource, RiseMapLoadState, RiseMapMergePreviewResult, RiseMapMutationOutcome,
};
use crate::modules::map::engine::rise_map_live_repository::RiseMapLiveRepository;
use crate::modules::map::engine::rise_map_presentation::{RiseMapPresentation, RiseMapSessionIdentity};
use crate::modules::map::recorder::map_recorder_journal::MapRecorderJournal;
use crate::modules::map::recorder::map_track_codec::MapTrackGeometry;
use crate::modules::map::stores::map_actions::map_tracks_types::{
    MapPlace, MapPlaceKind, MapPlacePolicy, MapTileKey, MapTrackPeriod, MapTrackSummary, map_track_period_codable,
};
use crate::modules::map::stores::map_actions::map_tracks_wire_types::{ProtoRejectedSegmentExt, TrackTileExt};
use crate::modules::map::stores::map_actions::map_types::MapMergePreviewRequest;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseMapViewPreferences {
    pub shows_lines: bool,
    #[serde(with = "map_track_period_codable")]
    pub period: MapTrackPeriod,
}

impl Default for RiseMapViewPreferences {
    fn default() -> Self {
        Self { shows_lines: true, period: MapTrackPeriod::Week }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseMapTracksSnapshot {
    pub owner_id: Option<String>,
    pub summary: Option<MapTrackSummary>,
    pub period: MapTrackPeriod,
    pub shows_lines: bool,
    pub home: Option<MapPlace>,
    pub places: Vec<MapPlace>,
    pub load_state: RiseMapLoadState,
    pub error_text: Option<String>,
    pub cache_source: RiseMapCacheSource,
    pub revision: u64,
}

impl RiseMapTracksSnapshot {
    pub const EMPTY: RiseMapTracksSnapshot = RiseMapTracksSnapshot {
        owner_id: None,
        summary: None,
        period: MapTrackPeriod::Week,
        shows_lines: false,
        home: None,
        places: Vec::new(),
        load_state: RiseMapLoadState::Idle,
        error_text: None,
        cache_source: RiseMapCacheSource::Miss,
        revision: 0,
    };
}

/// what the map layer must redraw
/// a tick and not state, arriving tiles must not re-render the distance panel
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseMapTrackTilesTick {
    pub generation: u64,
    pub invalidated: Vec<MapTileKey>,
    pub invalidates_all: bool,
}

impl RiseMapTrackTilesTick {
    pub const IDLE: RiseMapTrackTilesTick =
        RiseMapTrackTilesTick { generation: 0, invalidated: Vec::new(), invalidates_all: false };

    pub fn idle() -> Self {
        Self::IDLE
    }
}

#[derive(Clone, Debug)]
pub enum MapPlaceSaveResult {
    Saved(MapPlace),
    Failed(Option<String>),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseMapTracksRepositoryStoredTile {
    revision: i64,
    since_day: i32,
    #[serde(with = "rise_map_tracks_stored_payload")]
    payload: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    part_lines: Option<Vec<u32>>,
}

mod rise_map_tracks_stored_payload {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(payload: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(payload))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        STANDARD.decode(encoded).map_err(D::Error::custom)
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct RiseMapTracksRepositoryStoredIndex {
    keys: Vec<String>,
}

static NAMESPACE: LazyLock<RiseStoreNamespace> = LazyLock::new(|| RiseStoreNamespace::new("map.tiles.v1"));
static SUMMARY_INTERVAL_MS: LazyLock<i64> = LazyLock::new(|| RiseTimingCanon::freshness_ms("map_track_summary"));
static BATCH_WINDOW_MS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("map_track_tile_batch_window_ms"));
static TILES_PER_REQUEST: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::service_value("map_max_tiles_per_request")).unwrap_or(0));
static PERSISTED_TILES: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::client_value("map_track_tiles_persisted")).unwrap_or(0));
static SEGMENTS_PER_UPLOAD: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::service_value("map_max_segments_per_upload")).unwrap_or(0));
static STAYS_PER_UPLOAD: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::service_value("map_max_stays_per_upload")).unwrap_or(0));
static DELETE_MAXIMUM_CALLS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("map_delete_tracks_max_calls"));
static LIVE_REFRESH_INTERVAL_MS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("map_watched_update_interval_ms"));
static HOME_RADIUS_METERS: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_home_radius_m"));
static MAXIMUM_FRIENDS: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::service_value("map_max_friends")).unwrap_or(0));
static MAXIMUM_HELD_PLACES: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(MapPlacePolicy::maximum_places() + 2).unwrap_or(0));

type RiseMapTracksHomeHandler = Arc<dyn Fn(Option<MapPlace>) -> BoxFuture<'static, ()> + Send + Sync>;

struct RiseMapTracksRepositoryState {
    owner_id: Option<String>,
    summary: Option<MapTrackSummary>,
    period: MapTrackPeriod,
    shows_lines: bool,
    own_preferences: RiseMapViewPreferences,
    friend_preferences: HashMap<String, RiseMapViewPreferences>,
    preferences_loaded: bool,
    preferences_load_task: Option<Shared<BoxFuture<'static, Option<RiseMapViewPreferences>>>>,
    preferences_revision: u64,
    home: Option<MapPlace>,
    places: Vec<MapPlace>,
    places_hydrated: bool,
    places_load_task: Option<Shared<BoxFuture<'static, Option<Vec<MapPlace>>>>>,
    places_generation: u64,
    is_refreshing_places: bool,
    is_mutating_place: bool,
    places_refreshed_ms: i64,
    has_loaded_home: bool,
    load_state: RiseMapLoadState,
    error_text: Option<String>,
    cache_source: RiseMapCacheSource,
    revision: u64,
    last_summary_ms: i64,

    wanted: HashSet<MapTileKey>,
    in_flight: HashSet<MapTileKey>,
    flush_task: Option<AbortHandle>,
    tick_generation: u64,
    owner_generation: u64,
    index: Option<RiseMapTracksRepositoryStoredIndex>,
    version_clock: i64,
    is_draining: bool,
    is_watching: bool,
    watching_sequence: u64,
    is_refreshing_summary: bool,
    last_checkpoint_ms: i64,
    checkpoint_interval_ms: i64,
    last_live_refresh_ms: i64,
    on_home_changed: Option<RiseMapTracksHomeHandler>,
    is_shutdown: bool,
    pending_erasure_request: Option<Vec<MapEraseStroke>>,
    open_sequence: u64,
    open_forced: bool,
}

impl RiseMapTracksRepositoryState {
    fn cancel_flush(&mut self) {
        if let Some(flush_task) = self.flush_task.take() {
            flush_task.abort();
        }
    }

    fn snapshot(&self) -> RiseMapTracksSnapshot {
        RiseMapTracksSnapshot {
            owner_id: self.owner_id.clone(),
            summary: self.summary.clone(),
            period: self.period,
            shows_lines: self.shows_lines,
            home: self.home.clone(),
            places: self.places.clone(),
            load_state: self.load_state,
            error_text: self.error_text.clone(),
            cache_source: self.cache_source,
            revision: self.revision,
        }
    }
}

/// guard that runs when the call ends, also when its future is dropped
struct RiseMapTracksRepositoryDefer<'a> {
    state: &'a RiseIsolated<RiseMapTracksRepositoryState>,
    body: Option<Box<dyn FnOnce(&mut RiseMapTracksRepositoryState) + Send + 'a>>,
}

impl<'a> RiseMapTracksRepositoryDefer<'a> {
    fn new(
        state: &'a RiseIsolated<RiseMapTracksRepositoryState>,
        body: impl FnOnce(&mut RiseMapTracksRepositoryState) + Send + 'a,
    ) -> Self {
        Self { state, body: Some(Box::new(body)) }
    }
}

impl Drop for RiseMapTracksRepositoryDefer<'_> {
    fn drop(&mut self) {
        if let Some(body) = self.body.take() {
            self.state.with(body);
        }
    }
}

/// tracks with summary of pyramid and periods, map tiles and sealed uploads
/// tiles are cache first, misses are batched into one request
pub struct RiseMapTracksRepository {
    pub tile_store: Arc<MapTrackTileStore>,
    rpc: RiseMapRPC,
    engine: Arc<RiseEngine>,
    resources: Arc<RiseMapResourceRepository>,
    identity: RiseMapSessionIdentity,
    key_prefix: String,
    state: RiseIsolated<RiseMapTracksRepositoryState>,
}

impl RiseMapTracksRepository {
    pub fn new(
        rpc: RiseMapRPC,
        engine: Arc<RiseEngine>,
        resources: Arc<RiseMapResourceRepository>,
        identity: RiseMapSessionIdentity,
    ) -> Self {
        let key_prefix = format!("{}:{}|", identity.account_id.len(), identity.account_id);
        Self {
            tile_store: Arc::new(MapTrackTileStore::new()),
            rpc,
            engine,
            resources,
            identity,
            key_prefix,
            state: RiseIsolated::new(RiseMapTracksRepositoryState {
                owner_id: None,
                summary: None,
                period: MapTrackPeriod::Week,
                shows_lines: false,
                own_preferences: RiseMapViewPreferences::default(),
                friend_preferences: HashMap::new(),
                preferences_loaded: false,
                preferences_load_task: None,
                preferences_revision: 0,
                home: None,
                places: Vec::new(),
                places_hydrated: false,
                places_load_task: None,
                places_generation: 0,
                is_refreshing_places: false,
                is_mutating_place: false,
                places_refreshed_ms: 0,
                has_loaded_home: false,
                load_state: RiseMapLoadState::Idle,
                error_text: None,
                cache_source: RiseMapCacheSource::Miss,
                revision: 0,
                last_summary_ms: 0,
                wanted: HashSet::new(),
                in_flight: HashSet::new(),
                flush_task: None,
                tick_generation: 0,
                owner_generation: 0,
                index: None,
                version_clock: 0,
                is_draining: false,
                is_watching: false,
                watching_sequence: 0,
                is_refreshing_summary: false,
                last_checkpoint_ms: 0,
                checkpoint_interval_ms: RiseMapLiveRepository::unwatched_update_ms(),
                last_live_refresh_ms: 0,
                on_home_changed: None,
                is_shutdown: false,
                pending_erasure_request: None,
                open_sequence: 0,
                open_forced: false,
            }),
        }
    }

    pub async fn has_loaded_home(&self) -> bool {
        self.state.with(|s| s.has_loaded_home)
    }

    pub async fn current_home(&self) -> Option<MapPlace> {
        self.state.with(|s| s.home.clone())
    }

    pub async fn current_places(&self) -> Vec<MapPlace> {
        self.state.with(|s| s.places.clone())
    }

    pub async fn set_on_home_changed(
        &self,
        handler: impl Fn(Option<MapPlace>) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    ) {
        let handler: RiseMapTracksHomeHandler = Arc::new(handler);
        self.state.with(|s| s.on_home_changed = Some(handler));
    }

    pub async fn set_upload_interval(&self, milliseconds: i64, watched: bool) {
        let unwatched = RiseMapLiveRepository::unwatched_update_ms();
        self.state.with(|s| {
            if s.is_shutdown {
                return;
            }
            let interval = if watched { milliseconds } else { unwatched };
            s.checkpoint_interval_ms = (*LIVE_REFRESH_INTERVAL_MS).max(interval.min(unwatched));
        });
    }

    pub async fn set_watching(&self, watching: bool, sequence: u64) {
        self.state.with(|s| {
            if s.is_shutdown || sequence <= s.watching_sequence {
                return;
            }
            s.watching_sequence = sequence;
            s.is_watching = watching;
        });
    }

    pub async fn friend_moved(&self, user_id: &str) {
        let refreshes = self.state.with(|s| {
            if s.is_shutdown || !s.is_watching || s.owner_id.as_deref() != Some(user_id) {
                return false;
            }
            let now = Self::now_milliseconds();
            if now - s.last_live_refresh_ms < *LIVE_REFRESH_INTERVAL_MS {
                return false;
            }
            s.last_live_refresh_ms = now;
            true
        });
        if refreshes {
            self.refresh_summary().await;
        }
    }

    /// refreshes whoever's tracks are open
    /// a reconnect or foreground return must not throw the reader
    /// out of a friend's routes back into their own
    pub async fn revalidate(&self, force: bool) {
        let owner_id = self.state.with(|s| s.owner_id.clone());
        self.open(owner_id, force).await;
    }

    /// opens the tracks of owner_id, none means own
    /// another owner starts from an empty memory store so tiles of two people never mix
    pub async fn open(&self, owner_id: Option<String>, force: bool) {
        let sequence = self.state.with(|s| {
            s.open_sequence = s.open_sequence.wrapping_add(1);
            s.open_forced |= force;
            s.open_sequence
        });
        self.load_view_preferences().await;
        self.hydrate_places().await;
        let refreshes_places = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            Some(force || Self::now_milliseconds() - s.places_refreshed_ms >= *SUMMARY_INTERVAL_MS)
        });
        let Some(refreshes_places) = refreshes_places else {
            return;
        };
        if refreshes_places {
            self.refresh_places().await;
        }
        if self.is_shut_down() {
            return;
        }
        let latest_force = self.state.with(|s| (s.open_sequence == sequence).then(|| std::mem::take(&mut s.open_forced)));
        let Some(latest_force) = latest_force else {
            tracing::warn!(target: "riseonly::X2284", requested = ?owner_id, "tracks open superseded by a newer open");
            return;
        };
        let force = force || latest_force;
        let next = owner_id
            .filter(|requested| *requested != self.identity.account_id && !requested.is_empty());
        let switched = self.state.with(|s| {
            if next != s.owner_id || (s.summary.is_none() && s.load_state == RiseMapLoadState::Idle) {
                s.owner_generation += 1;
                let generation = s.owner_generation;
                s.cancel_flush();
                s.in_flight.clear();
                s.is_refreshing_summary = false;
                s.last_live_refresh_ms = 0;
                s.owner_id = next.clone();
                let preferences = match next.as_ref() {
                    Some(owner) => s.friend_preferences.get(owner).cloned().unwrap_or_default(),
                    None => s.own_preferences.clone(),
                };
                s.period = preferences.period;
                s.shows_lines = preferences.shows_lines;
                s.summary = None;
                s.last_summary_ms = 0;
                s.wanted.clear();
                self.tile_store.remove_all();
                s.load_state = RiseMapLoadState::Idle;
                s.cache_source = RiseMapCacheSource::Miss;
                Some(generation)
            } else {
                None
            }
        });

        if let Some(generation) = switched {
            if next.is_none()
                && let Some(cached) =
                    self.resources.cached::<MapTrackSummary>(RiseMapResourceRepositoryResource::Tracks).await
            {
                let adopted = self.state.with(|s| {
                    if s.is_shutdown || generation != s.owner_generation || s.owner_id.is_some() {
                        return false;
                    }
                    self.adopt(s, cached, RiseMapCacheSource::PersistentStore);
                    true
                });
                if !adopted {
                    return;
                }
            }
            if !self.is_owner_generation(generation) {
                return;
            }
            self.publish().await;
            if !self.is_owner_generation(generation) {
                return;
            }
            self.tick(Vec::new(), true).await;
            if !self.is_owner_generation(generation) {
                return;
            }
        }

        let refreshes = self.state.with(|s| {
            let is_stale = Self::now_milliseconds() - s.last_summary_ms >= *SUMMARY_INTERVAL_MS;
            force || is_stale || s.summary.is_none()
        });
        if !refreshes {
            return;
        }
        self.refresh_summary().await;
        if self.state.with(|s| s.owner_id.is_none() && s.home.is_none()) {
            self.refresh_places().await;
        }
    }

    pub async fn set_period(&self, next: MapTrackPeriod) {
        self.load_view_preferences().await;
        let stored_preferences = self.state.with(|s| {
            if s.is_shutdown || next == s.period {
                return None;
            }
            s.period = next;
            let stored_preferences = Self::record_current_preferences(s);
            self.configure_store(s);
            Some(stored_preferences)
        });
        let Some(stored_preferences) = stored_preferences else {
            return;
        };
        self.publish().await;
        self.tick(Vec::new(), true).await;
        if let Some((preferences, preference_revision)) = stored_preferences {
            self.resources.store_view_preferences(preferences, preference_revision).await;
        }
    }

    pub async fn load_view_preferences(&self) {
        let task = self.state.with(|s| {
            if s.is_shutdown || s.preferences_loaded {
                return None;
            }
            if s.preferences_load_task.is_none() {
                let resources = self.resources.clone();
                s.preferences_load_task = Some(
                    async move {
                        resources
                            .cached::<RiseMapViewPreferences>(RiseMapResourceRepositoryResource::ViewPreferences)
                            .await
                    }
                    .boxed()
                    .shared(),
                );
            }
            s.preferences_load_task.clone()
        });
        let Some(task) = task else {
            return;
        };
        let saved = task.await;
        let applied = self.state.with(|s| {
            if s.is_shutdown || s.preferences_loaded {
                return false;
            }
            s.preferences_load_task = None;
            s.preferences_loaded = true;
            let preferences = saved.unwrap_or_default();
            s.shows_lines = preferences.shows_lines;
            s.period = preferences.period;
            s.own_preferences = preferences;
            self.configure_store(s);
            true
        });
        if applied {
            self.publish().await;
        }
    }

    fn record_current_preferences(s: &mut RiseMapTracksRepositoryState) -> Option<(RiseMapViewPreferences, u64)> {
        let preferences = RiseMapViewPreferences { shows_lines: s.shows_lines, period: s.period };
        if let Some(owner_id) = s.owner_id.clone() {
            if s.friend_preferences.len() >= *MAXIMUM_FRIENDS && !s.friend_preferences.contains_key(&owner_id) {
                s.friend_preferences.clear();
            }
            s.friend_preferences.insert(owner_id, preferences);
            return None;
        }
        s.own_preferences = preferences.clone();
        s.preferences_revision = s.preferences_revision.wrapping_add(1);
        Some((preferences, s.preferences_revision))
    }

    pub async fn toggle_lines_visibility(&self) -> Option<bool> {
        self.change_lines_visibility(None).await
    }

    pub async fn set_lines_visible(&self, visible: bool) {
        let _ = self.change_lines_visibility(Some(visible)).await;
    }

    async fn change_lines_visibility(&self, visible: Option<bool>) -> Option<bool> {
        self.load_view_preferences().await;
        let step = self.state.with(|s| {
            if s.is_shutdown {
                return Err(None);
            }
            let next = visible.unwrap_or(!s.shows_lines);
            if next == s.shows_lines {
                return Err(Some(next));
            }
            s.shows_lines = next;
            Ok((next, Self::record_current_preferences(s)))
        });
        let (next, stored_preferences) = match step {
            Ok(step) => step,
            Err(answer) => return answer,
        };
        self.publish().await;
        if let Some((preferences, preference_revision)) = stored_preferences {
            self.resources.store_view_preferences(preferences, preference_revision).await;
        }
        if self.is_shut_down() { None } else { Some(next) }
    }

    /// misses reported by the map layer from its background queue
    pub async fn request(self: &Arc<Self>, keys: Vec<MapTileKey>) {
        let this = Arc::downgrade(self);
        self.state.with(|s| {
            if s.is_shutdown || s.summary.is_none() {
                tracing::warn!(target: "riseonly::X2068", keys = keys.len(), has_summary = s.summary.is_some(), "track tiles request skipped");
                return;
            }
            let fresh: Vec<MapTileKey> = keys.into_iter().filter(|key| !s.in_flight.contains(key)).collect();
            if fresh.is_empty() {
                return;
            }
            s.wanted.extend(fresh);
            if s.flush_task.is_some() {
                return;
            }
            s.flush_task = Some(Self::start_flush(this, Some(*BATCH_WINDOW_MS)));
        });
    }

    /// sends what the recorder sealed
    /// server folds a segment it already took so repeating after a lost answer is safe
    /// an id neither accepted nor finally refused stays on disk
    pub async fn drain_uploads(&self, checkpoint: bool) {
        let starts = self.state.with(|s| {
            if s.is_shutdown || s.is_draining {
                return false;
            }
            if checkpoint && Self::now_milliseconds() - s.last_checkpoint_ms < s.checkpoint_interval_ms {
                return false;
            }
            s.is_draining = true;
            true
        });
        if !starts {
            return;
        }
        let _draining = RiseMapTracksRepositoryDefer::new(&self.state, |s| s.is_draining = false);

        let can_call = matches!(self.engine.can_call(&RiseMapEndpoints::upload_track().descriptor()).await, Ok(true));
        if !can_call || self.is_shut_down() {
            return;
        }
        if checkpoint {
            self.state.with(|s| s.last_checkpoint_ms = Self::now_milliseconds());
        }
        let journal = MapRecorderJournal::for_account(&self.identity.account_id);
        let mut did_checkpoint = false;
        while !self.is_shut_down() {
            let mut segments = journal.pending_segments(*SEGMENTS_PER_UPLOAD);
            let stays: Vec<_> = journal.pending_stays().into_iter().take(*STAYS_PER_UPLOAD).collect();
            if checkpoint && !did_checkpoint && segments.is_empty() {
                did_checkpoint = true;
                journal.seal_open_segment(true);
                segments = journal.pending_segments(*SEGMENTS_PER_UPLOAD);
            }
            if segments.is_empty() && stays.is_empty() {
                return;
            }
            let Ok(response) = self.rpc.upload_track(&segments, &stays).await else {
                return;
            };
            if self.is_shut_down() {
                return;
            }

            let mut settled: HashSet<String> = response.accepted_ids.iter().cloned().collect();
            for rejected in response.rejected.iter().filter(|rejected| rejected.is_final()) {
                settled.insert(rejected.id.clone());
            }
            if !journal.resolve(&settled, &stays) {
                return;
            }

            let refreshes_summary = self.state.with(|s| {
                if s.owner_id.is_none()
                    && let Some(summary) = s.summary.as_ref()
                    && response.revision != summary.revision
                {
                    s.last_summary_ms = 0;
                    return true;
                }
                false
            });
            if refreshes_summary {
                self.refresh_summary().await;
            }
            if response.places_changed {
                self.refresh_places().await;
            }
            if settled.is_empty() && stays.is_empty() {
                return;
            }
        }
    }

    /// merges a sample scene or the owner's own walks with settings still being chosen
    /// nothing is saved and no tile touched, the settings page draws the answer
    pub async fn preview_merge(&self, request: MapMergePreviewRequest) -> RiseMapMergePreviewResult {
        if self.is_shut_down() {
            return RiseMapMergePreviewResult::Failed(None);
        }
        match self.rpc.preview_track_merge(&request).await {
            Ok(preview) => RiseMapMergePreviewResult::Loaded(preview),
            Err(error) => RiseMapMergePreviewResult::Failed(RiseMapFailure::text(&error)),
        }
    }

    pub async fn apply_own_colouring(&self, cooling_days: i64, heat_walks: i64) {
        if self.state.with(|s| s.is_shutdown || s.owner_id.is_some()) {
            return;
        }
        if !self.tile_store.set_colouring(cooling_days, heat_walks) {
            return;
        }
        self.tick(Vec::new(), true).await;
    }

    async fn hydrate_places(&self) {
        let step = self.state.with(|s| {
            if s.is_shutdown || s.places_hydrated {
                return None;
            }
            let generation = s.places_generation;
            let task = match s.places_load_task.as_ref() {
                Some(pending) => pending.clone(),
                None => {
                    let resources = self.resources.clone();
                    let task = async move {
                        resources.cached::<Vec<MapPlace>>(RiseMapResourceRepositoryResource::Places).await
                    }
                    .boxed()
                    .shared();
                    s.places_load_task = Some(task.clone());
                    task
                }
            };
            Some((generation, task))
        });
        let Some((generation, task)) = step else {
            return;
        };
        let cached = task.await;
        let hydrated = self.state.with(|s| {
            if s.is_shutdown || s.places_hydrated || generation != s.places_generation {
                return None;
            }
            s.places_hydrated = true;
            s.places_load_task = None;
            let cached = cached?;
            s.places = cached.into_iter().take(*MAXIMUM_HELD_PLACES).collect();
            s.home = s.places.iter().find(|place| place.kind == MapPlaceKind::Home).cloned();
            s.has_loaded_home = true;
            Some((s.home.clone(), s.on_home_changed.clone()))
        });
        let Some((home, on_home_changed)) = hydrated else {
            return;
        };
        if let Some(on_home_changed) = on_home_changed {
            on_home_changed(home).await;
        }
        self.publish().await;
    }

    pub async fn refresh_places(&self) {
        self.hydrate_places().await;
        let generation = self.state.with(|s| {
            if s.is_shutdown || s.is_refreshing_places || s.is_mutating_place {
                return None;
            }
            s.is_refreshing_places = true;
            Some(s.places_generation)
        });
        let Some(generation) = generation else {
            return;
        };
        let _refreshing = RiseMapTracksRepositoryDefer::new(&self.state, |s| s.is_refreshing_places = false);
        let Ok(fresh) = self.rpc.get_places().await else {
            return;
        };
        let accepts = self.state.with(|s| {
            if s.is_shutdown || generation != s.places_generation {
                return false;
            }
            s.places_generation = s.places_generation.wrapping_add(1);
            s.places_refreshed_ms = Self::now_milliseconds();
            true
        });
        if accepts {
            self.accept_places(fresh).await;
        }
    }

    pub async fn save_place(&self, place: MapPlace, creating: bool) -> MapPlaceSaveResult {
        self.hydrate_places().await;
        let step = self.state.with(|s| {
            if s.is_shutdown || s.is_mutating_place {
                return Err(MapPlaceSaveResult::Failed(None));
            }
            let custom = s.places.iter().filter(|held| held.kind == MapPlaceKind::Custom).count();
            if creating && i64::try_from(custom).unwrap_or(i64::MAX) >= MapPlacePolicy::maximum_places() {
                return Err(MapPlaceSaveResult::Failed(Some(tr("map_place_limit"))));
            }
            if !creating && !s.places.iter().any(|held| held.id == place.id) {
                return Err(MapPlaceSaveResult::Failed(None));
            }
            s.is_mutating_place = true;
            s.places_generation = s.places_generation.wrapping_add(1);
            let previous = s.places.clone();
            let mut optimistic: Vec<MapPlace> = s.places.iter().filter(|held| held.id != place.id).cloned().collect();
            optimistic.push(place.clone());
            Ok((s.places_generation, previous, optimistic))
        });
        let (generation, previous, optimistic) = match step {
            Ok(step) => step,
            Err(result) => return result,
        };
        let _mutating = RiseMapTracksRepositoryDefer::new(&self.state, |s| s.is_mutating_place = false);
        self.accept_places(optimistic).await;
        if !self.is_places_generation(generation) {
            return MapPlaceSaveResult::Failed(None);
        }
        match self.rpc.save_place(&place, creating).await {
            Ok(confirmed) => {
                if !self.is_places_generation(generation) {
                    return MapPlaceSaveResult::Failed(None);
                }
                let next = self.state.with(|s| {
                    let mut next: Vec<MapPlace> = s
                        .places
                        .iter()
                        .filter(|held| held.id != place.id && held.id != confirmed.id)
                        .cloned()
                        .collect();
                    next.push(confirmed.clone());
                    next
                });
                self.accept_places(next).await;
                MapPlaceSaveResult::Saved(confirmed)
            }
            Err(error) => {
                if !self.is_places_generation(generation) {
                    return MapPlaceSaveResult::Failed(None);
                }
                self.accept_places(previous).await;
                MapPlaceSaveResult::Failed(RiseMapFailure::text(&error))
            }
        }
    }

    pub async fn delete_place(&self, place: &MapPlace) -> RiseMapMutationOutcome {
        let step = self.state.with(|s| {
            if s.is_shutdown || s.is_mutating_place {
                return None;
            }
            s.is_mutating_place = true;
            s.places_generation = s.places_generation.wrapping_add(1);
            let previous = s.places.clone();
            let remaining: Vec<MapPlace> = s.places.iter().filter(|held| held.id != place.id).cloned().collect();
            Some((s.places_generation, previous, remaining))
        });
        let Some((generation, previous, remaining)) = step else {
            return RiseMapMutationOutcome::Failed(None);
        };
        let _mutating = RiseMapTracksRepositoryDefer::new(&self.state, |s| s.is_mutating_place = false);
        self.accept_places(remaining).await;
        if !self.is_places_generation(generation) {
            return RiseMapMutationOutcome::Failed(None);
        }
        match self.rpc.delete_place(&place.id).await {
            Ok(()) => {
                if !self.is_places_generation(generation) {
                    return RiseMapMutationOutcome::Failed(None);
                }
                RiseMapMutationOutcome::Done
            }
            Err(error) => {
                if !self.is_places_generation(generation) {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.accept_places(previous).await;
                RiseMapMutationOutcome::Failed(RiseMapFailure::text(&error))
            }
        }
    }

    async fn accept_places(&self, fresh: Vec<MapPlace>) {
        let places = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            s.places = fresh.into_iter().take(*MAXIMUM_HELD_PLACES).collect();
            s.home = s.places.iter().find(|place| place.kind == MapPlaceKind::Home).cloned();
            s.has_loaded_home = true;
            Some(s.places.clone())
        });
        let Some(places) = places else {
            return;
        };
        self.resources.store(RiseMapResourceRepositoryResource::Places, places).await;
        let notify = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            Some((s.home.clone(), s.on_home_changed.clone()))
        });
        let Some((home, on_home_changed)) = notify else {
            return;
        };
        if let Some(on_home_changed) = on_home_changed {
            on_home_changed(home).await;
        }
        self.publish().await;
    }

    /// a manual home beats the one the server infers from night stays
    /// recorder stops writing inside it at once
    pub async fn set_home(&self, latitude: f64, longitude: f64) -> RiseMapMutationOutcome {
        self.hydrate_places().await;
        let generation = self.state.with(|s| {
            if s.is_shutdown || s.is_mutating_place {
                return None;
            }
            s.is_mutating_place = true;
            s.places_generation = s.places_generation.wrapping_add(1);
            Some(s.places_generation)
        });
        let Some(generation) = generation else {
            return RiseMapMutationOutcome::Failed(None);
        };
        let _mutating = RiseMapTracksRepositoryDefer::new(&self.state, |s| s.is_mutating_place = false);
        let place = match self
            .rpc
            .set_place(None, MapPlaceKind::Home, "", latitude, longitude, *HOME_RADIUS_METERS)
            .await
        {
            Ok(place) => place,
            Err(error) => return RiseMapMutationOutcome::Failed(RiseMapFailure::text(&error)),
        };
        if !self.is_places_generation(generation) {
            return RiseMapMutationOutcome::Failed(None);
        }
        let Some(place) = place else {
            return RiseMapMutationOutcome::Failed(None);
        };
        let next = self.state.with(|s| {
            let mut next: Vec<MapPlace> =
                s.places.iter().filter(|held| held.kind != MapPlaceKind::Home).cloned().collect();
            next.push(place);
            next
        });
        self.accept_places(next).await;
        RiseMapMutationOutcome::Done
    }

    pub async fn delete_home(&self) -> RiseMapMutationOutcome {
        let Some(home) = self.state.with(|s| s.home.clone()) else {
            return RiseMapMutationOutcome::Done;
        };
        self.delete_place(&home).await
    }

    /// flush visible local geometry before saving a stable retryable edit batch
    pub async fn erase_tracks(&self, strokes: Vec<MapEraseStroke>) -> RiseMapMutationOutcome {
        let generation = self.state.with(|s| {
            if s.is_shutdown || s.owner_id.is_some() || strokes.is_empty() {
                return None;
            }
            Some(s.owner_generation)
        });
        let Some(generation) = generation else {
            return RiseMapMutationOutcome::Failed(None);
        };
        let stroke_ids: Vec<&str> = strokes.iter().map(|stroke| stroke.id.as_str()).collect();
        let is_new_request = self.state.with(|s| {
            s.pending_erasure_request
                .as_ref()
                .map(|pending| pending.iter().map(|stroke| stroke.id.as_str()).eq(stroke_ids.iter().copied()))
                != Some(true)
        });
        if is_new_request {
            let journal = MapRecorderJournal::for_account(&self.identity.account_id);
            if !journal.seal_open_segment(true) {
                return RiseMapMutationOutcome::Failed(None);
            }
            let cutoff = Self::now_milliseconds();
            self.drain_uploads(false).await;
            if !self.is_owner_generation(generation) || !journal.pending_segments(1).is_empty() {
                return RiseMapMutationOutcome::Failed(None);
            }
            let request: Vec<MapEraseStroke> = strokes
                .iter()
                .map(|stroke| MapEraseStroke {
                    id: stroke.id.clone(),
                    since_day: stroke.since_day,
                    until_ms: cutoff,
                    radius: stroke.radius,
                    points: stroke.points.clone(),
                })
                .collect();
            self.state.with(|s| s.pending_erasure_request = Some(request));
        }
        let Some(request) = self.state.with(|s| s.pending_erasure_request.clone()) else {
            return RiseMapMutationOutcome::Failed(None);
        };
        let response = match self.rpc.erase_tracks(&request).await {
            Ok(response) => response,
            Err(error) => return RiseMapMutationOutcome::Failed(RiseMapFailure::text(&error)),
        };
        let edited_generation = self.state.with(|s| {
            if s.is_shutdown || generation != s.owner_generation {
                return None;
            }
            s.pending_erasure_request = None;
            s.owner_generation = s.owner_generation.wrapping_add(1);
            s.cancel_flush();
            s.wanted.clear();
            s.in_flight.clear();
            s.is_refreshing_summary = false;
            Some(s.owner_generation)
        });
        let Some(edited_generation) = edited_generation else {
            return RiseMapMutationOutcome::Failed(None);
        };
        self.tile_store.remove_all();
        self.remove_persisted_tiles().await;
        let edited = self.state.with(|s| {
            if s.is_shutdown || s.owner_generation != edited_generation || s.owner_id.is_some() {
                return Err(());
            }
            let Some(summary) = s.summary.clone() else {
                return Ok(None);
            };
            let edited = MapTrackSummary { revision: response.revision, ..summary };
            self.adopt(s, edited.clone(), RiseMapCacheSource::Network);
            Ok(Some(edited))
        });
        let Ok(edited) = edited else {
            return RiseMapMutationOutcome::Done;
        };
        if let Some(edited) = edited {
            self.resources.store(RiseMapResourceRepositoryResource::Tracks, edited).await;
            if !self.is_owner_generation(edited_generation) {
                return RiseMapMutationOutcome::Done;
            }
            self.publish().await;
        }
        if !self.is_owner_generation(edited_generation) {
            return RiseMapMutationOutcome::Done;
        }
        self.state.with(|s| s.last_summary_ms = 0);
        self.refresh_summary().await;
        self.tick(Vec::new(), true).await;
        RiseMapMutationOutcome::Done
    }

    pub async fn delete_all_tracks(&self) -> RiseMapMutationOutcome {
        let is_shutdown = self.state.with(|s| {
            if s.is_shutdown {
                return true;
            }
            s.pending_erasure_request = None;
            false
        });
        if is_shutdown {
            return RiseMapMutationOutcome::Failed(None);
        }
        let mut calls = 0_i64;
        while calls < *DELETE_MAXIMUM_CALLS {
            calls += 1;
            match self.rpc.delete_all_tracks().await {
                Ok(response) => {
                    if self.is_shut_down() {
                        return RiseMapMutationOutcome::Failed(None);
                    }
                    if !response.has_more {
                        break;
                    }
                }
                Err(error) => return RiseMapMutationOutcome::Failed(RiseMapFailure::text(&error)),
            }
        }
        MapRecorderJournal::for_account(&self.identity.account_id).erase_all();
        self.tile_store.remove_all();
        self.remove_persisted_tiles().await;
        self.state.with(|s| s.last_summary_ms = 0);
        self.refresh_summary().await;
        self.tick(Vec::new(), true).await;
        if calls < *DELETE_MAXIMUM_CALLS {
            RiseMapMutationOutcome::Done
        } else {
            RiseMapMutationOutcome::Failed(None)
        }
    }

    pub async fn reset(&self) {
        let reset = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            s.pending_erasure_request = None;
            s.owner_generation += 1;
            s.is_refreshing_summary = false;
            s.cancel_flush();
            s.owner_id = None;
            s.summary = None;
            s.home = None;
            s.places.clear();
            s.places_load_task = None;
            s.places_hydrated = true;
            s.places_generation = s.places_generation.wrapping_add(1);
            s.places_refreshed_ms = 0;
            s.has_loaded_home = false;
            s.wanted.clear();
            s.in_flight.clear();
            s.load_state = RiseMapLoadState::Idle;
            s.cache_source = RiseMapCacheSource::Miss;
            s.last_summary_ms = 0;
            Some(s.on_home_changed.clone())
        });
        let Some(on_home_changed) = reset else {
            return;
        };
        self.tile_store.remove_all();
        self.remove_persisted_tiles().await;
        if let Some(on_home_changed) = on_home_changed {
            on_home_changed(None).await;
        }
        self.publish().await;
        self.tick(Vec::new(), true).await;
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            s.places_load_task = None;
            s.pending_erasure_request = None;
            s.preferences_load_task = None;
            s.cancel_flush();
        });
        self.tile_store.remove_all();
    }

    async fn refresh_summary(&self) {
        let step = self.state.with(|s| {
            if s.is_shutdown || s.is_refreshing_summary {
                return None;
            }
            s.is_refreshing_summary = true;
            Some(s.owner_generation)
        });
        let Some(generation) = step else {
            return;
        };
        let _refreshing = RiseMapTracksRepositoryDefer::new(&self.state, move |s| {
            if generation == s.owner_generation {
                s.is_refreshing_summary = false;
            }
        });
        let shows_loading = self.state.with(|s| {
            if s.summary.is_some() {
                return false;
            }
            s.load_state = RiseMapLoadState::Loading;
            true
        });
        if shows_loading {
            self.publish().await;
        }
        let now = RiseClock::now();
        let today = MapTrackGeometry::local_day(now.ms(), i64::from(TimeZone::current().seconds_from_gmt(&now)) / 60);
        let owner_id = self.state.with(|s| s.owner_id.clone());
        match self.rpc.get_track_summary(owner_id.as_deref(), today).await {
            Ok(fresh) => {
                let stores = self.state.with(|s| {
                    if s.is_shutdown || generation != s.owner_generation {
                        return None;
                    }
                    s.last_summary_ms = Self::now_milliseconds();
                    self.adopt(s, fresh.clone(), RiseMapCacheSource::Network);
                    Some(s.owner_id.is_none())
                });
                let Some(stores) = stores else {
                    return;
                };
                if stores {
                    self.resources.store(RiseMapResourceRepositoryResource::Tracks, fresh).await;
                }
                self.publish().await;
                if self.is_owner_generation(generation) {
                    self.tick(Vec::new(), true).await;
                }
            }
            Err(error) => {
                if !self.is_owner_generation(generation) {
                    return;
                }
                let is_unavailable = error
                    .downcast_ref::<RiseWireRemoteError>()
                    .is_some_and(|remote| remote.code == ErrorCode::MapTargetUnavailable.wire());
                if is_unavailable {
                    self.state.with(|s| s.summary = None);
                    self.tile_store.remove_all();
                    self.tick(Vec::new(), true).await;
                }
                let failed = self.state.with(|s| {
                    if s.summary.is_some() {
                        return false;
                    }
                    s.load_state = RiseMapLoadState::Failed;
                    s.error_text = RiseMapFailure::text(&error);
                    true
                });
                if failed {
                    self.publish().await;
                }
            }
        }
    }

    fn adopt(&self, s: &mut RiseMapTracksRepositoryState, next: MapTrackSummary, source: RiseMapCacheSource) {
        s.summary = Some(next);
        s.load_state = RiseMapLoadState::Loaded;
        s.error_text = None;
        s.cache_source = source;
        self.configure_store(s);
    }

    fn configure_store(&self, s: &RiseMapTracksRepositoryState) {
        let Some(summary) = s.summary.as_ref() else {
            return;
        };
        self.tile_store.configure(
            summary.bands.clone(),
            summary.revision,
            summary.stats(s.period).map(|stats| stats.since_day).unwrap_or(0),
        );
    }

    /// the batching task, a plain function
    /// so the flush it spawns doesnt make the type of flush depend on itself
    fn start_flush(this: Weak<Self>, delay_ms: Option<i64>) -> AbortHandle {
        let flush = RiseAccountRuntime::current().spawn(async move {
            if let Some(delay_ms) = delay_ms {
                tokio::time::sleep(Duration::from_millis(u64::try_from(delay_ms).unwrap_or(0))).await;
            }
            if let Some(this) = this.upgrade() {
                this.flush().await;
            }
        });
        flush.abort_handle()
    }

    fn start_summary_refresh(this: Weak<Self>) {
        RiseAccountRuntime::current().spawn(async move {
            if let Some(this) = this.upgrade() {
                this.refresh_summary().await;
            }
        });
    }

    async fn flush(self: &Arc<Self>) {
        let step = self.state.with(|s| {
            s.flush_task = None;
            if s.is_shutdown {
                return None;
            }
            let summary = s.summary.clone()?;
            let batch: Vec<MapTileKey> =
                s.wanted.difference(&s.in_flight).copied().take(*TILES_PER_REQUEST).collect();
            if batch.is_empty() {
                return None;
            }
            for key in &batch {
                s.wanted.remove(key);
                s.in_flight.insert(*key);
            }
            Some((summary, s.owner_generation, batch))
        });
        let Some((summary, generation, batch)) = step else {
            return;
        };
        let this = Arc::downgrade(self);
        let _settle = RiseMapTracksRepositoryDefer::new(&self.state, {
            let batch = batch.clone();
            move |s| {
                if generation == s.owner_generation {
                    for key in &batch {
                        s.in_flight.remove(key);
                    }
                }
                if generation == s.owner_generation && !s.wanted.is_empty() && s.flush_task.is_none() && !s.is_shutdown {
                    s.flush_task = Some(Self::start_flush(this, None));
                }
            }
        });

        if self.state.with(|s| s.owner_id.is_none()) {
            let mut from_store: Vec<MapTileKey> = Vec::new();
            for key in batch.iter().copied() {
                if self.tile_store.snapshot(key).is_some() {
                    continue;
                }
                let Some(stored) = self.stored_tile(key).await else {
                    continue;
                };
                let Some(band) = summary.bands.iter().find(|band| band.band == key.band) else {
                    continue;
                };
                let Some(decoded) = MapTrackTileStore::decode(
                    key,
                    band,
                    &stored.payload,
                    stored.revision,
                    stored.since_day,
                    stored.part_lines.as_deref().unwrap_or_default(),
                ) else {
                    continue;
                };
                if !self.is_owner_generation(generation) {
                    return;
                }
                self.tile_store.put(decoded);
                from_store.push(key);
            }
            if !self.is_owner_generation(generation) {
                return;
            }
            if !from_store.is_empty() {
                self.tick(from_store, false).await;
            }
        }

        let period = self.state.with(|s| s.period);
        let since_day = summary.stats(period).map(|stats| stats.since_day).unwrap_or(0);
        let detail_band = summary.bands.last().map(|band| band.band);
        let mut missing: Vec<MapTileKey> = Vec::new();
        let mut held_by_revision: HashMap<i64, Vec<MapTileKey>> = HashMap::new();
        for key in batch.iter().copied() {
            let Some((held_revision, held_since_day)) = self.tile_store.snapshot(key) else {
                missing.push(key);
                continue;
            };
            let is_shallow = Some(key.band) == detail_band && held_since_day > since_day;
            if is_shallow {
                missing.push(key);
            } else if held_revision != summary.revision {
                held_by_revision.entry(held_revision).or_default().push(key);
            }
        }

        let mut changed: Vec<MapTileKey> = Vec::new();
        if !missing.is_empty() {
            changed.extend(self.fetch(&missing, since_day, 0, generation).await);
        }
        for (known, keys) in held_by_revision {
            if !self.is_owner_generation(generation) {
                return;
            }
            changed.extend(self.fetch(&keys, since_day, known, generation).await);
        }
        if !self.is_owner_generation(generation) || changed.is_empty() {
            return;
        }
        self.tick(changed, false).await;
    }

    async fn fetch(
        self: &Arc<Self>,
        keys: &[MapTileKey],
        since_day: i32,
        known_revision: i64,
        generation: u64,
    ) -> Vec<MapTileKey> {
        let request = self.state.with(|s| {
            if s.is_shutdown || generation != s.owner_generation {
                return None;
            }
            Some((s.owner_id.clone(), s.summary.as_ref().map(|summary| summary.revision)))
        });
        let Some((owner_id, requested_revision)) = request else {
            return Vec::new();
        };
        let response = match self.rpc.get_track_tiles(owner_id.as_deref(), keys, since_day, known_revision).await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(target: "riseonly::X2068", keys = keys.len(), error = %format!("{error:#}"), "get_track_tiles failed");
                return Vec::new();
            }
        };
        tracing::warn!(
            target: "riseonly::X2068",
            keys = keys.len(),
            tiles = response.tiles.len(),
            unchanged = response.unchanged,
            revision = response.revision,
            since_day,
            "get_track_tiles answered"
        );
        let current = self.state.with(|s| {
            if s.is_shutdown || generation != s.owner_generation {
                return None;
            }
            s.summary.clone().map(|summary| (summary, s.owner_id.is_none()))
        });
        let Some((summary, is_own)) = current else {
            return Vec::new();
        };
        if Some(summary.revision) != requested_revision && summary.revision != response.revision {
            return Vec::new();
        }

        let cooling_changed = self.tile_store.set_colouring(response.cooling_days, response.heat_walks);
        if response.unchanged {
            self.tile_store.confirm(keys, response.revision);
            if cooling_changed {
                self.tick(Vec::new(), true).await;
            }
            return Vec::new();
        }
        let mut changed: Vec<MapTileKey> = Vec::new();
        let mut returned: HashSet<MapTileKey> = HashSet::new();
        let mut to_persist: Vec<(MapTileKey, RiseMapTracksRepositoryStoredTile)> = Vec::new();
        for tile in &response.tiles {
            let key = tile.key();
            if !keys.contains(&key) {
                continue;
            }
            returned.insert(key);
            let Some(band) = summary.bands.iter().find(|band| band.band == key.band) else {
                continue;
            };
            let Ok(payload) = STANDARD.decode(tile.payload.as_deref().unwrap_or_default()) else {
                tracing::warn!(target: "riseonly::X2068", band = key.band, x = key.x, y = key.y, "track tile payload not base64");
                continue;
            };
            let lines = tile.lines();
            let Some(decoded) =
                MapTrackTileStore::decode(key, band, &payload, response.revision, tile.day(), &lines)
            else {
                tracing::warn!(target: "riseonly::X2068", band = key.band, x = key.x, y = key.y, bytes = payload.len(), "track tile decode failed");
                continue;
            };
            self.tile_store.put(decoded);
            changed.push(key);
            if is_own {
                to_persist.push((
                    key,
                    RiseMapTracksRepositoryStoredTile {
                        revision: response.revision,
                        since_day: tile.day(),
                        payload,
                        part_lines: Some(lines),
                    },
                ));
            }
        }
        for key in keys.iter().copied().filter(|key| !returned.contains(key)) {
            self.tile_store.put(MapDecodedTile {
                key,
                revision: response.revision,
                since_day,
                extent: 1.0,
                lines: Arc::from([]),
            });
            changed.push(key);
        }
        if response.revision != summary.revision {
            self.state.with(|s| {
                s.summary = Some(MapTrackSummary { revision: response.revision, ..summary });
                self.configure_store(s);
                s.last_summary_ms = 0;
            });
            Self::start_summary_refresh(Arc::downgrade(self));
        }
        if cooling_changed {
            self.tick(Vec::new(), true).await;
        }
        for (key, stored) in to_persist {
            if !self.is_owner_generation(generation) {
                return Vec::new();
            }
            self.persist(key, stored).await;
        }
        if !self.is_owner_generation(generation) {
            return Vec::new();
        }
        changed
    }

    async fn stored_tile(&self, key: MapTileKey) -> Option<RiseMapTracksRepositoryStoredTile> {
        let stored = self.engine.value(NAMESPACE.clone(), self.storage_key(key)).await.ok().flatten()?;
        self.state.with(|s| s.version_clock = s.version_clock.max(stored.version));
        serde_json::from_slice(&stored.data).ok()
    }

    /// tile and recency index move in one transaction
    /// the tile evicted from the index is removed in the same one
    async fn persist(&self, key: MapTileKey, tile: RiseMapTracksRepositoryStoredTile) {
        let Ok(data) = serde_json::to_vec(&tile) else {
            return;
        };
        let storage = self.storage_key(key);
        let mut next_index = self.load_index().await;
        next_index.keys.retain(|held| *held != storage);
        next_index.keys.push(storage.clone());
        let overflow = next_index.keys.len().saturating_sub(*PERSISTED_TILES);
        let evicted: Vec<String> = next_index.keys.drain(..overflow).collect();
        let Ok(index_data) = serde_json::to_vec(&next_index) else {
            return;
        };

        let now = Self::now_milliseconds();
        let (tile_version, index_version) = self.state.with(|s| (Self::next_version(s), Self::next_version(s)));
        let mut mutations = vec![
            RiseStoreMutation::SetValue {
                namespace: NAMESPACE.clone(),
                key: storage,
                value: RiseStoredValue::new(data, tile_version, now),
                precondition: RiseStoreVersionPrecondition::Any,
                policy: RiseStoredValueWritePolicy::ServerAuthoritative,
            },
            RiseStoreMutation::SetValue {
                namespace: NAMESPACE.clone(),
                key: self.index_key(),
                value: RiseStoredValue::new(index_data, index_version, now),
                precondition: RiseStoreVersionPrecondition::Any,
                policy: RiseStoredValueWritePolicy::ServerAuthoritative,
            },
        ];
        mutations.extend(evicted.into_iter().map(|key| RiseStoreMutation::RemoveValue {
            namespace: NAMESPACE.clone(),
            key,
            precondition: RiseStoreVersionPrecondition::Any,
        }));
        if self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await.is_err() {
            return;
        }
        self.state.with(|s| s.index = Some(next_index));
    }

    async fn remove_persisted_tiles(&self) {
        let current = self.load_index().await;
        let mut mutations: Vec<RiseStoreMutation> = current
            .keys
            .into_iter()
            .map(|key| RiseStoreMutation::RemoveValue {
                namespace: NAMESPACE.clone(),
                key,
                precondition: RiseStoreVersionPrecondition::Any,
            })
            .collect();
        mutations.push(RiseStoreMutation::RemoveValue {
            namespace: NAMESPACE.clone(),
            key: self.index_key(),
            precondition: RiseStoreVersionPrecondition::Any,
        });
        let _ = self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await;
        self.state.with(|s| s.index = Some(RiseMapTracksRepositoryStoredIndex::default()));
    }

    async fn load_index(&self) -> RiseMapTracksRepositoryStoredIndex {
        if let Some(index) = self.state.with(|s| s.index.clone()) {
            return index;
        }
        let Some(stored) = self.engine.value(NAMESPACE.clone(), self.index_key()).await.ok().flatten() else {
            return RiseMapTracksRepositoryStoredIndex::default();
        };
        let decoded: RiseMapTracksRepositoryStoredIndex = serde_json::from_slice(&stored.data).unwrap_or_default();
        self.state.with(|s| {
            s.version_clock = s.version_clock.max(stored.version);
            s.index = Some(decoded.clone());
        });
        decoded
    }

    fn storage_key(&self, key: MapTileKey) -> String {
        format!("{}t|{}/{}/{}", self.key_prefix, key.band, key.x, key.y)
    }

    fn index_key(&self) -> String {
        format!("{}index", self.key_prefix)
    }

    fn next_version(s: &mut RiseMapTracksRepositoryState) -> i64 {
        s.version_clock = (s.version_clock + 1).max(Self::now_milliseconds());
        s.version_clock
    }

    async fn tick(&self, invalidated: Vec<MapTileKey>, invalidates_all: bool) {
        let tick = self.state.with(|s| {
            s.tick_generation += 1;
            RiseMapTrackTilesTick { generation: s.tick_generation, invalidated, invalidates_all }
        });
        let identity = self.identity.clone();
        Self::main_actor_run(move |cx| {
            RiseMapPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_track_tiles(tick, &identity, cx);
            });
        })
        .await;
    }

    async fn publish(&self) {
        let snapshot = self.state.with(|s| {
            s.revision += 1;
            s.snapshot()
        });
        let identity = self.identity.clone();
        Self::main_actor_run(move |cx| {
            RiseMapPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_tracks(snapshot, &identity, cx);
            });
        })
        .await;
    }

    fn main_actor_run(work: impl FnOnce(&mut App) + Send + 'static) -> BoxFuture<'static, ()> {
        let Some(main) = RiseEngineMainHop::installed() else {
            return async {}.boxed();
        };
        let run = main.run(move |cx: AsyncApp| {
            async move {
                cx.update(work);
                Ok(())
            }
            .boxed_local()
        });
        async move {
            let _ = run.await;
        }
        .boxed()
    }

    fn is_shut_down(&self) -> bool {
        self.state.with(|s| s.is_shutdown)
    }

    fn is_owner_generation(&self, generation: u64) -> bool {
        self.state.with(|s| !s.is_shutdown && generation == s.owner_generation)
    }

    fn is_places_generation(&self, generation: u64) -> bool {
        self.state.with(|s| !s.is_shutdown && generation == s.places_generation)
    }

    fn now_milliseconds() -> i64 {
        RiseClock::now().ms()
    }
}
