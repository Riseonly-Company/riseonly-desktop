use gpui::{App, Context, Entity};
use rise_i18n::{AppLanguageCatalog, AppRegionCatalog, LanguageStore};

use crate::modules::user::engine::rise_user_locale_presentation::RiseUserLocalePresentation;
use crate::modules::user::stores::user_locale::user_locale_actions::user_locale_actions::UserLocaleActionsStore;
use crate::modules::user::stores::user_locale::user_locale_services::user_locale_services::UserLocaleServicesStore;

pub struct UserLocaleInteractionsStore {}

impl UserLocaleInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    fn actions(cx: &mut App) -> Entity<UserLocaleActionsStore> {
        UserLocaleActionsStore::shared(cx)
    }

    pub fn open_region_settings(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.load_action(false, cx));
    }

    pub fn open_language_settings(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.load_action(false, cx));
    }

    pub fn select_region(&mut self, region: &str, cx: &mut Context<Self>) {
        let Some(normalized) = AppRegionCatalog::normalize(region) else {
            return;
        };
        let effective = UserLocaleServicesStore::shared(cx).update(cx, |services, cx| services.effective_region(cx));
        if normalized == effective {
            return;
        }
        RiseUserLocalePresentation::shared(cx)
            .update(cx, |presentation, cx| presentation.mark_pending_region(normalized.to_owned(), cx));
        if LanguageStore::active(cx).is_some() {
            LanguageStore::update(cx, |store, _| store.select_region(normalized));
        }
        Self::actions(cx).update(cx, |actions, cx| actions.update_region_action(normalized.to_owned(), cx));
    }

    pub fn select_language(&mut self, language: &str, cx: &mut Context<Self>) {
        let Some(normalized) = AppLanguageCatalog::normalize(language) else {
            return;
        };
        let effective =
            UserLocaleServicesStore::shared(cx).update(cx, |services, cx| services.effective_app_language(cx));
        if normalized == effective {
            return;
        }
        RiseUserLocalePresentation::shared(cx)
            .update(cx, |presentation, cx| presentation.mark_pending_language(normalized.to_owned(), cx));
        if LanguageStore::active(cx).is_some() {
            LanguageStore::update(cx, |store, cx| store.select_language(normalized, cx));
        }
        Self::actions(cx).update(cx, |actions, cx| actions.update_app_language_action(normalized.to_owned(), cx));
    }

    pub fn select_notification_language(&mut self, language: Option<String>, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.update_notification_language_action(language, cx));
    }
}
