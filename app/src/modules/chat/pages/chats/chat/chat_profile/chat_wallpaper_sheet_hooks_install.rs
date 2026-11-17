use crate::modules::chat::components::chat::backdrop::chat_wallpaper_sheet_hooks::{
    ChatWallpaperSheetHooks, set_chat_wallpaper_sheet_hooks,
};
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;

pub fn install_chat_wallpaper_sheet_hooks() {
    set_chat_wallpaper_sheet_hooks(ChatWallpaperSheetHooks { is_owner: ChatProfilePermissions::is_owner });
}
