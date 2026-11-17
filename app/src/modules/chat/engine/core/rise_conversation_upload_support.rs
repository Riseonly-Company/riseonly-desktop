use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use gpui::{App, AsyncApp};
use reqwest::header::{ACCEPT, AUTHORIZATION};
use rise_canon::RiseWireKey;
use rise_canon::rise_key::upload;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_chunked_upload::{
    RiseChunkedUploadAuthorize, RiseChunkedUploadClient, RiseChunkedUploadConfiguration, RiseChunkedUploadResult,
    RiseChunkedUploadStep,
};
use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode, URLRequest};
use rise_engine::RiseJSONValue;
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;
use serde_json::{Map, Value};

use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::core::media::remote_media_disk_cache::RemoteMediaDiskCache;
use crate::core::media::rise_media_transfer_hub::{RiseMediaCacheIdentity, RiseMediaTransferHub};
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::stores::http::http_service::HttpService;
use crate::core::ui::clever_image_disk_cache::CleverImageDiskCache;
use crate::modules::auth::services::auth_token_refresh::AuthTokenRefresh;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::composer::remote_media_url_type_hooks::RemoteMediaURL;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressorResult;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;

const ACCEPT_JSON: &str = "application/json";
const DOCUMENT_MEDIA_TYPE: &str = "document";
const FALLBACK_MEDIA_STEM: &str = "media";
const FALLBACK_FILE_NAME: &str = "file";
const FALLBACK_VIDEO_EXTENSION: &str = "mp4";
const SENT_VIDEO_STAGING_PREFIX: &str = "rise-sent-video-";

pub enum RiseConversationUploadSupport {}

impl RiseConversationUploadSupport {
    pub async fn prepare_authentication(
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<()> {
        Self::validate(account_user_id.as_deref(), account_epoch.as_deref())?;
        let refresh = cx.update(AuthTokenRefresh::ensure_valid_access_token);
        if let Err(error) = refresh.await {
            let Some(code) = Self::connection_loss_code(&error) else {
                return Err(error);
            };
            cx.update(|cx| {
                Sai::log("Upload", move || format!("token refresh deferred: {code:?}"), SaiLogKind::Network, cx);
            });
        }
        Self::validate(account_user_id.as_deref(), account_epoch.as_deref())
    }

    pub fn make_client(
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        additional_validation: Option<RiseChunkedUploadStep>,
        cx: &mut App,
    ) -> RiseChunkedUploadClient {
        let additional_validation = additional_validation.unwrap_or_else(Self::no_validation);
        let hop = RiseEngineMainHop::shared(cx);
        let authorize_account_request: RiseChunkedUploadAuthorize = {
            let hop = hop.clone();
            let account_user_id = account_user_id.clone();
            let account_epoch = account_epoch.clone();
            Arc::new(move |request: URLRequest| -> BoxFuture<'static, anyhow::Result<URLRequest>> {
                let account_user_id = account_user_id.clone();
                let account_epoch = account_epoch.clone();
                hop.run(move |cx| {
                    async move {
                        Self::validate(account_user_id.as_deref(), account_epoch.as_deref())?;
                        let mut request = request;
                        let token = cx.update(|cx| {
                            AuthServicesStore::shared(cx).read(cx).tokens_and_other_data().access_token.clone()
                        });
                        if !token.is_empty() {
                            request.set_value(&format!("Bearer {token}"), AUTHORIZATION.as_str());
                        }
                        Ok(request)
                    }
                    .boxed_local()
                })
            })
        };
        let authorize: RiseChunkedUploadAuthorize = {
            let additional_validation = additional_validation.clone();
            let authorize_account_request = authorize_account_request.clone();
            Arc::new(move |request: URLRequest| -> BoxFuture<'static, anyhow::Result<URLRequest>> {
                let additional_validation = additional_validation.clone();
                let authorize_account_request = authorize_account_request.clone();
                async move {
                    additional_validation().await?;
                    authorize_account_request(request).await
                }
                .boxed()
            })
        };
        let refresh_authorization: RiseChunkedUploadStep = {
            let additional_validation = additional_validation.clone();
            let hop = hop.clone();
            let account_user_id = account_user_id.clone();
            let account_epoch = account_epoch.clone();
            Arc::new(move || -> BoxFuture<'static, anyhow::Result<()>> {
                let additional_validation = additional_validation.clone();
                let hop = hop.clone();
                let account_user_id = account_user_id.clone();
                let account_epoch = account_epoch.clone();
                async move {
                    additional_validation().await?;
                    hop.run(move |cx| {
                        async move {
                            cx.update(|cx| AuthTokenRefresh::refresh_access_token(None, cx)).await?;
                            Self::validate(account_user_id.as_deref(), account_epoch.as_deref())
                        }
                        .boxed_local()
                    })
                    .await
                }
                .boxed()
            })
        };
        let validate_context: RiseChunkedUploadStep = {
            let additional_validation = additional_validation.clone();
            let hop = hop.clone();
            let account_user_id = account_user_id.clone();
            let account_epoch = account_epoch.clone();
            Arc::new(move || -> BoxFuture<'static, anyhow::Result<()>> {
                let additional_validation = additional_validation.clone();
                let hop = hop.clone();
                let account_user_id = account_user_id.clone();
                let account_epoch = account_epoch.clone();
                async move {
                    additional_validation().await?;
                    hop.run(move |_| {
                        async move { Self::validate(account_user_id.as_deref(), account_epoch.as_deref()) }.boxed_local()
                    })
                    .await
                }
                .boxed()
            })
        };
        let premium = AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| profile.more.is_premium);
        let mut default_headers = HashMap::new();
        default_headers.insert(ACCEPT.as_str().to_owned(), ACCEPT_JSON.to_owned());
        RiseChunkedUploadClient::new(RiseChunkedUploadConfiguration::new(
            HttpService::resolved_api_base_url(),
            default_headers,
            Some(authorize),
            Some(authorize_account_request),
            Some(refresh_authorization),
            Some(validate_context),
            premium,
        ))
    }

    pub fn validate(account_user_id: Option<&str>, account_epoch: Option<&str>) -> anyhow::Result<()> {
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch) {
            return Err(CancellationError.into());
        }
        Ok(())
    }

    pub fn prepared_local_file(
        original: &RiseOutgoingMediaFile,
        compressed: &OutgoingMediaCompressorResult,
    ) -> RiseOutgoingMediaFile {
        let mut prepared = original.clone();
        let transformed = Some(compressed.url.standardized())
            != Url::from_file_path(&original.url).map(|url| url.standardized());
        let is_document = original
            .media_type
            .as_deref()
            .is_some_and(|media_type| trimming_whitespaces_and_newlines(media_type).to_lowercase() == DOCUMENT_MEDIA_TYPE);
        if transformed && !is_document {
            prepared.filename = Self::upload_filename(&original.filename, &compressed.content_type);
        }
        if let Some(path) = compressed.url.to_file_path() {
            prepared.url = path;
        }
        prepared.content_type = compressed.content_type.clone();
        prepared.byte_count = Some(compressed.byte_count);
        prepared
    }

    pub fn completion_dictionary(
        result: &RiseChunkedUploadResult,
        compressed: &OutgoingMediaCompressorResult,
    ) -> Map<String, Value> {
        let mut completion: Map<String, Value> = result
            .metadata
            .iter()
            .map(|(key, value)| (key.clone(), Self::foundation_value(value)))
            .collect();
        completion.insert(upload::ASSET_ID.to_owned(), Value::from(result.asset_id.clone()));
        completion.insert(upload::FILE_URL.to_owned(), Value::from(result.file_url.clone()));
        completion.insert(upload::FILE_KEY.to_owned(), Value::from(result.file_key.clone()));
        completion.insert(upload::MIME_TYPE.to_owned(), Value::from(result.mime_type.clone()));
        if let Some(access_url) = &result.access_url {
            completion.insert(upload::ACCESS_URL.to_owned(), Value::from(access_url.clone()));
        }
        if let Some(thumbnail_url) = &result.thumbnail_url {
            completion.insert(upload::THUMBNAIL_URL.to_owned(), Value::from(thumbnail_url.clone()));
        }
        if let Some(file_id) = &result.file_id {
            completion.insert(upload::FILE_ID.to_owned(), Value::from(file_id.clone()));
        }
        completion.insert(
            upload::FILE_SIZE.to_owned(),
            Value::from(result.file_size.unwrap_or(compressed.byte_count)),
        );
        if let Some(width) = result.width.or(compressed.pixel_width).filter(|width| *width > 0) {
            completion.insert(upload::WIDTH.to_owned(), Value::from(width));
        }
        if let Some(height) = result.height.or(compressed.pixel_height).filter(|height| *height > 0) {
            completion.insert(upload::HEIGHT.to_owned(), Value::from(height));
        }
        if let Some(duration) = result.duration {
            completion.insert(upload::DURATION.to_owned(), Value::from(duration));
        }
        completion
    }

    pub fn sanitized_slot_for_outbound(slot: &Map<String, Value>) -> Map<String, Value> {
        let mut outbound = slot.clone();
        for key in [RiseLocalKey::LOCAL_PREVIEW_URL, RiseLocalKey::LOCAL_SLOT_INDEX, RiseLocalKey::IS_CANCELLED] {
            outbound.remove(key);
            outbound.remove(&RiseWireKey::camel(key));
        }
        outbound
    }

    pub async fn seed_prepared_image_cache_if_possible(
        compressed: &OutgoingMediaCompressorResult,
        raw_remote_url: &str,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
    ) {
        Self::seed_prepared_image_cache_if_possible_local_file_url(
            &compressed.url,
            &compressed.content_type,
            raw_remote_url,
            account_user_id,
            account_epoch,
        )
        .await;
    }

    pub async fn seed_prepared_image_cache_if_possible_local_file_url(
        source_url: &Url,
        content_type: &str,
        raw_remote_url: &str,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
    ) {
        if !content_type.starts_with("image/") {
            return;
        }
        let Some(remote_url) = RemoteMediaURL::optional_normalized(Some(raw_remote_url)) else {
            return;
        };
        if !remote_url.to_lowercase().starts_with("https://")
            || !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
        {
            return;
        }
        let Some(source_path) = source_url.to_file_path() else {
            return;
        };
        let invalidation_token = CleverImageDiskCache::invalidation_token(account_user_id.as_deref());
        let _ = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
                    return;
                }
                let Ok(data) = std::fs::read(&source_path) else {
                    return;
                };
                if data.is_empty() {
                    return;
                }
                CleverImageDiskCache::store_data(
                    &data,
                    &remote_url,
                    account_user_id.as_deref(),
                    Some(invalidation_token),
                );
            })
            .await;
    }

    /// files the sent video under the address the server gave
    /// so bubble, viewer and note player use local bytes instead of downloading
    /// clone is awaited since the caller deletes its compressed file
    pub async fn seed_prepared_video_cache_if_possible(
        compressed: &OutgoingMediaCompressorResult,
        raw_remote_url: &str,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
    ) {
        let content_type = compressed.content_type.clone();
        if !content_type.to_lowercase().starts_with("video/") {
            return;
        }
        let Some(normalized) = RemoteMediaURL::optional_normalized(Some(raw_remote_url)) else {
            return;
        };
        let Some(remote) = Url::from_string(&normalized) else {
            return;
        };
        let scheme = remote.scheme().map(str::to_lowercase);
        if !matches!(scheme.as_deref(), Some("https" | "http"))
            || !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
        {
            return;
        }
        let Some(source_path) = compressed.url.to_file_path() else {
            return;
        };
        let cache_key = RiseMediaCacheIdentity::key(&remote);
        let path_extension = compressed.url.path_extension();
        let file_extension =
            if path_extension.is_empty() { FALLBACK_VIDEO_EXTENSION.to_owned() } else { path_extension };
        let suggested_filename = compressed.url.last_path_component();
        let staging_source = source_path.clone();
        let staged = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let staging = temporary_directory().join(format!(
                    "{SENT_VIDEO_STAGING_PREFIX}{}.{file_extension}",
                    Uuid::new_v4().uuid_string().to_lowercase()
                ));
                FileManager.copy_item(&staging_source, &staging).ok().map(|_| staging)
            })
            .await
            .ok()
            .flatten();
        let Some(staged) = staged else {
            return;
        };
        let runtime = RiseAccountRuntime::current();
        drop(runtime.spawn(async move {
            let adopt_key = cache_key.clone();
            let adopt_user_id = account_user_id.clone();
            let adopt_epoch = account_epoch.clone();
            let adopted = RiseAccountRuntime::current()
                .spawn_blocking(move || {
                    let adopted = RemoteMediaDiskCache::shared()
                        .adopt(
                            &staged,
                            &remote,
                            Some(adopt_key.as_str()),
                            Some(suggested_filename.as_str()),
                            Some(content_type),
                            adopt_user_id.as_deref(),
                            adopt_epoch.as_deref(),
                        )
                        .ok();
                    let _ = FileManager.remove_item(&staged);
                    adopted.map(|adopted| FileManager.attributes_of_item(&adopted).map_or(0, |attributes| attributes.size))
                })
                .await
                .ok()
                .flatten();
            let Some(byte_count) = adopted else {
                return;
            };
            RiseMediaTransferHub::shared().note(cache_key, byte_count, account_user_id, account_epoch).await;
        }));
    }

    fn upload_filename(original: &str, content_type: &str) -> String {
        let path = trimming_whitespaces_and_newlines(original);
        let stem = Path::new(path).with_extension("").to_string_lossy().into_owned();
        let safe_stem = if stem.is_empty() { FALLBACK_MEDIA_STEM } else { stem.as_str() };
        match content_type.to_lowercase().as_str() {
            "image/jpeg" => format!("{safe_stem}.jpg"),
            "video/mp4" => format!("{safe_stem}.mp4"),
            _ => {
                if path.is_empty() {
                    FALLBACK_FILE_NAME.to_owned()
                } else {
                    path.to_owned()
                }
            }
        }
    }

    fn foundation_value(value: &RiseJSONValue) -> Value {
        Value::from(value.clone())
    }

    fn connection_loss_code(error: &anyhow::Error) -> Option<URLErrorCode> {
        error
            .chain()
            .find_map(|cause| cause.downcast_ref::<URLError>())
            .map(|error| error.code)
            .filter(|code| RiseChunkedUploadClient::CONNECTION_LOSS_CODES.contains(code))
    }

    fn no_validation() -> RiseChunkedUploadStep {
        Arc::new(|| -> BoxFuture<'static, anyhow::Result<()>> { async { Ok(()) }.boxed() })
    }
}
