use crate::core::feature_hooks::FeatureHookSlot;
use crate::core::shared_element_transition::renderer::shared_element_video_frame_grabber::SharedElementPlayerLayerView;

/// player layer of a post list row, reported to the post list shared element capture
pub struct PostListPlayerLayerHooks {
    pub update_player_layer: fn(u64, &str, Option<SharedElementPlayerLayerView>),
}

pub(crate) static POST_LIST_PLAYER_LAYER_HOOKS: FeatureHookSlot<PostListPlayerLayerHooks> =
    FeatureHookSlot::new("POST_LIST_PLAYER_LAYER_HOOKS");

pub fn set_post_list_player_layer_hooks(hooks: PostListPlayerLayerHooks) {
    let _ = POST_LIST_PLAYER_LAYER_HOOKS.set(hooks);
}

pub fn post_list_player_layer_hooks() -> Option<&'static PostListPlayerLayerHooks> {
    POST_LIST_PLAYER_LAYER_HOOKS.get()
}
