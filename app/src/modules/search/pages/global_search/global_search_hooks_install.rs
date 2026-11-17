use rise_ui::swiftui::prelude::*;

use crate::modules::chat::components::chat_search::chat_search_global_search_hooks::{
    ChatSearchGlobalSearchHooks, set_chat_search_global_search_hooks,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::search::pages::global_search::global_search::GridPostCell;

pub fn install_global_search_hooks() {
    set_chat_search_global_search_hooks(ChatSearchGlobalSearchHooks {
        grid_post_cell: |post: &GetPostFeedResponse| GridPostCell::new(post.clone()).into_any_element(),
    });
}
