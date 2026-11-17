use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_compare::contains_case_diacritic_insensitive;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{AppLanguage, AppLanguageCatalog, AppRegion, AppRegionCatalog, LanguageStore};

use crate::modules::user::engine::core::rise_user_locale_wire_models::RiseUserLocaleSettings;
use crate::modules::user::engine::rise_user_locale_presentation::RiseUserLocalePresentation;
use crate::modules::user::engine::rise_user_locale_repository::RiseUserLocaleSnapshot;

pub struct UserLocaleServicesStore {
    ordered_languages_cache: Option<(String, Vec<AppLanguage>)>,
    ordered_regions_cache: Option<(String, Vec<AppRegion>)>,
}

impl UserLocaleServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            ordered_languages_cache: None,
            ordered_regions_cache: None,
        }
    }

    fn presentation(cx: &mut App) -> Entity<RiseUserLocalePresentation> {
        RiseUserLocalePresentation::shared(cx)
    }

    pub fn snapshot(&self, cx: &mut App) -> RiseUserLocaleSnapshot {
        Self::presentation(cx).read(cx).snapshot().clone()
    }

    pub fn settings(&self, cx: &mut App) -> RiseUserLocaleSettings {
        Self::presentation(cx).read(cx).snapshot().settings.clone()
    }

    pub fn has_settings(&self, cx: &mut App) -> bool {
        Self::presentation(cx).read(cx).snapshot().has_settings
    }

    pub fn is_refreshing(&self, cx: &mut App) -> bool {
        Self::presentation(cx).read(cx).snapshot().is_refreshing
    }

    pub fn is_mutating(&self, cx: &mut App) -> bool {
        Self::presentation(cx).read(cx).snapshot().is_mutating
    }

    pub fn error(&self, cx: &mut App) -> Option<String> {
        Self::presentation(cx).read(cx).snapshot().error.clone()
    }

    pub fn effective_region(&self, cx: &mut App) -> String {
        let entity = Self::presentation(cx);
        let presentation = entity.read(cx);
        if let Some(pending) = presentation.pending_region() {
            return pending.to_owned();
        }
        let stored = &presentation.snapshot().settings.region_code;
        if stored.is_empty() {
            LanguageStore::active(cx).map(|store| store.region().to_owned()).unwrap_or_default()
        } else {
            stored.clone()
        }
    }

    pub fn effective_app_language(&self, cx: &mut App) -> String {
        let entity = Self::presentation(cx);
        if let Some(pending) = entity.read(cx).pending_language() {
            return pending.to_owned();
        }
        LanguageStore::active(cx)
            .map(|store| store.code().to_owned())
            .unwrap_or_else(|| AppLanguageCatalog::BASE_LANGUAGE.to_owned())
    }

    pub fn should_offer_region_update(&self, cx: &mut App) -> bool {
        let entity = Self::presentation(cx);
        let snapshot = entity.read(cx).snapshot();
        snapshot.has_settings && snapshot.settings.detection_disagrees_with_choice()
    }

    pub fn suggested_region(&self, cx: &mut App) -> String {
        Self::presentation(cx).read(cx).snapshot().settings.detected_region.clone()
    }

    pub fn languages(&mut self, matching: &str, cx: &mut App) -> Vec<AppLanguage> {
        let rows = self.ordered_languages(cx);
        let needle = trimming_whitespaces_and_newlines(matching);
        if needle.is_empty() {
            return rows;
        }
        rows.into_iter()
            .filter(|row| Self::matches(row.native_name, row.english_name, row.code, needle))
            .collect()
    }

    pub fn regions(&mut self, matching: &str, cx: &mut App) -> Vec<AppRegion> {
        let rows = self.ordered_regions(cx);
        let needle = trimming_whitespaces_and_newlines(matching);
        if needle.is_empty() {
            return rows;
        }
        rows.into_iter()
            .filter(|row| Self::matches(&row.localized_name(), row.english_name, row.code, needle))
            .collect()
    }

    fn matches(primary: &str, secondary: &str, code: &str, needle: &str) -> bool {
        contains_case_diacritic_insensitive(primary, needle)
            || contains_case_diacritic_insensitive(secondary, needle)
            || code.to_lowercase().starts_with(&needle.to_lowercase())
    }

    fn language_key(cx: &App) -> String {
        LanguageStore::active(cx)
            .map(|store| store.code().to_owned())
            .unwrap_or_else(|| AppLanguageCatalog::BASE_LANGUAGE.to_owned())
    }

    fn ordered_languages(&mut self, cx: &mut App) -> Vec<AppLanguage> {
        let key = Self::language_key(cx);
        if let Some((language, rows)) = &self.ordered_languages_cache
            && *language == key
        {
            return rows.clone();
        }
        let rows = AppLanguageCatalog::ordered_for_display();
        self.ordered_languages_cache = Some((key, rows.clone()));
        rows
    }

    fn ordered_regions(&mut self, cx: &mut App) -> Vec<AppRegion> {
        let key = Self::language_key(cx);
        if let Some((language, rows)) = &self.ordered_regions_cache
            && *language == key
        {
            return rows.clone();
        }
        let preferring = self.effective_region(cx);
        let rows = AppRegionCatalog::ordered_for_display(&preferring);
        self.ordered_regions_cache = Some((key, rows.clone()));
        rows
    }
}
