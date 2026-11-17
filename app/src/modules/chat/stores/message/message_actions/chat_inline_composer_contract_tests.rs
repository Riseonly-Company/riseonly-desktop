use gpui::TestAppContext;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::foundation::ns_range::NSRange;
use serde_json::{Map, Value};

use crate::modules::bot::stores::bot::bot_actions::bot_inline_types::{BotInlinePreviewMessageModel, BotInlineResultModel};
use crate::modules::chat::components::chat::composer::chat_composer_inline_context::{
    ChatComposerInlineContext, ChatComposerMentionEntityCodec,
};
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatComposerUserMention;
use crate::modules::chat::components::chat::composer::chat_inline_composer_panel::{
    ChatInlineComposerPanelMetrics, ChatInlineComposerPresentationContract,
};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageBotKeyboardButton, MessageBotReplyMarkup, MessageReplyTo,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

const FEED_INSET_SPACING: f32 = 10.0;
const EMPTY_PAGE_MAXIMUM_ATTEMPTS: usize = 3;
const BOT_TEXT_CYRILLIC_QUERY: &str = "\u{43a}\u{43e}\u{442}";

fn caret(text: &str) -> Option<NSRange> {
    Some(NSRange::new(text.encode_utf16().count(), 0))
}

fn mention(user_id: &str, tag: &str, range: NSRange) -> ChatComposerUserMention {
    ChatComposerUserMention { user_id: user_id.to_owned(), tag: tag.to_owned(), range }
}

fn entities(encoded: Option<String>) -> Vec<Map<String, Value>> {
    let Some(encoded) = encoded else {
        panic!("the codec encodes entities");
    };
    match serde_json::from_str::<Vec<Map<String, Value>>>(&encoded) {
        Ok(entities) => entities,
        Err(error) => panic!("entities are a JSON array: {error}"),
    }
}

fn collapsed_height(available_height: f32, is_expanded: bool) -> f32 {
    ChatInlineComposerPanelMetrics::height(available_height, is_expanded, ChatInlineComposerPanelMetrics::COLLAPSED_HEIGHT)
}

fn make_result(query_id: &str, preview_query_id: &str, reply_markup: &str) -> BotInlineResultModel {
    BotInlineResultModel {
        id: Uuid::new_v4().uuid_string(),
        query_id: query_id.to_owned(),
        r#type: "article".to_owned(),
        title: "Result".to_owned(),
        description: String::new(),
        thumbnail_url: String::new(),
        media_url: String::new(),
        width: None,
        height: None,
        duration: None,
        mime_type: String::new(),
        content: "Result".to_owned(),
        caption: String::new(),
        send_message: Some(BotInlinePreviewMessageModel {
            content: "Result".to_owned(),
            original_content: "Result".to_owned(),
            content_type: "text".to_owned(),
            caption: String::new(),
            media_items: Vec::new(),
            entities: String::new(),
            reply_markup: reply_markup.to_owned(),
            inline_bot_id: "bot-1".to_owned(),
            inline_query_id: preview_query_id.to_owned(),
            inline_result_id: "result-1".to_owned(),
        }),
    }
}

#[test]
fn parser_uses_utf16_ranges_for_candidates_and_bot_queries() {
    let candidate_text = "\u{1F600} @alice";
    let ChatComposerInlineContext::Candidates { query, token_range } =
        ChatComposerInlineContext::resolve(candidate_text, caret(candidate_text), &[], false, false)
    else {
        panic!("expected a mention-candidate context");
    };
    assert_eq!(query, "alice");
    assert_eq!(token_range, NSRange::new(3, 6));

    let bot_text = format!("@imagesbot {BOT_TEXT_CYRILLIC_QUERY}");
    let ChatComposerInlineContext::Bot { bot_tag, query, query_range } =
        ChatComposerInlineContext::resolve(&bot_text, caret(&bot_text), &[], false, false)
    else {
        panic!("expected an inline-bot context");
    };
    assert_eq!(bot_tag, "imagesbot");
    assert_eq!(query, BOT_TEXT_CYRILLIC_QUERY);
    assert_eq!(query_range, NSRange::new(11, 3));
}

#[test]
fn leading_selected_user_mention_does_not_become_an_inline_bot_invocation() {
    let user_text = "@alicebot hello";
    let user_mention = mention("user-1", "alicebot", NSRange::new(0, 9));
    let selected_user_context = ChatComposerInlineContext::resolve(user_text, caret(user_text), &[user_mention], false, false);
    let bot_text = "@imagesbot cats";
    let bot_context = ChatComposerInlineContext::resolve(bot_text, caret(bot_text), &[], false, false);
    let invalid_bot_text = "@pic cats";
    let invalid_bot_context = ChatComposerInlineContext::resolve(invalid_bot_text, caret(invalid_bot_text), &[], false, false);

    assert_eq!(selected_user_context, ChatComposerInlineContext::Inactive);
    assert_eq!(
        bot_context,
        ChatComposerInlineContext::Bot {
            bot_tag: "imagesbot".to_owned(),
            query: "cats".to_owned(),
            query_range: NSRange::new(11, 4),
        }
    );
    assert_eq!(invalid_bot_context, ChatComposerInlineContext::Inactive);
}

#[test]
fn replacement_preserves_utf16_caret_position() {
    let source = "\u{1F600} @al";
    let (text, selection) = ChatComposerInlineContext::replacing(NSRange::new(3, 3), source, "@alice ");

    assert_eq!(text, "\u{1F600} @alice ");
    assert_eq!(selection, NSRange::new(10, 0));
    assert_eq!(selection.location, text.encode_utf16().count());
}

#[test]
fn candidate_replacement_range_includes_token_suffix_after_caret() {
    let text = "\u{1F600} @alice rest";
    let context = ChatComposerInlineContext::resolve(text, Some(NSRange::new(6, 0)), &[], false, false);

    let ChatComposerInlineContext::Candidates { query, token_range } = context else {
        panic!("expected a mention-candidate context inside the token");
    };
    let (replaced, _) = ChatComposerInlineContext::replacing(token_range, text, "@alice");

    assert_eq!(query, "al");
    assert_eq!(token_range, NSRange::new(3, 6));
    assert_eq!(replaced, text);
}

#[test]
fn mention_entity_merges_custom_emoji_and_removes_overlapping_role_in_utf16() {
    let text = "\u{1F600} hi @alice @admins";
    let existing = r#"[
        {"type":"custom_emoji","offset":0,"length":2,"custom_emoji_id":"smile"},
        {"type":"role_mention","role_id":"overlap","offset":6,"length":6},
        {"type":"role_mention","role_id":"admins","offset":13,"length":7}
    ]"#;
    let output = entities(ChatComposerMentionEntityCodec::encode(
        text,
        &[mention("user-1", "alice", NSRange::new(6, 6))],
        Some(existing),
    ));
    let entity_type = |entity: &Map<String, Value>| entity.get("type").and_then(Value::as_str).map(str::to_owned);
    let role_id = |entity: &Map<String, Value>| entity.get("role_id").and_then(Value::as_str).map(str::to_owned);
    let Some(mention_entity) = output.iter().find(|entity| entity_type(entity).as_deref() == Some("mention")) else {
        panic!("the mention entity is present");
    };

    assert_eq!(mention_entity.get("user_id").and_then(Value::as_str), Some("user-1"));
    assert_eq!(mention_entity.get("offset").and_then(Value::as_i64), Some(6));
    assert_eq!(mention_entity.get("length").and_then(Value::as_i64), Some(6));
    assert!(output.iter().any(|entity| entity_type(entity).as_deref() == Some("custom_emoji")));
    assert!(!output.iter().any(|entity| role_id(entity).as_deref() == Some("overlap")));
    assert!(output.iter().any(|entity| role_id(entity).as_deref() == Some("admins")));
}

#[test]
fn selected_mention_keeps_one_range_and_drops_when_its_token_is_edited() {
    let selected = mention("user-1", "alice", NSRange::new(0, 6));
    let duplicate_text = "@alice and @alice";
    let encoded = entities(ChatComposerMentionEntityCodec::encode(duplicate_text, std::slice::from_ref(&selected), None));

    assert_eq!(encoded.len(), 1);
    assert_eq!(encoded.first().and_then(|entity| entity.get("offset")).and_then(Value::as_i64), Some(0));

    let moved = ChatComposerMentionEntityCodec::rebased(&[selected], "@alice and @alice", "\u{1F600} @alice and @alice");
    assert_eq!(moved.first().map(|mention| mention.range), Some(NSRange::new(3, 6)));

    let removed =
        ChatComposerMentionEntityCodec::rebased(&moved, "\u{1F600} @alice and @alice", "\u{1F600} @alicia and @alice");
    assert!(removed.is_empty());
}

#[test]
fn selected_mention_range_survives_composer_trimming() {
    let mentions = [mention("user-1", "alice", NSRange::new(3, 6))];

    let trimmed = ChatComposerMentionEntityCodec::rebased_for_trimming(&mentions, "  \n@alice  ");

    assert_eq!(trimmed.first().map(|mention| mention.range), Some(NSRange::new(0, 6)));
}

#[test]
fn slash_command_waits_for_an_argument() {
    assert_eq!(ChatComposerInlineContext::command_awaiting_argument("/search ").as_deref(), Some("/search"));
    assert_eq!(ChatComposerInlineContext::command_awaiting_argument(" /search\n").as_deref(), Some("/search"));
    assert_eq!(ChatComposerInlineContext::command_awaiting_argument("/search cats"), None);
    assert_eq!(ChatComposerInlineContext::command_awaiting_argument("cats"), None);
    assert_eq!(ChatComposerInlineContext::command_awaiting_argument("/"), None);
}

#[test]
fn result_owned_query_id_wins_over_merged_page_fallback() {
    let explicit = make_result("result-query", "preview-query", "");
    let preview = make_result("", "preview-query", "");
    let fallback = make_result("", "", "");

    assert_eq!(explicit.effective_query_id("latest-page"), "result-query");
    assert_eq!(preview.effective_query_id("latest-page"), "preview-query");
    assert_eq!(fallback.effective_query_id("latest-page"), "latest-page");
}

#[gpui::test]
fn optimistic_inline_metadata_survives_authoritative_acknowledgement(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let result = make_result(
            "query-1",
            "query-1",
            r#"{"inline_keyboard":[[{"text":"Open","url":"https://riseonly.net"}]]}"#,
        );
        let body = result.optimistic_message_body(Some("topic-1"));

        assert_eq!(body.get("via_bot").and_then(Value::as_str), Some("bot-1"));
        assert_eq!(body.get("is_bot_message").and_then(Value::as_bool), Some(true));
        assert_eq!(body.get("topic_id").and_then(Value::as_str), Some("topic-1"));
        assert!(body.get("reply_markup").and_then(Value::as_str).is_some_and(|markup| markup.contains("inline_keyboard")));

        let service = MessageServicesStore::shared(cx);
        let outbound = Value::Object(body);
        let optimistic_row = service.update(cx, |service, cx| {
            service.create_optimistic_message_data(&outbound, "chat-1", "temp-request", &[], &[], &[], cx)
        });
        let optimistic = service.read(cx).map_optimistic_temp(&optimistic_row, "chat-1", &[], &[], &[]);
        assert_eq!(optimistic.topic_id.as_deref(), Some("topic-1"));
        assert_eq!(optimistic.via_bot.as_deref(), Some("bot-1"));
        assert!(optimistic.is_bot_message);
        assert_eq!(
            optimistic
                .reply_markup
                .as_ref()
                .and_then(|markup| markup.inline_keyboard.first())
                .and_then(|row| row.first())
                .map(|button| button.text.as_str()),
            Some("Open")
        );

        let mut kept = GetMessageMessage::new("temp-request".to_owned());
        let mut real = GetMessageMessage::new("server-message".to_owned());
        real.server_id = Some("server-message".to_owned());
        real.topic_id = Some("topic-1".to_owned());
        real.entities = Some(r#"[{"type":"bold","offset":0,"length":6}]"#.to_owned());
        real.reply_to = Some(MessageReplyTo {
            message_id: "reply-1".to_owned(),
            sender_id: "user-1".to_owned(),
            sender_name: "Alice".to_owned(),
            ..MessageReplyTo::default()
        });
        real.reply_markup = Some(MessageBotReplyMarkup {
            inline_keyboard: vec![vec![MessageBotKeyboardButton {
                text: "Open".to_owned(),
                url: Some("https://riseonly.net".to_owned()),
                ..MessageBotKeyboardButton::default()
            }]],
            ..MessageBotReplyMarkup::default()
        });
        real.via_bot = Some("bot-1".to_owned());
        real.is_bot_message = true;

        MessageServicesStore::merge_real_message_fields(&real, &mut kept, "temp-request");

        assert_eq!(kept.server_id.as_deref(), Some("server-message"));
        assert_eq!(kept.topic_id.as_deref(), Some("topic-1"));
        assert_eq!(kept.reply_to.as_ref().map(|reply| reply.message_id.as_str()), Some("reply-1"));
        assert_eq!(
            kept.reply_markup
                .as_ref()
                .and_then(|markup| markup.inline_keyboard.first())
                .and_then(|row| row.first())
                .map(|button| button.text.as_str()),
            Some("Open")
        );
        assert_eq!(kept.via_bot.as_deref(), Some("bot-1"));
        assert!(kept.is_bot_message);
        assert_eq!(kept.entities, real.entities);
    });
}

#[test]
fn panel_height_clamps_and_expands_to_available_space() {
    assert_eq!(
        ChatInlineComposerPanelMetrics::preferred_collapsed_height(
            &ChatComposerInlineContext::Candidates { query: String::new(), token_range: NSRange::new(0, 1) },
            1,
            0,
            0,
            &[],
        ),
        48.0
    );
    assert_eq!(
        ChatInlineComposerPanelMetrics::preferred_collapsed_height(
            &ChatComposerInlineContext::Bot {
                bot_tag: "imagesbot".to_owned(),
                query: String::new(),
                query_range: NSRange::new(11, 0),
            },
            0,
            0,
            1,
            &[],
        ),
        52.0
    );
    assert_eq!(collapsed_height(800.0, false), ChatInlineComposerPanelMetrics::COLLAPSED_HEIGHT);
    assert_eq!(collapsed_height(800.0, true), 800.0);
    assert_eq!(collapsed_height(120.0, false), 120.0);
    assert_eq!(collapsed_height(-1.0, true), 0.0);
}

#[test]
fn inline_overlay_never_changes_feed_inset_and_picker_keeps_inline_only_controls() {
    let collapsed = collapsed_height(720.0, false);
    let expanded = collapsed_height(720.0, true);
    let collapsed_feed_inset = ChatInlineComposerPresentationContract::feed_bottom_inset(80.0, FEED_INSET_SPACING);
    let expanded_feed_inset = ChatInlineComposerPresentationContract::feed_bottom_inset(80.0, FEED_INSET_SPACING);
    let controls_active = ChatInlineComposerPresentationContract::inline_controls_are_active(true, true);

    assert_eq!(expanded, 720.0);
    assert!(expanded > collapsed);
    assert_eq!(collapsed_feed_inset, 77.0);
    assert_eq!(expanded_feed_inset, collapsed_feed_inset);
    assert!(controls_active);
    assert!(!ChatInlineComposerPresentationContract::panel_is_visible(controls_active, true));
    assert!(ChatInlineComposerPresentationContract::should_auto_load_empty_page(
        0,
        true,
        Some("next-1"),
        0,
        EMPTY_PAGE_MAXIMUM_ATTEMPTS,
    ));
    assert!(!ChatInlineComposerPresentationContract::should_auto_load_empty_page(
        0,
        true,
        Some("next-4"),
        3,
        EMPTY_PAGE_MAXIMUM_ATTEMPTS,
    ));
    assert!(ChatInlineComposerPresentationContract::requires_manual_empty_page_continuation(
        0,
        true,
        Some("next-4"),
        3,
        EMPTY_PAGE_MAXIMUM_ATTEMPTS,
    ));
    assert!(!ChatInlineComposerPresentationContract::requires_manual_empty_page_continuation(
        1,
        true,
        Some("next-4"),
        3,
        EMPTY_PAGE_MAXIMUM_ATTEMPTS,
    ));
}
