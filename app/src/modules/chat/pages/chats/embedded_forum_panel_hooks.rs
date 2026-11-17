use gpui::{AnyElement, App, Entity, Window};

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::chat::pages::chats::embedded_forum_transition::{
    EmbeddedForumAction, EmbeddedForumOpenTopicAction, EmbeddedForumTransitionState,
};
use crate::modules::chat::stores::chats::chat_services::chat_roles_services::ForumTopicListSnapshot;

/// args that the stable topic list hands to the forum topic list content
pub(crate) struct EmbeddedForumPanelForumTopicListArgs {
    pub(crate) chat_id: String,
    pub(crate) top_inset: f32,
    pub(crate) bottom_inset: f32,
    pub(crate) interaction_scroll_locked: bool,
    pub(crate) installs_horizontal_swipe_gate: bool,
    pub(crate) initial_presentation_snapshot: ForumTopicListSnapshot,
    pub(crate) presentation_epoch: u64,
    pub(crate) presentation_transition: Entity<EmbeddedForumTransitionState>,
    pub(crate) on_open_all: EmbeddedForumAction,
    pub(crate) on_open_topic: EmbeddedForumOpenTopicAction,
    pub(crate) on_create_section: EmbeddedForumAction,
}

pub(crate) struct EmbeddedForumPanelHooks {
    pub(crate) forum_topic_list: fn(EmbeddedForumPanelForumTopicListArgs, &mut Window, &mut App) -> AnyElement,
}

pub(crate) static EMBEDDED_FORUM_PANEL_HOOKS: FeatureHookSlot<EmbeddedForumPanelHooks> = FeatureHookSlot::new("EMBEDDED_FORUM_PANEL_HOOKS");

/// second install is ignored, first owner keeps the slot
pub(crate) fn set_embedded_forum_panel_hooks(hooks: EmbeddedForumPanelHooks) {
    let _ = EMBEDDED_FORUM_PANEL_HOOKS.set(hooks);
}

pub(crate) fn embedded_forum_panel_hooks() -> Option<&'static EmbeddedForumPanelHooks> {
    EMBEDDED_FORUM_PANEL_HOOKS.get()
}
