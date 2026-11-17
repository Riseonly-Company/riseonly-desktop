use std::fmt;
use std::path::PathBuf;
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::FutureExt;
use futures::future::{BoxFuture, Either, select};
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_canon::rise_request::message::SharePayload;
use rise_canon::rise_response::message::SharePayload as SharePayloadResponse;
use rise_canon::rise_wire_methods;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_chunked_upload::{
    RiseChunkedUploadProgressHandler, RiseChunkedUploadRequest, RiseChunkedUploadTransferProgress,
};
use rise_i18n::tr;
use rise_platform::file_manager::FileManager;
use serde_json::{Map, Value};

use crate::core::lib::sai::modules::sai_ws::sai_ws_chunked_file_upload::{SaiWsChunkedFileUpload, SaiWsUploadOnProgress};
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::core::rise_conversation_rpc::RiseConversationEndpoints;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::chat::engine::messages::rise_message_media_upload_pipeline::RiseOutgoingMediaUploadHints;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressorResult;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::share_extension::send_handoff::share_send_handoff_models::RiseStoredSharePayloadReply;

const UPLOAD_FILE_CONTEXT: &str = "chat";

pub type ShareSendHandoffUploadProgress = Box<dyn Fn(f64) + Send + Sync>;

/// upload transfer progress as the chunked uploader reports it
pub type ShareSendHandoffTransferProgress = Box<dyn Fn(RiseChunkedUploadTransferProgress) + Send + Sync>;

/// runs on main thread right before the request goes out, if it fails the send stops
pub type ShareSendHandoffCommitStarted = Box<dyn FnOnce(&mut App) -> anyhow::Result<()>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShareSendHandoffTimeoutError;

impl ShareSendHandoffTimeoutError {
    fn error_description(&self) -> Option<String> {
        Some(tr("share_send_handoff_timeout"))
    }
}

impl fmt::Display for ShareSendHandoffTimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for ShareSendHandoffTimeoutError {}

pub struct ShareSendHandoffActionsStore {}

impl ShareSendHandoffActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upload_attachment(
        &mut self,
        file_url: PathBuf,
        filename: String,
        content_type: String,
        chat_id: String,
        expected_account_id: String,
        expected_account_epoch: Option<String>,
        on_progress: ShareSendHandoffUploadProgress,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Map<String, Value>>> {
        let on_progress: SaiWsUploadOnProgress = Rc::new(move |fraction: f64, _: &mut App| on_progress(fraction));
        SaiWsChunkedFileUpload::upload(
            file_url,
            filename,
            content_type,
            UPLOAD_FILE_CONTEXT.to_owned(),
            Some(chat_id),
            Some(expected_account_id),
            expected_account_epoch,
            Some(on_progress),
            None,
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upload_media(
        &mut self,
        original: RiseOutgoingMediaFile,
        compressed: OutgoingMediaCompressorResult,
        filename: String,
        chat_id: String,
        expected_account_id: String,
        expected_account_epoch: Option<String>,
        on_progress: ShareSendHandoffTransferProgress,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Map<String, Value>>> {
        cx.spawn(async move |_, cx| {
            Self::upload_media_operation(
                original,
                compressed,
                filename,
                chat_id,
                expected_account_id,
                expected_account_epoch,
                on_progress,
                cx,
            )
            .await
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn upload_media_operation(
        original: RiseOutgoingMediaFile,
        compressed: OutgoingMediaCompressorResult,
        filename: String,
        chat_id: String,
        expected_account_id: String,
        expected_account_epoch: Option<String>,
        on_progress: ShareSendHandoffTransferProgress,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Map<String, Value>> {
        let mut cancellation = ShareSendUploadCancellation::default();
        let cancelled = cancellation.cancelled.clone();
        let is_cancelled = Arc::new(move || -> BoxFuture<'static, bool> {
            let cancelled = cancelled.load(Ordering::Acquire);
            async move { cancelled }.boxed()
        });
        let hints = RiseOutgoingMediaUploadHints::make(&compressed, &original, None, is_cancelled).await;
        let _generated_thumbnail = ShareSendGeneratedThumbnail(
            hints.as_ref().and_then(|hints| hints.generated_thumbnail_url.clone()),
        );
        RiseConversationUploadSupport::prepare_authentication(
            Some(expected_account_id.clone()),
            expected_account_epoch.clone(),
            cx,
        )
        .await?;
        let client = cx.update(|cx| {
            RiseConversationUploadSupport::make_client(
                Some(expected_account_id.clone()),
                expected_account_epoch.clone(),
                None,
                cx,
            )
        });
        let Some(file_url) = compressed.url.to_file_path() else {
            return Err(CancellationError.into());
        };
        let mut request = RiseChunkedUploadRequest::new(
            file_url,
            filename,
            compressed.content_type.clone(),
            UPLOAD_FILE_CONTEXT.to_owned(),
        );
        request.resource_id = Some(chat_id);
        request.video_width = hints.as_ref().and_then(|hints| hints.width);
        request.video_height = hints.as_ref().and_then(|hints| hints.height);
        request.video_duration = hints.as_ref().and_then(|hints| hints.duration);
        request.video_thumbnail_file_url = hints.as_ref().and_then(|hints| hints.thumbnail_url.clone());
        let progress: RiseChunkedUploadProgressHandler = Arc::from(on_progress);
        let upload = RiseAccountRuntime::current()
            .spawn(async move { client.upload(request, None, None, Some(progress)).await });
        cancellation.upload = Some(upload.abort_handle());
        let result = upload.await.map_err(|_| CancellationError)??;
        Ok(RiseConversationUploadSupport::completion_dictionary(&result, &compressed))
    }

    pub fn share_payload_action(
        &mut self,
        request: SharePayload,
        defer_until_connected: bool,
        on_commit_started: ShareSendHandoffCommitStarted,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseStoredSharePayloadReply>> {
        let _ = defer_until_connected;
        let deadline = cx.background_executor().timer(RiseConversationEndpoints::share_payload().timeout);
        cx.spawn(async move |_, cx| {
            let call = pin!(Self::share_payload_call(request, on_commit_started, cx));
            match select(call, deadline).await {
                Either::Left((result, _)) => result,
                Either::Right(((), _)) => Err(ShareSendHandoffTimeoutError.into()),
            }
        })
    }

    async fn share_payload_call(
        request: SharePayload,
        on_commit_started: ShareSendHandoffCommitStarted,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<RiseStoredSharePayloadReply> {
        RiseConversationActionExecutor::require_domain(cx).await?;
        cx.update(on_commit_started)?;
        let response: SharePayloadResponse = RiseConversationActionExecutor::call(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::SHARE_PAYLOAD,
            &request,
            cx,
        )
        .await?;
        Ok(RiseStoredSharePayloadReply::from_canon(&response))
    }
}

#[derive(Default)]
struct ShareSendUploadCancellation {
    cancelled: Arc<AtomicBool>,
    upload: Option<tokio::task::AbortHandle>,
}

impl Drop for ShareSendUploadCancellation {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(upload) = self.upload.take() {
            upload.abort();
        }
    }
}

struct ShareSendGeneratedThumbnail(Option<PathBuf>);

impl Drop for ShareSendGeneratedThumbnail {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = FileManager.remove_item(path);
        }
    }
}
