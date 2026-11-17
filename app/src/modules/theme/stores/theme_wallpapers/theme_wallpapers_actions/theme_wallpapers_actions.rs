use gpui::{App, AsyncApp, Context, Entity};

use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;
use crate::modules::theme::engine::rise_theme_domain::RiseThemeActionExecutor;
use crate::modules::theme::engine::rise_theme_wallpaper_upload::{RiseChatWallpaperDraft, RiseChatWallpaperUpload};

pub type ThemeWallpaperCreated = Box<dyn FnOnce(Option<String>, &mut App)>;

pub struct ThemeWallpapersActionsStore {}

impl ThemeWallpapersActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    pub fn load_action(&mut self, force: bool, cx: &mut Context<Self>) {
        let domain = RiseThemeActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.refresh(force).await;
        })
        .detach();
    }

    pub fn add_action(
        &mut self,
        draft: RiseChatWallpaperDraft,
        look: ChatWallpaperLook,
        on_created: Option<ThemeWallpaperCreated>,
        cx: &mut Context<Self>,
    ) {
        let domain = RiseThemeActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let created = match domain.await {
                Ok(domain) => domain.add_wallpaper(draft, look).await,
                Err(_) => {
                    RiseChatWallpaperUpload::discard_staged_file(&draft.local_url);
                    None
                }
            };
            if let Some(on_created) = on_created {
                cx.update(|cx| on_created(created, cx));
            }
        })
        .detach();
    }

    pub fn select_action(&mut self, id: Option<String>, refresh_first: bool, cx: &mut Context<Self>) {
        let domain = RiseThemeActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            if refresh_first && id.is_some() {
                domain.refresh(true).await;
            }
            domain.select_wallpaper(id).await;
        })
        .detach();
    }

    pub fn delete_action(&mut self, id: String, cx: &mut Context<Self>) {
        let domain = RiseThemeActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.delete_wallpaper(id).await;
        })
        .detach();
    }

    pub fn update_look_action(&mut self, id: String, look: ChatWallpaperLook, cx: &mut Context<Self>) {
        let domain = RiseThemeActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.update_wallpaper_look(id, look).await;
        })
        .detach();
    }

    pub fn set_own_only_action(&mut self, own_only: bool, cx: &mut Context<Self>) {
        let domain = RiseThemeActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.set_own_wallpapers_only(own_only).await;
        })
        .detach();
    }

    pub fn set_shows_chat_wallpaper_action(&mut self, shows: bool, chat_id: String, cx: &mut Context<Self>) {
        let domain = RiseThemeActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.set_shows_chat_wallpaper(shows, chat_id).await;
        })
        .detach();
    }

    pub fn clear_mutation_error_action(&mut self, cx: &mut Context<Self>) {
        let domain = RiseThemeActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.clear_mutation_error().await;
        })
        .detach();
    }
}
