use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Weak};

use futures::FutureExt;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_canon::{RiseEvent, rise_key, rise_wire_canon};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_sync::RiseSyncPushHandler;
use rise_engine::rise_wire::rise_json_value::RiseJSONValue;
use rise_engine::rise_wire::rise_wire_contracts::RiseWirePushEnvelope;
use rise_engine::{CancellationError, RiseEngine};
use tokio::task::JoinHandle;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::comment::engine::rise_comment_engine_models::{
    RiseCommentCreateDraft, RiseCommentCreateOutcome, RiseCommentError, RiseCommentScope,
};
use crate::modules::comment::engine::rise_comment_presentation::RiseCommentPresentation;
use crate::modules::comment::engine::rise_comment_repository::RiseCommentRepository;
use crate::modules::comment::stores::comment_actions::comment_types::{
    CommentMedia, CommentReaction, GetCommentsResponse,
};
use crate::modules::comment::stores::comment_actions::comment_wire_types::{GetCommentItemDTO, RiseCommentWireRow};

struct RiseCommentDomainState {
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
    observed: HashMap<RiseCommentScope, i64>,
    refresh_task: Option<JoinHandle<()>>,
}

/// account scoped root for the comment service
/// one repository since there is one resource, every list the app draws is a view over the
/// same entities
pub struct RiseCommentDomain {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub engine: Arc<RiseEngine>,
    pub comments: Arc<RiseCommentRepository>,
    this: Weak<RiseCommentDomain>,
    state: RiseIsolated<RiseCommentDomainState>,
}

impl RiseCommentDomain {
    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        let session = Uuid::new_v4();
        let account_id = engine.account.account_id.clone();
        let account_epoch = engine.account.epoch.clone();
        let comments = Arc::new(RiseCommentRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session,
        ));
        Arc::new_cyclic(|this| Self {
            account_id,
            account_epoch,
            session_id: session,
            engine,
            comments,
            this: this.clone(),
            state: RiseIsolated::new(RiseCommentDomainState {
                is_prepared: false,
                is_active: false,
                is_shutdown: false,
                observed: HashMap::new(),
                refresh_task: None,
            }),
        })
    }

    /// reducers are installed here never in activate
    /// the runtime snapshots the push claim set once right after bootstrap returns
    pub async fn prepare(&self, context: &RiseAppEngineBootstrapContext) -> anyhow::Result<()> {
        let (is_shutdown, is_prepared) = self.state.with(|state| (state.is_shutdown, state.is_prepared));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if is_prepared {
            return Ok(());
        }
        let comments = self.comments.clone();
        context
            .register_push_handler(RiseSyncPushHandler::new(
                RiseCommentRealtime::DELETED_EVENT_TYPE,
                move |envelope| {
                    let comments = comments.clone();
                    async move {
                        if let Some(event) = RiseCommentRealtime::decode_deleted(&envelope) {
                            comments
                                .apply_remote_delete(
                                    &event.comment_id,
                                    &event.post_id,
                                    event.parent_id.as_deref(),
                                )
                                .await;
                        }
                        Ok(None)
                    }
                },
            ))
            .await?;
        let comments = self.comments.clone();
        context
            .register_push_handler(RiseSyncPushHandler::new(
                RiseCommentRealtime::UPDATED_EVENT_TYPE,
                move |envelope| {
                    let comments = comments.clone();
                    async move {
                        if let Some(event) = RiseCommentRealtime::decode_updated(&envelope) {
                            comments.apply_remote_update(
                                &event.comment_id,
                                event.content,
                                event.original_content,
                                event.updated_at,
                                event.entities,
                            );
                        }
                        Ok(None)
                    }
                },
            ))
            .await?;
        let comments = self.comments.clone();
        context
            .register_push_handler(RiseSyncPushHandler::new(
                RiseCommentRealtime::CREATED_EVENT_TYPE,
                move |envelope| {
                    let comments = comments.clone();
                    async move {
                        if let Some(comment) = RiseCommentRealtime::decode_created(&envelope) {
                            comments.apply_remote_create(comment).await;
                        }
                        Ok(None)
                    }
                },
            ))
            .await?;
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
            return Err(RiseCommentError::MissingActiveSession.into());
        }
        if is_active {
            return Ok(());
        }

        let is_current_account =
            SaiAccountCacheScope::is_current_account_epoch(Some(&self.account_id), Some(&self.account_epoch));
        if !is_current_account {
            return Err(RiseCommentError::StaleAccount.into());
        }

        self.state.with(|state| state.is_active = true);
        let Some(domain) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let Some(main) = RiseEngineMainHop::installed() else {
            return Ok(());
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        main.run(move |cx: AsyncApp| {
            async move {
                cx.update(|cx| {
                    RiseCommentPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                    });
                    RiseCommentDomainRegistry::shared(cx).update(cx, |registry, cx| registry.install(domain, cx));
                });
                Ok(())
            }
            .boxed_local()
        })
        .await
    }

    pub async fn did_activate_ownership(&self) {}

    /// started never awaited, an rpc awaited from the lifecycle queue blocks delivery of its own reply
    pub async fn socket_connected(&self) {
        let comments = self.comments.clone();
        self.state.with(|state| {
            if !state.is_active || state.is_shutdown {
                return;
            }
            let scopes: Vec<RiseCommentScope> = state.observed.keys().cloned().collect();
            if scopes.is_empty() {
                return;
            }
            if let Some(refresh_task) = state.refresh_task.take() {
                refresh_task.abort();
            }
            state.refresh_task = Some(RiseAccountRuntime::current().spawn(async move {
                comments.refresh(&scopes).await;
            }));
        });
    }

    pub async fn begin_observing(&self, scope: &RiseCommentScope) {
        self.state.with(|state| {
            *state.observed.entry(scope.clone()).or_insert(0) += 1;
        });
    }

    pub async fn end_observing(&self, scope: &RiseCommentScope) {
        let was_last = self.state.with(|state| {
            let Some(count) = state.observed.get(scope).copied() else {
                return false;
            };
            if count > 1 {
                state.observed.insert(scope.clone(), count - 1);
            } else {
                state.observed.remove(scope);
            }
            count == 1
        });
        if !was_last {
            return;
        }
        let Some(domain) = self.this.upgrade() else {
            return;
        };
        let scope = scope.clone();
        drop(RiseAccountRuntime::current().spawn(async move {
            domain.trim_if_unobserved(&scope);
        }));
    }

    fn trim_if_unobserved(&self, scope: &RiseCommentScope) {
        let is_unobserved = self.state.with(|state| !state.is_shutdown && !state.observed.contains_key(scope));
        if is_unobserved {
            self.comments.trim_unobserved(scope);
        }
    }

    fn is_live(&self) -> bool {
        self.state.with(|state| state.is_active && !state.is_shutdown)
    }

    pub async fn open(
        &self,
        scope: &RiseCommentScope,
        force: bool,
        focus_comment_id: Option<&str>,
    ) -> Option<String> {
        if !self.is_live() {
            return None;
        }
        self.comments.open(scope, force, focus_comment_id).await
    }

    pub async fn load_more(&self, scope: &RiseCommentScope) {
        if !self.is_live() {
            return;
        }
        self.comments.load_more(scope).await;
    }

    pub async fn create(&self, draft: RiseCommentCreateDraft, send_now: bool) -> RiseCommentCreateOutcome {
        if !self.is_live() {
            return RiseCommentCreateOutcome::Failed { draft, refusal: None };
        }
        self.comments.create(draft, send_now).await
    }

    pub async fn complete_create(&self, temp_id: &str, media: CommentMedia) -> RiseCommentCreateOutcome {
        if !self.is_live() {
            return RiseCommentCreateOutcome::Withdrawn;
        }
        self.comments.complete_create(temp_id, media).await
    }

    pub async fn discard_create(&self, temp_id: &str) {
        if !self.is_live() {
            return;
        }
        self.comments.discard_create(temp_id).await;
    }

    pub async fn delete(&self, comment_id: &str) {
        if !self.is_live() {
            return;
        }
        self.comments.delete(comment_id).await;
    }

    pub async fn set_desired_reaction(&self, comment_id: &str, desired: CommentReaction) {
        if !self.is_live() {
            return;
        }
        self.comments.set_desired_reaction(comment_id, desired);
    }

    pub async fn flush_reaction(&self, comment_id: &str) {
        if !self.is_live() {
            return;
        }
        self.comments.flush_reaction(comment_id).await;
    }

    pub async fn apply_moderation_status(&self, comment_id: &str, status: &str) {
        if !self.is_live() {
            return;
        }
        self.comments.apply_moderation_status(comment_id, status);
    }

    pub async fn shutdown(&self) {
        let refresh_task = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            state.is_shutdown = true;
            state.is_active = false;
            state.observed.clear();
            Some(state.refresh_task.take())
        });
        let Some(refresh_task) = refresh_task else {
            return;
        };
        if let Some(refresh_task) = refresh_task {
            refresh_task.abort();
        }
        self.comments.shutdown().await;
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let domain = self.this.clone();
        let account_id = self.account_id.clone();
        let session_id = self.session_id;
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        RiseCommentDomainRegistry::shared(cx).update(cx, |registry, cx| registry.remove(&domain, cx));
                        RiseCommentPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, session_id, cx);
                        });
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }
}

pub struct RiseCommentDomainRegistry {
    active: Option<Arc<RiseCommentDomain>>,
}

impl RiseCommentDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<&Arc<RiseCommentDomain>> {
        self.active.as_ref()
    }

    pub fn install(&mut self, domain: Arc<RiseCommentDomain>, cx: &mut Context<Self>) {
        self.active = Some(domain);
        cx.notify();
    }

    /// only the domain that installed itself may take itself out
    pub fn remove(&mut self, domain: &Weak<RiseCommentDomain>, cx: &mut Context<Self>) {
        let Some(active) = &self.active else {
            return;
        };
        if !std::ptr::eq(Arc::as_ptr(active), domain.as_ptr()) {
            return;
        }
        self.active = None;
        cx.notify();
    }

    pub fn require_domain(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Arc<RiseCommentDomain>>> {
        if let Some(active) = &self.active {
            return Task::ready(Ok(active.clone()));
        }
        let prepare = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |registry, cx| {
            prepare.await?;
            let active = registry.read_with(cx, |registry, _| registry.active.clone())?;
            active.ok_or_else(|| RiseCommentError::MissingActiveSession.into())
        })
    }
}

pub enum RiseCommentActionExecutor {}

impl RiseCommentActionExecutor {
    pub fn require_domain(cx: &mut App) -> Task<anyhow::Result<Arc<RiseCommentDomain>>> {
        RiseCommentDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseCommentRealtimeDeleted {
    pub comment_id: String,
    pub post_id: String,
    pub parent_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseCommentRealtimeUpdated {
    pub comment_id: String,
    pub content: Option<String>,
    pub original_content: Option<String>,
    pub updated_at: Option<String>,
    pub entities: Option<Option<String>>,
}

/// pure and total, a reducer that throws makes sync do a full resync
/// gateway lays nested objects flat into data so keys are read from root then nested data
/// both spellings of each key, this shit was hard
pub enum RiseCommentRealtime {}

impl RiseCommentRealtime {
    pub const DELETED_EVENT_TYPE: &'static str = rise_events::name::COMMENT_DELETED;
    pub const UPDATED_EVENT_TYPE: &'static str = rise_events::name::COMMENT_UPDATED;
    pub const CREATED_EVENT_TYPE: &'static str = rise_events::name::COMMENT_CREATED;

    pub fn decode_deleted(envelope: &RiseWirePushEnvelope) -> Option<RiseCommentRealtimeDeleted> {
        if let Some(RiseEvent::CommentDeleted(deleted)) = Self::canon_event(envelope)
            && deleted.comment_id > 0
        {
            return Some(RiseCommentRealtimeDeleted {
                comment_id: deleted.comment_id.to_string(),
                post_id: deleted.post_id.to_string(),
                parent_id: deleted.parent_id.and_then(|id| (id > 0).then(|| id.to_string())),
            });
        }
        let root = Self::payload(envelope)?;
        let comment_id = Self::string(root, rise_key::comment::COMMENT_ID).unwrap_or_default();
        if comment_id.is_empty() {
            return None;
        }
        let parent_id = Self::string(root, rise_key::comment::PARENT_ID).unwrap_or_default();
        Some(RiseCommentRealtimeDeleted {
            comment_id,
            post_id: Self::string(root, rise_key::comment::POST_ID).unwrap_or_default(),
            parent_id: if parent_id.is_empty() { None } else { Some(parent_id) },
        })
    }

    pub fn decode_updated(envelope: &RiseWirePushEnvelope) -> Option<RiseCommentRealtimeUpdated> {
        let root = Self::payload(envelope)?;
        let typed_id = match Self::canon_event(envelope) {
            Some(RiseEvent::CommentUpdated(updated)) if updated.comment_id > 0 => Some(updated.comment_id.to_string()),
            _ => None,
        };
        let comment_id =
            typed_id.or_else(|| Self::string(root, rise_key::comment::COMMENT_ID)).unwrap_or_default();
        if comment_id.is_empty() {
            return None;
        }
        let Some(RiseJSONValue::Object(comment)) = root.get(rise_key::comment::COMMENT) else {
            return None;
        };

        let entities = match comment.get(rise_key::comment::ENTITIES) {
            Some(RiseJSONValue::String(value)) => Some(Some(value.clone())),
            Some(RiseJSONValue::Null) => Some(None),
            _ => None,
        };
        let content = Self::string(comment, rise_key::comment::CONTENT);
        let original_content = Self::string(comment, rise_key::events::camel::ORIGINAL_CONTENT);
        let updated_at = Self::string(comment, rise_key::events::camel::UPDATED_AT);
        if content.is_none() && original_content.is_none() && updated_at.is_none() && entities.is_none() {
            return None;
        }
        Some(RiseCommentRealtimeUpdated { comment_id, content, original_content, updated_at, entities })
    }

    pub fn decode_created(envelope: &RiseWirePushEnvelope) -> Option<GetCommentsResponse> {
        let root = Self::payload(envelope)?;
        let body = root.get(rise_key::comment::COMMENT)?;
        if !matches!(body, RiseJSONValue::Object(_)) {
            return None;
        }
        let row: GetCommentItemDTO = body.decode_as().ok()?;
        let mut comment = row.as_comment()?;
        if comment.post_id.is_empty() {
            comment.post_id = Self::string(root, rise_key::comment::POST_ID).unwrap_or_default();
        }
        if comment.parent_id.is_none()
            && let Some(parent_id) = Self::string(root, rise_key::comment::PARENT_ID)
            && !parent_id.is_empty()
            && parent_id != "0"
        {
            comment.parent_id = Some(parent_id);
        }
        if comment.post_id.is_empty() {
            return None;
        }
        Some(comment)
    }

    fn canon_event(envelope: &RiseWirePushEnvelope) -> Option<RiseEvent> {
        let data = envelope.data.data(false).ok()?;
        RiseEvent::decode(&envelope.r#type, &data).ok()
    }

    fn payload(envelope: &RiseWirePushEnvelope) -> Option<&BTreeMap<String, RiseJSONValue>> {
        let RiseJSONValue::Object(root) = &envelope.data else {
            return None;
        };
        if !root.contains_key(rise_key::comment::COMMENT_ID)
            && let Some(RiseJSONValue::Object(inner)) = root.get(rise_wire_canon::key::DATA)
        {
            return Some(inner);
        }
        Some(root)
    }

    fn string(object: &BTreeMap<String, RiseJSONValue>, key: &str) -> Option<String> {
        match object.get(key) {
            Some(RiseJSONValue::String(value)) => Some(value.clone()),
            Some(RiseJSONValue::Integer(value)) => Some(value.to_string()),
            Some(RiseJSONValue::UnsignedInteger(value)) => Some(value.to_string()),
            Some(RiseJSONValue::Number(value)) => Some((*value as i64).to_string()),
            _ => None,
        }
    }
}
