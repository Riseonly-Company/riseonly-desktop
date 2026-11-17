use rise_canon::RiseTimingCanon;
use rise_core::foundation::foundation_date::{Date, TimeInterval};

use crate::core::lib::canon_timing::RiseTimingCanonExt;

pub enum RiseRTCRaiseHandCooldown {}

impl RiseRTCRaiseHandCooldown {
    /// paired in canon with the gateway bucket for rtc.request_to_speak (1
    /// rps, burst 5) so the window follows the bucket and not a hardcoded 1.5
    pub fn window_seconds() -> TimeInterval {
        RiseTimingCanon::debounce("rtc_raise_hand").client_debounce_ms as TimeInterval / 1_000.0
    }

    /// seconds left before another raise is allowed, zero means now
    /// a future last_raised_at is ignored since the store stamps it from the device clock
    /// and a forward clock jump would lock the button out
    pub fn remaining(last_raised_at: Option<Date>, now: Date, window: TimeInterval) -> TimeInterval {
        let Some(last_raised_at) = last_raised_at else {
            return 0.0;
        };
        let elapsed = now.time_interval_since(&last_raised_at);
        if elapsed < 0.0 {
            return 0.0;
        }
        (window - elapsed).max(0.0)
    }

    pub fn allows_raise(last_raised_at: Option<Date>, now: Date, window: TimeInterval) -> bool {
        Self::remaining(last_raised_at, now, window) <= 0.0
    }
}
