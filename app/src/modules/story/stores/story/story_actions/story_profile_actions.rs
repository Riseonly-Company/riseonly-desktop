use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_key::stories as key;
use rise_canon::{RiseTimingCanon, rise_request, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use serde_json::{Map, Value};

use crate::core::lib::canon_timing::{RiseTimingCanonExt, RiseTimingPageExt};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::story::stores::story::story_actions::story_actions::{
    StoryActionDoneHandler, StoryActionErrorHandler, StoryActionStoryHandler, StoryActionTextHandler,
    StoryActionsStore,
};
use crate::modules::story::stores::story::story_actions::story_failure_text::StoryFailureText;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::{
    GetProfileStoriesGatewayDTO, StoryProfileScope,
};
use crate::modules::story::stores::story::story_actions::story_types::StoryAudience;
use crate::modules::story::stores::story::story_services::story_services::{StoryCacheTransform, StoryServicesStore};

pub type StoryActionProfileStoriesHandler = Rc<dyn Fn(GetProfileStoriesGatewayDTO, &mut App)>;

impl StoryActionsStore {
    pub fn profile_stories_page_size() -> i64 {
        RiseTimingCanon::page("profile_stories_grid").page_size_items()
    }

    pub fn get_profile_stories_action(
        &mut self,
        target_user_id: Option<String>,
        scope: StoryProfileScope,
        relative_id: Option<i64>,
        on_success: Option<StoryActionProfileStoriesHandler>,
        on_error: Option<StoryActionErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        let is_first_page = relative_id.is_none();
        let cache_key = StoryServicesStore::profile_stories_cache_key(
            target_user_id.as_deref().unwrap_or(&user_id),
            scope,
        );

        let body = rise_request::story::GetProfileStories {
            limit: Some(Self::profile_stories_page_size()),
            relative_id,
            scope: Some(scope.raw_value().to_owned()),
            target_user_id,
        };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::story::SERVICE.to_owned(),
            method: rise_wire_methods::story::GET_PROFILE_STORIES.to_owned(),
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
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id: relative_id.map(|value| value.to_string()),
                has_more_path: Some(key::HAS_MORE.to_owned()),
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Rc::clone(&self.get_profile_stories_sai);
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(response) = Self::payload(payload, &slice)
                && let Some(on_success) = on_success.as_ref()
            {
                on_success(response, cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("get_profile_stories error: {error}"), cx);
            if let Some(on_error) = on_error.as_ref() {
                on_error(error, cx);
            }
        });

        SaiWs::request_slice(&self.get_profile_stories_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn resolved_profile_owner_id(owner_user_id: Option<&str>, cx: &mut App) -> Option<String> {
        let owner = owner_user_id.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if !owner.is_empty() {
            return Some(owner.to_owned());
        }
        AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .filter(|mine| !mine.is_empty())
    }

    fn profile_write_options(method: &str) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::story::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_stories_archived_action(
        &mut self,
        story_ids: &[i64],
        archived: bool,
        owner_user_id: Option<&str>,
        on_success: Option<StoryActionDoneHandler>,
        on_error: Option<StoryActionTextHandler>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::resolved_profile_owner_id(owner_user_id, cx) else {
            return;
        };
        let ids: Vec<i64> = story_ids.iter().copied().filter(|id| *id > 0).collect();
        if ids.is_empty() {
            return;
        }

        let services = StoryServicesStore::shared(cx);
        let mutation = services.update(cx, |services, cx| {
            let transforms = services.archive_stories_mutation_transforms(&ids, archived, &user_id, cx);
            services.begin_story_mutation(transforms, cx)
        });

        let body = rise_request::story::SetStoriesArchived { archived, story_ids: Some(ids) };
        let options = Self::profile_write_options(rise_wire_methods::story::SET_STORIES_ARCHIVED);

        let slice = Rc::clone(&self.set_stories_archived_sai);
        let (success_services, success_mutation) = (services.clone(), mutation.clone());
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let stories = Self::payload(payload, &slice).map(|response| response.stories).unwrap_or_default();
            success_services.update(cx, |services, cx| {
                let authoritative = services.stories_upsert_transforms(&stories, &user_id, cx);
                services.commit_story_mutation(&success_mutation, authoritative, cx);
            });
            if let Some(on_success) = on_success.as_ref() {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("set_stories_archived error: {error}"), cx);
            services.update(cx, |services, cx| services.rollback_story_mutation(&mutation, cx));
            if let Some(on_error) = on_error.as_ref() {
                on_error(StoryFailureText::message(error), cx);
            }
        });

        SaiWs::request_slice(&self.set_stories_archived_sai, body, options, Some(success), Some(failure), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_stories_pinned_action(
        &mut self,
        story_ids: &[i64],
        pinned: bool,
        owner_user_id: Option<&str>,
        on_success: Option<StoryActionDoneHandler>,
        on_error: Option<StoryActionTextHandler>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::resolved_profile_owner_id(owner_user_id, cx) else {
            return;
        };
        let ids: Vec<i64> = story_ids.iter().copied().filter(|id| *id > 0).collect();
        if ids.is_empty() {
            return;
        }

        let services = StoryServicesStore::shared(cx);
        let mutation = services.update(cx, |services, cx| {
            let transforms = services.pin_stories_mutation_transforms(&ids, pinned, &user_id, cx);
            services.begin_story_mutation(transforms, cx)
        });

        let body = rise_request::story::SetStoriesPinned { pinned, story_ids: Some(ids) };
        let options = Self::profile_write_options(rise_wire_methods::story::SET_STORIES_PINNED);

        let slice = Rc::clone(&self.set_stories_pinned_sai);
        let (success_services, success_mutation) = (services.clone(), mutation.clone());
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let stories = Self::payload(payload, &slice).map(|response| response.stories).unwrap_or_default();
            success_services.update(cx, |services, cx| {
                let authoritative = services.stories_upsert_transforms(&stories, &user_id, cx);
                services.commit_story_mutation(&success_mutation, authoritative, cx);
            });
            if let Some(on_success) = on_success.as_ref() {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("set_stories_pinned error: {error}"), cx);
            services.update(cx, |services, cx| services.rollback_story_mutation(&mutation, cx));
            if let Some(on_error) = on_error.as_ref() {
                on_error(StoryFailureText::message(error), cx);
            }
        });

        SaiWs::request_slice(&self.set_stories_pinned_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn delete_stories_action(
        &mut self,
        story_ids: &[i64],
        owner_user_id: Option<&str>,
        on_success: Option<StoryActionDoneHandler>,
        on_error: Option<StoryActionTextHandler>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::resolved_profile_owner_id(owner_user_id, cx) else {
            return;
        };
        let ids: Vec<i64> = story_ids.iter().copied().filter(|id| *id > 0).collect();
        if ids.is_empty() {
            return;
        }

        let services = StoryServicesStore::shared(cx);
        let mutation = services.update(cx, |services, cx| {
            let mut transforms: HashMap<String, StoryCacheTransform> = HashMap::new();
            for id in &ids {
                for (cache_key, transform) in services.delete_story_mutation_transforms(*id, &user_id, cx) {
                    let composed: StoryCacheTransform = match transforms.remove(&cache_key) {
                        Some(existing) => {
                            Rc::new(move |root: Option<Map<String, Value>>| transform(existing(root)))
                        }
                        None => transform,
                    };
                    transforms.insert(cache_key, composed);
                }
            }
            let mutation = services.begin_story_mutation(transforms, cx);
            services.reconcile_story_state_from_caches(&user_id, cx);
            mutation
        });

        let body = rise_request::story::DeleteStories { story_ids: Some(ids) };
        let options = Self::profile_write_options(rise_wire_methods::story::DELETE_STORIES);

        let (success_services, success_mutation, success_user_id) =
            (services.clone(), mutation.clone(), user_id.clone());
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            success_services.update(cx, |services, cx| {
                services.commit_story_mutation(&success_mutation, HashMap::new(), cx);
                services.reconcile_story_state_from_caches(&success_user_id, cx);
            });
            if let Some(on_success) = on_success.as_ref() {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("delete_stories error: {error}"), cx);
            services.update(cx, |services, cx| {
                services.rollback_story_mutation(&mutation, cx);
                services.reconcile_story_state_from_caches(&user_id, cx);
            });
            if let Some(on_error) = on_error.as_ref() {
                on_error(StoryFailureText::message(error), cx);
            }
        });

        SaiWs::request_slice(&self.delete_stories_sai, body, options, Some(success), Some(failure), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_story_action(
        &mut self,
        story_id: i64,
        caption: Option<String>,
        audience: Option<StoryAudience>,
        allow_screenshots: Option<bool>,
        post_to_profile: Option<bool>,
        excluded_user_ids: Option<Vec<String>>,
        selected_user_ids: Option<Vec<String>>,
        media_file_id: Option<String>,
        media_type: Option<String>,
        canvas: Option<rise_request::story::update_story::Canvas>,
        link_areas: Option<Vec<rise_request::story::update_story::LinkAreasElement>>,
        track_plate: Option<rise_request::story::update_story::TrackPlate>,
        on_success: Option<StoryActionStoryHandler>,
        on_error: Option<StoryActionTextHandler>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };
        if story_id <= 0 {
            return;
        }

        let mut body = rise_request::story::UpdateStory {
            allow_screenshots,
            audience: audience.map(|audience| audience.wire().to_owned()),
            canvas: None,
            caption,
            excluded_user_ids,
            file_id: media_file_id,
            link_areas: None,
            media_type,
            post_to_profile,
            selected_user_ids,
            story_id: Some(story_id),
            track_plate: None,
            update_audience: None,
        };
        if body.file_id.is_some() {
            body.canvas = canvas;
            body.link_areas = link_areas;
            body.track_plate = track_plate;
        }

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::story::SERVICE.to_owned(),
            method: rise_wire_methods::story::UPDATE_STORY.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: false,
            send_immediately: true,
            allows_deferred_send: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Rc::clone(&self.update_story_sai);
        let success_on_error = on_error.clone();
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(story) = Self::payload(payload, &slice).and_then(|response| response.story) else {
                if let Some(on_error) = success_on_error.as_ref() {
                    on_error(tr("story_edit_error"), cx);
                }
                return;
            };
            StoryServicesStore::shared(cx).update(cx, |services, cx| services.apply_updated_story(&story, &user_id, cx));
            if let Some(on_success) = on_success.as_ref() {
                on_success(story, cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("update_story error: {error}"), cx);
            if let Some(on_error) = on_error.as_ref() {
                on_error(StoryFailureText::message(error), cx);
            }
        });

        SaiWs::request_slice(&self.update_story_sai, body, options, Some(success), Some(failure), cx);
    }
}
