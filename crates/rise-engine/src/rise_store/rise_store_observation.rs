use std::collections::HashSet;

use thiserror::Error;
use tokio::sync::broadcast;

const CHANNEL_CAPACITY: usize = 32;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RevisionChange {
    pub view_key: String,
    pub revision: i64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ObserveError {
    #[error("observer fell {0} revisions behind and must re-read from the store")]
    Overflow(u64),
    #[error("the store closed")]
    Closed,
}

/// Broadcasts which materialized views moved, never what changed inside them.
///
/// A subscriber that falls behind gets [`ObserveError::Overflow`] instead of a
/// silent drop; the correct response is to re-read the view, not to retry.
#[derive(Clone)]
pub struct RevisionBus {
    sender: broadcast::Sender<RevisionChange>,
}

impl RevisionBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { sender }
    }

    pub fn publish(&self, view_key: impl Into<String>, revision: i64) {
        let _ = self.sender.send(RevisionChange {
            view_key: view_key.into(),
            revision,
        });
    }

    pub fn subscribe(&self) -> RevisionSubscription {
        RevisionSubscription {
            receiver: self.sender.subscribe(),
            watching: None,
        }
    }

    /// A subscription that only wakes for the views it was asked about.
    pub fn subscribe_to(&self, keys: impl IntoIterator<Item = String>) -> RevisionSubscription {
        RevisionSubscription {
            receiver: self.sender.subscribe(),
            watching: Some(keys.into_iter().collect()),
        }
    }

    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for RevisionBus {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RevisionSubscription {
    receiver: broadcast::Receiver<RevisionChange>,
    watching: Option<HashSet<String>>,
}

impl RevisionSubscription {
    pub async fn next(&mut self) -> Result<RevisionChange, ObserveError> {
        loop {
            match self.receiver.recv().await {
                Ok(change) => {
                    if self
                        .watching
                        .as_ref()
                        .is_none_or(|keys| keys.contains(&change.view_key))
                    {
                        return Ok(change);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    return Err(ObserveError::Overflow(missed));
                }
                Err(broadcast::error::RecvError::Closed) => return Err(ObserveError::Closed),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_subscriber_receives_a_published_change() {
        let bus = RevisionBus::new();
        let mut subscription = bus.subscribe();

        bus.publish("message.timeline:c1", 7);

        let change = subscription.next().await.unwrap();
        assert_eq!(change.view_key, "message.timeline:c1");
        assert_eq!(change.revision, 7);
    }

    #[tokio::test]
    async fn publishing_without_subscribers_is_not_an_error() {
        let bus = RevisionBus::new();
        bus.publish("nobody.listening", 1);
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn a_scoped_subscription_ignores_other_views() {
        let bus = RevisionBus::new();
        let mut subscription = bus.subscribe_to(["chat.list".to_owned()]);

        bus.publish("message.timeline:c1", 1);
        bus.publish("post.feed", 2);
        bus.publish("chat.list", 3);

        let change = subscription.next().await.unwrap();
        assert_eq!(
            change.view_key, "chat.list",
            "a scoped subscriber must not wake for views it does not render"
        );
        assert_eq!(change.revision, 3);
    }

    #[tokio::test]
    async fn two_subscribers_both_see_the_same_change() {
        let bus = RevisionBus::new();
        let mut first = bus.subscribe();
        let mut second = bus.subscribe();

        bus.publish("shared", 1);

        assert_eq!(first.next().await.unwrap().revision, 1);
        assert_eq!(second.next().await.unwrap().revision, 1);
    }

    #[tokio::test]
    async fn a_slow_subscriber_is_told_it_fell_behind_rather_than_losing_changes_silently() {
        let bus = RevisionBus::new();
        let mut subscription = bus.subscribe();

        for revision in 0..(CHANNEL_CAPACITY as i64 + 8) {
            bus.publish("busy.view", revision);
        }

        let result = subscription.next().await;
        assert!(
            matches!(result, Err(ObserveError::Overflow(_))),
            "a dropped revision must surface as an error, not a stale snapshot"
        );
    }

    #[tokio::test]
    async fn a_subscription_recovers_after_an_overflow() {
        let bus = RevisionBus::new();
        let mut subscription = bus.subscribe();

        for revision in 0..(CHANNEL_CAPACITY as i64 + 8) {
            bus.publish("busy.view", revision);
        }
        assert!(subscription.next().await.is_err());

        let change = subscription.next().await.unwrap();
        assert!(change.revision > 0);
    }

    #[tokio::test]
    async fn dropping_the_bus_closes_its_subscriptions() {
        let bus = RevisionBus::new();
        let mut subscription = bus.subscribe();
        drop(bus);

        assert_eq!(subscription.next().await, Err(ObserveError::Closed));
    }

    #[tokio::test]
    async fn a_clone_of_the_bus_feeds_the_same_subscribers() {
        let bus = RevisionBus::new();
        let mut subscription = bus.subscribe();
        let clone = bus.clone();

        clone.publish("via.clone", 5);

        assert_eq!(subscription.next().await.unwrap().revision, 5);
    }
}
