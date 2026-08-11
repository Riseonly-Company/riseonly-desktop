use crate::app_language_catalog::BASE_LANGUAGE;
use crate::app_language_resolution::normalize;

/// The languages this build ships a locale file for — a subset of the catalogue.
///
/// Adding one means an entry here plus its file in `assets/locales`; the
/// `shipped_locales_exist` test fails when the two disagree.
pub const SHIPPED: &[&str] = &["ru"];

/// Fallback interface language. Russian rather than [`BASE_LANGUAGE`]: this build
/// ships no English file, and falling back to one would be a screen of raw keys.
pub const FALLBACK: &str = "ru";

pub fn is_shipped(code: &str) -> bool {
    normalize(code).is_some_and(|code| SHIPPED.contains(&code))
}

/// Picks the interface language for a device, honouring an explicit choice.
///
/// An explicit setting wins and is never overwritten by detection; detection then
/// scans the WHOLE preference list, not just its first entry, before [`FALLBACK`].
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
                crate::app_language_catalog::language(code).is_some(),
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
