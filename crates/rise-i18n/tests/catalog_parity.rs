//! Agreement with `languages.json` and the shipped locale files; both live outside
//! this checkout, so every test here SKIPS rather than fails when they are absent.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use rise_i18n::app_language_catalog::{ALIASES, ALL, PluralRules};
use rise_i18n::{Catalogue, FormatArg};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn locales_dir() -> PathBuf {
    crate_root().join("../../assets/locales")
}

fn languages_json() -> PathBuf {
    crate_root().join("../../../backend/i18n/languages.json")
}

fn skip(reason: &str) {
    println!("SKIPPED: {reason}");
}

#[test]
fn every_catalogue_code_has_a_shipped_locale_file() {
    let dir = locales_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        skip(&format!(
            "{} does not exist yet — locale files are not in the tree",
            dir.display()
        ));
        return;
    };
    let shipped: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    if shipped.is_empty() {
        skip(&format!("{} is empty", dir.display()));
        return;
    }

    let missing: Vec<&str> = ALL
        .iter()
        .map(|language| language.code)
        .filter(|code| !shipped.contains(&format!("locale-{code}.json")))
        .collect();
    assert!(missing.is_empty(), "no locale file for: {missing:?}");
}

#[test]
fn every_shipped_locale_parses_as_a_flat_string_map() {
    let dir = locales_dir();
    if !dir.is_dir() {
        skip(&format!("{} does not exist yet", dir.display()));
        return;
    }

    let mut checked = 0usize;
    for language in ALL {
        let path = dir.join(format!("locale-{}.json", language.code));
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let parsed: Result<BTreeMap<String, String>, _> = serde_json::from_str(&raw);
        assert!(
            parsed.is_ok(),
            "{} is not a flat object of strings: {:?}",
            path.display(),
            parsed.err()
        );
        checked += 1;
    }
    if checked == 0 {
        skip("no locale files present");
    }
}

/// Each formatter is blind to the other's syntax, so a value carrying both
/// dialects would render one placeholder and leave the other on screen.
#[test]
fn no_shipped_value_mixes_the_two_dialects() {
    let dir = locales_dir();
    if !dir.is_dir() {
        skip(&format!("{} does not exist yet", dir.display()));
        return;
    }

    let markers: Vec<FormatArg<'static>> = (0..9).map(|_| FormatArg::from("\u{1}")).collect();
    let mut checked = 0usize;
    for language in ALL {
        let path = dir.join(format!("locale-{}.json", language.code));
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let dict: BTreeMap<String, String> = serde_json::from_str(&raw).expect("flat string map");
        for (key, value) in &dict {
            let printf = rise_i18n::format_printf(value, &markers) != *value;
            assert!(
                !(printf && has_named_placeholder(value)),
                "{}: {key} mixes printf and braced interpolation: {value:?}",
                path.display()
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no locale files present");
}

fn has_named_placeholder(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut at = 0usize;
    while at < bytes.len() {
        if bytes[at] != b'{' {
            at += 1;
            continue;
        }
        let mut cursor = at + 1;
        while bytes
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            cursor += 1;
        }
        if cursor > at + 1 && bytes.get(cursor) == Some(&b'}') {
            return true;
        }
        at += 1;
    }
    false
}

/// The `chat_thread_title_*` family ships no `other`, so a language whose CLDR
/// family returns `other` has nothing to fall back to.
#[test]
fn the_thread_title_family_renders_from_the_shipped_catalogue() {
    let dir = locales_dir();
    if !dir.join("locale-ru.json").is_file() {
        skip(&format!("{} has no locale files yet", dir.display()));
        return;
    }

    let catalogue = Catalogue::directory(&dir);

    let ru = catalogue.load("ru");
    for (count, want) in [
        (1_i64, "1 комментарий"),
        (3, "3 комментария"),
        (7, "7 комментариев"),
        (21, "21 комментарий"),
    ] {
        assert_eq!(
            ru.translate_plural_named("chat_thread_title", count, &[("count", &count.to_string())]),
            want
        );
    }

    let en = catalogue.load("en");
    assert_eq!(
        en.translate_plural_named("chat_thread_title", 1, &[("count", "1")]),
        "1 comment"
    );
    assert_eq!(
        en.translate_plural_named("chat_thread_title", 5, &[("count", "5")]),
        "chat_thread_title_other",
        "en resolves to the `other` category, which this family does not define"
    );
}

/// A round trip through the real English file, in both dialects.
#[test]
fn shipped_english_renders_both_dialects() {
    let dir = locales_dir();
    if !dir.join("locale-en.json").is_file() {
        skip(&format!("{} has no locale files yet", dir.display()));
        return;
    }

    let en = Catalogue::directory(&dir).load("en");
    assert_eq!(
        en.translate_args(
            "notify_names_more",
            &[
                FormatArg::from("Ada"),
                FormatArg::from("Bob"),
                FormatArg::Int(3)
            ]
        ),
        "Ada, Bob and 3 more"
    );
    assert_eq!(
        en.translate_args(
            "duration_hours_minutes_abbr",
            &[FormatArg::Int(2), FormatArg::Int(5)]
        ),
        "2 h 5 min"
    );
    assert_eq!(
        en.translate_named("chat_delete_for_both", &[("name", "Ada")]),
        "Delete for me and Ada"
    );
}

#[test]
fn the_generated_catalogue_matches_languages_json() {
    let source = languages_json();
    if !source.is_file() {
        skip(&format!(
            "{} is not present — the backend tree is not checked out beside this one",
            source.display()
        ));
        return;
    }

    let raw = std::fs::read_to_string(&source).expect("read languages.json");
    let doc: serde_json::Value = serde_json::from_str(&raw).expect("parse languages.json");

    let languages = doc["languages"].as_array().expect("languages array");
    assert_eq!(languages.len(), ALL.len());
    for (declared, ported) in languages.iter().zip(ALL) {
        assert_eq!(declared["code"], ported.code);
        assert_eq!(declared["native"], ported.native_name);
        assert_eq!(declared["english"], ported.english_name);
        assert_eq!(declared["rtl"], ported.is_rtl);
        assert_eq!(declared["locale"], ported.locale_identifier);
        assert_eq!(
            PluralRules::from_name(declared["plural"].as_str().expect("plural family")),
            Some(ported.plural),
            "{} declares a plural family the port does not implement",
            ported.code
        );
    }

    let aliases = doc["aliases"].as_object().expect("aliases object");
    assert_eq!(aliases.len(), ALIASES.len());
    for (alias, target) in aliases {
        assert_eq!(
            rise_i18n::alias_target(alias),
            target.as_str(),
            "alias {alias}"
        );
    }
}

/// A hand edit of `app_language_catalog.rs` is a defect the same way a stale one is.
#[test]
fn the_generator_reproduces_the_checked_in_catalogue() {
    let source = languages_json();
    let generator = crate_root().join("tools/gen_catalog.py");
    if !source.is_file() || !generator.is_file() {
        skip("languages.json or the generator is not present");
        return;
    }
    let Ok(output) = Command::new("python3")
        .arg(&generator)
        .arg(&source)
        .arg("--check")
        .output()
    else {
        skip("python3 is not available");
        return;
    };
    assert!(
        output.status.success(),
        "{} is stale — rerun tools/gen_catalog.py\n{}",
        Path::new("src/app_language_catalog.rs").display(),
        String::from_utf8_lossy(&output.stdout)
    );
}
