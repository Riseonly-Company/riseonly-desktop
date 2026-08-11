use std::collections::{BTreeMap, BTreeSet};

use super::core::rise_presence_rpc::PresenceUpdate;

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PresenceState {
    pub is_online: bool,
    pub last_seen_ms: Option<i64>,
    pub in_chat_id: Option<String>,
    pub is_known: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PresenceIndicator {
    Hidden,
    Online,
    InChat,
}

#[derive(Default)]
pub struct PresenceRepository {
    states: BTreeMap<String, PresenceState>,
    desired_by_owner: BTreeMap<String, BTreeSet<String>>,
    subscribed: BTreeSet<String>,
    revision: u64,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SubscriptionDelta {
    pub subscribe: Vec<String>,
    pub unsubscribe: Vec<String>,
}

impl SubscriptionDelta {
    pub fn is_empty(&self) -> bool {
        self.subscribe.is_empty() && self.unsubscribe.is_empty()
    }
}

impl PresenceRepository {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn state(&self, user_id: &str) -> Option<&PresenceState> {
        self.states.get(user_id)
    }

    pub fn subscribed(&self) -> Vec<String> {
        self.subscribed.iter().cloned().collect()
    }

    pub fn apply(&mut self, update: PresenceUpdate) -> bool {
        let entry = self.states.entry(update.user_id.clone()).or_default();
        let before = entry.clone();

        if let Some(is_online) = update.is_online {
            entry.is_online = is_online;
            entry.is_known = true;
        }
        if let Some(last_seen) = update.last_seen_ms {
            entry.last_seen_ms = Some(last_seen);
        }
        if update.clears_in_chat {
            entry.in_chat_id = None;
        } else if update.in_chat_id.is_some() {
            entry.in_chat_id = update.in_chat_id;
        }

        let changed = *entry != before;
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }

    pub fn set_desired(
        &mut self,
        owner: &str,
        user_ids: impl IntoIterator<Item = String>,
        self_id: Option<&str>,
    ) -> SubscriptionDelta {
        let cleaned: BTreeSet<String> = user_ids
            .into_iter()
            .filter(|id| !id.is_empty() && Some(id.as_str()) != self_id)
            .collect();

        if cleaned.is_empty() {
            self.desired_by_owner.remove(owner);
        } else {
            self.desired_by_owner.insert(owner.to_owned(), cleaned);
        }

        self.reconcile()
    }

    pub fn stop_observing(&mut self, owner: &str) -> SubscriptionDelta {
        self.desired_by_owner.remove(owner);
        self.reconcile()
    }

    fn reconcile(&mut self) -> SubscriptionDelta {
        let union: BTreeSet<String> = self
            .desired_by_owner
            .values()
            .flat_map(|set| set.iter().cloned())
            .collect();

        let delta = SubscriptionDelta {
            subscribe: union.difference(&self.subscribed).cloned().collect(),
            unsubscribe: self.subscribed.difference(&union).cloned().collect(),
        };

        self.subscribed = union;
        delta
    }

    // Subscriptions live on the connection: a reconnect silently drops every one.
    pub fn resubscribe_all(&self) -> Vec<String> {
        self.subscribed.iter().cloned().collect()
    }

    pub fn reset(&mut self) {
        self.states.clear();
        self.desired_by_owner.clear();
        self.subscribed.clear();
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn indicator(
        &self,
        user_id: &str,
        chat_id: &str,
        fallback_online: bool,
    ) -> PresenceIndicator {
        let state = self.states.get(user_id);

        if let Some(state) = state
            && state.in_chat_id.as_deref() == Some(chat_id)
            && !chat_id.is_empty()
        {
            return PresenceIndicator::InChat;
        }

        let online = match state {
            Some(state) if state.is_known => state.is_online,
            _ => fallback_online,
        };

        if online {
            PresenceIndicator::Online
        } else {
            PresenceIndicator::Hidden
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::presence::engine::core::rise_presence_rpc::{
        decode_in_chat, decode_online_status,
    };
    use serde_json::json;

    fn online(user_id: &str) -> PresenceUpdate {
        decode_online_status(&json!({"user_id": user_id, "is_online": true})).unwrap()
    }

    fn offline(user_id: &str) -> PresenceUpdate {
        decode_online_status(&json!({"user_id": user_id, "is_online": false})).unwrap()
    }

    fn in_chat(user_id: &str, chat_id: &str) -> PresenceUpdate {
        decode_in_chat(&json!({"user_id": user_id, "in_chat": true, "chat_id": chat_id})).unwrap()
    }

    #[test]
    fn two_screens_watching_the_same_user_subscribe_once() {
        let mut repository = PresenceRepository::new();

        let first = repository.set_desired("ChatList", ["u1".into(), "u2".into()], None);
        assert_eq!(first.subscribe, vec!["u1", "u2"]);

        let second = repository.set_desired("Chat:c1", ["u1".into()], None);
        assert!(
            second.is_empty(),
            "a second screen onto the same user must not re-subscribe to them"
        );
    }

    #[test]
    fn closing_one_screen_only_unsubscribes_what_nobody_else_wants() {
        let mut repository = PresenceRepository::new();
        repository.set_desired("ChatList", ["u1".into(), "u2".into()], None);
        repository.set_desired("Chat:c1", ["u1".into()], None);

        let delta = repository.stop_observing("ChatList");
        assert_eq!(delta.unsubscribe, vec!["u2"]);
        assert!(delta.subscribe.is_empty());
        assert_eq!(repository.subscribed(), vec!["u1"]);
    }

    #[test]
    fn a_user_never_subscribes_to_their_own_presence() {
        let mut repository = PresenceRepository::new();
        let delta = repository.set_desired("Chat:c1", ["me".into(), "u1".into()], Some("me"));

        assert_eq!(
            delta.subscribe,
            vec!["u1"],
            "the server does not push a user their own presence"
        );
    }

    #[test]
    fn an_empty_set_releases_the_owner_rather_than_leaving_it_registered() {
        let mut repository = PresenceRepository::new();
        repository.set_desired("Chat:c1", ["u1".into()], None);

        let delta = repository.set_desired("Chat:c1", Vec::<String>::new(), None);
        assert_eq!(delta.unsubscribe, vec!["u1"]);
        assert!(repository.subscribed().is_empty());
    }

    #[test]
    fn a_reconnect_re_subscribes_everything_the_connection_lost() {
        let mut repository = PresenceRepository::new();
        repository.set_desired("ChatList", ["u1".into(), "u2".into()], None);

        assert_eq!(
            repository.resubscribe_all(),
            vec!["u1", "u2"],
            "subscriptions live on the connection, so a reconnect silently drops them"
        );
    }

    #[test]
    fn an_update_that_changes_nothing_does_not_bump_the_revision() {
        let mut repository = PresenceRepository::new();

        assert!(repository.apply(online("u1")));
        let revision = repository.revision();

        assert!(
            !repository.apply(online("u1")),
            "a repeated push must not invalidate every row that draws this user"
        );
        assert_eq!(repository.revision(), revision);
    }

    #[test]
    fn going_offline_takes_the_in_chat_marker_with_it() {
        let mut repository = PresenceRepository::new();
        repository.apply(online("u1"));
        repository.apply(in_chat("u1", "c1"));
        assert_eq!(
            repository.indicator("u1", "c1", false),
            PresenceIndicator::InChat
        );

        repository.apply(offline("u1"));
        assert_eq!(
            repository.indicator("u1", "c1", false),
            PresenceIndicator::Hidden
        );
    }

    #[test]
    fn the_in_chat_marker_only_shows_in_the_conversation_it_names() {
        let mut repository = PresenceRepository::new();
        repository.apply(online("u1"));
        repository.apply(in_chat("u1", "c1"));

        assert_eq!(
            repository.indicator("u1", "c2", false),
            PresenceIndicator::Online
        );
    }

    #[test]
    fn an_unknown_user_shows_what_the_payload_that_built_the_row_claimed() {
        let repository = PresenceRepository::new();

        assert_eq!(
            repository.indicator("u1", "c1", true),
            PresenceIndicator::Online,
            "otherwise every row reads offline until the first push arrives"
        );
        assert_eq!(
            repository.indicator("u1", "c1", false),
            PresenceIndicator::Hidden
        );
    }

    #[test]
    fn live_presence_beats_the_payloads_claim_once_it_exists() {
        let mut repository = PresenceRepository::new();
        repository.apply(offline("u1"));

        assert_eq!(
            repository.indicator("u1", "c1", true),
            PresenceIndicator::Hidden,
            "a stale cached flag must not outrank what the server just said"
        );
    }

    #[test]
    fn a_reset_forgets_the_previous_accounts_contacts() {
        let mut repository = PresenceRepository::new();
        repository.set_desired("ChatList", ["u1".into()], None);
        repository.apply(online("u1"));

        repository.reset();

        assert!(repository.subscribed().is_empty());
        assert_eq!(repository.state("u1"), None);
        assert_eq!(
            repository.indicator("u1", "c1", false),
            PresenceIndicator::Hidden
        );
    }

    #[test]
    fn last_seen_survives_an_update_that_does_not_carry_one() {
        let mut repository = PresenceRepository::new();
        repository.apply(
            decode_online_status(&json!({
                "user_id": "u1", "is_online": false, "last_seen": 1_700_000_000_000i64
            }))
            .unwrap(),
        );

        repository.apply(in_chat("u1", "c1"));
        assert_eq!(
            repository.state("u1").unwrap().last_seen_ms,
            Some(1_700_000_000_000)
        );
    }
}
