use rise_canon::rise_key::{events, file};
use rise_canon::rise_wire_canon::key as wire_key;
use rise_canon::{rise_event_payload, rise_proto, rise_response};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_moderation::ContentStatus;
use serde_json::{Map, Value};

use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;

pub type StoryAuthorDTO = rise_proto::stories::AuthorInfo;
pub type StoryTrackPlateDTO = rise_proto::stories::StoryTrackPlate;

pub type StoryDTO = rise_proto::stories::Story;

pub trait StoryDTOExt {
    fn is_moderation_disabled(&self) -> bool;
    fn is_pinned_in_profile(&self) -> bool;
    fn is_archived(&self) -> bool;
    fn sits_in_profile_archive(&self, now_ms: i64) -> bool;
    fn was_edited(&self) -> bool;
    fn media_source(&self) -> String;
}

impl StoryDTOExt for StoryDTO {
    fn is_moderation_disabled(&self) -> bool {
        self.moderation_status == ContentStatus::Disabled.wire()
    }

    fn is_pinned_in_profile(&self) -> bool {
        self.pinned_at_ms.unwrap_or(0) > 0
    }

    fn is_archived(&self) -> bool {
        self.archived_at_ms.unwrap_or(0) > 0
    }

    fn sits_in_profile_archive(&self, now_ms: i64) -> bool {
        self.is_archived() || (!self.post_to_profile && self.expires_at_ms <= now_ms)
    }

    fn was_edited(&self) -> bool {
        self.edited_at_ms.unwrap_or(0) > 0
    }

    fn media_source(&self) -> String {
        RemoteMediaURL::optional_normalized(Some(&self.file_id)).unwrap_or_default()
    }
}

pub type StoryGroupDTO = rise_proto::stories::StoryGroup;
pub type StoryViewerDTO = rise_proto::stories::StoryViewer;
pub type HighlightDTO = rise_proto::stories::Highlight;
pub type StoriesStateDTO = rise_proto::stories::UserStoryState;
pub type CreateStoryGatewayDTO = rise_response::story::CreateStory;
pub type CreateHighlightGatewayDTO = rise_response::story::CreateHighlight;
pub type GetUserHighlightsGatewayDTO = rise_response::story::GetUserHighlights;

pub trait HighlightDTOExt {
    fn is_archived(&self) -> bool;
}

impl HighlightDTOExt for HighlightDTO {
    fn is_archived(&self) -> bool {
        self.archived_at_ms.unwrap_or(0) > 0
    }
}
const UPLOAD_LOCATION_KEY: &str = "location";
pub enum StoryUploadWire {}

impl StoryUploadWire {
    pub fn remote_file_id(done: &Map<String, Value>) -> Option<String> {
        if let Some(value) = done.get(wire_key::DATA).and_then(Value::as_object).and_then(Self::remote_file_id) {
            return Some(value);
        }
        for key in [events::FILE_URL, file::URL, UPLOAD_LOCATION_KEY] {
            let Some(raw) = done.get(key).and_then(Value::as_str) else {
                continue;
            };
            let raw = trimming_whitespaces_and_newlines(raw);
            if raw.is_empty() {
                continue;
            }
            return Some(raw.to_owned());
        }
        None
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HighlightScope {
    Profile,
    Archive,
}

impl HighlightScope {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Archive => "archive",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "profile" => Some(Self::Profile),
            "archive" => Some(Self::Archive),
            _ => None,
        }
    }
}

pub type RemoveStoryFromHighlightGatewayDTO = rise_proto::stories::RemoveStoryFromHighlightResponse;
pub type GetStoriesFeedGatewayDTO = rise_proto::stories::GetStoriesFeedResponse;

pub type GetUserStoriesGatewayDTO = rise_proto::stories::GetUserStoriesResponse;

pub type ViewStoryGatewayDTO = rise_proto::stories::ViewStoryResponse;
pub type GetStoryViewersGatewayDTO = rise_proto::stories::GetStoryViewersResponse;
pub type GetHighlightStoriesGatewayDTO = rise_proto::stories::GetHighlightStoriesResponse;
pub type GetStoryGatewayDTO = rise_proto::stories::GetStoryResponse;

pub trait StoryDTOFromCanon {
    fn from_canon(event: &rise_event_payload::StoryCreatedData) -> Self;
}

impl StoryDTOFromCanon for StoryDTO {
    fn from_canon(event: &rise_event_payload::StoryCreatedData) -> Self {
        let author = &event.author;
        Self {
            allow_screenshots: event.allow_screenshots,
            audience: event.audience.clone(),
            author: Some(rise_proto::stories::AuthorInfo {
                id: author.id.clone(),
                is_online: author.is_online,
                is_premium: author.is_premium,
                last_seen_ms: author.last_seen_ms,
                logo: author.logo.clone(),
                name: author.name.clone(),
                name_color: author.name_color.clone(),
                premium_badge_emoji: author.premium_badge_emoji.clone(),
                tag: author.tag.clone(),
                ..rise_proto::stories::AuthorInfo::default()
            }),
            author_id: event.author_id.clone(),
            caption: event.caption.clone(),
            created_at_ms: event.created_at_ms,
            duration_days: event.duration_days,
            expires_at_ms: event.expires_at_ms,
            file_id: event.file_id.clone(),
            id: event.story_id,
            media_type: event.media_type.clone(),
            track_end_ms: event.track_end_ms,
            track_id: event.track_id.clone(),
            track_plate: event.track_plate.as_ref().map(|plate| rise_proto::stories::StoryTrackPlate {
                artist: plate.artist.clone(),
                center_x: plate.center_x,
                center_y: plate.center_y,
                cover_url: plate.cover_url.clone(),
                is_dark: plate.is_dark,
                rotation: plate.rotation,
                scale: plate.scale,
                title: plate.title.clone(),
            }),
            track_start_ms: event.track_start_ms,
            ..Self::default()
        }
    }
}
