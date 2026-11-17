use crate::modules::map::components::reactions::map_reaction_stage::MapReactionStage;
use crate::modules::map::engine::map_reaction_stage_hooks::{MapReactionStageHooks, set_map_reaction_stage_hooks};

pub fn install_map_reaction_stage_hooks() {
    set_map_reaction_stage_hooks(MapReactionStageHooks {
        set_unread: |entries, cx| {
            MapReactionStage::shared(cx).update(cx, |stage, cx| stage.set_unread(entries, cx));
        },
        reset: |cx| {
            MapReactionStage::shared(cx).update(cx, |stage, cx| stage.reset(cx));
        },
    });
}
