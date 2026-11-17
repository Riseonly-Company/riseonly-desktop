use std::sync::Arc;

use gpui::{App, AsyncApp, Context, Task};
use rise_canon::rise_request::map::UpdatePhotoGrid;
use rise_engine::CancellationError;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::map::engine::rise_map_domain::{RiseMapActionExecutor, RiseMapDomain};
use crate::modules::map::engine::rise_map_engine_models::RiseMapMutationOutcome;
use crate::modules::map::engine::rise_map_profiles_repository::{MapProfile, MapProfileSnapshot, MapProfileTab};
use crate::modules::map::stores::map_actions::map_actions::MapActionsStore;
use crate::modules::map::stores::map_actions::map_photos_types::MapPhotoGrid;
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;

impl MapActionsStore {
    fn profile_domain(cx: &mut App) -> Task<anyhow::Result<Arc<RiseMapDomain>>> {
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_| {
            let domain = domain.await?;
            if Some(&domain.account_id) != account.as_ref() || Some(&domain.account_epoch) != epoch.as_ref() {
                return Err(CancellationError.into());
            }
            Ok(domain)
        })
    }

    fn is_domain_account_current(domain: &RiseMapDomain) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(Some(&domain.account_id), Some(&domain.account_epoch))
    }

    pub fn profile_action(
        &mut self,
        owner_id: String,
        tab: Option<MapProfileTab>,
        more: bool,
        cx: &mut Context<Self>,
    ) -> Task<Option<MapProfileSnapshot>> {
        let domain = Self::profile_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return None;
            };
            let result = match tab {
                Some(tab) => domain.profiles.page(&owner_id, tab, more).await,
                None => domain.profiles.open(&owner_id).await,
            };
            if !Self::is_domain_account_current(&domain) {
                return None;
            }
            Some(result)
        })
    }

    pub fn peer_profile_action(&mut self, owner_id: String, cx: &mut Context<Self>) -> Task<Option<MapProfile>> {
        let domain = Self::profile_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let domain = match domain.await {
                Ok(domain) => domain,
                Err(error) => {
                    tracing::warn!(target: "riseonly::X2131", %error, "map peer profile: no current domain");
                    return None;
                }
            };
            let result = match domain.profiles.peer(&owner_id).await {
                Ok(result) => result,
                Err(error) => {
                    tracing::warn!(target: "riseonly::X2131", %error, "map peer profile failed");
                    return None;
                }
            };
            if !Self::is_domain_account_current(&domain) {
                return None;
            }
            Some(result)
        })
    }

    pub fn update_grid_action(&mut self, body: UpdatePhotoGrid, cx: &mut Context<Self>) -> Task<RiseMapMutationOutcome> {
        let domain = Self::profile_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            let grid_id = body.grid_id.clone();
            let result = domain.photos.update_grid(body).await;
            if !Self::is_domain_account_current(&domain) {
                return RiseMapMutationOutcome::Failed(None);
            }
            if result == RiseMapMutationOutcome::Done
                && let Some(grid) = domain.photos.grid(&grid_id)
            {
                domain.profiles.update_grid(grid).await;
            }
            result
        })
    }

    pub fn focus_profile_grid_action(&mut self, grid: MapPhotoGrid, cx: &mut Context<Self>) -> Task<bool> {
        let domain = Self::profile_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return false;
            };
            let is_focused = cx.update(|cx| {
                let interactions = MapInteractionsStore::shared(cx);
                let interactions = interactions.read(cx);
                interactions.focused_profile_grid_id.as_ref() == Some(&grid.id)
                    && interactions
                        .routes_owner
                        .as_ref()
                        .map_or(domain.account_id.as_str(), |owner| owner.id.as_str())
                        == grid.owner_id
            });
            if !is_focused {
                return false;
            }
            domain.photos.focus_grid(&grid).await;
            Self::is_domain_account_current(&domain)
        })
    }
}
