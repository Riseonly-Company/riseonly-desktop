use std::collections::HashMap;

use rise_canon::RiseLimits;

use super::chat_forum_ordering_policy::{ChatForumOrderingPolicy, ChatForumTopicPlacement, ForumOrderEntry};
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatChannelKind, ChatTopic};

fn topic(id: &str, section: &str, position: i64, kind: ChatChannelKind) -> ChatTopic {
    ChatTopic {
        id: id.to_owned(),
        title: id.to_owned(),
        icon: ChatTopic::DEFAULT_ICON.to_owned(),
        position,
        is_general: false,
        is_closed: false,
        section_id: section.to_owned(),
        channel_kind: kind,
        ..ChatTopic::default()
    }
}

fn placement(topic_id: &str, section_id: &str, position: i64) -> ChatForumTopicPlacement {
    ChatForumTopicPlacement { topic_id: topic_id.to_owned(), section_id: section_id.to_owned(), position }
}

fn ids(placements: &[ChatForumTopicPlacement]) -> Vec<&str> {
    placements.iter().map(|placement| placement.topic_id.as_str()).collect()
}

#[test]
fn dragging_past_a_section_header_reparents_the_topic() {
    let entries = vec![
        ForumOrderEntry::SectionHeader(String::new()),
        ForumOrderEntry::SectionHeader("s1".to_owned()),
        ForumOrderEntry::Topic("a".to_owned()),
        ForumOrderEntry::Topic("b".to_owned()),
    ];

    let placements = ForumOrderEntry::placements(&entries);
    assert_eq!(placements, vec![placement("a", "s1", 0), placement("b", "s1", 1)]);

    let mut moved = entries;
    moved.swap(1, 2);
    assert_eq!(ForumOrderEntry::placements(&moved), vec![placement("a", "", 0), placement("b", "s1", 1)]);
}

#[test]
fn only_moved_rows_reach_the_wire() {
    let current = HashMap::from([
        ("a".to_owned(), topic("a", "", 0, ChatChannelKind::Text)),
        ("b".to_owned(), topic("b", "s1", 1, ChatChannelKind::Text)),
        ("c".to_owned(), topic("c", "s1", 2, ChatChannelKind::Text)),
    ]);
    let placements = [placement("a", "", 0), placement("c", "s1", 1), placement("b", "s1", 2)];

    let changed = ChatForumOrderingPolicy::changed_placements(&placements, &current);
    assert_eq!(ids(&changed), vec!["c", "b"]);
}

#[test]
fn moving_out_of_a_section_is_a_change_even_at_the_same_position() {
    let current = HashMap::from([("a".to_owned(), topic("a", "s1", 3, ChatChannelKind::Text))]);
    let changed = ChatForumOrderingPolicy::changed_placements(&[placement("a", "", 3)], &current);

    assert_eq!(changed.len(), 1);
    assert_eq!(changed.first().map(|placement| placement.section_id.is_empty()), Some(true));
}

#[test]
fn unknown_and_duplicated_rows_are_dropped() {
    let current = HashMap::from([("a".to_owned(), topic("a", "", 5, ChatChannelKind::Text))]);
    let changed = ChatForumOrderingPolicy::changed_placements(
        &[placement("a", "", 0), placement("a", "s1", 1), placement("ghost", "", 2)],
        &current,
    );

    assert_eq!(ids(&changed), vec!["a"]);
    assert_eq!(changed.first().map(|placement| placement.position), Some(0));
}

#[test]
fn reorder_ceiling_comes_from_canon() {
    assert_eq!(ChatForumOrderingPolicy::maximum_reorder_ids(), RiseLimits::service_value("max_reorder_ids"));
}
