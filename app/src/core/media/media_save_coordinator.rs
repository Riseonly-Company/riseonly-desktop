use std::path::PathBuf;

use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_platform::file_manager::FileManager;
use rise_platform::haptics::{NotificationKind, notification};
use rise_widgets::overlay::blocking_loader_store::BlockingLoaderStore;

use crate::core::media::save_to_files_exporter::{SaveToFilesExporter, SaveToFilesExporterOutcome};
use crate::core::media_viewer::media_save_to_library::MediaSaveToLibrary;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::core::widgets::sheets::message_external_share_materializer::MessageExternalShareMaterializer;
pub use crate::modules::chat::shared::media_save_target_hooks::{MediaSaveTargetKind, MessageMediaSaveClassifier};
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageMediaItem};
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_track_file_export::{
    MusicTrackFileMaterializer, MusicTrackSavePolicy,
};

#[derive(Clone, Debug)]
struct PreparedSave {
    target: MediaSaveTargetKind,
    files: Vec<PathBuf>,
    temporary_directory: Option<PathBuf>,
    is_video: Vec<bool>,
}

impl PreparedSave {
    /// blocking file io, run it off the main thread
    fn cleanup(&self) {
        let Some(temporary_directory) = self.temporary_directory.as_ref() else {
            return;
        };
        let _ = FileManager.remove_item(temporary_directory);
    }

    fn cleanup_in_background(self) {
        RiseAccountRuntime::current().spawn_blocking(move || self.cleanup());
    }
}

/// single owner of every save to device action
/// one save at a time, a new request cancels the old and late completions recheck their generation
pub struct MediaSaveCoordinator {
    generation: Option<Uuid>,
    task: Option<Task<()>>,
}

impl MediaSaveCoordinator {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            generation: None,
            task: None,
        }
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.generation = None;
        BlockingLoaderStore::shared(cx).update(cx, |loader, cx| loader.hide(cx));
    }

    pub fn save_to_gallery(&mut self, message: GetMessageMessage, items: Vec<MessageMediaItem>, cx: &mut Context<Self>) {
        if items.is_empty() || message.is_system() || message.protects_content_locally() {
            return;
        }

        self.run(
            async move |cx: &AsyncApp| -> anyhow::Result<PreparedSave> {
                let prepared = MessageExternalShareMaterializer::prepare_files(&message, &items, cx).await?;
                tracing::warn!(target: "riseonly::X655", "save media to folder files={}", prepared.files.len());
                Ok(PreparedSave {
                    target: MediaSaveTargetKind::Files,
                    files: prepared.files,
                    temporary_directory: prepared.temporary_directory,
                    is_video: items.iter().map(MessageMediaSaveClassifier::is_video).collect(),
                })
            },
            cx,
        );
    }

    pub fn save_to_files(&mut self, message: GetMessageMessage, items: Vec<MessageMediaItem>, cx: &mut Context<Self>) {
        if items.is_empty() || message.is_system() || message.protects_content_locally() {
            return;
        }

        self.run(
            async move |cx: &AsyncApp| -> anyhow::Result<PreparedSave> {
                let prepared = MessageExternalShareMaterializer::prepare_files(&message, &items, cx).await?;
                Ok(PreparedSave {
                    target: MediaSaveTargetKind::Files,
                    files: prepared.files,
                    temporary_directory: prepared.temporary_directory,
                    is_video: Vec::new(),
                })
            },
            cx,
        );
    }

    pub fn save_track_to_files(&mut self, track: TrackModel, cx: &mut Context<Self>) {
        if !MusicTrackSavePolicy::can_save(&track) {
            return;
        }

        self.run(
            async move |cx: &AsyncApp| -> anyhow::Result<PreparedSave> {
                let prepared = MusicTrackFileMaterializer::prepare(&track, cx).await?;
                Ok(PreparedSave {
                    target: MediaSaveTargetKind::Files,
                    files: prepared.files,
                    temporary_directory: prepared.temporary_directory,
                    is_video: Vec::new(),
                })
            },
            cx,
        );
    }

    fn run(
        &mut self,
        prepare: impl AsyncFnOnce(&AsyncApp) -> anyhow::Result<PreparedSave> + 'static,
        cx: &mut Context<Self>,
    ) {
        self.task = None;
        let current_generation = Uuid::new_v4();
        self.generation = Some(current_generation);
        let weak = cx.weak_entity();
        BlockingLoaderStore::shared(cx).update(cx, |loader, cx| {
            loader.show(
                move |cx| {
                    let _ = weak.update(cx, |this, _| this.cancel(current_generation));
                },
                cx,
            );
        });

        self.task = Some(cx.spawn(async move |this, cx| {
            let prepared = match prepare(&*cx).await {
                Ok(prepared) => prepared,
                Err(error) => {
                    let is_cancellation = error.downcast_ref::<CancellationError>().is_some();
                    let _ = this.update(cx, |this, cx| this.fail(current_generation, is_cancellation, cx));
                    return;
                }
            };
            let accepted = this.update(cx, |this, _| {
                if this.generation != Some(current_generation) {
                    return false;
                }
                this.release_task();
                true
            });
            if !matches!(accepted, Ok(true)) {
                prepared.cleanup_in_background();
                return;
            }
            if prepared.target == MediaSaveTargetKind::Files {
                let release = this.clone();
                cx.update(|cx| Self::present_files_export(release, prepared, Some(current_generation), cx));
                return;
            }
            cx.update(|cx| BlockingLoaderStore::shared(cx).update(cx, |loader, cx| loader.hide(cx)));
            Self::deliver(&this, prepared, cx).await;
            let _ = this.update(cx, |this, _| {
                if this.generation == Some(current_generation) {
                    this.generation = None;
                }
            });
        }));
    }

    async fn deliver(this: &WeakEntity<Self>, prepared: PreparedSave, cx: &mut AsyncApp) {
        match prepared.target {
            MediaSaveTargetKind::Gallery => {
                let files: Vec<(PathBuf, bool)> = prepared
                    .files
                    .iter()
                    .enumerate()
                    .map(|(index, url)| (url.clone(), prepared.is_video.get(index).copied().unwrap_or(false)))
                    .collect();
                let result = MediaSaveToLibrary::save_files(files, cx).await;
                prepared.cleanup_in_background();
                cx.update(|cx| MediaSaveToLibrary::announce(result, NotifierPosition::Top, 0.0, cx));
            }
            MediaSaveTargetKind::Files => {
                let Ok(generation) = this.read_with(&*cx, |this, _| this.generation) else {
                    prepared.cleanup_in_background();
                    return;
                };
                let release = this.clone();
                cx.update(|cx| Self::present_files_export(release, prepared, generation, cx));
            }
        }
    }

    /// loader stays until the save panel is on screen so the user doesnt click twice
    /// clicking the loader still releases it
    fn present_files_export(this: WeakEntity<Self>, prepared: PreparedSave, current: Option<Uuid>, cx: &mut App) {
        let on_presented_owner = this.clone();
        let completion_save = prepared.clone();
        let presented = SaveToFilesExporter::present(
            prepared.files.clone(),
            Some(Box::new(move |cx: &mut App| Self::release(&on_presented_owner, current, cx))),
            move |outcome, cx| {
                completion_save.cleanup_in_background();
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
                    SaveToFilesExporterOutcome::Cancelled => {}
                    SaveToFilesExporterOutcome::Unavailable => {
                        notification(NotificationKind::Error);
                        Notifier::send(
                            tr("save_failed"),
                            NotifierSendOptions {
                                r#type: NotifierType::Error,
                                position: NotifierPosition::Top,
                                icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                                ..NotifierSendOptions::default()
                            },
                            cx,
                        );
                    }
                }
            },
            cx,
        );
        if !presented {
            Self::release(&this, current, cx);
        }
    }

    fn release(this: &WeakEntity<Self>, current: Option<Uuid>, cx: &mut App) {
        let _ = this.update(cx, |this, cx| {
            if this.generation != current {
                return;
            }
            this.generation = None;
            BlockingLoaderStore::shared(cx).update(cx, |loader, cx| loader.hide(cx));
        });
    }

    fn fail(&mut self, current_generation: Uuid, is_cancellation: bool, cx: &mut Context<Self>) {
        if self.generation != Some(current_generation) {
            return;
        }
        self.release_task();
        self.generation = None;
        BlockingLoaderStore::shared(cx).update(cx, |loader, cx| loader.hide(cx));
        if is_cancellation {
            return;
        }
        Self::report_failure(cx);
    }

    fn report_failure(cx: &mut App) {
        notification(NotificationKind::Error);
        Notifier::send(
            tr("save_failed"),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn release_task(&mut self) {
        if let Some(task) = self.task.take() {
            task.detach();
        }
    }

    fn cancel(&mut self, generation: Uuid) {
        if self.generation != Some(generation) {
            return;
        }
        self.task = None;
        self.generation = None;
    }
}
