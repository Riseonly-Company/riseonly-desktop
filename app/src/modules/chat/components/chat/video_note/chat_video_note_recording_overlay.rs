use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{
    App, Bounds, BoxShadow, Context, ElementId, FontWeight, IntoElement, Pixels, Render, RenderOnce, SharedString, Size,
    Task, Window, black, canvas, div, point, prelude::*, px, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_media::camera_capture::call_camera::host_nv12_of;
use rise_media::texture::native_surface::{
    PreparedHostFrame, mirror_host_nv12_frame, prepare_host_frame, present_host_frame,
};
use rise_theme::AppTheme;
use rise_ui::error_text_ui::system_red;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::linear;
use rise_ui::swiftui::shapes::{Shape, centred_stroke};
use rise_ui::swiftui::stacks::{hstack, spacer_min_h, vstack};

use crate::modules::chat::components::chat::video_note::video_note_floating_pip::paint_progress_trim;
use crate::modules::chat::components::chat::video_note::video_note_recorder::VideoNoteCaptureSession;
use crate::modules::chat::components::chat::voice::chat_voice_recording_ui::ChatVoiceLockHint;

static NEXT_VIDEO_NOTE_PREVIEW_STREAM: AtomicU64 = AtomicU64::new(0x00cb_0000_0000_0000);

const RING_MOTION_DURATION: f64 = 0.08;
const RING_LINE_WIDTH: f32 = 3.0;

pub struct VideoNoteCameraPreviewView {
    session: VideoNoteCaptureSession,
    stream_id: u64,
    shown_generation: u64,
    picture: Option<PreparedHostFrame>,
    picture_size: Size<f32>,
    box_size: Size<f32>,
    prepare_task: Option<Task<()>>,
}

impl VideoNoteCameraPreviewView {
    fn new(session: VideoNoteCaptureSession) -> Self {
        Self {
            session,
            stream_id: NEXT_VIDEO_NOTE_PREVIEW_STREAM.fetch_add(1, Ordering::Relaxed),
            shown_generation: 0,
            picture: None,
            picture_size: Size::default(),
            box_size: Size::default(),
            prepare_task: None,
        }
    }

    fn pull_latest_frame(&mut self, cx: &mut Context<Self>) {
        if !self.session.is_running() {
            self.picture = None;
            return;
        }
        let generation = self.session.frame_generation();
        if generation == self.shown_generation || self.prepare_task.is_some() {
            return;
        }
        let Some(frame) = self.session.latest_frame() else {
            return;
        };
        self.shown_generation = generation;
        let stream_id = self.stream_id;
        let prepared = cx.background_spawn(async move {
            let host = host_nv12_of(&frame, stream_id, generation).ok()?;
            let (width, height) = (host.width, host.height);
            let host = if frame.mirrored { mirror_host_nv12_frame(&host)? } else { host };
            let prepared = prepare_host_frame(host).ok()?;
            Some((prepared, size(width as f32, height as f32)))
        });
        self.prepare_task = Some(cx.spawn(async move |this, cx| {
            let prepared = prepared.await;
            let _ = this.update(cx, |view, cx| {
                view.prepare_task = None;
                if let Some((picture, picture_size)) = prepared {
                    view.picture = Some(picture);
                    view.picture_size = picture_size;
                    cx.notify();
                }
            });
        }));
    }

    fn picture_frame(&self) -> Bounds<f32> {
        if self.picture_size.width <= 0.0 || self.picture_size.height <= 0.0 {
            return Bounds { origin: point(0.0, 0.0), size: self.box_size };
        }
        let scale =
            (self.box_size.width / self.picture_size.width).max(self.box_size.height / self.picture_size.height);
        let drawn = size(self.picture_size.width * scale, self.picture_size.height * scale);
        Bounds {
            origin: point((self.box_size.width - drawn.width) / 2.0, (self.box_size.height - drawn.height) / 2.0),
            size: drawn,
        }
    }
}

impl Render for VideoNoteCameraPreviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.pull_latest_frame(cx);
        window.request_animation_frame();
        let picture = self.picture.as_ref().and_then(|picture| present_host_frame(picture).ok());
        let drawn = self.picture_frame();
        let this = cx.weak_entity();

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .left(px(drawn.origin.x))
                    .top(px(drawn.origin.y))
                    .w(px(drawn.size.width))
                    .h(px(drawn.size.height))
                    .grid()
                    .grid_cols(1)
                    .grid_rows(1)
                    .children(picture),
            )
            .child(measure("video-note-camera-preview-size", move |bounds, _, cx| {
                let box_size = size(f32::from(bounds.size.width), f32::from(bounds.size.height));
                let _ = this.update(cx, |view, cx| {
                    if view.box_size != box_size {
                        view.box_size = box_size;
                        cx.notify();
                    }
                });
            }))
    }
}

#[derive(IntoElement)]
pub struct VideoNoteCameraPreview {
    session: VideoNoteCaptureSession,
}

impl VideoNoteCameraPreview {
    pub fn new(session: VideoNoteCaptureSession) -> Self {
        Self { session }
    }
}

impl RenderOnce for VideoNoteCameraPreview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let initial_session = self.session.clone();
        let view = window.use_keyed_state("video-note-camera-preview", cx, move |_, _| {
            VideoNoteCameraPreviewView::new(initial_session)
        });
        view.update(cx, |view, _| view.session = self.session);
        div().size_full().child(view)
    }
}

/// circle and controls, the blur is placed separately behind the composer in the chat page
#[derive(IntoElement)]
pub struct ChatVideoNoteRecordingOverlay {
    id: ElementId,
    theme: AppTheme,
    session: VideoNoteCaptureSession,
    elapsed_milliseconds: i64,
    max_duration_milliseconds: i64,
    is_locked: bool,
    shows_lock_hint: bool,
    shows_cancel_hint: bool,
    drag_translation: Size<Pixels>,
}

impl ChatVideoNoteRecordingOverlay {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        theme: AppTheme,
        session: VideoNoteCaptureSession,
        elapsed_milliseconds: i64,
        max_duration_milliseconds: i64,
        is_locked: bool,
        shows_lock_hint: bool,
        shows_cancel_hint: bool,
        drag_translation: Size<Pixels>,
    ) -> Self {
        Self {
            id: id.into(),
            theme,
            session,
            elapsed_milliseconds,
            max_duration_milliseconds,
            is_locked,
            shows_lock_hint,
            shows_cancel_hint,
            drag_translation,
        }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn recording_progress(&self) -> f32 {
        if self.max_duration_milliseconds <= 0 {
            return 0.0;
        }
        (self.elapsed_milliseconds as f32 / self.max_duration_milliseconds as f32).min(1.0)
    }

    fn camera_circle(&self, circle_size: f32, window: &mut Window, cx: &mut App) -> impl IntoElement + use<> {
        let progress = self.recording_progress();
        let ring = window.use_keyed_state(self.child_id("ring"), cx, |_, _| Animated::new(progress));
        let now = RiseClock::monotonic();
        let shown = ring.update(cx, |animated, cx| {
            animated.set_in(progress, linear(RING_MOTION_DURATION), now, cx);
            animated.frame_value(now, window)
        });
        let accent = self.theme.primary._100;
        div()
            .relative()
            .size(px(circle_size))
            .rounded_full()
            .shadow(vec![BoxShadow {
                color: black().opacity(0.3),
                offset: point(px(0.0), px(6.0)),
                blur_radius: px(20.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded_full()
                    .overflow_hidden()
                    .child(VideoNoteCameraPreview::new(self.session.clone())),
            )
            .child(centred_stroke(white().opacity(0.2), RING_LINE_WIDTH, Shape::Circle))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, _| {
                        paint_progress_trim(bounds, shown, RING_LINE_WIDTH, accent, window);
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}

impl RenderOnce for ChatVideoNoteRecordingOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let geometry = window.use_keyed_state(self.child_id("geometry"), cx, |_, _| Size::<f32>::default());
        let geo = *geometry.read(cx);
        let circle_size = (geo.width * 0.73).min(geo.height * 0.58).min(300.0);
        let red = system_red(cx);
        let camera_circle = self.camera_circle(circle_size, window, cx);
        let cancel_offset = (f32::from(self.drag_translation.width) * 0.25).min(0.0);
        let chevron_tint = self.theme.text.primary;
        let measure_id = self.child_id("geometry-measure");

        div()
            .relative()
            .size_full()
            .child(
                vstack(0.0_f32)
                    .size_full()
                    .child(spacer_min_h(0.0))
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(camera_circle)
                            .when(self.shows_cancel_hint, |stack| {
                                stack.child(
                                    div().absolute().inset_0().flex().items_center().pl(px(24.0)).child(
                                        hstack(6.0_f32)
                                            .relative()
                                            .left(px(cancel_offset))
                                            .children(IconUi::sized("chevron.left", px(13.0), chevron_tint, false))
                                            .child(
                                                MainText::new(tr("cancel"))
                                                    .px(15.0)
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .color(red.opacity(0.95)),
                                            ),
                                    ),
                                )
                            }),
                    )
                    .child(spacer_min_h(0.0))
                    .when(self.shows_lock_hint && !self.is_locked, |stack| {
                        stack.child(div().pb(px(16.0)).child(ChatVoiceLockHint::new(self.theme.clone())))
                    }),
            )
            .child(measure(measure_id, move |bounds, _, cx| {
                let measured = size(f32::from(bounds.size.width), f32::from(bounds.size.height));
                geometry.update(cx, |geo, cx| {
                    if *geo != measured {
                        *geo = measured;
                        cx.notify();
                    }
                });
            }))
    }
}
