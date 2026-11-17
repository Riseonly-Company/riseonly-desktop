use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::OnceLock;

use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_regions::{ASSET_REGION_CODE_LENGTH_CHARS, REGIONAL_INDICATOR_BASE, REGIONS, SubRegion};

use crate::language_store::Locale;

pub type AppSubRegion = SubRegion;

/// a market the user can belong to
/// all fields except the localized name come from canon so server and app agree
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppRegion {
    pub code: &'static str,
    pub english_name: &'static str,
    pub languages: &'static [&'static str],
    pub sub_region: AppSubRegion,
}

impl AppRegion {
    pub fn id(&self) -> &'static str {
        self.code
    }

    /// country name in the user language, english as fallback
    pub fn localized_name(&self) -> String {
        Locale::app_locale()
            .localized_string_for_region_code(self.code)
            .unwrap_or_else(|| self.english_name.to_owned())
    }

    /// emoji flag built from the iso code regional indicator pair
    pub fn flag(&self) -> String {
        self.code
            .chars()
            .map(|c| c.to_ascii_uppercase())
            .filter_map(|c| char::from_u32(REGIONAL_INDICATOR_BASE + u32::from(c)))
            .collect()
    }
}

pub struct AppRegionCatalog;

impl AppRegionCatalog {
    pub fn all() -> &'static [AppRegion] {
        static ALL: OnceLock<Vec<AppRegion>> = OnceLock::new();
        ALL.get_or_init(|| {
            REGIONS
                .iter()
                .map(|region| AppRegion {
                    code: region.code,
                    english_name: region.english_name,
                    languages: region.languages,
                    sub_region: region.sub_region,
                })
                .collect()
        })
    }

    fn index() -> &'static HashMap<&'static str, AppRegion> {
        static INDEX: OnceLock<HashMap<&'static str, AppRegion>> = OnceLock::new();
        INDEX.get_or_init(|| Self::all().iter().map(|region| (region.code, *region)).collect())
    }

    pub fn region(for_code: &str) -> Option<AppRegion> {
        Self::index().get(for_code.to_uppercase().as_str()).copied()
    }

    /// uppercases and checks an iso 3166-1 alpha-2 code against the catalogue
    /// rejects XX and T1 placeholders an edge proxy sends for unknown networks
    pub fn normalize(raw: &str) -> Option<&'static str> {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        if trimmed.chars().count() != ASSET_REGION_CODE_LENGTH_CHARS {
            return None;
        }
        let upper = trimmed.to_uppercase();
        if !upper.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        Self::index().get(upper.as_str()).map(|region| region.code)
    }

    pub fn languages(for_code: &str) -> &'static [&'static str] {
        Self::normalize(for_code)
            .and_then(|code| Self::index().get(code))
            .map(|region| region.languages)
            .unwrap_or_default()
    }

    pub fn ordered_for_display(preferring: &str) -> Vec<AppRegion> {
        let selected = Self::normalize(preferring);
        let mut named: Vec<(AppRegion, String)> = Self::all()
            .iter()
            .map(|region| (*region, region.localized_name()))
            .collect();
        named.sort_by(|(lhs, lhs_name), (rhs, rhs_name)| {
            if Some(lhs.code) == selected {
                return Ordering::Less;
            }
            if Some(rhs.code) == selected {
                return Ordering::Greater;
            }
            compare_case_insensitive(lhs_name, rhs_name)
        });
        named.into_iter().map(|(region, _)| region).collect()
    }
}
