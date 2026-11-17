use std::any::Any;
use std::rc::Rc;
use std::sync::Arc;

use futures::FutureExt;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::Shared;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_platform::process_activity::begin_process_activity;
use rise_canon::{RiseUploadFormat, rise_request, rise_wire_methods};
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_i18n::tr;
use serde_json::{Map, Value};

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, UIApplication};
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::desktop_platform::rise_app_quit_drain::QuitDrain;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiTypedSlice, SaiWireEmptyDTO};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::sai::Sai;
use crate::core::restriction::restriction_store::RestrictionStore;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind, MediaPickerTrackPlate};
use crate::modules::media_picker::api::short_video_limits::ShortVideoLimits;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::post::engine::rise_post_content_repository::{
    RisePostContentRepositoryLoadIntent, RisePostEngagementRefusalHandler,
};
use crate::modules::post::engine::rise_post_domain::{
    RisePostActionExecutor, RisePostDomain, RisePostDomainRegistry, RisePostPendingHandler, RisePostProgressHandler,
};
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostContentKind, RisePostCreateDraft, RisePostEditMediaItem, RisePostFeedKind, RisePostLocalUpload,
    RisePostMediaEdit, RisePostPageScope, RisePostProfileKind, RisePostSearchKind, RisePostTrackPlateDraft,
    RisePostUpdateDraft, RisePostVisibilityIntent,
};
use crate::modules::post::shared::post_field_rules::PostFieldRules;
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::stores::post_actions::post_likes_wire_types::GetPostLikesGatewayDTO;
use crate::modules::post::stores::post_actions::post_types::{
    AuthorInfo, AuthorInfoMore, GetPostFeedResponse, PostCollaboratorRejection, PostCountVisibility,
    PostEditMediaItem, PostHashtagParsing, PostTagInputDraft,
};
use crate::modules::post::stores::post_actions::post_wire_types::{GetFeedGatewayDTO, GetViewHistoryGatewayDTO};
use crate::modules::post::stores::post_actions::rise_post_publish_failure::RisePostPublishFailure;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

type PostActionErrorHandler = Box<dyn FnOnce(String, &mut App)>;
type PostActionRejectionsHandler = Box<dyn FnOnce(Vec<PostCollaboratorRejection>, &mut App)>;
type PostActionPostHandler = Box<dyn FnOnce(GetPostFeedResponse, &mut App)>;
type PostActionDoneHandler = Box<dyn FnOnce(&mut App)>;
type PostActionAppliedHandler = Box<dyn FnOnce(bool, &mut App)>;
type PostActionProgressHandler = Arc<dyn Fn(f64, &mut App) + Send + Sync>;

pub struct PostActionsStore {
    author_relationship_write: Option<Shared<Task<()>>>,
    pub not_interested_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_feed_sai: Rc<SaiTypedSlice<GetFeedGatewayDTO>>,
    pub get_posts_by_track_sai: Rc<SaiTypedSlice<GetFeedGatewayDTO>>,
    pub get_user_posts_sai: Rc<SaiTypedSlice<GetFeedGatewayDTO>>,
    pub get_user_reposts_sai: Rc<SaiTypedSlice<GetFeedGatewayDTO>>,
    pub get_post_view_history_sai: Rc<SaiTypedSlice<GetViewHistoryGatewayDTO>>,
    pub clear_view_history_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_post_likes_sai: Rc<SaiTypedSlice<GetPostLikesGatewayDTO>>,
}

impl PostActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            author_relationship_write: None,
            not_interested_sai: SaiTypedSlice::new(cx),
            get_feed_sai: SaiTypedSlice::new(cx),
            get_posts_by_track_sai: SaiTypedSlice::new(cx),
            get_user_posts_sai: SaiTypedSlice::new(cx),
            get_user_reposts_sai: SaiTypedSlice::new(cx),
            get_post_view_history_sai: SaiTypedSlice::new(cx),
            clear_view_history_sai: SaiTypedSlice::new(cx),
            get_post_likes_sai: SaiTypedSlice::new_unobserved(cx),
        }
    }

    pub fn not_interested_action(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if post_id.is_empty() {
            return;
        }
        let Some(int_id) = int_from_string(post_id) else {
            return;
        };
        let has_me = AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| !profile.id.is_empty());
        if !has_me {
            return;
        }

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::post::SERVICE.to_owned(),
            method: rise_wire_methods::post::NOT_INTERESTED.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {});
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Self::log_network("Post", format!("not_interested error: {error}"), cx);
        });
        SaiWs::request_slice(
            &self.not_interested_sai,
            rise_request::post::NotInterested { post_id: Some(int_id) },
            options,
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn get_feed_action(
        &mut self,
        relative_id: Option<&str>,
        new_feed: bool,
        need_pending: bool,
        fetch_if_have_data: bool,
        kind: RisePostFeedKind,
        cx: &mut Context<Self>,
    ) {
        let _ = (need_pending, fetch_if_have_data);
        if relative_id.is_some() || !new_feed {
            self.load_more_feed_action(kind, cx).detach();
        } else {
            self.activate_feed_action(kind, cx).detach();
        }
    }

    pub fn activate_feed_action(&mut self, kind: RisePostFeedKind, cx: &mut Context<Self>) -> Task<()> {
        Self::on_domain(cx, move |domain| async move { domain.activate_feed(kind).await })
    }

    pub fn refresh_feed_action(&mut self, kind: RisePostFeedKind, cx: &mut Context<Self>) -> Task<()> {
        Self::on_domain(cx, move |domain| async move { domain.refresh_feed(kind).await })
    }

    pub fn load_more_feed_action(&mut self, kind: RisePostFeedKind, cx: &mut Context<Self>) -> Task<()> {
        Self::on_domain(cx, move |domain| async move { domain.load_more_feed(kind).await })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_user_posts_action(
        &mut self,
        tag: &str,
        profile_user_id: &str,
        relative_id: Option<&str>,
        sort: &str,
        need_pending: bool,
        fetch_if_have_data: bool,
        kind: RisePostProfileKind,
        cx: &mut Context<Self>,
    ) {
        if tag.is_empty() || profile_user_id.is_empty() {
            return;
        }
        let _ = (sort, need_pending, fetch_if_have_data);
        if relative_id.is_none() {
            self.activate_profile_action(profile_user_id, tag, kind, cx).detach();
        } else {
            self.load_more_profile_action(profile_user_id, tag, kind, cx).detach();
        }
    }

    pub fn activate_profile_action(
        &mut self,
        user_id: &str,
        tag: &str,
        kind: RisePostProfileKind,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let (user_id, tag) = (user_id.to_owned(), tag.to_owned());
        Self::on_domain(cx, move |domain| async move { domain.activate_profile(&user_id, &tag, kind).await })
    }

    pub fn refresh_profile_action(
        &mut self,
        user_id: &str,
        tag: &str,
        kind: RisePostProfileKind,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let (user_id, tag) = (user_id.to_owned(), tag.to_owned());
        Self::on_domain(cx, move |domain| async move { domain.refresh_profile(&user_id, &tag, kind).await })
    }

    pub fn load_more_profile_action(
        &mut self,
        user_id: &str,
        tag: &str,
        kind: RisePostProfileKind,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let (user_id, tag) = (user_id.to_owned(), tag.to_owned());
        Self::on_domain(cx, move |domain| async move { domain.load_more_profile(&user_id, &tag, kind).await })
    }

    pub fn get_user_reposts_action(
        &mut self,
        tag: &str,
        profile_user_id: &str,
        relative_id: Option<&str>,
        need_pending: bool,
        fetch_if_have_data: bool,
        cx: &mut Context<Self>,
    ) {
        if tag.is_empty() || profile_user_id.is_empty() {
            return;
        }
        let _ = (need_pending, fetch_if_have_data);
        if relative_id.is_none() {
            self.activate_profile_action(profile_user_id, tag, RisePostProfileKind::Reposts, cx).detach();
        } else {
            self.load_more_profile_action(profile_user_id, tag, RisePostProfileKind::Reposts, cx).detach();
        }
    }

    pub fn activate_post_search_action(
        &mut self,
        query: &str,
        kind: RisePostSearchKind,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let query = query.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.activate_post_search(&query, kind).await })
    }

    pub fn refresh_post_search_action(
        &mut self,
        query: &str,
        kind: RisePostSearchKind,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let query = query.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.refresh_post_search(&query, kind).await })
    }

    pub fn load_more_post_search_action(
        &mut self,
        query: &str,
        kind: RisePostSearchKind,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let query = query.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.load_more_post_search(&query, kind).await })
    }

    pub fn observe_content_action(
        &mut self,
        scope: RisePostPageScope,
        owner_id: Uuid,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let domain = domain.await?;
            domain.observe_content(scope, owner_id, generation).await
        })
    }

    pub fn load_shorts_action(&mut self, intent: RisePostContentRepositoryLoadIntent, cx: &mut Context<Self>) -> Task<()> {
        Self::on_domain(cx, move |domain| async move { domain.load_shorts(intent).await })
    }

    pub fn stop_observing_content_action(
        &mut self,
        scope: RisePostPageScope,
        owner_id: Uuid,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let Some(domain) = RisePostDomainRegistry::shared(cx).read(cx).active().cloned() else {
            return Task::ready(());
        };
        cx.spawn(async move |_, _: &mut AsyncApp| {
            domain.stop_observing_content(&scope, owner_id, generation).await;
        })
    }

    pub fn get_post_view_history_action(
        &mut self,
        cursor: Option<i64>,
        cursor_post_id: Option<i64>,
        need_pending: bool,
        keeps_loaded_tail: bool,
        on_settled: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        let services = PostServicesStore::shared(cx);
        services.update(cx, |services, cx| services.mark_post_view_history_pending(cx));

        let is_first_page = cursor.is_none();
        let cache_key = PostServicesStore::post_view_history_cache_key(Some(user_id.as_str()), cx);
        let body = rise_request::post::GetViewHistory { cursor, cursor_post_id, limit: Some(20) };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::post::SERVICE.to_owned(),
            method: rise_wire_methods::post::GET_VIEW_HISTORY.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: need_pending,
            send_immediately: true,
            store_response_in_cache: false,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::MemoryOnly),
            ..SaiWsRequestOptionsInit::default()
        });

        let success_settled = on_settled.clone();
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            PostServicesStore::shared(cx).update(cx, |services, cx| {
                services.get_post_view_history_success_handler(payload, is_first_page, keeps_loaded_tail, cx);
            });
            if let Some(on_settled) = &success_settled {
                on_settled(cx);
            }
        });
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            PostServicesStore::shared(cx)
                .update(cx, |services, cx| services.get_post_view_history_error_handler(error, cx));
            if let Some(on_settled) = &on_settled {
                on_settled(cx);
            }
        });

        SaiWs::request_slice(&self.get_post_view_history_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn clear_view_history_action(&mut self, cx: &mut Context<Self>) {
        if Self::wire_user_id(cx).is_none() {
            return;
        }

        let services = PostServicesStore::shared(cx);
        let snapshot: Option<Map<String, Value>> =
            services.update(cx, |services, cx| services.post_view_history_snapshot(cx));
        services.update(cx, |services, cx| services.clear_view_history_local(cx));

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::post::SERVICE.to_owned(),
            method: rise_wire_methods::post::CLEAR_VIEW_HISTORY.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {});
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network("Post", format!("clear_view_history error: {error}"), cx);
            if let Some(snapshot) = &snapshot {
                let snapshot = snapshot.clone();
                PostServicesStore::shared(cx).update(cx, |services, cx| services.restore_view_history(snapshot, cx));
            }
        });

        SaiWs::request_slice(
            &self.clear_view_history_sai,
            rise_request::post::ClearViewHistory {},
            options,
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn get_post_action(&mut self, post_id: &str, cx: &mut Context<Self>) {
        self.activate_post_detail_action(post_id, cx).detach();
    }

    pub fn activate_post_detail_action(&mut self, post_id: &str, cx: &mut Context<Self>) -> Task<()> {
        let post_id = post_id.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.activate_post_detail(&post_id).await })
    }

    pub fn refresh_post_detail_action(&mut self, post_id: &str, cx: &mut Context<Self>) -> Task<()> {
        let post_id = post_id.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.refresh_post_detail(&post_id).await })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_post_action(
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
        coauthor_ids: Vec<String>,
        tagged_users: Vec<PostTagInputDraft>,
        on_rejections: Option<PostActionRejectionsHandler>,
        on_progress: Option<PostActionProgressHandler>,
        on_settled: Option<PostActionDoneHandler>,
        on_success: Option<PostActionPostHandler>,
        on_error: Option<PostActionErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        let has_track =
            track_id.as_deref().is_some_and(|track_id| !trimming_whitespaces_and_newlines(track_id).is_empty());
        let has_achievement = achievement_id.unwrap_or(0) > 0;
        let has_media = !photos.is_empty() || video.is_some();
        if has_achievement && (has_media || has_track) {
            return Self::report_error(on_error, "achievement_cannot_contain_media", cx);
        }
        match content_kind {
            RisePostContentKind::Thread => {
                if has_media || has_track {
                    return Self::report_error(on_error, "thread_cannot_contain_media", cx);
                }
            }
            RisePostContentKind::Publication => {
                if !(has_media || has_track || has_achievement) {
                    return Self::report_error(on_error, "publication_requires_media", cx);
                }
            }
        }
        if let Some(video) = &video {
            if video.kind != MediaPickerItemKind::Video {
                return Self::report_error(on_error, "publication_requires_media", cx);
            }
            if let Some(payload) = &video.video_payload
                && payload.duration > ShortVideoLimits::MAX_DURATION_SECONDS as f64 + 0.05
            {
                let message = tr("create_post_short_too_long");
                PostServicesStore::shared(cx)
                    .update(cx, |services, cx| services.set_publish_error(Some(message.clone()), cx));
                return Self::report_error(on_error, &message, cx);
            }
        }
        let Some(user_id) = Self::wire_user_id(cx) else {
            return Self::report_error(on_error, "create_post_error_connection", cx);
        };
        let Some(account_epoch) = SaiAccountCacheScope::active_epoch()
            .filter(|epoch| SaiAccountCacheScope::is_current_account_epoch(Some(user_id.as_str()), Some(epoch.as_str())))
        else {
            return Self::report_error(on_error, "create_post_error_connection", cx);
        };

        let trimmed_title = trimming_whitespaces_and_newlines(title).to_owned();
        let trimmed_content = trimming_whitespaces_and_newlines(content).to_owned();
        let hashtags = PostHashtagParsing::merge(manual_hashtags, &format!("{trimmed_content} {trimmed_title}"));
        let author = {
            let auth = AuthServicesStore::shared(cx);
            let profile = auth.read(cx).profile();
            AuthorInfo {
                id: Some(user_id.clone()),
                name: profile.map(|profile| profile.name.clone()).unwrap_or_default(),
                tag: profile.map(|profile| profile.tag.clone()).unwrap_or_default(),
                more: AuthorInfoMore {
                    logo: profile
                        .and_then(|profile| profile.avatar_url.clone())
                        .or_else(|| profile.map(|profile| profile.more.logo.clone()))
                        .unwrap_or_default(),
                    ..AuthorInfoMore::default()
                },
                ..AuthorInfo::default()
            }
        };
        let audio_pre_baked = video
            .as_ref()
            .and_then(|video| video.video_payload.as_ref())
            .is_some_and(|payload| payload.audio_pre_baked);
        let draft = RisePostCreateDraft {
            content_kind,
            title: trimmed_title,
            content: trimmed_content,
            hashtags,
            can_comment,
            local_uploads: photos.into_iter().map(Self::local_upload).collect(),
            local_video: video.as_ref().map(Self::local_video_upload),
            video_position: if video.is_none() { None } else { video_position },
            requests_riff: is_riff,
            requests_publications: show_in_publications,
            achievement_id: if has_achievement { achievement_id } else { None },
            track_id,
            track: attached_track.clone(),
            track_start_milliseconds: track_start_ms,
            track_end_milliseconds: track_end_ms,
            track_plate: track_plate.as_ref().map(RisePostTrackPlateDraft::new),
            audio_pre_baked,
            author,
            count_visibility,
            coauthor_ids,
            tagged_users,
            map_journey_id,
            map_journey_show_heat,
        };
        let placement = draft.placement();
        let is_riff_only = placement.is_riff && !placement.show_in_publications;
        let lists_in_riffs = placement.is_riff;
        let counts_in_profile =
            PostServicesStore::counts_as_profile_publication(content_kind.wire(), placement.show_in_publications);
        PostServicesStore::shared(cx).update(cx, |services, cx| {
            services.begin_create_upload(is_riff_only, cx);
            if counts_in_profile {
                services.begin_own_publication_upload(&user_id, cx);
            }
        });

        let main = RiseEngineMainHop::shared(cx);
        let on_pending = Self::pending_short_handler(&main, &user_id, &account_epoch, lists_in_riffs);
        let on_progress = Self::upload_progress_handler(&main, Some((user_id.clone(), account_epoch.clone())), on_progress);
        let domain = RisePostActionExecutor::require_domain(cx);
        let mut background_lease = Some((
            begin_process_activity("riseonly.post.publish"),
            QuitDrain::lease("riseonly.post.publish", cx),
        ));

        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let mut publication_upload_open = counts_in_profile;
            let is_current = || SaiAccountCacheScope::is_current_account_epoch(Some(user_id.as_str()), Some(account_epoch.as_str()));
            let outcome = match domain.await {
                Ok(domain) if is_current() => Some(domain.create_post(draft, Some(on_pending), on_progress).await),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            };
            x597_mark(|| match &outcome {
                None => "x597 create_post skipped: account no longer current".to_owned(),
                Some(Ok(created)) => format!("x597 create_post ok id={}", created.post.id),
                Some(Err(error)) => format!("x597 create_post err: {error:#}"),
            });
            match outcome {
                None => {}
                Some(Ok(created)) => {
                    cx.update(|cx| Self::close_publication_upload(&mut publication_upload_open, &user_id, cx));
                    if is_current() {
                        cx.update(|cx| {
                            if lists_in_riffs {
                                let temp_id = created.post.presentation_id.clone().unwrap_or_default();
                                let post = created.post.clone();
                                ShortsServicesStore::shared(cx)
                                    .update(cx, |shorts, cx| shorts.settle_pending_short(&temp_id, post, cx));
                            }
                            PostServicesStore::shared(cx).update(cx, |services, cx| {
                                services.apply_created_audio_post(&created.post, attached_track.as_ref(), cx);
                                services.set_publish_error(None, cx);
                                if counts_in_profile {
                                    services.reconcile_own_publications_count(cx);
                                }
                            });
                            if let Some(on_success) = on_success {
                                on_success(created.post, cx);
                            }
                            if let Some(on_rejections) = on_rejections {
                                on_rejections(created.rejected, cx);
                            }
                        });
                    }
                }
                Some(Err(error)) => {
                    drop(background_lease.take());
                    Self::wait_until_application_active(cx).await;
                    cx.update(|cx| Self::close_publication_upload(&mut publication_upload_open, &user_id, cx));
                    if is_current() {
                        cx.update(|cx| {
                            let services = PostServicesStore::shared(cx);
                            if counts_in_profile {
                                services.update(cx, |services, cx| {
                                    services.adjust_own_publications_count(-1, &user_id, cx);
                                });
                            }
                            let message = RisePostPublishFailure::message(&error);
                            let explained =
                                RestrictionStore::shared(cx).update(cx, |restriction, cx| restriction.explains_error(&error, cx));
                            let shown = if explained { None } else { Some(message.clone()) };
                            services.update(cx, |services, cx| services.set_publish_error(shown, cx));
                            if let Some(on_error) = on_error {
                                on_error(message, cx);
                            }
                        });
                    }
                }
            }
            cx.update(|cx| {
                Self::close_publication_upload(&mut publication_upload_open, &user_id, cx);
                if is_current() {
                    PostServicesStore::shared(cx)
                        .update(cx, |services, cx| services.finish_create_upload(is_riff_only, cx));
                }
            });
            drop(background_lease);
            if let Some(on_settled) = on_settled {
                cx.update(on_settled);
            }
        })
        .detach();
    }

    pub fn set_post_like_action(
        &mut self,
        post_id: &str,
        liked: bool,
        seed: Option<GetPostFeedResponse>,
        on_applied: Option<PostActionAppliedHandler>,
        on_refused: Option<RisePostEngagementRefusalHandler>,
        cx: &mut Context<Self>,
    ) {
        let post_id = post_id.to_owned();
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let applied = match domain.await {
                Ok(domain) => match domain.set_post_like(&post_id, liked, seed, on_refused).await {
                    Ok(()) => {
                        x251_mark(|| format!("x251 set_post_like ok post={post_id} liked={liked}"));
                        true
                    }
                    Err(error) => {
                        x251_mark(|| format!("x251 set_post_like err post={post_id}: {error:#}"));
                        false
                    }
                },
                Err(error) => {
                    x251_mark(|| format!("x251 require_domain err: {error:#}"));
                    false
                }
            };
            if let Some(on_applied) = on_applied {
                cx.update(|cx| on_applied(applied, cx));
            }
        })
        .detach();
    }

    pub fn set_post_favorite_action(
        &mut self,
        post_id: &str,
        favorited: bool,
        seed: Option<GetPostFeedResponse>,
        on_applied: Option<PostActionAppliedHandler>,
        on_refused: Option<RisePostEngagementRefusalHandler>,
        cx: &mut Context<Self>,
    ) {
        let post_id = post_id.to_owned();
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let applied = match domain.await {
                Ok(domain) => domain.set_post_favorite(&post_id, favorited, seed, on_refused).await.is_ok(),
                Err(_) => false,
            };
            if let Some(on_applied) = on_applied {
                cx.update(|cx| on_applied(applied, cx));
            }
        })
        .detach();
    }

    pub fn set_author_relationship_action(
        &mut self,
        author_id: &str,
        is_subscribed: bool,
        is_subscriber: bool,
        is_friend: bool,
        cx: &mut Context<Self>,
    ) {
        if author_id.is_empty() {
            return;
        }
        let author_id = author_id.to_owned();
        let previous = self.author_relationship_write.take();
        let write = cx.spawn(async move |_, cx: &mut AsyncApp| {
            if let Some(previous) = previous {
                previous.await;
            }
            let Ok(domain) = cx.update(RisePostActionExecutor::require_domain).await else {
                return;
            };
            let _ = domain.set_author_relationship(&author_id, is_subscribed, is_subscriber, is_friend).await;
        });
        self.author_relationship_write = Some(write.shared());
    }

    pub fn adjust_post_comments_count_action(&mut self, post_id: &str, delta: i64, cx: &mut Context<Self>) {
        let post_id = post_id.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.adjust_post_comments_count(&post_id, delta).await })
            .detach();
    }

    pub fn get_post_stats_action(&mut self, post_id: &str, days: i64, force: bool, cx: &mut Context<Self>) {
        let Some(post_id_int) = int_from_string(post_id).filter(|value| *value > 0) else {
            return;
        };
        let post_id = post_id.to_owned();
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => domain.load_stats(post_id_int, days, force).await,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                cx.update(|cx| {
                    Self::log_network("RisePost", format!("get_post_stats error postId={post_id}: {error}"), cx);
                });
            }
        })
        .detach();
    }

    pub fn close_post_stats_action(&mut self, post_id: &str, cx: &mut Context<Self>) {
        let Some(post_id_int) = int_from_string(post_id).filter(|value| *value > 0) else {
            return;
        };
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Some(domain) = cx.update(|cx| RisePostDomainRegistry::shared(cx).read(cx).active().cloned()) else {
                return;
            };
            domain.close_stats(post_id_int).await;
        })
        .detach();
    }

    pub fn record_post_view_action(&mut self, intent: RisePostVisibilityIntent, cx: &mut Context<Self>) -> Task<bool> {
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => domain.record_view(&intent).await,
                Err(error) => Err(error),
            };
            match result {
                Ok(handled) => handled,
                Err(error) if error.is::<CancellationError>() => false,
                Err(error) => {
                    let text = format!("record_post_views error postId={}: {error}", intent.post_id);
                    cx.update(|cx| Self::log_network("RisePost", text, cx));
                    false
                }
            }
        })
    }

    pub fn create_repost_action(
        &mut self,
        post: GetPostFeedResponse,
        content: Option<&str>,
        on_success: Option<PostActionDoneHandler>,
        on_error: Option<PostActionErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        if Self::wire_user_id(cx).is_none() {
            return;
        }
        let trimmed = Self::repost_content(content.unwrap_or(""));
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let content = if trimmed.is_empty() { None } else { Some(trimmed) };
            let result = match domain.await {
                Ok(domain) => domain.set_repost(post, true, content).await,
                Err(error) => Err(error),
            };
            cx.update(|cx| Self::settle(result, on_success, on_error, cx));
        })
        .detach();
    }

    pub fn delete_repost_action(
        &mut self,
        post: GetPostFeedResponse,
        owner_id: Option<&str>,
        on_success: Option<PostActionDoneHandler>,
        on_error: Option<PostActionErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        let viewer_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let foreign_owner = owner_id.and_then(|owner| {
            let trimmed = trimming_whitespaces_and_newlines(owner);
            if trimmed.is_empty() || Some(trimmed) == viewer_id.as_deref() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        });
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => match foreign_owner {
                    Some(foreign_owner) => domain.remove_repost(&foreign_owner, &post).await,
                    None => domain.set_repost(post, false, None).await,
                },
                Err(error) => Err(error),
            };
            cx.update(|cx| Self::settle(result, on_success, on_error, cx));
        })
        .detach();
    }

    pub fn update_repost_opinion_action(
        &mut self,
        post: GetPostFeedResponse,
        content: &str,
        entities: Option<String>,
        on_success: Option<PostActionDoneHandler>,
        on_error: Option<PostActionErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        if Self::wire_user_id(cx).is_none() {
            return Self::report_error(on_error, "missing_user_id", cx);
        }
        if !int_from_string(&post.id).is_some_and(|value| value > 0) {
            return Self::report_error(on_error, "invalid_post_id", cx);
        }
        let trimmed = Self::repost_content(content);
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let entities = if trimmed.is_empty() { None } else { entities };
            let result = match domain.await {
                Ok(domain) => domain.update_repost_opinion(post, trimmed, entities).await,
                Err(error) => Err(error),
            };
            cx.update(|cx| Self::settle(result, on_success, on_error, cx));
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_post_action(
        &mut self,
        post: GetPostFeedResponse,
        content_kind: RisePostContentKind,
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
        on_rejections: Option<PostActionRejectionsHandler>,
        on_success: Option<PostActionPostHandler>,
        on_error: Option<PostActionErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        if Self::wire_user_id(cx).is_none() {
            return Self::report_error(on_error, "missing_user_id", cx);
        }
        let Some(post_id_int) = int_from_string(&post.id).filter(|value| *value > 0) else {
            return Self::report_error(on_error, "invalid_post_id", cx);
        };

        let media_edit = media
            .map(|items| RisePostMediaEdit::new(items.into_iter().map(Self::edit_media_item).collect()))
            .filter(|edit| !edit.keeps_media(&post));
        let uploads_media = media_edit.as_ref().is_some_and(RisePostMediaEdit::has_local_uploads);
        let trimmed_title = trimming_whitespaces_and_newlines(title).to_owned();
        let trimmed_content = trimming_whitespaces_and_newlines(content).to_owned();
        let hashtags = PostHashtagParsing::merge(manual_hashtags, &format!("{trimmed_content} {trimmed_title}"));
        let is_publication = content_kind == RisePostContentKind::Publication;
        let draft = RisePostUpdateDraft {
            post_id: post_id_int.to_string(),
            count_visibility: count_visibility.unwrap_or(post.count_visibility),
            current_post: post,
            content_kind,
            title: trimmed_title,
            content: trimmed_content,
            hashtags,
            can_comment,
            track_id,
            track,
            track_start_milliseconds: track_start_ms,
            track_end_milliseconds: track_end_ms,
            track_plate: track_plate.as_ref().map(RisePostTrackPlateDraft::new),
            is_riff: if is_publication { is_riff } else { None },
            show_in_publications: if is_publication { show_in_publications } else { None },
            media: media_edit,
            tagged_users,
        };
        if uploads_media {
            PostServicesStore::shared(cx).update(cx, |services, cx| services.begin_create_upload(false, cx));
        }
        let on_progress: RisePostProgressHandler = if uploads_media {
            Self::upload_progress_handler(&RiseEngineMainHop::shared(cx), None, None)
        } else {
            Arc::new(|_: f64| async {}.boxed())
        };
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => domain.update_post(draft, Some(on_progress)).await,
                Err(error) => Err(error),
            };
            cx.update(|cx| {
                match result {
                    Ok(outcome) => {
                        if let Some(on_success) = on_success {
                            on_success(outcome.post, cx);
                        }
                        if let Some(on_rejections) = on_rejections {
                            on_rejections(outcome.rejected, cx);
                        }
                    }
                    Err(error) => {
                        RestrictionStore::shared(cx).update(cx, |restriction, cx| restriction.explains_error(&error, cx));
                        if let Some(on_error) = on_error {
                            on_error(RisePostPublishFailure::message(&error), cx);
                        }
                    }
                }
                if uploads_media {
                    PostServicesStore::shared(cx).update(cx, |services, cx| services.finish_create_upload(false, cx));
                }
            });
        })
        .detach();
    }

    fn edit_media_item(item: PostEditMediaItem) -> RisePostEditMediaItem {
        match item {
            PostEditMediaItem::Existing(PostMediaSlide::Image { url, size, .. }) => {
                RisePostEditMediaItem::RemoteImage { url, size }
            }
            PostEditMediaItem::Existing(PostMediaSlide::Video { url, thumbnail, size }) => {
                RisePostEditMediaItem::RemoteVideo { url, thumbnail, size }
            }
            PostEditMediaItem::Photo(file) => RisePostEditMediaItem::LocalImage(Self::local_upload(file)),
            PostEditMediaItem::Video(picked) => RisePostEditMediaItem::LocalVideo {
                upload: Self::local_video_upload(&picked),
                audio_pre_baked: picked.video_payload.as_ref().is_some_and(|payload| payload.audio_pre_baked),
            },
        }
    }

    pub fn record_share_action(&mut self, post_id: &str, channel: &str, cx: &mut Context<Self>) {
        let Some(post_id_int) = int_from_string(post_id).filter(|value| *value > 0) else {
            return;
        };
        let (Some(account_user_id), Some(account_epoch)) =
            (SaiAccountCacheScope::active_user_id(), SaiAccountCacheScope::active_epoch())
        else {
            return;
        };
        let (post_id, channel) = (post_id.to_owned(), channel.to_owned());
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let is_current =
                || SaiAccountCacheScope::is_current_account_epoch(Some(account_user_id.as_str()), Some(account_epoch.as_str()));
            if !is_current() {
                return;
            }
            let Ok(domain) = cx.update(RisePostActionExecutor::require_domain).await else {
                return;
            };
            if domain.account_id != account_user_id || domain.account_epoch != account_epoch || !is_current() {
                return;
            }
            let Ok(receipt) = domain.record_share(post_id_int, &channel).await else {
                return;
            };
            if !is_current() {
                return;
            }
            cx.update(|cx| {
                PostServicesStore::shared(cx)
                    .update(cx, |services, cx| services.apply_share_count(&post_id, receipt.share_count, cx));
            });
        })
        .detach();
    }

    pub fn record_link_open_action(&mut self, post_id: &str, cx: &mut Context<Self>) {
        let Some(post_id_int) = int_from_string(post_id).filter(|value| *value > 0) else {
            return;
        };
        let post_id = post_id.to_owned();
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            let Ok(receipt) = domain.record_link_open(post_id_int).await else {
                return;
            };
            cx.update(|cx| {
                PostServicesStore::shared(cx).update(cx, |services, cx| {
                    services.apply_link_open_count(&post_id, receipt.link_open_count, cx);
                });
            });
        })
        .detach();
    }

    pub fn delete_post_action(
        &mut self,
        post_id: &str,
        on_success: Option<PostActionDoneHandler>,
        on_error: Option<PostActionErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        if Self::wire_user_id(cx).is_none() {
            return Self::report_error(on_error, "missing_user_id", cx);
        }
        let Some(post_id_int) = int_from_string(post_id).filter(|value| *value > 0) else {
            return Self::report_error(on_error, "invalid_post_id", cx);
        };
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => domain.delete_post(post_id_int).await,
                Err(error) => Err(error),
            };
            cx.update(|cx| Self::settle(result, on_success, on_error, cx));
        })
        .detach();
    }

    fn on_domain<Operation, Work>(cx: &mut Context<Self>, operation: Operation) -> Task<()>
    where
        Operation: FnOnce(Arc<RisePostDomain>) -> Work + 'static,
        Work: Future<Output = anyhow::Result<()>> + 'static,
    {
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            let _ = operation(domain).await;
        })
    }

    fn wire_user_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .filter(|user_id| !user_id.is_empty())
    }

    fn report_error(on_error: Option<PostActionErrorHandler>, message: &str, cx: &mut App) {
        if let Some(on_error) = on_error {
            on_error(message.to_owned(), cx);
        }
    }

    fn settle(
        result: anyhow::Result<()>,
        on_success: Option<PostActionDoneHandler>,
        on_error: Option<PostActionErrorHandler>,
        cx: &mut App,
    ) {
        match result {
            Ok(()) => {
                if let Some(on_success) = on_success {
                    on_success(cx);
                }
            }
            Err(error) => {
                if let Some(on_error) = on_error {
                    on_error(CanonErrors::refusal_text(&error), cx);
                }
            }
        }
    }

    fn repost_content(raw: &str) -> String {
        PostFieldRules::repost_content()
            .schema
            .as_ref()
            .map_or_else(|| raw.to_owned(), |schema| schema.normalized_clamped(raw))
    }

    fn local_upload(file: SaiWsOptimisticLocalFile) -> RisePostLocalUpload {
        RisePostLocalUpload { url: file.url, filename: file.filename, content_type: file.content_type }
    }

    fn local_video_upload(video: &MediaPickerItem) -> RisePostLocalUpload {
        RisePostLocalUpload {
            url: video.url.clone(),
            filename: video.suggested_file_name.clone(),
            content_type: RiseUploadFormat::VIDEO_MP4.mime.to_owned(),
        }
    }

    fn close_publication_upload(open: &mut bool, user_id: &str, cx: &mut App) {
        if !*open {
            return;
        }
        *open = false;
        PostServicesStore::shared(cx).update(cx, |services, cx| services.end_own_publication_upload(user_id, cx));
    }

    fn pending_short_handler(
        main: &RiseEngineMainHop,
        user_id: &str,
        account_epoch: &str,
        lists_in_riffs: bool,
    ) -> RisePostPendingHandler {
        let (main, user_id, account_epoch) = (main.clone(), user_id.to_owned(), account_epoch.to_owned());
        Arc::new(move |temp_id: String, pending: Option<GetPostFeedResponse>| {
            let (main, user_id, account_epoch) = (main.clone(), user_id.clone(), account_epoch.clone());
            async move {
                if !lists_in_riffs {
                    return;
                }
                let _ = main
                    .run(move |cx: AsyncApp| {
                        async move {
                            cx.update(|cx| {
                                if !SaiAccountCacheScope::is_current_account_epoch(Some(user_id.as_str()), Some(account_epoch.as_str())) {
                                    return;
                                }
                                ShortsServicesStore::shared(cx).update(cx, |shorts, cx| match pending {
                                    Some(pending) => shorts.upsert_pending_short(pending, cx),
                                    None => shorts.remove_pending_short(&temp_id, cx),
                                });
                            });
                            Ok(())
                        }
                        .boxed_local()
                    })
                    .await;
            }
            .boxed()
        })
    }

    fn upload_progress_handler(
        main: &RiseEngineMainHop,
        account: Option<(String, String)>,
        on_progress: Option<PostActionProgressHandler>,
    ) -> RisePostProgressHandler {
        let main = main.clone();
        Arc::new(move |progress: f64| {
            let (main, account, on_progress) = (main.clone(), account.clone(), on_progress.clone());
            async move {
                let _ = main
                    .run(move |cx: AsyncApp| {
                        async move {
                            cx.update(|cx| {
                                if let Some((user_id, account_epoch)) = &account
                                    && !SaiAccountCacheScope::is_current_account_epoch(Some(user_id.as_str()), Some(account_epoch.as_str()))
                                {
                                    return;
                                }
                                PostServicesStore::shared(cx)
                                    .update(cx, |services, cx| services.set_upload_progress(progress * 100.0, cx));
                                if let Some(on_progress) = &on_progress {
                                    on_progress(progress, cx);
                                }
                            });
                            Ok(())
                        }
                        .boxed_local()
                    })
                    .await;
            }
            .boxed()
        })
    }

    async fn wait_until_application_active(cx: &mut AsyncApp) {
        if crate::app::dev_drive::ENABLED {
            return;
        }
        let (signals, mut activations) = mpsc::unbounded::<()>();
        let subscription =
            RiseAppEvents::default_center().observe(UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, move |_| {
                let _ = signals.unbounded_send(());
            });
        if cx.update(|cx| RiseAppLifecycle::scene_phase(cx)) != ScenePhase::Active {
            while activations.next().await.is_some() {
                if cx.update(|cx| RiseAppLifecycle::scene_phase(cx)) == ScenePhase::Active {
                    break;
                }
            }
        }
        drop(subscription);
    }

    fn log_network(name: &str, text: String, cx: &mut App) {
        Sai::log(name, move || text, SaiLogKind::Network, cx);
    }
}

fn x597_mark(stage: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X597", "{}", stage());
}

fn x251_mark(stage: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X251", "{}", stage());
}
