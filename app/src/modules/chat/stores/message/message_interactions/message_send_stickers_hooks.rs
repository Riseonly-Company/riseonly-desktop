use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{GifItem, PackKind};

pub struct MessageSendStickersHooks {
    pub write_recent: fn(user_id: &str, sticker_id: &str, kind: Option<PackKind>, cx: &mut App),
    pub add_gif_favourite: fn(user_id: &str, gif: &GifItem, cx: &mut App),
}

pub(crate) static MESSAGE_SEND_STICKERS_HOOKS: FeatureHookSlot<MessageSendStickersHooks> =
    FeatureHookSlot::new("MESSAGE_SEND_STICKERS_HOOKS");

pub fn set_message_send_stickers_hooks(hooks: MessageSendStickersHooks) {
    let _ = MESSAGE_SEND_STICKERS_HOOKS.set(hooks);
}

pub fn message_send_stickers_hooks() -> Option<&'static MessageSendStickersHooks> {
    MESSAGE_SEND_STICKERS_HOOKS.get()
}
