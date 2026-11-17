use gpui::IntoElement;

use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatDecorativeBackdrop;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperPreset;
use crate::modules::media_picker::editor::editor_link_backdrop_hooks::{
    EditorLinkBackdropHooks, set_editor_link_backdrop_hooks,
};

pub fn install_editor_link_backdrop_hooks() {
    set_editor_link_backdrop_hooks(EditorLinkBackdropHooks {
        default_wallpaper_preview: || {
            ChatDecorativeBackdrop::new()
                .ignores_safe_area(false)
                .preview_preset(Some(ChatWallpaperPreset::default_preset().clone()))
                .animates(false)
                .into_any_element()
        },
    });
}
