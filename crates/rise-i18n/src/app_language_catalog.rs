use std::collections::HashMap;
use std::sync::OnceLock;

use rise_i18n_catalog::{ALIASES, BASE_LANGUAGE, LANGUAGES, PluralFamily};

pub type AppPluralRules = PluralFamily;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppLanguage {
    pub code: &'static str,
    pub native_name: &'static str,
    pub english_name: &'static str,
    pub is_rtl: bool,
    pub locale_identifier: &'static str,
    pub plural: AppPluralRules,
}

impl AppLanguage {
    pub fn id(&self) -> &'static str {
        self.code
    }
}

pub struct AppLanguageCatalog;

impl AppLanguageCatalog {
    pub const BASE_LANGUAGE: &'static str = BASE_LANGUAGE;

    pub fn all() -> &'static [AppLanguage] {
        static ALL: OnceLock<Vec<AppLanguage>> = OnceLock::new();
        ALL.get_or_init(|| {
            LANGUAGES
                .iter()
                .map(|language| AppLanguage {
                    code: language.code,
                    native_name: language.native_name,
                    english_name: language.english_name,
                    is_rtl: language.rtl,
                    locale_identifier: language.locale,
                    plural: language.plural,
                })
                .collect()
        })
    }

    /// alias tags match case insensitively and normalize retries once per truncation step
    /// lowercasing the keys once makes each step a map hit and not a scan of all 29 aliases
    pub fn aliases() -> &'static HashMap<String, &'static str> {
        static ALIAS_INDEX: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
        ALIAS_INDEX.get_or_init(|| {
            ALIASES
                .iter()
                .map(|(tag, code)| (tag.to_lowercase(), *code))
                .collect()
        })
    }

    fn index() -> &'static HashMap<&'static str, AppLanguage> {
        static INDEX: OnceLock<HashMap<&'static str, AppLanguage>> = OnceLock::new();
        INDEX.get_or_init(|| {
            Self::all()
                .iter()
                .map(|language| (language.code, *language))
                .collect()
        })
    }

    pub fn codes() -> &'static [&'static str] {
        static CODES: OnceLock<Vec<&'static str>> = OnceLock::new();
        CODES.get_or_init(|| Self::all().iter().map(|language| language.code).collect())
    }

    pub fn language(for_code: &str) -> Option<AppLanguage> {
        Self::index().get(for_code).copied()
    }
}
