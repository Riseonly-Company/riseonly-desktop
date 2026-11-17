use gpui::{AnyElement, App, IntoElement, Window};

use crate::modules::chat::pages::chats::chat::forum_home::forum_topic_list_content::ForumTopicListContent;
use crate::modules::chat::pages::chats::embedded_forum_panel_hooks::{
    EmbeddedForumPanelForumTopicListArgs, EmbeddedForumPanelHooks, set_embedded_forum_panel_hooks,
};

pub fn install_embedded_forum_panel_hooks() {
    set_embedded_forum_panel_hooks(EmbeddedForumPanelHooks { forum_topic_list });
}

fn forum_topic_list(args: EmbeddedForumPanelForumTopicListArgs, _: &mut Window, _: &mut App) -> AnyElement {
    ForumTopicListContent::new(args.chat_id, args.on_open_all, args.on_open_topic)
        .top_inset(args.top_inset)
        .bottom_inset(args.bottom_inset)
        .interaction_scroll_locked(args.interaction_scroll_locked)
        .installs_horizontal_swipe_gate(args.installs_horizontal_swipe_gate)
        .initial_presentation_snapshot(Some(args.initial_presentation_snapshot))
        .presentation_epoch(args.presentation_epoch)
        .presentation_transition(Some(args.presentation_transition))
        .on_create_section(args.on_create_section)
        .into_any_element()
}
