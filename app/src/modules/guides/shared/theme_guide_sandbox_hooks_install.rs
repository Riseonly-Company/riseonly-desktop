use gpui::App;

use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::theme::engine::rise_theme_wallpaper_upload::RiseChatWallpaperDraft;
use crate::modules::theme::engine::rise_theme_wallpapers_presentation::RiseThemeWallpapersSnapshot;
use crate::modules::theme::stores::theme_guide_sandbox_hooks::{ThemeGuideSandboxHooks, set_theme_guide_sandbox_hooks};

pub(crate) fn install_theme_guide_sandbox_hooks() {
    let _ = set_theme_guide_sandbox_hooks(ThemeGuideSandboxHooks {
        owns_app_wallpaper_scene,
        app_wallpapers_snapshot,
        adopt_app_wallpaper,
        select_app_wallpaper,
        update_app_wallpaper_look,
        delete_app_wallpaper,
        shows_chat_wallpaper,
        set_shows_chat_wallpaper,
    });
}

fn owns_app_wallpaper_scene(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_app_wallpaper_scene())
}

fn app_wallpapers_snapshot(real: RiseThemeWallpapersSnapshot, cx: &App) -> RiseThemeWallpapersSnapshot {
    match GuideSandbox::existing(cx) {
        Some(sandbox) => sandbox.read(cx).app_wallpapers_snapshot(real),
        None => real,
    }
}

fn adopt_app_wallpaper(draft: RiseChatWallpaperDraft, look: ChatWallpaperLook, cx: &mut App) -> Option<String> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.adopt_app_wallpaper(draft, look, cx))
}

fn select_app_wallpaper(id: Option<String>, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.select_app_wallpaper(id.as_deref(), cx));
}

fn update_app_wallpaper_look(id: &str, look: ChatWallpaperLook, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.update_app_wallpaper_look(id, look, cx));
}

fn delete_app_wallpaper(id: &str, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_app_wallpaper(id, cx));
}

fn shows_chat_wallpaper(chat_id: &str, cx: &App) -> Option<bool> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).shows_chat_wallpaper(chat_id))
}

fn set_shows_chat_wallpaper(shows: bool, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_shows_chat_wallpaper(shows, cx));
}
