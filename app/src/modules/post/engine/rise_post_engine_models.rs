use std::collections::HashMap;
use std::sync::LazyLock;

use rise_canon::rise_key::events as key;
use rise_canon::rise_proto::post::{
    GatewayCollaboratorsMutationResponse, PostCollaboratorRow as ProtoPostCollaboratorRow, UserInfo,
};
use rise_canon::rise_request;
use rise_canon::rise_response::post::{CreateRepost, DeleteRepost, SetPostFavorite, SetPostLike};
use rise_canon::{RiseWireObject, rise_decode};
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use serde::{Deserialize, Deserializer, Serialize};

use crate::core::lib::canon_identity::RiseIdentity;
use crate::modules::media_picker::api::media_picker_result::MediaPickerTrackPlate;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::post::shared::post_media_slides::PostMediaSlides;
use crate::modules::post::stores::post_actions::post_types::{
    AuthorInfo, GetPostFeedResponse, PostCollaboratorKind, PostCollaboratorRejection, PostCollaboratorRow,
    PostCollaboratorStatus, PostCollaboratorUser, PostCountVisibility, PostMediaSize, PostTagInputDraft,
    PostTaggedUser, PostTrackPlateDTO, PostViewerCollaboration, RiseStoredPostStats,
};
use crate::modules::post::stores::post_actions::post_wire_types::{GetFeedPostItemDTO, PostLinkPreviewDTO, WireID};

pub type RisePostContentKind = rise_catalog::PostContentKind;
pub type RisePostFeedKind = rise_catalog::PostFeedKind;
pub type RisePostProfileKind = rise_catalog::PostProfileKind;
pub type RisePostSearchKind = rise_catalog::PostSearchKind;
pub type RisePostPageStatus = rise_catalog::PostPageStatus;

/// kind of a post a row resolves to from what it carries
pub trait RisePostContentKindResolve: Sized {
    fn resolve(
        authoritative: Option<&str>,
        images: &[String],
        video: Option<&str>,
        is_short: bool,
        is_image_reel: bool,
        track_id: Option<&str>,
    ) -> Self;
}

impl RisePostContentKindResolve for RisePostContentKind {
    fn resolve(
        authoritative: Option<&str>,
        images: &[String],
        video: Option<&str>,
        is_short: bool,
        is_image_reel: bool,
        track_id: Option<&str>,
    ) -> Self {
        if let Some(authoritative) = authoritative
            && let Some(value) = Self::from_wire(&RiseIdentity::content_kind_authoritative(authoritative))
        {
            return value;
        }
        if RiseIdentity::has_publication_signal(images, video, track_id, is_short, is_image_reel) {
            return RisePostContentKind::Publication;
        }
        Self::from_wire(RiseIdentity::content_kind_fallback()).unwrap_or(RisePostContentKind::Thread)
    }
}

#[derive(Clone, PartialEq)]
pub struct RisePostPagePresentation {
    pub items: Vec<GetPostFeedResponse>,
    pub status: RisePostPageStatus,
    pub error_message: Option<String>,
    pub has_more: bool,
    pub is_refreshing: bool,
    pub is_loading_more: bool,
    pub cache_source: RisePostCacheSource,
    pub revision: i64,
}

impl RisePostPagePresentation {
    pub const IDLE: RisePostPagePresentation = RisePostPagePresentation {
        items: Vec::new(),
        status: RisePostPageStatus::Idle,
        error_message: None,
        has_more: true,
        is_refreshing: false,
        is_loading_more: false,
        cache_source: RisePostCacheSource::Miss,
        revision: 0,
    };
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RisePostPageScope {
    Feed(RisePostFeedKind),
    Shorts,
    Profile { user_id: String, tag: String, kind: RisePostProfileKind },
    Search { query: String, kind: RisePostSearchKind },
    Detail { post_id: String },
}

impl RisePostPageScope {
    pub fn key(&self) -> String {
        match self {
            RisePostPageScope::Feed(kind) => {
                RiseIdentity::composite_key(&["feed".to_owned(), kind.wire().to_owned()])
            }
            RisePostPageScope::Shorts => {
                RiseIdentity::composite_key(&["shorts".to_owned(), "withImageReels".to_owned()])
            }
            RisePostPageScope::Profile { user_id, tag, kind } => {
                RiseIdentity::composite_key(&[
                    "profile".to_owned(),
                    RiseIdentity::component(&RiseIdentity::scope_identifier(user_id)),
                    RiseIdentity::component(&RiseIdentity::scope_text(tag)),
                    kind.wire().to_owned(),
                ])
            }
            RisePostPageScope::Search { query, kind } => RiseIdentity::composite_key(&[
                "search".to_owned(),
                RiseIdentity::component(&Self::normalized_query(query)),
                kind.wire().to_owned(),
            ]),
            RisePostPageScope::Detail { post_id } => RiseIdentity::composite_key(&[
                "detail".to_owned(),
                RiseIdentity::component(&RiseIdentity::scope_identifier(post_id)),
            ]),
        }
    }

    pub fn normalized_query(query: &str) -> String {
        RiseIdentity::scope_text(query)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RisePostPageCursor {
    pub scope_key: String,
    pub relative_id: Option<i64>,
    pub feed_session_id: Option<String>,
    pub page: i64,
    #[serde(default)]
    pub next_ordinal: Option<i64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RisePostStoredEntity {
    pub post: GetPostFeedResponse,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RisePostLocalUpload {
    pub url: Url,
    pub filename: String,
    pub content_type: String,
}

/// on media track card placed in the editor, theme plus normalized transform
/// goes on the wire as the track_plate of the request
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RisePostTrackPlateDraft {
    pub is_dark: bool,
    pub center_x: f64,
    pub center_y: f64,
    pub rotation: f64,
    pub scale: f64,
}

impl RisePostTrackPlateDraft {
    pub fn new(picker: &MediaPickerTrackPlate) -> Self {
        Self {
            is_dark: picker.is_dark,
            center_x: picker.center_x,
            center_y: picker.center_y,
            rotation: picker.rotation,
            scale: picker.scale,
        }
    }

    pub fn as_dto(&self) -> PostTrackPlateDTO {
        PostTrackPlateDTO {
            center_x: self.center_x,
            center_y: self.center_y,
            is_dark: self.is_dark,
            rotation: self.rotation,
            scale: self.scale,
        }
    }

    pub fn create_wire(&self) -> rise_request::post::create_post::TrackPlate {
        rise_request::post::create_post::TrackPlate {
            center_x: Some(self.center_x),
            center_y: Some(self.center_y),
            is_dark: Some(self.is_dark),
            rotation: Some(self.rotation),
            scale: Some(self.scale),
        }
    }

    pub fn update_wire(&self) -> rise_request::post::update_post::TrackPlate {
        rise_request::post::update_post::TrackPlate {
            center_x: Some(self.center_x),
            center_y: Some(self.center_y),
            is_dark: Some(self.is_dark),
            rotation: Some(self.rotation),
            scale: Some(self.scale),
        }
    }
}

impl From<&PostTagInputDraft> for rise_request::post::create_post::TaggedUsersElement {
    fn from(draft: &PostTagInputDraft) -> Self {
        Self {
            placements: Some(
                draft
                    .placements
                    .iter()
                    .map(|placement| rise_request::post::create_post::tagged_users_element::PlacementsElement {
                        is_dark: Some(placement.is_dark),
                        slide_index: Some(placement.slide_index),
                        x: Some(placement.x),
                        y: Some(placement.y),
                    })
                    .collect(),
            ),
            user_id: Some(draft.user_id.clone()),
        }
    }
}

impl From<&PostTagInputDraft> for rise_request::post::update_post::TaggedUsersElement {
    fn from(draft: &PostTagInputDraft) -> Self {
        Self {
            placements: Some(
                draft
                    .placements
                    .iter()
                    .map(|placement| rise_request::post::update_post::tagged_users_element::PlacementsElement {
                        is_dark: Some(placement.is_dark),
                        slide_index: Some(placement.slide_index),
                        x: Some(placement.x),
                        y: Some(placement.y),
                    })
                    .collect(),
            ),
            user_id: Some(draft.user_id.clone()),
        }
    }
}

impl From<&PostTagInputDraft> for rise_request::post::set_post_tags::TaggedUsersElement {
    fn from(draft: &PostTagInputDraft) -> Self {
        Self {
            placements: Some(
                draft
                    .placements
                    .iter()
                    .map(|placement| rise_request::post::set_post_tags::tagged_users_element::PlacementsElement {
                        is_dark: Some(placement.is_dark),
                        slide_index: Some(placement.slide_index),
                        x: Some(placement.x),
                        y: Some(placement.y),
                    })
                    .collect(),
            ),
            user_id: Some(draft.user_id.clone()),
        }
    }
}

/// reply to every coauthorship mutation, the whole post plus whoever the server refused
/// row updates from post, rejections go to the callers toast
pub type RisePostCollaborationResponse = GatewayCollaboratorsMutationResponse;

pub trait RiseProtoPostCollaboratorRowExt {
    fn as_row(&self) -> Option<PostCollaboratorRow>;
}

impl RiseProtoPostCollaboratorRowExt for ProtoPostCollaboratorRow {
    fn as_row(&self) -> Option<PostCollaboratorRow> {
        let user = self.user.clone()?;
        Some(PostCollaboratorRow {
            user,
            kind: if self.kind.is_empty() {
                PostCollaboratorKind::Coauthor.raw_value().to_string()
            } else {
                self.kind.clone()
            },
            status: if self.status.is_empty() {
                PostCollaboratorStatus::Accepted.raw_value().to_string()
            } else {
                self.status.clone()
            },
            show_in_grid: self.show_in_grid,
            placements: self.placements.clone(),
        })
    }
}

/// what a repost create or delete acknowledged, whichever answered
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RisePostRepostAcknowledgement {
    pub is_reposted: bool,
    pub repost_count: i64,
    pub my_repost_id: Option<i64>,
    pub my_repost_content: Option<String>,
    pub my_repost_entities: Option<String>,
    pub client_request_id: Option<String>,
    pub client_session_id: Option<String>,
    pub client_sequence: Option<i64>,
}

impl From<&CreateRepost> for RisePostRepostAcknowledgement {
    fn from(reply: &CreateRepost) -> Self {
        Self {
            is_reposted: reply.is_reposted,
            repost_count: reply.repost_count,
            my_repost_id: reply.my_repost_id,
            my_repost_content: reply.my_repost_content.clone(),
            my_repost_entities: reply.my_repost_entities.clone(),
            client_request_id: reply.client_request_id.clone(),
            client_session_id: reply.client_session_id.clone(),
            client_sequence: reply.client_sequence,
        }
    }
}

impl From<&DeleteRepost> for RisePostRepostAcknowledgement {
    fn from(reply: &DeleteRepost) -> Self {
        Self {
            is_reposted: reply.is_reposted,
            repost_count: reply.repost_count,
            my_repost_id: reply.my_repost_id,
            my_repost_content: None,
            my_repost_entities: None,
            client_request_id: reply.client_request_id.clone(),
            client_session_id: reply.client_session_id.clone(),
            client_sequence: reply.client_sequence,
        }
    }
}

/// what a like or favourite write acknowledged, each reply carries only its own flag
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RisePostEngagementAcknowledgement {
    pub is_liked: Option<bool>,
    pub is_favorited: Option<bool>,
    pub likes_count: i64,
    pub favorites_count: i64,
    pub client_request_id: Option<String>,
    pub client_session_id: Option<String>,
    pub client_sequence: Option<i64>,
}

impl From<&SetPostLike> for RisePostEngagementAcknowledgement {
    fn from(reply: &SetPostLike) -> Self {
        Self {
            is_liked: Some(reply.is_liked),
            is_favorited: None,
            likes_count: reply.likes_count,
            favorites_count: reply.favorites_count,
            client_request_id: reply.client_request_id.clone(),
            client_session_id: reply.client_session_id.clone(),
            client_sequence: reply.client_sequence,
        }
    }
}

impl From<&SetPostFavorite> for RisePostEngagementAcknowledgement {
    fn from(reply: &SetPostFavorite) -> Self {
        Self {
            is_liked: None,
            is_favorited: Some(reply.is_favorited),
            likes_count: reply.likes_count,
            favorites_count: reply.favorites_count,
            client_request_id: reply.client_request_id.clone(),
            client_session_id: reply.client_session_id.clone(),
            client_sequence: reply.client_sequence,
        }
    }
}

/// coauthor and tag ceilings
/// values come from canon by limit name, the client has no number of its own
pub enum RisePostCollaboratorLimits {}

static MAXIMUM_COAUTHORS: LazyLock<i64> = LazyLock::new(|| RisePostCollaboratorLimits::value("post_service_max_coauthors"));
static MAXIMUM_TAGGED_USERS: LazyLock<i64> =
    LazyLock::new(|| RisePostCollaboratorLimits::value("post_service_max_tagged_users"));
static MAXIMUM_TAG_PLACEMENTS: LazyLock<i64> =
    LazyLock::new(|| RisePostCollaboratorLimits::value("post_service_max_tag_placements"));

impl RisePostCollaboratorLimits {
    pub fn maximum_coauthors() -> i64 {
        *MAXIMUM_COAUTHORS
    }

    pub fn maximum_tagged_users() -> i64 {
        *MAXIMUM_TAGGED_USERS
    }

    pub fn maximum_tag_placements() -> i64 {
        *MAXIMUM_TAG_PLACEMENTS
    }

    fn value(name: &str) -> i64 {
        let Some(spec) = rise_limits::SERVICE_LIMITS.iter().find(|spec| spec.name == name) else {
            if cfg!(debug_assertions) {
                panic!("Canon service limit {name} is missing");
            }
            return 0;
        };
        rise_limits::signed(spec.value)
    }
}

/// post_collaborators_changed
/// coauthors and tags as this reader sees them
/// every field decodes leniently so an odd shape in one cant wipe the other five
#[derive(Clone)]
pub struct RisePostCollaboratorsChangedEvent {
    pub post_id: Option<WireID>,
    pub author_id: Option<String>,
    pub coauthors: Option<Vec<PostCollaboratorUser>>,
    pub coauthors_count: Option<i64>,
    pub pending_coauthors_count: Option<i64>,
    pub my_collaboration: Option<PostViewerCollaboration>,
    pub tagged_users: Option<Vec<PostTaggedUser>>,
    pub tagged_users_count: Option<i64>,
    pub updated_at_milliseconds: Option<i64>,
}

impl<'de> Deserialize<'de> for RisePostCollaboratorsChangedEvent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                post_id: c.rise::<WireID>(key::POST_ID).ok().flatten(),
                author_id: c.rise::<String>(key::AUTHOR_ID).ok().flatten(),
                coauthors: c.rise::<Vec<PostCollaboratorUser>>(key::COAUTHORS).ok().flatten(),
                coauthors_count: c.rise::<i64>(key::COAUTHORS_COUNT).ok().flatten(),
                pending_coauthors_count: c.rise::<i64>(key::PENDING_COAUTHORS_COUNT).ok().flatten(),
                my_collaboration: c.rise::<PostViewerCollaboration>(key::MY_COLLABORATION).ok().flatten(),
                tagged_users: c.rise::<Vec<PostTaggedUser>>(key::TAGGED_USERS).ok().flatten(),
                tagged_users_count: c.rise::<i64>(key::TAGGED_USERS_COUNT).ok().flatten(),
                updated_at_milliseconds: c.rise::<i64>(key::UPDATED_AT_MS).ok().flatten(),
            })
        })
    }
}

#[derive(Clone, PartialEq)]
pub struct PostCollaboratorsSnapshot {
    pub items: Vec<PostCollaboratorRow>,
    pub has_more: bool,
    pub is_loading: bool,
    pub loaded_once: bool,
}

impl PostCollaboratorsSnapshot {
    pub const EMPTY: PostCollaboratorsSnapshot =
        PostCollaboratorsSnapshot { items: Vec::new(), has_more: false, is_loading: false, loaded_once: false };
}

#[derive(Clone)]
pub struct RisePostCreateOutcome {
    pub post: GetPostFeedResponse,
    pub rejected: Vec<PostCollaboratorRejection>,
}

/// where a publication is listed
/// a riff is the same row with a second placement, normalised like the server stores it
/// thread is never a riff and a riff needs media
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RisePostPlacement {
    pub is_riff: bool,
    pub show_in_publications: bool,
}

impl RisePostPlacement {
    pub const PUBLICATION: RisePostPlacement = RisePostPlacement { is_riff: false, show_in_publications: true };

    pub fn resolve(
        content_kind: RisePostContentKind,
        requested_riff: bool,
        requested_publications: bool,
        has_video: bool,
        image_count: i64,
    ) -> RisePostPlacement {
        if !(content_kind == RisePostContentKind::Publication && requested_riff && (has_video || image_count > 0)) {
            return Self::PUBLICATION;
        }
        RisePostPlacement { is_riff: true, show_in_publications: requested_publications }
    }

    /// legacy pair old servers still read
    /// a riff with a video is a short, a photo only riff is an image reel
    pub fn legacy_is_short(&self, has_video: bool) -> bool {
        self.is_riff && has_video
    }

    pub fn legacy_is_image_reel(&self, has_video: bool, image_count: i64) -> bool {
        self.is_riff && !has_video && image_count > 0
    }
}

impl GetPostFeedResponse {
    pub fn placement(&self) -> RisePostPlacement {
        RisePostPlacement { is_riff: self.is_riff, show_in_publications: self.show_in_publications }
    }
}

#[derive(Clone)]
pub struct RisePostCreateDraft {
    pub content_kind: RisePostContentKind,
    pub title: String,
    pub content: String,
    pub hashtags: Vec<String>,
    pub can_comment: bool,
    pub local_uploads: Vec<RisePostLocalUpload>,
    pub local_video: Option<RisePostLocalUpload>,
    pub video_position: Option<i64>,
    pub requests_riff: bool,
    pub requests_publications: bool,
    pub achievement_id: Option<i64>,
    pub track_id: Option<String>,
    pub track: Option<RiseStoredMusicTrack>,
    pub track_start_milliseconds: Option<i64>,
    pub track_end_milliseconds: Option<i64>,
    pub track_plate: Option<RisePostTrackPlateDraft>,
    pub audio_pre_baked: bool,
    pub author: AuthorInfo,
    pub count_visibility: PostCountVisibility,
    pub coauthor_ids: Vec<String>,
    pub tagged_users: Vec<PostTagInputDraft>,
    pub map_journey_id: Option<String>,
    pub map_journey_show_heat: bool,
}

impl RisePostCreateDraft {
    pub fn placement(&self) -> RisePostPlacement {
        RisePostPlacement::resolve(
            self.content_kind,
            self.requests_riff,
            self.requests_publications,
            self.local_video.is_some(),
            self.local_uploads.len() as i64,
        )
    }

    pub fn is_short(&self) -> bool {
        self.placement().legacy_is_short(self.local_video.is_some())
    }

    pub fn is_image_reel(&self) -> bool {
        self.placement().legacy_is_image_reel(self.local_video.is_some(), self.local_uploads.len() as i64)
    }

    pub fn resolved_video_position(&self) -> Option<i64> {
        if self.local_video.is_none() {
            return None;
        }
        Some(PostMediaSlides::clamped_video_position(self.video_position, self.local_uploads.len() as i64))
    }
}

#[derive(Clone)]
pub struct RisePostUpdateDraft {
    pub post_id: String,
    pub current_post: GetPostFeedResponse,
    pub content_kind: RisePostContentKind,
    pub title: String,
    pub content: String,
    pub hashtags: Vec<String>,
    pub can_comment: bool,
    pub track_id: Option<String>,
    pub track: Option<RiseStoredMusicTrack>,
    pub track_start_milliseconds: Option<i64>,
    pub track_end_milliseconds: Option<i64>,
    pub track_plate: Option<RisePostTrackPlateDraft>,
    pub count_visibility: PostCountVisibility,
    pub is_riff: Option<bool>,
    pub show_in_publications: Option<bool>,
    pub media: Option<RisePostMediaEdit>,
    pub tagged_users: Option<Vec<PostTagInputDraft>>,
}

impl RisePostUpdateDraft {
    pub fn changes_placement(&self) -> bool {
        self.is_riff.is_some() || self.show_in_publications.is_some()
    }

    /// placement the edit asks for, resolved against the media the post will carry
    /// the edited set if there is one, else the stored images and video
    pub fn placement(&self, stored: &GetPostFeedResponse) -> RisePostPlacement {
        RisePostPlacement::resolve(
            self.content_kind,
            self.is_riff.unwrap_or(stored.is_riff),
            self.show_in_publications.unwrap_or(stored.show_in_publications),
            self.media.as_ref().map(RisePostMediaEdit::has_video).unwrap_or_else(|| stored.has_video_media()),
            self.media.as_ref().map(RisePostMediaEdit::image_count).unwrap_or(stored.images.len() as i64),
        )
    }
}

/// one slide of an edited post in its new order
/// media already on the post travels by url, new media is a local file uploaded before the update
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RisePostEditMediaItem {
    RemoteImage { url: String, size: Option<PostMediaSize> },
    RemoteVideo { url: String, thumbnail: Option<String>, size: Option<PostMediaSize> },
    LocalImage(RisePostLocalUpload),
    LocalVideo { upload: RisePostLocalUpload, audio_pre_baked: bool },
}

impl RisePostEditMediaItem {
    pub fn is_video(&self) -> bool {
        match self {
            RisePostEditMediaItem::RemoteVideo { .. } | RisePostEditMediaItem::LocalVideo { .. } => true,
            RisePostEditMediaItem::RemoteImage { .. } | RisePostEditMediaItem::LocalImage(_) => false,
        }
    }

    pub fn local_upload(&self) -> Option<&RisePostLocalUpload> {
        match self {
            RisePostEditMediaItem::LocalImage(file) | RisePostEditMediaItem::LocalVideo { upload: file, .. } => {
                Some(file)
            }
            RisePostEditMediaItem::RemoteImage { .. } | RisePostEditMediaItem::RemoteVideo { .. } => None,
        }
    }
}

/// media set an edit asks for, normalised like the server stores it
/// at most one video, which takes the slide it was placed on
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RisePostMediaEdit {
    items: Vec<RisePostEditMediaItem>,
}

impl RisePostMediaEdit {
    pub fn new(items: Vec<RisePostEditMediaItem>) -> Self {
        let mut seen_video = false;
        let items = items
            .into_iter()
            .filter(|item| {
                if !item.is_video() {
                    return true;
                }
                let keep = !seen_video;
                seen_video = true;
                keep
            })
            .collect();
        Self { items }
    }

    pub fn items(&self) -> &[RisePostEditMediaItem] {
        &self.items
    }

    pub fn image_count(&self) -> i64 {
        self.items.len() as i64 - i64::from(self.has_video())
    }

    pub fn has_video(&self) -> bool {
        self.items.iter().any(RisePostEditMediaItem::is_video)
    }

    /// the videos slide, which is the number of photos placed before it
    pub fn video_position(&self) -> Option<i64> {
        self.items.iter().position(RisePostEditMediaItem::is_video).map(|index| index as i64)
    }

    pub fn local_images(&self) -> Vec<RisePostLocalUpload> {
        self.items
            .iter()
            .filter_map(|item| match item {
                RisePostEditMediaItem::LocalImage(file) => Some(file.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn local_video(&self) -> Option<RisePostLocalUpload> {
        self.items.iter().find_map(|item| match item {
            RisePostEditMediaItem::LocalVideo { upload, .. } => Some(upload.clone()),
            _ => None,
        })
    }

    pub fn has_local_uploads(&self) -> bool {
        self.items.iter().any(|item| item.local_upload().is_some())
    }

    pub fn slide_keys(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|item| match item {
                RisePostEditMediaItem::RemoteImage { url, .. } | RisePostEditMediaItem::RemoteVideo { url, .. } => {
                    trimming_whitespaces_and_newlines(url).to_owned()
                }
                RisePostEditMediaItem::LocalImage(file) | RisePostEditMediaItem::LocalVideo { upload: file, .. } => {
                    file.url.absolute_string().to_owned()
                }
            })
            .collect()
    }

    pub fn keeps_media(&self, of: &GetPostFeedResponse) -> bool {
        !self.has_local_uploads() && self.slide_keys() == of.media_slide_keys()
    }

    /// writes the edited set into a row the way it is shown before the upload
    pub fn apply(&self, to: &mut GetPostFeedResponse) {
        let mut images: Vec<String> = Vec::with_capacity(self.items.len());
        let mut image_sizes: Vec<PostMediaSize> = Vec::with_capacity(self.items.len());
        let mut video: Option<String> = None;
        let mut video_thumbnail: Option<String> = None;
        let mut video_size: Option<PostMediaSize> = None;
        let mut local_video_url: Option<String> = None;
        let mut audio_pre_baked: Option<bool> = None;
        for item in &self.items {
            match item {
                RisePostEditMediaItem::RemoteImage { url, size } => {
                    images.push(url.clone());
                    image_sizes.push(size.clone().unwrap_or_default());
                }
                RisePostEditMediaItem::LocalImage(file) => {
                    images.push(file.url.absolute_string().to_owned());
                    image_sizes.push(PostMediaSize::default());
                }
                RisePostEditMediaItem::RemoteVideo { url, thumbnail, size } => {
                    video = Some(url.clone());
                    video_thumbnail = thumbnail.clone();
                    video_size = size.clone();
                }
                RisePostEditMediaItem::LocalVideo { upload, audio_pre_baked: pre_baked } => {
                    video = Some(upload.url.absolute_string().to_owned());
                    local_video_url = Some(upload.url.absolute_string().to_owned());
                    audio_pre_baked = Some(*pre_baked);
                }
            }
        }
        let video_changed = video.as_deref().map(trimming_whitespaces_and_newlines)
            != to.video.as_deref().map(trimming_whitespaces_and_newlines);
        to.images = images;
        to.image_sizes = image_sizes;
        to.video = video;
        to.video_thumbnail = video_thumbnail;
        to.video_size = video_size;
        to.video_position = self.video_position();
        to.local_preview_url = local_video_url;
        if video_changed {
            to.audio_baked = audio_pre_baked.unwrap_or(false);
        }
    }
}

/// what an edit sends once every new file is uploaded
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RisePostResolvedMediaEdit {
    pub images: Vec<String>,
    pub video: Option<String>,
    pub video_thumbnail: Option<String>,
    pub video_position: Option<i64>,
}

#[derive(Clone)]
pub struct RisePostUpdateOutcome {
    pub post: GetPostFeedResponse,
    pub rejected: Vec<PostCollaboratorRejection>,
}

#[derive(Clone)]
pub struct RisePostLinkPreviewEvent {
    pub post_id: Option<WireID>,
    pub id: Option<WireID>,
    pub content_kind: Option<String>,
    pub link_preview: Option<PostLinkPreviewDTO>,
}

impl<'de> Deserialize<'de> for RisePostLinkPreviewEvent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                post_id: c.rise::<WireID>(key::POST_ID)?,
                id: c.rise::<WireID>(key::ID)?,
                content_kind: c.rise::<String>(key::CONTENT_KIND)?,
                link_preview: c.rise::<PostLinkPreviewDTO>(key::LINK_PREVIEW)?,
            })
        })
    }
}

/// post_media_ready
/// server mixed audio into the video or pulled the original track out
/// only signal that ends the processing state since the ffmpeg job reports no progress
#[derive(Clone)]
pub struct RisePostMediaReadyEvent {
    pub post_id: Option<WireID>,
    pub video: Option<String>,
    pub track: Option<RiseStoredMusicTrack>,
    pub audio_baked: Option<bool>,
    pub media_status: Option<String>,
}

impl<'de> Deserialize<'de> for RisePostMediaReadyEvent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                post_id: c.rise::<WireID>(key::POST_ID).ok().flatten(),
                video: c.rise::<String>(key::VIDEO).ok().flatten(),
                track: c.rise::<RiseStoredMusicTrack>(key::TRACK).ok().flatten(),
                audio_baked: c.rise::<bool>(key::AUDIO_BAKED).ok().flatten(),
                media_status: c.rise::<String>(key::MEDIA_STATUS).ok().flatten(),
            })
        })
    }
}

#[derive(Clone)]
pub struct RisePostCreatedEvent {
    pub post: Option<GetFeedPostItemDTO>,
    pub client_request_id: Option<String>,
}

impl<'de> Deserialize<'de> for RisePostCreatedEvent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self { post: rise_post_event_card(c)?, client_request_id: c.rise::<String>(key::CLIENT_REQUEST_ID)? })
        })
    }
}

/// authors copy of a row level change made on their behalf
/// post_deleted carries the id, post_updated the reloaded row
#[derive(Clone)]
pub struct RisePostAuthorEvent {
    pub post_id: Option<WireID>,
    pub post: Option<GetFeedPostItemDTO>,
}

impl<'de> Deserialize<'de> for RisePostAuthorEvent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self { post_id: c.rise::<WireID>(key::POST_ID)?, post: rise_post_event_card(c)? })
        })
    }
}

struct RisePostEventCardAuthorShape {
    more: Option<UserInfo>,
}

impl<'de> Deserialize<'de> for RisePostEventCardAuthorShape {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            let author = c.rise::<RisePostEventCardAuthorShapeAuthorShape>(key::AUTHOR).ok().flatten();
            Ok(Self { more: author.and_then(|author| author.more) })
        })
    }
}

struct RisePostEventCardAuthorShapeAuthorShape {
    more: Option<UserInfo>,
}

impl<'de> Deserialize<'de> for RisePostEventCardAuthorShapeAuthorShape {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| Ok(Self { more: c.rise::<UserInfo>(key::MORE).ok().flatten() }))
    }
}

fn rise_post_event_card(c: &RiseWireObject) -> Result<Option<GetFeedPostItemDTO>, serde_json::Error> {
    let Some(mut post) = c.rise::<GetFeedPostItemDTO>(key::POST)? else {
        return Ok(None);
    };
    let Some(mut author) = post.author.clone() else {
        return Ok(Some(post));
    };
    let Some(more) = c.rise::<RisePostEventCardAuthorShape>(key::POST).ok().flatten().and_then(|shape| shape.more)
    else {
        return Ok(Some(post));
    };
    if author.logo.is_empty() {
        author.logo = more.logo;
    }
    if author.role.is_empty() {
        author.role = more.role;
    }
    if author.who.is_empty() {
        author.who = more.who;
    }
    if author.p_lang.is_empty() {
        author.p_lang = more.p_lang;
    }
    author.is_premium = author.is_premium || more.is_premium;
    author.is_official = author.is_official || more.is_official;
    author.premium_badge_emoji = author.premium_badge_emoji.or(more.premium_badge_emoji);
    author.name_color = author.name_color.or(more.name_color);
    post.author = Some(author);
    Ok(Some(post))
}

pub type RisePostViewSurface = rise_catalog::PostViewSurface;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RisePostVisibilityIntent {
    pub post_id: i64,
    pub surface: RisePostViewSurface,
    pub scope_id: Uuid,
    pub scope_generation: u64,
    pub account_id: String,
    pub account_epoch: String,
}

pub type RisePostCacheSource = rise_catalog::PostCacheSource;

#[derive(Clone, PartialEq)]
pub struct RisePostStatsPresentationState {
    pub value: Option<RiseStoredPostStats>,
    pub is_loading: bool,
    pub error_message: Option<String>,
    pub source: RisePostCacheSource,
    pub updated_at_milliseconds: i64,
}

#[derive(Clone, PartialEq)]
pub struct RisePostPresentationSnapshot {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub revision: i64,
    pub stats_by_post_id: HashMap<String, RisePostStatsPresentationState>,
    pub view_counts_by_post_id: HashMap<String, i64>,
}

static PRESENTATION_SNAPSHOT_EMPTY: LazyLock<RisePostPresentationSnapshot> =
    LazyLock::new(|| RisePostPresentationSnapshot {
        account_id: String::new(),
        account_epoch: String::new(),
        session_id: Uuid::new_v4(),
        revision: 0,
        stats_by_post_id: HashMap::new(),
        view_counts_by_post_id: HashMap::new(),
    });

impl RisePostPresentationSnapshot {
    pub fn empty() -> RisePostPresentationSnapshot {
        PRESENTATION_SNAPSHOT_EMPTY.clone()
    }
}

#[derive(Clone, PartialEq)]
pub struct RisePostContentPresentationSnapshot {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub revision: i64,
    pub pages_by_scope_key: HashMap<String, RisePostPagePresentation>,
    pub collaborators_by_post_id: HashMap<String, PostCollaboratorsSnapshot>,
}

static CONTENT_PRESENTATION_SNAPSHOT_EMPTY: LazyLock<RisePostContentPresentationSnapshot> =
    LazyLock::new(|| RisePostContentPresentationSnapshot {
        account_id: String::new(),
        account_epoch: String::new(),
        session_id: Uuid::new_v4(),
        revision: 0,
        pages_by_scope_key: HashMap::new(),
        collaborators_by_post_id: HashMap::new(),
    });

impl RisePostContentPresentationSnapshot {
    pub fn empty() -> RisePostContentPresentationSnapshot {
        CONTENT_PRESENTATION_SNAPSHOT_EMPTY.clone()
    }
}

#[derive(Clone)]
pub struct RisePostViewRecordResult {
    pub handled: bool,
    pub snapshot: Option<RisePostPresentationSnapshot>,
}

impl GetPostFeedResponse {
    pub fn rise_view_target_post_id(&self) -> String {
        match int_from_string(&self.id) {
            Some(value) if value > 0 => value.to_string(),
            _ => self.id.clone(),
        }
    }
}
