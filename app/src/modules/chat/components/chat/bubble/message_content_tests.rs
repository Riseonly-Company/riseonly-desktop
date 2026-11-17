use std::collections::BTreeMap;

use gpui::{hsla, size};
use rise_engine::RiseJSONValue;

use crate::modules::chat::components::chat::bubble::bubble_corners::TelegramReactionPalette;
use crate::modules::chat::components::chat::bubble::message_content::{
    MessageContent, MessageContentKind, MessageContentTerminalSection, TelegramReactionLayout, TelegramReactionMetrics,
};
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageReaction, ReactionReactor};

fn reaction_of(emoji: &str, count: i64, reacted_by_me: bool, ids: &[&str]) -> MessageReaction {
    MessageReaction {
        emoji: emoji.to_owned(),
        count,
        reacted_by_me,
        reactors: ids.iter().map(|id| reactor(id)).collect(),
    }
}

fn reactor(id: &str) -> ReactionReactor {
    ReactionReactor { user_id: id.to_owned(), name: String::new(), avatar_url: None, created_at: 0.0 }
}

fn object(entries: Vec<(&str, RiseJSONValue)>) -> BTreeMap<String, RiseJSONValue> {
    entries.into_iter().map(|(key, value)| (key.to_owned(), value)).collect()
}

fn string(value: &str) -> RiseJSONValue {
    RiseJSONValue::String(value.to_owned())
}

#[test]
fn reaction_height_estimator_accounts_for_wrapped_rows() {
    let reactions = vec![
        reaction_of("a", 12, false, &[]),
        reaction_of("b", 34, false, &[]),
        reaction_of("c", 56, false, &[]),
        reaction_of("d", 78, false, &[]),
    ];
    assert_eq!(TelegramReactionLayout::estimated_height(&reactions, 130.0, false, None, 0, false), 68.0);
    assert_eq!(TelegramReactionLayout::estimated_height(&reactions, 50.0, true, None, 0, false), 140.0);
}

#[test]
fn avatar_reaction_widths_match_telegram_source_metrics() {
    assert_eq!(TelegramReactionLayout::pill_width(&reaction_of("a", 1, false, &["user-0"]), true), 70.0);
    assert_eq!(TelegramReactionLayout::pill_width(&reaction_of("a", 2, false, &["user-0", "user-1"]), true), 84.0);
    assert_eq!(TelegramReactionLayout::pill_width(&reaction_of("a", 3, false, &["user-0", "user-1", "user-2"]), true), 96.0);
}

#[test]
fn outgoing_flow_starts_at_right_edge_and_continues_right_to_left_per_row() {
    let sizes = [size(42.0_f32, 30.0_f32), size(50.0, 30.0), size(60.0, 30.0)];
    let outgoing = TelegramReactionLayout::flow_plan(&sizes, 100.0, true);
    let incoming = TelegramReactionLayout::flow_plan(&sizes, 100.0, false);

    assert_eq!(outgoing.size, size(98.0, 68.0));
    assert_eq!(outgoing.items.iter().map(|item| item.row).collect::<Vec<_>>(), vec![0, 0, 1]);
    assert_eq!(outgoing.items.iter().map(|item| item.frame.origin.x).collect::<Vec<_>>(), vec![57.0, 1.0, 39.0]);
    assert_eq!(incoming.items.iter().map(|item| item.frame.origin.x).collect::<Vec<_>>(), vec![-1.0, 47.0, -1.0]);
}

#[test]
fn compact_count_strings_follow_format_canon() {
    assert!(TelegramReactionLayout::compact_count_string(0).is_empty());
    assert_eq!(TelegramReactionLayout::compact_count_string(999), "999");
    assert_eq!(TelegramReactionLayout::compact_count_string(1_000), "1K");
    assert_eq!(TelegramReactionLayout::compact_count_string(1_101), "1.1K");
    assert_eq!(TelegramReactionLayout::compact_count_string(1_102), "1.1K");
    assert_eq!(TelegramReactionLayout::compact_count_string(1_999), "2K");
    assert_eq!(TelegramReactionLayout::compact_count_string(1_000_000), "1M");
    assert_eq!(TelegramReactionLayout::compact_count_string(1_100_000), "1.1M");
    assert_eq!(TelegramReactionLayout::compact_count_string(1_000_000_000), "1000M");
    assert_eq!(TelegramReactionLayout::compact_count_string(1_100_000_000), "1100M");
}

#[test]
fn current_user_leads_bounded_avatar_stack_without_reordering_others() {
    let reactors = vec![reactor("first"), reactor("second"), reactor("third"), reactor("me")];
    let ids = |current: Option<&str>| {
        TelegramReactionLayout::displayed_reactors(&reactors, current).into_iter().map(|reactor| reactor.user_id).collect::<Vec<_>>()
    };
    assert_eq!(ids(Some("me")), vec!["me", "first", "second"]);
    assert_eq!(ids(None), vec!["first", "second", "third"]);
    assert_eq!(TelegramReactionMetrics::AVATAR_OVERLAP, 10.0);
}

#[test]
fn avatars_require_complete_deduplicated_reaction_coverage() {
    let shows = |reaction: MessageReaction, current: Option<&str>| TelegramReactionLayout::shows_complete_avatar_set(&[reaction], current);
    assert!(shows(reaction_of("a", 2, false, &["one", "two"]), None));
    assert!(!shows(reaction_of("a", 2, false, &["one"]), None));
    assert!(!shows(reaction_of("a", 2, false, &["one", "one"]), None));
    assert!(shows(reaction_of("a", 2, true, &["other", "me"]), Some("me")));
    assert!(!shows(reaction_of("a", 2, true, &["one", "two"]), Some("me")));
    assert!(!shows(reaction_of("a", 2, false, &["other", "me"]), Some("me")));
    assert!(!shows(reaction_of("a", 4, false, &["1", "2", "3", "4"]), None));
}

#[test]
fn reaction_palette_uses_theme_background_luminance() {
    assert!(TelegramReactionPalette::is_dark_background(hsla(0.0, 0.0, 0.0, 1.0)));
    assert!(!TelegramReactionPalette::is_dark_background(hsla(0.0, 0.0, 1.0, 1.0)));
    assert!(TelegramReactionPalette::is_dark_background(hsla(0.0, 0.0, 0.2, 1.0)));
    assert!(!TelegramReactionPalette::is_dark_background(hsla(0.0, 0.0, 0.9, 1.0)));
}

#[test]
fn link_preview_remains_inside_padded_bubble_content() {
    for kind in MessageContentKind::ALL_CASES.into_iter().filter(|kind| kind.needs_bubble_padding()) {
        assert_eq!(
            MessageContent::terminal_sections(kind, true, false, false),
            vec![MessageContentTerminalSection::LinkPreview, MessageContentTerminalSection::Meta]
        );
    }
}

#[test]
fn chromeless_content_does_not_embed_accessory_sections() {
    for kind in MessageContentKind::ALL_CASES.into_iter().filter(|kind| !kind.needs_bubble_padding()) {
        assert!(MessageContent::terminal_sections(kind, true, false, false).is_empty());
    }
}

#[test]
fn terminal_sections_are_absent_without_link_preview() {
    for kind in MessageContentKind::ALL_CASES {
        assert!(MessageContent::terminal_sections(kind, false, false, false).is_empty());
    }
}

#[test]
fn reactions_stay_in_the_terminal_message_content_section() {
    for kind in MessageContentKind::ALL_CASES {
        let expected = if kind == MessageContentKind::Text {
            vec![MessageContentTerminalSection::LinkPreview, MessageContentTerminalSection::Reactions]
        } else if kind.needs_bubble_padding() {
            vec![
                MessageContentTerminalSection::LinkPreview,
                MessageContentTerminalSection::Meta,
                MessageContentTerminalSection::Reactions,
            ]
        } else if kind == MessageContentKind::Call {
            Vec::new()
        } else {
            vec![MessageContentTerminalSection::Reactions]
        };
        assert_eq!(MessageContent::terminal_sections(kind, true, true, false), expected);
    }
}

#[test]
fn fourth_optimistic_reaction_evicts_the_oldest_own_reaction() {
    let account_id = "me";
    let existing = RiseMessageWireCodec::wire(object(vec![
        ("id", string("message")),
        ("chat_id", string("chat")),
        (
            "reactions",
            RiseJSONValue::Array(vec![reaction_summary("a", 2, true), reaction_summary("b", 1, true), reaction_summary("c", 1, true)]),
        ),
        (
            "reacted_by",
            RiseJSONValue::Array(vec![
                reacted_by("a", account_id, 1_000),
                reacted_by("a", "other", 1_500),
                reacted_by("b", account_id, 2_000),
                reacted_by("c", account_id, 3_000),
            ]),
        ),
    ]))
    .expect("wire");

    let payload = object(vec![("reaction", string("d")), ("user_id", string(account_id))]);
    let updated = RiseMessageWireCodec::applying_reaction(&existing, &payload, account_id, true, Some(3)).expect("reaction");

    let reactions = updated.reactions.clone().unwrap_or_default();
    assert_eq!(reactions.len(), 4);
    let first = reactions.iter().find(|summary| summary.reaction == "a").expect("first");
    assert_eq!(first.count, 1);
    assert_ne!(first.reacted_by_you, Some(true));
    let added = reactions.iter().find(|summary| summary.reaction == "d").expect("added");
    assert_eq!(added.reacted_by_you, Some(true));
    assert_eq!(reactions.iter().filter(|summary| summary.reacted_by_you == Some(true)).count(), 3);

    let by_account = |emoji: &str| {
        updated.reacted_by.as_ref().is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry.reaction.as_deref() == Some(emoji) && entry.sender.as_ref().and_then(|sender| sender.id.as_deref()) == Some(account_id)
            })
        })
    };
    assert!(by_account("d"));
    assert!(!by_account("a"));
}

fn reaction_summary(reaction: &str, count: i64, mine: bool) -> RiseJSONValue {
    RiseJSONValue::Object(object(vec![
        ("reaction", string(reaction)),
        ("count", RiseJSONValue::Integer(count)),
        ("reacted_by_you", RiseJSONValue::Bool(mine)),
    ]))
}

fn reacted_by(reaction: &str, user_id: &str, created_at: i64) -> RiseJSONValue {
    RiseJSONValue::Object(object(vec![
        ("reaction", string(reaction)),
        ("created_at", RiseJSONValue::Integer(created_at)),
        ("sender", RiseJSONValue::Object(object(vec![("id", string(user_id))]))),
        ("chat_id", string("chat")),
    ]))
}
