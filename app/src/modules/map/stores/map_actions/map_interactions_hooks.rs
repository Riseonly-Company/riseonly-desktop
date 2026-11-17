use gpui::App;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::map::components::tracks::map_tracks_panel::MapPanelMode;
use crate::modules::map::stores::map_actions::map_photos_types::MapPhotoGrid;

/// moves the flat map camera once per id, the way back from the globe
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapCameraRequest {
    pub id: Uuid,
    pub latitude: f64,
    pub longitude: f64,
    pub zoom_level: f64,
    pub is_animated: bool,
}

impl MapCameraRequest {
    /// plain init, animated is left at its default
    pub fn new(id: Uuid, latitude: f64, longitude: f64, zoom_level: f64) -> Self {
        Self {
            id,
            latitude,
            longitude,
            zoom_level,
            is_animated: false,
        }
    }
}

/// what the guide sandbox and the map actions ask of the map interactions store
pub struct MapInteractionsHooks {
    pub set_panel_mode: fn(MapPanelMode, &mut App),
    pub withdraw_guide_scene: fn(&str, &str, &mut App),
    pub set_camera_request: fn(Option<MapCameraRequest>, &mut App),
    pub selected_grid_id: fn(&mut App) -> Option<String>,
    pub grid_to_edit_id: fn(&mut App) -> Option<String>,
    pub withdraw_guide_grids: fn(bool, bool, &mut App),
    pub is_profile_grid_focused: fn(&MapPhotoGrid, &str, &mut App) -> bool,
}

pub(crate) static MAP_INTERACTIONS_HOOKS: FeatureHookSlot<MapInteractionsHooks> = FeatureHookSlot::new("MAP_INTERACTIONS_HOOKS");

pub fn set_map_interactions_hooks(hooks: MapInteractionsHooks) {
    let _ = MAP_INTERACTIONS_HOOKS.set(hooks);
}

pub fn map_interactions_hooks() -> Option<&'static MapInteractionsHooks> {
    MAP_INTERACTIONS_HOOKS.get()
}
