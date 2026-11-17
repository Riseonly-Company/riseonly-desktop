use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;

pub struct ChatWallpaperSheetHooks {
    pub is_owner: fn(&ChatInfo) -> bool,
}

pub(crate) static CHAT_WALLPAPER_SHEET_HOOKS: FeatureHookSlot<ChatWallpaperSheetHooks> = FeatureHookSlot::new("CHAT_WALLPAPER_SHEET_HOOKS");

/// second install is ignored, first owner keeps the slot
pub fn set_chat_wallpaper_sheet_hooks(hooks: ChatWallpaperSheetHooks) {
    let _ = CHAT_WALLPAPER_SHEET_HOOKS.set(hooks);
}

pub(crate) fn chat_wallpaper_sheet_hooks() -> Option<&'static ChatWallpaperSheetHooks> {
    CHAT_WALLPAPER_SHEET_HOOKS.get()
}

impl ChatWallpaperSheetHooks {
    pub(crate) fn resolved_is_owner(chat: &ChatInfo) -> bool {
        chat_wallpaper_sheet_hooks().is_some_and(|hooks| (hooks.is_owner)(chat))
    }
}
