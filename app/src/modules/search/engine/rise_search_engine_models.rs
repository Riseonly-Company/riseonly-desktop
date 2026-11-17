use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use rise_canon::rise_response;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use serde::{Deserialize, Serialize};

use crate::modules::post::engine::rise_post_engine_models::{RisePostCacheSource, RisePostPageStatus};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::search::stores::search_actions::search_types::SearchEntityRow;

pub type RiseGlobalSearchCategory = rise_catalog::GlobalSearchCategory;

pub mod rise_global_search_category_codable {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::RiseGlobalSearchCategory;

    pub fn serialize<S: Serializer>(category: &RiseGlobalSearchCategory, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(category.wire())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<RiseGlobalSearchCategory, D::Error> {
        let raw = String::deserialize(deserializer)?;
        RiseGlobalSearchCategory::ALL
            .iter()
            .copied()
            .find(|category| category.wire() == raw)
            .ok_or_else(|| D::Error::unknown_variant(&raw, RiseGlobalSearchCategory::WIRES))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RiseGlobalSearchRow {
    Post(GetPostFeedResponse),
    Profile(SearchEntityRow),
    Channel(SearchEntityRow),
    Group(SearchEntityRow),
}

impl RiseGlobalSearchRow {
    pub fn id(&self) -> String {
        match self {
            Self::Post(post) => format!("post:{}", post.id),
            Self::Profile(row) => format!("profile:{}", row.id),
            Self::Channel(row) => format!("channel:{}", row.id),
            Self::Group(row) => format!("group:{}", row.id),
        }
    }
}

impl Eq for RiseGlobalSearchRow {}

impl Hash for RiseGlobalSearchRow {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseGlobalSearchPagePresentation {
    pub items: Vec<RiseGlobalSearchRow>,
    pub status: RisePostPageStatus,
    pub error_message: Option<String>,
    pub has_more: bool,
    pub is_refreshing: bool,
    pub is_loading_more: bool,
    pub cache_source: RisePostCacheSource,
    pub revision: i64,
}

impl RiseGlobalSearchPagePresentation {
    pub const IDLE: Self = Self {
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

#[derive(Clone, Debug, PartialEq)]
pub struct RiseSearchPresentationSnapshot {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub revision: i64,
    pub pages_by_scope_key: HashMap<String, RiseGlobalSearchPagePresentation>,
}

static EMPTY: LazyLock<RiseSearchPresentationSnapshot> = LazyLock::new(|| RiseSearchPresentationSnapshot {
    account_id: String::new(),
    account_epoch: String::new(),
    session_id: Uuid::new_v4(),
    revision: 0,
    pages_by_scope_key: HashMap::new(),
});

impl RiseSearchPresentationSnapshot {
    pub fn empty() -> Self {
        EMPTY.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseGlobalSearchScope {
    pub query: String,
    pub category: RiseGlobalSearchCategory,
}

impl RiseGlobalSearchScope {
    pub fn normalized_query(&self) -> String {
        trimming_whitespaces_and_newlines(&self.query).to_lowercase()
    }

    pub fn key(&self) -> String {
        let normalized_query = self.normalized_query();
        format!(
            "global|{}:{}|{}",
            normalized_query.len(),
            normalized_query,
            self.category.wire()
        )
    }
}

pub type RiseGlobalSearchResponse = rise_response::search::GlobalSearch;

pub trait RiseGlobalSearchResponseExt {
    fn error(&self) -> Option<String>;
    fn has_more(&self) -> bool;
    fn next_cursor(&self) -> Option<String>;
}

impl RiseGlobalSearchResponseExt for RiseGlobalSearchResponse {
    fn error(&self) -> Option<String> {
        match self {
            Self::CategorizedPosts(page) => page.error.clone(),
            Self::CategorizedProfiles(page) => page.error.clone(),
            Self::CategorizedChats(page) => page.error.clone(),
            Self::MixedResults(page) => page.global_error.clone(),
        }
    }

    fn has_more(&self) -> bool {
        match self {
            Self::CategorizedPosts(page) => page.has_more,
            Self::CategorizedProfiles(page) => page.has_more,
            Self::CategorizedChats(page) => page.has_more,
            Self::MixedResults(page) => page.global_has_more,
        }
    }

    fn next_cursor(&self) -> Option<String> {
        match self {
            Self::CategorizedPosts(page) => page.next_cursor.clone(),
            Self::CategorizedProfiles(page) => page.next_cursor.clone(),
            Self::CategorizedChats(page) => page.next_cursor.clone(),
            Self::MixedResults(page) => page.global_next_cursor.clone(),
        }
    }
}

/// disk row of a search entity in the encrypted store
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RiseSearchStoredEntity {
    pub id: String,
    #[serde(rename = "entityType")]
    pub entity_type: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "tagLine")]
    pub tag_line: String,
    #[serde(rename = "logoURL")]
    pub logo_url: String,
    #[serde(rename = "userID", default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(rename = "userTag", default, skip_serializing_if = "Option::is_none")]
    pub user_tag: Option<String>,
    #[serde(rename = "chatID", default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(rename = "isPremium")]
    pub is_premium: bool,
    #[serde(rename = "isOfficial")]
    pub is_official: bool,
    #[serde(rename = "premiumBadgeEmoji", default, skip_serializing_if = "Option::is_none")]
    pub premium_badge_emoji: Option<String>,
    #[serde(rename = "nameColor", default, skip_serializing_if = "Option::is_none")]
    pub name_color: Option<String>,
    #[serde(rename = "isBot")]
    pub is_bot: bool,
    #[serde(rename = "botIsActive")]
    pub bot_is_active: bool,
    #[serde(rename = "memberCount", default, skip_serializing_if = "Option::is_none")]
    pub member_count: Option<i64>,
    #[serde(rename = "isPublic", default, skip_serializing_if = "Option::is_none")]
    pub is_public: Option<bool>,
}

impl RiseSearchStoredEntity {
    pub fn new(row: &SearchEntityRow) -> Self {
        Self {
            id: row.id.clone(),
            entity_type: row.entity_type.clone(),
            display_name: row.display_name.clone(),
            tag_line: row.tag_line.clone(),
            logo_url: row.logo_url.clone(),
            user_id: row.user_id.clone(),
            user_tag: row.user_tag.clone(),
            chat_id: row.chat_id.clone(),
            is_premium: row.is_premium,
            is_official: row.is_official,
            premium_badge_emoji: row.premium_badge_emoji.clone(),
            name_color: row.name_color.clone(),
            is_bot: row.is_bot,
            bot_is_active: row.bot_is_active,
            member_count: row.member_count,
            is_public: row.is_public,
        }
    }

    pub fn row(&self) -> SearchEntityRow {
        SearchEntityRow {
            id: self.id.clone(),
            entity_type: self.entity_type.clone(),
            display_name: self.display_name.clone(),
            tag_line: self.tag_line.clone(),
            logo_url: self.logo_url.clone(),
            user_id: self.user_id.clone(),
            user_tag: self.user_tag.clone(),
            chat_id: self.chat_id.clone(),
            is_premium: self.is_premium,
            is_official: self.is_official,
            premium_badge_emoji: self.premium_badge_emoji.clone(),
            name_color: self.name_color.clone(),
            is_bot: self.is_bot,
            bot_is_active: self.bot_is_active,
            member_count: self.member_count,
            is_public: self.is_public,
        }
    }
}
