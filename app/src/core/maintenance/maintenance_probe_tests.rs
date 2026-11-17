use rise_canon::rise_maintenance::{
    POLL_FOREGROUND_MINIMUM_GAP_MS, POLL_LADDER_MS, POLL_MINIMUM_INTERVAL_MS, STATUS_PATH,
};

use super::maintenance_gate::{
    MaintenanceSnapshot, rise_ladder_delay_ms, rise_may_probe_again, rise_may_probe_on_foreground,
};
use super::maintenance_probe::MaintenanceProbe;
use crate::core::stores::http::http_service::HttpService;

fn ms(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[test]
fn the_status_path_is_not_appended_under_the_api_prefix_twice() {
    let url = HttpService::absolute_url(STATUS_PATH);

    assert_eq!(url.path(), STATUS_PATH);
    assert!(!url.path().contains("/api/api"));
}

#[test]
fn the_locale_bootstrap_path_resolves_the_same_way() {
    let url = HttpService::absolute_url("/api/bootstrap/locale");

    assert_eq!(url.path(), "/api/bootstrap/locale");
    assert!(!url.path().contains("/api/api"));
}

#[test]
fn the_origin_of_the_configured_host_is_preserved() {
    let base = HttpService::resolved_api_base_url();
    let url = HttpService::absolute_url(STATUS_PATH);

    assert_eq!(url.scheme(), base.scheme());
    assert_eq!(url.host(), base.host());
    assert_eq!(url.port(), base.port());
}

#[test]
fn a_path_is_normalised_whichever_way_it_is_written() {
    assert_eq!(
        HttpService::absolute_url("/api/bootstrap/maintenance").path(),
        HttpService::absolute_url("api/bootstrap/maintenance").path()
    );
}

#[test]
fn an_unknown_state_reads_as_open() {
    let json = br#"{"state":"hibernating","revision":7}"#;
    let reading = MaintenanceProbe::reading_from_json(json);
    assert!(reading.is_some());
    let Some(reading) = reading else {
        return;
    };

    assert!(!reading.is_closed);
    assert_eq!(reading.revision, 7);
}

#[test]
fn the_closed_state_blocks_traffic() {
    let json = br#"{"state":"closed","revision":5,"ends_at_ms":0,"server_time_ms":1000,"retry_after_ms":15000}"#;
    let reading = MaintenanceProbe::reading_from_json(json);
    assert!(reading.is_some());
    let Some(reading) = reading else {
        return;
    };

    assert!(reading.is_closed);
    assert_eq!(reading.retry_after_ms, 15000);
    assert_eq!(MaintenanceSnapshot::remaining_ms(0, 1000), None);
}

#[test]
fn the_remainder_is_measured_against_server_time() {
    assert_eq!(MaintenanceSnapshot::remaining_ms(1_000_600_000, 1_000_000_000), Some(600_000));
    assert_eq!(MaintenanceSnapshot::remaining_ms(1_000_000_000, 1_000_600_000), None);
}

#[test]
fn the_probe_gap_allows_the_first_ask_and_survives_a_backwards_clock() {
    assert!(rise_may_probe_on_foreground(1_000, 0));
    assert!(!rise_may_probe_on_foreground(1_000, 999));
    assert!(rise_may_probe_on_foreground(ms(POLL_FOREGROUND_MINIMUM_GAP_MS) + 1_000, 1_000));
    assert!(rise_may_probe_on_foreground(1_000, 5_000));
}

#[test]
fn a_dropped_socket_is_throttled_by_the_poll_floor_not_the_foreground_gap() {
    let floor = ms(POLL_MINIMUM_INTERVAL_MS);
    let foreground = ms(POLL_FOREGROUND_MINIMUM_GAP_MS);
    assert!(floor > foreground);

    let just_after_the_foreground_gap = foreground + 1_000;
    assert!(rise_may_probe_on_foreground(just_after_the_foreground_gap, 1_000));
    assert!(!rise_may_probe_again(just_after_the_foreground_gap, 1_000, floor));
    assert!(rise_may_probe_again(floor + 1_000, 1_000, floor));
}

#[test]
fn the_ladder_starts_at_its_first_rung() {
    let first = POLL_LADDER_MS.first().copied().map(ms);
    let last = POLL_LADDER_MS.last().copied().map(ms);
    let beyond = u32::try_from(POLL_LADDER_MS.len()).unwrap_or(u32::MAX).saturating_add(10);

    assert_eq!(Some(rise_ladder_delay_ms(0)), first);
    assert_eq!(Some(rise_ladder_delay_ms(1)), first);
    assert_eq!(Some(rise_ladder_delay_ms(beyond)), last);
}
