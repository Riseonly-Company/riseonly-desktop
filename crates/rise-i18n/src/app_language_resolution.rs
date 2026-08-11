//! Language tag handling; must stay in agreement with `backend/common/src/i18n/mod.rs`.

use crate::app_language_catalog::{ALL, AppLanguage, BASE_LANGUAGE, alias_target, language};

/// The languages a picker offers first; everything else follows, sorted by native name.
pub static POPULAR_CODES: &[&str] = &[
    "en", "ru", "es", "pt-BR", "de", "fr", "it", "tr", "ar", "hi", "zh-Hans", "ja", "ko", "id",
    "uk", "kk",
];

impl AppLanguage {
    /// Region subtag of the language's own formatting locale (`pt_BR` -> `BR`,
    /// `zh_Hant_TW` -> `TW`).
    pub fn region_code(&self) -> Option<&'static str> {
        let candidate = self.locale_identifier.rsplit('_').next()?;
        let is_region =
            candidate.len() == 2 && candidate.bytes().all(|byte| byte.is_ascii_uppercase());
        is_region.then_some(candidate)
    }
}

/// [`POPULAR_CODES`] first, then the rest by a case-insensitive native-name comparison.
pub fn ordered_for_display() -> Vec<&'static AppLanguage> {
    let rank = |code: &str| POPULAR_CODES.iter().position(|popular| *popular == code);

    let mut out: Vec<&'static AppLanguage> = ALL.iter().collect();
    out.sort_by(|lhs, rhs| match (rank(lhs.code), rank(rhs.code)) {
        (Some(l), Some(r)) => l.cmp(&r),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => lhs
            .native_name
            .to_lowercase()
            .cmp(&rhs.native_name.to_lowercase()),
    });
    out
}

/// Catalogue casing: language lowercase, script title case, region uppercase.
fn canonical_case(tag: &str) -> String {
    tag.split('-')
        .enumerate()
        .map(|(index, part)| {
            if index == 0 {
                return part.to_lowercase();
            }
            if part.chars().count() == 4 && part.chars().all(char::is_alphabetic) {
                let mut chars = part.chars();
                let head = chars.next().expect("four characters").to_uppercase();
                return head.chain(chars.flat_map(char::to_lowercase)).collect();
            }
            part.to_uppercase()
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// Map any system-supplied language tag onto a shipped catalogue code.
///
/// POSIX underscores (`ru_RU`), deprecated ISO codes (`iw`, `in`), script and
/// region variants (`zh-TW` -> `zh-Hant`) and progressive truncation
/// (`de-AT-1996` -> `de`). `None` when nothing in the tag maps to a shipped language.
pub fn normalize(raw: &str) -> Option<&'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 35 {
        return None;
    }
    if trimmed.eq_ignore_ascii_case("und") || trimmed.eq_ignore_ascii_case("mul") {
        return None;
    }

    let mut tag = canonical_case(&trimmed.replace('_', "-"));

    loop {
        if let Some(entry) = language(&tag) {
            return Some(entry.code);
        }
        if let Some(target) = alias_target(&tag) {
            return Some(target);
        }
        let cut = tag.rfind('-')?;
        tag.truncate(cut);
    }
}

/// Catalogue lookup order: the chosen language, then the base language of a
/// regional variant (`pt-BR` -> `pt`), then the app base language.
pub fn lookup_chain(code: &str) -> Vec<&'static str> {
    let resolved = normalize(code).unwrap_or(BASE_LANGUAGE);
    let mut chain = vec![resolved];

    if let Some(dash) = resolved.find('-') {
        let base = &resolved[..dash];
        if let Some(entry) = language(base) {
            chain.push(entry.code);
        }
    }
    if !chain.contains(&BASE_LANGUAGE) {
        chain.push(BASE_LANGUAGE);
    }
    chain
}

/// The caller-supplied preferred languages, narrowed to what the app ships and
/// deduplicated, in the order the OS reported them.
pub fn device_preferences<S: AsRef<str>>(raw: &[S]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for preferred in raw {
        let Some(code) = normalize(preferred.as_ref()) else {
            continue;
        };
        if !out.contains(&code) {
            out.push(code);
        }
    }
    out
}

/// Explicit choice, then the device's own order, then the region's primary
/// language, then [`BASE_LANGUAGE`].
pub fn resolve<D: AsRef<str>, R: AsRef<str>>(
    explicit: Option<&str>,
    device_preferences: &[D],
    region_languages: &[R],
) -> &'static str {
    if let Some(code) = explicit.and_then(normalize) {
        return code;
    }
    for candidate in device_preferences {
        if let Some(code) = normalize(candidate.as_ref()) {
            return code;
        }
    }
    for candidate in region_languages {
        if let Some(code) = normalize(candidate.as_ref()) {
            return code;
        }
    }
    BASE_LANGUAGE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_language_catalog::{ALIASES, PluralRules, codes};

    #[test]
    fn catalogue_ships_forty_one_languages() {
        assert_eq!(ALL.len(), 41);
        assert_eq!(codes().count(), 41);
        assert!(language(BASE_LANGUAGE).is_some());
    }

    #[test]
    fn every_alias_points_at_a_shipped_language() {
        for (alias, target) in ALIASES {
            assert!(
                language(target).is_some(),
                "alias {alias} -> {target} is not a shipped language"
            );
            assert!(
                language(alias).is_none(),
                "{alias} is both a language and an alias"
            );
        }
    }

    #[test]
    fn plural_family_is_declared_for_every_language() {
        for entry in ALL {
            assert_eq!(
                PluralRules::from_name(entry.plural.as_str()),
                Some(entry.plural),
                "{} round-trips through its family name",
                entry.code
            );
        }
    }

    #[test]
    fn posix_underscores_normalize() {
        assert_eq!(normalize("ru_RU"), Some("ru"));
        assert_eq!(normalize("en_US"), Some("en"));
        assert_eq!(normalize("zh_hans_cn"), Some("zh-Hans"));
        assert_eq!(normalize("pt_BR"), Some("pt-BR"));
    }

    #[test]
    fn deprecated_iso_codes_normalize() {
        for (raw, want) in [
            ("iw", "he"),
            ("in", "id"),
            ("ji", "he"),
            ("nb", "no"),
            ("nn", "no"),
            ("fil", "tl"),
            ("mo", "ro"),
            ("sh", "sr"),
            ("hr", "sr"),
            ("bs", "sr"),
        ] {
            assert_eq!(normalize(raw), Some(want), "{raw}");
        }
    }

    #[test]
    fn script_and_region_variants_normalize() {
        assert_eq!(normalize("zh-TW"), Some("zh-Hant"));
        assert_eq!(normalize("zh-HK"), Some("zh-Hant"));
        assert_eq!(normalize("zh"), Some("zh-Hans"));
        assert_eq!(normalize("zh-Hans-CN"), Some("zh-Hans"));
        assert_eq!(normalize("pt-PT"), Some("pt"));
        assert_eq!(normalize("pt-BR"), Some("pt-BR"));
        assert_eq!(normalize("sr-Latn"), Some("sr"));
        assert_eq!(normalize("sr-latn"), Some("sr"));
        assert_eq!(normalize("es-419"), Some("es"));
    }

    #[test]
    fn progressive_truncation_reaches_the_base_language() {
        assert_eq!(normalize("de-AT-1996"), Some("de"));
        assert_eq!(normalize("en-GB-oxendict"), Some("en"));
        assert_eq!(normalize("fr-CA"), Some("fr"));
    }

    #[test]
    fn undetermined_and_junk_tags_are_rejected() {
        assert_eq!(normalize("und"), None);
        assert_eq!(normalize("UND"), None);
        assert_eq!(normalize("mul"), None);
        assert_eq!(normalize(""), None);
        assert_eq!(normalize("   "), None);
        assert_eq!(normalize("klingon"), None);
        assert_eq!(normalize(&"x".repeat(36)), None);
        assert_eq!(
            normalize("en-GB-oxendict-with-a-very-long-tail"),
            None,
            "over-long tags are rejected before any truncation"
        );
    }

    #[test]
    fn whitespace_is_trimmed_before_matching() {
        assert_eq!(normalize("  ru_RU \n"), Some("ru"));
    }

    #[test]
    fn lookup_chain_walks_variant_then_base_then_english() {
        assert_eq!(lookup_chain("pt-BR"), vec!["pt-BR", "pt", "en"]);
        assert_eq!(lookup_chain("ru"), vec!["ru", "en"]);
        assert_eq!(lookup_chain("en"), vec!["en"]);
        assert_eq!(
            lookup_chain("zh-Hans"),
            vec!["zh-Hans", "en"],
            "zh is an alias, not a shipped language, so it is not a fallback"
        );
        assert_eq!(lookup_chain("klingon"), vec!["en"]);
    }

    #[test]
    fn every_chain_ends_at_the_base_language() {
        for code in codes() {
            let chain = lookup_chain(code);
            assert_eq!(
                *chain.last().expect("non-empty chain"),
                BASE_LANGUAGE,
                "{code}"
            );
            assert_eq!(chain[0], code);
        }
    }

    #[test]
    fn device_preferences_are_deduplicated_in_order() {
        let raw = ["en-GB", "en_US", "ru-RU", "klingon", "pt-BR", "pt_PT"];
        assert_eq!(device_preferences(&raw), vec!["en", "ru", "pt-BR", "pt"]);
    }

    #[test]
    fn explicit_choice_wins_over_the_device() {
        assert_eq!(resolve(Some("de"), &["ru"], &["kk"]), "de");
        assert_eq!(
            resolve(Some("klingon"), &["ru"], &["kk"]),
            "ru",
            "an unshippable explicit choice falls through instead of pinning"
        );
        assert_eq!(resolve(None, &["klingon"], &["kk"]), "kk");
        assert_eq!(resolve(None::<&str>, &[] as &[&str], &[] as &[&str]), "en");
    }

    #[test]
    fn region_code_comes_from_the_formatting_locale() {
        assert_eq!(language("pt-BR").unwrap().region_code(), Some("BR"));
        assert_eq!(language("zh-Hant").unwrap().region_code(), Some("TW"));
        assert_eq!(language("en").unwrap().region_code(), Some("US"));
        assert_eq!(language("tl").unwrap().region_code(), Some("PH"));
    }

    #[test]
    fn display_order_puts_popular_languages_first() {
        let ordered = ordered_for_display();
        assert_eq!(ordered.len(), ALL.len());
        let head: Vec<&str> = ordered
            .iter()
            .take(POPULAR_CODES.len())
            .map(|entry| entry.code)
            .collect();
        assert_eq!(head, POPULAR_CODES.to_vec());
    }
}
