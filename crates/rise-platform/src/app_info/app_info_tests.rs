use crate::app_info::*;

const BUNDLE_SCRIPT: &str = include_str!("../../../../scripts/bundle-macos.sh");
const PLIST_TEMPLATE: &str = include_str!("../../../../app/resources/macos/Info.plist.in");

fn bundle(version: &str, identifier: &str) -> BundleValues {
    BundleValues {
        short_version: Some(version.to_owned()),
        build: Some("42".to_owned()),
        bundle_identifier: Some(identifier.to_owned()),
        display_name: Some("Riseonly Dev".to_owned()),
    }
}

#[test]
fn the_table_follows_the_canon_environments() {
    let names: Vec<&str> = rise_deeplink::ENVIRONMENTS.iter().map(|row| row.name).collect();
    let table: Vec<&str> = IDENTITIES.iter().map(|identity| identity.environment).collect();
    assert_eq!(table, names, "one identity per canon environment, in canon order");
}

#[test]
fn every_identity_is_distinct() {
    for (index, identity) in IDENTITIES.iter().enumerate() {
        for other in &IDENTITIES[index + 1..] {
            assert_ne!(identity.bundle_identifier, other.bundle_identifier);
            assert_ne!(identity.display_name, other.display_name);
        }
    }
}

#[test]
fn the_bundle_script_writes_the_same_identities() {
    for identity in IDENTITIES {
        let arm = format!("{})", identity.environment);
        let name = format!("APP_NAME=\"{}\";", identity.display_name);
        let id = format!("BUNDLE_ID=\"{}\"", identity.bundle_identifier);
        assert!(
            BUNDLE_SCRIPT.lines().any(|candidate| {
                candidate.split_whitespace().next() == Some(arm.as_str())
                    && candidate.contains(&name)
                    && candidate.contains(&id)
            }),
            "scripts/bundle-macos.sh lost the {} identity: {name} {id}",
            identity.environment
        );
    }
}

#[test]
fn the_plist_template_takes_version_and_build_from_the_bundler() {
    for placeholder in [
        "<string>@SHORT_VERSION@</string>",
        "<string>@BUILD_NUMBER@</string>",
        "<string>@BUNDLE_ID@</string>",
        "<string>@APP_NAME@</string>",
    ] {
        assert!(
            PLIST_TEMPLATE.contains(placeholder),
            "Info.plist.in must not hard-code what the table owns: {placeholder}"
        );
    }
    assert!(
        BUNDLE_SCRIPT.contains(&format!("BUILD_NUMBER=\"${{RISE_BUILD_NUMBER:-{BUILD_NUMBER_FALLBACK}}}\"")),
        "the bundler falls back to the same build number as the compiled table"
    );
}

#[test]
fn identity_lookup_is_exact() {
    assert_eq!(
        identity_for("production").map(|identity| identity.bundle_identifier),
        Some("net.riseonly.desktop")
    );
    assert_eq!(identity_for("Production"), None);
    assert_eq!(identity_for("prod"), None, "APP_ENV spellings resolve in the app first");
}

#[test]
fn the_marketing_version_is_the_workspace_version() {
    let parts: Vec<&str> = MARKETING_VERSION.split('.').collect();
    assert_eq!(parts.len(), 3, "major.minor.patch, got {MARKETING_VERSION}");
    assert!(parts.iter().all(|part| part.parse::<u32>().is_ok()));
}

#[test]
fn build_numbers_follow_the_cfbundleversion_format() {
    assert_eq!(build_number_from(Some("120")), "120");
    assert_eq!(build_number_from(Some("2.0.7")), "2.0.7");
    for bad in [None, Some(""), Some(" 12"), Some("12a"), Some("1..2"), Some(".1")] {
        assert_eq!(build_number_from(bad), BUILD_NUMBER_FALLBACK, "{bad:?}");
    }
}

#[test]
fn a_bare_binary_answers_from_the_table() {
    let identity = &IDENTITIES[0];
    let info = AppInfo::resolve(identity, BundleValues::default());
    assert_eq!(info, AppInfo::compiled(identity));
    assert_eq!(info.source, AppInfoSource::Compiled);
    assert_eq!(info.short_version, MARKETING_VERSION);
    assert_eq!(info.bundle_identifier, "net.riseonly.desktop.dev");
}

#[test]
fn a_folder_bundle_without_an_identifier_is_not_the_app() {
    let mut values = bundle("9.9.9", "net.riseonly.desktop.dev");
    values.bundle_identifier = None;
    assert_eq!(
        AppInfo::resolve(&IDENTITIES[0], values).source,
        AppInfoSource::Compiled
    );
}

#[test]
fn a_launched_bundle_answers_from_its_info_plist() {
    let info = AppInfo::resolve(&IDENTITIES[0], bundle("9.9.9", "net.riseonly.desktop.dev"));
    assert_eq!(info.source, AppInfoSource::InfoPlist);
    assert_eq!(info.short_version, "9.9.9");
    assert_eq!(info.build, "42");
}

#[test]
fn a_missing_plist_key_is_filled_from_the_table() {
    let mut values = bundle("9.9.9", "net.riseonly.desktop.dev");
    values.build = Some(String::new());
    values.display_name = None;
    let info = AppInfo::resolve(&IDENTITIES[0], values);
    assert_eq!(info.build, compiled_build_number());
    assert_eq!(info.display_name, "Riseonly Dev");
}

#[test]
fn an_unknown_environment_has_no_app_info() {
    assert_eq!(AppInfo::current("qa"), None);
}

#[test]
fn the_current_app_always_has_a_version() {
    let info = AppInfo::current("dev").expect("dev is in the table");
    assert!(!info.short_version.is_empty());
    assert!(!info.bundle_identifier.is_empty());
}
