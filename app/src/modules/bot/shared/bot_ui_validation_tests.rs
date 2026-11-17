use std::collections::HashMap;

use gpui::TestAppContext;
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsRequestOptions;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;
use crate::modules::bot::shared::bot_ui_validation::{
    BotChatManagement, BotFormValidation, BotMiniAppInitData, BotMiniAppURLPolicy,
};
use crate::modules::bot::stores::bot::bot_actions::bot_types::{BotMediaKind, BotTagRules};
use crate::modules::bot::stores::bot::bot_actions::bot_wire_types::{BotGatewayDTO, BotWireDTOExt};
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType, User};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageContentType, MessageMediaKind};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    MessageWireDTO, RiseStoredOutgoingMessage,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

fn decode<T: DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).expect("wire json decodes")
}

fn url(string: &str) -> Url {
    Url::from_string(string).expect("valid url")
}

#[test]
fn bot_wire_mapping_preserves_gateway_fields_and_milliseconds() {
    let dto: BotGatewayDTO = decode(
        r#"{
          "success": true,
          "error": "",
          "bot": {
            "id": "bot-1",
            "owner_user_id": "owner-1",
            "bot_user_id": "user-bot-1",
            "private_chat_id": "chat-1",
            "tag": "rise_helperbot",
            "name": "Rise Helper",
            "description": "Answers questions",
            "avatar_url": "https://cdn.example/avatar.jpg",
            "banner_url": "https://cdn.example/banner.jpg",
            "is_active": true,
            "can_join_groups": true,
            "privacy_mode": false,
            "created_at": 1783800123456,
            "updated_at": 1783800456789,
            "bot_user_chat_id": "user-chat-1",
            "is_owned_by_requester": true
          }
        }"#,
    );
    let bot = dto.bot.as_ref().expect("bot in response").to_bot_model();

    assert_eq!(bot.id, "bot-1");
    assert_eq!(bot.owner_user_id, "owner-1");
    assert_eq!(bot.bot_user_id, "user-bot-1");
    assert_eq!(bot.private_chat_id, "chat-1");
    assert_eq!(bot.tag, "rise_helperbot");
    assert_eq!(bot.banner_url, "https://cdn.example/banner.jpg");
    assert!(bot.can_join_groups);
    assert!(!bot.privacy_mode);
    assert_eq!(bot.created_at, 1_783_800_123_456);
    assert_eq!(bot.updated_at, 1_783_800_456_789);
    assert!(bot.is_owned_by_requester);
}

#[test]
fn bot_tag_sanitizer_produces_strict_lowercase_bot_suffix() {
    assert_eq!(BotTagRules::sanitize(" @My Helper ", true, false), "myhelperbot");
    assert_eq!(BotTagRules::sanitize("alpha__", true, false), "alphabot");
    assert_eq!(BotTagRules::sanitize("readybot", true, false), "readybot");
    assert!(BotTagRules::is_valid("rise_helperbot"));
    assert!(!BotTagRules::is_valid("Rise_helperbot"));
    assert!(!BotTagRules::is_valid("rise_helperBot"));
    assert!(!BotTagRules::is_valid("_helperbot"));
    assert!(!BotTagRules::is_valid("helper__bot"));
    assert_eq!(BotMediaKind::Avatar.file_context(), "bot_avatar");
    assert_eq!(BotMediaKind::Avatar.wire_key(), "avatar_url");
    assert_eq!(BotMediaKind::Banner.file_context(), "bot_banner");
    assert_eq!(BotMediaKind::Banner.wire_key(), "banner_url");
}

#[test]
fn message_wire_decodes_bot_keyboards_and_web_app_data() {
    let dto: MessageWireDTO = decode(
        r#"{
          "id": "message-1",
          "chat_id": "chat-1",
          "sender_id": "bot-user-1",
          "content": "Choose an action",
          "content_type": "text",
          "created_at": 1783800123456,
          "is_bot_message": true,
          "via_bot": "bot-user-1",
          "reply_markup": {
            "inline_keyboard": [
              [
                {"text": "Confirm", "callback_data": "confirm:1"},
                {"text": "Open", "web_app": {"url": "https://example.com/app", "bot_id": "bot-1", "mini_app_id": "mini-1"}}
              ]
            ]
          },
          "web_app_data": {"data": "payload", "button_text": "Open"}
        }"#,
    );
    let message = dto.to_message();
    let markup = message.reply_markup.as_ref().expect("reply markup");
    let buttons = markup.inline_keyboard.first().expect("inline row");
    let web_app_data = message.web_app_data.as_ref().expect("web app data");

    assert!(message.is_bot_message);
    assert_eq!(message.via_bot.as_deref(), Some("bot-user-1"));
    assert_eq!(message.created_at, 1_783_800_123.456);
    assert_eq!(buttons.len(), 2);
    assert_eq!(buttons[0].callback_data.as_deref(), Some("confirm:1"));
    assert_eq!(buttons[1].web_app_url.as_deref(), Some("https://example.com/app"));
    assert_eq!(buttons[1].bot_id.as_deref(), Some("bot-1"));
    assert_eq!(buttons[1].mini_app_id.as_deref(), Some("mini-1"));
    assert_eq!(web_app_data.data, "payload");
    assert_eq!(web_app_data.button_text.as_deref(), Some("Open"));
}

#[test]
fn message_wire_normalizes_persisted_gif_media_for_autoplay() {
    let dto: MessageWireDTO = decode(
        r#"{
          "id": "message-gif-1",
          "chat_id": "chat-1",
          "content_type": "animation",
          "media_items": [{
            "media_type": "image",
            "file_url": "https://cdn.example/animation.gif",
            "mime_type": "image/gif",
            "width": 320,
            "height": 240
          }]
        }"#,
    );
    let message = dto.to_message();

    assert_eq!(message.content_type, MessageContentType::Animation);
    assert_eq!(message.media_items.first().map(|item| item.kind), Some(MessageMediaKind::Animation));
    assert_eq!(message.grid_media_items().first().map(|item| item.kind), Some(MessageMediaKind::Animation));
}

#[test]
fn message_wire_normalizes_gif_media_type_for_autoplay() {
    let dto: MessageWireDTO = decode(
        r#"{
          "id": "message-gif-2",
          "chat_id": "chat-1",
          "content_type": "gif",
          "media_items": [{
            "media_type": "gif",
            "file_url": "https://cdn.example/animation.gif"
          }]
        }"#,
    );

    assert_eq!(dto.to_message().media_items.first().map(|item| item.kind), Some(MessageMediaKind::Animation));
}

#[test]
fn message_wire_tolerates_string_encoded_reply_markup_and_reply_buttons() {
    let dto: MessageWireDTO = decode(
        r#"{"id":"message-2","reply_markup":"{\"keyboard\":[[\"Help\",{\"text\":\"Launch\",\"web_app\":\"https://example.com/app\"}]],\"resize_keyboard\":true}"}"#,
    );
    let message = dto.to_message();
    let markup = message.reply_markup.as_ref().expect("reply markup");

    assert!(markup.resize_keyboard);
    assert_eq!(markup.keyboard.len(), 1);
    assert!(markup.keyboard[0][0].is_plain_text());
    assert_eq!(markup.keyboard[0][0].text, "Help");
    assert_eq!(markup.keyboard[0][1].web_app_url.as_deref(), Some("https://example.com/app"));
}

#[test]
fn chat_participant_wire_preserves_bot_ownership_flags() {
    let dto: RiseStoredChat = decode(
        r#"{
          "id": "chat-1",
          "type": "PRIVATE",
          "participant": {
            "id": "bot-user-1",
            "name": "Helper",
            "tag": "helperbot",
            "is_bot": true,
            "bot_owner_id": "owner-1",
            "bot_is_active": true
          }
        }"#,
    );
    let chat = dto.to_chat_info();

    assert!(chat.is_active_bot_chat());
    assert_eq!(chat.participant.bot_owner_id.as_deref(), Some("owner-1"));
}

#[test]
fn bot_chat_management_requires_bot_identity_and_owner() {
    let bot_chat = ChatInfo {
        id: "chat-1".to_owned(),
        r#type: ChatType::Direct,
        participant: User {
            id: "bot-user-1".to_owned(),
            tag: "helperbot".to_owned(),
            is_bot: true,
            bot_owner_id: Some("owner-1".to_owned()),
            bot_is_active: true,
            ..User::default()
        },
        ..ChatInfo::default()
    };
    let regular_chat = ChatInfo { id: "chat-2".to_owned(), r#type: ChatType::Direct, ..ChatInfo::default() };

    assert_eq!(BotChatManagement::normalized_tag(&bot_chat), "helperbot");
    assert!(BotChatManagement::can_manage(&bot_chat, "owner-1", None));
    assert!(!BotChatManagement::can_manage(&bot_chat, "other", None));
    assert!(!BotChatManagement::can_manage(&regular_chat, "owner-1", None));
}

#[test]
fn mini_app_url_policy_separates_production_from_owner_development_hosts() {
    let production = url("https://example.com/app");
    let local_http = url("http://127.0.0.1:5173/app");
    let local_https = url("https://192.168.1.8/app");
    let metadata = url("https://169.254.169.254/latest/meta-data");

    assert!(BotMiniAppURLPolicy::allows(&production, false));
    assert!(!BotMiniAppURLPolicy::allows(&local_http, false));
    assert!(BotMiniAppURLPolicy::allows(&local_http, true));
    assert!(!BotMiniAppURLPolicy::allows(&local_https, false));
    assert!(!BotMiniAppURLPolicy::allows(&metadata, false));
    assert!(!BotMiniAppURLPolicy::allows_development_http(&production));
    assert!(BotMiniAppURLPolicy::allows_development_http(&local_http));
    assert!(!BotMiniAppURLPolicy::allows_development_http(&local_https));
}

#[test]
fn mini_app_launch_data_reads_fragment_without_leaking_into_query() {
    let launch_url = url("https://app.example/open?theme=dark#screen=details&riseonlyInitData=user%253D42%2526hash%253Dsigned");

    assert_eq!(BotMiniAppInitData::extract(&launch_url), "user%3D42%26hash%3Dsigned");
    let leaks_into_query = UrlComponents::from_url(&launch_url, false)
        .and_then(|components| components.query_items())
        .map(|items| items.iter().any(|item| item.name == "riseonlyInitData"));
    assert_eq!(leaks_into_query, Some(false));
}

#[test]
fn mini_app_launch_data_prefers_last_signed_fragment_over_conflicting_url_state() {
    let launch_url = url(
        "https://app.example/open?riseonlyInitData=query-forged#riseonlyInitData=fragment-forged&screen=details&riseonlyInitData=auth_date%3D1%26hash%3Dsigned",
    );

    assert_eq!(BotMiniAppInitData::extract(&launch_url), "auth_date=1&hash=signed");
}

#[test]
fn web_app_submission_carries_signed_launch_proof_and_stable_temp_id() {
    let body = RiseStoredOutgoingMessage {
        chat_id: Some("chat-1".to_owned()),
        content_type: "text".to_owned(),
        web_app_data: Some(r#"{"data":"payload","init_data":"auth_date=1&hash=signed"}"#.to_owned()),
        temp_id: Some("temp-web-app-1".to_owned()),
        ..RiseStoredOutgoingMessage::default()
    };
    let object = serde_json::to_value(&body).expect("body encodes");
    let web_app_data = object.get("web_app_data").and_then(Value::as_str).expect("web_app_data string");
    let web_app_object: HashMap<String, String> = decode(web_app_data);

    assert_eq!(object.get("temp_id").and_then(Value::as_str), Some("temp-web-app-1"));
    assert_eq!(web_app_object.get("data").map(String::as_str), Some("payload"));
    assert_eq!(web_app_object.get("init_data").map(String::as_str), Some("auth_date=1&hash=signed"));
}

#[gpui::test]
fn web_app_submission_temp_id_is_deterministic_and_does_not_expose_launch_proof(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let services = MessageServicesStore::shared(cx);
        let services = services.read(cx);
        let init_data = "auth_date=1783800000000&query_id=q-1&hash=secret-signature";
        let first = services.web_app_submission_temp_id(init_data, "payload", Some("Send"));
        let retry = services.web_app_submission_temp_id(init_data, "payload", Some("Send"));
        let changed_data = services.web_app_submission_temp_id(init_data, "different", Some("Send"));
        let changed_button = services.web_app_submission_temp_id(init_data, "payload", None);
        let changed_launch = services.web_app_submission_temp_id(&format!("{init_data}x"), "payload", Some("Send"));

        assert_eq!(first, retry);
        assert_ne!(first, changed_data);
        assert_ne!(first, changed_button);
        assert_ne!(first, changed_launch);
        assert!(first.starts_with("webapp_"));
        assert_eq!(first.len(), 71);
        assert!(first.bytes().all(|byte| byte == 95 || byte.is_ascii_digit() || byte.is_ascii_lowercase()));
        assert!(!first.contains("secret-signature"));
        assert!(first.len() <= 128);
    });
}

#[gpui::test]
fn callback_answer_survives_realtime_before_acknowledgement(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let services = BotServicesStore::shared(cx);
        services.update(cx, |services, cx| services.reset_account_state(cx));

        services.update(cx, |services, cx| services.begin_callback("message-1:button-1", cx));
        let payload: Value = decode(
            r#"{
              "callback_query_id": "callback-1",
              "text": "Done",
              "show_alert": true,
              "url": "https://example.com/result"
            }"#,
        );
        services.update(cx, |services, cx| services.callback_query_answered_handler(Some(&payload), cx));
        assert!(services.read(cx).pending_callback_scopes.contains("message-1:button-1"));

        services.update(cx, |services, cx| services.bind_callback("message-1:button-1", "callback-1", cx));
        let answer = services.read(cx).callback_answer("message-1:button-1").expect("callback answer");
        assert!(!services.read(cx).pending_callback_scopes.contains("message-1:button-1"));
        assert_eq!(answer.text, "Done");
        assert!(answer.show_alert);
        assert_eq!(answer.url, "https://example.com/result");

        services.update(cx, |services, cx| services.reset_account_state(cx));
    });
}

#[gpui::test]
fn callback_answer_completes_pending_acknowledged_request(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let services = BotServicesStore::shared(cx);
        services.update(cx, |services, cx| services.reset_account_state(cx));

        services.update(cx, |services, cx| {
            services.begin_callback("message-2:button-2", cx);
            services.bind_callback("message-2:button-2", "callback-2", cx);
        });
        let payload: Value = decode(
            r#"{
              "callback_query_id": "callback-2",
              "text": "Accepted",
              "show_alert": false,
              "url": ""
            }"#,
        );
        services.update(cx, |services, cx| services.callback_query_answered_handler(Some(&payload), cx));

        let store = services.read(cx);
        assert!(!store.pending_callback_scopes.contains("message-2:button-2"));
        assert_eq!(store.callback_answer("message-2:button-2").map(|answer| answer.text).as_deref(), Some("Accepted"));
        assert!(store.early_callback_answers_by_id.is_empty());

        services.update(cx, |services, cx| services.reset_account_state(cx));
    });
}

#[test]
fn bot_cache_keys_are_scoped_and_stable() {
    let first = BotServicesStore::owned_bots_cache_key("owner-1");
    let repeated = BotServicesStore::owned_bots_cache_key("owner-1");
    let other_owner = BotServicesStore::owned_bots_cache_key("owner-2");
    let first_bot = BotServicesStore::mini_apps_cache_key("owner-1", "bot-1");
    let other_bot = BotServicesStore::mini_apps_cache_key("owner-1", "bot-2");

    assert_eq!(first, repeated);
    assert_ne!(first, other_owner);
    assert_ne!(first_bot, other_bot);
    assert!(BotServicesStore::bot_by_tag_cache_key("helperbot").contains("helperbot"));
}

#[test]
fn sensitive_options_disable_caches_debt_and_debugger_tracing() {
    let options = BotServicesStore::sensitive_request_options("reveal_bot_token", true);

    assert_eq!(options.cache_key, None);
    assert!(!options.store_response_in_cache);
    assert!(!options.persist_cache_to_disk());
    assert!(!options.track_until_acknowledged);
    assert!(options.exclude_from_debugger);

    let default_dictionary: Map<String, Value> = decode(r#"{"service": "bot", "method": "get_bot"}"#);
    let default_options = SaiWsRequestOptions::from_options_dictionary(&default_dictionary).expect("default options");
    let explicit_dictionary: Map<String, Value> =
        decode(r#"{"service": "bot", "method": "reveal_bot_token", "excludeFromDebugger": true}"#);
    let explicit_options = SaiWsRequestOptions::from_options_dictionary(&explicit_dictionary).expect("explicit options");

    assert!(!default_options.exclude_from_debugger);
    assert!(explicit_options.exclude_from_debugger);
}

#[test]
fn bot_ui_validation_enforces_developer_url_boundary() {
    let production = url("https://apps.example.com/shop");
    let localhost = url("http://localhost:5173");
    let local_domain = url("http://demo.local:8080");
    let private_ten = url("http://10.20.30.40:3000");
    let private_172 = url("http://172.31.1.2");
    let private_192 = url("http://192.168.1.9");
    let private_ipv6 = url("http://[fd00::1]:5173");
    let public_http = url("http://8.8.8.8/app");
    let public_prefix_domain = url("http://ffmpeg.org/app");
    let private_prefix_domain = url("http://192.168.1.9.evil.example/app");

    assert!(BotMiniAppURLPolicy::allows(&production, false));
    assert!(!BotMiniAppURLPolicy::allows(&localhost, false));
    assert!(BotMiniAppURLPolicy::allows(&localhost, true));
    assert!(BotMiniAppURLPolicy::allows(&local_domain, true));
    assert!(BotMiniAppURLPolicy::allows(&private_ten, true));
    assert!(BotMiniAppURLPolicy::allows(&private_172, true));
    assert!(BotMiniAppURLPolicy::allows(&private_192, true));
    assert!(BotMiniAppURLPolicy::allows(&private_ipv6, true));
    assert!(!BotMiniAppURLPolicy::allows(&public_http, true));
    assert!(!BotMiniAppURLPolicy::allows(&public_prefix_domain, true));
    assert!(!BotMiniAppURLPolicy::allows(&private_prefix_domain, true));
    assert!(BotFormValidation::is_valid_webhook_url("https://hooks.example.com/riseonly"));
    assert!(!BotFormValidation::is_valid_webhook_url("http://hooks.example.com/riseonly"));
    assert!(!BotFormValidation::is_valid_webhook_url("https://localhost/hook"));
    assert!(!BotFormValidation::is_valid_webhook_url("https://169.254.169.254/hook"));
    assert!(!BotFormValidation::is_valid_webhook_url("https://192.168.1.10/hook"));
    assert!(BotFormValidation::is_valid_command("start"));
    assert_eq!(BotFormValidation::normalized_command("/Start!"), "start");
    assert!(BotFormValidation::is_valid_mini_app_short_name("shop_app"));
}

#[test]
fn bot_mini_app_deep_links_preserve_unresolved_source() {
    let universal = url("https://riseonly.net/b/helperbot/shop?startapp=order_42");
    let custom = url("riseonly://b/helperbot/shop?start=debug");

    assert_eq!(
        UniversalLinkParser::parse(&universal, AppEnvironment::Prod),
        Some(PushDeepLink::BotMiniApp {
            tag: "helperbot".to_owned(),
            slug: "shop".to_owned(),
            start_param: Some("order_42".to_owned()),
        })
    );
    assert_eq!(
        UniversalLinkParser::parse(&custom, AppEnvironment::Prod),
        Some(PushDeepLink::BotMiniApp {
            tag: "helperbot".to_owned(),
            slug: "shop".to_owned(),
            start_param: Some("debug".to_owned()),
        })
    );
}

#[test]
fn bot_navigation_routes_expose_stable_sai_screens() {
    let settings = RootRoute::BotSettings { bot_id: "bot-1".to_owned() };
    let mini_app = RootRoute::BotMiniApp {
        url: url("https://apps.example.com/launch?initData=signed"),
        title: "Shop".to_owned(),
        allows_development_http: false,
        chat_id: Some("chat-1".to_owned()),
        bot_id: None,
        is_web_link: false,
    };
    let unresolved = RootRoute::BotMiniAppLink {
        tag: "helperbot".to_owned(),
        slug: "shop".to_owned(),
        start_param: Some("42".to_owned()),
    };

    assert_eq!(settings.sai_active_screen().id, "BotSettings");
    assert_eq!(mini_app.sai_active_screen().id, "BotMiniApp");
    assert_eq!(unresolved.sai_active_screen().id, "BotMiniApp");
    assert_ne!(mini_app, unresolved);
}
