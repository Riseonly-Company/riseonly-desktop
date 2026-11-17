use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use futures::channel::mpsc;
use futures::future::{FutureExt, LocalBoxFuture};
use gpui::AsyncApp;
use rise_canon::rise_key::{
    auth as auth_key, chat as chat_key, file as file_key, message as message_key, support as support_key,
    upload as upload_key,
};
use rise_canon::rise_request::chat::{GetShareTargets, GetShareTopics};
use rise_canon::rise_request::message::{SharePayload as SharePayloadRequest, share_payload};
use rise_canon::rise_wire_canon::key as wire_key;
use rise_canon::rise_rpc::LIMITS_MAXIMUM_INBOUND_FRAME_BYTES;
use rise_canon::{RiseIdentityCanon, RiseLimits, rise_upload, rise_wire_methods};
use rise_chat::{ChatType, MediaKind, MessageContentType};
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::{character_prefix, trimming_characters, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::RiseIsolated;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::{HTTPURLResponse, URLError, URLErrorCode, URLRequest, URLSession};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::SEC_WEBSOCKET_PROTOCOL;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message, WebSocketConfig};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::core::lib::sai::modules::sai_ws::sai_ws_envelope::SaiWsEnvelopeBuilder;
use crate::core::lib::sai::sai_auth_hooks::{sai_auth_is_expired, sai_auth_refresh_access_token};
use crate::modules::share_extension::entry::rise_share_wire::{
    RiseShareJSONObject, RiseShareJSONValue, RiseSharePayloadWireResponse, RiseShareTargetsWireResponse,
    RiseShareTopicsWireResponse, RiseShareTransportError,
};
use crate::modules::share_extension::entry::share_extension_configuration::ShareExtensionConfiguration;
use crate::modules::share_extension::entry::share_extension_dependencies::{
    ShareContentSending, ShareDestinationPage, ShareExtensionDependencyResolver, ShareExtensionRemoteProviding,
    ShareSendCommitStarted, ShareSendProgressHandler,
};
use crate::modules::share_extension::entry::share_extension_models::{
    ShareAttachment, ShareAttachmentKind, ShareDestination, ShareDestinationKind, ShareDestinationTitle,
    ShareExtensionError, SharePartialDeliveryError, SharePayload, ShareSelection, ShareSendProgress,
    ShareSendRequest, ShareTopic,
};
use crate::modules::share_extension::entry::share_extension_strings::ShareStrings;
use crate::modules::share_extension::entry::share_outgoing_media_preparer::ShareOutgoingMediaPreparer;
use crate::modules::share_extension::shared::rise_outgoing_media::RiseOutgoingVideoTranscoderProgress;
use crate::modules::share_extension::shared::rise_share_models::{
    RiseShareCredential, RiseShareSection, RiseShareSnapshot, RiseShareTopicStructure,
};
use crate::modules::share_extension::shared::rise_upload_context_rules::RiseLimitsUploadContextRulesExt;

const UPLOAD_FILE_CONTEXT: &str = "chat";
const PATH_INITIATE: &str = "file/upload/initiate";
const PATH_SLAB: &str = "file/upload/slab";
const PATH_COMPLETE: &str = "file/upload/complete";
const PATH_ABORT: &str = "file/upload/abort";
const METHOD_POST: &str = "POST";
const METHOD_PUT: &str = "PUT";
const HEADER_ACCEPT: &str = "Accept";
const HEADER_CONTENT_TYPE: &str = "Content-Type";
const HEADER_AUTHORIZATION: &str = "Authorization";
const HEADER_UPLOAD_ID: &str = "Upload-Id";
const HEADER_LAST_SLAB: &str = "Last-Slab";
const ACCEPT_JSON: &str = "application/json";
const CONTENT_TYPE_JSON: &str = "application/json; charset=utf-8";
const CONTENT_TYPE_OCTET_STREAM: &str = "application/octet-stream";
const HTTP_UNAUTHORIZED: i64 = 401;
const HTTP_FORBIDDEN: i64 = 403;
const SERVER_MESSAGE_LIMIT: usize = 500;
const ACCESS_TOKEN_EXPIRY_LEEWAY_SECONDS: f64 = 30.0;
const MAXIMUM_SHARE_ATTEMPTS: usize = 3;
const FIRST_SHARE_RETRY_DELAY: Duration = Duration::from_millis(350);
const LATER_SHARE_RETRY_DELAY: Duration = Duration::from_millis(900);
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(30);
const SHARE_PAYLOAD_TIMEOUT: Duration = Duration::from_secs(55);
const DATA_REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
const ABORT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const SOCKET_IDLE_TIMEOUT: Duration = Duration::from_secs(45);
const MINIMUM_RECEIVE_TIMEOUT: Duration = Duration::from_millis(100);
const RESOURCE_TIMEOUT: Duration = Duration::from_secs(180);

/// error for a share upload destination that was denied
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShareUploadDestinationDenied;

impl fmt::Display for ShareUploadDestinationDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&ShareStrings::invalid_selection())
    }
}

impl std::error::Error for ShareUploadDestinationDenied {}

type ShareWebSocketStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct ShareUploadAbortOnCancel(Option<URLRequest>);

impl ShareUploadAbortOnCancel {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for ShareUploadAbortOnCancel {
    fn drop(&mut self) {
        if let Some(request) = self.0.take() {
            drop(RiseAccountRuntime::current().spawn(RiseShareAPIClient::send_abort(request)));
        }
    }
}

struct ShareRuntimeTaskAbortOnDrop(tokio::task::AbortHandle);

impl Drop for ShareRuntimeTaskAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct ShareWebSocketCloseOnDrop(Option<ShareWebSocketStream>);

impl Drop for ShareWebSocketCloseOnDrop {
    fn drop(&mut self) {
        if let Some(mut socket) = self.0.take() {
            drop(RiseAccountRuntime::current().spawn(async move {
                let _ = socket.close(Some(CloseFrame { code: CloseCode::Normal, reason: Default::default() })).await;
            }));
        }
    }
}

struct RiseShareAPIClientState {
    #[allow(dead_code)]
    credential: RiseShareCredential,
    uploaded_media_by_idempotency_key: HashMap<Uuid, HashMap<Uuid, RiseShareJSONValue>>,
    prepared_attachments_by_idempotency_key: HashMap<Uuid, HashMap<Uuid, ShareAttachment>>,
}

/// api client of the share layer
pub struct RiseShareAPIClient {
    snapshot: RiseShareSnapshot,
    configuration: ShareExtensionConfiguration,
    state: RiseIsolated<RiseShareAPIClientState>,
}

impl RiseShareAPIClient {
    /// builds the share api client from snapshot, credential and config
    pub fn new(
        snapshot: RiseShareSnapshot,
        credential: RiseShareCredential,
        configuration: ShareExtensionConfiguration,
    ) -> Self {
        Self {
            snapshot,
            configuration,
            state: RiseIsolated::new(RiseShareAPIClientState {
                credential,
                uploaded_media_by_idempotency_key: HashMap::new(),
                prepared_attachments_by_idempotency_key: HashMap::new(),
            }),
        }
    }

    async fn destinations_page(
        &self,
        query: Option<String>,
        payload: SharePayload,
        limit: i64,
        offset: i64,
        cx: &AsyncApp,
    ) -> anyhow::Result<ShareDestinationPage> {
        let trimmed_query = query.as_deref().map(trimming_whitespaces_and_newlines);
        let request = GetShareTargets {
            has_documents: Some(payload.has_documents()),
            has_link: Some(payload.has_link()),
            has_media: Some(payload.has_media()),
            limit: Some(ShareExtensionConfiguration::share_targets_page_limit().min(limit.max(1))),
            offset: (offset > 0).then_some(offset),
            q: trimmed_query.filter(|query| !query.is_empty()).map(str::to_owned),
        };

        let value = self
            .authenticated_call(
                rise_wire_methods::chat::SERVICE,
                rise_wire_methods::chat::GET_SHARE_TARGETS,
                Self::canon_body(&request)?,
                DEFAULT_CALL_TIMEOUT,
                None,
                cx,
            )
            .await?;
        let response: RiseShareTargetsWireResponse = Self::decode(&value)?;

        let mut destinations: Vec<ShareDestination> = response
            .targets
            .iter()
            .enumerate()
            .filter_map(|(index, target)| {
                if !target.can_send || target.chat_id.is_empty() {
                    return None;
                }
                let kind = Self::destination_kind(&target.r#type);
                Some(ShareDestination::new(
                    target.chat_id.clone(),
                    kind,
                    ShareDestinationTitle::resolve(kind, &target.title),
                    Self::safe_remote_url(Some(target.avatar_url.as_str())),
                    (!target.color_key.is_empty()).then(|| target.color_key.clone()),
                    target.topics_enabled,
                    target.can_send,
                    target.last_activity,
                    offset + index as i64,
                ))
            })
            .collect();
        destinations.sort_by(Self::destination_sort);
        let next = (response.has_more && response.next_offset > offset).then_some(response.next_offset);
        Ok(ShareDestinationPage { destinations, next_offset: next })
    }

    async fn topics_structure(
        &self,
        destination_id: String,
        payload: SharePayload,
        cx: &AsyncApp,
    ) -> anyhow::Result<RiseShareTopicStructure<ShareTopic>> {
        let mut request = GetShareTopics::new(destination_id.clone());
        request.has_documents = Some(payload.has_documents());
        request.has_link = Some(payload.has_link());
        request.has_media = Some(payload.has_media());
        let value = self
            .authenticated_call(
                rise_wire_methods::chat::SERVICE,
                rise_wire_methods::chat::GET_SHARE_TOPICS,
                Self::canon_body(&request)?,
                DEFAULT_CALL_TIMEOUT,
                None,
                cx,
            )
            .await?;
        let response: RiseShareTopicsWireResponse = Self::decode(&value)?;

        let sections = response
            .sections
            .iter()
            .filter_map(|section| {
                let section_id = trimming_whitespaces_and_newlines(&section.section_id);
                let chat_id = trimming_whitespaces_and_newlines(&section.chat_id);
                if section_id.is_empty() || !(chat_id.is_empty() || chat_id == destination_id) {
                    return None;
                }
                let created_at = if section.created_at > RiseIdentityCanon::TIMESTAMP_MILLISECOND_FLOOR {
                    section.created_at
                } else {
                    section.created_at.saturating_mul(RiseIdentityCanon::TIMESTAMP_MILLISECONDS_PER_SECOND)
                };
                Some(RiseShareSection {
                    id: section_id.to_owned(),
                    chat_id: if chat_id.is_empty() { destination_id.clone() } else { chat_id.to_owned() },
                    name: trimming_whitespaces_and_newlines(&section.name).to_owned(),
                    position: section.position,
                    created_at_ms: created_at,
                })
            })
            .collect();
        let topics = response
            .topics
            .iter()
            .filter_map(|topic| {
                let topic_id = trimming_whitespaces_and_newlines(&topic.topic_id);
                let chat_id = trimming_whitespaces_and_newlines(&topic.chat_id);
                if !topic.can_send
                    || !(chat_id.is_empty() || chat_id == destination_id)
                    || !(topic.is_general || !topic_id.is_empty())
                {
                    return None;
                }
                let section_id = trimming_whitespaces_and_newlines(&topic.section_id);
                Some(ShareTopic {
                    id: topic_id.to_owned(),
                    destination_id: destination_id.clone(),
                    title: if topic.is_general { ShareStrings::general() } else { topic.title.clone() },
                    icon: (!topic.icon.is_empty()).then(|| topic.icon.clone()),
                    section_id: (!section_id.is_empty()).then(|| section_id.to_owned()),
                    is_general: topic.is_general,
                    is_writable: topic.can_send,
                    sort_index: topic.position,
                })
            })
            .collect();
        Ok(RiseShareTopicStructure { sections, topics })
    }

    async fn send_operation(
        &self,
        request: ShareSendRequest,
        on_commit_started: ShareSendCommitStarted,
        progress: ShareSendProgressHandler,
        cx: &AsyncApp,
    ) -> anyhow::Result<()> {
        self.validate(&request, cx)?;
        let _active_credential = self.validated_credential(cx)?;
        Self::report(&progress, 0.02, cx);

        let files = self.prepared_attachments(&request, &progress, cx).await?;
        let byte_count = |attachment: &ShareAttachment| attachment.byte_count.unwrap_or(0).max(0);
        let total_file_bytes: i64 = files.iter().map(byte_count).sum();
        let key = request.idempotency_key;
        let mut slots = self.state.with(|state| {
            state.uploaded_media_by_idempotency_key.get(&key).cloned().unwrap_or_default()
        });
        let mut completed_file_bytes: i64 = files
            .iter()
            .filter(|attachment| slots.contains_key(&attachment.id))
            .map(byte_count)
            .sum();
        if completed_file_bytes > 0 && total_file_bytes > 0 {
            let cached_fraction = completed_file_bytes as f64 / total_file_bytes as f64;
            Self::report(&progress, 0.04 + (cached_fraction * 0.68), cx);
        }
        let Some(mut upload_chat_id) = request
            .selections
            .first()
            .map(|selection| selection.destination.id.clone())
            .filter(|id| !id.is_empty())
        else {
            return Err(ShareExtensionError::SendFailed(ShareStrings::invalid_selection()).into());
        };
        let mut denied_upload_chat_ids: HashSet<String> = HashSet::new();
        while !files.is_empty() {
            let mut denied = false;
            for attachment in &files {
                if slots.contains_key(&attachment.id) {
                    continue;
                }
                match self
                    .upload(attachment, &upload_chat_id, completed_file_bytes, total_file_bytes, &progress, cx)
                    .await
                {
                    Ok(media_item) => {
                        slots.insert(attachment.id, RiseShareJSONValue::Object(media_item));
                        let cached = slots.clone();
                        self.state.with(|state| {
                            state.uploaded_media_by_idempotency_key.insert(key, cached);
                        });
                        completed_file_bytes += byte_count(attachment);
                    }
                    Err(error) if error.is::<ShareUploadDestinationDenied>() => {
                        denied = true;
                        break;
                    }
                    Err(error) => return Err(error),
                }
            }
            if !denied {
                break;
            }
            denied_upload_chat_ids.insert(upload_chat_id.clone());
            slots.clear();
            self.state.with(|state| {
                state.uploaded_media_by_idempotency_key.remove(&key);
            });
            completed_file_bytes = 0;
            let Some(next) = request.selections.iter().find(|selection| {
                !selection.destination.id.is_empty() && !denied_upload_chat_ids.contains(&selection.destination.id)
            }) else {
                return Err(SharePartialDeliveryError {
                    delivered_count: 0,
                    failed_count: request.selections.len() as i64,
                    details: request
                        .selections
                        .iter()
                        .map(|selection| selection.destination.title.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                }
                .into());
            };
            upload_chat_id = next.destination.id.clone();
        }
        let media_items: Vec<RiseShareJSONValue> =
            files.iter().filter_map(|attachment| slots.get(&attachment.id).cloned()).collect();
        if media_items.len() != files.len() {
            return Err(ShareExtensionError::InvalidServerResponse.into());
        }

        Self::report(&progress, if files.is_empty() { 0.18 } else { 0.74 }, cx);
        let idempotency_key = request.idempotency_key.uuid_string().to_lowercase();
        let mut pending: Vec<ShareSelection> = request
            .selections
            .iter()
            .filter(|selection| !denied_upload_chat_ids.contains(&selection.destination.id))
            .cloned()
            .collect();
        let mut terminal_failures: Vec<String> = request
            .selections
            .iter()
            .filter(|selection| denied_upload_chat_ids.contains(&selection.destination.id))
            .map(|selection| selection.destination.title.clone())
            .collect();
        let mut delivered_keys: HashSet<String> = HashSet::new();

        for attempt in 0..MAXIMUM_SHARE_ATTEMPTS {
            if pending.is_empty() {
                break;
            }
            let body = Self::make_share_body(&pending, &request, &media_items, &idempotency_key)?;
            let value = self
                .authenticated_call(
                    rise_wire_methods::message::SERVICE,
                    rise_wire_methods::message::SHARE_PAYLOAD,
                    body,
                    SHARE_PAYLOAD_TIMEOUT,
                    Some(&on_commit_started),
                    cx,
                )
                .await?;
            let response: RiseSharePayloadWireResponse = Self::decode(&value)?;
            if response.idempotency_key != idempotency_key {
                return Err(ShareExtensionError::InvalidServerResponse.into());
            }

            if response.all_succeeded {
                for selection in &pending {
                    delivered_keys
                        .insert(Self::result_key(&selection.destination.id, selection.wire_topic_id()));
                }
                pending.clear();
                Self::report(&progress, 1.0, cx);
                break;
            }

            let mut results_by_key = HashMap::new();
            for result in &response.results {
                results_by_key
                    .entry(Self::result_key(&result.chat_id, result.topic_id.as_deref()))
                    .or_insert(result);
            }
            let mut retryable_selections: Vec<ShareSelection> = Vec::new();
            for selection in &pending {
                let key = Self::result_key(&selection.destination.id, selection.wire_topic_id());
                let Some(result) = results_by_key.get(&key) else {
                    return Err(ShareExtensionError::InvalidServerResponse.into());
                };
                if result.success {
                    delivered_keys.insert(key);
                    continue;
                }

                let message = Self::normalized_failure_message(Some(result.error.as_str()), &selection.destination.title);
                if result.retryable && attempt + 1 < MAXIMUM_SHARE_ATTEMPTS {
                    retryable_selections.push(selection.clone());
                } else {
                    terminal_failures.push(message);
                }
            }
            pending = retryable_selections;

            let send_base = if files.is_empty() { 0.18 } else { 0.74 };
            let send_fraction = (attempt + 1) as f64 / MAXIMUM_SHARE_ATTEMPTS as f64;
            Self::report(&progress, send_base + ((0.98 - send_base) * send_fraction), cx);
            if !pending.is_empty() {
                let delay = if attempt == 0 { FIRST_SHARE_RETRY_DELAY } else { LATER_SHARE_RETRY_DELAY };
                cx.background_executor().timer(delay).await;
            }
        }

        if !pending.is_empty() {
            terminal_failures.extend(pending.iter().map(|selection| selection.destination.title.clone()));
        }
        if !terminal_failures.is_empty() {
            let unique: BTreeSet<String> = terminal_failures.into_iter().collect();
            return Err(SharePartialDeliveryError {
                delivered_count: delivered_keys.len() as i64,
                failed_count: request.selections.len() as i64 - delivered_keys.len() as i64,
                details: unique.into_iter().collect::<Vec<_>>().join("\n"),
            }
            .into());
        }

        self.state.with(|state| {
            state.uploaded_media_by_idempotency_key.remove(&key);
            state.prepared_attachments_by_idempotency_key.remove(&key);
        });
        Self::report(&progress, 1.0, cx);
        Ok(())
    }

    fn report(progress: &ShareSendProgressHandler, fraction: f64, cx: &AsyncApp) {
        cx.update(|cx| progress(ShareSendProgress::new(fraction), cx));
    }

    async fn prepared_attachments(
        &self,
        request: &ShareSendRequest,
        progress: &ShareSendProgressHandler,
        cx: &AsyncApp,
    ) -> anyhow::Result<Vec<ShareAttachment>> {
        let originals = request.payload.file_attachments();
        if originals.is_empty() {
            return Ok(originals);
        }
        let key = request.idempotency_key;
        let mut prepared = self.state.with(|state| {
            state.prepared_attachments_by_idempotency_key.get(&key).cloned().unwrap_or_default()
        });
        let count = originals.len() as f64;
        for (index, attachment) in originals.iter().enumerate() {
            if prepared.contains_key(&attachment.id) {
                continue;
            }
            let base = index as f64 / count;
            let (sender, mut receiver) = mpsc::unbounded::<f64>();
            let on_progress: RiseOutgoingVideoTranscoderProgress = Arc::new(move |fraction: f64| {
                let _ = sender.unbounded_send(fraction);
            });
            let forwarded = progress.clone();
            let forwarder = cx.spawn(async move |cx| {
                while let Some(fraction) = receiver.next().await {
                    Self::report(&forwarded, 0.02 + (base + fraction / count) * 0.02, cx);
                }
            });
            let result = ShareOutgoingMediaPreparer::prepare(
                cx.background_executor().clone(),
                attachment.clone(),
                request.payload.session_directory_url.clone(),
                on_progress,
            )
            .await;
            drop(forwarder);
            prepared.insert(attachment.id, result?);
            let cached = prepared.clone();
            self.state.with(|state| {
                state.prepared_attachments_by_idempotency_key.insert(key, cached);
            });
        }
        Ok(originals.iter().filter_map(|original| prepared.get(&original.id).cloned()).collect())
    }

    fn validate(&self, request: &ShareSendRequest, cx: &AsyncApp) -> anyhow::Result<()> {
        if request.selections.is_empty() {
            return Err(ShareExtensionError::SendFailed(ShareStrings::select_at_least_one()).into());
        }
        let configuration = &self.configuration;
        if request.selections.len() as i64 > configuration.maximum_destination_count {
            return Err(ShareExtensionError::TooManyDestinations(configuration.maximum_destination_count).into());
        }
        if request.payload.file_attachments().len() as i64 > configuration.maximum_file_attachment_count {
            return Err(ShareExtensionError::TooManyItems(configuration.maximum_file_attachment_count).into());
        }
        let content = request.payload.textual_content();
        if content.chars().count() as i64 > configuration.maximum_content_length {
            return Err(ShareExtensionError::ContentTooLong(configuration.maximum_content_length).into());
        }
        if request.comment.chars().count() as i64 > configuration.maximum_comment_length {
            return Err(ShareExtensionError::CommentTooLong(configuration.maximum_comment_length).into());
        }
        if request.payload.file_attachments().is_empty() {
            let trimmed_comment = trimming_whitespaces_and_newlines(&request.comment);
            let assembled = if trimmed_comment.is_empty() {
                content
            } else if trimming_whitespaces_and_newlines(&content).is_empty() {
                trimmed_comment.to_owned()
            } else {
                format!("{trimmed_comment}\n{content}")
            };
            if assembled.chars().count() as i64 > configuration.maximum_content_length {
                return Err(ShareExtensionError::ContentTooLong(configuration.maximum_content_length).into());
            }
        }

        let mut destination_ids: HashSet<&str> = HashSet::new();
        for selection in &request.selections {
            if !selection.destination.is_writable || !destination_ids.insert(selection.destination.id.as_str()) {
                return Err(ShareExtensionError::SendFailed(ShareStrings::invalid_selection()).into());
            }
            if let Some(topic) = &selection.topic {
                if topic.destination_id != selection.destination.id || !topic.is_writable {
                    return Err(ShareExtensionError::SendFailed(ShareStrings::invalid_selection()).into());
                }
            }
        }
        self.validated_credential(cx)?;
        Ok(())
    }

    fn make_share_body(
        selections: &[ShareSelection],
        request: &ShareSendRequest,
        media_items: &[RiseShareJSONValue],
        idempotency_key: &str,
    ) -> anyhow::Result<RiseShareJSONValue> {
        let items = media_items
            .iter()
            .map(|item| serde_json::to_value(item).and_then(serde_json::from_value))
            .collect::<Result<Vec<share_payload::MediaItemsElement>, _>>()?;
        let comment = trimming_whitespaces_and_newlines(&request.comment);
        let mut payload = SharePayloadRequest::new(idempotency_key.to_owned());
        payload.comment = (!comment.is_empty()).then(|| comment.to_owned());
        payload.content = Some(request.payload.textual_content());
        payload.content_type = Some(
            items
                .first()
                .and_then(|item| item.media_type.clone())
                .unwrap_or_else(|| MessageContentType::Text.wire().to_owned()),
        );
        payload.destinations = Some(
            selections
                .iter()
                .map(|selection| share_payload::DestinationsElement {
                    chat_id: Some(selection.destination.id.clone()),
                    topic_id: selection.wire_topic_id().map(str::to_owned),
                })
                .collect(),
        );
        payload.media_items = Some(items);
        Self::canon_body(&payload)
    }

    fn result_key(chat_id: &str, topic_id: Option<&str>) -> String {
        let normalized_topic = topic_id.map(trimming_whitespaces_and_newlines).unwrap_or("");
        format!("{chat_id}#{normalized_topic}")
    }

    fn normalized_failure_message(value: Option<&str>, fallback: &str) -> String {
        let _ = value;
        fallback.to_owned()
    }

    async fn upload(
        &self,
        attachment: &ShareAttachment,
        chat_id: &str,
        already_uploaded_bytes: i64,
        total_payload_bytes: i64,
        progress: &ShareSendProgressHandler,
        cx: &AsyncApp,
    ) -> anyhow::Result<HashMap<String, RiseShareJSONValue>> {
        let Some(file_url) = attachment.local_file_url.clone() else {
            return Err(ShareExtensionError::FileUnavailable.into());
        };
        let metadata_url = file_url.clone();
        let length = cx
            .background_executor()
            .spawn(async move { std::fs::metadata(&metadata_url).map(|metadata| metadata.len()) })
            .await?;
        let file_size = i64::try_from(length).unwrap_or(i64::MAX);
        if file_size <= 0 {
            return Err(ShareExtensionError::FileUnavailable.into());
        }
        let maximum_bytes = self.configuration.maximum_file_byte_count.min(RiseLimits::upload_maximum_bytes(
            UPLOAD_FILE_CONTEXT,
            &attachment.mime_type,
            false,
        ));
        if file_size > maximum_bytes {
            return Err(ShareExtensionError::FileTooLarge.into());
        }

        let filename = attachment.original_file_name.clone().unwrap_or_else(|| {
            file_url.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
        });
        let initiate_body = RiseShareJSONValue::Object(HashMap::from([
            (upload_key::FILENAME.to_owned(), RiseShareJSONValue::String(filename.clone())),
            (message_key::CONTENT_TYPE.to_owned(), RiseShareJSONValue::String(attachment.mime_type.clone())),
            (upload_key::FILE_CONTEXT.to_owned(), RiseShareJSONValue::String(UPLOAD_FILE_CONTEXT.to_owned())),
            (upload_key::RESOURCE_ID.to_owned(), RiseShareJSONValue::String(chat_id.to_owned())),
            (upload_key::TOTAL_SIZE.to_owned(), RiseShareJSONValue::Integer(file_size)),
        ]));
        let initiated = match self.perform_authorized_json_request(PATH_INITIATE, METHOD_POST, initiate_body, &[], cx).await {
            Ok(initiated) => initiated,
            Err(error)
                if matches!(
                    error.downcast_ref::<RiseShareTransportError>(),
                    Some(RiseShareTransportError::HttpStatus(HTTP_FORBIDDEN, _))
                ) =>
            {
                return Err(ShareUploadDestinationDenied.into());
            }
            Err(error) => return Err(error),
        };
        let Some(upload_id) = initiated.first_string(&[upload_key::UPLOAD_ID]).filter(|id| !id.is_empty()) else {
            return Err(ShareExtensionError::InvalidServerResponse.into());
        };
        let requested_chunk_size =
            initiated.first_integer(&[upload_key::CHUNK_SIZE]).unwrap_or(rise_upload::SLAB_DEFAULT_BYTES as i64);
        let chunk_size = requested_chunk_size
            .max(rise_upload::SLAB_MINIMUM_BYTES as i64)
            .min(rise_upload::SLAB_MAXIMUM_BYTES as i64);

        let open_url = file_url.clone();
        let mut handle = Some(
            cx.background_executor().spawn(async move { File::open(&open_url) }).await?,
        );
        let mut server_accepted_bytes: i64 = 0;
        let mut abort_on_cancel = ShareUploadAbortOnCancel(self.abort_request(&upload_id, cx));

        let result: anyhow::Result<HashMap<String, RiseShareJSONValue>> = async {
            let mut offset: i64 = 0;
            while offset < file_size {
                let desired = chunk_size.min(file_size - offset) as u64;
                let Some(mut file) = handle.take() else {
                    return Err(ShareExtensionError::FileUnavailable.into());
                };
                let (file, read) = cx
                    .background_executor()
                    .spawn(async move {
                        let mut chunk = Vec::new();
                        let read = (&mut file).take(desired).read_to_end(&mut chunk).map(|_| chunk);
                        (file, read)
                    })
                    .await;
                handle = Some(file);
                let chunk = read?;
                if chunk.is_empty() {
                    return Err(ShareExtensionError::FileUnavailable.into());
                }
                let next_offset = offset + chunk.len() as i64;
                let last_slab = (next_offset >= file_size).to_string();
                self.perform_authorized_data_request(
                    PATH_SLAB,
                    METHOD_PUT,
                    chunk,
                    CONTENT_TYPE_OCTET_STREAM,
                    &[(HEADER_UPLOAD_ID, upload_id.as_str()), (HEADER_LAST_SLAB, last_slab.as_str())],
                    cx,
                )
                .await?;
                offset = next_offset;
                server_accepted_bytes = offset;
                if server_accepted_bytes >= file_size {
                    abort_on_cancel.disarm();
                }

                let uploaded = already_uploaded_bytes + offset;
                let fraction =
                    if total_payload_bytes > 0 { uploaded as f64 / total_payload_bytes as f64 } else { 1.0 };
                Self::report(progress, 0.04 + (fraction.min(1.0) * 0.68), cx);
            }

            let maximum_attempts = rise_upload::RETRY_COMPLETE_MAXIMUM_ATTEMPTS;
            let mut completed = None;
            let mut last_complete_error: Option<anyhow::Error> = None;
            for attempt in 0..maximum_attempts {
                match self
                    .perform_authorized_json_request(
                        PATH_COMPLETE,
                        METHOD_POST,
                        RiseShareJSONValue::Object(HashMap::new()),
                        &[(HEADER_UPLOAD_ID, upload_id.as_str())],
                        cx,
                    )
                    .await
                {
                    Ok(value) => {
                        completed = Some(value);
                        break;
                    }
                    Err(error) => {
                        last_complete_error = Some(error);
                        if attempt >= maximum_attempts - 1 {
                            break;
                        }
                        let delays = rise_upload::RETRY_COMPLETE_DELAYS_MS;
                        let milliseconds = delays
                            .get((attempt as usize).min(delays.len().saturating_sub(1)))
                            .copied()
                            .unwrap_or(0);
                        cx.background_executor().timer(Duration::from_millis(milliseconds)).await;
                    }
                }
            }
            let Some(completed) = completed else {
                return Err(last_complete_error.unwrap_or_else(|| ShareExtensionError::InvalidServerResponse.into()));
            };
            Self::make_media_item(&completed, attachment, file_size, &filename)
        }
        .await;
        drop(handle);
        abort_on_cancel.disarm();

        match result {
            Ok(item) => Ok(item),
            Err(error) => {
                if server_accepted_bytes < file_size {
                    self.abort_upload_best_effort(&upload_id, cx).await;
                }
                Err(error)
            }
        }
    }

    fn make_media_item(
        completed: &HashMap<String, RiseShareJSONValue>,
        attachment: &ShareAttachment,
        file_size: i64,
        filename: &str,
    ) -> anyhow::Result<HashMap<String, RiseShareJSONValue>> {
        let Some(file_url) =
            completed.first_string(&[message_key::FILE_URL, file_key::URL]).filter(|url| !url.is_empty())
        else {
            return Err(ShareExtensionError::InvalidServerResponse.into());
        };
        let media_type = match attachment.kind {
            ShareAttachmentKind::Image => MediaKind::Image.wire(),
            ShareAttachmentKind::Video => MediaKind::Video.wire(),
            ShareAttachmentKind::Audio => MediaKind::Audio.wire(),
            ShareAttachmentKind::File => MediaKind::Document.wire(),
            ShareAttachmentKind::Link | ShareAttachmentKind::Text => MessageContentType::Text.wire(),
        };

        let mut item: HashMap<String, RiseShareJSONValue> = HashMap::from([
            (message_key::MEDIA_TYPE.to_owned(), RiseShareJSONValue::String(media_type.to_owned())),
            (message_key::FILE_URL.to_owned(), RiseShareJSONValue::String(file_url)),
            (message_key::FILE_NAME.to_owned(), RiseShareJSONValue::String(filename.to_owned())),
            (message_key::FILE_SIZE.to_owned(), RiseShareJSONValue::Integer(file_size)),
            (message_key::MIME_TYPE.to_owned(), RiseShareJSONValue::String(attachment.mime_type.clone())),
        ]);
        let passthrough_keys = [
            message_key::FILE_ID,
            message_key::THUMBNAIL_URL,
            message_key::WIDTH,
            message_key::HEIGHT,
            message_key::DURATION,
            message_key::BITRATE,
            message_key::FPS,
            message_key::CODEC,
            message_key::WAVEFORM,
        ];
        for key in passthrough_keys {
            if let Some(value) = completed.get(key) {
                item.insert(key.to_owned(), value.clone());
            }
        }
        if !item.contains_key(message_key::FILE_ID) {
            if let Some(file_key_value) = completed.first_string(&[upload_key::FILE_KEY]) {
                let stem = Path::new(&file_key_value)
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !stem.is_empty() {
                    item.insert(message_key::FILE_ID.to_owned(), RiseShareJSONValue::String(stem));
                }
            }
        }
        Ok(item)
    }

    async fn abort_upload_best_effort(&self, upload_id: &str, cx: &AsyncApp) {
        let Some(request) = self.abort_request(upload_id, cx) else {
            return;
        };
        let _ = RiseAccountRuntime::current().spawn(Self::send_abort(request)).await;
    }

    async fn send_abort(request: URLRequest) {
        let _ = URLSession::shared().data_for(request).await;
    }

    fn abort_request(&self, upload_id: &str, cx: &AsyncApp) -> Option<URLRequest> {
        let active = self.validated_credential(cx).ok()?;
        let url = self.api_url(PATH_ABORT).ok()?;
        let body = serde_json::to_vec(&RiseShareJSONValue::Object(HashMap::new())).ok()?;
        let mut request = URLRequest::new(url);
        request.http_method = METHOD_POST.to_owned();
        request.timeout_interval = ABORT_REQUEST_TIMEOUT;
        request.http_body = Some(body);
        request.set_value(ACCEPT_JSON, HEADER_ACCEPT);
        request.set_value(CONTENT_TYPE_JSON, HEADER_CONTENT_TYPE);
        request.set_value(&format!("Bearer {}", active.access_token), HEADER_AUTHORIZATION);
        request.set_value(upload_id, HEADER_UPLOAD_ID);
        Some(request)
    }

    /// turns the canon request into the json value the layer sends
    fn canon_body<Request: Serialize>(request: &Request) -> anyhow::Result<RiseShareJSONValue> {
        Ok(serde_json::from_value(serde_json::to_value(request)?)?)
    }

    fn validated_credential(&self, cx: &AsyncApp) -> anyhow::Result<RiseShareCredential> {
        let Some(latest) = cx.update(ShareExtensionDependencyResolver::session_credential) else {
            return Err(ShareExtensionError::AuthenticationUnavailable.into());
        };
        ShareExtensionDependencyResolver::validate_account(&self.snapshot, &latest)?;
        let stored = latest.clone();
        self.state.with(|state| state.credential = stored);
        Ok(latest)
    }

    async fn authenticated_call(
        &self,
        service: &str,
        method: &str,
        body: RiseShareJSONValue,
        timeout: Duration,
        on_commit_started: Option<&ShareSendCommitStarted>,
        cx: &AsyncApp,
    ) -> anyhow::Result<RiseShareJSONValue> {
        let mut active = self.validated_credential(cx)?;
        if Self::access_token_expires_soon(&active.access_token) {
            active = self.credential_after_unauthorized_attempt(active, cx).await?;
        }

        match self
            .web_socket_call(service, method, &body, &active.access_token, timeout, on_commit_started, cx)
            .await
        {
            Err(error)
                if error
                    .downcast_ref::<RiseShareTransportError>()
                    .is_some_and(RiseShareTransportError::is_authentication_failure) =>
            {
                let refreshed = self.credential_after_unauthorized_attempt(active, cx).await?;
                self.web_socket_call(service, method, &body, &refreshed.access_token, timeout, on_commit_started, cx)
                    .await
            }
            other => other,
        }
    }

    async fn refresh_credential(
        &self,
        current: RiseShareCredential,
        cx: &AsyncApp,
    ) -> anyhow::Result<RiseShareCredential> {
        let _ = current;
        let refresh = cx.update(sai_auth_refresh_access_token);
        let refreshed = refresh.await;
        let Some(resolved) = cx.update(ShareExtensionDependencyResolver::session_credential) else {
            return Err(ShareExtensionError::SessionExpired.into());
        };
        refreshed?;
        ShareExtensionDependencyResolver::validate_account(&self.snapshot, &resolved)?;
        let stored = resolved.clone();
        self.state.with(|state| state.credential = stored);
        Ok(resolved)
    }

    async fn credential_after_unauthorized_attempt(
        &self,
        attempted: RiseShareCredential,
        cx: &AsyncApp,
    ) -> anyhow::Result<RiseShareCredential> {
        let latest = self.validated_credential(cx)?;
        if latest.access_token != attempted.access_token
            || latest.refresh_token != attempted.refresh_token
            || latest.session_id != attempted.session_id
        {
            return Ok(latest);
        }
        self.refresh_credential(attempted, cx).await
    }

    fn access_token_expires_soon(token: &str) -> bool {
        sai_auth_is_expired(token, ACCESS_TOKEN_EXPIRY_LEEWAY_SECONDS)
    }

    async fn perform_authorized_json_request(
        &self,
        path: &str,
        method: &str,
        body: RiseShareJSONValue,
        headers: &[(&str, &str)],
        cx: &AsyncApp,
    ) -> anyhow::Result<HashMap<String, RiseShareJSONValue>> {
        let data = serde_json::to_vec(&body)?;
        let response_data =
            self.perform_authorized_data_request(path, method, data, CONTENT_TYPE_JSON, headers, cx).await?;
        let root: RiseShareJSONValue = serde_json::from_slice(&response_data)?;
        let Some(root_object) = root.object_value() else {
            return Err(ShareExtensionError::InvalidServerResponse.into());
        };
        if root_object.first_bool(&[auth_key::SUCCESS]) == Some(false) {
            return Err(RiseShareTransportError::Server {
                code: root_object.first_string(&[wire_key::CODE]),
                message: Self::json_error_message(root_object).unwrap_or_else(ShareStrings::send_failed),
            }
            .into());
        }
        if let Some(data_object) = root_object.get(wire_key::DATA).and_then(RiseShareJSONValue::object_value) {
            return Ok(data_object.clone());
        }
        Ok(root_object.clone())
    }

    async fn perform_authorized_data_request(
        &self,
        path: &str,
        method: &str,
        body: Vec<u8>,
        content_type: &str,
        headers: &[(&str, &str)],
        cx: &AsyncApp,
    ) -> anyhow::Result<Vec<u8>> {
        let mut active = self.validated_credential(cx)?;
        if Self::access_token_expires_soon(&active.access_token) {
            active = self.credential_after_unauthorized_attempt(active, cx).await?;
        }

        let make_request = |token: &str| -> anyhow::Result<URLRequest> {
            let mut request = URLRequest::new(self.api_url(path)?);
            request.http_method = method.to_owned();
            request.timeout_interval = DATA_REQUEST_TIMEOUT.min(RESOURCE_TIMEOUT);
            request.http_body = Some(body.clone());
            request.set_value(ACCEPT_JSON, HEADER_ACCEPT);
            request.set_value(content_type, HEADER_CONTENT_TYPE);
            request.set_value(&format!("Bearer {token}"), HEADER_AUTHORIZATION);
            for (key, value) in headers {
                request.set_value(value, key);
            }
            Ok(request)
        };

        let (mut data, mut response) = Self::session_data(make_request(&active.access_token)?).await?;
        if response.status_code == HTTP_UNAUTHORIZED {
            active = self.credential_after_unauthorized_attempt(active, cx).await?;
            (data, response) = Self::session_data(make_request(&active.access_token)?).await?;
        }
        if !(200..300).contains(&response.status_code) {
            return Err(RiseShareTransportError::HttpStatus(response.status_code, Self::server_message(&data)).into());
        }
        Ok(data)
    }

    async fn session_data(request: URLRequest) -> anyhow::Result<(Vec<u8>, HTTPURLResponse)> {
        Ok(Self::on_account_runtime(async move { URLSession::shared().data_for(request).await }).await??)
    }

    async fn on_account_runtime<Output: Send + 'static>(
        work: impl std::future::Future<Output = Output> + Send + 'static,
    ) -> anyhow::Result<Output> {
        let task = RiseAccountRuntime::current().spawn(work);
        let _abort_on_drop = ShareRuntimeTaskAbortOnDrop(task.abort_handle());
        task.await.map_err(|_| anyhow::Error::from(CancellationError))
    }

    fn api_url(&self, path: &str) -> anyhow::Result<Url> {
        let base = trimming_characters(self.snapshot.api_base_url.absolute_string(), |c| c == '/');
        let suffix = if path.starts_with('/') { path.to_owned() } else { format!("/{path}") };
        Url::from_string(&format!("{base}{suffix}")).ok_or_else(|| RiseShareTransportError::InvalidURL.into())
    }

    #[allow(clippy::too_many_arguments)]
    async fn web_socket_call(
        &self,
        service: &str,
        method: &str,
        body: &RiseShareJSONValue,
        access_token: &str,
        timeout: Duration,
        on_commit_started: Option<&ShareSendCommitStarted>,
        cx: &AsyncApp,
    ) -> anyhow::Result<RiseShareJSONValue> {
        let request_id = SaiWsEnvelopeBuilder::make_outbound_id();
        let timestamp_seconds = SaiWsEnvelopeBuilder::make_timestamp_seconds(request_id);
        let data_value = serde_json::to_value(body).map_err(|_| RiseShareTransportError::InvalidResponse)?;
        let wire_text = SaiWsEnvelopeBuilder::json_string(service, method, &data_value, request_id, timestamp_seconds)
            .map_err(|_| RiseShareTransportError::InvalidResponse)?;

        let url = self.snapshot.web_socket_url.absolute_string().to_owned();
        let subprotocol = format!("Bearer.{access_token}");
        let socket = Self::on_account_runtime(Self::open_and_ping(url, subprotocol)).await??;
        let mut socket = ShareWebSocketCloseOnDrop(Some(socket));
        if let Some(on_commit_started) = on_commit_started {
            cx.update(|cx| on_commit_started(cx));
        }
        let Some(opened) = socket.0.take() else {
            return Err(RiseShareTransportError::InvalidResponse.into());
        };
        let object = Self::on_account_runtime(Self::send_and_receive(opened, wire_text, request_id, timeout)).await??;
        if let Some(error) = Self::gateway_error(&object) {
            return Err(error.into());
        }
        let Some(payload) = object.first_value(&[wire_key::DATA, wire_key::RESULT]) else {
            return Err(RiseShareTransportError::InvalidResponse.into());
        };
        Ok(payload.clone())
    }

    async fn open_and_ping(url: String, subprotocol: String) -> anyhow::Result<ShareWebSocketStream> {
        let mut request = url.as_str().into_client_request().map_err(|_| URLError::new(URLErrorCode::BadURL))?;
        let protocol = HeaderValue::from_str(&subprotocol).map_err(|_| URLError::new(URLErrorCode::BadURL))?;
        request.headers_mut().insert(SEC_WEBSOCKET_PROTOCOL, protocol);
        let maximum = usize::try_from(LIMITS_MAXIMUM_INBOUND_FRAME_BYTES).unwrap_or(usize::MAX);
        let config = WebSocketConfig::default().max_message_size(Some(maximum)).max_frame_size(Some(maximum));
        let opening = Self::open_socket(request, config);
        match tokio::time::timeout(SOCKET_IDLE_TIMEOUT, opening).await {
            Ok(opened) => Ok(opened?),
            Err(_) => Err(URLError::new(URLErrorCode::TimedOut).into()),
        }
    }

    async fn open_socket(
        request: tokio_tungstenite::tungstenite::handshake::client::Request,
        config: WebSocketConfig,
    ) -> Result<ShareWebSocketStream, URLError> {
        let (mut socket, _) = crate::core::engine_bridge::network_transport_config::connect_websocket(request, Some(config))
            .await
            .map_err(|error| Self::socket_url_error(&error, true))?;
        socket.send(Message::Ping(Default::default())).await.map_err(|error| Self::socket_url_error(&error, false))?;
        loop {
            match socket.next().await {
                Some(Ok(Message::Pong(_))) => return Ok(socket),
                Some(Ok(_)) => continue,
                Some(Err(error)) => return Err(Self::socket_url_error(&error, false)),
                None => return Err(URLError::new(URLErrorCode::NetworkConnectionLost)),
            }
        }
    }

    async fn send_and_receive(
        mut socket: ShareWebSocketStream,
        wire_text: String,
        request_id: u64,
        timeout: Duration,
    ) -> anyhow::Result<HashMap<String, RiseShareJSONValue>> {
        let answer = Self::exchange(&mut socket, wire_text, request_id, timeout).await;
        let _ = socket.close(Some(CloseFrame { code: CloseCode::Normal, reason: Default::default() })).await;
        answer
    }

    async fn exchange(
        socket: &mut ShareWebSocketStream,
        wire_text: String,
        request_id: u64,
        timeout: Duration,
    ) -> anyhow::Result<HashMap<String, RiseShareJSONValue>> {
        socket.send(Message::text(wire_text)).await.map_err(|error| Self::socket_url_error(&error, false))?;
        let deadline = RiseClock::monotonic() + timeout;
        loop {
            let remaining = deadline.saturating_sub(RiseClock::monotonic());
            if remaining.is_zero() {
                return Err(RiseShareTransportError::Timeout.into());
            }
            let message = match tokio::time::timeout(remaining.max(MINIMUM_RECEIVE_TIMEOUT), socket.next()).await {
                Err(_) => return Err(RiseShareTransportError::Timeout.into()),
                Ok(None) => return Err(URLError::new(URLErrorCode::NetworkConnectionLost).into()),
                Ok(Some(Err(error))) => return Err(Self::socket_url_error(&error, false).into()),
                Ok(Some(Ok(message))) => message,
            };
            let root: Option<RiseShareJSONValue> = match &message {
                Message::Text(text) => serde_json::from_str(text.as_str()).ok(),
                Message::Binary(data) => serde_json::from_slice(data).ok(),
                Message::Close(_) => return Err(URLError::new(URLErrorCode::NetworkConnectionLost).into()),
                _ => continue,
            };
            let Some(RiseShareJSONValue::Object(object)) = root else {
                continue;
            };
            let inbound_id = object.first_integer(&[chat_key::REQUEST_ID]).and_then(|id| u64::try_from(id).ok());
            if inbound_id != Some(request_id) {
                continue;
            }
            return Ok(object);
        }
    }

    fn socket_url_error(error: &tokio_tungstenite::tungstenite::Error, connecting: bool) -> URLError {
        use tokio_tungstenite::tungstenite::Error as SocketError;
        let code = match error {
            SocketError::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => URLErrorCode::TimedOut,
            SocketError::Url(_) => URLErrorCode::BadURL,
            SocketError::Tls(_) => URLErrorCode::SecureConnectionFailed,
            SocketError::Io(_) if connecting => URLErrorCode::CannotConnectToHost,
            _ => URLErrorCode::NetworkConnectionLost,
        };
        URLError::new(code)
    }

    fn gateway_error(object: &HashMap<String, RiseShareJSONValue>) -> Option<RiseShareTransportError> {
        let r#type = object.first_string(&[wire_key::FRAME_TYPE]).map(|value| value.to_lowercase()).unwrap_or_default();
        let data_object = object.get(wire_key::DATA).and_then(RiseShareJSONValue::object_value);
        let has_nonempty_error = match object.get(wire_key::ERROR) {
            None | Some(RiseShareJSONValue::Null) => false,
            Some(RiseShareJSONValue::String(string)) => !trimming_whitespaces_and_newlines(string).is_empty(),
            Some(RiseShareJSONValue::Object(error_object)) => !error_object.is_empty(),
            Some(_) => true,
        };
        let is_failure = r#type.contains(wire_key::ERROR)
            || object.first_bool(&[auth_key::SUCCESS]) == Some(false)
            || data_object.and_then(|data| data.first_bool(&[auth_key::SUCCESS])) == Some(false)
            || has_nonempty_error;
        if !is_failure {
            return None;
        }

        let message = Self::json_error_message(object)
            .or_else(|| data_object.and_then(Self::json_error_message))
            .unwrap_or_else(ShareStrings::send_failed);
        let code = object
            .first_string(&[wire_key::CODE, message_key::ERROR_CODE])
            .or_else(|| data_object.and_then(|data| data.first_string(&[wire_key::CODE, message_key::ERROR_CODE])));
        Some(RiseShareTransportError::Server {
            code,
            message: character_prefix(&message, SERVER_MESSAGE_LIMIT).to_owned(),
        })
    }

    fn decode<T: DeserializeOwned>(value: &RiseShareJSONValue) -> Result<T, ShareExtensionError> {
        serde_json::to_value(value)
            .ok()
            .and_then(|json| serde_json::from_value(json).ok())
            .ok_or(ShareExtensionError::InvalidServerResponse)
    }

    fn destination_kind(raw: &str) -> ShareDestinationKind {
        match ChatType::matching(raw) {
            Some(ChatType::Favourites) => ShareDestinationKind::Saved,
            Some(ChatType::Group) => ShareDestinationKind::Group,
            Some(ChatType::Channel) => ShareDestinationKind::Channel,
            Some(ChatType::Direct) | None => ShareDestinationKind::Direct,
        }
    }

    fn destination_sort(lhs: &ShareDestination, rhs: &ShareDestination) -> std::cmp::Ordering {
        if lhs.last_activity_at_milliseconds != rhs.last_activity_at_milliseconds {
            return rhs.last_activity_at_milliseconds.cmp(&lhs.last_activity_at_milliseconds);
        }
        compare_case_insensitive(&lhs.title, &rhs.title)
    }

    fn safe_remote_url(raw: Option<&str>) -> Option<Url> {
        let url = Url::from_string(raw?)?;
        let scheme = url.scheme()?.to_lowercase();
        if scheme == "https" || scheme == "http" { Some(url) } else { None }
    }

    fn server_message(data: &[u8]) -> String {
        let object = serde_json::from_slice::<RiseShareJSONValue>(data)
            .ok()
            .and_then(|value| value.object_value().cloned());
        let Some(object) = object else {
            return std::str::from_utf8(data)
                .map(|text| character_prefix(text, SERVER_MESSAGE_LIMIT).to_owned())
                .unwrap_or_default();
        };
        let message = Self::json_error_message(&object).unwrap_or_else(ShareStrings::send_failed);
        character_prefix(&message, SERVER_MESSAGE_LIMIT).to_owned()
    }

    fn json_error_message(object: &HashMap<String, RiseShareJSONValue>) -> Option<String> {
        for key in [wire_key::ERROR, wire_key::MESSAGE, support_key::DETAIL] {
            let Some(value) = object.get(key) else {
                continue;
            };
            if let Some(string) = value.string_value().filter(|string| !string.is_empty()) {
                return Some(string);
            }
            if let Some(string) = value
                .object_value()
                .and_then(Self::json_error_message)
                .filter(|string| !string.is_empty())
            {
                return Some(string);
            }
        }
        None
    }
}

impl ShareExtensionRemoteProviding for RiseShareAPIClient {
    fn destinations(
        &self,
        query: Option<String>,
        payload: SharePayload,
        limit: i64,
        offset: i64,
        cx: AsyncApp,
    ) -> LocalBoxFuture<'_, anyhow::Result<ShareDestinationPage>> {
        async move { self.destinations_page(query, payload, limit, offset, &cx).await }.boxed_local()
    }

    fn topics(
        &self,
        destination_id: String,
        payload: SharePayload,
        cx: AsyncApp,
    ) -> LocalBoxFuture<'_, anyhow::Result<RiseShareTopicStructure<ShareTopic>>> {
        async move { self.topics_structure(destination_id, payload, &cx).await }.boxed_local()
    }
}

impl ShareContentSending for RiseShareAPIClient {
    fn send(
        &self,
        request: ShareSendRequest,
        on_commit_started: ShareSendCommitStarted,
        progress: ShareSendProgressHandler,
        cx: AsyncApp,
    ) -> LocalBoxFuture<'_, anyhow::Result<()>> {
        async move { self.send_operation(request, on_commit_started, progress, &cx).await }.boxed_local()
    }

    fn discard_cached_uploads(&self, idempotency_key: Uuid) -> LocalBoxFuture<'_, ()> {
        async move {
            self.state.with(|state| {
                state.uploaded_media_by_idempotency_key.remove(&idempotency_key);
                state.prepared_attachments_by_idempotency_key.remove(&idempotency_key);
            });
        }
        .boxed_local()
    }
}
