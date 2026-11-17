use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};

use gpui::{App, RenderImage};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, RgbaImage};
use rise_canon::rise_key::{file, session, upload, user};
use rise_canon::{RiseLimits, RiseUploadContextFormats, RiseUploadFormat};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_media::audio::audio_file_metadata::AudioFileMetadata;
use rise_media::audio::decoder::AudioDecoder as _;
use rise_media::audio::symphonia_backend::SymphoniaDecoder;
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;
use rise_platform::uniform_type::UTType;
use serde_json::{Map, Value};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_configuration::SaiFetchHttpConfiguration;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::clever_image_disk_cache::CleverImageBitmapDecoder;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::music::shared::utils::music::music_cover_letterbox::MusicCoverLetterbox;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicTrackFileUploadError {
    MissingRemoteUrl,
    MissingLocalFile,
    Mp3Required,
    FileTooLarge,
}

impl MusicTrackFileUploadError {
    pub fn error_description(&self) -> Option<String> {
        Some(match self {
            Self::MissingRemoteUrl => tr("music_upload_missing_url"),
            Self::MissingLocalFile => tr("music_upload_missing_local_file"),
            Self::Mp3Required => tr("music_local_upload_mp3_required"),
            Self::FileTooLarge => tr("music_local_upload_too_large"),
        })
    }
}

impl fmt::Display for MusicTrackFileUploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for MusicTrackFileUploadError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicTrackImportedMetadata {
    pub title: String,
    pub artist: Option<String>,
    pub duration_ms: i64,
    pub artwork_data: Option<Vec<u8>>,
}

pub enum MusicTrackFileUpload {}

const ARTWORK_JPEG_QUALITY: u8 = 90;

impl MusicTrackFileUpload {
    pub const MP3_EXTENSIONS: &'static [&'static str] = &["mp3"];

    pub const CHAT_AUDIO_EXTENSIONS: &'static [&'static str] = &["mp3", "m4a", "aac"];

    /// narrows the music_audio context by the mp3 profile, library takes mp3 only
    pub fn prepare_mp3(source_url: &Path) -> anyhow::Result<PathBuf> {
        Self::prepare_audio(source_url, Self::MP3_EXTENSIONS)
    }

    pub fn prepare_chat_audio(source_url: &Path) -> anyhow::Result<PathBuf> {
        Self::prepare_audio(source_url, Self::CHAT_AUDIO_EXTENSIONS)
    }

    pub fn prepare_studio_audio(source_url: &Path) -> anyhow::Result<PathBuf> {
        let file_extension = Self::path_extension(source_url).to_lowercase();
        if file_extension.is_empty() {
            return Err(MusicTrackFileUploadError::MissingLocalFile.into());
        }
        Self::prepare_audio(source_url, &[file_extension.as_str()])
    }

    pub fn exceeds_track_file_limit(url: &Path) -> bool {
        let Ok(attributes) = FileManager.attributes_of_item(url) else { return false };
        attributes.size > RiseLimits::value("music_track_file")
    }

    fn prepare_audio(source_url: &Path, allowed_extensions: &[&str]) -> anyhow::Result<PathBuf> {
        let file_extension = Self::path_extension(source_url).to_lowercase();
        if !allowed_extensions.contains(&file_extension.as_str()) {
            return Err(MusicTrackFileUploadError::Mp3Required.into());
        }

        let size = FileManager.attributes_of_item(source_url)?.size;
        if size <= 0 || size > RiseLimits::value("music_track_file") {
            return Err(MusicTrackFileUploadError::FileTooLarge.into());
        }

        let destination =
            temporary_directory().join(format!("local-track-{}.{file_extension}", Uuid::new_v4().uuid_string()));
        FileManager.copy_item(source_url, &destination)?;
        Ok(destination)
    }

    /// prepare_mp3 copies the pick to local-track-uuid.mp3 so url lost the picked name
    /// without original_file_name an untagged file falls back to that uuid
    pub async fn imported_metadata(url: PathBuf, original_file_name: Option<String>) -> MusicTrackImportedMetadata {
        let source = url.clone();
        let tags = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let extension = Self::path_extension(&source);
                std::fs::read(&source)
                    .map(|bytes| AudioFileMetadata::read(bytes, Some(extension.as_str())))
                    .unwrap_or_default()
            })
            .await
            .unwrap_or_default();

        let title = Self::non_empty_trimmed(tags.title.as_deref());
        let artist = Self::non_empty_trimmed(tags.artist.as_deref())
            .or_else(|| Self::non_empty_trimmed(tags.album_artist.as_deref()));

        let (fallback_title, fallback_artist) =
            Self::file_name_metadata(&original_file_name.unwrap_or_else(|| Self::last_path_component(&url)));

        MusicTrackImportedMetadata {
            title: title.unwrap_or(fallback_title),
            artist: artist.or(fallback_artist),
            duration_ms: Self::audio_duration_ms(url).await,
            artwork_data: tags.artwork,
        }
    }

    pub fn file_name_metadata(file_name: &str) -> (String, Option<String>) {
        let stem = trimming_whitespaces_and_newlines(Self::deleting_path_extension(file_name));
        if stem.is_empty() {
            return (String::new(), None);
        }
        for separator in [" \u{2014} ", " \u{2013} ", " - "] {
            let parts: Vec<&str> = stem.split(separator).collect();
            if parts.len() < 2 {
                continue;
            }
            let artist = trimming_whitespaces_and_newlines(parts[0]);
            let joined = parts[1..].join(separator);
            let title = trimming_whitespaces_and_newlines(&joined);
            if artist.is_empty() || title.is_empty() {
                continue;
            }
            return (title.to_owned(), Some(artist.to_owned()));
        }
        (stem.to_owned(), None)
    }

    pub fn local_artwork_file(data: &[u8]) -> Option<SaiWsOptimisticLocalFile> {
        let image = CleverImageBitmapDecoder::decode(data, None)?;
        let jpeg = Self::jpeg_data(&MusicCoverLetterbox::trimmed(&image), ARTWORK_JPEG_QUALITY)?;

        let path = temporary_directory().join(format!(
            "local-track-cover-{}.{}",
            Uuid::new_v4().uuid_string(),
            RiseUploadFormat::IMAGE_JPEG.extension
        ));
        if !FileManager.create_file(&path, Some(&jpeg)) {
            return None;
        }
        let url = Self::file_url(&path)?;
        Some(SaiWsOptimisticLocalFile {
            filename: url.last_path_component(),
            url,
            content_type: RiseUploadFormat::IMAGE_JPEG.mime.to_owned(),
            file_context_override: Some(RiseUploadContextFormats::MUSIC_COVER.context.to_owned()),
            preferred_media_type: None,
            byte_count: i64::try_from(jpeg.len()).ok(),
        })
    }

    pub fn remote_url(done: &Map<String, Value>, cx: &mut App) -> Option<String> {
        if let Some(url) = done.get(user::DATA).and_then(Value::as_object).and_then(|nested| Self::remote_url(nested, cx)) {
            return Some(url);
        }
        let candidates = [
            done.get(upload::FILE_URL).and_then(Value::as_str),
            done.get(file::URL).and_then(Value::as_str),
            done.get(session::LOCATION).and_then(Value::as_str),
        ];
        candidates
            .into_iter()
            .flatten()
            .find_map(|raw| Self::absolute_api_url(raw, cx).filter(|normalized| !normalized.is_empty()))
    }

    pub fn remote_access_url(done: &Map<String, Value>) -> Option<String> {
        if let Some(url) = done.get(user::DATA).and_then(Value::as_object).and_then(Self::remote_access_url) {
            return Some(url);
        }
        RemoteMediaURL::optional_normalized(done.get(upload::ACCESS_URL).and_then(Value::as_str))
    }

    pub fn remote_asset_id(done: &Map<String, Value>) -> Option<String> {
        if let Some(asset_id) = done.get(user::DATA).and_then(Value::as_object).and_then(Self::remote_asset_id) {
            return Some(asset_id);
        }
        let candidates = [
            done.get(upload::ASSET_ID).and_then(Value::as_str),
            done.get(upload::UPLOAD_ID).and_then(Value::as_str),
        ];
        candidates
            .into_iter()
            .map(|raw| trimming_whitespaces_and_newlines(raw.unwrap_or_default()))
            .find(|value| !value.is_empty())
            .map(str::to_owned)
    }

    fn absolute_api_url(raw: &str, cx: &mut App) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        if trimmed.to_lowercase().starts_with("http") {
            return RemoteMediaURL::optional_normalized(Some(trimmed));
        }
        if !trimmed.starts_with('/') {
            return None;
        }
        let base = SaiFetchHttpConfiguration::shared(cx).read(cx).base_url.clone()?;
        let mut components = UrlComponents::from_url(&base, false)?;
        components.set_path(trimmed);
        components.set_query(None);
        components.set_fragment(None);
        components.url().map(|url| url.absolute_string().to_owned())
    }

    pub fn mime_type(url: &Path) -> String {
        UTType::from_filename_extension(&Self::path_extension(url))
            .and_then(|uniform_type| uniform_type.preferred_mime_type())
            .unwrap_or_else(|| RiseUploadFormat::APPLICATION_OCTET_STREAM.mime.to_owned())
    }

    pub fn audio_mime_type(url: &Path) -> String {
        let mime = Self::mime_type(url);
        if mime.starts_with("audio/") { mime } else { RiseUploadFormat::AUDIO_MPEG.mime.to_owned() }
    }

    pub async fn audio_duration_ms(url: PathBuf) -> i64 {
        RiseAccountRuntime::current()
            .spawn_blocking(move || Self::duration_ms_via_audio_file(&url))
            .await
            .ok()
            .flatten()
            .unwrap_or(0)
    }

    fn duration_ms_via_audio_file(url: &Path) -> Option<i64> {
        let bytes = std::fs::read(url).ok()?;
        let decoder = SymphoniaDecoder::open(bytes).ok()?;
        let milliseconds = i64::try_from(decoder.duration_millis()?).ok()?;
        if milliseconds <= 50 {
            return None;
        }
        Some(milliseconds)
    }

    fn non_empty_trimmed(value: Option<&str>) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(value?);
        if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
    }

    fn path_extension(url: &Path) -> String {
        url.extension().and_then(OsStr::to_str).unwrap_or_default().to_owned()
    }

    fn last_path_component(url: &Path) -> String {
        url.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn deleting_path_extension(file_name: &str) -> &str {
        match file_name.rfind('.') {
            Some(dot) if dot > 0 && !file_name[dot + 1..].contains('/') && !file_name[..dot].ends_with('/') => {
                &file_name[..dot]
            }
            _ => file_name,
        }
    }

    /// file url of a path for the upload runner
    fn file_url(path: &Path) -> Option<Url> {
        Url::from_file_path(path)
    }

    fn jpeg_data(image: &RenderImage, quality: u8) -> Option<Vec<u8>> {
        let size = image.size(0);
        let width = u32::try_from(size.width.0).ok()?;
        let height = u32::try_from(size.height.0).ok()?;
        let mut bytes = image.as_bytes(0)?.to_vec();
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let rgb = DynamicImage::ImageRgba8(RgbaImage::from_raw(width, height, bytes)?).into_rgb8();
        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut data, quality).encode_image(&rgb).ok()?;
        Some(data)
    }
}
