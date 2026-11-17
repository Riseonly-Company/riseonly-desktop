use rise_engine::desktop_runtime::rise_runtime_timer::RiseRuntimeTimer;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_catalog::PostLinkPreviewStatus;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseEngine};
use tokio::task::JoinHandle;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{SaiActiveScreen, SaiNavigationBridge};
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::engine::rise_post_content_repository::{
    RisePostContentRepository, RisePostContentRepositoryCollaborationOperation,
    RisePostContentRepositoryLoadIntent, RisePostEngagementRefusalHandler,
};
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostContentKind, RisePostContentPresentationSnapshot, RisePostCreateDraft, RisePostCreateOutcome,
    RisePostFeedKind, RisePostPageScope, RisePostPresentationSnapshot, RisePostProfileKind,
    RisePostSearchKind, RisePostUpdateDraft, RisePostUpdateOutcome, RisePostVisibilityIntent,
};
use crate::modules::post::engine::rise_post_presentation::RisePostPresentation;
use crate::modules::post::engine::rise_post_repository::{RisePostRepository, RisePostRepositoryError};
use crate::modules::post::stores::post_actions::post_types::{
    GetPostFeedResponse, PostCollaboratorKind, PostCollaboratorRejection, PostCollaboratorStatus,
    PostCollaboratorUser, PostTagInputDraft,
};
use crate::modules::post::stores::post_actions::post_wire_types::{
    RecordPostLinkOpenGatewayDTO, RecordPostShareGatewayDTO,
};
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::sparks::engine::rise_sparks_engine_models::{RiseStoredSparksSendResult, RiseStoredSparksTop};

/// upload progress handler of a create or an update
pub type RisePostProgressHandler = Arc<dyn Fn(f64) -> BoxFuture<'static, ()> + Send + Sync>;

/// optimistic row of a create keyed by its temp id
/// none once it is rolled back
pub type RisePostPendingHandler =
    Arc<dyn Fn(String, Option<GetPostFeedResponse>) -> BoxFuture<'static, ()> + Send + Sync>;

/// generation and access counters of the content owner
#[derive(Clone, Copy)]
struct RisePostDomainContentOwnerVersion {
    generation: u64,
    access: u64,
}

struct RisePostDomainState {
    active_stats_post_ids: HashSet<i64>,
    recent_content_scopes: HashMap<RisePostPageScope, u64>,
    content_scope_owners: HashMap<Uuid, RisePostPageScope>,
    content_owner_versions: HashMap<Uuid, RisePostDomainContentOwnerVersion>,
    content_owner_clock: u64,
    content_scope_clock: u64,
    content_observation_tasks: Vec<JoinHandle<()>>,
    pending_preview_tasks: HashMap<RisePostPageScope, oneshot::Sender<()>>,
    pending_preview_generations: HashMap<RisePostPageScope, u64>,
    exhausted_pending_preview_scopes: HashSet<RisePostPageScope>,
    pending_preview_clock: u64,
    feed_warmup_task: Option<JoinHandle<()>>,
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
}

/// rollback of an optimistic mutation must also run when the task is cancelled
/// a dropped future never reaches code after its await so the rollback is held here and starts on drop
struct RisePostDomainRollback {
    rollback: Option<BoxFuture<'static, ()>>,
}

impl RisePostDomainRollback {
    fn new(rollback: BoxFuture<'static, ()>) -> Self {
        Self { rollback: Some(rollback) }
    }

    fn disarm(mut self) {
        self.rollback = None;
    }

    async fn run(mut self) {
        if let Some(rollback) = self.rollback.take() {
            rollback.await;
        }
    }
}

impl Drop for RisePostDomainRollback {
    fn drop(&mut self) {
        if let Some(rollback) = self.rollback.take() {
            drop(RiseAccountRuntime::current().spawn(rollback));
        }
    }
}

pub struct RisePostDomain {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub engine: Arc<RiseEngine>,
    pub repository: Arc<RisePostRepository>,
    pub content_repository: Arc<RisePostContentRepository>,
    this: Weak<RisePostDomain>,
    state: RiseIsolated<RisePostDomainState>,
}

impl RisePostDomain {
    pub const RECONNECT_REFRESH_SCREEN_IDS: [&'static str; 10] = [
        "Posts",
        "Shorts",
        "PostsInteresting",
        "PostDetail",
        "ShortViewer",
        "PostStats",
        "PostViewHistory",
        "UserPage",
        "Profile",
        "Achievements",
    ];

    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        let account_id = engine.account.account_id.clone();
        let account_epoch = engine.account.epoch.clone();
        let session_id = Uuid::new_v4();
        let repository = RisePostRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session_id,
        );
        let content_repository = RisePostContentRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session_id,
        );
        Arc::new_cyclic(|this| Self {
            account_id,
            account_epoch,
            session_id,
            engine,
            repository,
            content_repository,
            this: this.clone(),
            state: RiseIsolated::new(RisePostDomainState {
                active_stats_post_ids: HashSet::new(),
                recent_content_scopes: HashMap::new(),
                content_scope_owners: HashMap::new(),
                content_owner_versions: HashMap::new(),
                content_owner_clock: 0,
                content_scope_clock: 0,
                content_observation_tasks: Vec::new(),
                pending_preview_tasks: HashMap::new(),
                pending_preview_generations: HashMap::new(),
                exhausted_pending_preview_scopes: HashSet::new(),
                pending_preview_clock: 0,
                feed_warmup_task: None,
                is_prepared: false,
                is_active: false,
                is_shutdown: false,
            }),
        })
    }

    pub async fn prepare(&self, context: &RiseAppEngineBootstrapContext) -> anyhow::Result<()> {
        let (is_shutdown, is_prepared) = self.state.with(|state| (state.is_shutdown, state.is_prepared));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if is_prepared {
            return Ok(());
        }
        context.register_push_handler(self.content_repository.link_preview_push_handler()).await?;
        context.register_push_handler(self.content_repository.post_created_push_handler()).await?;
        context.register_push_handler(self.content_repository.post_deleted_push_handler()).await?;
        context.register_push_handler(self.content_repository.post_updated_push_handler()).await?;
        context.register_push_handler(self.content_repository.media_ready_push_handler()).await?;
        context
            .register_push_handler(self.content_repository.collaborators_changed_push_handler())
            .await?;
        context.register_push_handler(self.content_repository.sparks_push_handler()).await?;
        self.state.with(|state| state.is_prepared = true);
        Ok(())
    }

    pub async fn activate(&self) -> anyhow::Result<()> {
        let (is_shutdown, is_prepared, is_active) =
            self.state.with(|state| (state.is_shutdown, state.is_prepared, state.is_active));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if !is_prepared {
            return Err(RisePostRepositoryError::MissingActiveSession.into());
        }
        if is_active {
            return Ok(());
        }
        let is_current = SaiAccountCacheScope::is_current_account_epoch(
            Some(self.account_id.as_str()),
            Some(self.account_epoch.as_str()),
        );
        if !is_current {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }

        let seed = self.repository.activate().await;
        let content_seed = self.content_repository.activate().await;
        let activated = self.state.with(|state| {
            if state.is_shutdown {
                return false;
            }
            state.is_active = true;
            true
        });
        if !activated {
            return Err(CancellationError.into());
        }
        let Some(domain) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        if let Some(main) = RiseEngineMainHop::installed() {
            let account_id = self.account_id.clone();
            let account_epoch = self.account_epoch.clone();
            let session_id = self.session_id;
            let _ = main
                .run(move |cx: AsyncApp| {
                    async move {
                        cx.update(|cx| {
                            RisePostPresentation::shared(cx).update(cx, |presentation, cx| {
                                presentation.activate(&account_id, &account_epoch, session_id, cx);
                            });
                            RisePostContentPresentation::shared(cx).update(cx, |presentation, cx| {
                                presentation.activate(&account_id, &account_epoch, session_id, cx);
                            });
                            RisePostPresentation::shared(cx).update(cx, |presentation, cx| {
                                let _ = presentation.publish(seed, cx);
                            });
                            RisePostContentPresentation::shared(cx).update(cx, |presentation, cx| {
                                let _ = presentation.publish(content_seed, cx);
                            });
                            RisePostDomainRegistry::shared(cx)
                                .update(cx, |registry, cx| registry.install(domain, cx));
                        });
                        Ok(())
                    }
                    .boxed_local()
                })
                .await;
        }
        self.start_content_observations();
        Ok(())
    }

    /// activation already published the persisted first page
    /// revalidate it after the first live socket
    /// a view mounting mid flight shares the in flight scopes
    pub async fn did_activate_ownership(&self) {
        let this = self.this.clone();
        self.state.with(|state| {
            if !state.is_active || state.is_shutdown || state.feed_warmup_task.is_some() {
                return;
            }
            state.feed_warmup_task = Some(RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = this.upgrade() {
                    this.load_content(
                        RisePostPageScope::Feed(RisePostFeedKind::All),
                        RisePostContentRepositoryLoadIntent::Activate,
                    )
                    .await;
                }
            }));
        });
    }

    pub async fn socket_connected(&self) {
        if !self.is_live() {
            return;
        }
        let Some(screen) = Self::current_screen().await else {
            return;
        };
        if !Self::RECONNECT_REFRESH_SCREEN_IDS.iter().any(|id| *id == screen.id) {
            return;
        }
        let post_ids: Vec<i64> =
            self.state.with(|state| state.active_stats_post_ids.iter().take(8).copied().collect());
        let _ = futures::future::join_all(
            post_ids.into_iter().map(|post_id| self.load_stats(post_id, 30, true)),
        )
        .await;
        let scopes: Vec<RisePostPageScope> = self.state.with(|state| {
            let active: HashSet<&RisePostPageScope> = state.content_scope_owners.values().collect();
            let mut scopes: Vec<RisePostPageScope> = active.into_iter().cloned().collect();
            scopes.sort_by(|lhs, rhs| {
                let lhs = state.recent_content_scopes.get(lhs).copied().unwrap_or(0);
                let rhs = state.recent_content_scopes.get(rhs).copied().unwrap_or(0);
                rhs.cmp(&lhs)
            });
            scopes.truncate(6);
            scopes
        });
        for scope in scopes {
            self.state.with(|state| {
                state.exhausted_pending_preview_scopes.remove(&scope);
            });
            let intent = Self::reconnect_intent(&scope);
            self.load_content(scope, intent).await;
        }
    }

    pub fn reconnect_intent(scope: &RisePostPageScope) -> RisePostContentRepositoryLoadIntent {
        match scope {
            RisePostPageScope::Feed(_) | RisePostPageScope::Profile { .. } | RisePostPageScope::Search { .. } => {
                RisePostContentRepositoryLoadIntent::Activate
            }
            RisePostPageScope::Shorts | RisePostPageScope::Detail { .. } => {
                RisePostContentRepositoryLoadIntent::Refresh
            }
        }
    }

    pub async fn activate_feed(&self, kind: RisePostFeedKind) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        self.load_content(RisePostPageScope::Feed(kind), RisePostContentRepositoryLoadIntent::Activate)
            .await;
        Ok(())
    }

    pub async fn refresh_feed(&self, kind: RisePostFeedKind) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        let scope = RisePostPageScope::Feed(kind);
        self.forget_exhausted_preview_scope(&scope);
        self.load_content(scope, RisePostContentRepositoryLoadIntent::Refresh).await;
        Ok(())
    }

    pub async fn load_more_feed(&self, kind: RisePostFeedKind) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        self.load_content(RisePostPageScope::Feed(kind), RisePostContentRepositoryLoadIntent::LoadMore)
            .await;
        Ok(())
    }

    pub async fn load_shorts(&self, intent: RisePostContentRepositoryLoadIntent) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        self.load_content(RisePostPageScope::Shorts, intent).await;
        Ok(())
    }

    pub async fn activate_profile(
        &self,
        user_id: &str,
        tag: &str,
        kind: RisePostProfileKind,
    ) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        if user_id.is_empty() || tag.is_empty() {
            return Ok(());
        }
        let scope = RisePostPageScope::Profile { user_id: user_id.to_owned(), tag: tag.to_owned(), kind };
        self.load_content(scope, RisePostContentRepositoryLoadIntent::Activate).await;
        Ok(())
    }

    pub async fn refresh_profile(
        &self,
        user_id: &str,
        tag: &str,
        kind: RisePostProfileKind,
    ) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        if user_id.is_empty() || tag.is_empty() {
            return Ok(());
        }
        let scope = RisePostPageScope::Profile { user_id: user_id.to_owned(), tag: tag.to_owned(), kind };
        self.forget_exhausted_preview_scope(&scope);
        self.load_content(scope, RisePostContentRepositoryLoadIntent::Refresh).await;
        Ok(())
    }

    pub async fn load_more_profile(
        &self,
        user_id: &str,
        tag: &str,
        kind: RisePostProfileKind,
    ) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        if user_id.is_empty() || tag.is_empty() {
            return Ok(());
        }
        let scope = RisePostPageScope::Profile { user_id: user_id.to_owned(), tag: tag.to_owned(), kind };
        self.load_content(scope, RisePostContentRepositoryLoadIntent::LoadMore).await;
        Ok(())
    }

    pub async fn activate_post_search(&self, query: &str, kind: RisePostSearchKind) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        if RisePostPageScope::normalized_query(query).is_empty() {
            return Ok(());
        }
        let scope = RisePostPageScope::Search { query: query.to_owned(), kind };
        self.load_content(scope, RisePostContentRepositoryLoadIntent::Activate).await;
        Ok(())
    }

    pub async fn refresh_post_search(&self, query: &str, kind: RisePostSearchKind) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        if RisePostPageScope::normalized_query(query).is_empty() {
            return Ok(());
        }
        let scope = RisePostPageScope::Search { query: query.to_owned(), kind };
        self.forget_exhausted_preview_scope(&scope);
        self.load_content(scope, RisePostContentRepositoryLoadIntent::Refresh).await;
        Ok(())
    }

    pub async fn load_more_post_search(&self, query: &str, kind: RisePostSearchKind) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        if RisePostPageScope::normalized_query(query).is_empty() {
            return Ok(());
        }
        let scope = RisePostPageScope::Search { query: query.to_owned(), kind };
        self.load_content(scope, RisePostContentRepositoryLoadIntent::LoadMore).await;
        Ok(())
    }

    pub async fn activate_post_detail(&self, post_id: &str) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        if !int_from_string(post_id).is_some_and(|value| value > 0) {
            return Ok(());
        }
        let scope = RisePostPageScope::Detail { post_id: post_id.to_owned() };
        self.load_content(scope, RisePostContentRepositoryLoadIntent::Activate).await;
        Ok(())
    }

    pub async fn refresh_post_detail(&self, post_id: &str) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        if !int_from_string(post_id).is_some_and(|value| value > 0) {
            return Ok(());
        }
        let scope = RisePostPageScope::Detail { post_id: post_id.to_owned() };
        self.load_content(scope, RisePostContentRepositoryLoadIntent::Refresh).await;
        Ok(())
    }

    pub async fn observe_content(
        &self,
        scope: RisePostPageScope,
        owner_id: Uuid,
        generation: u64,
    ) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        let accepted = self.state.with(|state| {
            let current = state.content_owner_versions.get(&owner_id).map_or(0, |version| version.generation);
            if generation <= current {
                return false;
            }
            state.content_owner_clock = state.content_owner_clock.wrapping_add(1);
            let access = state.content_owner_clock;
            state
                .content_owner_versions
                .insert(owner_id, RisePostDomainContentOwnerVersion { generation, access });
            state.content_scope_owners.insert(owner_id, scope.clone());
            Self::trim_content_owner_versions(state);
            true
        });
        if !accepted {
            return Ok(());
        }
        self.load_content(scope, RisePostContentRepositoryLoadIntent::Activate).await;
        Ok(())
    }

    pub async fn stop_observing_content(&self, scope: &RisePostPageScope, owner_id: Uuid, generation: u64) {
        self.state.with(|state| {
            let current = state.content_owner_versions.get(&owner_id).map_or(0, |version| version.generation);
            if generation <= current {
                return;
            }
            state.content_owner_clock = state.content_owner_clock.wrapping_add(1);
            let access = state.content_owner_clock;
            state
                .content_owner_versions
                .insert(owner_id, RisePostDomainContentOwnerVersion { generation, access });
            if state.content_scope_owners.get(&owner_id) == Some(scope) {
                state.content_scope_owners.remove(&owner_id);
            }
            if !state.content_scope_owners.values().any(|owned| owned == scope) {
                Self::cancel_pending_preview_revalidation(state, scope);
            }
            Self::trim_content_owner_versions(state);
        });
    }

    /// on_pending mirrors the optimistic row for surfaces outside the engine
    /// temp row then each progress step then none once rolled back
    /// success keeps the temp id in presentation_id
    pub async fn create_post(
        &self,
        draft: RisePostCreateDraft,
        on_pending: Option<RisePostPendingHandler>,
        on_progress: RisePostProgressHandler,
    ) -> anyhow::Result<RisePostCreateOutcome> {
        self.validate_active(None, None).await?;
        let Some(this) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let ticket = Arc::new(self.content_repository.begin_create(draft).await?);
        let rollback = RisePostDomainRollback::new({
            let this = this.clone();
            let ticket = ticket.clone();
            let on_pending = on_pending.clone();
            async move {
                let rollback = this.content_repository.rollback_create(&ticket).await;
                this.publish_content(rollback).await;
                if let Some(on_pending) = on_pending {
                    on_pending(ticket.temp_id.clone(), None).await;
                }
            }
            .boxed()
        });
        self.publish_content(ticket.snapshot.clone()).await;
        if let Some(on_pending) = &on_pending {
            on_pending(ticket.temp_id.clone(), Some(ticket.post.clone())).await;
        }
        let weak = self.this.clone();
        let progress_ticket = ticket.clone();
        let progress_pending = on_pending.clone();
        let progress: RisePostProgressHandler = Arc::new(move |progress: f64| {
            let weak = weak.clone();
            let ticket = progress_ticket.clone();
            let on_pending = progress_pending.clone();
            let on_progress = on_progress.clone();
            async move {
                let Some(this) = weak.upgrade() else {
                    return;
                };
                if let Some((snapshot, post)) =
                    this.content_repository.update_create_progress(&ticket, progress).await
                {
                    this.publish_content(snapshot).await;
                    if let Some(on_pending) = on_pending {
                        on_pending(ticket.temp_id.clone(), Some(post)).await;
                    }
                }
                on_progress(progress).await;
            }
            .boxed()
        });
        match self.content_repository.finish_create(&ticket, progress).await {
            Ok((post, rejected, snapshot)) => {
                rollback.disarm();
                self.publish_content(snapshot).await;
                Ok(RisePostCreateOutcome { post, rejected })
            }
            Err(error) => {
                rollback.run().await;
                Err(error)
            }
        }
    }

    pub async fn update_post(
        &self,
        draft: RisePostUpdateDraft,
        on_progress: Option<RisePostProgressHandler>,
    ) -> anyhow::Result<RisePostUpdateOutcome> {
        self.validate_active(None, None).await?;
        let Some(this) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let ticket = Arc::new(self.content_repository.begin_update(draft).await?);
        let rollback = RisePostDomainRollback::new({
            let ticket = ticket.clone();
            async move {
                let rollback = this.content_repository.rollback_update(&ticket).await;
                this.publish_content(rollback).await;
            }
            .boxed()
        });
        self.publish_content(ticket.snapshot.clone()).await;
        let on_progress: RisePostProgressHandler = match on_progress {
            Some(on_progress) => on_progress,
            None => Arc::new(|_: f64| async {}.boxed()),
        };
        match self.content_repository.finish_update(&ticket, on_progress).await {
            Ok((post, rejected, snapshot, placement_scopes)) => {
                rollback.disarm();
                self.publish_content(snapshot).await;
                self.refresh_observed_placement_scopes(placement_scopes);
                Ok(RisePostUpdateOutcome { post, rejected })
            }
            Err(error) => {
                rollback.run().await;
                Err(error)
            }
        }
    }

    fn refresh_observed_placement_scopes(&self, scopes: Vec<RisePostPageScope>) {
        let targets = self.state.with(|state| {
            let observed: HashSet<&RisePostPageScope> = state.content_scope_owners.values().collect();
            let targets: Vec<RisePostPageScope> =
                scopes.into_iter().filter(|scope| observed.contains(scope)).collect();
            if targets.is_empty() || !state.is_active || state.is_shutdown {
                return None;
            }
            Some(targets)
        });
        let Some(targets) = targets else {
            return;
        };
        let this = self.this.clone();
        drop(RiseAccountRuntime::current().spawn(async move {
            for scope in targets {
                let Some(this) = this.upgrade() else {
                    return;
                };
                this.refresh_placement_scope(scope).await;
            }
        }));
    }

    async fn refresh_placement_scope(&self, scope: RisePostPageScope) {
        if !self.is_live() {
            return;
        }
        self.forget_exhausted_preview_scope(&scope);
        self.load_content(scope, RisePostContentRepositoryLoadIntent::Refresh).await;
    }

    pub async fn delete_post(&self, post_id: i64) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        let Some(this) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let ticket = Arc::new(self.content_repository.begin_delete(post_id).await?);
        let rollback = RisePostDomainRollback::new({
            let ticket = ticket.clone();
            async move {
                let rollback = this.content_repository.rollback_delete(&ticket).await;
                this.publish_content(rollback).await;
            }
            .boxed()
        });
        self.publish_content(ticket.snapshot.clone()).await;
        match self.content_repository.finish_delete(&ticket).await {
            Ok(snapshot) => {
                rollback.disarm();
                self.publish_content(snapshot).await;
                Ok(())
            }
            Err(error) => {
                rollback.run().await;
                Err(error)
            }
        }
    }

    pub async fn set_post_like(
        &self,
        post_id: &str,
        liked: bool,
        seed: Option<GetPostFeedResponse>,
        on_refused: Option<RisePostEngagementRefusalHandler>,
    ) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        let snapshot = self.content_repository.set_like_intent(post_id, liked, seed, on_refused).await?;
        self.publish_content(snapshot).await;
        Ok(())
    }

    pub async fn apply_post_moderation_status(&self, post_id: &str, status: &str) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        let Some(snapshot) = self.content_repository.apply_moderation_status(post_id, status).await? else {
            return Ok(());
        };
        self.publish_content(snapshot).await;
        Ok(())
    }

    pub async fn apply_post_sparks_delta(&self, post_id: &str, delta: i64) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        let Some(snapshot) = self.content_repository.apply_sparks_delta(post_id, delta).await? else {
            return Ok(());
        };
        self.publish_content(snapshot).await;
        Ok(())
    }

    pub async fn send_post_sparks(
        &self,
        post_id: &str,
        amount: i64,
        anonymous: bool,
        idempotency_key: &str,
    ) -> anyhow::Result<RiseStoredSparksSendResult> {
        self.validate_active(None, None).await?;
        let Some(this) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let rollback = RisePostDomainRollback::new({
            let post_id = post_id.to_owned();
            async move {
                if let Ok(Some(snapshot)) = this.content_repository.apply_sparks_delta(&post_id, -amount).await {
                    this.publish_content(snapshot).await;
                }
            }
            .boxed()
        });
        match self.content_repository.send_sparks(post_id, amount, anonymous, idempotency_key).await {
            Ok((response, snapshot)) => {
                rollback.disarm();
                if let Some(snapshot) = snapshot {
                    self.publish_content(snapshot).await;
                }
                Ok(response)
            }
            Err(error) => {
                rollback.run().await;
                Err(error)
            }
        }
    }

    pub async fn post_sparks_top(&self, post_id: &str, limit: i64) -> anyhow::Result<RiseStoredSparksTop> {
        self.validate_active(None, None).await?;
        self.content_repository.sparks_top(post_id, limit).await
    }

    pub async fn set_post_favorite(
        &self,
        post_id: &str,
        favorited: bool,
        seed: Option<GetPostFeedResponse>,
        on_refused: Option<RisePostEngagementRefusalHandler>,
    ) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        let snapshot =
            self.content_repository.set_favorite_intent(post_id, favorited, seed, on_refused).await?;
        self.publish_content(snapshot).await;
        Ok(())
    }

    pub async fn set_author_relationship(
        &self,
        author_id: &str,
        is_subscribed: bool,
        is_subscriber: bool,
        is_friend: bool,
    ) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        let Some(snapshot) = self
            .content_repository
            .set_author_relationship(author_id, is_subscribed, is_subscriber, is_friend)
            .await?
        else {
            return Ok(());
        };
        self.publish_content(snapshot).await;
        Ok(())
    }

    pub async fn adjust_post_comments_count(&self, post_id: &str, delta: i64) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        let Some(snapshot) = self.content_repository.adjust_comments_count(post_id, delta).await? else {
            return Ok(());
        };
        self.publish_content(snapshot).await;
        Ok(())
    }

    pub async fn set_repost(
        &self,
        post: GetPostFeedResponse,
        desired: bool,
        content: Option<String>,
    ) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        let Some(this) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let ticket = Arc::new(self.content_repository.begin_repost(&post, desired, content).await?);
        let rollback = RisePostDomainRollback::new({
            let ticket = ticket.clone();
            async move {
                let rollback = this.content_repository.rollback_repost(&ticket).await;
                this.publish_content(rollback).await;
            }
            .boxed()
        });
        self.publish_content(ticket.snapshot.clone()).await;
        match self.content_repository.finish_repost(&ticket).await {
            Ok(snapshot) => {
                rollback.disarm();
                self.publish_content(snapshot).await;
                Ok(())
            }
            Err(error) => {
                rollback.run().await;
                Err(error)
            }
        }
    }

    pub async fn remove_repost(&self, owner_id: &str, post: &GetPostFeedResponse) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        let Some(root_id) = int_from_string(&post.id) else {
            return Err(RisePostRepositoryError::StaleAccount.into());
        };
        let Some(this) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let ticket =
            Arc::new(self.content_repository.begin_foreign_repost_removal(root_id, owner_id).await?);
        let rollback = RisePostDomainRollback::new({
            let ticket = ticket.clone();
            async move {
                let rollback = this.content_repository.rollback_foreign_repost_removal(&ticket).await;
                this.publish_content(rollback).await;
            }
            .boxed()
        });
        self.publish_content(ticket.snapshot.clone()).await;
        match self.content_repository.finish_foreign_repost_removal(&ticket).await {
            Ok(snapshot) => {
                rollback.disarm();
                self.publish_content(snapshot).await;
                Ok(())
            }
            Err(error) => {
                rollback.run().await;
                Err(error)
            }
        }
    }

    pub async fn update_repost_opinion(
        &self,
        post: GetPostFeedResponse,
        content: String,
        entities: Option<String>,
    ) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        let Some(this) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let ticket = Arc::new(self.content_repository.begin_opinion_update(&post, content, entities).await?);
        let rollback = RisePostDomainRollback::new({
            let ticket = ticket.clone();
            async move {
                let rollback = this.content_repository.rollback_opinion_update(&ticket).await;
                this.publish_content(rollback).await;
            }
            .boxed()
        });
        self.publish_content(ticket.snapshot.clone()).await;
        match self.content_repository.finish_opinion_update(&ticket).await {
            Ok(snapshot) => {
                rollback.disarm();
                self.publish_content(snapshot).await;
                Ok(())
            }
            Err(error) => {
                rollback.run().await;
                Err(error)
            }
        }
    }

    pub async fn invite_coauthors(
        &self,
        post_id: i64,
        users: Vec<PostCollaboratorUser>,
    ) -> anyhow::Result<Vec<PostCollaboratorRejection>> {
        self.run_collaboration(post_id, RisePostContentRepositoryCollaborationOperation::Invite(users)).await
    }

    pub async fn revoke_coauthor(&self, post_id: i64, user_id: &str) -> anyhow::Result<Vec<PostCollaboratorRejection>> {
        self.run_collaboration(
            post_id,
            RisePostContentRepositoryCollaborationOperation::RevokeCoauthor(user_id.to_owned()),
        )
        .await
    }

    pub async fn accept_coauthor_invite(&self, post_id: i64) -> anyhow::Result<Vec<PostCollaboratorRejection>> {
        self.run_collaboration(post_id, RisePostContentRepositoryCollaborationOperation::AcceptInvite).await
    }

    pub async fn decline_coauthor_invite(&self, post_id: i64) -> anyhow::Result<Vec<PostCollaboratorRejection>> {
        self.run_collaboration(post_id, RisePostContentRepositoryCollaborationOperation::DeclineInvite).await
    }

    pub async fn leave_coauthorship(&self, post_id: i64) -> anyhow::Result<Vec<PostCollaboratorRejection>> {
        self.run_collaboration(post_id, RisePostContentRepositoryCollaborationOperation::Leave).await
    }

    pub async fn set_coauthor_grid_visibility(
        &self,
        post_id: i64,
        show_in_grid: bool,
    ) -> anyhow::Result<Vec<PostCollaboratorRejection>> {
        self.run_collaboration(
            post_id,
            RisePostContentRepositoryCollaborationOperation::SetGridVisibility(show_in_grid),
        )
        .await
    }

    pub async fn set_post_tags(
        &self,
        post_id: i64,
        tags: Vec<PostTagInputDraft>,
    ) -> anyhow::Result<Vec<PostCollaboratorRejection>> {
        self.run_collaboration(post_id, RisePostContentRepositoryCollaborationOperation::SetTags(tags)).await
    }

    pub async fn remove_post_tag(&self, post_id: i64, user_id: &str) -> anyhow::Result<Vec<PostCollaboratorRejection>> {
        self.run_collaboration(
            post_id,
            RisePostContentRepositoryCollaborationOperation::RemoveTag(user_id.to_owned()),
        )
        .await
    }

    pub async fn remove_own_tag(&self, post_id: i64) -> anyhow::Result<Vec<PostCollaboratorRejection>> {
        self.run_collaboration(post_id, RisePostContentRepositoryCollaborationOperation::RemoveOwnTag).await
    }

    pub async fn load_post_collaborators(
        &self,
        post_id: i64,
        kind: Option<PostCollaboratorKind>,
        status: Option<PostCollaboratorStatus>,
        reset: bool,
    ) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        if post_id <= 0 {
            return Ok(());
        }
        let snapshot = self.content_repository.load_post_collaborators(post_id, kind, status, reset).await;
        self.publish_content(snapshot).await;
        Ok(())
    }

    async fn run_collaboration(
        &self,
        post_id: i64,
        operation: RisePostContentRepositoryCollaborationOperation,
    ) -> anyhow::Result<Vec<PostCollaboratorRejection>> {
        self.validate_active(None, None).await?;
        let Some(this) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let ticket = Arc::new(self.content_repository.begin_collaboration(post_id, operation).await?);
        let rollback = RisePostDomainRollback::new({
            let ticket = ticket.clone();
            async move {
                let rollback = this.content_repository.rollback_collaboration(&ticket).await;
                this.publish_content(rollback).await;
            }
            .boxed()
        });
        self.publish_content(ticket.snapshot.clone()).await;
        match self.content_repository.finish_collaboration(&ticket).await {
            Ok((rejected, snapshot)) => {
                rollback.disarm();
                self.publish_content(snapshot).await;
                Ok(rejected)
            }
            Err(error) => {
                rollback.run().await;
                Err(error)
            }
        }
    }

    pub async fn load_stats(&self, post_id: i64, days: i64, force: bool) -> anyhow::Result<()> {
        self.validate_active(None, None).await?;
        if post_id <= 0 {
            return Ok(());
        }
        self.state.with(|state| {
            state.active_stats_post_ids.insert(post_id);
        });
        let ticket = self.repository.begin_stats_refresh(post_id, force).await;
        let generation = ticket.generation;
        self.publish(ticket.snapshot).await;
        let Some(generation) = generation else {
            return Ok(());
        };
        let final_snapshot = self.repository.finish_stats_refresh(post_id, days, generation).await;
        self.publish(final_snapshot).await;
        Ok(())
    }

    pub async fn close_stats(&self, post_id: i64) {
        self.state.with(|state| {
            state.active_stats_post_ids.remove(&post_id);
        });
    }

    pub async fn record_view(&self, intent: &RisePostVisibilityIntent) -> anyhow::Result<bool> {
        self.validate_active(Some(intent.account_id.as_str()), Some(intent.account_epoch.as_str())).await?;
        let result = self.repository.record_view(intent).await;
        if let Some(snapshot) = result.snapshot {
            self.publish(snapshot).await;
        }
        Ok(result.handled)
    }

    pub async fn record_share(&self, post_id: i64, channel: &str) -> anyhow::Result<RecordPostShareGatewayDTO> {
        self.validate_active(None, None).await?;
        self.repository.record_share(post_id, channel).await
    }

    pub async fn record_link_open(&self, post_id: i64) -> anyhow::Result<RecordPostLinkOpenGatewayDTO> {
        self.validate_active(None, None).await?;
        self.repository.record_link_open(post_id).await
    }

    pub async fn shutdown(&self) {
        let tasks = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            state.is_shutdown = true;
            state.is_active = false;
            state.active_stats_post_ids.clear();
            state.recent_content_scopes.clear();
            state.content_scope_owners.clear();
            state.content_owner_versions.clear();
            let observations = std::mem::take(&mut state.content_observation_tasks);
            let feed_warmup = state.feed_warmup_task.take();
            state.pending_preview_tasks.clear();
            state.pending_preview_generations.clear();
            state.exhausted_pending_preview_scopes.clear();
            Some((observations, feed_warmup))
        });
        let Some((observations, feed_warmup)) = tasks else {
            return;
        };
        for observation in observations {
            observation.abort();
        }
        if let Some(feed_warmup) = feed_warmup {
            feed_warmup.abort();
        }
        self.content_repository.shutdown().await;
        self.repository.shutdown().await;
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let domain = self.this.clone();
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        RisePostDomainRegistry::shared(cx).update(cx, |registry, cx| registry.remove(&domain, cx));
                        RisePostPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, &account_epoch, session_id, cx);
                        });
                        RisePostContentPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, &account_epoch, session_id, cx);
                        });
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    fn load_content(
        &self,
        scope: RisePostPageScope,
        intent: RisePostContentRepositoryLoadIntent,
    ) -> BoxFuture<'_, ()> {
        async move {
            self.touch_content_scope(&scope);
            let ticket = self.content_repository.begin_load(scope.clone(), intent).await;
            let this = self.this.clone();
            let abandon = self.content_repository.load_guard(&ticket, move |snapshot| {
                drop(RiseAccountRuntime::current().spawn(async move {
                    if let Some(this) = this.upgrade() {
                        this.publish_content(snapshot).await;
                    }
                }));
            });
            self.update_pending_preview_revalidation(&scope, &ticket.snapshot);
            self.publish_content(ticket.snapshot.clone()).await;
            if ticket.generation.is_none() || self.is_shutdown() {
                abandon.disarm();
                for post_id in &ticket.processing_post_ids {
                    if self.is_shutdown() {
                        continue;
                    }
                    self.load_content(
                        RisePostPageScope::Detail { post_id: post_id.clone() },
                        RisePostContentRepositoryLoadIntent::Refresh,
                    )
                    .await;
                }
                return;
            }
            let final_snapshot = self.content_repository.finish_load(&ticket).await;
            abandon.disarm();
            self.update_pending_preview_revalidation(&scope, &final_snapshot);
            self.publish_content(final_snapshot).await;
        }
        .boxed()
    }

    fn touch_content_scope(&self, scope: &RisePostPageScope) {
        self.state.with(|state| {
            state.content_scope_clock = state.content_scope_clock.wrapping_add(1);
            state.recent_content_scopes.insert(scope.clone(), state.content_scope_clock);
            if state.recent_content_scopes.len() > 12
                && let Some(oldest) = state
                    .recent_content_scopes
                    .iter()
                    .min_by_key(|(_, clock)| **clock)
                    .map(|(scope, _)| scope.clone())
            {
                state.recent_content_scopes.remove(&oldest);
            }
        });
    }

    fn trim_content_owner_versions(state: &mut RisePostDomainState) {
        while state.content_owner_versions.len() > 256 {
            let removable_owner_id = state
                .content_owner_versions
                .iter()
                .filter(|(owner_id, _)| !state.content_scope_owners.contains_key(*owner_id))
                .min_by_key(|(_, version)| version.access)
                .map(|(owner_id, _)| *owner_id);
            let Some(owner_id) = removable_owner_id else {
                break;
            };
            state.content_owner_versions.remove(&owner_id);
        }
    }

    fn update_pending_preview_revalidation(
        &self,
        scope: &RisePostPageScope,
        snapshot: &RisePostContentPresentationSnapshot,
    ) {
        let has_pending = snapshot
            .pages_by_scope_key
            .get(&scope.key())
            .map(|page| page.items.iter().any(Self::has_pending_link_preview));
        let this = self.this.clone();
        self.state.with(|state| {
            let is_observed = state.content_scope_owners.values().any(|owned| owned == scope);
            let Some(has_pending) = has_pending.filter(|_| is_observed) else {
                Self::cancel_pending_preview_revalidation(state, scope);
                return;
            };
            if !has_pending {
                state.exhausted_pending_preview_scopes.remove(scope);
                Self::cancel_pending_preview_revalidation(state, scope);
                return;
            }
            if state.pending_preview_tasks.contains_key(scope)
                || state.exhausted_pending_preview_scopes.contains(scope)
                || state.pending_preview_tasks.len() >= 6
            {
                return;
            }
            state.pending_preview_clock = state.pending_preview_clock.wrapping_add(1);
            let generation = state.pending_preview_clock;
            state.pending_preview_generations.insert(scope.clone(), generation);
            let (cancel, cancelled) = oneshot::channel::<()>();
            let task_scope = scope.clone();
            drop(RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = this.upgrade() {
                    this.run_pending_preview_revalidation(task_scope, generation, cancelled).await;
                }
            }));
            state.pending_preview_tasks.insert(scope.clone(), cancel);
        });
    }

    async fn run_pending_preview_revalidation(
        &self,
        scope: RisePostPageScope,
        generation: u64,
        mut cancelled: oneshot::Receiver<()>,
    ) {
        for delay in [Duration::from_secs(2), Duration::from_secs(5), Duration::from_secs(12)] {
            let was_cancelled = tokio::select! {
                _ = RiseRuntimeTimer::sleep(delay) => false,
                _ = &mut cancelled => true,
            };
            if was_cancelled {
                break;
            }
            let proceeds = self.state.with(|state| {
                !state.is_shutdown
                    && state.pending_preview_generations.get(&scope) == Some(&generation)
                    && state.content_scope_owners.values().any(|owned| owned == &scope)
            });
            if !proceeds {
                break;
            }
            self.load_content(scope.clone(), RisePostContentRepositoryLoadIntent::Refresh).await;
        }
        self.state.with(|state| {
            if state.pending_preview_generations.get(&scope) == Some(&generation) {
                state.pending_preview_tasks.remove(&scope);
                state.pending_preview_generations.remove(&scope);
                state.exhausted_pending_preview_scopes.insert(scope);
            }
        });
    }

    fn cancel_pending_preview_revalidation(state: &mut RisePostDomainState, scope: &RisePostPageScope) {
        state.pending_preview_tasks.remove(scope);
        state.pending_preview_generations.remove(scope);
        if !state.content_scope_owners.values().any(|owned| owned == scope) {
            state.exhausted_pending_preview_scopes.remove(scope);
        }
    }

    fn has_pending_link_preview(post: &GetPostFeedResponse) -> bool {
        if post.content_kind != RisePostContentKind::Thread.wire() {
            return false;
        }
        let Some(preview) = post.link_preview.as_ref() else {
            return false;
        };
        let status = preview.status.to_lowercase();
        status != PostLinkPreviewStatus::Ready.wire() && status != PostLinkPreviewStatus::Failed.wire()
    }

    fn start_content_observations(&self) {
        let this = self.this.clone();
        let repository = self.content_repository.clone();
        self.state.with(|state| {
            if !state.content_observation_tasks.is_empty() {
                return;
            }
            let ranges: [Range<usize>; 2] = [0..16, 16..32];
            for range in ranges {
                let this = this.clone();
                let repository = repository.clone();
                state.content_observation_tasks.push(RiseAccountRuntime::current().spawn(async move {
                    let Ok(observation) = repository.observe_slots(range).await else {
                        return;
                    };
                    let mut updates = observation.updates.make_async_iterator();
                    while let Some(commit) = updates.next().await {
                        let Some(snapshot) = repository.refresh_pages_from_store(commit).await else {
                            continue;
                        };
                        if let Some(this) = this.upgrade() {
                            this.refresh_pending_preview_policies(&snapshot);
                            this.publish_content(snapshot).await;
                        }
                    }
                }));
            }
        });
    }

    fn refresh_pending_preview_policies(&self, snapshot: &RisePostContentPresentationSnapshot) {
        let scopes: HashSet<RisePostPageScope> =
            self.state.with(|state| state.content_scope_owners.values().cloned().collect());
        for scope in &scopes {
            self.update_pending_preview_revalidation(scope, snapshot);
        }
    }

    fn forget_exhausted_preview_scope(&self, scope: &RisePostPageScope) {
        self.state.with(|state| {
            state.exhausted_pending_preview_scopes.remove(scope);
        });
    }

    async fn validate_active(
        &self,
        expected_account_id: Option<&str>,
        expected_account_epoch: Option<&str>,
    ) -> anyhow::Result<()> {
        if !self.is_live() {
            return Err(RisePostRepositoryError::MissingActiveSession.into());
        }
        if let Some(expected_account_id) = expected_account_id
            && expected_account_id != self.account_id
        {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        if let Some(expected_account_epoch) = expected_account_epoch
            && expected_account_epoch != self.account_epoch
        {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        let Some(main) = RiseEngineMainHop::installed() else {
            return Err(RisePostRepositoryError::StaleAccount.into());
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let is_current = main
            .run(move |cx: AsyncApp| {
                async move {
                    Ok(cx.update(|cx| {
                        RisePostDomainRegistry::shared(cx).read(cx).matches(
                            &account_id,
                            &account_epoch,
                            session_id,
                        ) && SaiAccountCacheScope::is_current_account_epoch(
                            Some(account_id.as_str()),
                            Some(account_epoch.as_str()),
                        )
                    }))
                }
                .boxed_local()
            })
            .await
            .unwrap_or(false);
        if !is_current {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        Ok(())
    }

    async fn publish(&self, snapshot: RisePostPresentationSnapshot) {
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        if !RisePostDomainRegistry::shared(cx).read(cx).matches(
                            &account_id,
                            &account_epoch,
                            session_id,
                        ) {
                            return;
                        }
                        RisePostPresentation::shared(cx).update(cx, |presentation, cx| {
                            let _ = presentation.publish(snapshot, cx);
                        });
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    async fn publish_content(&self, snapshot: RisePostContentPresentationSnapshot) {
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        if !RisePostDomainRegistry::shared(cx).read(cx).matches(
                            &account_id,
                            &account_epoch,
                            session_id,
                        ) {
                            return;
                        }
                        let published = RisePostContentPresentation::shared(cx)
                            .update(cx, |presentation, cx| presentation.publish(snapshot, cx));
                        if !published {
                            return;
                        }
                        let selected_post_id = PostServicesStore::shared(cx)
                            .read(cx)
                            .selected_post
                            .as_ref()
                            .map(|post| post.id.clone());
                        if let Some(post_id) = selected_post_id {
                            PostServicesStore::shared(cx)
                                .update(cx, |store, cx| store.sync_selected_post_comment_state(&post_id, cx));
                        }
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    async fn current_screen() -> Option<SaiActiveScreen> {
        let main = RiseEngineMainHop::installed()?;
        main.run(move |cx: AsyncApp| {
            async move { Ok(cx.update(|cx| SaiNavigationBridge::current_screen(cx))) }.boxed_local()
        })
        .await
        .ok()
        .flatten()
    }

    fn is_live(&self) -> bool {
        self.state.with(|state| state.is_active && !state.is_shutdown)
    }

    fn is_shutdown(&self) -> bool {
        self.state.with(|state| state.is_shutdown)
    }
}

pub struct RisePostDomainRegistry {
    active: Option<Arc<RisePostDomain>>,
}

impl RisePostDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<&Arc<RisePostDomain>> {
        self.active.as_ref()
    }

    pub fn install(&mut self, domain: Arc<RisePostDomain>, cx: &mut Context<Self>) {
        self.active = Some(domain);
        cx.notify();
    }

    pub fn remove(&mut self, domain: &Weak<RisePostDomain>, cx: &mut Context<Self>) {
        let Some(active) = &self.active else {
            return;
        };
        if !std::ptr::eq(Arc::as_ptr(active), domain.as_ptr()) {
            return;
        }
        self.active = None;
        cx.notify();
    }

    pub fn matches(&self, account_id: &str, account_epoch: &str, session_id: Uuid) -> bool {
        let Some(active) = &self.active else {
            return false;
        };
        active.account_id == account_id && active.account_epoch == account_epoch && active.session_id == session_id
    }

    pub fn require_domain(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Arc<RisePostDomain>>> {
        if let Some(active) = &self.active {
            return Task::ready(Ok(active.clone()));
        }
        let prepare = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |registry, cx| {
            prepare.await?;
            let active = registry.read_with(cx, |registry, _| registry.active.clone())?;
            active.ok_or_else(|| RisePostRepositoryError::MissingActiveSession.into())
        })
    }
}

pub enum RisePostActionExecutor {}

impl RisePostActionExecutor {
    pub fn require_domain(cx: &mut App) -> Task<anyhow::Result<Arc<RisePostDomain>>> {
        RisePostDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx))
    }
}
