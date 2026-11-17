use rise_core::foundation::foundation_url::Url;

use crate::host_os::HostOs;
use crate::open_url::{OPEN_SETTINGS_URL_STRING, OpenUrlDestination, destination, settings_url_on};

const SCHEME: &str = "riseonly";

fn url(raw: &str) -> Url {
    let Some(url) = Url::from_string(raw) else {
        panic!("{raw} is not a URL");
    };
    url
}

fn on_mac(raw: &str) -> OpenUrlDestination {
    destination(&url(raw), HostOs::MacOs, Some(SCHEME))
}

#[test]
fn own_scheme_link_stays_in_the_app() {
    for host in HostOs::ALL {
        assert_eq!(
            destination(&url("riseonly://chat/42"), host, Some(SCHEME)),
            OpenUrlDestination::InApp,
            "{host:?} handed an own link to the OS"
        );
    }
    assert_eq!(on_mac("RISEONLY://chat/42"), OpenUrlDestination::InApp);
}

#[test]
fn own_web_link_with_a_screen_stays_in_the_app() {
    assert_eq!(on_mac("https://riseonly.net/post/9"), OpenUrlDestination::InApp);
}

#[test]
fn own_web_link_without_a_screen_goes_to_the_browser() {
    assert_eq!(
        on_mac("https://riseonly.net/"),
        OpenUrlDestination::System("https://riseonly.net/".to_owned())
    );
}

#[test]
fn another_environments_scheme_is_not_ours() {
    assert_eq!(on_mac("riseonly-staging://chat/42"), OpenUrlDestination::Refused);
}

#[test]
fn before_the_route_is_installed_own_links_never_reach_the_os() {
    assert_eq!(
        destination(&url("riseonly://chat/42"), HostOs::MacOs, None),
        OpenUrlDestination::Refused
    );
}

#[test]
fn web_mail_and_phone_links_go_to_the_system() {
    for raw in [
        "https://t.me/riseonly_bot?start=7999",
        "http://example.com/a",
        "mailto:support@example.com",
        "tel:+15550100",
    ] {
        assert_eq!(on_mac(raw), OpenUrlDestination::System(raw.to_owned()));
    }
}

#[test]
fn links_that_could_start_a_program_are_refused() {
    for raw in ["file:///etc/passwd", "javascript:alert(1)", "ms-msdt:/id", "tgx://resolve?domain=a"] {
        assert_eq!(on_mac(raw), OpenUrlDestination::Refused, "{raw} would reach the OS");
    }
}

#[test]
fn an_application_link_goes_to_the_system_only_through_the_handler_check() {
    for raw in ["tg://resolve?domain=a", "TG://resolve?domain=a"] {
        for host in HostOs::ALL {
            assert_eq!(
                destination(&url(raw), host, Some(SCHEME)),
                OpenUrlDestination::SystemIfHandled(url(raw).absolute_string().to_owned()),
                "{host:?} {raw}"
            );
        }
    }
}

#[test]
fn settings_url_opens_the_privacy_settings_where_the_host_has_them() {
    for host in HostOs::ALL {
        let expected = match settings_url_on(host) {
            Some(settings) => OpenUrlDestination::System(settings.to_owned()),
            None => OpenUrlDestination::Refused,
        };
        assert_eq!(
            destination(&url(OPEN_SETTINGS_URL_STRING), host, Some(SCHEME)),
            expected
        );
    }
    assert!(settings_url_on(HostOs::MacOs).is_some());
    assert!(settings_url_on(HostOs::Windows).is_some());
    assert_eq!(settings_url_on(HostOs::Linux), None);
}
