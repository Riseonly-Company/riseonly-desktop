use rise_platform::process_info::ThermalState;

use crate::modules::chat::stores::message::message_services::message_services::{
    MessageHistoryWarmCacheLimits, MessageHistoryWarmCachePolicy, MessageHistoryWarmCacheRecord,
};

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn record(cache_key: &str, estimated_cost: i64, last_accessed_at: f64, expires_at: f64) -> MessageHistoryWarmCacheRecord {
    MessageHistoryWarmCacheRecord { cache_key: cache_key.to_owned(), estimated_cost, last_accessed_at, expires_at }
}

#[test]
fn production_limits_bound_heat_and_resident_memory() {
    let limits = MessageHistoryWarmCacheLimits::production();

    assert_eq!(limits.maximum_concurrent_loads, 2);
    assert_eq!(limits.maximum_entry_count, 5);
    assert_eq!(limits.maximum_estimated_cost, 20 * 1_024 * 1_024);
    assert_eq!(limits.time_to_live, 90.0);
    assert_eq!(MessageHistoryWarmCachePolicy::maximum_speculative_concurrent_loads(limits.maximum_concurrent_loads), 1);
    assert_eq!(MessageHistoryWarmCachePolicy::maximum_speculative_concurrent_loads(1), 1);
}

#[test]
fn speculative_warmup_stops_for_power_thermal_and_background_pressure() {
    assert!(MessageHistoryWarmCachePolicy::permits_speculative_warmup(false, ThermalState::Nominal, false));
    assert!(MessageHistoryWarmCachePolicy::permits_speculative_warmup(false, ThermalState::Fair, false));
    assert!(!MessageHistoryWarmCachePolicy::permits_speculative_warmup(true, ThermalState::Nominal, false));
    assert!(!MessageHistoryWarmCachePolicy::permits_speculative_warmup(false, ThermalState::Serious, false));
    assert!(!MessageHistoryWarmCachePolicy::permits_speculative_warmup(false, ThermalState::Critical, false));
    assert!(!MessageHistoryWarmCachePolicy::permits_speculative_warmup(false, ThermalState::Nominal, true));
}

#[test]
fn visible_candidates_win_and_duplicates_do_not_consume_capacity() {
    let scopes = MessageHistoryWarmCachePolicy::prioritized_scopes(
        &strings(&[" visible-a ", "visible-b", "visible-a", ""]),
        &strings(&["recent-a", "visible-b", "recent-b", "recent-c"]),
        5,
    );

    assert_eq!(scopes, strings(&["visible-a", "visible-b", "recent-a", "recent-b", "recent-c"]));
}

#[test]
fn expired_entries_leave_before_least_recently_used_budget_eviction() {
    let limits = MessageHistoryWarmCacheLimits::new(2, 100, 2, 30.0);
    let records = [
        record("expired", 1, 0.0, 9.0),
        record("oldest", 60, 1.0, 100.0),
        record("middle", 60, 2.0, 100.0),
        record("newest", 10, 3.0, 100.0),
    ];

    assert_eq!(MessageHistoryWarmCachePolicy::eviction_keys(&records, 10.0, &limits), strings(&["expired", "oldest"]));
}

#[test]
fn normal_histories_respect_twenty_mi_b_budget_before_fifth_entry() {
    let cost = MessageHistoryWarmCachePolicy::estimated_resident_cost(100);
    let budget = MessageHistoryWarmCacheLimits::production().maximum_estimated_cost;

    assert!(cost * 4 <= budget);
    assert!(cost * 5 > budget);
    assert_eq!(MessageHistoryWarmCachePolicy::estimated_resident_cost(-1), 192 * 1_024);
}
