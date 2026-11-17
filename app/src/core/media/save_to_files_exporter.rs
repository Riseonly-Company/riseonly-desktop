use std::path::{Path, PathBuf};

use gpui::{App, Global, Task};
use rise_platform::download_mark::{self, DownloadOrigin};
use rise_platform::file_dialog::{self, OpenRequest, Selection};
use rise_platform::file_manager::FileManager;
use rise_ui::top_presenter::TopPresenter;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveToFilesExporterOutcome {
    Saved,
    Cancelled,
    Unavailable,
}

/// save to files for already materialized files
/// desktop asks the system folder panel for a folder and copies files in
/// staging files are removed after
pub struct SaveToFilesExporter {
    completion: Option<Box<dyn FnOnce(SaveToFilesExporterOutcome, &mut App)>>,
    finished: bool,
}

/// the export in flight, a newer one replaces it and the older never reports
#[derive(Default)]
struct SaveToFilesExporterActive {
    generation: u64,
    task: Option<Task<()>>,
}

impl Global for SaveToFilesExporterActive {}

impl SaveToFilesExporter {
    fn new(completion: Box<dyn FnOnce(SaveToFilesExporterOutcome, &mut App)>) -> Self {
        Self {
            completion: Some(completion),
            finished: false,
        }
    }

    /// on_presented runs once the panel is on screen, callers keep their own progress up until then
    pub fn present(
        files: Vec<PathBuf>,
        on_presented: Option<Box<dyn FnOnce(&mut App)>>,
        completion: impl FnOnce(SaveToFilesExporterOutcome, &mut App) + 'static,
        cx: &mut App,
    ) -> bool {
        if files.is_empty()
            || !files.iter().all(|file| FileManager.file_exists(file))
            || TopPresenter::current(cx).is_none()
        {
            completion(SaveToFilesExporterOutcome::Unavailable, cx);
            return false;
        }

        let mut exporter = SaveToFilesExporter::new(Box::new(completion));
        let selection = file_dialog::open(cx, &OpenRequest::directories());
        if let Some(on_presented) = on_presented {
            on_presented(cx);
        }
        let generation = cx.default_global::<SaveToFilesExporterActive>().generation.wrapping_add(1);
        let task = cx.spawn(async move |cx| {
            let outcome = match selection.await {
                Selection::Picked(picked) => match picked.accepted.into_iter().next() {
                    Some(directory) => {
                        cx.background_executor()
                            .spawn(async move { Self::copy_files(&files, &directory) })
                            .await
                    }
                    None => SaveToFilesExporterOutcome::Cancelled,
                },
                Selection::Cancelled => SaveToFilesExporterOutcome::Cancelled,
                Selection::Failed(_) => SaveToFilesExporterOutcome::Unavailable,
            };
            cx.update(|cx| {
                exporter.finish(outcome, cx);
                let active = cx.default_global::<SaveToFilesExporterActive>();
                if active.generation == generation {
                    if let Some(task) = active.task.take() {
                        task.detach();
                    }
                }
            });
        });
        cx.set_global(SaveToFilesExporterActive {
            generation,
            task: Some(task),
        });
        true
    }

    fn finish(&mut self, outcome: SaveToFilesExporterOutcome, cx: &mut App) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some(completion) = self.completion.take() {
            completion(outcome, cx);
        }
    }

    fn copy_files(files: &[PathBuf], directory: &Path) -> SaveToFilesExporterOutcome {
        for file in files {
            let Some(name) = file.file_name() else {
                return SaveToFilesExporterOutcome::Unavailable;
            };
            let destination = Self::available_destination(directory, Path::new(name));
            if FileManager.copy_item(file, &destination).is_err() {
                return SaveToFilesExporterOutcome::Unavailable;
            }
            if let Err(error) = download_mark::mark_downloaded(&destination, &DownloadOrigin::default()) {
                tracing::warn!(target: "riseonly", %error, "exported file stays without the download mark");
            }
        }
        SaveToFilesExporterOutcome::Saved
    }

    fn available_destination(directory: &Path, name: &Path) -> PathBuf {
        let candidate = directory.join(name);
        if !FileManager.file_exists(&candidate) {
            return candidate;
        }
        let stem = name.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        let extension = name.extension().map(|extension| extension.to_string_lossy().into_owned());
        let mut index: i64 = 2;
        loop {
            let numbered = match &extension {
                Some(extension) => format!("{stem} {index}.{extension}"),
                None => format!("{stem} {index}"),
            };
            let candidate = directory.join(numbered);
            if !FileManager.file_exists(&candidate) {
                return candidate;
            }
            index += 1;
        }
    }
}
