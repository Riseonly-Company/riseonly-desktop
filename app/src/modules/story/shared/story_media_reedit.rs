use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{AnyWindowHandle, App, AsyncApp, Context, ElementId, Entity, FocusHandle, Render, SharedString, Subscription, Window, point};
use rise_canon::rise_request::story::update_story::{Canvas as UpdateStoryCanvas, LinkAreasElement as UpdateStoryLinkArea, TrackPlate as UpdateStoryTrackPlate};
use rise_canon::{RiseMediaLinkPreviewMode, RiseMediaLinkStyle, RiseStoryCanvasItemKind, RiseStoryCanvasPhotoShape};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::file_manager::FileManager;
use rise_theme::{RiseThemeAppearance, ThemeStore};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::top_presenter::TopPresenter;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_chunked_file_upload::{SaiWsChunkedFileUpload, SaiWsUploadOnProgress};
use crate::core::lib::sai::sai_outgoing_media_hooks::OutgoingMediaProgress;
use crate::core::media::remote_media_disk_cache::{RemoteMediaDiskCache, RemoteMediaDiskCacheMaterializeOptions};
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder};
use crate::modules::auth::services::auth_token_refresh::AuthTokenRefresh;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_presenter_resolver::MediaPickerPresenterResolver;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerTrackPlate, MediaPickerTrackSelection};
use crate::modules::media_picker::editor::editor_link_entity::EditorLinkEntity;
use crate::modules::media_picker::editor::editor_state::{
    EditorImageState, EditorTextEntity, EditorTextEntityStyleMode, EditorTextEntityTextAlignment,
};
use crate::modules::media_picker::editor::media_editor_screen::{MediaEditorOutcome, MediaEditorScreen};
use crate::modules::media_picker::editor::media_reedit_launcher::MediaReeditLauncher;
use crate::modules::media_picker::editor::media_reedit_store::{MediaReeditStore, ReeditDocument};
use crate::modules::media_picker::export::export_progress_view_state::ExportProgressViewState;
use crate::modules::media_picker::export::media_exporter::MediaExporter;
use crate::modules::media_picker::export::media_picker_export_progress_overlay::MediaPickerExportProgressOverlay;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::media_picker::library::media_asset::{MediaAsset, MediaAssetMediaType};
use crate::modules::media_picker::editor::story_canvas_model::{
    EditorCanvasPhotoEntity, EditorStoryCanvas, StoryCanvasBackground, StoryCanvasGeometry, StoryCanvasPhotoStore,
};
use crate::modules::media_picker::selection::media_selection_state::MediaSelectionState;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::story::components::story_music_media_renderer::StoryMusicMediaRenderer;
use crate::modules::story::shared::story_canvas_publish::StoryCanvasPublish;
use crate::modules::story::shared::story_publish_staging::StoryPublishStaging;
use crate::modules::story::shared::story_track_plate_wire::StoryTrackPlateWire;
use crate::modules::story::stores::story::story_actions::story_actions::{
    StoryActionStoryHandler, StoryActionTextHandler, StoryActionsStore,
};
use crate::modules::story::stores::story::story_actions::story_wire_types::{
    StoryDTO, StoryDTOExt, StoryUploadWire,
};
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;

const CROSS_DISSOLVE_SECONDS: f64 = 0.3;
const VIDEO_MEDIA_TYPE: &str = "video";
const IMAGE_MEDIA_TYPE: &str = "image";
const STORY_FILE_CONTEXT: &str = "story";
const VIDEO_CONTENT_TYPE: &str = "video/mp4";
const IMAGE_CONTENT_TYPE: &str = "image/png";
const COMPRESSION_PROGRESS_SHARE: f64 = 0.2;
const UPLOAD_PROGRESS_SHARE: f64 = 0.8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoryMediaReeditFailure {
    Unavailable,
    Cancelled,
}

impl fmt::Display for StoryMediaReeditFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("story media re-edit unavailable"),
            Self::Cancelled => formatter.write_str("story media re-edit cancelled"),
        }
    }
}

impl std::error::Error for StoryMediaReeditFailure {}

#[derive(Clone, Debug, PartialEq)]
pub struct StoryMediaReeditResult {
    pub file_id: String,
    pub media_type: String,
    pub canvas: Option<UpdateStoryCanvas>,
    pub link_areas: Option<Vec<UpdateStoryLinkArea>>,
    pub local_file_url: Option<PathBuf>,
    pub local_aspect: Option<f32>,
    pub track_plate: Option<UpdateStoryTrackPlate>,
}

struct StoryMediaReeditRestoredCanvas {
    state: EditorImageState,
    first_photo: CleverImage,
}

pub type StoryMediaReeditOnProgress = Rc<dyn Fn(f64, &mut App)>;

struct GeneratedTemporaryFile(Option<PathBuf>);

impl Drop for GeneratedTemporaryFile {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = FileManager.remove_item(path);
        }
    }
}

/// re renders the media of a published story
/// from the saved editor document or else the flat media
/// flat is the only path for video and old ones
pub enum StoryMediaReedit {}

impl StoryMediaReedit {
    pub fn is_in_edit_window(story: &StoryDTO) -> bool {
        Self::is_in_edit_window_at(story, Date::now())
    }

    pub fn is_in_edit_window_at(story: &StoryDTO, now: Date) -> bool {
        if story.created_at_ms <= 0 {
            return false;
        }
        let now_ms = (now.time_interval_since_1970() * 1_000.0) as i64;
        now_ms < story.created_at_ms + StoryCanvasGeometry::edit_window_ms()
    }

    /// editor as the story composer has it, canvas and gallery button and links
    fn canvas_configuration(story: &StoryDTO) -> MediaPickerConfiguration {
        let mut configuration = MediaPickerConfiguration::chat_attachments();
        configuration.uses_story_canvas = true;
        configuration.allows_media_links = true;
        if Self::has_track(story) {
            configuration.allows_music_selection = true;
            configuration.consumer_mixes_attached_track = true;
        }
        configuration
    }

    fn has_track(story: &StoryDTO) -> bool {
        !trimming_whitespaces_and_newlines(story.track_id.as_deref().unwrap_or_default()).is_empty()
    }

    /// presents the editor and uploads the render, none means the author backed out
    pub async fn run(
        story: StoryDTO,
        theme_store: Entity<ThemeStore>,
        on_progress: Option<StoryMediaReeditOnProgress>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Option<StoryMediaReeditResult>> {
        let is_video = story.media_type.to_lowercase() == VIDEO_MEDIA_TYPE;
        let caption = story.caption.clone().unwrap_or_default();
        let document = cx
            .update(|cx| Self::load_document(story.id, cx))
            .filter(|document| !is_video || document.state.story_canvas.is_some());
        let music_source = if is_video && Self::has_track(&story) && (document.is_some() || story.canvas.is_some()) {
            let Some(published) = Self::published_media_url(&story, cx).await else {
                return Err(StoryMediaReeditFailure::Unavailable.into());
            };
            Some(published)
        } else {
            None
        };

        if let Some(document) = document {
            let is_canvas = document.state.story_canvas.is_some();
            let configuration =
                if is_canvas { Self::canvas_configuration(&story) } else { MediaPickerConfiguration::chat_attachments() };
            let attached_track =
                if is_canvas { cx.update(|cx| Self::attached_track(&story, &document.state, cx)) } else { None };
            let item = Self::present_image_editor(
                document.original_image,
                Some(document.state),
                story.id,
                caption,
                theme_store,
                configuration,
                attached_track,
                cx,
            )
            .await;
            return match item {
                Some(item) => Self::upload(item, false, music_source, on_progress, cx).await.map(Some),
                None => Ok(None),
            };
        }
        if (!is_video || story.canvas.is_some())
            && let Some(restored) = Self::restored_canvas(&story, cx).await
        {
            let attached_track = cx.update(|cx| Self::attached_track(&story, &restored.state, cx));
            let item = Self::present_image_editor(
                restored.first_photo,
                Some(restored.state),
                story.id,
                caption,
                theme_store,
                Self::canvas_configuration(&story),
                attached_track,
                cx,
            )
            .await;
            return match item {
                Some(item) => Self::upload(item, false, music_source, on_progress, cx).await.map(Some),
                None => Ok(None),
            };
        }

        let Some(local_url) = Self::published_media_url(&story, cx).await else {
            return Err(StoryMediaReeditFailure::Unavailable.into());
        };

        if is_video {
            let item = Self::present_video_editor(local_url, story.id, caption, theme_store, cx).await;
            return match item {
                Some(item) => Self::upload(item, true, None, on_progress, cx).await.map(Some),
                None => Ok(None),
            };
        }

        let image = cx
            .background_executor()
            .spawn(async move {
                let data = std::fs::read(&local_url).ok()?;
                CleverImageBitmapDecoder::decode(&data, None)
            })
            .await;
        let Some(image) = image else {
            return Err(StoryMediaReeditFailure::Unavailable.into());
        };
        let item = Self::present_image_editor(
            image,
            None,
            story.id,
            caption,
            theme_store,
            MediaPickerConfiguration::chat_attachments(),
            None,
            cx,
        )
        .await;
        match item {
            Some(item) => Self::upload(item, false, None, on_progress, cx).await.map(Some),
            None => Ok(None),
        }
    }

    pub fn has_layered_document(story: &StoryDTO, cx: &mut App) -> bool {
        let Some(document) = Self::load_document(story.id, cx) else {
            return false;
        };
        story.media_type.to_lowercase() != VIDEO_MEDIA_TYPE || document.state.story_canvas.is_some()
    }

    fn load_document(story_id: i64, cx: &mut App) -> Option<ReeditDocument> {
        MediaReeditStore::shared(cx).update(cx, |store, _| {
            let edit_id = store.edit_id_for_any(&[StoryServicesStore::reedit_identity(story_id)])?;
            store.load_document(&edit_id, StoryServicesStore::reedit_retention_seconds())
        })
    }

    /// published bytes from wherever the player already put them, a watched story is normally on disk
    async fn published_media_url(story: &StoryDTO, cx: &AsyncApp) -> Option<PathBuf> {
        let source = story.media_source();
        if source.is_empty() {
            return None;
        }
        if is_local_absolute_path(&source) {
            return Some(PathBuf::from(source));
        }
        let url = Url::from_string(&source)?;
        if url.is_file_url() {
            return url.to_file_path();
        }
        let options = RemoteMediaDiskCacheMaterializeOptions {
            suggested_filename: Some(url.last_path_component()),
            cache_key: Some(source),
            ..RemoteMediaDiskCacheMaterializeOptions::default()
        };
        let cache = RemoteMediaDiskCache::shared();
        cache.ensure_cached(&url, options, cx).await
    }

    async fn present_image_editor(
        original: CleverImage,
        state: Option<EditorImageState>,
        story_id: i64,
        caption: String,
        theme_store: Entity<ThemeStore>,
        configuration: MediaPickerConfiguration,
        attached_track: Option<MediaPickerTrackSelection>,
        cx: &mut AsyncApp,
    ) -> Option<MediaPickerItem> {
        let (done, answer) = oneshot::channel::<Option<MediaPickerItem>>();
        cx.update(|cx| {
            MediaReeditLauncher::present_with_configuration(
                original,
                state,
                StoryServicesStore::reedit_identity(story_id),
                caption,
                theme_store,
                configuration,
                attached_track,
                move |result, _| {
                    let _ = done.send(result.map(|result| result.item));
                },
                cx,
            );
        });
        answer.await.ok().flatten()
    }

    async fn present_video_editor(
        source_url: PathBuf,
        story_id: i64,
        caption: String,
        theme_store: Entity<ThemeStore>,
        cx: &mut AsyncApp,
    ) -> Option<MediaPickerItem> {
        let (done, answer) = oneshot::channel::<Option<MediaPickerItem>>();
        cx.update(|cx| {
            StoryVideoReeditLauncher::present(
                source_url,
                StoryServicesStore::reedit_identity(story_id),
                caption,
                theme_store,
                move |item, _| {
                    let _ = done.send(item);
                },
                cx,
            );
        });
        answer.await.ok().flatten()
    }

    /// compressor reports from its worker, the callback runs on the main thread
    fn compression_progress(
        on_progress: Option<StoryMediaReeditOnProgress>,
        cx: &AsyncApp,
    ) -> Option<OutgoingMediaProgress> {
        let on_progress = on_progress?;
        let (sender, mut receiver) = mpsc::unbounded::<f64>();
        cx.spawn(async move |cx: &mut AsyncApp| {
            while let Some(fraction) = receiver.next().await {
                cx.update(|cx| on_progress(fraction * COMPRESSION_PROGRESS_SHARE, cx));
            }
        })
        .detach();
        Some(Arc::new(move |fraction: f64| {
            let _ = sender.unbounded_send(fraction);
        }))
    }

    async fn upload(
        edited_item: MediaPickerItem,
        edited_is_video: bool,
        music_source: Option<PathBuf>,
        on_progress: Option<StoryMediaReeditOnProgress>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<StoryMediaReeditResult> {
        let canvas_state = if edited_is_video { None } else { StoryCanvasPublish::canvas_state(&edited_item) };
        let mut item = edited_item;
        let mut is_video = edited_is_video;
        if let Some(state) = canvas_state.as_ref().filter(|state| StoryCanvasPublish::animates(state)) {
            let state = state.clone();
            let still = item;
            item = cx.update(move |cx| StoryCanvasPublish::animated_video_item(still, state, cx)).await?;
            is_video = true;
        }
        let mut music_render = GeneratedTemporaryFile(None);
        if let Some(music_source) = music_source.as_deref()
            && canvas_state.is_some()
        {
            item = StoryMusicMediaRenderer::render_published_audio(item, music_source).await?;
            music_render.0 = item.url.to_file_path();
            is_video = true;
        }
        let upload_url: Url = if is_video {
            item.video_payload.as_ref().map_or_else(|| item.url.clone(), |payload| payload.url.clone())
        } else {
            item.url.clone()
        };
        let Some(upload_path) = upload_url.to_file_path() else {
            return Err(StoryMediaReeditFailure::Unavailable.into());
        };
        let mut original = RiseOutgoingMediaFile::new(
            upload_path,
            upload_url.last_path_component(),
            if is_video { VIDEO_CONTENT_TYPE } else { IMAGE_CONTENT_TYPE }.to_owned(),
        );
        original.file_context = Some(STORY_FILE_CONTEXT.to_owned());
        original.media_type = Some(if is_video { VIDEO_MEDIA_TYPE } else { IMAGE_MEDIA_TYPE }.to_owned());

        let compressed = OutgoingMediaCompressor::compress_if_needed(
            original.clone(),
            None,
            MediaCompressionConfiguration::publication(),
            Self::compression_progress(on_progress.clone(), cx),
            None,
        )
        .await?;
        let Some(compressed_path) = compressed.url.to_file_path() else {
            return Err(StoryMediaReeditFailure::Unavailable.into());
        };
        let _generated_temporary_file = GeneratedTemporaryFile(
            (compressed.url.standardized() != upload_url.standardized()).then(|| compressed_path.clone()),
        );
        let prepared = RiseConversationUploadSupport::prepared_local_file(&original, &compressed);
        let staging_source = compressed_path.clone();

        cx.update(AuthTokenRefresh::ensure_valid_access_token).await?;
        let upload_progress: Option<SaiWsUploadOnProgress> = on_progress.map(|on_progress| {
            let progress: SaiWsUploadOnProgress = Rc::new(move |fraction: f64, cx: &mut App| {
                on_progress(COMPRESSION_PROGRESS_SHARE + fraction * UPLOAD_PROGRESS_SHARE, cx);
            });
            progress
        });
        let upload = cx.update(|cx| {
            SaiWsChunkedFileUpload::upload(
                compressed_path,
                prepared.filename.clone(),
                compressed.content_type.clone(),
                STORY_FILE_CONTEXT.to_owned(),
                None,
                SaiAccountCacheScope::active_user_id(),
                SaiAccountCacheScope::active_epoch(),
                upload_progress,
                None,
                cx,
            )
        });
        let done = upload.await?;
        let Some(file_id) = StoryUploadWire::remote_file_id(&done) else {
            return Err(StoryMediaReeditFailure::Unavailable.into());
        };
        let mut result = StoryMediaReeditResult {
            file_id,
            media_type: if is_video { VIDEO_MEDIA_TYPE } else { IMAGE_MEDIA_TYPE }.to_owned(),
            canvas: None,
            link_areas: None,
            local_file_url: StoryPublishStaging::stage(&staging_source),
            local_aspect: None,
            track_plate: None,
        };
        let pixel_size = item
            .video_payload
            .as_ref()
            .map(|payload| payload.pixel_size)
            .or_else(|| item.image_payload.as_ref().map(|payload| payload.pixel_size));
        if let Some(size) = pixel_size.filter(|size| size.width > 0.0 && size.height > 0.0) {
            result.local_aspect = Some(size.width / size.height);
        }
        if let Some(state) = canvas_state.clone()
            && let Some(account_user_id) = SaiAccountCacheScope::active_user_id()
            && let Some(account_epoch) = SaiAccountCacheScope::active_epoch()
        {
            let link_areas = StoryCanvasPublish::link_areas(&state);
            let wire = cx
                .update(move |cx| StoryCanvasPublish::upload_canvas(state, account_user_id, account_epoch, cx))
                .await
                .ok()
                .flatten();
            result.canvas = wire.and_then(|wire| StoryCanvasPublish::reshaped::<_, UpdateStoryCanvas>(&wire));
            result.link_areas = Some(
                link_areas
                    .iter()
                    .filter_map(|area| StoryCanvasPublish::reshaped::<_, UpdateStoryLinkArea>(area))
                    .collect(),
            );
        }
        if music_source.is_some()
            && let Some(plate) = canvas_state.as_ref().and_then(|state| state.music_plate.as_ref())
        {
            result.track_plate = Some(UpdateStoryTrackPlate {
                artist: Some(plate.artist.clone()),
                center_x: Some(f64::from(plate.center.x)),
                center_y: Some(f64::from(plate.center.y)),
                cover_url: Some(StoryTrackPlateWire::cover_url(Some(&plate.cover_url))),
                is_dark: Some(plate.is_dark),
                rotation: Some(f64::from(plate.rotation)),
                scale: Some(f64::from(plate.scale)),
                title: Some(plate.title.clone()),
            });
        }
        drop(music_render);
        Ok(result)
    }

    /// the stories own track as the editor attached music so the music tool keeps its dot
    fn attached_track(story: &StoryDTO, state: &EditorImageState, cx: &mut App) -> Option<MediaPickerTrackSelection> {
        if !Self::has_track(story) {
            return None;
        }
        let track_id = story.track_id.clone()?;
        let track = MusicServicesStore::shared(cx)
            .update(cx, |store, cx| store.sound_track(&track_id, cx))
            .unwrap_or_else(|| RiseStoredMusicTrack {
                id: Some(track_id.clone()),
                title: story.track_plate.as_ref().map(|plate| plate.title.clone()),
                cover_url: story.track_plate.as_ref().map(|plate| plate.cover_url.clone()),
                ..RiseStoredMusicTrack::default()
            });
        let plate = state.music_plate.as_ref().map(|placed| MediaPickerTrackPlate {
            is_dark: placed.is_dark,
            center_x: f64::from(placed.center.x),
            center_y: f64::from(placed.center.y),
            rotation: f64::from(placed.rotation),
            scale: f64::from(placed.scale),
        });
        Some(MediaPickerTrackSelection {
            track,
            start_ms: story.track_start_ms.unwrap_or(0),
            end_ms: story.track_end_ms.unwrap_or(0),
            plate,
        })
    }

    async fn restored_canvas(story: &StoryDTO, cx: &mut AsyncApp) -> Option<StoryMediaReeditRestoredCanvas> {
        let (done, answer) = oneshot::channel::<Option<StoryDTO>>();
        let done = Rc::new(RefCell::new(Some(done)));
        let failed = done.clone();
        let story_id = story.id;
        cx.update(move |cx| {
            let on_success: StoryActionStoryHandler = Rc::new(move |fresh, _| {
                if let Some(done) = done.borrow_mut().take() {
                    let _ = done.send(Some(fresh));
                }
            });
            let on_error: StoryActionTextHandler = Rc::new(move |_, _| {
                if let Some(done) = failed.borrow_mut().take() {
                    let _ = done.send(None);
                }
            });
            StoryActionsStore::shared(cx)
                .update(cx, |actions, cx| actions.get_story_action(story_id, on_success, on_error, cx));
        });
        let fresh = answer.await.ok().flatten();
        let canvas = fresh.and_then(|fresh| fresh.canvas).or_else(|| story.canvas.clone())?;
        if canvas.items.is_empty() {
            return None;
        }
        let blocked = canvas.items.iter().any(|item| {
            matches!(RiseStoryCanvasItemKind::from_wire(&item.kind), None | Some(RiseStoryCanvasItemKind::Sticker))
        });
        if blocked {
            return None;
        }

        let mut photos: Vec<EditorCanvasPhotoEntity> = Vec::new();
        let mut texts: Vec<EditorTextEntity> = Vec::new();
        let mut links: Vec<EditorLinkEntity> = Vec::new();
        let is_dark = cx.update(|cx| ThemeStore::shared(cx).read(cx).resolved_appearance() == RiseThemeAppearance::Dark);
        for item in &canvas.items {
            let center = point(item.center_x as f32, item.center_y as f32);
            let scale = if item.scale > 0.0 { item.scale as f32 } else { 1.0 };
            match RiseStoryCanvasItemKind::from_wire(&item.kind) {
                Some(RiseStoryCanvasItemKind::Photo) => {
                    let source = RemoteMediaURL::optional_normalized(Some(&item.file_id))?;
                    let remote = Url::from_string(&source)?;
                    let options = RemoteMediaDiskCacheMaterializeOptions {
                        suggested_filename: Some(remote.last_path_component()),
                        cache_key: Some(source.clone()),
                        ..RemoteMediaDiskCacheMaterializeOptions::default()
                    };
                    let local = RemoteMediaDiskCache::shared().ensure_cached(&remote, options, cx).await?;
                    let (photo_key, pixel_size) = cx
                        .background_executor()
                        .spawn(async move { StoryCanvasPhotoStore::shared().import_file(&local) })
                        .await?;
                    let shape = RiseStoryCanvasPhotoShape::from_wire(&item.shape).unwrap_or(RiseStoryCanvasPhotoShape::Square);
                    photos.push(EditorCanvasPhotoEntity {
                        id: Uuid::new_v4(),
                        photo_key,
                        file_id: Some(item.file_id.clone()),
                        pixel_width: pixel_size.width as i64,
                        pixel_height: pixel_size.height as i64,
                        center,
                        scale,
                        rotation: item.rotation as f32,
                        shape: shape.wire().to_owned(),
                    });
                }
                Some(RiseStoryCanvasItemKind::Text) => texts.push(EditorTextEntity {
                    id: Uuid::new_v4(),
                    text: item.text.clone(),
                    font_size: if item.font_size > 0.0 { item.font_size as f32 } else { 0.08 },
                    color: StoryCanvasBackground::color(&item.color).into(),
                    center,
                    rotation: item.rotation as f32,
                    scale,
                    style_mode: EditorTextEntityStyleMode::try_from(item.style)
                        .unwrap_or(EditorTextEntityStyleMode::Background),
                    alignment: EditorTextEntityTextAlignment::try_from(item.alignment)
                        .unwrap_or(EditorTextEntityTextAlignment::Center),
                }),
                Some(RiseStoryCanvasItemKind::Link) => {
                    let mut link = EditorLinkEntity::new(item.link_url.clone(), is_dark, center);
                    link.style = RiseMediaLinkStyle::from_wire(&item.link_style)
                        .unwrap_or(RiseMediaLinkStyle::ThemeTranslucent)
                        .wire()
                        .to_owned();
                    link.preview = RiseMediaLinkPreviewMode::from_wire(&item.link_preview)
                        .unwrap_or(RiseMediaLinkPreviewMode::Full)
                        .wire()
                        .to_owned();
                    link.title = item.link_title.clone();
                    link.summary = item.link_description.clone();
                    link.image_url = item.link_image_url.clone();
                    link.rotation = item.rotation as f32;
                    link.scale = scale;
                    links.push(link);
                }
                Some(RiseStoryCanvasItemKind::Sticker) | None => return None,
            }
        }
        let first_key = photos.first()?.photo_key.clone();
        let first_photo = cx
            .background_executor()
            .spawn(async move { StoryCanvasPhotoStore::shared().image(&first_key) })
            .await?;
        let mut state = EditorImageState::empty(
            Some(StoryServicesStore::reedit_identity(story.id)),
            StoryCanvasGeometry::RENDER_SIZE,
        );
        let background = |hex: &str| {
            if hex.is_empty() { StoryCanvasBackground::FALLBACK_HEX.to_owned() } else { hex.to_owned() }
        };
        state.story_canvas = Some(EditorStoryCanvas {
            background_top: background(&canvas.background_top),
            background_bottom: background(&canvas.background_bottom),
            photos,
        });
        state.texts = texts;
        state.links = if links.is_empty() { None } else { Some(links) };
        state.was_edited = true;
        Some(StoryMediaReeditRestoredCanvas { state, first_photo })
    }
}

type StoryVideoReeditCompletion = Box<dyn FnOnce(Option<MediaPickerItem>, &mut App)>;

/// video reedit layers on screen, each with the window it was presented from
pub struct StoryVideoReeditLauncherHosts {
    hosts: Vec<(AnyWindowHandle, Entity<StoryVideoReeditHostView>)>,
}

impl StoryVideoReeditLauncherHosts {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self { hosts: Vec::new() }
    }

    fn hosts(&self, window: &Window) -> Vec<Entity<StoryVideoReeditHostView>> {
        let window_id = window.window_handle().window_id();
        self.hosts
            .iter()
            .filter(|(presenter, _)| presenter.window_id() == window_id)
            .map(|(_, host)| host.clone())
            .collect()
    }
}

/// video twin of the still image reedit launcher, video goes through the exporter
pub enum StoryVideoReeditLauncher {}

impl StoryVideoReeditLauncher {
    fn shared(cx: &mut App) -> Entity<StoryVideoReeditLauncherHosts> {
        rise_ui::shared::shared(cx, StoryVideoReeditLauncherHosts::new)
    }

    pub fn present(
        source_url: PathBuf,
        asset_id: String,
        caption: String,
        theme_store: Entity<ThemeStore>,
        completion: impl FnOnce(Option<MediaPickerItem>, &mut App) + 'static,
        cx: &mut App,
    ) {
        let Some(presenter) = MediaPickerPresenterResolver::top_presenter(cx) else {
            completion(None, cx);
            return;
        };
        let host = cx.new(|cx| {
            StoryVideoReeditHostView::new(source_url, asset_id, caption, theme_store, Box::new(completion), cx)
        });
        Self::shared(cx).update(cx, |hosts, cx| {
            hosts.hosts.push((presenter, host));
            cx.notify();
        });
    }
}

#[derive(IntoElement, Default)]
pub struct StoryVideoReeditHostLayer;

impl StoryVideoReeditHostLayer {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for StoryVideoReeditHostLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let launcher = StoryVideoReeditLauncher::shared(cx);
        let observed = launcher.clone();
        let _observation = window.use_keyed_state(
            "story-video-reedit-layer",
            cx,
            move |_, cx: &mut Context<Subscription>| cx.observe(&observed, |_, _, cx| cx.notify()),
        );
        let hosts = launcher.read(cx).hosts(window);
        div().when(!hosts.is_empty(), |layer| layer.absolute().inset_0().children(hosts))
    }
}

pub struct StoryVideoReeditHostView {
    source_url: PathBuf,
    asset_id: String,
    theme_store: Entity<ThemeStore>,
    on_finish: Option<StoryVideoReeditCompletion>,
    selection: Entity<MediaSelectionState>,
    caption: Entity<String>,
    progress_state: Entity<ExportProgressViewState>,
    is_exporting: bool,
    opacity: Animated<f32>,
    did_appear: bool,
    did_finish: bool,
    focus_before_presentation: Option<(AnyWindowHandle, FocusHandle)>,
}

impl StoryVideoReeditHostView {
    fn new(
        source_url: PathBuf,
        asset_id: String,
        initial_caption: String,
        theme_store: Entity<ThemeStore>,
        on_finish: StoryVideoReeditCompletion,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            source_url,
            asset_id,
            theme_store,
            on_finish: Some(on_finish),
            selection: cx.new(MediaSelectionState::new),
            caption: cx.new(|_| initial_caption),
            progress_state: cx.new(ExportProgressViewState::new),
            is_exporting: false,
            opacity: Animated::new(0.0),
            did_appear: false,
            did_finish: false,
            focus_before_presentation: None,
        }
    }

    fn media_asset(asset_id: &str) -> MediaAsset {
        MediaAsset::new(asset_id.to_owned(), MediaAssetMediaType::Video, 0, 0, 0.0, None)
    }

    /// fades the layer out then answers, once
    fn finish(&mut self, item: Option<MediaPickerItem>, cx: &mut Context<Self>) {
        if self.did_finish {
            return;
        }
        self.did_finish = true;
        self.opacity.set_in(0.0, ease_in_out(CROSS_DISSOLVE_SECONDS), RiseClock::monotonic(), cx);
        cx.notify();
        let this_entity = cx.entity();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(CROSS_DISSOLVE_SECONDS)).await;
            let _ = this.update(cx, |this, cx| {
                StoryVideoReeditLauncher::shared(cx).update(cx, |hosts, cx| {
                    hosts.hosts.retain(|(_, host)| *host != this_entity);
                    cx.notify();
                });
                if let Some((window_handle, focus)) = this.focus_before_presentation.take() {
                    let _ = window_handle.update(cx, |_, window, cx| window.focus(&focus, cx));
                }
                if let Some(on_finish) = this.on_finish.take() {
                    let app: &mut App = cx;
                    on_finish(item, app);
                }
            });
        })
        .detach();
    }

    fn export(&mut self, outcome: MediaEditorOutcome, cx: &mut Context<Self>) {
        if self.is_exporting {
            return;
        }
        self.is_exporting = true;
        cx.notify();
        let source_url: PathBuf = self.source_url.clone();
        let media_asset = Self::media_asset(&self.asset_id);
        let progress_state = self.progress_state.downgrade();
        cx.spawn(async move |this, cx| {
            let exporter = MediaExporter::new();
            let exported = exporter
                .export_external(source_url, None, media_asset, outcome.edit, None, progress_state, cx)
                .await;
            let _ = this.update(cx, |this, cx| {
                this.is_exporting = false;
                this.finish(exported.ok(), cx);
            });
        })
        .detach();
    }

    fn source_video_url(path: &Path) -> Option<Url> {
        Url::from_file_path(path)
    }
}

impl Render for StoryVideoReeditHostView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        TopPresenter::note_presented("story-video-reedit-top-presenter", window, cx);
        let now = RiseClock::monotonic();
        if !self.did_appear {
            self.did_appear = true;
            self.focus_before_presentation = window.focused(cx).map(|focus| (window.window_handle(), focus));
            self.opacity.set_in(1.0, ease_in_out(CROSS_DISSOLVE_SECONDS), now, cx);
        }
        let opacity = self.opacity.frame_value(now, window);

        let caption = Binding::entity(
            self.caption.clone(),
            |caption: &String| caption.clone(),
            |caption: &mut String, value, _| *caption = value,
        );
        let cancel = cx.weak_entity();
        let done = cx.weak_entity();
        let screen = MediaEditorScreen::new(
            ElementId::Name(SharedString::from(format!("story-video-reedit-screen-{}", self.asset_id))),
            self.selection.clone(),
            caption,
            Self::media_asset(&self.asset_id),
            None,
            self.theme_store.clone(),
            MediaPickerConfiguration::chat_attachments(),
            None,
            true,
            move |_: &mut Window, cx: &mut App| {
                let _ = cancel.update(cx, |this, cx| this.finish(None, cx));
            },
            move |outcome: MediaEditorOutcome, _: &mut Window, cx: &mut App| {
                let _ = done.update(cx, |this, cx| this.export(outcome, cx));
            },
        )
        .source_video_url_override(Self::source_video_url(&self.source_url))
        .enables_standalone_swipe_dismiss(false)
        .editor_presentation(None)
        .embed_in_hero_host(false);

        div()
            .id("story-video-reedit-host")
            .absolute()
            .inset_0()
            .occlude()
            .opacity(opacity)
            .child(screen)
            .when(self.is_exporting, |host| {
                host.child(MediaPickerExportProgressOverlay::new(self.progress_state.clone(), self.theme_store.clone()))
            })
    }
}
