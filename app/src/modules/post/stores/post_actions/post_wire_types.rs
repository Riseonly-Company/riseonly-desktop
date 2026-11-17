use rise_canon::rise_integer;
use rise_canon::rise_proto::post::{
    GatewayGetFeedResponse, GatewayGetPostsByTrackResponse, GatewayGetShortsFeedResponse,
    GatewayGetUserPostsResponse, GatewaySearchPostsByHashtagResponse, GatewaySearchPostsResponse,
    GetUserRepostsGatewayResponse, GetViewHistoryGatewayResponse, PostLikedByUser as CanonPostLikedByUser,
    PostLinkPreview as CanonPostLinkPreview, PostResponseWithExtras, UserInfo,
};
use rise_canon::rise_response::post::{CreatePost, GetFeed, GetPostStats, RecordPostLinkOpen, RecordPostShare};
use rise_core::foundation::foundation_character::int_from_string;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::comment::stores::comment_actions::comment_types::CommentEntities;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::post::engine::rise_post_engine_models::{RisePostContentKind, RisePostContentKindResolve};
use crate::modules::post::stores::post_actions::post_types::{
    AuthorInfo, AuthorInfoMore, GetPostFeedResponse, PostCountVisibility, PostLikedByUser, PostLinkPreview,
    PostStatsCountry, PostStatsDay, RiseStoredPostStats, VirtualList,
};

/// an id the gateway writes as a json string or a number
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum WireID {
    String(String),
    Int(i64),
}

impl WireID {
    pub fn as_string(&self) -> String {
        match self {
            WireID::String(s) => s.clone(),
            WireID::Int(i) => i.to_string(),
        }
    }
}

impl Serialize for WireID {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            WireID::String(s) => serializer.serialize_str(s),
            WireID::Int(i) => serializer.serialize_i64(*i),
        }
    }
}

impl<'de> Deserialize<'de> for WireID {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) => Ok(WireID::String(s)),
            serde_json::Value::Number(number) => match rise_integer(&number) {
                Some(i) => Ok(WireID::Int(i)),
                None => Err(serde::de::Error::custom("Expected String or Int for id")),
            },
            _ => Err(serde::de::Error::custom("Expected String or Int for id")),
        }
    }
}

pub type GetFeedPostItemDTO = PostResponseWithExtras;
pub type PostLinkPreviewDTO = CanonPostLinkPreview;
pub type PostLikedByUserDTO = CanonPostLikedByUser;

pub trait PostLikedByUserDTOExt {
    fn as_domain(&self) -> Option<PostLikedByUser>;
}

impl PostLikedByUserDTOExt for PostLikedByUserDTO {
    fn as_domain(&self) -> Option<PostLikedByUser> {
        if self.id.is_empty() {
            return None;
        }
        Some(PostLikedByUser {
            id: self.id.clone(),
            name: self.name.clone(),
            tag: self.tag.clone(),
            logo: self.logo.clone(),
            is_premium: self.is_premium,
            is_official: self.is_official,
            is_friend: self.is_friend,
        })
    }
}

pub type GetViewHistoryGatewayDTO = GetViewHistoryGatewayResponse;
/// generic post page for the response caches, every post page reply of the canon decodes into it
pub type GetFeedGatewayDTO = GetFeed;
pub type CreatePostGatewayDTO = CreatePost;
pub type RecordPostShareGatewayDTO = RecordPostShare;
pub type RecordPostLinkOpenGatewayDTO = RecordPostLinkOpen;
pub type PostStatsGatewayDTO = GetPostStats;

pub enum PostWirePostId {}

impl PostWirePostId {
    /// post id as the request body carries it, a number when the id is numeric
    pub fn outbound(id: &str) -> serde_json::Value {
        if let Some(n) = int_from_string(id) {
            return serde_json::Value::from(n);
        }
        serde_json::Value::from(id)
    }
}

/// converts the feed item dto to a feed response
pub trait GetFeedPostItemDTOExt {
    fn as_feed_response(&self) -> Option<GetPostFeedResponse>;
}

impl GetFeedPostItemDTOExt for GetFeedPostItemDTO {
    fn as_feed_response(&self) -> Option<GetPostFeedResponse> {
        let mut author = self.author.as_ref().map(UserInfoExt::as_author_info).unwrap_or_default();
        if author.id.as_deref().is_none_or(str::is_empty) {
            author.id = Some(self.author_id.clone());
        }
        let liked = self.is_liked.unwrap_or(false);
        let favorited = self.is_favorited.unwrap_or(false);
        Some(GetPostFeedResponse {
            id: self.id.to_string(),
            title: self.title.clone(),
            content: self.content.clone(),
            original_content: self.original_content.clone(),
            hashtags: self.hashtags.clone(),
            tags: self.tags.clone(),
            views_count: self.views_count,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            author_id: self.author_id.clone(),
            main_comments_count: self.main_comments_count,
            comments_count: self.comments_count,
            can_comment: self.can_comment,
            likes_count: self.likes_count,
            favorites_count: self.favorites_count,
            images: self.images.clone(),
            image_sizes: self.image_sizes.clone(),
            is_liked: liked,
            is_favorited: favorited,
            old_is_liked: liked,
            old_is_favorited: favorited,
            author,
            video: RemoteMediaURL::optional_normalized(self.video.as_deref()),
            video_thumbnail: RemoteMediaURL::optional_normalized(self.video_thumbnail.as_deref()),
            video_size: self.video_size.clone(),
            is_short: self.is_short,
            is_image_reel: self.is_image_reel,
            is_riff: self.is_riff.unwrap_or(self.is_short || self.is_image_reel),
            show_in_publications: self.show_in_publications.unwrap_or(true),
            video_position: self.video_position.and_then(|value| int_from_string(&value.to_string())),
            achievement_id: self.achievement_id.and_then(|value| int_from_string(&value.to_string())),
            map_journey_id: self.map_journey_id.clone(),
            track_id: self.track_id.clone(),
            track: self.track.as_ref().map(RiseStoredMusicTrack::from_canon),
            track_start_ms: self.track_start_ms,
            track_end_ms: self.track_end_ms,
            track_plate: self.track_plate.clone(),
            link_areas: self.link_areas.clone(),
            audio_baked: self.audio_baked,
            media_status: self.media_status.clone(),
            moderation_status: self.moderation_status.clone(),
            repost_count: self.repost_count,
            is_reposted: self.is_reposted.unwrap_or(false),
            my_repost_id: self.my_repost_id.map(|value| value.to_string()),
            my_repost_content: self.my_repost_content.clone(),
            liked_by: self.liked_by.iter().filter_map(PostLikedByUserDTOExt::as_domain).collect(),
            coauthors: self.coauthors.clone(),
            coauthors_count: self.coauthors_count,
            pending_coauthors_count: self.pending_coauthors_count,
            my_collaboration: self.my_collaboration.clone(),
            tagged_users: self.tagged_users.clone(),
            tagged_users_count: self.tagged_users_count,
            content_kind: RisePostContentKind::resolve(
                Some(self.content_kind.as_str()),
                &self.images,
                self.video.as_deref(),
                self.is_short,
                self.is_image_reel,
                self.track_id.as_deref(),
            )
            .wire()
            .to_owned(),
            link_preview: self.link_preview.as_ref().map(PostLinkPreviewDTOExt::as_domain),
            created_at_ms: Some(self.created_at_ms),
            updated_at_ms: Some(self.updated_at_ms),
            count_visibility: PostCountVisibility {
                likes: self.show_likes_count,
                comments: self.show_comments_count,
                reposts: self.show_reposts_count,
                shares: self.show_shares_count,
                comment_dislikes: self.show_comment_dislikes,
                sparks: self.show_sparks,
                allows_sparks: self.allow_sparks,
                allows_download: self.allow_download.unwrap_or(true),
                allows_download_without_watermark: self.allow_download_without_watermark.unwrap_or(false),
            },
            share_count: self.share_count,
            link_open_count: self.link_open_count,
            sparks_total: self.sparks_total.max(0),
            my_sparks: self.my_sparks.max(0),
            my_repost_entities: CommentEntities::normalized(self.my_repost_entities.as_deref()).0,
            reposter_content: self.reposter_content.clone(),
            reposter_entities: CommentEntities::normalized(self.reposter_entities.as_deref()).0,
            ..GetPostFeedResponse::default()
        })
    }
}

/// converts the link preview dto to the domain model
pub trait PostLinkPreviewDTOExt {
    fn as_domain(&self) -> PostLinkPreview;
}

impl PostLinkPreviewDTOExt for PostLinkPreviewDTO {
    fn as_domain(&self) -> PostLinkPreview {
        PostLinkPreview {
            url: self.url.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            image_url: self.image_url.clone(),
            status: self.status.clone(),
            revision: self.revision,
        }
    }
}

pub type GetFeedAuthorDTO = UserInfo;

pub trait UserInfoExt {
    fn as_author_info(&self) -> AuthorInfo;
}

impl UserInfoExt for UserInfo {
    fn as_author_info(&self) -> AuthorInfo {
        AuthorInfo {
            id: Some(self.id.clone()),
            name: self.name.clone(),
            tag: self.tag.clone(),
            more: AuthorInfoMore {
                p_lang: self.p_lang.clone(),
                who: self.who.clone(),
                role: self.role.clone(),
                logo: self.logo.clone(),
                is_premium: self.is_premium,
                is_official: self.is_official,
                phone: None,
                premium_badge_emoji: self.premium_badge_emoji.clone(),
                name_color: self.name_color.clone(),
            },
            is_subscribed: self.is_subscribed,
            is_subscriber: self.is_subscriber,
            is_friend: self.is_friend,
            is_deleted: self.is_deleted,
            friend_request_id: self.friend_request_id,
        }
    }
}

/// fields every post page reply of the canon shares
pub trait RisePostWirePage: Send + Sync {
    fn list(&self) -> &[PostResponseWithExtras];
    fn is_have_more(&self) -> bool;
    fn relative_id(&self) -> Option<i64>;
    fn limit(&self) -> i64;
    fn page_total(&self) -> i64;

    fn page_feed_session_id(&self) -> Option<String> {
        None
    }

    fn as_virtual_list(&self) -> VirtualList<GetPostFeedResponse> {
        VirtualList {
            list: self.list().iter().filter_map(GetFeedPostItemDTOExt::as_feed_response).collect(),
            total: Some(self.page_total()),
            limit: self.limit(),
            relative_id: self.relative_id().map(|value| value.to_string()),
            is_have_more_bot_or_top: Some(self.is_have_more()),
            feed_session_id: self.page_feed_session_id(),
        }
    }
}

impl RisePostWirePage for GatewayGetFeedResponse {
    fn list(&self) -> &[PostResponseWithExtras] {
        &self.list
    }
    fn is_have_more(&self) -> bool {
        self.is_have_more
    }
    fn relative_id(&self) -> Option<i64> {
        self.relative_id
    }
    fn limit(&self) -> i64 {
        self.limit
    }
    fn page_total(&self) -> i64 {
        self.total
    }
    fn page_feed_session_id(&self) -> Option<String> {
        self.feed_session_id.clone()
    }
}

impl RisePostWirePage for GatewayGetUserPostsResponse {
    fn list(&self) -> &[PostResponseWithExtras] {
        &self.list
    }
    fn is_have_more(&self) -> bool {
        self.is_have_more
    }
    fn relative_id(&self) -> Option<i64> {
        self.relative_id
    }
    fn limit(&self) -> i64 {
        self.limit
    }
    fn page_total(&self) -> i64 {
        self.total
    }
    fn page_feed_session_id(&self) -> Option<String> {
        self.feed_session_id.clone()
    }
}

impl RisePostWirePage for GatewayGetShortsFeedResponse {
    fn list(&self) -> &[PostResponseWithExtras] {
        &self.list
    }
    fn is_have_more(&self) -> bool {
        self.is_have_more
    }
    fn relative_id(&self) -> Option<i64> {
        self.relative_id
    }
    fn limit(&self) -> i64 {
        self.limit
    }
    fn page_total(&self) -> i64 {
        self.total
    }
}

impl RisePostWirePage for GatewayGetPostsByTrackResponse {
    fn list(&self) -> &[PostResponseWithExtras] {
        &self.list
    }
    fn is_have_more(&self) -> bool {
        self.is_have_more
    }
    fn relative_id(&self) -> Option<i64> {
        self.relative_id
    }
    fn limit(&self) -> i64 {
        self.limit
    }
    fn page_total(&self) -> i64 {
        self.total
    }
}

impl RisePostWirePage for GatewaySearchPostsResponse {
    fn list(&self) -> &[PostResponseWithExtras] {
        &self.list
    }
    fn is_have_more(&self) -> bool {
        self.is_have_more
    }
    fn relative_id(&self) -> Option<i64> {
        self.relative_id
    }
    fn limit(&self) -> i64 {
        self.limit
    }
    fn page_total(&self) -> i64 {
        self.total
    }
}

impl RisePostWirePage for GatewaySearchPostsByHashtagResponse {
    fn list(&self) -> &[PostResponseWithExtras] {
        &self.list
    }
    fn is_have_more(&self) -> bool {
        self.is_have_more
    }
    fn relative_id(&self) -> Option<i64> {
        self.relative_id
    }
    fn limit(&self) -> i64 {
        self.limit
    }
    fn page_total(&self) -> i64 {
        self.total
    }
}

impl RisePostWirePage for GetUserRepostsGatewayResponse {
    fn list(&self) -> &[PostResponseWithExtras] {
        &self.list
    }
    fn is_have_more(&self) -> bool {
        self.is_have_more
    }
    fn relative_id(&self) -> Option<i64> {
        self.relative_id
    }
    fn limit(&self) -> i64 {
        self.limit
    }
    fn page_total(&self) -> i64 {
        self.total
    }
}

pub trait PostStatsGatewayDTOExt {
    fn as_domain(&self) -> RiseStoredPostStats;
}

impl PostStatsGatewayDTOExt for PostStatsGatewayDTO {
    fn as_domain(&self) -> RiseStoredPostStats {
        let s = self.stats.as_ref();
        RiseStoredPostStats {
            views_total: s.map_or(0, |s| s.views_total),
            unique_viewers: s.map_or(0, |s| s.unique_viewers),
            likes_total: s.map_or(0, |s| s.likes_total),
            favorites_total: s.map_or(0, |s| s.favorites_total),
            comments_total: s.map_or(0, |s| s.comments_total),
            shares_total: s.map_or(0, |s| s.shares_total),
            reposts_total: s.map_or(0, |s| s.reposts_total),
            follows_gained: s.map_or(0, |s| s.follows_gained),
            watch_count: s.map_or(0, |s| s.watch_count),
            watched_ms_sum: s.map_or(0, |s| s.watched_ms_sum),
            avg_dwell_ms: s.map_or(0, |s| s.avg_dwell_ms),
            days: s
                .map(|s| s.days.as_slice())
                .unwrap_or_default()
                .iter()
                .map(|day| PostStatsDay {
                    day: day.day.clone(),
                    views: day.views,
                    likes: day.likes,
                    comments: day.comments,
                    favorites: day.favorites,
                    shares: day.shares,
                    follows: day.follows,
                })
                .collect(),
            countries: s
                .map(|s| s.countries.as_slice())
                .unwrap_or_default()
                .iter()
                .map(|country| PostStatsCountry {
                    country: country.country.clone(),
                    unique_views: country.unique_views,
                })
                .collect(),
        }
    }
}
