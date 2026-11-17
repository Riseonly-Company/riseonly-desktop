use serde_json::Value;

use super::*;
use crate::modules::notify::stores::notify_actions::notify_types::NotificationKind;

const ME: &str = "me";
const PEER: &str = "peer";

fn event(raw: &str) -> MessageCreatedEvent {
    serde_json::from_str(raw).expect("message vector")
}

fn reaction(raw: &str) -> ReactionEventPayload {
    serde_json::from_str(raw).expect("reaction vector")
}

fn value<'a>(push: &'a SocketPush, key: &str) -> Option<&'a str> {
    push.user_info.get(key).and_then(Value::as_str)
}

const PRIVATE_MESSAGE: &str = r#"{"message_id":"m1","chat_id":"c1","sender_id":"peer","content":"hello","content_type":"text","created_at":1800000000000,"sender_info":{"name":"Ann","tag":"ann"}}"#;
const TOPIC_MESSAGE: &str = r#"{"message_id":"m2","chat_id":"c2","topic_id":"t1","sender_id":"peer","content":"hi","content_type":"text","created_at":1800000000000,"sender_info":{"name":"Ann"},"chat":{"title":"Team","type_":"GROUP","logo_url":"https://cdn/team.png"}}"#;
const OWN_MESSAGE: &str = r#"{"message_id":"m3","chat_id":"c1","sender_id":"me","content":"x","content_type":"text","created_at":1}"#;
const CALL_MESSAGE: &str = r#"{"message_id":"m4","chat_id":"c1","sender_id":"peer","content":"","content_type":"call","created_at":1}"#;
const PRIVATE_REACTION: &str = r#"{"message_id":"m1","chat_id":"c1","reaction":"❤","actor_id":"peer","timestamp":1800000000000,"reacted_by":[{"reaction":"❤","sender":{"id":"peer","name":"Ann"}}]}"#;
const TOPIC_REACTION: &str = r#"{"message_id":"m2","chat_id":"c2","topic_id":"t1","reaction":"❤","actor_id":"peer","timestamp":1800000000000,"reacted_by":[{"reaction":"❤","sender":{"id":"peer","name":"Ann"}}],"chat":{"title":"Team","chat_type":"group","logo_url":"https://cdn/team.png"}}"#;

#[test]
fn private_message_becomes_the_server_dictionary() {
    let push = message_created_push(&event(PRIVATE_MESSAGE), ME, "en").expect("push");

    assert_eq!(push.title, "Ann");
    assert_eq!(push.body, "hello");
    assert_eq!(value(&push, push_route::TYPE), Some(RisePushType::NEW_MESSAGE));
    assert_eq!(value(&push, push::CHAT_ID), Some("c1"));
    assert_eq!(value(&push, push::SENDER_ID), Some(PEER));
    assert_eq!(value(&push, push::SENDER_NAME), Some("Ann"));
    assert_eq!(value(&push, push::MESSAGE_PREVIEW), Some("hello"));
    assert_eq!(value(&push, push::RECIPIENT_USER_ID), Some(ME));
    assert_eq!(value(&push, push::CHAT_TYPE), Some(ChatType::Direct.internal_wire()));
    assert_eq!(value(&push, push::TOPIC_ID), None);
    assert_eq!(value(&push, push::CHAT_TITLE), None);
    assert_eq!(push.tag, conversation_tag("c1", None));
}

#[test]
fn group_message_in_a_topic_names_the_chat_and_keeps_the_topic() {
    let push = message_created_push(&event(TOPIC_MESSAGE), ME, "en").expect("push");

    assert_eq!(push.title, "Team");
    assert_eq!(push.body, "Ann: hi");
    assert_eq!(value(&push, push::TOPIC_ID), Some("t1"));
    assert_eq!(value(&push, push::CHAT_TITLE), Some("Team"));
    assert_eq!(value(&push, push::AVATAR_URL), Some("https://cdn/team.png"));
    assert_eq!(value(&push, push::CHAT_TYPE), Some(ChatType::Group.internal_wire()));
    assert_eq!(push.tag, conversation_tag("c2", Some("t1")));
}

#[test]
fn own_message_and_call_history_push_nothing() {
    assert!(message_created_push(&event(OWN_MESSAGE), ME, "en").is_none());
    assert!(message_created_push(&event(CALL_MESSAGE), ME, "en").is_none());
}

#[test]
fn reaction_on_my_message_in_a_private_chat() {
    let push = reaction_added_push(&reaction(PRIVATE_REACTION), Some(ME), None, ME, "en").expect("push");

    assert_eq!(push.title, "Ann");
    assert_eq!(value(&push, push_route::TYPE), Some(RisePushType::REACTION_ADDED));
    assert_eq!(value(&push, push::SENDER_ID), Some(PEER));
    assert_eq!(value(&push, push::REACTION), Some("\u{2764}"));
    assert_eq!(value(&push, push::MESSAGE_PREVIEW), Some(push.body.as_str()));
    assert_eq!(value(&push, push::TOPIC_ID), None);
}

const BARE_REACTION: &str = r#"{"message_id":"m1","chat_id":"c1","reaction":"❤","actor_id":"peer","timestamp":1800000000000,"reacted_by":[]}"#;

#[test]
fn reaction_without_reacted_by_names_the_loaded_peer() {
    let mut peer = User { id: PEER.to_owned(), name: "Sid".to_owned(), ..User::default() };
    peer.more.logo = "https://cdn/sid.png".to_owned();
    let push = reaction_added_push(&reaction(BARE_REACTION), Some(ME), Some(&peer), ME, "en").expect("push");

    assert_eq!(push.title, "Sid");
    assert_eq!(value(&push, push::AVATAR_URL), Some("https://cdn/sid.png"));

    let stranger = User { id: "other".to_owned(), name: "Bob".to_owned(), ..User::default() };
    let push = reaction_added_push(&reaction(BARE_REACTION), Some(ME), Some(&stranger), ME, "en").expect("push");
    assert_ne!(push.title, "Bob");
    assert_eq!(value(&push, push::AVATAR_URL), None);
}

#[test]
fn reaction_in_a_group_topic_uses_the_chat_chrome() {
    let push = reaction_added_push(&reaction(TOPIC_REACTION), Some(ME), None, ME, "en").expect("push");

    assert_eq!(push.title, "Team");
    assert_eq!(value(&push, push::AVATAR_URL), Some("https://cdn/team.png"));
    assert_eq!(value(&push, push::TOPIC_ID), Some("t1"));
}

#[test]
fn reaction_goes_only_to_the_author_of_the_message() {
    let payload = reaction(PRIVATE_REACTION);

    assert!(reaction_added_push(&payload, Some(PEER), None, ME, "en").is_none());
    assert!(reaction_added_push(&payload, None, None, ME, "en").is_none());
    assert!(reaction_added_push(&payload, Some(PEER), None, PEER, "en").is_none());
}

fn feed_item(title: &str, body: &str) -> NotificationItem {
    NotificationItem {
        id: "42".to_owned(),
        kind: NotificationKind::PostLike,
        title: title.to_owned(),
        body: body.to_owned(),
        data: BTreeMap::from([(push_route::POST_ID.to_owned(), "p1".to_owned())]),
        is_read: false,
        created_at_ms: 1_800_000_000_000,
        group_count: 1,
        actors: Vec::new(),
    }
}

fn created() -> NotificationCreatedData {
    NotificationCreatedData {
        notification_id: 42,
        notification_type: RisePushType::LIKE.to_owned(),
        unread_count: 3,
    }
}

#[test]
fn created_row_keeps_its_route_and_rendered_text() {
    let push = notification_created_push(&created(), &feed_item("Ann", "liked your post"), ME).expect("push");

    assert_eq!(push.title, "Ann");
    assert_eq!(push.body, "liked your post");
    assert_eq!(value(&push, push_route::TYPE), Some(RisePushType::LIKE));
    assert_eq!(value(&push, push_route::POST_ID), Some("p1"));
    assert_eq!(value(&push, push::NOTIFICATION_ID), Some("42"));
    assert_eq!(value(&push, push::RECIPIENT_USER_ID), Some(ME));
}

#[test]
fn created_row_without_text_shows_nothing() {
    assert!(notification_created_push(&created(), &feed_item(" ", ""), ME).is_none());
}

#[test]
fn replay_after_a_reconnect_does_not_toast_twice() {
    let mut dedup = SocketNotificationDedup::default();
    let push = message_created_push(&event(PRIVATE_MESSAGE), ME, "en").expect("push");

    assert!(dedup.admit(&push.dedup_key));
    assert!(!dedup.admit(&push.dedup_key));
    assert!(dedup.admit("message:other"));
}

#[test]
fn dedup_window_forgets_only_the_oldest() {
    let mut dedup = SocketNotificationDedup::default();
    for index in 0..=SESSION_DEDUP_CAPACITY {
        assert!(dedup.admit(&index.to_string()));
    }
    assert!(!dedup.admit(&SESSION_DEDUP_CAPACITY.to_string()));
    assert!(dedup.admit("0"));
}

#[test]
fn timestamps_in_seconds_and_milliseconds_compare_alike() {
    assert_eq!(normalized_ms(1_800_000_000), 1_800_000_000_000);
    assert_eq!(normalized_ms(1_800_000_000_000), 1_800_000_000_000);
}

#[test]
fn closing_the_last_window_by_os_and_key() {
    assert_eq!(last_window_close(HostOs::MacOs, false), LastWindowClose::StayInDock);
    assert_eq!(last_window_close(HostOs::MacOs, true), LastWindowClose::StayInDock);
    assert_eq!(last_window_close(HostOs::Windows, true), LastWindowClose::HideToTray);
    assert_eq!(last_window_close(HostOs::Windows, false), LastWindowClose::Quit);
    assert_eq!(last_window_close(HostOs::Linux, true), LastWindowClose::CloseToTray);
    assert_eq!(last_window_close(HostOs::Linux, false), LastWindowClose::Quit);
}

#[test]
fn account_switches_gate_each_scope() {
    let mut settings = NotificationSettings {
        all_enabled: true,
        messages_enabled: false,
        mentions_enabled: true,
        ..Default::default()
    };

    assert!(!scope_enabled(&settings, Some(NotificationScope::Messages.as_str())));
    assert!(!scope_enabled(&settings, Some(NotificationScope::Reactions.as_str())));
    assert!(scope_enabled(&settings, Some(NotificationScope::Mentions.as_str())));
    assert!(scope_enabled(&settings, Some(NotificationScope::Events.as_str())));
    assert!(scope_enabled(&settings, None));

    settings.all_enabled = false;
    assert!(!scope_enabled(&settings, Some(NotificationScope::Events.as_str())));
    assert!(!scope_enabled(&settings, None));
}

fn mentioning(user_id: &str) -> MessageCreatedEvent {
    let kind = MessageEntityType::ALL
        .into_iter()
        .find(|kind| kind.is_user_mention() && kind.field() == Some(MessageEntityField::UserId))
        .expect("user mention kind");
    let mut entity = Map::new();
    entity.insert(MessageEntityField::EntityType.wire().to_owned(), Value::from(kind.wire()));
    entity.insert(MessageEntityField::UserId.wire().to_owned(), Value::from(user_id));
    let mut message = event(PRIVATE_MESSAGE);
    message.entities = Some(Value::Array(vec![Value::Object(entity)]));
    message
}

#[test]
fn private_mention_is_the_mentions_scope_without_the_socket_flag() {
    let push = message_created_push(&mentioning(ME), ME, "en").expect("push");
    assert_eq!(value(&push, push::NOTIFICATION_SCOPE), Some(NotificationScope::Mentions.as_str()));

    let other = message_created_push(&mentioning(PEER), ME, "en").expect("push");
    assert_eq!(value(&other, push::NOTIFICATION_SCOPE), Some(NotificationScope::Messages.as_str()));
}

fn typed(content: &str) -> MessageCreatedEvent {
    let mut message = event(PRIVATE_MESSAGE);
    message.content = content.to_owned();
    message
}

#[test]
fn typed_tag_without_entity_is_the_mentions_scope() {
    let scope = |content: &str, tag: Option<&str>| {
        let push = message_created_push_for(&typed(content), ME, tag, "en").expect("push");
        value(&push, push::NOTIFICATION_SCOPE).map(str::to_owned)
    };
    let mentions = Some(NotificationScope::Mentions.as_str().to_owned());
    let messages = Some(NotificationScope::Messages.as_str().to_owned());
    assert_eq!(scope("@Me_Tag hi", Some("me_tag")), mentions);
    assert_eq!(scope("hi @me_tag.", Some("me_tag")), mentions);
    assert_eq!(scope("/start@me_tag", Some("me_tag")), mentions);
    assert_eq!(scope("mail@me_tag", Some("me_tag")), messages);
    assert_eq!(scope("@me_tagx", Some("me_tag")), messages);
    assert_eq!(scope("@me_tag", None), messages);
}

#[test]
fn named_reply_author_is_a_mention_and_unnamed_stays_a_reply() {
    let mut named = mentioning(ME);
    named.reply_to = typed_reply();
    let push = message_created_push(&named, ME, "en").expect("push");
    assert_eq!(value(&push, push::NOTIFICATION_SCOPE), Some(NotificationScope::Mentions.as_str()));

    let mut plain = event(PRIVATE_MESSAGE);
    plain.reply_to = typed_reply();
    let push = message_created_push(&plain, ME, "en").expect("push");
    assert_eq!(value(&push, push::NOTIFICATION_SCOPE), Some(NotificationScope::Replies.as_str()));
}

fn typed_reply() -> Option<rise_canon::rise_event_payload::MessageReplyTo> {
    serde_json::from_str(r#"{"message_id":"m0","sender_id":"me","content":"q","content_type":"text","created_at":1}"#).ok()
}

fn missed_chat(unread: i64, created_at: f64) -> ChatInfo {
    let mut message = GetMessageMessage::new("m9".to_owned());
    message.chat_id = "c1".to_owned();
    message.sender_id = PEER.to_owned();
    message.text = "while offline".to_owned();
    message.created_at = created_at;
    ChatInfo {
        id: "c1".to_owned(),
        title: "Sid".to_owned(),
        unread_count: unread,
        last_message: Some(message),
        participant: User { id: PEER.to_owned(), name: "Sid".to_owned(), ..User::default() },
        ..ChatInfo::default()
    }
}

#[test]
fn message_missed_while_offline_is_toasted_from_the_chat_row() {
    let push = missed_message_push(&missed_chat(1, 1_800_000_000.0), ME, None, 1_799_999_000_000, "en").expect("push");

    assert_eq!(push.title, "Sid");
    assert_eq!(push.body, "while offline");
    assert_eq!(push.dedup_key, "message:m9");
    assert_eq!(push.created_at_ms, 1_800_000_000_000);
    assert_eq!(value(&push, push::CHAT_ID), Some("c1"));
    assert_eq!(value(&push, push::SENDER_NAME), Some("Sid"));
}

#[test]
fn read_older_or_own_rows_toast_nothing() {
    assert!(missed_message_push(&missed_chat(0, 1_800_000_000.0), ME, None, 0, "en").is_none());
    assert!(missed_message_push(&missed_chat(1, 1_700_000_000.0), ME, None, 1_799_999_000_000, "en").is_none());
    assert!(missed_message_push(&missed_chat(1, 1_800_000_000.0), PEER, None, 0, "en").is_none());
}
