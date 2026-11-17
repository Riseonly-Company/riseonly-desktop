use std::cell::Cell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::{Either, select};
use gpui::AppContext;
use gpui::{
    App, AsyncApp, ClipboardEntry, Context, ElementId, Entity, Image, ImageFormat, PromptButton, PromptLevel, RenderImage,
    Size, Task, WeakEntity, Window, size,
};
use image::{Delay, Frame, RgbaImage};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_media::video_export::{poster as video_poster, probe as video_probe};
use rise_platform::capture_permission::capture_permission::{CaptureKind, capture_support};
use rise_platform::file_dialog::{FileFilter, FilterLabel};
use rise_platform::file_manager::FileManager;
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::paths::temporary_directory;
use rise_platform::photo_library::folder_library::folder_asset_kind;
use rise_platform::photo_library::{ImageContentMode, ImageDeliveryMode, LibraryAsset, LibraryAssetKind};
use rise_platform::gpui_shim::PlatformSupport;
use rise_platform::uniform_type::UTType;
use rise_theme::ThemeStore;
use rise_ui::rich_text::document::rise_document::RiseDocument;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::prelude::on_change;
use smallvec::SmallVec;

use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::modules::camera::capture::rise_camera_capture::RiseCameraShot;
use crate::modules::camera::capture::rise_camera_presenter::RiseCameraPresenter;
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatDraftCustomEmoji;
use crate::modules::chat::stores::message::message_actions::message_poll_codec::MessagePollDraft;
use crate::modules::chat::stores::message::message_actions::message_types::MessageLocationInfo;
use crate::modules::media_picker::api::media_picker_configuration::{MediaPickerConfiguration, MediaPickerSourceTab};
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions};
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellInteractionsStore;
use crate::modules::media_picker::api::media_picker_presenter_resolver::MediaPickerPresenterResolver;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerItem, MediaPickerItemKind, MediaPickerResult, MediaPickerTrackSelection, MediaPickerViewMode,
};
use crate::modules::media_picker::editor::editor_edit_preview_cache::EditorEditPreviewCache;
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::media_picker::editor::editor_state::{EditorImageState, EditorVideoState};
use crate::modules::media_picker::editor::media_editor_presentation_state::MediaEditorPresentationState;
use crate::modules::media_picker::editor::media_editor_screen::MediaEditorOutcome;
use crate::modules::media_picker::editor::media_editor_window_presenter::MediaEditorWindowLog;
use crate::modules::media_picker::export::export_progress_view_state::ExportProgressViewState;
use crate::modules::media_picker::export::media_exporter::MediaExporter;
use crate::modules::media_picker::export::media_picker_export_progress_overlay::MediaPickerExportProgressOverlay;
use crate::modules::media_picker::library::media_asset::{MediaAsset, MediaAssetMediaType};
use crate::modules::media_picker::library::photo_library_fetcher::PhotoLibraryFetcher;
use crate::modules::media_picker::library::photos_image_request::{PhotoImageDelivery, PhotosImageRequest};
use crate::modules::media_picker::selection::media_selection_state::{
    MediaSelectionEdit, MediaSelectionEditPayload, MediaSelectionState,
};
use crate::modules::media_picker::send::document_export_bridge::DocumentExportBridge;
use crate::modules::media_picker::shared::media_picker_recent_files::{
    MediaPickerRecentAccountStamp, MediaPickerRecentFile, MediaPickerRecentFiles, MediaPickerRecentFilesError,
    MediaPickerRecentKind,
};
use crate::modules::music::shared::utils::music::music_track_file_upload::MusicTrackFileUpload;

const OPTIMISTIC_POSTER_MAX_SIDE: u32 = 1_280;
const EDIT_PREVIEW_MAX_PIXEL_SIDE: f32 = 480.0;
const EDIT_PREVIEW_REQUEST_MAX_SIDE: f32 = 1_200.0;
const OPTIMISTIC_POSTER_DEADLINE: Duration = Duration::from_secs(3);
const MINIMUM_AUDIO_DURATION_MS: i64 = 500;
const LONG_VIDEO_TOLERANCE_SECONDS: f64 = 0.05;
const EXTERNAL_VIDEO_EXTENSIONS: [&str; 3] = ["mp4", "mov", "m4v"];

#[derive(Clone, Debug, PartialEq)]
pub struct MediaPickerAudioDraft {
    pub id: String,
    pub audio_url: Url,
    pub original_file_name: String,
    pub title: String,
    pub artist: String,
    pub duration: f64,
    pub cover_url: Option<Url>,
}

#[derive(Clone, Debug)]
pub struct MediaPickerExternalVideoDraft {
    pub id: String,
    pub source_url: Url,
    pub suggested_file_name: String,
    pub media_asset: MediaAsset,
}

/// layer for the export card that the picker screen and the editor layer draw over themselves
pub(crate) struct MediaPickerExportProgressPresenter {
    host: Option<(Entity<ExportProgressViewState>, Entity<ThemeStore>)>,
}

impl MediaPickerExportProgressPresenter {
    fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { host: None })
    }

    fn present(state: Entity<ExportProgressViewState>, theme_store: Entity<ThemeStore>, cx: &mut App) {
        Self::shared(cx).update(cx, |presenter, cx| {
            presenter.host = Some((state, theme_store));
            cx.notify();
        });
    }

    fn dismiss(state: &Entity<ExportProgressViewState>, cx: &mut App) {
        Self::shared(cx).update(cx, |presenter, cx| {
            if presenter.host.as_ref().is_some_and(|(host, _)| host == state) {
                presenter.host = None;
                cx.notify();
            }
        });
    }

    /// card to draw over the current surface while an export is up
    /// reading it through the shared entity makes the callers view re-render when the card comes and goes
    pub(crate) fn overlay(cx: &mut App) -> Option<MediaPickerExportProgressOverlay> {
        let presenter = Self::shared(cx);
        let host = presenter.read(cx).host.clone()?;
        Some(MediaPickerExportProgressOverlay::new(host.0, host.1))
    }
}

pub struct MediaPickerSession {
    pub configuration: MediaPickerConfiguration,
    pub theme_store: Entity<ThemeStore>,
    pub selection: Entity<MediaSelectionState>,
    pub caption: Binding<String>,
    pub caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>,
    pub fetcher: Entity<PhotoLibraryFetcher>,
    pub editor_presentation: Entity<MediaEditorPresentationState>,
    account_stamp: MediaPickerRecentAccountStamp,
    did_finish: bool,
    pub source_tab: MediaPickerSourceTab,
    pub search_query: String,
    pub is_searching: bool,
    pub pending_audio_draft: Option<MediaPickerAudioDraft>,
    pub pending_external_video_draft: Option<MediaPickerExternalVideoDraft>,
    is_exporting_external_video: bool,
    pub external_video_export_progress: Entity<ExportProgressViewState>,
    pub error_text: Option<String>,

    on_complete: Rc<dyn Fn(MediaPickerResult, &mut Window, &mut App)>,
    export_task: Option<Task<()>>,
    attachment_import_task: Option<Task<()>>,
    attachment_import_cancelled: Rc<Cell<bool>>,
    active_exporter: Option<MediaExporter>,
    staged_audio_urls: Vec<PathBuf>,
    document_panel: Option<Task<()>>,
}

impl Drop for MediaPickerSession {
    fn drop(&mut self) {
        tracing::warn!(target: "riseonly::X844", finished = self.did_finish, exporting = self.export_task.is_some(), "picker session dropped");
    }
}

impl MediaPickerSession {
    /// defaults are caption, caption custom emojis and initial track all none
    pub fn new(
        configuration: MediaPickerConfiguration,
        theme_store: Entity<ThemeStore>,
        caption: Option<Binding<String>>,
        caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>,
        initial_track: Option<MediaPickerTrackSelection>,
        on_complete: Rc<dyn Fn(MediaPickerResult, &mut Window, &mut App)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let selection = cx.new(|cx| {
            let mut selection = MediaSelectionState::new(cx);
            selection.attached_track = initial_track;
            selection.max_allowed = configuration.max_selection_count;
            selection.replaces_single_pick =
                configuration.max_selection_count == 1 && !configuration.shows_selection_controls;
            selection.enforces_single_kind = configuration.enforces_single_kind;
            selection.max_video_count = configuration.max_video_count;
            selection.max_image_count = configuration.max_image_count;
            selection
        });
        let caption = caption.unwrap_or_else(|| {
            Binding::entity(
                selection.clone(),
                |selection: &MediaSelectionState| selection.shared_caption.clone(),
                |selection: &mut MediaSelectionState, value: String, _| selection.shared_caption = value,
            )
        });
        let fetcher = cx.new(|cx| PhotoLibraryFetcher::new(configuration.kind, cx));
        let source_tab = configuration.source_tabs.first().copied().unwrap_or(MediaPickerSourceTab::Gallery);
        Self {
            configuration,
            theme_store,
            selection,
            caption,
            caption_custom_emojis,
            fetcher,
            editor_presentation: cx.new(|_| MediaEditorPresentationState::new()),
            account_stamp: MediaPickerRecentAccountStamp::current(),
            did_finish: false,
            source_tab,
            search_query: String::new(),
            is_searching: false,
            pending_audio_draft: None,
            pending_external_video_draft: None,
            is_exporting_external_video: false,
            external_video_export_progress: cx.new(ExportProgressViewState::new),
            error_text: None,
            on_complete,
            export_task: None,
            attachment_import_task: None,
            attachment_import_cancelled: Rc::new(Cell::new(false)),
            active_exporter: None,
            staged_audio_urls: Vec::new(),
            document_panel: None,
        }
    }

    pub fn did_finish(&self) -> bool {
        self.did_finish
    }

    pub fn is_exporting_external_video(&self) -> bool {
        self.is_exporting_external_video
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X844", exporting = self.active_exporter.is_some(), "picker session cancel");
        self.attachment_import_cancelled.set(true);
        if let Some(active_exporter) = &self.active_exporter {
            active_exporter.cancel();
        }
        self.pending_external_video_draft = None;
        self.pending_audio_draft = None;
        self.finish(MediaPickerResult::CANCELLED, cx);
    }

    pub fn reset_completion(&mut self, cx: &mut Context<Self>) {
        self.did_finish = false;
        cx.notify();
    }

    pub fn handle_editor_outcome(
        &mut self,
        outcome: MediaEditorOutcome,
        media_asset: MediaAsset,
        cx: &mut Context<Self>,
    ) {
        let selected_asset = self.selection.read(cx).assets_by_identifier().get(&media_asset.id).cloned();
        let ph_asset = selected_asset.or_else(|| {
            let fetcher = self.fetcher.read(cx);
            fetcher
                .fetch_result()
                .iter()
                .find(|asset| asset.id == media_asset.id)
                .or_else(|| fetcher.picked_asset(&media_asset.id))
                .cloned()
        });
        tracing::warn!(target: "riseonly::X908", asset = %media_asset.id, found = ph_asset.is_some(), immediate = outcome.immediate_send, selects = outcome.selects_asset, "editor outcome");
        let Some(ph_asset) = ph_asset else {
            return;
        };
        let selected = outcome.selects_asset
            && self.selection.update(cx, |selection, cx| selection.ensure_selected(&ph_asset, cx));
        if selected {
            let edit = outcome.edit.clone();
            self.selection.update(cx, |selection, cx| selection.set_edit_for_asset(edit, &ph_asset, cx));
            if let MediaSelectionEditPayload::Image(state) = &outcome.edit.payload {
                self.refresh_edit_preview(&media_asset.id, &ph_asset, state.clone(), cx);
            }
        } else {
            if !outcome.immediate_send || self.selection.read(cx).is_empty() {
                return;
            }
            let edit = outcome.edit.clone();
            self.selection.update(cx, |selection, cx| selection.set_edit(edit, &ph_asset.id, cx));
        }
        if outcome.immediate_send || self.configuration.auto_confirm_after_editing {
            tracing::warn!(target: "riseonly::X908", selected, "editor outcome exports");
            self.export_selection(false, cx);
        }
    }

    fn refresh_edit_preview(
        &mut self,
        asset_id: &str,
        ph_asset: &LibraryAsset,
        state: EditorImageState,
        cx: &mut Context<Self>,
    ) {
        if !state.was_edited {
            EditorEditPreviewCache::remove(asset_id, cx);
            return;
        }
        let target = size(
            EDIT_PREVIEW_REQUEST_MAX_SIDE.min(ph_asset.pixel_width as f32),
            EDIT_PREVIEW_REQUEST_MAX_SIDE.min(ph_asset.pixel_height as f32),
        );
        let (deliveries, mut delivered) = mpsc::unbounded::<PhotoImageDelivery>();
        let _ = PhotosImageRequest::request_image(
            &ph_asset.id,
            target,
            ImageContentMode::AspectFit,
            ImageDeliveryMode::HighQuality,
            true,
            move |delivery| {
                let _ = deliveries.unbounded_send(delivery);
            },
        );
        let asset_id = asset_id.to_string();
        cx.spawn(async move |_, cx| {
            while let Some(delivery) = delivered.next().await {
                let Some(image) = delivery.image else {
                    continue;
                };
                if delivery.is_degraded {
                    continue;
                }
                let state = state.clone();
                let thumb = cx
                    .background_executor().spawn(async move {
                        EditorImageBake::thumbnail_preview(&image, &state, EDIT_PREVIEW_MAX_PIXEL_SIDE)
                    })
                    .await;
                let asset_id = asset_id.clone();
                cx.update(|cx| EditorEditPreviewCache::store(Some(thumb), &asset_id, cx));
            }
        })
        .detach();
    }

    pub fn export_selection(&mut self, as_file: bool, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X844", busy = self.export_task.is_some(), account = self.account_stamp.is_current(), "picker export requested");
        if self.export_task.is_some() || !self.account_stamp.is_current() {
            return;
        }
        let assets = self.selection.read(cx).ordered_assets();
        if assets.is_empty() {
            return;
        }
        if let Some(message_key) = self.configuration.untrimmed_long_video_message_key.as_deref()
            && self.has_untrimmed_long_video(&assets, cx)
        {
            self.error_text = Some(tr(message_key));
            cx.notify();
            return;
        }
        let shared_caption = self.caption.get(cx);
        if let Some(refusal) = self.configuration.caption_rule.as_ref().and_then(|rule| rule.error_text(&shared_caption))
        {
            self.error_text = Some(refusal);
            cx.notify();
            return;
        }
        let mut edits = self.selection.read(cx).edits_by_identifier().clone();
        for asset in &assets {
            let id = asset.id.clone();
            let existing = edits.get(&id).map(|edit| edit.payload.clone());
            let payload = match existing {
                Some(MediaSelectionEditPayload::Image(mut state)) => {
                    state.synchronize_edit_flag();
                    state.caption = shared_caption.clone();
                    MediaSelectionEditPayload::Image(state)
                }
                Some(MediaSelectionEditPayload::Video(mut state)) => {
                    if let Some(spatial) = &mut state.spatial_edit {
                        spatial.synchronize_edit_flag();
                    }
                    state.was_edited = state.has_meaningful_edits();
                    state.caption = shared_caption.clone();
                    MediaSelectionEditPayload::Video(state)
                }
                None => {
                    if asset.kind == LibraryAssetKind::Video {
                        let mut state = self
                            .clamp_video_edit_state(EditorVideoState::empty(Some(id.clone()), asset.duration_secs));
                        state.caption = shared_caption.clone();
                        MediaSelectionEditPayload::Video(state)
                    } else {
                        let mut state = EditorImageState::empty(
                            Some(id.clone()),
                            size(asset.pixel_width as f32, asset.pixel_height as f32),
                        );
                        state.caption = shared_caption.clone();
                        MediaSelectionEditPayload::Image(state)
                    }
                }
            };
            edits.insert(id, MediaSelectionEdit { payload });
        }

        let progress_view_state = cx.new(ExportProgressViewState::new);
        let total = i64::try_from(assets.len()).unwrap_or(i64::MAX);
        progress_view_state.update(cx, |state, cx| state.reset(total, cx));
        let mut progress_host: Option<Entity<ExportProgressViewState>> = None;
        if self.configuration.shows_export_progress_overlay {
            if MediaPickerPresenterResolver::top_presenter(cx).is_none() {
                self.error_text = Some(tr("error"));
                cx.notify();
                return;
            }
            let editor_presented = self.editor_presentation.read(cx).is_presented;
            MediaEditorWindowLog::event("export-overlay", || format!("editorPresented={editor_presented}"), cx);
            MediaPickerExportProgressPresenter::present(progress_view_state.clone(), self.theme_store.clone(), cx);
            progress_host = Some(progress_view_state.clone());
        }

        let exporter = MediaExporter::new();
        self.active_exporter = Some(exporter.clone());
        let defer_video_processing = self.configuration.defers_video_processing_to_consumer;
        let keeps_animated_images = self.configuration.keeps_animated_images;
        let attached_track = if self.configuration.consumer_mixes_attached_track {
            None
        } else {
            self.selection.read(cx).attached_track.clone()
        };
        self.export_task = Some(cx.spawn(async move |this, cx| {
            let items = exporter
                .export(
                    assets,
                    edits,
                    as_file,
                    defer_video_processing,
                    keeps_animated_images,
                    attached_track,
                    progress_view_state.downgrade(),
                    cx,
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.active_exporter = None;
                this.release_export_task();
                if let Some(progress_host) = &progress_host {
                    MediaPickerExportProgressPresenter::dismiss(progress_host, cx);
                }
                let (failure, is_cancelled) = {
                    let progress = progress_view_state.read(cx);
                    (progress.failure_description.clone(), progress.is_cancelled)
                };
                tracing::warn!(target: "riseonly::X844", items = items.len(), failed = failure.is_some(), is_cancelled, "picker export done");
                if let Some(failure) = failure
                    && !is_cancelled
                {
                    this.error_text = Some(failure);
                    cx.notify();
                    return;
                }
                if items.is_empty() && !is_cancelled {
                    this.error_text = Some(tr("error"));
                    cx.notify();
                    return;
                }
                if items.is_empty() {
                    return;
                }
                let (hides_under_spoiler, selected_view_mode) = {
                    let selection = this.selection.read(cx);
                    (selection.hides_under_spoiler, selection.view_mode.clone())
                };
                let spoiler = this.configuration.allows_media_spoiler && hides_under_spoiler && !as_file;
                let view_mode =
                    if this.configuration.allows_view_once && !as_file { selected_view_mode } else { None };
                let sent = if spoiler || view_mode.is_some() {
                    items
                        .into_iter()
                        .map(|mut item| {
                            if item.is_file
                                || !matches!(item.kind, MediaPickerItemKind::Image | MediaPickerItemKind::Video)
                            {
                                return item;
                            }
                            if spoiler {
                                item.has_spoiler = true;
                            }
                            item.view_mode = if item.kind == MediaPickerItemKind::Video
                                && view_mode.as_ref().and_then(MediaPickerViewMode::seconds).is_some()
                            {
                                Some(MediaPickerViewMode::Once)
                            } else {
                                view_mode.clone()
                            };
                            item
                        })
                        .collect()
                } else {
                    items
                };
                this.finish(MediaPickerResult::new(sent, false), cx);
            });
        }));
        cx.notify();
    }

    fn release_export_task(&mut self) {
        if let Some(task) = self.export_task.take() {
            task.detach();
        }
    }

    fn release_attachment_import_task(&mut self) {
        if let Some(task) = self.attachment_import_task.take() {
            task.detach();
        }
    }

    fn begin_attachment_import(&mut self) -> Rc<Cell<bool>> {
        let cancelled = Rc::new(Cell::new(false));
        self.attachment_import_cancelled = cancelled.clone();
        cancelled
    }

    pub fn error_alert(id: ElementId, session: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let error_text = session.read(cx).error_text.clone();
        let session = session.clone();
        on_change(id, error_text, false, window, cx, move |_, message, window, cx| {
            let Some(message) = message else {
                return;
            };
            let answer = window.prompt(
                PromptLevel::Info,
                &tr("error"),
                Some(message.as_str()),
                &[PromptButton::cancel(tr("ok"))],
                cx,
            );
            cx.spawn(async move |cx| {
                let _ = answer.await;
                session.update(cx, |session, cx| {
                    session.error_text = None;
                    cx.notify();
                });
            })
            .detach();
        });
    }

    pub fn present_document_picker(&mut self, for_tab: MediaPickerSourceTab, cx: &mut Context<Self>) {
        if !self.account_stamp.is_current() {
            return;
        }
        let Some(presenter) = MediaPickerPresenterResolver::top_presenter(cx) else {
            return;
        };
        let (content_types, allows_multiple_selection) = match for_tab {
            MediaPickerSourceTab::Audio => {
                let mut extensions: Vec<&str> = MusicTrackFileUpload::CHAT_AUDIO_EXTENSIONS.to_vec();
                extensions.sort_unstable();
                let audio_types: Vec<UTType> =
                    extensions.into_iter().filter_map(UTType::from_filename_extension).collect();
                let content_types = if audio_types.is_empty() { vec![UTType::audio()] } else { audio_types };
                (content_types, false)
            }
            MediaPickerSourceTab::File => {
                (self.allowed_document_types(), self.configuration.max_selection_count > 1)
            }
            _ => return,
        };
        let file_filter = if for_tab == MediaPickerSourceTab::File && self.configuration.offers_file_chooser {
            FileFilter::new(FilterLabel::IMAGES, self.configuration.allowed_file_extensions.iter().flatten())
        } else {
            FileFilter::any()
        };
        let session = cx.weak_entity();
        let refused_session = session.clone();
        let bridge = DocumentExportBridge::new(
            Some(content_types),
            allows_multiple_selection,
            Box::new(move |urls: Vec<PathBuf>, cx: &mut App| {
                let _ = session.update(cx, |session, cx| session.handle_document_picker_result(urls, for_tab, cx));
            }),
        )
        .file_filter(file_filter)
        .on_failure(Box::new(move |message: String, cx: &mut App| {
            let _ = refused_session.update(cx, |session, cx| {
                session.error_text = Some(message);
                cx.notify();
            });
        }));
        self.document_panel = Some(bridge.present_owned(presenter, cx));
    }

    /// desktop only, files dropped on the picker window or a document page take the document picker path
    /// so the preset count and extension rules hold, no file source or chooser means nowhere to put them
    /// on a media page of a picker listing brought files images and videos join the selection
    pub fn receive_external_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let takes_media = self.source_tab != MediaPickerSourceTab::Audio
            && self.source_tab != MediaPickerSourceTab::File
            && self.fetcher.read(cx).reads_picked_files();
        let paths = if takes_media {
            let (media, rest): (Vec<PathBuf>, Vec<PathBuf>) =
                paths.into_iter().partition(|path| folder_asset_kind(path).is_some());
            PhotoLibraryFetcher::add_picked_files(&self.fetcher, &self.selection, media, cx);
            if rest.is_empty() {
                return;
            }
            rest
        } else {
            paths
        };
        let tab = if self.source_tab == MediaPickerSourceTab::Audio {
            MediaPickerSourceTab::Audio
        } else {
            MediaPickerSourceTab::File
        };
        if !self.configuration.source_tabs.contains(&tab) && !self.configuration.offers_file_chooser {
            return;
        }
        let urls: Vec<PathBuf> = paths.into_iter().filter(|path| path.is_file()).collect();
        self.handle_document_picker_result(urls, tab, cx);
    }

    pub fn import_pasteboard(&mut self, cx: &mut Context<Self>) {
        let urls = UIPasteboard::general().file_paths(cx);
        if !urls.is_empty() {
            self.receive_external_paths(urls, cx);
            return;
        }
        let image = cx.read_from_clipboard().and_then(|clipboard| {
            clipboard.entries().iter().find_map(|entry| match entry {
                ClipboardEntry::Image(pasted) => Some(pasted.clone()),
                ClipboardEntry::ExternalPaths(_) | ClipboardEntry::String(_) => None,
            })
        });
        let Some(image) = image else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let written = cx.background_executor().spawn(async move { Self::write_pasted_image(&image) }).await;
            let _ = this.update(cx, |this, cx| match written {
                Ok(url) => this.receive_external_paths(vec![url], cx),
                Err(error) => {
                    this.error_text = Some(error.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// desktop only, stages a clipboard image as a file in the temp directory
    /// runs off the main thread since a tiff or bmp is decoded here
    pub fn write_pasted_image(image: &Image) -> std::io::Result<PathBuf> {
        let source_format = match image.format {
            ImageFormat::Tiff => Some(image::ImageFormat::Tiff),
            ImageFormat::Bmp => Some(image::ImageFormat::Bmp),
            ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Webp
            | ImageFormat::Gif
            | ImageFormat::Svg
            | ImageFormat::Ico
            | ImageFormat::Pnm => None,
        };
        let file_name = |extension: &str| format!("ro-paste-{}.{extension}", Uuid::new_v4().uuid_string());
        let Some(source_format) = source_format else {
            let url = temporary_directory().join(file_name(Self::pasted_image_extension(image.format)));
            return std::fs::write(&url, image.bytes()).map(|()| url);
        };
        let decoded = image::load_from_memory_with_format(image.bytes(), source_format).map_err(std::io::Error::other)?;
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(decoded.into_rgba8())
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(std::io::Error::other)?;
        let url = temporary_directory().join(file_name("png"));
        std::fs::write(&url, png.into_inner()).map(|()| url)
    }

    fn pasted_image_extension(format: ImageFormat) -> &'static str {
        match format {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
            ImageFormat::Gif => "gif",
            ImageFormat::Svg => "svg",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Tiff => "tiff",
            ImageFormat::Ico => "ico",
            ImageFormat::Pnm => "pnm",
        }
    }

    pub fn select_recent(&mut self, entry: &MediaPickerRecentFile, cx: &mut Context<Self>) {
        if !self.account_stamp.is_current() {
            return;
        }
        if !self.is_allowed_document(&entry.local_url()) {
            self.error_text = Some(tr("sticker_pack_media_unsupported"));
            cx.notify();
            return;
        }
        let item = MediaPickerRecentFiles::shared(cx).update(cx, |files, cx| files.item(entry, cx));
        let Some(item) = item else {
            return;
        };
        if entry.kind == MediaPickerRecentKind::Audio
            && let Some(audio) = item.audio_payload.clone()
        {
            self.discard_staged_audio(cx);
            self.pending_audio_draft = Some(MediaPickerAudioDraft {
                id: Uuid::new_v4().uuid_string(),
                audio_url: item.url.clone(),
                original_file_name: item.suggested_file_name.clone(),
                title: audio.title,
                artist: audio.artist,
                duration: audio.duration,
                cover_url: audio.cover_url,
            });
            cx.notify();
        } else {
            if self.attachment_import_task.is_some() {
                return;
            }
            let cancelled = self.begin_attachment_import();
            self.attachment_import_task = Some(cx.spawn(async move |this, cx| {
                Self::route_document_items(this.clone(), vec![item], cancelled, cx).await;
                let _ = this.update(cx, |this, _| this.release_attachment_import_task());
            }));
        }
    }

    async fn optimistic_external_video_poster(
        source_url: PathBuf,
        state: &EditorVideoState,
        cx: &mut AsyncApp,
    ) -> Option<Url> {
        let spatial_edit = state.spatial_edit.clone().filter(EditorImageState::has_baked_edits)?;
        let start_seconds = state.start_seconds;
        let frame = cx.background_executor().spawn(async move {
            let frame = video_poster(&source_url, start_seconds.max(0.0), OPTIMISTIC_POSTER_MAX_SIDE).ok()?;
            let source = Self::render_image_from_poster_frame(frame.width, frame.height, frame.bgra)?;
            MediaExporter::write_optimistic_media_preview(&source, &spatial_edit)
        });
        let deadline = cx.background_executor().timer(OPTIMISTIC_POSTER_DEADLINE);
        let poster = match select(pin!(frame), pin!(deadline)).await {
            Either::Left((poster, _)) => poster,
            Either::Right(((), _)) => None,
        };
        poster.and_then(|path| Url::from_file_path(&path))
    }

    fn render_image_from_poster_frame(width: u32, height: u32, bgra: Vec<u8>) -> Option<CleverImage> {
        let bitmap = RgbaImage::from_raw(width, height, bgra)?;
        let frame = Frame::from_parts(bitmap, 0, 0, Delay::from_numer_denom_ms(0, 1));
        let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([frame]);
        Some(Arc::new(RenderImage::new(frames)))
    }

    pub fn cancel_external_video_edit(&mut self, cx: &mut Context<Self>) {
        if self.is_exporting_external_video {
            return;
        }
        self.pending_external_video_draft = None;
        cx.notify();
    }

    pub fn complete_external_video_edit(
        &mut self,
        draft: &MediaPickerExternalVideoDraft,
        outcome: MediaEditorOutcome,
        cx: &mut Context<Self>,
    ) {
        let is_pending = self.pending_external_video_draft.as_ref().is_some_and(|pending| pending.id == draft.id);
        if self.is_exporting_external_video || !is_pending || !self.account_stamp.is_current() || self.did_finish {
            return;
        }

        let mut edit = outcome.edit;
        if let MediaSelectionEditPayload::Video(state) = &edit.payload {
            edit = MediaSelectionEdit {
                payload: MediaSelectionEditPayload::Video(self.clamp_video_edit_state(state.clone())),
            };
        }

        if self.configuration.defers_video_processing_to_consumer
            && let MediaSelectionEditPayload::Video(state) = &edit.payload
        {
            let state = state.clone();
            let source_url = draft.source_url.clone();
            let suggested_file_name = draft.suggested_file_name.clone();
            let pixel_size: Size<f32> = size(
                draft.media_asset.pixel_width.max(1) as f32,
                draft.media_asset.pixel_height.max(1) as f32,
            );
            self.pending_external_video_draft = None;
            cx.notify();
            cx.spawn(async move |this, cx| {
                let is_live = this
                    .read_with(cx, |this, _| this.account_stamp.is_current() && !this.did_finish)
                    .unwrap_or(false);
                if !is_live {
                    return;
                }
                let optimistic_poster_url = match source_url.to_file_path() {
                    Some(path) => Self::optimistic_external_video_poster(path, &state, cx).await,
                    None => None,
                };
                let item = MediaPickerItem::deferred_external_video(
                    source_url,
                    suggested_file_name,
                    pixel_size,
                    state,
                    optimistic_poster_url,
                );
                let _ = this.update(cx, |this, cx| {
                    if !this.account_stamp.is_current() || this.did_finish {
                        return;
                    }
                    this.finish(MediaPickerResult::new(vec![item], false), cx);
                });
            })
            .detach();
            return;
        }

        let Some(source_path) = draft.source_url.to_file_path() else {
            self.error_text = Some(tr("error"));
            cx.notify();
            return;
        };
        let exporter = MediaExporter::new();
        self.active_exporter = Some(exporter.clone());
        self.is_exporting_external_video = true;
        let attached_track = if self.configuration.consumer_mixes_attached_track {
            None
        } else {
            self.selection.read(cx).attached_track.clone()
        };
        let progress_state = self.external_video_export_progress.downgrade();
        let draft = draft.clone();
        self.export_task = Some(cx.spawn(async move |this, cx| {
            let exported = exporter
                .export_external(source_path, None, draft.media_asset.clone(), edit, attached_track, progress_state, cx)
                .await;
            match exported {
                Ok(item) => {
                    let delivered = this.update(cx, |this, cx| {
                        let is_pending =
                            this.pending_external_video_draft.as_ref().is_some_and(|pending| pending.id == draft.id);
                        if !this.account_stamp.is_current() || this.did_finish || !is_pending {
                            return Err(item);
                        }
                        this.active_exporter = None;
                        this.release_export_task();
                        this.is_exporting_external_video = false;
                        this.pending_external_video_draft = None;
                        this.finish(MediaPickerResult::new(vec![item], false), cx);
                        Ok(())
                    });
                    if let Ok(Err(item)) = delivered {
                        Self::remove_generated_external_video(&item);
                    }
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        let is_pending =
                            this.pending_external_video_draft.as_ref().is_some_and(|pending| pending.id == draft.id);
                        if !is_pending {
                            return;
                        }
                        let is_cancellation = error.downcast_ref::<CancellationError>().is_some();
                        if !is_cancellation && this.did_finish {
                            return;
                        }
                        this.active_exporter = None;
                        this.release_export_task();
                        this.is_exporting_external_video = false;
                        if !is_cancellation {
                            this.pending_external_video_draft = None;
                            this.error_text = Some(error.to_string());
                        }
                        cx.notify();
                    });
                }
            }
        }));
        cx.notify();
    }

    /// true if the device has a camera for the live tile
    /// a sheet also needs shows_camera_tile in its config, the feed picker shows it on its own
    pub fn offers_camera_tile(&self) -> bool {
        matches!(capture_support(CaptureKind::Camera), PlatformSupport::Performed)
    }

    pub fn open_camera(&mut self, opens_video_editor: bool, cx: &mut Context<Self>) {
        if !self.account_stamp.is_current() || self.did_finish || self.export_task.is_some() {
            return;
        }
        let session = cx.weak_entity();
        RiseCameraPresenter::present_for_capture(
            self.configuration.clone(),
            Box::new(move |shot: RiseCameraShot, cx: &mut App| {
                let Some(session) = session.upgrade() else {
                    shot.discard();
                    return;
                };
                session.update(cx, |session, cx| {
                    if !session.account_stamp.is_current() || session.did_finish {
                        shot.discard();
                        return;
                    }
                    session.take_camera_shot(shot, opens_video_editor, cx);
                });
            }),
            cx,
        );
    }

    fn take_camera_shot(&mut self, shot: RiseCameraShot, opens_video_editor: bool, cx: &mut Context<Self>) {
        let shared_caption = self.caption.get(cx);
        let media_asset = shot.media_asset();
        if shot.is_video() {
            let draft = MediaPickerExternalVideoDraft {
                id: shot.id.clone(),
                source_url: shot.url.clone(),
                suggested_file_name: shot.url.last_path_component(),
                media_asset: media_asset.clone(),
            };
            self.pending_external_video_draft = Some(draft.clone());
            cx.notify();
            if opens_video_editor {
                return;
            }
            let mut state = EditorVideoState::empty(Some(media_asset.id.clone()), shot.duration);
            state.caption = shared_caption;
            self.complete_external_video_edit(
                &draft,
                MediaEditorOutcome {
                    edit: MediaSelectionEdit { payload: MediaSelectionEditPayload::Video(state) },
                    immediate_send: true,
                    selects_asset: false,
                },
                cx,
            );
            return;
        }
        if let Some(refusal) = self.configuration.caption_rule.as_ref().and_then(|rule| rule.error_text(&shared_caption))
        {
            shot.discard();
            self.error_text = Some(refusal);
            cx.notify();
            return;
        }
        let Some(source_path) = shot.url.to_file_path() else {
            shot.discard();
            self.error_text = Some(tr("error"));
            cx.notify();
            return;
        };
        let mut state = EditorImageState::empty(Some(media_asset.id.clone()), shot.pixel_size);
        state.caption = shared_caption;
        let edit = MediaSelectionEdit { payload: MediaSelectionEditPayload::Image(state) };
        let track = if self.configuration.consumer_mixes_attached_track {
            None
        } else {
            self.selection.read(cx).attached_track.clone()
        };
        let exporter = MediaExporter::new();
        self.active_exporter = Some(exporter.clone());
        let progress_state = cx.new(ExportProgressViewState::new);
        let source_image = shot.image.clone();
        self.export_task = Some(cx.spawn(async move |this, cx| {
            let exported = exporter
                .export_external(source_path, source_image, media_asset, edit, track, progress_state.downgrade(), cx)
                .await;
            match exported {
                Ok(item) => {
                    let _ = this.update(cx, |this, cx| {
                        this.active_exporter = None;
                        this.release_export_task();
                        if !this.account_stamp.is_current() || this.did_finish {
                            return;
                        }
                        this.finish(MediaPickerResult::new(vec![item], false), cx);
                    });
                }
                Err(error) => {
                    shot.discard();
                    let _ = this.update(cx, |this, cx| {
                        this.active_exporter = None;
                        this.release_export_task();
                        if error.downcast_ref::<CancellationError>().is_some() || this.did_finish {
                            return;
                        }
                        this.error_text = Some(error.to_string());
                        cx.notify();
                    });
                }
            }
        }));
    }

    pub fn cancel_audio_draft(&mut self, cx: &mut Context<Self>) {
        self.pending_audio_draft = None;
        self.discard_staged_audio(cx);
        cx.notify();
    }

    pub fn complete_audio_draft(&mut self, draft: MediaPickerAudioDraft, cx: &mut Context<Self>) {
        let clean_title = trimming_whitespaces_and_newlines(&draft.title).to_string();
        let clean_artist = trimming_whitespaces_and_newlines(&draft.artist).to_string();
        if self.attachment_import_task.is_some()
            || !self.account_stamp.is_current()
            || clean_title.is_empty()
            || clean_artist.is_empty()
            || draft.duration <= 0.0
        {
            return;
        }
        let Some(audio_path) = draft.audio_url.to_file_path() else {
            return;
        };
        let file_extension = draft.audio_url.path_extension().to_lowercase();
        let display_name = format!(
            "{clean_artist} - {clean_title}.{}",
            if file_extension.is_empty() { "mp3" } else { &file_extension }
        );
        let cover_path = draft.cover_url.as_ref().and_then(Url::to_file_path);
        let account_stamp = self.account_stamp.clone();
        let duration = draft.duration;
        let cancelled = self.begin_attachment_import();
        self.attachment_import_task = Some(cx.spawn(async move |this, cx| {
            let Ok(staged) = this.update(cx, |this, _| this.claim_staged_audio()) else {
                return;
            };
            let import = cx.update(|cx| {
                MediaPickerRecentFiles::shared(cx).update(cx, |files, cx| {
                    files.import_file_async(
                        audio_path,
                        MediaPickerRecentKind::Audio,
                        display_name,
                        Some(clean_title),
                        Some(clean_artist),
                        Some(duration),
                        cover_path,
                        Some(account_stamp),
                        cx,
                    )
                })
            });
            match import.await {
                Ok(entry) => {
                    cx.update(|cx| Self::remove_generated_files(staged.clone(), cx)).await;
                    if cancelled.get() {
                        let _ = this.update(cx, |this, cx| {
                            this.release_attachment_import_task();
                            this.release_staged_audio(staged, cx);
                        });
                        return;
                    }
                    let _ = this.update(cx, |this, cx| {
                        this.release_attachment_import_task();
                        if !this.account_stamp.is_current() || this.did_finish {
                            return;
                        }
                        let item = MediaPickerRecentFiles::shared(cx).update(cx, |files, cx| files.item(&entry, cx));
                        let Some(item) = item else {
                            return;
                        };
                        this.pending_audio_draft = None;
                        this.finish(MediaPickerResult::new(vec![item], false), cx);
                    });
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.release_attachment_import_task();
                        this.release_staged_audio(staged, cx);
                        if matches!(
                            error,
                            MediaPickerRecentFilesError::Cancelled
                        ) || this.did_finish
                        {
                            return;
                        }
                        this.error_text = Some(Self::recent_files_error_text(&error));
                        cx.notify();
                    });
                }
            }
        }));
    }

    fn recent_files_error_text(
        error: &MediaPickerRecentFilesError,
    ) -> String {
        match error {
            MediaPickerRecentFilesError::Cancelled => CancellationError.to_string(),
            MediaPickerRecentFilesError::File(error) => error.to_string(),
        }
    }

    fn handle_document_picker_result(
        &mut self,
        urls: Vec<PathBuf>,
        tab: MediaPickerSourceTab,
        cx: &mut Context<Self>,
    ) {
        tracing::warn!(
            target: "riseonly::X1428",
            urls = urls.len(),
            importing = self.attachment_import_task.is_some(),
            current = self.account_stamp.is_current(),
            finished = self.did_finish,
            "document pick received"
        );
        if self.attachment_import_task.is_some() || !self.account_stamp.is_current() || urls.is_empty() {
            return;
        }
        if tab == MediaPickerSourceTab::Audio {
            self.prepare_audio_draft(urls[0].clone(), cx);
            return;
        }

        let limit = usize::try_from(self.configuration.max_selection_count.max(1)).unwrap_or(1);
        let overflowed = urls.len() > limit;
        let selected_urls: Vec<PathBuf> = urls.into_iter().take(limit).collect();
        if !selected_urls.iter().all(|url| self.is_allowed_document(url)) {
            tracing::warn!(target: "riseonly::X1428", "document pick refused by extension");
            self.error_text = Some(tr("sticker_pack_media_unsupported"));
            cx.notify();
            return;
        }
        if selected_urls.iter().any(|url| url.is_dir()) {
            tracing::warn!(target: "riseonly::X1428", "document pick refused a folder");
            self.error_text = Some(tr("sticker_pack_media_unsupported"));
            cx.notify();
            return;
        }

        if overflowed {
            self.announce_document_overflow(cx);
        }

        let account_stamp = self.account_stamp.clone();
        let cancelled = self.begin_attachment_import();
        self.attachment_import_task = Some(cx.spawn(async move |this, cx| {
            Self::import_document_urls(this.clone(), selected_urls, account_stamp, cancelled, cx).await;
            let _ = this.update(cx, |this, _| this.release_attachment_import_task());
        }));
    }

    fn announce_document_overflow(&self, cx: &mut Context<Self>) {
        let maximum = self.configuration.max_selection_count;
        if let Some(premium_ceiling) = self.configuration.premium_selection_ceiling {
            PremiumUpsellInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.offer_client_ceiling("media_picker_selection", maximum, Some(premium_ceiling), cx);
            });
            return;
        }
        let message_key = self.configuration.selection_limit_message_key.as_deref().unwrap_or("share_error_too_many_items");
        Notifier::send(tr_args(message_key, &[FormatArg::Int(maximum)]), NotifierSendOptions::default(), cx);
    }

    async fn import_document_urls(
        this: WeakEntity<Self>,
        selected_urls: Vec<PathBuf>,
        account_stamp: MediaPickerRecentAccountStamp,
        cancelled: Rc<Cell<bool>>,
        cx: &mut AsyncApp,
    ) {
        let mut items: Vec<MediaPickerItem> = Vec::new();
        for url in selected_urls {
            if cancelled.get() {
                return;
            }
            let last_path_component = url.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let suggested_name = if last_path_component.is_empty() {
                let uuid = Uuid::new_v4().uuid_string();
                format!("file-{}", uuid.chars().take(8).collect::<String>())
            } else {
                last_path_component
            };
            let stamp = account_stamp.clone();
            let import = cx.update(|cx| {
                MediaPickerRecentFiles::shared(cx).update(cx, |files, cx| {
                    files.import_file_async(
                        url,
                        MediaPickerRecentKind::File,
                        suggested_name,
                        None,
                        None,
                        None,
                        None,
                        Some(stamp),
                        cx,
                    )
                })
            });
            match import.await {
                Ok(entry) => {
                    if cancelled.get() || !account_stamp.is_current() {
                        return;
                    }
                    let item = cx.update(|cx| {
                        MediaPickerRecentFiles::shared(cx).update(cx, |files, cx| files.item(&entry, cx))
                    });
                    if let Some(item) = item {
                        items.push(item);
                    }
                }
                Err(MediaPickerRecentFilesError::Cancelled) => {
                    return;
                }
                Err(error) => {
                    tracing::warn!(target: "riseonly::X1428", ?error, "document import failed");
                    let is_finished = this
                        .update(cx, |this, cx| {
                            if this.did_finish {
                                return true;
                            }
                            this.error_text = Some(Self::recent_files_error_text(&error));
                            cx.notify();
                            false
                        })
                        .unwrap_or(true);
                    if is_finished {
                        return;
                    }
                }
            }
        }
        let is_live = this.read_with(cx, |this, _| !this.did_finish).unwrap_or(false);
        tracing::warn!(target: "riseonly::X1428", items = items.len(), is_live, "document import copied");
        if !account_stamp.is_current() || !is_live || items.is_empty() {
            return;
        }
        Self::route_document_items(this, items, cancelled, cx).await;
    }

    async fn route_document_items(
        this: WeakEntity<Self>,
        items: Vec<MediaPickerItem>,
        cancelled: Rc<Cell<bool>>,
        cx: &mut AsyncApp,
    ) {
        let maximum_duration =
            this.read_with(cx, |this, _| this.configuration.max_video_duration_seconds).ok().flatten();
        let routes_to_editor = items.len() == 1
            && maximum_duration.is_some_and(|maximum_duration| maximum_duration > 0)
            && EXTERNAL_VIDEO_EXTENSIONS.contains(&items[0].url.path_extension().to_lowercase().as_str());
        let source_path = if routes_to_editor { items[0].url.to_file_path() } else { None };
        let (Some(source_path), Some(maximum_duration)) = (source_path, maximum_duration) else {
            Self::finish_with_items(&this, items, cx);
            return;
        };

        let probed = cx.background_executor().spawn(async move { video_probe(&source_path) }).await;
        if cancelled.get() {
            return;
        }
        match probed {
            Ok(probe) => {
                let seconds = probe.duration_seconds;
                if !seconds.is_finite() || seconds <= maximum_duration as f64 {
                    Self::finish_with_items(&this, items, cx);
                    return;
                }
                let (pixel_width, pixel_height) = if probe.has_video {
                    let transformed = probe.display_size();
                    (transformed.width.abs().max(1.0), transformed.height.abs().max(1.0))
                } else {
                    (1.0, 1.0)
                };
                let item = &items[0];
                let draft = MediaPickerExternalVideoDraft {
                    id: Uuid::new_v4().uuid_string(),
                    source_url: item.url.clone(),
                    suggested_file_name: item.suggested_file_name.clone(),
                    media_asset: MediaAsset::new(
                        format!("external-video:{}", item.id),
                        MediaAssetMediaType::Video,
                        pixel_width.round() as i64,
                        pixel_height.round() as i64,
                        seconds,
                        None,
                    ),
                };
                let _ = this.update(cx, |this, cx| {
                    if !this.account_stamp.is_current() || this.did_finish {
                        return;
                    }
                    this.pending_external_video_draft = Some(draft);
                    cx.notify();
                });
            }
            Err(error) => {
                let _ = this.update(cx, |this, cx| {
                    if !this.account_stamp.is_current() || this.did_finish {
                        return;
                    }
                    this.error_text = Some(error.to_string());
                    cx.notify();
                });
            }
        }
    }

    fn finish_with_items(this: &WeakEntity<Self>, items: Vec<MediaPickerItem>, cx: &mut AsyncApp) {
        let was_cancelled = items.is_empty();
        let _ = this.update(cx, |this, cx| this.finish(MediaPickerResult::new(items, was_cancelled), cx));
    }

    fn remove_generated_external_video(item: &MediaPickerItem) {
        let Some(temporary_root) = Url::from_file_path(&temporary_directory()).map(|url| url.standardized().path() + "/")
        else {
            return;
        };
        let poster_url = item.video_payload.as_ref().and_then(|payload| payload.poster_url.as_ref());
        for url in std::iter::once(&item.url).chain(poster_url) {
            let candidate = url.standardized();
            if !candidate.is_file_url() || !candidate.path().starts_with(&temporary_root) {
                continue;
            }
            if let Some(path) = candidate.to_file_path() {
                let _ = FileManager.remove_item(&path);
            }
        }
    }

    fn prepare_audio_draft(&mut self, source_url: PathBuf, cx: &mut Context<Self>) {
        if self.attachment_import_task.is_some() || !self.account_stamp.is_current() {
            return;
        }
        let account_stamp = self.account_stamp.clone();
        let cancelled = self.begin_attachment_import();
        self.attachment_import_task = Some(cx.spawn(async move |this, cx| {
            Self::prepare_audio_draft_body(this.clone(), source_url, account_stamp, cancelled, cx).await;
            let _ = this.update(cx, |this, _| this.release_attachment_import_task());
        }));
    }

    async fn prepare_audio_draft_body(
        this: WeakEntity<Self>,
        source_url: PathBuf,
        account_stamp: MediaPickerRecentAccountStamp,
        cancelled: Rc<Cell<bool>>,
        cx: &mut AsyncApp,
    ) {
        if !account_stamp.is_current() {
            return;
        }
        let original_file_name =
            source_url.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let mut generated_urls: Vec<PathBuf> = Vec::new();
        let prepared = cx.background_executor().spawn(async move { MusicTrackFileUpload::prepare_chat_audio(&source_url) }).await;
        let audio_path = match prepared {
            Ok(audio_path) => audio_path,
            Err(error) => {
                let _ = this.update(cx, |this, cx| {
                    if this.did_finish {
                        return;
                    }
                    this.error_text = Some(error.to_string());
                    cx.notify();
                });
                return;
            }
        };
        generated_urls.push(audio_path.clone());
        if cancelled.get() {
            cx.update(|cx| Self::remove_generated_files(generated_urls, cx)).await;
            return;
        }

        let metadata = MusicTrackFileUpload::imported_metadata(audio_path.clone(), Some(original_file_name.clone())).await;
        let mut cover_url: Option<Url> = None;
        if let Some(artwork) = metadata.artwork_data {
            cover_url = cx
                .background_executor().spawn(async move {
                    MusicTrackFileUpload::local_artwork_file(&artwork).map(|file| file.url)
                })
                .await;
            if let Some(cover_path) = cover_url.as_ref().and_then(Url::to_file_path) {
                generated_urls.push(cover_path);
            }
        }
        if cancelled.get() || !account_stamp.is_current() {
            cx.update(|cx| Self::remove_generated_files(generated_urls, cx)).await;
            return;
        }
        if metadata.duration_ms < MINIMUM_AUDIO_DURATION_MS {
            cx.update(|cx| Self::remove_generated_files(generated_urls, cx)).await;
            let _ = this.update(cx, |this, cx| {
                this.error_text = Some(tr("music_upload_invalid_duration"));
                cx.notify();
            });
            return;
        }
        let Some(audio_url) = Url::from_file_path(&audio_path) else {
            cx.update(|cx| Self::remove_generated_files(generated_urls, cx)).await;
            return;
        };
        let draft = MediaPickerAudioDraft {
            id: Uuid::new_v4().uuid_string(),
            audio_url,
            original_file_name,
            title: metadata.title,
            artist: metadata.artist.unwrap_or_else(|| tr("music_unknown_artist")),
            duration: metadata.duration_ms as f64 / 1000.0,
            cover_url,
        };
        let _ = this.update(cx, |this, cx| {
            this.staged_audio_urls = generated_urls;
            this.pending_audio_draft = Some(draft);
            cx.notify();
        });
    }

    /// editor can be dismissed while import_file is still copying, that mustnt delete a file the copy reads
    /// taking the list hands ownership to the import while it runs
    fn claim_staged_audio(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.staged_audio_urls)
    }

    /// an import that ended without committing gives the files back while the editor is still up
    /// user can press send again
    fn release_staged_audio(&mut self, urls: Vec<PathBuf>, cx: &mut Context<Self>) {
        if urls.is_empty() {
            return;
        }
        if self.pending_audio_draft.is_some() && !self.did_finish {
            self.staged_audio_urls = urls;
            return;
        }
        Self::remove_generated_files(urls, cx).detach();
    }

    fn discard_staged_audio(&mut self, cx: &mut Context<Self>) {
        let urls = self.claim_staged_audio();
        if urls.is_empty() {
            return;
        }
        Self::remove_generated_files(urls, cx).detach();
    }

    fn remove_generated_files(urls: Vec<PathBuf>, cx: &App) -> Task<()> {
        cx.background_executor().spawn(async move {
            for url in urls {
                let _ = FileManager.remove_item(&url);
            }
        })
    }

    fn has_untrimmed_long_video(&self, assets: &[LibraryAsset], cx: &App) -> bool {
        let Some(max_sec) = self.configuration.max_video_duration_seconds.filter(|max_sec| *max_sec > 0) else {
            return false;
        };
        let limit = max_sec as f64 + LONG_VIDEO_TOLERANCE_SECONDS;
        let selection = self.selection.read(cx);
        assets.iter().any(|asset| {
            if asset.kind != LibraryAssetKind::Video {
                return false;
            }
            match selection.edits_by_identifier().get(&asset.id).map(|edit| &edit.payload) {
                Some(MediaSelectionEditPayload::Video(state)) => state.end_seconds - state.start_seconds > limit,
                Some(MediaSelectionEditPayload::Image(_)) => false,
                None => asset.duration_secs > limit,
            }
        })
    }

    fn clamp_video_edit_state(&self, state: EditorVideoState) -> EditorVideoState {
        let Some(max_sec) = self.configuration.max_video_duration_seconds.filter(|max_sec| *max_sec > 0) else {
            return state;
        };
        let max_trim = max_sec as f64;
        let mut clamped = state;
        if clamped.end_seconds - clamped.start_seconds > max_trim {
            clamped.end_seconds = clamped.duration.min(clamped.start_seconds + max_trim);
        }
        if clamped.end_seconds - clamped.start_seconds > max_trim {
            clamped.start_seconds = (clamped.end_seconds - max_trim).max(0.0);
        }
        clamped
    }

    fn allowed_document_types(&self) -> Vec<UTType> {
        let Some(extensions) = self.configuration.allowed_file_extensions.as_ref().filter(|set| !set.is_empty()) else {
            return vec![UTType::item(), UTType::data()];
        };
        let mut sorted: Vec<&str> = extensions.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        let mut seen: HashSet<String> = HashSet::new();
        let mut has_undeclared_extension = false;
        let mut types: Vec<UTType> = Vec::new();
        for file_extension in sorted {
            match UTType::from_filename_extension(file_extension) {
                Some(content_type) => {
                    if seen.insert(content_type.identifier()) {
                        types.push(content_type);
                    }
                }
                None => has_undeclared_extension = true,
            }
        }
        if has_undeclared_extension && seen.insert(UTType::data().identifier()) {
            types.push(UTType::data());
        }
        if types.is_empty() { vec![UTType::data()] } else { types }
    }

    fn is_allowed_document(&self, url: &Path) -> bool {
        let Some(extensions) = &self.configuration.allowed_file_extensions else {
            return true;
        };
        let file_extension =
            url.extension().map(|file_extension| file_extension.to_string_lossy().to_lowercase()).unwrap_or_default();
        extensions.contains(&file_extension)
    }

    pub fn finish_with_document(
        &mut self,
        document: RiseDocument,
        items: Vec<MediaPickerItem>,
        entities: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.finish(MediaPickerResult::document(document, items, entities), cx);
    }

    pub fn finish_with_location(&mut self, location: MessageLocationInfo, cx: &mut Context<Self>) {
        self.finish(MediaPickerResult::location(location), cx);
    }

    pub fn finish_with_poll(&mut self, poll: MessagePollDraft, cx: &mut Context<Self>) {
        self.finish(MediaPickerResult::poll(poll), cx);
    }

    fn finish(&mut self, result: MediaPickerResult, cx: &mut Context<Self>) {
        if self.did_finish {
            return;
        }
        self.did_finish = true;
        self.discard_staged_audio(cx);
        let mut result = result;
        if result.attached_track.is_none() && !result.was_cancelled {
            result.attached_track = self.selection.read(cx).attached_track.clone();
        }
        cx.notify();
        let on_complete = self.on_complete.clone();
        cx.defer(move |cx| {
            let Some(presenter) = MediaPickerPresenterResolver::top_presenter(cx) else {
                tracing::warn!(target: "riseonly::X844", "picker finish without a presenter window");
                return;
            };
            let delivered = presenter.update(cx, |_, window, cx| on_complete(result, window, cx)).is_ok();
            tracing::warn!(target: "riseonly::X844", delivered, "picker finish delivered");
        });
    }
}
