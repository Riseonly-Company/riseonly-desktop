use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{App, Context, Entity};
use rise_canon::rise_key::{chat as chat_key, message as message_key};
use rise_canon::{RiseLimits, RiseRequestBody};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiTypedSlice};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::rise_engine_bridge::rise_canon_fields::RiseCanonFields;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::share_extension::stores::share_targets::share_targets_services::ShareTargetsServicesStore;
use crate::modules::share_extension::stores::share_targets::share_targets_wire_types::{
    GetShareTargetsGatewayDTO, GetShareTopicsGatewayDTO,
};

type ShareTargetsSlice = Rc<SaiTypedSlice<GetShareTargetsGatewayDTO>>;
type ShareTopicsSlice = Rc<SaiTypedSlice<GetShareTopicsGatewayDTO>>;

const SEARCH_SLICE_CAPACITY: usize = 12;
const BASE_SLICE_CAPACITY: usize = 16;
const TOPIC_SLICE_CAPACITY: usize = 12;

static PAGE_LIMIT: LazyLock<i64> = LazyLock::new(|| {
    let service_clamp = RiseLimits::service_value("get_share_targets_limit_clamp");
    let limit_pointer = format!("/{}", chat_key::LIMIT);
    let endpoint = rise_canon::rise_request::chat::GetShareTargets::CONTRACT;
    let Some(contract_max) = RiseCanonFields::required(endpoint, &limit_pointer)
        .and_then(|address| address.bound)
        .and_then(|bound| bound.max)
    else {
        return service_clamp;
    };
    contract_max.min(service_clamp)
});

pub struct ShareTargetsActionsStore {
    request_epoch: Option<String>,
    pub get_share_targets_sai: ShareTargetsSlice,
    pub get_share_targets_page_sai: ShareTargetsSlice,
    search_pages_in_flight: HashSet<String>,
    pub get_share_target_base_slices: HashMap<String, ShareTargetsSlice>,
    pub get_share_target_base_cache_order: Vec<String>,
    pub get_share_target_search_slices: HashMap<String, ShareTargetsSlice>,
    pub get_share_target_search_cache_order: Vec<String>,
    pub get_share_topic_slices: HashMap<String, ShareTopicsSlice>,
    pub get_share_topic_cache_order: Vec<String>,
}

impl ShareTargetsActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            request_epoch: None,
            get_share_targets_sai: SaiTypedSlice::new(cx),
            get_share_targets_page_sai: SaiTypedSlice::new(cx),
            search_pages_in_flight: HashSet::new(),
            get_share_target_base_slices: HashMap::new(),
            get_share_target_base_cache_order: Vec::new(),
            get_share_target_search_slices: HashMap::new(),
            get_share_target_search_cache_order: Vec::new(),
            get_share_topic_slices: HashMap::new(),
            get_share_topic_cache_order: Vec::new(),
        }
    }

    fn page_limit() -> i64 {
        *PAGE_LIMIT
    }

    fn synchronize_request_epoch(&mut self, cx: &mut Context<Self>) {
        let epoch = SaiAccountCacheScope::active_epoch();
        if self.request_epoch == epoch {
            return;
        }
        self.request_epoch = epoch;
        self.get_share_targets_sai.reset_to_idle();
        self.get_share_targets_page_sai.reset_to_idle();
        self.search_pages_in_flight.clear();
        self.get_share_target_base_slices.clear();
        self.get_share_target_base_cache_order.clear();
        self.get_share_target_search_slices.clear();
        self.get_share_target_search_cache_order.clear();
        self.get_share_topic_slices.clear();
        self.get_share_topic_cache_order.clear();
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_share_targets_action(
        &mut self,
        query: Option<&str>,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        on_success: impl Fn(GetShareTargetsGatewayDTO, &mut App) + 'static,
        on_error: impl Fn(&anyhow::Error, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        self.synchronize_request_epoch(cx);
        let effective_query = Self::effective_query(query);
        let body = rise_canon::rise_request::chat::GetShareTargets {
            has_documents: Some(has_documents),
            has_link: Some(has_link),
            has_media: Some(has_media),
            limit: Some(Self::page_limit()),
            offset: None,
            q: effective_query.clone(),
        };
        let cache_key = ShareTargetsServicesStore::targets_cache_key(
            &user_id,
            effective_query.as_deref(),
            has_media,
            has_link,
            has_documents,
        );
        let uses_snapshot_slice = effective_query.is_none() && !has_media && !has_link && !has_documents;
        let request_slice = if uses_snapshot_slice {
            Rc::clone(&self.get_share_targets_sai)
        } else if effective_query.is_none() {
            self.target_base_request_slice(&cache_key, cx)
        } else {
            self.target_request_slice(&cache_key, cx)
        };

        if request_slice.is_pending() {
            return;
        }
        request_slice.mark_pending();

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_canon::rise_wire_methods::chat::GET_SHARE_TARGETS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(chat_key::TARGETS.to_owned()),
            path_to_id: Some(message_key::CHAT_ID.to_owned()),
            ..SaiWsRequestOptionsInit::default()
        });
        let rejected_slice = Rc::clone(&request_slice);
        let on_error_rejecting: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            rejected_slice.mark_rejected(error);
            on_error(error, cx);
        });

        SaiWs::request_slice(
            &request_slice,
            body,
            options,
            Some(Self::decoded_success(on_success)),
            Some(on_error_rejecting),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_share_targets_page_action(
        &mut self,
        offset: i64,
        query: Option<&str>,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        on_success: impl Fn(GetShareTargetsGatewayDTO, &mut App) + 'static,
        on_error: impl Fn(&anyhow::Error, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if Self::wire_user_id(cx).is_none() {
            return;
        }

        self.synchronize_request_epoch(cx);
        let effective_query = Self::effective_query(query);
        let search_page_key = effective_query
            .as_ref()
            .map(|query| format!("{}#{offset}", query.to_lowercase()));
        if let Some(search_page_key) = &search_page_key {
            if !self.search_pages_in_flight.insert(search_page_key.clone()) {
                return;
            }
            cx.notify();
        }
        let request_slice = if effective_query.is_none() {
            Rc::clone(&self.get_share_targets_page_sai)
        } else {
            SaiTypedSlice::new(cx)
        };
        if request_slice.is_pending() {
            return;
        }
        request_slice.mark_pending();

        let body = rise_canon::rise_request::chat::GetShareTargets {
            has_documents: Some(has_documents),
            has_link: Some(has_link),
            has_media: Some(has_media),
            limit: Some(Self::page_limit()),
            offset: Some(offset),
            q: effective_query,
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_canon::rise_wire_methods::chat::GET_SHARE_TARGETS.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let decoded = Self::decoded_success(on_success);
        let success_store = cx.weak_entity();
        let success_page_key = search_page_key.clone();
        let on_success_releasing: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(search_page_key) = &success_page_key {
                let _ = success_store.update(cx, |store, cx| {
                    store.search_pages_in_flight.remove(search_page_key);
                    cx.notify();
                });
            }
            decoded(payload, cx);
        });
        let error_store = cx.weak_entity();
        let rejected_slice = Rc::clone(&request_slice);
        let on_error_releasing: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(search_page_key) = &search_page_key {
                let _ = error_store.update(cx, |store, cx| {
                    store.search_pages_in_flight.remove(search_page_key);
                    cx.notify();
                });
            }
            rejected_slice.mark_rejected(error);
            on_error(error, cx);
        });

        SaiWs::request_slice(
            &request_slice,
            body,
            options,
            Some(on_success_releasing),
            Some(on_error_releasing),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_share_topics_action(
        &mut self,
        chat_id: &str,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        on_success: impl Fn(GetShareTopicsGatewayDTO, &mut App) + 'static,
        on_error: impl Fn(&anyhow::Error, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };
        if chat_id.is_empty() {
            return;
        }

        self.synchronize_request_epoch(cx);
        let body = rise_canon::rise_request::chat::GetShareTopics {
            chat_id: chat_id.to_owned(),
            has_documents: Some(has_documents),
            has_link: Some(has_link),
            has_media: Some(has_media),
        };

        let cache_key =
            ShareTargetsServicesStore::topics_cache_key(&user_id, chat_id, has_media, has_link, has_documents);

        let request_slice = self.topic_request_slice(&cache_key, cx);
        if request_slice.is_pending() {
            return;
        }
        request_slice.mark_pending();

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_canon::rise_wire_methods::chat::GET_SHARE_TOPICS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            ..SaiWsRequestOptionsInit::default()
        });
        let rejected_slice = Rc::clone(&request_slice);
        let on_error_rejecting: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            rejected_slice.mark_rejected(error);
            on_error(error, cx);
        });

        SaiWs::request_slice(
            &request_slice,
            body,
            options,
            Some(Self::decoded_success(on_success)),
            Some(on_error_rejecting),
            cx,
        );
    }

    fn target_request_slice(&mut self, cache_key: &str, cx: &mut Context<Self>) -> ShareTargetsSlice {
        let slice = Self::bounded_slice(
            &mut self.get_share_target_search_slices,
            &mut self.get_share_target_search_cache_order,
            SEARCH_SLICE_CAPACITY,
            cache_key,
            cx,
        );
        cx.notify();
        slice
    }

    fn target_base_request_slice(&mut self, cache_key: &str, cx: &mut Context<Self>) -> ShareTargetsSlice {
        let slice = Self::bounded_slice(
            &mut self.get_share_target_base_slices,
            &mut self.get_share_target_base_cache_order,
            BASE_SLICE_CAPACITY,
            cache_key,
            cx,
        );
        cx.notify();
        slice
    }

    fn topic_request_slice(&mut self, cache_key: &str, cx: &mut Context<Self>) -> ShareTopicsSlice {
        let slice = Self::bounded_slice(
            &mut self.get_share_topic_slices,
            &mut self.get_share_topic_cache_order,
            TOPIC_SLICE_CAPACITY,
            cache_key,
            cx,
        );
        cx.notify();
        slice
    }

    fn bounded_slice<T: DeserializeOwned + 'static>(
        slices: &mut HashMap<String, Rc<SaiTypedSlice<T>>>,
        cache_order: &mut Vec<String>,
        capacity: usize,
        cache_key: &str,
        cx: &mut App,
    ) -> Rc<SaiTypedSlice<T>> {
        if let Some(existing) = slices.get(cache_key) {
            let existing = Rc::clone(existing);
            cache_order.retain(|key| key != cache_key);
            cache_order.push(cache_key.to_owned());
            return existing;
        }

        if slices.len() >= capacity && !cache_order.is_empty() {
            let evicted_key = cache_order.remove(0);
            slices.remove(&evicted_key);
            SaiWsResponseCache::remove(&evicted_key, SaiCacheCleanLayer::Memory, cx);
        }

        let slice = SaiTypedSlice::new(cx);
        slices.insert(cache_key.to_owned(), Rc::clone(&slice));
        cache_order.push(cache_key.to_owned());
        slice
    }

    fn wire_user_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .filter(|user_id| !user_id.is_empty())
    }

    fn effective_query(query: Option<&str>) -> Option<String> {
        query
            .map(trimming_whitespaces_and_newlines)
            .filter(|normalized| !normalized.is_empty())
            .map(str::to_owned)
    }

    fn decoded_success<T: DeserializeOwned + 'static>(
        on_success: impl Fn(T, &mut App) + 'static,
    ) -> SaiWsOnSuccess {
        Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(payload) = payload else {
                return;
            };
            match payload.downcast::<T>() {
                Ok(dto) => on_success(*dto, cx),
                Err(payload) => {
                    if let Some(dto) = SaiJSONCodec::decode::<T>(payload.downcast_ref::<Value>()) {
                        on_success(dto, cx);
                    }
                }
            }
        })
    }
}
