use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_key::stories as key;
use rise_canon::{RiseLimits, rise_request, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::modules::story::stores::story::story_actions::story_actions::{
    StoryActionCloseFriendsHandler, StoryActionDoneHandler, StoryActionErrorHandler, StoryActionTextHandler,
    StoryActionsStore,
};
use crate::modules::story::stores::story::story_actions::story_failure_text::StoryFailureText;
use crate::modules::story::stores::story::story_actions::story_wire_types::StoryAuthorDTO;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;

impl StoryActionsStore {
    /// same service ceiling that bounds an ad hoc story audience
    pub fn close_friends_limit() -> i64 {
        RiseLimits::service_value("max_audience_user_ids")
    }

    pub fn get_close_friends_action(
        &mut self,
        on_success: Option<StoryActionCloseFriendsHandler>,
        on_error: Option<StoryActionErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        if Self::wire_user_id(cx).is_none() {
            return;
        }

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::story::SERVICE.to_owned(),
            method: rise_wire_methods::story::GET_CLOSE_FRIENDS.to_owned(),
            cache_key: Some(StoryServicesStore::close_friends_cache_key()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(key::FRIENDS.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Rc::clone(&self.get_close_friends_sai);
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(response) = Self::payload(payload, &slice)
                && let Some(on_success) = &on_success
            {
                on_success(response, cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("get_close_friends error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
        });

        SaiWs::request_slice(
            &self.get_close_friends_sai,
            rise_request::story::GetCloseFriends {},
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn set_close_friends_action(
        &mut self,
        friends: &[StoryAuthorDTO],
        on_success: StoryActionDoneHandler,
        on_error: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        if Self::wire_user_id(cx).is_none() {
            return;
        }

        let services = StoryServicesStore::shared(cx);
        let mutation = services.update(cx, |services, cx| {
            let transforms = services.close_friends_replace_transforms(friends);
            services.begin_story_mutation(transforms, cx)
        });

        let ids = friends.iter().map(|friend| friend.id.clone()).filter(|id| !id.is_empty()).collect::<Vec<_>>();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::story::SERVICE.to_owned(),
            method: rise_wire_methods::story::SET_CLOSE_FRIENDS.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: false,
            send_immediately: true,
            allows_deferred_send: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Rc::clone(&self.set_close_friends_sai);
        let (success_services, success_mutation) = (services.clone(), mutation.clone());
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response = Self::payload(payload, &slice);
            success_services.update(cx, |services, cx| {
                let authoritative = response
                    .map(|response| services.close_friends_replace_transforms(&response.friends))
                    .unwrap_or_else(HashMap::new);
                services.commit_story_mutation(&success_mutation, authoritative, cx);
            });
            on_success(cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("set_close_friends error: {error}"), cx);
            services.update(cx, |services, cx| services.rollback_story_mutation(&mutation, cx));
            on_error(StoryFailureText::message(error), cx);
        });

        SaiWs::request_slice(
            &self.set_close_friends_sai,
            rise_request::story::SetCloseFriends { friend_ids: Some(ids) },
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn set_close_friend_action(
        &mut self,
        author: &StoryAuthorDTO,
        is_close: bool,
        on_success: StoryActionDoneHandler,
        on_limit_reached: StoryActionDoneHandler,
        on_error: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        if Self::wire_user_id(cx).is_none() {
            return;
        }
        let target_id = trimming_whitespaces_and_newlines(&author.id).to_owned();
        if target_id.is_empty() {
            return;
        }

        let services = StoryServicesStore::shared(cx);
        let close_count = services.update(cx, |services, cx| services.close_friends(cx).len());
        if is_close && i64::try_from(close_count).unwrap_or(i64::MAX) >= Self::close_friends_limit() {
            on_limit_reached(cx);
            return;
        }

        let mutation = services.update(cx, |services, cx| {
            let transforms = services.close_friend_toggle_transforms(author, is_close);
            services.begin_story_mutation(transforms, cx)
        });

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::story::SERVICE.to_owned(),
            method: rise_wire_methods::story::SET_CLOSE_FRIEND.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            track_until_acknowledged: true,
            ..SaiWsRequestOptionsInit::default()
        });

        let (success_services, success_mutation) = (services.clone(), mutation.clone());
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            success_services
                .update(cx, |services, cx| services.commit_story_mutation(&success_mutation, HashMap::new(), cx));
            on_success(cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("set_close_friend error: {error}"), cx);
            services.update(cx, |services, cx| services.rollback_story_mutation(&mutation, cx));
            on_error(StoryFailureText::message(error), cx);
        });

        SaiWs::request_slice(
            &self.set_close_friend_sai,
            rise_request::story::SetCloseFriend::new(is_close, target_id),
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }
}
