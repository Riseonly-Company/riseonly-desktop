use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::map::engine::rise_map_reactions_repository::MapReactionInboxEntry;

/// unread reactions pushed to the map reaction stage
pub struct MapReactionStageHooks {
    pub set_unread: fn(&[MapReactionInboxEntry], &mut App),
    pub reset: fn(&mut App),
}

pub(crate) static MAP_REACTION_STAGE_HOOKS: FeatureHookSlot<MapReactionStageHooks> =
    FeatureHookSlot::new("MAP_REACTION_STAGE_HOOKS");

pub fn set_map_reaction_stage_hooks(hooks: MapReactionStageHooks) {
    let _ = MAP_REACTION_STAGE_HOOKS.set(hooks);
}

pub fn map_reaction_stage_hooks() -> Option<&'static MapReactionStageHooks> {
    MAP_REACTION_STAGE_HOOKS.get()
}
