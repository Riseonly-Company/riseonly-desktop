use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, ElementId, Entity, FontWeight, Hsla, IntoElement, ParentElement, PathBuilder, Pixels,
    RenderOnce, Styled, Window, canvas, div, point, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_theme::ThemeStore;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::motion::{CubicBezier, MotionSpec};

use crate::core::widgets::notifier::notifier_host::NotifierWindowPresenter;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierStore, NotifierType,
};

/// background job shows as a bottom banner with a ring that fills and a cancel button, no timer
/// stays until the job ends, a tap only hides it and the job goes on
pub struct NotifierProgressStore {
    entries: HashMap<Uuid, NotifierProgressStoreEntry>,
    fractions: HashMap<Uuid, f64>,
    cancels: HashMap<Uuid, Rc<dyn Fn(&mut App)>>,
    hidden_by_user: HashSet<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifierProgressStoreEntry {
    pub action_title: Option<String>,
    pub title_template: Option<String>,
}

impl NotifierProgressStore {
    pub const PROGRESS_TOKEN: &'static str = "{progress}";

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            entries: HashMap::new(),
            fractions: HashMap::new(),
            cancels: HashMap::new(),
            hidden_by_user: HashSet::new(),
        }
    }

    pub fn entries(&self) -> &HashMap<Uuid, NotifierProgressStoreEntry> {
        &self.entries
    }

    pub fn entry(&self, id: Uuid) -> Option<&NotifierProgressStoreEntry> {
        self.entries.get(&id)
    }

    pub fn register(
        &mut self,
        id: Uuid,
        fraction: f64,
        action_title: Option<String>,
        title_template: Option<String>,
        on_cancel: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        self.prune(cx);
        self.entries.insert(id, NotifierProgressStoreEntry { action_title, title_template });
        self.fractions.insert(id, Self::clamp(fraction));
        match on_cancel {
            Some(handler) => {
                self.cancels.insert(id, handler);
            }
            None => {
                self.cancels.remove(&id);
            }
        }
        NotifierWindowPresenter::recheck_bottom_chrome_after_transition(cx);
        cx.notify();
    }

    /// false once the banner is gone (hidden by tap, replaced or dismissed)
    pub fn update(&mut self, id: Uuid, fraction: f64, cx: &mut Context<Self>) -> bool {
        if !self.is_showing(id, cx) || !self.entries.contains_key(&id) {
            return false;
        }
        let value = Self::clamp(fraction);
        if Self::percent(value) == Self::percent(self.fractions.get(&id).copied().unwrap_or(0.0)) {
            return true;
        }
        self.fractions.insert(id, value);
        NotifierWindowPresenter::recheck_bottom_chrome(cx);
        cx.notify();
        true
    }

    pub fn fraction(&self, id: Uuid) -> f64 {
        self.fractions.get(&id).copied().unwrap_or(0.0)
    }

    /// whole percent the ring shows 0 to 100, the title gets the same number
    pub fn percent(fraction: f64) -> i64 {
        (Self::clamp(fraction) * 100.0) as i64
    }

    pub fn title(template: &str, fraction: f64) -> String {
        template.replace(Self::PROGRESS_TOKEN, &Self::percent(fraction).to_string())
    }

    pub fn is_showing(&self, id: Uuid, cx: &App) -> bool {
        let Some(store) = rise_ui::shared::try_read::<NotifierStore>(cx) else {
            return false;
        };
        store.items().iter().any(|item| item.id == id) || store.pending_latest().is_some_and(|item| item.id == id)
    }

    /// tap or swipe on the island, the person put it away and it doesnt come back alone
    pub fn hide(&mut self, id: Uuid) {
        if !self.entries.contains_key(&id) {
            return;
        }
        self.hidden_by_user.insert(id);
    }

    /// true when another banner took this ones place and left
    /// the run is still going so it returns once nothing else is on screen or waiting
    pub fn can_return(&self, id: Uuid, cx: &App) -> bool {
        if !self.entries.contains_key(&id) || self.hidden_by_user.contains(&id) {
            return false;
        }
        let Some(store) = rise_ui::shared::try_read::<NotifierStore>(cx) else {
            return true;
        };
        store.items().is_empty() && store.pending_latest().is_none()
    }

    /// cancel button of the island
    /// entry stays until the next banner so the leaving island keeps its ring
    pub fn cancel(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let handler = self.cancels.remove(&id);
        tracing::warn!(target: "riseonly::X807", "island cancel id={} handler={}", id.uuid_string(), handler.is_some());
        Notifier::dismiss(id, cx);
        if let Some(handler) = handler {
            cx.defer(move |cx| handler(cx));
        }
    }

    fn prune(&mut self, cx: &App) {
        let gone: Vec<Uuid> = self.entries.keys().copied().filter(|id| !self.is_showing(*id, cx)).collect();
        for id in gone {
            self.entries.remove(&id);
            self.fractions.remove(&id);
            self.cancels.remove(&id);
            self.hidden_by_user.remove(&id);
        }
    }

    fn clamp(value: f64) -> f64 {
        value.clamp(0.0, 1.0)
    }
}

impl Notifier {
    /// action_title none with on_cancel none shows ring and title without a button
    /// cancel for a download, nothing for a publish the person cant stop
    pub fn send_progress(
        title: impl Into<String>,
        action_title: Option<String>,
        fraction: f64,
        position: NotifierPosition,
        offset: f32,
        on_cancel: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut App,
    ) -> Uuid {
        let title = title.into();
        let template = title.contains(NotifierProgressStore::PROGRESS_TOKEN).then(|| title.clone());
        let shown = template.as_deref().map(|template| NotifierProgressStore::title(template, fraction)).unwrap_or(title);
        let options = NotifierSendOptions {
            r#type: NotifierType::Info,
            position,
            offset,
            icon: NotifierIcon::None,
            duration: 0.0,
            ..NotifierSendOptions::default()
        };
        let id = Notifier::send(shown, options, cx);
        NotifierProgressStore::shared(cx).update(cx, |store, cx| {
            store.register(id, fraction, action_title, template, on_cancel, cx);
        });
        id
    }
}

/// ring of one banner, reads the step itself so a step redraws only this view
#[derive(IntoElement)]
pub struct NotifierProgressLiveRing {
    pub id: Uuid,
}

impl RenderOnce for NotifierProgressLiveRing {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let fraction = NotifierProgressStore::shared(cx).read(cx).fraction(self.id);
        NotifierProgressRing {
            id: ElementId::Name(format!("notifier-progress-ring-{}", self.id.uuid_string()).into()),
            fraction,
        }
    }
}

/// title of one banner with the live percent, a step redraws only this text
#[derive(IntoElement)]
pub struct NotifierProgressTitle {
    pub id: Uuid,
    pub template: String,
    pub color: Hsla,
    pub number_of_lines: usize,
}

impl RenderOnce for NotifierProgressTitle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let fraction = NotifierProgressStore::shared(cx).read(cx).fraction(self.id);
        MainText::new(NotifierProgressStore::title(&self.template, fraction))
            .px(15.0)
            .font_weight(FontWeight::SEMIBOLD)
            .color(self.color)
            .number_of_lines(self.number_of_lines)
    }
}

const RING_SIDE: f32 = 26.0;
const RING_PADDING: f32 = 2.0;
const RING_LINE_WIDTH: f32 = 3.0;
const RING_TRACK_OPACITY: f32 = 0.22;
const RING_SEGMENTS: usize = 96;
const RING_STEP_MOTION: MotionSpec = MotionSpec::curve(CubicBezier::LINEAR, Duration::from_millis(200));

#[derive(IntoElement)]
pub struct NotifierProgressRing {
    pub id: ElementId,
    pub fraction: f64,
}

impl RenderOnce for NotifierProgressRing {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ink = ThemeStore::shared(cx).read(cx).current_theme().primary._100;
        let value = self.fraction.clamp(0.0, 1.0) as f32;
        let now = RiseClock::monotonic();
        let reduce = reduce_motion(cx);
        let state = window.use_keyed_state(self.id.clone(), cx, move |_, _| Animated::new(value));
        state.update(cx, |animated, _| animated.set(value, RING_STEP_MOTION, now, reduce));
        let shown = state.read(cx).frame_value(now, window);

        div().p(px(RING_PADDING)).child(
            div().size(px(RING_SIDE)).relative().child(
                canvas(|_, _, _| (), move |bounds, (), window, _| paint_progress_ring(bounds, shown, ink, window))
                    .absolute()
                    .inset_0(),
            ),
        )
    }
}

fn paint_progress_ring(bounds: Bounds<Pixels>, value: f32, ink: Hsla, window: &mut Window) {
    let center = bounds.center();
    let radius = f32::from(bounds.size.width.min(bounds.size.height)) / 2.0;
    let at = |fraction: f32| {
        let angle = fraction * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
    };
    let arc = |to: f32| -> Option<gpui::Path<Pixels>> {
        let steps = ((RING_SEGMENTS as f32) * to).ceil().max(1.0) as usize;
        let mut builder = PathBuilder::stroke(px(RING_LINE_WIDTH));
        for step in 0..=steps {
            let position = at(to * step as f32 / steps as f32);
            if step == 0 {
                builder.move_to(position);
            } else {
                builder.line_to(position);
            }
        }
        builder.build().ok()
    };
    if let Some(track) = arc(1.0) {
        window.paint_path(track, ink.opacity(RING_TRACK_OPACITY));
    }
    if value <= 0.0 {
        return;
    }
    if let Some(filled) = arc(value) {
        window.paint_path(filled, ink);
    }
    if value < 1.0 {
        for end in [at(0.0), at(value)] {
            if let Some(cap) = round_cap(end, RING_LINE_WIDTH / 2.0) {
                window.paint_path(cap, ink);
            }
        }
    }
}

fn round_cap(center: gpui::Point<Pixels>, radius: f32) -> Option<gpui::Path<Pixels>> {
    const CAP_SEGMENTS: usize = 12;
    let mut builder = PathBuilder::fill();
    for step in 0..CAP_SEGMENTS {
        let angle = step as f32 / CAP_SEGMENTS as f32 * std::f32::consts::TAU;
        let position = point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()));
        if step == 0 {
            builder.move_to(position);
        } else {
            builder.line_to(position);
        }
    }
    builder.close();
    builder.build().ok()
}
