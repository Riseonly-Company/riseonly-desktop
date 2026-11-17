use gpui::App;

use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::map::engine::map_guide_sandbox_hooks::{MapGuideSandboxHooks, set_map_guide_sandbox_hooks};
use crate::modules::map::engine::rise_map_engine_models::{
    RiseMapFriendsSnapshot, RiseMapLiveSnapshot, RiseMapStateSnapshot,
};
use crate::modules::map::engine::rise_map_photos_repository::RiseMapPhotosSnapshot;
use crate::modules::map::engine::rise_map_tracks_repository::RiseMapTracksSnapshot;

pub fn install_map_guide_sandbox_hooks() {
    let _ = set_map_guide_sandbox_hooks(MapGuideSandboxHooks {
        owns_map_scene,
        map_state,
        map_friends,
        map_live,
        map_tracks,
        map_photos,
    });
}

fn owns_map_scene(cx: &mut App) -> bool {
    GuideSandbox::shared(cx).read(cx).owns_map_scene()
}

fn map_state(real: RiseMapStateSnapshot, cx: &mut App) -> RiseMapStateSnapshot {
    GuideSandbox::shared(cx).read(cx).map_state(real)
}

fn map_friends(real: RiseMapFriendsSnapshot, cx: &mut App) -> RiseMapFriendsSnapshot {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.map_friends(real, cx))
}

fn map_live(real: RiseMapLiveSnapshot, cx: &mut App) -> RiseMapLiveSnapshot {
    GuideSandbox::shared(cx).read(cx).map_live(real)
}

fn map_tracks(real: RiseMapTracksSnapshot, cx: &mut App) -> RiseMapTracksSnapshot {
    GuideSandbox::shared(cx).read(cx).map_tracks(real)
}

fn map_photos(real: RiseMapPhotosSnapshot, cx: &mut App) -> RiseMapPhotosSnapshot {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.map_photos(real, cx))
}
