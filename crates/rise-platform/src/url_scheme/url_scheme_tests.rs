use rise_deeplink::ENVIRONMENTS;

use super::url_scheme::{
    UrlSchemeRegistrar, UrlSchemeRegistrationStep, UrlSchemeUnavailable, registration_step,
    url_scheme_for_environment,
};
use crate::host_os::HostOs;

struct Answers(Result<bool, UrlSchemeUnavailable>);

impl UrlSchemeRegistrar for Answers {
    fn is_default_handler(&self, _scheme: &str) -> Result<bool, UrlSchemeUnavailable> {
        self.0.clone()
    }
}

struct DefaultBody;

impl UrlSchemeRegistrar for DefaultBody {}

const OWN_SCHEME: &str = "riseonly-dev";

#[test]
fn every_environment_has_its_own_canon_scheme() {
    for row in &ENVIRONMENTS {
        assert_eq!(url_scheme_for_environment(row.name), Some(row.url_scheme));
    }
    assert_eq!(url_scheme_for_environment("production"), Some("riseonly"));
    assert_eq!(url_scheme_for_environment("dev"), Some("riseonly-dev"));
    assert_eq!(url_scheme_for_environment("staging"), Some("riseonly-staging"));
    assert_eq!(url_scheme_for_environment("beta"), None);
}

#[test]
fn a_held_claim_is_not_asked_for_again() {
    assert_eq!(
        registration_step(&Answers(Ok(true)), OWN_SCHEME),
        UrlSchemeRegistrationStep::AlreadyDefault
    );
}

#[test]
fn a_claim_held_elsewhere_is_renewed() {
    assert_eq!(
        registration_step(&Answers(Ok(false)), OWN_SCHEME),
        UrlSchemeRegistrationStep::Register
    );
}

#[test]
fn a_scheme_outside_canon_is_never_claimed() {
    for scheme in ["tg", "https", "riseonly-beta", ""] {
        assert_eq!(
            registration_step(&Answers(Ok(false)), scheme),
            UrlSchemeRegistrationStep::ForeignScheme
        );
    }
}

#[test]
fn a_process_outside_an_install_registers_nothing() {
    assert_eq!(
        registration_step(&Answers(Err(UrlSchemeUnavailable::NotInstalled)), OWN_SCHEME),
        UrlSchemeRegistrationStep::Unavailable(UrlSchemeUnavailable::NotInstalled)
    );
}

#[test]
fn the_default_body_reports_an_unwritten_branch() {
    assert_eq!(
        registration_step(&DefaultBody, OWN_SCHEME),
        UrlSchemeRegistrationStep::Unavailable(UrlSchemeUnavailable::BranchNotWritten {
            host: HostOs::current(),
        })
    );
}
