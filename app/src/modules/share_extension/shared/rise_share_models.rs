use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

/// kind of share target, raw values are the snapshot disk format
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RiseShareTargetKind {
    #[serde(rename = "private")]
    PrivateChat,
    #[serde(rename = "group")]
    Group,
    #[serde(rename = "channel")]
    Channel,
    #[serde(rename = "favourites")]
    Favourites,
}

/// account stored in the share snapshot
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseShareAccount {
    pub id: String,
    pub name: String,
    pub tag: String,
    #[serde(rename = "avatarURL", default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseShareTarget {
    pub id: String,
    pub title: String,
    #[serde(rename = "avatarURL", default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_color_key: Option<String>,
    pub kind: RiseShareTargetKind,
    pub topics_enabled: bool,
    pub can_send: bool,
    pub last_activity_ms: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_deleted: bool,
}

impl RiseShareTarget {
    /// builds a target, pass None for avatar_color_key
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        title: String,
        avatar_url: Option<String>,
        avatar_color_key: Option<String>,
        kind: RiseShareTargetKind,
        topics_enabled: bool,
        can_send: bool,
        last_activity_ms: i64,
    ) -> Self {
        Self {
            id,
            title,
            avatar_url,
            avatar_color_key,
            kind,
            topics_enabled,
            can_send,
            last_activity_ms,
            is_deleted: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseShareTopic {
    pub id: String,
    pub chat_id: String,
    pub title: String,
    pub icon: String,
    pub section_id: String,
    pub position: i64,
    pub is_general: bool,
    pub can_send: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseShareSection {
    pub id: String,
    pub chat_id: String,
    pub name: String,
    pub position: i64,
    pub created_at_ms: i64,
}

/// topics split into groups
#[derive(Clone, Debug, PartialEq)]
pub struct RiseShareTopicStructure<Topic: Send> {
    pub sections: Vec<RiseShareSection>,
    pub topics: Vec<Topic>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseShareTopicGroup<Topic> {
    pub id: String,
    pub section: Option<RiseShareSection>,
    pub topics: Vec<Topic>,
}

pub enum RiseShareTopicGrouping {}

impl RiseShareTopicGrouping {
    /// general and unsectioned topics first, then each known section by position with its topics
    pub fn groups<Topic>(
        sections: &[RiseShareSection],
        topics: Vec<Topic>,
        topic_id: impl Fn(&Topic) -> String,
        section_id: impl Fn(&Topic) -> String,
        position: impl Fn(&Topic) -> i64,
        is_general: impl Fn(&Topic) -> bool,
    ) -> Vec<RiseShareTopicGroup<Topic>> {
        let mut seen_section_ids: HashSet<String> = HashSet::new();
        let mut ordered_sections: Vec<RiseShareSection> = sections
            .iter()
            .filter(|section| !trimming_whitespaces_and_newlines(&section.id).is_empty())
            .cloned()
            .collect();
        ordered_sections.sort_by(|lhs, rhs| {
            lhs.position
                .cmp(&rhs.position)
                .then_with(|| lhs.id.cmp(&rhs.id))
        });
        ordered_sections.retain(|section| {
            seen_section_ids.insert(trimming_whitespaces_and_newlines(&section.id).to_owned())
        });
        let known_section_ids: HashSet<String> = ordered_sections
            .iter()
            .map(|section| trimming_whitespaces_and_newlines(&section.id).to_owned())
            .collect();

        let mut ordered_topics = topics;
        ordered_topics.sort_by(|lhs, rhs| {
            let lhs_general = is_general(lhs);
            let rhs_general = is_general(rhs);
            rhs_general
                .cmp(&lhs_general)
                .then_with(|| position(lhs).cmp(&position(rhs)))
                .then_with(|| topic_id(lhs).cmp(&topic_id(rhs)))
        });

        let mut unsectioned: Vec<Topic> = Vec::new();
        let mut topics_by_section_id: BTreeMap<String, Vec<Topic>> = BTreeMap::new();
        for topic in ordered_topics {
            let raw_section_id = section_id(&topic);
            let topic_section_id = trimming_whitespaces_and_newlines(&raw_section_id);
            if is_general(&topic)
                || topic_section_id.is_empty()
                || !known_section_ids.contains(topic_section_id)
            {
                unsectioned.push(topic);
            } else {
                topics_by_section_id
                    .entry(topic_section_id.to_owned())
                    .or_default()
                    .push(topic);
            }
        }

        let mut result: Vec<RiseShareTopicGroup<Topic>> = Vec::new();
        if !unsectioned.is_empty() {
            result.push(RiseShareTopicGroup {
                id: "rise-share-topics-unsectioned".to_owned(),
                section: None,
                topics: unsectioned,
            });
        }
        for section in ordered_sections {
            let normalized_section_id = trimming_whitespaces_and_newlines(&section.id).to_owned();
            let Some(section_topics) = topics_by_section_id.remove(&normalized_section_id) else {
                continue;
            };
            if section_topics.is_empty() {
                continue;
            }
            result.push(RiseShareTopicGroup {
                id: format!("rise-share-section-{normalized_section_id}"),
                section: Some(section),
                topics: section_topics,
            });
        }
        result
    }
}

/// saved share targets of the signed in account
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseShareSnapshot {
    pub schema_version: i64,
    pub generated_at_ms: i64,
    #[serde(rename = "apiBaseURL")]
    pub api_base_url: Url,
    #[serde(rename = "webSocketURL")]
    pub web_socket_url: Url,
    pub account: RiseShareAccount,
    pub targets: Vec<RiseShareTarget>,
}

impl RiseShareSnapshot {
    pub const CURRENT_SCHEMA_VERSION: i64 = 1;

    /// builds the snapshot
    pub fn new(
        generated_at_ms: i64,
        api_base_url: Url,
        web_socket_url: Url,
        account: RiseShareAccount,
        targets: Vec<RiseShareTarget>,
    ) -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            generated_at_ms,
            api_base_url,
            web_socket_url,
            account,
            targets,
        }
    }
}

/// credential used by the share layer
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseShareCredential {
    pub account_id: String,
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub session_id: String,
    pub updated_at_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiseShareStoryMediaKind {
    Image,
    Video,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseShareStoryItem {
    pub id: String,
    pub kind: RiseShareStoryMediaKind,
    pub relative_path: String,
    pub suggested_filename: String,
    pub content_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseShareStoryPublishCheckpoint {
    pub item_id: String,
    pub idempotency_key: String,
    #[serde(rename = "remoteFileURL")]
    pub remote_file_url: String,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    pub audience: String,
    pub allow_screenshots: bool,
    pub post_to_profile: bool,
    pub duration_days: i64,
    pub excluded_user_ids: Vec<String>,
    pub selected_user_ids: Vec<String>,
}

impl RiseShareStoryPublishCheckpoint {
    /// key from length prefixed fields so a retry of the same item never publishes twice
    pub fn stable_idempotency_key(account_id: &str, session_id: &str, item_id: &str) -> String {
        let material = [account_id, session_id, item_id]
            .iter()
            .map(|field| format!("{}:{}", field.len(), field))
            .collect::<Vec<_>>()
            .join("|");
        let digest = Sha256::digest(material.as_bytes());
        let mut key = String::from("rise-story-v1-");
        for byte in digest.iter() {
            let _ = write!(key, "{byte:02x}");
        }
        key
    }
}

pub enum RiseShareIntentDestinationID {}

impl RiseShareIntentDestinationID {
    /// resolves the destination id from the conversation or recipient fields
    pub fn resolve(
        conversation_identifier: Option<&str>,
        recipient_custom_identifier: Option<&str>,
        recipient_handle: Option<&str>,
    ) -> Option<String> {
        [conversation_identifier, recipient_custom_identifier, recipient_handle]
            .into_iter()
            .map(|candidate| trimming_whitespaces_and_newlines(candidate.unwrap_or("")))
            .find(|value| !value.is_empty())
            .map(str::to_owned)
    }
}

pub enum RiseShareSuggestionEligibility {}

impl RiseShareSuggestionEligibility {
    /// chat suggestion only for the snapshot account and a target that can send
    pub fn can_donate_incoming_message(
        snapshot: Option<&RiseShareSnapshot>,
        recipient_account_id: Option<&str>,
        chat_id: Option<&str>,
    ) -> bool {
        let Some(snapshot) = snapshot else { return false };
        let Some(recipient_account_id) = Self::normalized(recipient_account_id) else {
            return false;
        };
        let Some(chat_id) = Self::normalized(chat_id) else { return false };
        if Self::normalized(Some(&snapshot.account.id)) != Some(recipient_account_id) {
            return false;
        }
        snapshot
            .targets
            .iter()
            .any(|target| Self::normalized(Some(&target.id)) == Some(chat_id) && target.can_send)
    }

    fn normalized(value: Option<&str>) -> Option<&str> {
        let result = trimming_whitespaces_and_newlines(value?);
        (!result.is_empty()).then_some(result)
    }
}

/// story session on disk, older ones may lack next item index and checkpoint
/// those read as 0 and none
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseShareStorySession {
    pub schema_version: i64,
    pub id: String,
    pub account_id: String,
    pub created_at_ms: i64,
    pub items: Vec<RiseShareStoryItem>,
    #[serde(default)]
    pub next_item_index: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_checkpoint: Option<RiseShareStoryPublishCheckpoint>,
}

impl RiseShareStorySession {
    pub const CURRENT_SCHEMA_VERSION: i64 = 2;

    /// builds a story session, next item index defaults to 0 and checkpoint to none
    pub fn new(
        id: String,
        account_id: String,
        created_at_ms: i64,
        items: Vec<RiseShareStoryItem>,
        next_item_index: i64,
        publish_checkpoint: Option<RiseShareStoryPublishCheckpoint>,
    ) -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            id,
            account_id,
            created_at_ms,
            items,
            next_item_index,
            publish_checkpoint,
        }
    }
}

/// saved json value of an uploaded media payload
#[derive(Clone, Debug, PartialEq)]
pub enum RiseShareSendJSONValue {
    String(String),
    Integer(i64),
    Double(f64),
    Bool(bool),
    Array(Vec<RiseShareSendJSONValue>),
    Object(BTreeMap<String, RiseShareSendJSONValue>),
    Null,
}

impl RiseShareSendJSONValue {
    fn from_json(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(value) => Self::Bool(value),
            serde_json::Value::Number(number) => {
                if let Some(value) = number.as_i64() {
                    Self::Integer(value)
                } else if let Some(value) = number.as_f64() {
                    if value.fract() == 0.0 && value >= i64::MIN as f64 && value < i64::MAX as f64 {
                        Self::Integer(value as i64)
                    } else {
                        Self::Double(value)
                    }
                } else {
                    Self::Null
                }
            }
            serde_json::Value::String(value) => Self::String(value),
            serde_json::Value::Array(values) => {
                Self::Array(values.into_iter().map(Self::from_json).collect())
            }
            serde_json::Value::Object(map) => Self::Object(
                map.into_iter()
                    .map(|(key, value)| (key, Self::from_json(value)))
                    .collect(),
            ),
        }
    }
}

impl Serialize for RiseShareSendJSONValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::String(value) => serializer.serialize_str(value),
            Self::Integer(value) => serializer.serialize_i64(*value),
            Self::Double(value) => serializer.serialize_f64(*value),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Array(value) => value.serialize(serializer),
            Self::Object(value) => value.serialize(serializer),
            Self::Null => serializer.serialize_unit(),
        }
    }
}

impl<'de> Deserialize<'de> for RiseShareSendJSONValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if let serde_json::Value::Number(number) = &value {
            if number.as_i64().is_none() && number.as_f64().is_none() {
                return Err(D::Error::custom("Unsupported persisted share JSON value."));
            }
        }
        Ok(Self::from_json(value))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseShareSendDestination {
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic_id: Option<String>,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseShareSendAttachment {
    pub id: String,
    pub kind: String,
    pub content_type_identifier: String,
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_count: Option<i64>,
    pub source_index: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uploaded_media: Option<BTreeMap<String, RiseShareSendJSONValue>>,
}

/// durable message share that survives quitting the app
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseShareSendSession {
    pub schema_version: i64,
    pub id: String,
    pub account_id: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub idempotency_key: String,
    pub destinations: Vec<RiseShareSendDestination>,
    pub comment: String,
    pub text_content: String,
    pub auto_resume: bool,
    pub commit_started: bool,
    pub claimed_by_app: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at_ms: Option<i64>,
    pub attachments: Vec<RiseShareSendAttachment>,
}

impl RiseShareSendSession {
    pub const CURRENT_SCHEMA_VERSION: i64 = 1;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        account_id: String,
        created_at_ms: i64,
        idempotency_key: String,
        destinations: Vec<RiseShareSendDestination>,
        comment: String,
        text_content: String,
        attachments: Vec<RiseShareSendAttachment>,
    ) -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            id,
            account_id,
            created_at_ms,
            updated_at_ms: created_at_ms,
            idempotency_key,
            destinations,
            comment,
            text_content,
            auto_resume: true,
            commit_started: false,
            claimed_by_app: false,
            claimed_at_ms: None,
            attachments,
        }
    }
}
