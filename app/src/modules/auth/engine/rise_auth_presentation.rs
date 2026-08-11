use std::sync::Arc;

use tokio::sync::watch;

use super::rise_auth_engine_models::{AccountSummary, AuthUser};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TagAvailability {
    #[default]
    Idle,
    Checking,
    Available,
    Taken,
    Invalid,
    Unknown,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct AuthFlowState {
    pub is_busy: bool,
    pub error_key: Option<&'static str>,
    pub bot_username: Option<String>,
    pub phone_payload: Option<String>,
    pub telegram_redirect_active: bool,
    pub code_entry_active: bool,
    pub tag_availability: TagAvailability,
    pub checked_tag: Option<String>,
    pub tag_problem_key: Option<&'static str>,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct AuthSnapshot {
    pub revision: u64,
    pub is_authenticated: bool,
    pub active: Option<AuthUser>,
    pub active_account_id: Option<String>,
    pub accounts: Vec<AccountSummary>,
    pub flow: AuthFlowState,
    pub is_restoring: bool,
}

impl AuthSnapshot {
    pub fn account_limit_reached(&self, is_premium: bool) -> bool {
        !super::rise_auth_engine_models::AccountLimitPolicy::can_add(
            self.accounts.len(),
            is_premium,
        )
    }
}

pub struct RiseAuthPresentation {
    sender: watch::Sender<Arc<AuthSnapshot>>,
    revision: u64,
}

impl RiseAuthPresentation {
    pub fn new() -> (Self, watch::Receiver<Arc<AuthSnapshot>>) {
        let (sender, receiver) = watch::channel(Arc::new(AuthSnapshot::default()));
        (
            Self {
                sender,
                revision: 0,
            },
            receiver,
        )
    }

    pub fn current(&self) -> Arc<AuthSnapshot> {
        Arc::clone(&self.sender.borrow())
    }

    pub fn publish(&mut self, mut snapshot: AuthSnapshot) -> bool {
        let previous = self.sender.borrow().clone();
        snapshot.revision = previous.revision;
        if snapshot == *previous {
            return false;
        }

        self.revision = self.revision.wrapping_add(1);
        snapshot.revision = self.revision;
        // `send` leaves the value unchanged once every receiver is dropped; `send_replace` cannot.
        self.sender.send_replace(Arc::new(snapshot));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: &str) -> AuthUser {
        AuthUser {
            id: id.to_owned(),
            ..AuthUser::default()
        }
    }

    #[test]
    fn an_unchanged_snapshot_is_not_republished() {
        let (mut presentation, receiver) = RiseAuthPresentation::new();

        let snapshot = AuthSnapshot {
            is_authenticated: true,
            active: Some(user("u1")),
            ..AuthSnapshot::default()
        };

        assert!(presentation.publish(snapshot.clone()));
        let first = receiver.borrow().revision;

        assert!(
            !presentation.publish(snapshot),
            "a heartbeat that changes nothing must not invalidate the shell"
        );
        assert_eq!(receiver.borrow().revision, first);
    }

    #[test]
    fn a_changed_snapshot_gets_a_new_revision() {
        let (mut presentation, receiver) = RiseAuthPresentation::new();

        presentation.publish(AuthSnapshot {
            is_authenticated: true,
            active: Some(user("u1")),
            ..AuthSnapshot::default()
        });
        let first = receiver.borrow().revision;

        presentation.publish(AuthSnapshot {
            is_authenticated: true,
            active: Some(user("u2")),
            ..AuthSnapshot::default()
        });

        assert!(receiver.borrow().revision > first);
        assert_eq!(receiver.borrow().active.as_ref().unwrap().id, "u2");
    }

    #[test]
    fn a_flow_change_alone_is_enough_to_republish() {
        let (mut presentation, receiver) = RiseAuthPresentation::new();
        presentation.publish(AuthSnapshot::default());
        let first = receiver.borrow().revision;

        assert!(presentation.publish(AuthSnapshot {
            flow: AuthFlowState {
                is_busy: true,
                ..AuthFlowState::default()
            },
            ..AuthSnapshot::default()
        }));
        assert!(receiver.borrow().revision > first);
    }

    #[test]
    fn a_failed_availability_check_is_not_the_same_as_a_taken_tag() {
        assert_ne!(TagAvailability::Unknown, TagAvailability::Taken);
    }

    #[test]
    fn the_first_snapshot_a_reader_sees_is_signed_out_rather_than_empty_and_authenticated() {
        let (_presentation, receiver) = RiseAuthPresentation::new();
        let snapshot = receiver.borrow();

        assert!(!snapshot.is_authenticated);
        assert_eq!(snapshot.revision, 0);
        assert!(snapshot.active.is_none());
    }
}
