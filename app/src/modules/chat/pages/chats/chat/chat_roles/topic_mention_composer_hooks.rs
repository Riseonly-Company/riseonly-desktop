use gpui::App;

use crate::app::router::navigation_types::RiseRTCRoomTarget;
use crate::core::feature_hooks::FeatureHookSlot;

/// what a tapped voice channel mention asks the rtc layer
pub struct TopicMentionComposerHooks {
    pub present_room: fn(RiseRTCRoomTarget, &mut App),
}

pub(crate) static TOPIC_MENTION_COMPOSER_HOOKS: FeatureHookSlot<TopicMentionComposerHooks> =
    FeatureHookSlot::new("TOPIC_MENTION_COMPOSER_HOOKS");

pub fn set_topic_mention_composer_hooks(hooks: TopicMentionComposerHooks) {
    let _ = TOPIC_MENTION_COMPOSER_HOOKS.set(hooks);
}

pub fn topic_mention_composer_hooks() -> Option<&'static TopicMentionComposerHooks> {
    TOPIC_MENTION_COMPOSER_HOOKS.get()
}
