use std::cell::Cell;

use rise_core::foundation::foundation_url::Url;
use rise_platform::host_os::HostOs;
use rise_platform::open_url::{OpenUrlDestination, destination};

use super::desktop_link_routing::{
    ExternalUrlOutcome, OwnLinkRoute, is_canon_web_link, own_link_route, parse_external_link,
    resolve_external_url,
};
use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;

const TRACK_ID: &str = "11111111-2222-4333-8444-555555555555";
const ENVIRONMENT: AppEnvironment = AppEnvironment::Prod;

fn url(raw: &str) -> Url {
    let Some(url) = Url::from_string(raw) else {
        panic!("{raw} is not a URL");
    };
    url
}

fn own_web_link() -> Url {
    url(&format!("https://riseonly.net/track/{TRACK_ID}"))
}

fn own_scheme_link() -> Url {
    url(&format!("riseonly://track/{TRACK_ID}"))
}

fn track_link() -> PushDeepLink {
    PushDeepLink::MusicTrack { track_id: TRACK_ID.to_owned() }
}

#[test]
fn a_share_handoff_answers_before_the_deep_link_parser() {
    let asked = Cell::new(0);
    let outcome = resolve_external_url(&own_scheme_link(), ENVIRONMENT, |_| {
        asked.set(asked.get() + 1);
        true
    });
    assert_eq!(outcome, ExternalUrlOutcome::Shared);
    assert_eq!(asked.get(), 1);
}

#[test]
fn a_link_no_share_handoff_claims_reaches_the_parser() {
    let outcome = resolve_external_url(&own_scheme_link(), ENVIRONMENT, |_| false);
    assert_eq!(outcome, ExternalUrlOutcome::Link(track_link()));
}

#[test]
fn a_malformed_music_link_still_opens_its_screen() {
    let malformed = url("https://riseonly.net/track/not-a-uuid");
    let outcome = resolve_external_url(&malformed, ENVIRONMENT, |_| false);
    assert_eq!(
        outcome,
        ExternalUrlOutcome::Link(PushDeepLink::MusicTrack { track_id: "not-a-uuid".to_owned() })
    );
}

#[test]
fn a_foreign_link_is_ignored_from_outside() {
    let outcome = resolve_external_url(&url("https://example.com/track/x"), ENVIRONMENT, |_| false);
    assert_eq!(outcome, ExternalUrlOutcome::Ignored);
}

#[test]
fn an_own_web_link_stays_inside_the_app() {
    let scheme = ENVIRONMENT.app_url_scheme();
    assert_eq!(
        destination(&own_web_link(), HostOs::current(), Some(scheme.as_str())),
        OpenUrlDestination::InApp
    );
    assert_eq!(own_link_route(&own_web_link(), ENVIRONMENT), OwnLinkRoute::Shell);
    assert_eq!(parse_external_link(&own_web_link(), ENVIRONMENT), Some(track_link()));
}

#[test]
fn a_foreign_web_link_goes_to_the_browser() {
    let foreign = url("https://example.com/track/x");
    let scheme = ENVIRONMENT.app_url_scheme();
    assert_eq!(
        destination(&foreign, HostOs::current(), Some(scheme.as_str())),
        OpenUrlDestination::System("https://example.com/track/x".to_owned())
    );
    assert_eq!(own_link_route(&foreign, ENVIRONMENT), OwnLinkRoute::Browser);
}

#[test]
fn the_app_scheme_always_reaches_the_shell() {
    assert_eq!(own_link_route(&url("riseonly://no-such-screen"), ENVIRONMENT), OwnLinkRoute::Shell);
    assert_eq!(own_link_route(&url("RISEONLY://no-such-screen"), ENVIRONMENT), OwnLinkRoute::Shell);
}

#[test]
fn a_web_link_no_screen_answers_goes_to_the_browser() {
    assert_eq!(own_link_route(&url("https://riseonly.net/"), ENVIRONMENT), OwnLinkRoute::Browser);
}

#[test]
fn the_scheme_of_another_environment_is_not_this_builds() {
    let link = url(&format!("riseonly-dev://track/{TRACK_ID}"));
    assert_eq!(own_link_route(&link, ENVIRONMENT), OwnLinkRoute::Browser);
    assert_eq!(resolve_external_url(&link, ENVIRONMENT, |_| false), ExternalUrlOutcome::Ignored);
}

#[test]
fn a_post_link_of_another_environment_is_a_canon_web_link_not_a_share() {
    let link = url("https://riseonly.net/p/440");
    assert_eq!(resolve_external_url(&link, AppEnvironment::Dev, |_| false), ExternalUrlOutcome::Ignored);
    assert!(is_canon_web_link(&link));
    assert!(is_canon_web_link(&url("https://WWW.riseonly.net/u/qa_seed_005")));
}

#[test]
fn a_post_link_of_this_environment_opens_the_post() {
    let link = url("https://dev.riseonly.net/p/440");
    assert_eq!(
        resolve_external_url(&link, AppEnvironment::Dev, |_| false),
        ExternalUrlOutcome::Link(PushDeepLink::Post { post_id: "440".to_owned() })
    );
}

#[test]
fn a_foreign_web_link_is_not_a_canon_web_link() {
    assert!(!is_canon_web_link(&url("https://example.com/p/440")));
    assert!(!is_canon_web_link(&url("riseonly://p/440")));
}
