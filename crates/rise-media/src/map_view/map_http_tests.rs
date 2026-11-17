use std::io::Write;
use std::time::Duration;

use flate2::Compression;
use flate2::write::{GzEncoder, ZlibEncoder};

use super::map_http::*;

const NOW: i64 = 1_000_000;
const RFC_DATE: &str = "Sun, 06 Nov 1994 08:49:37 GMT";
const RFC_EPOCH: i64 = 784_111_777;

fn reply(status: u16, headers: &[(&str, &str)], body: &[u8]) -> MapHttpAnswer {
    Ok(MapHttpReply {
        status,
        headers: headers.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect(),
        body: body.to_vec(),
    })
}

fn header<'a>(request: &'a MapHttpRequest, name: &str) -> Option<&'a str> {
    request.headers.iter().find(|(candidate, _)| *candidate == name).map(|(_, value)| value.as_str())
}

fn kind(outcome: &MapHttpOutcome) -> Option<MapHttpErrorKind> {
    match &outcome.content {
        MapHttpContent::Failed { kind, .. } => Some(*kind),
        _ => None,
    }
}

#[test]
fn only_http_urls_are_served() {
    assert!(is_http_url("https://tiles.example/1/2/3.pbf"));
    assert!(is_http_url("http://tiles.example/style.json"));
    assert!(!is_http_url("file:///tmp/style.json"));
    assert!(!is_http_url("mbtiles:///tmp/a.mbtiles"));
}

#[test]
fn the_request_names_agent_and_encoding() {
    let request = map_http_request("https://a/b", None, None, None);
    assert_eq!(request.url, "https://a/b");
    assert_eq!(header(&request, "User-Agent"), Some(MAP_HTTP_USER_AGENT));
    assert_eq!(header(&request, "Accept-Encoding"), Some("gzip, deflate"));
    assert_eq!(request.headers.len(), 2);
}

#[test]
fn the_entity_tag_wins_over_the_date() {
    let both = map_http_request("https://a/b", Some("\"v1\""), Some(RFC_EPOCH), None);
    assert_eq!(header(&both, "If-None-Match"), Some("\"v1\""));
    assert_eq!(header(&both, "If-Modified-Since"), None);
    let date = map_http_request("https://a/b", None, Some(RFC_EPOCH), None);
    assert_eq!(header(&date, "If-Modified-Since"), Some(RFC_DATE));
}

#[test]
fn the_byte_range_is_inclusive() {
    let request = map_http_request("https://a/b", None, None, Some((10, 19)));
    assert_eq!(header(&request, "Range"), Some("bytes=10-19"));
}

#[test]
fn a_body_comes_with_its_cache_headers() {
    let outcome = map_http_outcome(
        reply(
            200,
            &[("etag", "\"v2\""), ("LAST-MODIFIED", RFC_DATE), ("Cache-Control", "public, max-age=60, must-revalidate")],
            b"tile",
        ),
        true,
        NOW,
    );
    assert_eq!(outcome.content, MapHttpContent::Data(b"tile".to_vec()));
    assert_eq!(outcome.etag.as_deref(), Some("\"v2\""));
    assert_eq!(outcome.modified, Some(RFC_EPOCH));
    assert_eq!(outcome.expires, Some(NOW + 60));
    assert!(outcome.must_revalidate);
}

#[test]
fn max_age_wins_over_expires_and_expires_stands_alone() {
    let both = map_http_outcome(reply(200, &[("Expires", RFC_DATE), ("Cache-Control", "max-age=5")], b""), false, NOW);
    assert_eq!(both.expires, Some(NOW + 5));
    let alone = map_http_outcome(reply(200, &[("Expires", RFC_DATE)], b""), false, NOW);
    assert_eq!(alone.expires, Some(RFC_EPOCH));
    assert!(!alone.must_revalidate);
    let broken = map_http_outcome(reply(200, &[("Expires", "0")], b""), false, NOW);
    assert_eq!(broken.expires, None);
}

#[test]
fn a_partial_body_is_data() {
    let outcome = map_http_outcome(reply(206, &[], b"part"), false, NOW);
    assert_eq!(outcome.content, MapHttpContent::Data(b"part".to_vec()));
}

#[test]
fn compressed_bodies_are_inflated() {
    let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
    gzip.write_all(b"vector tile").unwrap();
    let gzip = gzip.finish().unwrap();
    let outcome = map_http_outcome(reply(200, &[("Content-Encoding", "gzip")], &gzip), true, NOW);
    assert_eq!(outcome.content, MapHttpContent::Data(b"vector tile".to_vec()));

    let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
    zlib.write_all(b"style").unwrap();
    let zlib = zlib.finish().unwrap();
    let outcome = map_http_outcome(reply(200, &[("content-encoding", "deflate")], &zlib), false, NOW);
    assert_eq!(outcome.content, MapHttpContent::Data(b"style".to_vec()));
}

#[test]
fn identity_and_an_empty_body_pass_as_they_are() {
    let identity = map_http_outcome(reply(200, &[("Content-Encoding", "identity")], b"raw"), false, NOW);
    assert_eq!(identity.content, MapHttpContent::Data(b"raw".to_vec()));
    let empty = map_http_outcome(reply(200, &[("Content-Encoding", "gzip")], b""), false, NOW);
    assert_eq!(empty.content, MapHttpContent::Data(Vec::new()));
}

#[test]
fn an_unreadable_body_is_an_error_and_not_garbage() {
    let broken = map_http_outcome(reply(200, &[("Content-Encoding", "gzip")], b"not gzip"), true, NOW);
    assert_eq!(kind(&broken), Some(MapHttpErrorKind::Other));
    let unknown = map_http_outcome(reply(200, &[("Content-Encoding", "br")], b"x"), true, NOW);
    assert_eq!(kind(&unknown), Some(MapHttpErrorKind::Other));
}

#[test]
fn statuses_read_as_the_core_reads_them() {
    assert_eq!(map_http_outcome(reply(204, &[], b""), false, NOW).content, MapHttpContent::NoContent);
    assert_eq!(map_http_outcome(reply(304, &[], b""), false, NOW).content, MapHttpContent::NotModified);
    assert_eq!(map_http_outcome(reply(404, &[], b""), true, NOW).content, MapHttpContent::NoContent);
    assert_eq!(kind(&map_http_outcome(reply(404, &[], b""), false, NOW)), Some(MapHttpErrorKind::NotFound));
    assert_eq!(kind(&map_http_outcome(reply(503, &[], b""), true, NOW)), Some(MapHttpErrorKind::Server));
    assert_eq!(kind(&map_http_outcome(reply(403, &[], b""), true, NOW)), Some(MapHttpErrorKind::Other));
}

#[test]
fn not_modified_keeps_the_new_lifetime() {
    let outcome = map_http_outcome(reply(304, &[("Cache-Control", "max-age=10"), ("ETag", "\"v3\"")], b""), true, NOW);
    assert_eq!(outcome.expires, Some(NOW + 10));
    assert_eq!(outcome.etag.as_deref(), Some("\"v3\""));
}

fn retry_after(headers: &[(&str, &str)]) -> Option<i64> {
    match map_http_outcome(reply(429, headers, b""), true, NOW).content {
        MapHttpContent::Failed { kind: MapHttpErrorKind::RateLimit, retry_after, .. } => retry_after,
        other => panic!("not a rate limit: {other:?}"),
    }
}

#[test]
fn the_rate_limit_carries_its_retry_time() {
    assert_eq!(retry_after(&[("Retry-After", "30")]), Some(NOW + 30));
    assert_eq!(retry_after(&[("Retry-After", RFC_DATE)]), Some(RFC_EPOCH));
    assert_eq!(retry_after(&[("X-Rate-Limit-Reset", "2000000")]), Some(2_000_000));
    assert_eq!(retry_after(&[("Retry-After", "30"), ("X-Rate-Limit-Reset", "2000000")]), Some(NOW + 30));
    assert_eq!(retry_after(&[]), None);
}

#[test]
fn transfer_failures_keep_their_kind() {
    let connection = map_http_outcome(Err(MapHttpFailure::Connection("refused".to_owned())), true, NOW);
    assert_eq!(kind(&connection), Some(MapHttpErrorKind::Connection));
    let other = map_http_outcome(Err(MapHttpFailure::Other("reset".to_owned())), true, NOW);
    assert_eq!(kind(&other), Some(MapHttpErrorKind::Other));
}

#[test]
fn the_rfc_date_reads_and_writes() {
    assert_eq!(parse_http_date(RFC_DATE), Some(RFC_EPOCH));
    assert_eq!(format_http_date(RFC_EPOCH).as_deref(), Some(RFC_DATE));
    assert_eq!(format_http_date(0).as_deref(), Some("Thu, 01 Jan 1970 00:00:00 GMT"));
}

#[test]
fn dates_round_trip_across_leap_days() {
    for date in ["Thu, 29 Feb 2024 23:59:59 GMT", "Fri, 01 Mar 2024 00:00:00 GMT", "Sun, 31 Dec 2000 12:00:00 GMT"] {
        let epoch = parse_http_date(date).unwrap();
        assert_eq!(format_http_date(epoch).as_deref(), Some(date));
    }
}

#[test]
fn other_date_forms_are_not_read() {
    for date in [
        "",
        "0",
        "Sunday, 06-Nov-94 08:49:37 GMT",
        "Sun Nov  6 08:49:37 1994",
        "Sun, 06 Nov 1994 08:49:37 UTC",
        "Sun, 06 Foo 1994 08:49:37 GMT",
        "Sun, 32 Nov 1994 08:49:37 GMT",
        "Sun, 06 Nov 1994 25:49:37 GMT",
        "Sun, 06 Nov 1994 08:49:37 GMT extra",
    ] {
        assert_eq!(parse_http_date(date), None, "{date}");
    }
}

fn seconds(value: u64) -> Option<Duration> {
    Some(Duration::from_secs(value))
}

#[test]
fn a_server_error_retries_each_second_three_times_then_doubles() {
    let delay = |failed| error_retry_delay(MapHttpErrorKind::Server, failed, None, NOW);
    assert_eq!(delay(1), seconds(1));
    assert_eq!(delay(3), seconds(1));
    assert_eq!(delay(4), seconds(2));
    assert_eq!(delay(5), seconds(4));
    assert_eq!(delay(40), seconds(1 << 31));
}

#[test]
fn a_lost_connection_doubles_at_once_up_to_the_ceiling() {
    let delay = |failed| error_retry_delay(MapHttpErrorKind::Connection, failed, None, NOW);
    assert_eq!(delay(1), seconds(1));
    assert_eq!(delay(2), seconds(2));
    assert_eq!(delay(6), seconds(32));
    assert_eq!(delay(40), seconds(32));
}

#[test]
fn a_rate_limit_waits_for_its_time_or_the_default() {
    let delay = |retry_after| error_retry_delay(MapHttpErrorKind::RateLimit, 1, retry_after, NOW);
    assert_eq!(delay(Some(NOW + 90)), seconds(90));
    assert_eq!(delay(Some(NOW - 90)), seconds(0));
    assert_eq!(delay(None), seconds(5));
}

#[test]
fn other_failures_are_final() {
    assert_eq!(error_retry_delay(MapHttpErrorKind::NotFound, 1, None, NOW), None);
    assert_eq!(error_retry_delay(MapHttpErrorKind::Other, 1, None, NOW), None);
}

#[test]
fn a_cached_copy_waits_for_its_expiry_and_the_minimum_interval() {
    let none = Duration::ZERO;
    assert_eq!(first_request_delay(None, Duration::from_secs(60), NOW), none);
    assert_eq!(first_request_delay(Some(NOW + 100), none, NOW), Duration::from_secs(100));
    assert_eq!(first_request_delay(Some(NOW - 100), none, NOW), none);
    assert_eq!(first_request_delay(Some(NOW - 100), Duration::from_secs(60), NOW), Duration::from_secs(60));
    assert_eq!(first_request_delay(Some(NOW + 10), Duration::from_secs(60), NOW), Duration::from_secs(60));
}

fn outcome_with(modified: Option<i64>, expires: Option<i64>, etag: Option<&str>) -> MapHttpOutcome {
    MapHttpOutcome {
        content: MapHttpContent::NoContent,
        modified,
        expires,
        etag: etag.map(str::to_owned),
        must_revalidate: false,
    }
}

fn attempts(prior: MapHttpPrior, minimum_seconds: u64) -> MapHttpAttempts {
    MapHttpAttempts::new(prior, None, Duration::from_secs(minimum_seconds))
}

fn failed_with(kind: MapHttpErrorKind) -> MapHttpOutcome {
    MapHttpOutcome {
        content: MapHttpContent::Failed { kind, message: "failed".to_owned(), retry_after: None },
        ..outcome_with(None, None, None)
    }
}

#[test]
fn validators_the_answer_left_out_keep_the_cached_values() {
    let prior = MapHttpPrior { modified: Some(7), expires: None, etag: Some("old".to_owned()) };
    let (kept, _) = attempts(prior.clone(), 0).completed(outcome_with(None, None, None), NOW);
    assert_eq!((kept.modified, kept.etag.as_deref()), (Some(7), Some("old")));
    let (replaced, _) = attempts(prior, 0).completed(outcome_with(Some(8), None, Some("new")), NOW);
    assert_eq!((replaced.modified, replaced.etag.as_deref()), (Some(8), Some("new")));
}

#[test]
fn the_next_attempt_asks_with_the_validators_of_the_last_answer() {
    let mut attempts = attempts(MapHttpPrior { etag: Some("old".to_owned()), ..MapHttpPrior::default() }, 0);
    let _ = attempts.completed(outcome_with(Some(8), Some(NOW + 5), Some("new")), NOW);
    assert_eq!(attempts.prior.etag.as_deref(), Some("new"));
    assert_eq!(attempts.prior.modified, Some(8));
    assert_eq!(attempts.prior.expires, Some(NOW + 5));
}

fn not_modified() -> MapHttpOutcome {
    MapHttpOutcome { content: MapHttpContent::NotModified, ..outcome_with(None, None, None) }
}

#[test]
fn the_first_not_modified_hands_over_the_prior_data_and_the_next_stays_not_modified() {
    let mut attempts = MapHttpAttempts::new(MapHttpPrior::default(), Some(vec![1, 2]), Duration::ZERO);
    let (first, _) = attempts.completed(not_modified(), NOW);
    assert_eq!(first.content, MapHttpContent::Data(vec![1, 2]));
    let (second, _) = attempts.completed(not_modified(), NOW);
    assert_eq!(second.content, MapHttpContent::NotModified);
}

#[test]
fn the_prior_data_waits_for_a_not_modified_through_other_answers() {
    let mut attempts = MapHttpAttempts::new(MapHttpPrior::default(), Some(vec![1]), Duration::ZERO);
    let (failed, _) = attempts.completed(failed_with(MapHttpErrorKind::Server), NOW);
    assert!(matches!(failed.content, MapHttpContent::Failed { .. }));
    let (first, _) = attempts.completed(not_modified(), NOW);
    assert_eq!(first.content, MapHttpContent::Data(vec![1]));
}

#[test]
fn not_modified_without_prior_data_stays_not_modified() {
    let (answer, _) = attempts(MapHttpPrior::default(), 0).completed(not_modified(), NOW);
    assert_eq!(answer.content, MapHttpContent::NotModified);
}

#[test]
fn a_lifetime_already_over_is_corrected_only_when_it_moves_forward() {
    let expires = |current, prior| {
        let prior = MapHttpPrior { expires: prior, ..MapHttpPrior::default() };
        attempts(prior, 0).completed(outcome_with(None, Some(current), None), NOW).0.expires
    };
    assert_eq!(expires(NOW + 5, Some(NOW - 50)), Some(NOW + 5));
    assert_eq!(expires(NOW - 5, None), Some(NOW - 5));
    assert_eq!(expires(NOW - 50, Some(NOW - 5)), Some(NOW - 50));
    assert_eq!(expires(NOW - 5, Some(NOW - 5)), Some(NOW - 5));
    assert_eq!(expires(NOW - 5, Some(NOW - 105)), Some(NOW + 100));
    assert_eq!(expires(NOW - 5, Some(NOW - 10)), Some(NOW + 30));
}

#[test]
fn a_fresh_answer_is_asked_again_when_it_expires_and_not_before_the_minimum() {
    let next = |expires, minimum| {
        attempts(MapHttpPrior::default(), minimum).completed(outcome_with(None, expires, None), NOW).1
    };
    assert_eq!(next(Some(NOW + 100), 0), Some(Duration::from_secs(100)));
    assert_eq!(next(Some(NOW + 10), 60), Some(Duration::from_secs(60)));
    assert_eq!(next(None, 60), None);
}

#[test]
fn the_same_expired_answer_backs_off_until_a_fresh_one_comes() {
    let mut attempts = attempts(MapHttpPrior::default(), 0);
    let stale = || outcome_with(None, Some(NOW - 5), None);
    assert_eq!(attempts.completed(stale(), NOW).1, Some(Duration::from_secs(1)));
    assert_eq!(attempts.completed(stale(), NOW).1, Some(Duration::from_secs(2)));
    assert_eq!(attempts.completed(stale(), NOW).1, Some(Duration::from_secs(4)));
    let fresh = outcome_with(None, Some(NOW + 50), None);
    assert_eq!(attempts.completed(fresh, NOW).1, Some(Duration::from_secs(50)));
}

#[test]
fn a_failure_retries_on_the_table_and_a_final_one_stops() {
    let mut retried = attempts(MapHttpPrior::default(), 0);
    assert_eq!(
        retried.completed(failed_with(MapHttpErrorKind::Connection), NOW).1,
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        retried.completed(failed_with(MapHttpErrorKind::Connection), NOW).1,
        Some(Duration::from_secs(2))
    );
    let _ = retried.completed(outcome_with(None, None, None), NOW);
    assert_eq!(
        retried.completed(failed_with(MapHttpErrorKind::Connection), NOW).1,
        Some(Duration::from_secs(1))
    );
    let mut stopped = attempts(MapHttpPrior::default(), 0);
    assert_eq!(stopped.completed(failed_with(MapHttpErrorKind::NotFound), NOW).1, None);
}

#[test]
fn no_more_than_the_limit_runs_and_a_finished_one_frees_its_place() {
    let now = Duration::from_secs(100);
    let mut scheduler = MapHttpScheduler::new();
    let total = MAXIMUM_CONCURRENT_REQUESTS as u64 + 1;
    for id in 0..total {
        scheduler.schedule(id, false, now);
    }
    scheduler.release_due(now);
    for id in 0..MAXIMUM_CONCURRENT_REQUESTS as u64 {
        assert_eq!(scheduler.next_start(), Some(id));
    }
    assert_eq!(scheduler.next_start(), None);
    scheduler.finished(0);
    assert_eq!(scheduler.next_start(), Some(total - 1));
}

#[test]
fn regular_requests_pass_low_priority_ones() {
    let now = Duration::from_secs(100);
    let mut scheduler = MapHttpScheduler::new();
    scheduler.schedule(1, true, now);
    scheduler.schedule(2, false, now);
    scheduler.schedule(3, true, now);
    scheduler.schedule(4, false, now);
    scheduler.release_due(now);
    let order: Vec<_> = std::iter::from_fn(|| scheduler.next_start()).collect();
    assert_eq!(order, [2, 4, 1, 3]);
}

#[test]
fn a_timed_wait_starts_only_when_its_time_comes() {
    let now = Duration::from_secs(100);
    let later = now + Duration::from_secs(5);
    let mut scheduler = MapHttpScheduler::new();
    scheduler.schedule(1, false, later);
    assert_eq!(scheduler.next_due(), Some(later));
    scheduler.release_due(now);
    assert_eq!(scheduler.next_start(), None);
    scheduler.release_due(later);
    assert_eq!(scheduler.next_due(), None);
    assert_eq!(scheduler.next_start(), Some(1));
}

#[test]
fn a_cancelled_transfer_leaves_every_list() {
    let now = Duration::from_secs(100);
    let mut scheduler = MapHttpScheduler::new();
    scheduler.schedule(1, false, now);
    scheduler.schedule(2, true, now);
    scheduler.release_due(now);
    scheduler.schedule(3, false, now + Duration::from_secs(5));
    scheduler.cancel(1);
    scheduler.cancel(2);
    scheduler.cancel(3);
    assert_eq!(scheduler.next_due(), None);
    assert_eq!(scheduler.next_start(), None);
}
