use std::sync::{Mutex, MutexGuard, PoisonError};

use rise_limits::limit::{DIAGNOSTICS_BREADCRUMB_BYTES, DIAGNOSTICS_BREADCRUMBS};

use crate::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    let guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    RiseBreadcrumbs::reset();
    guard
}

#[test]
fn records_oldest_first() {
    let _serial = serial();
    RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Socket, || "connecting".to_owned());
    RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Rpc, || "call".to_owned());

    let entries = RiseBreadcrumbs::snapshot();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].channel, RiseBreadcrumbsChannel::Socket);
    assert_eq!(entries[0].message, "connecting");
    assert_eq!(entries[1].channel, RiseBreadcrumbsChannel::Rpc);
}

#[test]
fn truncates_message_to_two_hundred_characters() {
    let _serial = serial();
    RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || "\u{00e9}".repeat(500));

    let entries = RiseBreadcrumbs::snapshot();
    assert_eq!(entries[0].message.chars().count(), 200);
}

#[test]
fn evicts_oldest_by_count() {
    let _serial = serial();
    let maximum = DIAGNOSTICS_BREADCRUMBS as usize;
    for index in 0..(maximum + 5) {
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Screen, move || index.to_string());
    }

    let entries = RiseBreadcrumbs::snapshot();
    assert_eq!(entries.len(), maximum);
    assert_eq!(entries[0].message, "5");
}

#[test]
fn evicts_oldest_by_bytes() {
    let _serial = serial();
    let per_entry = 200 + 16;
    let fitting = DIAGNOSTICS_BREADCRUMB_BYTES as usize / per_entry;
    for _ in 0..(fitting + 3) {
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Media, || "x".repeat(200));
    }

    let entries = RiseBreadcrumbs::snapshot();
    assert_eq!(entries.len(), fitting);
    assert!(entries.len() * per_entry <= DIAGNOSTICS_BREADCRUMB_BYTES as usize);
}

#[test]
fn reset_empties_the_ring() {
    let _serial = serial();
    RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Session, || "signed in".to_owned());
    RiseBreadcrumbs::reset();
    assert!(RiseBreadcrumbs::snapshot().is_empty());
}

#[test]
fn channel_raw_values_round_trip() {
    let channels = [
        RiseBreadcrumbsChannel::Lifecycle,
        RiseBreadcrumbsChannel::Session,
        RiseBreadcrumbsChannel::Socket,
        RiseBreadcrumbsChannel::Rpc,
        RiseBreadcrumbsChannel::Media,
        RiseBreadcrumbsChannel::Screen,
        RiseBreadcrumbsChannel::Failure,
    ];
    for channel in channels {
        assert_eq!(RiseBreadcrumbsChannel::from_raw_value(channel.raw_value()), Some(channel));
    }
}
