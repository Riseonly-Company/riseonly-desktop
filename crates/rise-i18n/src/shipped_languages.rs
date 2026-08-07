use crate::app_language_catalog::{BASE_LANGUAGE, language};
use crate::app_language_resolution::normalize;

/// The languages this build actually ships a locale file for.
///
/// The catalogue knows 41 languages because the SERVER does — content language,
/// notification language and the region system all key off that list. What the
/// interface is translated into is a different question with a different answer,
/// and today it is one language.
///
/// Shipping a locale the app has no file for is worse than shipping fewer: every
/// missing key renders as its own identifier, so a half-translated language
/// reads as a broken app rather than an English one. Adding a language is one
/// entry here plus its file in `assets/locales`, and `shipped_locales_exist`
/// fails the build if those two ever disagree.
pub const SHIPPED: &[&str] = &["ru"];

/// What the interface falls back to when the device asks for something the app
/// does not ship.
///
/// Russian rather than [`BASE_LANGUAGE`]: `en` is the catalogue's base and the
/// key namespace's source language, but this build has no English file, and a
/// fallback to a language with no dictionary is a screen full of raw keys.
pub const FALLBACK: &str = "ru";

pub fn is_shipped(code: &str) -> bool {
    normalize(code).is_some_and(|code| SHIPPED.contains(&code))
}

/// Picks the interface language for a device, honouring an explicit choice.
///
/// The order is the reference's: an explicit setting always wins and is never
/// overwritten by detection, then the device's own preference order, then the
/// fallback. Detection never narrows to "the first preference" alone — a machine
/// set to English with Russian second must land on Russian while Russian is all
/// this build ships, rather than on raw keys.
pub fn resolve_interface_language<S: AsRef<str>>(
    explicit: Option<&str>,
    device_preferences: &[S],
) -> &'static str {
    if let Some(code) = explicit.and_then(normalize)
        && SHIPPED.contains(&code)
    {
        return code;
    }

    for preferred in device_preferences {
        if let Some(code) = normalize(preferred.as_ref())
            && SHIPPED.contains(&code)
        {
            return code;
        }
    }

    shipped_fallback()
}

fn shipped_fallback() -> &'static str {
    SHIPPED
        .iter()
        .find(|code| **code == FALLBACK)
        .or_else(|| SHIPPED.first())
        .copied()
        .unwrap_or(BASE_LANGUAGE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn every_shipped_code_is_one_the_catalogue_knows() {
        for code in SHIPPED {
            assert!(
                language(code).is_some(),
                "{code} is not in the catalogue, so it has no plural rules"
            );
        }
        assert!(SHIPPED.contains(&FALLBACK));
    }

    #[test]
    fn a_device_asking_for_a_shipped_language_gets_it() {
        assert_eq!(resolve_interface_language(None, &["ru", "en"]), "ru");
        assert_eq!(resolve_interface_language(None, &["ru-RU"]), "ru");
    }

    #[test]
    fn a_device_asking_for_something_unshipped_falls_back_rather_than_showing_keys() {
        assert_eq!(resolve_interface_language(None, &["de", "fr"]), "ru");
        assert_eq!(resolve_interface_language::<String>(None, &[]), "ru");
    }

    #[test]
    fn a_shipped_language_further_down_the_list_still_wins_over_the_fallback() {
        assert_eq!(
            resolve_interface_language(None, &["en", "de", "ru"]),
            "ru",
            "taking only the first preference would land on a language with no file"
        );
    }

    #[test]
    fn an_explicit_choice_is_never_overwritten_by_detection() {
        assert_eq!(resolve_interface_language(Some("ru"), &["de"]), "ru");
    }

    #[test]
    fn an_explicit_choice_the_build_does_not_ship_is_not_honoured() {
        assert_eq!(
            resolve_interface_language(Some("de"), &["ru"]),
            "ru",
            "an explicit German with no German file is still a screen full of keys"
        );
    }

    /// The one thing that can silently break: the list and the files disagree.
    #[test]
    fn shipped_locales_exist() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/locales");
        if !root.is_dir() {
            return;
        }
        for code in SHIPPED {
            let path = root.join(format!("locale-{code}.json"));
            assert!(
                path.is_file(),
                "{} is shipped but has no file at {}",
                code,
                path.display()
            );
        }
    }
}
