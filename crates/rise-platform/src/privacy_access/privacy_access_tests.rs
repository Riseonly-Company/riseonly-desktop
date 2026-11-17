use std::collections::HashSet;

use crate::capture_permission::{CaptureAccess, CaptureKind};
use crate::contacts::ContactAuthorization;
use crate::host_os::HostOs;
use crate::location::LocationAuthorization;
use crate::notifications::{DeliveryPrecondition, NotificationAvailability};
use crate::open_url::settings_url_on;
use crate::photo_library::PhotoLibraryAccessLevel;
use crate::privacy_access::*;

const PLIST_TEMPLATE: &str = include_str!("../../../../app/resources/macos/Info.plist.in");
const RU_STRINGS: &str = include_str!("../../../../app/resources/macos/ru.lproj/InfoPlist.strings");
const ENTITLEMENTS: &str = include_str!("../../../../app/resources/macos/Riseonly.entitlements");

fn plist_key(key: &str) -> String {
    format!("<key>{key}</key>")
}

#[test]
fn every_kind_routes_to_its_seam() {
    let table = [
        (PrivacyAccessKind::Camera, AccessSeam::CapturePermission),
        (PrivacyAccessKind::Microphone, AccessSeam::CapturePermission),
        (PrivacyAccessKind::Screen, AccessSeam::ScreenCapture),
        (PrivacyAccessKind::Notifications, AccessSeam::Notifications),
        (PrivacyAccessKind::Location, AccessSeam::Location),
        (PrivacyAccessKind::Contacts, AccessSeam::Contacts),
        (PrivacyAccessKind::Photos, AccessSeam::PhotoLibrary),
    ];
    assert_eq!(table.len(), PrivacyAccessKind::ALL.len());
    for (kind, seam) in table {
        assert_eq!(kind.seam(), seam, "{kind:?}");
    }

    let routed: HashSet<AccessSeam> = PrivacyAccessKind::ALL.iter().map(|kind| kind.seam()).collect();
    assert_eq!(routed.len(), AccessSeam::ALL.len(), "a seam is left without a kind");
}

#[test]
fn only_screen_is_read_outside_this_crate() {
    for seam in AccessSeam::ALL {
        assert_eq!(seam.answered_here(), seam != AccessSeam::ScreenCapture, "{seam:?}");
        let crate_name = if seam.answered_here() { "rise_platform::" } else { "rise_media::" };
        assert!(seam.module_path().starts_with(crate_name), "{seam:?}");
    }
}

#[test]
fn capture_keys_come_from_the_capture_seam() {
    assert_eq!(
        PrivacyAccessKind::Camera.usage_description_keys(),
        &[CaptureKind::Camera.usage_description_key()]
    );
    assert_eq!(
        PrivacyAccessKind::Microphone.usage_description_keys(),
        &[CaptureKind::Microphone.usage_description_key()]
    );
}

#[test]
fn every_usage_key_is_in_the_bundle_and_translated() {
    for kind in PrivacyAccessKind::ALL {
        for key in kind.usage_description_keys() {
            assert!(PLIST_TEMPLATE.contains(&plist_key(key)), "Info.plist.in lacks {key} for {kind:?}");
            assert!(RU_STRINGS.contains(&format!("\"{key}\"")), "ru InfoPlist.strings lacks {key}");
        }
    }
}

#[test]
fn the_bundle_carries_no_usage_key_without_a_kind() {
    let known: HashSet<&str> =
        PrivacyAccessKind::ALL.iter().flat_map(|kind| kind.usage_description_keys().iter().copied()).collect();
    for line in PLIST_TEMPLATE.lines() {
        let Some(start) = line.find("<key>NS") else { continue };
        let rest = &line[start + "<key>".len()..];
        let Some(end) = rest.find("</key>") else { continue };
        let key = &rest[..end];
        if key.ends_with("UsageDescription") {
            assert!(known.contains(key), "{key} belongs to no access kind");
        }
    }
}

#[test]
fn every_entitlement_is_granted() {
    for kind in PrivacyAccessKind::ALL {
        if let Some(entitlement) = kind.entitlement() {
            assert!(ENTITLEMENTS.contains(&plist_key(entitlement)), "Riseonly.entitlements lacks {entitlement} for {kind:?}");
        }
    }
}

#[test]
fn kinds_without_a_prompt_text_need_no_entitlement() {
    assert!(PrivacyAccessKind::Screen.usage_description_keys().is_empty());
    assert!(PrivacyAccessKind::Notifications.usage_description_keys().is_empty());
    assert_eq!(PrivacyAccessKind::Screen.entitlement(), None);
    assert_eq!(PrivacyAccessKind::Notifications.entitlement(), None);
}

#[test]
fn capture_status_folds_one_to_one() {
    assert_eq!(AccessStatus::from_capture(CaptureAccess::NotDetermined), AccessStatus::NotDetermined);
    assert_eq!(AccessStatus::from_capture(CaptureAccess::Denied), AccessStatus::Denied);
    assert_eq!(AccessStatus::from_capture(CaptureAccess::Restricted), AccessStatus::Restricted);
    assert_eq!(AccessStatus::from_capture(CaptureAccess::Granted), AccessStatus::Authorized);
}

#[test]
fn location_grants_read_as_authorized() {
    assert_eq!(AccessStatus::from_location(LocationAuthorization::NotDetermined), AccessStatus::NotDetermined);
    assert_eq!(AccessStatus::from_location(LocationAuthorization::Denied), AccessStatus::Denied);
    assert_eq!(AccessStatus::from_location(LocationAuthorization::Restricted), AccessStatus::Restricted);
    assert_eq!(AccessStatus::from_location(LocationAuthorization::WhenInUse), AccessStatus::Authorized);
    assert_eq!(AccessStatus::from_location(LocationAuthorization::Always), AccessStatus::Authorized);
}

#[test]
fn limited_contacts_and_photos_read_as_authorized() {
    assert_eq!(AccessStatus::from_contacts(ContactAuthorization::Limited), AccessStatus::Authorized);
    assert_eq!(AccessStatus::from_contacts(ContactAuthorization::Authorized), AccessStatus::Authorized);
    assert_eq!(AccessStatus::from_contacts(ContactAuthorization::Restricted), AccessStatus::Restricted);
    assert_eq!(AccessStatus::from_contacts(ContactAuthorization::Denied), AccessStatus::Denied);
    assert_eq!(AccessStatus::from_contacts(ContactAuthorization::NotDetermined), AccessStatus::NotDetermined);

    assert_eq!(AccessStatus::from_photo_library(PhotoLibraryAccessLevel::Limited), AccessStatus::Authorized);
    assert_eq!(AccessStatus::from_photo_library(PhotoLibraryAccessLevel::Authorized), AccessStatus::Authorized);
    assert_eq!(AccessStatus::from_photo_library(PhotoLibraryAccessLevel::Restricted), AccessStatus::Restricted);
    assert_eq!(AccessStatus::from_photo_library(PhotoLibraryAccessLevel::Denied), AccessStatus::Denied);
    assert_eq!(
        AccessStatus::from_photo_library(PhotoLibraryAccessLevel::NotDetermined),
        AccessStatus::NotDetermined
    );
}

#[test]
fn a_blocked_notification_route_is_restricted() {
    assert_eq!(
        AccessStatus::from_notification_availability(NotificationAvailability::Available),
        AccessStatus::Authorized
    );
    for precondition in [
        DeliveryPrecondition::AppBundle,
        DeliveryPrecondition::AppIdentity,
        DeliveryPrecondition::NotificationServer,
    ] {
        assert_eq!(
            AccessStatus::from_notification_availability(NotificationAvailability::Blocked(precondition)),
            AccessStatus::Restricted
        );
    }
}

#[test]
fn only_an_undecided_status_asks() {
    assert!(AccessStatus::NotDetermined.can_request());
    for status in [AccessStatus::Denied, AccessStatus::Restricted, AccessStatus::Authorized] {
        assert!(!status.can_request(), "{status:?}");
    }
    assert!(AccessStatus::Authorized.is_authorized());
    assert!(!AccessStatus::Denied.is_authorized());
}

#[test]
fn settings_open_the_whole_privacy_section() {
    for host in HostOs::ALL {
        assert_eq!(settings_url(host), settings_url_on(host), "{host:?}");
    }
    assert_eq!(settings_url(HostOs::Linux), None);
}
