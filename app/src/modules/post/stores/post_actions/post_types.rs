use std::collections::HashSet;
use std::sync::LazyLock;

use rise_canon::rise_key::events;
use rise_canon::rise_key::post as key;
use rise_canon::rise_proto::post::{
    PostCollaboratorRejection as CanonPostCollaboratorRejection, PostCollaboratorUser as CanonPostCollaboratorUser,
    PostMediaSize as CanonPostMediaSize, PostTagPlacement as CanonPostTagPlacement,
    PostLinkArea, PostTaggedUser as CanonPostTaggedUser, PostTrackPlate, PostViewerCollaboration as CanonPostViewerCollaboration,
};
use rise_canon::{RiseLimits, RiseWireKey, RiseWireObject, rise_decode};
use rise_catalog::{PostMediaStatus, PostModerationStatus};
use rise_core::foundation::foundation_character::{characters, int_from_string};
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::{character_count, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::formatted_text_ui::FormattedText;
use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageCustomEmojiEntity, RemoteMediaURL,
};
use crate::modules::comment::stores::comment_actions::comment_types::CommentEntities;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerTrackSelection};
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_playback_url::MusicPlaybackUrl;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::post::engine::rise_post_engine_models::{RisePostContentKind, RisePostContentKindResolve};
use crate::modules::post::shared::post_media_slides::PostMediaSlide;

pub type PostMediaSize = CanonPostMediaSize;

pub trait PostMediaSizeAspectRatio {
    fn aspect_ratio(&self) -> Option<f64>;
}

impl PostMediaSizeAspectRatio for CanonPostMediaSize {
    fn aspect_ratio(&self) -> Option<f64> {
        if self.width <= 0 || self.height <= 0 {
            return None;
        }
        Some(self.width as f64 / self.height as f64)
    }
}

/// track card on the media, theme plus normalized transform (center 0..1 of the media frame
/// card width 0.72 of the short side at scale 1)
/// rendered live over the media so the equalizer keeps moving
pub type PostTrackPlateDTO = PostTrackPlate;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct AuthorInfoMore {
    pub p_lang: Vec<String>,
    pub who: String,
    pub role: String,
    pub logo: String,
    pub is_premium: bool,
    pub is_official: bool,
    pub phone: Option<String>,
    pub premium_badge_emoji: Option<String>,
    pub name_color: Option<String>,
}

impl<'de> Deserialize<'de> for AuthorInfoMore {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                p_lang: container.rise_required::<Vec<String>>(key::P_LANG)?,
                who: container.rise_required::<String>(key::WHO)?,
                role: container.rise_required::<String>(key::ROLE)?,
                logo: container.rise_required::<String>(key::LOGO)?,
                is_premium: container.rise_required::<bool>(key::IS_PREMIUM)?,
                is_official: container.rise_required::<bool>(key::IS_OFFICIAL)?,
                phone: container.rise::<String>(events::PHONE)?,
                premium_badge_emoji: container.rise::<String>(key::PREMIUM_BADGE_EMOJI)?,
                name_color: container.rise::<String>(key::NAME_COLOR)?,
            })
        })
    }
}

impl Serialize for AuthorInfoMore {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(&RiseWireKey::camel(key::P_LANG), &self.p_lang)?;
        container.serialize_entry(key::WHO, &self.who)?;
        container.serialize_entry(key::ROLE, &self.role)?;
        container.serialize_entry(key::LOGO, &self.logo)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_PREMIUM), &self.is_premium)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_OFFICIAL), &self.is_official)?;
        if let Some(phone) = &self.phone {
            container.serialize_entry(events::PHONE, phone)?;
        }
        if let Some(premium_badge_emoji) = &self.premium_badge_emoji {
            container.serialize_entry(&RiseWireKey::camel(key::PREMIUM_BADGE_EMOJI), premium_badge_emoji)?;
        }
        if let Some(name_color) = &self.name_color {
            container.serialize_entry(&RiseWireKey::camel(key::NAME_COLOR), name_color)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct AuthorInfo {
    pub id: Option<String>,
    pub name: String,
    pub tag: String,
    pub more: AuthorInfoMore,
    pub is_subscribed: bool,
    pub is_subscriber: bool,
    pub is_friend: bool,
    pub is_deleted: bool,
    pub friend_request_id: Option<i64>,
}

impl<'de> Deserialize<'de> for AuthorInfo {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                id: container.rise::<String>(key::ID)?,
                name: container.rise_required::<String>(key::NAME)?,
                tag: container.rise_required::<String>(key::TAG)?,
                more: container.rise_required::<AuthorInfoMore>(events::MORE)?,
                is_deleted: container.rise_required::<bool>(key::IS_DELETED)?,
                is_subscribed: container.rise_required::<bool>(key::IS_SUBSCRIBED)?,
                is_subscriber: container.rise_required::<bool>(key::IS_SUBSCRIBER)?,
                is_friend: container.rise_required::<bool>(key::IS_FRIEND)?,
                friend_request_id: container.rise::<i64>(key::FRIEND_REQUEST_ID)?,
            })
        })
    }
}

impl Serialize for AuthorInfo {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(id) = &self.id {
            container.serialize_entry(key::ID, id)?;
        }
        container.serialize_entry(key::NAME, &self.name)?;
        container.serialize_entry(key::TAG, &self.tag)?;
        container.serialize_entry(events::MORE, &self.more)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_DELETED), &self.is_deleted)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_SUBSCRIBED), &self.is_subscribed)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_SUBSCRIBER), &self.is_subscriber)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_FRIEND), &self.is_friend)?;
        if let Some(friend_request_id) = &self.friend_request_id {
            container.serialize_entry(&RiseWireKey::camel(key::FRIEND_REQUEST_ID), friend_request_id)?;
        }
        container.end()
    }
}

impl AuthorInfo {
    pub fn display_name(&self) -> String {
        if self.is_deleted { tr("chat_deleted_account") } else { self.name.clone() }
    }

    /// server keeps a placeholder tag on an erased account and it leads nowhere
    pub fn display_tag(&self) -> String {
        if self.is_deleted { String::new() } else { self.tag.clone() }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PostLikedByUser {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub logo: String,
    pub is_premium: bool,
    pub is_official: bool,
    pub is_friend: bool,
}

impl<'de> Deserialize<'de> for PostLikedByUser {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                id: container.rise_required::<String>(key::ID)?,
                name: container.rise_required::<String>(key::NAME)?,
                tag: container.rise_required::<String>(key::TAG)?,
                logo: container.rise_required::<String>(key::LOGO)?,
                is_premium: container.rise_required::<bool>(key::IS_PREMIUM)?,
                is_official: container.rise_required::<bool>(key::IS_OFFICIAL)?,
                is_friend: container.rise_required::<bool>(key::IS_FRIEND)?,
            })
        })
    }
}

impl Serialize for PostLikedByUser {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(key::ID, &self.id)?;
        container.serialize_entry(key::NAME, &self.name)?;
        container.serialize_entry(key::TAG, &self.tag)?;
        container.serialize_entry(key::LOGO, &self.logo)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_PREMIUM), &self.is_premium)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_OFFICIAL), &self.is_official)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_FRIEND), &self.is_friend)?;
        container.end()
    }
}

/// which counters a stranger sees, the authors choice
/// server zeroes a hidden counter for everyone but the author and always sends the flags
/// so the ui hides the label, sparks switch rides here too
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostCountVisibility {
    pub likes: bool,
    pub comments: bool,
    pub reposts: bool,
    pub shares: bool,
    pub comment_dislikes: bool,
    pub sparks: bool,
    pub allows_sparks: bool,
    pub allows_download: bool,
    pub allows_download_without_watermark: bool,
}

impl PostCountVisibility {
    pub const SHOWN: PostCountVisibility = PostCountVisibility {
        likes: true,
        comments: true,
        reposts: true,
        shares: true,
        comment_dislikes: true,
        sparks: true,
        allows_sparks: true,
        allows_download: true,
        allows_download_without_watermark: false,
    };
}

impl Default for PostCountVisibility {
    fn default() -> Self {
        Self::SHOWN
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostCountVisibilityStored {
    #[serde(default)]
    likes: Option<bool>,
    #[serde(default)]
    comments: Option<bool>,
    #[serde(default)]
    reposts: Option<bool>,
    #[serde(default)]
    shares: Option<bool>,
    #[serde(default)]
    comment_dislikes: Option<bool>,
    #[serde(default)]
    sparks: Option<bool>,
    #[serde(default)]
    allows_sparks: Option<bool>,
    #[serde(default)]
    allows_download: Option<bool>,
    #[serde(default)]
    allows_download_without_watermark: Option<bool>,
}

impl<'de> Deserialize<'de> for PostCountVisibility {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored = PostCountVisibilityStored::deserialize(deserializer)?;
        Ok(Self {
            likes: stored.likes.unwrap_or(true),
            comments: stored.comments.unwrap_or(true),
            reposts: stored.reposts.unwrap_or(true),
            shares: stored.shares.unwrap_or(true),
            comment_dislikes: stored.comment_dislikes.unwrap_or(true),
            sparks: stored.sparks.unwrap_or(true),
            allows_sparks: stored.allows_sparks.unwrap_or(true),
            allows_download: stored.allows_download.unwrap_or(true),
            allows_download_without_watermark: stored.allows_download_without_watermark.unwrap_or(false),
        })
    }
}

impl GetPostFeedResponse {
    /// true when this viewer may see a hidden counter, the author always can
    pub fn shows_count(&self, key_path: fn(&PostCountVisibility) -> bool, viewer: Option<&str>) -> bool {
        if key_path(&self.count_visibility) {
            return true;
        }
        let Some(viewer) = viewer.filter(|viewer| !viewer.is_empty()) else {
            return false;
        };
        viewer == self.author_id || Some(viewer) == self.author.id.as_deref()
    }

    /// true when this viewer gets the sparks button
    /// with sparks off strangers lose it and the author keeps it only while there are received sparks
    pub fn shows_sparks_button(&self, viewer: Option<&str>) -> bool {
        if self.count_visibility.allows_sparks {
            return true;
        }
        let Some(viewer) = viewer.filter(|viewer| !viewer.is_empty()) else {
            return false;
        };
        if !(viewer == self.author_id || Some(viewer) == self.author.id.as_deref()) {
            return false;
        }
        self.sparks_total > 0
    }

    /// custom emojis of the viewers own repost opinion, decoded from the raw entities
    pub fn my_repost_custom_emojis(&self) -> Vec<MessageCustomEmojiEntity> {
        CommentEntities::normalized(self.my_repost_entities.as_deref()).1
    }

    pub fn reposter_custom_emojis(&self) -> Vec<MessageCustomEmojiEntity> {
        CommentEntities::normalized(self.reposter_entities.as_deref()).1
    }
}

pub type PostCollaboratorUser = CanonPostCollaboratorUser;

pub type PostTagPlacement = CanonPostTagPlacement;

pub type PostTaggedUser = CanonPostTaggedUser;

pub trait PostTaggedUserIdentifiable {
    fn id(&self) -> String;
}

impl PostTaggedUserIdentifiable for CanonPostTaggedUser {
    fn id(&self) -> String {
        self.user.as_ref().map(|user| user.id.clone()).unwrap_or_default()
    }
}

pub type PostViewerCollaboration = CanonPostViewerCollaboration;
pub type PostCollaboratorRejection = CanonPostCollaboratorRejection;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostCollaboratorKind {
    Coauthor,
    Tag,
}

impl PostCollaboratorKind {
    pub fn raw_value(self) -> &'static str {
        match self {
            PostCollaboratorKind::Coauthor => "coauthor",
            PostCollaboratorKind::Tag => "tag",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostCollaboratorStatus {
    Pending,
    Accepted,
}

impl PostCollaboratorStatus {
    pub fn raw_value(self) -> &'static str {
        match self {
            PostCollaboratorStatus::Pending => "pending",
            PostCollaboratorStatus::Accepted => "accepted",
        }
    }
}

/// author only plate on a post others cant see yet or at all
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostModerationBanner {
    Disabled,
    UnderReview,
    Rejected,
}

impl PostModerationBanner {
    pub fn from_status(status: &str) -> Option<Self> {
        match PostModerationStatus::from_wire(status) {
            Some(PostModerationStatus::Disabled) => Some(Self::Disabled),
            Some(PostModerationStatus::PendingReview) => Some(Self::UnderReview),
            Some(PostModerationStatus::Rejected) => Some(Self::Rejected),
            Some(PostModerationStatus::Active) | None => None,
        }
    }

    pub fn text_key(self) -> &'static str {
        match self {
            Self::Disabled => "post_disabled_by_moderation",
            Self::UnderReview => "post_review_pending",
            Self::Rejected => "post_review_rejected",
        }
    }

    /// short word for a grid tile, a hidden post keeps its own eye overlay there
    pub fn badge_key(self) -> Option<&'static str> {
        match self {
            Self::Disabled => None,
            Self::UnderReview => Some("post_review_badge"),
            Self::Rejected => Some("post_rejected_badge"),
        }
    }

    pub fn symbol_name(self) -> &'static str {
        match self {
            Self::Disabled => "eye.slash",
            Self::UnderReview => "clock",
            Self::Rejected => "xmark.octagon",
        }
    }

    /// a post waiting for review did nothing wrong so it reads in accent and not error color
    pub fn is_error(self) -> bool {
        self != Self::UnderReview
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostTagInputDraft {
    pub user_id: String,
    pub placements: Vec<PostTagPlacement>,
    pub user: Option<PostCollaboratorUser>,
}

impl PostTagInputDraft {
    pub fn new(user_id: String) -> Self {
        Self { user_id, placements: Vec::new(), user: None }
    }
}

pub enum PostEditMediaItem {
    Existing(PostMediaSlide),
    Photo(SaiWsOptimisticLocalFile),
    Video(MediaPickerItem),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PostCollaboratorRow {
    pub user: PostCollaboratorUser,
    pub kind: String,
    pub status: String,
    pub show_in_grid: bool,
    pub placements: Vec<PostTagPlacement>,
}

impl PostCollaboratorRow {
    pub fn new(user: PostCollaboratorUser) -> Self {
        Self {
            user,
            kind: PostCollaboratorKind::Coauthor.raw_value().to_owned(),
            status: PostCollaboratorStatus::Accepted.raw_value().to_owned(),
            show_in_grid: true,
            placements: Vec::new(),
        }
    }

    pub fn id(&self) -> String {
        format!("{}/{}", self.user.id, self.kind)
    }
}

impl<'de> Deserialize<'de> for PostCollaboratorRow {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                user: container.rise_required::<PostCollaboratorUser>(key::USER)?,
                kind: container.rise_required::<String>(key::KIND)?,
                status: container.rise_required::<String>(key::STATUS)?,
                show_in_grid: container.rise_required::<bool>(key::SHOW_IN_GRID)?,
                placements: container.rise_required::<Vec<PostTagPlacement>>(key::PLACEMENTS)?,
            })
        })
    }
}

impl Serialize for PostCollaboratorRow {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(key::USER, &self.user)?;
        container.serialize_entry(key::KIND, &self.kind)?;
        container.serialize_entry(key::STATUS, &self.status)?;
        container.serialize_entry(&RiseWireKey::camel(key::SHOW_IN_GRID), &self.show_in_grid)?;
        container.serialize_entry(key::PLACEMENTS, &self.placements)?;
        container.end()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GetPostFeedResponse {
    pub id: String,
    pub title: String,
    pub content: String,
    pub original_content: String,
    pub hashtags: Vec<String>,
    pub tags: Vec<String>,
    pub views_count: i64,
    pub created_at: String,
    pub updated_at: String,
    pub author_id: String,
    pub main_comments_count: i64,
    pub comments_count: i64,
    pub can_comment: bool,
    pub likes_count: i64,
    pub favorites_count: i64,
    pub images: Vec<String>,
    pub image_sizes: Vec<PostMediaSize>,
    pub is_liked: bool,
    pub is_favorited: bool,
    pub old_is_liked: bool,
    pub old_is_favorited: bool,
    pub author: AuthorInfo,
    pub video: Option<String>,
    pub video_thumbnail: Option<String>,
    pub video_size: Option<PostMediaSize>,
    pub is_short: bool,
    pub is_image_reel: bool,
    pub is_riff: bool,
    pub show_in_publications: bool,
    pub video_position: Option<i64>,
    pub achievement_id: Option<i64>,
    pub map_journey_id: Option<String>,
    pub track_id: Option<String>,
    pub track: Option<RiseStoredMusicTrack>,
    pub track_start_ms: Option<i64>,
    pub track_end_ms: Option<i64>,
    pub track_plate: Option<PostTrackPlateDTO>,
    pub link_areas: Vec<PostLinkArea>,
    pub audio_baked: bool,
    pub media_status: String,
    pub moderation_status: String,
    pub repost_count: i64,
    pub is_reposted: bool,
    pub my_repost_id: Option<String>,
    pub my_repost_content: Option<String>,
    pub liked_by: Vec<PostLikedByUser>,
    pub coauthors: Vec<PostCollaboratorUser>,
    pub coauthors_count: i64,
    pub pending_coauthors_count: i64,
    pub my_collaboration: Option<PostViewerCollaboration>,
    pub tagged_users: Vec<PostTaggedUser>,
    pub tagged_users_count: i64,
    pub content_kind: String,
    pub link_preview: Option<PostLinkPreview>,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub count_visibility: PostCountVisibility,
    pub share_count: i64,
    pub link_open_count: i64,
    pub sparks_total: i64,
    pub my_sparks: i64,
    pub my_repost_entities: Option<String>,
    pub reposter_content: Option<String>,
    pub reposter_entities: Option<String>,
    pub presentation_id: Option<String>,
    pub is_temp: bool,
    pub is_uploading: bool,
    pub is_compressing: bool,
    pub upload_progress: f64,
    pub compression_progress: f64,
    pub local_preview_url: Option<String>,
}

impl Default for GetPostFeedResponse {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            content: String::new(),
            original_content: String::new(),
            hashtags: Vec::new(),
            tags: Vec::new(),
            views_count: 0,
            created_at: String::new(),
            updated_at: String::new(),
            author_id: String::new(),
            main_comments_count: 0,
            comments_count: 0,
            can_comment: true,
            likes_count: 0,
            favorites_count: 0,
            images: Vec::new(),
            image_sizes: Vec::new(),
            is_liked: false,
            is_favorited: false,
            old_is_liked: false,
            old_is_favorited: false,
            author: AuthorInfo::default(),
            video: None,
            video_thumbnail: None,
            video_size: None,
            is_short: false,
            is_image_reel: false,
            is_riff: false,
            show_in_publications: true,
            video_position: None,
            achievement_id: None,
            map_journey_id: None,
            track_id: None,
            track: None,
            track_start_ms: None,
            track_end_ms: None,
            track_plate: None,
            link_areas: Vec::new(),
            audio_baked: false,
            media_status: PostMediaStatus::Ready.wire().to_owned(),
            moderation_status: PostModerationStatus::Active.wire().to_owned(),
            repost_count: 0,
            is_reposted: false,
            my_repost_id: None,
            my_repost_content: None,
            liked_by: Vec::new(),
            coauthors: Vec::new(),
            coauthors_count: 0,
            pending_coauthors_count: 0,
            my_collaboration: None,
            tagged_users: Vec::new(),
            tagged_users_count: 0,
            content_kind: RisePostContentKind::Thread.wire().to_owned(),
            link_preview: None,
            created_at_ms: None,
            updated_at_ms: None,
            count_visibility: PostCountVisibility::SHOWN,
            share_count: 0,
            link_open_count: 0,
            sparks_total: 0,
            my_sparks: 0,
            my_repost_entities: None,
            reposter_content: None,
            reposter_entities: None,
            presentation_id: None,
            is_temp: false,
            is_uploading: false,
            is_compressing: false,
            upload_progress: 0.0,
            compression_progress: 0.0,
            local_preview_url: None,
        }
    }
}

impl GetPostFeedResponse {
    pub fn shows_upload_overlay(&self) -> bool {
        self.is_temp && (self.is_uploading || self.is_compressing)
    }

    /// server is still baking the audio into the video, not a display state
    /// the post plays from the source file meanwhile with the track on top
    /// flag is there so the page can ask again if the ready push got lost
    pub fn is_processing_media(&self) -> bool {
        self.media_status == PostMediaStatus::Processing.wire()
    }

    /// only signal that the posts visual is a shared achievement
    /// server refuses achievement_id together with any
    /// media key, so it never mixes with images or video
    pub fn shared_achievement_id(&self) -> Option<i64> {
        let value = self.achievement_id?;
        if value <= 0 {
            return None;
        }
        Some(value)
    }

    pub fn is_moderation_disabled(&self) -> bool {
        self.moderation_status == PostModerationStatus::Disabled.wire()
    }

    pub fn is_under_review(&self) -> bool {
        self.moderation_status == PostModerationStatus::PendingReview.wire()
    }

    pub fn is_moderation_rejected(&self) -> bool {
        self.moderation_status == PostModerationStatus::Rejected.wire()
    }

    pub fn moderation_banner(&self) -> Option<PostModerationBanner> {
        PostModerationBanner::from_status(&self.moderation_status)
    }

    pub fn is_pending_coauthor_invite(&self) -> bool {
        self.my_collaboration.as_ref().map(|c| c.kind.as_str()) == Some(PostCollaboratorKind::Coauthor.raw_value())
            && self.my_collaboration.as_ref().map(|c| c.status.as_str())
                == Some(PostCollaboratorStatus::Pending.raw_value())
    }

    pub fn is_accepted_coauthor(&self) -> bool {
        self.my_collaboration.as_ref().map(|c| c.kind.as_str()) == Some(PostCollaboratorKind::Coauthor.raw_value())
            && self.my_collaboration.as_ref().map(|c| c.status.as_str())
                == Some(PostCollaboratorStatus::Accepted.raw_value())
    }

    pub fn is_tagged_viewer(&self) -> bool {
        self.my_collaboration.as_ref().map(|c| c.kind.as_str()) == Some(PostCollaboratorKind::Tag.raw_value())
    }

    pub fn coauthor_shows_in_grid(&self) -> bool {
        self.my_collaboration.as_ref().is_some_and(|c| c.show_in_grid)
    }

    pub fn tagged_users(&self, on_slide: i64) -> Vec<PostTaggedUser> {
        self.tagged_users
            .iter()
            .filter_map(|entry| {
                let placements: Vec<PostTagPlacement> =
                    entry.placements.iter().filter(|p| p.slide_index == on_slide).cloned().collect();
                if placements.is_empty() {
                    return None;
                }
                Some(PostTaggedUser { placements, user: entry.user.clone() })
            })
            .collect()
    }

    pub fn has_tags(&self, on_slide: i64) -> bool {
        self.tagged_users.iter().any(|entry| entry.placements.iter().any(|p| p.slide_index == on_slide))
    }

    pub fn track_model(&self) -> Option<TrackModel> {
        self.track.as_ref().and_then(|track| track.as_track_model())
    }

    pub fn music_segment_start_ms(&self) -> i64 {
        self.track_start_ms.unwrap_or(0).max(0)
    }

    pub fn music_segment_end_ms(&self) -> i64 {
        if let Some(end) = self.track_end_ms
            && end > self.music_segment_start_ms()
        {
            return end;
        }
        let duration = self.track.as_ref().and_then(|track| track.duration_ms).unwrap_or(0);
        if duration > 0 { duration.min(30_000) } else { 30_000 }
    }

    /// the posts sound as heard, the track and the cut segment
    /// used by the sound page opened from the post and as default clip for a new post made from it
    pub fn music_use_selection(&self) -> Option<MediaPickerTrackSelection> {
        let track = self.track.as_ref()?;
        if track.id.as_deref().unwrap_or("").is_empty() {
            return None;
        }
        Some(MediaPickerTrackSelection {
            track: track.clone(),
            start_ms: self.music_segment_start_ms(),
            end_ms: self.music_segment_end_ms(),
            plate: None,
        })
    }

    pub fn music_audio_url(&self) -> Option<Url> {
        self.track_id.as_ref()?;
        let normalized =
            RemoteMediaURL::optional_normalized(self.track.as_ref().and_then(|track| track.audio_url.as_deref()))?;
        if normalized.is_empty() {
            return None;
        }
        if normalized.starts_with('/') {
            return MusicPlaybackUrl::playback_url(&normalized);
        }
        Url::from_string(&normalized)
    }

    pub fn resolved_video_url(&self) -> Option<String> {
        if let Some(local) = self.local_preview_url.as_deref().map(trimming_whitespaces_and_newlines)
            && !local.is_empty()
            && (local.starts_with("file://") || is_local_absolute_path(local))
        {
            return Some(local.to_owned());
        }
        let raw = trimming_whitespaces_and_newlines(self.video.as_deref()?);
        if raw.is_empty() {
            return None;
        }
        Some(raw.to_owned())
    }

    pub fn presentation_id(&self) -> String {
        self.presentation_id.clone().unwrap_or_else(|| self.id.clone())
    }
}

impl Serialize for GetPostFeedResponse {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(key::ID, &self.id)?;
        container.serialize_entry(key::TITLE, &self.title)?;
        container.serialize_entry(key::CONTENT, &self.content)?;
        container.serialize_entry(&RiseWireKey::camel(key::ORIGINAL_CONTENT), &self.original_content)?;
        container.serialize_entry(key::HASHTAGS, &self.hashtags)?;
        container.serialize_entry(key::TAGS, &self.tags)?;
        container.serialize_entry(&RiseWireKey::camel(key::VIEWS_COUNT), &self.views_count)?;
        container.serialize_entry(&RiseWireKey::camel(key::CREATED_AT), &self.created_at)?;
        container.serialize_entry(&RiseWireKey::camel(key::UPDATED_AT), &self.updated_at)?;
        container.serialize_entry(&RiseWireKey::camel(key::AUTHOR_ID), &self.author_id)?;
        container.serialize_entry(&RiseWireKey::camel(key::MAIN_COMMENTS_COUNT), &self.main_comments_count)?;
        container.serialize_entry(&RiseWireKey::camel(key::COMMENTS_COUNT), &self.comments_count)?;
        container.serialize_entry(&RiseWireKey::camel(key::CAN_COMMENT), &self.can_comment)?;
        container.serialize_entry(&RiseWireKey::camel(key::LIKES_COUNT), &self.likes_count)?;
        container.serialize_entry(&RiseWireKey::camel(key::FAVORITES_COUNT), &self.favorites_count)?;
        container.serialize_entry(key::IMAGES, &self.images)?;
        container.serialize_entry(&RiseWireKey::camel(key::IMAGE_SIZES), &self.image_sizes)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_LIKED), &self.is_liked)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_FAVORITED), &self.is_favorited)?;
        container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::OLD_IS_LIKED), &self.old_is_liked)?;
        container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::OLD_IS_FAVORITED), &self.old_is_favorited)?;
        container.serialize_entry(key::AUTHOR, &self.author)?;
        if let Some(video) = &self.video {
            container.serialize_entry(key::VIDEO, video)?;
        }
        if let Some(video_thumbnail) = &self.video_thumbnail {
            container.serialize_entry(&RiseWireKey::camel(key::VIDEO_THUMBNAIL), video_thumbnail)?;
        }
        if let Some(video_size) = &self.video_size {
            container.serialize_entry(&RiseWireKey::camel(key::VIDEO_SIZE), video_size)?;
        }
        container.serialize_entry(&RiseWireKey::camel(key::IS_SHORT), &self.is_short)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_IMAGE_REEL), &self.is_image_reel)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_RIFF), &self.is_riff)?;
        container.serialize_entry(&RiseWireKey::camel(key::SHOW_IN_PUBLICATIONS), &self.show_in_publications)?;
        if let Some(video_position) = &self.video_position {
            container.serialize_entry(&RiseWireKey::camel(key::VIDEO_POSITION), video_position)?;
        }
        if let Some(achievement_id) = &self.achievement_id {
            container.serialize_entry(&RiseWireKey::camel(key::ACHIEVEMENT_ID), achievement_id)?;
        }
        if let Some(map_journey_id) = &self.map_journey_id {
            container.serialize_entry(&RiseWireKey::camel(key::MAP_JOURNEY_ID), map_journey_id)?;
        }
        if let Some(track_id) = &self.track_id {
            container.serialize_entry(&RiseWireKey::camel(key::TRACK_ID), track_id)?;
        }
        if let Some(track) = &self.track {
            container.serialize_entry(key::TRACK, track)?;
        }
        if let Some(track_start_ms) = &self.track_start_ms {
            container.serialize_entry(&RiseWireKey::camel(key::TRACK_START_MS), track_start_ms)?;
        }
        if let Some(track_end_ms) = &self.track_end_ms {
            container.serialize_entry(&RiseWireKey::camel(key::TRACK_END_MS), track_end_ms)?;
        }
        if let Some(track_plate) = &self.track_plate {
            container.serialize_entry(&RiseWireKey::camel(key::TRACK_PLATE), track_plate)?;
        }
        container.serialize_entry(&RiseWireKey::camel(key::LINK_AREAS), &self.link_areas)?;
        container.serialize_entry(&RiseWireKey::camel(key::AUDIO_BAKED), &self.audio_baked)?;
        container.serialize_entry(&RiseWireKey::camel(key::MEDIA_STATUS), &self.media_status)?;
        container.serialize_entry(&RiseWireKey::camel(key::MODERATION_STATUS), &self.moderation_status)?;
        container.serialize_entry(&RiseWireKey::camel(key::REPOST_COUNT), &self.repost_count)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_REPOSTED), &self.is_reposted)?;
        if let Some(my_repost_id) = &self.my_repost_id {
            container.serialize_entry(&RiseWireKey::camel(key::MY_REPOST_ID), my_repost_id)?;
        }
        if let Some(my_repost_content) = &self.my_repost_content {
            container.serialize_entry(&RiseWireKey::camel(key::MY_REPOST_CONTENT), my_repost_content)?;
        }
        container.serialize_entry(&RiseWireKey::camel(key::LIKED_BY), &self.liked_by)?;
        container.serialize_entry(key::COAUTHORS, &self.coauthors)?;
        container.serialize_entry(&RiseWireKey::camel(key::COAUTHORS_COUNT), &self.coauthors_count)?;
        container.serialize_entry(&RiseWireKey::camel(key::PENDING_COAUTHORS_COUNT), &self.pending_coauthors_count)?;
        if let Some(my_collaboration) = &self.my_collaboration {
            container.serialize_entry(&RiseWireKey::camel(key::MY_COLLABORATION), my_collaboration)?;
        }
        container.serialize_entry(&RiseWireKey::camel(key::TAGGED_USERS), &self.tagged_users)?;
        container.serialize_entry(&RiseWireKey::camel(key::TAGGED_USERS_COUNT), &self.tagged_users_count)?;
        container.serialize_entry(&RiseWireKey::camel(key::CONTENT_KIND), &self.content_kind)?;
        if let Some(link_preview) = &self.link_preview {
            container.serialize_entry(&RiseWireKey::camel(key::LINK_PREVIEW), link_preview)?;
        }
        if let Some(created_at_ms) = &self.created_at_ms {
            container.serialize_entry(&RiseWireKey::camel(key::CREATED_AT_MS), created_at_ms)?;
        }
        if let Some(updated_at_ms) = &self.updated_at_ms {
            container.serialize_entry(&RiseWireKey::camel(key::UPDATED_AT_MS), updated_at_ms)?;
        }
        container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::COUNT_VISIBILITY), &self.count_visibility)?;
        container.serialize_entry(&RiseWireKey::camel(key::SHARE_COUNT), &self.share_count)?;
        container.serialize_entry(&RiseWireKey::camel(key::LINK_OPEN_COUNT), &self.link_open_count)?;
        container.serialize_entry(&RiseWireKey::camel(key::SPARKS_TOTAL), &self.sparks_total)?;
        container.serialize_entry(&RiseWireKey::camel(key::MY_SPARKS), &self.my_sparks)?;
        if let Some(my_repost_entities) = &self.my_repost_entities {
            container.serialize_entry(&RiseWireKey::camel(key::MY_REPOST_ENTITIES), my_repost_entities)?;
        }
        if let Some(reposter_content) = &self.reposter_content {
            container.serialize_entry(&RiseWireKey::camel(key::REPOSTER_CONTENT), reposter_content)?;
        }
        if let Some(reposter_entities) = &self.reposter_entities {
            container.serialize_entry(&RiseWireKey::camel(key::REPOSTER_ENTITIES), reposter_entities)?;
        }
        if let Some(presentation_id) = &self.presentation_id {
            container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::PRESENTATION_ID), presentation_id)?;
        }
        container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::IS_TEMP), &self.is_temp)?;
        container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::IS_UPLOADING), &self.is_uploading)?;
        container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::IS_COMPRESSING), &self.is_compressing)?;
        container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::UPLOAD_PROGRESS), &self.upload_progress)?;
        container.serialize_entry(
            &RiseWireKey::camel(RiseLocalKey::COMPRESSION_PROGRESS),
            &self.compression_progress,
        )?;
        if let Some(local_preview_url) = &self.local_preview_url {
            container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::LOCAL_PREVIEW_URL), local_preview_url)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct PostDeleteCachePlacement {
    pub cache_key: String,
    pub index: usize,
    pub persists_to_disk: bool,
}

#[derive(Clone, Debug)]
pub struct PostDeleteOptimisticSnapshot {
    pub post: GetPostFeedResponse,
    pub placements: Vec<PostDeleteCachePlacement>,
    pub was_selected: bool,
    pub detail_context_index: Option<usize>,
    pub profile_posts_count: Option<i64>,
    pub shorts_feed_post: Option<GetPostFeedResponse>,
    pub shorts_feed_index: Option<usize>,
    pub external_short_post: Option<GetPostFeedResponse>,
}

impl GetPostFeedResponse {
    fn decode_video_position(container: &RiseWireObject, key: &str) -> Option<i64> {
        if let Some(value) = container.rise::<i64>(key).ok().flatten() {
            return Some(value);
        }
        if let Some(text) = container.rise::<String>(key).ok().flatten() {
            return int_from_string(trimming_whitespaces_and_newlines(&text));
        }
        None
    }
}

impl<'de> Deserialize<'de> for GetPostFeedResponse {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            let id = c.rise_required::<String>(key::ID)?;
            let title = c.rise::<String>(key::TITLE)?.unwrap_or_default();
            let content = c.rise::<String>(key::CONTENT)?.unwrap_or_default();
            let original_content = c.rise::<String>(key::ORIGINAL_CONTENT)?.unwrap_or_default();
            let hashtags = c.rise::<Vec<String>>(key::HASHTAGS)?.unwrap_or_default();
            let tags = c.rise::<Vec<String>>(key::TAGS)?.unwrap_or_default();
            let views_count = c.rise::<i64>(key::VIEWS_COUNT)?.unwrap_or(0);
            let created_at = c.rise::<String>(key::CREATED_AT)?.unwrap_or_default();
            let updated_at = c.rise::<String>(key::UPDATED_AT)?.unwrap_or_default();
            let author_id = c.rise::<String>(key::AUTHOR_ID)?.unwrap_or_default();
            let main_comments_count = c.rise::<i64>(key::MAIN_COMMENTS_COUNT)?.unwrap_or(0);
            let comments_count = c.rise::<i64>(key::COMMENTS_COUNT)?.unwrap_or(0);
            let can_comment = c.rise::<bool>(key::CAN_COMMENT)?.unwrap_or(true);
            let likes_count = c.rise::<i64>(key::LIKES_COUNT)?.unwrap_or(0);
            let favorites_count = c.rise::<i64>(key::FAVORITES_COUNT)?.unwrap_or(0);
            let images = c.rise::<Vec<String>>(key::IMAGES)?.unwrap_or_default();
            let image_sizes = c.rise::<Vec<PostMediaSize>>(key::IMAGE_SIZES)?.unwrap_or_default();
            let is_liked = c.rise::<bool>(key::IS_LIKED)?.unwrap_or(false);
            let is_favorited = c.rise::<bool>(key::IS_FAVORITED)?.unwrap_or(false);
            let old_is_liked = c.rise::<bool>(RiseLocalKey::OLD_IS_LIKED)?.unwrap_or(is_liked);
            let old_is_favorited = c.rise::<bool>(RiseLocalKey::OLD_IS_FAVORITED)?.unwrap_or(is_favorited);
            let author = c.rise::<AuthorInfo>(key::AUTHOR)?.unwrap_or_default();
            let video = c.rise::<String>(key::VIDEO)?;
            let video_thumbnail = c.rise::<String>(key::VIDEO_THUMBNAIL)?;
            let video_size = c.rise::<PostMediaSize>(key::VIDEO_SIZE)?;
            let is_short = c.rise::<bool>(key::IS_SHORT)?.unwrap_or(false);
            let is_image_reel = c.rise::<bool>(key::IS_IMAGE_REEL)?.unwrap_or(false);
            let is_riff = c.rise::<bool>(key::IS_RIFF)?.unwrap_or(is_short || is_image_reel);
            let show_in_publications = c.rise::<bool>(key::SHOW_IN_PUBLICATIONS)?.unwrap_or(true);
            let video_position = Self::decode_video_position(c, key::VIDEO_POSITION);
            let map_journey_id = c.rise::<String>(key::MAP_JOURNEY_ID)?;
            let achievement_id = if let Some(value) = c.rise::<i64>(key::ACHIEVEMENT_ID).ok().flatten() {
                Some(value)
            } else if let Some(text) = c.rise::<String>(key::ACHIEVEMENT_ID).ok().flatten() {
                int_from_string(&text)
            } else {
                None
            };
            let track_id = c.rise::<String>(key::TRACK_ID)?;
            let track = c.rise::<RiseStoredMusicTrack>(key::TRACK).ok().flatten();
            let track_start_ms = c.rise::<i64>(key::TRACK_START_MS)?;
            let track_end_ms = c.rise::<i64>(key::TRACK_END_MS)?;
            let track_plate = c.rise::<PostTrackPlateDTO>(key::TRACK_PLATE).ok().flatten();
            let link_areas = c.rise::<Vec<PostLinkArea>>(key::LINK_AREAS).ok().flatten().unwrap_or_default();
            let audio_baked = c.rise::<bool>(key::AUDIO_BAKED)?.unwrap_or(false);
            let media_status =
                c.rise::<String>(key::MEDIA_STATUS)?.unwrap_or_else(|| PostMediaStatus::Ready.wire().to_owned());
            let moderation_status = c
                .rise::<String>(key::MODERATION_STATUS)?
                .unwrap_or_else(|| PostModerationStatus::Active.wire().to_owned());
            let repost_count = c.rise::<i64>(key::REPOST_COUNT)?.unwrap_or(0);
            let is_reposted = c.rise::<bool>(key::IS_REPOSTED)?.unwrap_or(false);
            let my_repost_id = if let Some(mrid_int) = c.rise::<i64>(key::MY_REPOST_ID).ok().flatten() {
                Some(mrid_int.to_string())
            } else {
                c.rise::<String>(key::MY_REPOST_ID)?
            };
            let my_repost_content = c.rise::<String>(key::MY_REPOST_CONTENT)?;
            let liked_by = c.rise::<Vec<PostLikedByUser>>(key::LIKED_BY)?.unwrap_or_default();
            let coauthors = c.rise::<Vec<PostCollaboratorUser>>(key::COAUTHORS).ok().flatten().unwrap_or_default();
            let coauthors_count = c.rise::<i64>(key::COAUTHORS_COUNT)?.unwrap_or(coauthors.len() as i64);
            let pending_coauthors_count = c.rise::<i64>(key::PENDING_COAUTHORS_COUNT)?.unwrap_or(0);
            let my_collaboration = c.rise::<PostViewerCollaboration>(key::MY_COLLABORATION).ok().flatten();
            let tagged_users = c.rise::<Vec<PostTaggedUser>>(key::TAGGED_USERS).ok().flatten().unwrap_or_default();
            let tagged_users_count = c.rise::<i64>(key::TAGGED_USERS_COUNT)?.unwrap_or(tagged_users.len() as i64);
            let content_kind = match c.rise::<String>(key::CONTENT_KIND)? {
                Some(content_kind) => content_kind,
                None => RisePostContentKind::resolve(
                    None,
                    &images,
                    video.as_deref(),
                    is_short,
                    is_image_reel,
                    track_id.as_deref(),
                )
                .wire()
                .to_owned(),
            };
            let link_preview = c.rise::<PostLinkPreview>(key::LINK_PREVIEW)?;
            let created_at_ms = c.rise::<i64>(key::CREATED_AT_MS)?;
            let updated_at_ms = c.rise::<i64>(key::UPDATED_AT_MS)?;
            let count_visibility =
                c.rise::<PostCountVisibility>(RiseLocalKey::COUNT_VISIBILITY)?.unwrap_or(PostCountVisibility::SHOWN);
            let share_count = c.rise::<i64>(key::SHARE_COUNT)?.unwrap_or(0);
            let link_open_count = c.rise::<i64>(key::LINK_OPEN_COUNT)?.unwrap_or(0);
            let sparks_total = c.rise::<i64>(key::SPARKS_TOTAL)?.unwrap_or(0);
            let my_sparks = c.rise::<i64>(key::MY_SPARKS)?.unwrap_or(0);
            let my_repost_entities = c.rise::<String>(key::MY_REPOST_ENTITIES)?;
            let reposter_content = c.rise::<String>(key::REPOSTER_CONTENT)?;
            let reposter_entities = c.rise::<String>(key::REPOSTER_ENTITIES)?;
            let presentation_id = c.rise::<String>(RiseLocalKey::PRESENTATION_ID)?;
            let is_temp = c.rise::<bool>(RiseLocalKey::IS_TEMP)?.unwrap_or(false);
            let is_uploading = c.rise::<bool>(RiseLocalKey::IS_UPLOADING)?.unwrap_or(false);
            let is_compressing = c.rise::<bool>(RiseLocalKey::IS_COMPRESSING)?.unwrap_or(false);
            let upload_progress = c.rise::<f64>(RiseLocalKey::UPLOAD_PROGRESS)?.unwrap_or(0.0);
            let compression_progress = c.rise::<f64>(RiseLocalKey::COMPRESSION_PROGRESS)?.unwrap_or(0.0);
            let local_preview_url = c.rise::<String>(RiseLocalKey::LOCAL_PREVIEW_URL)?;
            Ok(Self {
                id,
                title,
                content,
                original_content,
                hashtags,
                tags,
                views_count,
                created_at,
                updated_at,
                author_id,
                main_comments_count,
                comments_count,
                can_comment,
                likes_count,
                favorites_count,
                images,
                image_sizes,
                is_liked,
                is_favorited,
                old_is_liked,
                old_is_favorited,
                author,
                video,
                video_thumbnail,
                video_size,
                is_short,
                is_image_reel,
                is_riff,
                show_in_publications,
                video_position,
                achievement_id,
                map_journey_id,
                track_id,
                track,
                track_start_ms,
                track_end_ms,
                track_plate,
                link_areas,
                audio_baked,
                media_status,
                moderation_status,
                repost_count,
                is_reposted,
                my_repost_id,
                my_repost_content,
                liked_by,
                coauthors,
                coauthors_count,
                pending_coauthors_count,
                my_collaboration,
                tagged_users,
                tagged_users_count,
                content_kind,
                link_preview,
                created_at_ms,
                updated_at_ms,
                count_visibility,
                share_count,
                link_open_count,
                sparks_total,
                my_sparks,
                my_repost_entities,
                reposter_content,
                reposter_entities,
                presentation_id,
                is_temp,
                is_uploading,
                is_compressing,
                upload_progress,
                compression_progress,
                local_preview_url,
            })
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RisePostLinkPreview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub status: String,
    pub revision: i64,
}

impl RisePostLinkPreview {
    pub fn image_url(&self) -> Option<&str> {
        self.image_url.as_deref()
    }
}

impl<'de> Deserialize<'de> for RisePostLinkPreview {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                url: container.rise_required::<String>(key::URL)?,
                title: container.rise::<String>(key::TITLE)?,
                description: container.rise::<String>(key::DESCRIPTION)?,
                status: container.rise_required::<String>(key::STATUS)?,
                revision: container.rise_required::<i64>(key::REVISION)?,
                image_url: container.rise::<String>(key::IMAGE_URL)?,
            })
        })
    }
}

impl Serialize for RisePostLinkPreview {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(key::URL, &self.url)?;
        if let Some(title) = &self.title {
            container.serialize_entry(key::TITLE, title)?;
        }
        if let Some(description) = &self.description {
            container.serialize_entry(key::DESCRIPTION, description)?;
        }
        container.serialize_entry(key::STATUS, &self.status)?;
        container.serialize_entry(key::REVISION, &self.revision)?;
        if let Some(image_url) = &self.image_url {
            container.serialize_entry(&RiseWireKey::camel(key::IMAGE_URL), image_url)?;
        }
        container.end()
    }
}

pub type PostLinkPreview = RisePostLinkPreview;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseStoredPostStats {
    pub views_total: i64,
    pub unique_viewers: i64,
    pub likes_total: i64,
    pub favorites_total: i64,
    pub comments_total: i64,
    pub shares_total: i64,
    pub reposts_total: i64,
    pub follows_gained: i64,
    pub watch_count: i64,
    pub watched_ms_sum: i64,
    pub avg_dwell_ms: i64,
    pub days: Vec<PostStatsDay>,
    pub countries: Vec<PostStatsCountry>,
}

impl RiseStoredPostStats {
    pub fn has_no_data(&self) -> bool {
        self.views_total == 0
            && self.unique_viewers == 0
            && self.likes_total == 0
            && self.favorites_total == 0
            && self.comments_total == 0
            && self.shares_total == 0
            && self.reposts_total == 0
            && self.follows_gained == 0
            && self.watch_count == 0
            && self.days.iter().all(PostStatsDay::is_empty)
            && self.countries.is_empty()
    }
}

impl<'de> Deserialize<'de> for RiseStoredPostStats {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                views_total: container.rise_required::<i64>(key::VIEWS_TOTAL)?,
                unique_viewers: container.rise_required::<i64>(key::UNIQUE_VIEWERS)?,
                likes_total: container.rise_required::<i64>(key::LIKES_TOTAL)?,
                favorites_total: container.rise_required::<i64>(key::FAVORITES_TOTAL)?,
                comments_total: container.rise_required::<i64>(key::COMMENTS_TOTAL)?,
                shares_total: container.rise_required::<i64>(key::SHARES_TOTAL)?,
                reposts_total: container.rise_required::<i64>(key::REPOSTS_TOTAL)?,
                follows_gained: container.rise_required::<i64>(key::FOLLOWS_GAINED)?,
                watch_count: container.rise_required::<i64>(key::WATCH_COUNT)?,
                watched_ms_sum: container.rise_required::<i64>(key::WATCHED_MS_SUM)?,
                avg_dwell_ms: container.rise_required::<i64>(key::AVG_DWELL_MS)?,
                days: container.rise_required::<Vec<PostStatsDay>>(key::DAYS)?,
                countries: container.rise_required::<Vec<PostStatsCountry>>(key::COUNTRIES)?,
            })
        })
    }
}

impl Serialize for RiseStoredPostStats {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(&RiseWireKey::camel(key::VIEWS_TOTAL), &self.views_total)?;
        container.serialize_entry(&RiseWireKey::camel(key::UNIQUE_VIEWERS), &self.unique_viewers)?;
        container.serialize_entry(&RiseWireKey::camel(key::LIKES_TOTAL), &self.likes_total)?;
        container.serialize_entry(&RiseWireKey::camel(key::FAVORITES_TOTAL), &self.favorites_total)?;
        container.serialize_entry(&RiseWireKey::camel(key::COMMENTS_TOTAL), &self.comments_total)?;
        container.serialize_entry(&RiseWireKey::camel(key::SHARES_TOTAL), &self.shares_total)?;
        container.serialize_entry(&RiseWireKey::camel(key::REPOSTS_TOTAL), &self.reposts_total)?;
        container.serialize_entry(&RiseWireKey::camel(key::FOLLOWS_GAINED), &self.follows_gained)?;
        container.serialize_entry(&RiseWireKey::camel(key::WATCH_COUNT), &self.watch_count)?;
        container.serialize_entry(&RiseWireKey::camel(key::WATCHED_MS_SUM), &self.watched_ms_sum)?;
        container.serialize_entry(&RiseWireKey::camel(key::AVG_DWELL_MS), &self.avg_dwell_ms)?;
        container.serialize_entry(key::DAYS, &self.days)?;
        container.serialize_entry(key::COUNTRIES, &self.countries)?;
        container.end()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PostStatsDay {
    pub day: String,
    pub views: i64,
    pub likes: i64,
    pub comments: i64,
    pub favorites: i64,
    pub shares: i64,
    pub follows: i64,
}

impl PostStatsDay {
    pub fn id(&self) -> String {
        self.day.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.views == 0
            && self.likes == 0
            && self.comments == 0
            && self.favorites == 0
            && self.shares == 0
            && self.follows == 0
    }
}

impl<'de> Deserialize<'de> for PostStatsDay {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                day: container.rise_required::<String>(key::DAY)?,
                views: container.rise_required::<i64>(key::VIEWS)?,
                likes: container.rise_required::<i64>(key::LIKES)?,
                comments: container.rise_required::<i64>(key::COMMENTS)?,
                favorites: container.rise_required::<i64>(key::FAVORITES)?,
                shares: container.rise_required::<i64>(key::SHARES)?,
                follows: container.rise_required::<i64>(key::FOLLOWS)?,
            })
        })
    }
}

impl Serialize for PostStatsDay {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(key::DAY, &self.day)?;
        container.serialize_entry(key::VIEWS, &self.views)?;
        container.serialize_entry(key::LIKES, &self.likes)?;
        container.serialize_entry(key::COMMENTS, &self.comments)?;
        container.serialize_entry(key::FAVORITES, &self.favorites)?;
        container.serialize_entry(key::SHARES, &self.shares)?;
        container.serialize_entry(key::FOLLOWS, &self.follows)?;
        container.end()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PostStatsCountry {
    pub country: String,
    pub unique_views: i64,
}

impl PostStatsCountry {
    pub fn id(&self) -> String {
        self.country.clone()
    }
}

impl<'de> Deserialize<'de> for PostStatsCountry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                country: container.rise_required::<String>(key::COUNTRY)?,
                unique_views: container.rise_required::<i64>(key::UNIQUE_VIEWS)?,
            })
        })
    }
}

impl Serialize for PostStatsCountry {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(key::COUNTRY, &self.country)?;
        container.serialize_entry(&RiseWireKey::camel(key::UNIQUE_VIEWS), &self.unique_views)?;
        container.end()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VirtualList<Item: PartialEq> {
    pub list: Vec<Item>,
    pub total: Option<i64>,
    pub limit: i64,
    pub relative_id: Option<String>,
    pub is_have_more_bot_or_top: Option<bool>,
    pub feed_session_id: Option<String>,
}

impl<Item: PartialEq> Default for VirtualList<Item> {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            total: None,
            limit: 0,
            relative_id: None,
            is_have_more_bot_or_top: None,
            feed_session_id: None,
        }
    }
}

/// hashtags like telegram finds them
/// a # not glued to a word or link path then letters, digits and _ in any script
/// tags and tag count are cut to server limits so text full of hashtags never fails
pub enum PostHashtagParsing {}

static POST_HASHTAG_PARSING_MAXIMUM_COUNT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("max_hashtags"));

impl PostHashtagParsing {
    pub fn extract(from: &str) -> Vec<String> {
        Self::tags(from, false)
    }

    /// field tags first, a tag of digits alone only counts when typed in the field
    /// so #1 in the text stays plain text
    pub fn merge(manual: &str, content: &str) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut merged: Vec<String> = Vec::new();
        let mut found = Self::tags(manual, false);
        found.extend(Self::tags(content, true));
        for tag in found {
            if (merged.len() as i64) < *POST_HASHTAG_PARSING_MAXIMUM_COUNT && seen.insert(tag.clone()) {
                merged.push(tag);
            }
        }
        merged
    }

    fn tags(text: &str, requires_letter: bool) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        FormattedText::hashtag_runs(text)
            .into_iter()
            .filter_map(|run| {
                if (!requires_letter || run.has_letter) && seen.insert(run.tag.clone()) {
                    Some(run.tag)
                } else {
                    None
                }
            })
            .collect()
    }
}

/// the hashtags field, every word gets a # and only letters
/// digits and _ stay, a space opens the next tag
/// deleting that # takes the space with it
pub enum PostHashtagInputFormatting {}

impl PostHashtagInputFormatting {
    pub fn format(value: &str, previous: &str) -> String {
        let is_deleting = character_count(value) < character_count(previous);
        let ends_with_space = characters(value).next_back().is_some_and(|c| c.is_whitespace());
        let mut words: Vec<String> = Vec::new();
        let mut current = String::new();
        for c in characters(value) {
            if c.is_whitespace() {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            } else {
                current.push_str(c.as_str());
            }
        }
        if !current.is_empty() {
            words.push(current);
        }
        let mut tags: Vec<String> = Vec::new();
        let last_index = words.len().saturating_sub(1);
        for (index, word) in words.iter().enumerate() {
            let body: String = characters(word)
                .filter(|c| c.is_letter() || c.is_number() || c.as_str() == "_")
                .map(|c| c.as_str())
                .collect();
            if !body.is_empty() {
                tags.push(format!("#{body}"));
            } else if index == last_index && !ends_with_space && word.contains('#') {
                tags.push("#".to_owned());
            }
        }
        let formatted = tags.join(" ");
        if !ends_with_space || is_deleting || formatted.is_empty() || tags.last().map(String::as_str) == Some("#") {
            return formatted;
        }
        format!("{formatted} #")
    }
}
