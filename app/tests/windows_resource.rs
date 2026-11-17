#[path = "../../scripts/windows_resource.rs"]
mod windows_resource;

use rise_platform::app_info::{IDENTITIES, MARKETING_VERSION, build_number_from};
use riseonly::core::config::app_environment::AppEnvironment;
use windows_resource::*;

const TEMPLATE: &str = include_str!("../resources/windows/riseonly.rc.in");
const ICON: &[u8] = include_bytes!("../resources/windows/app.ico");

const RAW_ENVIRONMENTS: [Option<&str>; 9] = [
    None,
    Some(""),
    Some("dev"),
    Some("staging"),
    Some("production"),
    Some(" Production\n"),
    Some("STAGING"),
    Some("prod"),
    Some("nonsense"),
];

fn values(rise_env: Option<&str>, build: Option<&str>, debug_build: bool) -> ResourceValues {
    ResourceValues::resolve(rise_env, build, debug_build, ["0.1.0", "0", "1", "0"])
        .expect("every canon environment has a display name")
}

#[test]
fn the_environment_is_the_one_the_binary_talks_to() {
    for raw in RAW_ENVIRONMENTS {
        for debug_build in [true, false] {
            assert_eq!(
                environment_for_build(raw, debug_build).name,
                AppEnvironment::from_build_value(raw, debug_build).as_str(),
                "RISE_ENV {raw:?}, debug {debug_build}"
            );
        }
    }
}

#[test]
fn every_identity_has_its_display_name() {
    for identity in IDENTITIES {
        assert_eq!(display_name(identity.environment), Some(identity.display_name));
    }
    for environment in rise_deeplink::ENVIRONMENTS.iter() {
        assert!(
            display_name(environment.name).is_some(),
            "{} has no display name",
            environment.name
        );
    }
    for environment in AppEnvironment::ALL {
        assert_eq!(display_name(environment.as_str()), Some(environment.display_name()));
    }
}

#[test]
fn the_build_number_follows_app_info() {
    for raw in [
        None,
        Some(""),
        Some("42"),
        Some("1.2.3"),
        Some("1..2"),
        Some(".1"),
        Some("1."),
        Some("12a"),
        Some("-3"),
    ] {
        assert_eq!(build_number(raw), build_number_from(raw), "RISE_BUILD_NUMBER {raw:?}");
    }
}

#[test]
fn the_file_version_carries_version_and_build() {
    assert_eq!(file_version("0", "1", "0", "42"), [0, 1, 0, 42]);
    assert_eq!(file_version("2", "10", "3", "7.4.1"), [2, 10, 3, 7]);
    assert_eq!(file_version("1", "0", "0", "70000"), [1, 0, 0, u16::MAX]);
    assert_eq!(file_version("1", "0", "0", BUILD_NUMBER_FALLBACK), [1, 0, 0, 1]);
}

#[test]
fn the_product_version_is_the_marketing_version() {
    let resolved = ResourceValues::resolve(
        Some("production"),
        None,
        false,
        [
            MARKETING_VERSION,
            env!("CARGO_PKG_VERSION_MAJOR"),
            env!("CARGO_PKG_VERSION_MINOR"),
            env!("CARGO_PKG_VERSION_PATCH"),
        ],
    )
    .expect("production has a display name");
    assert_eq!(resolved.short_version, MARKETING_VERSION);
    assert_eq!(resolved.app_name, "Riseonly");
}

#[test]
fn the_template_renders_every_field() {
    let rendered = values(Some("staging"), Some("42"), true)
        .render(TEMPLATE, r"C:\src\app\resources\windows\app.ico")
        .expect("the template renders");
    assert!(!rendered.contains('@'), "a placeholder stayed: {rendered}");
    assert!(rendered.contains(r#"1 ICON "C:/src/app/resources/windows/app.ico""#));
    assert!(rendered.contains("FILEVERSION 0,1,0,42"));
    assert!(rendered.contains("PRODUCTVERSION 0,1,0,42"));
    assert!(rendered.contains("FILEFLAGS 0x1L"));
    assert!(rendered.contains(r#"VALUE "ProductName", "Riseonly Staging""#));
    assert!(rendered.contains(r#"VALUE "FileVersion", "0.1.0 (42)""#));
    assert!(rendered.contains(r#"VALUE "ProductVersion", "0.1.0""#));
    assert!(rendered.contains(r#"VALUE "OriginalFilename", "riseonly.exe""#));

    let release = values(None, None, false)
        .render(TEMPLATE, "/ws/app.ico")
        .expect("the template renders");
    assert!(release.contains("FILEFLAGS 0x0L"));
    assert!(release.contains(r#"VALUE "ProductName", "Riseonly""#));
}

#[test]
fn a_path_with_an_at_sign_still_renders() {
    let rendered = values(Some("dev"), None, true).render(TEMPLATE, "/Users/me@host/app.ico");
    assert!(rendered.is_ok_and(|text| text.contains(r#"1 ICON "/Users/me@host/app.ico""#)));
}

#[test]
fn a_broken_template_is_refused() {
    let lost = TEMPLATE.replace("@APP_NAME@", "Riseonly");
    assert!(values(None, None, true).render(&lost, "/ws/app.ico").is_err());
    let unknown = format!("{TEMPLATE}\n// @COMPANY@\n");
    assert!(values(None, None, true).render(&unknown, "/ws/app.ico").is_err());
}

#[test]
fn an_rc_string_doubles_its_quotes() {
    assert_eq!(rc_text(r#"say "hi""#), r#"say ""hi"""#);
}

#[test]
fn the_icon_carries_every_shell_size() {
    assert_eq!(&ICON[0..4], &[0, 0, 1, 0]);
    let count = usize::from(u16::from_le_bytes([ICON[4], ICON[5]]));
    let sizes: Vec<u16> = (0..count)
        .map(|index| match ICON[6 + 16 * index] {
            0 => 256,
            width => u16::from(width),
        })
        .collect();
    for size in [16, 20, 24, 32, 40, 48, 64, 256] {
        assert!(sizes.contains(&size), "app.ico lacks {size}px: {sizes:?}");
    }
}
