use super::capture_permission::{CaptureAccess, CaptureKind};
use super::consent_store::*;

#[test]
fn capability_names_match_the_manifest_and_registry() {
    assert_eq!(capability_name(CaptureKind::Camera), "webcam");
    assert_eq!(capability_name(CaptureKind::Microphone), "microphone");
}

#[test]
fn every_capability_status_maps() {
    assert_eq!(
        access_from_capability_status(STATUS_DENIED_BY_SYSTEM),
        CaptureAccess::Restricted
    );
    assert_eq!(
        access_from_capability_status(STATUS_NOT_DECLARED_BY_APP),
        CaptureAccess::Restricted
    );
    assert_eq!(access_from_capability_status(STATUS_DENIED_BY_USER), CaptureAccess::Denied);
    assert_eq!(
        access_from_capability_status(STATUS_USER_PROMPT_REQUIRED),
        CaptureAccess::NotDetermined
    );
    assert_eq!(access_from_capability_status(STATUS_ALLOWED), CaptureAccess::Granted);
}

#[test]
fn capability_status_raw_values_follow_the_winrt_enum() {
    assert_eq!(
        [
            STATUS_DENIED_BY_SYSTEM,
            STATUS_NOT_DECLARED_BY_APP,
            STATUS_DENIED_BY_USER,
            STATUS_USER_PROMPT_REQUIRED,
            STATUS_ALLOWED,
        ],
        [0, 1, 2, 3, 4]
    );
}

#[test]
fn an_unknown_capability_status_is_a_refusal() {
    for raw in [-1, 5, 42, i32::MAX, i32::MIN] {
        assert_eq!(access_from_capability_status(raw), CaptureAccess::Denied);
    }
}

#[test]
fn consent_words_ignore_case() {
    assert_eq!(Consent::from_value(Some("Allow")), Consent::Allow);
    assert_eq!(Consent::from_value(Some("allow")), Consent::Allow);
    assert_eq!(Consent::from_value(Some("ALLOW")), Consent::Allow);
    assert_eq!(Consent::from_value(Some("Deny")), Consent::Deny);
    assert_eq!(Consent::from_value(Some("deny")), Consent::Deny);
    assert_eq!(Consent::from_value(None), Consent::Unset);
    assert_eq!(Consent::from_value(Some("")), Consent::Other);
    assert_eq!(Consent::from_value(Some("Prompt")), Consent::Other);
    assert_eq!(Consent::from_value(Some("Allow ")), Consent::Other);
}

#[test]
fn a_device_deny_is_restricted_whatever_the_user_says() {
    for user in Consent::ALL {
        for desktop_apps in Consent::ALL {
            assert_eq!(
                access_from_consent(Consent::Deny, user, desktop_apps),
                CaptureAccess::Restricted
            );
        }
    }
}

#[test]
fn a_user_or_desktop_apps_deny_is_denied() {
    for device in [Consent::Allow, Consent::Unset] {
        for other in [Consent::Allow, Consent::Unset] {
            assert_eq!(access_from_consent(device, Consent::Deny, other), CaptureAccess::Denied);
            assert_eq!(access_from_consent(device, other, Consent::Deny), CaptureAccess::Denied);
        }
        assert_eq!(
            access_from_consent(device, Consent::Deny, Consent::Deny),
            CaptureAccess::Denied
        );
    }
}

#[test]
fn an_unknown_word_anywhere_below_a_device_deny_is_denied() {
    for a in Consent::ALL {
        for b in Consent::ALL {
            if a == Consent::Deny {
                continue;
            }
            assert_eq!(access_from_consent(Consent::Other, a, b), CaptureAccess::Denied);
            assert_eq!(access_from_consent(a, Consent::Other, b), CaptureAccess::Denied);
            assert_eq!(access_from_consent(a, b, Consent::Other), CaptureAccess::Denied);
        }
    }
}

#[test]
fn unset_and_allow_toggles_grant() {
    for device in [Consent::Allow, Consent::Unset] {
        for user in [Consent::Allow, Consent::Unset] {
            for desktop_apps in [Consent::Allow, Consent::Unset] {
                assert_eq!(
                    access_from_consent(device, user, desktop_apps),
                    CaptureAccess::Granted
                );
            }
        }
    }
}
