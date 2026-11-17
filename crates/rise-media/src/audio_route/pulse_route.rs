use std::collections::BTreeSet;
use std::time::Duration;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

const DEFAULT_SINK_LABEL: &str = "Default Sink:";

pub const FIRST_RESTART_DELAY: Duration = Duration::from_millis(250);

pub const MAX_RESTART_DELAY: Duration = Duration::from_secs(30);

pub const STABLE_RUN: Duration = Duration::from_secs(5);

/// true for a route event line from pactl subscribe (new or remove on a sink or change on server)
/// a sink own volume change also shows up but the snapshot comparison drops it
pub fn is_route_event(line: &str) -> bool {
    line.contains(" on sink #") || line.contains(" on server")
}

/// the Default Sink line of pactl info, n/a or an empty name is no default
pub fn default_sink(info: &str) -> Option<String> {
    info.lines()
        .find_map(|line| trimming_whitespaces_and_newlines(line).strip_prefix(DEFAULT_SINK_LABEL))
        .map(trimming_whitespaces_and_newlines)
        .filter(|name| !name.is_empty() && *name != "n/a")
        .map(str::to_owned)
}

/// sink names from pactl list short sinks, tab separated columns
pub fn sink_names(listing: &str) -> BTreeSet<String> {
    listing
        .lines()
        .filter_map(|line| line.split('\t').nth(1))
        .map(trimming_whitespaces_and_newlines)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// wait before restart number attempt (0 first), doubles from the first delay and is capped
pub fn restart_delay(attempt: u32) -> Duration {
    let factor = 1u32.checked_shl(attempt.min(16)).unwrap_or(u32::MAX);
    FIRST_RESTART_DELAY.saturating_mul(factor).min(MAX_RESTART_DELAY)
}

/// attempt number after a subscriber that ran for ran, a stable run resets the backoff
pub fn next_attempt(attempt: u32, ran: Duration) -> u32 {
    if ran >= STABLE_RUN { 0 } else { attempt.saturating_add(1) }
}
