use std::f32::consts::PI;
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, BoxShadow, Bounds, Context, ElementId, Entity, Render, Subscription, Hsla, InteractiveElement, IntoElement, ParentElement, PathBuilder,
    Pixels, Point, RenderOnce, SharedString, Size, StatefulInteractiveElement, Styled, Window, black, canvas, div, fill,
    point, px, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::IconUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::motion::{ease_out, linear, spring};
use rise_ui::swiftui::transition::Transition;

use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::widgets::notifier::notifier_host::NotifierWindowPresenter;
use crate::modules::chat::components::chat::video_note::video_note_player_layer_view::VideoNotePlayerLayerView;
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;

const EDGE_PADDING: f32 = 12.0;
const TAP_DISTANCE: f32 = 8.0;
const SPRING_RESPONSE: f64 = 0.36;
const SPRING_DAMPING_FRACTION: f64 = 0.78;
const PROGRESS_LINE_WIDTH: f32 = 3.0;
const PROGRESS_MOTION_DURATION: f64 = 0.05;
const BADGE_SIZE: f32 = 30.0;
const BADGE_OPACITY: f32 = 0.45;
const BADGE_ICON_SIZE: f32 = 12.0;
const PLAY_ICON_OFFSET_X: f32 = 1.0;
const SHADOW_OPACITY: f32 = 0.3;
const SHADOW_RADIUS: f32 = 10.0;
const SHADOW_OFFSET_Y: f32 = 4.0;
const INITIAL_BOTTOM_CLEARANCE: f32 = 80.0;
const SNAP_BOTTOM_CLEARANCE: f32 = 60.0;
const DISMISS_DURATION: f64 = 0.22;
const DISMISS_STOP_DELAY: Duration = Duration::from_millis(200);
const DECODE_SCALE: f32 = 3.0;
const TRANSITION_SCALE: f32 = 0.3;

struct VideoNoteFloatingPipState {
    position: Animated<Point<Pixels>>,
    drag_offset: Animated<Size<Pixels>>,
    is_positioned: bool,
    is_dismissing: bool,
    host: Bounds<Pixels>,
    container: Size<Pixels>,
    pip_size: Animated<f32>,
    progress: Animated<f32>,
    is_hovered: bool,
}

impl VideoNoteFloatingPipState {
    fn dismiss_direction(&self, value: &DragGestureValue, size: Size<Pixels>) -> Option<i32> {
        if self.is_dismissing {
            return None;
        }
        let direction = VideoNoteFloatingPip::fling_direction(value.predicted_end_translation, value.velocity, size)?;
        let carried = f32::from(value.translation.width).abs();
        let released_x = f32::from(self.position.target().x + value.translation.width);
        let off_column = released_x < 0.0 || released_x > f32::from(size.width);
        if carried > f32::from(size.width) * VideoNoteFloatingPip::DISMISS_CARRY_FRACTION && !off_column {
            return None;
        }
        Some(direction)
    }
}

/// floating circle for the note playing after its bubble scrolls off
/// fills the chat column so drags stay inside
/// left drags and flings, hover shows pause badge
#[derive(IntoElement)]
pub struct VideoNoteFloatingPip {
    id: ElementId,
    area: Option<Bounds<Pixels>>,
}

impl VideoNoteFloatingPip {
    pub const PIP_SIZE_PAUSED: f32 = 110.0;
    pub const PIP_SIZE_PLAYING: f32 = 138.0;

    pub const DISMISS_TRAVEL_FRACTION: f32 = 0.32;
    pub const DISMISS_MINIMUM_TRAVEL: f32 = 110.0;
    pub const DISMISS_HORIZONTAL_DOMINANCE: f32 = 1.25;
    pub const DISMISS_MINIMUM_SPEED: f32 = 600.0;
    pub const DISMISS_CARRY_FRACTION: f32 = 0.5;
    pub const SNAP_TOP_INSET: f32 = 200.0;

    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), area: None }
    }

    /// window bounds of the chat column the pip floats over, none uses the whole host
    pub fn area(mut self, area: Option<Bounds<Pixels>>) -> Self {
        self.area = area.filter(|area| area.size.width > Pixels::ZERO && area.size.height > Pixels::ZERO);
        self
    }

    /// transition of the circle for the presenter presence
    pub fn transition() -> Transition {
        Transition::asymmetric(
            Transition::scale(TRANSITION_SCALE).combined(Transition::opacity()),
            Transition::scale(TRANSITION_SCALE).combined(Transition::opacity()),
        )
    }

    /// pip size, depends on whether its playing
    pub fn pip_size(is_playing: bool) -> f32 {
        if is_playing { Self::PIP_SIZE_PLAYING } else { Self::PIP_SIZE_PAUSED }
    }

    pub fn fling_direction(predicted: Size<Pixels>, velocity: Size<Pixels>, size: Size<Pixels>) -> Option<i32> {
        if f32::from(velocity.width).abs() < Self::DISMISS_MINIMUM_SPEED {
            return None;
        }
        let width = f32::from(predicted.width);
        let travel = width.abs();
        let threshold = Self::DISMISS_MINIMUM_TRAVEL.max(f32::from(size.width) * Self::DISMISS_TRAVEL_FRACTION);
        if travel < threshold || travel < f32::from(predicted.height).abs() * Self::DISMISS_HORIZONTAL_DOMINANCE {
            return None;
        }
        Some(if width < 0.0 { -1 } else { 1 })
    }

    /// snaps to the nearest corner
    pub fn snapped(point_in: Point<Pixels>, size: Size<Pixels>) -> Point<Pixels> {
        let half = Self::PIP_SIZE_PLAYING / 2.0;
        let left = half + EDGE_PADDING;
        let right = f32::from(size.width) - half - EDGE_PADDING;
        let bottom = f32::from(size.height) - half - EDGE_PADDING - SNAP_BOTTOM_CLEARANCE;
        let top = bottom.min(Self::SNAP_TOP_INSET + half);

        let x = f32::from(point_in.x);
        let y = f32::from(point_in.y);
        let snapped_x = if (x - left).abs() < (x - right).abs() { left } else { right };
        let snapped_y = if (y - top).abs() < (y - bottom).abs() { top } else { bottom };

        point(px(snapped_x), px(snapped_y))
    }

    pub fn initial_position(size: Size<Pixels>) -> Point<Pixels> {
        let half = Self::PIP_SIZE_PLAYING / 2.0;
        point(
            size.width - px(half + EDGE_PADDING),
            size.height - px(half + EDGE_PADDING + INITIAL_BOTTOM_CLEARANCE),
        )
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(name))
    }
}

/// pip as its own view, player notifies on every time tick and only the circle redraws
/// floats over the chat column the root reports
pub struct VideoNoteFloatingPipView {
    column: Option<Bounds<Pixels>>,
    _player_observation: Subscription,
    _column_observation: Subscription,
}

impl VideoNoteFloatingPipView {
    pub fn new(cx: &mut App) -> Entity<Self> {
        let player = VideoNotePlayerStore::shared(cx);
        let presenter = NotifierWindowPresenter::shared(cx);
        let column = presenter.read(cx).centre_column();
        cx.new(|cx| Self {
            column,
            _player_observation: cx.observe(&player, |_: &mut Self, _, cx| cx.notify()),
            _column_observation: cx.observe(&presenter, |view: &mut Self, presenter, cx| {
                let column = presenter.read(cx).centre_column();
                if column != view.column {
                    view.column = column;
                    cx.notify();
                }
            }),
        })
    }
}

impl Render for VideoNoteFloatingPipView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        VideoNoteFloatingPip::new("root-video-note-floating-pip").area(self.column)
    }
}

impl RenderOnce for VideoNoteFloatingPip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let store = VideoNotePlayerStore::shared(cx);
        let (is_active, is_playing, player, display_epoch, progress, thumbnail_url) = {
            let store = store.read(cx);
            (
                store.is_active(),
                store.is_playing(),
                store.player(),
                store.display_epoch(),
                store.progress(),
                store.thumbnail_url().map(str::to_owned),
            )
        };
        let target_size = Self::pip_size(is_playing);

        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| VideoNoteFloatingPipState {
            position: Animated::new(Point::default()),
            drag_offset: Animated::new(Size::default()),
            is_positioned: false,
            is_dismissing: false,
            host: Bounds::default(),
            container: Size::default(),
            pip_size: Animated::new(target_size),
            progress: Animated::new(progress),
            is_hovered: false,
        });
        let now = RiseClock::monotonic();
        let area = self.area;
        let (center, pip_size, shown_progress, is_hovered, area_origin, area_size) = state.update(cx, |state, cx| {
            let area = area.unwrap_or(state.host);
            if area.size != state.container && area.size.width > Pixels::ZERO && area.size.height > Pixels::ZERO {
                state.container = area.size;
                if state.is_positioned && !state.is_dismissing {
                    let moved = Self::snapped(*state.position.target(), area.size);
                    state.position.jump(moved);
                }
            }
            if !state.is_positioned && state.container.width > Pixels::ZERO && state.container.height > Pixels::ZERO {
                state.position.jump(Self::initial_position(state.container));
                state.is_positioned = true;
            }
            state.pip_size.set_in(target_size, spring(SPRING_RESPONSE, SPRING_DAMPING_FRACTION), now, cx);
            state.progress.set_in(progress, linear(PROGRESS_MOTION_DURATION), now, cx);
            let position = state.position.frame_value(now, window);
            let offset = state.drag_offset.frame_value(now, window);
            (
                point(position.x + offset.width, position.y + offset.height),
                state.pip_size.frame_value(now, window),
                state.progress.frame_value(now, window),
                state.is_hovered,
                point(area.origin.x - state.host.origin.x, area.origin.y - state.host.origin.y),
                area.size,
            )
        });

        let content = if is_active {
            VideoNotePlayerLayerView::new(player, display_epoch).into_any_element()
        } else if let Some(thumb) = thumbnail_url {
            CleverImageUi::new(thumb)
                .content_mode(ContentMode::Fill)
                .corner_radius(pip_size / 2.0)
                .persist_to_disk(true)
                .maximum_decode_pixel_dimension(((pip_size * DECODE_SCALE).ceil() as i64).max(1))
                .into_any_element()
        } else {
            div().size_full().rounded_full().bg(black()).into_any_element()
        };

        let progress_color = theme.primary._100;
        let badge_symbol = if !is_playing {
            Some(("play.fill", PLAY_ICON_OFFSET_X))
        } else if is_hovered {
            Some(("pause.fill", 0.0))
        } else {
            None
        };

        let measure_state = state.clone();
        let hover_state = state.clone();
        let changed_state = state.clone();
        let ended_state = state.clone();
        let tap_store = store.clone();
        let dismiss_store = store.clone();

        let circle = div()
            .id(self.child_id("circle"))
            .size(px(pip_size))
            .rounded_full()
            .cursor_pointer()
            .occlude()
            .shadow(vec![BoxShadow {
                color: black().opacity(SHADOW_OPACITY),
                offset: point(Pixels::ZERO, px(SHADOW_OFFSET_Y)),
                blur_radius: px(SHADOW_RADIUS),
                spread_radius: Pixels::ZERO,
                inset: false,
            }])
            .on_hover(move |hovered, _, cx| {
                hover_state.update(cx, |state, cx| {
                    state.is_hovered = *hovered;
                    cx.notify();
                });
            })
            .child(div().absolute().inset_0().rounded_full().overflow_hidden().child(content))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, _| {
                        paint_progress_trim(bounds, shown_progress, PROGRESS_LINE_WIDTH, progress_color, window);
                    },
                )
                .absolute()
                .inset_0(),
            )
            .when_some(badge_symbol, |circle, (symbol, offset_x)| {
                circle.child(
                    div().absolute().inset_0().flex().items_center().justify_center().child(
                        div()
                            .size(px(BADGE_SIZE))
                            .rounded_full()
                            .bg(black().opacity(BADGE_OPACITY))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .relative()
                                    .left(px(offset_x))
                                    .children(IconUi::sized(symbol, px(BADGE_ICON_SIZE), white(), true)),
                            ),
                    ),
                )
            })
            .gesture(
                self.child_id("drag"),
                DragGesture::new(0.0)
                    .on_changed(move |value, _, cx| {
                        changed_state.update(cx, |state, cx| {
                            if state.is_dismissing {
                                return;
                            }
                            state.drag_offset.jump(value.translation);
                            cx.notify();
                        });
                    })
                    .on_ended(move |value, _, cx| {
                        let translation = value.translation;
                        let distance = f32::from(translation.width).hypot(f32::from(translation.height));
                        if distance < TAP_DISTANCE {
                            tap_store.update(cx, |store, cx| store.pause_or_resume(cx));
                            ended_state.update(cx, |state, cx| {
                                state.drag_offset.jump(Size::default());
                                cx.notify();
                            });
                            return;
                        }
                        let now = RiseClock::monotonic();
                        let direction = ended_state.read(cx).dismiss_direction(value, ended_state.read(cx).container);
                        if let Some(direction) = direction {
                            ended_state.update(cx, |state, cx| {
                                state.is_dismissing = true;
                                let current_size = *state.pip_size.target();
                                let offscreen_x = if direction < 0 {
                                    -(current_size + EDGE_PADDING)
                                } else {
                                    f32::from(state.container.width) + current_size + EDGE_PADDING
                                };
                                let target = size(px(offscreen_x) - state.position.target().x, translation.height);
                                state.drag_offset.set_in(target, ease_out(DISMISS_DURATION), now, cx);
                                cx.notify();
                            });
                            let fly_state = ended_state.downgrade();
                            let dismiss_store = dismiss_store.clone();
                            cx.spawn(async move |cx| {
                                cx.background_executor().timer(DISMISS_STOP_DELAY).await;
                                cx.update(|cx| {
                                    dismiss_store.update(cx, |store, cx| store.stop(cx));
                                    fly_state
                                        .update(cx, |state, cx| {
                                            state.drag_offset.jump(Size::default());
                                            state.is_dismissing = false;
                                            cx.notify();
                                        })
                                        .ok();
                                });
                            })
                            .detach();
                            return;
                        }
                        ended_state.update(cx, |state, cx| {
                            let origin = *state.position.target();
                            let candidate = point(origin.x + translation.width, origin.y + translation.height);
                            let snapped = Self::snapped(candidate, state.container);
                            state.position.jump(candidate);
                            state.drag_offset.jump(Size::default());
                            state.position.set_in(snapped, spring(SPRING_RESPONSE, SPRING_DAMPING_FRACTION), now, cx);
                            cx.notify();
                        });
                    }),
            );
        let placed_circle = div()
            .absolute()
            .left(center.x - px(pip_size / 2.0))
            .top(center.y - px(pip_size / 2.0))
            .size(px(pip_size))
            .child(circle);

        let is_positioned = state.read(cx).is_positioned;
        div()
            .relative()
            .size_full()
            .child(
                canvas(
                    move |bounds, _, cx| {
                        measure_state.update(cx, |state, cx| {
                            if state.host != bounds {
                                state.host = bounds;
                                cx.notify();
                            }
                        });
                    },
                    |_, (), _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .when(is_positioned, |root| {
                root.child(
                    div()
                        .absolute()
                        .left(area_origin.x)
                        .top(area_origin.y)
                        .w(area_size.width)
                        .h(area_size.height)
                        .child(placed_circle),
                )
            })
    }
}

/// arc trimmed to progress with round caps, starts at twelve oclock and runs clockwise
/// the playback bar draws its ring with it too
pub(crate) fn paint_progress_trim(bounds: Bounds<Pixels>, progress: f32, line_width: f32, color: Hsla, window: &mut Window) {
    let fraction = progress.clamp(0.0, 1.0);
    if fraction <= 0.0 {
        return;
    }
    let center = bounds.center();
    let radius = f32::from(bounds.size.width.min(bounds.size.height)) / 2.0;
    let sweep = 2.0 * PI * fraction;
    let start = -PI / 2.0;
    let steps = ((fraction * 96.0).ceil() as usize).max(2);
    let points: Vec<Point<Pixels>> = (0..=steps)
        .map(|step| {
            let angle = start + sweep * step as f32 / steps as f32;
            point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
        })
        .collect();
    let mut builder = PathBuilder::stroke(px(line_width));
    builder.add_polygon(&points, false);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
    let half = line_width / 2.0;
    for end in [points.first(), points.last()].into_iter().flatten() {
        let cap = Bounds::new(point(end.x - px(half), end.y - px(half)), size(px(line_width), px(line_width)));
        window.paint_quad(fill(cap, color).corner_radii(px(half)));
    }
}
