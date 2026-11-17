use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;
use crate::modules::theme::engine::rise_theme_wallpaper_upload::RiseChatWallpaperDraft;
use crate::modules::theme::engine::rise_theme_wallpapers_presentation::RiseThemeWallpapersSnapshot;

pub struct ThemeGuideSandboxHooks {
    pub owns_app_wallpaper_scene: fn(cx: &App) -> bool,
    pub app_wallpapers_snapshot: fn(real: RiseThemeWallpapersSnapshot, cx: &App) -> RiseThemeWallpapersSnapshot,
    pub adopt_app_wallpaper: fn(draft: RiseChatWallpaperDraft, look: ChatWallpaperLook, cx: &mut App) -> Option<String>,
    pub select_app_wallpaper: fn(id: Option<String>, cx: &mut App),
    pub update_app_wallpaper_look: fn(id: &str, look: ChatWallpaperLook, cx: &mut App),
    pub delete_app_wallpaper: fn(id: &str, cx: &mut App),
    pub shows_chat_wallpaper: fn(chat_id: &str, cx: &App) -> Option<bool>,
    pub set_shows_chat_wallpaper: fn(shows: bool, cx: &mut App),
}

pub(crate) static HOOKS: FeatureHookSlot<ThemeGuideSandboxHooks> = FeatureHookSlot::new("HOOKS");

static UNINSTALLED: ThemeGuideSandboxHooks = ThemeGuideSandboxHooks {
    owns_app_wallpaper_scene: |_| false,
    app_wallpapers_snapshot: |real, _| real,
    adopt_app_wallpaper: |_, _, _| None,
    select_app_wallpaper: |_, _| {},
    update_app_wallpaper_look: |_, _, _| {},
    delete_app_wallpaper: |_, _| {},
    shows_chat_wallpaper: |_, _| None,
    set_shows_chat_wallpaper: |_, _| {},
};

pub fn set_theme_guide_sandbox_hooks(hooks: ThemeGuideSandboxHooks) -> Result<(), ThemeGuideSandboxHooks> {
    HOOKS.set(hooks)
}

pub(crate) fn theme_guide_sandbox_hooks() -> &'static ThemeGuideSandboxHooks {
    HOOKS.get().unwrap_or(&UNINSTALLED)
}
