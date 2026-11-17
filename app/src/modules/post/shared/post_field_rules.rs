use std::sync::LazyLock;

use rise_canon::{RiseRequestBody, rise_request};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::modules::post::engine::rise_post_engine_models::RisePostContentKind;

/// wire text field rules for posts and reposts
/// post title and body travel as one string under two keys
/// content and original_content, caps 20000 and 10000
pub enum PostFieldRules {}

const POST_CREATE_POST: &str = <rise_request::post::CreatePost as RiseRequestBody>::CONTRACT;
const POST_CREATE_REPOST: &str = <rise_request::post::CreateRepost as RiseRequestBody>::CONTRACT;

static TITLE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(POST_CREATE_POST, "/title"));
static CONTENT: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(POST_CREATE_POST, "/content"));
static SOURCE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(POST_CREATE_POST, "/original_content"));
static REPOST_CONTENT: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(POST_CREATE_REPOST, "/content"));
static HASHTAG: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(POST_CREATE_POST, "/hashtags/*"));

impl PostFieldRules {
    pub fn title() -> RiseFieldRule {
        *TITLE
    }

    pub fn content() -> RiseFieldRule {
        *CONTENT
    }

    /// the stricter of the two keys the post body is sent under
    pub fn source() -> RiseFieldRule {
        *SOURCE
    }

    pub fn repost_content() -> RiseFieldRule {
        *REPOST_CONTENT
    }

    pub fn hashtag() -> RiseFieldRule {
        *HASHTAG
    }

    /// a post lives by its media and a thread by its description
    /// title and hashtags are optional for both
    /// server rejects a thread without a description and canon rules cant express that
    pub fn is_description_missing(kind: RisePostContentKind, content: &str) -> bool {
        kind == RisePostContentKind::Thread && trimming_whitespaces_and_newlines(content).is_empty()
    }
}
