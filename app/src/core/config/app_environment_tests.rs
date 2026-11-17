use rise_deeplink::{APP_ENV_FALLBACK, ENVIRONMENTS};

use super::*;

#[test]
fn every_variant_names_its_own_canon_row_in_canon_order() {
    assert_eq!(AppEnvironment::ALL.len(), ENVIRONMENTS.len());
    for (index, environment) in AppEnvironment::ALL.into_iter().enumerate() {
        assert_eq!(environment.canon(), &ENVIRONMENTS[index]);
        assert_eq!(AppEnvironment::from_canon(environment.canon()), Some(environment));
    }
}

#[test]
fn the_names_are_the_ios_raw_values() {
    let names: Vec<&str> = AppEnvironment::ALL.iter().map(|e| e.as_str()).collect();
    assert_eq!(names, vec!["dev", "staging", "production"]);
    assert_eq!(AppEnvironment::Prod.to_string(), "production");
}

#[test]
fn scheme_and_host_come_from_the_canon_row() {
    for environment in AppEnvironment::ALL {
        assert_eq!(environment.url_scheme(), environment.canon().url_scheme);
        assert_eq!(environment.web_host(), environment.canon().web_host);
        assert!(environment.accepts_web_host(environment.web_host()));
    }
    assert!(AppEnvironment::Prod.accepts_web_host("WWW.riseonly.net"));
    assert!(!AppEnvironment::Dev.accepts_web_host(AppEnvironment::Prod.web_host()));
}

#[test]
fn every_canon_spelling_parses_exactly() {
    for environment in AppEnvironment::ALL {
        for raw in environment.canon().app_env_values {
            assert_eq!(AppEnvironment::parse(raw), Some(environment), "{raw}");
        }
    }
    assert_eq!(AppEnvironment::parse("PRODUCTION"), None);
    assert_eq!(AppEnvironment::parse(" dev "), None);
    assert_eq!(AppEnvironment::parse("nonsense"), None);
}

#[test]
fn every_environment_is_nameable_and_distinguishable_at_a_glance() {
    let names: Vec<&str> = AppEnvironment::ALL
        .iter()
        .map(|environment| environment.display_name())
        .collect();

    assert_eq!(names, vec!["Riseonly Dev", "Riseonly Staging", "Riseonly"]);
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), names.len());
}

#[test]
fn the_bundle_identity_is_never_shared_between_environments() {
    let script = std::fs::read_to_string(
        std::path::PathBuf::from(env!("RISE_WORKSPACE_DIR")).join("scripts/bundle-macos.sh"),
    )
    .unwrap_or_default();
    if script.is_empty() {
        return;
    }

    for id in [
        "net.riseonly.desktop.dev",
        "net.riseonly.desktop.staging",
        "net.riseonly.desktop\"",
    ] {
        assert!(
            script.contains(id.trim_end_matches('"')),
            "{id} is not written by the bundling script; two environments would \
             share a Keychain ACL and a deep-link registration"
        );
    }
}

#[test]
fn each_environment_owns_a_distinct_url_scheme() {
    let schemes: Vec<&str> = AppEnvironment::ALL.iter().map(|e| e.url_scheme()).collect();
    let mut unique = schemes.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        schemes.len(),
        "two environments sharing a scheme would steal each other's deep links"
    );
}

#[test]
fn each_environment_owns_a_distinct_data_directory() {
    let suffixes: Vec<&str> = AppEnvironment::ALL.iter().map(|e| e.data_dir_suffix()).collect();
    let mut unique = suffixes.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), suffixes.len());
}

#[test]
fn production_refuses_to_be_repointed() {
    assert!(!AppEnvironment::Prod.allows_local_override());
    assert!(AppEnvironment::Dev.allows_local_override());
    assert!(AppEnvironment::Staging.allows_local_override());
}

#[test]
fn the_default_and_the_canon_server_fallback_are_dev() {
    assert_eq!(AppEnvironment::default(), AppEnvironment::Dev);
    assert_eq!(AppEnvironment::from_canon(APP_ENV_FALLBACK), Some(AppEnvironment::Dev));
}

#[test]
fn a_build_value_is_trimmed_and_lowercased_like_ios() {
    for debug_build in [true, false] {
        for (raw, expected) in [
            (" Staging\n", AppEnvironment::Staging),
            ("PROD", AppEnvironment::Prod),
            ("Development", AppEnvironment::Dev),
        ] {
            assert_eq!(AppEnvironment::from_build_value(Some(raw), debug_build), expected);
        }
    }
}

#[test]
fn an_unknown_build_value_is_dev_only_in_a_debug_build() {
    for raw in [None, Some(""), Some("  "), Some("nonsense")] {
        assert_eq!(AppEnvironment::from_build_value(raw, true), AppEnvironment::Dev, "{raw:?}");
        assert_eq!(AppEnvironment::from_build_value(raw, false), AppEnvironment::Prod, "{raw:?}");
    }
}

#[test]
fn the_packaged_environment_wins_over_the_build_value() {
    assert_eq!(AppEnvironment::packaged_or_compiled(Some("staging\n")), AppEnvironment::Staging);
    assert_eq!(AppEnvironment::packaged_or_compiled(Some(" Production ")), AppEnvironment::Prod);
}

#[test]
fn a_package_without_a_known_environment_keeps_the_build_value() {
    assert_eq!(AppEnvironment::packaged_or_compiled(None), AppEnvironment::compiled());
    assert_eq!(AppEnvironment::packaged_or_compiled(Some("moon")), AppEnvironment::compiled());
}
