use std::collections::VecDeque;
use std::sync::Mutex;

use rise_engine::RemoteError;
use serde_json::json;

use super::*;

#[derive(Default)]
struct ScriptedTransport {
    replies: Mutex<VecDeque<Result<Value, WireError>>>,
    calls: Mutex<Vec<Value>>,
}

impl ScriptedTransport {
    fn answering(replies: Vec<Result<Value, WireError>>) -> Arc<Self> {
        let transport = Arc::new(Self::default());
        *transport.replies.lock().unwrap() = replies.into();
        transport
    }

    fn bodies(&self) -> Vec<Value> {
        self.calls.lock().unwrap().clone()
    }
}

impl SessionTransport for ScriptedTransport {
    fn call(
        &self,
        _descriptor: &'static MethodDescriptor,
        body: Value,
    ) -> BoxFuture<'static, Result<Value, WireError>> {
        self.calls.lock().unwrap().push(body);
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(json!({})));
        Box::pin(async move { reply })
    }
}

fn identity() -> SessionIdentity {
    SessionIdentity {
        user_id: "u1".into(),
        session_id: "current".into(),
    }
}

fn state(
    transport: Arc<ScriptedTransport>,
) -> (SessionState, watch::Receiver<Arc<SessionSnapshot>>) {
    let (publisher, snapshot) = watch::channel(Arc::new(SessionSnapshot::default()));
    (
        SessionState {
            transport,
            publisher,
            revision: 0,
            rows: Vec::new(),
            previous: None,
            is_loading: false,
            is_mutating: false,
            has_more: false,
            cursor: None,
            error_key: None,
            should_sign_out: false,
            did_load: false,
        },
        snapshot,
    )
}

fn session(id: &str, last: &str) -> Value {
    json!({
        "id": id,
        "device_info": id,
        "last_accessed_at": last,
        "is_current": id == "current"
    })
}

fn page(ids: &[(&str, &str)], cursor: Option<&str>, has_more: bool) -> Value {
    json!({
        "success": true,
        "sessions": ids.iter().map(|(id, last)| session(id, last)).collect::<Vec<_>>(),
        "relative_id": cursor,
        "is_have_more": has_more
    })
}

#[tokio::test]
async fn a_first_page_lands_with_the_current_session_first() {
    let transport = ScriptedTransport::answering(vec![Ok(page(
        &[
            ("a", "2024-01-01T00:00:00Z"),
            ("current", "2024-02-01T00:00:00Z"),
            ("b", "2024-03-01T00:00:00Z"),
        ],
        Some("b"),
        true,
    ))]);
    let (mut state, snapshot) = state(transport);

    state.load(identity(), false).await;

    let snapshot = snapshot.borrow().clone();
    assert_eq!(
        snapshot
            .rows
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec!["current", "b", "a"]
    );
    assert!(snapshot.did_load);
    assert!(snapshot.has_more);
    assert_eq!(snapshot.cursor.as_deref(), Some("b"));
    assert!(!snapshot.is_loading);
}

#[tokio::test]
async fn a_second_load_is_ignored_unless_it_is_forced() {
    let transport = ScriptedTransport::answering(vec![
        Ok(page(&[("a", "2024-01-01T00:00:00Z")], None, false)),
        Ok(page(&[("b", "2024-01-01T00:00:00Z")], None, false)),
    ]);
    let (mut state, _snapshot) = state(Arc::clone(&transport));

    state.load(identity(), false).await;
    state.load(identity(), false).await;
    assert_eq!(
        transport.bodies().len(),
        1,
        "opening a screen twice is one fetch"
    );

    state.load(identity(), true).await;
    assert_eq!(
        transport.bodies().len(),
        2,
        "a manual refresh bypasses the gate"
    );
}

#[tokio::test]
async fn the_next_page_merges_without_duplicating_a_row() {
    let transport = ScriptedTransport::answering(vec![
        Ok(page(&[("a", "2024-03-01T00:00:00Z")], Some("a"), true)),
        Ok(page(
            &[("a", "2024-03-01T00:00:00Z"), ("b", "2024-02-01T00:00:00Z")],
            None,
            false,
        )),
    ]);
    let (mut state, snapshot) = state(transport);

    state.load(identity(), false).await;
    state.load_more(identity()).await;

    let rows = snapshot.borrow().rows.clone();
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "b"],
        "a row the server repeats across pages must appear once"
    );
    assert!(!snapshot.borrow().has_more);
}

#[tokio::test]
async fn the_edge_never_asks_past_the_end_or_with_no_cursor() {
    let transport = ScriptedTransport::answering(vec![Ok(page(
        &[("a", "2024-01-01T00:00:00Z")],
        None,
        false,
    ))]);
    let (mut state, _snapshot) = state(Arc::clone(&transport));

    state.load(identity(), false).await;
    state.load_more(identity()).await;
    state.load_more(identity()).await;

    assert_eq!(
        transport.bodies().len(),
        1,
        "a list that reached its end would otherwise ask for the same page forever"
    );
}

#[tokio::test]
async fn a_failed_first_load_reports_without_clearing_what_is_on_screen() {
    let transport = ScriptedTransport::answering(vec![
        Ok(page(&[("a", "2024-01-01T00:00:00Z")], None, false)),
        Err(WireError::NotConnected),
    ]);
    let (mut state, snapshot) = state(transport);

    state.load(identity(), false).await;
    state.load(identity(), true).await;

    assert_eq!(
        snapshot.borrow().rows.len(),
        1,
        "a valid page must survive a failed refresh"
    );
    assert!(snapshot.borrow().error_key.is_some());
}

#[tokio::test]
async fn ending_a_session_removes_its_row_before_the_server_answers() {
    let transport = ScriptedTransport::answering(vec![
        Ok(page(
            &[
                ("current", "2024-03-01T00:00:00Z"),
                ("a", "2024-02-01T00:00:00Z"),
            ],
            None,
            false,
        )),
        Ok(json!({"success": true})),
    ]);
    let (mut state, snapshot) = state(transport);

    state.load(identity(), false).await;
    state.terminate(identity(), Some("a".into()), false).await;

    let rows = snapshot.borrow().rows.clone();
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["current"]
    );
    assert!(!snapshot.borrow().is_mutating);
}

#[tokio::test]
async fn a_refused_termination_puts_the_row_back() {
    let transport = ScriptedTransport::answering(vec![
        Ok(page(
            &[
                ("current", "2024-03-01T00:00:00Z"),
                ("a", "2024-02-01T00:00:00Z"),
            ],
            None,
            false,
        )),
        Err(WireError::Remote(RemoteError {
            code: Some("INTERNAL".into()),
            message: "no".into(),
        })),
    ]);
    let (mut state, snapshot) = state(transport);

    state.load(identity(), false).await;
    state.terminate(identity(), Some("a".into()), false).await;

    let rows = snapshot.borrow().rows.clone();
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["current", "a"],
        "an optimistic removal the server refused has to come back"
    );
    assert!(snapshot.borrow().error_key.is_some());
}

#[tokio::test]
async fn a_two_hundred_that_says_error_rolls_back_just_the_same() {
    let transport = ScriptedTransport::answering(vec![
        Ok(page(
            &[
                ("current", "2024-03-01T00:00:00Z"),
                ("a", "2024-02-01T00:00:00Z"),
            ],
            None,
            false,
        )),
        Ok(json!({"success": false, "error": "Cannot end that session"})),
    ]);
    let (mut state, snapshot) = state(transport);

    state.load(identity(), false).await;
    state.terminate(identity(), Some("a".into()), false).await;

    assert_eq!(snapshot.borrow().rows.len(), 2);
}

#[tokio::test]
async fn ending_every_other_session_keeps_only_this_one() {
    let transport = ScriptedTransport::answering(vec![
        Ok(page(
            &[
                ("current", "2024-03-01T00:00:00Z"),
                ("a", "2024-02-01T00:00:00Z"),
                ("b", "2024-01-01T00:00:00Z"),
            ],
            None,
            false,
        )),
        Ok(json!({"success": true})),
    ]);
    let (mut state, snapshot) = state(Arc::clone(&transport));

    state.load(identity(), false).await;
    state.terminate(identity(), None, true).await;

    assert_eq!(
        snapshot
            .borrow()
            .rows
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec!["current"]
    );
    assert_eq!(transport.bodies()[1]["is_all"], true);
}

#[tokio::test]
async fn the_server_replacing_the_list_wins_over_the_optimistic_guess() {
    let transport = ScriptedTransport::answering(vec![
        Ok(page(
            &[
                ("current", "2024-03-01T00:00:00Z"),
                ("a", "2024-02-01T00:00:00Z"),
            ],
            None,
            false,
        )),
        Ok(json!({
            "success": true,
            "remaining_sessions": [{"id": "current", "device_info": "current", "is_current": true}]
        })),
    ]);
    let (mut state, snapshot) = state(transport);

    state.load(identity(), false).await;
    state.terminate(identity(), Some("a".into()), false).await;

    assert_eq!(
        snapshot
            .borrow()
            .rows
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec!["current"]
    );
}

#[tokio::test]
async fn ending_this_clients_own_session_is_reported_for_the_shell_to_act_on() {
    let transport = ScriptedTransport::answering(vec![
        Ok(page(&[("current", "2024-03-01T00:00:00Z")], None, false)),
        Ok(json!({"success": true, "should_logout": true})),
    ]);
    let (mut state, snapshot) = state(transport);

    state.load(identity(), false).await;
    state
        .terminate(identity(), Some("current".into()), false)
        .await;

    assert!(
        snapshot.borrow().should_sign_out,
        "every request after this one would be refused; the shell has to sign out"
    );
}

#[tokio::test]
async fn a_second_termination_while_one_is_in_flight_is_ignored() {
    let transport = ScriptedTransport::answering(vec![Ok(json!({"success": true}))]);
    let (mut state, _snapshot) = state(Arc::clone(&transport));

    state.is_mutating = true;
    state.terminate(identity(), Some("a".into()), false).await;

    assert!(
        transport.bodies().is_empty(),
        "two overlapping deletes would each roll back to a different list"
    );
}

#[tokio::test]
async fn every_request_carries_the_identity_it_was_made_for() {
    let transport = ScriptedTransport::answering(vec![Ok(page(&[], None, false))]);
    let (mut state, _snapshot) = state(Arc::clone(&transport));

    state.load(identity(), false).await;

    let body = &transport.bodies()[0];
    assert_eq!(body["user_id"], "u1");
    assert_eq!(body["current_session_id"], "current");
}

#[tokio::test]
async fn a_reset_forgets_the_previous_accounts_sessions() {
    let transport = ScriptedTransport::answering(vec![Ok(page(
        &[("a", "2024-01-01T00:00:00Z")],
        Some("a"),
        true,
    ))]);
    let (mut state, snapshot) = state(transport);

    state.load(identity(), false).await;
    state.rows.clear();
    state.cursor = None;
    state.has_more = false;
    state.did_load = false;
    state.publish();

    let snapshot = snapshot.borrow().clone();
    assert!(snapshot.rows.is_empty());
    assert!(!snapshot.did_load);
    assert_eq!(snapshot.cursor, None);
}
