use std::any::Any;

use gpui::{App, Context, Entity, Subscription};
use rise_canon::{RiseLimits, RiseTimingCanon};
use rise_core::foundation::foundation_text::{trimming_characters, trimming_whitespaces_and_newlines};

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::{RiseTimingCanonExt, RiseTimingPageExt};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::engine::rise_post_engine_models::{RisePostPagePresentation, RisePostSearchKind};
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, GetPostFeedResponse};
use crate::modules::post::stores::post_actions::post_wire_types::{GetFeedGatewayDTO, RisePostWirePage};
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::search::engine::rise_search_engine_models::{
    RiseGlobalSearchCategory, RiseGlobalSearchPagePresentation, RiseGlobalSearchRow,
};
use crate::modules::search::engine::rise_search_presentation::RiseSearchPresentation;
use crate::modules::search::stores::search_actions::search_actions::SearchActionsStore;
use crate::modules::search::stores::search_actions::search_types::{
    SearchEntityRow, SearchQueryKind, SearchResultsTab,
};
use crate::modules::search::stores::search_actions::search_wire_types::{
    GlobalSearchByTagGatewayDTO, GlobalSearchByTagGatewayDTOExt, SearchUsersListDTO, SearchUsersListDTOExt,
};

pub struct SearchServicesStore {
    pub query_string: String,
    pub results_tab: SearchResultsTab,

    pub hashtag_current_tag: Option<String>,

    search_post_last_error: Option<String>,
    hashtag_post_last_error: Option<String>,
    tag_entity_last_error: Option<String>,
    plain_user_last_error: Option<String>,
    global_search_last_error: Option<String>,
    global_search_chats_last_error: Option<String>,

    _subscriptions: Vec<Subscription>,
}

impl SearchServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        let search_presentation = RiseSearchPresentation::shared(cx);
        let post_presentation = RisePostContentPresentation::shared(cx);
        let post_services = PostServicesStore::shared(cx);
        Self {
            query_string: String::new(),
            results_tab: SearchResultsTab::POST,
            hashtag_current_tag: None,
            search_post_last_error: None,
            hashtag_post_last_error: None,
            tag_entity_last_error: None,
            plain_user_last_error: None,
            global_search_last_error: None,
            global_search_chats_last_error: None,
            _subscriptions: vec![
                cx.observe(&cache_revisions, |_, _, cx| cx.notify()),
                cx.observe(&search_presentation, |_, _, cx| cx.notify()),
                cx.observe(&post_presentation, |_, _, cx| cx.notify()),
                cx.observe(&post_services, |_, _, cx| cx.notify()),
            ],
        }
    }

    pub fn search_posts_limit() -> i64 {
        RiseTimingCanon::page("search_posts_by_hashtag").page_size_items()
    }

    pub fn search_users_limit() -> i64 {
        RiseLimits::client_value("search_services_store_search_users_limit")
    }

    pub fn search_chats_limit() -> i64 {
        RiseLimits::client_value("search_services_store_search_chats_limit")
    }

    pub fn search_tag_limit() -> i64 {
        RiseTimingCanon::page("search_users_by_tag").page_size_items()
    }

    pub fn global_search_page(
        &self,
        query: &str,
        category: RiseGlobalSearchCategory,
        cx: &mut App,
    ) -> RiseGlobalSearchPagePresentation {
        let presentation = RiseSearchPresentation::shared(cx);
        presentation.read(cx).page(query, category, cx).into_owned()
    }

    pub fn search_post_last_error(&self) -> Option<&str> {
        self.search_post_last_error.as_deref()
    }

    pub fn hashtag_post_last_error(&self) -> Option<&str> {
        self.hashtag_post_last_error.as_deref()
    }

    pub fn tag_entity_last_error(&self) -> Option<&str> {
        self.tag_entity_last_error.as_deref()
    }

    pub fn plain_user_last_error(&self) -> Option<&str> {
        self.plain_user_last_error.as_deref()
    }

    pub fn global_search_last_error(&self) -> Option<&str> {
        self.global_search_last_error.as_deref()
    }

    pub fn global_search_chats_last_error(&self) -> Option<&str> {
        self.global_search_chats_last_error.as_deref()
    }

    pub fn search_posts_cache_key(query: &str) -> String {
        SaiCache::new(["searchPosts", query]).key
    }

    pub fn hashtag_posts_cache_key(tag: &str) -> String {
        SaiCache::new(["searchPostsByHashtag", tag]).key
    }

    pub fn global_search_by_tag_cache_key(tag: &str, user_id: &str) -> String {
        SaiCache::new(["globalSearchByTag", tag, user_id]).key
    }

    pub fn global_search_cache_key(query: &str, user_id: &str) -> String {
        SaiCache::new(["globalSearch", query.to_lowercase().as_str(), user_id]).key
    }

    pub fn global_search_chats_cache_key(query: &str, user_id: &str) -> String {
        SaiCache::new(["globalSearchChats", query.to_lowercase().as_str(), user_id]).key
    }

    pub fn search_users_cache_key(query: &str) -> String {
        SaiCache::new(["searchUsers", query.to_lowercase().as_str()]).key
    }

    fn current_trimmed_query(&self) -> &str {
        trimming_whitespaces_and_newlines(&self.query_string)
    }

    fn current_tag_from_query(&self) -> &str {
        trimming_characters(self.current_trimmed_query(), |c| c == '@')
    }

    pub fn active_query_kind(&self) -> SearchQueryKind {
        let q = self.current_trimmed_query();
        if q.is_empty() {
            return SearchQueryKind::FeedDefault;
        }
        if q.starts_with('#') {
            return SearchQueryKind::Hashtag;
        }
        if self.results_tab == SearchResultsTab::USER {
            return SearchQueryKind::GlobalUnified;
        }
        SearchQueryKind::Posts
    }

    fn global_search_query(&self) -> &str {
        trimming_characters(self.current_trimmed_query(), |c| c == '@')
    }

    fn profile_id(cx: &mut App) -> String {
        AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default()
    }

    fn hashtag_posts_dto(&self, observed: bool, cx: &mut App) -> Option<GetFeedGatewayDTO> {
        let tag = self.hashtag_current_tag.as_deref().filter(|tag| !tag.is_empty())?;

        let key = Self::hashtag_posts_cache_key(tag);
        let payload = if observed {
            SaiWsResponseCache::observed_payload(
                &key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::MemoryOnly,
                cx,
            )
        } else {
            SaiWsResponseCache::current_payload(
                &key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::MemoryOnly,
                cx,
            )
        }?;

        SaiJSONCodec::decode(Some(&payload))
    }

    fn tag_entities_dto(&self, observed: bool, cx: &mut App) -> Option<GlobalSearchByTagGatewayDTO> {
        let user_id = Self::profile_id(cx);
        let key = Self::global_search_by_tag_cache_key(self.current_tag_from_query(), &user_id);
        let payload = if observed {
            SaiWsResponseCache::observed_payload(
                &key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::MemoryOnly,
                cx,
            )
        } else {
            SaiWsResponseCache::current_payload(
                &key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::MemoryOnly,
                cx,
            )
        }?;

        SaiJSONCodec::decode(Some(&payload))
    }

    fn plain_users_dto(&self, observed: bool, cx: &mut App) -> Option<SearchUsersListDTO> {
        let key = Self::search_users_cache_key(self.current_trimmed_query());
        let payload = if observed {
            SaiWsResponseCache::observed_payload(
                &key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
        } else {
            SaiWsResponseCache::current_payload(
                &key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
        }?;

        SaiJSONCodec::decode(Some(&payload))
    }

    pub fn search_users_page(&self, query: &str, cx: &mut App) -> Option<SearchUsersListDTO> {
        let trimmed = trimming_whitespaces_and_newlines(query);
        if trimmed.is_empty() {
            return None;
        }
        let payload = SaiWsResponseCache::observed_payload(
            &Self::search_users_cache_key(trimmed),
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )?;

        let dto: Option<SearchUsersListDTO> = SaiJSONCodec::decode(Some(&payload));
        Some(dto.unwrap_or_default())
    }

    fn publications_search_page(&self, cx: &mut App) -> RisePostPagePresentation {
        let query = self.current_trimmed_query();
        PostServicesStore::shared(cx)
            .update(cx, |posts, cx| posts.search_post_page(query, RisePostSearchKind::Publications, cx))
    }

    pub fn search_post_items(&self, cx: &mut App) -> Vec<GetPostFeedResponse> {
        self.publications_search_page(cx).items
    }

    pub fn search_post_has_more(&self, cx: &mut App) -> bool {
        self.publications_search_page(cx).has_more
    }

    pub fn search_post_relative_id(&self) -> Option<String> {
        None
    }

    pub fn hashtag_post_items(&self, cx: &mut App) -> Vec<GetPostFeedResponse> {
        self.hashtag_posts_dto(true, cx).map(|dto| dto.as_virtual_list().list).unwrap_or_default()
    }

    pub fn hashtag_post_has_more(&self, cx: &mut App) -> bool {
        self.hashtag_posts_dto(true, cx).and_then(|dto| dto.as_virtual_list().is_have_more_bot_or_top).unwrap_or(false)
    }

    pub fn hashtag_post_relative_id(&self, cx: &mut App) -> Option<String> {
        self.hashtag_posts_dto(true, cx).and_then(|dto| dto.as_virtual_list().relative_id)
    }

    pub fn hashtag_post_total(&self, cx: &mut App) -> Option<i64> {
        self.hashtag_posts_dto(true, cx).map(|dto| dto.page_total())
    }

    pub fn hashtag_post_did_load(&self, cx: &mut App) -> bool {
        SearchActionsStore::shared(cx).read(cx).search_posts_by_hashtag_sai.status() == SaiWsSliceStatus::Fulfilled
    }

    pub fn tag_entity_rows(&self, cx: &mut App) -> Vec<SearchEntityRow> {
        self.tag_entities_dto(true, cx).map(|dto| dto.entity_rows()).unwrap_or_default()
    }

    pub fn tag_entity_has_more(&self, cx: &mut App) -> bool {
        self.tag_entities_dto(true, cx).is_some_and(|dto| dto.has_more)
    }

    pub fn tag_entity_next_cursor(&self, cx: &mut App) -> Option<String> {
        self.tag_entities_dto(true, cx).and_then(|dto| dto.next_cursor)
    }

    pub fn plain_user_authors(&self, cx: &mut App) -> Vec<AuthorInfo> {
        self.plain_users_dto(true, cx).map(|dto| dto.authors()).unwrap_or_default()
    }

    pub fn global_search_chat_rows(&self) -> Vec<ChatInfo> {
        Vec::new()
    }

    pub fn global_search_chats_has_more(&self, cx: &mut App) -> bool {
        self.global_search_page(self.global_search_query(), RiseGlobalSearchCategory::Channels, cx).has_more
    }

    pub fn global_search_chats_next_cursor(&self) -> Option<String> {
        None
    }

    pub fn global_search_entity_rows(&self, cx: &mut App) -> Vec<SearchEntityRow> {
        self.global_search_page(self.global_search_query(), RiseGlobalSearchCategory::Profiles, cx)
            .items
            .into_iter()
            .filter_map(|row| match row {
                RiseGlobalSearchRow::Profile(row) => Some(row),
                _ => None,
            })
            .collect()
    }

    pub fn global_search_has_more(&self, cx: &mut App) -> bool {
        self.global_search_page(self.global_search_query(), RiseGlobalSearchCategory::Profiles, cx).has_more
    }

    pub fn global_search_next_cursor(&self) -> Option<String> {
        None
    }

    pub fn global_search_is_empty(&self, cx: &mut App) -> bool {
        self.global_search_chat_rows().is_empty() && self.global_search_entity_rows(cx).is_empty()
    }

    pub fn plain_user_has_more(&self, cx: &mut App) -> bool {
        self.plain_users_dto(true, cx).is_some_and(|dto| dto.is_have_more)
    }

    pub fn plain_user_relative_id(&self, cx: &mut App) -> Option<String> {
        self.plain_users_dto(true, cx).and_then(|dto| dto.relative_id).map(|relative_id| relative_id.to_string())
    }

    pub fn visible_post_items(&self, cx: &mut App) -> Vec<GetPostFeedResponse> {
        match self.active_query_kind() {
            SearchQueryKind::FeedDefault => {
                PostServicesStore::shared(cx).update(cx, |posts, cx| posts.feed_items(cx))
            }
            SearchQueryKind::Posts => self.search_post_items(cx),
            SearchQueryKind::Hashtag => self.hashtag_post_items(cx),
            _ => Vec::new(),
        }
    }

    pub fn post_results_status(&self, cx: &mut App) -> SaiWsSliceStatus {
        match self.active_query_kind() {
            SearchQueryKind::FeedDefault => {
                let posts = PostServicesStore::shared(cx);
                let did_load = posts.update(cx, |posts, cx| posts.feed_did_load(cx));
                if did_load || !posts.update(cx, |posts, cx| posts.feed_items(cx)).is_empty() {
                    return SaiWsSliceStatus::Fulfilled;
                }
                PostActionsStore::shared(cx).read(cx).get_feed_sai.status()
            }
            SearchQueryKind::Posts => SearchActionsStore::shared(cx).read(cx).search_posts_sai.status(),
            SearchQueryKind::Hashtag => SearchActionsStore::shared(cx).read(cx).search_posts_by_hashtag_sai.status(),
            _ => SaiWsSliceStatus::Idle,
        }
    }

    pub fn post_results_error_text(&self, cx: &mut App) -> Option<String> {
        match self.active_query_kind() {
            SearchQueryKind::FeedDefault => {
                PostInteractionsStore::shared(cx).update(cx, |posts, cx| posts.feed_error_text(cx))
            }
            SearchQueryKind::Posts => self
                .search_post_last_error
                .clone()
                .or_else(|| SearchActionsStore::shared(cx).read(cx).search_posts_sai.error()),
            SearchQueryKind::Hashtag => self
                .hashtag_post_last_error
                .clone()
                .or_else(|| SearchActionsStore::shared(cx).read(cx).search_posts_by_hashtag_sai.error()),
            _ => None,
        }
    }

    pub fn post_results_has_more(&self, cx: &mut App) -> bool {
        match self.active_query_kind() {
            SearchQueryKind::FeedDefault => {
                PostServicesStore::shared(cx).update(cx, |posts, cx| posts.feed_has_more(cx))
            }
            SearchQueryKind::Posts => self.search_post_has_more(cx),
            SearchQueryKind::Hashtag => self.hashtag_post_has_more(cx),
            _ => false,
        }
    }

    pub fn post_results_relative_id(&self, cx: &mut App) -> Option<String> {
        match self.active_query_kind() {
            SearchQueryKind::FeedDefault => PostServicesStore::shared(cx).read(cx).feed_relative_id(),
            SearchQueryKind::Posts => self.search_post_relative_id(),
            SearchQueryKind::Hashtag => self.hashtag_post_relative_id(cx),
            _ => None,
        }
    }

    pub fn post_results_cache_key(&self) -> String {
        match self.active_query_kind() {
            SearchQueryKind::FeedDefault => PostServicesStore::feed_cache_key(),
            SearchQueryKind::Posts => Self::search_posts_cache_key(self.current_trimmed_query()),
            SearchQueryKind::Hashtag => Self::hashtag_posts_cache_key(
                self.hashtag_current_tag.as_deref().unwrap_or_else(|| self.current_trimmed_query()),
            ),
            _ => SaiCache::new(["searchPostsInactive"]).key,
        }
    }

    pub fn current_global_search_cache_key(&self, cx: &mut App) -> String {
        Self::global_search_cache_key(self.global_search_query(), &Self::profile_id(cx))
    }

    pub fn current_global_search_chats_cache_key(&self, cx: &mut App) -> String {
        Self::global_search_chats_cache_key(self.global_search_query(), &Self::profile_id(cx))
    }

    pub fn current_tag_entity_cache_key(&self, cx: &mut App) -> String {
        Self::global_search_by_tag_cache_key(self.current_tag_from_query(), &Self::profile_id(cx))
    }

    pub fn user_entity_rows(&self, cx: &mut App) -> Vec<SearchEntityRow> {
        if self.active_query_kind() == SearchQueryKind::UsersByTag {
            return self.tag_entity_rows(cx);
        }
        Vec::new()
    }

    pub fn plain_user_rows(&self, cx: &mut App) -> Vec<AuthorInfo> {
        if self.active_query_kind() == SearchQueryKind::UsersPlain {
            return self.plain_user_authors(cx);
        }
        Vec::new()
    }

    pub fn user_results_status(&self, cx: &mut App) -> SaiWsSliceStatus {
        match self.active_query_kind() {
            SearchQueryKind::GlobalUnified => {
                let actions = SearchActionsStore::shared(cx);
                let global_status = actions.read(cx).global_search_sai.status();
                let chats_status = actions.read(cx).global_search_chats_sai.status();
                if !self.global_search_chat_rows().is_empty() || !self.global_search_entity_rows(cx).is_empty() {
                    return SaiWsSliceStatus::Fulfilled;
                }
                if global_status == SaiWsSliceStatus::Pending || chats_status == SaiWsSliceStatus::Pending {
                    return SaiWsSliceStatus::Pending;
                }
                if global_status == SaiWsSliceStatus::Rejected || chats_status == SaiWsSliceStatus::Rejected {
                    return SaiWsSliceStatus::Rejected;
                }
                if global_status == SaiWsSliceStatus::Fulfilled || chats_status == SaiWsSliceStatus::Fulfilled {
                    return SaiWsSliceStatus::Fulfilled;
                }
                SaiWsSliceStatus::Idle
            }
            SearchQueryKind::UsersByTag => SearchActionsStore::shared(cx).read(cx).global_search_by_tag_sai.status(),
            SearchQueryKind::UsersPlain => SearchActionsStore::shared(cx).read(cx).search_users_sai.status(),
            _ => SaiWsSliceStatus::Idle,
        }
    }

    pub fn user_results_error_text(&self, cx: &mut App) -> Option<String> {
        match self.active_query_kind() {
            SearchQueryKind::GlobalUnified => {
                let actions = SearchActionsStore::shared(cx);
                self.global_search_last_error
                    .clone()
                    .or_else(|| self.global_search_chats_last_error.clone())
                    .or_else(|| actions.read(cx).global_search_sai.error())
                    .or_else(|| actions.read(cx).global_search_chats_sai.error())
            }
            SearchQueryKind::UsersByTag => self
                .tag_entity_last_error
                .clone()
                .or_else(|| SearchActionsStore::shared(cx).read(cx).global_search_by_tag_sai.error()),
            SearchQueryKind::UsersPlain => self
                .plain_user_last_error
                .clone()
                .or_else(|| SearchActionsStore::shared(cx).read(cx).search_users_sai.error()),
            _ => None,
        }
    }

    pub fn user_results_has_more(&self, cx: &mut App) -> bool {
        match self.active_query_kind() {
            SearchQueryKind::GlobalUnified => self.global_search_has_more(cx),
            SearchQueryKind::UsersByTag => self.tag_entity_has_more(cx),
            SearchQueryKind::UsersPlain => self.plain_user_has_more(cx),
            _ => false,
        }
    }

    pub fn begin_hashtag_search(&mut self, tag: &str, cx: &mut Context<Self>) {
        self.hashtag_current_tag = Some(tag.to_owned());
        cx.notify();
    }

    pub fn reset_hashtag_search(&mut self, cx: &mut Context<Self>) {
        if let Some(tag) = self.hashtag_current_tag.as_deref().filter(|tag| !tag.is_empty()) {
            SaiWsResponseCache::remove(&Self::hashtag_posts_cache_key(tag), SaiCacheCleanLayer::Memory, cx);
        }

        self.hashtag_current_tag = None;
        self.hashtag_post_last_error = None;
        cx.notify();
    }

    pub fn search_posts_success_handler(&mut self, payload: Option<&dyn Any>, cx: &mut Context<Self>) {
        self.search_post_last_error = None;
        PostServicesStore::cache_own_posts_from_payload(payload, cx);
        cx.notify();
    }

    pub fn search_posts_error_handler(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.search_post_last_error = Some(CanonErrors::load_failure_text(error));
        Sai::log("Search", || format!("search_posts error: {error}"), SaiLogKind::Network, cx);
        cx.notify();
    }

    pub fn hashtag_posts_success_handler(&mut self, payload: Option<&dyn Any>, cx: &mut Context<Self>) {
        self.hashtag_post_last_error = None;
        PostServicesStore::cache_own_posts_from_payload(payload, cx);
        cx.notify();
    }

    pub fn hashtag_posts_error_handler(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.hashtag_post_last_error = Some(CanonErrors::load_failure_text(error));
        Sai::log("Search", || format!("search_posts_by_hashtag error: {error}"), SaiLogKind::Network, cx);
        cx.notify();
    }

    pub fn global_search_success_handler(&mut self, cx: &mut Context<Self>) {
        self.global_search_last_error = None;
        cx.notify();
    }

    pub fn global_search_error_handler(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.global_search_last_error = Some(CanonErrors::load_failure_text(error));
        Sai::log("Search", || format!("global_search error: {error}"), SaiLogKind::Network, cx);
        cx.notify();
    }

    pub fn global_search_chats_success_handler(&mut self, cx: &mut Context<Self>) {
        self.global_search_chats_last_error = None;
        cx.notify();
    }

    pub fn global_search_chats_error_handler(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.global_search_chats_last_error = Some(CanonErrors::load_failure_text(error));
        Sai::log("Search", || format!("global_search chats error: {error}"), SaiLogKind::Network, cx);
        cx.notify();
    }

    pub fn global_search_by_tag_success_handler(&mut self, cx: &mut Context<Self>) {
        self.tag_entity_last_error = None;
        cx.notify();
    }

    pub fn global_search_by_tag_error_handler(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.tag_entity_last_error = Some(CanonErrors::load_failure_text(error));
        Sai::log("Search", || format!("global_search_by_tag error: {error}"), SaiLogKind::Network, cx);
        cx.notify();
    }

    pub fn search_users_success_handler(&mut self, cx: &mut Context<Self>) {
        self.plain_user_last_error = None;
        cx.notify();
    }

    pub fn search_users_error_handler(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.plain_user_last_error = Some(CanonErrors::load_failure_text(error));
        Sai::log("Search", || format!("search_users error: {error}"), SaiLogKind::Network, cx);
        cx.notify();
    }

    pub fn reset_search_results(&mut self, cx: &mut Context<Self>) {
        let q = self.current_trimmed_query().to_owned();
        if !q.is_empty() {
            SaiWsResponseCache::remove(&Self::search_posts_cache_key(&q), SaiCacheCleanLayer::Memory, cx);
            SaiWsResponseCache::remove(&Self::search_users_cache_key(&q), SaiCacheCleanLayer::Memory, cx);
        }

        let user_id = Self::profile_id(cx);
        let tag_key = Self::global_search_by_tag_cache_key(self.current_tag_from_query(), &user_id);
        SaiWsResponseCache::remove(&tag_key, SaiCacheCleanLayer::Memory, cx);
        let global_key = Self::global_search_cache_key(self.global_search_query(), &user_id);
        SaiWsResponseCache::remove(&global_key, SaiCacheCleanLayer::Memory, cx);
        let chats_key = Self::global_search_chats_cache_key(self.global_search_query(), &user_id);
        SaiWsResponseCache::remove(&chats_key, SaiCacheCleanLayer::Memory, cx);

        self.search_post_last_error = None;
        self.plain_user_last_error = None;
        self.tag_entity_last_error = None;
        self.global_search_last_error = None;
        self.global_search_chats_last_error = None;

        self.reset_hashtag_search(cx);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.query_string = String::new();
        self.results_tab = SearchResultsTab::POST;
        self.hashtag_current_tag = None;
        self.search_post_last_error = None;
        self.hashtag_post_last_error = None;
        self.tag_entity_last_error = None;
        self.plain_user_last_error = None;
        self.global_search_last_error = None;
        self.global_search_chats_last_error = None;
        cx.notify();
    }
}
