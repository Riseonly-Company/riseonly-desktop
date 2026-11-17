use std::path::{Path, PathBuf};

use gpui::Size;
use rise_core::foundation::foundation_url::Url;
use rise_platform::file_manager::FileManager;
use serde::{Deserialize, Serialize};

use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::shared::utils::voice::voice_message_metadata::VoiceMessageMetadata;
use crate::modules::chat::stores::message::message_actions::message_wire_types::RiseStoredOutgoingMessage;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerAudioPayload, MediaPickerEditedImage, MediaPickerEditedVideo, MediaPickerItem, MediaPickerItemKind,
};
use crate::modules::media_picker::editor::editor_state::{EditorImageState, EditorPersonTagEntity};
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;

/// a failed attachment send kept on its row
/// composer body, scope and upload sources so send again can upload anew even after a relaunch
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseMessageUploadRetry {
    pub body: RiseStoredOutgoingMessage,
    pub scope: RiseMessageScope,
    pub sources: Vec<u8>,
    pub file_paths: Vec<String>,
}

impl RiseMessageUploadRetry {
    pub fn is_resendable(&self) -> bool {
        self.file_paths.iter().all(|path| FileManager.file_exists(Path::new(path)))
    }
}

#[derive(Clone, Debug)]
pub struct RiseMessageUploadResend {
    pub body: RiseStoredOutgoingMessage,
    pub scope: RiseMessageScope,
    pub local_files: Vec<RiseOutgoingMediaFile>,
    pub picker_items: Vec<MediaPickerItem>,
    pub voice_metadata_slots: Vec<VoiceMessageMetadata>,
}

pub enum RiseMessageUploadRetryCodec {}

impl RiseMessageUploadRetryCodec {
    pub fn retry(
        body: &RiseStoredOutgoingMessage,
        scope: &RiseMessageScope,
        local_files: &[RiseOutgoingMediaFile],
        picker_items: &[MediaPickerItem],
        voice_metadata_slots: &[VoiceMessageMetadata],
    ) -> Option<RiseMessageUploadRetry> {
        if local_files.is_empty() || !local_files.iter().all(|file| file.video_note.is_none()) {
            return None;
        }
        let sources = RiseMessageUploadSources {
            files: local_files.iter().map(File::new).collect(),
            items: picker_items.iter().map(Item::new).collect(),
            voices: voice_metadata_slots.iter().map(Voice::new).collect(),
        };
        let data = serde_json::to_vec(&sources).ok()?;
        Some(RiseMessageUploadRetry {
            body: body.clone(),
            scope: scope.clone(),
            sources: data,
            file_paths: local_files
                .iter()
                .filter(|file| !file.url.as_os_str().is_empty())
                .map(|file| file.url.to_string_lossy().into_owned())
                .collect(),
        })
    }

    /// a text waiting behind an unfinished upload has nothing to upload but a relaunch fails it like one
    /// its body alone lets send again work
    pub fn text_retry(body: &RiseStoredOutgoingMessage, scope: &RiseMessageScope) -> Option<RiseMessageUploadRetry> {
        let sources = RiseMessageUploadSources { files: Vec::new(), items: Vec::new(), voices: Vec::new() };
        let data = serde_json::to_vec(&sources).ok()?;
        Some(RiseMessageUploadRetry { body: body.clone(), scope: scope.clone(), sources: data, file_paths: Vec::new() })
    }

    pub fn resend(retry: &RiseMessageUploadRetry) -> Option<RiseMessageUploadResend> {
        if !retry.is_resendable() {
            return None;
        }
        let sources: RiseMessageUploadSources = serde_json::from_slice(&retry.sources).ok()?;
        if sources.files.is_empty() && !(sources.items.is_empty() && sources.voices.is_empty()) {
            return None;
        }
        let picker_items: Vec<MediaPickerItem> = sources.items.iter().map(Item::item).collect::<Option<Vec<_>>>()?;
        Some(RiseMessageUploadResend {
            body: retry.body.clone(),
            scope: retry.scope.clone(),
            local_files: sources.files.iter().map(File::file).collect(),
            picker_items,
            voice_metadata_slots: sources.voices.iter().map(Voice::metadata).collect(),
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct File {
    url: PathBuf,
    filename: String,
    content_type: String,
    file_context: Option<String>,
    media_type: Option<String>,
    byte_count: Option<i64>,
}

impl File {
    fn new(file: &RiseOutgoingMediaFile) -> Self {
        Self {
            url: file.url.clone(),
            filename: file.filename.clone(),
            content_type: file.content_type.clone(),
            file_context: file.file_context.clone(),
            media_type: file.media_type.clone(),
            byte_count: file.byte_count,
        }
    }

    fn file(&self) -> RiseOutgoingMediaFile {
        RiseOutgoingMediaFile {
            url: self.url.clone(),
            filename: self.filename.clone(),
            content_type: self.content_type.clone(),
            file_context: self.file_context.clone(),
            media_type: self.media_type.clone(),
            byte_count: self.byte_count,
            video_note: None,
        }
    }
}

/// exported image bytes stay out, the upload reads the file and not the payload
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImagePayload {
    was_edited: bool,
    pixel_size: Size<f32>,
    caption: String,
    editor_state_data: Option<Vec<u8>>,
    original_image_url: Option<Url>,
    deferred_edit_state: Option<EditorImageState>,
    is_processing_deferred: bool,
    optimistic_preview_url: Option<Url>,
    person_tags: Vec<EditorPersonTagEntity>,
}

impl ImagePayload {
    fn new(payload: &MediaPickerEditedImage) -> Self {
        Self {
            was_edited: payload.was_edited,
            pixel_size: payload.pixel_size,
            caption: payload.caption.clone(),
            editor_state_data: payload.editor_state_data.clone(),
            original_image_url: payload.original_image_url.clone(),
            deferred_edit_state: payload.deferred_edit_state.clone(),
            is_processing_deferred: payload.is_processing_deferred,
            optimistic_preview_url: payload.optimistic_preview_url.clone(),
            person_tags: payload.person_tags.clone(),
        }
    }

    fn payload(&self) -> MediaPickerEditedImage {
        MediaPickerEditedImage {
            data: Vec::new(),
            was_edited: self.was_edited,
            pixel_size: self.pixel_size,
            caption: self.caption.clone(),
            editor_state_data: self.editor_state_data.clone(),
            original_image_url: self.original_image_url.clone(),
            deferred_edit_state: self.deferred_edit_state.clone(),
            is_processing_deferred: self.is_processing_deferred,
            optimistic_preview_url: RiseMessageUploadSources::surviving(self.optimistic_preview_url.as_ref()),
            person_tags: self.person_tags.clone(),
            metadata_source_url: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoPayload {
    url: Url,
    start_seconds: f64,
    end_seconds: f64,
    duration: f64,
    is_muted: bool,
    caption: String,
    pixel_size: Size<f32>,
    poster_url: Option<Url>,
    optimistic_poster_url: Option<Url>,
    spatial_edit: Option<EditorImageState>,
    is_processing_deferred: bool,
    audio_pre_baked: bool,
}

impl VideoPayload {
    fn new(payload: &MediaPickerEditedVideo) -> Self {
        Self {
            url: payload.url.clone(),
            start_seconds: payload.start_seconds,
            end_seconds: payload.end_seconds,
            duration: payload.duration,
            is_muted: payload.is_muted,
            caption: payload.caption.clone(),
            pixel_size: payload.pixel_size,
            poster_url: payload.poster_url.clone(),
            optimistic_poster_url: payload.optimistic_poster_url.clone(),
            spatial_edit: payload.spatial_edit.clone(),
            is_processing_deferred: payload.is_processing_deferred,
            audio_pre_baked: payload.audio_pre_baked,
        }
    }

    fn payload(&self) -> MediaPickerEditedVideo {
        MediaPickerEditedVideo {
            url: self.url.clone(),
            start_seconds: self.start_seconds,
            end_seconds: self.end_seconds,
            duration: self.duration,
            is_muted: self.is_muted,
            caption: self.caption.clone(),
            pixel_size: self.pixel_size,
            poster_url: self.poster_url.clone(),
            optimistic_poster_url: RiseMessageUploadSources::surviving(self.optimistic_poster_url.as_ref()),
            spatial_edit: self.spatial_edit.clone(),
            is_processing_deferred: self.is_processing_deferred,
            audio_pre_baked: self.audio_pre_baked,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AudioPayload {
    title: String,
    artist: String,
    duration: f64,
    cover_url: Option<Url>,
}

impl AudioPayload {
    fn new(payload: &MediaPickerAudioPayload) -> Self {
        Self {
            title: payload.title.clone(),
            artist: payload.artist.clone(),
            duration: payload.duration,
            cover_url: payload.cover_url.clone(),
        }
    }

    fn payload(&self) -> MediaPickerAudioPayload {
        MediaPickerAudioPayload {
            title: self.title.clone(),
            artist: self.artist.clone(),
            duration: self.duration,
            cover_url: self.cover_url.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    id: String,
    kind: String,
    url: Url,
    suggested_file_name: String,
    asset_identifier: Option<String>,
    image: Option<ImagePayload>,
    video: Option<VideoPayload>,
    audio: Option<AudioPayload>,
    is_file: bool,
}

impl Item {
    fn new(item: &MediaPickerItem) -> Self {
        let kind = match item.kind {
            MediaPickerItemKind::Image => "image",
            MediaPickerItemKind::Video => "video",
            MediaPickerItemKind::File => "file",
        };
        Self {
            id: item.id.clone(),
            kind: kind.to_owned(),
            url: item.url.clone(),
            suggested_file_name: item.suggested_file_name.clone(),
            asset_identifier: item.asset_identifier.clone(),
            image: item.image_payload.as_ref().map(ImagePayload::new),
            video: item.video_payload.as_ref().map(VideoPayload::new),
            audio: item.audio_payload.as_ref().map(AudioPayload::new),
            is_file: item.is_file,
        }
    }

    fn item(&self) -> Option<MediaPickerItem> {
        let resolved_kind = match self.kind.as_str() {
            "image" => MediaPickerItemKind::Image,
            "video" => MediaPickerItemKind::Video,
            "file" => MediaPickerItemKind::File,
            _ => return None,
        };
        Some(MediaPickerItem::new(
            self.id.clone(),
            resolved_kind,
            self.url.clone(),
            self.suggested_file_name.clone(),
            self.asset_identifier.clone(),
            self.image.as_ref().map(ImagePayload::payload),
            self.video.as_ref().map(VideoPayload::payload),
            self.audio.as_ref().map(AudioPayload::payload),
            self.is_file,
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Voice {
    waveform: Vec<f32>,
    duration_seconds: i64,
    duration_milliseconds: i64,
}

impl Voice {
    fn new(metadata: &VoiceMessageMetadata) -> Self {
        Self {
            waveform: metadata.waveform.clone(),
            duration_seconds: metadata.duration_seconds,
            duration_milliseconds: metadata.duration_milliseconds,
        }
    }

    fn metadata(&self) -> VoiceMessageMetadata {
        VoiceMessageMetadata {
            waveform: self.waveform.clone(),
            duration_seconds: self.duration_seconds,
            duration_milliseconds: self.duration_milliseconds,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct RiseMessageUploadSources {
    files: Vec<File>,
    items: Vec<Item>,
    voices: Vec<Voice>,
}

impl RiseMessageUploadSources {
    /// a preview file may be gone after a relaunch, then the row draws from the source
    fn surviving(url: Option<&Url>) -> Option<Url> {
        let url = url?;
        if !url.is_file_url() {
            return Some(url.clone());
        }
        let path = url.to_file_path()?;
        FileManager.file_exists(&path).then(|| url.clone())
    }
}
