use std::collections::HashMap;
use std::ffi::OsString;
use std::panic::Location;
use std::path::PathBuf;

use crate::app::perf::rise_perf_log::{PerfSummary, dirty_by_line, miss_by_line, perf_log_path, views_by_type_line};

#[test]
fn an_empty_run_is_all_zeros() {
    assert_eq!(PerfSummary::of(&[]), PerfSummary::default());
}

#[test]
fn percentiles_take_the_nearest_rank() {
    let draws: Vec<f64> = (1..=100).map(f64::from).collect();
    let summary = PerfSummary::of(&draws);
    assert_eq!(summary.frames, 100);
    assert_eq!(summary.p50_ms, 50.0);
    assert_eq!(summary.p95_ms, 95.0);
    assert_eq!(summary.p99_ms, 99.0);
    assert_eq!(summary.max_ms, 100.0);
    assert_eq!(summary.over_budget, 92);
    assert_eq!(summary.over_ceiling, 84);
}

#[test]
fn a_single_frame_is_every_percentile_and_order_does_not_matter() {
    let summary = PerfSummary::of(&[3.0]);
    assert_eq!((summary.p50_ms, summary.p99_ms, summary.frames), (3.0, 3.0, 1));
    let shuffled = PerfSummary::of(&[20.0, 1.0, 9.0, f64::NAN]);
    assert_eq!(shuffled.frames, 3);
    assert_eq!(shuffled.p50_ms, 9.0);
    assert_eq!((shuffled.over_budget, shuffled.over_ceiling), (2, 1));
}

#[test]
fn the_summary_line_names_every_number() {
    let line = PerfSummary::of(&[4.0, 12.0, 30.0]).line();
    assert_eq!(line, "summary frames=3 p50=12.00 p95=30.00 p99=30.00 max=30.00 over8=2 over16=1");
}

#[test]
fn only_a_named_file_turns_the_log_on() {
    assert_eq!(perf_log_path(None), None);
    assert_eq!(perf_log_path(Some(OsString::new())), None);
    assert_eq!(perf_log_path(Some(OsString::from("/tmp/perf.log"))), Some(PathBuf::from("/tmp/perf.log")));
}

#[test]
fn views_by_type_writes_only_the_renders_since_the_previous_drain() {
    let mut previous = HashMap::new();
    let first = views_by_type_line(1.0, &[("a::Row", 30), ("b::Header", 2)], &mut previous);
    assert_eq!(first.as_deref(), Some("views_by_type\t1.0\ta::Row\t30\tb::Header\t2\n"));
    let second = views_by_type_line(2.0, &[("a::Row", 30), ("b::Header", 7), ("c::Gen<x, y>", 1)], &mut previous);
    assert_eq!(second.as_deref(), Some("views_by_type\t2.0\tb::Header\t5\tc::Gen<x,y>\t1\n"));
    assert_eq!(views_by_type_line(3.0, &[("a::Row", 30)], &mut previous), None);
}

#[track_caller]
fn here() -> &'static Location<'static> {
    Location::caller()
}

#[test]
fn dirty_by_names_new_call_sites_by_delta_and_skips_a_quiet_drain() {
    let first_site = here();
    let second_site = here();
    let name = |location: &Location<'_>| format!("{}:{}", location.file(), location.line());
    let mut previous = HashMap::new();
    let first = dirty_by_line(1.0, &[(first_site, 3), (second_site, 1)], &mut previous);
    assert_eq!(first, Some(format!("dirty_by\t1.0\t{}\t3\t{}\t1\n", name(first_site), name(second_site))));
    let second = dirty_by_line(2.0, &[(first_site, 3), (second_site, 6)], &mut previous);
    assert_eq!(second, Some(format!("dirty_by\t2.0\t{}\t5\n", name(second_site))));
    assert_eq!(dirty_by_line(3.0, &[(first_site, 3), (second_site, 6)], &mut previous), None);
}

#[test]
fn miss_by_names_type_and_reason_by_delta_and_skips_a_quiet_drain() {
    let mut previous = HashMap::new();
    let first = miss_by_line(1.0, &[(("a::Row", "env"), 4), (("b::Host", "paint_gap"), 1)], &mut previous);
    assert_eq!(first.as_deref(), Some("miss_by\t1.0\ta::Row|env\t4\tb::Host|paint_gap\t1\n"));
    let second = miss_by_line(2.0, &[(("a::Row", "env"), 4), (("b::Host", "paint_gap"), 3)], &mut previous);
    assert_eq!(second.as_deref(), Some("miss_by\t2.0\tb::Host|paint_gap\t2\n"));
    assert_eq!(miss_by_line(3.0, &[(("a::Row", "env"), 4)], &mut previous), None);
}
