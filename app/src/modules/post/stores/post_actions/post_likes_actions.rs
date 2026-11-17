use std::any::Any;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_key::post as key;
use rise_canon::{rise_request, rise_wire_methods};
use rise_core::foundation::foundation_character::int_from_string;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_services::post_likes_services::PostLikesTarget;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

impl PostActionsStore {
    pub fn get_post_likes_action(
        &mut self,
        target: &PostLikesTarget,
        search: Option<&str>,
        relative_id: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let has_user = AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .is_some_and(|user_id| !user_id.is_empty());
        let Some(target_id_value) = int_from_string(target.id()).filter(|value| *value > 0) else {
            return;
        };
        if !has_user {
            return;
        }

        let normalized_search = PostServicesStore::normalized_post_likes_search(search);
        let cache_key = PostServicesStore::post_likes_cache_key(target, normalized_search.as_deref());
        let is_first_page = relative_id.is_none();

        PostServicesStore::shared(cx).update(cx, |services, cx| services.prepare_post_likes_request(&cache_key, cx));

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: target.wire_service().to_owned(),
            method: target.wire_method().to_owned(),
            cache_key: Some(cache_key.clone()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: is_first_page,
            send_immediately: true,
            store_response_in_cache: true,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::MemoryOnly),
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(key::ITEMS.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id: relative_id.map(|value| value.to_string()),
                response_relative_id_path: key::RELATIVE_ID.to_owned(),
                has_more_path: Some(key::HAS_MORE.to_owned()),
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        });

        let success_key = cache_key.clone();
        let on_success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            PostServicesStore::shared(cx)
                .update(cx, |services, cx| services.clear_post_likes_failure(&success_key, cx));
        });
        let wire_method = target.wire_method();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            PostServicesStore::shared(cx).update(cx, |services, cx| services.mark_post_likes_failure(&cache_key, cx));
            let text = format!("{wire_method} error: {error}");
            Sai::log("Post", move || text, SaiLogKind::Network, cx);
        });

        match target {
            PostLikesTarget::Post(_) => SaiWs::request_slice(
                &self.get_post_likes_sai,
                rise_request::post::GetPostLikes {
                    limit: Some(30),
                    post_id: Some(target_id_value),
                    relative_id,
                    search: normalized_search,
                },
                options,
                Some(on_success),
                Some(on_error),
                cx,
            ),
            PostLikesTarget::Comment(_) => SaiWs::request_slice(
                &self.get_post_likes_sai,
                rise_request::comment::GetCommentLikes {
                    comment_id: Some(target_id_value),
                    limit: Some(30),
                    relative_id,
                    search: normalized_search,
                },
                options,
                Some(on_success),
                Some(on_error),
                cx,
            ),
        }
    }
}

impl PostLikesTarget {
    pub fn wire_service(&self) -> &'static str {
        match self {
            Self::Post(_) => rise_wire_methods::post::SERVICE,
            Self::Comment(_) => rise_wire_methods::comment::SERVICE,
        }
    }

    pub fn wire_method(&self) -> &'static str {
        match self {
            Self::Post(_) => rise_wire_methods::post::GET_POST_LIKES,
            Self::Comment(_) => rise_wire_methods::comment::GET_COMMENT_LIKES,
        }
    }
}
