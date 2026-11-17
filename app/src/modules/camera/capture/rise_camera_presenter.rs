use std::collections::HashSet;
use std::time::Duration;

use gpui::{App, AppContext, Bounds, Context, Edges, Entity, Pixels, Task, Window};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::motion::{MotionSpec, Spring, spring};

use crate::app::router::navigation_types::CreatePostIntent;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::camera::capture::rise_camera_capture::{RiseCameraCapture, RiseCameraShot};
use crate::modules::camera::capture::rise_camera_mode::{RiseCameraMode, RiseCameraModePreference};
use crate::modules::camera::components::feed_media_picker_reveal::FeedMediaPickerReveal;
use crate::modules::media_picker::api::media_picker_configuration::{MediaPickerConfiguration, MediaPickerMediaKind};
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerResult};
use crate::modules::media_picker::editor::editor_state::{EditorImageState, EditorVideoState};
use crate::modules::media_picker::export::export_progress_view_state::ExportProgressViewState;
use crate::modules::media_picker::export::media_exporter::MediaExporter;
use crate::modules::media_picker::selection::media_selection_state::{MediaSelectionEdit, MediaSelectionEditPayload};
use crate::modules::media_picker::ui::media_picker_camera_cell::MediaPickerCameraFeed;
use crate::modules::story::components::story_composer_host::StoryFeedComposerPresenter;

/// a shot on its way into the existing flow of the mode it was taken in
#[derive(Clone)]
pub struct RiseCameraDraft {
    pub id: Uuid,
    pub shot: RiseCameraShot,
    pub mode: RiseCameraMode,
}

/// exported story waiting in the story publish sheet
#[derive(Clone)]
pub struct RiseCameraStoryComposition {
    pub id: Uuid,
    pub draft: RiseCameraDraft,
    pub item: MediaPickerItem,
}

#[derive(Clone)]
pub enum RiseCameraPresenterStage {
    Camera,
    Exporting(RiseCameraDraft),
    Story(RiseCameraStoryComposition),
}

/// takes a fresh shot into the post composer of its mode
pub type RiseCameraOpenPost = Box<dyn FnOnce(CreatePostIntent, MediaPickerResult, &mut App)>;
pub type RiseCameraOpenPostPicker = Box<dyn FnOnce(CreatePostIntent, &mut App)>;
/// the picker sheet that opened the camera takes the shot as its own result
pub type RiseCameraCaptureSink = Box<dyn FnOnce(RiseCameraShot, &mut App)>;

/// feed camera presentation, how far it slid in and which step of the flow is on screen
/// the feed pager drives progress live while the pointer drags it in from the left edge
pub struct RiseCameraPresenter {
    pub capture: Entity<RiseCameraCapture>,
    pub export_progress: Entity<ExportProgressViewState>,

    progress: f32,
    progress_motion: Animated<f32>,
    is_dragging: bool,
    stage: RiseCameraPresenterStage,
    is_from_picker: bool,
    is_windowed: bool,
    mode: RiseCameraMode,

    origin_rect: Option<Bounds<Pixels>>,
    pending_origin_rect: Option<Bounds<Pixels>>,

    capture_sink: Option<RiseCameraCaptureSink>,
    capture_configuration: Option<MediaPickerConfiguration>,
    window_safe_area: Edges<Pixels>,

    export_task: Option<Task<()>>,
    close_generation: u64,
    is_settling_closed: bool,
    awaits_entrance_mount: bool,
}

impl RiseCameraPresenter {
    pub const OPEN_THRESHOLD: f32 = 0.3;
    pub const FLING_VELOCITY: f32 = 600.0;
    pub const ENTRANCE_MOUNT_TIMEOUT: TimeInterval = 0.5;
    const SETTLE_RESPONSE: f64 = 0.34;
    const SETTLE_DAMPING_FRACTION: f64 = 0.92;
    const GALLERY_OPEN_DELAY: TimeInterval = 0.2;
    const ENTRANCE_MOUNT_PROGRESS: f32 = 0.002;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            capture: cx.new(RiseCameraCapture::new),
            export_progress: cx.new(ExportProgressViewState::new),
            progress: 0.0,
            progress_motion: Animated::new(0.0),
            is_dragging: false,
            stage: RiseCameraPresenterStage::Camera,
            is_from_picker: false,
            is_windowed: false,
            mode: RiseCameraModePreference::mode(),
            origin_rect: None,
            pending_origin_rect: None,
            capture_sink: None,
            capture_configuration: None,
            window_safe_area: Edges::default(),
            export_task: None,
            close_generation: 0,
            is_settling_closed: false,
            awaits_entrance_mount: false,
        }
    }

    pub fn progress(&self) -> f32 {
        self.progress
    }

    /// progress the host draws this frame, settle spring included
    pub fn presented_progress(&self, window: &Window) -> f32 {
        self.progress_motion.frame_value(RiseClock::monotonic(), window)
    }

    pub fn is_progress_animating(&self, now: MotionTime) -> bool {
        self.progress_motion.is_running(now)
    }

    pub fn is_dragging(&self) -> bool {
        self.is_dragging
    }

    pub fn stage(&self) -> &RiseCameraPresenterStage {
        &self.stage
    }

    pub fn is_from_picker(&self) -> bool {
        self.is_from_picker
    }

    pub fn is_windowed(&self) -> bool {
        self.is_windowed
    }

    pub fn is_settling_closed(&self) -> bool {
        self.is_settling_closed
    }

    pub fn mode(&self) -> RiseCameraMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: RiseCameraMode, cx: &mut Context<Self>) {
        self.mode = mode;
        if !self.is_windowed {
            RiseCameraModePreference::set_mode(mode);
        }
        cx.notify();
    }

    pub fn origin_rect(&self) -> Option<Bounds<Pixels>> {
        self.origin_rect
    }

    pub fn window_safe_area(&self) -> Edges<Pixels> {
        self.window_safe_area
    }

    pub fn is_on_screen(&self) -> bool {
        self.is_dragging || self.progress > 0.001 || self.is_settling_closed
    }

    pub fn is_open(&self) -> bool {
        !self.is_dragging && self.progress >= 0.999
    }

    fn settle_spec() -> MotionSpec {
        spring(Self::SETTLE_RESPONSE, Self::SETTLE_DAMPING_FRACTION)
    }

    fn set_progress(&mut self, value: f32, spec: Option<MotionSpec>, cx: &mut Context<Self>) {
        self.progress = value;
        self.progress_motion.set_in(value, spec, RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn is_stage_camera(&self) -> bool {
        matches!(self.stage, RiseCameraPresenterStage::Camera)
    }

    pub fn feed_drag_changed(&mut self, fraction: f32, cx: &mut Context<Self>) {
        if !self.is_dragging {
            self.is_dragging = true;
            self.origin_rect = None;
            self.pending_origin_rect = None;
            if self.is_windowed {
                self.release_window(cx);
            }
            self.close_generation = self.close_generation.wrapping_add(1);
            if !self.is_stage_camera() {
                self.reset_stage(cx);
            }
            self.set_mode(RiseCameraModePreference::mode(), cx);
            self.capture.update(cx, |capture, cx| capture.warm_up(cx));
        }
        self.set_progress(fraction.clamp(0.0, 1.0), None, cx);
    }

    pub fn feed_drag_ended(&mut self, fraction: f32, velocity: f32, cx: &mut Context<Self>) {
        if !self.is_dragging {
            return;
        }
        self.is_dragging = false;
        let opens = (fraction > Self::OPEN_THRESHOLD && velocity > -Self::FLING_VELOCITY)
            || (velocity > Self::FLING_VELOCITY && fraction > 0.0);
        self.settle(opens, cx);
    }

    pub fn feed_drag_cancelled(&mut self, cx: &mut Context<Self>) {
        if !self.is_dragging {
            return;
        }
        self.is_dragging = false;
        self.settle(false, cx);
    }

    pub fn note_origin_tile(&mut self, rect: Bounds<Pixels>) {
        if self.is_on_screen() || f32::from(rect.size.width) <= 1.0 || f32::from(rect.size.height) <= 1.0 {
            return;
        }
        self.pending_origin_rect = Some(rect);
    }

    /// growing out of a tile needs a first frame at the tile size
    /// host mounts the camera at a sliver of progress, spring starts after it reports
    /// no next turn start, it runs before first render
    fn open_after_mount(&mut self, cx: &mut Context<Self>) {
        if self.origin_rect.is_none() {
            self.settle(true, cx);
            return;
        }
        self.set_progress(Self::ENTRANCE_MOUNT_PROGRESS, None, cx);
        self.awaits_entrance_mount = true;
        let generation = self.close_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(Self::ENTRANCE_MOUNT_TIMEOUT)).await;
            let _ = this.update(cx, |this, cx| this.start_entrance_spring(generation, cx));
        })
        .detach();
    }

    pub fn entrance_did_mount(&mut self, cx: &mut Context<Self>) {
        if !self.awaits_entrance_mount {
            return;
        }
        let generation = self.close_generation;
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| this.start_entrance_spring(generation, cx));
        })
        .detach();
    }

    fn start_entrance_spring(&mut self, generation: u64, cx: &mut Context<Self>) {
        if !self.awaits_entrance_mount || self.close_generation != generation || self.is_dragging {
            return;
        }
        self.settle(true, cx);
    }

    fn take_origin_rect(&mut self) {
        self.origin_rect = self.pending_origin_rect.take();
    }

    pub fn present_from_picker(&mut self, cx: &mut Context<Self>) {
        if self.is_on_screen() {
            return;
        }
        self.is_from_picker = true;
        self.take_origin_rect();
        self.close_generation = self.close_generation.wrapping_add(1);
        if !self.is_stage_camera() {
            self.reset_stage(cx);
        }
        self.set_mode(RiseCameraModePreference::mode(), cx);
        MediaPickerCameraFeed::suspend(cx);
        self.capture.update(cx, |capture, cx| capture.warm_up(cx));
        self.open_after_mount(cx);
    }

    pub fn present_for_capture(
        configuration: MediaPickerConfiguration,
        on_capture: RiseCameraCaptureSink,
        cx: &mut App,
    ) {
        Self::shared(cx).update(cx, |this, cx| this.present_for_capture_now(configuration, on_capture, cx));
    }

    fn present_for_capture_now(
        &mut self,
        configuration: MediaPickerConfiguration,
        on_capture: RiseCameraCaptureSink,
        cx: &mut Context<Self>,
    ) {
        if self.is_on_screen() {
            return;
        }
        self.close_generation = self.close_generation.wrapping_add(1);
        if !self.is_stage_camera() {
            self.reset_stage(cx);
        }
        self.is_windowed = true;
        self.is_from_picker = true;
        self.take_origin_rect();
        self.capture_sink = Some(on_capture);
        self.capture_configuration = Some(configuration);
        self.set_mode(RiseCameraMode::Publication, cx);
        self.window_safe_area = Edges::default();

        MediaPickerCameraFeed::suspend(cx);
        self.capture.update(cx, |capture, cx| capture.warm_up(cx));
        self.open_after_mount(cx);
    }

    /// photo shutter is refused here, the picker takes videos only
    pub fn takes_photos(&self) -> bool {
        self.mode.takes_photos()
            && self
                .capture_configuration
                .as_ref()
                .is_none_or(|configuration| configuration.kind != MediaPickerMediaKind::Videos)
    }

    pub fn records_video(&self) -> bool {
        self.capture_configuration
            .as_ref()
            .is_none_or(|configuration| configuration.kind != MediaPickerMediaKind::Images)
    }

    pub fn max_recording_seconds(&self, cx: &App) -> TimeInterval {
        if let Some(limit) = self
            .capture_configuration
            .as_ref()
            .and_then(|configuration| configuration.max_video_duration_seconds)
            .filter(|limit| *limit > 0)
        {
            return limit as TimeInterval;
        }
        self.mode.max_recording_seconds(cx)
    }

    /// layer of the sheet camera goes once the camera slid away
    fn release_window(&mut self, cx: &mut Context<Self>) {
        self.capture_sink = None;
        self.capture_configuration = None;
        self.is_windowed = false;
        self.set_mode(RiseCameraModePreference::mode(), cx);
    }

    pub fn close_drag_changed(&mut self, fraction: f32, cx: &mut Context<Self>) {
        if !self.is_stage_camera() || self.capture.read(cx).is_recording() {
            return;
        }
        self.is_dragging = true;
        self.set_progress(1.0 - fraction.clamp(0.0, 1.0), None, cx);
    }

    pub fn close_drag_ended(&mut self, fraction: f32, velocity: f32, cx: &mut Context<Self>) {
        if !self.is_dragging {
            return;
        }
        self.is_dragging = false;
        let closes = (fraction > Self::OPEN_THRESHOLD && velocity < Self::FLING_VELOCITY)
            || velocity < -Self::FLING_VELOCITY;
        self.settle(!closes, cx);
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.is_dragging = false;
        self.settle(false, cx);
    }

    fn settle(&mut self, open: bool, cx: &mut Context<Self>) {
        self.awaits_entrance_mount = false;
        self.is_settling_closed = !open;
        if open {
            self.set_progress(1.0, Some(Self::settle_spec()), cx);
            self.capture.update(cx, |capture, cx| capture.did_open(cx));
            return;
        }
        self.close_generation = self.close_generation.wrapping_add(1);
        let generation = self.close_generation;
        self.set_progress(0.0, Some(Self::settle_spec()), cx);
        let settling = Spring::new(Self::SETTLE_RESPONSE, Self::SETTLE_DAMPING_FRACTION).settling_duration();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(settling).await;
            let _ = this.update(cx, |this, cx| this.finish_close(generation, cx));
        })
        .detach();
        self.capture.update(cx, |capture, cx| capture.shut_down(cx));
        if self.is_from_picker {
            self.is_from_picker = false;
            MediaPickerCameraFeed::resume(cx);
        }
        cx.notify();
    }

    fn finish_close(&mut self, generation: u64, cx: &mut Context<Self>) {
        if self.close_generation != generation
            || !self.is_settling_closed
            || self.is_dragging
            || self.progress > 0.001
        {
            return;
        }
        self.is_settling_closed = false;
        self.origin_rect = None;
        self.reset_stage(cx);
        if self.is_windowed {
            self.release_window(cx);
        }
        cx.notify();
    }

    pub fn did_take(
        &mut self,
        shot: RiseCameraShot,
        open_post: impl FnOnce(CreatePostIntent, MediaPickerResult, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let open_post: RiseCameraOpenPost = Box::new(open_post);
        if !self.is_open() || !self.is_stage_camera() {
            shot.discard();
            return;
        }
        if let Some(sink) = self.capture_sink.take() {
            self.close(cx);
            cx.defer(move |cx| sink(shot, cx));
            return;
        }
        let draft = RiseCameraDraft { id: Uuid::new_v4(), shot: shot.clone(), mode: self.mode };
        self.stage = RiseCameraPresenterStage::Exporting(draft.clone());
        self.capture.update(cx, |capture, cx| capture.suspend(cx));
        cx.notify();

        let edit = Self::untouched_edit(&shot);
        let progress_state = self.export_progress.downgrade();
        self.export_task = Some(cx.spawn(async move |this, cx| {
            let exporter = MediaExporter::new();
            let exported = match shot.url.to_file_path() {
                Some(source) => {
                    exporter
                        .export_external(
                            source,
                            shot.image.clone(),
                            shot.media_asset(),
                            edit,
                            None,
                            progress_state,
                            cx,
                        )
                        .await
                }
                None => Err(anyhow::anyhow!("camera shot outside the file system")),
            };
            let _ = this.update(cx, |this, cx| this.export_finished(draft, exported, open_post, cx));
        }));
    }

    fn export_finished(
        &mut self,
        draft: RiseCameraDraft,
        exported: anyhow::Result<MediaPickerItem>,
        open_post: RiseCameraOpenPost,
        cx: &mut Context<Self>,
    ) {
        let is_current =
            matches!(&self.stage, RiseCameraPresenterStage::Exporting(current) if current.id == draft.id);
        match exported {
            Ok(item) => {
                if !is_current {
                    Self::remove_generated_files(&item, &draft.shot.url);
                    return;
                }
                self.export_task = None;
                if let Some(intent) = draft.mode.post_intent() {
                    let result = MediaPickerResult::new(vec![item], false);
                    self.stage = RiseCameraPresenterStage::Camera;
                    open_post(intent, result, cx);
                    FeedMediaPickerReveal::shared(cx).update(cx, |reveal, cx| reveal.close(false, cx));
                    self.close(cx);
                } else {
                    self.stage = RiseCameraPresenterStage::Story(RiseCameraStoryComposition {
                        id: Uuid::new_v4(),
                        draft,
                        item,
                    });
                    cx.notify();
                }
            }
            Err(_) => {
                if !is_current {
                    return;
                }
                self.export_task = None;
                draft.shot.discard();
                self.stage = RiseCameraPresenterStage::Camera;
                self.capture.update(cx, |capture, cx| capture.resume(cx));
                cx.notify();
                Notifier::send(
                    tr("story_share_export_error_title"),
                    NotifierSendOptions {
                        content: Some(tr("story_share_export_error_message")),
                        r#type: NotifierType::Error,
                        position: NotifierPosition::Top,
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            }
        }
    }

    fn untouched_edit(shot: &RiseCameraShot) -> MediaSelectionEdit {
        let asset_id = shot.media_asset().id;
        if shot.is_video() {
            return MediaSelectionEdit {
                payload: MediaSelectionEditPayload::Video(EditorVideoState::empty(Some(asset_id), shot.duration)),
            };
        }
        MediaSelectionEdit {
            payload: MediaSelectionEditPayload::Image(EditorImageState::empty(Some(asset_id), shot.pixel_size)),
        }
    }

    pub fn story_sheet_closed(&mut self, published: bool, cx: &mut Context<Self>) {
        let RiseCameraPresenterStage::Story(composition) = &self.stage else {
            return;
        };
        let composition = composition.clone();
        if published {
            self.stage = RiseCameraPresenterStage::Camera;
            FeedMediaPickerReveal::shared(cx).update(cx, |reveal, cx| reveal.close(false, cx));
            self.close(cx);
            return;
        }
        Self::remove_generated_files(&composition.item, &composition.draft.shot.url);
        composition.draft.shot.discard();
        self.stage = RiseCameraPresenterStage::Camera;
        self.capture.update(cx, |capture, cx| capture.resume(cx));
        cx.notify();
    }

    pub fn open_gallery(
        &mut self,
        open_post_picker: impl FnOnce(CreatePostIntent, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let open_post_picker: RiseCameraOpenPostPicker = Box::new(open_post_picker);
        let mode = self.mode;
        if self.is_windowed || (self.is_from_picker && mode.post_intent().is_some()) {
            self.close(cx);
            return;
        }
        self.close(cx);
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(Self::GALLERY_OPEN_DELAY)).await;
            cx.update(|cx| match mode.post_intent() {
                Some(intent) => open_post_picker(intent, cx),
                None => StoryFeedComposerPresenter::shared(cx).update(cx, |presenter, cx| presenter.present(cx)),
            });
        })
        .detach();
    }

    fn reset_stage(&mut self, cx: &mut Context<Self>) {
        self.export_task = None;
        match &self.stage {
            RiseCameraPresenterStage::Camera => {}
            RiseCameraPresenterStage::Exporting(draft) => draft.shot.discard(),
            RiseCameraPresenterStage::Story(composition) => {
                Self::remove_generated_files(&composition.item, &composition.draft.shot.url);
                composition.draft.shot.discard();
            }
        }
        self.stage = RiseCameraPresenterStage::Camera;
        cx.notify();
    }

    fn remove_generated_files(item: &MediaPickerItem, keeping: &Url) {
        let mut urls = vec![item.url.clone()];
        if let Some(poster) = item.video_payload.as_ref().and_then(|video| video.poster_url.clone()) {
            urls.push(poster);
        }
        if let Some(original) = item.image_payload.as_ref().and_then(|image| image.original_image_url.clone()) {
            urls.push(original);
        }
        let kept = keeping.to_file_path();
        let mut removed = HashSet::new();
        for path in urls.iter().filter_map(Url::to_file_path) {
            if Some(&path) != kept.as_ref() && removed.insert(path.clone()) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}
