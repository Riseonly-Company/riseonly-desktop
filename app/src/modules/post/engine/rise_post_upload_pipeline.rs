use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use gpui::AsyncApp;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_chunked_upload::{
    RiseChunkedUploadClient, RiseChunkedUploadProgressHandler, RiseChunkedUploadRequest, RiseChunkedUploadStep,
    RiseChunkedUploadTransferProgress,
};
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::post::engine::rise_post_domain::RisePostDomainRegistry;
use crate::modules::post::engine::rise_post_engine_models::RisePostLocalUpload;

/// upload progress callback with a fraction
pub type RisePostUploadProgress = Arc<dyn Fn(f64) -> BoxFuture<'static, ()> + Send + Sync>;

const FILE_CONTEXT: &str = "post";
const THUMBNAIL_CONTENT_TYPE: &str = "image/jpeg";
const COMPRESSION_SHARE: f64 = 0.35;
const VIDEO_SHARE: f64 = 0.9;
const THUMBNAIL_TIME_SECONDS: f64 = 0.1;
const THUMBNAIL_MAXIMUM_WIDTH: u32 = 1_080;
const THUMBNAIL_MAXIMUM_HEIGHT: u32 = 1_920;
const THUMBNAIL_JPEG_QUALITY: u8 = 82;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RisePostUploadPipeline {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
}

impl RisePostUploadPipeline {
    pub async fn upload(
        &self,
        files: &[RisePostLocalUpload],
        request_id: &str,
        on_progress: RisePostUploadProgress,
    ) -> anyhow::Result<Vec<String>> {
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let progress_relay = Arc::new(RisePostUploadProgressRelay::new(on_progress));
        let Some(hop) = RiseEngineMainHop::installed() else {
            return Err(CancellationError.into());
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        hop.run({
            let account_id = account_id.clone();
            let account_epoch = account_epoch.clone();
            move |mut cx: AsyncApp| {
                async move {
                    RiseConversationUploadSupport::prepare_authentication(Some(account_id), Some(account_epoch), &mut cx)
                        .await
                }
                .boxed_local()
            }
        })
        .await?;
        let additional_validation = self.additional_validation(hop.clone());
        let client: RiseChunkedUploadClient = hop
            .run({
                let account_id = account_id.clone();
                let account_epoch = account_epoch.clone();
                move |cx: AsyncApp| {
                    async move {
                        Ok(cx.update(|cx| {
                            RiseConversationUploadSupport::make_client(
                                Some(account_id),
                                Some(account_epoch),
                                Some(additional_validation),
                                cx,
                            )
                        }))
                    }
                    .boxed_local()
                }
            })
            .await?;

        let count = files.len().max(1) as f64;
        let mut urls: Vec<String> = Vec::with_capacity(files.len());
        for (index, file) in files.iter().enumerate() {
            let base = index as f64 / count;
            let Some(path) = file.url.to_file_path() else {
                return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
            };
            let mut local = RiseOutgoingMediaFile::new(path, file.filename.clone(), file.content_type.clone());
            local.file_context = Some(FILE_CONTEXT.to_owned());
            let compression_progress: Arc<dyn Fn(f64) + Send + Sync> = {
                let progress_relay = progress_relay.clone();
                Arc::new(move |fraction: f64| progress_relay.r#yield(base + fraction * COMPRESSION_SHARE / count))
            };
            let compressed = OutgoingMediaCompressor::compress_if_needed(
                local.clone(),
                None,
                MediaCompressionConfiguration::publication(),
                Some(compression_progress),
                None,
            )
            .await?;
            let prepared = RiseConversationUploadSupport::prepared_local_file(&local, &compressed);
            let Some(compressed_path) = compressed.url.to_file_path() else {
                return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
            };
            let mut request = RiseChunkedUploadRequest::new(
                compressed_path,
                prepared.filename.clone(),
                compressed.content_type.clone(),
                FILE_CONTEXT.to_owned(),
            );
            request.resource_id = Some(format!("{account_id}:{request_id}:{index}"));
            let transfer_progress: RiseChunkedUploadProgressHandler = {
                let progress_relay = progress_relay.clone();
                Arc::new(move |progress: RiseChunkedUploadTransferProgress| {
                    let fraction = COMPRESSION_SHARE + progress.fraction * (1.0 - COMPRESSION_SHARE);
                    progress_relay.r#yield(base + fraction / count);
                })
            };
            let result = client.upload(request, None, None, Some(transfer_progress)).await?;
            RiseConversationUploadSupport::seed_prepared_image_cache_if_possible(
                &compressed,
                &result.file_url,
                Some(account_id.clone()),
                Some(account_epoch.clone()),
            )
            .await;
            urls.push(result.file_url);
        }
        progress_relay.r#yield(1.0);
        progress_relay.finish().await;
        Ok(urls)
    }

    pub async fn upload_video(
        &self,
        file: &RisePostLocalUpload,
        request_id: &str,
        on_progress: RisePostUploadProgress,
    ) -> anyhow::Result<(String, Option<String>)> {
        let video_progress: RisePostUploadProgress = {
            let on_progress = on_progress.clone();
            Arc::new(move |progress: f64| on_progress(progress * VIDEO_SHARE))
        };
        let uploaded = self.upload(std::slice::from_ref(file), &format!("{request_id}:video"), video_progress).await?;
        let Some(video) = uploaded.into_iter().next() else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };
        let Some(thumbnail) = self.make_thumbnail(&file.url, request_id).await else {
            on_progress(1.0).await;
            return Ok((video, None));
        };
        let thumbnail_progress: RisePostUploadProgress = {
            let on_progress = on_progress.clone();
            Arc::new(move |progress: f64| on_progress(VIDEO_SHARE + progress * (1.0 - VIDEO_SHARE)))
        };
        let thumbnail_url = self
            .upload(std::slice::from_ref(&thumbnail), &format!("{request_id}:thumbnail"), thumbnail_progress)
            .await
            .ok()
            .and_then(|urls| urls.into_iter().next());
        if let Some(path) = thumbnail.url.to_file_path() {
            let _ = FileManager.remove_item(&path);
        }
        on_progress(1.0).await;
        Ok((video, thumbnail_url))
    }

    async fn make_thumbnail(&self, video_url: &Url, request_id: &str) -> Option<RisePostLocalUpload> {
        let video_path = video_url.to_file_path()?;
        let data = RiseAccountRuntime::current()
            .spawn_blocking(move || Self::thumbnail_jpeg_data(&video_path))
            .await
            .ok()
            .flatten()?;
        let path = temporary_directory().join(format!("rise-post-thumbnail-{request_id}.jpg"));
        let written = RiseAccountRuntime::current()
            .spawn_blocking({
                let path = path.clone();
                move || Self::write_atomically(&path, &data)
            })
            .await
            .ok()?;
        written.ok()?;
        Some(RisePostLocalUpload {
            url: Url::from_file_path(&path)?,
            filename: format!("thumbnail-{request_id}.jpg"),
            content_type: THUMBNAIL_CONTENT_TYPE.to_owned(),
        })
    }

    fn thumbnail_jpeg_data(video_path: &Path) -> Option<Vec<u8>> {
        let frame =
            rise_media::video_export::poster(video_path, THUMBNAIL_TIME_SECONDS, THUMBNAIL_MAXIMUM_HEIGHT).ok()?;
        let mut bytes = frame.bgra;
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let mut image = DynamicImage::ImageRgba8(RgbaImage::from_raw(frame.width, frame.height, bytes)?);
        if image.width() > THUMBNAIL_MAXIMUM_WIDTH || image.height() > THUMBNAIL_MAXIMUM_HEIGHT {
            image = image.resize(THUMBNAIL_MAXIMUM_WIDTH, THUMBNAIL_MAXIMUM_HEIGHT, FilterType::Triangle);
        }
        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut data, THUMBNAIL_JPEG_QUALITY).encode_image(&image.to_rgb8()).ok()?;
        Some(data)
    }

    fn write_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut staging = path.as_os_str().to_owned();
        staging.push(format!(".{}", Uuid::new_v4().uuid_string()));
        let staging = PathBuf::from(staging);
        std::fs::write(&staging, data)?;
        std::fs::rename(&staging, path).inspect_err(|_| {
            let _ = FileManager.remove_item(&staging);
        })
    }

    /// client asks from its own tasks so each check hops to the main thread where the account
    /// scope and domain registry live
    fn additional_validation(&self, hop: RiseEngineMainHop) -> RiseChunkedUploadStep {
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        Arc::new(move || {
            let account_id = account_id.clone();
            let account_epoch = account_epoch.clone();
            hop.run(move |cx: AsyncApp| {
                async move {
                    let valid = cx.update(|cx| {
                        SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&account_epoch))
                            && RisePostDomainRegistry::shared(cx).read(cx).matches(
                                &account_id,
                                &account_epoch,
                                session_id,
                            )
                    });
                    if !valid {
                        return Err(CancellationError.into());
                    }
                    Ok(())
                }
                .boxed_local()
            })
        })
    }
}

struct RisePostUploadProgressRelay {
    continuation: Mutex<Option<watch::Sender<f64>>>,
    consumer: Mutex<Option<JoinHandle<()>>>,
}

impl RisePostUploadProgressRelay {
    const MINIMUM_STEP: f64 = 0.03;
    const MINIMUM_INTERVAL_SECONDS: f64 = 0.1;

    fn new(handler: RisePostUploadProgress) -> Self {
        let (sender, mut receiver) = watch::channel(-1.0_f64);
        let consumer = RiseAccountRuntime::current().spawn(async move {
            let mut last_value = -1.0_f64;
            let mut last_timestamp = 0.0_f64;
            while receiver.changed().await.is_ok() {
                let raw_value = *receiver.borrow_and_update();
                let value = raw_value.clamp(0.0, 1.0);
                let timestamp = Date::now().time_interval_since_reference_date();
                if value < 1.0
                    && value - last_value < Self::MINIMUM_STEP
                    && timestamp - last_timestamp < Self::MINIMUM_INTERVAL_SECONDS
                {
                    continue;
                }
                last_value = value;
                last_timestamp = timestamp;
                handler(value).await;
            }
        });
        Self { continuation: Mutex::new(Some(sender)), consumer: Mutex::new(Some(consumer)) }
    }

    fn r#yield(&self, value: f64) {
        if let Some(sender) = self.continuation.lock().as_ref() {
            sender.send_replace(value);
        }
    }

    async fn finish(&self) {
        self.continuation.lock().take();
        let consumer = self.consumer.lock().take();
        if let Some(consumer) = consumer {
            let _ = consumer.await;
        }
    }
}
