use std::time::Duration;

use rise_engine::MethodDescriptor;
use serde::{Deserialize, Serialize};

pub const GET_FEED: MethodDescriptor =
    MethodDescriptor::read("post", "get_feed").with_timeout(Duration::from_secs(30));
pub const GET_PUBLICATIONS_FEED: MethodDescriptor =
    MethodDescriptor::read("post", "get_publications_feed").with_timeout(Duration::from_secs(30));
pub const GET_THREADS_FEED: MethodDescriptor =
    MethodDescriptor::read("post", "get_threads_feed").with_timeout(Duration::from_secs(30));
pub const GET_POST: MethodDescriptor = MethodDescriptor::read("post", "get_post");

pub const GET_USER_PUBLICATIONS: MethodDescriptor =
    MethodDescriptor::read("post", "get_user_publications").with_timeout(Duration::from_secs(30));
pub const GET_USER_THREADS: MethodDescriptor =
    MethodDescriptor::read("post", "get_user_threads").with_timeout(Duration::from_secs(30));
pub const GET_USER_REPOSTS: MethodDescriptor =
    MethodDescriptor::read("post", "get_user_reposts").with_timeout(Duration::from_secs(30));

pub const SET_POST_LIKE: MethodDescriptor =
    MethodDescriptor::idempotent_mutation("post", "set_post_like", "client_request_id");
pub const SET_POST_FAVORITE: MethodDescriptor =
    MethodDescriptor::idempotent_mutation("post", "set_post_favorite", "client_request_id");
pub const CREATE_REPOST: MethodDescriptor =
    MethodDescriptor::idempotent_mutation("post", "create_repost", "client_request_id")
        .with_timeout(Duration::from_secs(30));
pub const DELETE_REPOST: MethodDescriptor =
    MethodDescriptor::idempotent_mutation("post", "delete_repost", "client_request_id")
        .with_timeout(Duration::from_secs(30));
pub const RECORD_POST_VIEWS: MethodDescriptor =
    MethodDescriptor::mutation("post", "record_post_views");

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum FeedKind {
    #[default]
    All,
    Publications,
    Threads,
}

impl FeedKind {
    pub const ALL: [FeedKind; 3] = [FeedKind::All, FeedKind::Publications, FeedKind::Threads];

    pub fn descriptor(self) -> &'static MethodDescriptor {
        match self {
            Self::All => &GET_FEED,
            Self::Publications => &GET_PUBLICATIONS_FEED,
            Self::Threads => &GET_THREADS_FEED,
        }
    }

    pub fn title_key(self) -> &'static str {
        match self {
            Self::All => "post_feed_tab_all",
            Self::Publications => "post_feed_tab_publications",
            Self::Threads => "post_feed_tab_threads",
        }
    }

    pub fn empty_key(self) -> &'static str {
        match self {
            Self::All => "post_feed_empty_all",
            Self::Publications => "post_feed_empty_publications",
            Self::Threads => "post_feed_empty_threads",
        }
    }

    pub fn empty_symbol(self) -> &'static str {
        match self {
            Self::All => "sparkles",
            Self::Publications => "photo.on.rectangle.angled",
            Self::Threads => "text.bubble",
        }
    }

    pub fn tab_id(self) -> &'static str {
        match self {
            Self::All => "feed.all",
            Self::Publications => "feed.publications",
            Self::Threads => "feed.threads",
        }
    }
}

/// The three post lists a profile shows. They are addressed by the owner's tag
/// rather than their id, which is what the gateway takes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum UserFeedKind {
    Publications,
    Threads,
    Reposts,
}

impl UserFeedKind {
    pub const ALL: [UserFeedKind; 3] = [
        UserFeedKind::Publications,
        UserFeedKind::Threads,
        UserFeedKind::Reposts,
    ];

    pub fn descriptor(self) -> &'static MethodDescriptor {
        match self {
            Self::Publications => &GET_USER_PUBLICATIONS,
            Self::Threads => &GET_USER_THREADS,
            Self::Reposts => &GET_USER_REPOSTS,
        }
    }

    pub fn title_key(self) -> &'static str {
        match self {
            Self::Publications => "profile_tab_publications",
            Self::Threads => "profile_tab_threads",
            Self::Reposts => "profile_tab_reposts",
        }
    }

    pub fn empty_key(self) -> &'static str {
        match self {
            Self::Publications => "profile_publications_empty",
            Self::Threads => "profile_threads_empty",
            Self::Reposts => "profile_reposts_empty",
        }
    }

    pub fn empty_symbol(self) -> &'static str {
        match self {
            Self::Publications => "photo.on.rectangle.angled",
            Self::Threads => "text.bubble",
            Self::Reposts => "arrow.2.squarepath",
        }
    }

    pub fn tab_id(self) -> &'static str {
        match self {
            Self::Publications => "profile.publications",
            Self::Threads => "profile.threads",
            Self::Reposts => "profile.reposts",
        }
    }
}

/// Which list a page is. One repository serves them all, because a post shown in
/// two of them is one post: liking it in a profile tab has to move the feed row
/// too, and two repositories would each hold their own copy.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum PostScope {
    Feed(FeedKind),
    User { tag: String, kind: UserFeedKind },
}

impl PostScope {
    pub fn user(tag: &str, kind: UserFeedKind) -> Self {
        Self::User {
            tag: tag.trim().to_lowercase(),
            kind,
        }
    }

    pub fn descriptor(&self) -> &'static MethodDescriptor {
        match self {
            Self::Feed(kind) => kind.descriptor(),
            Self::User { kind, .. } => kind.descriptor(),
        }
    }

    pub fn empty_key(&self) -> &'static str {
        match self {
            Self::Feed(kind) => kind.empty_key(),
            Self::User { kind, .. } => kind.empty_key(),
        }
    }

    pub fn empty_symbol(&self) -> &'static str {
        match self {
            Self::Feed(kind) => kind.empty_symbol(),
            Self::User { kind, .. } => kind.empty_symbol(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GetFeedRequest {
    pub relative_id: Option<String>,
    pub limit: u32,
    pub new_feed: bool,
    pub up: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feed_session_id: Option<String>,
}

/// `relative_id` is a NUMBER here, not the string the global feeds accept:
/// `execute_get_user_posts` reads the cursor with a bare `as_i64()`, so a string
/// silently becomes "no cursor" and the second page repeats the first.
#[derive(Clone, Debug, Serialize)]
pub struct GetUserPostsRequest {
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub limit: u32,
    pub relative_id: Option<i64>,
    pub up: bool,
    pub new_feed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feed_session_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GetUserRepostsRequest {
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub limit: u32,
    pub relative_id: Option<i64>,
    pub up: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SetPostLikeRequest {
    pub user_id: String,
    pub post_id: i64,
    pub liked: bool,
    pub client_request_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SetPostFavoriteRequest {
    pub user_id: String,
    pub post_id: i64,
    pub favorited: bool,
    pub client_request_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CreateRepostRequest {
    pub user_id: String,
    pub post_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub client_request_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeleteRepostRequest {
    pub user_id: String,
    pub post_id: i64,
    pub client_request_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RecordPostViewsRequest {
    pub user_id: String,
    pub post_ids: Vec<i64>,
    pub surface: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GetPostRequest {
    pub post_id: i64,
    pub user_id: String,
}

// The gateway sends ids as numbers or strings for the same field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireId(pub String);

impl<'de> Deserialize<'de> for WireId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(text) => Ok(Self(text)),
            serde_json::Value::Number(number) => Ok(Self(number.to_string())),
            other => Err(D::Error::custom(format!("{other} is not an id"))),
        }
    }
}

impl WireId {
    pub fn into_string(self) -> String {
        self.0
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AuthorMoreDto {
    #[serde(alias = "pLang")]
    pub p_lang: Vec<String>,
    pub who: String,
    pub role: String,
    pub logo: String,
    #[serde(alias = "isPremium")]
    pub is_premium: bool,
    #[serde(alias = "isOfficial")]
    pub is_official: bool,
}

// The gateway sends the author nested under `more` on some methods and flat on others.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AuthorDto {
    pub id: Option<String>,
    pub name: Option<String>,
    pub tag: Option<String>,
    pub more: Option<AuthorMoreDto>,
    pub logo: Option<String>,
    pub who: Option<String>,
    pub role: Option<String>,
    #[serde(alias = "isPremium")]
    pub is_premium: Option<bool>,
    #[serde(alias = "isOfficial")]
    pub is_official: Option<bool>,
    #[serde(alias = "isSubscribed")]
    pub is_subscribed: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LikedByDto {
    pub id: Option<String>,
    pub name: Option<String>,
    pub tag: Option<String>,
    pub logo: Option<String>,
    #[serde(alias = "isPremium")]
    pub is_premium: Option<bool>,
    #[serde(alias = "isOfficial")]
    pub is_official: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default)]
pub struct MediaSizeDto {
    pub width: i64,
    pub height: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LinkPreviewDto {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(alias = "imageUrl")]
    pub image_url: Option<String>,
    pub status: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct PostDto {
    pub id: Option<WireId>,
    pub title: Option<String>,
    pub content: Option<String>,
    pub hashtags: Vec<String>,
    #[serde(alias = "viewsCount")]
    pub views_count: i64,
    #[serde(alias = "createdAt")]
    pub created_at: Option<String>,
    #[serde(alias = "createdAtMs")]
    pub created_at_ms: Option<i64>,
    #[serde(alias = "authorId")]
    pub author_id: Option<String>,
    #[serde(alias = "commentsCount")]
    pub comments_count: i64,
    #[serde(alias = "canComment")]
    pub can_comment: Option<bool>,
    #[serde(alias = "likesCount")]
    pub likes_count: i64,
    #[serde(alias = "favoritesCount")]
    pub favorites_count: i64,
    pub images: Vec<String>,
    #[serde(alias = "imageSizes")]
    pub image_sizes: Vec<MediaSizeDto>,
    #[serde(alias = "isLiked")]
    pub is_liked: Option<bool>,
    #[serde(alias = "isFavorited")]
    pub is_favorited: Option<bool>,
    pub author: Option<AuthorDto>,
    pub video: Option<String>,
    #[serde(alias = "videoThumbnail")]
    pub video_thumbnail: Option<String>,
    #[serde(alias = "isShort")]
    pub is_short: bool,
    #[serde(alias = "repostCount")]
    pub repost_count: i64,
    #[serde(alias = "isReposted")]
    pub is_reposted: Option<bool>,
    #[serde(alias = "myRepostId")]
    pub my_repost_id: Option<WireId>,
    #[serde(alias = "myRepostContent")]
    pub my_repost_content: Option<String>,
    #[serde(alias = "likedBy")]
    pub liked_by: Vec<LikedByDto>,
    #[serde(alias = "contentKind")]
    pub content_kind: Option<String>,
    #[serde(alias = "mediaStatus")]
    pub media_status: Option<String>,
    #[serde(alias = "moderationStatus")]
    pub moderation_status: Option<String>,
    #[serde(alias = "linkPreview")]
    pub link_preview: Option<LinkPreviewDto>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct FeedResponse {
    pub error: Option<String>,
    pub list: Vec<PostDto>,
    pub items: Vec<PostDto>,
    #[serde(alias = "relativeId")]
    pub relative_id: Option<WireId>,
    // The gateway stamps `has_more`, `is_have_more` AND `hasMore` onto every
    // successful response, so folding them onto one field with `alias` is a
    // duplicate-field error that loses the whole page, not belt and braces.
    pub has_more: Option<bool>,
    pub is_have_more: Option<bool>,
    #[serde(rename = "hasMore")]
    pub has_more_camel: Option<bool>,
    #[serde(rename = "isHaveMore")]
    pub is_have_more_camel: Option<bool>,
    #[serde(alias = "feedSessionId")]
    pub feed_session_id: Option<String>,
}

impl FeedResponse {
    pub fn rows(&self) -> &[PostDto] {
        if self.list.is_empty() {
            &self.items
        } else {
            &self.list
        }
    }

    pub fn more(&self) -> bool {
        self.has_more
            .or(self.is_have_more)
            .or(self.has_more_camel)
            .or(self.is_have_more_camel)
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct InteractionResponse {
    pub error: Option<String>,
    #[serde(alias = "postId")]
    pub post_id: Option<WireId>,
    #[serde(alias = "isLiked")]
    pub is_liked: Option<bool>,
    #[serde(alias = "isFavorited")]
    pub is_favorited: Option<bool>,
    #[serde(alias = "isReposted")]
    pub is_reposted: Option<bool>,
    #[serde(alias = "likesCount")]
    pub likes_count: Option<i64>,
    #[serde(alias = "favoritesCount")]
    pub favorites_count: Option<i64>,
    #[serde(alias = "repostCount")]
    pub repost_count: Option<i64>,
    #[serde(alias = "myRepostId")]
    pub my_repost_id: Option<WireId>,
    #[serde(alias = "myRepostContent")]
    pub my_repost_content: Option<String>,
    pub changed: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct SinglePostResponse {
    pub error: Option<String>,
    pub post: Option<PostDto>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rise_engine::ReplayPolicy;
    use serde_json::json;

    #[test]
    fn only_the_verbs_the_backend_promises_are_replayable() {
        assert_eq!(GET_FEED.replay, ReplayPolicy::ReadOnly);
        assert_eq!(
            SET_POST_LIKE.replay,
            ReplayPolicy::IdempotencyKey {
                field: "client_request_id"
            }
        );
        assert_eq!(
            CREATE_REPOST.replay,
            ReplayPolicy::IdempotencyKey {
                field: "client_request_id"
            }
        );
        assert_eq!(
            RECORD_POST_VIEWS.replay,
            ReplayPolicy::Never,
            "the server dedupes a view by (user, post) but promises nothing about a retried batch"
        );
    }

    #[test]
    fn every_feed_kind_addresses_its_own_method() {
        let methods: Vec<&str> = FeedKind::ALL
            .iter()
            .map(|kind| kind.descriptor().method)
            .collect();

        assert_eq!(
            methods,
            vec!["get_feed", "get_publications_feed", "get_threads_feed"]
        );
    }

    #[test]
    fn a_first_page_still_sends_a_null_cursor() {
        let body = serde_json::to_value(GetFeedRequest {
            relative_id: None,
            limit: 20,
            new_feed: true,
            up: false,
            feed_session_id: None,
        })
        .unwrap();

        assert!(
            body.get("relative_id").is_some(),
            "the reference encodes this one even when null, and the gateway reads the difference"
        );
        assert!(body["relative_id"].is_null());
        assert!(
            body.get("feed_session_id").is_none(),
            "and omits this one, which is the asymmetry being copied"
        );
    }

    #[test]
    fn a_page_decodes_the_camel_case_the_gateway_actually_sends() {
        let response: FeedResponse = serde_json::from_value(json!({
            "list": [{
                "id": 42,
                "title": "Day 370",
                "likesCount": 3,
                "isLiked": true,
                "author": {"id": "u1", "name": "Aianov", "more": {"logo": "https://cdn/a.png"}}
            }],
            "relativeId": 42,
            "isHaveMore": true,
            "feedSessionId": "s1"
        }))
        .unwrap();

        assert_eq!(response.rows().len(), 1);
        let post = &response.rows()[0];
        assert_eq!(post.id.clone().unwrap().into_string(), "42");
        assert_eq!(post.likes_count, 3);
        assert_eq!(post.is_liked, Some(true));
        assert_eq!(
            post.author.as_ref().unwrap().more.as_ref().unwrap().logo,
            "https://cdn/a.png"
        );
        assert_eq!(response.relative_id.clone().unwrap().into_string(), "42");
        assert!(response.more());
    }

    #[test]
    fn a_page_carrying_every_spelling_of_the_more_flag_still_decodes() {
        let response: FeedResponse = serde_json::from_value(json!({
            "list": [{"id": 1494, "title": "Test Post for Comments"}],
            "relative_id": 730,
            "has_more": true,
            "hasMore": true,
            "is_have_more": true,
            "feed_session_id": "feed_dd2d1a41_all_1786265828059866",
            "limit": 20,
            "total": 96,
            "page": 1,
            "total_pages": 5,
            "up": null
        }))
        .expect("the gateway sends all three spellings on every successful page");

        assert_eq!(response.rows().len(), 1);
        assert!(response.more());
    }

    #[test]
    fn a_page_decodes_the_snake_case_form_too() {
        let response: FeedResponse = serde_json::from_value(json!({
            "list": [{"id": "7", "likes_count": 9, "is_liked": false}],
            "relative_id": "7",
            "has_more": false
        }))
        .unwrap();

        assert_eq!(response.rows()[0].likes_count, 9);
        assert!(!response.more());
    }

    #[test]
    fn a_post_with_every_proto_default_omitted_still_decodes() {
        let post: PostDto = serde_json::from_value(json!({"id": 1})).unwrap();

        assert_eq!(post.likes_count, 0);
        assert_eq!(post.is_liked, None);
        assert!(post.images.is_empty());
        assert!(post.liked_by.is_empty());
        assert!(!post.is_short);
    }

    #[test]
    fn the_flat_author_form_decodes_beside_the_nested_one() {
        let flat: AuthorDto =
            serde_json::from_value(json!({"id": "u1", "name": "A", "logo": "https://cdn/a.png"}))
                .unwrap();
        assert!(flat.more.is_none());
        assert_eq!(flat.logo.as_deref(), Some("https://cdn/a.png"));

        let nested: AuthorDto =
            serde_json::from_value(json!({"id": "u1", "more": {"logo": "https://cdn/b.png"}}))
                .unwrap();
        assert_eq!(nested.more.unwrap().logo, "https://cdn/b.png");
    }

    #[test]
    fn a_repost_reads_as_counters_on_the_post_itself() {
        let post: PostDto = serde_json::from_value(json!({
            "id": 1,
            "repostCount": 3,
            "isReposted": true,
            "myRepostId": 77,
            "myRepostContent": "worth reading"
        }))
        .unwrap();

        assert_eq!(post.repost_count, 3);
        assert_eq!(post.is_reposted, Some(true));
        assert_eq!(post.my_repost_id.unwrap().into_string(), "77");
        assert_eq!(post.my_repost_content.as_deref(), Some("worth reading"));
    }

    #[test]
    fn an_interaction_answers_with_authoritative_counters() {
        let response: InteractionResponse = serde_json::from_value(json!({
            "postId": 42,
            "isLiked": true,
            "likesCount": 101,
            "changed": true
        }))
        .unwrap();

        assert_eq!(response.is_liked, Some(true));
        assert_eq!(response.likes_count, Some(101));
        assert_eq!(response.changed, Some(true));
    }

    #[test]
    fn a_no_op_interaction_is_not_an_error() {
        let response: InteractionResponse =
            serde_json::from_value(json!({"postId": 42, "isReposted": true, "changed": false}))
                .unwrap();

        assert_eq!(response.changed, Some(false));
        assert!(rise_engine::remote_message(response.error.as_deref()).is_none());
    }

    #[test]
    fn an_empty_error_string_is_not_an_error() {
        let response: InteractionResponse =
            serde_json::from_value(json!({"error": "", "postId": 1})).unwrap();
        assert!(rise_engine::remote_message(response.error.as_deref()).is_none());
    }
}
