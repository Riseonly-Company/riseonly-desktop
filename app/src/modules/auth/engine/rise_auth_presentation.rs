use std::sync::Arc;

use tokio::sync::watch;

use super::rise_auth_engine_models::{AccountSummary, AuthUser};

/// What the sign-up flow knows about the tag the user is typing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TagAvailability {
    #[default]
    Idle,
    Checking,
    Available,
    Taken,
    Invalid,
    /// The check itself failed. Distinct from `Taken` on purpose: a network
    /// error must not tell somebody their tag belongs to another person.
    Unknown,
}

/// Where the multi-step sign-up has got to.
///
/// The Telegram hop is not incidental: the code is delivered by a bot, so
/// send-code succeeds by handing back a bot username and a phone payload, and
/// the next thing the user does is leave the app. The flow has to survive that.
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
}

/// The immutable snapshot every auth selector reads.
///
/// Bounded and prepared off the UI thread, published in one commit. The revision
/// is what lets a reader skip work when nothing changed — without it the socket
/// reconnecting would redraw the whole shell.
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
    /// The id every wire request is stamped with.
    ///
    /// The reference prefers the token's own claim over the profile: a profile
    /// can be stale after a switch, and the token is what the gateway will
    /// actually authenticate the call as.
    pub fn account_limit_reached(&self, is_premium: bool) -> bool {
        !super::rise_auth_engine_models::AccountLimitPolicy::can_add(
            self.accounts.len(),
            is_premium,
        )
    }
}

/// Publishes snapshots, and refuses to publish one that changed nothing.
///
/// GPUI has no automatic dependency tracking, so a store reading this will
/// `cx.notify()` on every change it sees. Republishing an identical snapshot
/// would invalidate the shell on every socket heartbeat.
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
        // `send_replace`, not `send`: `send` refuses and leaves the value
        // UNCHANGED when every receiver has been dropped, which would make the
        // published snapshot silently stale the moment the last view closed.
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
