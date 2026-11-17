use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use futures::FutureExt;
use gpui::{App, AsyncApp, Context, Entity};
use rise_canon::RiseLimits;
use rise_canon::rise_response;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_i18n::tr;
use rise_media::map_view::LatLng;
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::map::engine::core::map_track_heat::MapHeatStretch;
use crate::modules::map::engine::core::rise_map_rpc::RiseMapRPC;
use crate::modules::map::engine::map_journey_heat::MapJourneyHeat;
use crate::modules::map::engine::rise_map_engine_models::RiseMapError;
use crate::modules::map::engine::rise_map_presentation::RiseMapSessionIdentity;
use crate::modules::map::stores::map_actions::map_photos_types::{MapMediaKind, MapPhoto, MapPhotoGrid};

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct MapJourneyPoint {
    pub lat: f64,
    pub lng: f64,
}

impl Eq for MapJourneyPoint {}

impl Hash for MapJourneyPoint {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.lat.to_bits().hash(state);
        self.lng.to_bits().hash(state);
    }
}

impl MapJourneyPoint {
    pub fn coordinate(&self) -> LatLng {
        LatLng::new(self.lat, self.lng)
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct MapJourneySnapshotPhoto {
    pub id: String,
    pub url: String,
    pub thumbnail_url: String,
    pub width: i64,
    pub height: i64,
    pub media_kind: String,
    pub duration_ms: i64,
    pub taken_at: i64,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct MapJourneySnapshotGrid {
    pub id: String,
    pub lat: f64,
    pub lng: f64,
    pub photos: Vec<MapJourneySnapshotPhoto>,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct MapJourneySnapshot {
    pub id: String,
    pub owner_id: String,
    pub from_ms: i64,
    pub until_ms: i64,
    pub polygon: Vec<MapJourneyPoint>,
    pub lines: Vec<Vec<MapJourneyPoint>>,
    pub grids: Vec<MapJourneySnapshotGrid>,
    #[serde(default)]
    pub show_heat: Option<bool>,
    #[serde(default)]
    pub heat_walks: Option<i64>,
}

impl MapJourneySnapshot {
    pub fn map_grids(&self) -> Vec<MapPhotoGrid> {
        self.grids
            .iter()
            .map(|grid| {
                MapPhotoGrid::new(
                    grid.id.clone(),
                    self.owner_id.clone(),
                    grid.lat,
                    grid.lng,
                    grid.photos.len() as i64,
                    grid.photos
                        .first()
                        .map(|photo| {
                            if photo.thumbnail_url.is_empty() { photo.url.clone() } else { photo.thumbnail_url.clone() }
                        })
                        .unwrap_or_default(),
                    self.from_ms,
                    self.until_ms,
                )
            })
            .collect()
    }

    pub fn photos(&self, grid: &MapPhotoGrid) -> Vec<MapPhoto> {
        self.grids
            .iter()
            .find(|candidate| candidate.id == grid.id)
            .map(|candidate| candidate.photos.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|photo| MapPhoto {
                grid_id: grid.id.clone(),
                media_kind: MapMediaKind::from_wire(&photo.media_kind),
                duration_ms: photo.duration_ms,
                ..MapPhoto::new(
                    photo.id.clone(),
                    self.owner_id.clone(),
                    photo.url.clone(),
                    photo.thumbnail_url.clone(),
                    grid.latitude,
                    grid.longitude,
                    photo.width,
                    photo.height,
                    photo.taken_at,
                    photo.taken_at,
                    String::new(),
                )
            })
            .collect()
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct MapJourneySelection {
    pub id: String,
    pub polygon: Vec<MapJourneyPoint>,
    pub grid_ids: Vec<String>,
    pub from_ms: i64,
    pub until_ms: i64,
}

#[derive(Default)]
pub struct RiseMapJourneyPresentation {
    pub snapshot: Option<MapJourneySnapshot>,
    pub loading: bool,
    pub error: Option<String>,
    pub post_id: Option<i64>,
    pub scope: Option<Uuid>,
    pub repository_id: Option<Uuid>,
    pub request_generation: u64,
    pub grids: Vec<MapPhotoGrid>,
    pub photos_by_grid: HashMap<String, Vec<MapPhoto>>,
    pub heated_lines: Vec<MapHeatStretch>,
}

impl RiseMapJourneyPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_: &mut Context<Self>| Self::default())
    }

    pub fn clear(&mut self) {
        self.snapshot = None;
        self.loading = false;
        self.error = None;
        self.post_id = None;
        self.scope = None;
        self.grids = Vec::new();
        self.photos_by_grid = HashMap::new();
        self.heated_lines = Vec::new();
    }
}

struct RiseMapJourneysRepositoryState {
    active_scope: Option<Uuid>,
    generation: u64,
    is_shutdown: bool,
}

static BYTE_LIMIT: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_journey_max_snapshot_bytes"));

pub struct RiseMapJourneysRepository {
    rpc: RiseMapRPC,
    identity: RiseMapSessionIdentity,
    repository_id: Uuid,
    state: RiseIsolated<RiseMapJourneysRepositoryState>,
}

impl RiseMapJourneysRepository {
    fn byte_limit() -> i64 {
        *BYTE_LIMIT
    }

    pub fn new(rpc: RiseMapRPC, identity: RiseMapSessionIdentity) -> Self {
        Self {
            rpc,
            identity,
            repository_id: Uuid::new_v4(),
            state: RiseIsolated::new(RiseMapJourneysRepositoryState {
                active_scope: None,
                generation: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn prepare(&self, selection: &MapJourneySelection) -> anyhow::Result<MapJourneySnapshot> {
        let (is_shutdown, epoch) = self.state.with(|s| (s.is_shutdown, s.generation));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        let wire = self.rpc.prepare_journey(selection).await?;
        let snapshot = Self::decode(&wire)?;
        let (is_shutdown, generation) = self.state.with(|s| (s.is_shutdown, s.generation));
        if is_shutdown || generation != epoch || !self.current().await {
            return Err(CancellationError.into());
        }
        Ok(snapshot)
    }

    pub async fn open(&self, post_id: i64, scope: Uuid) {
        if self.state.with(|s| s.is_shutdown) || !self.current().await {
            return;
        }
        let epoch = self.state.with(|s| {
            s.generation = s.generation.wrapping_add(1);
            s.active_scope = Some(scope);
            s.generation
        });
        let identity = self.identity.clone();
        let repository_id = self.repository_id;
        Self::on_main(move |cx| {
            if !SaiAccountCacheScope::is_current_account_epoch(
                Some(&identity.account_id),
                Some(&identity.account_epoch),
            ) {
                return;
            }
            RiseMapJourneyPresentation::shared(cx).update(cx, |presentation, cx| {
                if presentation.repository_id == Some(repository_id) && presentation.request_generation > epoch {
                    return;
                }
                presentation.clear();
                presentation.request_generation = epoch;
                presentation.scope = Some(scope);
                presentation.repository_id = Some(repository_id);
                presentation.post_id = Some(post_id);
                presentation.loading = true;
                cx.notify();
            });
        })
        .await;
        let loaded = async {
            let wire = self.rpc.get_journey(post_id).await?;
            let snapshot = Self::decode(&wire)?;
            let heated_lines = MapJourneyHeat::prepare(&snapshot);
            let grids = snapshot.map_grids();
            let photos: HashMap<String, Vec<MapPhoto>> =
                grids.iter().map(|grid| (grid.id.clone(), snapshot.photos(grid))).collect();
            anyhow::Ok((snapshot, heated_lines, grids, photos))
        }
        .await;
        let (is_shutdown, generation) = self.state.with(|s| (s.is_shutdown, s.generation));
        if is_shutdown || epoch != generation || !self.current().await {
            return;
        }
        let identity = self.identity.clone();
        Self::on_main(move |cx| {
            if !SaiAccountCacheScope::is_current_account_epoch(
                Some(&identity.account_id),
                Some(&identity.account_epoch),
            ) {
                return;
            }
            RiseMapJourneyPresentation::shared(cx).update(cx, |presentation, cx| {
                if presentation.scope != Some(scope) || presentation.request_generation != epoch {
                    return;
                }
                match loaded {
                    Ok((snapshot, heated_lines, grids, photos)) => {
                        presentation.heated_lines = heated_lines;
                        presentation.grids = grids;
                        presentation.photos_by_grid = photos;
                        presentation.snapshot = Some(snapshot);
                        presentation.loading = false;
                    }
                    Err(error) => {
                        tracing::warn!(target: "riseonly::X1539", "journey open failed post={post_id} error={error:#}");
                        presentation.loading = false;
                        presentation.error = Some(tr("map_journey_unavailable"));
                    }
                }
                cx.notify();
            });
        })
        .await;
    }

    pub async fn close(&self, scope: Option<Uuid>) {
        let Some(epoch) = self.state.with(|s| {
            if scope.is_some() && scope != s.active_scope {
                return None;
            }
            s.generation = s.generation.wrapping_add(1);
            s.active_scope = None;
            Some(s.generation)
        }) else {
            return;
        };
        let repository_id = self.repository_id;
        Self::on_main(move |cx| {
            RiseMapJourneyPresentation::shared(cx).update(cx, |presentation, cx| {
                if presentation.repository_id.is_some_and(|owner| owner != repository_id)
                    || (scope.is_some() && presentation.scope.is_some() && presentation.scope != scope)
                {
                    return;
                }
                presentation.clear();
                presentation.repository_id = Some(repository_id);
                presentation.request_generation = epoch;
                cx.notify();
            });
        })
        .await;
    }

    pub async fn revoke(&self, id: &str) -> anyhow::Result<()> {
        self.rpc.revoke_journey(id).await?;
        self.close(None).await;
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| s.is_shutdown = true);
        self.close(None).await;
    }

    async fn current(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(
            Some(&self.identity.account_id),
            Some(&self.identity.account_epoch),
        )
    }

    /// runs work on the main thread, no-op before the main thread installed its hop
    async fn on_main(work: impl FnOnce(&mut App) + Send + 'static) {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = hop
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(work);
                    Ok::<_, anyhow::Error>(())
                }
                .boxed_local()
            })
            .await;
    }

    fn decode(wire: &rise_response::map::GetJourney) -> anyhow::Result<MapJourneySnapshot> {
        let data = wire.snapshot_json.as_bytes();
        if data.len() as i64 > Self::byte_limit() {
            return Err(RiseMapError::StaleAccount.into());
        }
        Ok(serde_json::from_slice::<MapJourneySnapshot>(data)?)
    }
}
