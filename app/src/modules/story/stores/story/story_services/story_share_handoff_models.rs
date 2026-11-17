use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_limits::limit::{STORY_HANDOFF_EDITOR_DIMENSION, STORY_HANDOFF_FILE, STORY_HANDOFF_ITEMS, STORY_HANDOFF_PIXELS};
use rise_platform::uniform_type::UTType;
use serde::{Deserialize, Serialize};

use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerItem, MediaPickerTrackPlate, MediaPickerTrackSelection,
};
use crate::modules::media_picker::editor::editor_state_codable::EditorStateCodec;
use crate::modules::media_picker::library::media_asset::MediaAsset;
use crate::modules::media_picker::selection::media_selection_state::{MediaSelectionEdit, MediaSelectionEditPayload};
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::share_extension::shared::rise_share_container::RiseShareContainer;
use crate::modules::share_extension::shared::rise_share_models::{
    RiseShareStoryItem, RiseShareStoryMediaKind, RiseShareStoryPublishCheckpoint,
};
use crate::modules::story::stores::story::story_actions::story_types::StoryAudience;

#[derive(Clone, Debug)]
pub struct StoryShareHandoffContext {
    pub session_id: String,
    pub account_id: String,
    pub directory: PathBuf,
    pub items: Vec<RiseShareStoryItem>,
    pub current_index: usize,
    pub publish_checkpoint: Option<RiseShareStoryPublishCheckpoint>,
}

impl StoryShareHandoffContext {
    pub fn current_item(&self) -> &RiseShareStoryItem {
        &self.items[self.current_index]
    }

    pub fn display_index(&self) -> usize {
        self.current_index + 1
    }

    pub fn total_count(&self) -> usize {
        self.items.len()
    }

    pub fn advanced(&self) -> Option<StoryShareHandoffContext> {
        let next_index = self.current_index + 1;
        if next_index >= self.items.len() {
            return None;
        }
        Some(StoryShareHandoffContext {
            session_id: self.session_id.clone(),
            account_id: self.account_id.clone(),
            directory: self.directory.clone(),
            items: self.items.clone(),
            current_index: next_index,
            publish_checkpoint: None,
        })
    }

    pub fn publish_context(&self) -> StorySharePublishContext {
        StorySharePublishContext {
            account_id: self.account_id.clone(),
            session_id: self.session_id.clone(),
            item_id: self.current_item().id.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StorySharePublishContext {
    pub account_id: String,
    pub session_id: String,
    pub item_id: String,
}

impl StorySharePublishContext {
    pub fn idempotency_key(&self) -> String {
        RiseShareStoryPublishCheckpoint::stable_idempotency_key(&self.account_id, &self.session_id, &self.item_id)
    }
}

#[derive(Clone)]
pub struct StoryShareHandoffDraft {
    pub id: Uuid,
    pub context: StoryShareHandoffContext,
    pub source_url: PathBuf,
    pub media_asset: MediaAsset,
    pub source_image: Option<CleverImage>,
    pub initial_edit: Option<MediaSelectionEdit>,
    pub initial_caption: String,
    pub initial_track: Option<MediaPickerTrackSelection>,
}

impl StoryShareHandoffDraft {
    pub fn retrying(&self, edit: MediaSelectionEdit, caption: String) -> StoryShareHandoffDraft {
        StoryShareHandoffDraft {
            id: Uuid::new_v4(),
            context: self.context.clone(),
            source_url: self.source_url.clone(),
            media_asset: self.media_asset.clone(),
            source_image: self.source_image.clone(),
            initial_edit: Some(edit),
            initial_caption: caption,
            initial_track: self.initial_track.clone(),
        }
    }
}

#[derive(Clone)]
pub struct StoryShareEditorDraft {
    pub edit: MediaSelectionEdit,
    pub caption: String,
    pub track: Option<MediaPickerTrackSelection>,
}

#[derive(Clone)]
pub struct StorySharePublishRequest {
    pub caption: String,
    pub audience: StoryAudience,
    pub allow_screenshots: bool,
    pub post_to_profile: bool,
    pub duration_days: i64,
    pub selected_user_ids: Vec<String>,
    pub track: Option<MediaPickerTrackSelection>,
}

mod stored_data_base64 {
    use rise_core::foundation::foundation_data::{base64_encoded_string, data_base64_encoded};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64_encoded_string(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        data_base64_encoded(&text).ok_or_else(|| serde::de::Error::custom("invalid base64 data"))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredTrackPlate {
    is_dark: bool,
    center_x: f64,
    center_y: f64,
    rotation: f64,
    scale: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredTrack {
    track: RiseStoredMusicTrack,
    start_ms: i64,
    end_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plate: Option<StoredTrackPlate>,
}

impl StoredTrack {
    fn new(selection: &MediaPickerTrackSelection) -> Self {
        Self {
            track: selection.track.clone(),
            start_ms: selection.start_ms,
            end_ms: selection.end_ms,
            plate: selection.plate.map(|plate| StoredTrackPlate {
                is_dark: plate.is_dark,
                center_x: plate.center_x,
                center_y: plate.center_y,
                rotation: plate.rotation,
                scale: plate.scale,
            }),
        }
    }

    fn selection(&self) -> MediaPickerTrackSelection {
        MediaPickerTrackSelection {
            track: self.track.clone(),
            start_ms: self.start_ms,
            end_ms: self.end_ms,
            plate: self.plate.as_ref().map(|plate| MediaPickerTrackPlate {
                is_dark: plate.is_dark,
                center_x: plate.center_x,
                center_y: plate.center_y,
                rotation: plate.rotation,
                scale: plate.scale,
            }),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    item_id: String,
    kind: String,
    #[serde(with = "stored_data_base64")]
    state: Vec<u8>,
    caption: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    track: Option<StoredTrack>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredPublish {
    item_id: String,
    caption: String,
    audience: String,
    allow_screenshots: bool,
    post_to_profile: bool,
    duration_days: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selected_user_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    track: Option<StoredTrack>,
}

fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut temporary = url.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, data)?;
    std::fs::rename(&temporary, url).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

pub enum StoryShareEditorDraftFile {}

impl StoryShareEditorDraftFile {
    const IMAGE_KIND: &'static str = "image";
    const VIDEO_KIND: &'static str = "video";

    fn draft_url(context: &StoryShareHandoffContext) -> PathBuf {
        context.directory.join(format!("editor-draft-{}.json", context.current_index))
    }

    fn publish_url(context: &StoryShareHandoffContext) -> PathBuf {
        context.directory.join(format!("publish-request-{}.json", context.current_index))
    }

    pub async fn save(draft: StoryShareEditorDraft, context: &StoryShareHandoffContext) {
        let url = Self::draft_url(context);
        let item_id = context.current_item().id.clone();
        let _ = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let (kind, state) = match &draft.edit.payload {
                    MediaSelectionEditPayload::Image(image) => (Self::IMAGE_KIND, EditorStateCodec::encode_image(image)),
                    MediaSelectionEditPayload::Video(video) => (Self::VIDEO_KIND, EditorStateCodec::encode_video(video)),
                };
                let Some(state) = state else {
                    return;
                };
                let stored = Stored {
                    item_id,
                    kind: kind.to_owned(),
                    state,
                    caption: draft.caption,
                    track: draft.track.as_ref().map(StoredTrack::new),
                };
                let Ok(data) = serde_json::to_vec(&stored) else {
                    return;
                };
                let _ = write_atomically(&url, &data);
            })
            .await;
    }

    pub async fn load(context: &StoryShareHandoffContext) -> Option<StoryShareEditorDraft> {
        let url = Self::draft_url(context);
        let item_id = context.current_item().id.clone();
        RiseAccountRuntime::current()
            .spawn_blocking(move || -> Option<StoryShareEditorDraft> {
                let data = std::fs::read(&url).ok()?;
                let stored: Stored = serde_json::from_slice(&data).ok()?;
                if stored.item_id != item_id {
                    return None;
                }
                let edit = match stored.kind.as_str() {
                    Self::IMAGE_KIND => MediaSelectionEdit {
                        payload: MediaSelectionEditPayload::Image(EditorStateCodec::decode_image(&stored.state)?),
                    },
                    Self::VIDEO_KIND => MediaSelectionEdit {
                        payload: MediaSelectionEditPayload::Video(EditorStateCodec::decode_video(&stored.state)?),
                    },
                    _ => return None,
                };
                Some(StoryShareEditorDraft {
                    edit,
                    caption: stored.caption,
                    track: stored.track.as_ref().map(StoredTrack::selection),
                })
            })
            .await
            .ok()
            .flatten()
    }

    pub async fn save_publish_request(request: &StorySharePublishRequest, context: &StoryShareHandoffContext) {
        let url = Self::publish_url(context);
        let stored = StoredPublish {
            item_id: context.current_item().id.clone(),
            caption: request.caption.clone(),
            audience: request.audience.wire().to_owned(),
            allow_screenshots: request.allow_screenshots,
            post_to_profile: request.post_to_profile,
            duration_days: request.duration_days,
            selected_user_ids: Some(request.selected_user_ids.clone()),
            track: request.track.as_ref().map(StoredTrack::new),
        };
        let _ = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let Ok(data) = serde_json::to_vec(&stored) else {
                    return;
                };
                let _ = write_atomically(&url, &data);
            })
            .await;
    }

    pub async fn load_publish_request(context: &StoryShareHandoffContext) -> Option<StorySharePublishRequest> {
        let url = Self::publish_url(context);
        let item_id = context.current_item().id.clone();
        let stored = RiseAccountRuntime::current()
            .spawn_blocking(move || -> Option<StoredPublish> {
                let data = std::fs::read(&url).ok()?;
                serde_json::from_slice(&data).ok()
            })
            .await
            .ok()
            .flatten()?;
        if stored.item_id != item_id {
            return None;
        }
        let audience = StoryAudience::from_wire(&stored.audience)?;
        Some(StorySharePublishRequest {
            caption: stored.caption,
            audience,
            allow_screenshots: stored.allow_screenshots,
            post_to_profile: stored.post_to_profile,
            duration_days: stored.duration_days,
            selected_user_ids: stored.selected_user_ids.unwrap_or_default(),
            track: stored.track.as_ref().map(StoredTrack::selection),
        })
    }
}

#[derive(Clone)]
pub struct StoryShareHandoffExport {
    pub draft: StoryShareHandoffDraft,
    pub edit: MediaSelectionEdit,
    pub caption: String,
    pub immediate_send: bool,
}

#[derive(Clone)]
pub struct StoryShareHandoffComposition {
    pub context: StoryShareHandoffContext,
    pub item: MediaPickerItem,
    pub caption: String,
    pub immediate_send: bool,
    pub attached_track: Option<MediaPickerTrackSelection>,
    pub publish_request: Option<StorySharePublishRequest>,
}

impl StoryShareHandoffComposition {
    pub fn generated_urls(&self) -> Vec<Url> {
        let mut urls = vec![self.item.url.clone()];
        if let Some(poster_url) = self.item.video_payload.as_ref().and_then(|video| video.poster_url.clone()) {
            urls.push(poster_url);
        }
        if let Some(original_url) = self.item.image_payload.as_ref().and_then(|image| image.original_image_url.clone()) {
            urls.push(original_url);
        }
        urls.iter()
            .map(Url::standardized)
            .collect::<HashSet<Url>>()
            .into_iter()
            .collect()
    }
}

#[derive(Clone)]
pub struct StoryShareHandoffRecovery {
    pub context: StoryShareHandoffContext,
    pub checkpoint: RiseShareStoryPublishCheckpoint,
}

#[derive(Clone)]
pub struct StoryShareHandoffFinalization {
    pub context: StoryShareHandoffContext,
    pub generated_item: Option<MediaPickerItem>,
    pub error_message: Option<String>,
}

#[derive(Clone)]
pub enum StoryShareHandoffPhase {
    Idle,
    Preparing {
        session_id: String,
        current: usize,
        total: Option<usize>,
    },
    Editing(StoryShareHandoffDraft),
    Exporting(StoryShareHandoffExport),
    Composing(StoryShareHandoffComposition),
    Recovering(StoryShareHandoffRecovery),
    Finalizing(StoryShareHandoffFinalization),
}

impl StoryShareHandoffPhase {
    pub fn active_session_id(&self) -> Option<&str> {
        match self {
            Self::Idle => None,
            Self::Preparing { session_id, .. } => Some(session_id),
            Self::Editing(draft) => Some(&draft.context.session_id),
            Self::Exporting(export) => Some(&export.draft.context.session_id),
            Self::Composing(composition) => Some(&composition.context.session_id),
            Self::Recovering(recovery) => Some(&recovery.context.session_id),
            Self::Finalizing(finalization) => Some(&finalization.context.session_id),
        }
    }

    pub fn presentation_stage(&self, is_publishing: bool, has_publish_error: bool) -> StoryShareHandoffPresentationStage {
        match self {
            Self::Idle => StoryShareHandoffPresentationStage::Idle,
            Self::Preparing { .. } => StoryShareHandoffPresentationStage::Preparing,
            Self::Editing(_) => StoryShareHandoffPresentationStage::Editing,
            Self::Exporting(export) => {
                if export.immediate_send {
                    StoryShareHandoffPresentationStage::BackgroundWork
                } else {
                    StoryShareHandoffPresentationStage::ForegroundExport
                }
            }
            Self::Composing(composition) => {
                if !composition.immediate_send {
                    return StoryShareHandoffPresentationStage::ForegroundComposer;
                }
                if has_publish_error && !is_publishing {
                    StoryShareHandoffPresentationStage::BackgroundFailure
                } else {
                    StoryShareHandoffPresentationStage::BackgroundWork
                }
            }
            Self::Recovering(_) => {
                if has_publish_error && !is_publishing {
                    StoryShareHandoffPresentationStage::BackgroundFailure
                } else {
                    StoryShareHandoffPresentationStage::BackgroundWork
                }
            }
            Self::Finalizing(finalization) => {
                if finalization.error_message.is_none() {
                    StoryShareHandoffPresentationStage::BackgroundWork
                } else {
                    StoryShareHandoffPresentationStage::BackgroundFailure
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StoryShareHandoffPresentationStage {
    Idle,
    Preparing,
    Editing,
    ForegroundExport,
    ForegroundComposer,
    BackgroundWork,
    BackgroundFailure,
}

impl StoryShareHandoffPresentationStage {
    pub const ALL_CASES: [Self; 7] = [
        Self::Idle,
        Self::Preparing,
        Self::Editing,
        Self::ForegroundExport,
        Self::ForegroundComposer,
        Self::BackgroundWork,
        Self::BackgroundFailure,
    ];

    pub fn blocks_interface(self) -> bool {
        match self {
            Self::Preparing | Self::Editing | Self::ForegroundExport | Self::ForegroundComposer => true,
            Self::Idle | Self::BackgroundWork | Self::BackgroundFailure => false,
        }
    }

    pub fn shows_profile_avatar_activity(self) -> bool {
        self == Self::BackgroundWork
    }
}

#[derive(Clone, Debug)]
pub struct StorySharePreparedItem {
    pub context: StoryShareHandoffContext,
    pub source_url: PathBuf,
    pub kind: RiseShareStoryMediaKind,
    pub pixel_width: i64,
    pub pixel_height: i64,
    pub duration: f64,
    pub image_data: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StoryShareHandoffLoadError {
    #[error("invalidManifest")]
    InvalidManifest,
    #[error("invalidFile")]
    InvalidFile,
    #[error("unsupportedMedia")]
    UnsupportedMedia,
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn check_cancellation(cancelled: &AtomicBool) -> Result<(), CancellationError> {
    if cancelled.load(Ordering::SeqCst) {
        return Err(CancellationError);
    }
    Ok(())
}

pub enum StoryShareHandoffLoader {}

impl StoryShareHandoffLoader {
    const MAXIMUM_ITEM_COUNT: usize = STORY_HANDOFF_ITEMS as usize;
    const MAXIMUM_FILE_BYTE_COUNT: i64 = rise_limits::signed(STORY_HANDOFF_FILE);
    const MAXIMUM_IMAGE_PIXEL_COUNT: i64 = rise_limits::signed(STORY_HANDOFF_PIXELS);
    const EDITOR_IMAGE_MAX_PIXEL_SIZE: u32 = STORY_HANDOFF_EDITOR_DIMENSION as u32;
    const MAXIMUM_VIDEO_SIDE: f64 = 16_384.0;

    pub async fn load_context(session_id: String, expected_account_id: String) -> anyhow::Result<StoryShareHandoffContext> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel = CancelOnDrop(Arc::clone(&cancelled));
        RiseAccountRuntime::current()
            .spawn_blocking(move || Self::load_context_blocking(&session_id, &expected_account_id, &cancelled))
            .await?
    }

    fn load_context_blocking(
        session_id: &str,
        expected_account_id: &str,
        cancelled: &AtomicBool,
    ) -> anyhow::Result<StoryShareHandoffContext> {
        check_cancellation(cancelled)?;
        let (session, directory) = RiseShareContainer::load_story_session(session_id)?;
        let items = &session.items;
        let next_item_index = usize::try_from(session.next_item_index).ok();
        let unique_ids: HashSet<&str> = items.iter().map(|item| item.id.as_str()).collect();
        let unique_paths: HashSet<&str> = items.iter().map(|item| item.relative_path.as_str()).collect();
        let Some(current_index) = next_item_index.filter(|index| *index < items.len()) else {
            return Err(StoryShareHandoffLoadError::InvalidManifest.into());
        };
        if expected_account_id.is_empty()
            || session.account_id != expected_account_id
            || items.is_empty()
            || items.len() > Self::MAXIMUM_ITEM_COUNT
            || !items.iter().all(|item| RiseShareContainer::is_valid_session_identifier(&item.id))
            || unique_ids.len() != items.len()
            || unique_paths.len() != items.len()
        {
            return Err(StoryShareHandoffLoadError::InvalidManifest.into());
        }
        for item in items {
            check_cancellation(cancelled)?;
            let url = RiseShareContainer::resolved_story_item_url(item, &directory)?;
            Self::validate_regular_file(&url)?;
            Self::validate_declared_type(&item.content_type, item.kind)?;
            Self::validate_extension(&Self::path_extension(&url), item.kind)?;
        }
        Ok(StoryShareHandoffContext {
            session_id: session_id.to_owned(),
            account_id: session.account_id.clone(),
            directory,
            items: session.items.clone(),
            current_index,
            publish_checkpoint: session.publish_checkpoint.clone(),
        })
    }

    pub async fn prepare(context: StoryShareHandoffContext) -> anyhow::Result<StorySharePreparedItem> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel = CancelOnDrop(Arc::clone(&cancelled));
        RiseAccountRuntime::current()
            .spawn_blocking(move || Self::prepare_blocking(context, &cancelled))
            .await?
    }

    fn prepare_blocking(context: StoryShareHandoffContext, cancelled: &AtomicBool) -> anyhow::Result<StorySharePreparedItem> {
        check_cancellation(cancelled)?;
        let item = context.current_item().clone();
        let url = RiseShareContainer::resolved_story_item_url(&item, &context.directory)?;
        Self::validate_regular_file(&url)?;
        Self::validate_declared_type(&item.content_type, item.kind)?;
        Self::validate_extension(&Self::path_extension(&url), item.kind)?;

        let prepared = match item.kind {
            RiseShareStoryMediaKind::Image => Self::prepare_image(context, url)?,
            RiseShareStoryMediaKind::Video => Self::prepare_video(context, url)?,
        };
        check_cancellation(cancelled)?;
        Ok(prepared)
    }

    fn path_extension(url: &Path) -> String {
        url.extension().map(|extension| extension.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn validate_regular_file(url: &Path) -> Result<(), StoryShareHandoffLoadError> {
        let metadata = std::fs::symlink_metadata(url).map_err(|_| StoryShareHandoffLoadError::InvalidFile)?;
        let bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
        if !metadata.file_type().is_file()
            || metadata.file_type().is_symlink()
            || bytes <= 0
            || bytes > Self::MAXIMUM_FILE_BYTE_COUNT
        {
            return Err(StoryShareHandoffLoadError::InvalidFile);
        }
        Ok(())
    }

    fn expected_type(kind: RiseShareStoryMediaKind) -> UTType {
        match kind {
            RiseShareStoryMediaKind::Image => UTType::image(),
            RiseShareStoryMediaKind::Video => UTType::movie(),
        }
    }

    fn validate_declared_type(identifier: &str, kind: RiseShareStoryMediaKind) -> Result<(), StoryShareHandoffLoadError> {
        let Some(declared) = UTType::from_identifier(identifier) else {
            return Ok(());
        };
        if declared.conforms_to(&Self::expected_type(kind)) {
            return Ok(());
        }
        if declared == UTType::data() || declared == UTType::item() || declared == UTType::content() {
            return Ok(());
        }
        Err(StoryShareHandoffLoadError::UnsupportedMedia)
    }

    fn validate_extension(path_extension: &str, kind: RiseShareStoryMediaKind) -> Result<(), StoryShareHandoffLoadError> {
        if path_extension.is_empty() {
            return Err(StoryShareHandoffLoadError::UnsupportedMedia);
        }
        match UTType::from_filename_extension(path_extension) {
            Some(declared) if declared.conforms_to(&Self::expected_type(kind)) => Ok(()),
            _ => Err(StoryShareHandoffLoadError::UnsupportedMedia),
        }
    }

    fn prepare_image(context: StoryShareHandoffContext, url: PathBuf) -> Result<StorySharePreparedItem, StoryShareHandoffLoadError> {
        let reader = ImageReader::open(&url)
            .ok()
            .and_then(|reader| reader.with_guessed_format().ok())
            .ok_or(StoryShareHandoffLoadError::InvalidFile)?;
        let mut decoder = reader.into_decoder().map_err(|_| StoryShareHandoffLoadError::InvalidFile)?;
        let (stored_width, stored_height) = decoder.dimensions();
        let (width, height) = (i64::from(stored_width), i64::from(stored_height));
        if width <= 0 || height <= 0 || width > Self::MAXIMUM_IMAGE_PIXEL_COUNT / height {
            return Err(StoryShareHandoffLoadError::InvalidFile);
        }
        let orientation = decoder.orientation().ok();
        let mut image = DynamicImage::from_decoder(decoder).map_err(|_| StoryShareHandoffLoadError::InvalidFile)?;
        if let Some(orientation) = orientation {
            image.apply_orientation(orientation);
        }
        let long_side = image.width().max(image.height());
        if long_side > Self::EDITOR_IMAGE_MAX_PIXEL_SIZE {
            image = image.resize(
                Self::EDITOR_IMAGE_MAX_PIXEL_SIZE,
                Self::EDITOR_IMAGE_MAX_PIXEL_SIZE,
                FilterType::CatmullRom,
            );
        }
        let mut image_data = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut image_data), ImageFormat::Png)
            .map_err(|_| StoryShareHandoffLoadError::InvalidFile)?;

        Ok(StorySharePreparedItem {
            context,
            source_url: url,
            kind: RiseShareStoryMediaKind::Image,
            pixel_width: i64::from(image.width()),
            pixel_height: i64::from(image.height()),
            duration: 0.0,
            image_data: Some(image_data),
        })
    }

    fn prepare_video(context: StoryShareHandoffContext, url: PathBuf) -> Result<StorySharePreparedItem, StoryShareHandoffLoadError> {
        let probe = rise_media::video_export::probe(&url).map_err(|_| StoryShareHandoffLoadError::InvalidFile)?;
        if !probe.has_video {
            return Err(StoryShareHandoffLoadError::InvalidFile);
        }

        let duration = probe.duration_seconds;
        if !duration.is_finite() || duration <= 0.0 {
            return Err(StoryShareHandoffLoadError::InvalidFile);
        }
        let (_, _, transformed) = probe.preferred_transform.bounds_of(probe.natural_size);
        let resolved_width = transformed.width.abs();
        let resolved_height = transformed.height.abs();
        if !resolved_width.is_finite()
            || !resolved_height.is_finite()
            || resolved_width < 1.0
            || resolved_height < 1.0
            || resolved_width > Self::MAXIMUM_VIDEO_SIDE
            || resolved_height > Self::MAXIMUM_VIDEO_SIDE
            || resolved_width * resolved_height > Self::MAXIMUM_IMAGE_PIXEL_COUNT as f64
        {
            return Err(StoryShareHandoffLoadError::InvalidFile);
        }

        Ok(StorySharePreparedItem {
            context,
            source_url: url,
            kind: RiseShareStoryMediaKind::Video,
            pixel_width: resolved_width.round() as i64,
            pixel_height: resolved_height.round() as i64,
            duration,
            image_data: None,
        })
    }
}
