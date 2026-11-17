use crate::modules::map::stores::map_actions::map_interactions_hooks::{
    MapInteractionsHooks, set_map_interactions_hooks,
};
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;

pub fn install_map_interactions_hooks() {
    set_map_interactions_hooks(MapInteractionsHooks {
        set_panel_mode: |mode, cx| {
            MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.set_panel_mode(mode, cx));
        },
        withdraw_guide_scene: |panda_id, place_id, cx| {
            MapInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                if interactions.selected_friend_id() == Some(panda_id) {
                    interactions.select_friend(None, cx);
                }
                if interactions
                    .place_sheet()
                    .is_some_and(|sheet| sheet.draft.id == place_id)
                {
                    interactions.set_place_sheet(None, cx);
                }
                if interactions.routes_owner().is_some_and(|owner| owner.id == panda_id) {
                    interactions.show_own_routes(cx);
                }
            });
        },
        set_camera_request: |request, cx| {
            MapInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.set_camera_request(request, cx));
        },
        selected_grid_id: |cx| MapInteractionsStore::shared(cx).read(cx).selected_grid_id().map(str::to_owned),
        grid_to_edit_id: |cx| MapInteractionsStore::shared(cx).read(cx).grid_to_edit_id().map(str::to_owned),
        withdraw_guide_grids: |selected, editing, cx| {
            MapInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                if selected {
                    interactions.select_grid(None, cx);
                }
                if editing {
                    interactions.set_grid_to_edit_id(None, cx);
                }
            });
        },
        is_profile_grid_focused: |grid, account_id, cx| {
            let interactions = MapInteractionsStore::shared(cx);
            let interactions = interactions.read(cx);
            interactions.focused_profile_grid_id.as_ref() == Some(&grid.id)
                && interactions
                    .routes_owner
                    .as_ref()
                    .map_or(account_id, |owner| owner.id.as_str())
                    == grid.owner_id
        },
    });
}
