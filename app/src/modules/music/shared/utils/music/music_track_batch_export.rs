use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use futures::channel::oneshot;
use futures::future::{Either, select};
use gpui::{App, AppContext, Context, Entity, Task};
use rise_engine::CancellationError;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::file_manager::{CocoaError, FileManager};
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::process_activity::begin_process_activity;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::media::save_to_folder_exporter::SaveToFolderExporter;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::media_picker::export::export_progress_view_state::ExportProgressViewState;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_track_file_export::{
    MusicTrackFileExportError, MusicTrackFileMaterializer, MusicTrackSavePolicy,
};

pub struct MusicTrackBatchExport {
    pub progress: Entity<ExportProgressViewState>,
    is_running: bool,
    task: Option<Task<()>>,
    cancel_signal: Option<oneshot::Sender<()>>,
}

impl MusicTrackBatchExport {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            progress: cx.new(ExportProgressViewState::new),
            is_running: false,
            task: None,
            cancel_signal: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.is_running
    }

    pub fn save(&mut self, tracks: &[TrackModel], cx: &mut Context<Self>) {
        let savable: Vec<TrackModel> =
            tracks.iter().filter(|track| MusicTrackSavePolicy::can_save(track)).cloned().collect();
        if savable.is_empty() || self.is_running {
            return;
        }
        let this = cx.weak_entity();
        SaveToFolderExporter::pick_folder(
            move |folder, cx| {
                let (Some(this), Some(folder)) = (this.upgrade(), folder) else {
                    return;
                };
                this.update(cx, |this, cx| this.run(savable, folder, cx));
            },
            cx,
        );
    }

    pub fn cancel(&mut self) {
        if let Some(signal) = self.cancel_signal.take() {
            let _ = signal.send(());
        }
    }

    pub fn reset_account_state(&mut self) {
        self.cancel();
    }

    fn run(&mut self, tracks: Vec<TrackModel>, folder: PathBuf, cx: &mut Context<Self>) {
        if self.is_running {
            return;
        }
        self.is_running = true;
        let total = tracks.len();
        let weak = cx.weak_entity();
        self.progress.update(cx, |progress, cx| {
            progress.reset(total as i64, cx);
            progress.on_cancel = Some(Rc::new(move |cx: &mut App| {
                if let Some(this) = weak.upgrade() {
                    this.update(cx, |this, _| this.cancel());
                }
            }));
        });
        let (cancel_signal, mut cancelled) = oneshot::channel::<()>();
        self.cancel_signal = Some(cancel_signal);
        cx.notify();
        let activity = begin_process_activity("music-batch-save");
        self.task = Some(cx.spawn(async move |this, cx| {
            let _activity = activity;
            let mut saved = 0usize;
            let mut taken: HashSet<String> = HashSet::new();
            for (index, track) in tracks.iter().enumerate() {
                if !matches!(cancelled.try_recv(), Ok(None)) {
                    break;
                }
                let published = this.update(cx, |this, cx| {
                    this.progress.update(cx, |progress, cx| {
                        progress.processed_items = index as i64;
                        progress.fraction_completed = index as f64 / total as f64;
                        progress.current_label = tr_args(
                            "music_save_progress",
                            &[FormatArg::Int(index as i64 + 1), FormatArg::Int(total as i64)],
                        );
                        cx.notify();
                    });
                });
                if published.is_err() {
                    return;
                }
                let prepared = match select(Box::pin(MusicTrackFileMaterializer::prepare(track, cx)), &mut cancelled).await
                {
                    Either::Left((Ok(prepared), _)) => prepared,
                    Either::Left((Err(error), _)) => {
                        if error.downcast_ref::<CancellationError>().is_some()
                            || matches!(
                                error.downcast_ref::<MusicTrackFileExportError>(),
                                Some(MusicTrackFileExportError::AccountChanged)
                            )
                        {
                            break;
                        }
                        let message = format!("batch save {} failed: {error}", track.id);
                        cx.update(|cx| Sai::log("Music", || message, SaiLogKind::Ui, cx));
                        continue;
                    }
                    Either::Right(_) => break,
                };
                let target = folder.clone();
                let mut batch_names = std::mem::take(&mut taken);
                let (names, copied) = cx
                    .background_executor()
                    .spawn(async move {
                        let copied = Self::copy_files(&prepared.files, &target, &mut batch_names);
                        prepared.cleanup();
                        (batch_names, copied)
                    })
                    .await;
                taken = names;
                match copied {
                    Ok(()) => saved += 1,
                    Err(error) => {
                        let message = format!("batch save {} failed: {error:?}", track.id);
                        cx.update(|cx| Sai::log("Music", || message, SaiLogKind::Ui, cx));
                    }
                }
            }
            let _ = this.update(cx, |this, cx| {
                if let Some(task) = this.task.take() {
                    task.detach();
                }
                this.cancel_signal = None;
                this.is_running = false;
                cx.notify();
                Self::report(saved, total, cx);
            });
        }));
    }

    fn copy_files(files: &[PathBuf], folder: &Path, taken: &mut HashSet<String>) -> Result<(), CocoaError> {
        for file in files {
            let last_path_component = file
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let name = Self::free_name(&last_path_component, folder, taken);
            FileManager.copy_item(file, &folder.join(name))?;
        }
        Ok(())
    }

    fn report(saved: usize, total: usize, cx: &mut App) {
        if saved == 0 {
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
            return;
        }
        let _ = notification(NotificationKind::Success);
        let title = if saved == total {
            tr("save_to_files_done")
        } else {
            tr_args("music_save_partial", &[FormatArg::Int(saved as i64), FormatArg::Int(total as i64)])
        };
        Notifier::send(
            title,
            NotifierSendOptions {
                r#type: NotifierType::Success,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("checkmark.circle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn free_name(name: &str, folder: &Path, taken: &mut HashSet<String>) -> String {
        let path = Path::new(name);
        let ext = path.extension().map(|ext| ext.to_string_lossy().into_owned()).unwrap_or_default();
        let base = if ext.is_empty() {
            name.to_owned()
        } else {
            path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
        };
        let mut candidate = name.to_owned();
        let mut suffix = 2;
        while taken.contains(&candidate.to_lowercase()) || FileManager.file_exists(&folder.join(&candidate)) {
            candidate = if ext.is_empty() { format!("{base} ({suffix})") } else { format!("{base} ({suffix}).{ext}") };
            suffix += 1;
        }
        taken.insert(candidate.to_lowercase());
        candidate
    }
}
