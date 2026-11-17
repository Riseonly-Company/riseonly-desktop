use super::camera_portal::*;
use super::capture_permission::{CaptureAccess, CaptureKind};

#[test]
fn outside_a_sandbox_both_are_granted_without_asking() {
    for kind in CaptureKind::ALL {
        for answer in [None, Some(true), Some(false)] {
            assert_eq!(linux_access(false, kind, answer), CaptureAccess::Granted);
        }
    }
}

#[test]
fn inside_flatpak_the_camera_waits_for_the_portal() {
    assert_eq!(linux_access(true, CaptureKind::Camera, None), CaptureAccess::NotDetermined);
    assert_eq!(linux_access(true, CaptureKind::Camera, Some(true)), CaptureAccess::Granted);
    assert_eq!(linux_access(true, CaptureKind::Camera, Some(false)), CaptureAccess::Denied);
}

#[test]
fn inside_flatpak_the_microphone_comes_with_the_socket() {
    for answer in [None, Some(true), Some(false)] {
        assert_eq!(linux_access(true, CaptureKind::Microphone, answer), CaptureAccess::Granted);
    }
}

#[test]
fn portal_response_codes_map_as_the_spec_says() {
    assert_eq!(PortalResponse::from_code(0), PortalResponse::Granted);
    assert_eq!(PortalResponse::from_code(1), PortalResponse::Cancelled);
    assert_eq!(PortalResponse::from_code(2), PortalResponse::Other);
    assert_eq!(PortalResponse::from_code(7), PortalResponse::Other);
    assert!(PortalResponse::from_code(0).is_granted());
    assert!(!PortalResponse::from_code(1).is_granted());
    assert!(!PortalResponse::from_code(2).is_granted());
}

#[test]
fn request_token_uses_only_allowed_characters() {
    let token = request_token(42);
    assert_eq!(token, "riseonly_camera_42");
    assert!(token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
}

#[test]
fn request_path_follows_the_sender_rule() {
    assert_eq!(
        request_path(":1.42", "riseonly_camera_1"),
        "/org/freedesktop/portal/desktop/request/1_42/riseonly_camera_1"
    );
    assert_eq!(
        request_path("1.7", "t"),
        "/org/freedesktop/portal/desktop/request/1_7/t"
    );
}
