use std::any::Any;
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_canon::rise_key;
use rise_canon::{rise_request, rise_wire_methods};
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_text::{trimming_characters, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiTypedSlice;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationCursorFallback, SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions,
    SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::post::engine::rise_post_engine_models::RisePostSearchKind;
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_actions::post_wire_types::GetFeedGatewayDTO;
use crate::modules::search::engine::rise_search_domain::{
    RiseSearchActionExecutor, RiseSearchDomain, RiseSearchDomainRegistry,
};
use crate::modules::search::engine::rise_search_engine_models::RiseGlobalSearchCategory;
use crate::modules::search::stores::search_actions::search_wire_types::{
    GlobalSearchByTagGatewayDTO, GlobalSearchGatewayDTO, SearchUsersListDTO,
};
use crate::modules::search::stores::search_services::search_services::SearchServicesStore;

pub struct SearchActionsStore {
    pub search_posts_sai: Rc<SaiTypedSlice<GetFeedGatewayDTO>>,
    pub search_posts_by_hashtag_sai: Rc<SaiTypedSlice<GetFeedGatewayDTO>>,
    pub global_search_sai: Rc<SaiTypedSlice<GlobalSearchGatewayDTO>>,
    pub global_search_chats_sai: Rc<SaiTypedSlice<GlobalSearchGatewayDTO>>,
    pub global_search_by_tag_sai: Rc<SaiTypedSlice<GlobalSearchByTagGatewayDTO>>,
    pub search_users_sai: Rc<SaiTypedSlice<SearchUsersListDTO>>,
}

impl SearchActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            search_posts_sai: SaiTypedSlice::new(cx),
            search_posts_by_hashtag_sai: SaiTypedSlice::new(cx),
            global_search_sai: SaiTypedSlice::new(cx),
            global_search_chats_sai: SaiTypedSlice::new(cx),
            global_search_by_tag_sai: SaiTypedSlice::new(cx),
            search_users_sai: SaiTypedSlice::new(cx),
        }
    }

    pub fn search_posts_action(
        &mut self,
        query: &str,
        relative_id: Option<String>,
        need_pending: bool,
        fetch_if_have_data: bool,
        cx: &mut Context<Self>,
    ) {
        let q = trimming_whitespaces_and_newlines(query);
        if q.is_empty() || q.starts_with('#') || q.starts_with('@') {
            return;
        }
        let _ = need_pending;
        let _ = fetch_if_have_data;
        let posts = PostActionsStore::shared(cx);
        let task = if relative_id.is_none() {
            posts.update(cx, |posts, cx| posts.activate_post_search_action(q, RisePostSearchKind::Publications, cx))
        } else {
            posts.update(cx, |posts, cx| posts.load_more_post_search_action(q, RisePostSearchKind::Publications, cx))
        };
        task.detach();
    }

    pub fn search_posts_by_hashtag_action(
        &mut self,
        hashtag: &str,
        relative_id: Option<String>,
        need_pending: bool,
        fetch_if_have_data: bool,
        cx: &mut Context<Self>,
    ) {
        let tag = trimming_characters(trimming_whitespaces_and_newlines(hashtag), |c| c == '#');
        if tag.is_empty() {
            return;
        }

        let is_first_page = relative_id.is_none();
        if is_first_page {
            SearchServicesStore::shared(cx).update(cx, |services, cx| services.begin_hashtag_search(tag, cx));
        }

        let body = rise_request::post::SearchPostsByHashtag {
            hashtag: Some(tag.to_owned()),
            limit: Some(SearchServicesStore::search_posts_limit()),
            up: Some(false),
            relative_id: Self::outbound_relative_id(relative_id.as_deref()),
            ..rise_request::post::SearchPostsByHashtag::default()
        };

        let cache_key = SearchServicesStore::hashtag_posts_cache_key(tag);

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::post::SERVICE.to_owned(),
            method: rise_wire_methods::post::SEARCH_POSTS_BY_HASHTAG.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: fetch_if_have_data,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: need_pending,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::MemoryOnly),
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(rise_key::post::LIST.to_owned()),
            path_to_id: Some(rise_key::post::ID.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id,
                response_relative_id_fallback: Some(SaiWsPaginationCursorFallback::LastItemId),
                has_more_path: Some(rise_key::post::IS_HAVE_MORE.to_owned()),
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(|payload: Option<Box<dyn Any>>, cx: &mut App| {
            SearchServicesStore::shared(cx)
                .update(cx, |services, cx| services.hashtag_posts_success_handler(payload.as_deref(), cx));
        });
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            SearchServicesStore::shared(cx).update(cx, |services, cx| services.hashtag_posts_error_handler(error, cx));
        });

        SaiWs::request_slice(&self.search_posts_by_hashtag_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn global_search_action(
        &mut self,
        query: &str,
        relative_id: Option<String>,
        need_pending: bool,
        fetch_if_have_data: bool,
        cx: &mut Context<Self>,
    ) {
        let clean = trimming_characters(trimming_whitespaces_and_newlines(query), |c| c == '@');
        if clean.is_empty() {
            return;
        }
        let _ = need_pending;
        let _ = fetch_if_have_data;
        let task = if relative_id.is_none() {
            self.activate_global_search_action(clean, RiseGlobalSearchCategory::Profiles, cx)
        } else {
            self.load_more_global_search_action(clean, RiseGlobalSearchCategory::Profiles, cx)
        };
        task.detach();
    }

    pub fn activate_global_search_action(
        &mut self,
        query: &str,
        category: RiseGlobalSearchCategory,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let query = query.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.activate_global_search(&query, category).await })
    }

    pub fn observe_global_search_action(
        &mut self,
        query: &str,
        category: RiseGlobalSearchCategory,
        owner_id: Uuid,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let query = query.to_owned();
        Self::on_domain(cx, move |domain| async move {
            domain.observe_global_search(&query, category, owner_id, generation).await
        })
    }

    pub fn stop_observing_global_search_action(
        &mut self,
        query: &str,
        category: RiseGlobalSearchCategory,
        owner_id: Uuid,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let Some(domain) = RiseSearchDomainRegistry::shared(cx).read(cx).active().cloned() else {
            return Task::ready(());
        };
        let query = query.to_owned();
        cx.spawn(async move |_, _: &mut AsyncApp| {
            domain.stop_observing_global_search(&query, category, owner_id, generation).await;
        })
    }

    pub fn refresh_global_search_action(
        &mut self,
        query: &str,
        category: RiseGlobalSearchCategory,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let query = query.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.refresh_global_search(&query, category).await })
    }

    pub fn load_more_global_search_action(
        &mut self,
        query: &str,
        category: RiseGlobalSearchCategory,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let query = query.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.load_more_global_search(&query, category).await })
    }

    pub fn global_search_chats_action(
        &mut self,
        query: &str,
        relative_id: Option<String>,
        need_pending: bool,
        fetch_if_have_data: bool,
        cx: &mut Context<Self>,
    ) {
        let clean = trimming_characters(trimming_whitespaces_and_newlines(query), |c| c == '@');
        if clean.is_empty() {
            return;
        }
        let _ = need_pending;
        let _ = fetch_if_have_data;
        let task = if relative_id.is_none() {
            self.activate_global_search_action(clean, RiseGlobalSearchCategory::Channels, cx)
        } else {
            self.load_more_global_search_action(clean, RiseGlobalSearchCategory::Channels, cx)
        };
        task.detach();
    }

    pub fn global_search_by_tag_action(
        &mut self,
        tag: &str,
        relative_id: Option<String>,
        need_pending: bool,
        fetch_if_have_data: bool,
        cx: &mut Context<Self>,
    ) {
        let clean = trimming_characters(trimming_whitespaces_and_newlines(tag), |c| c == '@');
        if clean.is_empty() {
            return;
        }

        let user_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone());

        let body = rise_request::tag::GlobalSearchByTag {
            limit: Some(SearchServicesStore::search_tag_limit()),
            tag: Some(clean.to_owned()),
            up: Some(false),
            relative_id: relative_id
                .as_deref()
                .map(trimming_whitespaces_and_newlines)
                .filter(|relative_id| !relative_id.is_empty())
                .map(str::to_owned),
            ..rise_request::tag::GlobalSearchByTag::default()
        };

        let cache_key = SearchServicesStore::global_search_by_tag_cache_key(clean, user_id.as_deref().unwrap_or(""));

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::tag::SERVICE.to_owned(),
            method: rise_wire_methods::tag::GLOBAL_SEARCH_BY_TAG.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: fetch_if_have_data,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: need_pending,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::MemoryOnly),
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(rise_key::tag::RESULTS.to_owned()),
            path_to_id: Some(rise_key::user::ENTITY_ID.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id,
                response_relative_id_path: rise_key::post::NEXT_CURSOR.to_owned(),
                has_more_path: Some(rise_key::post::HAS_MORE.to_owned()),
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, cx: &mut App| {
            SearchServicesStore::shared(cx).update(cx, |services, cx| services.global_search_by_tag_success_handler(cx));
        });
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            SearchServicesStore::shared(cx)
                .update(cx, |services, cx| services.global_search_by_tag_error_handler(error, cx));
        });

        SaiWs::request_slice(&self.global_search_by_tag_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn search_users_action(
        &mut self,
        query: &str,
        relative_id: Option<String>,
        need_pending: bool,
        fetch_if_have_data: bool,
        cx: &mut Context<Self>,
    ) {
        let q = trimming_whitespaces_and_newlines(query);
        if q.is_empty() {
            return;
        }

        let body = rise_request::user::SearchUsers {
            query: q.to_owned(),
            limit: Some(SearchServicesStore::search_users_limit()),
            up: Some(false),
            relative_id: Self::outbound_relative_id(relative_id.as_deref()),
        };

        let cache_key = SearchServicesStore::search_users_cache_key(q);
        let (logged_query, logged_cursor) = (q.to_owned(), relative_id.clone());

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::user::SERVICE.to_owned(),
            method: rise_wire_methods::user::SEARCH_USERS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: fetch_if_have_data,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: need_pending,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(rise_key::user::LIST.to_owned()),
            path_to_id: Some(rise_key::user::ID.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id,
                has_more_path: Some(rise_key::post::IS_HAVE_MORE.to_owned()),
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let page = SearchServicesStore::shared(cx).update(cx, |services, cx| services.search_users_page(&logged_query, cx));
            tracing::warn!(
                target: "riseonly::X2145",
                query = %logged_query,
                requested = ?logged_cursor,
                rows = page.as_ref().map_or(0, |page| page.list.len()),
                has_more = page.as_ref().is_some_and(|page| page.is_have_more),
                next = ?page.as_ref().and_then(|page| page.relative_id),
                "search users page"
            );
            SearchServicesStore::shared(cx).update(cx, |services, cx| services.search_users_success_handler(cx));
        });
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            SearchServicesStore::shared(cx).update(cx, |services, cx| services.search_users_error_handler(error, cx));
        });

        SaiWs::request_slice(&self.search_users_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    fn on_domain<Operation, Work>(cx: &mut Context<Self>, operation: Operation) -> Task<()>
    where
        Operation: FnOnce(Arc<RiseSearchDomain>) -> Work + 'static,
        Work: Future<Output = anyhow::Result<()>> + 'static,
    {
        let domain = RiseSearchActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            let _ = operation(domain).await;
        })
    }

    fn outbound_relative_id(value: Option<&str>) -> Option<i64> {
        let value = value.filter(|value| !value.is_empty())?;
        int_from_string(value)
    }
}
