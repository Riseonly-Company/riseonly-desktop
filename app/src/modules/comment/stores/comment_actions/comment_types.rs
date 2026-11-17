use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_moderation::ContentStatus;

use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageCustomEmojiEntity, MessageMediaItem, MessageMediaKind, MessageNormalizedEntities,
};
use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, GetPostFeedResponse};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{GifItem, StickerItem};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommentSortType {
    Feed,
    New,
    Old,
    My,
}

impl CommentSortType {
    pub const ALL: [CommentSortType; 4] =
        [CommentSortType::Feed, CommentSortType::New, CommentSortType::Old, CommentSortType::My];

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Feed => "feed",
            Self::New => "new",
            Self::Old => "old",
            Self::My => "my",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sort| sort.raw_value() == raw)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RepliesSortType {
    Popular,
    New,
    Old,
    My,
}

impl RepliesSortType {
    pub const ALL: [RepliesSortType; 4] =
        [RepliesSortType::Popular, RepliesSortType::New, RepliesSortType::Old, RepliesSortType::My];

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Popular => "popular",
            Self::New => "new",
            Self::Old => "old",
            Self::My => "my",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sort| sort.raw_value() == raw)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommentReaction {
    None,
    Like,
    Dislike,
}

impl CommentReaction {
    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Like => "like",
            Self::Dislike => "dislike",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "none" => Some(Self::None),
            "like" => Some(Self::Like),
            "dislike" => Some(Self::Dislike),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommentReactionEndpoint {
    Like,
    Dislike,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentReactionCommand {
    pub comment_id: String,
    pub endpoint: CommentReactionEndpoint,
    pub enabled: bool,
    pub generation: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommentMediaType {
    Gif,
    Sticker,
    Photo,
}

impl CommentMediaType {
    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Sticker => "sticker",
            Self::Photo => "photo",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "gif" => Some(Self::Gif),
            "sticker" => Some(Self::Sticker),
            "photo" => Some(Self::Photo),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CommentMedia {
    pub r#type: CommentMediaType,
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

impl CommentMedia {
    pub fn new(r#type: CommentMediaType, file_url: String) -> Self {
        Self {
            r#type,
            file_url,
            thumbnail_url: None,
            width: 0,
            height: 0,
            duration_ms: None,
            sticker_id: None,
            sticker_type: None,
            pack_id: None,
            associated_emojis: Vec::new(),
        }
    }

    pub fn from_gif(gif: &GifItem) -> Self {
        Self {
            thumbnail_url: if gif.preview_url.is_empty() { None } else { Some(gif.preview_url.clone()) },
            width: gif.width,
            height: gif.height,
            duration_ms: if gif.duration_ms > 0 { Some(gif.duration_ms) } else { None },
            ..Self::new(CommentMediaType::Gif, gif.file_url.clone())
        }
    }

    pub fn from_sticker(sticker: &StickerItem) -> Self {
        Self {
            thumbnail_url: sticker.thumbnail_url.clone(),
            width: sticker.width,
            height: sticker.height,
            duration_ms: sticker.duration_ms,
            sticker_id: Some(sticker.id.clone()),
            sticker_type: Some(sticker.asset_type.wire().to_owned()),
            pack_id: Some(sticker.pack_id.clone()),
            associated_emojis: sticker.associated_emojis.clone(),
            ..Self::new(CommentMediaType::Sticker, sticker.file_url.clone())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommentPhotoUpload {
    pub is_compressing: bool,
    pub compression_progress: f64,
    pub upload_progress: f64,
    pub uploaded_bytes: i64,
    pub total_bytes: i64,
}

impl Default for CommentPhotoUpload {
    fn default() -> Self {
        Self::QUEUED
    }
}

impl CommentPhotoUpload {
    pub const QUEUED: CommentPhotoUpload = CommentPhotoUpload {
        is_compressing: true,
        compression_progress: 0.0,
        upload_progress: 0.0,
        uploaded_bytes: 0,
        total_bytes: 0,
    };

    pub fn sent(total_bytes: i64) -> CommentPhotoUpload {
        CommentPhotoUpload {
            is_compressing: false,
            compression_progress: 100.0,
            upload_progress: 100.0,
            uploaded_bytes: total_bytes,
            total_bytes,
        }
    }

    pub fn media_item(&self, id: String, media: &CommentMedia) -> MessageMediaItem {
        let mut item = MessageMediaItem::new(id, MessageMediaKind::Image, media.file_url.clone());
        item.width = media.width as f32;
        item.height = media.height as f32;
        item.is_uploading = true;
        item.is_compressing = self.is_compressing;
        item.compression_progress = self.compression_progress;
        item.upload_progress = self.upload_progress;
        item.uploaded_bytes = self.uploaded_bytes;
        item.total_bytes = self.total_bytes;
        item
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CommentPhotoUploadProgress {
    Compressing(f64),
    Uploading { completed_bytes: i64, total_bytes: i64 },
}

pub struct CommentEntities;

impl CommentEntities {
    pub fn normalized(raw: Option<&str>) -> (Option<String>, Vec<MessageCustomEmojiEntity>) {
        let Some(trimmed) = raw.map(trimming_whitespaces_and_newlines).filter(|text| !text.is_empty()) else {
            return (None, Vec::new());
        };
        (Some(trimmed.to_owned()), MessageNormalizedEntities::parse(Some(trimmed)).custom_emojis)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GetCommentsResponse {
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
    pub media: Option<CommentMedia>,
    pub entities: Option<String>,
    pub custom_emojis: Vec<MessageCustomEmojiEntity>,
    pub is_temp: bool,
    pub moderation_status: String,
    pub is_first: bool,
}

impl Default for GetCommentsResponse {
    fn default() -> Self {
        Self {
            id: String::new(),
            content: String::new(),
            original_content: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
            post_id: String::new(),
            author_id: String::new(),
            parent_id: None,
            replies_count: 0,
            likes_count: 0,
            dislikes_count: 0,
            user_liked: false,
            user_disliked: false,
            addressed_to_name: None,
            addressed_to_tag: None,
            author: AuthorInfo::default(),
            media: None,
            entities: None,
            custom_emojis: Vec::new(),
            is_temp: false,
            moderation_status: ContentStatus::fallback().wire().to_owned(),
            is_first: false,
        }
    }
}

impl GetCommentsResponse {
    pub fn reaction(&self) -> CommentReaction {
        if self.user_liked {
            return CommentReaction::Like;
        }
        if self.user_disliked {
            return CommentReaction::Dislike;
        }
        CommentReaction::None
    }

    pub fn is_moderation_disabled(&self) -> bool {
        self.moderation_status == ContentStatus::Disabled.wire()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct UserCommentPostPreview {
    pub id: String,
    pub title: String,
    pub image: Option<String>,
    pub author_id: String,
    pub author: AuthorInfo,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UserCommentEntry {
    pub comment: GetCommentsResponse,
    pub post: Option<UserCommentPostPreview>,
    pub post_card: Option<GetPostFeedResponse>,
    pub parent: Option<GetCommentsResponse>,
}

impl Eq for UserCommentEntry {}

impl std::hash::Hash for UserCommentEntry {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.comment.hash(state);
        self.post.hash(state);
        self.parent.hash(state);
    }
}

impl UserCommentEntry {
    pub fn id(&self) -> &str {
        &self.comment.id
    }
}
