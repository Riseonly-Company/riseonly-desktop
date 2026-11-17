use std::cmp::Ordering;
use std::collections::HashMap;

use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_i18n_catalog::{POPULAR_CODES, TAG_POSIX_SEPARATOR};
use rise_regions::{ASSET_REGION_CODE_LENGTH_CHARS, REGIONAL_INDICATOR_BASE};

use crate::app_language_catalog::{AppLanguage, AppLanguageCatalog};

const NO_FLAG: &str = "\u{1F3F3}\u{FE0F}";

impl AppLanguage {
    /// flag from the language formatting locale (eg pt_BR gives BR)
    /// not from a second hand kept table
    pub fn flag(&self) -> String {
        let Some(candidate) = self.locale_identifier.split(TAG_POSIX_SEPARATOR).last() else {
            return NO_FLAG.to_string();
        };
        let is_region = candidate.chars().count() == ASSET_REGION_CODE_LENGTH_CHARS
            && candidate.chars().all(|c| c.is_ascii_uppercase());
        if !is_region {
            return NO_FLAG.to_string();
        }

        let out: String = candidate
            .chars()
            .filter_map(|scalar| char::from_u32(REGIONAL_INDICATOR_BASE + u32::from(scalar)))
            .collect();
        if out.is_empty() { NO_FLAG.to_string() } else { out }
    }
}

impl AppLanguageCatalog {
    pub const POPULAR_CODES: &'static [&'static str] = &POPULAR_CODES;

    pub fn ordered_for_display() -> Vec<AppLanguage> {
        let rank: HashMap<&str, usize> = Self::POPULAR_CODES
            .iter()
            .enumerate()
            .map(|(offset, code)| (*code, offset))
            .collect();
        let mut out: Vec<AppLanguage> = Self::all().to_vec();
        out.sort_by(|lhs, rhs| match (rank.get(lhs.code), rank.get(rhs.code)) {
            (Some(l), Some(r)) => l.cmp(r),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => compare_case_insensitive(lhs.native_name, rhs.native_name),
        });
        out
    }

    /// maps any system language tag to a shipped catalogue code
    /// eg ru_RU and iw and in and zh-TW to zh-Hant
    /// none when nothing maps
    pub fn normalize(raw: &str) -> Option<&'static str> {
        rise_i18n_catalog::normalize(raw)
    }

    /// catalogue lookup order, chosen language then base of a regional variant (pt-BR to pt)
    /// then the app base language
    pub fn lookup_chain(for_code: &str) -> Vec<&'static str> {
        rise_i18n_catalog::lookup_chain(for_code)
    }

    /// device preferred languages narrowed to what the app ships, in system order
    pub fn device_preferences() -> Vec<&'static str> {
        let device = rise_platform::device_locale::current();
        let tags: Vec<&str> = device.languages.iter().map(String::as_str).collect();
        rise_i18n_catalog::narrow(&tags)
    }

    /// full language resolution in canon order, explicit choice then device order
    /// then region primary language then base language
    /// an explicit choice stops everything below it
    pub fn resolve<D: AsRef<str>, R: AsRef<str>>(
        explicit: Option<&str>,
        device_preferences: &[D],
        region_languages: &[R],
    ) -> &'static str {
        let device: Vec<&str> = device_preferences.iter().map(AsRef::as_ref).collect();
        let region: Vec<&str> = region_languages.iter().map(AsRef::as_ref).collect();
        rise_i18n_catalog::resolve(explicit, &device, &region)
    }
}
