use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnectionState {
    Idle,
    Connecting,
    Connected,
    Reconnecting,
}

impl ConnectionState {
    pub fn is_connected(self) -> bool {
        matches!(self, Self::Connected)
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum SocketCredential {
    #[default]
    Anonymous,
    Bearer(String),
}

impl SocketCredential {
    pub fn from_access_token(token: &str) -> Self {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return Self::Anonymous;
        }
        Self::Bearer(trimmed.to_owned())
    }

    // `Bearer.<jwt>`, dot not space: whitespace is illegal here and lands the socket anonymous.
    pub fn subprotocol(&self) -> Option<String> {
        match self {
            Self::Anonymous => None,
            Self::Bearer(token) => Some(format!("Bearer.{token}")),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BackoffSchedule {
    seed: u64,
}

impl BackoffSchedule {
    pub const BASE: Duration = Duration::from_secs(1);
    pub const CEILING: Duration = Duration::from_secs(60);

    pub fn new(seed: u64) -> Self {
        Self { seed }
    }

    pub fn delay_for(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }

        let exponent = attempt.saturating_sub(1).min(6);
        let base_ms = (Self::BASE.as_millis() as u64) << exponent;
        let capped = base_ms.min(Self::CEILING.as_millis() as u64);
        let jitter_range = (capped / 4).max(1);
        let jitter = (self.seed.wrapping_mul(attempt as u64 + 1)) % jitter_range;

        Duration::from_millis(capped + jitter)
    }
}

pub fn requires_reconnect(current: &SocketCredential, next: &SocketCredential) -> bool {
    current != next
}

pub fn socket_url(configured: &str) -> String {
    let trimmed = configured.trim().trim_end_matches('/');
    let after_scheme = trimmed.split_once("://").map_or(trimmed, |(_, rest)| rest);

    if after_scheme.contains('/') {
        return trimmed.to_owned();
    }

    format!("{trimmed}/ws")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_blank_token_is_an_anonymous_socket_rather_than_a_broken_one() {
        assert_eq!(
            SocketCredential::from_access_token(""),
            SocketCredential::Anonymous
        );
        assert_eq!(
            SocketCredential::from_access_token("   "),
            SocketCredential::Anonymous
        );
        assert_eq!(
            SocketCredential::from_access_token("  abc  "),
            SocketCredential::Bearer("abc".into())
        );
    }

    #[test]
    fn the_subprotocol_is_the_prefix_the_gateway_strips() {
        assert_eq!(
            SocketCredential::Bearer("a.b.c".into()).subprotocol(),
            Some("Bearer.a.b.c".to_owned())
        );
        assert_eq!(SocketCredential::Anonymous.subprotocol(), None);
    }

    #[test]
    fn the_subprotocol_never_contains_whitespace() {
        let value = SocketCredential::Bearer("header.payload.signature".into())
            .subprotocol()
            .unwrap();
        assert!(
            !value.chars().any(char::is_whitespace),
            "a subprotocol token with a space is rejected at the HTTP layer, and the \
             gateway would accept the upgrade anonymously instead of refusing it"
        );
    }

    #[test]
    fn backoff_grows_and_stops_at_the_ceiling() {
        let schedule = BackoffSchedule::new(0);
        assert_eq!(schedule.delay_for(0), Duration::ZERO);
        assert_eq!(schedule.delay_for(1), Duration::from_secs(1));
        assert_eq!(schedule.delay_for(2), Duration::from_secs(2));
        assert_eq!(schedule.delay_for(3), Duration::from_secs(4));

        for attempt in 7..40 {
            assert!(
                schedule.delay_for(attempt) <= BackoffSchedule::CEILING + Duration::from_secs(16),
                "attempt {attempt} exceeded the ceiling plus its jitter"
            );
        }
    }

    #[test]
    fn two_clients_do_not_retry_in_lockstep() {
        let first = BackoffSchedule::new(7);
        let second = BackoffSchedule::new(913);

        let differing = (1..8)
            .filter(|attempt| first.delay_for(*attempt) != second.delay_for(*attempt))
            .count();
        assert!(
            differing >= 5,
            "a gateway restart would bring every client back at the same instant"
        );
    }

    #[test]
    fn the_same_seed_gives_the_same_schedule_twice() {
        let schedule = BackoffSchedule::new(42);
        for attempt in 0..10 {
            assert_eq!(schedule.delay_for(attempt), schedule.delay_for(attempt));
        }
    }

    #[test]
    fn a_rotated_token_forces_a_new_handshake() {
        let old = SocketCredential::Bearer("one".into());
        let new = SocketCredential::Bearer("two".into());

        assert!(requires_reconnect(&old, &new));
        assert!(!requires_reconnect(&old, &old.clone()));
        assert!(
            requires_reconnect(&old, &SocketCredential::Anonymous),
            "signing out must not leave an authenticated socket open"
        );
        assert!(requires_reconnect(&SocketCredential::Anonymous, &new));
    }

    #[test]
    fn the_socket_url_gets_the_path_the_ingress_routes_on() {
        assert_eq!(socket_url("wss://riseonly.net"), "wss://riseonly.net/ws");
        assert_eq!(
            socket_url("wss://staging.riseonly.net/"),
            "wss://staging.riseonly.net/ws"
        );
        assert_eq!(socket_url("ws://127.0.0.1:8085"), "ws://127.0.0.1:8085/ws");
    }

    #[test]
    fn a_configured_path_is_left_alone() {
        assert_eq!(
            socket_url("wss://tunnel.example/socket"),
            "wss://tunnel.example/socket"
        );
        assert_eq!(
            socket_url("wss://tunnel.example/ws"),
            "wss://tunnel.example/ws",
            "appending twice would send the handshake to /ws/ws"
        );
    }
}
