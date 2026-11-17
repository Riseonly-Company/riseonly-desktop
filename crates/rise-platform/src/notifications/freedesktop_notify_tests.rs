use super::freedesktop_notify::*;
use crate::notifications::{LocalNotification, NotificationAction, NotificationTag};

fn every_capability() -> ServerCapabilities {
    ServerCapabilities::from_list(&["actions", "body", "body-markup", "persistence"])
}

fn message() -> LocalNotification {
    LocalNotification::new(NotificationTag::conversation("chat-1"), "Ayan", "a < b & c")
        .with_action(NotificationAction::reply("Reply"))
        .with_action(NotificationAction::mark_read("Mark read"))
}

fn hint<'a>(request: &'a NotifyRequest, key: &str) -> Option<&'a HintValue> {
    request
        .hints
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, value)| value)
}

#[test]
fn capabilities_read_only_the_names_the_spec_defines() {
    let capabilities = ServerCapabilities::from_list(&["actions", "x-vendor-actions", "body"]);
    assert!(capabilities.actions);
    assert!(!capabilities.body_markup);
    assert!(!capabilities.persistence);
    assert_eq!(
        ServerCapabilities::from_list::<&str>(&[]),
        ServerCapabilities::default()
    );
}

#[test]
fn a_markup_server_gets_the_body_escaped_and_a_plain_one_gets_it_as_is() {
    let escaped = notify_request(&message(), 0, &every_capability(), None);
    assert_eq!(escaped.body, "a &lt; b &amp; c");

    let plain = notify_request(&message(), 0, &ServerCapabilities::default(), None);
    assert_eq!(plain.body, "a < b & c");
    assert_eq!(plain.summary, "Ayan");
}

#[test]
fn buttons_travel_under_transport_keys_after_the_body_action() {
    let request = notify_request(&message(), 0, &every_capability(), None);
    assert_eq!(
        request.actions,
        vec![
            DEFAULT_ACTION_KEY.to_owned(),
            String::new(),
            transport_action_key(0),
            "Reply".to_owned(),
            transport_action_key(1),
            "Mark read".to_owned(),
        ]
    );
}

#[test]
fn a_server_without_actions_gets_no_action_array() {
    let request = notify_request(&message(), 0, &ServerCapabilities::default(), None);
    assert!(request.actions.is_empty());
}

#[test]
fn a_conversation_is_an_instant_message_and_a_standalone_one_is_not() {
    let conversation = notify_request(&message(), 0, &every_capability(), None);
    assert_eq!(
        hint(&conversation, "category"),
        Some(&HintValue::Text(CATEGORY_IM_RECEIVED.to_owned()))
    );

    let standalone = LocalNotification::new(NotificationTag::unique("e1"), "t", "b");
    let request = notify_request(&standalone, 0, &every_capability(), None);
    assert_eq!(hint(&request, "category"), None);
}

#[test]
fn the_avatar_goes_as_an_escaped_file_uri() {
    let notification = message().with_avatar("/home/ayan/.cache/riseonly/a b.png");
    let request = notify_request(&notification, 0, &every_capability(), None);
    assert_eq!(
        hint(&request, "image-path"),
        Some(&HintValue::Text(
            "file:///home/ayan/.cache/riseonly/a%20b.png".to_owned()
        ))
    );
}

#[test]
fn a_relative_avatar_path_is_not_sent() {
    let notification = message().with_avatar("avatars/a.png");
    let request = notify_request(&notification, 0, &every_capability(), None);
    assert_eq!(hint(&request, "image-path"), None);
}

#[test]
fn the_desktop_entry_is_sent_only_when_known() {
    let named = notify_request(&message(), 0, &every_capability(), Some("net.riseonly.desktop"));
    assert_eq!(
        hint(&named, "desktop-entry"),
        Some(&HintValue::Text("net.riseonly.desktop".to_owned()))
    );
    let empty = notify_request(&message(), 0, &every_capability(), Some(""));
    assert_eq!(hint(&empty, "desktop-entry"), None);
}

#[test]
fn the_server_decides_the_timeout_and_the_replace_id_is_passed_through() {
    let request = notify_request(&message(), 42, &every_capability(), None);
    assert_eq!(request.replaces_id, 42);
    assert_eq!(request.expire_timeout, EXPIRE_BY_SERVER);
}

#[test]
fn markup_escaping_covers_the_five_entities() {
    assert_eq!(escape_markup("<&>\"'x"), "&lt;&amp;&gt;&quot;&apos;x");
}

#[test]
fn a_repeated_tag_replaces_the_id_the_server_gave() {
    let mut live = LiveNotifications::default();
    let first = message();
    assert_eq!(live.replaces_id(&first.tag), 0);

    live.record(7, &first);
    assert_eq!(live.replaces_id(&first.tag), 7);

    live.record(9, &first);
    assert_eq!(live.replaces_id(&first.tag), 9);
    assert_eq!(live.len(), 1, "the replaced id is forgotten");
}

#[test]
fn a_dismiss_takes_the_id_once() {
    let mut live = LiveNotifications::default();
    let notification = message();
    live.record(7, &notification);
    assert_eq!(live.take_for_dismiss(&notification.tag), Some(7));
    assert_eq!(live.take_for_dismiss(&notification.tag), None);
    assert!(live.is_empty());
}

#[test]
fn clicks_resolve_to_the_caller_ids_and_the_body() {
    let mut live = LiveNotifications::default();
    let notification = message();
    live.record(7, &notification);

    let body = live.resolve(7, DEFAULT_ACTION_KEY).expect("body click");
    assert_eq!(body.tag, notification.tag);
    assert_eq!(body.action_id, None);

    let reply = live.resolve(7, &transport_action_key(0)).expect("reply click");
    assert_eq!(reply.action_id.as_deref(), Some(NotificationAction::REPLY_ID));

    let read = live.resolve(7, &transport_action_key(1)).expect("read click");
    assert_eq!(read.action_id.as_deref(), Some(NotificationAction::MARK_READ_ID));
}

#[test]
fn signals_for_other_clients_or_unknown_keys_resolve_to_nothing() {
    let mut live = LiveNotifications::default();
    live.record(7, &message());
    assert_eq!(live.resolve(8, DEFAULT_ACTION_KEY), None);
    assert_eq!(live.resolve(7, &transport_action_key(5)), None);
    assert_eq!(live.resolve(7, "REPLY_ACTION"), None);
}

#[test]
fn a_closed_notification_is_forgotten_except_an_expired_one_in_a_history() {
    let notification = message();

    let mut with_history = LiveNotifications::default();
    with_history.record(7, &notification);
    with_history.closed(7, CloseReason::from_code(1), &every_capability());
    assert_eq!(with_history.replaces_id(&notification.tag), 7);
    with_history.closed(7, CloseReason::from_code(2), &every_capability());
    assert_eq!(with_history.replaces_id(&notification.tag), 0);

    let mut without_history = LiveNotifications::default();
    without_history.record(7, &notification);
    without_history.closed(7, CloseReason::Expired, &ServerCapabilities::default());
    assert!(without_history.is_empty());
}

#[test]
fn a_late_close_of_a_replaced_id_keeps_the_newer_one() {
    let mut live = LiveNotifications::default();
    let notification = message();
    live.record(7, &notification);
    live.record(9, &notification);
    live.closed(7, CloseReason::ClosedByCall, &ServerCapabilities::default());
    assert_eq!(live.replaces_id(&notification.tag), 9);
}

#[test]
fn close_reasons_follow_the_spec_codes() {
    assert_eq!(CloseReason::from_code(1), CloseReason::Expired);
    assert_eq!(CloseReason::from_code(2), CloseReason::DismissedByUser);
    assert_eq!(CloseReason::from_code(3), CloseReason::ClosedByCall);
    assert_eq!(CloseReason::from_code(4), CloseReason::Undefined);
    assert_eq!(CloseReason::from_code(0), CloseReason::Undefined);
}

fn inline_capability() -> ServerCapabilities {
    ServerCapabilities::from_list(&["actions", "inline-reply"])
}

fn message_with_field() -> LocalNotification {
    LocalNotification::new(NotificationTag::conversation("chat-1"), "Ayan", "hi")
        .with_action(NotificationAction::mark_read("Mark read"))
        .with_action(NotificationAction::reply("Reply").with_text_input("Send", "Message"))
}

#[test]
fn the_text_input_button_goes_out_as_a_field_when_the_server_offers_one() {
    let request = notify_request(&message_with_field(), 0, &inline_capability(), None);
    assert_eq!(
        request.actions,
        vec![
            DEFAULT_ACTION_KEY.to_owned(),
            String::new(),
            transport_action_key(0),
            "Mark read".to_owned(),
            INLINE_REPLY_ACTION_KEY.to_owned(),
            "Reply".to_owned(),
        ]
    );
    assert!(request.hints.contains(&(REPLY_PLACEHOLDER_HINT, HintValue::Text("Message".to_owned()))));
    assert!(request.hints.contains(&(REPLY_SUBMIT_BUTTON_HINT, HintValue::Text("Send".to_owned()))));
}

#[test]
fn without_the_capability_the_text_input_button_is_a_plain_one() {
    let capabilities = ServerCapabilities::from_list(&["actions"]);
    let request = notify_request(&message_with_field(), 0, &capabilities, None);
    assert!(!request.actions.iter().any(|key| key == INLINE_REPLY_ACTION_KEY));
    assert!(request.actions.contains(&transport_action_key(1)));
    assert!(!request.hints.iter().any(|(key, _)| *key == REPLY_PLACEHOLDER_HINT));
}

#[test]
fn only_the_first_text_input_button_becomes_the_field() {
    let notification = message_with_field()
        .with_action(NotificationAction::new("OTHER", "Other").with_text_input("Go", "Text"));
    let request = notify_request(&notification, 0, &inline_capability(), None);
    let fields = request.actions.iter().filter(|key| *key == INLINE_REPLY_ACTION_KEY).count();
    assert_eq!(fields, 1);
    assert!(request.actions.contains(&transport_action_key(2)));
}

#[test]
fn a_reply_resolves_to_the_text_input_button_with_its_text() {
    let mut live = LiveNotifications::default();
    live.record(7, &message_with_field());

    let reply = live.resolve_reply(7, "on my way").expect("reply");
    assert_eq!(reply.action_id.as_deref(), Some(NotificationAction::REPLY_ID));
    assert_eq!(reply.user_text.as_deref(), Some("on my way"));

    let click = live.resolve(7, INLINE_REPLY_ACTION_KEY).expect("click on the field button");
    assert_eq!(click.action_id.as_deref(), Some(NotificationAction::REPLY_ID));
    assert_eq!(click.user_text, None);
}

#[test]
fn a_reply_to_a_foreign_id_or_a_post_without_a_field_resolves_to_nothing() {
    let mut live = LiveNotifications::default();
    live.record(7, &message());
    assert_eq!(live.resolve_reply(8, "text"), None);
    assert_eq!(live.resolve_reply(7, "text"), None);
    assert_eq!(live.resolve(7, INLINE_REPLY_ACTION_KEY), None);
}
