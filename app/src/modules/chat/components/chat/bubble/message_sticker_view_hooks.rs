use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{GifItem, PackKind, StickerItem};

pub struct MessageStickerViewHooks {
    pub effective_user_id_for_wire: fn(&App) -> Option<String>,
    pub protects_content: fn(&GetMessageMessage, &ChatInfo) -> bool,
    pub pack_stickers: fn(&str, &App) -> Vec<StickerItem>,
    pub is_favourite: fn(&str, &str, &App) -> bool,
    pub is_gif_favourite: fn(&str, &GifItem, &App) -> bool,
    pub toggle_favourite: fn(&str, &StickerItem, &mut App),
    pub toggle_gif_favourite: fn(&str, &GifItem, &mut App),
    pub resolve: fn(&str) -> String,
}

pub(crate) static MESSAGE_STICKER_VIEW_HOOKS: FeatureHookSlot<MessageStickerViewHooks> = FeatureHookSlot::new("MESSAGE_STICKER_VIEW_HOOKS");

/// second install is ignored, first owner keeps the slot
pub fn set_message_sticker_view_hooks(hooks: MessageStickerViewHooks) {
    let _ = MESSAGE_STICKER_VIEW_HOOKS.set(hooks);
}

pub fn message_sticker_view_hooks() -> Option<&'static MessageStickerViewHooks> {
    MESSAGE_STICKER_VIEW_HOOKS.get()
}

impl MessageStickerViewHooks {
    pub fn resolved_effective_user_id_for_wire(cx: &App) -> Option<String> {
        message_sticker_view_hooks().and_then(|hooks| (hooks.effective_user_id_for_wire)(cx))
    }

    /// true until installed, the favourite menu stays empty
    pub fn resolved_protects_content(message: &GetMessageMessage, chat: &ChatInfo) -> bool {
        message_sticker_view_hooks().is_none_or(|hooks| (hooks.protects_content)(message, chat))
    }

    pub fn resolved_pack_stickers(pack_id: &str, cx: &App) -> Vec<StickerItem> {
        message_sticker_view_hooks().map_or_else(Vec::new, |hooks| (hooks.pack_stickers)(pack_id, cx))
    }

    pub fn resolved_is_favourite(user_id: &str, sticker_id: &str, cx: &App) -> bool {
        message_sticker_view_hooks().is_some_and(|hooks| (hooks.is_favourite)(user_id, sticker_id, cx))
    }

    pub fn resolved_is_gif_favourite(user_id: &str, gif: &GifItem, cx: &App) -> bool {
        message_sticker_view_hooks().is_some_and(|hooks| (hooks.is_gif_favourite)(user_id, gif, cx))
    }

    pub fn resolved_toggle_favourite(user_id: &str, sticker: &StickerItem, cx: &mut App) {
        if let Some(hooks) = message_sticker_view_hooks() {
            (hooks.toggle_favourite)(user_id, sticker, cx);
        }
    }

    pub fn resolved_toggle_gif_favourite(user_id: &str, gif: &GifItem, cx: &mut App) {
        if let Some(hooks) = message_sticker_view_hooks() {
            (hooks.toggle_gif_favourite)(user_id, gif, cx);
        }
    }

    pub fn resolved_resolve(raw_value: &str) -> String {
        message_sticker_view_hooks().map_or_else(|| raw_value.to_owned(), |hooks| (hooks.resolve)(raw_value))
    }
}

pub struct MessageStickerViewPackSheetHooks {
    pub present: fn(&str, PackKind, &mut App),
}

pub(crate) static MESSAGE_STICKER_VIEW_PACK_SHEET_HOOKS: FeatureHookSlot<MessageStickerViewPackSheetHooks> = FeatureHookSlot::new("MESSAGE_STICKER_VIEW_PACK_SHEET_HOOKS");

/// second install is ignored, first owner keeps the slot
pub fn set_message_sticker_view_pack_sheet_hooks(hooks: MessageStickerViewPackSheetHooks) {
    let _ = MESSAGE_STICKER_VIEW_PACK_SHEET_HOOKS.set(hooks);
}

pub fn message_sticker_view_pack_sheet_hooks() -> Option<&'static MessageStickerViewPackSheetHooks> {
    MESSAGE_STICKER_VIEW_PACK_SHEET_HOOKS.get()
}

impl MessageStickerViewPackSheetHooks {
    pub fn resolved_present(pack_id: &str, kind: PackKind, cx: &mut App) {
        if let Some(hooks) = message_sticker_view_pack_sheet_hooks() {
            (hooks.present)(pack_id, kind, cx);
        }
    }
}
