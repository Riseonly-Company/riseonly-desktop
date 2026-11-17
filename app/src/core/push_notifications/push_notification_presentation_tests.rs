use std::collections::HashMap;
use std::time::Duration;

use gpui::{AppContext as _, TestAppContext};
use serde_json::{Map, Value, json};

use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiActiveScreen;
use crate::core::push_notifications::push_deep_link_store::{PushDeepLink, PushDeepLinkStore};
use crate::core::push_notifications::push_notification_manager::PushChatNotificationPolicy;
use crate::core::push_notifications::push_notification_presentation::{
    PushForegroundNotificationPresentation, PushNotificationRouteParser,
};
use crate::core::widgets::notifier::notifier_host::NotifierSwipeDismissPolicy;
use crate::core::widgets::notifier::notifier_store::{
    NotifierAction, NotifierSendOptions, NotifierStore,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatMuteScopes;

fn payload(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("test payload is an object")
}

fn screen(id: &str, params: &[(&str, &str)]) -> SaiActiveScreen {
    SaiActiveScreen::new(
        id.to_owned(),
        params
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect::<HashMap<_, _>>(),
    )
}

fn chat(chat_id: &str) -> PushDeepLink {
    PushDeepLink::Chat { chat_id: chat_id.to_owned() }
}

fn chat_topic(chat_id: &str, topic_id: &str) -> PushDeepLink {
    PushDeepLink::ChatTopic { chat_id: chat_id.to_owned(), topic_id: topic_id.to_owned() }
}

fn send_options(duration: f64) -> NotifierSendOptions {
    NotifierSendOptions { duration, ..NotifierSendOptions::default() }
}

#[test]
fn new_message_route_supports_snake_case_payload() {
    let route = PushNotificationRouteParser::route(&payload(json!({
        "type": "new_message",
        "chat_id": "chat-42",
    })));

    assert_eq!(route, Some(chat("chat-42")));
}

#[test]
fn route_parser_normalizes_camel_case_numeric_identifiers() {
    let route = PushNotificationRouteParser::route(&payload(json!({
        "type": "post_like",
        "postId": 84,
    })));

    assert_eq!(route, Some(PushDeepLink::Post { post_id: "84".to_owned() }));
}

#[test]
fn comment_route_keeps_comment_focus_when_present() {
    let route = PushNotificationRouteParser::route(&payload(json!({
        "type": "comment_reply",
        "post_id": "post-1",
        "comment_id": "comment-2",
    })));

    assert_eq!(
        route,
        Some(PushDeepLink::PostComment {
            post_id: "post-1".to_owned(),
            comment_id: Some("comment-2".to_owned()),
        })
    );
}

#[test]
fn reaction_route_opens_its_chat() {
    let route = PushNotificationRouteParser::route(&payload(json!({
        "type": "reaction_added",
        "chat_id": "chat-17",
    })));

    assert_eq!(route, Some(chat("chat-17")));
}

#[test]
fn message_route_opens_its_exact_topic() {
    let route = PushNotificationRouteParser::route(&payload(json!({
        "type": "new_message",
        "chat_id": "chat-17",
        "topic_id": "topic-3",
    })));

    assert_eq!(route, Some(chat_topic("chat-17", "topic-3")));
}

#[test]
fn story_aliases_open_story_viewer() {
    let route = PushNotificationRouteParser::route(&payload(json!({
        "type": "story_reaction",
        "storyId": 123,
    })));

    assert_eq!(route, Some(PushDeepLink::Story { story_id: "123".to_owned() }));
}

#[test]
fn repost_alias_opens_post() {
    let route = PushNotificationRouteParser::route(&payload(json!({
        "type": "post_repost",
        "postId": "post-99",
    })));

    assert_eq!(route, Some(PushDeepLink::Post { post_id: "post-99".to_owned() }));
}

#[test]
fn message_presentation_uses_sender_avatar_preview_and_chat_route() {
    let presentation = PushForegroundNotificationPresentation::make_with_active_screen(
        &payload(json!({
            "type": "new_message",
            "chat_id": "chat-42",
            "sender_id": "user-7",
            "sender_name": "\u{0418}\u{0441}\u{043C}\u{0430}\u{0438}\u{043B}",
            "avatar_url": "https://cdn.example/avatar.jpg",
            "message_preview": "\u{041F}\u{0440}\u{0438}\u{0432}\u{0435}\u{0442}!",
        })),
        "Riseonly",
        "Fallback body",
        None,
    );

    assert_eq!(
        presentation,
        Some(PushForegroundNotificationPresentation {
            title: "\u{0418}\u{0441}\u{043C}\u{0430}\u{0438}\u{043B}".to_owned(),
            body: Some("\u{041F}\u{0440}\u{0438}\u{0432}\u{0435}\u{0442}!".to_owned()),
            avatar_url: Some("https://cdn.example/avatar.jpg".to_owned()),
            avatar_name: Some("\u{0418}\u{0441}\u{043C}\u{0430}\u{0438}\u{043B}".to_owned()),
            avatar_user_id: Some("user-7".to_owned()),
            route: Some(chat("chat-42")),
            author: None,
        })
    );
}

#[test]
fn legacy_group_message_removes_duplicated_sender_prefix() {
    let presentation = PushForegroundNotificationPresentation::make_with_active_screen(
        &payload(json!({
            "type": "new_message",
            "chatId": "group-9",
            "senderName": "\u{0410}\u{043B}\u{0438}\u{043D}\u{0430}",
        })),
        "\u{0420}\u{0430}\u{0431}\u{043E}\u{0447}\u{0438}\u{0439} \u{0447}\u{0430}\u{0442}",
        "\u{0410}\u{043B}\u{0438}\u{043D}\u{0430}: \u{041E}\u{0431}\u{043D}\u{043E}\u{0432}\u{0438}\u{043B}\u{0430} \u{043C}\u{0430}\u{043A}\u{0435}\u{0442}",
        None,
    )
    .expect("the message has a presentation");

    assert_eq!(presentation.title, "\u{0410}\u{043B}\u{0438}\u{043D}\u{0430}");
    assert_eq!(presentation.body.as_deref(), Some("\u{041E}\u{0431}\u{043D}\u{043E}\u{0432}\u{0438}\u{043B}\u{0430} \u{043C}\u{0430}\u{043A}\u{0435}\u{0442}"));
    assert_eq!(presentation.route, Some(chat("group-9")));
}

#[test]
fn presentation_supports_legacy_actor_avatar_alias() {
    let presentation = PushForegroundNotificationPresentation::make_with_active_screen(
        &payload(json!({
            "type": "follow",
            "actor_id": "user-5",
            "actor_name": "\u{041B}\u{0435}\u{043D}\u{0430}",
            "actorAvatar": "https://cdn.example/actor.jpg",
        })),
        "\u{041D}\u{043E}\u{0432}\u{044B}\u{0439} \u{043F}\u{043E}\u{0434}\u{043F}\u{0438}\u{0441}\u{0447}\u{0438}\u{043A}",
        "\u{041B}\u{0435}\u{043D}\u{0430} \u{043F}\u{043E}\u{0434}\u{043F}\u{0438}\u{0441}\u{0430}\u{043B}\u{0430}\u{0441}\u{044C} \u{043D}\u{0430} \u{0432}\u{0430}\u{0441}",
        None,
    )
    .expect("the follow has a presentation");

    assert_eq!(presentation.avatar_url.as_deref(), Some("https://cdn.example/actor.jpg"));
    assert_eq!(
        presentation.route,
        Some(PushDeepLink::Profile { user_id: Some("user-5".to_owned()), tag: None })
    );
}

#[gpui::test]
fn activating_notifier_dismisses_it_and_enqueues_deep_link(cx: &mut TestAppContext) {
    let notifier_store = cx.update(NotifierStore::shared);
    let deep_link_store = cx.update(PushDeepLinkStore::shared);

    let id = notifier_store.update(cx, |store, cx| {
        store.send(
            "\u{041D}\u{043E}\u{0432}\u{043E}\u{0435} \u{0441}\u{043E}\u{043E}\u{0431}\u{0449}\u{0435}\u{043D}\u{0438}\u{0435}".to_owned(),
            NotifierSendOptions {
                action: Some(NotifierAction::DeepLink(chat("chat-88"))),
                ..send_options(0.0)
            },
            cx,
        )
    });

    notifier_store.read_with(cx, |store, _| {
        assert!(store.items().iter().any(|item| item.id == id));
    });

    notifier_store.update(cx, |store, cx| store.activate(id, cx));
    cx.run_until_parked();

    notifier_store.read_with(cx, |store, _| {
        assert!(!store.items().iter().any(|item| item.id == id));
    });
    deep_link_store.read_with(cx, |store, _| {
        assert_eq!(store.pending_route(), Some(&chat("chat-88")));
    });
}

#[test]
fn empty_alert_and_payload_do_not_create_an_island() {
    let presentation = PushForegroundNotificationPresentation::make_with_active_screen(
        &payload(json!({ "type": "unknown" })),
        "  ",
        "\n",
        None,
    );

    assert_eq!(presentation, None);
}

#[test]
fn chat_deep_links_match_only_their_exact_visible_destination() {
    let general = screen("Chat", &[("chatId", "chat-1")]);
    let topic = screen("Chat", &[("chatId", "chat-1"), ("topicId", "topic-1")]);
    let different_topic = screen("Chat", &[("chatId", "chat-1"), ("topicId", "topic-2")]);
    let thread = screen("Chat", &[("chatId", "chat-1"), ("threadRootId", "message-1")]);

    assert!(chat("chat-1").matches(Some(&general)));
    assert!(!chat("chat-1").matches(Some(&topic)));
    assert!(!chat("chat-1").matches(Some(&thread)));
    assert!(chat_topic("chat-1", "topic-1").matches(Some(&topic)));
    assert!(!chat_topic("chat-1", "topic-1").matches(Some(&different_topic)));
}

#[test]
fn forum_all_matches_every_topic_in_the_same_chat() {
    let active_screen = RootRoute::ForumAll { chat_id: Some("chat-1".to_owned()) }.sai_active_screen();

    assert_eq!(active_screen.params.get("topicMode").map(String::as_str), Some("all"));
    assert!(chat("chat-1").matches(Some(&active_screen)));
    assert!(chat_topic("chat-1", "topic-9").matches(Some(&active_screen)));
    assert!(!chat_topic("chat-2", "topic-9").matches(Some(&active_screen)));
}

#[test]
fn foreground_presentation_is_suppressed_only_for_the_visible_topic() {
    let user_info = payload(json!({
        "type": "new_message",
        "chat_id": "chat-1",
        "topic_id": "topic-1",
        "sender_name": "\u{0410}\u{043B}\u{0438}\u{043D}\u{0430}",
        "message_preview": "\u{041F}\u{0440}\u{043E}\u{0432}\u{0435}\u{0440}\u{043A}\u{0430}",
    }));

    let exact_presentation = PushForegroundNotificationPresentation::make_with_active_screen(
        &user_info,
        "Riseonly",
        "Fallback",
        Some(&screen("Chat", &[("chatId", "chat-1"), ("topicId", "topic-1")])),
    );
    let different_topic_presentation = PushForegroundNotificationPresentation::make_with_active_screen(
        &user_info,
        "Riseonly",
        "Fallback",
        Some(&screen("Chat", &[("chatId", "chat-1"), ("topicId", "topic-2")])),
    );

    assert_eq!(exact_presentation, None);
    assert_eq!(
        different_topic_presentation.and_then(|presentation| presentation.route),
        Some(chat_topic("chat-1", "topic-1"))
    );
}

#[test]
fn conversation_resource_screens_expose_their_cache_scope_to_refresh_matching() {
    let chat_id = Some("chat-1".to_owned());

    assert_eq!(
        RootRoute::ForumHome { chat_id: chat_id.clone() }.sai_active_screen(),
        screen("ForumHome", &[("chatId", "chat-1")])
    );
    assert_eq!(
        RootRoute::ChatTopics { chat_id: chat_id.clone() }.sai_active_screen(),
        screen("ChatTopics", &[("chatId", "chat-1")])
    );
    assert_eq!(
        RootRoute::PinnedMessages { chat_id, topic_id: None }.sai_active_screen(),
        screen("PinnedMessages", &[("chatId", "chat-1")])
    );
}

#[gpui::test]
fn deep_link_store_does_not_enqueue_the_visible_topic_twice(cx: &mut TestAppContext) {
    let store = cx.update(PushDeepLinkStore::shared);

    let route = chat_topic("chat-1", "topic-1");
    let exact_screen = screen("Chat", &[("chatId", "chat-1"), ("topicId", "topic-1")]);
    let different_screen = screen("Chat", &[("chatId", "chat-1"), ("topicId", "topic-2")]);

    let enqueued = store.update(cx, |store, cx| {
        store.enqueue_active_screen(route.clone(), Some(&exact_screen), cx)
    });
    assert!(!enqueued);
    store.read_with(cx, |store, _| assert_eq!(store.pending_route(), None));

    let enqueued = store.update(cx, |store, cx| {
        store.enqueue_active_screen(route.clone(), Some(&different_screen), cx)
    });
    assert!(enqueued);
    store.read_with(cx, |store, _| assert_eq!(store.pending_route(), Some(&route)));
}

#[gpui::test]
fn notifier_burst_keeps_only_the_newest_arrival_and_ignores_the_old_timer(cx: &mut TestAppContext) {
    let store = cx.new(|cx| NotifierStore::new(2_000_000, cx));

    let (first_id, newest_id) = store.update(cx, |store, cx| {
        let first_id = store.send("\u{041F}\u{0435}\u{0440}\u{0432}\u{043E}\u{0435}".to_owned(), send_options(0.012), cx);
        store.send("\u{0412}\u{0442}\u{043E}\u{0440}\u{043E}\u{0435}".to_owned(), send_options(0.0), cx);
        let newest_id = store.send("\u{0422}\u{0440}\u{0435}\u{0442}\u{044C}\u{0435}".to_owned(), send_options(0.0), cx);
        (first_id, newest_id)
    });

    store.read_with(cx, |store, _| assert!(store.items().is_empty()));

    cx.executor().advance_clock(Duration::from_millis(40));
    cx.run_until_parked();

    store.read_with(cx, |store, _| {
        let ids: Vec<_> = store.items().iter().map(|item| item.id).collect();
        assert_eq!(ids, vec![newest_id]);
        assert!(!store.items().iter().any(|item| item.id == first_id));
        assert_eq!(store.items().first().map(|item| item.title.as_str()), Some("\u{0422}\u{0440}\u{0435}\u{0442}\u{044C}\u{0435}"));
    });

    store.update(cx, |store, cx| store.dismiss(first_id, cx));

    store.read_with(cx, |store, _| {
        let ids: Vec<_> = store.items().iter().map(|item| item.id).collect();
        assert_eq!(ids, vec![newest_id]);
    });
}

#[test]
fn notifier_swipe_requires_enough_upward_distance_or_momentum() {
    assert!(!NotifierSwipeDismissPolicy::should_dismiss(-20.0, -70.0));
    assert!(NotifierSwipeDismissPolicy::should_dismiss(-40.0, -45.0));
    assert!(NotifierSwipeDismissPolicy::should_dismiss(-16.0, -100.0));
    assert!(!NotifierSwipeDismissPolicy::should_dismiss(60.0, 120.0));
}

#[test]
fn foreground_chat_policy_uses_the_exact_notification_scope() {
    let scopes = ChatMuteScopes { messages: false, mentions: true, replies: true, reactions: false, events: true };

    assert!(!PushChatNotificationPolicy::allows(Some("messages"), Some(&scopes)));
    assert!(PushChatNotificationPolicy::allows(Some("mentions"), Some(&scopes)));
    assert!(!PushChatNotificationPolicy::allows(Some("reactions"), Some(&scopes)));
    assert!(PushChatNotificationPolicy::allows(Some("events"), Some(&scopes)));
    assert!(PushChatNotificationPolicy::allows(None, Some(&scopes)));
    assert!(PushChatNotificationPolicy::allows(Some("unknown"), Some(&scopes)));
}
