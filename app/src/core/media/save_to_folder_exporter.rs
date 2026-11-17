use std::path::PathBuf;

use gpui::{App, Global, Task};
use rise_platform::file_dialog::{self, OpenRequest, Selection};
use rise_ui::top_presenter::TopPresenter;

/// asks the user for a folder and hands back its path
/// a batch then writes files one by one so only one staged copy sits on disk
pub struct SaveToFolderExporter {
    completion: Option<Box<dyn FnOnce(Option<PathBuf>, &mut App)>>,
    finished: bool,
}

/// the pick in flight, a newer one replaces it and the older never reports
#[derive(Default)]
struct SaveToFolderExporterActive {
    generation: u64,
    task: Option<Task<()>>,
}

impl Global for SaveToFolderExporterActive {}

impl SaveToFolderExporter {
    fn new(completion: Box<dyn FnOnce(Option<PathBuf>, &mut App)>) -> Self {
        Self {
            completion: Some(completion),
            finished: false,
        }
    }

    /// completion gets none when the user cancels or no presenter exists
    pub fn pick_folder(completion: impl FnOnce(Option<PathBuf>, &mut App) + 'static, cx: &mut App) {
        if TopPresenter::current(cx).is_none() {
            completion(None, cx);
            return;
        }
        let mut exporter = SaveToFolderExporter::new(Box::new(completion));
        let selection = file_dialog::open(cx, &OpenRequest::directories());
        let generation = cx.default_global::<SaveToFolderExporterActive>().generation.wrapping_add(1);
        let task = cx.spawn(async move |cx| {
            let folder = match selection.await {
                Selection::Picked(picked) => picked.accepted.into_iter().next(),
                Selection::Cancelled | Selection::Failed(_) => None,
            };
            cx.update(|cx| {
                exporter.finish(folder, cx);
                let active = cx.default_global::<SaveToFolderExporterActive>();
                if active.generation == generation {
                    if let Some(task) = active.task.take() {
                        task.detach();
                    }
                }
            });
        });
        cx.set_global(SaveToFolderExporterActive {
            generation,
            task: Some(task),
        });
    }

    fn finish(&mut self, folder: Option<PathBuf>, cx: &mut App) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some(completion) = self.completion.take() {
            completion(folder, cx);
        }
    }
}
