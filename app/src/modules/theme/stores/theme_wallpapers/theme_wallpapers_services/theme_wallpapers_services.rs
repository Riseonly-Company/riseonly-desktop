use gpui::{App, Context, Entity, Subscription};

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::theme::engine::core::rise_theme_wallpaper_wire_models::{RiseChatWallpaper, RiseWallpaperPreferences};
use crate::modules::theme::engine::rise_theme_wallpapers_presentation::{
    RiseThemeWallpapersLoadState, RiseThemeWallpapersPresentation, RiseThemeWallpapersSnapshot,
};
use crate::modules::theme::engine::rise_theme_wallpapers_repository::RiseThemeWallpapersRepository;
use crate::modules::theme::stores::theme_guide_sandbox_hooks::theme_guide_sandbox_hooks;

pub struct ThemeWallpapersServicesStore {
    presentation: Entity<RiseThemeWallpapersPresentation>,
    auth: Entity<AuthServicesStore>,
    _subscriptions: [Subscription; 2],
}

impl ThemeWallpapersServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseThemeWallpapersPresentation::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let subscriptions = [
            cx.observe(&presentation, |_, _, cx| cx.notify()),
            cx.observe(&auth, |_, _, cx| cx.notify()),
        ];
        Self { presentation, auth, _subscriptions: subscriptions }
    }

    pub fn snapshot(&self, cx: &App) -> RiseThemeWallpapersSnapshot {
        let real = self.presentation.read(cx).snapshot().clone();
        (theme_guide_sandbox_hooks().app_wallpapers_snapshot)(real, cx)
    }

    pub fn wallpapers(&self, cx: &App) -> Vec<RiseChatWallpaper> {
        self.snapshot(cx).wallpapers
    }

    pub fn selected_id(&self, cx: &App) -> Option<String> {
        self.snapshot(cx).selected_id
    }

    pub fn preferences(&self, cx: &App) -> RiseWallpaperPreferences {
        self.snapshot(cx).preferences
    }

    pub fn load_state(&self, cx: &App) -> RiseThemeWallpapersLoadState {
        self.snapshot(cx).load_state
    }

    pub fn is_refreshing(&self, cx: &App) -> bool {
        self.snapshot(cx).is_refreshing
    }

    pub fn load_error(&self, cx: &App) -> Option<String> {
        self.snapshot(cx).error
    }

    pub fn mutation_error(&self, cx: &App) -> Option<String> {
        self.snapshot(cx).mutation_error
    }

    pub fn wallpaper(&self, id: &str, cx: &App) -> Option<RiseChatWallpaper> {
        self.snapshot(cx).wallpaper(id).cloned()
    }

    pub fn is_mutating(&self, id: &str, cx: &App) -> bool {
        self.snapshot(cx).mutating_ids.contains(id)
    }

    pub fn shows_chat_wallpaper(&self, chat_id: &str, cx: &App) -> bool {
        self.snapshot(cx).preferences.shows_chat_wallpaper(chat_id)
    }

    pub fn remaining_slots(&self, cx: &App) -> i64 {
        let is_premium = self.auth.read(cx).profile().map(|profile| profile.more.is_premium).unwrap_or(false);
        let used = i64::try_from(self.wallpapers(cx).len()).unwrap_or(i64::MAX);
        let ceiling = RiseThemeWallpapersRepository::maximum_wallpapers(is_premium);
        self.trace_slots(is_premium, used, ceiling, cx);
        (ceiling - used).max(0)
    }

    fn trace_slots(&self, is_premium: bool, used: i64, ceiling: i64, cx: &App) {
        thread_local! {
            static LAST_SLOTS: std::cell::RefCell<Option<(Option<String>, bool, i64, i64)>> =
                const { std::cell::RefCell::new(None) };
        }
        let auth = self.auth.read(cx);
        let profile_id = auth.profile().map(|profile| profile.id.clone());
        let key = (profile_id.clone(), is_premium, used, ceiling);
        let changed = LAST_SLOTS.with(|last| {
            let mut last = last.borrow_mut();
            if last.as_ref() == Some(&key) {
                return false;
            }
            *last = Some(key);
            true
        });
        if changed {
            tracing::warn!(
                target: "riseonly::X2229",
                profile_id = ?profile_id,
                wire_user_id = ?auth.effective_user_id_for_wire(),
                is_premium,
                subscription_status = ?auth.profile().map(|profile| profile.subscription_status),
                used,
                ceiling,
                "wallpaper slots"
            );
        }
    }
}
