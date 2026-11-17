use std::any::Any;
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_canon::rise_key::post as key;
use rise_canon::{rise_request, rise_wire_methods};
use rise_core::foundation::foundation_character::int_from_string;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiTypedSlice, SaiWireEmptyDTO};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationCursorFallback, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::post::stores::post_actions::post_wire_types::GetFeedGatewayDTO;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

pub struct ShortsActionsStore {
    pub get_shorts_feed_sai: Rc<SaiTypedSlice<GetFeedGatewayDTO>>,
    pub not_interested_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
}

impl ShortsActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self { get_shorts_feed_sai: SaiTypedSlice::new(cx), not_interested_sai: SaiTypedSlice::new(cx) }
    }

    pub fn get_shorts_feed_action(
        &mut self,
        relative_id: Option<&str>,
        need_pending: bool,
        fetch_if_have_data: bool,
        cx: &mut Context<Self>,
    ) {
        if self.get_shorts_feed_sai.is_pending() {
            return;
        }

        let mut body =
            rise_request::post::GetShortsFeed { include_image_reels: Some(true), limit: Some(10), relative_id: None };
        if let Some(int_id) = relative_id.and_then(int_from_string) {
            body.relative_id = Some(int_id);
        }

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::post::SERVICE.to_owned(),
            method: rise_wire_methods::post::GET_SHORTS_FEED.to_owned(),
            cache_key: Some(ShortsServicesStore::feed_cache_key()),
            reload_if_cached: fetch_if_have_data,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: need_pending,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::MemoryOnly),
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(key::LIST.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id: relative_id.map(str::to_owned),
                response_relative_id_fallback: Some(SaiWsPaginationCursorFallback::LastItemId),
                has_more_path: Some(key::IS_HAVE_MORE.to_owned()),
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(|payload: Option<Box<dyn Any>>, cx: &mut App| {
            ShortsServicesStore::shared(cx).update(cx, |services, cx| {
                services.feed_page_success_handler(payload, cx);
            });
        });
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            let text = error.to_string();
            ShortsServicesStore::shared(cx).update(cx, |services, cx| {
                services.set_last_error_text(Some(text.clone()), cx);
            });
            Sai::log("Shorts", move || format!("get_shorts_feed error: {text}"), SaiLogKind::Network, cx);
        });
        SaiWs::request_slice(&self.get_shorts_feed_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn not_interested_action(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if post_id.is_empty() {
            return;
        }
        let Some(int_id) = int_from_string(post_id) else {
            return;
        };
        let services = ShortsServicesStore::shared(cx);
        let removed = services.update(cx, |services, cx| {
            services.hide_short(post_id, cx);
            services.remove_short_snapshot(post_id, cx)
        });

        let has_me = AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| !profile.id.is_empty());
        if !has_me {
            return;
        }

        let body = rise_request::post::NotInterested { post_id: Some(int_id) };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::post::SERVICE.to_owned(),
            method: rise_wire_methods::post::NOT_INTERESTED.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {});
        let error_post_id = post_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let removed = removed.clone();
            ShortsServicesStore::shared(cx).update(cx, |services, cx| {
                services.unhide_short(&error_post_id, cx);
                if let Some((snapshot, index)) = removed {
                    services.restore_short_snapshot(snapshot, index, cx);
                }
            });
            let text = error.to_string();
            Sai::log("Shorts", move || format!("not_interested error: {text}"), SaiLogKind::Network, cx);
        });
        SaiWs::request_slice(&self.not_interested_sai, body, options, Some(on_success), Some(on_error), cx);
    }
}
