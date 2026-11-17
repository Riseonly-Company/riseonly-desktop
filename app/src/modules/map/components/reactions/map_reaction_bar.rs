use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, DispatchPhase, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, Render, SharedString, Task, Window, canvas, point,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;

pub type MapReactionShot = Rc<dyn Fn(String, Point<Pixels>, &mut Window, &mut App) -> bool>;
pub type MapReactionPicker = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;
pub type MapReactionBurst = Rc<dyn Fn(i64, &mut Window, &mut App)>;

/// quick reaction row in a friend's card
/// click sends one, holding keeps sending faster until release or a full burst
/// bar reads the pointer itself so click and hold are one gesture
#[derive(IntoElement)]
pub struct MapReactionBar {
    emojis: Vec<String>,
    on_shot: MapReactionShot,
    on_picker: MapReactionPicker,
    on_burst: MapReactionBurst,
}

impl MapReactionBar {
    pub fn new(
        emojis: Vec<String>,
        on_shot: impl Fn(String, Point<Pixels>, &mut Window, &mut App) -> bool + 'static,
        on_picker: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { emojis, on_shot: Rc::new(on_shot), on_picker: Rc::new(on_picker), on_burst: Rc::new(|_, _, _| {}) }
    }

    pub fn on_burst(mut self, on_burst: impl Fn(i64, &mut Window, &mut App) + 'static) -> Self {
        self.on_burst = Rc::new(on_burst);
        self
    }
}

impl RenderOnce for MapReactionBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view: Entity<MapReactionBarView> =
            window.use_keyed_state("map-reaction-bar-view", cx, |_, _| MapReactionBarView::new());
        view.update(cx, |view, cx| {
            view.configure(self.emojis, cx);
            view.on_shot = Some(self.on_shot);
            view.on_picker = Some(self.on_picker);
            view.on_burst = Some(self.on_burst);
        });
        div().w_full().h(px(MapReactionBarView::HEIGHT)).child(view)
    }
}

/// one emoji of the row and the scale its press springs to
pub struct MapReactionEmojiLabel {
    pub text: String,
    transform: Animated<f32>,
}

impl MapReactionEmojiLabel {
    fn new(text: String) -> Self {
        Self { text, transform: Animated::new(1.0) }
    }
}

pub struct MapReactionBarView {
    pub on_shot: Option<MapReactionShot>,
    pub on_picker: Option<MapReactionPicker>,
    pub on_burst: Option<MapReactionBurst>,
    emojis: Vec<String>,
    labels: Vec<MapReactionEmojiLabel>,
    firing_index: Option<usize>,
    timer: Option<Task<()>>,
    shots_in_hold: i64,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl MapReactionBarView {
    const HEIGHT: f32 = 44.0;
    const EMOJI_POINT_SIZE: f32 = 28.0;
    const HOLD_DELAY: f64 = 0.28;
    const SLOWEST_INTERVAL: f64 = 0.13;
    const FASTEST_INTERVAL: f64 = 0.055;

    fn new() -> Self {
        Self {
            on_shot: None,
            on_picker: None,
            on_burst: None,
            emojis: Vec::new(),
            labels: Vec::new(),
            firing_index: None,
            timer: None,
            shots_in_hold: 0,
            bounds: Rc::new(Cell::new(None)),
        }
    }

    pub fn configure(&mut self, next: Vec<String>, cx: &mut Context<Self>) {
        if next == self.emojis {
            return;
        }
        self.labels = next.iter().cloned().map(MapReactionEmojiLabel::new).collect();
        self.emojis = next;
        cx.notify();
    }

    fn slot(&self, bounds: &Bounds<Pixels>) -> Pixels {
        bounds.size.width / (self.labels.len() + 1) as f32
    }

    fn touches_began(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.index(position) else {
            return;
        };
        self.firing_index = Some(index);
        self.shots_in_hold = 0;
        self.press(index, true, cx);
        if !self.fire(window, cx) {
            return;
        }
        self.schedule(Self::HOLD_DELAY, window, cx);
    }

    fn touches_moved(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(firing_index) = self.firing_index else {
            return;
        };
        let next = self.index(position);
        if next == Some(firing_index) {
            return;
        }
        self.press(firing_index, false, cx);
        let Some(next) = next else {
            return self.stop_firing(cx);
        };
        self.firing_index = Some(next);
        self.press(next, true, cx);
    }

    pub fn stop_firing(&mut self, cx: &mut Context<Self>) {
        self.timer = None;
        if let Some(firing_index) = self.firing_index {
            self.press(firing_index, false, cx);
        }
        self.firing_index = None;
    }

    fn schedule(&mut self, interval: f64, window: &mut Window, cx: &mut Context<Self>) {
        let delay = Duration::from_secs_f64(interval);
        self.timer = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update_in(cx, |this, window, cx| this.timer_fired(window, cx));
        }));
    }

    fn timer_fired(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.firing_index.is_none() {
            return;
        }
        if !self.fire(window, cx) {
            return self.stop_firing(cx);
        }
        let ramp = (self.shots_in_hold as f64 / 12.0).min(1.0);
        self.schedule(
            Self::SLOWEST_INTERVAL - (Self::SLOWEST_INTERVAL - Self::FASTEST_INTERVAL) * ramp,
            window,
            cx,
        );
    }

    fn fire(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let (Some(firing_index), Some(bounds)) = (self.firing_index, self.bounds.get()) else {
            return false;
        };
        let Some(emoji) = self.emojis.get(firing_index).cloned() else {
            return false;
        };
        let slot = self.slot(&bounds);
        let source = point(bounds.origin.x + slot * (firing_index as f32 + 0.5), bounds.center().y);
        self.shots_in_hold += 1;
        let delivered = self.on_shot.clone().is_some_and(|on_shot| on_shot(emoji, source, window, cx));
        if delivered && let Some(on_burst) = self.on_burst.clone() {
            on_burst(self.shots_in_hold, window, cx);
        }
        delivered
    }

    fn press(&mut self, index: usize, down: bool, cx: &mut Context<Self>) {
        let Some(label) = self.labels.get_mut(index) else {
            return;
        };
        let spec = if down { spring(0.12, 1.0) } else { spring(0.3, 0.5) };
        label.transform.set_in(if down { 1.32 } else { 1.0 }, spec, RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn index(&self, position: Point<Pixels>) -> Option<usize> {
        let bounds = self.bounds.get()?;
        if self.labels.is_empty() {
            return None;
        }
        let expanded = Bounds::from_corners(
            point(bounds.origin.x, bounds.origin.y - px(12.0)),
            point(bounds.origin.x + bounds.size.width, bounds.origin.y + bounds.size.height + px(12.0)),
        );
        if !expanded.contains(&position) {
            return None;
        }
        let slot = self.slot(&bounds);
        let index = ((position.x - bounds.origin.x) / slot).max(0.0) as usize;
        (index < self.labels.len()).then_some(index)
    }

    fn picker_center(&self) -> Option<Point<Pixels>> {
        let bounds = self.bounds.get()?;
        let slot = self.slot(&bounds);
        Some(point(bounds.origin.x + bounds.size.width - slot / 2.0, bounds.center().y))
    }
}

impl Render for MapReactionBarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let now = RiseClock::monotonic();
        let mut row = div()
            .id("map-reaction-bar")
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.touches_began(event.position, window, cx);
                }),
            );
        for (index, label) in self.labels.iter().enumerate() {
            let scale = label.transform.frame_value(now, window);
            row = row.child(
                div()
                    .id(SharedString::from(format!("map-reaction-bar-emoji-{index}")))
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(Self::EMOJI_POINT_SIZE * scale))
                    .child(SharedString::from(label.text.clone())),
            );
        }
        let picker = cx.entity().downgrade();
        row = row.child(
            div().flex_1().h_full().child(
                plain("map-reaction-bar-picker")
                    .on_press(move |window, cx| {
                        let target = picker.update(cx, |this, cx| {
                            this.stop_firing(cx);
                            this.picker_center().zip(this.on_picker.clone())
                        });
                        if let Ok(Some((center, on_picker))) = target {
                            on_picker(center, window, cx);
                        }
                    })
                    .child(
                        div()
                            .id("map-reaction-bar-picker-face")
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .tooltip_label(tr("settings_emoji_title"))
                            .children(IconUi::sized("face.smiling", px(22.0), theme.text.primary, false)),
                    )
                    .guide_anchor(RiseGuideAnchor::MapReactionPicker),
            ),
        );

        let bounds_cell = self.bounds.clone();
        let tracking = self.firing_index.is_some();
        let weak = cx.entity().downgrade();
        row.child(
            canvas(
                move |bounds, _, _| bounds_cell.set(Some(bounds)),
                move |_, _, window, _| {
                    if !tracking {
                        return;
                    }
                    let move_target = weak.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase != DispatchPhase::Capture || !event.dragging() {
                            return;
                        }
                        let _ = move_target.update(cx, |this, cx| this.touches_moved(event.position, cx));
                    });
                    let up_target = weak.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                        if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                            return;
                        }
                        let _ = up_target.update(cx, |this, cx| this.stop_firing(cx));
                    });
                },
            )
            .absolute()
            .size_full(),
        )
    }
}
