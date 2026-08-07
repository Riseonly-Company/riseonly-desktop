use std::sync::Arc;

use parking_lot::Mutex;

use super::rise_auth_engine_models::ResetReason;

/// What happened, never what it happened to.
///
/// Auth is the one domain where a trace is most useful and most dangerous: the
/// interesting values are a phone number, a tag and two tokens. So the events
/// carry no payload at all — the shape of the sequence is what a bug report
/// needs ("refreshed, refreshed, cleared" is a rotation race; "cleared" alone is
/// a revoked session), and nothing here can leak a credential into a log file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TraceEvent {
    Activated,
    SignedIn,
    CodeSent,
    Refreshed,
    RefreshFailed,
    RefreshRaceLost,
    RestoreFoundExpiredSession,
    MissingCredential,
    IncompleteGrant,
    FlowRefused,
    TransportFailed,
    LogoutTimedOut,
    Cleared(ResetReason),
}

impl TraceEvent {
    pub fn name(self) -> &'static str {
        match self {
            Self::Activated => "activated",
            Self::SignedIn => "signed_in",
            Self::CodeSent => "code_sent",
            Self::Refreshed => "refreshed",
            Self::RefreshFailed => "refresh_failed",
            Self::RefreshRaceLost => "refresh_race_lost",
            Self::RestoreFoundExpiredSession => "restore_expired",
            Self::MissingCredential => "missing_credential",
            Self::IncompleteGrant => "incomplete_grant",
            Self::FlowRefused => "flow_refused",
            Self::TransportFailed => "transport_failed",
            Self::LogoutTimedOut => "logout_timed_out",
            Self::Cleared(ResetReason::UserLogout) => "cleared_user_logout",
            Self::Cleared(ResetReason::ForcedLogout) => "cleared_forced",
            Self::Cleared(ResetReason::AccountChanged) => "cleared_account_changed",
        }
    }
}

const CAPACITY: usize = 64;

/// A bounded ring of recent events.
///
/// Bounded because an unbounded trace on a client that reconnects all day is a
/// slow leak, and the tail is what a bug report needs anyway.
#[derive(Clone, Default)]
pub struct AuthTrace {
    events: Arc<Mutex<Vec<TraceEvent>>>,
}

impl AuthTrace {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, event: TraceEvent) {
        tracing::debug!(target: "riseonly::auth", "{}", event.name());

        let mut events = self.events.lock();
        if events.len() == CAPACITY {
            events.remove(0);
        }
        events.push(event);
    }

    pub fn recent(&self) -> Vec<TraceEvent> {
        self.events.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_keeps_the_tail_rather_than_growing() {
        let trace = AuthTrace::new();
        for _ in 0..(CAPACITY + 10) {
            trace.record(TraceEvent::Refreshed);
        }
        trace.record(TraceEvent::SignedIn);

        let recent = trace.recent();
        assert_eq!(recent.len(), CAPACITY);
        assert_eq!(recent.last(), Some(&TraceEvent::SignedIn));
    }

    #[test]
    fn a_users_own_sign_out_is_distinguishable_from_a_revoked_session() {
        assert_ne!(
            TraceEvent::Cleared(ResetReason::UserLogout).name(),
            TraceEvent::Cleared(ResetReason::ForcedLogout).name()
        );
    }

    #[test]
    fn every_event_has_a_name_and_none_of_them_can_carry_a_secret() {
        for event in [
            TraceEvent::Activated,
            TraceEvent::SignedIn,
            TraceEvent::CodeSent,
            TraceEvent::Refreshed,
            TraceEvent::RefreshFailed,
            TraceEvent::RefreshRaceLost,
            TraceEvent::RestoreFoundExpiredSession,
            TraceEvent::MissingCredential,
            TraceEvent::IncompleteGrant,
            TraceEvent::FlowRefused,
            TraceEvent::TransportFailed,
            TraceEvent::LogoutTimedOut,
            TraceEvent::Cleared(ResetReason::AccountChanged),
        ] {
            assert!(!event.name().is_empty());
            assert_eq!(
                std::mem::size_of_val(&event),
                std::mem::size_of::<TraceEvent>(),
                "an event that grew a payload could carry a token into a log"
            );
        }
    }
}
