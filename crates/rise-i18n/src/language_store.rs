use std::sync::{OnceLock, PoisonError, RwLock};

use gpui::{App, BorrowAppContext, Global};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_platform::user_defaults::UserDefaults;

use crate::app_language_catalog::AppLanguageCatalog;
use crate::app_region_catalog::AppRegionCatalog;
use crate::i18n::I18n;
use crate::language_store_keys::{EXPLICIT_KEY, LANGUAGE_KEY, REGION_KEY};

/// locale as the app formats with it, an icu identifier like pt_BR
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Locale {
    identifier: String,
}

impl Locale {
    pub fn new(identifier: impl Into<String>) -> Self {
        Self { identifier: identifier.into() }
    }

    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// formatting locale of the app language, never the device one
    pub fn app_locale() -> Locale {
        LanguageStore::current_locale()
    }

    /// region name from the canon table, pass the whole identifier
    /// zh_Hant_TW and zh are different scripts
    pub fn localized_string_for_region_code(&self, code: &str) -> Option<String> {
        rise_regions::region_name(&self.identifier, code).map(str::to_owned)
    }
}

/// layout direction, the window root lays its content out by it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutDirection {
    LeftToRight,
    RightToLeft,
}

/// app level language and region, works before any account
/// not an engine store, must answer on first frame of a cold launch
/// explicit choice is pushed up once an account exists
pub struct LanguageStore {
    code: String,
    region: String,
    language_is_explicit: bool,
    account_scope: Option<String>,
    account_has_stored_language: bool,
}

impl Global for LanguageStore {}

fn current_locale_slot() -> &'static RwLock<Locale> {
    static CURRENT: OnceLock<RwLock<Locale>> = OnceLock::new();
    CURRENT.get_or_init(|| {
        RwLock::new(LanguageStore::locale_for(&LanguageStore::resolve_initial_code()))
    })
}

impl LanguageStore {
    /// the one instance, built on first access
    pub fn shared(cx: &mut App) -> &LanguageStore {
        if !cx.has_global::<LanguageStore>() {
            let store = LanguageStore::new();
            cx.set_global(store);
        }
        cx.global::<LanguageStore>()
    }

    /// the instance the scene uses, none before shared built it
    pub fn active(cx: &App) -> Option<&LanguageStore> {
        cx.try_global::<LanguageStore>()
    }

    pub fn update<R>(cx: &mut App, f: impl FnOnce(&mut LanguageStore, &mut App) -> R) -> R {
        Self::shared(cx);
        cx.update_global::<LanguageStore, R>(f)
    }

    fn new() -> Self {
        let resolved = Self::resolve_initial_code();
        let defaults = UserDefaults::standard();
        let store = Self {
            region: defaults.string(REGION_KEY).unwrap_or_default(),
            language_is_explicit: defaults.bool(EXPLICIT_KEY),
            code: resolved.clone(),
            account_scope: None,
            account_has_stored_language: false,
        };
        Self::set_current_locale(Self::locale_for(&resolved));
        I18n::reload(&resolved);
        store
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    fn set_code(&mut self, code: String) {
        if code == self.code {
            return;
        }
        self.code = code;
        let defaults = UserDefaults::standard();
        defaults.set(&self.scoped(LANGUAGE_KEY), self.code.as_str());
        defaults.set(LANGUAGE_KEY, self.code.as_str());
        Self::set_current_locale(Self::locale_for(&self.code));
        I18n::reload(&self.code);
    }

    /// iso 3166 alpha-2, empty if nothing is resolved yet
    /// drives content recommendations and not formatting
    pub fn region(&self) -> &str {
        &self.region
    }

    fn set_region(&mut self, region: String) {
        if region == self.region {
            return;
        }
        self.region = region;
        let defaults = UserDefaults::standard();
        if self.region.is_empty() {
            defaults.remove_object(&self.scoped(REGION_KEY));
        } else {
            defaults.set(&self.scoped(REGION_KEY), self.region.as_str());
        }
    }

    pub fn language_is_explicit(&self) -> bool {
        self.language_is_explicit
    }

    pub fn locale(&self) -> Locale {
        Self::locale_for(&self.code)
    }

    pub fn is_rtl(&self) -> bool {
        AppLanguageCatalog::language(&self.code).is_some_and(|language| language.is_rtl)
    }

    pub fn supported_codes() -> &'static [&'static str] {
        AppLanguageCatalog::codes()
    }

    /// the window root mirrors its layout by this
    pub fn layout_direction(&self) -> LayoutDirection {
        if self.is_rtl() { LayoutDirection::RightToLeft } else { LayoutDirection::LeftToRight }
    }

    fn scoped(&self, key: &str) -> String {
        match &self.account_scope {
            Some(scope) => format!("{key}.{scope}"),
            None => key.to_owned(),
        }
    }

    pub fn current_locale() -> Locale {
        current_locale_slot()
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_current_locale(locale: Locale) {
        *current_locale_slot().write().unwrap_or_else(PoisonError::into_inner) = locale;
    }

    pub fn select_language(&mut self, candidate: &str, cx: &App) {
        let Some(normalized) = AppLanguageCatalog::normalize(candidate) else {
            return;
        };
        self.language_is_explicit = true;
        let defaults = UserDefaults::standard();
        defaults.set(&self.scoped(EXPLICIT_KEY), true);
        defaults.set(EXPLICIT_KEY, true);
        I18n::prewarm(normalized, cx);
        self.set_code(normalized.to_owned());
    }

    pub fn select_region(&mut self, candidate: &str) {
        let Some(normalized) = AppRegionCatalog::normalize(candidate) else {
            return;
        };
        self.set_region(normalized.to_owned());
    }

    /// points the stored preferences at an account, or back at the device on sign out
    /// the outgoing account keeps the language it had on screen
    /// an account with nothing stored keeps the screen language until its server value arrives
    pub fn bind(&mut self, account_id: Option<&str>, cx: &App) {
        let normalized_scope = account_id
            .map(trimming_whitespaces_and_newlines)
            .filter(|scope| !scope.is_empty())
            .map(str::to_owned);
        if normalized_scope == self.account_scope {
            return;
        }
        let defaults = UserDefaults::standard();
        if self.account_scope.is_some()
            && (self.account_has_stored_language || self.language_is_explicit)
        {
            defaults.set(&self.scoped(LANGUAGE_KEY), self.code.as_str());
        }
        self.account_scope = normalized_scope;

        self.language_is_explicit = defaults.bool(&self.scoped(EXPLICIT_KEY));
        let region = defaults.string(&self.scoped(REGION_KEY)).unwrap_or_default();
        self.set_region(region);

        let stored = defaults
            .string(&self.scoped(LANGUAGE_KEY))
            .and_then(|raw| AppLanguageCatalog::normalize(&raw));
        self.account_has_stored_language = self.account_scope.is_some() && stored.is_some();
        if let Some(stored) = stored
            && stored != self.code
        {
            I18n::prewarm(stored, cx);
            self.set_code(stored.to_owned());
        }
    }

    pub fn apply_account_language(&mut self, candidate: &str, server_explicit: bool, cx: &App) {
        if self.account_scope.is_none()
            || self.language_is_explicit
            || !(server_explicit || !self.account_has_stored_language)
        {
            return;
        }
        let Some(normalized) = AppLanguageCatalog::normalize(candidate) else {
            return;
        };
        let defaults = UserDefaults::standard();
        if server_explicit {
            self.language_is_explicit = true;
            defaults.set(&self.scoped(EXPLICIT_KEY), true);
        }
        self.account_has_stored_language = true;
        defaults.set(&self.scoped(LANGUAGE_KEY), normalized);
        if normalized == self.code {
            return;
        }
        I18n::prewarm(normalized, cx);
        self.set_code(normalized.to_owned());
    }

    pub fn apply_bootstrap(&mut self, region: Option<&str>, language: Option<&str>, cx: &App) {
        if self.region.is_empty()
            && let Some(normalized) = region.and_then(AppRegionCatalog::normalize)
        {
            self.set_region(normalized.to_owned());
        }
        if self.language_is_explicit {
            return;
        }
        let Some(normalized) = language.and_then(AppLanguageCatalog::normalize) else {
            return;
        };
        if normalized == self.code {
            return;
        }
        I18n::prewarm(normalized, cx);
        self.set_code(normalized.to_owned());
    }

    fn resolve_initial_code() -> String {
        let defaults = UserDefaults::standard();
        if let Some(normalized) = defaults
            .string(LANGUAGE_KEY)
            .and_then(|stored| AppLanguageCatalog::normalize(&stored))
        {
            return normalized.to_owned();
        }
        let stored_region = defaults.string(REGION_KEY);
        let device_region = rise_platform::device_locale::current().region;
        let region_languages = stored_region
            .or(device_region)
            .map(|region| AppRegionCatalog::languages(&region))
            .unwrap_or_default();

        AppLanguageCatalog::resolve(
            None,
            &AppLanguageCatalog::device_preferences(),
            region_languages,
        )
        .to_owned()
    }

    /// formatting locale for dates and numbers
    fn locale_for(code: &str) -> Locale {
        let identifier = AppLanguageCatalog::language(code)
            .or_else(|| AppLanguageCatalog::language(AppLanguageCatalog::BASE_LANGUAGE))
            .map(|language| language.locale_identifier);
        match identifier {
            Some(identifier) => Locale::new(identifier),
            None => {
                let device = rise_platform::device_locale::current();
                Locale::new(device.languages.first().cloned().unwrap_or_default())
            }
        }
    }
}
