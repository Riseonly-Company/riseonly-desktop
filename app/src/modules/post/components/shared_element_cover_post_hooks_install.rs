use gpui::{AnyElement, IntoElement};

use crate::core::shared_element_transition::renderer::shared_element_cover_hooks::SharedElementCoverPostHooks;
use crate::modules::post::components::post_collection_ui::PostGridTextUi;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

pub(crate) fn install_shared_element_cover_post_hooks() {
    SharedElementCoverPostHooks::set(SharedElementCoverPostHooks { post_grid_text_ui });
}

fn post_grid_text_ui(post: &GetPostFeedResponse) -> AnyElement {
    PostGridTextUi::new(post.clone()).into_any_element()
}
