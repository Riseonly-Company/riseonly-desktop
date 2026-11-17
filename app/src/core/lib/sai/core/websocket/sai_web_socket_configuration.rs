use rise_canon::RiseEndpointTimeoutSource;
use rise_canon::rise_wire_canon::{
    HANDSHAKE_WATCHDOG_MS, HEARTBEAT_INTERVAL_MS, POST_OPEN_PING_DELAY_MS, RECONNECT_BASE_DELAY_MS,
    RECONNECT_DOUBLING_CAP, RECONNECT_JITTER_FLOOR_MS, RECONNECT_JITTER_PERCENT, RECONNECT_MAX_DELAY_MS,
};

const NANOSECONDS_PER_MILLISECOND: u64 = 1_000_000;
const MILLISECONDS_PER_SECOND: f64 = 1_000.0;
const PERCENT_WHOLE: u64 = 100;

pub enum SaiWebSocketConfiguration {}

impl SaiWebSocketConfiguration {
    pub const POST_OPEN_PING_DELAY_NANOSECONDS: u64 = Self::nanoseconds(POST_OPEN_PING_DELAY_MS);

    pub const HANDSHAKE_WATCHDOG_NANOSECONDS: u64 = Self::nanoseconds(HANDSHAKE_WATCHDOG_MS);

    pub const HEARTBEAT_INTERVAL_NANOSECONDS: u64 = Self::nanoseconds(HEARTBEAT_INTERVAL_MS);

    pub const RECONNECT_BASE_DELAY_NANOSECONDS: u64 = Self::nanoseconds(RECONNECT_BASE_DELAY_MS);
    pub const RECONNECT_MAX_DELAY_NANOSECONDS: u64 = Self::nanoseconds(RECONNECT_MAX_DELAY_MS);
    pub const RECONNECT_BACKOFF_DOUBLING_CAP: u64 = RECONNECT_DOUBLING_CAP;

    /// shared ws deadline of the canon for offline acks
    /// the fallback repeats the canon figure since the source always declares one
    pub fn offline_ack_default_timeout_seconds() -> f64 {
        RiseEndpointTimeoutSource::SaiDefault.default_ms().unwrap_or_default() as f64 / MILLISECONDS_PER_SECOND
    }

    /// no max attempt count, capped exponential backoff plus jitter like telegram
    pub fn reconnect_delay_nanoseconds(after_consecutive_failures: u64) -> u64 {
        let base = Self::RECONNECT_BASE_DELAY_NANOSECONDS;
        let max_d = Self::RECONNECT_MAX_DELAY_NANOSECONDS;
        let mut d = base;
        let steps = after_consecutive_failures.min(Self::RECONNECT_BACKOFF_DOUBLING_CAP);
        for _ in 0..steps {
            d = max_d.min(d.saturating_mul(2));
        }
        let jitter_cap = Self::nanoseconds(RECONNECT_JITTER_FLOOR_MS).max(d / (PERCENT_WHOLE / RECONNECT_JITTER_PERCENT));
        let jitter = rand::random_range(0..=jitter_cap);
        max_d.min(d.saturating_add(jitter))
    }

    const fn nanoseconds(milliseconds: u64) -> u64 {
        milliseconds * NANOSECONDS_PER_MILLISECOND
    }
}
