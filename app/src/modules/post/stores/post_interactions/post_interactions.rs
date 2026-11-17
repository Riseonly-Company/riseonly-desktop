use std::collections::{HashMap, HashSet};
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_canon::{RiseGuideSignal, rise_wire_methods};
use rise_catalog::PostShareChannel;
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::RiseAppEvents;
use rise_engine::CancellationError;
use rise_errors::{ErrorCategory, ErrorCode, ErrorParam};
use rise_i18n::{FormatArg, tr, tr_args};
use rise_ui::native_confirm_alert::NativeConfirmAlert;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_errors::{CanonErrors, RiseFailure};
use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::rise_engine_bridge::rise_canon_endpoints::{RiseCanonEndpoints, RiseEndpointClientTimeout};
use crate::core::rise_engine_bridge::rise_engine_app_runtime::{RISE_ENGINE_CONNECTION_READY, RiseEngineAppRuntimeError};
use crate::core::widgets::notifier::notifier_upload_progress::NotifierUploadProgress;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::analytics::stores::engagement::engagement_actions::engagement_types::EngagementSurface;
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind, MediaPickerTrackPlate};
use crate::modules::media_picker::api::short_video_limits::ShortVideoLimits;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::components::repost::repost_opinion_editor::RepostOpinionEditorPresenter;
use crate::modules::post::components::repost::repost_overlay::RepostStickerStyle;
use crate::modules::post::engine::rise_post_content_repository::RisePostContentRepositoryLoadIntent;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostContentKind, RisePostContentKindResolve, RisePostFeedKind, RisePostPagePresentation, RisePostPageScope,
    RisePostPageStatus,
    RisePostPlacement, RisePostProfileKind, RisePostSearchKind, RisePostViewSurface, RisePostVisibilityIntent,
};
use crate::modules::post::engine::rise_post_repository::RisePostRepositoryError;
use crate::modules::post::shared::post_field_rules::PostFieldRules;
use crate::modules::post::shared::post_media_slides::PostMediaSlides;
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_actions::post_types::{
    GetPostFeedResponse, PostCollaboratorRejection, PostCollaboratorUser, PostCountVisibility, PostEditMediaItem,
    PostTagInputDraft, RiseStoredPostStats,
};
use crate::modules::post::stores::post_actions::rise_post_publish_failure::RisePostPublishFailure;
use crate::modules::post::stores::post_interactions::post_interactions_hooks::post_interactions_hooks;
use crate::modules::post::stores::post_services::post_guide_sandbox_hooks::post_guide_sandbox_hooks;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::post::stores::post_services::post_watch_services::PostWatchServicesStore;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

type PostInteractionsErrorHandler = Box<dyn FnOnce(String, &mut App)>;
type PostInteractionsDoneHandler = Box<dyn FnOnce(&mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostInteractionsStoreShareReceiptChannel {
    External,
    LinkCopy,
}

pub type ShareReceiptChannel = PostInteractionsStoreShareReceiptChannel;

impl PostInteractionsStoreShareReceiptChannel {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::External => PostShareChannel::ExternalApp.wire(),
            Self::LinkCopy => PostShareChannel::LinkCopy.wire(),
        }
    }
}

#[derive(Clone)]
struct PostInteractionsPublishRequest {
    content_kind: RisePostContentKind,
    title: String,
    content: String,
    manual_hashtags: String,
    can_comment: bool,
    photos: Vec<SaiWsOptimisticLocalFile>,
    video: Option<MediaPickerItem>,
    video_position: Option<i64>,
    placement: RisePostPlacement,
    achievement_id: Option<i64>,
    map_journey_id: Option<String>,
    map_journey_show_heat: bool,
    track_id: Option<String>,
    attached_track: Option<RiseStoredMusicTrack>,
    track_start_ms: Option<i64>,
    track_end_ms: Option<i64>,
    track_plate: Option<MediaPickerTrackPlate>,
    count_visibility: PostCountVisibility,
    coauthors: Vec<PostCollaboratorUser>,
    tagged_users: Vec<PostTagInputDraft>,
    started_at: Date,
    retry_user_id: Option<String>,
    on_success: Option<Rc<dyn Fn(&mut App)>>,
    on_error: Option<Rc<dyn Fn(String, &mut App)>>,
}

pub struct PostInteractionsStore {
    profile_tags_by_user_id: HashMap<String, String>,
    like_intents: HashMap<String, bool>,
    favorite_intents: HashMap<String, bool>,
    content_owner_clock: u64,
    pending_content_observations: HashMap<Uuid, u64>,
    post_view_history_load_more_deferred: bool,
}

impl PostInteractionsStore {
    const SLOW_PUBLISH_NOTICE_SECONDS: f64 = 3.0;
    const INTENT_MAP_LIMIT: usize = 256;
    const CONTENT_READY_POLL: Duration = Duration::from_millis(500);
    const POST_STATS_DAYS: i64 = 30;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            profile_tags_by_user_id: HashMap::new(),
            like_intents: HashMap::new(),
            favorite_intents: HashMap::new(),
            content_owner_clock: 0,
            pending_content_observations: HashMap::new(),
            post_view_history_load_more_deferred: false,
        }
    }

    fn actions(cx: &mut App) -> Entity<PostActionsStore> {
        PostActionsStore::shared(cx)
    }

    fn service(cx: &mut App) -> Entity<PostServicesStore> {
        PostServicesStore::shared(cx)
    }

    fn effective_user_id_for_wire(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()
    }

    fn has_wire_user(cx: &mut App) -> bool {
        Self::effective_user_id_for_wire(cx).is_some_and(|user_id| !user_id.is_empty())
    }

    fn is_positive_id(id: &str) -> bool {
        int_from_string(id).unwrap_or(0) > 0
    }

    fn is_open_page(page: &RisePostPagePresentation) -> bool {
        page.status == RisePostPageStatus::Idle
            || page.status == RisePostPageStatus::Failed
            || (page.status == RisePostPageStatus::Loading && page.items.is_empty())
    }

    pub fn load_initial_feed(&mut self, cx: &mut Context<Self>) {
        self.activate_feed(RisePostFeedKind::All, cx);
    }

    pub fn refresh_feed(&mut self, cx: &mut Context<Self>) {
        self.refresh_feed_kind(RisePostFeedKind::All, cx).detach();
    }

    pub fn load_more_feed(&mut self, cx: &mut Context<Self>) {
        self.load_more_feed_kind(RisePostFeedKind::All, cx);
    }

    pub fn activate_feed(&mut self, kind: RisePostFeedKind, cx: &mut Context<Self>) {
        let page = Self::service(cx).update(cx, |service, cx| service.feed_page(kind, cx));
        if !Self::is_open_page(&page) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.activate_feed_action(kind, cx)).detach();
    }

    pub fn refresh_feed_kind(&mut self, kind: RisePostFeedKind, cx: &mut Context<Self>) -> Task<()> {
        Self::actions(cx).update(cx, |actions, cx| actions.refresh_feed_action(kind, cx))
    }

    pub fn load_more_feed_kind(&mut self, kind: RisePostFeedKind, cx: &mut Context<Self>) {
        let page = Self::service(cx).update(cx, |service, cx| service.feed_page(kind, cx));
        if !page.has_more || page.is_loading_more || page.is_refreshing {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.load_more_feed_action(kind, cx)).detach();
    }

    pub fn observe_feed(&mut self, kind: RisePostFeedKind, owner_id: Uuid, cx: &mut Context<Self>) {
        let generation = self.next_content_owner_generation(cx);
        self.observe_content(RisePostPageScope::Feed(kind), owner_id, generation, cx).detach();
    }

    pub fn stop_observing_feed(&mut self, kind: RisePostFeedKind, owner_id: Uuid, cx: &mut Context<Self>) {
        let generation = self.next_content_owner_generation(cx);
        self.stop_observing_content(RisePostPageScope::Feed(kind), owner_id, generation, cx).detach();
    }

    pub fn observe_shorts(&mut self, owner_id: Uuid, cx: &mut Context<Self>) {
        let generation = self.next_content_owner_generation(cx);
        self.observe_content(RisePostPageScope::Shorts, owner_id, generation, cx).detach();
    }

    pub fn watch_posts(&mut self, post_ids: Vec<String>, owner: &str, cx: &mut Context<Self>) {
        PostWatchServicesStore::shared(cx).update(cx, |watch, cx| watch.set_desired(post_ids, owner, cx));
    }

    pub fn stop_watching_posts(&mut self, owner: &str, cx: &mut Context<Self>) {
        PostWatchServicesStore::shared(cx).update(cx, |watch, cx| watch.set_desired(Vec::<String>::new(), owner, cx));
    }

    pub fn stop_observing_shorts(&mut self, owner_id: Uuid, cx: &mut Context<Self>) {
        let generation = self.next_content_owner_generation(cx);
        self.stop_observing_content(RisePostPageScope::Shorts, owner_id, generation, cx).detach();
    }

    pub fn load_more_shorts(&mut self, cx: &mut Context<Self>) {
        let page = Self::service(cx).update(cx, |service, cx| service.page(&RisePostPageScope::Shorts, cx));
        if page.status != RisePostPageStatus::Loaded
            || !page.has_more
            || page.error_message.is_some()
            || page.is_loading_more
            || page.is_refreshing
        {
            return;
        }
        Self::actions(cx)
            .update(cx, |actions, cx| actions.load_shorts_action(RisePostContentRepositoryLoadIntent::LoadMore, cx))
            .detach();
    }

    pub fn retry_shorts(&mut self, cx: &mut Context<Self>) {
        let page = Self::service(cx).update(cx, |service, cx| service.page(&RisePostPageScope::Shorts, cx));
        if page.status == RisePostPageStatus::Loading || page.is_loading_more || page.is_refreshing {
            return;
        }
        let intent = if page.items.is_empty() || !page.has_more {
            RisePostContentRepositoryLoadIntent::Refresh
        } else {
            RisePostContentRepositoryLoadIntent::LoadMore
        };
        Self::actions(cx).update(cx, |actions, cx| actions.load_shorts_action(intent, cx)).detach();
    }

    pub fn toggle_like(&mut self, post: &GetPostFeedResponse, cx: &mut Context<Self>) {
        let post_id = post.id.clone();
        let desired = !self.like_intents.get(&post_id).copied().unwrap_or(post.is_liked);
        self.like_intents.insert(post_id.clone(), desired);
        self.trim_intent_maps();
        cx.notify();
        Self::service(cx).update(cx, |service, cx| service.apply_like_state(&post_id, desired, cx));
        let this = cx.weak_entity();
        let applied_post_id = post_id.clone();
        let refused_post_id = post_id.clone();
        let seed = post.clone();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.set_post_like_action(
                &post_id,
                desired,
                Some(seed),
                Some(Box::new(move |applied, cx: &mut App| {
                    if !applied {
                        PostServicesStore::shared(cx)
                            .update(cx, |service, cx| service.apply_like_state(&applied_post_id, !desired, cx));
                    }
                    this.update(cx, |this, cx| {
                        if this.like_intents.get(&applied_post_id) != Some(&desired) {
                            return;
                        }
                        this.like_intents.remove(&applied_post_id);
                        cx.notify();
                    })
                    .ok();
                })),
                Some(Arc::new(move |failure: RiseFailure, restored: bool, cx: &mut App| {
                    PostServicesStore::shared(cx)
                        .update(cx, |service, cx| service.apply_like_state(&refused_post_id, restored, cx));
                    Self::announce_refusal(&failure, cx);
                })),
                cx,
            );
        });
    }

    pub fn toggle_favorite(&mut self, post: &GetPostFeedResponse, cx: &mut Context<Self>) {
        let post_id = post.id.clone();
        let desired = !self.favorite_intents.get(&post_id).copied().unwrap_or(post.is_favorited);
        self.favorite_intents.insert(post_id.clone(), desired);
        self.trim_intent_maps();
        cx.notify();
        Self::service(cx).update(cx, |service, cx| service.apply_favorite_state(&post_id, desired, cx));
        let this = cx.weak_entity();
        let applied_post_id = post_id.clone();
        let refused_post_id = post_id.clone();
        let seed = post.clone();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.set_post_favorite_action(
                &post_id,
                desired,
                Some(seed),
                Some(Box::new(move |applied, cx: &mut App| {
                    if !applied {
                        PostServicesStore::shared(cx)
                            .update(cx, |service, cx| service.apply_favorite_state(&applied_post_id, !desired, cx));
                    }
                    this.update(cx, |this, cx| {
                        if this.favorite_intents.get(&applied_post_id) != Some(&desired) {
                            return;
                        }
                        this.favorite_intents.remove(&applied_post_id);
                        cx.notify();
                    })
                    .ok();
                })),
                Some(Arc::new(move |failure: RiseFailure, restored: bool, cx: &mut App| {
                    PostServicesStore::shared(cx)
                        .update(cx, |service, cx| service.apply_favorite_state(&refused_post_id, restored, cx));
                    Self::announce_refusal(&failure, cx);
                })),
                cx,
            );
        });
    }

    fn announce_refusal(failure: &RiseFailure, cx: &mut App) {
        let text = if CanonErrors::category(failure.code.as_deref()) == ErrorCategory::Maintenance {
            Some(tr("maintenance_action_blocked"))
        } else {
            CanonErrors::text_for(failure)
        };
        Notifier::send(
            text.unwrap_or_else(|| tr("retry_later_error")),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..Default::default()
            },
            cx,
        );
    }

    pub fn load_user_posts_if_needed(&mut self, tag: &str, profile_user_id: &str, cx: &mut Context<Self>) {
        self.activate_profile(profile_user_id, tag, RisePostProfileKind::Publications, cx);
    }

    pub fn refresh_user_posts(&mut self, tag: &str, profile_user_id: &str, cx: &mut Context<Self>) {
        self.refresh_profile(profile_user_id, tag, RisePostProfileKind::Publications, cx).detach();
    }

    pub fn load_more_user_posts(&mut self, tag: &str, profile_user_id: &str, cx: &mut Context<Self>) {
        self.load_more_profile(profile_user_id, tag, RisePostProfileKind::Publications, cx);
    }

    pub fn user_posts_items(&self, tag: &str, profile_user_id: &str, cx: &mut App) -> Vec<GetPostFeedResponse> {
        Self::service(cx).update(cx, |service, cx| service.user_posts(tag, Some(profile_user_id), cx).items)
    }

    pub fn user_posts_status(&self, tag: &str, profile_user_id: &str, cx: &mut App) -> SaiWsSliceStatus {
        Self::service(cx).update(cx, |service, cx| service.user_posts_status(tag, Some(profile_user_id), cx))
    }

    pub fn user_posts_error_text(&self, tag: &str, profile_user_id: &str, cx: &mut App) -> Option<String> {
        let state = Self::service(cx).update(cx, |service, cx| service.user_posts(tag, Some(profile_user_id), cx));
        state.last_error_text.or_else(|| Self::actions(cx).read(cx).get_user_posts_sai.error())
    }

    pub fn load_user_reposts_if_needed(&mut self, tag: &str, profile_user_id: &str, cx: &mut Context<Self>) {
        self.activate_profile(profile_user_id, tag, RisePostProfileKind::Reposts, cx);
    }

    pub fn refresh_user_reposts(&mut self, tag: &str, profile_user_id: &str, cx: &mut Context<Self>) {
        self.refresh_profile(profile_user_id, tag, RisePostProfileKind::Reposts, cx).detach();
    }

    pub fn load_more_user_reposts(&mut self, tag: &str, profile_user_id: &str, cx: &mut Context<Self>) {
        self.load_more_profile(profile_user_id, tag, RisePostProfileKind::Reposts, cx);
    }

    pub fn activate_profile(&mut self, user_id: &str, tag: &str, kind: RisePostProfileKind, cx: &mut Context<Self>) {
        if user_id.is_empty() || tag.is_empty() {
            return;
        }
        self.profile_tags_by_user_id.insert(user_id.to_owned(), tag.to_owned());
        cx.notify();
        let page = Self::service(cx).update(cx, |service, cx| service.profile_page_with_tag(user_id, tag, kind, cx));
        if !Self::is_open_page(&page) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.activate_profile_action(user_id, tag, kind, cx)).detach();
    }

    pub fn activate_profile_without_tag(&mut self, user_id: &str, kind: RisePostProfileKind, cx: &mut Context<Self>) {
        let Some(tag) = self.profile_tags_by_user_id.get(user_id).cloned() else {
            return;
        };
        self.activate_profile(user_id, &tag, kind, cx);
    }

    pub fn refresh_profile(
        &mut self,
        user_id: &str,
        tag: &str,
        kind: RisePostProfileKind,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        if user_id.is_empty() || tag.is_empty() {
            return Task::ready(());
        }
        self.profile_tags_by_user_id.insert(user_id.to_owned(), tag.to_owned());
        cx.notify();
        Self::actions(cx).update(cx, |actions, cx| actions.refresh_profile_action(user_id, tag, kind, cx))
    }

    pub fn refresh_profile_without_tag(
        &mut self,
        user_id: &str,
        kind: RisePostProfileKind,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let Some(tag) = self.profile_tags_by_user_id.get(user_id).cloned() else {
            return Task::ready(());
        };
        self.refresh_profile(user_id, &tag, kind, cx)
    }

    pub fn load_more_profile(&mut self, user_id: &str, tag: &str, kind: RisePostProfileKind, cx: &mut Context<Self>) {
        if user_id.is_empty() || tag.is_empty() {
            return;
        }
        if self.profile_tags_by_user_id.insert(user_id.to_owned(), tag.to_owned()).as_deref() != Some(tag) {
            cx.notify();
        }
        let page = Self::service(cx).update(cx, |service, cx| service.profile_page_with_tag(user_id, tag, kind, cx));
        if !page.has_more || page.is_loading_more || page.is_refreshing {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.load_more_profile_action(user_id, tag, kind, cx)).detach();
    }

    pub fn load_more_profile_without_tag(&mut self, user_id: &str, kind: RisePostProfileKind, cx: &mut Context<Self>) {
        let Some(tag) = self.profile_tags_by_user_id.get(user_id).cloned() else {
            return;
        };
        self.load_more_profile(user_id, &tag, kind, cx);
    }

    pub fn observe_profile(
        &mut self,
        user_id: &str,
        tag: &str,
        kind: RisePostProfileKind,
        owner_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        let generation = self.next_content_owner_generation(cx);
        let scope = RisePostPageScope::Profile { user_id: user_id.to_owned(), tag: tag.to_owned(), kind };
        self.observe_content(scope, owner_id, generation, cx).detach();
    }

    pub fn stop_observing_profile(
        &mut self,
        user_id: &str,
        tag: &str,
        kind: RisePostProfileKind,
        owner_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        let generation = self.next_content_owner_generation(cx);
        let scope = RisePostPageScope::Profile { user_id: user_id.to_owned(), tag: tag.to_owned(), kind };
        self.stop_observing_content(scope, owner_id, generation, cx).detach();
    }

    pub fn activate_post_search(&mut self, query: &str, kind: RisePostSearchKind, cx: &mut Context<Self>) {
        let page = Self::service(cx).update(cx, |service, cx| service.search_post_page(query, kind, cx));
        if !Self::is_open_page(&page) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.activate_post_search_action(query, kind, cx)).detach();
    }

    pub fn refresh_post_search(&mut self, query: &str, kind: RisePostSearchKind, cx: &mut Context<Self>) -> Task<()> {
        Self::actions(cx).update(cx, |actions, cx| actions.refresh_post_search_action(query, kind, cx))
    }

    pub fn load_more_post_search(&mut self, query: &str, kind: RisePostSearchKind, cx: &mut Context<Self>) {
        let page = Self::service(cx).update(cx, |service, cx| service.search_post_page(query, kind, cx));
        if !page.has_more || page.is_loading_more || page.is_refreshing {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.load_more_post_search_action(query, kind, cx)).detach();
    }

    pub fn observe_post_search(
        &mut self,
        query: &str,
        kind: RisePostSearchKind,
        owner_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        let generation = self.next_content_owner_generation(cx);
        let scope = RisePostPageScope::Search { query: query.to_owned(), kind };
        self.observe_content(scope, owner_id, generation, cx).detach();
    }

    pub fn stop_observing_post_search(
        &mut self,
        query: &str,
        kind: RisePostSearchKind,
        owner_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        let generation = self.next_content_owner_generation(cx);
        let scope = RisePostPageScope::Search { query: query.to_owned(), kind };
        self.stop_observing_content(scope, owner_id, generation, cx).detach();
    }

    pub fn user_reposts_items(&self, tag: &str, cx: &mut App) -> Vec<GetPostFeedResponse> {
        Self::service(cx).update(cx, |service, cx| service.user_reposts(tag, cx).items)
    }

    pub fn user_reposts_status(&self, tag: &str, cx: &mut App) -> SaiWsSliceStatus {
        Self::service(cx).update(cx, |service, cx| service.user_reposts_status(tag, cx))
    }

    pub fn remove_repost(&mut self, post: &GetPostFeedResponse, owner_id: Option<String>, cx: &mut Context<Self>) {
        let post = post.clone();
        NativeConfirmAlert::present_delete(
            &tr("repost_delete_confirm_title"),
            &tr("repost_delete_confirm_message"),
            None,
            None,
            false,
            None,
            None,
            move |cx: &mut App| {
                PostActionsStore::shared(cx).update(cx, |actions, cx| {
                    actions.delete_repost_action(post, owner_id.as_deref(), None, None, cx);
                });
            },
            cx,
        );
    }

    fn post_view_history_settled(cx: &mut Context<Self>) -> Option<Rc<dyn Fn(&mut App)>> {
        let this = cx.weak_entity();
        Some(Rc::new(move |cx: &mut App| {
            this.update(cx, |this, cx| this.resume_deferred_post_view_history_load_more(cx)).ok();
        }))
    }

    pub fn load_post_view_history_if_needed(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        if service.read(cx).post_view_history_pending() {
            return;
        }
        let did_load = service.update(cx, |service, cx| service.post_view_history(cx).did_load);
        let on_settled = Self::post_view_history_settled(cx);
        if did_load {
            Self::actions(cx).update(cx, |actions, cx| {
                actions.get_post_view_history_action(None, None, false, true, on_settled, cx);
            });
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_post_view_history_action(None, None, true, false, on_settled, cx);
        });
    }

    pub fn refresh_post_view_history(&mut self, cx: &mut Context<Self>) {
        if Self::service(cx).read(cx).post_view_history_pending() {
            return;
        }
        let on_settled = Self::post_view_history_settled(cx);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_post_view_history_action(None, None, false, false, on_settled, cx);
        });
    }

    pub fn load_more_post_view_history(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let state = service.update(cx, |service, cx| service.post_view_history(cx));
        let (true, Some(cursor), Some(cursor_post_id)) = (state.has_more, state.next_cursor, state.next_cursor_post_id)
        else {
            return;
        };
        if service.read(cx).post_view_history_pending() {
            self.post_view_history_load_more_deferred = true;
            cx.notify();
            return;
        }
        self.post_view_history_load_more_deferred = false;
        cx.notify();
        let on_settled = Self::post_view_history_settled(cx);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_post_view_history_action(Some(cursor), Some(cursor_post_id), false, false, on_settled, cx);
        });
    }

    fn resume_deferred_post_view_history_load_more(&mut self, cx: &mut Context<Self>) {
        if !self.post_view_history_load_more_deferred {
            return;
        }
        self.post_view_history_load_more_deferred = false;
        cx.notify();
        self.load_more_post_view_history(cx);
    }

    pub fn post_view_history_items(&self, cx: &mut App) -> Vec<GetPostFeedResponse> {
        Self::service(cx).update(cx, |service, cx| service.post_view_history(cx).items)
    }

    pub fn post_view_history_status(&self, cx: &mut App) -> SaiWsSliceStatus {
        Self::service(cx).update(cx, |service, cx| service.post_view_history_status(cx))
    }

    pub fn post_view_history_has_more(&self, cx: &mut App) -> bool {
        Self::service(cx).update(cx, |service, cx| service.post_view_history(cx).has_more)
    }

    pub fn post_view_history_error_text(&self, cx: &mut App) -> Option<String> {
        let state = Self::service(cx).update(cx, |service, cx| service.post_view_history(cx));
        state.last_error_text.or_else(|| Self::actions(cx).read(cx).get_post_view_history_sai.error())
    }

    pub fn clear_post_view_history(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        if actions.read(cx).clear_view_history_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.clear_view_history_action(cx));
    }

    pub fn delete_post_from_view_history(&mut self, post: &GetPostFeedResponse, cx: &mut Context<Self>) {
        if !Self::has_wire_user(cx) || !Self::is_positive_id(&post.id) {
            return;
        }
        let service = Self::service(cx);
        let Some(history_snapshot) =
            service.update(cx, |service, cx| service.remove_post_from_view_history_optimistic(&post.id, cx))
        else {
            return;
        };
        let Some(post_snapshot) = service.update(cx, |service, cx| service.apply_delete_optimistic(post, cx)) else {
            service.update(cx, |service, cx| service.restore_view_history(history_snapshot, cx));
            return;
        };
        let weak_service = service.downgrade();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.delete_post_action(
                &post.id,
                None,
                Some(Box::new(move |_error: String, cx: &mut App| {
                    weak_service
                        .update(cx, |service, cx| {
                            service.restore_view_history(history_snapshot, cx);
                            service.restore_delete_optimistic(post_snapshot, cx);
                        })
                        .ok();
                })),
                cx,
            );
        });
    }

    pub fn load_post_detail(&mut self, post_id: &str, cx: &mut Context<Self>) {
        let page = Self::service(cx).update(cx, |service, cx| service.detail_page(post_id, cx));
        if !Self::is_open_page(&page) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.get_post_action(post_id, cx));
    }

    pub fn refresh_post_detail(&mut self, post_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.refresh_post_detail_action(post_id, cx)).detach();
    }

    pub fn observe_post_detail(&mut self, post_id: &str, owner_id: Uuid, cx: &mut Context<Self>) {
        if !Self::is_positive_id(post_id) {
            return;
        }
        let generation = self.next_content_owner_generation(cx);
        let scope = RisePostPageScope::Detail { post_id: post_id.to_owned() };
        self.observe_content(scope, owner_id, generation, cx).detach();
    }

    pub fn stop_observing_post_detail(&mut self, post_id: &str, owner_id: Uuid, cx: &mut Context<Self>) {
        if !Self::is_positive_id(post_id) {
            return;
        }
        let generation = self.next_content_owner_generation(cx);
        let scope = RisePostPageScope::Detail { post_id: post_id.to_owned() };
        self.stop_observing_content(scope, owner_id, generation, cx).detach();
    }

    pub fn not_interested(&mut self, post: &GetPostFeedResponse, cx: &mut Context<Self>) {
        let post_id = post.rise_view_target_post_id();
        Self::actions(cx).update(cx, |actions, cx| actions.not_interested_action(&post_id, cx));
    }

    pub fn open_comments(&mut self, post: &GetPostFeedResponse, expand_caption: bool, cx: &mut Context<Self>) {
        let selected = post.clone();
        Self::service(cx).update(cx, |service, cx| {
            service.selected_post = Some(selected);
            cx.notify();
        });
        let post_id = post.rise_view_target_post_id();
        let author_id = post.author.id.clone().unwrap_or_else(|| post.author_id.clone());
        let surface = if post.is_short { EngagementSurface::ShortsInline } else { EngagementSurface::Feed };
        EngagementInteractionsStore::shared(cx).update(cx, |engagement, cx| {
            engagement.comments_opened(&post_id, Some(&author_id), surface, cx);
        });
        if let Some(hooks) = post_interactions_hooks() {
            (hooks.comment.open_comments_sheet)(post, expand_caption, cx);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_post(
        &mut self,
        post: &GetPostFeedResponse,
        title: &str,
        content: &str,
        manual_hashtags: &str,
        can_comment: bool,
        track_id: Option<String>,
        track: Option<RiseStoredMusicTrack>,
        track_start_ms: Option<i64>,
        track_end_ms: Option<i64>,
        track_plate: Option<MediaPickerTrackPlate>,
        count_visibility: Option<PostCountVisibility>,
        is_riff: Option<bool>,
        show_in_publications: Option<bool>,
        media: Option<Vec<PostEditMediaItem>>,
        tagged_users: Option<Vec<PostTagInputDraft>>,
        on_success: Option<PostInteractionsDoneHandler>,
        on_error: Option<PostInteractionsErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return Self::report_error(on_error, "missing_user_id", cx);
        }
        if !Self::is_positive_id(&post.id) {
            return Self::report_error(on_error, "invalid_post_id", cx);
        }
        let content_kind = RisePostContentKind::from_wire(&post.content_kind).unwrap_or_else(|| {
            RisePostContentKind::resolve(
                None,
                &post.images,
                post.video.as_deref(),
                post.is_short,
                post.is_image_reel,
                post.track_id.as_deref(),
            )
        });
        if PostFieldRules::is_description_missing(content_kind, content) {
            return Self::report_error(on_error, "thread_requires_description", cx);
        }
        let has_track =
            track_id.as_deref().is_some_and(|track_id| !trimming_whitespaces_and_newlines(track_id).is_empty());
        if content_kind == RisePostContentKind::Thread && has_track {
            return Self::report_error(on_error, "thread_cannot_contain_media", cx);
        }
        if let Some(media) = media.as_deref()
            && let Some(refusal) = self.edit_media_refusal(post, content_kind, media, has_track, cx)
        {
            return Self::report_error(on_error, &refusal, cx);
        }
        let edits_media_or_tags = media.is_some() || tagged_users.is_some();
        let slide_count = media.as_ref().map_or_else(|| post.media_slide_count(), |media| media.len() as i64);
        let resolved_tags = tagged_users.map(|drafts| self.edit_tag_drafts(drafts, slide_count, cx));
        let people: Vec<PostCollaboratorUser> = resolved_tags
            .as_ref()
            .map(|drafts| drafts.iter().filter_map(|draft| draft.user.clone()).collect())
            .unwrap_or_default();
        let this = cx.weak_entity();
        let service = Self::service(cx).downgrade();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_post_action(
                post.clone(),
                content_kind,
                title,
                content,
                manual_hashtags,
                can_comment,
                track_id,
                track,
                track_start_ms,
                track_end_ms,
                track_plate,
                count_visibility,
                is_riff,
                show_in_publications,
                media,
                resolved_tags,
                Some(Box::new(move |rejections: Vec<PostCollaboratorRejection>, cx: &mut App| {
                    this.update(cx, |this, cx| {
                        let limit = this.collaborator_limits(cx).1;
                        this.announce_collaborator_rejections(&rejections, &people, Some(limit), cx);
                    })
                    .ok();
                })),
                Some(Box::new(move |updated: GetPostFeedResponse, cx: &mut App| {
                    service
                        .update(cx, |service, cx| service.confirm_post_edit_post(&updated, edits_media_or_tags, cx))
                        .ok();
                    if let Some(on_success) = on_success {
                        on_success(cx);
                    }
                })),
                Some(Box::new(move |error: String, cx: &mut App| {
                    if let Some(on_error) = on_error {
                        on_error(error, cx);
                    }
                })),
                cx,
            );
        });
    }

    fn report_error(on_error: Option<PostInteractionsErrorHandler>, message: &str, cx: &mut App) {
        if let Some(on_error) = on_error {
            on_error(message.to_owned(), cx);
        }
    }

    fn maximum_edit_videos() -> i64 {
        1_i64.min(0_i64.max(MediaPickerConfiguration::post_media().max_video_count.unwrap_or(1)))
    }

    fn edit_media_refusal(
        &self,
        post: &GetPostFeedResponse,
        content_kind: RisePostContentKind,
        media: &[PostEditMediaItem],
        has_track: bool,
        cx: &mut App,
    ) -> Option<String> {
        let has_achievement = post.shared_achievement_id().is_some();
        match content_kind {
            RisePostContentKind::Thread => {
                return (!media.is_empty()).then(|| "thread_cannot_contain_media".to_owned());
            }
            RisePostContentKind::Publication => {
                if has_achievement && !media.is_empty() {
                    return Some("achievement_cannot_contain_media".to_owned());
                }
                if media.is_empty() && !has_track && !has_achievement {
                    return Some("publication_requires_media".to_owned());
                }
            }
        }

        let premium = AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| profile.more.is_premium);
        let ceiling = MediaPickerConfiguration::post_media_ceiling(premium);
        let video_count = media
            .iter()
            .filter(|item| match item {
                PostEditMediaItem::Existing(slide) => slide.is_video(),
                PostEditMediaItem::Video(_) => true,
                PostEditMediaItem::Photo(_) => false,
            })
            .count() as i64;
        let maximum_edit_videos = Self::maximum_edit_videos();
        if media.len() as i64 > ceiling || video_count > maximum_edit_videos {
            let limit = if video_count > maximum_edit_videos { maximum_edit_videos } else { ceiling };
            let details = HashMap::from([(ErrorParam::Limit.wire().to_owned(), limit.to_string())]);
            return Some(
                CanonErrors::text(Some(ErrorCode::CommonFieldTooManyItems.wire()), &details)
                    .unwrap_or_else(|| "create_post_error_connection".to_owned()),
            );
        }

        for item in media {
            let PostEditMediaItem::Video(picked) = item else {
                continue;
            };
            if picked.kind != MediaPickerItemKind::Video {
                return Some("publication_requires_media".to_owned());
            }
            if let Some(duration) = picked.video_payload.as_ref().map(|payload| payload.duration)
                && duration > ShortVideoLimits::MAX_DURATION_SECONDS as f64 + 0.05
            {
                return Some(tr("create_post_short_too_long"));
            }
        }
        None
    }

    fn edit_tag_drafts(&self, drafts: Vec<PostTagInputDraft>, slide_count: i64, cx: &mut App) -> Vec<PostTagInputDraft> {
        let (_, tagged_users_limit, tag_placements_limit) = self.collaborator_limits(cx);
        let mut seen: HashSet<String> = HashSet::new();
        let mut resolved: Vec<PostTagInputDraft> = Vec::new();
        for draft in drafts {
            if !seen.insert(draft.user_id.clone()) {
                continue;
            }
            if resolved.len() as i64 >= tagged_users_limit {
                break;
            }
            let in_range: Vec<_> = draft
                .placements
                .iter()
                .filter(|placement| placement.slide_index >= 0 && placement.slide_index < slide_count)
                .cloned()
                .collect();
            let placements = PostMediaSlides::first_placement_per_slide(&in_range);
            resolved.push(PostTagInputDraft {
                user_id: draft.user_id,
                placements: placements.into_iter().take(tag_placements_limit.max(0) as usize).collect(),
                user: draft.user,
            });
        }
        resolved
    }

    pub fn record_share(
        &mut self,
        post: &GetPostFeedResponse,
        channel: PostInteractionsStoreShareReceiptChannel,
        cx: &mut Context<Self>,
    ) {
        if post.is_temp {
            return;
        }
        let post_id = post.rise_view_target_post_id();
        Self::actions(cx).update(cx, |actions, cx| actions.record_share_action(&post_id, channel.raw_value(), cx));
        self.report_share_engagement(post, cx);
    }

    pub fn note_chat_shares(&mut self, post: &GetPostFeedResponse, chat_count: i64, cx: &mut Context<Self>) {
        if post.is_temp || chat_count <= 0 {
            return;
        }
        let post_id = post.rise_view_target_post_id();
        let share_count = post.share_count + chat_count;
        Self::service(cx).update(cx, |service, cx| service.apply_share_count(&post_id, share_count, cx));
    }

    pub fn report_share_engagement(&mut self, post: &GetPostFeedResponse, cx: &mut Context<Self>) {
        if post.is_temp {
            return;
        }
        let post_id = post.rise_view_target_post_id();
        let author_id = post.author.id.clone().unwrap_or_else(|| post.author_id.clone());
        let surface = if post.is_short { EngagementSurface::ShortViewer } else { EngagementSurface::Feed };
        let is_short = post.is_short;
        EngagementInteractionsStore::shared(cx).update(cx, |engagement, cx| {
            engagement.share(&post_id, &author_id, surface, is_short, cx);
        });
    }

    pub fn record_link_open(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if !Self::has_wire_user(cx) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.record_link_open_action(post_id, cx));
    }

    pub fn delete_own_post(
        &mut self,
        post: &GetPostFeedResponse,
        on_error: Option<PostInteractionsErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return Self::report_error(on_error, "missing_user_id", cx);
        }
        if !Self::is_positive_id(&post.id) {
            return Self::report_error(on_error, "invalid_post_id", cx);
        }
        let is_riff = post.is_riff;
        let post = post.clone();
        NativeConfirmAlert::present_delete(
            &tr(if is_riff { "riff_delete_confirm_title" } else { "post_delete_confirm_title" }),
            &tr(if is_riff { "riff_delete_confirm_message" } else { "post_delete_confirm_message" }),
            None,
            None,
            false,
            None,
            None,
            move |cx: &mut App| Self::confirm_delete_own_post(post, on_error, cx),
            cx,
        );
    }

    fn confirm_delete_own_post(
        post: GetPostFeedResponse,
        on_error: Option<PostInteractionsErrorHandler>,
        cx: &mut App,
    ) {
        let service = PostServicesStore::shared(cx);
        let resolved =
            service.update(cx, |service, cx| service.resolved_post(&post.id, cx)).unwrap_or_else(|| post.clone());
        let user_id = Self::effective_user_id_for_wire(cx).unwrap_or_default();
        let counts_in_profile = PostMediaCachePolicy::is_own_post(&resolved, cx)
            && PostServicesStore::counts_as_profile_publication(&resolved.content_kind, resolved.show_in_publications);
        if counts_in_profile {
            service.update(cx, |service, cx| service.adjust_own_publications_count(-1, &user_id, cx));
        }
        service.update(cx, |service, cx| service.mark_own_post_deleted(Some(&post.id), cx));
        let (feed_post, feed_index, external_post) = ShortsServicesStore::shared(cx)
            .update(cx, |shorts, cx| shorts.remove_short_everywhere_snapshot(&post.id, cx));
        let success_service = service.downgrade();
        let error_service = service.downgrade();
        let post_id = post.id.clone();
        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.delete_post_action(
                &post.id,
                Some(Box::new(move |cx: &mut App| {
                    if counts_in_profile {
                        success_service.update(cx, |service, cx| service.reconcile_own_publications_count(cx)).ok();
                    }
                })),
                Some(Box::new(move |error: String, cx: &mut App| {
                    ShortsServicesStore::shared(cx).update(cx, |shorts, cx| {
                        shorts.restore_short_everywhere_snapshot(feed_post, feed_index, external_post, cx);
                    });
                    error_service
                        .update(cx, |service, cx| {
                            service.restore_own_deleted_post(&post_id, cx);
                            if counts_in_profile {
                                service.adjust_own_publications_count(1, &user_id, cx);
                            }
                        })
                        .ok();
                    if let Some(on_error) = on_error {
                        on_error(error, cx);
                    } else {
                        Notifier::send(
                            error,
                            NotifierSendOptions {
                                r#type: NotifierType::Error,
                                position: NotifierPosition::Bottom,
                                ..Default::default()
                            },
                            cx,
                        );
                    }
                })),
                cx,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn publish(
        &mut self,
        content_kind: RisePostContentKind,
        title: &str,
        content: &str,
        manual_hashtags: &str,
        can_comment: bool,
        photos: Vec<SaiWsOptimisticLocalFile>,
        video: Option<MediaPickerItem>,
        video_position: Option<i64>,
        is_riff: bool,
        show_in_publications: bool,
        achievement_id: Option<i64>,
        map_journey_id: Option<String>,
        map_journey_show_heat: bool,
        track_id: Option<String>,
        attached_track: Option<RiseStoredMusicTrack>,
        track_start_ms: Option<i64>,
        track_end_ms: Option<i64>,
        track_plate: Option<MediaPickerTrackPlate>,
        count_visibility: PostCountVisibility,
        coauthors: Vec<PostCollaboratorUser>,
        tagged_users: Vec<PostTagInputDraft>,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if let Some(sandbox) = post_guide_sandbox_hooks()
            && (sandbox.intercepts_publish)(cx)
        {
            (sandbox.publish_composed_post)(cx);
            if let Some(on_success) = on_success {
                on_success(cx);
            }
            return;
        }
        let fail = |message: &str, cx: &mut App| {
            if let Some(on_error) = on_error.as_ref() {
                on_error(message.to_owned(), cx);
            }
        };
        let has_track =
            track_id.as_deref().is_some_and(|track_id| !trimming_whitespaces_and_newlines(track_id).is_empty());
        let has_achievement = achievement_id.unwrap_or(0) > 0;
        let has_media = !photos.is_empty() || video.is_some();
        if has_achievement && (has_media || has_track) {
            return fail("achievement_cannot_contain_media", cx);
        }
        match content_kind {
            RisePostContentKind::Thread => {
                if has_media || has_track {
                    return fail("thread_cannot_contain_media", cx);
                }
                if PostFieldRules::is_description_missing(content_kind, content) {
                    return fail("thread_requires_description", cx);
                }
            }
            RisePostContentKind::Publication => {
                if !has_media && !has_track && !has_achievement {
                    return fail("publication_requires_media", cx);
                }
            }
        }
        let placement = RisePostPlacement::resolve(
            content_kind,
            is_riff,
            show_in_publications,
            video.is_some(),
            photos.len() as i64,
        );
        let request = Rc::new(PostInteractionsPublishRequest {
            content_kind,
            title: title.to_owned(),
            content: content.to_owned(),
            manual_hashtags: manual_hashtags.to_owned(),
            can_comment,
            photos,
            video,
            video_position,
            placement,
            achievement_id: if has_achievement { achievement_id } else { None },
            map_journey_id,
            map_journey_show_heat,
            track_id,
            attached_track,
            track_start_ms,
            track_end_ms,
            track_plate,
            count_visibility,
            coauthors,
            tagged_users,
            started_at: RiseClock::now(),
            retry_user_id: Self::effective_user_id_for_wire(cx),
            on_success,
            on_error,
        });
        Self::send_publish(request, cx);
    }

    fn send_publish(request: Rc<PostInteractionsPublishRequest>, cx: &mut App) {
        PostServicesStore::shared(cx).update(cx, |service, _| service.publish_retry = None);
        let tagged_users: Vec<PostTagInputDraft> = request
            .tagged_users
            .iter()
            .map(|draft| PostTagInputDraft {
                user_id: draft.user_id.clone(),
                placements: PostMediaSlides::first_placement_per_slide(&draft.placements),
                user: draft.user.clone(),
            })
            .collect();
        let people: Vec<PostCollaboratorUser> = request
            .coauthors
            .iter()
            .cloned()
            .chain(request.tagged_users.iter().filter_map(|draft| draft.user.clone()))
            .collect();
        let progress_title = if request.placement.is_riff && !request.placement.show_in_publications {
            tr("riffs_upload_background")
        } else {
            tr("post_upload_background")
        };
        let has_media = !request.photos.is_empty() || request.video.is_some();
        let banner: Option<Rc<NotifierUploadProgress>> =
            has_media.then(|| Rc::new(NotifierUploadProgress::new(progress_title, None, None, cx)));
        let on_progress: Option<Arc<dyn Fn(f64, &mut App) + Send + Sync>> = banner.clone().map(|banner| {
            let (progress_values, mut values) = mpsc::unbounded::<f64>();
            cx.spawn(async move |cx: &mut AsyncApp| {
                while let Some(value) = values.next().await {
                    cx.update(|cx| banner.update(value, cx));
                }
            })
            .detach();
            Arc::new(move |value: f64, _: &mut App| {
                let _ = progress_values.unbounded_send(value);
            }) as Arc<dyn Fn(f64, &mut App) + Send + Sync>
        });
        let settled_banner = banner.clone();
        let success_banner = banner.clone();
        let error_banner = banner;
        let success_request = Rc::clone(&request);
        let error_request = Rc::clone(&request);
        let request = Rc::unwrap_or_clone(request);
        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.create_post_action(
                request.content_kind,
                &request.title,
                &request.content,
                &request.manual_hashtags,
                request.can_comment,
                request.photos,
                request.video,
                request.video_position,
                request.placement.is_riff,
                request.placement.show_in_publications,
                request.achievement_id,
                request.map_journey_id,
                request.map_journey_show_heat,
                request.track_id,
                request.attached_track,
                request.track_start_ms,
                request.track_end_ms,
                request.track_plate,
                request.count_visibility,
                request.coauthors.iter().map(|coauthor| coauthor.id.clone()).collect(),
                tagged_users,
                Some(Box::new(move |rejections: Vec<PostCollaboratorRejection>, cx: &mut App| {
                    PostInteractionsStore::shared(cx).update(cx, |this, cx| {
                        this.announce_collaborator_rejections(&rejections, &people, None, cx);
                    });
                })),
                on_progress,
                Some(Box::new(move |cx: &mut App| {
                    if let Some(banner) = &settled_banner {
                        banner.finish(cx);
                    }
                })),
                Some(Box::new(move |_created: GetPostFeedResponse, cx: &mut App| {
                    GuideSignalBus::emit(RiseGuideSignal::PostCreated);
                    let title = Self::published_title(success_request.content_kind, success_request.placement);
                    if let Some(banner) = &success_banner {
                        banner.succeed(title, None, cx);
                    } else {
                        Self::announce_published(title, &success_request.started_at, cx);
                    }
                    if let Some(on_success) = success_request.on_success.as_ref() {
                        on_success(cx);
                    }
                })),
                Some(Box::new(move |message: String, cx: &mut App| {
                    if let Some(banner) = &error_banner {
                        banner.finish(cx);
                    }
                    if RisePostPublishFailure::is_retryable(&message) {
                        let retry_request = Rc::clone(&error_request);
                        let retry: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
                            if Self::effective_user_id_for_wire(cx) != retry_request.retry_user_id {
                                return;
                            }
                            Self::send_publish(Rc::clone(&retry_request), cx);
                        });
                        PostServicesStore::shared(cx).update(cx, |service, _| service.publish_retry = Some(retry));
                    }
                    if let Some(on_error) = error_request.on_error.as_ref() {
                        on_error(message, cx);
                    }
                })),
                cx,
            );
        });
    }

    fn published_title(content_kind: RisePostContentKind, placement: RisePostPlacement) -> String {
        if content_kind == RisePostContentKind::Thread {
            return tr("create_post_thread_success_text");
        }
        if placement.is_riff && !placement.show_in_publications {
            return tr_args("create_post_short_success_text", &[FormatArg::from(tr("riffs"))]);
        }
        tr("create_post_success_text")
    }

    fn announce_published(title: String, started_at: &Date, cx: &mut App) {
        if RiseClock::now().time_interval_since(started_at) < Self::SLOW_PUBLISH_NOTICE_SECONDS {
            return;
        }
        Notifier::send(
            title,
            NotifierSendOptions {
                r#type: NotifierType::Success,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("checkmark.circle.fill".to_owned()),
                ..Default::default()
            },
            cx,
        );
    }

    pub fn can_retry_publish(&self, cx: &mut App) -> bool {
        Self::service(cx).read(cx).publish_retry.is_some()
    }

    pub fn retry_publish(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let retry = service.read(cx).publish_retry.clone();
        service.update(cx, |service, cx| {
            service.publish_retry = None;
            service.set_publish_error(None, cx);
        });
        if let Some(retry) = retry {
            retry(cx);
        }
    }

    pub fn dismiss_publish_error(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| {
            service.publish_retry = None;
            service.set_publish_error(None, cx);
        });
    }

    pub fn is_creating_post(&self, cx: &mut App) -> bool {
        let service = Self::service(cx);
        let service = service.read(cx);
        service.create_post_uploading || service.publish_short_pending
    }

    pub fn create_post_upload_progress(&self, cx: &mut App) -> f64 {
        Self::service(cx).read(cx).create_post_upload_progress
    }

    pub fn feed_items(&self, cx: &mut App) -> Vec<GetPostFeedResponse> {
        Self::service(cx).update(cx, |service, cx| service.feed_items(cx))
    }

    pub fn is_feed_loading(&self, cx: &mut App) -> bool {
        Self::actions(cx).read(cx).get_feed_sai.is_pending()
    }

    pub fn feed_status(&self, cx: &mut App) -> SaiWsSliceStatus {
        Self::actions(cx).read(cx).get_feed_sai.status()
    }

    pub fn feed_error_text(&self, cx: &mut App) -> Option<String> {
        let service_error = Self::service(cx).read(cx).feed_last_error_text().map(str::to_owned);
        service_error.or_else(|| Self::actions(cx).read(cx).get_feed_sai.error())
    }

    pub fn load_post_stats(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if let Some(sandbox) = post_guide_sandbox_hooks()
            && (sandbox.post_stats)(post_id, cx).is_some()
        {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_post_stats_action(post_id, Self::POST_STATS_DAYS, true, cx);
        });
    }

    pub fn refresh_post_stats(&mut self, post_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_post_stats_action(post_id, Self::POST_STATS_DAYS, true, cx);
        });
    }

    pub fn close_post_stats(&mut self, post_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.close_post_stats_action(post_id, cx));
    }

    pub fn post_stats(&self, post_id: &str, cx: &mut App) -> Option<RiseStoredPostStats> {
        Self::service(cx).update(cx, |service, cx| service.post_stats(post_id, cx))
    }

    pub fn is_post_stats_loading(&self, post_id: &str, cx: &mut App) -> bool {
        Self::service(cx).update(cx, |service, cx| service.post_stats_status(post_id, cx)) == SaiWsSliceStatus::Pending
    }

    pub fn post_stats_status(&self, post_id: &str, cx: &mut App) -> SaiWsSliceStatus {
        Self::service(cx).update(cx, |service, cx| service.post_stats_status(post_id, cx))
    }

    pub fn post_stats_error_text(&self, post_id: &str, cx: &mut App) -> Option<String> {
        Self::service(cx).update(cx, |service, cx| service.post_stats_error_text(post_id, cx))
    }

    pub fn prepare_post_view_intent(
        &self,
        post_id: &str,
        surface: RisePostViewSurface,
        scope_id: Uuid,
        scope_generation: u64,
    ) -> Option<RisePostVisibilityIntent> {
        let post_id = int_from_string(post_id).filter(|post_id| *post_id > 0)?;
        let account_id = SaiAccountCacheScope::active_user_id()?;
        let account_epoch = SaiAccountCacheScope::active_epoch()?;
        Some(RisePostVisibilityIntent { post_id, surface, scope_id, scope_generation, account_id, account_epoch })
    }

    pub fn record_post_view(&mut self, intent: RisePostVisibilityIntent, cx: &mut Context<Self>) -> Task<bool> {
        if !SaiAccountCacheScope::is_current_account_epoch(Some(&intent.account_id), Some(&intent.account_epoch)) {
            return Task::ready(false);
        }
        Self::actions(cx).update(cx, |actions, cx| actions.record_post_view_action(intent, cx))
    }

    const REPOST_OPTIMISTIC_HOLD: Duration = Duration::from_millis(600);

    pub fn toggle_repost(
        &mut self,
        post: &GetPostFeedResponse,
        on_error: Option<PostInteractionsErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        if !post.is_reposted && PostMediaCachePolicy::is_own_post(post, cx) {
            Notifier::send(
                tr("repost_error_own_post"),
                NotifierSendOptions {
                    r#type: NotifierType::Info,
                    position: NotifierPosition::Bottom,
                    ..Default::default()
                },
                cx,
            );
            return;
        }
        let post_id = post.id.clone();
        let desired = !post.is_reposted;
        Self::service(cx).update(cx, |service, cx| service.apply_repost_state(&post_id, desired, cx));
        let tapped_at = RiseClock::monotonic();
        let revert: PostInteractionsErrorHandler = Box::new(move |message: String, cx: &mut App| {
            let remaining = Self::REPOST_OPTIMISTIC_HOLD.saturating_sub(RiseClock::monotonic().saturating_sub(tapped_at));
            cx.spawn(async move |cx: &mut AsyncApp| {
                if !remaining.is_zero() {
                    cx.background_executor().timer(remaining).await;
                }
                let _ = cx.update(|cx| {
                    PostServicesStore::shared(cx)
                        .update(cx, |service, cx| service.apply_repost_state(&post_id, !desired, cx));
                    if let Some(on_error) = on_error {
                        on_error(message, cx);
                    } else {
                        Notifier::send(
                            message,
                            NotifierSendOptions {
                                r#type: NotifierType::Error,
                                position: NotifierPosition::Bottom,
                                ..Default::default()
                            },
                            cx,
                        );
                    }
                });
            })
            .detach();
        });

        if post.is_reposted {
            let post = post.clone();
            Self::actions(cx).update(cx, |actions, cx| {
                actions.delete_repost_action(post, None, None, Some(revert), cx);
            });
            return;
        }

        let opens_opinion = post.media_slide_count() == 0 && post.shared_achievement_id().is_none();
        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let opinion_post = post.clone();
        let on_success: Option<PostInteractionsDoneHandler> = opens_opinion.then(|| {
            Box::new(move |cx: &mut App| {
                let (Some(account_id), Some(account_epoch)) = (account_id, account_epoch) else {
                    return;
                };
                if !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&account_epoch)) {
                    return;
                }
                let mut reposted = opinion_post;
                reposted.is_reposted = true;
                RepostOpinionEditorPresenter::shared(cx).update(cx, |presenter, cx| {
                    presenter.present(reposted, Uuid::new_v4().uuid_string(), RepostStickerStyle::Feed, cx);
                });
            }) as PostInteractionsDoneHandler
        });
        let post = post.clone();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.create_repost_action(post, None, on_success, Some(revert), cx);
        });
    }

    pub fn edit_repost_opinion(
        &mut self,
        post: &GetPostFeedResponse,
        content: &str,
        entities: Option<String>,
        on_error: Option<PostInteractionsErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        if !post.is_reposted {
            return;
        }
        let trimmed = PostFieldRules::repost_content()
            .schema
            .map(|schema| schema.normalized_clamped(content))
            .unwrap_or_else(|| content.to_owned());
        let resolved_entities = if trimmed.is_empty() { None } else { entities };
        let post_id = post.id.clone();
        let previous = post.my_repost_content.clone();
        let previous_entities = post.my_repost_entities.clone();
        Self::service(cx).update(cx, |service, cx| {
            service.apply_repost_opinion(&post_id, Some(&trimmed), resolved_entities.as_deref(), cx);
        });
        let post = post.clone();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_repost_opinion_action(
                post,
                &trimmed,
                resolved_entities,
                None,
                Some(Box::new(move |error: String, cx: &mut App| {
                    PostServicesStore::shared(cx).update(cx, |service, cx| {
                        service.apply_repost_opinion(&post_id, previous.as_deref(), previous_entities.as_deref(), cx);
                    });
                    if let Some(on_error) = on_error {
                        on_error(error, cx);
                    }
                })),
                cx,
            );
        });
    }

    fn trim_intent_maps(&mut self) {
        while self.like_intents.len() > Self::INTENT_MAP_LIMIT {
            let Some(key) = self.like_intents.keys().next().cloned() else {
                break;
            };
            self.like_intents.remove(&key);
        }
        while self.favorite_intents.len() > Self::INTENT_MAP_LIMIT {
            let Some(key) = self.favorite_intents.keys().next().cloned() else {
                break;
            };
            self.favorite_intents.remove(&key);
        }
    }

    fn next_content_owner_generation(&mut self, cx: &mut Context<Self>) -> u64 {
        self.content_owner_clock = self.content_owner_clock.wrapping_add(1);
        cx.notify();
        self.content_owner_clock
    }

    fn observe_content(
        &mut self,
        scope: RisePostPageScope,
        owner_id: Uuid,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        self.pending_content_observations.insert(owner_id, generation);
        cx.notify();
        let deadline = RiseClock::monotonic()
            + RiseCanonEndpoints::required(rise_wire_methods::post::SERVICE, rise_wire_methods::post::GET_FEED)
                .client_timeout();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            Self::run_content_observation(&this, scope, owner_id, generation, deadline, cx).await;
            this.update(cx, |this, cx| {
                if this.pending_content_observations.get(&owner_id) == Some(&generation) {
                    this.pending_content_observations.remove(&owner_id);
                    cx.notify();
                }
            })
            .ok();
        })
    }

    async fn run_content_observation(
        this: &WeakEntity<Self>,
        scope: RisePostPageScope,
        owner_id: Uuid,
        generation: u64,
        deadline: Duration,
        cx: &mut AsyncApp,
    ) {
        while this
            .read_with(cx, |this, _| this.pending_content_observations.get(&owner_id) == Some(&generation))
            .unwrap_or(false)
        {
            let (sender, mut ready) = mpsc::unbounded::<()>();
            let _ready_subscription = RiseAppEvents::default_center().observe(RISE_ENGINE_CONNECTION_READY, move |_| {
                let _ = sender.unbounded_send(());
            });
            let attempt = this.update(cx, |_, cx| {
                PostActionsStore::shared(cx)
                    .update(cx, |actions, cx| actions.observe_content_action(scope.clone(), owner_id, generation, cx))
            });
            let Ok(attempt) = attempt else {
                return;
            };
            let Err(error) = attempt.await else {
                return;
            };
            let remaining = deadline.saturating_sub(RiseClock::monotonic());
            if !Self::content_engine_not_ready(&error) || remaining.is_zero() {
                return;
            }
            let timer = cx.background_executor().timer(remaining.min(Self::CONTENT_READY_POLL));
            futures::future::select(pin!(ready.next()), pin!(timer)).await;
        }
    }

    fn stop_observing_content(
        &mut self,
        scope: RisePostPageScope,
        owner_id: Uuid,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        self.pending_content_observations.remove(&owner_id);
        cx.notify();
        Self::actions(cx).update(cx, |actions, cx| actions.stop_observing_content_action(scope, owner_id, generation, cx))
    }

    fn content_engine_not_ready(error: &anyhow::Error) -> bool {
        error.downcast_ref::<RisePostRepositoryError>().is_some()
            || error.downcast_ref::<RiseEngineAppRuntimeError>().is_some()
            || error.downcast_ref::<CancellationError>().is_some()
    }
}
