use gpui::TestAppContext;
use rise_core::foundation::foundation_date::Date;

use crate::modules::session::stores::session_actions::session_types::SessionEntry;
use crate::modules::session::stores::session_services::session_services::SessionServicesStore;

fn entry(id: &str, is_current: bool, last_active_ms: i64) -> SessionEntry {
    SessionEntry {
        id: id.to_owned(),
        device: id.to_owned(),
        app_line: String::new(),
        location: String::new(),
        ip: String::new(),
        is_current,
        last_active: Date::from_ms(last_active_ms),
        first_active: None,
    }
}

fn ids(rows: Option<&[SessionEntry]>) -> Vec<String> {
    rows.unwrap_or_default()
        .iter()
        .map(|row| row.id.clone())
        .collect()
}

#[test]
fn normalize_puts_the_current_session_first_and_the_rest_by_last_activity() {
    let rows = vec![
        entry("old", false, 1_000),
        entry("current", true, 500),
        entry("new", false, 3_000),
        entry("middle", false, 2_000),
    ];

    let normalized = SessionServicesStore::normalize(rows);

    let order: Vec<&str> = normalized.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(order, vec!["current", "new", "middle", "old"]);
}

#[gpui::test]
fn ending_a_session_hides_its_row_before_the_server_answers(cx: &mut TestAppContext) {
    let service = cx.update(SessionServicesStore::shared);
    let snapshot = vec![entry("current", true, 2_000), entry("phone", false, 1_000)];

    service.update(cx, |service, cx| {
        service.begin_mutation(snapshot.clone(), cx);
        service.apply_optimistic_sessions(vec![snapshot[0].clone()], cx);
    });

    service.read_with(cx, |service, _| {
        assert!(service.is_mutating());
        assert_eq!(ids(service.optimistic_sessions()), vec!["current".to_owned()]);
    });
}

#[gpui::test]
fn a_refused_termination_puts_the_rows_back_and_shows_the_error(cx: &mut TestAppContext) {
    let service = cx.update(SessionServicesStore::shared);
    let snapshot = vec![entry("current", true, 2_000), entry("phone", false, 1_000)];

    service.update(cx, |service, cx| {
        service.begin_mutation(snapshot.clone(), cx);
        service.apply_optimistic_sessions(vec![snapshot[0].clone()], cx);
        service.delete_sessions_error_handler(&anyhow::anyhow!("refused"), cx);
    });

    service.read_with(cx, |service, _| {
        assert!(!service.is_mutating());
        assert_eq!(
            ids(service.optimistic_sessions()),
            vec!["current".to_owned(), "phone".to_owned()]
        );
        assert!(service.last_error_text().is_some_and(|text| !text.is_empty()));
    });
}

#[gpui::test]
fn a_failed_refresh_keeps_the_rows_already_on_screen(cx: &mut TestAppContext) {
    let service = cx.update(SessionServicesStore::shared);
    let rows = vec![entry("current", true, 2_000), entry("phone", false, 1_000)];

    service.update(cx, |service, cx| {
        service.apply_optimistic_sessions(rows.clone(), cx);
        service.get_sessions_error_handler(&anyhow::anyhow!("offline"), cx);
    });

    service.read_with(cx, |service, _| {
        assert_eq!(
            ids(service.optimistic_sessions()),
            vec!["current".to_owned(), "phone".to_owned()]
        );
        assert!(service.last_error_text().is_some());
    });
}

#[gpui::test]
fn a_successful_load_drops_the_optimistic_rows_and_the_error(cx: &mut TestAppContext) {
    let service = cx.update(SessionServicesStore::shared);

    service.update(cx, |service, cx| {
        service.apply_optimistic_sessions(vec![entry("current", true, 1_000)], cx);
        service.get_sessions_error_handler(&anyhow::anyhow!("offline"), cx);
        service.get_sessions_success_handler(cx);
    });

    service.read_with(cx, |service, _| {
        assert!(service.optimistic_sessions().is_none());
        assert!(service.last_error_text().is_none());
    });
}

#[gpui::test]
fn a_committed_termination_hands_the_list_back_to_the_cache(cx: &mut TestAppContext) {
    let service = cx.update(SessionServicesStore::shared);
    let snapshot = vec![entry("current", true, 2_000), entry("phone", false, 1_000)];

    service.update(cx, |service, cx| {
        service.begin_mutation(snapshot.clone(), cx);
        service.commit_mutation(cx);
    });

    service.read_with(cx, |service, _| {
        assert!(!service.is_mutating());
        assert!(service.optimistic_sessions().is_none());
    });
}

#[gpui::test]
fn a_reset_forgets_the_previous_accounts_state(cx: &mut TestAppContext) {
    let service = cx.update(SessionServicesStore::shared);

    service.update(cx, |service, cx| {
        service.begin_mutation(vec![entry("current", true, 1_000)], cx);
        service.delete_sessions_error_handler(&anyhow::anyhow!("refused"), cx);
        service.reset_account_state(cx);
    });

    service.read_with(cx, |service, _| {
        assert!(!service.is_mutating());
        assert!(service.optimistic_sessions().is_none());
        assert!(service.last_error_text().is_none());
    });
}
