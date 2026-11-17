use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::FutureExt;
use gpui::AsyncApp;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use serde::{Deserialize, Serialize};

use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::map::engine::core::rise_map_resource_repository::{
    RiseMapResourceRepository, RiseMapResourceRepositoryResource,
};
use crate::modules::map::engine::rise_map_presentation::{RiseMapPresentation, RiseMapSessionIdentity};
use crate::modules::map::shared::map_gallery_scout::{MapGalleryDraft, MapGalleryScout};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseMapDraftsSnapshot {
    pub drafts: Vec<MapGalleryDraft>,
    pub publishing: HashSet<String>,
    pub revision: u64,
}

impl RiseMapDraftsSnapshot {
    pub fn empty() -> Self {
        Self { drafts: Vec::new(), publishing: HashSet::new(), revision: 0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseMapDraftsRepositoryStored {
    cursor_ms: i64,
    drafts: Vec<MapGalleryDraft>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reconciliation_start_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dismissed: Option<HashMap<String, i64>>,
}

impl RiseMapDraftsRepositoryStored {
    fn new(cursor_ms: i64, drafts: Vec<MapGalleryDraft>) -> Self {
        Self { cursor_ms, drafts, reconciliation_start_ms: None, dismissed: None }
    }
}

struct RiseMapDraftsRepositoryState {
    stored: Option<RiseMapDraftsRepositoryStored>,
    publishing: HashSet<String>,
    revision: u64,
    scan_again: bool,
    scan_generation: u64,
    is_scanning: bool,
    is_shutdown: bool,
}

struct RiseMapDraftsScanningGuard<'a> {
    state: &'a RiseIsolated<RiseMapDraftsRepositoryState>,
}

impl Drop for RiseMapDraftsScanningGuard<'_> {
    fn drop(&mut self) {
        self.state.with(|s| s.is_scanning = false);
    }
}

/// gallery shots waiting to be sorted into grids
/// device state only, a draft is a gallery id plus place and time the gallery keeps
/// nothing is uploaded until its picked
pub struct RiseMapDraftsRepository {
    resources: Arc<RiseMapResourceRepository>,
    identity: RiseMapSessionIdentity,
    state: RiseIsolated<RiseMapDraftsRepositoryState>,
}

impl RiseMapDraftsRepository {
    pub fn new(resources: Arc<RiseMapResourceRepository>, identity: RiseMapSessionIdentity) -> Self {
        Self {
            resources,
            identity,
            state: RiseIsolated::new(RiseMapDraftsRepositoryState {
                stored: None,
                publishing: HashSet::new(),
                revision: 0,
                scan_again: false,
                scan_generation: 0,
                is_scanning: false,
                is_shutdown: false,
            }),
        }
    }

    pub async fn scan(&self, joined_at_ms: i64) {
        let Some(generation) = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            if s.is_scanning {
                s.scan_again = true;
                return None;
            }
            s.is_scanning = true;
            Some(s.scan_generation)
        }) else {
            return;
        };
        let _scanning = RiseMapDraftsScanningGuard { state: &self.state };
        let maximum_drafts = usize::try_from(MapGalleryScout::maximum_drafts()).unwrap_or(0);
        loop {
            self.state.with(|s| s.scan_again = false);
            let mut state = self.loaded().await;
            if self.state.with(|s| s.is_shutdown || generation != s.scan_generation) {
                return;
            }
            if state.reconciliation_start_ms.is_none() {
                state.reconciliation_start_ms = Some(joined_at_ms.max(state.cursor_ms));
                let remembered = state.clone();
                self.state.with(|s| s.stored = Some(remembered));
            }
            self.publish().await;
            let now_ms = Self::now_milliseconds();
            let since_ms = joined_at_ms.max(state.reconciliation_start_ms.unwrap_or(joined_at_ms));
            let excluded: HashSet<String> =
                state.dismissed.as_ref().map(|dismissed| dismissed.keys().cloned().collect()).unwrap_or_default();
            let found = RiseAccountRuntime::current()
                .spawn_blocking(move || MapGalleryScout::scan(since_ms, now_ms, &excluded))
                .await;
            if self.state.with(|s| s.is_shutdown || generation != s.scan_generation) {
                return;
            }
            let Ok(found) = found else {
                return;
            };
            if !MapGalleryScout::has_full_access() {
                return;
            }

            if let Some(latest) = self.state.with(|s| s.stored.clone()) {
                state = latest;
            }
            let dismissed = state.dismissed.clone().unwrap_or_default();
            let reconciliation_start_ms = state.reconciliation_start_ms.unwrap_or(0);
            let mut merged: HashMap<String, MapGalleryDraft> =
                state.drafts.iter().map(|draft| (draft.id.clone(), draft.clone())).collect();
            for draft in found {
                if !dismissed.contains_key(&draft.id) && draft.taken_at > reconciliation_start_ms {
                    merged.insert(draft.id.clone(), draft);
                }
            }
            let mut drafts: Vec<MapGalleryDraft> = merged.into_values().collect();
            drafts.sort_by(|lhs, rhs| {
                if lhs.taken_at == rhs.taken_at { lhs.id.cmp(&rhs.id) } else { rhs.taken_at.cmp(&lhs.taken_at) }
            });
            drafts.truncate(maximum_drafts);
            state.drafts = drafts;
            let lookback_floor = now_ms - MapGalleryScout::lookback_ms();
            state.dismissed =
                Some(dismissed.into_iter().filter(|(_, taken_at)| *taken_at >= lookback_floor).collect());
            state.cursor_ms = now_ms;
            self.save(state).await;
            if !self.state.with(|s| s.scan_again && !s.is_shutdown) {
                break;
            }
        }
    }

    pub async fn mark_publishing(&self, id: &str, is_publishing: bool) {
        let is_running = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            if is_publishing {
                s.publishing.insert(id.to_owned());
            } else {
                s.publishing.remove(id);
            }
            true
        });
        if !is_running {
            return;
        }
        self.publish().await;
    }

    /// drops drafts published into a grid or cleared by the reader, the gallery is never touched
    pub async fn remove(&self, ids: HashSet<String>) {
        if self.state.with(|s| s.is_shutdown) || ids.is_empty() {
            return;
        }
        let mut state = self.loaded().await;
        let removed: Vec<MapGalleryDraft> =
            state.drafts.iter().filter(|draft| ids.contains(&draft.id)).cloned().collect();
        self.remember_dismissed(&removed, &mut state);
        state.drafts.retain(|draft| !ids.contains(&draft.id));
        self.state.with(|s| s.publishing.retain(|id| !ids.contains(id)));
        self.save(state).await;
    }

    pub async fn remove_all(&self) {
        if self.state.with(|s| s.is_shutdown) {
            return;
        }
        let mut state = self.loaded().await;
        let publishing = self.state.with(|s| s.publishing.clone());
        let removed: Vec<MapGalleryDraft> =
            state.drafts.iter().filter(|draft| !publishing.contains(&draft.id)).cloned().collect();
        self.remember_dismissed(&removed, &mut state);
        state.drafts.retain(|draft| publishing.contains(&draft.id));
        self.save(state).await;
    }

    pub async fn reset(&self) {
        let is_running = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.scan_generation = s.scan_generation.wrapping_add(1);
            s.scan_again = false;
            s.publishing = HashSet::new();
            true
        });
        if !is_running {
            return;
        }
        self.save(RiseMapDraftsRepositoryStored::new(0, Vec::new())).await;
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| {
            s.scan_generation = s.scan_generation.wrapping_add(1);
            s.is_shutdown = true;
        });
    }

    fn remember_dismissed(&self, drafts: &[MapGalleryDraft], state: &mut RiseMapDraftsRepositoryStored) {
        let maximum_drafts = usize::try_from(MapGalleryScout::maximum_drafts()).unwrap_or(0);
        let mut dismissed = state.dismissed.clone().unwrap_or_default();
        for draft in drafts {
            dismissed.insert(draft.id.clone(), draft.taken_at);
        }
        let mut ordered: Vec<i64> = dismissed.values().copied().collect();
        ordered.sort_by(|lhs, rhs| rhs.cmp(lhs));
        if ordered.len() > maximum_drafts {
            let boundary = ordered[maximum_drafts];
            state.reconciliation_start_ms = Some(state.reconciliation_start_ms.unwrap_or(0).max(boundary));
            dismissed.retain(|_, taken_at| *taken_at > boundary);
        }
        state.dismissed = Some(dismissed);
    }

    async fn loaded(&self) -> RiseMapDraftsRepositoryStored {
        if let Some(stored) = self.state.with(|s| s.stored.clone()) {
            return stored;
        }
        let cached = self
            .resources
            .cached::<RiseMapDraftsRepositoryStored>(RiseMapResourceRepositoryResource::Drafts)
            .await
            .unwrap_or_else(|| RiseMapDraftsRepositoryStored::new(0, Vec::new()));
        self.state.with(|s| s.stored.get_or_insert(cached).clone())
    }

    async fn save(&self, state: RiseMapDraftsRepositoryStored) {
        let remembered = state.clone();
        let is_running = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.stored = Some(remembered);
            true
        });
        if !is_running {
            return;
        }
        self.resources.store(RiseMapResourceRepositoryResource::Drafts, state).await;
        self.publish().await;
    }

    async fn publish(&self) {
        let snapshot = self.state.with(|s| {
            s.revision += 1;
            RiseMapDraftsSnapshot {
                drafts: s.stored.as_ref().map(|stored| stored.drafts.clone()).unwrap_or_default(),
                publishing: s.publishing.clone(),
                revision: s.revision,
            }
        });
        let identity = self.identity.clone();
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = hop
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        RiseMapPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.publish_drafts(snapshot, &identity, cx);
                        });
                    });
                    Ok::<_, anyhow::Error>(())
                }
                .boxed_local()
            })
            .await;
    }

    fn now_milliseconds() -> i64 {
        RiseClock::now().ms()
    }
}
