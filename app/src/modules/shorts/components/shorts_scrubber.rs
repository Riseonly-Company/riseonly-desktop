use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, BoxShadow, Context, ElementId, Entity, IntoElement, ParentElement, Path, Pixels, Point, Render,
    StyleRefinement, Styled, Subscription, Window, black, canvas, div, point, px, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::haptics::selection_changed;
use rise_theme::alpha;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::swiftui::transition::{Presence, Transition};

use crate::core::media_viewer::media_viewer_video_scrubber::rounded_rect_path;
use crate::modules::shorts::components::shorts_feed_player::{ShortsPlaybackController, ShortsPlaybackTimeChanged};

const IDLE_BAR_HEIGHT: f32 = 3.0;
const ACTIVE_BAR_HEIGHT: f32 = 6.0;
const THUMB_SIDE: f32 = 14.0;
const SCRUBBER_HEIGHT: f32 = 22.0;
const FILL_STEP_POINTS: f32 = 0.5;

/// the scrubber as its own view, owned by the cell state
pub struct ShortsScrubber {
    id: ElementId,
    playback: Entity<ShortsPlaybackController>,
    drag_fraction: Option<f64>,
    width: f32,
    bar_height: Animated<f32>,
    thumb: Presence<()>,
    was_active: bool,
    shown_filled: Option<f32>,
    painted: Rc<Cell<bool>>,
    _playback: Subscription,
    _playhead: Subscription,
}

impl ShortsScrubber {
    pub fn new(playback: Entity<ShortsPlaybackController>, cx: &mut Context<Self>) -> Self {
        Self {
            id: ElementId::NamedInteger("shorts-scrubber".into(), cx.entity_id().as_u64()),
            _playback: cx.observe(&playback, |this, playback, cx| this.playback_did_change(playback, cx)),
            _playhead: cx.subscribe(&playback, |this, playback, _: &ShortsPlaybackTimeChanged, cx| {
                this.playback_did_change(playback, cx)
            }),
            playback,
            drag_fraction: None,
            width: 0.0,
            bar_height: Animated::new(IDLE_BAR_HEIGHT),
            thumb: Presence::new(None).with_transition(Transition::opacity()),
            was_active: false,
            shown_filled: None,
            painted: Rc::new(Cell::new(false)),
        }
    }

    fn playback_did_change(&mut self, playback: Entity<ShortsPlaybackController>, cx: &mut Context<Self>) {
        let (active, progress) = {
            let playback = playback.read(cx);
            (playback.is_scrubbing(), playback.progress())
        };
        if active != self.was_active || self.drag_fraction.is_some() {
            cx.notify();
            return;
        }
        let filled = Self::filled_width(progress, self.width);
        if self.shown_filled == Some(filled) {
            return;
        }
        let swappable = !active && self.painted.get() && !self.bar_height.is_running(RiseClock::monotonic());
        let path = swappable.then(|| Self::fill_path(filled, *self.bar_height.target(), Point::default())).flatten();
        let entity_id = cx.entity_id();
        let key = Self::swap_key(entity_id.as_u64());
        let swapped = path.is_some_and(|path| {
            cx.with_window(entity_id, |window, _| window.swap_paths(vec![(key, path, white().into())])).unwrap_or(false)
        });
        if swapped {
            self.shown_filled = Some(filled);
        } else {
            self.painted.set(false);
            cx.notify();
        }
    }

    fn filled_width(fraction: f64, width: f32) -> f32 {
        ((fraction.clamp(0.0, 1.0) as f32 * width) / FILL_STEP_POINTS).round() * FILL_STEP_POINTS
    }

    fn fill_path(filled: f32, height: f32, origin: Point<Pixels>) -> Option<Path<Pixels>> {
        rounded_rect_path(0.0, 0.0, filled, height, origin)
    }

    fn swap_key(entity: u64) -> u64 {
        entity ^ (4 << 56)
    }

    /// cached host the cell places where the bar goes
    pub fn element(scrubber: Entity<Self>) -> AnyElement {
        div()
            .w_full()
            .h(px(SCRUBBER_HEIGHT))
            .child(
                AnyView::from(scrubber)
                    .cached(StyleRefinement::default().w_full().h(px(SCRUBBER_HEIGHT)))
                    .replay_when_moved(),
            )
            .into_any_element()
    }

    fn fraction_at(&self, value: &DragGestureValue) -> f64 {
        (f64::from(f32::from(value.location.x)) / f64::from(self.width.max(1.0))).clamp(0.0, 1.0)
    }
}

impl Render for ShortsScrubber {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let playback = self.playback.clone();
        let active = playback.read(cx).is_scrubbing();
        let now = RiseClock::monotonic();
        if self.was_active != active {
            self.was_active = active;
            let target = if active { ACTIVE_BAR_HEIGHT } else { IDLE_BAR_HEIGHT };
            self.bar_height.set_in(target, ease_out(0.15), now, cx);
            self.thumb.set_in(active.then_some(()), ease_out(0.15), now, cx);
        }
        let bar_height = self.bar_height.frame_value(now, window);
        if self.bar_height.is_running(now) {
            window.request_animation_frame();
        }

        let shown_fraction = self.drag_fraction.unwrap_or_else(|| playback.read(cx).progress());
        let filled_width = Self::filled_width(shown_fraction, self.width);
        self.shown_filled = self.drag_fraction.is_none().then_some(filled_width);
        let painted = self.painted.clone();
        painted.set(false);
        let swap_key = Self::swap_key(cx.entity_id().as_u64());
        let thumb = self.thumb.render_transitioned(now, window, |_| {
            div()
                .absolute()
                .left(px(filled_width - THUMB_SIDE / 2.0))
                .top(px((bar_height - THUMB_SIDE) / 2.0))
                .size(px(THUMB_SIDE))
                .circle()
                .bg(white())
                .shadow(vec![BoxShadow {
                    color: alpha(black(), 0.35),
                    offset: point(px(0.0), px(1.0)),
                    blur_radius: px(2.0),
                    spread_radius: px(0.0),
                    inset: false,
                }])
        });

        let changed_state = cx.entity();
        let changed_playback = playback.clone();
        let ended_state = cx.entity();
        let ended_playback = playback;
        let gesture = DragGesture::new(0.0)
            .on_changed(move |value, _, cx| {
                if !changed_playback.read(cx).is_scrubbing() {
                    changed_playback.update(cx, |playback, cx| playback.set_is_scrubbing(true, cx));
                    let _ = selection_changed();
                }
                let fraction = changed_state.update(cx, |state, cx| {
                    let fraction = state.fraction_at(value);
                    state.drag_fraction = Some(fraction);
                    cx.notify();
                    fraction
                });
                changed_playback.update(cx, |playback, cx| playback.seek(fraction, false, cx));
            })
            .on_ended(move |value, _, cx| {
                let fraction = ended_state.update(cx, |state, cx| {
                    let fraction = state.fraction_at(value);
                    state.drag_fraction = None;
                    cx.notify();
                    fraction
                });
                ended_playback.update(cx, |playback, cx| {
                    playback.seek(fraction, true, cx);
                    playback.set_is_scrubbing(false, cx);
                });
            });

        let measured = cx.entity();
        div()
            .relative()
            .w_full()
            .h(px(SCRUBBER_HEIGHT))
            .flex()
            .items_center()
            .child(measure(ElementId::from((self.id.clone(), "geometry")), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                measured.update(cx, |state, cx| {
                    if state.width != width {
                        state.width = width;
                        cx.notify();
                    }
                });
            }))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(bar_height))
                    .capsule()
                    .bg(alpha(white(), 0.3))
                    .child(
                        canvas(|_, _, _| (), move |bounds, (), window, _| {
                            let fill = Self::fill_path(filled_width, bar_height, bounds.origin);
                            painted.set(fill.is_some());
                            if let Some(fill) = fill {
                                window.paint_swappable_path(fill, white(), swap_key, bounds.origin);
                            }
                        })
                        .absolute()
                        .left_0()
                        .top_0()
                        .size_full(),
                    )
                    .children(thumb),
            )
            .gesture(ElementId::from((self.id.clone(), "drag")), gesture)
    }
}
