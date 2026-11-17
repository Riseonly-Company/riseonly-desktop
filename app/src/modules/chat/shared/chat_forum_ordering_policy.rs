use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use rise_canon::RiseLimits;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatTopic;

/// where a topic lands after a drag
/// empty section_id means no section, position counts across the whole chat
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatForumTopicPlacement {
    pub topic_id: String,
    pub section_id: String,
    pub position: i64,
}

/// pure part of a reorder, what changed and how many rows may be touched
/// kept out of the view so it can be tested
pub enum ChatForumOrderingPolicy {}

static MAXIMUM_REORDER_IDS: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("max_reorder_ids"));

impl ChatForumOrderingPolicy {
    /// gateway cap on the reorder list, also bounds a batch of update_topic calls
    pub fn maximum_reorder_ids() -> i64 {
        *MAXIMUM_REORDER_IDS
    }

    /// rows that really moved
    /// same section and position is not sent, it would cost a call and an event per member
    /// cap is not applied here, truncating would accept half a reorder
    pub fn changed_placements(
        placements: &[ChatForumTopicPlacement],
        current: &HashMap<String, ChatTopic>,
    ) -> Vec<ChatForumTopicPlacement> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut changed: Vec<ChatForumTopicPlacement> = Vec::new();
        for placement in placements {
            if !seen.insert(placement.topic_id.as_str()) {
                continue;
            }
            let Some(topic) = current.get(&placement.topic_id) else {
                continue;
            };
            if topic.section_id == placement.section_id && topic.position == placement.position {
                continue;
            }
            changed.push(placement.clone());
        }
        changed
    }
}

/// flat order for reordering, a section header is a row like a topic so dragging across it
/// moves a topic to another section
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ForumOrderEntry {
    SectionHeader(String),
    Topic(String),
}

impl ForumOrderEntry {
    pub fn id(&self) -> String {
        match self {
            Self::SectionHeader(section_id) => format!("section:{section_id}"),
            Self::Topic(topic_id) => format!("topic:{topic_id}"),
        }
    }

    /// positions run across the whole flat list and the section is the nearest header above
    /// topic order is one field per chat
    pub fn placements(entries: &[ForumOrderEntry]) -> Vec<ChatForumTopicPlacement> {
        let mut current_section = String::new();
        let mut placements: Vec<ChatForumTopicPlacement> = Vec::new();
        for entry in entries {
            match entry {
                Self::SectionHeader(section_id) => {
                    current_section.clone_from(section_id);
                }
                Self::Topic(topic_id) => {
                    let position = placements.len() as i64;
                    placements.push(ChatForumTopicPlacement {
                        topic_id: topic_id.clone(),
                        section_id: current_section.clone(),
                        position,
                    });
                }
            }
        }
        placements
    }
}
