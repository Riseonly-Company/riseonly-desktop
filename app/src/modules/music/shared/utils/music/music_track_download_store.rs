use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_core::foundation::foundation_text::{character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_platform::file_manager::FileManager;
use rise_platform::haptics::{NotificationKind, notification};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::media::save_to_files_exporter::{SaveToFilesExporter, SaveToFilesExporterOutcome};
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::music::shared::utils::music::music_audio_effect_settings::{
    MusicAudioEffectKind, MusicAudioEffectKindExt, MusicAudioEffectSettings,
};
use crate::modules::music::shared::utils::music::music_effect_offline_renderer::MusicEffectOfflineRenderer;
use crate::modules::music::shared::utils::music::music_media_url::MusicMediaURL;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::shared::utils::music::music_track_file_export::{
    MusicTrackFileMaterializer, MusicTrackSavePolicy, PreparedTrackFile,
};
use crate::modules::music::shared::utils::music::music_track_id3_writer::MusicTrackID3Writer;
use crate::modules::music::shared::utils::music::music_track_metadata_writer::MusicTrackMetadataWriter;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicTrackDownloadPhase {
    Options,
    Metadata,
}

/// holds the files of a download in flight
/// a dropped future never reaches cleanup
/// so it lives here unless files were handed on
struct PreparedTrackFileGuard {
    prepared: Option<PreparedTrackFile>,
}

impl PreparedTrackFileGuard {
    fn new(prepared: PreparedTrackFile) -> Self {
        Self {
            prepared: Some(prepared),
        }
    }

    fn files(&self) -> &[PathBuf] {
        self.prepared
            .as_ref()
            .map(|prepared| prepared.files.as_slice())
            .unwrap_or(&[])
    }

    fn temporary_directory(&self) -> Option<&Path> {
        self.prepared
            .as_ref()
            .and_then(|prepared| prepared.temporary_directory.as_deref())
    }

    fn replace(&mut self, prepared: PreparedTrackFile) {
        self.prepared = Some(prepared);
    }

    fn take(mut self) -> Option<PreparedTrackFile> {
        self.prepared.take()
    }
}

impl Drop for PreparedTrackFileGuard {
    fn drop(&mut self) {
        if let Some(prepared) = self.prepared.take() {
            MusicTrackDownloadStore::cleanup_in_background(prepared);
        }
    }
}

/// owns the track download flow
/// effect options, details and save dialog
/// nothing is processed until the user confirms
pub struct MusicTrackDownloadStore {
    sheet_track: Option<TrackModel>,
    phase: MusicTrackDownloadPhase,
    working_track_id: Option<String>,
    progress: f64,

    is_sheet_presented: bool,
    draft_settings: MusicAudioEffectSettings,
    draft_title: String,
    draft_artist: String,
    draft_cover: Option<CleverImage>,
    draft_cover_file: Option<SaiWsOptimisticLocalFile>,

    task: Option<Task<()>>,
    generation: Uuid,
    pending_delivery: Option<PreparedTrackFile>,
    delivery_fallback_task: Option<Task<()>>,
}

impl MusicTrackDownloadStore {
    const DELIVERY_FALLBACK_MILLISECONDS: u64 = 1_200;
    const PRESENT_RETRY_MILLISECONDS: u64 = 250;
    const PRESENT_RETRY_LIMIT: i64 = 4;
    const EXPORT_STEM_CHARACTER_LIMIT: usize = 96;
    const EXPORT_FALLBACK_STEM: &str = "Riseonly";

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            sheet_track: None,
            phase: MusicTrackDownloadPhase::Options,
            working_track_id: None,
            progress: 0.0,
            is_sheet_presented: false,
            draft_settings: MusicAudioEffectSettings::off(),
            draft_title: String::new(),
            draft_artist: String::new(),
            draft_cover: None,
            draft_cover_file: None,
            task: None,
            generation: Uuid::new_v4(),
            pending_delivery: None,
            delivery_fallback_task: None,
        }
    }

    pub fn sheet_track(&self) -> Option<&TrackModel> {
        self.sheet_track.as_ref()
    }

    pub fn phase(&self) -> MusicTrackDownloadPhase {
        self.phase
    }

    pub fn working_track_id(&self) -> Option<&str> {
        self.working_track_id.as_deref()
    }

    pub fn progress(&self) -> f64 {
        self.progress
    }

    pub fn is_sheet_presented(&self) -> bool {
        self.is_sheet_presented
    }

    pub fn set_is_sheet_presented(&mut self, is_sheet_presented: bool, cx: &mut Context<Self>) {
        self.is_sheet_presented = is_sheet_presented;
        cx.notify();
    }

    pub fn draft_settings(&self) -> &MusicAudioEffectSettings {
        &self.draft_settings
    }

    pub fn set_draft_settings(&mut self, draft_settings: MusicAudioEffectSettings, cx: &mut Context<Self>) {
        self.draft_settings = draft_settings;
        cx.notify();
    }

    pub fn draft_title(&self) -> &str {
        &self.draft_title
    }

    pub fn set_draft_title(&mut self, draft_title: String, cx: &mut Context<Self>) {
        self.draft_title = draft_title;
        cx.notify();
    }

    pub fn draft_artist(&self) -> &str {
        &self.draft_artist
    }

    pub fn set_draft_artist(&mut self, draft_artist: String, cx: &mut Context<Self>) {
        self.draft_artist = draft_artist;
        cx.notify();
    }

    pub fn draft_cover(&self) -> Option<&CleverImage> {
        self.draft_cover.as_ref()
    }

    pub fn set_draft_cover(&mut self, draft_cover: Option<CleverImage>, cx: &mut Context<Self>) {
        self.draft_cover = draft_cover;
        cx.notify();
    }

    pub fn draft_cover_file(&self) -> Option<&SaiWsOptimisticLocalFile> {
        self.draft_cover_file.as_ref()
    }

    pub fn set_draft_cover_file(
        &mut self,
        draft_cover_file: Option<SaiWsOptimisticLocalFile>,
        cx: &mut Context<Self>,
    ) {
        self.draft_cover_file = draft_cover_file;
        cx.notify();
    }

    pub fn is_working(&self) -> bool {
        self.working_track_id.is_some()
    }

    /// only one export at a time, the encoder eats cpu and a queue would fight playback
    pub fn can_start_download(&self, track_id: &str) -> bool {
        self.working_track_id
            .as_deref()
            .is_none_or(|working| working == track_id)
    }

    pub fn present_options(&mut self, track: TrackModel, cx: &mut Context<Self>) {
        if !MusicTrackSavePolicy::can_save(&track) {
            return;
        }
        if !self.can_start_download(&track.id) {
            return;
        }

        self.phase = MusicTrackDownloadPhase::Options;
        self.progress = 0.0;
        self.draft_settings = MusicPlayerState::shared(cx)
            .read(cx)
            .audio_effect_settings()
            .clone();
        self.draft_title = track.title.clone();
        self.draft_artist = track.artist.clone();
        self.draft_cover = None;
        self.draft_cover_file = None;
        self.sheet_track = Some(track);
        self.is_sheet_presented = true;
        cx.notify();
    }

    pub fn dismiss_sheet(&mut self, cx: &mut Context<Self>) {
        self.is_sheet_presented = false;
        cx.notify();
    }

    pub fn continue_to_metadata(&mut self, cx: &mut Context<Self>) {
        if self.phase != MusicTrackDownloadPhase::Options || self.is_working() {
            return;
        }
        self.phase = MusicTrackDownloadPhase::Metadata;
        cx.notify();
    }

    pub fn back_to_options(&mut self, cx: &mut Context<Self>) {
        if self.phase != MusicTrackDownloadPhase::Metadata || self.is_working() {
            return;
        }
        self.phase = MusicTrackDownloadPhase::Options;
        cx.notify();
    }

    pub fn sheet_did_dismiss(&mut self, cx: &mut Context<Self>) {
        self.delivery_fallback_task = None;
        self.flush_delivery(cx);
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        let Some(track) = self.sheet_track.clone() else {
            return;
        };
        if self.working_track_id.is_some() {
            return;
        }

        let settings = self.draft_settings.clone();
        let title = trimming_whitespaces_and_newlines(&self.draft_title).to_owned();
        let artist = trimming_whitespaces_and_newlines(&self.draft_artist).to_owned();
        let picked_cover = self.draft_cover.clone();

        let current_generation = Uuid::new_v4();
        self.generation = current_generation;
        self.working_track_id = Some(track.id.clone());
        self.progress = 0.0;
        cx.notify();

        self.task = Some(cx.spawn(async move |this, cx| {
            let outcome = Self::produce(&this, &track, settings, &title, &artist, picked_cover, current_generation, cx)
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != current_generation {
                    return;
                }
                this.release_task();
                this.working_track_id = None;
                this.progress = 0.0;
                cx.notify();
                match outcome {
                    Ok(Some(prepared)) => {
                        this.pending_delivery = prepared.take();
                        this.request_dismiss_and_deliver(cx);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        if error.downcast_ref::<CancellationError>().is_some() {
                            return;
                        }
                        Self::report_failure(cx);
                    }
                }
            });
        }));
    }

    #[allow(clippy::too_many_arguments)]
    async fn produce(
        this: &WeakEntity<Self>,
        track: &TrackModel,
        settings: MusicAudioEffectSettings,
        title: &str,
        artist: &str,
        picked_cover: Option<CleverImage>,
        current_generation: Uuid,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Option<PreparedTrackFileGuard>> {
        let mut prepared = PreparedTrackFileGuard::new(MusicTrackFileMaterializer::prepare(track, cx).await?);
        if !Self::is_current(this, current_generation, cx) {
            return Ok(None);
        }
        if settings.is_any_enabled() {
            Self::apply_effect(this, &mut prepared, track, &settings, current_generation, cx).await?;
        }
        let cover = match picked_cover {
            Some(cover) => Some(cover),
            None => Self::track_artwork(&track.cover_image_url, cx).await,
        };
        if !Self::is_current(this, current_generation, cx) {
            return Ok(None);
        }
        Self::apply_metadata(
            this,
            &mut prepared,
            track,
            title,
            artist,
            cover,
            &settings,
            current_generation,
            cx,
        )
        .await?;
        if !Self::is_current(this, current_generation, cx) {
            return Ok(None);
        }
        Ok(Some(prepared))
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.delivery_fallback_task = None;
        self.generation = Uuid::new_v4();
        self.working_track_id = None;
        self.progress = 0.0;
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.cancel(cx);
        if let Some(prepared) = self.pending_delivery.take() {
            Self::cleanup_in_background(prepared);
        }
        self.sheet_track = None;
        self.is_sheet_presented = false;
        self.phase = MusicTrackDownloadPhase::Options;
        cx.notify();
    }

    fn request_dismiss_and_deliver(&mut self, cx: &mut Context<Self>) {
        self.is_sheet_presented = false;
        cx.notify();
        self.delivery_fallback_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(Self::DELIVERY_FALLBACK_MILLISECONDS))
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(task) = this.delivery_fallback_task.take() {
                    task.detach();
                }
                this.flush_delivery(cx);
            });
        }));
    }

    fn flush_delivery(&mut self, cx: &mut Context<Self>) {
        let Some(prepared) = self.pending_delivery.take() else {
            return;
        };
        cx.defer(move |cx| Self::present_exporter(prepared, 0, cx));
    }

    /// present returns false when no usable presenter exists, eg another window is closing
    /// then files stay for the next attempt and the retry loop owns cleanup and the error toast
    fn present_exporter(prepared: PreparedTrackFile, attempt: i64, cx: &mut App) {
        let completion_files = prepared.clone();
        let presented = SaveToFilesExporter::present(
            prepared.files.clone(),
            None,
            move |outcome, cx| {
                if outcome == SaveToFilesExporterOutcome::Unavailable {
                    return;
                }
                Self::cleanup_in_background(completion_files);
                match outcome {
                    SaveToFilesExporterOutcome::Saved => {
                        notification(NotificationKind::Success);
                        Notifier::send(
                            tr("save_to_files_done"),
                            NotifierSendOptions {
                                r#type: NotifierType::Success,
                                position: NotifierPosition::Top,
                                icon: NotifierIcon::System("checkmark.circle.fill".to_owned()),
                                ..NotifierSendOptions::default()
                            },
                            cx,
                        );
                    }
                    SaveToFilesExporterOutcome::Cancelled | SaveToFilesExporterOutcome::Unavailable => {}
                }
            },
            cx,
        );
        if presented {
            return;
        }
        if attempt >= Self::PRESENT_RETRY_LIMIT {
            Self::cleanup_in_background(prepared);
            Self::report_failure(cx);
            return;
        }
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(Self::PRESENT_RETRY_MILLISECONDS))
                .await;
            cx.update(|cx| Self::present_exporter(prepared, attempt + 1, cx));
        })
        .detach();
    }

    fn report_failure(cx: &mut App) {
        Notifier::send(
            tr("music_download_failed"),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    async fn apply_effect(
        this: &WeakEntity<Self>,
        prepared: &mut PreparedTrackFileGuard,
        track: &TrackModel,
        settings: &MusicAudioEffectSettings,
        current_generation: Uuid,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<()> {
        let (Some(source), Some(directory)) = (
            prepared.files().first().cloned(),
            prepared.temporary_directory().map(Path::to_path_buf),
        ) else {
            return Ok(());
        };

        let destination = directory.join(Self::processed_file_name(track, &source, settings));
        let (sender, mut receiver) = mpsc::unbounded::<f64>();
        let progress_owner = this.clone();
        cx.spawn(async move |cx| {
            while let Some(value) = receiver.next().await {
                let _ = progress_owner.update(cx, |this, cx| {
                    if this.generation != current_generation {
                        return;
                    }
                    if value < this.progress + 0.01 && value < 1.0 {
                        return;
                    }
                    this.progress = value;
                    cx.notify();
                });
            }
        })
        .detach();

        let rendered = MusicEffectOfflineRenderer::render(
            source.clone(),
            destination.clone(),
            settings.clone(),
            move |value| {
                let _ = sender.unbounded_send(value);
            },
        )
        .await;
        if let Err(error) = rendered {
            Self::remove_in_background(vec![destination]);
            return Err(anyhow::anyhow!("music effect render failed: {error:?}"));
        }

        Self::remove_in_background(vec![source]);
        prepared.replace(PreparedTrackFile {
            files: vec![destination],
            temporary_directory: Some(directory),
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_metadata(
        this: &WeakEntity<Self>,
        prepared: &mut PreparedTrackFileGuard,
        track: &TrackModel,
        title: &str,
        artist: &str,
        cover: Option<CleverImage>,
        settings: &MusicAudioEffectSettings,
        current_generation: Uuid,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<()> {
        let (Some(directory), Some(source)) = (
            prepared.temporary_directory().map(Path::to_path_buf),
            prepared.files().first().cloned(),
        ) else {
            return Ok(());
        };

        let file_extension = source
            .extension()
            .map(|extension| extension.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let resolved_title = if title.is_empty() {
            track.title.clone()
        } else {
            title.to_owned()
        };
        let resolved_artist = if artist.is_empty() {
            track.artist.clone()
        } else {
            artist.to_owned()
        };
        let staged = directory.join(format!("{}.{file_extension}", Self::uuid_string()));

        let written: anyhow::Result<()> = match file_extension.as_str() {
            "mp3" => {
                let source = source.clone();
                let staged = staged.clone();
                let title = resolved_title.clone();
                let artist = resolved_artist.clone();
                RiseAccountRuntime::current()
                    .spawn_blocking(move || {
                        let artwork = cover
                            .as_ref()
                            .and_then(MusicTrackMetadataWriter::artwork_jpeg_data);
                        MusicTrackID3Writer::write(&source, &staged, Some(&title), Some(&artist), artwork.as_deref())
                    })
                    .await
                    .map_err(|_| anyhow::Error::new(CancellationError))
                    .and_then(|written| written.map_err(anyhow::Error::new))
            }
            MusicEffectOfflineRenderer::OUTPUT_FILE_EXTENSION | "mp4" | "m4b" => {
                MusicTrackMetadataWriter::write(
                    &source,
                    &staged,
                    Some(&resolved_title),
                    Some(&resolved_artist),
                    cover.as_ref(),
                )
                .await
            }
            _ => return Ok(()),
        };
        if let Err(error) = written {
            Self::remove_in_background(vec![staged]);
            if error.downcast_ref::<CancellationError>().is_some() {
                return Err(error);
            }
            return Ok(());
        }

        if !Self::is_current(this, current_generation, cx) {
            Self::remove_in_background(vec![staged]);
            return Ok(());
        }

        let destination = directory.join(Self::export_file_name(
            &resolved_title,
            &resolved_artist,
            settings,
            &file_extension,
        ));
        let moved_destination = destination.clone();
        let moved_staged = staged.clone();
        let moved = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let _ = FileManager.remove_item(&source);
                let _ = FileManager.remove_item(&moved_destination);
                FileManager.move_item(&moved_staged, &moved_destination).is_ok()
            })
            .await
            .unwrap_or(false);
        let file = if moved { destination } else { staged };
        prepared.replace(PreparedTrackFile {
            files: vec![file],
            temporary_directory: Some(directory),
        });
        Ok(())
    }

    async fn track_artwork(cover_image_url: &str, cx: &mut AsyncApp) -> Option<CleverImage> {
        for source in MusicMediaURL::cover_url_strings_for_fetch(Some(cover_image_url)) {
            if let Some(image) = CleverImageBitmapLoader::load(
                &source,
                true,
                true,
                SaiAccountCacheScope::active_user_id(),
                SaiAccountCacheScope::active_epoch(),
                None,
                CleverImageCachePolicy::Standard,
                CleverImageWorkLane::Visible,
                cx,
            )
            .await
            {
                return Some(image);
            }
        }
        None
    }

    fn processed_file_name(track: &TrackModel, source: &Path, settings: &MusicAudioEffectSettings) -> String {
        let original = MusicTrackFileMaterializer::export_file_name(track, source);
        let stem = Self::deleting_path_extension(&original);
        format!(
            "{stem}{}.{}",
            Self::effect_suffix(settings),
            MusicEffectOfflineRenderer::OUTPUT_FILE_EXTENSION
        )
    }

    fn export_file_name(
        title: &str,
        artist: &str,
        settings: &MusicAudioEffectSettings,
        file_extension: &str,
    ) -> String {
        let stem = if artist.is_empty() {
            title.to_owned()
        } else {
            format!("{artist} - {title}")
        };
        let replaced = stem.replace('/', "_").replace(':', "_");
        let sanitized = trimming_whitespaces_and_newlines(&replaced);
        let resolved = if sanitized.is_empty() {
            Self::EXPORT_FALLBACK_STEM.to_owned()
        } else {
            character_prefix(sanitized, Self::EXPORT_STEM_CHARACTER_LIMIT).to_owned()
        };
        format!("{resolved}{}.{file_extension}", Self::effect_suffix(settings))
    }

    /// effects stay in the file name even after the metadata step renames it
    fn effect_suffix(settings: &MusicAudioEffectSettings) -> String {
        let tags: Vec<String> = settings
            .enabled_kinds()
            .into_iter()
            .map(|kind| match kind {
                MusicAudioEffectKind::Slowed | MusicAudioEffectKind::Nightcore => {
                    format!("{} {:.2}x", kind.raw_value(), settings.playback_rate())
                }
                _ => kind.raw_value().to_owned(),
            })
            .collect();
        if tags.is_empty() {
            String::new()
        } else {
            format!(" ({})", tags.join(", "))
        }
    }

    fn deleting_path_extension(name: &str) -> &str {
        match name.rfind('.') {
            Some(index) if index > 0 => &name[..index],
            _ => name,
        }
    }

    fn uuid_string() -> String {
        Uuid::new_v4().uuid_string()
    }

    fn is_current(this: &WeakEntity<Self>, current_generation: Uuid, cx: &AsyncApp) -> bool {
        this.read_with(cx, |this, _| this.generation == current_generation)
            .unwrap_or(false)
    }

    fn release_task(&mut self) {
        if let Some(task) = self.task.take() {
            task.detach();
        }
    }

    fn remove_in_background(paths: Vec<PathBuf>) {
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            for path in &paths {
                let _ = FileManager.remove_item(path);
            }
        }));
    }

    fn cleanup_in_background(prepared: PreparedTrackFile) {
        drop(RiseAccountRuntime::current().spawn_blocking(move || prepared.cleanup()));
    }
}
