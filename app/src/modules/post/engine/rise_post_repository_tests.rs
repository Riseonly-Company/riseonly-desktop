use std::sync::Mutex;

use super::*;
use crate::modules::post::engine::rise_post_rpc::FeedKind;
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

impl PostTransport for FakeTransport {
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

fn feed(rows: Vec<Value>, cursor: Option<i64>, more: bool) -> Value {
    json!({
        "list": rows,
        "relativeId": cursor,
        "isHaveMore": more,
        "feedSessionId": "session-1"
    })
}

fn post(id: i64, likes: i64) -> Value {
    json!({"id": id, "likesCount": likes, "author": {"id": "u1", "name": "Aianov"}})
}

fn state(transport: Arc<dyn PostTransport>) -> (PostState, watch::Receiver<Arc<PostSnapshot>>) {
    let (publisher, receiver) = watch::channel(Arc::new(PostSnapshot::default()));
    (
        PostState {
            transport,
            publisher,
            revision: 0,
            pages: BTreeMap::new(),
            cursors: BTreeMap::new(),
            generations: BTreeMap::new(),
            user_id: Some("u1".into()),
            request_seed: 0,
        },
        receiver,
    )
}

#[tokio::test]
async fn a_first_page_lands_and_the_feed_is_marked_loaded() {
    let transport = FakeTransport::new(vec![Ok(feed(vec![post(1, 5), post(2, 0)], Some(2), true))]);
    let (mut state, _receiver) = state(transport.clone());

    state.load_first(PostScope::Feed(FeedKind::All)).await;

    let page = state.pages[&PostScope::Feed(FeedKind::All)].clone();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.status, PageStatus::Loaded);
    assert!(page.did_load);
    assert!(page.has_more);
    assert_eq!(page.items[0].likes_count, 5);
}

#[tokio::test]
async fn the_envelope_the_deployed_gateway_sends_lands_as_rows() {
    let transport = FakeTransport::new(vec![Ok(json!({
        "limit": 20,
        "total": 96,
        "relative_id": 730,
        "list": [post(1494, 0), post(1495, 2)],
        "has_more": true,
        "hasMore": true,
        "is_have_more": true,
        "up": null,
        "page": 1,
        "total_pages": 5,
        "feed_session_id": "feed_dd2d1a41_all_1786265828059866"
    }))]);
    let (mut state, _receiver) = state(transport.clone());

    state.load_first(PostScope::Feed(FeedKind::All)).await;

    let page = state.pages[&PostScope::Feed(FeedKind::All)].clone();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.status, PageStatus::Loaded);
    assert!(page.has_more);
}

#[tokio::test]
async fn a_page_the_dtos_cannot_read_fails_rather_than_reading_as_empty() {
    let transport = FakeTransport::new(vec![Ok(json!({"list": "not a list"}))]);
    let (mut state, _receiver) = state(transport.clone());

    state.load_first(PostScope::Feed(FeedKind::All)).await;

    let page = state.pages[&PostScope::Feed(FeedKind::All)].clone();
    assert_eq!(
        page.status,
        PageStatus::Failed,
        "a silent empty page hides wire drift behind an empty state with no retry"
    );
    assert_eq!(page.error_key, Some("feed_load_error"));
}

#[tokio::test]
async fn opening_a_feed_that_is_already_loaded_asks_for_nothing() {
    let transport = FakeTransport::new(vec![Ok(feed(vec![post(1, 0)], Some(1), false))]);
    let (mut state, _receiver) = state(transport.clone());

    state.activate(PostScope::Feed(FeedKind::All)).await;
    state.activate(PostScope::Feed(FeedKind::All)).await;

    assert_eq!(
        transport.calls().len(),
        1,
        "cache-first: a second visit shows what is already there"
    );
}

#[tokio::test]
async fn the_three_feeds_are_fetched_from_their_own_methods() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 0)], None, false)),
        Ok(feed(vec![post(2, 0)], None, false)),
        Ok(feed(vec![post(3, 0)], None, false)),
    ]);
    let (mut state, _receiver) = state(transport.clone());

    for kind in FeedKind::ALL {
        state.activate(PostScope::Feed(kind)).await;
    }

    let methods: Vec<&str> = transport.calls().iter().map(|call| call.0).collect();
    assert_eq!(
        methods,
        vec!["get_feed", "get_publications_feed", "get_threads_feed"]
    );
}

#[tokio::test]
async fn a_failed_first_load_reports_the_failure_without_inventing_rows() {
    let transport = FakeTransport::new(vec![Err(WireError::NotConnected)]);
    let (mut state, _receiver) = state(transport);

    state.load_first(PostScope::Feed(FeedKind::All)).await;

    let page = state.pages[&PostScope::Feed(FeedKind::All)].clone();
    assert_eq!(page.status, PageStatus::Failed);
    assert_eq!(page.error_key, Some("feed_load_error"));
    assert!(page.items.is_empty());
    assert!(!page.did_load);
}

#[tokio::test]
async fn a_failed_refresh_keeps_the_rows_that_are_already_on_screen() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 0), post(2, 0)], Some(2), true)),
        Err(WireError::NotConnected),
    ]);
    let (mut state, _receiver) = state(transport);

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    state.load_first(PostScope::Feed(FeedKind::All)).await;

    let page = state.pages[&PostScope::Feed(FeedKind::All)].clone();
    assert_eq!(
        page.items.len(),
        2,
        "clearing a valid cache on a failed refresh is what section 16 forbids"
    );
    assert_eq!(page.status, PageStatus::Failed);
    assert_eq!(page.error_key, Some("feed_load_error"));
}

#[tokio::test]
async fn a_second_page_appends_and_carries_the_cursor_and_the_session() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 0), post(2, 0)], Some(2), true)),
        Ok(feed(vec![post(3, 0)], Some(3), false)),
    ]);
    let (mut state, _receiver) = state(transport.clone());

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    state.load_more(PostScope::Feed(FeedKind::All)).await;

    let page = state.pages[&PostScope::Feed(FeedKind::All)].clone();
    assert_eq!(page.items.len(), 3);
    assert!(!page.has_more);

    let second = &transport.calls()[1].1;
    assert_eq!(second["relative_id"], json!("2"));
    assert_eq!(second["new_feed"], json!(false));
    assert_eq!(
        second["feed_session_id"],
        json!("session-1"),
        "dropping the session id makes the ranker start a new one and repeat rows"
    );
}

#[tokio::test]
async fn a_repeated_row_is_not_appended_twice() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 0), post(2, 0)], Some(2), true)),
        Ok(feed(vec![post(2, 0), post(3, 0)], Some(3), false)),
    ]);
    let (mut state, _receiver) = state(transport);

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    state.load_more(PostScope::Feed(FeedKind::All)).await;

    let ids: Vec<&str> = state.pages[&PostScope::Feed(FeedKind::All)]
        .items
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["1", "2", "3"],
        "a duplicate id is a duplicate ElementId, which gpui drops in release without a word"
    );
}

#[tokio::test]
async fn the_end_of_a_feed_is_never_asked_for_again() {
    let transport = FakeTransport::new(vec![Ok(feed(vec![post(1, 0)], Some(1), false))]);
    let (mut state, _receiver) = state(transport.clone());

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    state.load_more(PostScope::Feed(FeedKind::All)).await;
    state.load_more(PostScope::Feed(FeedKind::All)).await;

    assert_eq!(transport.calls().len(), 1);
}

#[tokio::test]
async fn a_failed_page_leaves_the_edge_open_for_a_retry() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 0)], Some(1), true)),
        Err(WireError::NotConnected),
        Ok(feed(vec![post(2, 0)], Some(2), false)),
    ]);
    let (mut state, _receiver) = state(transport.clone());

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    state.load_more(PostScope::Feed(FeedKind::All)).await;

    let page = state.pages[&PostScope::Feed(FeedKind::All)].clone();
    assert_eq!(page.error_key, Some("feed_load_error"));
    assert!(!page.is_loading_more);
    assert!(page.has_more, "the edge must not be closed by one failure");

    state.load_more(PostScope::Feed(FeedKind::All)).await;
    assert_eq!(state.pages[&PostScope::Feed(FeedKind::All)].items.len(), 2);
}

#[tokio::test]
async fn a_like_is_visible_before_the_request_is_answered() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 10)], None, false)),
        Ok(json!({"postId": 1, "isLiked": true, "likesCount": 11, "changed": true})),
    ]);
    let (mut state, mut receiver) = state(transport.clone());

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    receiver.mark_unchanged();

    state.toggle("1".into(), PostToggle::Like, true).await;

    let page = state.pages[&PostScope::Feed(FeedKind::All)].clone();
    assert!(page.items[0].is_liked);
    assert_eq!(page.items[0].likes_count, 11);

    let body = &transport.calls()[1].1;
    assert_eq!(body["post_id"], json!(1));
    assert_eq!(body["liked"], json!(true));
    assert!(
        body["client_request_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "the idempotency key the descriptor promises has to actually be in the body"
    );
}

#[tokio::test]
async fn the_servers_counter_replaces_the_guess() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 10)], None, false)),
        Ok(json!({"postId": 1, "isLiked": true, "likesCount": 37, "changed": true})),
    ]);
    let (mut state, _receiver) = state(transport);

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    state.toggle("1".into(), PostToggle::Like, true).await;

    assert_eq!(
        state.pages[&PostScope::Feed(FeedKind::All)].items[0].likes_count,
        37
    );
}

#[tokio::test]
async fn a_refused_like_rolls_back_to_exactly_what_was_there() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 10)], None, false)),
        Err(WireError::NotConnected),
    ]);
    let (mut state, _receiver) = state(transport);

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    state.toggle("1".into(), PostToggle::Like, true).await;

    let entry = &state.pages[&PostScope::Feed(FeedKind::All)].items[0];
    assert!(!entry.is_liked);
    assert_eq!(entry.likes_count, 10);
}

#[tokio::test]
async fn a_like_refused_by_the_server_rolls_back_too() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 10)], None, false)),
        Ok(json!({"error": "post not found"})),
    ]);
    let (mut state, _receiver) = state(transport);

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    state.toggle("1".into(), PostToggle::Like, true).await;

    let entry = &state.pages[&PostScope::Feed(FeedKind::All)].items[0];
    assert!(!entry.is_liked);
    assert_eq!(entry.likes_count, 10);
}

#[tokio::test]
async fn a_no_op_acknowledgement_is_not_a_rollback() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 10)], None, false)),
        Ok(json!({"postId": 1, "isReposted": true, "repostCount": 4, "changed": false})),
    ]);
    let (mut state, _receiver) = state(transport);

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    state.toggle("1".into(), PostToggle::Repost, true).await;

    let entry = &state.pages[&PostScope::Feed(FeedKind::All)].items[0];
    assert!(entry.is_reposted);
    assert_eq!(entry.repost_count, 4);
}

#[tokio::test]
async fn one_post_in_two_feeds_is_toggled_in_both() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 10)], None, false)),
        Ok(feed(vec![post(1, 10)], None, false)),
        Ok(json!({"postId": 1, "isLiked": true, "likesCount": 11, "changed": true})),
    ]);
    let (mut state, _receiver) = state(transport);

    state.activate(PostScope::Feed(FeedKind::All)).await;
    state
        .activate(PostScope::Feed(FeedKind::Publications))
        .await;
    state.toggle("1".into(), PostToggle::Like, true).await;

    for kind in [FeedKind::All, FeedKind::Publications] {
        let entry = &state.pages[&PostScope::Feed(kind)].items[0];
        assert!(
            entry.is_liked,
            "the same post showing two different states in two tabs is the defect"
        );
        assert_eq!(entry.likes_count, 11);
    }
}

#[tokio::test]
async fn a_reposted_post_is_still_addressed_by_its_own_id() {
    let reposted = json!({
        "id": 1,
        "likesCount": 10,
        "isReposted": true,
        "repostCount": 2
    });
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![reposted], None, false)),
        Ok(json!({"postId": 1, "isLiked": true, "likesCount": 11, "changed": true})),
    ]);
    let (mut state, _receiver) = state(transport.clone());

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    state.toggle("1".into(), PostToggle::Like, true).await;

    assert_eq!(
        transport.calls()[1].1["post_id"],
        json!(1),
        "a repost creates no second row to address"
    );
    let entry = &state.pages[&PostScope::Feed(FeedKind::All)].items[0];
    assert_eq!(entry.likes_count, 11);
    assert!(entry.is_reposted);
}

#[tokio::test]
async fn a_mutation_with_nobody_signed_in_is_never_sent() {
    let transport = FakeTransport::new(vec![Ok(feed(vec![post(1, 10)], None, false))]);
    let (mut state, _receiver) = state(transport.clone());
    state.user_id = None;

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    state.toggle("1".into(), PostToggle::Like, true).await;

    assert_eq!(transport.calls().len(), 1);
    assert!(!state.pages[&PostScope::Feed(FeedKind::All)].items[0].is_liked);
}

#[tokio::test]
async fn switching_accounts_drops_every_feed() {
    let transport = FakeTransport::new(vec![Ok(feed(vec![post(1, 0)], None, false))]);
    let (mut state, _receiver) = state(transport);

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    assert!(!state.pages.is_empty());

    state.reset();

    assert!(
        state.pages.is_empty(),
        "one account's feed must not survive into another's session"
    );
    assert!(state.cursors.is_empty());
}

#[tokio::test]
async fn a_page_that_arrives_after_a_reset_is_dropped_rather_than_installed() {
    let transport = FakeTransport::new(vec![Ok(feed(vec![post(1, 0)], None, false))]);
    let (mut state, _receiver) = state(transport);

    let generation = state.bump_generation(&PostScope::Feed(FeedKind::All));
    state.bump_generation(&PostScope::Feed(FeedKind::All));

    assert_ne!(
        generation,
        state.generation(&PostScope::Feed(FeedKind::All))
    );

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    assert!(state.pages[&PostScope::Feed(FeedKind::All)].did_load);
}

#[tokio::test]
async fn every_publish_moves_the_revision_so_a_reader_can_tell_it_changed() {
    let transport = FakeTransport::new(vec![Ok(feed(vec![post(1, 0)], None, false))]);
    let (mut state, receiver) = state(transport);

    let before = receiver.borrow().revision;
    state.load_first(PostScope::Feed(FeedKind::All)).await;
    let after = receiver.borrow().revision;

    assert!(after > before);
}

#[tokio::test]
async fn a_snapshot_reads_the_page_a_screen_asks_for_by_kind() {
    let transport = FakeTransport::new(vec![Ok(feed(vec![post(1, 0)], None, false))]);
    let (mut state, receiver) = state(transport);

    state.load_first(PostScope::Feed(FeedKind::Threads)).await;

    let snapshot = receiver.borrow().clone();
    assert_eq!(
        snapshot
            .page(&PostScope::Feed(FeedKind::Threads))
            .unwrap()
            .items
            .len(),
        1
    );
    assert!(snapshot.page(&PostScope::Feed(FeedKind::All)).is_none());
}

#[tokio::test]
async fn a_profile_tab_is_addressed_by_its_owners_tag_and_its_own_method() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 0)], None, false)),
        Ok(feed(vec![post(2, 0)], None, false)),
        Ok(feed(vec![post(3, 0)], None, false)),
    ]);
    let (mut state, _receiver) = state(transport.clone());

    for kind in UserFeedKind::ALL {
        state.activate(PostScope::user("Aianov", kind)).await;
    }

    let calls = transport.calls();
    let methods: Vec<&str> = calls.iter().map(|call| call.0).collect();
    assert_eq!(
        methods,
        vec![
            "get_user_publications",
            "get_user_threads",
            "get_user_reposts"
        ]
    );
    assert_eq!(
        calls[0].1["tag"],
        json!("aianov"),
        "the tag keys the scope, so two spellings of one profile must not become two lists"
    );
    assert_eq!(calls[0].1["user_id"], json!("u1"));
}

#[tokio::test]
async fn a_profile_cursor_goes_out_as_a_number_not_a_string() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 0), post(2, 0)], Some(2), true)),
        Ok(feed(vec![post(3, 0)], Some(3), false)),
    ]);
    let (mut state, _receiver) = state(transport.clone());
    let scope = PostScope::user("aianov", UserFeedKind::Publications);

    state.load_first(scope.clone()).await;
    state.load_more(scope.clone()).await;

    assert_eq!(
        transport.calls()[1].1["relative_id"],
        json!(2),
        "execute_get_user_posts reads this with a bare as_i64(), so a string is silently no cursor"
    );
    assert_eq!(state.pages[&scope].items.len(), 3);
}

#[tokio::test]
async fn the_global_feed_still_sends_the_string_cursor_the_reference_sends() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 0)], Some(1), true)),
        Ok(feed(vec![post(2, 0)], Some(2), false)),
    ]);
    let (mut state, _receiver) = state(transport.clone());

    state.load_first(PostScope::Feed(FeedKind::All)).await;
    state.load_more(PostScope::Feed(FeedKind::All)).await;

    assert_eq!(transport.calls()[1].1["relative_id"], json!("1"));
}

#[tokio::test]
async fn reposts_are_asked_for_without_a_feed_session_the_gateway_would_ignore() {
    let transport = FakeTransport::new(vec![Ok(feed(vec![post(1, 0)], None, false))]);
    let (mut state, _receiver) = state(transport.clone());

    state
        .load_first(PostScope::user("aianov", UserFeedKind::Reposts))
        .await;

    let body = &transport.calls()[0].1;
    assert!(body.get("feed_session_id").is_none());
    assert!(body.get("new_feed").is_none());
    assert_eq!(body["up"], json!(false));
}

#[tokio::test]
async fn two_profiles_keep_two_lists_of_the_same_tab() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 0)], None, false)),
        Ok(feed(vec![post(2, 0)], None, false)),
    ]);
    let (mut state, _receiver) = state(transport.clone());

    state
        .activate(PostScope::user("aianov", UserFeedKind::Publications))
        .await;
    state
        .activate(PostScope::user("someone", UserFeedKind::Publications))
        .await;

    assert_eq!(transport.calls().len(), 2);
    assert_eq!(state.pages.len(), 2);
}

#[tokio::test]
async fn liking_a_post_in_a_profile_tab_moves_the_same_post_in_the_feed() {
    let transport = FakeTransport::new(vec![
        Ok(feed(vec![post(1, 10)], None, false)),
        Ok(feed(vec![post(1, 10)], None, false)),
        Ok(json!({"postId": 1, "isLiked": true, "likesCount": 11, "changed": true})),
    ]);
    let (mut state, _receiver) = state(transport);
    let tab = PostScope::user("aianov", UserFeedKind::Publications);

    state.activate(PostScope::Feed(FeedKind::All)).await;
    state.activate(tab.clone()).await;
    state.toggle("1".into(), PostToggle::Like, true).await;

    for scope in [PostScope::Feed(FeedKind::All), tab] {
        let entry = &state.pages[&scope].items[0];
        assert!(entry.is_liked);
        assert_eq!(entry.likes_count, 11);
    }
}

#[tokio::test]
async fn switching_accounts_drops_the_profile_tabs_too() {
    let transport = FakeTransport::new(vec![Ok(feed(vec![post(1, 0)], None, false))]);
    let (mut state, _receiver) = state(transport);
    let scope = PostScope::user("aianov", UserFeedKind::Publications);

    state.load_first(scope.clone()).await;
    let generation = state.generation(&scope);

    state.reset();

    assert!(state.pages.is_empty());
    assert_ne!(
        generation,
        state.generation(&scope),
        "a page still in flight for the old account must not install into the new one"
    );
}
