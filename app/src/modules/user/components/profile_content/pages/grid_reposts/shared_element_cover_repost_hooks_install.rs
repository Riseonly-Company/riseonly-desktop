use gpui::{AnyElement, IntoElement};

use crate::core::shared_element_transition::renderer::shared_element_cover_hooks::SharedElementCoverRepostHooks;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::user::components::profile_content::pages::grid_reposts::grid_reposts::ProfileGridRepostTextUi;

pub fn install_shared_element_cover_repost_hooks() {
    SharedElementCoverRepostHooks::set(SharedElementCoverRepostHooks { profile_grid_repost_text_ui });
}

fn profile_grid_repost_text_ui(post: &GetPostFeedResponse) -> AnyElement {
    ProfileGridRepostTextUi::new(post.clone()).into_any_element()
}
