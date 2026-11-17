use gpui::SharedString;
use rise_platform::notifications::{InMemoryNotificationPresenter, NotificationTextInput};

use super::*;

fn key(raw: &str) -> String {
    raw.to_owned()
}

fn state() -> NotificationDeliveryState {
    let mut state = NotificationDeliveryState::default();
    state.register_categories(notification_categories(&key));
    state
}

fn post(
    state: &mut NotificationDeliveryState,
    presenter: &InMemoryNotificationPresenter,
    chat_id: &str,
    topic_id: Option<&str>,
) -> NotificationDeliveryOutcome {
    state.deliver(
        conversation_notification(chat_id, topic_id, "title", "body"),
        None,
        Map::new(),
        ScenePhase::Background,
        presenter,
    )
}

fn response(tag: &NotificationTag, action_id: Option<&'static str>) -> NotificationActionResponse {
    NotificationActionResponse {
        tag: SharedString::from(tag.as_str().to_owned()),
        action_id: action_id.map(SharedString::from),
        user_text: None,
    }
}

#[test]
fn categories_match_the_canon_identifiers_ios_registers() {
    let categories = notification_categories(&key);
    let identifiers: Vec<&str> = categories.iter().map(|c| c.identifier.as_str()).collect();
    assert_eq!(identifiers, ["MESSAGE_CATEGORY", "MUSIC_CATEGORY"]);

    let message: Vec<&str> = categories[0]
        .actions
        .iter()
        .map(|a| a.id.as_str())
        .collect();
    assert_eq!(
        message,
        [NotificationAction::REPLY_ID, NotificationAction::MARK_READ_ID]
    );
    assert!(categories[1].actions.is_empty());
}

#[test]
fn only_the_reply_action_carries_a_text_input_with_canon_labels() {
    let categories = notification_categories(&key);
    let reply = &categories[0].actions[0];
    assert_eq!(reply.label, "notification_reply");
    assert_eq!(
        reply.text_input,
        Some(NotificationTextInput {
            button_title: "notification_send".to_owned(),
            placeholder: "notification_type_message".to_owned(),
        })
    );
    assert_eq!(categories[0].actions[1].text_input, None);
}

#[test]
fn the_tag_follows_chat_and_topic() {
    assert_eq!(conversation_tag("c1", None), conversation_tag("c1", Some("")));
    assert_ne!(conversation_tag("c1", None), conversation_tag("c1", Some("t1")));
    assert_ne!(conversation_tag("c1", Some("t1")), conversation_tag("c2", Some("t1")));
}

#[test]
fn a_chat_notification_groups_by_chat_id_even_inside_a_topic() {
    let notification = conversation_notification("c1", Some("t1"), "title", "body");
    assert_eq!(notification.thread.as_deref(), Some("c1"));
}

#[test]
fn the_category_buttons_reach_the_presenter() {
    let mut state = state();
    let presenter = InMemoryNotificationPresenter::new();
    let outcome = state.deliver(
        conversation_notification("c1", None, "title", "body"),
        Some("MESSAGE_CATEGORY"),
        Map::new(),
        ScenePhase::Background,
        &presenter,
    );
    assert_eq!(outcome, NotificationDeliveryOutcome::Delivered);
    assert_eq!(presenter.presented()[0].actions.len(), 2);
}

#[test]
fn a_repeat_in_one_chat_replaces_the_known_notification() {
    let mut state = state();
    let presenter = InMemoryNotificationPresenter::new();
    post(&mut state, &presenter, "c1", None);
    post(&mut state, &presenter, "c1", None);

    let posted = presenter.presented();
    assert_eq!(posted[0].tag, posted[1].tag);

    state.remove_delivered("c1", None, &presenter);
    assert_eq!(presenter.dismissed(), vec![conversation_tag("c1", None)]);
}

#[test]
fn a_focused_window_gets_no_toast() {
    let mut state = state();
    let presenter = InMemoryNotificationPresenter::new();
    let outcome = state.deliver(
        conversation_notification("c1", None, "title", "body"),
        None,
        Map::new(),
        ScenePhase::Active,
        &presenter,
    );
    assert_eq!(outcome, NotificationDeliveryOutcome::AppActive);
    assert!(presenter.presented().is_empty());
}

#[test]
fn a_blocked_host_reports_its_precondition() {
    let mut state = state();
    let presenter = InMemoryNotificationPresenter::blocked(DeliveryPrecondition::AppIdentity);
    assert_eq!(
        post(&mut state, &presenter, "c1", None),
        NotificationDeliveryOutcome::Blocked(DeliveryPrecondition::AppIdentity)
    );
}

#[test]
fn remove_delivered_takes_only_its_own_chat() {
    let mut state = state();
    let presenter = InMemoryNotificationPresenter::new();
    post(&mut state, &presenter, "c1", None);
    post(&mut state, &presenter, "c1", Some("t1"));
    post(&mut state, &presenter, "c2", None);

    state.remove_delivered("c1", None, &presenter);

    let dismissed = presenter.dismissed();
    assert!(dismissed.contains(&conversation_tag("c1", None)));
    assert!(dismissed.contains(&conversation_tag("c1", Some("t1"))));
    assert!(!dismissed.contains(&conversation_tag("c2", None)));
}

#[test]
fn remove_delivered_with_a_topic_keeps_the_rest_of_the_chat() {
    let mut state = state();
    let presenter = InMemoryNotificationPresenter::new();
    post(&mut state, &presenter, "c1", None);
    post(&mut state, &presenter, "c1", Some("t1"));

    state.remove_delivered("c1", Some("t1"), &presenter);
    assert_eq!(presenter.dismissed(), vec![conversation_tag("c1", Some("t1"))]);
}

#[test]
fn an_empty_chat_id_removes_nothing() {
    let mut state = state();
    let presenter = InMemoryNotificationPresenter::new();
    state.remove_delivered("", None, &presenter);
    assert!(presenter.dismissed().is_empty());
}

#[test]
fn a_banner_of_an_earlier_process_is_still_retracted_by_tag() {
    let mut state = state();
    let presenter = InMemoryNotificationPresenter::new();
    state.remove_delivered("c9", None, &presenter);
    assert_eq!(presenter.dismissed(), vec![conversation_tag("c9", None)]);
}

/// host whose seam has no bulk retraction, dismiss_all stays the trait default
struct NoBulkRetractionPresenter(InMemoryNotificationPresenter);

impl NotificationPresenter for NoBulkRetractionPresenter {
    fn present(&self, notification: &LocalNotification) -> PlatformSupport {
        self.0.present(notification)
    }

    fn dismiss(&self, tag: &NotificationTag) -> PlatformSupport {
        self.0.dismiss(tag)
    }

    fn availability(&self) -> NotificationAvailability {
        self.0.availability()
    }
}

#[test]
fn remove_all_delivered_retracts_everything_at_once_and_forgets_the_tags() {
    let mut state = state();
    let presenter = InMemoryNotificationPresenter::new();
    post(&mut state, &presenter, "c1", None);
    post(&mut state, &presenter, "c2", None);

    state.remove_all_delivered(&presenter);

    assert_eq!(presenter.dismissed_all(), 1);
    assert!(presenter.dismissed().is_empty());

    let fallback = NoBulkRetractionPresenter(InMemoryNotificationPresenter::new());
    state.remove_all_delivered(&fallback);
    assert!(fallback.0.dismissed().is_empty());
}

#[test]
fn remove_all_delivered_on_a_fresh_process_still_clears_the_notification_center() {
    let mut state = state();
    let presenter = InMemoryNotificationPresenter::new();

    state.remove_all_delivered(&presenter);

    assert_eq!(presenter.dismissed_all(), 1);
}

#[test]
fn remove_all_delivered_without_a_bulk_call_retracts_the_known_tags() {
    let mut state = state();
    let presenter = NoBulkRetractionPresenter(InMemoryNotificationPresenter::new());
    state.deliver(
        conversation_notification("c1", None, "title", "body"),
        None,
        Map::new(),
        ScenePhase::Background,
        &presenter,
    );

    state.remove_all_delivered(&presenter);

    assert_eq!(presenter.0.dismissed(), vec![conversation_tag("c1", None)]);
    assert_eq!(presenter.0.dismissed_all(), 0);
}

#[test]
fn the_three_responses_map_to_their_actions() {
    let mut state = state();
    let presenter = InMemoryNotificationPresenter::new();
    let tag = conversation_tag("c1", Some("t1"));

    post(&mut state, &presenter, "c1", Some("t1"));
    let click = state.take_response(&response(&tag, None));
    assert_eq!(
        click.as_ref().map(|r| r.action.clone()),
        Some(NotificationResponseAction::Default)
    );

    post(&mut state, &presenter, "c1", Some("t1"));
    let reply = state.take_response(&response(&tag, Some(NotificationAction::REPLY_ID)));
    assert_eq!(
        reply.as_ref().map(|r| r.action.clone()),
        Some(NotificationResponseAction::Reply { text: None })
    );

    post(&mut state, &presenter, "c1", Some("t1"));
    let typed = NotificationActionResponse {
        user_text: Some("hello".to_owned()),
        ..response(&tag, Some(NotificationAction::REPLY_ID))
    };
    let reply = state.take_response(&typed);
    assert_eq!(
        reply.as_ref().map(|r| r.action.clone()),
        Some(NotificationResponseAction::Reply { text: Some("hello".to_owned()) })
    );
    assert!(!reply.is_some_and(|r| r.action.raises_window()));

    post(&mut state, &presenter, "c1", Some("t1"));
    let read = state.take_response(&response(&tag, Some(NotificationAction::MARK_READ_ID)));
    let read = read.expect("mark read is a known action");
    assert_eq!(read.action, NotificationResponseAction::MarkRead);
    assert_eq!(read.chat_id.as_deref(), Some("c1"));
    assert_eq!(read.topic_id.as_deref(), Some("t1"));
}

#[test]
fn an_unknown_action_is_dropped_as_on_ios() {
    let mut state = state();
    let tag = conversation_tag("c1", None);
    assert_eq!(state.take_response(&response(&tag, Some("OTHER"))), None);
}

#[test]
fn a_response_returns_the_user_info_of_its_post() {
    let mut state = state();
    let presenter = InMemoryNotificationPresenter::new();
    let mut user_info = Map::new();
    user_info.insert("k".to_owned(), Value::from("v"));
    state.deliver(
        conversation_notification("c1", None, "title", "body"),
        None,
        user_info.clone(),
        ScenePhase::Background,
        &presenter,
    );

    let parsed = state.take_response(&response(&conversation_tag("c1", None), None));
    assert_eq!(parsed.map(|r| r.user_info), Some(user_info));
}

#[test]
fn a_click_on_a_banner_of_an_earlier_process_still_knows_its_chat() {
    let mut state = state();
    let parsed = state
        .take_response(&response(&conversation_tag("c1", Some("t1")), None))
        .expect("a body click is a known action");
    assert_eq!(parsed.chat_id.as_deref(), Some("c1"));
    assert_eq!(parsed.topic_id.as_deref(), Some("t1"));
    assert!(parsed.user_info.is_empty());
}

#[test]
fn only_a_response_that_shows_a_chat_raises_the_window() {
    assert!(NotificationResponseAction::Default.raises_window());
    assert!(NotificationResponseAction::Reply { text: None }.raises_window());
    assert!(
        !NotificationResponseAction::Reply {
            text: Some("hi".to_owned())
        }
        .raises_window()
    );
    assert!(!NotificationResponseAction::MarkRead.raises_window());
}

#[test]
fn the_permission_is_asked_once_and_only_while_active() {
    let mut state = state();
    assert!(!state.claim_permission_request(ScenePhase::Background));
    assert!(state.claim_permission_request(ScenePhase::Active));
    assert!(!state.claim_permission_request(ScenePhase::Active));
}
