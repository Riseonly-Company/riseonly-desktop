use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

pub struct PostRowWarmAnchorHooks {
    pub warm_anchor: fn(&GetPostFeedResponse, Option<f32>, Option<f32>, Option<f32>, &mut App),
}

pub(crate) static POST_ROW_WARM_ANCHOR_HOOKS: FeatureHookSlot<PostRowWarmAnchorHooks> =
    FeatureHookSlot::new("POST_ROW_WARM_ANCHOR_HOOKS");

pub fn set_post_row_warm_anchor_hooks(hooks: PostRowWarmAnchorHooks) {
    let _ = POST_ROW_WARM_ANCHOR_HOOKS.set(hooks);
}

pub fn post_row_warm_anchor_hooks() -> Option<&'static PostRowWarmAnchorHooks> {
    POST_ROW_WARM_ANCHOR_HOOKS.get()
}
