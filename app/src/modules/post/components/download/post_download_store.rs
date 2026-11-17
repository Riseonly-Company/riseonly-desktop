use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;

use crate::core::media::save_to_folder_exporter::SaveToFolderExporter;
use crate::core::media_viewer::media_save_to_library::MediaSaveToLibrary;
use crate::core::widgets::notifier::notifier_progress::NotifierProgressStore;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition};
use crate::modules::post::components::download::post_download_coordinator::{
    PostDownloadCache, PostDownloadCoordinator, PostDownloadCoordinatorProgress,
};
use crate::modules::post::components::download::post_download_policy::PostDownloadPolicy;
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostDownloadProgress {
    pub current: usize,
    pub total: usize,
    pub fraction: f64,
}

type PostDownloadSheetStoreAfterSheet = Box<dyn FnOnce(&mut PostDownloadSheetStore, &mut Context<PostDownloadSheetStore>)>;

/// every Download entry lands here
/// one media with no watermark choice starts at once
/// several media or a choice open the root sheet above the media viewer
pub struct PostDownloadSheetStore {
    is_presented: bool,
    post: Option<GetPostFeedResponse>,
    slides: Vec<PostMediaSlide>,
    policy: Option<PostDownloadPolicy>,
    focus_index: usize,
    selected: Vec<usize>,
    pub with_watermark: bool,
    progress: Option<PostDownloadProgress>,
    job: Option<Task<()>>,
    picking_folder: bool,
    toast_id: Option<Uuid>,
    notifier_offset: f32,
    sheet_closing: bool,
    after_sheet: Option<PostDownloadSheetStoreAfterSheet>,
    sheet_fallback: Option<Task<()>>,
    viewer_posts: HashMap<String, GetPostFeedResponse>,
}

impl PostDownloadSheetStore {
    const VIEWER_POSTS_CAPACITY: usize = 32;
    const PART_SWEEP_AGE: Duration = Duration::from_secs(60);
    const SHEET_FALLBACK_EXTRA: f64 = 0.4;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        cx.background_executor()
            .spawn(async { PostDownloadCache::sweep_parts(Self::PART_SWEEP_AGE) })
            .detach();
        Self {
            is_presented: false,
            post: None,
            slides: Vec::new(),
            policy: None,
            focus_index: 0,
            selected: Vec::new(),
            with_watermark: true,
            progress: None,
            job: None,
            picking_folder: false,
            toast_id: None,
            notifier_offset: 0.0,
            sheet_closing: false,
            after_sheet: None,
            sheet_fallback: None,
            viewer_posts: HashMap::new(),
        }
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    pub fn post(&self) -> Option<&GetPostFeedResponse> {
        self.post.as_ref()
    }

    pub fn slides(&self) -> &[PostMediaSlide] {
        &self.slides
    }

    pub fn policy(&self) -> Option<PostDownloadPolicy> {
        self.policy
    }

    pub fn focus_index(&self) -> usize {
        self.focus_index
    }

    pub fn selected(&self) -> &[usize] {
        &self.selected
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    /// only_focused is the media viewer, its save button acts on the page on screen
    /// notifier_offset lifts the banners over a bottom bar of the caller
    pub fn start(
        &mut self,
        post: GetPostFeedResponse,
        focus_index: usize,
        only_focused: bool,
        notifier_offset: f32,
        cx: &mut Context<Self>,
    ) {
        if self.job.is_some() {
            self.notifier_offset = notifier_offset;
            self.show_progress(cx);
            return;
        }
        if self.picking_folder {
            return;
        }
        let Some(policy) = post.download_policy(cx).filter(|policy| policy.allowed) else {
            return;
        };
        let all = post.media_slides();
        if all.is_empty() {
            return;
        }
        let focus = focus_index.min(all.len() - 1);
        let slides = if only_focused { vec![all[focus].clone()] } else { all };
        let focus_in_slides = if only_focused { 0 } else { focus };
        self.notifier_offset = notifier_offset;
        if slides.len() == 1 && !policy.chooses_watermark {
            self.run(post, slides, true, cx);
            return;
        }
        self.post = Some(post);
        self.slides = slides;
        self.policy = Some(policy);
        self.focus_index = focus_in_slides;
        self.selected = vec![focus_in_slides];
        self.with_watermark = true;
        self.is_presented = true;
        cx.notify();
    }

    pub fn remember_viewer_post(&mut self, post: GetPostFeedResponse) {
        if self.viewer_posts.len() > Self::VIEWER_POSTS_CAPACITY {
            self.viewer_posts.clear();
        }
        self.viewer_posts.insert(post.id.clone(), post);
    }

    pub fn viewer_post(&self, post_id: &str) -> Option<GetPostFeedResponse> {
        self.viewer_posts.get(post_id).cloned()
    }

    pub fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.is_running() || index >= self.slides.len() {
            return;
        }
        let _ = selection_changed();
        if let Some(position) = self.selected.iter().position(|selected| *selected == index) {
            self.selected.remove(position);
        } else {
            self.selected.push(index);
        }
        cx.notify();
    }

    pub fn toggle_watermark(&mut self, cx: &mut Context<Self>) {
        if self.is_running() {
            return;
        }
        let _ = selection_changed();
        self.with_watermark = !self.with_watermark;
        cx.notify();
    }

    pub fn position(&self, index: usize) -> Option<usize> {
        self.selected.iter().position(|selected| *selected == index).map(|position| position + 1)
    }

    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        let Some(post) = self.post.clone() else {
            return;
        };
        if self.selected.is_empty() || self.is_running() {
            return;
        }
        let watermark = if self.policy.is_some_and(|policy| policy.chooses_watermark) { self.with_watermark } else { true };
        let items: Vec<PostMediaSlide> =
            self.selected.iter().filter_map(|index| self.slides.get(*index).cloned()).collect();
        self.run(post, items, watermark, cx);
    }

    /// dropping the job stops the run at its next await
    /// what the other client does once the cancelled run returns nil happens here at once
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.job.take().is_none() {
            return;
        }
        self.progress = None;
        if let Some(toast_id) = self.toast_id.take() {
            Notifier::dismiss(toast_id, cx);
        }
        if self.sheet_closing {
            self.after_sheet = None;
        }
        cx.notify();
    }

    /// only closes the sheet, a run started from it already works under its own banner
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.is_presented = false;
        cx.notify();
    }

    pub fn sheet_did_dismiss(&mut self, cx: &mut Context<Self>) {
        self.sheet_fallback = None;
        if !self.sheet_closing {
            return;
        }
        self.sheet_closing = false;
        if let Some(pending) = self.after_sheet.take() {
            pending(self, cx);
        }
    }

    /// shows a banner now or once the sheet is off the screen
    fn when_sheet_gone(&mut self, action: PostDownloadSheetStoreAfterSheet, cx: &mut Context<Self>) {
        if self.sheet_closing {
            self.after_sheet = Some(action);
        } else {
            action(self, cx);
        }
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.job = None;
        self.picking_folder = false;
        self.sheet_fallback = None;
        self.sheet_closing = false;
        self.after_sheet = None;
        if let Some(toast_id) = self.toast_id.take() {
            Notifier::dismiss(toast_id, cx);
        }
        self.progress = None;
        self.is_presented = false;
        self.post = None;
        self.slides.clear();
        self.selected.clear();
        self.viewer_posts.clear();
        cx.notify();
    }

    /// nobody has to watch the sheet while it works
    /// sheet closes on the tap and the run goes on in the background
    /// bottom banner with ring, title and cancel
    fn run(&mut self, post: GetPostFeedResponse, items: Vec<PostMediaSlide>, watermark: bool, cx: &mut Context<Self>) {
        if self.is_presented {
            self.sheet_closing = true;
            let delay = Duration::from_secs_f64(BottomSheetMetrics::DISMISS_DURATION + Self::SHEET_FALLBACK_EXTRA);
            self.sheet_fallback = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(delay).await;
                let _ = this.update(cx, |store, cx| store.sheet_did_dismiss(cx));
            }));
        }
        self.is_presented = false;
        self.picking_folder = true;
        cx.notify();
        let this = cx.weak_entity();
        SaveToFolderExporter::pick_folder(
            move |folder, cx| {
                let _ = this.update(cx, |store, cx| {
                    store.picking_folder = false;
                    match folder {
                        Some(folder) => store.start_job(post, items, watermark, folder, cx),
                        None => {
                            if store.sheet_closing {
                                store.after_sheet = None;
                            }
                            cx.notify();
                        }
                    }
                });
            },
            cx,
        );
    }

    fn start_job(
        &mut self,
        post: GetPostFeedResponse,
        items: Vec<PostMediaSlide>,
        watermark: bool,
        folder: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let total = items.len();
        self.progress = Some(PostDownloadProgress { current: 1, total, fraction: 0.0 });
        self.when_sheet_gone(Box::new(|store, cx| store.show_progress(cx)), cx);
        self.job = Some(cx.spawn(async move |this, cx| {
            let reporting = this.clone();
            let on_progress: PostDownloadCoordinatorProgress = Rc::new(move |current, fraction, cx: &mut App| {
                let _ = reporting.update(cx, |store, cx| {
                    if store.progress.is_none() {
                        return;
                    }
                    store.progress = Some(PostDownloadProgress { current, total, fraction });
                    store.keep_progress_shown(fraction, cx);
                });
            });
            let result = PostDownloadCoordinator::run(post, items, watermark, folder, on_progress, cx).await;
            let _ = this.update(cx, |store, cx| {
                store.job = None;
                store.progress = None;
                store.toast_id = None;
                let offset = store.notifier_offset;
                store.when_sheet_gone(
                    Box::new(move |_, cx| MediaSaveToLibrary::announce(result, NotifierPosition::Bottom, offset, cx)),
                    cx,
                );
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn keep_progress_shown(&mut self, fraction: f64, cx: &mut Context<Self>) {
        let Some(toast_id) = self.toast_id else {
            return;
        };
        let progress_store = NotifierProgressStore::shared(cx);
        if progress_store.update(cx, |store, cx| store.update(toast_id, fraction, cx)) {
            return;
        }
        if progress_store.read(cx).can_return(toast_id, cx) {
            self.show_progress(cx);
        }
    }

    /// updates the banner in place or brings it back if a tap or another banner removed it
    fn show_progress(&mut self, cx: &mut Context<Self>) {
        if self.sheet_closing {
            return;
        }
        let Some(progress) = self.progress else {
            return;
        };
        if let Some(toast_id) = self.toast_id {
            let progress_store = NotifierProgressStore::shared(cx);
            if progress_store.update(cx, |store, cx| store.update(toast_id, progress.fraction, cx)) {
                return;
            }
        }
        let this = cx.weak_entity();
        let on_cancel: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
            let _ = this.update(cx, |store, cx| store.cancel(cx));
        });
        self.toast_id = Some(Notifier::send_progress(
            Self::progress_title(progress.total),
            Some(tr("cancel")),
            progress.fraction,
            NotifierPosition::Bottom,
            self.notifier_offset,
            Some(on_cancel),
            cx,
        ));
    }

    /// leaves progress to the banner, which fills it with the ring percent
    pub fn progress_title(count: usize) -> String {
        tr("post_download_background").replace("{count}", &count.to_string())
    }
}
