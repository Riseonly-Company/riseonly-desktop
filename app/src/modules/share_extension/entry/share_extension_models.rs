use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use rise_core::foundation::foundation_text::{characters, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use serde::{Deserialize, Serialize};

use crate::modules::share_extension::entry::share_extension_strings::ShareStrings;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareAttachmentKind {
    Image,
    Video,
    Audio,
    File,
    Link,
    Text,
}

impl ShareAttachmentKind {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::File => "file",
            Self::Link => "link",
            Self::Text => "text",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareAttachment {
    pub id: Uuid,
    pub kind: ShareAttachmentKind,
    pub content_type_identifier: String,
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_file_name: Option<String>,
    #[serde(rename = "localFileURL", default, skip_serializing_if = "Option::is_none")]
    pub local_file_url: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_count: Option<i64>,
    pub source_index: i64,
}

impl ShareAttachment {
    /// builds an attachment, pass Uuid new_v4 for id and None for the optionals
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Uuid,
        kind: ShareAttachmentKind,
        content_type_identifier: String,
        mime_type: String,
        original_file_name: Option<String>,
        local_file_url: Option<PathBuf>,
        text: Option<String>,
        byte_count: Option<i64>,
        source_index: i64,
    ) -> Self {
        Self {
            id,
            kind,
            content_type_identifier,
            mime_type,
            original_file_name,
            local_file_url,
            text,
            byte_count,
            source_index,
        }
    }

    pub fn is_file_backed(&self) -> bool {
        self.local_file_url.is_some()
    }
}

/// what is being shared
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SharePayload {
    pub session_id: Uuid,
    pub session_directory_url: PathBuf,
    pub attachments: Vec<ShareAttachment>,
}

impl SharePayload {
    const STORY_MAXIMUM_ITEM_COUNT: usize = 10;
    const STORY_MAXIMUM_ITEM_BYTES: i64 = 2_147_483_648;

    pub fn can_share_to_story(&self) -> bool {
        !self.attachments.is_empty()
            && self.attachments.len() <= Self::STORY_MAXIMUM_ITEM_COUNT
            && self.attachments.iter().all(|item| {
                let byte_count = item.byte_count.unwrap_or(0);
                (item.kind == ShareAttachmentKind::Image || item.kind == ShareAttachmentKind::Video)
                    && byte_count > 0
                    && byte_count <= Self::STORY_MAXIMUM_ITEM_BYTES
            })
    }

    pub fn has_media(&self) -> bool {
        self.attachments.iter().any(|item| {
            matches!(
                item.kind,
                ShareAttachmentKind::Image | ShareAttachmentKind::Video | ShareAttachmentKind::Audio
            )
        })
    }

    pub fn has_documents(&self) -> bool {
        self.attachments.iter().any(|item| item.kind == ShareAttachmentKind::File)
    }

    pub fn has_link(&self) -> bool {
        self.attachments.iter().any(|item| item.kind == ShareAttachmentKind::Link)
    }

    pub fn file_attachments(&self) -> Vec<ShareAttachment> {
        self.attachments.iter().filter(|item| item.is_file_backed()).cloned().collect()
    }

    pub fn textual_content(&self) -> String {
        self.attachments
            .iter()
            .filter(|item| item.kind == ShareAttachmentKind::Text || item.kind == ShareAttachmentKind::Link)
            .filter_map(|item| item.text.as_deref())
            .map(trimming_whitespaces_and_newlines)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn requires_containing_app_upload(&self, foreground_byte_cap: i64) -> bool {
        let file_attachments = self.file_attachments();
        if file_attachments.is_empty() {
            return false;
        }
        let mut total: i64 = 0;
        for attachment in &file_attachments {
            let Some(byte_count) = attachment.byte_count.filter(|count| *count > 0) else {
                return true;
            };
            let Some(next) = total.checked_add(byte_count) else {
                return true;
            };
            if byte_count > foreground_byte_cap || next > foreground_byte_cap {
                return true;
            }
            total = next;
        }
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareDestinationKind {
    Saved,
    Direct,
    Group,
    Channel,
}

impl ShareDestinationKind {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Saved => "saved",
            Self::Direct => "direct",
            Self::Group => "group",
            Self::Channel => "channel",
        }
    }
}

/// the self chat comes titled after its only member but always shows as Saved
pub enum ShareDestinationTitle {}

impl ShareDestinationTitle {
    pub fn resolve(kind: ShareDestinationKind, raw_title: &str) -> String {
        if kind == ShareDestinationKind::Saved {
            return ShareStrings::saved_messages();
        }
        let trimmed = trimming_whitespaces_and_newlines(raw_title);
        if trimmed.is_empty() {
            ShareStrings::untitled_chat()
        } else {
            trimmed.to_owned()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareAccountSnapshot {
    pub id: String,
    pub display_name: String,
    pub tag: String,
    #[serde(rename = "avatarURL", default)]
    pub avatar_url: Option<Url>,
}

/// one place you can share to
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareDestination {
    pub id: String,
    pub kind: ShareDestinationKind,
    pub title: String,
    #[serde(rename = "avatarURL", default)]
    pub avatar_url: Option<Url>,
    #[serde(default)]
    pub avatar_color_key: Option<String>,
    pub has_topics: bool,
    pub is_writable: bool,
    pub last_activity_at_milliseconds: i64,
    pub sort_index: i64,
}

impl ShareDestination {
    /// builds a destination, pass None for avatar_color_key
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        kind: ShareDestinationKind,
        title: String,
        avatar_url: Option<Url>,
        avatar_color_key: Option<String>,
        has_topics: bool,
        is_writable: bool,
        last_activity_at_milliseconds: i64,
        sort_index: i64,
    ) -> Self {
        Self {
            id,
            kind,
            title,
            avatar_url,
            avatar_color_key,
            has_topics,
            is_writable,
            last_activity_at_milliseconds,
            sort_index,
        }
    }

    pub fn initials(&self) -> String {
        let trimmed = trimming_whitespaces_and_newlines(&self.title);
        characters(trimmed)
            .next()
            .map(str::to_uppercase)
            .unwrap_or_else(|| "?".to_owned())
    }
}

/// topic of a chat you can share into
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareTopic {
    pub id: String,
    #[serde(rename = "destinationID")]
    pub destination_id: String,
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(rename = "sectionID", default)]
    pub section_id: Option<String>,
    pub is_general: bool,
    pub is_writable: bool,
    pub sort_index: i64,
}

impl ShareTopic {
    const GENERAL_SELECTION_ID: &'static str = "general";

    pub fn selection_id(&self) -> String {
        if self.id.is_empty() {
            Self::GENERAL_SELECTION_ID.to_owned()
        } else {
            self.id.clone()
        }
    }
}

/// chosen destination and topic
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareSelection {
    pub destination: ShareDestination,
    #[serde(default)]
    pub topic: Option<ShareTopic>,
}

impl ShareSelection {
    pub fn id(&self) -> String {
        let Some(topic) = &self.topic else {
            return self.destination.id.clone();
        };
        format!("{}#{}", self.destination.id, topic.selection_id())
    }

    pub fn wire_topic_id(&self) -> Option<&str> {
        let topic = self.topic.as_ref()?;
        if topic.id.is_empty() {
            return None;
        }
        Some(topic.id.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShareSendProgress {
    pub fraction_completed: f64,
}

impl ShareSendProgress {
    pub fn new(fraction_completed: f64) -> Self {
        Self {
            fraction_completed: fraction_completed.max(0.0).min(1.0),
        }
    }
}

impl Eq for ShareSendProgress {}

impl Hash for ShareSendProgress {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.fraction_completed.to_bits().hash(state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShareSendRequest {
    pub idempotency_key: Uuid,
    pub payload: SharePayload,
    pub selections: Vec<ShareSelection>,
    pub comment: String,
}

impl ShareSendRequest {
    /// builds a send request, pass a fresh Uuid new_v4 as the idempotency key
    pub fn new(
        idempotency_key: Uuid,
        payload: SharePayload,
        selections: Vec<ShareSelection>,
        comment: String,
    ) -> Self {
        Self {
            idempotency_key,
            payload,
            selections,
            comment,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ShareExtensionError {
    ConfigurationMissing,
    AppGroupUnavailable,
    SnapshotUnavailable,
    SnapshotInvalid,
    AuthenticationUnavailable,
    AccountChanged,
    UnsupportedContent,
    TooManyItems(i64),
    FileTooLarge,
    FileUnavailable,
    ContentTooLong(i64),
    CommentTooLong(i64),
    TooManyDestinations(i64),
    InvalidServerResponse,
    RequestTimedOut,
    SessionExpired,
    SendFailed(String),
    PartialFailure(String),
    StoryUnavailable,
    AppOpenFailed,
}

impl ShareExtensionError {
    pub fn error_description(&self) -> Option<String> {
        Some(match self {
            Self::ConfigurationMissing => ShareStrings::configuration_missing(),
            Self::AppGroupUnavailable => ShareStrings::app_group_unavailable(),
            Self::SnapshotUnavailable => ShareStrings::snapshot_unavailable(),
            Self::SnapshotInvalid => ShareStrings::snapshot_invalid(),
            Self::AuthenticationUnavailable => ShareStrings::authentication_unavailable(),
            Self::AccountChanged => ShareStrings::account_changed(),
            Self::UnsupportedContent => ShareStrings::unsupported_content(),
            Self::TooManyItems(maximum) => ShareStrings::too_many_items(*maximum),
            Self::FileTooLarge => ShareStrings::file_too_large(),
            Self::FileUnavailable => ShareStrings::file_unavailable(),
            Self::ContentTooLong(maximum) => ShareStrings::content_too_long(*maximum),
            Self::CommentTooLong(maximum) => ShareStrings::comment_too_long(*maximum),
            Self::TooManyDestinations(maximum) => ShareStrings::too_many_destinations(*maximum),
            Self::InvalidServerResponse => ShareStrings::invalid_server_response(),
            Self::RequestTimedOut => ShareStrings::request_timed_out(),
            Self::SessionExpired => ShareStrings::session_expired(),
            Self::SendFailed(message) | Self::PartialFailure(message) => {
                if message.is_empty() {
                    ShareStrings::send_failed()
                } else {
                    message.clone()
                }
            }
            Self::StoryUnavailable => ShareStrings::story_unavailable(),
            Self::AppOpenFailed => ShareStrings::app_open_failed(),
        })
    }

    /// error text to show the user
    pub fn localized_description(&self) -> String {
        self.error_description().unwrap_or_default()
    }
}

impl fmt::Display for ShareExtensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.localized_description())
    }
}

impl std::error::Error for ShareExtensionError {}

/// error when only some targets got the content
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharePartialDeliveryError {
    pub delivered_count: i64,
    pub failed_count: i64,
    pub details: String,
}

impl SharePartialDeliveryError {
    pub fn error_description(&self) -> Option<String> {
        let summary = ShareStrings::partial_delivery(self.delivered_count, self.failed_count);
        if self.details.is_empty() {
            Some(summary)
        } else {
            Some(format!("{summary}\n{}", self.details))
        }
    }

    /// error text to show the user
    pub fn localized_description(&self) -> String {
        self.error_description().unwrap_or_default()
    }
}

impl fmt::Display for SharePartialDeliveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.localized_description())
    }
}

impl std::error::Error for SharePartialDeliveryError {}
