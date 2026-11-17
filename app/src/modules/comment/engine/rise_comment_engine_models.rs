use std::fmt;

use serde::{Deserialize, Serialize};

use crate::core::lib::canon_errors::RiseFailure;
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatDraftCustomEmoji;
use crate::modules::comment::stores::comment_actions::comment_types::{
    CommentEntities, CommentMedia, CommentMediaType, CommentSortType, GetCommentsResponse, RepliesSortType,
    UserCommentEntry, UserCommentPostPreview,
};
use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, GetPostFeedResponse};

/// one list of comments as the server pages it
/// comments of a post, replies of a thread root or everything one user wrote
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RiseCommentScope {
    Post { post_id: String, sort: CommentSortType },
    Replies { root_id: String, sort: RepliesSortType },
    User { user_id: String },
}

impl RiseCommentScope {
    /// length prefixed so an id with a separator cant collide with another scope key
    pub fn storage_key(&self) -> String {
        let parts: Vec<&str> = match self {
            Self::Post { post_id, sort } => vec!["post", post_id.as_str(), sort.raw_value()],
            Self::Replies { root_id, sort } => vec!["replies", root_id.as_str(), sort.raw_value()],
            Self::User { user_id } => vec!["user", user_id.as_str()],
        };
        parts.iter().map(|part| format!("{}:{}", part.len(), part)).collect::<Vec<_>>().join("|")
    }

    /// a user comments stay in memory only, the page carries whole post cards and is rebuilt
    /// cheaply from one request
    pub fn is_persisted(&self) -> bool {
        match self {
            Self::Post { .. } | Self::Replies { .. } => true,
            Self::User { .. } => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseCommentLoadState {
    Idle,
    Loading,
    Loaded,
    Failed,
}

impl RiseCommentLoadState {
    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Loading => "loading",
            Self::Loaded => "loaded",
            Self::Failed => "failed",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "idle" => Some(Self::Idle),
            "loading" => Some(Self::Loading),
            "loaded" => Some(Self::Loaded),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseCommentCacheSource {
    Miss,
    Memory,
    PersistentStore,
    Network,
}

impl RiseCommentCacheSource {
    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Miss => "miss",
            Self::Memory => "memory",
            Self::PersistentStore => "persistent-store",
            Self::Network => "network",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "miss" => Some(Self::Miss),
            "memory" => Some(Self::Memory),
            "persistent-store" => Some(Self::PersistentStore),
            "network" => Some(Self::Network),
            _ => None,
        }
    }
}

/// what the profile tab draws around a comment, the post it was left on and the thread root
/// a reply answers
/// comments live in the repository entity table so a like lands on every list at once
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RiseCommentUserEntryContext {
    pub post: Option<UserCommentPostPreview>,
    pub post_card: Option<GetPostFeedResponse>,
    pub parent_id: Option<String>,
}

impl Eq for RiseCommentUserEntryContext {}

impl std::hash::Hash for RiseCommentUserEntryContext {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.post.hash(state);
        self.parent_id.hash(state);
    }
}

/// immutable, prepared off the main thread and published in one commit
#[derive(Clone, Debug, PartialEq)]
pub struct RiseCommentListSnapshot {
    pub scope: RiseCommentScope,
    pub items: Vec<GetCommentsResponse>,
    pub user_entries: Vec<UserCommentEntry>,
    pub has_more: bool,
    pub is_loading_more: bool,
    pub is_refreshing: bool,
    pub is_private: bool,
    pub load_state: RiseCommentLoadState,
    pub error_text: Option<String>,
    pub cache_source: RiseCommentCacheSource,
    pub revision: u64,
}

impl RiseCommentListSnapshot {
    pub fn empty(scope: RiseCommentScope) -> RiseCommentListSnapshot {
        RiseCommentListSnapshot {
            scope,
            items: Vec::new(),
            user_entries: Vec::new(),
            has_more: false,
            is_loading_more: false,
            is_refreshing: false,
            is_private: false,
            load_state: RiseCommentLoadState::Idle,
            error_text: None,
            cache_source: RiseCommentCacheSource::Miss,
            revision: 0,
        }
    }

    pub fn shows_initial_loader(&self) -> bool {
        self.items.is_empty()
            && self.user_entries.is_empty()
            && (self.load_state == RiseCommentLoadState::Idle || self.load_state == RiseCommentLoadState::Loading)
    }

    pub fn shows_blocking_error(&self) -> bool {
        self.items.is_empty() && self.user_entries.is_empty() && self.load_state == RiseCommentLoadState::Failed
    }
}

/// everything the composer captured when the row was drawn
/// repository keeps it until the server answers so a failed send can hand it back as it was
#[derive(Clone)]
pub struct RiseCommentCreateDraft {
    pub temp_id: String,
    pub post_id: String,
    pub content: String,
    pub parent_id: Option<String>,
    pub replies_sort: RepliesSortType,
    pub list_sort: CommentSortType,
    pub author: AuthorInfo,
    pub media: Option<CommentMedia>,
    pub entities: Option<String>,
    pub custom_emoji_drafts: Vec<ChatDraftCustomEmoji>,
    pub reply_target_id: Option<String>,
    pub reply_root_id: Option<String>,
    pub reply_target_tag: Option<String>,
    pub reply_target_name: Option<String>,
    pub reply_target_snippet: Option<String>,
    pub reply_target_is_official: bool,
    pub reply_target_is_premium: bool,
}

#[derive(Clone)]
pub enum RiseCommentCreateOutcome {
    Staged,
    Confirmed { temp_id: String, server_id: String },
    Withdrawn,
    Failed { draft: RiseCommentCreateDraft, refusal: Option<RiseFailure> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseCommentError {
    MissingActiveSession,
    StaleAccount,
    MissingCreateAcknowledgement,
    InvalidReactionAcknowledgement,
    Rejected(String),
}

impl RiseCommentError {
    pub fn error_description(&self) -> Option<String> {
        Some(match self {
            Self::MissingActiveSession => "No active account session.".to_owned(),
            Self::StaleAccount => "The account changed while the request was running.".to_owned(),
            Self::MissingCreateAcknowledgement => "The server did not acknowledge the comment.".to_owned(),
            Self::InvalidReactionAcknowledgement => "The server returned an invalid reaction state.".to_owned(),
            Self::Rejected(message) => message.clone(),
        })
    }
}

impl fmt::Display for RiseCommentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for RiseCommentError {}

/// persisted shape of a comment
/// the live model isnt serializable so the record stores the json only and parses again on load
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseCommentRecord {
    pub id: String,
    pub content: String,
    pub original_content: String,
    pub created_at: String,
    pub updated_at: String,
    pub post_id: String,
    pub author_id: String,
    pub parent_id: Option<String>,
    pub replies_count: i64,
    pub likes_count: i64,
    pub dislikes_count: i64,
    pub user_liked: bool,
    pub user_disliked: bool,
    pub addressed_to_name: Option<String>,
    pub addressed_to_tag: Option<String>,
    pub author: AuthorInfo,
    pub media: Option<RiseCommentMediaRecord>,
    pub entities: Option<String>,
    pub moderation_status: String,
    pub is_first: bool,
}

impl RiseCommentRecord {
    pub fn new(comment: &GetCommentsResponse) -> Self {
        Self {
            id: comment.id.clone(),
            content: comment.content.clone(),
            original_content: comment.original_content.clone(),
            created_at: comment.created_at.clone(),
            updated_at: comment.updated_at.clone(),
            post_id: comment.post_id.clone(),
            author_id: comment.author_id.clone(),
            parent_id: comment.parent_id.clone(),
            replies_count: comment.replies_count,
            likes_count: comment.likes_count,
            dislikes_count: comment.dislikes_count,
            user_liked: comment.user_liked,
            user_disliked: comment.user_disliked,
            addressed_to_name: comment.addressed_to_name.clone(),
            addressed_to_tag: comment.addressed_to_tag.clone(),
            author: comment.author.clone(),
            media: comment.media.as_ref().map(RiseCommentMediaRecord::new),
            entities: comment.entities.clone(),
            moderation_status: comment.moderation_status.clone(),
            is_first: comment.is_first,
        }
    }

    pub fn as_comment(&self) -> GetCommentsResponse {
        let (json, custom_emojis) = CommentEntities::normalized(self.entities.as_deref());
        GetCommentsResponse {
            id: self.id.clone(),
            content: self.content.clone(),
            original_content: self.original_content.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            post_id: self.post_id.clone(),
            author_id: self.author_id.clone(),
            parent_id: self.parent_id.clone(),
            replies_count: self.replies_count,
            likes_count: self.likes_count,
            dislikes_count: self.dislikes_count,
            user_liked: self.user_liked,
            user_disliked: self.user_disliked,
            addressed_to_name: self.addressed_to_name.clone(),
            addressed_to_tag: self.addressed_to_tag.clone(),
            author: self.author.clone(),
            media: self.media.as_ref().and_then(RiseCommentMediaRecord::as_media),
            entities: json,
            custom_emojis,
            is_temp: false,
            moderation_status: self.moderation_status.clone(),
            is_first: self.is_first,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseCommentMediaRecord {
    pub r#type: String,
    pub file_url: String,
    pub thumbnail_url: Option<String>,
    pub width: i64,
    pub height: i64,
    pub duration_ms: Option<i64>,
    pub sticker_id: Option<String>,
    pub sticker_type: Option<String>,
    pub pack_id: Option<String>,
    pub associated_emojis: Vec<String>,
}

impl RiseCommentMediaRecord {
    pub fn new(media: &CommentMedia) -> Self {
        Self {
            r#type: media.r#type.raw_value().to_owned(),
            file_url: media.file_url.clone(),
            thumbnail_url: media.thumbnail_url.clone(),
            width: media.width,
            height: media.height,
            duration_ms: media.duration_ms,
            sticker_id: media.sticker_id.clone(),
            sticker_type: media.sticker_type.clone(),
            pack_id: media.pack_id.clone(),
            associated_emojis: media.associated_emojis.clone(),
        }
    }

    pub fn as_media(&self) -> Option<CommentMedia> {
        let media_type = CommentMediaType::from_raw(&self.r#type)?;
        if self.file_url.is_empty() {
            return None;
        }
        Some(CommentMedia {
            r#type: media_type,
            file_url: self.file_url.clone(),
            thumbnail_url: self.thumbnail_url.clone(),
            width: self.width,
            height: self.height,
            duration_ms: self.duration_ms,
            sticker_id: self.sticker_id.clone(),
            sticker_type: self.sticker_type.clone(),
            pack_id: self.pack_id.clone(),
            associated_emojis: self.associated_emojis.clone(),
        })
    }
}
