use rise_core::RequestId;
use thiserror::Error;

/// Caps on one push transaction: a realtime burst commits as a single write, and
/// these stop a flood from making that write unbounded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PushBatchLimits {
    pub max_events: usize,
    pub max_mutations: usize,
    pub max_bytes: usize,
}

impl Default for PushBatchLimits {
    fn default() -> Self {
        Self {
            max_events: 32,
            max_mutations: 512,
            max_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlushReason {
    Events,
    Mutations,
    Bytes,
    Window,
}

#[derive(Debug, Default)]
pub struct PushBatch {
    limits: PushBatchLimits,
    events: usize,
    mutations: usize,
    bytes: usize,
}

impl PushBatch {
    pub fn new(limits: PushBatchLimits) -> Self {
        Self {
            limits,
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.events == 0
    }

    pub fn events(&self) -> usize {
        self.events
    }

    /// Adds one event and reports whether the batch must now be committed.
    pub fn push(&mut self, mutations: usize, bytes: usize) -> Option<FlushReason> {
        self.events += 1;
        self.mutations += mutations;
        self.bytes += bytes;

        if self.events >= self.limits.max_events {
            Some(FlushReason::Events)
        } else if self.mutations >= self.limits.max_mutations {
            Some(FlushReason::Mutations)
        } else if self.bytes >= self.limits.max_bytes {
            Some(FlushReason::Bytes)
        } else {
            None
        }
    }

    pub fn take(&mut self) -> (usize, usize, usize) {
        let taken = (self.events, self.mutations, self.bytes);
        self.events = 0;
        self.mutations = 0;
        self.bytes = 0;
        taken
    }
}

/// Exponential backoff with jitter derived from the request id, so the delay
/// spreads clients apart yet stays reproducible for a given id and attempt.
pub fn retry_delay_ms(request_id: RequestId, attempt_count: u32) -> i64 {
    let exponent = attempt_count.saturating_sub(1).min(6);
    let base = 1_000i64 << exponent;
    let jitter_range = (base / 4).max(1);
    let jitter = (request_id.get() % jitter_range as u64) as i64;
    base + jitter
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DurableState {
    Queued,
    InFlight,
    RetryWaiting,
    TerminalFailure,
}

impl DurableState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::InFlight => "in_flight",
            Self::RetryWaiting => "retry_waiting",
            Self::TerminalFailure => "terminal_failure",
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClaimError {
    #[error("operation is {0:?}, only a queued or waiting operation can be claimed")]
    NotClaimable(DurableState),
    #[error("claim token does not match the operation's current claim")]
    StaleClaim,
    #[error("operation is not in flight")]
    NotInFlight,
}

/// One row of the durable outbox.
///
/// The claim token makes completion safe across a reconnect: an interrupted drain
/// that returns after the operation was reclaimed is rejected, not applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableOperation {
    pub request_id: RequestId,
    pub ordering_key: String,
    pub state: DurableState,
    pub attempt_count: u32,
    pub next_attempt_ms: i64,
    pub claim_token: Option<String>,
    pub last_error_code: Option<String>,
}

impl DurableOperation {
    pub fn queued(request_id: RequestId, ordering_key: impl Into<String>, now_ms: i64) -> Self {
        Self {
            request_id,
            ordering_key: ordering_key.into(),
            state: DurableState::Queued,
            attempt_count: 0,
            next_attempt_ms: now_ms,
            claim_token: None,
            last_error_code: None,
        }
    }

    pub fn is_ready(&self, now_ms: i64) -> bool {
        matches!(
            self.state,
            DurableState::Queued | DurableState::RetryWaiting
        ) && self.next_attempt_ms <= now_ms
    }

    pub fn claim(&mut self, token: impl Into<String>) -> Result<(), ClaimError> {
        match self.state {
            DurableState::Queued | DurableState::RetryWaiting => {
                self.state = DurableState::InFlight;
                self.attempt_count += 1;
                self.claim_token = Some(token.into());
                Ok(())
            }
            other => Err(ClaimError::NotClaimable(other)),
        }
    }

    fn check_claim(&self, token: &str) -> Result<(), ClaimError> {
        match (&self.state, &self.claim_token) {
            (DurableState::InFlight, Some(current)) if current == token => Ok(()),
            (DurableState::InFlight, _) => Err(ClaimError::StaleClaim),
            _ => Err(ClaimError::NotInFlight),
        }
    }

    pub fn complete(&mut self, token: &str) -> Result<(), ClaimError> {
        self.check_claim(token)?;
        self.claim_token = None;
        Ok(())
    }

    pub fn fail_retryable(
        &mut self,
        token: &str,
        code: impl Into<String>,
        now_ms: i64,
    ) -> Result<i64, ClaimError> {
        self.check_claim(token)?;
        let delay = retry_delay_ms(self.request_id, self.attempt_count);
        self.state = DurableState::RetryWaiting;
        self.claim_token = None;
        self.last_error_code = Some(code.into());
        self.next_attempt_ms = now_ms + delay;
        Ok(delay)
    }

    pub fn fail_terminal(
        &mut self,
        token: &str,
        code: impl Into<String>,
    ) -> Result<(), ClaimError> {
        self.check_claim(token)?;
        self.state = DurableState::TerminalFailure;
        self.claim_token = None;
        self.last_error_code = Some(code.into());
        Ok(())
    }
}

/// Head-of-line ordering: at most one operation per ordering key may be in
/// flight, so two mutations against the same chat cannot land out of order.
pub fn next_claimable(operations: &[DurableOperation], now_ms: i64) -> Option<&DurableOperation> {
    let blocked: Vec<&str> = operations
        .iter()
        .filter(|op| op.state == DurableState::InFlight)
        .map(|op| op.ordering_key.as_str())
        .collect();

    operations
        .iter()
        .find(|op| op.is_ready(now_ms) && !blocked.contains(&op.ordering_key.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u64) -> RequestId {
        RequestId::new(raw).unwrap()
    }

    #[test]
    fn batch_limits_match_the_reference() {
        let limits = PushBatchLimits::default();
        assert_eq!(limits.max_events, 32);
        assert_eq!(limits.max_mutations, 512);
        assert_eq!(limits.max_bytes, 8 * 1024 * 1024);
    }

    #[test]
    fn a_batch_flushes_on_the_event_cap() {
        let mut batch = PushBatch::new(PushBatchLimits::default());
        for _ in 0..31 {
            assert_eq!(batch.push(1, 1), None);
        }
        assert_eq!(batch.push(1, 1), Some(FlushReason::Events));
    }

    #[test]
    fn a_batch_flushes_on_the_mutation_cap_before_the_event_cap() {
        let mut batch = PushBatch::new(PushBatchLimits::default());
        assert_eq!(batch.push(512, 1), Some(FlushReason::Mutations));
        assert_eq!(batch.events(), 1);
    }

    #[test]
    fn a_batch_flushes_on_the_byte_cap() {
        let mut batch = PushBatch::new(PushBatchLimits::default());
        assert_eq!(batch.push(1, 8 * 1024 * 1024), Some(FlushReason::Bytes));
    }

    #[test]
    fn taking_a_batch_resets_it() {
        let mut batch = PushBatch::new(PushBatchLimits::default());
        batch.push(3, 10);
        assert_eq!(batch.take(), (1, 3, 10));
        assert!(batch.is_empty());
        assert_eq!(batch.take(), (0, 0, 0));
    }

    #[test]
    fn backoff_doubles_and_then_saturates() {
        // 16000 divides every jitter range, so this id contributes zero jitter.
        let request = id(16_000);
        let base = |attempt| retry_delay_ms(request, attempt) / 1_000;
        assert_eq!(base(1), 1);
        assert_eq!(base(2), 2);
        assert_eq!(base(3), 4);
        assert_eq!(base(7), 64);
        assert_eq!(base(8), 64, "the exponent is capped at 6");
        assert_eq!(base(99), 64);
    }

    #[test]
    fn a_zero_attempt_count_does_not_underflow() {
        assert!(retry_delay_ms(id(1), 0) >= 1_000);
    }

    #[test]
    fn jitter_stays_within_a_quarter_of_the_base_and_is_deterministic() {
        for attempt in 1..=8u32 {
            let base = 1_000i64 << attempt.saturating_sub(1).min(6);
            for raw in [1u64, 7, 12_345, u64::MAX] {
                let delay = retry_delay_ms(id(raw), attempt);
                assert!(delay >= base);
                assert!(delay < base + base / 4 + 1);
                assert_eq!(
                    delay,
                    retry_delay_ms(id(raw), attempt),
                    "must be reproducible"
                );
            }
        }
    }

    #[test]
    fn different_requests_spread_out() {
        let a = retry_delay_ms(id(1_000_003), 4);
        let b = retry_delay_ms(id(2_000_017), 4);
        assert_ne!(a, b, "identical delays would resynchronise a retry storm");
    }

    #[test]
    fn claiming_moves_to_in_flight_and_counts_the_attempt() {
        let mut op = DurableOperation::queued(id(1), "chat:1", 0);
        op.claim("t1").unwrap();
        assert_eq!(op.state, DurableState::InFlight);
        assert_eq!(op.attempt_count, 1);
        assert_eq!(op.claim_token.as_deref(), Some("t1"));
    }

    #[test]
    fn an_in_flight_operation_cannot_be_claimed_twice() {
        let mut op = DurableOperation::queued(id(1), "chat:1", 0);
        op.claim("t1").unwrap();
        assert_eq!(
            op.claim("t2"),
            Err(ClaimError::NotClaimable(DurableState::InFlight))
        );
    }

    #[test]
    fn a_stale_claim_cannot_complete_an_operation_someone_else_reclaimed() {
        let mut op = DurableOperation::queued(id(1), "chat:1", 0);
        op.claim("first").unwrap();
        op.fail_retryable("first", "network", 0).unwrap();
        op.claim("second").unwrap();

        assert_eq!(op.complete("first"), Err(ClaimError::StaleClaim));
        assert!(op.complete("second").is_ok());
    }

    #[test]
    fn a_retryable_failure_schedules_the_next_attempt() {
        let mut op = DurableOperation::queued(id(1), "chat:1", 0);
        op.claim("t").unwrap();
        let delay = op.fail_retryable("t", "timeout", 10_000).unwrap();

        assert_eq!(op.state, DurableState::RetryWaiting);
        assert_eq!(op.next_attempt_ms, 10_000 + delay);
        assert_eq!(op.last_error_code.as_deref(), Some("timeout"));
        assert!(op.claim_token.is_none());
    }

    #[test]
    fn a_terminal_failure_is_not_retried() {
        let mut op = DurableOperation::queued(id(1), "chat:1", 0);
        op.claim("t").unwrap();
        op.fail_terminal("t", "forbidden").unwrap();

        assert_eq!(op.state, DurableState::TerminalFailure);
        assert!(
            !op.is_ready(i64::MAX),
            "a terminal operation is never ready"
        );
        assert_eq!(
            op.claim("t2"),
            Err(ClaimError::NotClaimable(DurableState::TerminalFailure))
        );
    }

    #[test]
    fn an_operation_waiting_for_its_backoff_is_not_ready_yet() {
        let mut op = DurableOperation::queued(id(1), "chat:1", 0);
        op.claim("t").unwrap();
        op.fail_retryable("t", "timeout", 1_000).unwrap();

        assert!(!op.is_ready(1_000));
        assert!(op.is_ready(op.next_attempt_ms));
    }

    #[test]
    fn one_ordering_key_admits_one_operation_at_a_time() {
        let mut first = DurableOperation::queued(id(1), "chat:1", 0);
        let second = DurableOperation::queued(id(2), "chat:1", 0);
        let other_lane = DurableOperation::queued(id(3), "chat:2", 0);

        first.claim("t").unwrap();
        let operations = vec![first, second, other_lane];

        let next = next_claimable(&operations, 0).expect("another lane should be claimable");
        assert_eq!(
            next.ordering_key, "chat:2",
            "the blocked lane must not admit a second operation"
        );
    }

    #[test]
    fn a_lane_resumes_once_its_operation_finishes() {
        let mut first = DurableOperation::queued(id(1), "chat:1", 0);
        let second = DurableOperation::queued(id(2), "chat:1", 0);
        first.claim("t").unwrap();
        first.complete("t").unwrap();
        first.state = DurableState::TerminalFailure;

        let operations = vec![first, second];
        assert_eq!(
            next_claimable(&operations, 0).map(|op| op.request_id),
            Some(id(2))
        );
    }

    #[test]
    fn nothing_is_claimable_before_its_scheduled_time() {
        let operations = vec![DurableOperation::queued(id(1), "chat:1", 5_000)];
        assert!(next_claimable(&operations, 4_999).is_none());
        assert!(next_claimable(&operations, 5_000).is_some());
    }
}
