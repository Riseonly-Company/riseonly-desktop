use gpui::{App, Context, Entity};

use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::{ChatWallpaperLook, ChatWallpaperStore};
use crate::modules::theme::engine::rise_theme_wallpaper_upload::{RiseChatWallpaperDraft, RiseChatWallpaperUpload};
use crate::modules::theme::stores::theme_guide_sandbox_hooks::theme_guide_sandbox_hooks;
use crate::modules::theme::stores::theme_wallpapers::theme_wallpapers_actions::theme_wallpapers_actions::{
    ThemeWallpaperCreated, ThemeWallpapersActionsStore,
};
use crate::modules::theme::stores::theme_wallpapers::theme_wallpapers_services::theme_wallpapers_services::ThemeWallpapersServicesStore;

pub struct ThemeWallpapersInteractionsStore {
    actions: Entity<ThemeWallpapersActionsStore>,
    services: Entity<ThemeWallpapersServicesStore>,
}

impl ThemeWallpapersInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self { actions: ThemeWallpapersActionsStore::shared(cx), services: ThemeWallpapersServicesStore::shared(cx) }
    }

    pub fn load(&mut self, force: bool, cx: &mut Context<Self>) {
        self.actions.update(cx, |actions, cx| actions.load_action(force, cx));
    }

    pub fn retry_load(&mut self, cx: &mut Context<Self>) {
        let access_token = AuthServicesStore::shared(cx).read(cx).tokens_and_other_data().access_token.clone();
        SaiWebSocketManager::shared(cx).update(cx, |socket, cx| socket.connect_if_needed(Some(&access_token), cx));
        self.actions.update(cx, |actions, cx| actions.load_action(true, cx));
    }

    pub fn can_add_more(&self, cx: &App) -> bool {
        (theme_guide_sandbox_hooks().owns_app_wallpaper_scene)(cx) || self.services.read(cx).remaining_slots(cx) > 0
    }

    pub fn add(
        &mut self,
        draft: RiseChatWallpaperDraft,
        look: ChatWallpaperLook,
        on_created: Option<ThemeWallpaperCreated>,
        cx: &mut Context<Self>,
    ) {
        let hooks = theme_guide_sandbox_hooks();
        if (hooks.owns_app_wallpaper_scene)(cx) {
            let adopted = (hooks.adopt_app_wallpaper)(draft, look.clamped(), cx);
            if let Some(on_created) = on_created {
                on_created(adopted, cx);
            }
            return;
        }
        if !self.can_add_more(cx) {
            RiseChatWallpaperUpload::discard_staged_file(&draft.local_url);
            if let Some(on_created) = on_created {
                on_created(None, cx);
            }
            return;
        }
        let account_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let selected_before = self.services.read(cx).selected_id(cx);
        let this = cx.weak_entity();
        let services = self.services.clone();
        let completion: ThemeWallpaperCreated = Box::new(move |created, cx| {
            if let Some(id) = created.as_deref().filter(|id| !id.is_empty())
                && AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire() == account_id
                && services.read(cx).selected_id(cx) == selected_before
            {
                let id = id.to_owned();
                this.update(cx, |store, cx| store.select(&id, cx)).ok();
            }
            if let Some(on_created) = on_created {
                on_created(created, cx);
            }
        });
        self.actions.update(cx, |actions, cx| actions.add_action(draft, look.clamped(), Some(completion), cx));
    }

    pub fn select(&mut self, id: &str, cx: &mut Context<Self>) {
        if (theme_guide_sandbox_hooks().owns_app_wallpaper_scene)(cx) {
            (theme_guide_sandbox_hooks().select_app_wallpaper)(Some(id.to_owned()), cx);
            return;
        }
        if id.is_empty() || self.services.read(cx).is_mutating(id, cx) {
            return;
        }
        let id = id.to_owned();
        self.actions.update(cx, |actions, cx| actions.select_action(Some(id), false, cx));
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if (theme_guide_sandbox_hooks().owns_app_wallpaper_scene)(cx) {
            (theme_guide_sandbox_hooks().select_app_wallpaper)(None, cx);
            return;
        }
        if self.services.read(cx).selected_id(cx).is_none() {
            return;
        }
        self.actions.update(cx, |actions, cx| actions.select_action(None, false, cx));
    }

    pub fn delete(&mut self, id: &str, cx: &mut Context<Self>) {
        if (theme_guide_sandbox_hooks().owns_app_wallpaper_scene)(cx) {
            (theme_guide_sandbox_hooks().delete_app_wallpaper)(id, cx);
            return;
        }
        if id.is_empty() || self.services.read(cx).is_mutating(id, cx) {
            return;
        }
        let id = id.to_owned();
        self.actions.update(cx, |actions, cx| actions.delete_action(id, cx));
    }

    pub fn update_look(&mut self, id: &str, look: ChatWallpaperLook, cx: &mut Context<Self>) {
        if (theme_guide_sandbox_hooks().owns_app_wallpaper_scene)(cx) {
            (theme_guide_sandbox_hooks().update_app_wallpaper_look)(id, look.clamped(), cx);
            return;
        }
        if id.is_empty() || self.services.read(cx).is_mutating(id, cx) {
            return;
        }
        let Some(row) = self.services.read(cx).wallpaper(id, cx) else {
            return;
        };
        if row.is_pending {
            return;
        }
        let id = id.to_owned();
        self.actions.update(cx, |actions, cx| actions.update_look_action(id, look.clamped(), cx));
    }

    pub fn set_own_only(&mut self, own_only: bool, cx: &mut Context<Self>) {
        if (theme_guide_sandbox_hooks().owns_app_wallpaper_scene)(cx) {
            return;
        }
        let wallpapers = ChatWallpaperStore::shared(cx);
        if wallpapers.read(cx).own_wallpapers_only() == own_only {
            return;
        }
        wallpapers.update(cx, |store, cx| store.apply_own_only_optimistic(own_only, cx));
        self.actions.update(cx, |actions, cx| actions.set_own_only_action(own_only, cx));
    }

    pub fn set_shows_chat_wallpaper(&mut self, shows: bool, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }
        if (theme_guide_sandbox_hooks().shows_chat_wallpaper)(chat_id, cx).is_some() {
            (theme_guide_sandbox_hooks().set_shows_chat_wallpaper)(shows, cx);
            return;
        }
        ChatWallpaperStore::shared(cx).update(cx, |store, cx| store.apply_shows_chat_wallpaper_optimistic(shows, chat_id, cx));
        let chat_id = chat_id.to_owned();
        self.actions.update(cx, |actions, cx| actions.set_shows_chat_wallpaper_action(shows, chat_id, cx));
    }

    pub fn dismiss_mutation_error(&mut self, cx: &mut Context<Self>) {
        self.actions.update(cx, |actions, cx| actions.clear_mutation_error_action(cx));
    }
}
