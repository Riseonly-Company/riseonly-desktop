use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_canon::rise_key::stories as key;
use rise_canon::{RiseWireRequestMetadata, rise_request, rise_response, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::{CancellationError, RiseWireMethodDescriptor, RiseWireReplayPolicy};
use rise_i18n::tr;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{
    SaiSliceState, SaiTypedSlice, SaiWireEmptyDTO, SaiWsSliceStatus,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_canon_endpoints::{RiseCanonEndpoints, RiseEndpointClientTimeout};
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::story::shared::story_stealth_preference::StoryStealthPreference;
use crate::modules::story::stores::story::story_actions::story_failure_text::StoryFailureText;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::{
    GetCloseFriendsGatewayDTO, GetProfileStoriesGatewayDTO,
};
use crate::modules::story::stores::story::story_actions::story_wire_types::{
    CreateHighlightGatewayDTO, CreateStoryGatewayDTO, GetHighlightStoriesGatewayDTO, GetStoriesFeedGatewayDTO,
    GetStoryGatewayDTO, GetStoryViewersGatewayDTO, GetUserHighlightsGatewayDTO, GetUserStoriesGatewayDTO,
    HighlightDTO, HighlightScope, RemoveStoryFromHighlightGatewayDTO, StoryDTO, ViewStoryGatewayDTO,
};
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::{StoryCacheTransform, StoryServicesStore};

pub type StoryActionStoryHandler = Rc<dyn Fn(StoryDTO, &mut App)>;
pub type StoryActionHighlightHandler = Rc<dyn Fn(HighlightDTO, &mut App)>;
pub type StoryActionTextHandler = Rc<dyn Fn(String, &mut App)>;
pub type StoryActionErrorHandler = Rc<dyn Fn(&anyhow::Error, &mut App)>;
pub type StoryActionDoneHandler = Rc<dyn Fn(&mut App)>;
pub type StoryActionResultHandler = Rc<dyn Fn(bool, &mut App)>;
pub type StoryActionUserStoriesHandler = Rc<dyn Fn(GetUserStoriesGatewayDTO, &mut App)>;
pub type StoryActionCloseFriendsHandler = Rc<dyn Fn(GetCloseFriendsGatewayDTO, &mut App)>;

pub struct StoryActionsStore {
    pub create_story_sai: Rc<SaiTypedSlice<CreateStoryGatewayDTO>>,
    pub get_stories_feed_sai: Rc<SaiTypedSlice<GetStoriesFeedGatewayDTO>>,
    pub get_profile_stories_sai: Rc<SaiTypedSlice<GetProfileStoriesGatewayDTO>>,
    pub set_stories_archived_sai: Rc<SaiTypedSlice<rise_response::story::SetStoriesArchived>>,
    pub set_stories_pinned_sai: Rc<SaiTypedSlice<rise_response::story::SetStoriesPinned>>,
    pub delete_stories_sai: Rc<SaiTypedSlice<rise_response::story::DeleteStories>>,
    pub update_story_sai: Rc<SaiTypedSlice<rise_response::story::UpdateStory>>,
    pub get_close_friends_sai: Rc<SaiTypedSlice<GetCloseFriendsGatewayDTO>>,
    pub set_close_friends_sai: Rc<SaiTypedSlice<rise_response::story::SetCloseFriends>>,
    pub set_close_friend_sai: Rc<SaiTypedSlice<rise_response::story::SetCloseFriend>>,
    pub get_user_stories_sai: Rc<SaiTypedSlice<GetUserStoriesGatewayDTO>>,
    pub view_story_sai: Rc<SaiTypedSlice<ViewStoryGatewayDTO>>,
    pub like_story_sai: Rc<SaiTypedSlice<rise_response::story::LikeStory>>,
    pub unlike_story_sai: Rc<SaiTypedSlice<rise_response::story::UnlikeStory>>,
    pub delete_story_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_stories_state_sai: Rc<SaiTypedSlice<rise_response::story::GetStoriesState>>,
    story_state_flights: HashMap<String, Uuid>,
    pub mute_user_stories_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub unmute_user_stories_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_story_sai: Rc<SaiTypedSlice<GetStoryGatewayDTO>>,
    pub get_story_viewers_sai: Rc<SaiTypedSlice<GetStoryViewersGatewayDTO>>,
    pub create_highlight_sai: Rc<SaiTypedSlice<CreateHighlightGatewayDTO>>,
    pub update_highlight_sai: Rc<SaiTypedSlice<rise_response::story::UpdateHighlight>>,
    pub get_user_highlights_sai: Rc<SaiTypedSlice<GetUserHighlightsGatewayDTO>>,
    pub add_stories_to_highlight_sai: Rc<SaiTypedSlice<rise_response::story::AddStoriesToHighlight>>,
    pub get_highlight_sai: Rc<SaiTypedSlice<rise_response::story::GetHighlight>>,
    pub set_highlight_archived_sai: Rc<SaiTypedSlice<rise_response::story::SetHighlightArchived>>,
    pub remove_story_from_highlight_sai: Rc<SaiTypedSlice<RemoveStoryFromHighlightGatewayDTO>>,
    pub delete_highlight_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_highlight_stories_sai: Rc<SaiTypedSlice<GetHighlightStoriesGatewayDTO>>,
}

impl StoryActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            create_story_sai: SaiTypedSlice::new(cx),
            get_stories_feed_sai: SaiTypedSlice::new(cx),
            get_profile_stories_sai: SaiTypedSlice::new(cx),
            set_stories_archived_sai: SaiTypedSlice::new(cx),
            set_stories_pinned_sai: SaiTypedSlice::new(cx),
            delete_stories_sai: SaiTypedSlice::new(cx),
            update_story_sai: SaiTypedSlice::new(cx),
            get_close_friends_sai: SaiTypedSlice::new(cx),
            set_close_friends_sai: SaiTypedSlice::new(cx),
            set_close_friend_sai: SaiTypedSlice::new(cx),
            get_user_stories_sai: SaiTypedSlice::new_unobserved(cx),
            view_story_sai: SaiTypedSlice::new_unobserved(cx),
            like_story_sai: SaiTypedSlice::new_unobserved(cx),
            unlike_story_sai: SaiTypedSlice::new_unobserved(cx),
            delete_story_sai: SaiTypedSlice::new_unobserved(cx),
            get_stories_state_sai: SaiTypedSlice::new(cx),
            story_state_flights: HashMap::new(),
            mute_user_stories_sai: SaiTypedSlice::new_unobserved(cx),
            unmute_user_stories_sai: SaiTypedSlice::new_unobserved(cx),
            get_story_sai: SaiTypedSlice::new(cx),
            get_story_viewers_sai: SaiTypedSlice::new(cx),
            create_highlight_sai: SaiTypedSlice::new(cx),
            update_highlight_sai: SaiTypedSlice::new(cx),
            get_user_highlights_sai: SaiTypedSlice::new(cx),
            add_stories_to_highlight_sai: SaiTypedSlice::new(cx),
            get_highlight_sai: SaiTypedSlice::new(cx),
            set_highlight_archived_sai: SaiTypedSlice::new(cx),
            remove_story_from_highlight_sai: SaiTypedSlice::new(cx),
            delete_highlight_sai: SaiTypedSlice::new(cx),
            get_highlight_stories_sai: SaiTypedSlice::new(cx),
        }
    }

    pub(crate) fn wire_user_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .filter(|user_id| !user_id.is_empty())
    }

    pub(crate) fn log_network(text: String, cx: &mut App) {
        Sai::log("Story", move || text, SaiLogKind::Network, cx);
    }

    pub(crate) fn payload<T: Clone + 'static>(payload: Option<Box<dyn Any>>, slice: &SaiTypedSlice<T>) -> Option<T> {
        payload.and_then(|value| value.downcast::<T>().ok()).map(|value| *value).or_else(|| slice.data())
    }

    fn story_options(method: &str, init: SaiWsRequestOptionsInit) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::story::SERVICE.to_owned(),
            method: method.to_owned(),
            ..init
        })
    }

    fn relative_id_pagination(relative_id: Option<String>) -> SaiWsPaginationOptions {
        SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
            relative_id,
            has_more_path: Some(key::HAS_MORE.to_owned()),
            duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
            ..SaiWsPaginationRelativeIdArgs::default()
        })
    }

    pub fn create_story_action(
        &mut self,
        body: rise_request::story::CreateStory,
        on_success: StoryActionStoryHandler,
        on_transport_drop: Option<StoryActionDoneHandler>,
        on_error: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        let options = Self::story_options(
            rise_wire_methods::story::CREATE_STORY,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: false,
                send_immediately: true,
                store_response_in_cache: false,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let slice = Rc::clone(&self.create_story_sai);
        let error_text = Rc::clone(&on_error);
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response = Self::payload(payload, &slice);
            let Some(story) = response.and_then(|response| response.story) else {
                error_text(tr("story_create_error"), cx);
                return;
            };
            on_success(story, cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("create_story error: {error}"), cx);
            tracing::warn!(target: "riseonly::X807", "create_story error: {error:#} debug={error:?}");
            if let Some(on_transport_drop) = &on_transport_drop
                && StoryFailureText::is_transport_failure(error, cx)
            {
                on_transport_drop(cx);
                return;
            }
            on_error(StoryFailureText::message(error), cx);
        });

        SaiWs::request_slice(&self.create_story_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_stories_feed_action(&mut self, relative_id: Option<String>, cx: &mut Context<Self>) {
        if Self::wire_user_id(cx).is_none() {
            return;
        }

        let is_first_page = relative_id.is_none();
        let cache_key = StoryServicesStore::feed_cache_key();

        let body = rise_request::story::GetStoriesFeed { limit: Some(20), relative_id: relative_id.clone() };

        let options = Self::story_options(
            rise_wire_methods::story::GET_STORIES_FEED,
            SaiWsRequestOptionsInit {
                cache_key: Some(cache_key),
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: is_first_page,
                send_immediately: true,
                store_response_in_cache: true,
                persist_cache_to_disk: true,
                reload_when_only_disk_cached: true,
                preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
                path_to_array: Some(key::TRAY.to_owned()),
                path_to_id: Some(format!("{}.{}", key::AUTHOR, key::ID)),
                pagination: Some(Self::relative_id_pagination(relative_id)),
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let slice = Rc::clone(&self.get_stories_feed_sai);
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response = Self::payload(payload, &slice);
            let services = StoryServicesStore::shared(cx);
            services.update(cx, |services, cx| {
                services.rebase_pending_story_publish(&StoryServicesStore::feed_cache_key(), cx);
            });
            let has_more = response.as_ref().is_none_or(|response| response.has_more);
            let tray = response.map(|response| response.tray).unwrap_or_default();
            if !is_first_page {
                services.update(cx, |services, cx| services.append_feed_page_to_open_player(&tray, cx));
            }
            let unsettled =
                services.update(cx, |services, cx| services.apply_stories_feed(tray, is_first_page, has_more, cx));
            if !unsettled.is_empty() {
                StoryInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.load_stories_state(&unsettled, cx));
            }
        });
        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("get_stories_feed error: {error}"), cx);
        });

        SaiWs::request_slice(&self.get_stories_feed_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn restore_stories_feed_from_cache(&mut self, cx: &mut Context<Self>) {
        if self.get_stories_feed_sai.status() != SaiWsSliceStatus::Idle {
            return;
        }
        let Some(cached) = SaiWsResponseCache::current_payload(
            &StoryServicesStore::feed_cache_key(),
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        ) else {
            return;
        };
        if cached.is_null() {
            return;
        }
        self.get_stories_feed_sai.mark_fulfilled(Some(&cached), None);
        let Some(response) = self.get_stories_feed_sai.data() else {
            return;
        };
        StoryServicesStore::shared(cx).update(cx, |services, cx| {
            services.apply_stories_feed(response.tray, true, response.has_more, cx);
        });
    }

    pub fn get_user_stories_action(
        &mut self,
        target_user_id: Option<String>,
        relative_id: Option<i64>,
        on_success: Option<StoryActionUserStoriesHandler>,
        on_error: Option<StoryActionErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        let is_first_page = relative_id.is_none();
        let target = target_user_id.clone().unwrap_or_else(|| user_id.clone());
        let cache_key = StoryServicesStore::user_stories_cache_key(&target);

        let body = rise_request::story::GetUserStories { limit: Some(50), relative_id, target_user_id };

        let options = Self::story_options(
            rise_wire_methods::story::GET_USER_STORIES,
            SaiWsRequestOptionsInit {
                cache_key: Some(cache_key),
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: is_first_page,
                send_immediately: true,
                store_response_in_cache: true,
                persist_cache_to_disk: false,
                preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
                path_to_array: Some(key::LIST.to_owned()),
                path_to_id: Some(key::ID.to_owned()),
                pagination: Some(Self::relative_id_pagination(relative_id.map(|value| value.to_string()))),
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let slice = Rc::clone(&self.get_user_stories_sai);
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            StoryServicesStore::shared(cx)
                .update(cx, |services, cx| services.did_load_authoritative_stories(&target, cx));
            if let Some(response) = Self::payload(payload, &slice)
                && let Some(on_success) = &on_success
            {
                on_success(response, cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("get_user_stories error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
        });

        SaiWs::request_slice(&self.get_user_stories_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn view_story_action(&mut self, story_id: i64, cx: &mut Context<Self>) {
        if Self::wire_user_id(cx).is_none() {
            return;
        }

        let mut body = rise_request::story::ViewStory { story_id: Some(story_id), ..Default::default() };
        let is_premium =
            AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| profile.more.is_premium);
        if is_premium && StoryStealthPreference::is_enabled() {
            body.stealth = Some(true);
        }

        let options = Self::story_options(
            rise_wire_methods::story::VIEW_STORY,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: true,
                reflect_pending_in_ui: false,
                send_immediately: true,
                store_response_in_cache: false,
                track_until_acknowledged: true,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {});
        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("view_story error: {error}"), cx);
        });

        SaiWs::request_slice(&self.view_story_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn like_story_action(&mut self, story_id: i64, cx: &mut Context<Self>) {
        if Self::wire_user_id(cx).is_none() {
            return;
        }

        StoryServicesStore::shared(cx).update(cx, |services, cx| services.apply_like_optimistic(story_id, true, cx));

        let body = rise_request::story::LikeStory { story_id: Some(story_id) };

        let options = Self::story_options(
            rise_wire_methods::story::LIKE_STORY,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: true,
                reflect_pending_in_ui: false,
                send_immediately: true,
                store_response_in_cache: false,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let slice = Rc::clone(&self.like_story_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let data = slice.data();
            let like_count = data.as_ref().map(|data| data.like_count);
            let is_liked = data.as_ref().is_none_or(|data| data.is_liked);
            StoryServicesStore::shared(cx)
                .update(cx, |services, cx| services.reconcile_like(story_id, like_count, is_liked, cx));
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("like_story error: {error}"), cx);
            StoryServicesStore::shared(cx)
                .update(cx, |services, cx| services.apply_like_optimistic(story_id, false, cx));
        });

        SaiWs::request_slice(&self.like_story_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn unlike_story_action(&mut self, story_id: i64, cx: &mut Context<Self>) {
        if Self::wire_user_id(cx).is_none() {
            return;
        }

        StoryServicesStore::shared(cx).update(cx, |services, cx| services.apply_like_optimistic(story_id, false, cx));

        let body = rise_request::story::UnlikeStory { story_id: Some(story_id) };

        let options = Self::story_options(
            rise_wire_methods::story::UNLIKE_STORY,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: true,
                reflect_pending_in_ui: false,
                send_immediately: true,
                store_response_in_cache: false,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let slice = Rc::clone(&self.unlike_story_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let data = slice.data();
            let like_count = data.as_ref().map(|data| data.like_count);
            let is_liked = data.as_ref().is_some_and(|data| data.is_liked);
            StoryServicesStore::shared(cx)
                .update(cx, |services, cx| services.reconcile_like(story_id, like_count, is_liked, cx));
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("unlike_story error: {error}"), cx);
            StoryServicesStore::shared(cx).update(cx, |services, cx| services.apply_like_optimistic(story_id, true, cx));
        });

        SaiWs::request_slice(&self.unlike_story_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn delete_story_action(&mut self, story_id: i64, author_id: Option<&str>, cx: &mut Context<Self>) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        let services = StoryServicesStore::shared(cx);
        let ring_owner_id = author_id.map(trimming_whitespaces_and_newlines).filter(|owner| !owner.is_empty());
        let ring_user_id = ring_owner_id.map_or(user_id, str::to_owned);
        let mutation = services.update(cx, |services, cx| {
            let transforms = services.delete_story_mutation_transforms(story_id, &ring_user_id, cx);
            let mutation = services.begin_story_mutation(transforms, cx);
            services.reconcile_story_state_from_caches(&ring_user_id, cx);
            services.remove_story_from_open_player(story_id, cx);
            mutation
        });

        let body = rise_request::story::DeleteStory { story_id: Some(story_id) };

        let options = Self::story_options(
            rise_wire_methods::story::DELETE_STORY,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: true,
                reflect_pending_in_ui: false,
                send_immediately: true,
                store_response_in_cache: false,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let (success_services, success_mutation, success_user) =
            (services.clone(), mutation.clone(), ring_user_id.clone());
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            success_services.update(cx, |services, cx| {
                services.commit_story_mutation(&success_mutation, HashMap::new(), cx);
                services.reconcile_story_state_from_caches(&success_user, cx);
                services.note_story_gone(story_id, cx);
            });
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("delete_story error: {error}"), cx);
            services.update(cx, |services, cx| {
                services.rollback_story_mutation(&mutation, cx);
                services.reconcile_story_state_from_caches(&ring_user_id, cx);
            });
        });

        SaiWs::request_slice(&self.delete_story_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_stories_state_action(&mut self, user_ids: &[String], cx: &mut Context<Self>) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };
        let Some(epoch) = SaiAccountCacheScope::active_epoch() else {
            return;
        };
        let mut cleaned = user_ids
            .iter()
            .map(|id| trimming_whitespaces_and_newlines(id).to_owned())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        cleaned.sort();
        if cleaned.is_empty() {
            return;
        }
        let key = SaiCache::new([user_id.clone(), epoch.clone()].into_iter().chain(cleaned.iter().cloned())).key;
        if self.story_state_flights.contains_key(&key) {
            return;
        }
        let identifier = Uuid::new_v4();
        self.story_state_flights.insert(key.clone(), identifier);

        let prepare = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |this, cx| {
            let (expected_user, expected_epoch, requested) = (&user_id, &epoch, &cleaned);
            let outcome: anyhow::Result<Option<rise_response::story::GetStoriesState>> = async move {
                let engine = prepare.await?;
                if engine.account.account_id != *expected_user || engine.account.epoch != *expected_epoch {
                    return Ok(None);
                }
                let endpoint = RiseCanonEndpoints::required(
                    rise_wire_methods::story::SERVICE,
                    rise_wire_methods::story::GET_STORIES_STATE,
                );
                let descriptor = RiseWireMethodDescriptor::new(endpoint.service, endpoint.method)
                    .with_timeout(endpoint.client_timeout())
                    .with_replay_policy(RiseWireReplayPolicy::ReadOnly);
                let response: rise_response::story::GetStoriesState = engine
                    .call_descriptor_response(
                        &descriptor,
                        &rise_request::story::GetStoriesState { user_ids: Some(requested.clone()) },
                        None,
                        RiseWireRequestMetadata { auth_required: true, ..RiseWireRequestMetadata::default() },
                    )
                    .await?;
                Ok(Some(response))
            }
            .await;

            let is_current = SaiAccountCacheScope::is_current_account_epoch(Some(&user_id), Some(&epoch));
            match outcome {
                Ok(Some(response)) if is_current => {
                    cx.update(|cx| {
                        StoryServicesStore::shared(cx)
                            .update(cx, |services, cx| services.apply_stories_state(response.states, &cleaned, cx));
                    });
                }
                Err(error) if is_current && !error.is::<CancellationError>() => {
                    cx.update(|cx| Self::log_network(format!("get_stories_state error: {error}"), cx));
                }
                _ => {}
            }

            let _ = this.update(cx, |this, _| {
                if this.story_state_flights.get(&key) == Some(&identifier) {
                    this.story_state_flights.remove(&key);
                }
            });
        })
        .detach();
    }

    pub fn mute_user_stories_action(
        &mut self,
        target_user_id: &str,
        on_result: StoryActionResultHandler,
        cx: &mut Context<Self>,
    ) {
        Self::user_stories_mute_request(
            target_user_id,
            rise_wire_methods::story::MUTE_USER_STORIES,
            Rc::clone(&self.mute_user_stories_sai),
            on_result,
            cx,
        );
    }

    pub fn unmute_user_stories_action(
        &mut self,
        target_user_id: &str,
        on_result: StoryActionResultHandler,
        cx: &mut Context<Self>,
    ) {
        Self::user_stories_mute_request(
            target_user_id,
            rise_wire_methods::story::UNMUTE_USER_STORIES,
            Rc::clone(&self.unmute_user_stories_sai),
            on_result,
            cx,
        );
    }

    fn user_stories_mute_request(
        target_user_id: &str,
        method: &'static str,
        slice: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
        on_result: StoryActionResultHandler,
        cx: &mut Context<Self>,
    ) {
        if Self::wire_user_id(cx).is_none() {
            on_result(false, cx);
            return;
        }
        let target = trimming_whitespaces_and_newlines(target_user_id).to_owned();
        if target.is_empty() {
            on_result(false, cx);
            return;
        }

        let options = Self::story_options(
            method,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: true,
                reflect_pending_in_ui: false,
                send_immediately: true,
                allows_deferred_send: false,
                store_response_in_cache: false,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let success_result = Rc::clone(&on_result);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| success_result(true, cx));
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("{method} error: {error}"), cx);
            on_result(false, cx);
        });

        if method == rise_wire_methods::story::MUTE_USER_STORIES {
            let body = rise_request::story::MuteUserStories { target_user_id: Some(target) };
            SaiWs::request_slice(&slice, body, options, Some(success), Some(failure), cx);
        } else {
            let body = rise_request::story::UnmuteUserStories { target_user_id: Some(target) };
            SaiWs::request_slice(&slice, body, options, Some(success), Some(failure), cx);
        }
    }

    pub fn get_story_action(
        &mut self,
        story_id: i64,
        on_success: StoryActionStoryHandler,
        on_error: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        if Self::wire_user_id(cx).is_none() {
            on_error(tr("error_fetching_data"), cx);
            return;
        }

        let body = rise_request::story::GetStory { story_id: Some(story_id) };

        let options = Self::story_options(
            rise_wire_methods::story::GET_STORY,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: false,
                send_immediately: true,
                store_response_in_cache: false,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let slice = Rc::clone(&self.get_story_sai);
        let error_text = Rc::clone(&on_error);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(story) = slice.data().and_then(|data| data.story) else {
                error_text(tr("error_fetching_data"), cx);
                return;
            };
            on_success(story, cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("get_story error: {error}"), cx);
            if StoryFailureText::is_story_gone(error) {
                StoryServicesStore::shared(cx).update(cx, |services, cx| services.note_story_gone(story_id, cx));
            }
            on_error(StoryFailureText::message(error), cx);
        });

        SaiWs::request_slice(&self.get_story_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_story_viewers_action(&mut self, story_id: i64, relative_id: Option<i64>, cx: &mut Context<Self>) {
        if Self::wire_user_id(cx).is_none() {
            return;
        }

        let is_first_page = relative_id.is_none();
        let cache_key = StoryServicesStore::viewers_cache_key(story_id);

        let services = StoryServicesStore::shared(cx);
        let has_no_viewers = services.update(cx, |services, cx| {
            services.mark_viewers_in_flight(story_id, cx);
            services.story_viewers(story_id, cx).is_empty()
        });

        let mut body =
            rise_request::story::GetStoryViewers { limit: Some(50), story_id: Some(story_id), ..Default::default() };
        if let Some(relative_id) = relative_id {
            body.relative_id = Some(relative_id);
        }

        let options = Self::story_options(
            rise_wire_methods::story::GET_STORY_VIEWERS,
            SaiWsRequestOptionsInit {
                cache_key: Some(cache_key),
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: is_first_page && has_no_viewers,
                send_immediately: true,
                allows_deferred_send: false,
                store_response_in_cache: true,
                persist_cache_to_disk: false,
                preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
                path_to_array: Some(key::VIEWERS.to_owned()),
                path_to_id: Some(key::ID.to_owned()),
                pagination: Some(Self::relative_id_pagination(relative_id.map(|value| value.to_string()))),
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            StoryServicesStore::shared(cx)
                .update(cx, |services, cx| services.clear_viewers_in_flight(story_id, false, cx));
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("get_story_viewers error: {error}"), cx);
            StoryServicesStore::shared(cx).update(cx, |services, cx| services.clear_viewers_in_flight(story_id, true, cx));
        });

        SaiWs::request_slice(&self.get_story_viewers_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn create_highlight_action(
        &mut self,
        title: &str,
        cover_story_id: Option<i64>,
        story_ids: &[i64],
        on_success: StoryActionHighlightHandler,
        on_error: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            on_error(tr("story_create_error"), cx);
            return;
        };

        let services = StoryServicesStore::shared(cx);
        let highlights_key = StoryServicesStore::highlights_cache_key(&user_id, HighlightScope::Profile);
        let (temporary_highlight_id, mutation) = services.update(cx, |services, cx| {
            services.note_highlights_target(&user_id, cx);
            let temporary_highlight = services.make_temporary_highlight(title, &user_id, story_ids, cx);
            let temporary_highlight_id = temporary_highlight.id;
            let transforms: HashMap<String, StoryCacheTransform> = HashMap::from([(
                highlights_key.clone(),
                StoryServicesStore::upsert_highlight_transform(temporary_highlight, None),
            )]);
            (temporary_highlight_id, services.begin_story_mutation(transforms, cx))
        });

        let body = rise_request::story::CreateHighlight {
            cover_story_id,
            story_ids: if story_ids.is_empty() { None } else { Some(story_ids.to_vec()) },
            title: Some(title.to_owned()),
        };

        let options = Self::story_options(
            rise_wire_methods::story::CREATE_HIGHLIGHT,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: false,
                send_immediately: true,
                allows_deferred_send: false,
                store_response_in_cache: false,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let slice = Rc::clone(&self.create_highlight_sai);
        let (success_services, success_mutation, error_text) = (services.clone(), mutation.clone(), Rc::clone(&on_error));
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response = Self::payload(payload, &slice);
            let Some(highlight) = response.and_then(|response| response.highlight) else {
                success_services.update(cx, |services, cx| services.rollback_story_mutation(&success_mutation, cx));
                error_text(tr("story_create_error"), cx);
                return;
            };
            let authoritative: HashMap<String, StoryCacheTransform> = HashMap::from([(
                highlights_key.clone(),
                StoryServicesStore::upsert_highlight_transform(highlight.clone(), Some(temporary_highlight_id)),
            )]);
            success_services
                .update(cx, |services, cx| services.commit_story_mutation(&success_mutation, authoritative, cx));
            on_success(highlight, cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("create_highlight error: {error}"), cx);
            services.update(cx, |services, cx| services.rollback_story_mutation(&mutation, cx));
            on_error(StoryFailureText::message(error), cx);
        });

        SaiWs::request_slice(&self.create_highlight_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn update_highlight_action(
        &mut self,
        highlight_id: i64,
        title: Option<String>,
        cover_story_id: Option<i64>,
        owner_user_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        let services = StoryServicesStore::shared(cx);
        let cache_owner = Self::highlight_cache_owner(owner_user_id, &user_id);
        let highlights_key = StoryServicesStore::highlights_cache_key(&cache_owner, HighlightScope::Profile);
        let mutation = services.update(cx, |services, cx| {
            services.note_highlights_target(&cache_owner, cx);
            let mut fields = Map::new();
            if let Some(title) = &title {
                fields.insert(key::TITLE.to_owned(), Value::String(title.clone()));
            }
            if let Some(cover_story_id) = cover_story_id {
                let cover = services
                    .highlight_stories(highlight_id, cx)
                    .into_iter()
                    .find(|story| story.id == cover_story_id)
                    .or_else(|| {
                        services.user_stories(&cache_owner, cx).into_iter().find(|story| story.id == cover_story_id)
                    })
                    .or_else(|| {
                        services
                            .highlight_picker_stories(&cache_owner, cx)
                            .into_iter()
                            .find(|story| story.id == cover_story_id)
                    });
                if let Some(row) = cover.as_ref().and_then(SaiJSONCodec::encode) {
                    fields.insert(key::COVER_STORY.to_owned(), Value::Object(row));
                }
            }
            let transforms: HashMap<String, StoryCacheTransform> = HashMap::from([(
                highlights_key.clone(),
                StoryServicesStore::patch_highlight_transform(highlight_id, fields),
            )]);
            services.begin_story_mutation(transforms, cx)
        });

        let body = rise_request::story::UpdateHighlight { cover_story_id, highlight_id: Some(highlight_id), title };

        let options = Self::story_options(
            rise_wire_methods::story::UPDATE_HIGHLIGHT,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: false,
                send_immediately: true,
                store_response_in_cache: false,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let slice = Rc::clone(&self.update_highlight_sai);
        let (success_services, success_mutation) = (services.clone(), mutation.clone());
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response = Self::payload(payload, &slice);
            let authoritative: HashMap<String, StoryCacheTransform> =
                match response.and_then(|response| response.highlight) {
                    Some(highlight) => HashMap::from([(
                        highlights_key.clone(),
                        StoryServicesStore::upsert_highlight_transform(highlight, None),
                    )]),
                    None => HashMap::new(),
                };
            success_services
                .update(cx, |services, cx| services.commit_story_mutation(&success_mutation, authoritative, cx));
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("update_highlight error: {error}"), cx);
            services.update(cx, |services, cx| services.rollback_story_mutation(&mutation, cx));
        });

        SaiWs::request_slice(&self.update_highlight_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_user_highlights_action(
        &mut self,
        target_user_id: Option<&str>,
        scope: HighlightScope,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        let resolved_target = target_user_id.map_or(user_id, str::to_owned);
        let cache_key = StoryServicesStore::highlights_cache_key(&resolved_target, scope);
        let has_no_highlights = StoryServicesStore::shared(cx)
            .update(cx, |services, cx| services.highlights(&resolved_target, scope, cx).is_empty());

        let body = rise_request::story::GetUserHighlights {
            scope: if scope == HighlightScope::Profile { None } else { Some(scope.raw_value().to_owned()) },
            target_user_id: Some(resolved_target),
        };

        let options = Self::story_options(
            rise_wire_methods::story::GET_USER_HIGHLIGHTS,
            SaiWsRequestOptionsInit {
                cache_key: Some(cache_key),
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: has_no_highlights,
                send_immediately: true,
                store_response_in_cache: true,
                persist_cache_to_disk: false,
                preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
                path_to_array: Some(key::HIGHLIGHTS.to_owned()),
                path_to_id: Some(key::ID.to_owned()),
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {});
        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("get_user_highlights error: {error}"), cx);
        });

        SaiWs::request_slice(&self.get_user_highlights_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn add_stories_to_highlight_action(
        &mut self,
        highlight_id: i64,
        story_ids: &[i64],
        owner_user_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        let services = StoryServicesStore::shared(cx);
        let cache_owner = Self::highlight_cache_owner(owner_user_id, &user_id);
        let highlights_key = StoryServicesStore::highlights_cache_key(&cache_owner, HighlightScope::Profile);
        let mutation = services.update(cx, |services, cx| {
            services.note_highlights_target(&cache_owner, cx);
            let transforms = services.highlight_story_mutation_transforms(
                highlight_id,
                &cache_owner,
                story_ids,
                &HashSet::new(),
                cx,
            );
            let mutation = services.begin_story_mutation(transforms, cx);
            services.refresh_open_player_highlight(highlight_id, cx);
            mutation
        });

        let body =
            rise_request::story::AddStoriesToHighlight { highlight_id: Some(highlight_id), story_ids: Some(story_ids.to_vec()) };

        let options = Self::story_options(
            rise_wire_methods::story::ADD_STORIES_TO_HIGHLIGHT,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: false,
                send_immediately: true,
                store_response_in_cache: false,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let slice = Rc::clone(&self.add_stories_to_highlight_sai);
        let (success_services, success_mutation) = (services.clone(), mutation.clone());
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response = Self::payload(payload, &slice);
            let authoritative: HashMap<String, StoryCacheTransform> =
                match response.and_then(|response| response.highlight) {
                    Some(highlight) => HashMap::from([(
                        highlights_key.clone(),
                        StoryServicesStore::upsert_highlight_transform(highlight, None),
                    )]),
                    None => HashMap::new(),
                };
            success_services.update(cx, |services, cx| {
                services.commit_story_mutation(&success_mutation, authoritative, cx);
                services.refresh_open_player_highlight(highlight_id, cx);
            });
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("add_stories_to_highlight error: {error}"), cx);
            services.update(cx, |services, cx| {
                services.rollback_story_mutation(&mutation, cx);
                services.refresh_open_player_highlight(highlight_id, cx);
            });
        });

        SaiWs::request_slice(&self.add_stories_to_highlight_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_highlight_action(
        &mut self,
        highlight_id: i64,
        on_success: StoryActionHighlightHandler,
        on_error: StoryActionErrorHandler,
        cx: &mut Context<Self>,
    ) {
        if Self::wire_user_id(cx).is_none() {
            on_error(&Self::fetching_data_error(), cx);
            return;
        }

        let body = rise_request::story::GetHighlight::new(highlight_id);

        let options = Self::story_options(
            rise_wire_methods::story::GET_HIGHLIGHT,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: false,
                send_immediately: true,
                store_response_in_cache: false,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let slice = Rc::clone(&self.get_highlight_sai);
        let error_handler = Rc::clone(&on_error);
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response = Self::payload(payload, &slice);
            let Some(highlight) = response.and_then(|response| response.highlight) else {
                error_handler(&Self::fetching_data_error(), cx);
                return;
            };
            on_success(highlight, cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("get_highlight error: {error}"), cx);
            on_error(error, cx);
        });

        SaiWs::request_slice(&self.get_highlight_sai, body, options, Some(success), Some(failure), cx);
    }

    fn fetching_data_error() -> anyhow::Error {
        anyhow::Error::new(SaiWsResponseError { message: tr("error_fetching_data"), ..SaiWsResponseError::default() })
    }

    pub fn set_highlight_archived_action(
        &mut self,
        highlight: &HighlightDTO,
        archived: bool,
        owner_user_id: Option<&str>,
        on_error: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        let services = StoryServicesStore::shared(cx);
        let cache_owner = Self::highlight_cache_owner(owner_user_id, &user_id);
        let mutation = services.update(cx, |services, cx| {
            services.note_highlights_target(&cache_owner, cx);
            let transforms = services.archive_highlight_transforms(highlight, archived, &cache_owner, cx);
            services.begin_story_mutation(transforms, cx)
        });

        let mut body = rise_request::story::SetHighlightArchived::new(highlight.id);
        body.archived = Some(archived);

        let options = Self::story_options(
            rise_wire_methods::story::SET_HIGHLIGHT_ARCHIVED,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: false,
                send_immediately: true,
                store_response_in_cache: false,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let slice = Rc::clone(&self.set_highlight_archived_sai);
        let (success_services, success_mutation) = (services.clone(), mutation.clone());
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response = Self::payload(payload, &slice);
            let destination = if archived { HighlightScope::Archive } else { HighlightScope::Profile };
            success_services.update(cx, |services, cx| {
                let authoritative: HashMap<String, StoryCacheTransform> =
                    match response.and_then(|response| response.highlight) {
                        Some(row) if services.highlights_cache_exists(&cache_owner, destination, cx) => {
                            HashMap::from([(
                                StoryServicesStore::highlights_cache_key(&cache_owner, destination),
                                StoryServicesStore::upsert_highlight_transform(row, None),
                            )])
                        }
                        _ => HashMap::new(),
                    };
                services.commit_story_mutation(&success_mutation, authoritative, cx);
            });
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("set_highlight_archived error: {error}"), cx);
            services.update(cx, |services, cx| services.rollback_story_mutation(&mutation, cx));
            on_error(StoryFailureText::message(error), cx);
        });

        SaiWs::request_slice(&self.set_highlight_archived_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn remove_story_from_highlight_action(
        &mut self,
        highlight_id: i64,
        story_id: i64,
        owner_user_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        let services = StoryServicesStore::shared(cx);
        let cache_owner = Self::highlight_cache_owner(owner_user_id, &user_id);
        let highlights_key = StoryServicesStore::highlights_cache_key(&cache_owner, HighlightScope::Profile);
        let mutation = services.update(cx, |services, cx| {
            services.note_highlights_target(&cache_owner, cx);
            let transforms = services.highlight_story_mutation_transforms(
                highlight_id,
                &cache_owner,
                &[],
                &HashSet::from([story_id]),
                cx,
            );
            let mutation = services.begin_story_mutation(transforms, cx);
            services.refresh_open_player_highlight(highlight_id, cx);
            mutation
        });

        let body =
            rise_request::story::RemoveStoryFromHighlight { highlight_id: Some(highlight_id), story_id: Some(story_id) };

        let options = Self::story_options(
            rise_wire_methods::story::REMOVE_STORY_FROM_HIGHLIGHT,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: false,
                send_immediately: true,
                store_response_in_cache: false,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let slice = Rc::clone(&self.remove_story_from_highlight_sai);
        let (success_services, success_mutation) = (services.clone(), mutation.clone());
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response = Self::payload(payload, &slice);
            let authoritative: HashMap<String, StoryCacheTransform> =
                if response.is_some_and(|response| response.highlight_deleted) {
                    HashMap::from([(
                        highlights_key.clone(),
                        StoryServicesStore::remove_highlight_transform(highlight_id),
                    )])
                } else {
                    HashMap::new()
                };
            success_services.update(cx, |services, cx| {
                services.commit_story_mutation(&success_mutation, authoritative, cx);
                services.refresh_open_player_highlight(highlight_id, cx);
            });
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("remove_story_from_highlight error: {error}"), cx);
            services.update(cx, |services, cx| {
                services.rollback_story_mutation(&mutation, cx);
                services.refresh_open_player_highlight(highlight_id, cx);
            });
        });

        SaiWs::request_slice(&self.remove_story_from_highlight_sai, body, options, Some(success), Some(failure), cx);
    }

    /// shelf cache a highlight mutation edits,
    /// the viewer own or the profile owner when the viewer acts as author
    fn highlight_cache_owner(owner_user_id: Option<&str>, viewer: &str) -> String {
        let trimmed = owner_user_id.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if trimmed.is_empty() { viewer.to_owned() } else { trimmed.to_owned() }
    }

    pub fn delete_highlight_action(
        &mut self,
        highlight_id: i64,
        owner_user_id: Option<&str>,
        scope: HighlightScope,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        let services = StoryServicesStore::shared(cx);
        let cache_owner = Self::highlight_cache_owner(owner_user_id, &user_id);
        let highlights_key = StoryServicesStore::highlights_cache_key(&cache_owner, scope);
        let mutation = services.update(cx, |services, cx| {
            services.note_highlights_target(&cache_owner, cx);
            let transforms: HashMap<String, StoryCacheTransform> =
                HashMap::from([(highlights_key, StoryServicesStore::remove_highlight_transform(highlight_id))]);
            services.begin_story_mutation(transforms, cx)
        });

        let body = rise_request::story::DeleteHighlight { highlight_id: Some(highlight_id) };

        let options = Self::story_options(
            rise_wire_methods::story::DELETE_HIGHLIGHT,
            SaiWsRequestOptionsInit {
                cache_key: None,
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: false,
                send_immediately: true,
                store_response_in_cache: false,
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let (success_services, success_mutation) = (services.clone(), mutation.clone());
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            success_services
                .update(cx, |services, cx| services.commit_story_mutation(&success_mutation, HashMap::new(), cx));
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("delete_highlight error: {error}"), cx);
            services.update(cx, |services, cx| services.rollback_story_mutation(&mutation, cx));
        });

        SaiWs::request_slice(&self.delete_highlight_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_highlight_stories_action(
        &mut self,
        highlight_id: i64,
        relative_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if Self::wire_user_id(cx).is_none() {
            return;
        }

        let cache_key = StoryServicesStore::highlight_stories_cache_key(highlight_id);
        let is_first_page = relative_id.is_none();
        let has_no_stories = StoryServicesStore::shared(cx)
            .update(cx, |services, cx| services.highlight_stories(highlight_id, cx).is_empty());

        let mut body = rise_request::story::GetHighlightStories {
            highlight_id: Some(highlight_id),
            limit: Some(50),
            ..Default::default()
        };
        if let Some(relative_id) = &relative_id {
            body.relative_id = Some(relative_id.clone());
        }

        let options = Self::story_options(
            rise_wire_methods::story::GET_HIGHLIGHT_STORIES,
            SaiWsRequestOptionsInit {
                cache_key: Some(cache_key),
                reload_if_cached: true,
                allow_concurrent_while_pending: false,
                reflect_pending_in_ui: is_first_page && has_no_stories,
                send_immediately: true,
                store_response_in_cache: true,
                persist_cache_to_disk: true,
                reload_when_only_disk_cached: true,
                preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
                path_to_array: Some(key::STORIES.to_owned()),
                path_to_id: Some(key::ID.to_owned()),
                pagination: Some(Self::relative_id_pagination(relative_id)),
                ..SaiWsRequestOptionsInit::default()
            },
        );

        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            StoryServicesStore::shared(cx).update(cx, |services, cx| {
                services.present_highlight_stories_if_ready(highlight_id, cx);
                services.refresh_open_player_highlight(highlight_id, cx);
            });
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("get_highlight_stories error: {error}"), cx);
            StoryServicesStore::shared(cx).update(cx, |services, cx| services.fail_highlight_open(highlight_id, cx));
        });

        SaiWs::request_slice(&self.get_highlight_stories_sai, body, options, Some(success), Some(failure), cx);
    }
}
