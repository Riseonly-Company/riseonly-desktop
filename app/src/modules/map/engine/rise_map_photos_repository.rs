use std::collections::HashSet;
use std::sync::LazyLock;

use futures::FutureExt;
use gpui::{App, AsyncApp};
use rise_canon::{RiseLimits, RiseTimingCanon, rise_request};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::CancellationError;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::map::engine::core::rise_map_resource_repository::RiseMapFailure;
use crate::modules::map::engine::core::rise_map_rpc::RiseMapRPC;
use crate::modules::map::engine::rise_map_engine_models::{RiseMapLoadState, RiseMapMutationOutcome};
use crate::modules::map::engine::rise_map_presentation::{RiseMapPresentation, RiseMapSessionIdentity};
use crate::modules::map::recorder::map_track_codec::MapTrackGeometry;
use crate::modules::map::stores::map_actions::map_photos_types::{MapPhoto, MapPhotoDraft, MapPhotoGrid};
use crate::modules::map::stores::map_actions::map_photos_wire_types::{ProtoMapPhotoExt, ProtoMapPhotoGridExt};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseMapOpenGridSnapshot {
    pub grid_id: String,
    pub photos: Vec<MapPhoto>,
    pub has_more: bool,
    pub load_state: RiseMapLoadState,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseMapPhotosSnapshot {
    pub owner_id: Option<String>,
    pub grids: Vec<MapPhotoGrid>,
    pub open_grid: Option<RiseMapOpenGridSnapshot>,
    pub load_state: RiseMapLoadState,
    pub revision: u64,
}

impl RiseMapPhotosSnapshot {
    pub const EMPTY: RiseMapPhotosSnapshot = RiseMapPhotosSnapshot {
        owner_id: None,
        grids: Vec::new(),
        open_grid: None,
        load_state: RiseMapLoadState::Idle,
        revision: 0,
    };
}

struct RiseMapPhotosRepositoryState {
    owner_id: Option<String>,
    grids: Vec<MapPhotoGrid>,
    load_state: RiseMapLoadState,
    revision: u64,
    last_refresh_ms: i64,
    load_generation: u64,
    is_loading: bool,
    is_shutdown: bool,
    open_grid_id: Option<String>,
    open_photos: Vec<MapPhoto>,
    open_cursor: Option<String>,
    open_has_more: bool,
    open_load_state: RiseMapLoadState,
    open_generation: u64,
    is_loading_open_page: bool,
}

static REFRESH_INTERVAL_MS: LazyLock<i64> = LazyLock::new(|| RiseTimingCanon::freshness_ms("map_track_summary"));
static PAGE_SIZE: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_default_list_limit"));
static GRID_RADIUS_METERS: LazyLock<f64> =
    LazyLock::new(|| RiseLimits::service_value("map_photo_grid_radius_m") as f64);
static DELETE_GRID_MAXIMUM_CALLS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("map_delete_photo_grid_max_calls"));

/// photo grids of whoever's routes are open plus the photos of the grid being looked into
/// map draws grids only
/// a create is optimistic twice
pub struct RiseMapPhotosRepository {
    rpc: RiseMapRPC,
    identity: RiseMapSessionIdentity,
    state: RiseIsolated<RiseMapPhotosRepositoryState>,
}

/// clears a loading flag only while its generation is still current
struct RiseMapPhotosLoadingReset<'a> {
    state: &'a RiseIsolated<RiseMapPhotosRepositoryState>,
    generation: u64,
    open_page: bool,
}

impl Drop for RiseMapPhotosLoadingReset<'_> {
    fn drop(&mut self) {
        let generation = self.generation;
        let open_page = self.open_page;
        self.state.with(|s| {
            if open_page {
                if generation == s.open_generation {
                    s.is_loading_open_page = false;
                }
            } else if generation == s.load_generation {
                s.is_loading = false;
            }
        });
    }
}

type RiseMapPhotosRollbackStep<'a> = Box<dyn FnOnce(&mut RiseMapPhotosRepositoryState) + Send + 'a>;

/// rollback for an optimistic step
/// a dropped call puts the state back and posts the snapshot since drop cant await
struct RiseMapPhotosRollback<'a> {
    repository: &'a RiseMapPhotosRepository,
    rollback: Option<RiseMapPhotosRollbackStep<'a>>,
}

impl<'a> RiseMapPhotosRollback<'a> {
    fn new(
        repository: &'a RiseMapPhotosRepository,
        rollback: impl FnOnce(&mut RiseMapPhotosRepositoryState) + Send + 'a,
    ) -> Self {
        Self { repository, rollback: Some(Box::new(rollback)) }
    }

    fn take(&mut self) -> Option<RiseMapPhotosRollbackStep<'a>> {
        self.rollback.take()
    }
}

impl Drop for RiseMapPhotosRollback<'_> {
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
            RiseMapPhotosRepository::post_snapshot(snapshot, self.repository.identity.clone());
        }
    }
}

enum RiseMapPhotosGridsStep {
    Next(Option<String>, Option<String>),
    Finished(bool),
}

enum RiseMapPhotosOpenPageStep {
    Skip,
    Publish,
    Fetch(String, u64, Option<String>),
}

enum RiseMapPhotosDeleteGridEnd {
    Finished,
    Shutdown,
}

impl RiseMapPhotosRepository {
    pub fn new(rpc: RiseMapRPC, identity: RiseMapSessionIdentity) -> Self {
        Self {
            rpc,
            identity,
            state: RiseIsolated::new(RiseMapPhotosRepositoryState {
                owner_id: None,
                grids: Vec::new(),
                load_state: RiseMapLoadState::Idle,
                revision: 0,
                last_refresh_ms: 0,
                load_generation: 0,
                is_loading: false,
                is_shutdown: false,
                open_grid_id: None,
                open_photos: Vec::new(),
                open_cursor: None,
                open_has_more: false,
                open_load_state: RiseMapLoadState::Idle,
                open_generation: 0,
                is_loading_open_page: false,
            }),
        }
    }

    /// refreshes whoever's grids are open, same as the tracks revalidate
    pub async fn revalidate(&self, force: bool) {
        let owner_id = self.state.with(|s| s.owner_id.clone());
        self.open(owner_id.as_deref(), force).await;
    }

    /// opens the grids of owner_id, none means own
    /// another owner starts from an empty map so grids of two people never mix
    /// a load still running for the previous owner is dropped by its generation
    pub async fn open(&self, requested: Option<&str>, force: bool) {
        let account_id = self.identity.account_id.as_str();
        let next = requested.filter(|id| *id != account_id && !id.is_empty()).map(str::to_owned);
        let switched = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            if next == s.owner_id {
                return Some(false);
            }
            s.load_generation = s.load_generation.wrapping_add(1);
            s.owner_id = next;
            s.grids.clear();
            s.load_state = RiseMapLoadState::Idle;
            s.last_refresh_ms = 0;
            s.is_loading = false;
            s.forget_open_grid();
            Some(true)
        });
        match switched {
            None => return,
            Some(true) => self.publish().await,
            Some(false) => {}
        }

        let now = Self::now_milliseconds();
        let start = self.state.with(|s| {
            if s.is_loading {
                return None;
            }
            let is_stale = now - s.last_refresh_ms >= *REFRESH_INTERVAL_MS;
            if !(force || is_stale || s.load_state != RiseMapLoadState::Loaded) {
                return None;
            }
            s.is_loading = true;
            s.load_generation = s.load_generation.wrapping_add(1);
            let announces = s.grids.is_empty();
            if announces {
                s.load_state = RiseMapLoadState::Loading;
            }
            Some((s.load_generation, announces))
        });
        let Some((generation, announces)) = start else {
            return;
        };
        let _loading = RiseMapPhotosLoadingReset { state: &self.state, generation, open_page: false };
        if announces {
            self.publish().await;
        }

        let mut fresh: Vec<MapPhotoGrid> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut owner_id = self.state.with(|s| s.owner_id.clone());
        loop {
            let page = self.rpc.get_photo_grids(owner_id.as_deref(), *PAGE_SIZE, cursor.as_deref()).await.ok();
            let now = Self::now_milliseconds();
            let step = self.state.with(|s| {
                let is_current = !s.is_shutdown && generation == s.load_generation;
                let Some(page) = page.filter(|_| is_current) else {
                    if is_current && (s.owner_id.is_some() || s.load_state != RiseMapLoadState::Loaded) {
                        if s.owner_id.is_some() {
                            s.grids.clear();
                            s.forget_open_grid();
                        }
                        s.load_state = RiseMapLoadState::Failed;
                        return RiseMapPhotosGridsStep::Finished(true);
                    }
                    return RiseMapPhotosGridsStep::Finished(false);
                };
                let known: HashSet<String> = fresh.iter().map(|grid| grid.id.clone()).collect();
                fresh.extend(
                    page.grids
                        .iter()
                        .filter_map(ProtoMapPhotoGridExt::model)
                        .filter(|grid| !known.contains(&grid.id)),
                );
                let next_cursor =
                    if page.has_more { page.next_cursor.filter(|next| Some(next) != cursor.as_ref()) } else { None };
                if next_cursor.is_some() {
                    return RiseMapPhotosGridsStep::Next(s.owner_id.clone(), next_cursor);
                }
                let stored: HashSet<String> = fresh.iter().map(|grid| grid.id.clone()).collect();
                let mut grids: Vec<MapPhotoGrid> = s
                    .grids
                    .iter()
                    .filter(|grid| grid.is_pending && !stored.contains(&grid.id))
                    .cloned()
                    .collect();
                grids.append(&mut fresh);
                s.grids = grids;
                s.last_refresh_ms = now;
                s.load_state = RiseMapLoadState::Loaded;
                RiseMapPhotosGridsStep::Finished(true)
            });
            match step {
                RiseMapPhotosGridsStep::Next(next_owner_id, next_cursor) => {
                    owner_id = next_owner_id;
                    cursor = next_cursor;
                }
                RiseMapPhotosGridsStep::Finished(publishes) => {
                    if publishes {
                        self.publish().await;
                    }
                    return;
                }
            }
        }
    }

    pub async fn focus_grid(&self, grid: &MapPhotoGrid) {
        self.open(Some(grid.owner_id.as_str()), true).await;
        let account_id = self.identity.account_id.as_str();
        let proceeds = self.state.with(|s| {
            !s.is_shutdown
                && s.load_state == RiseMapLoadState::Loaded
                && s.owner_id.as_deref().unwrap_or(account_id) == grid.owner_id
                && s.grids.iter().any(|known| known.id == grid.id)
        });
        if !proceeds {
            return;
        }
        self.open_grid(Some(grid.id.as_str())).await;
    }

    /// looks into one grid, first page newest first
    /// none closes it
    pub async fn open_grid(&self, id: Option<&str>) {
        let step = self.state.with(|s| {
            if s.is_shutdown || id == s.open_grid_id.as_deref() {
                return None;
            }
            s.forget_open_grid();
            s.open_grid_id = id.map(str::to_owned);
            if id.is_none() {
                return Some(false);
            }
            s.open_load_state = RiseMapLoadState::Loading;
            Some(true)
        });
        let Some(loads) = step else {
            return;
        };
        self.publish().await;
        if loads {
            self.load_open_page().await;
        }
    }

    pub async fn load_more_open_photos(&self) {
        let proceeds = self.state.with(|s| !s.is_shutdown && s.open_has_more);
        if !proceeds {
            return;
        }
        self.load_open_page().await;
    }

    async fn load_open_page(&self) {
        let step = self.state.with(|s| {
            let Some(grid_id) = s.open_grid_id.clone() else {
                return RiseMapPhotosOpenPageStep::Skip;
            };
            if s.is_loading_open_page {
                return RiseMapPhotosOpenPageStep::Skip;
            }
            if s.grids.iter().find(|grid| grid.id == grid_id).is_some_and(|grid| grid.is_pending) {
                s.open_load_state = RiseMapLoadState::Loaded;
                return RiseMapPhotosOpenPageStep::Publish;
            }
            s.is_loading_open_page = true;
            RiseMapPhotosOpenPageStep::Fetch(grid_id, s.open_generation, s.open_cursor.clone())
        });
        let (grid_id, generation, cursor) = match step {
            RiseMapPhotosOpenPageStep::Skip => return,
            RiseMapPhotosOpenPageStep::Publish => {
                self.publish().await;
                return;
            }
            RiseMapPhotosOpenPageStep::Fetch(grid_id, generation, cursor) => (grid_id, generation, cursor),
        };
        let _loading = RiseMapPhotosLoadingReset { state: &self.state, generation, open_page: true };

        let page = self.rpc.get_grid_photos(&grid_id, *PAGE_SIZE, cursor.as_deref()).await.ok();
        let publishes = self.state.with(|s| {
            let is_current = !s.is_shutdown && generation == s.open_generation;
            let Some(page) = page.filter(|_| is_current) else {
                if is_current && (s.owner_id.is_some() || s.open_load_state != RiseMapLoadState::Loaded) {
                    if s.owner_id.is_some() && cursor.is_none() {
                        s.open_photos.clear();
                        s.open_cursor = None;
                        s.open_has_more = false;
                    }
                    s.open_load_state = RiseMapLoadState::Failed;
                    return true;
                }
                return false;
            };
            let known: HashSet<String> = s.open_photos.iter().map(|photo| photo.id.clone()).collect();
            let added: Vec<MapPhoto> = page
                .photos
                .iter()
                .filter_map(ProtoMapPhotoExt::model)
                .filter(|photo| !known.contains(&photo.id))
                .collect();
            s.open_photos.extend(added);
            s.open_has_more = page.has_more && page.next_cursor.is_some();
            s.open_cursor = page.next_cursor;
            s.open_load_state = RiseMapLoadState::Loaded;
            true
        });
        if publishes {
            self.publish().await;
        }
    }

    pub async fn update_grid(&self, body: rise_request::map::UpdatePhotoGrid) -> RiseMapMutationOutcome {
        let generation = self.state.with(|s| {
            if s.is_shutdown || s.owner_id.is_some() {
                return None;
            }
            Some(s.load_generation)
        });
        let Some(generation) = generation else {
            return RiseMapMutationOutcome::Failed(None);
        };
        let Ok(updated) = self.rpc.update_photo_grid(body).await else {
            return RiseMapMutationOutcome::Failed(None);
        };
        let applied = self.state.with(|s| {
            if s.is_shutdown || generation != s.load_generation {
                return false;
            }
            if let Some(index) = s.grids.iter().position(|grid| grid.id == updated.id) {
                s.grids[index] = updated;
            }
            true
        });
        if !applied {
            return RiseMapMutationOutcome::Failed(None);
        }
        self.publish().await;
        RiseMapMutationOutcome::Done
    }

    pub fn grid(&self, id: &str) -> Option<MapPhotoGrid> {
        self.state.with(|s| s.grids.iter().find(|grid| grid.id == id).cloned())
    }

    pub fn grids_snapshot(&self) -> Vec<MapPhotoGrid> {
        self.state.with(|s| s.grids.clone())
    }

    pub async fn add(&self, draft: MapPhotoDraft) -> RiseMapMutationOutcome {
        let now = Self::now_milliseconds();
        let account_id = self.identity.account_id.clone();
        let prepared = self.state.with(|s| {
            if s.is_shutdown || s.owner_id.is_some() {
                return None;
            }
            let expected = s.nearest_own_grid(draft.latitude, draft.longitude);
            let grid_id = expected.as_ref().map_or_else(|| draft.grid_id.clone(), |grid| grid.id.clone());
            let pending = MapPhoto {
                id: draft.id.clone(),
                owner_id: account_id.clone(),
                url: draft.url.clone(),
                thumbnail_url: draft.thumbnail_url.clone(),
                latitude: draft.latitude,
                longitude: draft.longitude,
                width: draft.width,
                height: draft.height,
                taken_at: draft.taken_at,
                created_at: now,
                caption: draft.caption.clone(),
                grid_id: grid_id.clone(),
                media_kind: draft.media_kind,
                duration_ms: draft.duration_ms,
                is_pending: true,
            };

            if let Some(index) = s.grids.iter().position(|grid| grid.id == grid_id) {
                let grid = &mut s.grids[index];
                grid.photo_count += 1;
                if grid.cover_photo_id.is_none() {
                    grid.cover_url = pending.display_url().to_owned();
                }
                grid.updated_at = now;
            } else {
                let mut grid = MapPhotoGrid::new(
                    grid_id.clone(),
                    account_id.clone(),
                    draft.latitude,
                    draft.longitude,
                    1,
                    pending.display_url().to_owned(),
                    now,
                    now,
                );
                grid.is_pending = true;
                s.grids.insert(0, grid);
            }
            if s.open_grid_id.as_deref() == Some(grid_id.as_str()) {
                s.open_photos.retain(|photo| photo.id != draft.id);
                s.open_photos.insert(0, pending);
            }
            Some((grid_id, expected))
        });
        let Some((grid_id, previous_grid)) = prepared else {
            return RiseMapMutationOutcome::Failed(None);
        };
        let rollback_grid_id = grid_id.clone();
        let rollback_grid = previous_grid.clone();
        let draft_id = draft.id.clone();
        let mut rollback = RiseMapPhotosRollback::new(self, move |s| {
            s.roll_back_grid(&rollback_grid_id, rollback_grid);
            s.open_photos.retain(|photo| !(photo.id == draft_id && photo.is_pending));
        });
        self.publish().await;

        match self.rpc.create_photo(&draft).await {
            Ok(placement) => {
                rollback.take();
                let applied = self.state.with(|s| {
                    if s.is_shutdown {
                        return false;
                    }
                    if let Some(placement) = placement {
                        if placement.grid.id != grid_id {
                            s.roll_back_grid(&grid_id, previous_grid);
                            let was_open = s.open_grid_id.as_deref() == Some(grid_id.as_str());
                            s.open_photos.retain(|photo| !(photo.id == draft.id && was_open));
                        }
                        let placed_grid_id = placement.grid.id.clone();
                        s.upsert(placement.grid);
                        if s.open_grid_id.as_deref() == Some(placed_grid_id.as_str()) {
                            let photo_id = placement.photo.id.clone();
                            s.open_photos.retain(|photo| photo.id != photo_id);
                            s.open_photos.insert(0, placement.photo);
                        }
                    }
                    true
                });
                if !applied {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.publish().await;
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

    pub async fn delete(&self, photo_id: &str) -> RiseMapMutationOutcome {
        let prepared = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            let index = s.open_photos.iter().position(|photo| photo.id == photo_id)?;
            let removed = s.open_photos.remove(index);
            let previous_grid = s.grids.iter().find(|grid| grid.id == removed.grid_id).cloned();
            if let Some(grid_index) = s.grids.iter().position(|grid| grid.id == removed.grid_id) {
                let cover = s.open_photos.first().map(|photo| photo.display_url().to_owned());
                let grid = &mut s.grids[grid_index];
                grid.photo_count = (grid.photo_count - 1).max(0);
                if let Some(cover) = cover {
                    grid.cover_url = cover;
                }
                if grid.photo_count == 0 {
                    s.grids.remove(grid_index);
                }
            }
            Some((index, removed, previous_grid))
        });
        let Some((index, removed, previous_grid)) = prepared else {
            return self.delete_unopened(photo_id).await;
        };
        let owned_photo_id = photo_id.to_owned();
        let mut rollback = RiseMapPhotosRollback::new(self, move |s| {
            if let Some(previous_grid) = previous_grid {
                s.upsert(previous_grid);
            }
            if s.open_grid_id.as_deref() == Some(removed.grid_id.as_str())
                && !s.open_photos.iter().any(|photo| photo.id == owned_photo_id)
            {
                let at = index.min(s.open_photos.len());
                s.open_photos.insert(at, removed);
            }
        });
        self.publish().await;

        match self.rpc.delete_photo(photo_id).await {
            Ok(removal) => {
                rollback.take();
                let applied = self.state.with(|s| {
                    if s.is_shutdown {
                        return false;
                    }
                    if let Some(grid) = removal.grid {
                        s.upsert(grid);
                    }
                    if let Some(gone) = removal.removed_grid_id {
                        s.grids.retain(|grid| grid.id != gone);
                        if s.open_grid_id.as_deref() == Some(gone.as_str()) {
                            s.forget_open_grid();
                        }
                    }
                    true
                });
                if !applied {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.publish().await;
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

    async fn delete_unopened(&self, photo_id: &str) -> RiseMapMutationOutcome {
        if self.state.with(|s| s.is_shutdown) {
            return RiseMapMutationOutcome::Failed(None);
        }
        tracing::warn!(target: "riseonly::X1154", photo_id, "map photo delete outside the open grid");
        match self.rpc.delete_photo(photo_id).await {
            Ok(removal) => {
                let applied = self.state.with(|s| {
                    if s.is_shutdown {
                        return false;
                    }
                    s.open_photos.retain(|photo| photo.id != photo_id);
                    if let Some(grid) = removal.grid {
                        s.upsert(grid);
                    }
                    if let Some(gone) = removal.removed_grid_id {
                        s.grids.retain(|grid| grid.id != gone);
                        if s.open_grid_id.as_deref() == Some(gone.as_str()) {
                            s.forget_open_grid();
                        }
                    }
                    true
                });
                if !applied {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.publish().await;
                RiseMapMutationOutcome::Done
            }
            Err(error) => RiseMapMutationOutcome::Failed(RiseMapFailure::text(&error)),
        }
    }

    pub async fn delete_grid(&self, id: &str) -> RiseMapMutationOutcome {
        let prepared = self.state.with(|s| {
            if s.is_shutdown || s.owner_id.is_some() {
                return None;
            }
            let index = s.grids.iter().position(|grid| grid.id == id);
            let removed = index.map(|index| s.grids.remove(index));
            if s.open_grid_id.as_deref() == Some(id) {
                s.forget_open_grid();
            }
            Some((index, removed))
        });
        let Some((index, removed)) = prepared else {
            return RiseMapMutationOutcome::Failed(None);
        };
        let owned_id = id.to_owned();
        let mut rollback = RiseMapPhotosRollback::new(self, move |s| {
            if let Some(removed) = removed
                && !s.grids.iter().any(|grid| grid.id == owned_id)
            {
                let at = index.unwrap_or(0).min(s.grids.len());
                s.grids.insert(at, removed);
            }
        });
        self.publish().await;

        let deleted: anyhow::Result<RiseMapPhotosDeleteGridEnd> = async {
            let mut calls: i64 = 0;
            let mut has_more = true;
            while has_more && calls < *DELETE_GRID_MAXIMUM_CALLS {
                has_more = self.rpc.delete_photo_grid(id).await?;
                calls += 1;
                if self.is_shutdown() {
                    return Ok(RiseMapPhotosDeleteGridEnd::Shutdown);
                }
            }
            if has_more {
                return Err(anyhow::Error::new(CancellationError));
            }
            Ok(RiseMapPhotosDeleteGridEnd::Finished)
        }
        .await;
        match deleted {
            Ok(RiseMapPhotosDeleteGridEnd::Finished) => {
                rollback.take();
                RiseMapMutationOutcome::Done
            }
            Ok(RiseMapPhotosDeleteGridEnd::Shutdown) => {
                rollback.take();
                RiseMapMutationOutcome::Failed(None)
            }
            Err(error) => {
                let restore = rollback.take();
                if !self.apply_rollback(restore) {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.publish().await;
                let owner_id = self.state.with(|s| {
                    s.last_refresh_ms = 0;
                    s.owner_id.clone()
                });
                self.open(owner_id.as_deref(), true).await;
                RiseMapMutationOutcome::Failed(RiseMapFailure::text(&error))
            }
        }
    }

    pub async fn reset(&self) {
        let reset = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.load_generation = s.load_generation.wrapping_add(1);
            s.owner_id = None;
            s.grids.clear();
            s.load_state = RiseMapLoadState::Idle;
            s.last_refresh_ms = 0;
            s.forget_open_grid();
            true
        });
        if reset {
            self.publish().await;
        }
    }

    pub fn shutdown(&self) {
        self.state.with(|s| s.is_shutdown = true);
    }

    fn is_shutdown(&self) -> bool {
        self.state.with(|s| s.is_shutdown)
    }

    /// undoes an optimistic step unless the repository shut down meanwhile
    /// false means it did and the caller returns at once
    fn apply_rollback(&self, rollback: Option<RiseMapPhotosRollbackStep<'_>>) -> bool {
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

    async fn publish(&self) {
        let snapshot = self.state.with(RiseMapPhotosRepositoryState::next_snapshot);
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

    fn post_snapshot(snapshot: RiseMapPhotosSnapshot, identity: RiseMapSessionIdentity) {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        hop.post(move |cx: &mut App| Self::publish_on_main(snapshot, &identity, cx));
    }

    fn publish_on_main(snapshot: RiseMapPhotosSnapshot, identity: &RiseMapSessionIdentity, cx: &mut App) {
        RiseMapPresentation::shared(cx).update(cx, |presentation, cx| {
            presentation.publish_photos(snapshot, identity, cx);
        });
    }

    fn now_milliseconds() -> i64 {
        RiseClock::now().ms()
    }
}

impl RiseMapPhotosRepositoryState {
    fn forget_open_grid(&mut self) {
        self.open_generation = self.open_generation.wrapping_add(1);
        self.open_grid_id = None;
        self.open_photos.clear();
        self.open_cursor = None;
        self.open_has_more = false;
        self.open_load_state = RiseMapLoadState::Idle;
        self.is_loading_open_page = false;
    }

    fn upsert(&mut self, grid: MapPhotoGrid) {
        if let Some(index) = self.grids.iter().position(|known| known.id == grid.id) {
            self.grids[index] = grid;
        } else {
            self.grids.insert(0, grid);
        }
    }

    fn roll_back_grid(&mut self, id: &str, previous: Option<MapPhotoGrid>) {
        if let Some(previous) = previous {
            self.upsert(previous);
        } else {
            self.grids.retain(|grid| !(grid.id == id && grid.is_pending));
        }
    }

    fn nearest_own_grid(&self, latitude: f64, longitude: f64) -> Option<MapPhotoGrid> {
        self.grids
            .iter()
            .map(|grid| {
                let distance = MapTrackGeometry::haversine_meters(latitude, longitude, grid.latitude, grid.longitude);
                (grid, distance)
            })
            .filter(|(_, distance)| *distance <= *GRID_RADIUS_METERS)
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .map(|(grid, _)| grid.clone())
    }

    fn next_snapshot(&mut self) -> RiseMapPhotosSnapshot {
        self.revision = self.revision.wrapping_add(1);
        let cover_id = self
            .open_grid_id
            .as_ref()
            .and_then(|open_id| self.grids.iter().find(|grid| &grid.id == open_id))
            .and_then(|grid| grid.cover_photo_id.clone());
        let ordered_photos: Vec<MapPhoto> = match cover_id {
            Some(cover_id) => self
                .open_photos
                .iter()
                .filter(|photo| photo.id == cover_id)
                .chain(self.open_photos.iter().filter(|photo| photo.id != cover_id))
                .cloned()
                .collect(),
            None => self.open_photos.clone(),
        };
        RiseMapPhotosSnapshot {
            owner_id: self.owner_id.clone(),
            grids: self.grids.clone(),
            open_grid: self.open_grid_id.as_ref().map(|grid_id| RiseMapOpenGridSnapshot {
                grid_id: grid_id.clone(),
                photos: ordered_photos,
                has_more: self.open_has_more,
                load_state: self.open_load_state,
            }),
            load_state: self.load_state,
            revision: self.revision,
        }
    }
}
