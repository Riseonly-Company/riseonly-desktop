use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Bounds, BoxShadow, Context, Font, FontWeight, Hsla, IntoElement, Pixels, Point,
    Render, SharedString, Size, Task, TextRun, Window, black, font, point, px, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::haptics::SelectionFeedbackGenerator;
use rise_theme::FONT_FAMILY;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::minimum_scale_factor::MinimumScaleText;
use rise_ui::swiftui::motion::{ease_in_out, ease_out};
use rise_ui::swiftui::prelude::*;

pub enum MediaPickerDateScrubberLayout {}

impl MediaPickerDateScrubberLayout {
    pub const LEGACY_CAPSULE_HEIGHT: f32 = 44.0;
    pub const LEGACY_MINIMUM_CAPSULE_WIDTH: f32 = 104.0;
    pub const CAPSULE_HEIGHT: f32 = Self::LEGACY_CAPSULE_HEIGHT / 2.0;
    pub const MINIMUM_CAPSULE_WIDTH: f32 = Self::LEGACY_MINIMUM_CAPSULE_WIDTH / 2.0;
    pub const MAXIMUM_CAPSULE_WIDTH: f32 = 96.0;
    pub const HORIZONTAL_LABEL_INSET: f32 = 7.0;
    pub const HORIZONTAL_CONTENT_PADDING: f32 = Self::HORIZONTAL_LABEL_INSET * 2.0;
    pub const LABEL_FONT_SIZE: f32 = 11.0;
    pub const MINIMUM_TOUCH_DIMENSION: f32 = 44.0;
    pub const CAPSULE_HIT_OUTSET: f32 = (Self::MINIMUM_TOUCH_DIMENSION - Self::CAPSULE_HEIGHT) / 2.0;

    /// handle touches are the date capsule and the rail knob beside it, each grown to a 44pt target
    /// not the whole trailing strip
    /// that made any pan down the right edge start a scrub and block the last column
    pub fn is_inside_handle(
        point: Point<Pixels>,
        capsule_frame: Bounds<Pixels>,
        indicator_frame: Bounds<Pixels>,
    ) -> bool {
        if Self::capsule_hit_frame(capsule_frame).contains(&point) {
            return true;
        }
        Self::indicator_hit_frame(indicator_frame).contains(&point)
    }

    pub fn capsule_width(measured_label_width: f32, available_width: f32) -> f32 {
        let bounded_maximum = Self::MINIMUM_CAPSULE_WIDTH.max(Self::MAXIMUM_CAPSULE_WIDTH.min(available_width));
        bounded_maximum.min(Self::MINIMUM_CAPSULE_WIDTH.max(measured_label_width.ceil() + Self::HORIZONTAL_CONTENT_PADDING))
    }

    fn capsule_hit_frame(capsule_frame: Bounds<Pixels>) -> Bounds<Pixels> {
        outset_by(capsule_frame, Self::CAPSULE_HIT_OUTSET, Self::CAPSULE_HIT_OUTSET)
    }

    fn indicator_hit_frame(indicator_frame: Bounds<Pixels>) -> Bounds<Pixels> {
        let indicator_outset = ((Self::MINIMUM_TOUCH_DIMENSION - f32::from(indicator_frame.size.width)) / 2.0).max(0.0);
        outset_by(indicator_frame, indicator_outset, 0.0)
    }
}

/// grows a frame by dx and dy on each side
fn outset_by(frame: Bounds<Pixels>, dx: f32, dy: f32) -> Bounds<Pixels> {
    Bounds {
        origin: point(frame.origin.x - px(dx), frame.origin.y - px(dy)),
        size: size(frame.size.width + px(dx * 2.0), frame.size.height + px(dy * 2.0)),
    }
}

/// pan state with the vertical translation as the scrubber reads it
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MediaPickerDateScrubberPan {
    Began,
    Changed(f32),
    Ended,
}

pub type MediaPickerDateScrubberAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type MediaPickerDateScrubberDelta = Rc<dyn Fn(f32, &mut Window, &mut App)>;

const PAN_HYSTERESIS: f32 = 10.0;

const FADED_ALPHA: f32 = 0.03;

const HIDE_DELAY_SECONDS: f64 = 2.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MediaPickerDateScrubberPanPhase {
    Possible,
    Tracking,
    Failed,
}

pub struct MediaPickerDateScrubber {
    pub on_scrub_began: Option<MediaPickerDateScrubberAction>,
    pub on_scrub_delta: Option<MediaPickerDateScrubberDelta>,
    pub on_scrub_ended: Option<MediaPickerDateScrubberAction>,

    feedback: SelectionFeedbackGenerator,
    fraction: f32,
    top_inset: f32,
    bottom_inset: f32,
    is_dragging_indicator: bool,
    is_available: bool,
    hide_work_item: Option<Task<()>>,
    current_date_text: String,
    alpha: Animated<f32>,
    bounds_size: Size<Pixels>,
    capsule_frame: Bounds<Pixels>,
    line_indicator_frame: Bounds<Pixels>,
    measured_label_width: Option<f32>,
    pan_phase: Rc<Cell<MediaPickerDateScrubberPanPhase>>,
}

impl MediaPickerDateScrubber {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            on_scrub_began: None,
            on_scrub_delta: None,
            on_scrub_ended: None,
            feedback: SelectionFeedbackGenerator,
            fraction: 0.0,
            top_inset: 8.0,
            bottom_inset: 8.0,
            is_dragging_indicator: false,
            is_available: false,
            hide_work_item: None,
            current_date_text: String::new(),
            alpha: Animated::new(0.0),
            bounds_size: Size::default(),
            capsule_frame: Bounds::default(),
            line_indicator_frame: Bounds::default(),
            measured_label_width: None,
            pan_phase: Rc::new(Cell::new(MediaPickerDateScrubberPanPhase::Possible)),
        }
    }

    /// hit test in the scrubbers own space
    pub fn point_inside(&self, point: Point<Pixels>) -> bool {
        if !self.is_available || *self.alpha.target() <= FADED_ALPHA {
            return false;
        }
        MediaPickerDateScrubberLayout::is_inside_handle(point, self.capsule_frame, self.line_indicator_frame)
    }

    /// handle fades two seconds after a scroll but a drag starting where it rests still means to grab it
    /// else a scrub down couldnt be followed by one back up
    pub fn is_on_faded_handle(&self, point: Point<Pixels>) -> bool {
        if !self.is_available || *self.alpha.target() > FADED_ALPHA {
            return false;
        }
        MediaPickerDateScrubberLayout::is_inside_handle(point, self.capsule_frame, self.line_indicator_frame)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        fraction: f32,
        date_text: String,
        top_inset: f32,
        bottom_inset: f32,
        is_available: bool,
        is_active: bool,
        cx: &mut Context<Self>,
    ) {
        let drawn_before = (self.indicator_line_y_half_points(), self.top_inset, self.bottom_inset, self.is_available);
        self.fraction = fraction.clamp(0.0, 1.0);
        self.top_inset = top_inset;
        self.bottom_inset = bottom_inset;
        let was_available = self.is_available;
        self.is_available = is_available;

        let mut text_changed = false;
        if self.current_date_text != date_text {
            text_changed = true;
            if self.is_dragging_indicator && !self.current_date_text.is_empty() && !date_text.is_empty() {
                self.feedback.selection_changed();
                self.feedback.prepare();
            }
            self.current_date_text = date_text;
            self.measured_label_width = None;
        }
        if !is_available {
            if was_available || *self.alpha.target() > 0.0 {
                self.hide(false, cx);
                cx.notify();
            }
            return;
        }
        if is_active || self.is_dragging_indicator {
            self.show(cx);
            self.schedule_hide(HIDE_DELAY_SECONDS, cx);
        }
        let drawn_after = (self.indicator_line_y_half_points(), self.top_inset, self.bottom_inset, self.is_available);
        if text_changed || drawn_before != drawn_after {
            cx.notify();
        }
    }

    /// handles resting line y in half points, one device pixel on a 2x screen
    fn indicator_line_y_half_points(&self) -> i64 {
        let bounds_height = f32::from(self.bounds_size.height);
        let line_height: f32 = if self.is_dragging_indicator { 50.0 } else { 44.0 };
        let minimum_y = self.top_inset + 4.0;
        let maximum_y = minimum_y.max(bounds_height - self.bottom_inset - line_height - 4.0);
        let line_y = minimum_y + (maximum_y - minimum_y) * self.fraction;
        (line_y * 2.0).round() as i64
    }

    pub fn note_activity(&mut self, cx: &mut Context<Self>) {
        if !self.is_available {
            return;
        }
        self.show(cx);
        self.schedule_hide(HIDE_DELAY_SECONDS, cx);
    }

    pub fn hide_soon(&mut self, cx: &mut Context<Self>) {
        self.schedule_hide(HIDE_DELAY_SECONDS, cx);
    }

    /// runs from render because thats where the windows text system is at hand
    fn layout_indicator(&mut self, window: &Window) {
        let bounds_width = f32::from(self.bounds_size.width);
        let bounds_height = f32::from(self.bounds_size.height);
        if bounds_width <= 0.0 || bounds_height <= 0.0 {
            return;
        }
        let line_height: f32 = if self.is_dragging_indicator { 50.0 } else { 44.0 };
        let capsule_height = MediaPickerDateScrubberLayout::CAPSULE_HEIGHT;
        let minimum_y = self.top_inset + 4.0;
        let maximum_y = minimum_y.max(bounds_height - self.bottom_inset - line_height - 4.0);
        let line_y = minimum_y + (maximum_y - minimum_y) * self.fraction;

        self.line_indicator_frame = Bounds {
            origin: point(
                px(bounds_width - if self.is_dragging_indicator { 8.0 } else { 6.0 }),
                px(line_y),
            ),
            size: size(px(if self.is_dragging_indicator { 6.0 } else { 4.0 }), px(line_height)),
        };

        let available_width = MediaPickerDateScrubberLayout::MINIMUM_CAPSULE_WIDTH.max(bounds_width - 42.0);
        let measured_width = self.size_that_fits_label(window);
        let capsule_width = MediaPickerDateScrubberLayout::capsule_width(measured_width, available_width);
        let capsule_y = line_y + ((line_height - capsule_height) * 0.5).floor();
        self.capsule_frame = Bounds {
            origin: point(px(bounds_width - 18.0 - capsule_width), px(capsule_y)),
            size: size(px(capsule_width), px(capsule_height)),
        };
    }

    fn size_that_fits_label(&mut self, window: &Window) -> f32 {
        if let Some(measured) = self.measured_label_width {
            return measured;
        }
        let measured = if self.current_date_text.is_empty() {
            0.0
        } else {
            let run = TextRun {
                len: self.current_date_text.len(),
                font: Font { weight: FontWeight::SEMIBOLD, ..font(FONT_FAMILY) },
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            f32::from(
                window
                    .text_system()
                    .shape_line(
                        SharedString::from(self.current_date_text.clone()),
                        px(MediaPickerDateScrubberLayout::LABEL_FONT_SIZE),
                        &[run],
                        None,
                    )
                    .width(),
            )
        };
        self.measured_label_width = Some(measured);
        measured
    }

    /// also driven by a pan living outside this view where the host cant take touches
    pub fn handle_scrub_pan(&mut self, pan: MediaPickerDateScrubberPan, window: &mut Window, cx: &mut Context<Self>) {
        match pan {
            MediaPickerDateScrubberPan::Began => {
                self.hide_work_item = None;
                self.is_dragging_indicator = true;
                self.feedback.prepare();
                self.feedback.selection_changed();
                if let Some(on_scrub_began) = self.on_scrub_began.clone() {
                    on_scrub_began(window, cx);
                }
                self.show(cx);
                cx.notify();
            }
            MediaPickerDateScrubberPan::Changed(translation_y) => {
                if let Some(on_scrub_delta) = self.on_scrub_delta.clone() {
                    on_scrub_delta(translation_y, window, cx);
                }
            }
            MediaPickerDateScrubberPan::Ended => {
                self.is_dragging_indicator = false;
                if let Some(on_scrub_ended) = self.on_scrub_ended.clone() {
                    on_scrub_ended(window, cx);
                }
                cx.notify();
                self.hide_soon(cx);
            }
        }
    }

    fn show(&mut self, cx: &mut Context<Self>) {
        if !self.is_available {
            return;
        }
        self.hide_work_item = None;
        if *self.alpha.target() >= 0.999 {
            return;
        }
        self.alpha.set_in(1.0, ease_out(0.18), RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn schedule_hide(&mut self, delay: f64, cx: &mut Context<Self>) {
        self.hide_work_item = None;
        if self.is_dragging_indicator {
            return;
        }
        self.hide_work_item = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            let _ = this.update(cx, |this, cx| this.hide(true, cx));
        }));
    }

    fn hide(&mut self, animated: bool, cx: &mut Context<Self>) {
        self.hide_work_item = None;
        if self.is_dragging_indicator {
            return;
        }
        if animated {
            self.alpha.set_in(0.0, ease_in_out(0.25), RiseClock::monotonic(), cx);
        } else {
            self.alpha.jump(0.0);
        }
        cx.notify();
    }

    fn gesture_recognizer_should_begin(translation: Size<Pixels>) -> bool {
        translation.height.abs() >= translation.width.abs()
    }

    fn pan_gesture(&self, cx: &mut Context<Self>) -> DragGesture {
        let changed_scrubber = cx.weak_entity();
        let changed_phase = self.pan_phase.clone();
        let ended_scrubber = cx.weak_entity();
        let ended_phase = self.pan_phase.clone();
        DragGesture::new(PAN_HYSTERESIS)
            .on_changed(move |value, window, cx| {
                let pans: &[MediaPickerDateScrubberPan] = match changed_phase.get() {
                    MediaPickerDateScrubberPanPhase::Failed => return,
                    MediaPickerDateScrubberPanPhase::Tracking => &[],
                    MediaPickerDateScrubberPanPhase::Possible => {
                        let receives = changed_scrubber
                            .read_with(cx, |scrubber, _| scrubber.point_inside(value.start_location))
                            .unwrap_or(false);
                        if !receives || !Self::gesture_recognizer_should_begin(value.translation) {
                            changed_phase.set(MediaPickerDateScrubberPanPhase::Failed);
                            return;
                        }
                        changed_phase.set(MediaPickerDateScrubberPanPhase::Tracking);
                        &[MediaPickerDateScrubberPan::Began]
                    }
                };
                let translation_y = f32::from(value.translation.height);
                let _ = changed_scrubber.update(cx, |scrubber, cx| {
                    for pan in pans {
                        scrubber.handle_scrub_pan(*pan, window, cx);
                    }
                    scrubber.handle_scrub_pan(MediaPickerDateScrubberPan::Changed(translation_y), window, cx);
                });
            })
            .on_ended(move |_, window, cx| {
                let was_tracking = ended_phase.get() == MediaPickerDateScrubberPanPhase::Tracking;
                ended_phase.set(MediaPickerDateScrubberPanPhase::Possible);
                if was_tracking {
                    let _ = ended_scrubber.update(cx, |scrubber, cx| {
                        scrubber.handle_scrub_pan(MediaPickerDateScrubberPan::Ended, window, cx);
                    });
                }
            })
    }

    /// absolute frame in the scrubbers own space
    fn placed(frame: Bounds<Pixels>) -> Div {
        div()
            .absolute()
            .left(frame.origin.x)
            .top(frame.origin.y)
            .w(frame.size.width)
            .h(frame.size.height)
    }

    /// one transparent target of is_inside_handle
    /// press stops here so the photo under the handle doesnt take it, wheel still reaches the grid
    fn handle_target(id: &'static str, frame: Bounds<Pixels>, bounds: Bounds<Pixels>) -> impl IntoElement {
        Self::placed(frame.intersect(&bounds)).id(id).block_mouse_except_scroll()
    }

    fn capsule_container(&self) -> impl IntoElement {
        let capsule_width = f32::from(self.capsule_frame.size.width);
        let capsule_height = MediaPickerDateScrubberLayout::CAPSULE_HEIGHT;
        let capsule_effect_content = div()
            .relative()
            .w(px(capsule_width))
            .h(px(capsule_height))
            .child(div().absolute().inset_0().bg(black().opacity(0.20)))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .padding_edges(EdgeSet::HORIZONTAL, MediaPickerDateScrubberLayout::HORIZONTAL_LABEL_INSET)
                    .flex()
                    .items_center()
                    .text_size(px(MediaPickerDateScrubberLayout::LABEL_FONT_SIZE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(white())
                    .child(
                        div()
                            .w_full()
                            .text_center()
                            .child(MinimumScaleText::new(self.current_date_text.clone(), 0.78).line_limit(1)),
                    ),
            );
        Self::placed(self.capsule_frame)
            .rounded_full()
            .shadow(vec![BoxShadow {
                color: black().opacity(0.24),
                offset: point(px(0.0), px(1.5)),
                blur_radius: px(4.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(capsule_effect_content.liquid_chrome_capsule_with(0.12, false, None, false))
    }

    fn line_indicator(&self) -> impl IntoElement {
        Self::placed(self.line_indicator_frame)
            .rounded(self.line_indicator_frame.size.width * 0.5)
            .bg(white().opacity(0.82))
            .shadow(vec![BoxShadow {
                color: black().opacity(0.24),
                offset: point(px(0.0), px(1.0)),
                blur_radius: px(2.0),
                spread_radius: px(0.0),
                inset: false,
            }])
    }
}

impl Render for MediaPickerDateScrubber {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.layout_indicator(window);
        let alpha = self.alpha.frame_value(RiseClock::monotonic(), window);
        let bounds_size = self.bounds_size;
        let scrubber = cx.weak_entity();
        let measured = measure("media-picker-date-scrubber-bounds", move |bounds, _, cx| {
            let _ = scrubber.update(cx, |scrubber, cx| {
                if scrubber.bounds_size != bounds.size {
                    scrubber.bounds_size = bounds.size;
                    cx.notify();
                }
            });
        });

        let mut root = div().relative().size_full().opacity(alpha).child(measured);
        if self.is_available && bounds_size.width > px(0.0) {
            root = root.child(self.capsule_container()).child(self.line_indicator());
        }
        if self.is_available && *self.alpha.target() > FADED_ALPHA && bounds_size.width > px(0.0) {
            let local_bounds = Bounds { origin: Point::default(), size: bounds_size };
            root = root
                .child(Self::handle_target(
                    "media-picker-date-scrubber-capsule",
                    MediaPickerDateScrubberLayout::capsule_hit_frame(self.capsule_frame),
                    local_bounds,
                ))
                .child(Self::handle_target(
                    "media-picker-date-scrubber-knob",
                    MediaPickerDateScrubberLayout::indicator_hit_frame(self.line_indicator_frame),
                    local_bounds,
                ))
                .child(
                    div().absolute().inset_0().child(
                        div()
                            .w(bounds_size.width)
                            .h(bounds_size.height)
                            .gesture("media-picker-date-scrubber-pan", self.pan_gesture(cx)),
                    ),
                );
        }
        root
    }
}
