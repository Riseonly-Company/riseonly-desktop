use crate::app_language_catalog::AppLanguageCatalog;
use rise_i18n_catalog::LOCALE_FILE_PREFIX;
use std::path::PathBuf;

const NO_REGION: &[&str] = &[];

#[test]
fn a_device_asking_for_german_gets_german() {
    assert_eq!(AppLanguageCatalog::resolve(None, &["de-DE", "en"], NO_REGION), "de");
    assert_eq!(AppLanguageCatalog::resolve(None, &["de"], NO_REGION), "de");
}

#[test]
fn a_device_asking_only_for_unknown_languages_gets_the_base_language() {
    assert_eq!(AppLanguageCatalog::resolve(None, &["tlh", "xx-YY"], NO_REGION), AppLanguageCatalog::BASE_LANGUAGE);
    assert_eq!(AppLanguageCatalog::resolve(None, &[] as &[&str], NO_REGION), AppLanguageCatalog::BASE_LANGUAGE);
}

#[test]
fn a_known_language_further_down_the_device_list_wins_over_the_base_language() {
    assert_eq!(AppLanguageCatalog::resolve(None, &["tlh", "ja-JP"], NO_REGION), "ja");
}

#[test]
fn the_region_language_is_tried_after_the_device_list() {
    assert_eq!(AppLanguageCatalog::resolve(None, &["tlh"], &["kk", "ru"]), "kk");
}

#[test]
fn an_explicit_choice_is_never_overwritten_by_detection() {
    assert_eq!(AppLanguageCatalog::resolve(Some("ru"), &["de"], &["kk"]), "ru");
    assert_eq!(AppLanguageCatalog::resolve(Some("pt_BR"), &["de"], NO_REGION), "pt-BR");
}

#[test]
fn every_catalogue_language_has_a_locale_file() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/locales");
    if !root.is_dir() {
        return;
    }
    for code in AppLanguageCatalog::codes() {
        let path = root.join(format!("{LOCALE_FILE_PREFIX}{code}.json"));
        assert!(
            path.is_file(),
            "{code} can be resolved but has no file at {}",
            path.display()
        );
    }
}
