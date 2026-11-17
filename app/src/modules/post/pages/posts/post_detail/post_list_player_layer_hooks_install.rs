use crate::modules::post::components::post::post_list_player_layer_hooks::{
    PostListPlayerLayerHooks, set_post_list_player_layer_hooks,
};
use crate::modules::post::pages::posts::post_detail::post_list_shared_element_capture::PostListSharedElementCapture;

pub fn install_post_list_player_layer_hooks() {
    set_post_list_player_layer_hooks(PostListPlayerLayerHooks {
        update_player_layer: PostListSharedElementCapture::update_player_layer,
    });
}
