use std::sync::Arc;

use gpui::{App, Context, Entity};

use crate::modules::map::engine::map_guide_sandbox_hooks::map_guide_sandbox_hooks;
use crate::modules::map::engine::core::map_track_tile_store::MapTrackTileStore;
use crate::modules::map::engine::rise_map_drafts_repository::RiseMapDraftsSnapshot;
use crate::modules::map::engine::rise_map_engine_models::{
    RiseMapFriendsSnapshot, RiseMapLiveSnapshot, RiseMapStateSnapshot,
};
use crate::modules::map::engine::rise_map_journeys_repository::RiseMapJourneyPresentation;
use crate::modules::map::engine::rise_map_photos_repository::RiseMapPhotosSnapshot;
use crate::modules::map::engine::rise_map_presentation::RiseMapPresentation;
use crate::modules::map::engine::rise_map_privacy_repository::RiseMapPrivacySnapshot;
use crate::modules::map::engine::rise_map_reactions_repository::MapReactionInboxEntry;
use crate::modules::map::engine::rise_map_tracks_repository::{RiseMapTrackTilesTick, RiseMapTracksSnapshot};

pub struct MapServicesStore {}

impl MapServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    pub fn state(cx: &mut App) -> RiseMapStateSnapshot {
        let presentation = RiseMapPresentation::shared(cx);
        let real = presentation.read(cx).state.read(cx).snapshot().clone();
        (map_guide_sandbox_hooks().map_state)(real, cx)
    }

    pub fn friends(cx: &mut App) -> RiseMapFriendsSnapshot {
        let presentation = RiseMapPresentation::shared(cx);
        let real = presentation.read(cx).friends.read(cx).snapshot().clone();
        (map_guide_sandbox_hooks().map_friends)(real, cx)
    }

    pub fn live(cx: &mut App) -> RiseMapLiveSnapshot {
        let presentation = RiseMapPresentation::shared(cx);
        let real = presentation.read(cx).live.read(cx).snapshot().clone();
        (map_guide_sandbox_hooks().map_live)(real, cx)
    }

    pub fn tracks(cx: &mut App) -> RiseMapTracksSnapshot {
        let presentation = RiseMapPresentation::shared(cx);
        let real = presentation.read(cx).tracks.read(cx).snapshot().clone();
        (map_guide_sandbox_hooks().map_tracks)(real, cx)
    }

    pub fn privacy(cx: &mut App) -> RiseMapPrivacySnapshot {
        let presentation = RiseMapPresentation::shared(cx);
        presentation.read(cx).privacy.read(cx).snapshot().clone()
    }

    pub fn photos(cx: &mut App) -> RiseMapPhotosSnapshot {
        let presentation = RiseMapPresentation::shared(cx);
        let real = presentation.read(cx).photos.read(cx).snapshot().clone();
        (map_guide_sandbox_hooks().map_photos)(real, cx)
    }

    pub fn drafts(cx: &mut App) -> RiseMapDraftsSnapshot {
        let presentation = RiseMapPresentation::shared(cx);
        presentation.read(cx).drafts.read(cx).snapshot().clone()
    }

    pub fn track_tiles_tick(cx: &mut App) -> RiseMapTrackTilesTick {
        let presentation = RiseMapPresentation::shared(cx);
        presentation.read(cx).track_tiles.read(cx).tick().clone()
    }

    pub fn track_tile_store(cx: &mut App) -> Option<Arc<MapTrackTileStore>> {
        let presentation = RiseMapPresentation::shared(cx);
        presentation.read(cx).track_tiles.read(cx).store().cloned()
    }

    pub fn reaction_inbox(cx: &mut App) -> Vec<MapReactionInboxEntry> {
        let presentation = RiseMapPresentation::shared(cx);
        presentation.read(cx).reactions.read(cx).inbox().to_vec()
    }

    pub fn is_member(cx: &mut App) -> bool {
        Self::state(cx).is_member()
    }

    pub fn incoming_requests_count(cx: &mut App) -> i64 {
        let presentation = RiseMapPresentation::shared(cx);
        presentation
            .read(cx)
            .state
            .read(cx)
            .snapshot()
            .state
            .as_ref()
            .map_or(0, |state| state.incoming_requests_count)
    }

    pub fn journey(cx: &mut App) -> Entity<RiseMapJourneyPresentation> {
        RiseMapJourneyPresentation::shared(cx)
    }
}
