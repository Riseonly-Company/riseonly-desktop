use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::map::engine::rise_map_engine_models::{
    RiseMapFriendsSnapshot, RiseMapLiveSnapshot, RiseMapStateSnapshot,
};
use crate::modules::map::engine::rise_map_photos_repository::RiseMapPhotosSnapshot;
use crate::modules::map::engine::rise_map_tracks_repository::RiseMapTracksSnapshot;

/// what the map presentation and services ask the guide sandbox
pub struct MapGuideSandboxHooks {
    pub owns_map_scene: fn(cx: &mut App) -> bool,
    pub map_state: fn(real: RiseMapStateSnapshot, cx: &mut App) -> RiseMapStateSnapshot,
    pub map_friends: fn(real: RiseMapFriendsSnapshot, cx: &mut App) -> RiseMapFriendsSnapshot,
    pub map_live: fn(real: RiseMapLiveSnapshot, cx: &mut App) -> RiseMapLiveSnapshot,
    pub map_tracks: fn(real: RiseMapTracksSnapshot, cx: &mut App) -> RiseMapTracksSnapshot,
    pub map_photos: fn(real: RiseMapPhotosSnapshot, cx: &mut App) -> RiseMapPhotosSnapshot,
}

pub(crate) static MAP_GUIDE_SANDBOX_HOOKS: FeatureHookSlot<MapGuideSandboxHooks> =
    FeatureHookSlot::new("MAP_GUIDE_SANDBOX_HOOKS");

static UNINSTALLED: MapGuideSandboxHooks = MapGuideSandboxHooks {
    owns_map_scene: |_| false,
    map_state: |real, _| real,
    map_friends: |real, _| real,
    map_live: |real, _| real,
    map_tracks: |real, _| real,
    map_photos: |real, _| real,
};

pub fn set_map_guide_sandbox_hooks(hooks: MapGuideSandboxHooks) -> Result<(), MapGuideSandboxHooks> {
    MAP_GUIDE_SANDBOX_HOOKS.set(hooks)
}

pub fn map_guide_sandbox_hooks() -> &'static MapGuideSandboxHooks {
    MAP_GUIDE_SANDBOX_HOOKS.get().unwrap_or(&UNINSTALLED)
}
