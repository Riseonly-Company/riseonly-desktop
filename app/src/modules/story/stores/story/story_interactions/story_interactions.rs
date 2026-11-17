use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_canon::RiseGuideSignal;
use rise_canon::rise_key::events;
use rise_core::foundation::foundation_date::TimeInterval;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_limits::service_limit::MAX_STATE_UNIQUE_USER_IDS;
use rise_ui::native_confirm_alert::{NativeConfirmAlert, NativeConfirmAlertAction};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::{
    SaiDebouncer, SaiDebouncerOptions, sai_debounced_action,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::SaiWsPaginationEdge;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::User;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerTrackSelection};
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::stores::post_actions::post_types::AuthorInfoMore;
use crate::modules::share_extension::shared::rise_share_models::RiseShareStoryPublishCheckpoint;
use crate::modules::story::shared::story_guide_sandbox_hooks::story_guide_sandbox_hooks;
use crate::modules::story::stores::story::story_actions::story_actions::{
    StoryActionDoneHandler, StoryActionHighlightHandler, StoryActionTextHandler, StoryActionsStore,
};
use crate::modules::story::stores::story::story_actions::story_types::{StoryAudience, StoryPlayerEntrance};
use crate::modules::story::stores::story::story_actions::story_wire_types::{
    HighlightDTO, HighlightDTOExt, HighlightScope, StoryAuthorDTO, StoryDTO, StoryGroupDTO,
};
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::story::stores::story::story_services::story_share_handoff_models::StorySharePublishContext;
use crate::modules::user::components::profile_top::profile_highlights_shared_element_origin::ProfileHighlightsSharedElementOrigin;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

const SHARE_STORY_HOST: &str = "share-story";
const SHARE_HOST: &str = "share";
const SHARE_STORY_PATH: &str = "/story";
const LEGACY_SESSION_QUERY_NAME: &str = "session";
const STORY_STATE_BATCH_KEY: &str = "story-state-batch";
const STORY_STATE_BATCH_GROUP: &str = "story";
const STORY_STATE_BATCH_DELAY_MILLISECONDS: i64 = 350;
const STORY_STATE_BATCH_MAX_CACHE_DATA: i64 = 5;
const MISSING_STORY_AUTHOR: &str = "Missing story author";

pub struct StoryInteractionsStore {
    pub profile_stories_last_loaded_at: HashMap<String, TimeInterval>,
    pub profile_stories_ring_at_load: HashMap<String, bool>,
    pub profile_stories_pages_in_flight: HashSet<String>,
    pending_state_user_ids: HashSet<String>,
}

impl StoryInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            profile_stories_last_loaded_at: HashMap::new(),
            profile_stories_ring_at_load: HashMap::new(),
            profile_stories_pages_in_flight: HashSet::new(),
            pending_state_user_ids: HashSet::new(),
        }
    }

    pub fn actions(cx: &mut App) -> Entity<StoryActionsStore> {
        StoryActionsStore::shared(cx)
    }

    pub fn service(cx: &mut App) -> Entity<StoryServicesStore> {
        StoryServicesStore::shared(cx)
    }

    fn effective_user_id_for_wire(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()
    }

    fn trimmed_non_empty(value: Option<&str>) -> Option<String> {
        value
            .map(trimming_whitespaces_and_newlines)
            .filter(|trimmed| !trimmed.is_empty())
            .map(str::to_owned)
    }

    pub fn is_shared_story_url(&self, url: &Url) -> bool {
        if url.scheme().map(str::to_lowercase) != Some(AppEnvironment::current().app_url_scheme()) {
            return false;
        }
        let host = url.host().map(|host| host.to_lowercase());
        let path = url.path();
        if host.as_deref() == Some(SHARE_STORY_HOST) {
            return path.is_empty() || path == "/";
        }
        host.as_deref() == Some(SHARE_HOST) && path.to_lowercase() == SHARE_STORY_PATH
    }

    pub fn handle_shared_story_url(&mut self, url: &Url, cx: &mut Context<Self>) -> bool {
        if !self.is_shared_story_url(url) {
            return false;
        }
        let session_id = UrlComponents::from_url(url, false)
            .and_then(|components| components.query_items())
            .and_then(|items| {
                items
                    .into_iter()
                    .find(|item| item.name == events::SESSION_ID || item.name == LEGACY_SESSION_QUERY_NAME)
            })
            .and_then(|item| item.value)
            .map(|value| trimming_whitespaces_and_newlines(&value).to_owned())
            .unwrap_or_default();
        Self::service(cx).update(cx, |service, cx| service.enqueue_shared_story_handoff(&session_id, cx));
        true
    }

    pub fn resume_pending_shared_story_handoff_if_possible(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.resume_pending_shared_story_handoff_if_possible(cx));
    }

    pub fn discover_pending_shared_story_handoffs(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.discover_pending_shared_story_handoffs(cx));
    }

    pub fn retry_shared_story_background_work(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.retry_shared_story_background_work(cx));
    }

    pub fn cancel_shared_story_handoff(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.cancel_shared_story_handoff(cx));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn publish_story(
        &mut self,
        item: MediaPickerItem,
        caption: String,
        audience: StoryAudience,
        allow_screenshots: bool,
        post_to_profile: bool,
        duration_days: i64,
        selected_user_ids: Vec<String>,
        attached_track: Option<MediaPickerTrackSelection>,
        idempotency_key: Option<String>,
        shared_handoff_context: Option<StorySharePublishContext>,
        on_success: Option<StoryActionDoneHandler>,
        on_error: Option<StoryActionTextHandler>,
        cx: &mut Context<Self>,
    ) {
        let on_success: StoryActionDoneHandler = on_success.unwrap_or_else(|| Rc::new(|_: &mut App| {}));
        let on_error: StoryActionTextHandler = on_error.unwrap_or_else(|| Rc::new(|_: String, _: &mut App| {}));
        Self::service(cx).update(cx, |service, cx| {
            service.publish_story(
                item,
                caption,
                audience,
                allow_screenshots,
                post_to_profile,
                duration_days,
                selected_user_ids,
                attached_track,
                idempotency_key,
                shared_handoff_context,
                on_success,
                on_error,
                cx,
            );
        });
    }

    pub fn resume_persisted_shared_story_publish(
        &mut self,
        context: StorySharePublishContext,
        checkpoint: RiseShareStoryPublishCheckpoint,
        on_success: StoryActionDoneHandler,
        on_error: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        Self::service(cx).update(cx, |service, cx| {
            service.resume_persisted_shared_story_publish(context, checkpoint, on_success, on_error, cx);
        });
    }

    pub fn dismiss_publish_error(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.dismiss_publish_error(cx));
    }

    pub fn present_publish_error(&mut self, cx: &mut Context<Self>) {
        let (publish_error, can_retry) = Self::service(cx).update(cx, |service, cx| {
            (service.publish_error.clone(), service.can_retry_shared_story_background_work(cx))
        });
        let Some(message) = publish_error else {
            return;
        };
        if message == tr("story_publish_wait_previous") {
            return;
        }
        let retry_this = cx.weak_entity();
        let dismiss_this = cx.weak_entity();
        let on_retry: Option<NativeConfirmAlertAction> = if can_retry {
            Some(Box::new(move |cx: &mut App| {
                retry_this
                    .update(cx, |this, cx| {
                        this.dismiss_publish_error(cx);
                        this.retry_shared_story_background_work(cx);
                    })
                    .ok();
            }))
        } else {
            None
        };
        NativeConfirmAlert::present_notice(
            &tr("story_create_error"),
            &message,
            if can_retry { Some(tr("story_share_retry")) } else { None },
            on_retry,
            move |cx: &mut App| {
                dismiss_this
                    .update(cx, |this, cx| {
                        if can_retry {
                            this.cancel_shared_story_handoff(cx);
                        }
                        this.dismiss_publish_error(cx);
                    })
                    .ok();
            },
            cx,
        );
    }

    pub fn cancel_story_publish(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.cancel_active_story_publish(cx));
    }

    pub fn load_user_stories(&mut self, target_user_id: Option<&str>, cx: &mut Context<Self>) {
        let resolved = target_user_id.map(str::to_owned).or_else(|| Self::effective_user_id_for_wire(cx));
        if let Some(resolved) = resolved.filter(|resolved| !resolved.is_empty()) {
            Self::service(cx).update(cx, |service, cx| service.note_user_stories_target(&resolved, cx));
        }
        let target = target_user_id.map(str::to_owned);
        Self::actions(cx).update(cx, |actions, cx| actions.get_user_stories_action(target, None, None, None, cx));
    }

    pub fn load_more_user_stories(&mut self, target_user_id: &str, cx: &mut Context<Self>) {
        let cache_key = StoryServicesStore::user_stories_cache_key(target_user_id);
        let can_request = SaiWs::pagination_state(&cache_key, cx)
            .read(cx)
            .state(SaiWsPaginationEdge::Trailing)
            .can_request_automatically();
        if !can_request {
            return;
        }
        let service = Self::service(cx);
        if !service.update(cx, |service, cx| service.user_stories_has_more(target_user_id, cx)) {
            return;
        }
        let Some(cursor) = service.update(cx, |service, cx| service.user_stories_next_cursor(target_user_id, cx)) else {
            return;
        };
        let target = Some(target_user_id.to_owned());
        Self::actions(cx)
            .update(cx, |actions, cx| actions.get_user_stories_action(target, Some(cursor), None, None, cx));
    }

    pub fn load_more_highlight_stories(&mut self, highlight_id: i64, cx: &mut Context<Self>) {
        let cache_key = StoryServicesStore::highlight_stories_cache_key(highlight_id);
        let can_request = SaiWs::pagination_state(&cache_key, cx)
            .read(cx)
            .state(SaiWsPaginationEdge::Trailing)
            .can_request_automatically();
        if !can_request {
            return;
        }
        let service = Self::service(cx);
        if !service.update(cx, |service, cx| service.highlight_stories_has_more(highlight_id, cx)) {
            return;
        }
        let Some(cursor) = service.update(cx, |service, cx| service.highlight_stories_next_cursor(highlight_id, cx))
        else {
            return;
        };
        Self::actions(cx)
            .update(cx, |actions, cx| actions.get_highlight_stories_action(highlight_id, Some(cursor), cx));
    }

    pub fn load_user_highlights(&mut self, target_user_id: Option<&str>, scope: HighlightScope, cx: &mut Context<Self>) {
        let resolved = target_user_id.map(str::to_owned).or_else(|| Self::effective_user_id_for_wire(cx));
        if let Some(resolved) = resolved.filter(|resolved| !resolved.is_empty()) {
            Self::service(cx).update(cx, |service, cx| service.note_highlights_target(&resolved, cx));
        }
        Self::actions(cx).update(cx, |actions, cx| actions.get_user_highlights_action(target_user_id, scope, cx));
    }

    pub fn set_highlight_archived(
        &mut self,
        highlight: &HighlightDTO,
        archived: bool,
        owner_user_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let sandbox = story_guide_sandbox_hooks();
        if (sandbox.owns_highlight)(highlight.id, cx) {
            if archived {
                (sandbox.delete_highlight)(highlight.id, cx);
            }
            return;
        }
        if highlight.id <= 0 || highlight.is_archived() == archived {
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).set_highlight_archived_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| {
            actions.set_highlight_archived_action(
                highlight,
                archived,
                owner_user_id,
                Rc::new(|_: String, _: &mut App| {}),
                cx,
            );
        });
    }

    pub fn open_highlight_link(
        &mut self,
        highlight_id: i64,
        on_owner_resolved: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        if highlight_id <= 0 {
            return;
        }
        Self::service(cx)
            .update(cx, |service, cx| service.open_highlight_link(highlight_id, on_owner_resolved, cx));
    }

    pub fn share_highlight(&mut self, highlight: &HighlightDTO, cx: &mut Context<Self>) {
        if highlight.id <= 0 {
            return;
        }
        let highlight = highlight.clone();
        ShareSheetStore::shared(cx).update(cx, |sheet, cx| sheet.present_highlight(highlight, cx));
    }

    pub fn load_stories_feed(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        if actions.read(cx).get_stories_feed_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.get_stories_feed_action(None, cx));
    }

    pub fn restore_stories_feed_from_cache(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.restore_stories_feed_from_cache(cx));
    }

    pub fn load_more_stories_feed(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        if actions.read(cx).get_stories_feed_sai.is_pending() {
            return;
        }
        let cache_key = StoryServicesStore::feed_cache_key();
        let can_request = SaiWs::pagination_state(&cache_key, cx)
            .read(cx)
            .state(SaiWsPaginationEdge::Trailing)
            .can_request_automatically();
        if !can_request {
            return;
        }
        let service = Self::service(cx);
        if !service.update(cx, |service, cx| service.feed_has_more(cx)) {
            return;
        }
        let Some(cursor) = service.update(cx, |service, cx| service.feed_next_cursor(cx)) else {
            return;
        };
        actions.update(cx, |actions, cx| actions.get_stories_feed_action(Some(cursor), cx));
    }

    pub fn create_highlight(
        &mut self,
        title: &str,
        story_ids: &[i64],
        on_success: Option<StoryActionDoneHandler>,
        on_error: Option<StoryActionTextHandler>,
        cx: &mut Context<Self>,
    ) {
        let trimmed = trimming_whitespaces_and_newlines(title).to_owned();
        if trimmed.is_empty() {
            if let Some(on_error) = on_error.as_ref() {
                on_error(tr("story_highlight_name_required"), cx);
            }
            return;
        }
        let sandbox = story_guide_sandbox_hooks();
        if (sandbox.owns_stories)(story_ids, cx) {
            (sandbox.create_highlight)(&trimmed, story_ids, cx);
            if let Some(on_success) = on_success.as_ref() {
                on_success(cx);
            }
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).create_highlight_sai.is_pending() {
            return;
        }
        let created: StoryActionHighlightHandler = Rc::new(move |_: HighlightDTO, cx: &mut App| {
            GuideSignalBus::emit(RiseGuideSignal::HighlightCreated);
            if let Some(on_success) = on_success.as_ref() {
                on_success(cx);
            }
        });
        let failed: StoryActionTextHandler =
            on_error.unwrap_or_else(|| Rc::new(|_: String, _: &mut App| {}));
        actions.update(cx, |actions, cx| {
            actions.create_highlight_action(&trimmed, story_ids.first().copied(), story_ids, created, failed, cx);
        });
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.pending_state_user_ids.clear();
        self.profile_stories_last_loaded_at.clear();
        self.profile_stories_ring_at_load.clear();
        SaiDebouncer::cancel(STORY_STATE_BATCH_KEY, STORY_STATE_BATCH_GROUP, cx);
        cx.notify();
    }

    pub fn open_stories_for_user_id(
        &mut self,
        user_id: &str,
        on_unavailable: Option<StoryActionDoneHandler>,
        cx: &mut Context<Self>,
    ) {
        Self::service(cx).update(cx, |service, cx| service.open_stories_for_user(user_id, on_unavailable, cx));
    }

    pub fn open_stories(&mut self, groups: Vec<StoryGroupDTO>, starting_at: usize, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.open_stories_feed(groups, starting_at, cx));
    }

    pub fn open_story(&mut self, story_id: i64, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.open_story(story_id, cx));
    }

    pub fn is_story_gone(&self, story_id: i64, cx: &mut App) -> bool {
        Self::service(cx).update(cx, |service, cx| service.is_story_gone(story_id, cx))
    }

    pub fn dismiss_player(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.dismiss_player(cx));
    }

    pub fn load_stories_state(&mut self, user_ids: &[String], cx: &mut Context<Self>) {
        let cleaned: Vec<String> = user_ids
            .iter()
            .map(|user_id| trimming_whitespaces_and_newlines(user_id))
            .filter(|user_id| !user_id.is_empty())
            .map(str::to_owned)
            .collect();
        if cleaned.is_empty() {
            return;
        }

        self.pending_state_user_ids.extend(cleaned);

        let this = cx.weak_entity();
        sai_debounced_action(
            STORY_STATE_BATCH_KEY,
            Box::new(move |cx: &mut App| {
                let Ok(batch) = this.update(cx, |this, _| this.pending_state_user_ids.drain().collect::<Vec<String>>()) else {
                    return;
                };
                if batch.is_empty() {
                    return;
                }
                let mut ordered = batch;
                ordered.sort();
                let chunk_size = usize::try_from(MAX_STATE_UNIQUE_USER_IDS).unwrap_or(usize::MAX);
                let actions = Self::actions(cx);
                for chunk in ordered.chunks(chunk_size) {
                    actions.update(cx, |actions, cx| actions.get_stories_state_action(chunk, cx));
                }
            }),
            SaiDebouncerOptions {
                delay_milliseconds: STORY_STATE_BATCH_DELAY_MILLISECONDS,
                group_key: STORY_STATE_BATCH_GROUP.to_owned(),
                max_cache_data: STORY_STATE_BATCH_MAX_CACHE_DATA,
            },
            cx,
        );
    }

    pub fn load_story_viewers(&mut self, story_id: i64, relative_id: Option<i64>, cx: &mut Context<Self>) {
        if story_id <= 0 {
            return;
        }
        if Self::service(cx).update(cx, |service, cx| service.is_viewers_in_flight(story_id, cx)) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.get_story_viewers_action(story_id, relative_id, cx));
    }

    pub fn mark_user_stories_seen(&mut self, user_id: &str, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.mark_user_stories_seen_locally(user_id, cx));
    }

    pub fn view_story(&mut self, story_id: i64, author_id: &str, stories: &[StoryDTO], cx: &mut Context<Self>) {
        if story_id <= 0 {
            return;
        }
        Self::service(cx)
            .update(cx, |service, cx| service.mark_story_viewed_locally(story_id, author_id, stories, cx));
        Self::actions(cx).update(cx, |actions, cx| actions.view_story_action(story_id, cx));
    }

    pub fn toggle_like(&mut self, story: &StoryDTO, cx: &mut Context<Self>) {
        if story.id <= 0 {
            return;
        }
        let liked = Self::service(cx).update(cx, |service, cx| service.is_story_liked(story, cx));
        let story_id = story.id;
        Self::actions(cx).update(cx, |actions, cx| {
            if liked {
                actions.unlike_story_action(story_id, cx);
            } else {
                actions.like_story_action(story_id, cx);
            }
        });
    }

    pub fn send_story_reply(
        &mut self,
        story: Option<&StoryDTO>,
        author_id: &str,
        author: Option<&StoryAuthorDTO>,
        text: &str,
        on_failure: Option<StoryActionDoneHandler>,
        on_refused: Option<StoryActionDoneHandler>,
        cx: &mut Context<Self>,
    ) {
        Self::service(cx).update(cx, |service, cx| {
            service.send_story_reply(story, author_id, author, text, on_failure, on_refused, cx)
        });
    }

    pub fn open_author_chat(
        &mut self,
        author_id: Option<&str>,
        author: Option<&StoryAuthorDTO>,
        on_open: StoryActionTextHandler,
        on_error: Option<StoryActionTextHandler>,
        cx: &mut Context<Self>,
    ) {
        let stable_id = author_id.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        let nested_id = author.map(|author| trimming_whitespaces_and_newlines(&author.id)).unwrap_or_default();
        let id = if stable_id.is_empty() { nested_id } else { stable_id };
        if id.is_empty() {
            if let Some(on_error) = on_error.as_ref() {
                on_error(MISSING_STORY_AUTHOR.to_owned(), cx);
            }
            return;
        }

        let target = User {
            id: id.to_owned(),
            name: author.map(|author| author.name.clone()).unwrap_or_default(),
            tag: author.map(|author| author.tag.clone()).unwrap_or_default(),
            more: AuthorInfoMore {
                logo: author.map(|author| author.logo.clone()).unwrap_or_default(),
                is_premium: author.is_some_and(|author| author.is_premium),
                is_official: false,
                premium_badge_emoji: author.and_then(|author| author.premium_badge_emoji.clone()),
                name_color: author.and_then(|author| author.name_color.clone()),
                ..AuthorInfoMore::default()
            },
            ..User::default()
        };
        let chat_id =
            ChatInteractionsStore::shared(cx).update(cx, |chats, cx| chats.direct_conversation_id(&target, cx));
        if chat_id.is_empty() {
            if let Some(on_error) = on_error.as_ref() {
                on_error(MISSING_STORY_AUTHOR.to_owned(), cx);
            }
            return;
        }
        on_open(chat_id, cx);
    }

    pub fn open_highlight(
        &mut self,
        highlight_id: i64,
        owner_user_id: &str,
        shared_element_origin: Option<&Rc<ProfileHighlightsSharedElementOrigin>>,
        highlight: Option<&HighlightDTO>,
        cx: &mut Context<Self>,
    ) {
        if (story_guide_sandbox_hooks().owns_highlight)(highlight_id, cx) {
            return;
        }
        let request_id = Uuid::new_v4();
        let mut entrance = StoryPlayerEntrance::Slide;
        let service = Self::service(cx);
        if let Some(origin) = shared_element_origin
            && let Some(highlight) = highlight
            && highlight.id == highlight_id
        {
            let player_idle = service.update(cx, |service, cx| {
                !service.is_loading_player
                    && service.player_request.is_none()
                    && !service.highlight_stories(highlight_id, cx).is_empty()
            });
            if player_idle {
                entrance = origin.arm_open(highlight, request_id, cx);
            }
        }
        let player_origin = shared_element_origin.map(|origin| origin.player_origin(highlight_id));
        service.update(cx, |service, cx| {
            service.open_highlight(highlight_id, owner_user_id, request_id, entrance, player_origin, cx);
        });
    }

    pub fn delete_story(&mut self, story_id: i64, author_id: Option<&str>, cx: &mut Context<Self>) {
        if story_id <= 0 {
            self.cancel_story_publish(cx);
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.delete_story_action(story_id, author_id, cx));
    }

    pub fn delete_highlight(
        &mut self,
        highlight_id: i64,
        owner_user_id: Option<&str>,
        scope: HighlightScope,
        cx: &mut Context<Self>,
    ) {
        let sandbox = story_guide_sandbox_hooks();
        if (sandbox.owns_highlight)(highlight_id, cx) {
            (sandbox.delete_highlight)(highlight_id, cx);
            return;
        }
        if highlight_id <= 0 {
            return;
        }
        Self::actions(cx)
            .update(cx, |actions, cx| actions.delete_highlight_action(highlight_id, owner_user_id, scope, cx));
    }

    pub fn update_highlight(
        &mut self,
        highlight_id: i64,
        title: Option<&str>,
        cover_story_id: Option<i64>,
        owner_user_id: Option<&str>,
        on_error: Option<StoryActionTextHandler>,
        cx: &mut Context<Self>,
    ) {
        let trimmed = title.map(|title| trimming_whitespaces_and_newlines(title).to_owned());
        if trimmed.as_deref().is_some_and(str::is_empty) {
            if let Some(on_error) = on_error.as_ref() {
                on_error(tr("story_highlight_name_required"), cx);
            }
            return;
        }
        if trimmed.is_none() && cover_story_id.is_none() {
            return;
        }
        let sandbox = story_guide_sandbox_hooks();
        if (sandbox.owns_highlight)(highlight_id, cx) {
            (sandbox.update_highlight)(highlight_id, trimmed.as_deref(), cover_story_id, cx);
            return;
        }
        if highlight_id <= 0 {
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).update_highlight_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| {
            actions.update_highlight_action(highlight_id, trimmed, cover_story_id, owner_user_id, cx);
        });
        GuideSignalBus::emit(RiseGuideSignal::HighlightSaved);
    }

    pub fn add_stories_to_highlight(
        &mut self,
        highlight_id: i64,
        story_ids: &[i64],
        owner_user_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let sandbox = story_guide_sandbox_hooks();
        if (sandbox.owns_highlight)(highlight_id, cx) {
            (sandbox.add_stories)(story_ids, highlight_id, cx);
            return;
        }
        if highlight_id <= 0 || story_ids.is_empty() {
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).add_stories_to_highlight_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| {
            actions.add_stories_to_highlight_action(highlight_id, story_ids, owner_user_id, cx);
        });
    }

    pub fn remove_story_from_highlight(
        &mut self,
        highlight_id: i64,
        story_id: i64,
        owner_user_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let sandbox = story_guide_sandbox_hooks();
        if (sandbox.owns_highlight)(highlight_id, cx) {
            (sandbox.remove_story)(story_id, highlight_id, cx);
            return;
        }
        if highlight_id <= 0 || story_id <= 0 {
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).remove_story_from_highlight_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| {
            actions.remove_story_from_highlight_action(highlight_id, story_id, owner_user_id, cx);
        });
    }

    pub fn load_highlight_stories(&mut self, highlight_id: i64, cx: &mut Context<Self>) {
        if highlight_id <= 0 {
            return;
        }
        if !Self::service(cx).update(cx, |service, cx| service.highlight_stories(highlight_id, cx).is_empty()) {
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).get_highlight_stories_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.get_highlight_stories_action(highlight_id, None, cx));
    }

    pub fn report_author(&mut self, user_id: Option<&str>, cx: &mut Context<Self>) {
        let Some(id) = Self::trimmed_non_empty(user_id) else {
            return;
        };
        Self::service(cx).update(cx, |service, cx| service.dismiss_player(cx));
        cx.defer(move |cx| {
            ReportSheetStore::shared(cx).update(cx, |sheet, cx| sheet.present(ReportTarget::User(id), None, None, cx));
        });
    }

    pub fn report_story(&mut self, story_id: Option<i64>, author_id: Option<&str>, cx: &mut Context<Self>) {
        let Some(story_id) = story_id.filter(|story_id| *story_id > 0) else {
            self.report_author(author_id, cx);
            return;
        };
        Self::service(cx).update(cx, |service, cx| service.dismiss_player(cx));
        cx.defer(move |cx| {
            ReportSheetStore::shared(cx)
                .update(cx, |sheet, cx| sheet.present(ReportTarget::Story(story_id), None, None, cx));
        });
    }

    pub fn mute_author(&mut self, user_id: Option<&str>, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.mute_user_stories(user_id, cx));
    }

    pub fn unmute_author(&mut self, user_id: Option<&str>, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.unmute_user_stories(user_id, cx));
    }

    pub fn is_author_muted(&self, user_id: Option<&str>, cx: &mut App) -> bool {
        Self::service(cx).update(cx, |service, cx| service.is_stories_muted(user_id, cx))
    }

    pub fn toggle_author_relationship(&mut self, user_id: Option<&str>, tag: Option<&str>, cx: &mut Context<Self>) {
        let Some(id) = Self::trimmed_non_empty(user_id) else {
            return;
        };
        let tag = tag.unwrap_or_default().to_owned();
        UserInteractionsStore::shared(cx).update(cx, |user, cx| user.toggle_subscribe(&id, &tag, cx));
    }
}
