use gpui::{
    App, Bounds, Context, ElementId, Entity, FontWeight, Hsla, Pixels, Point, SharedString, Subscription, Window, canvas, deferred, img,
    svg, white,
};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_ui::grouped_btns::GroupedCardMetrics;
use rise_ui::lottie_view::LottieView;
use rise_ui::main_text::MainText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::animated::MotionTime;
use rise_ui::swiftui::motion::CubicBezier;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::shedding_hero_art::{SheddingHeroArt, SheddingHeroMetrics};
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::{SparksBurst, SparksBurstSignal, SparksTarget};
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;
use crate::modules::sparks::stores::sparks::sparks_services::sparks_services::SparksServicesStore;

const SPARKS_ICON_ASSET: &str = "images/SparksIcon.svg";
const SPARKS_ICON_SMALL_ASSET: &str = "images/SparksIconSmall.svg";

fn sparks_icon_asset(size: f32) -> &'static str {
    if size < 40.0 { SPARKS_ICON_SMALL_ASSET } else { SPARKS_ICON_ASSET }
}

/// sparks icon from the canon file, under 40 pt the small master is used so its readable in a chip
#[derive(IntoElement)]
pub struct SparksIconView {
    size: f32,
}

impl SparksIconView {
    pub fn new() -> Self {
        Self { size: 16.0 }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
}

impl Default for SparksIconView {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for SparksIconView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        img(sparks_icon_asset(self.size)).frame_size(self.size, self.size).flex_none()
    }
}

/// hero art of the sparks sheets, the sparks_hero loop drawn at 80% of size
/// power saving plays it once
#[derive(IntoElement)]
pub struct SparksHeroArt {
    size: f32,
}

impl SparksHeroArt {
    const ANIMATION: &'static str = "sparks_hero";

    pub fn new() -> Self {
        Self { size: 120.0 }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
}

impl Default for SparksHeroArt {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for SparksHeroArt {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        if LottieView::can_play(Self::ANIMATION, cx) {
            LottieView::new(Self::ANIMATION)
                .size(self.size)
                .looping(!RiseEnergySavingStore::snapshot_reduces(RiseEnergySavingFeature::LoopAnimations))
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .frame_size(self.size, self.size)
                .child(SparksIconView::new().size(self.size * 0.8))
                .into_any_element()
        }
    }
}

/// header of my sparks, the canon icon shedding yellow sparkles
#[derive(IntoElement)]
pub struct SparksPageHero;

impl SparksPageHero {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SparksPageHero {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for SparksPageHero {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        SheddingHeroArt::new("sparks_page_hero", || SparksIconView::new().size(SheddingHeroMetrics::SLOT * 0.8))
    }
}

struct SparksBurstPopKeyframe {
    to: f32,
    duration: f64,
    curve: CubicBezier,
}

const SPARKS_BURST_POP: [SparksBurstPopKeyframe; 4] = [
    SparksBurstPopKeyframe { to: 1.45, duration: 0.0795, curve: CubicBezier(0.0, 0.0, 0.386, 0.423) },
    SparksBurstPopKeyframe { to: 0.92, duration: 0.1069, curve: CubicBezier(0.279, 0.302, 0.621, 0.653) },
    SparksBurstPopKeyframe { to: 1.05, duration: 0.1119, curve: CubicBezier(0.313, 0.35, 0.651, 0.696) },
    SparksBurstPopKeyframe { to: 1.0, duration: 0.2217, curve: CubicBezier(0.334, 0.577, 0.686, 1.0) },
];

fn sparks_burst_pop_scale(elapsed: f64) -> Option<f32> {
    let mut from = 1.0_f32;
    let mut start = 0.0_f64;
    for keyframe in &SPARKS_BURST_POP {
        if elapsed < start + keyframe.duration {
            let progress = keyframe.curve.progress((elapsed - start) / keyframe.duration) as f32;
            return Some(from + (keyframe.to - from) * progress);
        }
        from = keyframe.to;
        start += keyframe.duration;
    }
    None
}

struct SparksBurstIconState {
    _signal: Entity<SparksBurstSignal>,
    last_tick: i64,
    pop_started_at: Option<MotionTime>,
    probe_deadline: Option<TimeInterval>,
    _signal_observation: Subscription,
}

impl SparksBurstIconState {
    fn new(target: &SparksTarget, cx: &mut Context<Self>) -> Self {
        let service = SparksServicesStore::shared(cx);
        let signal = service.update(cx, |service, cx| service.burst_signal(target, cx));
        let last_tick = signal.read(cx).tick();
        let signal_observation = cx.observe(&signal, |state: &mut Self, signal, cx| {
            let (tick, is_fresh) = {
                let signal = signal.read(cx);
                (signal.tick(), signal.is_fresh())
            };
            if tick == state.last_tick {
                return;
            }
            state.last_tick = tick;
            if is_fresh {
                state.celebrate(cx);
            }
        });
        Self { _signal: signal, last_tick, pop_started_at: None, probe_deadline: None, _signal_observation: signal_observation }
    }

    fn celebrate(&mut self, cx: &mut Context<Self>) {
        if reduce_motion(cx) {
            return;
        }
        let now = RiseClock::monotonic();
        self.pop_started_at = Some(now);
        cx.notify();
        if !SparksBurst::allows_decorative_motion() || !LottieView::can_play(SparksBurst::ANIMATION, cx) {
            return;
        }
        self.probe_deadline = Some(now.as_secs_f64() + SparksBurst::FRESHNESS);
    }

    fn take_probe(&mut self, deadline: TimeInterval, cx: &mut Context<Self>) -> bool {
        if self.probe_deadline != Some(deadline) {
            return false;
        }
        self.probe_deadline = None;
        cx.notify();
        true
    }
}

fn sparks_burst_probe(
    deadline: TimeInterval,
    report: impl FnOnce(Option<Bounds<Pixels>>, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            if bounds.size.width <= px(0.0) {
                return;
            }
            let is_shown = RiseClock::monotonic().as_secs_f64() <= deadline
                && !window.content_mask().bounds.intersect(&bounds).is_empty();
            let frame = is_shown.then_some(bounds);
            window.defer(cx, move |window, cx| report(frame, window, cx));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

/// sparks button icon, an accepted or sheet sent spark pops it
/// is_live false is plain icon
#[derive(IntoElement)]
pub struct SparksBurstIcon {
    size: f32,
    target: SparksTarget,
    taps: i64,
    is_live: bool,
}

impl SparksBurstIcon {
    pub fn new(size: f32, target: SparksTarget, taps: i64) -> Self {
        Self { size, target, taps, is_live: true }
    }

    pub fn is_live(mut self, is_live: bool) -> Self {
        self.is_live = is_live;
        self
    }

    fn state_id(target: &SparksTarget) -> ElementId {
        let key = match target {
            SparksTarget::Message { chat_id, message_id } => format!("sparks_burst_icon/message/{chat_id}/{message_id}"),
            SparksTarget::Post { post_id } => format!("sparks_burst_icon/post/{post_id}"),
        };
        ElementId::Name(SharedString::from(key))
    }
}

impl RenderOnce for SparksBurstIcon {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !self.is_live {
            return SparksIconView::new().size(self.size).into_any_element();
        }

        let target = self.target;
        let id = Self::state_id(&target);
        let state = window.use_keyed_state(id.clone(), cx, |_, cx| SparksBurstIconState::new(&target, cx));

        let on_taps = state.clone();
        on_change(
            (id, "taps"),
            self.taps,
            false,
            window,
            cx,
            move |_, _, _, cx| on_taps.update(cx, |state, cx| state.celebrate(cx)),
        );

        let (pop_started_at, probe_deadline) = {
            let state = state.read(cx);
            (state.pop_started_at, state.probe_deadline)
        };
        let scale = pop_started_at
            .and_then(|started| sparks_burst_pop_scale(RiseClock::monotonic().saturating_sub(started).as_secs_f64()));
        if scale.is_some() {
            window.request_animation_frame();
        }

        let side = self.size * scale.unwrap_or(1.0);
        let inset = (self.size - side) / 2.0;
        let mut body = div().relative().flex_none().frame_size(self.size, self.size).child(
            img(sparks_icon_asset(self.size)).absolute().left(px(inset)).top(px(inset)).frame_size(side, side),
        );
        if let Some(deadline) = probe_deadline {
            body = body.child(sparks_burst_probe(deadline, move |frame, _, cx| {
                if !state.update(cx, |state, cx| state.take_probe(deadline, cx)) {
                    return;
                }
                if let Some(frame) = frame {
                    SparksInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.burst(frame, cx));
                }
            }));
        }
        body.into_any_element()
    }
}

/// draws the sparks burst above content at each point an icon reported
/// crosses card edges, lives 60 frames
#[derive(IntoElement)]
pub struct SparksBurstLayer;

impl SparksBurstLayer {
    const LEVEL: usize = 1002;

    pub fn new() -> Self {
        Self
    }
}

impl Default for SparksBurstLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for SparksBurstLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let origin = window.use_keyed_state("sparks_burst_layer/origin", cx, |_, _| Point::<Pixels>::default());
        let bursts = SparksServicesStore::shared(cx).read(cx).bursts.clone();

        let adopted = origin.clone();
        let mut layer = pass_through().absolute().inset_0().child(measure(
            "sparks_burst_layer/frame",
            move |bounds, _, cx| {
                adopted.update(cx, |origin, cx| {
                    if *origin != bounds.origin {
                        *origin = bounds.origin;
                        cx.notify();
                    }
                });
            },
        ));
        if !bursts.is_empty() && LottieView::can_play(SparksBurst::ANIMATION, cx) {
            let origin = *origin.read(cx);
            let (origin_x, origin_y) = (f32::from(origin.x), f32::from(origin.y));
            for burst in bursts {
                let half = burst.side / 2.0;
                layer = layer.child(
                    div()
                        .id(SharedString::from(format!("sparks_burst/{}", burst.id)))
                        .absolute()
                        .left(px(burst.center.x - origin_x - half))
                        .top(px(burst.center.y - origin_y - half))
                        .child(LottieView::new(SparksBurst::ANIMATION).size(burst.side).looping(false)),
                );
            }
        }
        deferred(layer).with_priority(Self::LEVEL)
    }
}

/// an amount with the icon then the canon compact number
#[derive(IntoElement)]
pub struct SparksAmountLabel {
    amount: i64,
    px: f32,
    icon_size: f32,
    weight: FontWeight,
    color: Option<Hsla>,
}

impl SparksAmountLabel {
    pub fn new(amount: i64) -> Self {
        Self { amount, px: 15.0, icon_size: 16.0, weight: FontWeight::SEMIBOLD, color: None }
    }

    pub fn px(mut self, px: f32) -> Self {
        self.px = px;
        self
    }

    pub fn icon_size(mut self, icon_size: f32) -> Self {
        self.icon_size = icon_size;
        self
    }

    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight;
        self
    }

    pub fn color(mut self, color: impl Into<Option<Hsla>>) -> Self {
        self.color = color.into();
        self
    }
}

impl RenderOnce for SparksAmountLabel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        hstack(4.0)
            .child(SparksIconView::new().size(self.icon_size))
            .child(
                MainText::new(RiseFormat::compact_count(self.amount))
                    .px(self.px)
                    .font_weight(self.weight)
                    .color(self.color)
                    .number_of_lines(1)
                    .monospaced_digits(true),
            )
    }
}

/// wallet row tile in settings, canon icon on a pale tile
/// solid is the switch row one, white spark on backdrop
#[derive(IntoElement)]
pub struct SparksSettingsTile {
    solid: bool,
}

impl SparksSettingsTile {
    pub fn new() -> Self {
        Self { solid: false }
    }

    pub fn solid(mut self, solid: bool) -> Self {
        self.solid = solid;
        self
    }
}

impl Default for SparksSettingsTile {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for SparksSettingsTile {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let backdrop = SettingsIconPalette::sparks();
        let tile = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .frame_size(GroupedCardMetrics::ICON_SIDE, GroupedCardMetrics::ICON_SIDE)
            .continuous_corners(11.0)
            .bg(if self.solid { backdrop } else { backdrop.opacity(0.18) });
        if self.solid {
            tile.child(svg().path(SPARKS_ICON_SMALL_ASSET).frame_size(19.0, 19.0).flex_none().text_color(white()))
        } else {
            tile.child(SparksIconView::new().size(20.0))
        }
    }
}
