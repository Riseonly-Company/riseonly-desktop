use gpui::{AppContext, Entity, TestAppContext};
use rise_core::foundation::foundation_url::Url;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

fn new_store(cx: &mut TestAppContext) -> Entity<NavigationStore> {
    cx.new(NavigationStore::new)
}

fn chat(chat_id: &str) -> RootRoute {
    RootRoute::Chat { chat_id: Some(chat_id.to_owned()) }
}

fn chat_topic(chat_id: &str, topic_id: &str) -> RootRoute {
    RootRoute::ChatTopic { chat_id: Some(chat_id.to_owned()), topic_id: Some(topic_id.to_owned()) }
}

fn url(string: &str) -> Url {
    Url::from_string(string).expect("a literal https url parses")
}

fn bot_mini_app(url: Url, title: &str, chat_id: Option<&str>) -> RootRoute {
    RootRoute::BotMiniApp {
        url,
        title: title.to_owned(),
        allows_development_http: false,
        chat_id: chat_id.map(str::to_owned),
        bot_id: None,
        is_web_link: false,
    }
}

fn push(store: &Entity<NavigationStore>, route: RootRoute, cx: &mut TestAppContext) {
    store.update(cx, |store, cx| store.push(route, Default::default(), cx));
}

#[gpui::test]
fn authentication_and_chat_use_the_same_root_route_path(cx: &mut TestAppContext) {
    let store = new_store(cx);

    push(&store, RootRoute::SignUp, cx);
    store.read_with(cx, |store, _| assert_eq!(store.path, vec![RootRoute::SignUp]));

    store.update(cx, |store, cx| store.go_to_main_tabs_posts(cx));
    store.read_with(cx, |store, _| assert!(store.path.is_empty()));

    store.update(cx, |store, cx| store.switch_tab(MainTab::Chats, cx));
    push(&store, chat("chat-1"), cx);

    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Chats);
        assert_eq!(store.path, vec![chat("chat-1")]);
    });
}

#[gpui::test]
fn rapid_chat_push_transfers_first_frame_ownership_by_epoch(cx: &mut TestAppContext) {
    let store = new_store(cx);
    let first_route = chat_topic("chat-1", "topic-1");
    let second_route = chat_topic("chat-1", "topic-2");

    push(&store, first_route.clone(), cx);
    let first_epoch = store.read_with(cx, |store, _| store.chat_presentation_epoch_for(&first_route));

    assert!(first_epoch.is_some());
    store.read_with(cx, |store, _| assert_eq!(store.path, vec![first_route.clone()]));

    push(&store, second_route.clone(), cx);
    let second_epoch = store.read_with(cx, |store, _| store.chat_presentation_epoch_for(&second_route));

    assert!(second_epoch.is_some());
    assert_ne!(second_epoch, first_epoch);
    store.read_with(cx, |store, _| assert_eq!(store.path.last(), Some(&second_route)));

    store.update(cx, |store, cx| store.complete_chat_first_frame_transition(&first_route, first_epoch, cx));
    store.read_with(cx, |store, _| assert_eq!(store.chat_first_frame_ready_epoch(), None));

    store.update(cx, |store, cx| store.complete_chat_first_frame_transition(&second_route, second_epoch, cx));
    store.read_with(cx, |store, _| {
        assert_eq!(store.chat_first_frame_ready_epoch(), second_epoch);
        assert_eq!(store.chat_presentation_epoch(), second_epoch);
    });
}

#[gpui::test]
fn leaving_chat_destination_rejects_its_late_first_frame_callback(cx: &mut TestAppContext) {
    let store = new_store(cx);
    let first_route = chat("chat-1");

    push(&store, first_route.clone(), cx);
    let first_epoch = store.read_with(cx, |store, _| store.chat_presentation_epoch_for(&first_route));
    assert!(first_epoch.is_some());

    store.update(cx, |store, cx| {
        store.pop(cx);
        store.complete_chat_first_frame_transition(&first_route, first_epoch, cx);
    });

    store.read_with(cx, |store, _| {
        assert!(store.path.is_empty());
        assert_eq!(store.chat_first_frame_ready_epoch(), None);
    });

    let second_route = chat("chat-2");
    push(&store, second_route.clone(), cx);
    let second_epoch = store.read_with(cx, |store, _| store.chat_presentation_epoch_for(&second_route));
    assert!(second_epoch.is_some());

    store.update(cx, |store, cx| {
        store.switch_tab(MainTab::Profile, cx);
        store.complete_chat_first_frame_transition(&second_route, second_epoch, cx);
    });
    store.read_with(cx, |store, _| assert_eq!(store.chat_first_frame_ready_epoch(), None));
}

#[gpui::test]
fn repeated_same_chat_push_survives_interactive_pop_race(cx: &mut TestAppContext) {
    let store = new_store(cx);
    let route = chat("chat-1");

    push(&store, route.clone(), cx);
    let first_epoch = store.read_with(cx, |store, _| store.chat_presentation_epoch_for(&route));
    store.update(cx, |store, cx| store.complete_chat_first_frame_transition(&route, first_epoch, cx));

    store.read_with(cx, |store, _| {
        assert_eq!(store.chat_first_frame_ready_epoch(), first_epoch);
        assert_eq!(store.chat_presentation_epoch(), first_epoch);
    });

    push(&store, route.clone(), cx);

    store.read_with(cx, |store, _| {
        assert_eq!(store.path, vec![route.clone()]);
        assert_eq!(store.chat_presentation_epoch(), first_epoch);
    });

    store.update(cx, |store, cx| {
        store.path.pop();
        store.validate_chat_first_frame_transition_path(cx);
    });
    cx.run_until_parked();
    let second_epoch = store.read_with(cx, |store, _| store.chat_presentation_epoch_for(&route));

    store.read_with(cx, |store, _| {
        assert_eq!(store.path, vec![route.clone()]);
        assert_eq!(store.chat_first_frame_ready_epoch(), None);
    });
    assert!(second_epoch.is_some());
    assert_ne!(second_epoch, first_epoch);
}

#[test]
fn every_message_screen_route_uses_the_expected_presentation_scope() {
    assert_eq!(chat("chat-1").chat_first_frame_transition_scope().as_deref(), Some("chat-1"));
    assert_eq!(
        chat_topic("chat-1", "topic-2").chat_first_frame_transition_scope().as_deref(),
        Some("chat-1")
    );
    assert_eq!(
        RootRoute::ChatThread { chat_id: Some("chat-1".to_owned()), root_id: Some("message-3".to_owned()) }
            .chat_first_frame_transition_scope(),
        Some(MessageServicesStore::message_scope("chat-1", None, Some("message-3")))
    );
    assert_eq!(
        RootRoute::ForumAll { chat_id: Some("chat-1".to_owned()) }
            .chat_first_frame_transition_scope()
            .as_deref(),
        Some("chat-1")
    );
    assert_eq!(RootRoute::Chat { chat_id: None }.chat_first_frame_transition_scope(), None);
    assert_eq!(
        RootRoute::ChatProfile { chat_id: Some("chat-1".to_owned()) }.chat_first_frame_transition_scope(),
        None
    );
}

#[gpui::test]
fn mini_app_presentation_keeps_underlying_navigation_mounted(cx: &mut TestAppContext) {
    let store = new_store(cx);
    let chat_route = chat("chat-1");
    let url = url("https://apps.example.com/launch");

    push(&store, chat_route.clone(), cx);
    push(&store, bot_mini_app(url.clone(), "Shop", Some("chat-1")), cx);

    store.read_with(cx, |store, _| {
        assert_eq!(store.path, vec![chat_route.clone()]);
        assert_eq!(store.bot_mini_app_presentation().map(|presentation| &presentation.url), Some(&url));
        assert_eq!(store.sai_active_screen().id, "BotMiniApp");
    });
}

#[gpui::test]
fn resolved_mini_app_replaces_resolver_with_overlay(cx: &mut TestAppContext) {
    let store = new_store(cx);
    let url = url("https://apps.example.com/launch");

    push(
        &store,
        RootRoute::BotMiniAppLink { tag: "helperbot".to_owned(), slug: "shop".to_owned(), start_param: None },
        cx,
    );
    store.update(cx, |store, cx| store.replace_top(bot_mini_app(url.clone(), "Shop", None), cx));

    store.read_with(cx, |store, _| {
        assert!(store.path.is_empty());
        assert_eq!(store.bot_mini_app_presentation().map(|presentation| &presentation.url), Some(&url));
    });
}

#[gpui::test]
fn stale_mini_app_dismissal_cannot_close_replacement(cx: &mut TestAppContext) {
    let store = new_store(cx);
    let first_url = url("https://apps.example.com/first");
    let second_url = url("https://apps.example.com/second");

    push(&store, bot_mini_app(first_url, "First", None), cx);
    let first_id = store
        .read_with(cx, |store, _| store.bot_mini_app_presentation().map(|presentation| presentation.id))
        .expect("the first mini app is presented");

    push(&store, bot_mini_app(second_url.clone(), "Second", None), cx);
    store.update(cx, |store, cx| store.dismiss_bot_mini_app(first_id, cx));

    store.read_with(cx, |store, _| {
        assert_eq!(store.bot_mini_app_presentation().map(|presentation| &presentation.url), Some(&second_url));
    });
}

#[gpui::test]
fn every_section_keeps_its_own_stack_on_a_rail_pick(cx: &mut TestAppContext) {
    let store = new_store(cx);
    store.update(cx, |store, cx| store.switch_tab(MainTab::Chats, cx));
    push(&store, chat("chat-1"), cx);

    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Profile, cx)));
    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Profile);
        assert!(store.path.is_empty());
    });

    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Posts, cx)));
    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Posts);
        assert!(store.path.is_empty());
    });

    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Chats, cx)));
    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Chats);
        assert_eq!(store.path, vec![chat("chat-1")]);
    });

    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Chats, cx)));
    store.read_with(cx, |store, _| assert!(store.path.is_empty()));
    store.update(cx, |store, cx| assert!(!store.pick_tab(MainTab::Chats, cx)));
}

#[gpui::test]
fn picking_the_section_a_link_left_returns_its_open_chat(cx: &mut TestAppContext) {
    let store = new_store(cx);
    store.update(cx, |store, cx| store.switch_tab(MainTab::Chats, cx));
    push(&store, chat("chat-1"), cx);
    let post = RootRoute::PostDetail { post_id: Some("524".to_owned()), origin: None };
    store.update(cx, |store, cx| {
        store.switch_tab(MainTab::Posts, cx);
        store.push_in_centre(post.clone(), NavigationStoreSettlingPush::Wait, cx);
    });

    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Chats, cx)));
    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Chats);
        assert_eq!(store.path, vec![chat("chat-1")]);
    });

    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Posts, cx)));
    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Posts);
        assert_eq!(store.path, vec![post.clone()]);
    });
}

#[gpui::test]
fn a_post_link_in_an_open_chat_stays_over_the_chat(cx: &mut TestAppContext) {
    let store = new_store(cx);
    store.update(cx, |store, cx| store.switch_tab(MainTab::Chats, cx));
    push(&store, chat("chat-1"), cx);
    let post = RootRoute::PostDetail { post_id: Some("524".to_owned()), origin: None };
    store.update(cx, |store, cx| {
        store.switch_tab_for_link(MainTab::Posts, cx);
        store.push_in_centre(post.clone(), NavigationStoreSettlingPush::Wait, cx);
    });
    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Chats);
        assert_eq!(store.path, vec![chat("chat-1"), post.clone()]);
    });

    store.update(cx, |store, cx| store.pop(cx));
    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Chats);
        assert_eq!(store.path, vec![chat("chat-1")]);
    });

    store.update(cx, |store, cx| store.switch_tab(MainTab::Profile, cx));
    store.update(cx, |store, cx| store.pick_tab(MainTab::Profile, cx));
    store.read_with(cx, |store, _| assert!(store.path.is_empty()));
    store.update(cx, |store, cx| store.switch_tab_for_link(MainTab::Posts, cx));
    store.read_with(cx, |store, _| assert_eq!(store.current_tab, MainTab::Posts));
}

#[gpui::test]
fn going_to_posts_drops_every_parked_stack(cx: &mut TestAppContext) {
    let store = new_store(cx);
    store.update(cx, |store, cx| store.switch_tab(MainTab::Chats, cx));
    push(&store, chat("chat-1"), cx);
    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Profile, cx)));

    store.update(cx, |store, cx| store.go_to_main_tabs_posts(cx));
    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Chats, cx)));

    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Chats);
        assert!(store.path.is_empty());
    });
}

#[gpui::test]
fn a_tab_switch_over_open_settings_hands_the_rail_mark_back(cx: &mut TestAppContext) {
    let store = new_store(cx);
    push(&store, RootRoute::Settings, cx);
    store.read_with(cx, |store, _| assert_eq!(store.routes_since_tab_switch(), &[RootRoute::Settings]));

    store.update(cx, |store, cx| store.switch_tab(MainTab::Posts, cx));
    push(&store, RootRoute::PostDetail { post_id: Some("108".to_string()), origin: None }, cx);
    store.read_with(cx, |store, _| {
        assert_eq!(store.path.len(), 2);
        assert!(!store.routes_since_tab_switch().contains(&RootRoute::Settings));
    });

    store.update(cx, |store, cx| store.pop(cx));
    store.read_with(cx, |store, _| assert_eq!(store.routes_since_tab_switch(), &[RootRoute::Settings]));

    push(&store, RootRoute::SubscriptionSettings, cx);
    store.read_with(cx, |store, _| assert!(store.routes_since_tab_switch().contains(&RootRoute::Settings)));
}

#[gpui::test]
fn a_section_picked_after_the_settings_never_gets_them_back(cx: &mut TestAppContext) {
    let store = new_store(cx);
    push(&store, RootRoute::Settings, cx);
    push(&store, RootRoute::CustomizationSettings, cx);
    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Music, cx)));
    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Posts, cx)));

    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Posts);
        assert!(store.path.is_empty());
    });
}

#[gpui::test]
fn the_settings_leave_what_lay_below_them_with_their_section(cx: &mut TestAppContext) {
    let store = new_store(cx);
    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Chats, cx)));
    push(&store, chat("chat-1"), cx);
    push(&store, RootRoute::Settings, cx);
    push(&store, RootRoute::ThemeSettings, cx);
    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Posts, cx)));
    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Chats, cx)));
    store.read_with(cx, |store, _| assert_eq!(store.path, vec![chat("chat-1")]));

    push(&store, RootRoute::Settings, cx);
    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Chats, cx)));
    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Chats);
        assert_eq!(store.path, vec![chat("chat-1")]);
    });
}

#[gpui::test]
fn picking_open_settings_on_the_rail_pops_them_to_their_root(cx: &mut TestAppContext) {
    let store = new_store(cx);
    store.update(cx, |store, cx| store.open_rail_route(RootRoute::Settings, Default::default(), cx));
    push(&store, RootRoute::CustomizationSettings, cx);
    push(&store, RootRoute::ChatWallpapersSettings, cx);

    store.update(cx, |store, cx| store.open_rail_route(RootRoute::Settings, Default::default(), cx));
    store.read_with(cx, |store, _| assert_eq!(store.path, vec![RootRoute::Settings]));

    store.update(cx, |store, cx| store.open_rail_route(RootRoute::Settings, Default::default(), cx));
    store.read_with(cx, |store, _| assert_eq!(store.path, vec![RootRoute::Settings]));
}

#[gpui::test]
fn picking_open_settings_after_a_tab_switch_over_them_pops_them_to_their_root(cx: &mut TestAppContext) {
    let store = new_store(cx);
    store.update(cx, |store, cx| store.open_rail_route(RootRoute::Settings, Default::default(), cx));
    push(&store, RootRoute::CustomizationSettings, cx);
    store.update(cx, |store, cx| store.switch_tab(MainTab::Chats, cx));
    push(&store, RootRoute::ChatWallpapersSettings, cx);

    store.update(cx, |store, cx| store.open_rail_route(RootRoute::Settings, Default::default(), cx));
    store.read_with(cx, |store, _| assert_eq!(store.path, vec![RootRoute::Settings]));
    store.update(cx, |store, cx| store.open_rail_route(RootRoute::Settings, Default::default(), cx));
    store.read_with(cx, |store, _| assert_eq!(store.path, vec![RootRoute::Settings]));
}

#[gpui::test]
fn picking_settings_keeps_the_section_under_them(cx: &mut TestAppContext) {
    let store = new_store(cx);
    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Chats, cx)));
    push(&store, chat("chat-1"), cx);
    store.update(cx, |store, cx| store.open_rail_route(RootRoute::Settings, Default::default(), cx));
    push(&store, RootRoute::ThemeSettings, cx);

    store.update(cx, |store, cx| store.open_rail_route(RootRoute::Settings, Default::default(), cx));
    store.read_with(cx, |store, _| assert_eq!(store.path, vec![chat("chat-1"), RootRoute::Settings]));
    store.read_with(cx, |store, _| assert!(store.shows_settings_section_root()));
    push(&store, RootRoute::ThemeSettings, cx);
    store.read_with(cx, |store, _| assert!(!store.shows_settings_section_root()));
}

#[gpui::test]
fn a_settings_row_beside_the_list_replaces_the_open_section(cx: &mut TestAppContext) {
    let store = new_store(cx);
    push(&store, chat("chat-1"), cx);
    store.update(cx, |store, cx| store.open_rail_route(RootRoute::Settings, Default::default(), cx));
    push(&store, RootRoute::ProfileSettings, cx);
    push(&store, RootRoute::ThemeSettings, cx);
    store.update(cx, |store, cx| {
        store.set_settings_list_beside_stack(true);
        assert_eq!(store.settings_split_index(), Some(1));
        store.open_from_settings_list(RootRoute::PrivacySettings, cx);
    });
    store.read_with(cx, |store, _| {
        assert_eq!(store.path, vec![chat("chat-1"), RootRoute::Settings, RootRoute::PrivacySettings]);
    });
    store.update(cx, |store, cx| store.open_from_settings_list(RootRoute::PrivacySettings, cx));
    store.read_with(cx, |store, _| {
        assert_eq!(store.path, vec![chat("chat-1"), RootRoute::Settings, RootRoute::PrivacySettings]);
    });
}

#[gpui::test]
fn settings_pushed_from_a_page_go_back_to_it_beside_the_list(cx: &mut TestAppContext) {
    let store = new_store(cx);
    push(&store, RootRoute::Settings, cx);
    push(&store, RootRoute::ProfileSettings, cx);
    store.update(cx, |store, cx| {
        store.set_settings_list_beside_stack(true);
        assert!(store.shows_settings_back());
        store.pop_settings(cx);
    });
    store.read_with(cx, |store, _| assert!(store.path.is_empty()));

    store.update(cx, |store, cx| store.open_rail_route(RootRoute::Settings, Default::default(), cx));
    push(&store, RootRoute::ProfileSettings, cx);
    store.read_with(cx, |store, _| assert!(!store.shows_settings_back()));
}

#[gpui::test]
fn back_from_a_post_link_over_a_chat_returns_to_the_chat_section(cx: &mut TestAppContext) {
    let store = new_store(cx);
    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Profile, cx)));
    push(&store, RootRoute::Settings, cx);
    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Posts, cx)));
    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Chats, cx)));
    push(&store, chat("chat-150"), cx);

    let post = RootRoute::PostDetail { post_id: Some("108".to_owned()), origin: None };
    store.update(cx, |store, cx| store.switch_tab(MainTab::Posts, cx));
    push(&store, post.clone(), cx);
    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Posts);
        assert_eq!(store.path, vec![chat("chat-150"), post.clone()]);
    });

    store.update(cx, |store, cx| store.pop(cx));
    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Chats);
        assert_eq!(store.path, vec![chat("chat-150")]);
    });

    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Profile, cx)));
    store.read_with(cx, |store, _| assert_eq!(store.path, vec![RootRoute::Settings]));
}

#[gpui::test]
fn a_link_from_a_section_root_keeps_the_section_it_opened(cx: &mut TestAppContext) {
    let store = new_store(cx);
    store.update(cx, |store, cx| assert!(store.pick_tab(MainTab::Chats, cx)));
    store.update(cx, |store, cx| store.switch_tab(MainTab::Posts, cx));
    push(&store, RootRoute::PostDetail { post_id: Some("108".to_owned()), origin: None }, cx);
    store.update(cx, |store, cx| store.pop(cx));
    store.read_with(cx, |store, _| {
        assert_eq!(store.current_tab, MainTab::Posts);
        assert!(store.path.is_empty());
    });
}
