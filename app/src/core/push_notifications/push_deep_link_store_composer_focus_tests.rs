use std::collections::HashMap;
use std::time::Duration;

use gpui::TestAppContext;
use rise_core::foundation::foundation_date::{Date, RiseClock, TestClock};

use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiActiveScreen;
use crate::core::push_notifications::push_deep_link_store::{PushDeepLink, PushDeepLinkStore};

fn chat_link(chat_id: &str) -> PushDeepLink {
    PushDeepLink::Chat { chat_id: chat_id.to_owned() }
}

fn topic_link(chat_id: &str, topic_id: &str) -> PushDeepLink {
    PushDeepLink::ChatTopic { chat_id: chat_id.to_owned(), topic_id: topic_id.to_owned() }
}

fn chat_screen(chat_id: &str) -> SaiActiveScreen {
    let params = [("chatId".to_owned(), chat_id.to_owned())].into_iter().collect::<HashMap<_, _>>();
    SaiActiveScreen::new("Chat".to_owned(), params)
}

#[gpui::test]
fn request_waits_for_the_route_and_fires_once(cx: &mut TestAppContext) {
    let store = cx.update(PushDeepLinkStore::shared);
    let queued = store.update(cx, |store, cx| {
        store.enqueue_for_composer_active_screen(chat_link("chat-1"), None, cx)
    });
    assert!(queued);
    let while_queued = store.update(cx, |store, cx| store.take_composer_focus(&["chat-1"], None, false, cx));
    assert!(!while_queued);

    store.update(cx, |store, cx| store.consume(cx));
    let first = store.update(cx, |store, cx| store.take_composer_focus(&["chat-1"], None, false, cx));
    let second = store.update(cx, |store, cx| store.take_composer_focus(&["chat-1"], None, false, cx));
    assert!(first);
    assert!(!second);
}

#[gpui::test]
fn chat_already_on_screen_takes_it_without_a_route(cx: &mut TestAppContext) {
    let store = cx.update(PushDeepLinkStore::shared);
    let active = chat_screen("chat-1");
    let queued = store.update(cx, |store, cx| {
        store.enqueue_for_composer_active_screen(chat_link("chat-1"), Some(&active), cx)
    });
    assert!(!queued);
    let taken = store.update(cx, |store, cx| store.take_composer_focus(&["chat-1"], None, false, cx));
    assert!(taken);
}

#[gpui::test]
fn another_chat_or_topic_does_not_take_it(cx: &mut TestAppContext) {
    let store = cx.update(PushDeepLinkStore::shared);
    store.update(cx, |store, cx| {
        store.enqueue_for_composer_active_screen(topic_link("chat-1", "topic-1"), None, cx);
        store.consume(cx);
    });
    let other_chat = store.update(cx, |store, cx| store.take_composer_focus(&["chat-2"], Some("topic-1"), false, cx));
    let other_topic = store.update(cx, |store, cx| store.take_composer_focus(&["chat-1"], Some("topic-2"), false, cx));
    let no_topic = store.update(cx, |store, cx| store.take_composer_focus(&["chat-1"], None, false, cx));
    assert!(!other_chat);
    assert!(!other_topic);
    assert!(!no_topic);
    let all_topics = store.update(cx, |store, cx| store.take_composer_focus(&["chat-1"], None, true, cx));
    assert!(all_topics);
}

#[gpui::test]
fn plain_link_cancels_the_request(cx: &mut TestAppContext) {
    let store = cx.update(PushDeepLinkStore::shared);
    store.update(cx, |store, cx| {
        store.enqueue_for_composer_active_screen(chat_link("chat-1"), None, cx);
        store.enqueue_active_screen(chat_link("chat-1"), None, cx);
        store.consume(cx);
    });
    let taken = store.update(cx, |store, cx| store.take_composer_focus(&["chat-1"], None, false, cx));
    assert!(!taken);
}

#[gpui::test]
fn link_that_is_no_chat_asks_for_nothing(cx: &mut TestAppContext) {
    let store = cx.update(PushDeepLinkStore::shared);
    store.update(cx, |store, cx| {
        store.enqueue_for_composer_active_screen(PushDeepLink::Music, None, cx);
        store.consume(cx);
    });
    let taken = store.update(cx, |store, cx| store.take_composer_focus(&["chat-1"], None, false, cx));
    assert!(!taken);
}

#[gpui::test]
fn request_expires_only_after_the_route_was_served(cx: &mut TestAppContext) {
    let clock = TestClock::new(Date::from_ms(0));
    let _guard = RiseClock::install_test(clock.clone());
    let store = cx.update(PushDeepLinkStore::shared);
    store.update(cx, |store, cx| {
        store.enqueue_for_composer_active_screen(chat_link("chat-1"), None, cx);
    });
    clock.advance(Duration::from_secs(60));
    store.update(cx, |store, cx| store.consume(cx));
    clock.advance(Duration::from_secs(60));
    let late = store.update(cx, |store, cx| store.take_composer_focus(&["chat-1"], None, false, cx));
    assert!(!late);
}
