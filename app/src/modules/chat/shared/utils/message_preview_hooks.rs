use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

pub struct MessagePreviewHooks {
    pub contact_name: fn(Option<&str>, &str, &mut App) -> String,
    pub media_thumbnail_source: fn(Option<&GetMessageMessage>) -> Option<String>,
}

pub(crate) static MESSAGE_PREVIEW_HOOKS: FeatureHookSlot<MessagePreviewHooks> = FeatureHookSlot::new("MESSAGE_PREVIEW_HOOKS");

/// installers run once from install_feature_hooks which has no App, first install wins
pub fn set_message_preview_hooks(hooks: MessagePreviewHooks) {
    let _ = MESSAGE_PREVIEW_HOOKS.set(hooks);
}

impl MessagePreviewHooks {
    pub fn contact_name(user_id: Option<&str>, fallback: &str, cx: &mut App) -> String {
        match MESSAGE_PREVIEW_HOOKS.get() {
            Some(hooks) => (hooks.contact_name)(user_id, fallback, cx),
            None => fallback.to_owned(),
        }
    }

    pub fn media_thumbnail_source(message: Option<&GetMessageMessage>) -> Option<String> {
        MESSAGE_PREVIEW_HOOKS.get().and_then(|hooks| (hooks.media_thumbnail_source)(message))
    }
}
