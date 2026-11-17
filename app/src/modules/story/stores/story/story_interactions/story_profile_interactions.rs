use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::{RiseGuideSignal, RiseTimingCanon};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_limits::limit::pinned_stories;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::story::components::stories_grid::stories_grid_shared_element_origin::StoriesGridSharedElementOrigin;
use crate::modules::story::shared::story_guide_sandbox_hooks::story_guide_sandbox_hooks;
use crate::modules::story::stores::story::story_actions::story_actions::{
    StoryActionDoneHandler, StoryActionErrorHandler, StoryActionTextHandler, StoryActionsStore,
};
use crate::modules::story::stores::story::story_actions::story_profile_actions::StoryActionProfileStoriesHandler;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::{
    GetProfileStoriesGatewayDTO, StoryProfileScope,
};
use crate::modules::story::stores::story::story_actions::story_types::StoryPlayerEntrance;
use crate::modules::story::stores::story::story_actions::story_wire_types::StoryDTO;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;

impl StoryInteractionsStore {
    pub fn pinned_stories_limit(cx: &mut App) -> i64 {
        let is_premium = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .is_some_and(|profile| profile.more.is_premium);
        i64::try_from(pinned_stories(is_premium)).unwrap_or(i64::MAX)
    }

    pub fn load_profile_stories(
        &mut self,
        target_user_id: Option<&str>,
        scope: StoryProfileScope,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(owner_id) = StoryActionsStore::resolved_profile_owner_id(target_user_id, cx) else {
            return;
        };
        let key = StoryServicesStore::profile_stories_cache_key(&owner_id, scope);
        let service = Self::service(cx);
        let ring = service.update(cx, |service, cx| service.has_active_stories(Some(owner_id.as_str()), cx));
        let ring_moved = scope == StoryProfileScope::Profile
            && self.profile_stories_ring_at_load.get(&key).is_some_and(|at_load| *at_load != ring);
        let empty_under_live_ring = scope == StoryProfileScope::Profile
            && ring
            && service.update(cx, |service, cx| service.profile_stories(&owner_id, scope, cx).is_empty());

        if !force && !ring_moved && !empty_under_live_ring {
            let loaded = service.update(cx, |service, cx| service.profile_stories_loaded(&owner_id, scope, cx));
            if loaded
                && let Some(loaded_at) = self.profile_stories_last_loaded_at.get(&key)
                && RiseClock::now().time_interval_since_1970() - loaded_at
                    < RiseTimingCanon::freshness_seconds("profile_stories")
            {
                return;
            }
        }

        self.profile_stories_last_loaded_at
            .insert(key.clone(), RiseClock::now().time_interval_since_1970());
        self.profile_stories_ring_at_load.insert(key, ring);
        cx.notify();
        let wire_target = Self::wire_target(&owner_id, cx);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_profile_stories_action(wire_target, scope, None, None, None, cx);
        });
    }

    pub fn load_more_profile_stories(
        &mut self,
        target_user_id: Option<&str>,
        scope: StoryProfileScope,
        cx: &mut Context<Self>,
    ) {
        let Some(owner_id) = StoryActionsStore::resolved_profile_owner_id(target_user_id, cx) else {
            return;
        };
        let key = StoryServicesStore::profile_stories_cache_key(&owner_id, scope);
        let service = Self::service(cx);
        if !service.update(cx, |service, cx| service.profile_stories_has_more(&owner_id, scope, cx)) {
            return;
        }
        let Some(cursor) =
            service.update(cx, |service, cx| service.profile_stories_next_cursor(&owner_id, scope, cx))
        else {
            return;
        };
        if self.profile_stories_pages_in_flight.contains(&key) {
            return;
        }
        if SaiWsPendingRegistry::shared(cx).read(cx).has_in_flight_request(Some(&key)) {
            tracing::warn!(target: "riseonly::X2013", key = %key, cursor, "profile stories page waits for the request in flight");
            return;
        }

        self.profile_stories_pages_in_flight.insert(key.clone());
        cx.notify();
        let success_this = cx.weak_entity();
        let error_this = cx.weak_entity();
        let success_key = key.clone();
        let on_success: StoryActionProfileStoriesHandler =
            Rc::new(move |_page: GetProfileStoriesGatewayDTO, cx: &mut App| {
                let this = success_this.clone();
                let key = success_key.clone();
                cx.defer(move |cx| {
                    this.update(cx, |this, cx| {
                        this.profile_stories_pages_in_flight.remove(&key);
                        cx.notify();
                    })
                    .ok();
                });
            });
        let on_error: StoryActionErrorHandler = Rc::new(move |_error: &anyhow::Error, cx: &mut App| {
            let this = error_this.clone();
            let key = key.clone();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| {
                    this.profile_stories_pages_in_flight.remove(&key);
                    cx.notify();
                })
                .ok();
            });
        });
        let wire_target = Self::wire_target(&owner_id, cx);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_profile_stories_action(wire_target, scope, Some(cursor), Some(on_success), Some(on_error), cx);
        });
    }

    pub fn load_highlight_picker_stories(&mut self, owner_id: &str, cx: &mut Context<Self>) {
        self.load_profile_stories(Some(owner_id), StoryProfileScope::Profile, false, cx);
        self.load_profile_stories(Some(owner_id), StoryProfileScope::Archive, false, cx);
    }

    pub fn load_more_highlight_picker_stories(&mut self, owner_id: &str, cx: &mut Context<Self>) {
        let Some(scope) = Self::service(cx).update(cx, |service, cx| service.highlight_picker_paging_scope(owner_id, cx))
        else {
            return;
        };
        self.load_more_profile_stories(Some(owner_id), scope, cx);
    }

    pub fn refresh_profile_stories(
        &mut self,
        target_user_id: Option<&str>,
        scope: StoryProfileScope,
        cx: &mut Context<Self>,
    ) {
        self.load_profile_stories(target_user_id, scope, true, cx);
    }

    pub fn archive_stories(
        &mut self,
        story_ids: &[i64],
        archived: bool,
        owner_user_id: Option<&str>,
        on_error: Option<StoryActionTextHandler>,
        cx: &mut Context<Self>,
    ) {
        let sandbox = story_guide_sandbox_hooks();
        if (sandbox.owns_stories)(story_ids, cx) {
            (sandbox.archive_stories)(story_ids, archived, cx);
            return;
        }
        let ids: Vec<i64> = story_ids.iter().copied().filter(|id| *id > 0).collect();
        if ids.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.set_stories_archived_action(&ids, archived, owner_user_id, None, on_error, cx);
        });
        if archived {
            GuideSignalBus::emit(RiseGuideSignal::StoriesArchived);
        }
    }

    pub fn pin_stories(
        &mut self,
        story_ids: &[i64],
        pinned: bool,
        owner_user_id: Option<&str>,
        on_limit_reached: Option<StoryActionDoneHandler>,
        on_error: Option<StoryActionTextHandler>,
        cx: &mut Context<Self>,
    ) {
        let ids: Vec<i64> = story_ids.iter().copied().filter(|id| *id > 0).collect();
        if ids.is_empty() {
            return;
        }

        if pinned && let Some(owner_id) = StoryActionsStore::resolved_profile_owner_id(owner_user_id, cx) {
            let already_pinned: HashSet<i64> = Self::service(cx)
                .update(cx, |service, cx| service.pinned_profile_story_ids(&owner_id, cx))
                .into_iter()
                .collect();
            let additions = ids.iter().filter(|id| !already_pinned.contains(*id)).count();
            let total = i64::try_from(already_pinned.len() + additions).unwrap_or(i64::MAX);
            if total > Self::pinned_stories_limit(cx) {
                if let Some(on_limit_reached) = on_limit_reached.as_ref() {
                    on_limit_reached(cx);
                }
                return;
            }
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.set_stories_pinned_action(&ids, pinned, owner_user_id, None, on_error, cx);
        });
    }

    pub fn delete_stories(
        &mut self,
        story_ids: &[i64],
        owner_user_id: Option<&str>,
        on_error: Option<StoryActionTextHandler>,
        cx: &mut Context<Self>,
    ) {
        let sandbox = story_guide_sandbox_hooks();
        if (sandbox.owns_stories)(story_ids, cx) {
            (sandbox.delete_stories)(story_ids, cx);
            return;
        }
        let ids: Vec<i64> = story_ids.iter().copied().filter(|id| *id > 0).collect();
        if ids.is_empty() {
            return;
        }
        Self::actions(cx)
            .update(cx, |actions, cx| actions.delete_stories_action(&ids, owner_user_id, None, on_error, cx));
    }

    pub fn open_profile_story(
        &mut self,
        story: &StoryDTO,
        owner_user_id: &str,
        scope: StoryProfileScope,
        shared_element_origin: Option<&Rc<StoriesGridSharedElementOrigin>>,
        cx: &mut Context<Self>,
    ) {
        if (story_guide_sandbox_hooks().owns_story)(story.id, cx) {
            return;
        }
        let service = Self::service(cx);
        let stories = service.update(cx, |service, cx| service.profile_stories(owner_user_id, scope, cx));
        if stories.is_empty() {
            return;
        }
        let request_id = Uuid::new_v4();
        let origin = if scope == StoryProfileScope::Profile { shared_element_origin } else { None };
        let entrance = match origin {
            Some(origin) => origin.arm_open(story, &stories, request_id, cx),
            None => StoryPlayerEntrance::Slide,
        };
        let player_origin = origin.map(|origin| origin.player_origin());
        let author = story.author.clone();
        let story_id = story.id;
        service.update(cx, |service, cx| {
            service.open_stories(
                stories,
                owner_user_id,
                author,
                story_id,
                request_id,
                entrance,
                player_origin,
                cx,
            );
        });
    }

    fn wire_target(owner_id: &str, cx: &mut App) -> Option<String> {
        let mine = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        if owner_id == mine { None } else { Some(owner_id.to_owned()) }
    }
}
