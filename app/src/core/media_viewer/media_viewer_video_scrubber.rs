use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Bounds, Context, DispatchPhase, Entity, FontFeatures, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Path, Pixels, Point, Render,
    StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, black, canvas, div, px, white,
};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_i18n::tr;
use rise_ui::liquid_chrome::LiquidChromeModifiers as _;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{ease_out, linear};

use crate::core::lib::canon_format::RiseFormat;
use crate::core::media_viewer::media_viewer_types::MediaViewerVideoTiming;
use crate::core::media_viewer::media_viewer_video_player::MediaViewerVideoController;
pub use crate::core::ui::animated_tabs_ui::rounded_rect_path_hooks::rounded_rect_path;

/// seek bar under a video in the viewer, its own view like the rail
/// drag with left button, hover shows the time above the track
pub struct MediaViewerVideoScrubber {
    controller: Entity<MediaViewerVideoController>,
    state: Entity<MediaViewerVideoScrubberState>,
    shown_labels: (String, String),
    painted: Rc<Cell<bool>>,
    glide_running: bool,
    _subscriptions: Vec<Subscription>,
}

enum MediaViewerVideoScrubberMetrics {}

impl MediaViewerVideoScrubberMetrics {
    const ROW_HEIGHT: f32 = 44.0;
    const VERTICAL_PADDING: f32 = 2.0;
    const TRACK_HEIGHT: f32 = 7.0;
    const SCRUBBING_TRACK_HEIGHT: f32 = 9.0;
    const THUMB_DIAMETER: f32 = 15.0;
    const SCRUBBING_THUMB_SCALE: f32 = 1.28;
    const LABEL_SPACING: f32 = 12.0;
    const GLIDE_DURATION: f64 = MediaViewerVideoTiming::PROGRESS_PUBLICATION * 1.2;
    const GRAB_DURATION: f64 = 0.16;
    const THUMB_SHADOW_OPACITY: f32 = 0.2;
    const THUMB_SHADOW_SPREAD: f32 = 1.5;
    const THUMB_SHADOW_OFFSET_Y: f32 = 1.0;
}

/// moving geometry of the track and the pointer state of one scrubber
struct MediaViewerVideoScrubberState {
    track_bounds: Bounds<Pixels>,
    filled: Animated<f32>,
    track_height: Animated<f32>,
    thumb_scale: Animated<f32>,
    is_pressed: bool,
    hover_x: Option<f32>,
}

impl MediaViewerVideoScrubberState {
    fn new() -> Self {
        Self {
            track_bounds: Bounds::default(),
            filled: Animated::new(MediaViewerVideoScrubberMetrics::THUMB_DIAMETER / 2.0),
            track_height: Animated::new(MediaViewerVideoScrubberMetrics::TRACK_HEIGHT),
            thumb_scale: Animated::new(1.0),
            is_pressed: false,
            hover_x: None,
        }
    }
}

/// fill and thumb shadow and thumb as painted, in the track coordinates
struct MediaViewerVideoScrubberShapes {
    fill: Option<Path<Pixels>>,
    shadow: Option<Path<Pixels>>,
    thumb: Option<Path<Pixels>>,
}

impl MediaViewerVideoScrubber {
    pub const HEIGHT: f32 =
        MediaViewerVideoScrubberMetrics::ROW_HEIGHT + MediaViewerVideoScrubberMetrics::VERTICAL_PADDING * 2.0;

    pub fn new(controller: Entity<MediaViewerVideoController>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let timeline = controller.read(cx).timeline().clone();
        let subscriptions =
            vec![cx.observe_in(&timeline, window, |this: &mut Self, _, window, cx| this.timeline_did_step(window, cx))];
        Self {
            controller,
            state: cx.new(|_| MediaViewerVideoScrubberState::new()),
            shown_labels: (String::new(), String::new()),
            painted: Rc::new(Cell::new(false)),
            glide_running: false,
            _subscriptions: subscriptions,
        }
    }

    fn labels(position: TimeInterval, total: TimeInterval) -> (String, String) {
        (
            format!("{}", RiseFormat::duration_time_interval(position)),
            format!("{}", RiseFormat::duration_time_interval(total)),
        )
    }

    /// the thumb is inset by its own radius at both ends
    /// one definition of where the playhead is so fill and circle cant disagree
    fn filled_target(width: f32, fraction: f64) -> f32 {
        let travel = (width - MediaViewerVideoScrubberMetrics::THUMB_DIAMETER).max(1.0);
        MediaViewerVideoScrubberMetrics::THUMB_DIAMETER / 2.0 + travel * fraction as f32
    }

    fn timeline_did_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (position, total, fraction, scrubbing) = {
            let controller = self.controller.read(cx);
            (controller.progress(), controller.duration(), controller.fraction_complete(), controller.is_scrubbing())
        };
        if scrubbing || Self::labels(position, total) != self.shown_labels {
            cx.notify();
            return;
        }
        let width = f32::from(self.state.read(cx).track_bounds.size.width);
        let target = Self::filled_target(width, fraction);
        let now = RiseClock::monotonic();
        self.state.update(cx, |state, cx| {
            state.filled.set_in(target, linear(MediaViewerVideoScrubberMetrics::GLIDE_DURATION), now, cx);
        });
        if self.painted.get() {
            self.start_glide(window, cx);
        }
    }

    fn start_glide(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.glide_running {
            return;
        }
        self.glide_running = true;
        Self::schedule_glide_swap(cx.weak_entity(), window);
    }

    fn schedule_glide_swap(weak: WeakEntity<Self>, window: &Window) {
        window.on_next_frame(move |window, cx| {
            let _ = weak.clone().update(cx, |scrubber, cx| scrubber.swap_glide(weak, window, cx));
        });
    }

    fn swap_glide(&mut self, weak: WeakEntity<Self>, window: &mut Window, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        let (filled, is_running, height, thumb_scale) = {
            let state = self.state.read(cx);
            (
                state.filled.value(now),
                state.filled.is_running(now),
                state.track_height.value(now),
                state.thumb_scale.value(now),
            )
        };
        let swapped = self.painted.get() && {
            let shapes = Self::shapes(filled, height, thumb_scale, Point::default());
            let base = cx.entity_id().as_u64();
            let mut found = true;
            for (key, path) in [(1, shapes.fill), (2, shapes.shadow), (3, shapes.thumb)] {
                found &= path.is_some_and(|path| window.swap_path(Self::swap_key(base, key), path));
            }
            found
        };
        if swapped && is_running {
            Self::schedule_glide_swap(weak, window);
            return;
        }
        self.glide_running = false;
        if !swapped && self.painted.get() {
            self.painted.set(false);
            cx.notify();
        }
    }

    fn swap_key(base: u64, shape: u64) -> u64 {
        base ^ (shape << 56)
    }

    fn shapes(filled: f32, height: f32, thumb_scale: f32, origin: Point<Pixels>) -> MediaViewerVideoScrubberShapes {
        let row_height = MediaViewerVideoScrubberMetrics::ROW_HEIGHT;
        let thumb_radius = MediaViewerVideoScrubberMetrics::THUMB_DIAMETER * thumb_scale / 2.0;
        let shadow_radius = thumb_radius + MediaViewerVideoScrubberMetrics::THUMB_SHADOW_SPREAD;
        let shadow_y = row_height / 2.0 + MediaViewerVideoScrubberMetrics::THUMB_SHADOW_OFFSET_Y;
        MediaViewerVideoScrubberShapes {
            fill: rounded_rect_path(0.0, (row_height - height) / 2.0, filled.max(0.5), height, origin),
            shadow: rounded_rect_path(
                filled - shadow_radius,
                shadow_y - shadow_radius,
                shadow_radius * 2.0,
                shadow_radius * 2.0,
                origin,
            ),
            thumb: rounded_rect_path(
                filled - thumb_radius,
                row_height / 2.0 - thumb_radius,
                thumb_radius * 2.0,
                thumb_radius * 2.0,
                origin,
            ),
        }
    }

    /// both labels reserve the width of the total
    /// otherwise the track ends jump when the playhead crosses 9:59 and a digit appears
    fn time_label(seconds: TimeInterval, total: TimeInterval, trailing: bool) -> impl IntoElement {
        div()
            .relative()
            .flex_none()
            .child(div().invisible().child(RiseFormat::duration_time_interval(total)))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .when(trailing, |this| this.right_0())
                    .when(!trailing, |this| this.left_0())
                    .child(RiseFormat::duration_time_interval(seconds)),
            )
    }

    fn scrub_fraction(at_x: f32, width: f32) -> f64 {
        let travel = (width - MediaViewerVideoScrubberMetrics::THUMB_DIAMETER).max(1.0);
        f64::from((at_x - MediaViewerVideoScrubberMetrics::THUMB_DIAMETER / 2.0) / travel).clamp(0.0, 1.0)
    }
}

impl Render for MediaViewerVideoScrubber {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (position, total, fraction, scrubbing) = {
            let controller = self.controller.read(cx);
            (
                controller.progress(),
                controller.duration(),
                controller.fraction_complete(),
                controller.is_scrubbing(),
            )
        };
        self.shown_labels = Self::labels(position, total);
        let state = self.state.clone();

        let now = RiseClock::monotonic();
        let width = f32::from(state.read(cx).track_bounds.size.width);
        let filled_target = Self::filled_target(width, fraction);
        let height_target = if scrubbing {
            MediaViewerVideoScrubberMetrics::SCRUBBING_TRACK_HEIGHT
        } else {
            MediaViewerVideoScrubberMetrics::TRACK_HEIGHT
        };
        let scale_target = if scrubbing { MediaViewerVideoScrubberMetrics::SCRUBBING_THUMB_SCALE } else { 1.0 };
        state.update(cx, |state, cx| {
            let glide = (!scrubbing).then(|| linear(MediaViewerVideoScrubberMetrics::GLIDE_DURATION));
            state.filled.set_in(filled_target, glide, now, cx);
            let grab = ease_out(MediaViewerVideoScrubberMetrics::GRAB_DURATION);
            state.track_height.set_in(height_target, grab, now, cx);
            state.thumb_scale.set_in(scale_target, grab, now, cx);
        });
        let (filled, filled_is_running, height, thumb_scale, hover_x) = {
            let state = state.read(cx);
            (
                state.filled.value(now),
                state.filled.is_running(now),
                state.track_height.frame_value(now, window),
                state.thumb_scale.frame_value(now, window),
                state.hover_x,
            )
        };
        if filled_is_running {
            self.start_glide(window, cx);
        }
        let row_height = MediaViewerVideoScrubberMetrics::ROW_HEIGHT;

        let hover_label = hover_x.filter(|_| total > 0.0 && width > 0.0).map(|x| {
            let seconds = total * Self::scrub_fraction(x, width);
            div()
                .absolute()
                .bottom(px(row_height - 4.0))
                .left(px(x))
                .child(
                    div()
                        .relative()
                        .left(px(-24.0))
                        .w(px(48.0))
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(6.0))
                                .bg(black().opacity(0.6))
                                .child(RiseFormat::duration_time_interval(seconds)),
                        ),
                )
        });

        let base_key = cx.entity_id().as_u64();
        let painted = Rc::clone(&self.painted);
        let shapes = canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                let shapes = Self::shapes(filled, height, thumb_scale, bounds.origin);
                let shadow = black().opacity(MediaViewerVideoScrubberMetrics::THUMB_SHADOW_OPACITY);
                let mut complete = true;
                for (key, path, color) in [(1, shapes.fill, white()), (2, shapes.shadow, shadow), (3, shapes.thumb, white())] {
                    match path {
                        Some(path) => {
                            window.paint_swappable_path(path, color, Self::swap_key(base_key, key), bounds.origin);
                        }
                        None => complete = false,
                    }
                }
                painted.set(complete);
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let pointer_state = state.clone();
        let pointer_controller = self.controller.clone();
        let track = div()
            .id("media-viewer-video-scrubber-track")
            .relative()
            .flex_1()
            .h(px(row_height))
            .cursor_pointer()
            .aria_label(format!(
                "{}, {} / {}",
                tr("progress_label"),
                RiseFormat::duration_time_interval(position),
                RiseFormat::duration_time_interval(total)
            ))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px((row_height - height) / 2.0))
                    .h(px(height))
                    .rounded_full()
                    .bg(white().opacity(0.3)),
            )
            .child(shapes)
            .children(hover_label)
            .child(measure("media-viewer-video-scrubber-bounds", {
                let state = state.clone();
                move |bounds, _, cx| {
                    state.update(cx, |state, cx| {
                        if state.track_bounds != bounds {
                            state.track_bounds = bounds;
                            cx.notify();
                        }
                    });
                }
            }))
            .child(canvas(
                |_, _, _| (),
                move |_, (), window, _| {
                    let state = pointer_state.clone();
                    let controller = pointer_controller.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble || !state.read(cx).is_pressed {
                            return;
                        }
                        let bounds = state.read(cx).track_bounds;
                        let fraction = Self::scrub_fraction(
                            f32::from(event.position.x - bounds.origin.x),
                            f32::from(bounds.size.width),
                        );
                        controller.update(cx, |controller, cx| {
                            controller.begin_scrubbing(cx);
                            controller.scrub(fraction, cx);
                        });
                    });
                    let state = pointer_state.clone();
                    let controller = pointer_controller.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble
                            || event.button != MouseButton::Left
                            || !state.read(cx).is_pressed
                        {
                            return;
                        }
                        let bounds = state.read(cx).track_bounds;
                        state.update(cx, |state, cx| {
                            state.is_pressed = false;
                            cx.notify();
                        });
                        let fraction = Self::scrub_fraction(
                            f32::from(event.position.x - bounds.origin.x),
                            f32::from(bounds.size.width),
                        );
                        controller.update(cx, |controller, cx| controller.end_scrubbing(fraction, cx));
                    });
                },
            ))
            .on_mouse_down(MouseButton::Left, {
                let state = state.clone();
                let controller = self.controller.clone();
                move |event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let bounds = state.read(cx).track_bounds;
                    state.update(cx, |state, cx| {
                        state.is_pressed = true;
                        cx.notify();
                    });
                    let fraction = Self::scrub_fraction(
                        f32::from(event.position.x - bounds.origin.x),
                        f32::from(bounds.size.width),
                    );
                    controller.update(cx, |controller, cx| {
                        controller.begin_scrubbing(cx);
                        controller.scrub(fraction, cx);
                    });
                }
            })
            .on_mouse_move({
                let state = state.clone();
                move |event: &MouseMoveEvent, _, cx| {
                    let bounds = state.read(cx).track_bounds;
                    let x = f32::from(event.position.x - bounds.origin.x);
                    state.update(cx, |state, cx| {
                        state.hover_x = Some(x.clamp(0.0, f32::from(bounds.size.width)));
                        cx.notify();
                    });
                }
            })
            .on_hover({
                let state = state.clone();
                move |hovered: &bool, _, cx| {
                    if *hovered {
                        return;
                    }
                    state.update(cx, |state, cx| {
                        state.hover_x = None;
                        cx.notify();
                    });
                }
            });

        div()
            .size_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(MediaViewerVideoScrubberMetrics::LABEL_SPACING))
            .text_size(px(14.0))
            .font_weight(FontWeight::MEDIUM)
            .font_features(FontFeatures(Arc::new(vec![("tnum".to_string(), 1)])))
            .text_color(white())
            .px(px(14.0))
            .py(px(MediaViewerVideoScrubberMetrics::VERTICAL_PADDING))
            .child(Self::time_label(position, total, false))
            .child(track)
            .child(Self::time_label(total, total, true))
            .liquid_chrome_capsule_with(0.12, true, Some(0.86), false)
    }
}
