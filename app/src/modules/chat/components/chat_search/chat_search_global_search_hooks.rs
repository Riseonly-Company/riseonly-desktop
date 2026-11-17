use gpui::{AnyElement, IntoElement};
use rise_ui::swiftui::prelude::*;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

pub struct ChatSearchGlobalSearchHooks {
    pub grid_post_cell: fn(&GetPostFeedResponse) -> AnyElement,
}

pub(crate) static CHAT_SEARCH_GLOBAL_SEARCH_HOOKS: FeatureHookSlot<ChatSearchGlobalSearchHooks> = FeatureHookSlot::new("CHAT_SEARCH_GLOBAL_SEARCH_HOOKS");

/// installs the hooks once, a second call keeps the first ones
pub fn set_chat_search_global_search_hooks(hooks: ChatSearchGlobalSearchHooks) {
    let _ = CHAT_SEARCH_GLOBAL_SEARCH_HOOKS.set(hooks);
}

pub fn chat_search_global_search_hooks() -> Option<&'static ChatSearchGlobalSearchHooks> {
    CHAT_SEARCH_GLOBAL_SEARCH_HOOKS.get()
}

impl ChatSearchGlobalSearchHooks {
    pub(crate) fn grid_post_cell_or_default(post: &GetPostFeedResponse) -> AnyElement {
        match chat_search_global_search_hooks() {
            Some(hooks) => (hooks.grid_post_cell)(post),
            None => div().into_any_element(),
        }
    }
}
