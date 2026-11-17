use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

pub struct PostInteractionsCommentHooks {
    pub open_comments_sheet: fn(&GetPostFeedResponse, bool, &mut App),
}

pub struct PostInteractionsHooks {
    pub comment: PostInteractionsCommentHooks,
}

pub(crate) static POST_INTERACTIONS_HOOKS: FeatureHookSlot<PostInteractionsHooks> = FeatureHookSlot::new("POST_INTERACTIONS_HOOKS");

pub fn set_post_interactions_hooks(hooks: PostInteractionsHooks) {
    let _ = POST_INTERACTIONS_HOOKS.set(hooks);
}

pub(crate) fn post_interactions_hooks() -> Option<&'static PostInteractionsHooks> {
    POST_INTERACTIONS_HOOKS.get()
}
