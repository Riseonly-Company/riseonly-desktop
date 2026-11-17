use std::sync::LazyLock;
use std::time::Duration;

use rise_canon::RiseTimingCanon;
use rise_engine::RiseWireError;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::modules::rtc::engine::rise_rtc_engine_models::RiseRTCMediaDisconnectCode;
use crate::modules::rtc::system::rise_rtc_system_bridges::RiseRTCSystemDomainBridgeError;

const PARTICIPANT_HEARTBEAT: &str = "rtc_participant";

static WINDOW: LazyLock<Duration> =
    LazyLock::new(|| Duration::from_millis(RiseTimingCanon::heartbeat(PARTICIPANT_HEARTBEAT).effective_window_ms().max(0) as u64));

/// how a call waits for a connection that isnt there yet instead of hanging up on the first refusal
/// it says connecting and retries until the window runs out or the server ends the call
pub enum RiseRTCConnectionWaitPolicy {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseRTCConnectionWaitPolicySignallingFailure {
    Unsent,
    Unconfirmed,
}

impl RiseRTCConnectionWaitPolicy {
    const BACKOFF_STEP_MILLISECONDS: i64 = 700;
    const BACKOFF_STEP_CAP: i64 = 6;

    /// one patience for every connection wait of a call, the shortest lease any environment grants
    /// someone silent this long is already gone on the server
    pub fn window() -> Duration {
        *WINDOW
    }

    /// pause before the next attempt, a socket that comes back cuts it short
    pub fn backoff_milliseconds(attempt: i64) -> i64 {
        Self::BACKOFF_STEP_MILLISECONDS * attempt.max(1).min(Self::BACKOFF_STEP_CAP)
    }

    /// failure of the transport itself, before the server decided anything
    /// everything else (refusals, rate limits
    /// a call that already ended) is an answer and not retried
    pub fn signalling_failure(error: &anyhow::Error) -> Option<RiseRTCConnectionWaitPolicySignallingFailure> {
        if let Some(error) = error.downcast_ref::<RiseWireError>() {
            return match error {
                RiseWireError::NoSocketSession
                | RiseWireError::NotConnected
                | RiseWireError::Suspended
                | RiseWireError::StaleSession
                | RiseWireError::PendingLimitReached(_) => Some(RiseRTCConnectionWaitPolicySignallingFailure::Unsent),
                RiseWireError::Timeout(_) | RiseWireError::Disconnected(_) => Some(RiseRTCConnectionWaitPolicySignallingFailure::Unconfirmed),
                _ => None,
            };
        }
        if error.downcast_ref::<RiseRTCSystemDomainBridgeError>() == Some(&RiseRTCSystemDomainBridgeError::DomainUnavailable) {
            return Some(RiseRTCConnectionWaitPolicySignallingFailure::Unsent);
        }
        None
    }

    /// sfu failures a fresh attempt can fix, link failed or credentials aged out
    /// removals and deleted rooms and identity clashes and missing permissions and a broken audio session
    /// are decisions or local faults so no retry
    pub fn is_transient_media_failure(code: &str) -> bool {
        matches!(
            code,
            RiseRTCMediaDisconnectCode::LINK_LOST
                | RiseRTCMediaDisconnectCode::TIMED_OUT
                | RiseRTCMediaDisconnectCode::NETWORK_FAILED
                | RiseRTCMediaDisconnectCode::CONNECT_FAILED
                | RiseRTCMediaDisconnectCode::JOIN_FAILED
                | RiseRTCMediaDisconnectCode::TOKEN_REJECTED
                | RiseRTCMediaDisconnectCode::CREDENTIALS_EXPIRED
        )
    }

    /// true when the next attempt must ask the server for new credentials
    /// reusing live ones costs no request, every rejoin draws
    /// on the same per user call budget the hangup needs
    pub fn needs_fresh_credentials(failure_code: &str, credentials_expire_at_ms: i64, now_ms: i64) -> bool {
        failure_code == RiseRTCMediaDisconnectCode::TOKEN_REJECTED
            || failure_code == RiseRTCMediaDisconnectCode::CREDENTIALS_EXPIRED
            || credentials_expire_at_ms <= now_ms
    }
}
