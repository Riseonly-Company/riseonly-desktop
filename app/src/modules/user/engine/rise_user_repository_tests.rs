use std::sync::Mutex;

use super::*;
use serde_json::json;

struct FakeTransport {
    replies: Mutex<Vec<Result<Value, WireError>>>,
    calls: Mutex<Vec<(&'static str, Value)>>,
}

impl FakeTransport {
    fn new(replies: Vec<Result<Value, WireError>>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into_iter().rev().collect()),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<(&'static str, Value)> {
        self.calls.lock().unwrap().clone()
    }
}

impl UserTransport for FakeTransport {
    fn call(
        &self,
        descriptor: &'static MethodDescriptor,
        body: Value,
    ) -> BoxFuture<'static, Result<Value, WireError>> {
        self.calls.lock().unwrap().push((descriptor.method, body));
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop()
            .unwrap_or(Err(WireError::NotConnected));
        Box::pin(async move { reply })
    }
}

fn profile(id: &str, tag: &str) -> Value {
    json!({
        "profile": {
            "id": id,
            "tag": tag,
            "name": "Aianov",
            "subscribers_count": 10,
            "friends_count": 2,
            "more": {"description": "Day 370", "logo": "https://cdn/a.png"}
        }
    })
}

fn state(transport: Arc<dyn UserTransport>) -> (UserState, watch::Receiver<Arc<UserSnapshot>>) {
    let (publisher, receiver) = watch::channel(Arc::new(UserSnapshot::default()));
    let (commands, inbox) = mpsc::channel(64);
    // Held so the debounced commit has somewhere to land; the tests drive
    // `commit_follow` directly rather than waiting out a real window.
    std::mem::forget(inbox);

    (
        UserState {
            transport,
            publisher,
            revision: 0,
            viewer_id: Some("u1".into()),
            profiles: BTreeMap::new(),
            generations: BTreeMap::new(),
            committed: BTreeMap::new(),
            follow_debouncers: BTreeMap::new(),
            follow_debounce: Duration::from_millis(600),
            commands,
        },
        receiver,
    )
}

fn foreign(state: &UserState) -> ProfileModel {
    state.profiles[&ProfileKey::Id("u2".into())].profile.clone()
}

#[tokio::test]
async fn a_profile_lands_and_the_page_is_marked_loaded() {
    let transport = FakeTransport::new(vec![Ok(profile("u2", "friend"))]);
    let (mut state, _receiver) = state(transport.clone());

    state.load(ProfileKey::Id("u2".into())).await;

    let entry = state.profiles[&ProfileKey::Id("u2".into())].clone();
    assert_eq!(entry.status, PageStatus::Loaded);
    assert!(entry.did_load);
    assert_eq!(entry.profile.name, "Aianov");
    assert_eq!(
        entry.profile.avatar_url.as_deref(),
        Some("https://cdn/a.png")
    );
    assert_eq!(transport.calls()[0].0, "get_profile_by_id");
}

#[tokio::test]
async fn each_key_addresses_the_method_that_answers_for_it() {
    let transport = FakeTransport::new(vec![
        Ok(profile("u1", "me")),
        Ok(profile("u2", "friend")),
        Ok(profile("u3", "bytag")),
    ]);
    let (mut state, _receiver) = state(transport.clone());

    state.load(ProfileKey::Own).await;
    state.load(ProfileKey::Id("u2".into())).await;
    state.load(ProfileKey::Tag("bytag".into())).await;

    let methods: Vec<&str> = transport.calls().iter().map(|call| call.0).collect();
    assert_eq!(
        methods,
        vec!["get_my_profile", "get_profile_by_id", "get_profile_by_tag"]
    );
}

#[tokio::test]
async fn a_signed_out_viewer_can_still_read_a_profile_by_tag_and_nothing_else() {
    let transport = FakeTransport::new(vec![Ok(profile("u2", "friend"))]);
    let (mut state, _receiver) = state(transport.clone());
    state.viewer_id = None;

    state.load(ProfileKey::Id("u2".into())).await;
    assert!(transport.calls().is_empty());
    assert_eq!(
        state.profiles[&ProfileKey::Id("u2".into())].status,
        PageStatus::Failed
    );

    state.load(ProfileKey::Tag("friend".into())).await;
    assert_eq!(transport.calls().len(), 1);
    assert!(
        transport.calls()[0].1.get("user_id").is_none(),
        "an anonymous read must not send an empty viewer"
    );
}

#[tokio::test]
async fn reading_your_own_profile_teaches_the_repository_who_the_viewer_is() {
    let transport = FakeTransport::new(vec![Ok(profile("u9", "me"))]);
    let (mut state, receiver) = state(transport.clone());

    state.load(ProfileKey::Own).await;

    assert_eq!(state.viewer_id.as_deref(), Some("u9"));
    assert_eq!(receiver.borrow().viewer_id.as_deref(), Some("u9"));
}

#[tokio::test]
async fn opening_a_profile_that_is_already_loaded_asks_for_nothing() {
    let transport = FakeTransport::new(vec![Ok(profile("u2", "friend"))]);
    let (mut state, _receiver) = state(transport.clone());

    state.activate(ProfileKey::Id("u2".into())).await;
    state.activate(ProfileKey::Id("u2".into())).await;

    assert_eq!(transport.calls().len(), 1);
}

#[tokio::test]
async fn a_failed_read_keeps_the_cached_profile_on_screen() {
    let transport = FakeTransport::new(vec![
        Ok(profile("u2", "friend")),
        Err(WireError::NotConnected),
    ]);
    let (mut state, _receiver) = state(transport.clone());

    state.load(ProfileKey::Id("u2".into())).await;
    state.load(ProfileKey::Id("u2".into())).await;

    let entry = state.profiles[&ProfileKey::Id("u2".into())].clone();
    assert_eq!(entry.status, PageStatus::Failed);
    assert_eq!(entry.error_key, Some("profile_load_error"));
    assert_eq!(
        entry.profile.name, "Aianov",
        "a network error must not empty a screen that already had an answer"
    );
}

#[tokio::test]
async fn a_remote_error_string_fails_the_read_even_though_the_call_succeeded() {
    let transport = FakeTransport::new(vec![Ok(json!({"error": "User not found"}))]);
    let (mut state, _receiver) = state(transport.clone());

    state.load(ProfileKey::Id("u2".into())).await;

    assert_eq!(
        state.profiles[&ProfileKey::Id("u2".into())].status,
        PageStatus::Failed
    );
}

#[tokio::test]
async fn following_moves_the_relationship_and_the_count_before_anything_is_sent() {
    let transport = FakeTransport::new(vec![Ok(profile("u2", "friend"))]);
    let (mut state, _receiver) = state(transport.clone());
    state.load(ProfileKey::Id("u2".into())).await;

    state.toggle_follow(ProfileKey::Id("u2".into()));

    let profile = foreign(&state);
    assert!(profile.relationship.is_subbed);
    assert_eq!(profile.subscribers_count, 11);
    assert_eq!(
        transport.calls().len(),
        1,
        "the optimistic write happens before the debounce, so only the read has gone out"
    );
}

#[tokio::test]
async fn a_follow_committed_after_the_debounce_sends_the_friend_request() {
    let transport = FakeTransport::new(vec![
        Ok(profile("u2", "friend")),
        Ok(json!({"id": 77, "status": "accepted"})),
    ]);
    let (mut state, _receiver) = state(transport.clone());
    state.load(ProfileKey::Id("u2".into())).await;

    state.toggle_follow(ProfileKey::Id("u2".into()));
    let generation = state.generation(&ProfileKey::Id("u2".into()));
    state
        .commit_follow(ProfileKey::Id("u2".into()), generation)
        .await;

    let calls = transport.calls();
    assert_eq!(calls[1].0, "send_friend_request");
    assert_eq!(calls[1].1["receiver_id"], json!("u2"));
    assert_eq!(calls[1].1["user_id"], json!("u1"));

    let profile = foreign(&state);
    assert!(profile.relationship.is_subbed);
    assert_eq!(profile.relationship.friend_request_id, Some(77));
    assert!(!state.profiles[&ProfileKey::Id("u2".into())].follow_in_flight);
}

#[tokio::test]
async fn unfollowing_takes_the_relationship_the_server_answers_with() {
    let transport = FakeTransport::new(vec![
        Ok(
            json!({"profile": {"id": "u2", "tag": "f", "is_subbed": true, "is_friend": true,
                              "is_subscriber": true, "subscribers_count": 10, "friends_count": 3}}),
        ),
        Ok(json!({"is_friend": false, "is_subbed": false, "is_subscriber": true})),
    ]);
    let (mut state, _receiver) = state(transport.clone());
    state.load(ProfileKey::Id("u2".into())).await;

    state.toggle_follow(ProfileKey::Id("u2".into()));
    let generation = state.generation(&ProfileKey::Id("u2".into()));
    state
        .commit_follow(ProfileKey::Id("u2".into()), generation)
        .await;

    assert_eq!(transport.calls()[1].0, "unfollow_user");
    let profile = foreign(&state);
    assert!(!profile.relationship.is_subbed);
    assert!(!profile.relationship.is_friend);
    assert!(profile.relationship.is_subscriber);
    assert_eq!(profile.subscribers_count, 9);
    assert_eq!(profile.friends_count, 2);
}

#[tokio::test]
async fn toggling_back_to_where_it_started_sends_nothing_at_all() {
    let transport = FakeTransport::new(vec![Ok(profile("u2", "friend"))]);
    let (mut state, _receiver) = state(transport.clone());
    state.load(ProfileKey::Id("u2".into())).await;

    state.toggle_follow(ProfileKey::Id("u2".into()));
    state.toggle_follow(ProfileKey::Id("u2".into()));
    let generation = state.generation(&ProfileKey::Id("u2".into()));
    state
        .commit_follow(ProfileKey::Id("u2".into()), generation)
        .await;

    assert_eq!(transport.calls().len(), 1, "only the initial read");
    let profile = foreign(&state);
    assert!(!profile.relationship.is_subbed);
    assert_eq!(profile.subscribers_count, 10);
    assert!(!state.profiles[&ProfileKey::Id("u2".into())].follow_in_flight);
}

#[tokio::test]
async fn a_failed_follow_rolls_back_to_the_last_committed_relationship() {
    let transport = FakeTransport::new(vec![
        Ok(profile("u2", "friend")),
        Err(WireError::NotConnected),
    ]);
    let (mut state, _receiver) = state(transport.clone());
    state.load(ProfileKey::Id("u2".into())).await;

    state.toggle_follow(ProfileKey::Id("u2".into()));
    let generation = state.generation(&ProfileKey::Id("u2".into()));
    state
        .commit_follow(ProfileKey::Id("u2".into()), generation)
        .await;

    let profile = foreign(&state);
    assert!(!profile.relationship.is_subbed);
    assert_eq!(profile.subscribers_count, 10);
}

#[tokio::test]
async fn a_rejected_follow_rolls_back_even_though_the_call_succeeded() {
    let transport = FakeTransport::new(vec![
        Ok(profile("u2", "friend")),
        Ok(json!({"error": "blocked"})),
    ]);
    let (mut state, _receiver) = state(transport.clone());
    state.load(ProfileKey::Id("u2".into())).await;

    state.toggle_follow(ProfileKey::Id("u2".into()));
    let generation = state.generation(&ProfileKey::Id("u2".into()));
    state
        .commit_follow(ProfileKey::Id("u2".into()), generation)
        .await;

    assert!(!foreign(&state).relationship.is_subbed);
}

#[tokio::test]
async fn a_commit_from_a_stale_generation_never_reaches_the_wire() {
    let transport = FakeTransport::new(vec![Ok(profile("u2", "friend"))]);
    let (mut state, _receiver) = state(transport.clone());
    state.load(ProfileKey::Id("u2".into())).await;

    state.toggle_follow(ProfileKey::Id("u2".into()));
    let stale = state.generation(&ProfileKey::Id("u2".into()));
    state.toggle_follow(ProfileKey::Id("u2".into()));

    state
        .commit_follow(ProfileKey::Id("u2".into()), stale)
        .await;

    assert_eq!(transport.calls().len(), 1);
}

#[tokio::test]
async fn a_read_that_lands_during_a_follow_does_not_undo_it() {
    let transport = FakeTransport::new(vec![
        Ok(profile("u2", "friend")),
        Ok(profile("u2", "friend")),
    ]);
    let (mut state, _receiver) = state(transport.clone());
    state.load(ProfileKey::Id("u2".into())).await;

    state.toggle_follow(ProfileKey::Id("u2".into()));
    state.load(ProfileKey::Id("u2".into())).await;

    assert!(
        foreign(&state).relationship.is_subbed,
        "the pending follow is newer than the read that raced it"
    );
}

#[tokio::test]
async fn you_cannot_follow_yourself() {
    let transport = FakeTransport::new(vec![Ok(profile("u1", "me"))]);
    let (mut state, _receiver) = state(transport.clone());

    state.load(ProfileKey::Id("u1".into())).await;
    state.toggle_follow(ProfileKey::Id("u1".into()));

    assert!(
        !state.profiles[&ProfileKey::Id("u1".into())]
            .profile
            .relationship
            .is_subbed
    );
}

#[tokio::test]
async fn switching_account_drops_every_profile_and_invalidates_what_was_in_flight() {
    let transport = FakeTransport::new(vec![Ok(profile("u2", "friend"))]);
    let (mut state, receiver) = state(transport.clone());
    state.load(ProfileKey::Id("u2".into())).await;
    state.toggle_follow(ProfileKey::Id("u2".into()));

    let stale = state.generation(&ProfileKey::Id("u2".into()));
    state.viewer_id = Some("u5".into());
    state.reset();

    assert!(receiver.borrow().profiles.is_empty());
    state
        .commit_follow(ProfileKey::Id("u2".into()), stale)
        .await;
    assert_eq!(
        transport.calls().len(),
        1,
        "the old account's write is dead"
    );
}

#[tokio::test]
async fn every_publish_moves_the_revision_so_a_reader_can_tell_it_changed() {
    let transport = FakeTransport::new(vec![Ok(profile("u2", "friend"))]);
    let (mut state, receiver) = state(transport.clone());

    let before = receiver.borrow().revision;
    state.load(ProfileKey::Id("u2".into())).await;

    assert!(receiver.borrow().revision > before);
}
