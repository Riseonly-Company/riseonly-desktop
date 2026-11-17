use super::device_locale::DeviceLocale;
use super::posix_locale::*;

fn environment(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_owned())
    }
}

#[test]
fn language_comes_before_the_locale_of_messages() {
    let identifiers = gettext_identifiers(environment(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", "ru:en")]));
    assert_eq!(identifiers, vec!["ru", "en", "en_US.UTF-8"]);
    let locale = DeviceLocale::parse(&identifiers);
    assert_eq!(locale.languages, vec!["ru", "en"], "the interface language chosen in LANGUAGE leads");
    assert_eq!(locale.region.as_deref(), Some("US"));
}

#[test]
fn the_first_non_empty_messages_variable_wins() {
    let identifiers = gettext_identifiers(environment(&[
        ("LC_ALL", ""),
        ("LC_MESSAGES", "kk_KZ.UTF-8"),
        ("LANG", "en_US.UTF-8"),
    ]));
    assert_eq!(identifiers, vec!["kk_KZ.UTF-8"], "LANG is not read once LC_MESSAGES is set");
}

#[test]
fn lc_all_overrides_the_others() {
    let identifiers = gettext_identifiers(environment(&[("LC_ALL", "de_DE.UTF-8"), ("LANG", "en_US.UTF-8")]));
    assert_eq!(identifiers, vec!["de_DE.UTF-8"]);
}

#[test]
fn the_c_locale_ignores_language() {
    assert!(gettext_identifiers(environment(&[("LANG", "C.UTF-8"), ("LANGUAGE", "ru")])).is_empty());
    assert!(gettext_identifiers(environment(&[("LC_ALL", "POSIX"), ("LANGUAGE", "ru")])).is_empty());
}

#[test]
fn no_locale_of_messages_names_nothing() {
    assert!(gettext_identifiers(environment(&[("LANGUAGE", "ru")])).is_empty());
    assert!(gettext_identifiers(environment(&[])).is_empty());
}

#[test]
fn empty_language_entries_are_skipped_and_the_messages_locale_is_not_repeated() {
    let identifiers = gettext_identifiers(environment(&[("LANG", "ru_RU.UTF-8"), ("LANGUAGE", "ru_RU.UTF-8::en")]));
    assert_eq!(identifiers, vec!["ru_RU.UTF-8", "en"]);
}

#[test]
fn untranslated_names_are_recognised_with_an_encoding() {
    assert!(is_untranslated("C"));
    assert!(is_untranslated("c.utf8"));
    assert!(is_untranslated("POSIX"));
    assert!(!is_untranslated("ca_ES.UTF-8"), "Catalan starts with C but is a language");
}
