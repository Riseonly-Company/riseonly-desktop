use std::rc::Rc;

use gpui::{App, Context, Entity, Task};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::media::remote_media_disk_cache::RemoteMediaDiskCache;
use crate::core::media::rise_media_transfer_hub::MediaViewerVideoCacheKey;
use crate::core::media_viewer::media_save_to_library::{MediaSaveToLibrary, MediaSaveToLibraryResult};
use crate::core::widgets::notifier::notifier_progress::NotifierProgressStore;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::post::components::download::post_download_coordinator::PostDownloadCoordinator;
use crate::modules::story::stores::story::story_actions::story_wire_types::{StoryDTO, StoryDTOExt};

const LOG_NAME: &str = "StoryDownload";

/// author saves own story to the gallery from the story menu, no watermark
/// photo as photo, rest as video
pub struct StoryDownloadStore {
    job: Option<Task<()>>,
    toast_id: Option<Uuid>,
    fraction: f64,
}

impl StoryDownloadStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { job: None, toast_id: None, fraction: 0.0 })
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn start(&mut self, story: StoryDTO, notifier_offset: f32, cx: &mut Context<Self>) {
        if self.job.is_some() {
            return;
        }
        let is_video = story.media_type == "video";
        let source = story.media_source();
        let Some((normalized, remote)) = RemoteMediaURL::optional_normalized(Some(&source))
            .and_then(|normalized| Url::from_string(&normalized).map(|remote| (normalized, remote)))
        else {
            MediaSaveToLibrary::announce(MediaSaveToLibraryResult::Failed, NotifierPosition::Bottom, notifier_offset, cx);
            return;
        };
        self.fraction = 0.0;
        self.show_progress(notifier_offset, cx);
        self.job = Some(cx.spawn(async move |this, cx| {
            let cache_key = if is_video { MediaViewerVideoCacheKey::resolve(&normalized) } else { None };
            let filename = remote.last_path_component();
            let reporting = this.clone();
            let progress_cx = cx.clone();
            let on_fraction = move |fraction: f64| {
                progress_cx.update(|cx| {
                    let _ = reporting.update(cx, |store, cx| store.update_fraction(fraction * 0.95, notifier_offset, cx));
                });
            };
            let fetched = PostDownloadCoordinator::fetch_source(&remote, cache_key, &filename, &on_fraction, cx).await;
            let result = match fetched {
                Ok(file) => {
                    let lease = RemoteMediaDiskCache::lease(Some(file.as_path()));
                    let _ = this.update(cx, |store, cx| store.update_fraction(1.0, notifier_offset, cx));
                    let result = MediaSaveToLibrary::save_files(vec![(file, is_video)], cx).await;
                    drop(lease);
                    result
                }
                Err(error) => {
                    cx.update(|cx| Sai::log(LOG_NAME, || format!("failed: {error}"), SaiLogKind::Ui, cx));
                    MediaSaveToLibraryResult::Failed
                }
            };
            let _ = this.update(cx, |store, cx| {
                store.job = None;
                if let Some(toast_id) = store.toast_id.take() {
                    Notifier::dismiss(toast_id, cx);
                }
                MediaSaveToLibrary::announce(result, NotifierPosition::Bottom, notifier_offset, cx);
            });
        }));
    }

    /// dropping the job stops the run at its next await,
    /// what iOS does after a cancelled run returns nil happens here at once
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.job = None;
        if let Some(toast_id) = self.toast_id.take() {
            Notifier::dismiss(toast_id, cx);
        }
    }

    fn update_fraction(&mut self, value: f64, offset: f32, cx: &mut Context<Self>) {
        if value < self.fraction {
            return;
        }
        self.fraction = value;
        let Some(toast_id) = self.toast_id else {
            return;
        };
        let progress_store = NotifierProgressStore::shared(cx);
        if progress_store.update(cx, |store, cx| store.update(toast_id, value, cx)) {
            return;
        }
        if progress_store.read(cx).can_return(toast_id, cx) {
            self.show_progress(offset, cx);
        }
    }

    fn show_progress(&mut self, offset: f32, cx: &mut Context<Self>) {
        if let Some(toast_id) = self.toast_id {
            let fraction = self.fraction;
            let progress_store = NotifierProgressStore::shared(cx);
            if progress_store.update(cx, |store, cx| store.update(toast_id, fraction, cx)) {
                return;
            }
        }
        let this = cx.weak_entity();
        let on_cancel: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
            let _ = this.update(cx, |store, cx| store.cancel(cx));
        });
        self.toast_id = Some(Notifier::send_progress(
            tr("story_download_background"),
            Some(tr("cancel")),
            self.fraction,
            NotifierPosition::Bottom,
            offset,
            Some(on_cancel),
            cx,
        ));
    }
}
