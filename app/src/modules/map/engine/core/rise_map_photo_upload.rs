use std::sync::Arc;

use gpui::AsyncApp;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_engine::rise_upload::rise_chunked_upload::{RiseChunkedUploadRequest, RiseChunkedUploadTransferProgress};
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_platform::photo_library::asset_details::{picture_original_date, picture_place};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::stores::http::http_service::HttpService;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::map::stores::map_actions::map_photos_types::MapMediaKind;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind};
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapPhotoUploadResult {
    pub file_url: String,
    pub thumbnail_url: String,
    pub width: i64,
    pub height: i64,
    pub media_kind: MapMediaKind,
    pub duration_ms: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum MapPhotoUploadError {
    #[error("The picked item is not a photo or a video.")]
    NotMedia,
    #[error("The picked video is longer than a map photo allows.")]
    VideoTooLong,
    #[error("The map photo upload did not return a file URL.")]
    EmptyRemoteUrl,
}

/// map photo upload progress in 0 to 1, called from the upload threads
pub type RiseMapPhotoUploadProgress = Arc<dyn Fn(f64) + Send + Sync>;

/// uploads one picked photo or short video under the private context map_photo
/// same chunked client and compressor as other pictures
/// only the context differs and so who file service serves it to
pub enum RiseMapPhotoUpload {}

impl RiseMapPhotoUpload {
    pub const UPLOAD_CONTEXT: &'static str = "map_photo";

    fn maximum_video_ms() -> i64 {
        RiseLimits::service_value("map_max_photo_video_ms")
    }

    /// main thread call
    /// the caller runs it on the foreground executor with its async app
    pub async fn upload(
        item: &MediaPickerItem,
        request_id: &str,
        on_progress: RiseMapPhotoUploadProgress,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<MapPhotoUploadResult> {
        if item.kind == MediaPickerItemKind::File || item.is_file {
            return Err(MapPhotoUploadError::NotMedia.into());
        }
        let is_video = item.kind == MediaPickerItemKind::Video;
        let maximum_video_ms = Self::maximum_video_ms();
        let picked_seconds =
            item.video_payload.as_ref().map_or(0.0, |video| (video.end_seconds - video.start_seconds).max(0.0));
        if is_video && picked_seconds * 1_000.0 > maximum_video_ms as f64 + 500.0 {
            return Err(MapPhotoUploadError::VideoTooLong.into());
        }

        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        RiseConversationUploadSupport::prepare_authentication(account_id.clone(), account_epoch.clone(), cx).await?;
        let client = cx.update(|cx| {
            RiseConversationUploadSupport::make_client(account_id.clone(), account_epoch.clone(), None, cx)
        });

        let Some(local_path) = item.url.to_file_path() else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };
        let filename = if item.suggested_file_name.is_empty() {
            format!("map-{request_id}.{}", if is_video { "mp4" } else { "jpg" })
        } else {
            item.suggested_file_name.clone()
        };
        let mut local = RiseOutgoingMediaFile::new(
            local_path,
            filename,
            if is_video { "video/mp4" } else { "image/jpeg" }.to_owned(),
        );
        local.file_context = Some(Self::UPLOAD_CONTEXT.to_owned());
        let compress_progress = on_progress.clone();
        let compressed = OutgoingMediaCompressor::compress_if_needed(
            local.clone(),
            None,
            MediaCompressionConfiguration::chat(),
            Some(Arc::new(move |fraction: f64| compress_progress(0.3_f64.min(fraction * 0.3)))),
            None,
        )
        .await?;
        let prepared = RiseConversationUploadSupport::prepared_local_file(&local, &compressed);
        let Some(compressed_path) = compressed.url.to_file_path() else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };
        let upload_progress = on_progress.clone();
        let result = client
            .upload(
                RiseChunkedUploadRequest::new(
                    compressed_path,
                    prepared.filename.clone(),
                    compressed.content_type.clone(),
                    Self::UPLOAD_CONTEXT.to_owned(),
                ),
                None,
                None,
                Some(Arc::new(move |progress: RiseChunkedUploadTransferProgress| {
                    upload_progress(0.3 + progress.fraction * 0.7)
                })),
            )
            .await?;

        let remote_url = trimming_whitespaces_and_newlines(&result.file_url).to_owned();
        if remote_url.is_empty() {
            return Err(MapPhotoUploadError::EmptyRemoteUrl.into());
        }
        on_progress(1.0);
        let reported_ms = result.duration.map_or(0, |duration| if duration > 600 { duration } else { duration * 1_000 });
        let length_ms = if picked_seconds > 0.0 { (picked_seconds * 1_000.0) as i64 } else { reported_ms };
        Ok(MapPhotoUploadResult {
            file_url: remote_url,
            thumbnail_url: result
                .thumbnail_url
                .as_deref()
                .map_or_else(String::new, |thumbnail| trimming_whitespaces_and_newlines(thumbnail).to_owned()),
            width: result.width.or(compressed.pixel_width).unwrap_or(0),
            height: result.height.or(compressed.pixel_height).unwrap_or(0),
            media_kind: if is_video { MapMediaKind::Video } else { MapMediaKind::Image },
            duration_ms: if is_video { maximum_video_ms.min(length_ms.max(0)) } else { 0 },
        })
    }
}

/// where and when a picture was taken, from the picked file
/// both optional since an edited photo often has neither
/// then its pinned where the reader is looking
pub enum MapPhotoMetadata {}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapPhotoMetadataReading {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub taken_at_ms: i64,
}

impl MapPhotoMetadata {
    const EXIF_DATE_FORMAT: &'static str = "yyyy:MM:dd HH:mm:ss";

    /// blocking file read, call it off the main thread
    pub fn read(url: &Url) -> MapPhotoMetadataReading {
        let empty = MapPhotoMetadataReading { latitude: None, longitude: None, taken_at_ms: 0 };
        let Some(path) = url.to_file_path() else {
            return empty;
        };

        let place = picture_place(&path);
        let mut taken_at_ms = 0;
        if let Some(stamp) = picture_original_date(&path) {
            let mut formatter = DateFormatter::new();
            formatter.set_locale("en_US_POSIX");
            formatter.set_date_format(Self::EXIF_DATE_FORMAT);
            if let Some(date) = formatter.date_from(&stamp) {
                taken_at_ms = date.ms();
            }
        }
        MapPhotoMetadataReading {
            latitude: place.as_ref().map(|place| place.latitude),
            longitude: place.as_ref().map(|place| place.longitude),
            taken_at_ms,
        }
    }
}

/// a map_photo lives on a secure route stored site relative
/// so the loader needs the api origin prefixed and attaches the bearer itself for secure file paths
pub enum MapPhotoURL {}

impl MapPhotoURL {
    pub const ROUTE: &'static str = "/api/file/secure/";
    const MAXIMUM_LENGTH: usize = 2_048;
    const POST_QUERY_ITEM: &'static str = "map_post_id";

    pub fn absolute(raw: &str) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw);
        if value.is_empty() {
            return None;
        }
        if value.starts_with("https://") {
            return Some(value.to_owned());
        }
        if GuideSandbox::is_practice_map_file(value) {
            return Some(value.to_owned());
        }
        let relative = UrlComponents::from_string(value)?;
        let path = relative.path();
        if relative.scheme().is_some()
            || relative.host().is_some()
            || !path.starts_with(Self::ROUTE)
            || value.chars().count() > Self::MAXIMUM_LENGTH
            || path.contains("..")
            || relative.fragment().is_some()
            || path.contains('\\')
        {
            return None;
        }
        let query = relative.query_items();
        if let Some(query) = &query {
            let map_photo_route = format!("{}{}/", Self::ROUTE, Self::UPLOAD_CONTEXT_FOLDER);
            let [item] = query.as_slice() else {
                return None;
            };
            let raw_post_id = item.value.as_deref()?;
            let post_id = raw_post_id.parse::<i32>().ok()?;
            if !path.starts_with(&map_photo_route)
                || item.name != Self::POST_QUERY_ITEM
                || post_id <= 0
                || post_id.to_string() != raw_post_id
            {
                return None;
            }
        }
        let mut components = UrlComponents::from_url(&HttpService::resolved_api_base_url(), false)?;
        let scheme = components.scheme()?.to_lowercase();
        if (scheme != "http" && scheme != "https") || components.host().is_none() {
            return None;
        }
        components.set_path(&path);
        components.set_query_items(query);
        components.set_fragment(None);
        components.url().map(|url| url.absolute_string().to_owned())
    }

    const UPLOAD_CONTEXT_FOLDER: &'static str = RiseMapPhotoUpload::UPLOAD_CONTEXT;
}
