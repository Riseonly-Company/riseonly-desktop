use std::any::Any;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_key::post as key;
use rise_canon::{rise_request, rise_wire_methods};
use rise_core::foundation::foundation_character::int_from_string;

use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationCursorFallback, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

impl PostActionsStore {
    pub fn get_posts_by_track_action(
        &mut self,
        track_id: &str,
        relative_id: Option<&str>,
        need_pending: bool,
        fetch_if_have_data: bool,
        cx: &mut Context<Self>,
    ) {
        if track_id.is_empty() {
            return;
        }
        if relative_id.is_none() {
            PostServicesStore::shared(cx).update(cx, |services, cx| services.mark_audio_posts_pending(track_id, cx));
        }

        let mut body = rise_request::post::GetPostsByTrack {
            limit: Some(20),
            track_id: Some(track_id.to_owned()),
            ..rise_request::post::GetPostsByTrack::default()
        };
        if let Some(int_id) = relative_id.filter(|value| !value.is_empty()).and_then(int_from_string) {
            body.relative_id = Some(int_id);
        }

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::post::SERVICE.to_owned(),
            method: rise_wire_methods::post::GET_POSTS_BY_TRACK.to_owned(),
            cache_key: Some(PostServicesStore::audio_posts_cache_key(track_id)),
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

        let success_track_id = track_id.to_owned();
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            PostServicesStore::shared(cx).update(cx, |services, cx| {
                services.get_posts_by_track_success_handler(&success_track_id, payload, cx);
            });
        });
        let error_track_id = track_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            PostServicesStore::shared(cx).update(cx, |services, cx| {
                services.get_posts_by_track_error_handler(&error_track_id, error, cx);
            });
        });

        SaiWs::request_slice(&self.get_posts_by_track_sai, body, options, Some(on_success), Some(on_error), cx);
    }
}
