use gpui::{AsyncApp, Context, Task};

use crate::modules::map::engine::rise_map_domain::RiseMapActionExecutor;
use crate::modules::map::engine::rise_map_engine_models::RiseMapMutationOutcome;
use crate::modules::map::engine::rise_map_tracks_repository::MapPlaceSaveResult;
use crate::modules::map::stores::map_actions::map_actions::MapActionsStore;
use crate::modules::map::stores::map_actions::map_tracks_types::MapPlace;

impl MapActionsStore {
    pub fn save_place_action(&mut self, place: MapPlace, creating: bool, cx: &mut Context<Self>) -> Task<MapPlaceSaveResult> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return MapPlaceSaveResult::Failed(None);
            };
            domain.tracks.save_place(place, creating).await
        })
    }

    pub fn delete_place_action(&mut self, place: MapPlace, cx: &mut Context<Self>) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            domain.tracks.delete_place(&place).await
        })
    }
}
