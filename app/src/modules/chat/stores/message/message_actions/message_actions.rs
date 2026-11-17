use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_canon::rise_key::bot as bot_key;
use rise_canon::rise_view::message as view;
use rise_canon::{rise_request, rise_wire_methods};
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseJSONValue};
use rise_errors::ErrorCode;
use rise_i18n::tr;
use serde_json::Value;

use super::message_types::{MessageContentType, MessageForwardError, MessageForwardLimits, MessageHistoryDirection};
use super::message_wire_types::{
    DeleteAllMessagesFromUserGatewayDTO, EditMessageGatewayDTO, ForwardMessageDestinationBody, ForwardMessagesGatewayDTO,
    MessagePayloadCodec, MessageWireDTO, RiseStoredOutgoingMessage,
};
use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::rise_engine_bridge::rise_transport_status::{RiseTransportDropWatch, RiseTransportStatus};
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::{
    RiseConversationActionExecutor, RiseConversationActionExecutorError,
};
use crate::modules::chat::engine::core::rise_conversation_request_state::RiseConversationStateError;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::chat::engine::messages::rise_message_contracts::{
    RiseMessageDraft, RiseMessageLocalUpload, RiseMessagePublishRefusal, RiseMessageRepositoryError,
    RiseMessageSubmissionMode, RiseMessageUploadPipeline,
};
use crate::modules::chat::engine::messages::rise_message_media_upload_pipeline::{
    RiseDeferredImageReeditDocument, RiseMessageMediaUploadPipeline,
};
use crate::modules::chat::engine::messages::rise_message_scope::{
    RiseMessagePageDirection, RiseMessageScope, RiseMessageWindowConfiguration,
};
use crate::modules::chat::engine::rise_conversation_domain::{RiseConversationDomain, RiseConversationDomainRegistry};
use crate::modules::chat::shared::chat_permission_resolution::ChatPermissionResolution;
use crate::modules::chat::shared::utils::voice::voice_message_metadata::VoiceMessageMetadata;
use crate::modules::chat::stores::chat_guide_sandbox_hooks::chat_guide_sandbox_hooks;
use crate::modules::chat::stores::chat_stores_hooks::chat_stores_share_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::media_picker::api::media_picker_result::MediaPickerItem;
use crate::modules::media_picker::editor::media_reedit_store::MediaReeditStore;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;

const MESSAGE_WINDOW_SIDE_MAX: i64 = 159;
const EDITED_MEDIA_TYPE: &str = "image";
const EDITED_MEDIA_MIME_TYPE: &str = "image/png";

pub type MessageActionCompletion = Box<dyn FnOnce(anyhow::Result<()>, &mut App)>;
pub type MessageActionErrorHandler = Rc<dyn Fn(&anyhow::Error, &mut App)>;
pub type MessageActionFlagHandler = Box<dyn FnOnce(bool, &mut App)>;
pub type MessageActionHandler = Box<dyn FnOnce(&mut App)>;
pub type MessageForwardSuccessHandler =
    Box<dyn FnOnce(ForwardMessagesGatewayDTO, Vec<String>, Vec<ForwardMessageDestinationBody>, &mut App)>;
pub type MessageForwardErrorHandler = Box<dyn FnOnce(anyhow::Error, &mut App)>;

pub struct MessageActionsStore {
    pub last_draft_send_local_id: Option<String>,
    read_receipt_reconnect_watches: HashMap<String, RiseTransportDropWatch>,
    outbox_turns: HashMap<String, Rc<MessageOutboxTurn>>,
}

struct AccountRuntimeAbortOnDrop(tokio::task::AbortHandle);

impl Drop for AccountRuntimeAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) async fn on_account_runtime<F>(work: F) -> anyhow::Result<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let task = RiseAccountRuntime::current().spawn(work);
    let _abort = AccountRuntimeAbortOnDrop(task.abort_handle());
    task.await.map_err(|_| CancellationError.into())
}

pub(super) fn guide_sandbox_owns_chat(chat_id: &str, cx: &App) -> bool {
    (chat_guide_sandbox_hooks().owns_chat_id)(chat_id, cx)
}

pub(super) fn log_message_failure(text: String, cx: &mut App) {
    tracing::warn!(target: "riseonly::message", "{text}");
    Sai::log("Message", move || text, SaiLogKind::Network, cx);
}

pub(super) fn profile_user_id(cx: &mut App) -> Option<String> {
    AuthServicesStore::shared(cx)
        .read(cx)
        .profile()
        .map(|profile| profile.id.clone())
        .filter(|user_id| !user_id.is_empty())
}

pub(super) fn effective_user_id_for_wire(cx: &mut App) -> Option<String> {
    AuthServicesStore::shared(cx)
        .read(cx)
        .effective_user_id_for_wire()
        .filter(|user_id| !user_id.is_empty())
}

fn ensure_current_send(
    domain: &RiseConversationDomain,
    send_account_user_id: &str,
    send_account_epoch: &str,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let is_active = cx.update(|cx| domain.is_registry_active(cx));
    if is_active && SaiAccountCacheScope::is_current_account_epoch(Some(send_account_user_id), Some(send_account_epoch)) {
        Ok(())
    } else {
        Err(CancellationError.into())
    }
}

fn is_slow_mode_watch_refusal(code: &str) -> bool {
    code == ErrorCode::ChatSendRestricted.wire()
        || code == ErrorCode::ChatMemberMuted.wire()
        || code == ErrorCode::ModerationCreateBlocked.wire()
}

fn report_media_edit_failure(
    on_complete: Option<MessageActionFlagHandler>,
    on_error: Option<MessageActionErrorHandler>,
    error: &anyhow::Error,
    cx: &mut App,
) {
    if let Some(on_complete) = on_complete {
        on_complete(false, cx);
    }
    if let Some(on_error) = on_error {
        on_error(error, cx);
    }
}

struct DeferredOriginalImageRemoval(Option<PathBuf>);

impl Drop for DeferredOriginalImageRemoval {
    fn drop(&mut self) {
        if let Some(original_url) = self.0.take() {
            let _ = RiseAccountRuntime::current().spawn_blocking(move || {
                let _ = std::fs::remove_file(&original_url);
            });
        }
    }
}

impl MessageActionsStore {
    pub const OUTGOING_DONATION_DELAY: Duration = Duration::from_millis(600);

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            last_draft_send_local_id: None,
            read_receipt_reconnect_watches: HashMap::new(),
            outbox_turns: HashMap::new(),
        }
    }

    fn release_outbox_turn(&mut self, chat_id: &str, turn: Option<&Rc<MessageOutboxTurn>>) {
        let Some(turn) = turn else {
            return;
        };
        turn.release();
        if self.outbox_turns.get(chat_id).is_some_and(|current| Rc::ptr_eq(current, turn)) {
            self.outbox_turns.remove(chat_id);
        }
    }

    pub fn reject_guide_chat_mutation(&self, chat_id: &str, cx: &mut App) -> bool {
        if !guide_sandbox_owns_chat(chat_id, cx) {
            return false;
        }
        Notifier::send(
            tr("guide_sandbox_action_unavailable"),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_messages_action(
        &self,
        chat_id: &str,
        user_chat_id: Option<&str>,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        relative_id: Option<&str>,
        direction: MessageHistoryDirection,
        _history_request_token: Option<u64>,
        _fetch_if_have_data: bool,
        _reflect_pending_in_ui: Option<bool>,
        cx: &mut App,
    ) {
        if chat_id.is_empty() && user_chat_id.is_none_or(str::is_empty) {
            return;
        }
        let effective_id = if chat_id.is_empty() { user_chat_id.unwrap_or_default() } else { chat_id };
        let is_first_page = relative_id.is_none();
        let rise_scope = RiseMessageScope::new(
            effective_id.to_owned(),
            topic_id.map(str::to_owned),
            thread_root_id.map(str::to_owned),
        );
        cx.spawn(async move |cx| match RiseConversationActionExecutor::require_domain(cx).await {
            Ok(domain) => {
                drop(RiseAccountRuntime::current().spawn(async move {
                    if is_first_page {
                        let _ = domain.load_initial_messages(&rise_scope, None, None, None, None, None).await;
                    } else {
                        let page_direction = if direction == MessageHistoryDirection::Older {
                            RiseMessagePageDirection::Older
                        } else {
                            RiseMessagePageDirection::Newer
                        };
                        let _ = domain.load_message_page(&rise_scope, page_direction).await;
                    }
                }));
            }
            Err(error) => cx.update(|cx| {
                log_message_failure(format!("Rise get_messages({}) failed: {error}", rise_scope.storage_key()), cx);
            }),
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_messages_around_action(
        &self,
        chat_id: &str,
        user_chat_id: Option<&str>,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        anchor: &str,
        before_limit: i64,
        after_limit: i64,
        _reflect_pending_in_ui: bool,
        completion: Option<MessageActionFlagHandler>,
        cx: &mut App,
    ) {
        let normalized_anchor = trimming_whitespaces_and_newlines(anchor).to_owned();
        if (chat_id.is_empty() && user_chat_id.is_none_or(str::is_empty)) || normalized_anchor.is_empty() {
            if let Some(completion) = completion {
                cx.defer(move |cx| completion(false, cx));
            }
            return;
        }
        let effective_id = if chat_id.is_empty() { user_chat_id.unwrap_or_default() } else { chat_id };
        let rise_scope = RiseMessageScope::new(
            effective_id.to_owned(),
            topic_id.map(str::to_owned),
            thread_root_id.map(str::to_owned),
        );
        cx.spawn(async move |cx| {
            let is_installed = match RiseConversationActionExecutor::require_domain(cx).await {
                Ok(domain) => on_account_runtime(async move {
                    domain
                        .jump_to_message(
                            &rise_scope,
                            &normalized_anchor,
                            RiseMessageWindowConfiguration::new(
                                before_limit.clamp(1, MESSAGE_WINDOW_SIDE_MAX),
                                after_limit.clamp(1, MESSAGE_WINDOW_SIDE_MAX),
                            ),
                        )
                        .await
                        .is_installed()
                })
                .await
                .unwrap_or(false),
                Err(error) => {
                    cx.update(|cx| {
                        log_message_failure(
                            format!("Rise get_messages_around({}) failed: {error}", rise_scope.storage_key()),
                            cx,
                        );
                    });
                    false
                }
            };
            if let Some(completion) = completion {
                cx.update(|cx| completion(is_installed, cx));
            }
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_message_action(
        &mut self,
        body: RiseStoredOutgoingMessage,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        local_files: Vec<RiseOutgoingMediaFile>,
        picker_items: Vec<MediaPickerItem>,
        voice_metadata_slots: Vec<VoiceMessageMetadata>,
        temp_id: &str,
        defer_until_connected: bool,
        _response_timeout_seconds: Option<TimeInterval>,
        bound_domain: Option<Arc<RiseConversationDomain>>,
        completion: Option<MessageActionCompletion>,
        cx: &mut App,
    ) {
        let fail_now = |completion: Option<MessageActionCompletion>, error: anyhow::Error, cx: &mut App| {
            if let Some(completion) = completion {
                cx.defer(move |cx| completion(Err(error), cx));
            }
        };
        if self.reject_guide_chat_mutation(chat_id, cx) {
            fail_now(completion, SaiWsSendError::GuideSandbox.into(), cx);
            return;
        }
        let (Some(send_account_user_id), Some(send_account_epoch)) =
            (SaiAccountCacheScope::active_user_id(), SaiAccountCacheScope::active_epoch())
        else {
            fail_now(completion, CancellationError.into(), cx);
            return;
        };
        let send_domain = bound_domain.or_else(|| RiseConversationDomainRegistry::shared(cx).read(cx).active());
        if let Some(send_domain) = &send_domain
            && (send_domain.account_id != send_account_user_id || !send_domain.is_registry_active(cx))
        {
            fail_now(completion, CancellationError.into(), cx);
            return;
        }
        if ChatServicesStore::is_draft_direct_chat_id(chat_id) {
            self.last_draft_send_local_id = Some(temp_id.to_owned());
        }
        ChatServicesStore::shared(cx).update(cx, |store, cx| store.start_slow_mode(chat_id, Date::now(), cx));
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let turn = (local_files.is_empty() && defer_until_connected).then(|| Rc::new(MessageOutboxTurn::new()));
        let previous_turn = turn.as_ref().and_then(|turn| self.outbox_turns.insert(chat_id.to_owned(), Rc::clone(turn)));
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.map(str::to_owned);
        let thread_root_id = thread_root_id.map(str::to_owned);
        let temp_id = temp_id.to_owned();
        cx.spawn(async move |cx| {
            let started: anyhow::Result<Arc<RiseConversationDomain>> = async {
                let domain = match send_domain {
                    Some(domain) => domain,
                    None => RiseConversationActionExecutor::require_domain(cx).await?,
                };
                ensure_current_send(&domain, &send_account_user_id, &send_account_epoch, cx)?;
                if domain.account_id != send_account_user_id {
                    return Err(CancellationError.into());
                }
                Ok(domain)
            }
            .await;
            match started {
                Ok(domain) => cx.update(|cx| {
                    Self::create_message_with_rise(
                        domain,
                        send_account_user_id,
                        send_account_epoch,
                        body,
                        chat_id,
                        topic_id,
                        thread_root_id,
                        scope,
                        local_files,
                        picker_items,
                        voice_metadata_slots,
                        temp_id,
                        defer_until_connected,
                        previous_turn,
                        turn,
                        completion,
                        cx,
                    );
                }),
                Err(error) => cx.update(|cx| {
                    Self::shared(cx).update(cx, |store, _| store.release_outbox_turn(&chat_id, turn.as_ref()));
                    log_message_failure(format!("Rise create_message({scope}) failed to start: {error}"), cx);
                    if let Some(completion) = completion {
                        completion(Err(error), cx);
                    }
                }),
            }
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn create_message_with_rise(
        domain: Arc<RiseConversationDomain>,
        send_account_user_id: String,
        send_account_epoch: String,
        body: RiseStoredOutgoingMessage,
        chat_id: String,
        topic_id: Option<String>,
        thread_root_id: Option<String>,
        message_scope: String,
        local_files: Vec<RiseOutgoingMediaFile>,
        picker_items: Vec<MediaPickerItem>,
        voice_metadata_slots: Vec<VoiceMessageMetadata>,
        temp_id: String,
        defer_until_connected: bool,
        previous_turn: Option<Rc<MessageOutboxTurn>>,
        turn: Option<Rc<MessageOutboxTurn>>,
        completion: Option<MessageActionCompletion>,
        cx: &mut App,
    ) {
        let mut outbound_body = body;
        outbound_body.chat_id = outbound_body.chat_id.or_else(|| Some(chat_id.clone()));
        outbound_body.topic_id = outbound_body.topic_id.or_else(|| topic_id.clone());
        outbound_body.temp_id = Some(temp_id.clone());

        let optimistic_wire: Option<MessageWireDTO> = match serde_json::to_value(&outbound_body) {
            Ok(outbound) => {
                let row = MessageServicesStore::shared(cx).update(cx, |store, cx| {
                    store.create_optimistic_message_data(
                        &outbound,
                        &chat_id,
                        &temp_id,
                        &local_files,
                        &picker_items,
                        &voice_metadata_slots,
                        cx,
                    )
                });
                let context_scope = message_scope.clone();
                let decoded = MessagePayloadCodec::decode_reporting::<MessageWireDTO>(
                    Some(&Value::Object(row)),
                    move || format!("create_message optimistic {context_scope}"),
                    cx,
                );
                if decoded.is_none() {
                    let text = format!("Rise create_message({message_scope}) uses typed optimistic fallback");
                    Sai::log("Message", move || text, SaiLogKind::Cache, cx);
                }
                decoded
            }
            Err(error) => {
                let text = format!("Rise create_message({message_scope}) optimistic preparation fallback: {error}");
                Sai::log("Message", move || text, SaiLogKind::Cache, cx);
                None
            }
        };

        let uploads: Option<Vec<RiseMessageLocalUpload>> = local_files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                rise_core::foundation::foundation_url::Url::from_file_path(&file.url).map(|file_url| {
                    RiseMessageLocalUpload::new(
                        format!("{temp_id}-{index}"),
                        file_url,
                        file.media_type.clone().unwrap_or_else(|| file.content_type.clone()),
                    )
                })
            })
            .collect();
        let has_local_files = !local_files.is_empty();
        let upload_pipeline: Option<Arc<dyn RiseMessageUploadPipeline>> = has_local_files.then(|| {
            Arc::new(RiseMessageMediaUploadPipeline::new(
                local_files,
                picker_items,
                voice_metadata_slots,
                domain.account_id.clone(),
                Some(send_account_epoch.clone()),
                domain.upload_realtime_progress.clone(),
            )) as Arc<dyn RiseMessageUploadPipeline>
        });
        let rise_scope = RiseMessageScope::new(chat_id.clone(), topic_id, thread_root_id);
        let chat_services = ChatServicesStore::shared(cx);
        let mode = if chat_services.read(cx).is_undeliverable_draft_direct_chat(&chat_id) {
            RiseMessageSubmissionMode::Undeliverable
        } else if defer_until_connected && chat_services.read(cx).is_draft_direct_chat_awaiting_connection(&chat_id) {
            RiseMessageSubmissionMode::AwaitingConversation
        } else if defer_until_connected {
            RiseMessageSubmissionMode::Durable
        } else {
            RiseMessageSubmissionMode::Immediate
        };
        let has_completion = completion.is_some();
        cx.spawn(async move |cx| {
            let outcome: anyhow::Result<()> = async {
                if let Some(previous_turn) = &previous_turn {
                    previous_turn.wait().await;
                }
                ensure_current_send(&domain, &send_account_user_id, &send_account_epoch, cx)?;
                let observing = {
                    let domain = domain.clone();
                    let scope = rise_scope.clone();
                    on_account_runtime(async move { domain.observe_messages(&scope).await.map(|_| ()) })
                };
                if let Err(error) = observing.await.and_then(|observed| observed) {
                    cx.update(|cx| {
                        log_message_failure(format!("Rise create_message({message_scope}) sends unobserved: {error}"), cx);
                    });
                }
                ensure_current_send(&domain, &send_account_user_id, &send_account_epoch, cx)?;
                let local_uploads = uploads.ok_or(RiseConversationActionExecutorError::InvalidRequestBody)?;
                let publish_refusal: Option<RiseMessagePublishRefusal> = (!local_uploads.is_empty()).then(|| {
                    let refusal_chat_id = chat_id.clone();
                    Arc::new(move || {
                        let refusal_chat_id = refusal_chat_id.clone();
                        async move {
                            let main = RiseEngineMainHop::installed()?;
                            main.run(move |cx: AsyncApp| {
                                async move {
                                    Ok(cx.update(|cx| MessageActionsStore::channel_publish_refusal(&refusal_chat_id, cx)))
                                }
                                .boxed_local()
                            })
                            .await
                            .ok()
                            .flatten()
                        }
                        .boxed()
                    }) as RiseMessagePublishRefusal
                });
                let mut draft = RiseMessageDraft::new(outbound_body.clone());
                draft.optimistic_wire = optimistic_wire;
                draft.local_uploads = local_uploads;
                draft.upload_pipeline = upload_pipeline;
                draft.publish_refusal = publish_refusal;
                let submission = {
                    let domain = domain.clone();
                    let scope = rise_scope.clone();
                    on_account_runtime(async move { domain.submit_message(draft, &scope, mode).await }).await??
                };
                cx.update(|cx| {
                    Self::shared(cx).update(cx, |store, _| store.release_outbox_turn(&chat_id, turn.as_ref()));
                });
                ensure_current_send(&domain, &send_account_user_id, &send_account_epoch, cx)?;
                if mode == RiseMessageSubmissionMode::Undeliverable {
                    let carries_text = outbound_body.content_type != MessageContentType::Location.raw_value();
                    let content = if carries_text { outbound_body.content.as_str() } else { "" };
                    let caption = outbound_body.caption.as_deref().unwrap_or_default();
                    let preview = [content, caption]
                        .into_iter()
                        .map(trimming_whitespaces_and_newlines)
                        .find(|text| !text.is_empty())
                        .unwrap_or_default();
                    cx.update(|cx| {
                        ChatServicesStore::shared(cx).update(cx, |store, cx| {
                            store.note_undeliverable_draft_message(
                                &chat_id,
                                preview,
                                MessageContentType::from_raw_value(&outbound_body.content_type),
                                cx,
                            );
                        });
                    });
                } else if mode != RiseMessageSubmissionMode::AwaitingConversation {
                    let donation_chat_id = chat_id.clone();
                    let donation_user_id = send_account_user_id.clone();
                    let donation_epoch = send_account_epoch.clone();
                    cx.update(|cx| {
                        cx.spawn(async move |cx| {
                            cx.background_executor().timer(Self::OUTGOING_DONATION_DELAY).await;
                            if !SaiAccountCacheScope::is_current_account_epoch(
                                Some(&donation_user_id),
                                Some(&donation_epoch),
                            ) {
                                return;
                            }
                            cx.update(|cx| {
                                if let Some(hooks) = chat_stores_share_hooks() {
                                    (hooks.donate_outgoing_interaction)(&donation_chat_id, cx);
                                }
                            });
                        })
                        .detach();
                    });
                }

                let watches_slow_mode = mode == RiseMessageSubmissionMode::Durable
                    && cx.update(|cx| {
                        ChatServicesStore::shared(cx).update(cx, |store, cx| store.slow_mode_may_apply(&chat_id, cx))
                    });
                let watches_refusal = mode == RiseMessageSubmissionMode::Durable;
                if defer_until_connected
                    && mode != RiseMessageSubmissionMode::AwaitingConversation
                    && (has_completion || watches_slow_mode || watches_refusal)
                {
                    let picked_again_in_one_tap = !has_local_files
                        && trimming_whitespaces_and_newlines(&outbound_body.content).is_empty()
                        && (outbound_body.content_type == MessageContentType::Sticker.raw_value()
                            || outbound_body.content_type == MessageContentType::Animation.raw_value());
                    let waiting = {
                        let domain = domain.clone();
                        let local_id = submission.local_id.clone();
                        on_account_runtime(async move { domain.wait_for_message_submission(&local_id).await })
                    };
                    match waiting.await.and_then(|settled| settled) {
                        Ok(()) => {
                            if watches_slow_mode
                                && SaiAccountCacheScope::is_current_account_epoch(
                                    Some(&send_account_user_id),
                                    Some(&send_account_epoch),
                                )
                            {
                                cx.update(|cx| {
                                    ChatServicesStore::shared(cx).update(cx, |store, cx| {
                                        if store.slow_mode_remaining_seconds(&chat_id, cx) == 0 {
                                            store.start_slow_mode(&chat_id, Date::now(), cx);
                                        }
                                    });
                                });
                            }
                        }
                        Err(error) => {
                            let refused_code = match error.downcast_ref::<RiseMessageRepositoryError>() {
                                Some(RiseMessageRepositoryError::RemoteFailure(code)) if watches_refusal => {
                                    Some(code.clone())
                                }
                                _ => None,
                            };
                            if let Some(code) = refused_code {
                                let is_slow_mode = code == ErrorCode::ChatSlowModeActive.wire();
                                if (is_slow_mode && watches_slow_mode) || is_slow_mode_watch_refusal(&code) {
                                    if picked_again_in_one_tap || !is_slow_mode {
                                        let domain = domain.clone();
                                        let local_id = submission.local_id.clone();
                                        let scope = rise_scope.clone();
                                        let _ = on_account_runtime(async move {
                                            domain.cancel_outgoing_message(&local_id, &scope).await;
                                        })
                                        .await;
                                    }
                                    if SaiAccountCacheScope::is_current_account_epoch(
                                        Some(&send_account_user_id),
                                        Some(&send_account_epoch),
                                    ) {
                                        cx.update(|cx| {
                                            ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                                                if is_slow_mode {
                                                    store.slow_mode_refused_send(&chat_id, cx);
                                                } else {
                                                    store.member_send_refused(&code, cx);
                                                }
                                            });
                                        });
                                    }
                                }
                            }
                            return Err(error);
                        }
                    }
                }
                ensure_current_send(&domain, &send_account_user_id, &send_account_epoch, cx)
            }
            .await;
            cx.update(|cx| match outcome {
                Ok(()) => {
                    if let Some(completion) = completion {
                        completion(Ok(()), cx);
                    }
                }
                Err(error) => {
                    Self::shared(cx).update(cx, |store, _| store.release_outbox_turn(&chat_id, turn.as_ref()));
                    log_message_failure(format!("Rise create_message({message_scope}) failed: {error}"), cx);
                    if let Some(completion) = completion {
                        completion(Err(error), cx);
                    }
                }
            });
        })
        .detach();
    }

    pub fn channel_publish_refusal(chat_id: &str, cx: &mut App) -> Option<String> {
        let chat = ChatServicesStore::shared(cx).update(cx, |store, cx| store.chat_info(chat_id, cx))?;
        if chat.r#type != ChatType::Channel || ChatPermissionResolution::can_post_to_channel(chat.role.as_deref()) {
            return None;
        }
        Some(ErrorCode::ChatPermissionDenied.wire().to_owned())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn edit_message_action(
        &self,
        message_id: &str,
        content: &str,
        entities: Option<String>,
        link_preview_type: Option<String>,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        on_error: Option<MessageActionErrorHandler>,
        cx: &mut App,
    ) {
        if profile_user_id(cx).is_none() {
            return;
        }
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let wire_id =
            MessageServicesStore::shared(cx).update(cx, |store, cx| store.server_message_id(message_id, &scope, cx));
        let body = rise_request::message::EditMessage {
            caption: None,
            content: Some(content.to_owned()),
            document: None,
            entities,
            link_preview_type,
            media_items: None,
            message_id: wire_id,
            reply_markup: None,
            web_app_data: None,
        };
        let message_id = message_id.to_owned();
        cx.spawn(async move |cx| {
            let result = RiseConversationActionExecutor::call::<_, EditMessageGatewayDTO>(
                rise_wire_methods::message::SERVICE,
                rise_wire_methods::message::EDIT_MESSAGE,
                &body,
                cx,
            )
            .await;
            if let Err(error) = result {
                cx.update(|cx| {
                    log_message_failure(format!("Rise edit_message({scope}/{message_id}) failed: {error}"), cx);
                    if let Some(on_error) = on_error {
                        on_error(&error, cx);
                    }
                });
            }
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn edit_message_media_action(
        &self,
        message_id: &str,
        chat_id: &str,
        caption: &str,
        entities: Option<String>,
        item: MediaPickerItem,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        on_complete: Option<MessageActionFlagHandler>,
        on_error: Option<MessageActionErrorHandler>,
        cx: &mut App,
    ) {
        let fail_now = |on_complete: Option<MessageActionFlagHandler>, cx: &mut App| {
            if let Some(on_complete) = on_complete {
                cx.defer(move |cx| on_complete(false, cx));
            }
        };
        let Some(user_id) = profile_user_id(cx) else {
            fail_now(on_complete, cx);
            return;
        };
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        if account_user_id.as_deref() != Some(user_id.as_str())
            || account_epoch.is_none()
            || !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
        {
            fail_now(on_complete, cx);
            return;
        }

        let pixel_size = item.image_payload.as_ref().map(|payload| payload.pixel_size);
        let pixel_width = pixel_size.map_or(0.0, |size| size.width);
        let pixel_height = pixel_size.map_or(0.0, |size| size.height);
        let rise_scope =
            RiseMessageScope::new(chat_id.to_owned(), topic_id.map(str::to_owned), thread_root_id.map(str::to_owned));
        let existing_slot_id = RiseConversationPresentation::shared(cx)
            .read(cx)
            .messages(&rise_scope)
            .iter()
            .find(|message| message.id == message_id)
            .and_then(|message| message.media_items.first())
            .map(|media| media.id.clone())
            .unwrap_or_else(|| format!("{message_id}-0"));
        let item_url = item.url.absolute_string().to_owned();
        let mut local_media: BTreeMap<String, RiseJSONValue> = BTreeMap::from([
            (bot_key::ID.to_owned(), RiseJSONValue::String(existing_slot_id)),
            (bot_key::MEDIA_TYPE.to_owned(), RiseJSONValue::String(EDITED_MEDIA_TYPE.to_owned())),
            (bot_key::FILE_URL.to_owned(), RiseJSONValue::String(item_url.clone())),
            (bot_key::THUMBNAIL_URL.to_owned(), RiseJSONValue::String(item_url)),
            (RiseLocalKey::IS_UPLOADING.to_owned(), RiseJSONValue::Bool(true)),
            (RiseLocalKey::UPLOAD_PROGRESS.to_owned(), RiseJSONValue::Number(0.0)),
            (bot_key::MIME_TYPE.to_owned(), RiseJSONValue::String(EDITED_MEDIA_MIME_TYPE.to_owned())),
        ]);
        if pixel_width > 0.0 {
            local_media.insert(bot_key::WIDTH.to_owned(), RiseJSONValue::Integer(pixel_width as i64));
        }
        if pixel_height > 0.0 {
            local_media.insert(bot_key::HEIGHT.to_owned(), RiseJSONValue::Integer(pixel_height as i64));
        }
        if let Some(preview_url) = item.image_payload.as_ref().and_then(|payload| payload.optimistic_preview_url.as_ref()) {
            let preview = preview_url.absolute_string().to_owned();
            local_media.insert(bot_key::THUMBNAIL_URL.to_owned(), RiseJSONValue::String(preview.clone()));
            local_media.insert(RiseLocalKey::LOCAL_PREVIEW_URL.to_owned(), RiseJSONValue::String(preview));
        }
        let message_id = message_id.to_owned();
        let chat_id = chat_id.to_owned();
        let caption = caption.to_owned();
        cx.spawn(async move |cx| {
            let begun = async {
                let domain = RiseConversationActionExecutor::require_domain(cx).await?;
                let checkpoint = {
                    let domain = domain.clone();
                    let message_id = message_id.clone();
                    let caption = caption.clone();
                    let entities = entities.clone();
                    on_account_runtime(async move {
                        domain.begin_media_edit(&message_id, &caption, local_media, entities).await
                    })
                    .await??
                };
                anyhow::Ok((domain, checkpoint))
            }
            .await;
            let (domain, checkpoint) = match begun {
                Ok(begun) => begun,
                Err(error) => {
                    cx.update(|cx| report_media_edit_failure(on_complete, on_error, &error, cx));
                    return;
                }
            };
            let upload_owner_id = checkpoint.entity.local_id.clone();
            let outcome: anyhow::Result<()> = async {
                let cancellation_signal = {
                    let domain = domain.clone();
                    let owner = upload_owner_id.clone();
                    on_account_runtime(async move { domain.media_edit_cancellation_signal(&owner).await }).await?
                };
                let is_cancelled: Arc<dyn Fn() -> BoxFuture<'static, bool> + Send + Sync> = {
                    let domain = domain.clone();
                    let owner = upload_owner_id.clone();
                    Arc::new(move || {
                        let domain = domain.clone();
                        let owner = owner.clone();
                        async move {
                            on_account_runtime(async move { domain.is_media_edit_cancelled(&owner).await })
                                .await
                                .unwrap_or(true)
                        }
                        .boxed()
                    })
                };
                let on_progress: Arc<dyn Fn(f64) -> BoxFuture<'static, ()> + Send + Sync> = {
                    let domain = domain.clone();
                    let owner = upload_owner_id.clone();
                    Arc::new(move |fraction: f64| {
                        let domain = domain.clone();
                        let owner = owner.clone();
                        async move {
                            let _ = on_account_runtime(async move {
                                domain.publish_media_edit_progress(&owner, fraction).await;
                            })
                            .await;
                        }
                        .boxed()
                    })
                };
                let upload = MessageServicesStore::upload_edited_media(
                    &item,
                    &chat_id,
                    account_user_id.clone(),
                    account_epoch.clone(),
                    cancellation_signal,
                    is_cancelled,
                    on_progress,
                    cx,
                )
                .await?;
                let _original_removal = DeferredOriginalImageRemoval(
                    upload.deferred_reedit_document.as_ref().map(|document| document.original_image_url.clone()),
                );
                if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
                    return Err(RiseConversationStateError::StaleAccount.into());
                }
                let is_edit_cancelled = {
                    let domain = domain.clone();
                    let owner = upload_owner_id.clone();
                    on_account_runtime(async move { domain.is_media_edit_cancelled(&owner).await }).await?
                };
                if is_edit_cancelled {
                    return Err(CancellationError.into());
                }
                let mut wire_slot = RiseConversationUploadSupport::sanitized_slot_for_outbound(&upload.slot);
                for local_key in [
                    RiseLocalKey::IS_UPLOADING,
                    RiseLocalKey::UPLOAD_PROGRESS,
                    RiseLocalKey::COMPRESSION_PROGRESS,
                    RiseLocalKey::IS_COMPRESSING,
                ] {
                    wire_slot.remove(local_key);
                }
                if !wire_slot.contains_key(bot_key::WIDTH) && pixel_width > 0.0 {
                    wire_slot.insert(bot_key::WIDTH.to_owned(), Value::from(pixel_width as i64));
                }
                if !wire_slot.contains_key(bot_key::HEIGHT) && pixel_height > 0.0 {
                    wire_slot.insert(bot_key::HEIGHT.to_owned(), Value::from(pixel_height as i64));
                }
                let media_item = SaiJSONCodec::decode::<rise_request::message::edit_message::MediaItemsElement>(Some(
                    &Value::Object(wire_slot),
                ))
                .ok_or(RiseConversationActionExecutorError::InvalidRequestBody)?;
                let request = rise_request::message::EditMessage {
                    caption: None,
                    content: Some(caption.clone()),
                    document: None,
                    entities: entities.clone(),
                    link_preview_type: None,
                    media_items: Some(vec![media_item]),
                    message_id: checkpoint.server_id.clone(),
                    reply_markup: None,
                    web_app_data: None,
                };
                let _: EditMessageGatewayDTO = RiseConversationActionExecutor::call(
                    rise_wire_methods::message::SERVICE,
                    rise_wire_methods::message::EDIT_MESSAGE,
                    &request,
                    cx,
                )
                .await?;
                if let Some(document) = &upload.deferred_reedit_document {
                    let identities = [message_id.clone(), upload_owner_id.clone(), checkpoint.server_id.clone()];
                    cx.update(|cx| Self::persist_deferred_media_edit_document(document, &identities, cx));
                }
                {
                    let domain = domain.clone();
                    let owner = upload_owner_id.clone();
                    on_account_runtime(async move { domain.finish_media_edit(&owner).await }).await?;
                }
                Ok(())
            }
            .await;
            match outcome {
                Ok(()) => {
                    if let Some(on_complete) = on_complete {
                        cx.update(|cx| on_complete(true, cx));
                    }
                }
                Err(error) => {
                    let _ = on_account_runtime(async move { domain.rollback_media_edit(checkpoint).await }).await;
                    cx.update(|cx| report_media_edit_failure(on_complete, on_error, &error, cx));
                }
            }
        })
        .detach();
    }

    fn persist_deferred_media_edit_document(
        document: &RiseDeferredImageReeditDocument,
        identities: &[String],
        cx: &mut App,
    ) {
        MediaReeditStore::shared(cx).update(cx, |store, _| {
            let Some(edit_id) = store.save_document(
                &document.original_image_url,
                &document.editor_state_data,
                MediaReeditStore::chat_ttl_seconds(),
            ) else {
                return;
            };
            let unique: HashSet<&String> = identities.iter().collect();
            for identity in unique {
                if !identity.is_empty() {
                    store.link(identity, &edit_id);
                }
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn delete_messages_action(
        &self,
        message_ids: Vec<String>,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        for_everyone: bool,
        on_error: Option<MessageActionErrorHandler>,
        cx: &mut App,
    ) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        if message_ids.is_empty() {
            return;
        }
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let Some(deletion_scope) = RiseMessageScope::parse(&scope) else {
            return;
        };
        let account_epoch = SaiAccountCacheScope::active_epoch();
        cx.spawn(async move |cx| {
            let outcome: anyhow::Result<()> = async {
                let domain = RiseConversationActionExecutor::require_domain(cx).await?;
                if domain.account_id != user_id
                    || !SaiAccountCacheScope::is_current_account_epoch(Some(&user_id), account_epoch.as_deref())
                {
                    return Err(CancellationError.into());
                }
                on_account_runtime(async move {
                    domain.delete_messages(&message_ids, &deletion_scope, for_everyone.then_some(true)).await
                })
                .await?
            }
            .await;
            if let Err(error) = outcome {
                cx.update(|cx| {
                    log_message_failure(format!("Rise delete_messages({scope}) failed: {error}"), cx);
                    if let Some(on_error) = on_error {
                        on_error(&error, cx);
                    }
                });
            }
        })
        .detach();
    }

    pub fn forward_messages_action(
        &self,
        message_ids: Vec<String>,
        destinations: Vec<ForwardMessageDestinationBody>,
        idempotency_key: String,
        on_success: MessageForwardSuccessHandler,
        on_error: MessageForwardErrorHandler,
        cx: &mut App,
    ) {
        let fail_invalid = |on_error: MessageForwardErrorHandler, cx: &mut App| {
            cx.defer(move |cx| on_error(anyhow::Error::from(MessageForwardError::InvalidRequest), cx));
        };
        if effective_user_id_for_wire(cx).is_none() {
            fail_invalid(on_error, cx);
            return;
        }
        let mut seen_message_ids: HashSet<String> = HashSet::new();
        let normalized_message_ids: Vec<String> = message_ids
            .iter()
            .filter_map(|raw| {
                let value = trimming_whitespaces_and_newlines(raw).to_owned();
                (!value.is_empty() && !GetSaiServerId::is_temp_id(Some(&value)) && seen_message_ids.insert(value.clone()))
                    .then_some(value)
            })
            .collect();
        let unique_destinations: HashSet<ForwardMessageDestinationBody> = destinations
            .iter()
            .filter_map(|destination| {
                let chat_id = trimming_whitespaces_and_newlines(destination.chat_id.as_deref().unwrap_or_default());
                if chat_id.is_empty() {
                    return None;
                }
                let topic_id = destination
                    .topic_id
                    .as_deref()
                    .map(trimming_whitespaces_and_newlines)
                    .filter(|topic_id| !topic_id.is_empty());
                Some(ForwardMessageDestinationBody {
                    chat_id: Some(chat_id.to_owned()),
                    topic_id: topic_id.map(str::to_owned),
                })
            })
            .collect();
        let mut normalized_destinations: Vec<ForwardMessageDestinationBody> = unique_destinations.into_iter().collect();
        normalized_destinations.sort_by(|left, right| {
            left.chat_id
                .as_deref()
                .unwrap_or_default()
                .cmp(right.chat_id.as_deref().unwrap_or_default())
                .then_with(|| left.topic_id.as_deref().unwrap_or_default().cmp(right.topic_id.as_deref().unwrap_or_default()))
        });
        let request_key = trimming_whitespaces_and_newlines(&idempotency_key).to_owned();
        let message_count = normalized_message_ids.len() as i64;
        let destination_count = normalized_destinations.len() as i64;
        if !(1..=MessageForwardLimits::max_messages()).contains(&message_count)
            || normalized_destinations.is_empty()
            || destination_count > MessageForwardLimits::max_destinations()
            || message_count * destination_count > MessageForwardLimits::max_copies()
            || request_key.is_empty()
        {
            fail_invalid(on_error, cx);
            return;
        }

        let body = rise_request::message::ForwardMessages {
            caption: None,
            destinations: Some(normalized_destinations.clone()),
            idempotency_key: Some(request_key),
            message_ids: Some(normalized_message_ids.clone()),
            target_chat_ids: Some(Vec::new()),
        };
        cx.spawn(async move |cx| {
            let result = RiseConversationActionExecutor::call::<_, view::Forwarded>(
                rise_wire_methods::message::SERVICE,
                rise_wire_methods::message::FORWARD_MESSAGES,
                &body,
                cx,
            )
            .await;
            cx.update(|cx| match result {
                Ok(forwarded) => on_success(
                    ForwardMessagesGatewayDTO::from_canon(&forwarded),
                    normalized_message_ids,
                    normalized_destinations,
                    cx,
                ),
                Err(error) => on_error(error, cx),
            });
        })
        .detach();
    }

    pub fn delete_all_messages_from_user_action(
        &self,
        chat_id: &str,
        target_user_id: &str,
        on_error: Option<MessageActionErrorHandler>,
        cx: &mut App,
    ) {
        if profile_user_id(cx).is_none() || chat_id.is_empty() || target_user_id.is_empty() {
            return;
        }
        let body = rise_request::message::DeleteAllMessagesFromUser {
            chat_id: chat_id.to_owned(),
            target_user_id: target_user_id.to_owned(),
        };
        let chat_id = chat_id.to_owned();
        cx.spawn(async move |cx| {
            let result = RiseConversationActionExecutor::call::<_, DeleteAllMessagesFromUserGatewayDTO>(
                rise_wire_methods::message::SERVICE,
                rise_wire_methods::message::DELETE_ALL_MESSAGES_FROM_USER,
                &body,
                cx,
            )
            .await;
            if let Err(error) = result {
                cx.update(|cx| {
                    log_message_failure(format!("Rise delete_all_messages_from_user({chat_id}) failed: {error}"), cx);
                    if let Some(on_error) = on_error {
                        on_error(&error, cx);
                    }
                });
            }
        })
        .detach();
    }

    pub fn clear_draft_chat_history_action(
        &self,
        chat_id: &str,
        completion: MessageActionFlagHandler,
        cx: &mut App,
    ) -> Option<Task<()>> {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let domain = RiseConversationDomainRegistry::shared(cx).read(cx).active();
        let (true, Some(account_user_id), Some(account_epoch), Some(domain)) =
            (ChatServicesStore::is_draft_direct_chat_id(chat_id), account_user_id, account_epoch, domain)
        else {
            cx.defer(move |cx| completion(false, cx));
            return None;
        };
        if domain.account_id != account_user_id {
            cx.defer(move |cx| completion(false, cx));
            return None;
        }
        let chat_id = chat_id.to_owned();
        Some(cx.spawn(async move |cx| {
            let is_active = cx.update(|cx| domain.is_registry_active(cx));
            if !is_active
                || !SaiAccountCacheScope::is_current_account_epoch(Some(&account_user_id), Some(&account_epoch))
            {
                return;
            }
            let clearing = {
                let domain = domain.clone();
                on_account_runtime(async move { domain.clear_deleted_chat_local_history(&chat_id).await })
            };
            let cleared = clearing.await.and_then(|cleared| cleared).is_ok();
            let is_active = cx.update(|cx| domain.is_registry_active(cx));
            if (cleared && !is_active)
                || !SaiAccountCacheScope::is_current_account_epoch(Some(&account_user_id), Some(&account_epoch))
            {
                return;
            }
            cx.update(|cx| completion(cleared, cx));
        }))
    }

    pub fn load_message_read_receipts_action(&mut self, chat_id: &str, message_id: &str, force: bool, cx: &mut App) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            tracing::warn!(target: "riseonly::X563", "read receipts skipped chat={chat_id} message={message_id}: no user id for wire");
            return;
        };
        let offline = !RiseTransportStatus::is_connected(cx);
        tracing::warn!(target: "riseonly::X563", "read receipts load chat={chat_id} message={message_id} force={force} offline={offline}");
        let watch_key = format!("{chat_id}|{message_id}");
        if let Some(watch) = self.read_receipt_reconnect_watches.remove(&watch_key) {
            watch.stop();
        }
        if offline {
            RiseTransportStatus::wake_if_offline(cx);
            let handler_key = watch_key.clone();
            let handler_chat_id = chat_id.to_owned();
            let handler_message_id = message_id.to_owned();
            let watch = RiseTransportStatus::on_next_connect(
                move |cx: &mut App| {
                    Self::shared(cx).update(cx, |store, cx| {
                        store.read_receipt_reconnect_watches.remove(&handler_key);
                        store.load_message_read_receipts_action(&handler_chat_id, &handler_message_id, true, cx);
                    });
                },
                cx,
            );
            self.read_receipt_reconnect_watches.insert(watch_key, watch);
        }
        let chat_id = chat_id.to_owned();
        let message_id = message_id.to_owned();
        cx.spawn(async move |cx| match RiseConversationActionExecutor::require_domain(cx).await {
            Ok(domain) => {
                drop(RiseAccountRuntime::current().spawn(async move {
                    domain.load_message_read_receipts(&user_id, &chat_id, &message_id, force, offline).await;
                }));
            }
            Err(error) => cx.update(|cx| {
                log_message_failure(format!("Rise get_message_read_users failed: {error}"), cx);
            }),
        })
        .detach();
    }
}

#[derive(Default)]
pub struct MessageOutboxTurn {
    released: Cell<bool>,
    waiters: RefCell<Vec<oneshot::Sender<()>>>,
}

impl MessageOutboxTurn {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn wait(&self) {
        if self.released.get() {
            return;
        }
        let (sender, receiver) = oneshot::channel();
        self.waiters.borrow_mut().push(sender);
        let _ = receiver.await;
    }

    pub fn release(&self) {
        if self.released.replace(true) {
            return;
        }
        let pending = std::mem::take(&mut *self.waiters.borrow_mut());
        for waiter in pending {
            let _ = waiter.send(());
        }
    }
}
