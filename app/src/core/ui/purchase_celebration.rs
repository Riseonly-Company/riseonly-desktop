use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext, Bounds, Context, ElementId, Entity, FontWeight, Global,
    InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render, RenderImage, RenderOnce, SharedString,
    Styled, Subscription, Task, WeakEntity, Window, anchored, deferred, div, img, point, px, white,
};
use image::{Delay, Frame, RgbaImage};
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_media::lottie::raster_policy::{self, SequenceKey};
use rise_media::lottie::sequence::{DecodedFrame, DecodedFrameCache, FrameSequence};
use rise_platform::haptics::{self, NotificationKind};
use rise_theme::ThemeStore;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::button_ui::ButtonUi;
use rise_ui::lottie_view::{LottieView, LottieViewModel};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::measure::measure;
use smallvec::SmallVec;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PurchaseCelebration {
    Sparks,
    Premium,
}

impl PurchaseCelebration {
    pub const CANVAS_SIDE: f32 = 844.0;
    pub const ICON_ON_CANVAS: Point<Pixels> = Point {
        x: px(422.0),
        y: px(575.0),
    };
    pub const ICON_BELOW_SHEET_TOP: f32 = 122.0;
    pub const ICON_TIP_TO_TIP: f32 = 110.0;

    pub fn animation(self) -> &'static str {
        match self {
            Self::Sparks => "sparks_success",
            Self::Premium => "premium_success",
        }
    }

    /// low power mode is covered by the energy saving gate, on desktop it follows the system power state
    pub fn allows_motion(cx: &App) -> bool {
        !reduce_motion(cx)
            && !RiseEnergySavingStore::snapshot_reduces(RiseEnergySavingFeature::InterfaceEffects)
    }

    /// while the purchase runs the 460 KB file is parsed off the main thread so the first frame of
    /// the sheet doesnt wait for it
    pub fn prepare(self, cx: &mut App) {
        LottieView::prewarm(self.animation(), cx);
        PurchaseCelebrationHaptics::prepare();
    }

    pub fn configure(player: &mut PurchaseCelebrationPlayer, cx: &mut Context<PurchaseCelebrationPlayer>) {
        player.stop(cx);
    }
}

/// geometry of a success sheet, set by the celebration canvas
/// its icon sits ICON_BELOW_SHEET_TOP below the sheet top edge where the canvas was drawn
pub enum PurchaseCelebrationSheetLayout {}

impl PurchaseCelebrationSheetLayout {
    pub const HANDLE: f32 =
        BottomSheetMetrics::HANDLE_TOP_PADDING + BottomSheetMetrics::HANDLE_HEIGHT + BottomSheetMetrics::HANDLE_BOTTOM_PADDING;
    pub const ICON_Y: f32 = PurchaseCelebration::ICON_BELOW_SHEET_TOP - Self::HANDLE;
    pub const HERO_HEIGHT: f32 = Self::ICON_Y + PurchaseCelebration::ICON_TIP_TO_TIP / 2.0 + 22.0;
    pub const FOOTER_VERTICAL_PADDING: f32 = 10.0;
    pub const INITIAL_FRACTION: f32 = 0.48;

    /// sheet is sized from a fraction of the window, handle
    /// content and the footer with its button
    /// a desktop window has no bottom safe area
    pub fn fraction(content_height: f32, button_height: f32, window: &Window) -> f32 {
        let screen = f32::from(window.viewport_size().height).max(1.0);
        let sheet = Self::HANDLE + content_height + button_height + 2.0 * Self::FOOTER_VERTICAL_PADDING;
        (sheet / screen).max(0.3).min(0.86)
    }
}

type SheetSlot = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// small sheet after a purchase as tall as its content, the celebration over its icon
/// the caller lines under it and one button
/// still is the icon when the file doesnt resolve
#[derive(IntoElement)]
pub struct PurchaseCelebrationSheet {
    kind: PurchaseCelebration,
    is_presented: Binding<bool>,
    button_title: SharedString,
    on_done: SheetAction,
    still: SheetSlot,
    lines: SheetSlot,
}

impl PurchaseCelebrationSheet {
    pub fn new<S: IntoElement, L: IntoElement>(
        kind: PurchaseCelebration,
        is_presented: Binding<bool>,
        button_title: impl Into<SharedString>,
        on_done: impl Fn(&mut Window, &mut App) + 'static,
        still: impl Fn(&mut Window, &mut App) -> S + 'static,
        lines: impl Fn(&mut Window, &mut App) -> L + 'static,
    ) -> Self {
        Self {
            kind,
            is_presented,
            button_title: button_title.into(),
            on_done: Rc::new(on_done),
            still: Rc::new(move |window, cx| still(window, cx).into_any_element()),
            lines: Rc::new(move |window, cx| lines(window, cx).into_any_element()),
        }
    }

    fn adopt(fraction: &Entity<f32>, height: f32, button_height: f32, window: &Window, cx: &mut App) {
        let target = PurchaseCelebrationSheetLayout::fraction(height, button_height, window);
        fraction.update(cx, |fraction, cx| {
            if (target - *fraction).abs() <= 0.005 {
                return;
            }
            *fraction = target;
            cx.notify();
        });
    }
}

impl RenderOnce for PurchaseCelebrationSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let kind = self.kind;
        let (sheet_background, button_height) = {
            let store = ThemeStore::shared(cx);
            let theme = store.read(cx).current_theme();
            (theme.bg._100, f32::from(theme.button.height_300))
        };
        let fraction = window.use_keyed_state(
            ElementId::Name(SharedString::from(format!("purchase_celebration_sheet/{}", kind.animation()))),
            cx,
            |_, _| PurchaseCelebrationSheetLayout::INITIAL_FRACTION,
        );
        let snap_fraction = *fraction.read(cx);

        let still = self.still.clone();
        let lines = self.lines.clone();
        let on_done = self.on_done.clone();
        let button_title = self.button_title.clone();
        BottomSheetUi::new(
            ElementId::Name(SharedString::from(format!("purchase_celebration_bottom_sheet/{}", kind.animation()))),
            self.is_presented,
            move |_window, _cx| {
                let fraction = fraction.clone();
                PurchaseCelebrationSheetContent {
                    kind,
                    on_height: Rc::new(move |height, window, cx| {
                        Self::adopt(&fraction, height, button_height, window, cx);
                    }),
                    still: still.clone(),
                    lines: lines.clone(),
                }
            },
            move |_window, _cx| {
                let on_done = on_done.clone();
                div()
                    .w_full()
                    .px(px(16.0))
                    .py(px(PurchaseCelebrationSheetLayout::FOOTER_VERTICAL_PADDING))
                    .child(
                        ButtonUi::new(ElementId::Name(SharedString::from(format!(
                            "purchase_celebration_done/{}",
                            kind.animation()
                        ))))
                        .on_press(move |window, cx| on_done(window, cx))
                        .child(
                            MainText::new(button_title.clone())
                                .px(16.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(white()),
                        ),
                    )
            },
        )
        .snap_fractions(vec![snap_fraction])
        .modal_sized_by_detent(true)
        .footer_shows_top_border(false)
        .footer_horizontal_padding(Some(0.0))
        .footer_top_padding(Some(0.0))
        .sheet_background(Some(sheet_background))
        .footer_background(Some(sheet_background))
        .on_live_sheet_frame(move |frame, cx| {
            PurchaseCelebrationDirector::of(kind, cx).update(cx, |director, cx| director.sheet_frame_changed(frame, cx));
        })
    }
}

type HeightAction = Rc<dyn Fn(f32, &mut Window, &mut App)>;

#[derive(IntoElement)]
struct PurchaseCelebrationSheetContent {
    kind: PurchaseCelebration,
    on_height: HeightAction,
    still: SheetSlot,
    lines: SheetSlot,
}

impl PurchaseCelebrationSheetContent {
    fn celebration(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        if LottieView::can_play(self.kind.animation(), cx) {
            return div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(PurchaseCelebrationSheetLayout::ICON_Y) - PurchaseCelebration::ICON_ON_CANVAS.y)
                .flex()
                .justify_center()
                .child(PurchaseCelebrationCanvas { kind: self.kind })
                .into_any_element();
        }
        div()
            .absolute()
            .left_0()
            .right_0()
            .top_0()
            .h(px(PurchaseCelebrationSheetLayout::ICON_Y * 2.0))
            .flex()
            .justify_center()
            .items_center()
            .child((self.still)(window, cx))
            .into_any_element()
    }
}

impl RenderOnce for PurchaseCelebrationSheetContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let celebration = self.celebration(window, cx);
        let lines = (self.lines)(window, cx);
        let on_height = self.on_height.clone();
        div()
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .pb(px(20.0))
            .child(celebration)
            .child(div().h(px(PurchaseCelebrationSheetLayout::HERO_HEIGHT)).flex_none())
            .child(lines)
            .child(measure(
                ElementId::Name(SharedString::from(format!(
                    "purchase_celebration_content_height/{}",
                    self.kind.animation()
                ))),
                move |bounds, window, cx| on_height(f32::from(bounds.size.height), window, cx),
            ))
    }
}

#[derive(IntoElement)]
pub struct PurchaseCelebrationCanvas {
    pub kind: PurchaseCelebration,
}

impl RenderOnce for PurchaseCelebrationCanvas {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let kind = self.kind;
        let view = window.use_keyed_state(
            ElementId::Name(SharedString::from(format!("purchase_celebration_canvas/{}", kind.animation()))),
            cx,
            |window, cx| PurchaseCelebrationCanvasView::new(kind, window, cx),
        );
        let player = view.read(cx).player.clone();
        let appearing = player.clone();
        div()
            .w(px(PurchaseCelebration::CANVAS_SIDE))
            .h(px(PurchaseCelebration::CANVAS_SIDE))
            .flex_none()
            .child(view)
            .on_appear(
                ElementId::Name(SharedString::from(format!("purchase_celebration_canvas_window/{}", kind.animation()))),
                move |_window, cx| {
                    PurchaseCelebrationDirector::of(kind, cx).update(cx, |director, cx| director.attach(&appearing, cx));
                },
            )
            .and_on_disappear(move |_window, cx| {
                let director = PurchaseCelebrationDirector::of(kind, cx);
                director.update(cx, |director, cx| {
                    director.canvas_left_window(&player, cx);
                    director.detach(&player, cx);
                });
            })
    }
}

pub struct PurchaseCelebrationCanvasView {
    pub kind: PurchaseCelebration,
    pub player: Entity<PurchaseCelebrationPlayer>,
    _observers: [Subscription; 2],
}

impl PurchaseCelebrationCanvasView {
    pub fn new(kind: PurchaseCelebration, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dimension = raster_policy::dimension(PurchaseCelebration::CANVAS_SIDE, window.scale_factor());
        let player = cx.new(|cx| {
            let mut player = PurchaseCelebrationPlayer::new(kind.animation(), dimension, cx);
            PurchaseCelebration::configure(&mut player, cx);
            player
        });
        let director = PurchaseCelebrationDirector::of(kind, cx);
        let observers = [
            cx.observe(&player, |_, _, cx| cx.notify()),
            cx.observe(&director, |_, _, cx| cx.notify()),
        ];
        Self {
            kind,
            player,
            _observers: observers,
        }
    }
}

impl Render for PurchaseCelebrationCanvasView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        let side = px(PurchaseCelebration::CANVAS_SIDE);
        let image = self.player.read(cx).image.clone();
        let window_handle = window.window_handle();
        let viewport = window.viewport_size();
        let player = self.player.clone();

        let mut canvas = div().relative().w(side).h(side);
        if let Some(image) = image.clone() {
            canvas = canvas.child(
                img(image.clone())
                    .id(ElementId::NamedInteger(
                        SharedString::from(format!("purchase_celebration_sheet_copy/{}", kind.animation())),
                        image.id.0 as u64,
                    ))
                    .w(side)
                    .h(side),
            );
        }
        canvas = canvas.child(measure(
            ElementId::Name(SharedString::from(format!("purchase_celebration_canvas_layout/{}", kind.animation()))),
            move |bounds, window, cx| {
                let window_handle = window.window_handle();
                PurchaseCelebrationDirector::of(kind, cx).update(cx, |director, cx| {
                    director.sheet_canvas_did_layout(&player, bounds, window_handle, cx);
                });
            },
        ));

        let overlay = PurchaseCelebrationDirector::of(kind, cx).read(cx).overlay.clone();
        if let (Some(overlay), Some(image)) = (overlay, image)
            && overlay.window == window_handle
        {
            canvas = canvas.child(overlay.layer(kind, image, viewport));
        }
        canvas
    }
}

/// desktop twin of a configured lottie view, plays the timeline once on request
/// rests on a single frame otherwise, the lottie view rests at mid timeline and starts by itself
/// celebration needs both copies to start together
pub struct PurchaseCelebrationPlayer {
    name: SharedString,
    dimension: u32,
    pub image: Option<Arc<RenderImage>>,
    resting: Option<Arc<RenderImage>>,
    _work: Task<()>,
    _release: Subscription,
}

impl PurchaseCelebrationPlayer {
    pub fn new(name: &str, dimension: u32, cx: &mut Context<Self>) -> Self {
        let release = cx.on_release(|player: &mut Self, cx| {
            if let Some(image) = player.image.take() {
                cx.drop_image(image, None);
            }
            if let Some(resting) = player.resting.take() {
                cx.drop_image(resting, None);
            }
        });
        let mut player = Self {
            name: SharedString::from(name.to_string()),
            dimension,
            image: None,
            resting: None,
            _work: Task::ready(()),
            _release: release,
        };
        player.load_resting(cx);
        player
    }

    fn load_resting(&mut self, cx: &mut Context<Self>) {
        let Some(model) = LottieView::make_player(&self.name, cx) else {
            return;
        };
        let dimension = self.dimension;
        let rasterizing = cx.background_spawn(async move { rasterize_frame(&model, dimension, 0) });
        self._work = cx.spawn(async move |player, cx| {
            let Some(resting) = rasterizing.await else {
                return;
            };
            let _ = player.update(cx, |player, cx| {
                if let Some(previous) = player.resting.replace(resting.clone()) {
                    cx.drop_image(previous, None);
                }
                player.show(Some(resting), cx);
            });
        });
    }

    /// stops the run and shows the resting icon
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.resting.is_none() {
            return;
        }
        self._work = Task::ready(());
        let resting = self.resting.clone();
        self.show(resting, cx);
    }

    /// one run from frame zero
    /// completion gets true when the run reached its last frame
    /// and never runs for a run stopped or replaced first
    pub fn play(&mut self, completion: impl FnOnce(bool, &mut App) + 'static, cx: &mut Context<Self>) {
        let Some(model) = LottieView::make_player(&self.name, cx) else {
            return;
        };
        let dimension = self.dimension;
        let rasterizing = cx.background_spawn(async move { rasterize_run(&model, dimension) });
        self._work = cx.spawn(async move |player, cx| {
            let Some(run) = rasterizing.await else {
                return;
            };
            let duration = run.duration;
            let shown = player.update(cx, |player, cx| player.show(Some(run.image), cx));
            if shown.is_err() {
                return;
            }
            cx.background_executor().timer(duration).await;
            let finished = player.update(cx, |player, cx| player.show(run.last_frame, cx));
            if finished.is_ok() {
                cx.update(|cx| completion(true, cx));
            }
        });
    }

    fn show(&mut self, image: Option<Arc<RenderImage>>, cx: &mut Context<Self>) {
        let keeps = |candidate: &Arc<RenderImage>, resting: &Option<Arc<RenderImage>>| {
            resting.as_ref().is_some_and(|resting| Arc::ptr_eq(resting, candidate))
        };
        if let Some(previous) = std::mem::replace(&mut self.image, image)
            && !keeps(&previous, &self.resting)
            && !self.image.as_ref().is_some_and(|current| Arc::ptr_eq(current, &previous))
        {
            cx.drop_image(previous, None);
        }
        cx.notify();
    }
}

struct RasterizedRun {
    image: Arc<RenderImage>,
    last_frame: Option<Arc<RenderImage>>,
    duration: Duration,
}

fn rasterize_frame(model: &LottieViewModel, dimension: u32, index: usize) -> Option<Arc<RenderImage>> {
    let sequence = open_sequence(model, dimension)?;
    let frame = sequence.frame(index.min(sequence.frame_count().saturating_sub(1)));
    let image = frame.and_then(|frame| single_frame_image(&frame, Delay::from_saturating_duration(Duration::from_secs(1))));
    sequence.teardown();
    image
}

fn rasterize_run(model: &LottieViewModel, dimension: u32) -> Option<RasterizedRun> {
    let sequence = open_sequence(model, dimension)?;
    let stride = sequence.presentation_stride().max(1);
    let rate = sequence.frame_rate().max(1.0);
    let delay = Delay::from_saturating_duration(Duration::from_secs_f64((stride as f64 / rate).clamp(0.001, 1.0)));
    let mut frames: SmallVec<[Frame; 1]> = SmallVec::new();
    let mut last = None;
    for index in (0..sequence.frame_count()).step_by(stride) {
        let Some(decoded) = sequence.frame(index) else {
            sequence.teardown();
            return None;
        };
        let Some(buffer) = frame_buffer(&decoded) else {
            sequence.teardown();
            return None;
        };
        frames.push(Frame::from_parts(buffer, 0, 0, delay));
        last = Some(decoded);
    }
    sequence.teardown();
    let last = last?;
    let duration = Duration::from(delay) * u32::try_from(frames.len()).unwrap_or(u32::MAX);
    Some(RasterizedRun {
        image: Arc::new(RenderImage::new(frames)),
        last_frame: single_frame_image(&last, delay),
        duration,
    })
}

fn open_sequence(model: &LottieViewModel, dimension: u32) -> Option<FrameSequence> {
    let rasterizer = model.open_rasterizer()?;
    let key = SequenceKey::new(model.name().to_string(), dimension);
    let cache = Arc::new(DecodedFrameCache::new(raster_policy::MAXIMUM_BYTES_PER_SEQUENCE));
    FrameSequence::open(key, rasterizer, cache).ok()
}

fn frame_buffer(decoded: &DecodedFrame) -> Option<RgbaImage> {
    RgbaImage::from_raw(decoded.dimension, decoded.dimension, decoded.bytes().to_vec())
}

fn single_frame_image(decoded: &DecodedFrame, delay: Delay) -> Option<Arc<RenderImage>> {
    let frame = Frame::from_parts(frame_buffer(decoded)?, 0, 0, delay);
    let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([frame]);
    Some(Arc::new(RenderImage::new(frames)))
}

struct PurchaseCelebrationDirectors {
    sparks: Entity<PurchaseCelebrationDirector>,
    premium: Entity<PurchaseCelebrationDirector>,
}

impl Global for PurchaseCelebrationDirectors {}

/// starts both copies once the sheet copy is in a window and laid out
/// keeps the upper copy on the sheet icon through slide, drags and dismissal
/// and takes the layer down when the burst ends or the sheet goes
pub struct PurchaseCelebrationDirector {
    pub kind: PurchaseCelebration,
    sheet_player: Option<WeakEntity<PurchaseCelebrationPlayer>>,
    sheet_canvas: Option<(AnyWindowHandle, Bounds<Pixels>)>,
    sheet_frame: Option<Bounds<Pixels>>,
    overlay: Option<PurchaseCelebrationOverlay>,
    armed: bool,
    run: u64,
}

impl PurchaseCelebrationDirector {
    fn new(kind: PurchaseCelebration) -> Self {
        Self {
            kind,
            sheet_player: None,
            sheet_canvas: None,
            sheet_frame: None,
            overlay: None,
            armed: false,
            run: 0,
        }
    }

    pub fn of(kind: PurchaseCelebration, cx: &mut App) -> Entity<Self> {
        if !cx.has_global::<PurchaseCelebrationDirectors>() {
            let directors = PurchaseCelebrationDirectors {
                sparks: cx.new(|_| Self::new(PurchaseCelebration::Sparks)),
                premium: cx.new(|_| Self::new(PurchaseCelebration::Premium)),
            };
            cx.set_global(directors);
        }
        let directors = cx.global::<PurchaseCelebrationDirectors>();
        match kind {
            PurchaseCelebration::Sparks => directors.sparks.clone(),
            PurchaseCelebration::Premium => directors.premium.clone(),
        }
    }

    pub fn sparks(cx: &mut App) -> Entity<Self> {
        Self::of(PurchaseCelebration::Sparks, cx)
    }

    pub fn premium(cx: &mut App) -> Entity<Self> {
        Self::of(PurchaseCelebration::Premium, cx)
    }

    fn is_sheet_player(&self, player: &Entity<PurchaseCelebrationPlayer>) -> bool {
        self.sheet_player
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .is_some_and(|current| current == *player)
    }

    /// success sheet is opening and its canvas plays once, with motion off the icon stays still
    pub fn arm(&mut self, cx: &mut Context<Self>) {
        self.finish(cx);
        self.run = self.run.wrapping_add(1);
        self.armed = PurchaseCelebration::allows_motion(cx);
        if !self.armed {
            PurchaseCelebrationHaptics::play_simple();
            return;
        }
        self.start_if_ready(cx);
    }

    pub fn attach(&mut self, player: &Entity<PurchaseCelebrationPlayer>, cx: &mut Context<Self>) {
        if !self.is_sheet_player(player) {
            self.sheet_player = Some(player.downgrade());
        }
        self.start_if_ready(cx);
    }

    pub fn detach(&mut self, player: &Entity<PurchaseCelebrationPlayer>, cx: &mut Context<Self>) {
        if !self.is_sheet_player(player) {
            return;
        }
        self.sheet_player = None;
        self.sheet_canvas = None;
        if self.armed {
            return;
        }
        self.finish(cx);
    }

    pub fn canvas_left_window(&mut self, player: &Entity<PurchaseCelebrationPlayer>, cx: &mut Context<Self>) {
        if !self.is_sheet_player(player) || self.overlay.is_none() {
            return;
        }
        self.overlay = None;
        self.run = self.run.wrapping_add(1);
        player.update(cx, |player, cx| player.stop(cx));
        self.sheet_frame = None;
        self.armed = PurchaseCelebration::allows_motion(cx);
        cx.notify();
    }

    pub fn sheet_canvas_did_layout(
        &mut self,
        player: &Entity<PurchaseCelebrationPlayer>,
        bounds: Bounds<Pixels>,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if !self.is_sheet_player(player) {
            return;
        }
        self.sheet_canvas = Some((window, bounds));
        self.start_if_ready(cx);
        let sheet_top = self.sheet_frame.map(|frame| frame.top());
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.follow(bounds, sheet_top);
            cx.notify();
        }
    }

    pub fn sheet_frame_changed(&mut self, frame: Option<Bounds<Pixels>>, cx: &mut Context<Self>) {
        self.sheet_frame = frame;
        let Some(frame) = frame else {
            self.finish(cx);
            return;
        };
        self.start_if_ready(cx);
        if let (Some(overlay), Some((_, canvas))) = (self.overlay.as_mut(), self.sheet_canvas) {
            overlay.follow(canvas, Some(frame.top()));
            cx.notify();
        }
    }

    pub fn finish(&mut self, cx: &mut Context<Self>) {
        self.armed = false;
        if self.overlay.take().is_some() {
            cx.notify();
        }
    }

    /// waits for no report of the sheet frame
    /// a sheet that appears in place or comes back after being taken down reports none
    /// and the burst still has to play
    fn start_if_ready(&mut self, cx: &mut Context<Self>) {
        if !self.armed {
            return;
        }
        let Some(sheet_player) = self.sheet_player.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        let Some((window, canvas)) = self.sheet_canvas else {
            return;
        };
        if canvas.size.width <= px(0.0) {
            return;
        }
        self.armed = false;
        let current = self.run;

        let mut overlay = PurchaseCelebrationOverlay::new(window);
        overlay.follow(canvas, self.sheet_frame.map(|frame| frame.top()));
        self.overlay = Some(overlay);
        let director = cx.weak_entity();
        sheet_player.update(cx, |player, cx| {
            player.stop(cx);
            player.play(
                move |_, cx| {
                    let _ = director.update(cx, |director, cx| {
                        if director.run != current {
                            return;
                        }
                        director.overlay = None;
                        cx.notify();
                    });
                },
                cx,
            );
        });
        PurchaseCelebrationHaptics::play();
        cx.notify();
    }
}

/// haptics of the celebrations
/// why: the mac manager plays only fixed patterns and linux and windows none, so no engine to keep
/// the plain success haptic is the whole answer
pub struct PurchaseCelebrationHaptics;

impl PurchaseCelebrationHaptics {
    fn supported() -> bool {
        haptics::availability().is_ok()
    }

    pub fn prepare() {
        if !Self::supported() {
            return;
        }
        let _ = haptics::prepare();
    }

    pub fn play() {
        if !Self::supported() {
            return;
        }
        Self::play_simple();
    }

    pub fn play_simple() {
        let _ = haptics::notification(NotificationKind::Success);
    }
}

/// upper copy, a layer of the sheet window just above every kit layer
/// cut off at the sheet top edge so it never draws over the sheet
/// no hitbox so every press reaches the app
#[derive(Clone)]
struct PurchaseCelebrationOverlay {
    window: AnyWindowHandle,
    frame: Bounds<Pixels>,
    cut: Pixels,
}

impl PurchaseCelebrationOverlay {
    const LEVEL: usize = 1001;

    fn new(window: AnyWindowHandle) -> Self {
        Self {
            window,
            frame: Bounds::default(),
            cut: px(0.0),
        }
    }

    /// puts the copy exactly where the sheet copy is in the window this frame
    /// without a reported sheet frame the cut goes where the canvas puts the sheet edge
    /// the resting icon hangs ICON_BELOW_SHEET_TOP under it
    fn follow(&mut self, on_screen: Bounds<Pixels>, sheet_top: Option<Pixels>) {
        let scale = f32::from(on_screen.size.height) / PurchaseCelebration::CANVAS_SIDE;
        let sheet_top = sheet_top.unwrap_or_else(|| {
            on_screen.top()
                + px((f32::from(PurchaseCelebration::ICON_ON_CANVAS.y) - PurchaseCelebration::ICON_BELOW_SHEET_TOP) * scale)
        });
        self.frame = on_screen;
        self.cut = sheet_top.max(px(0.0));
    }

    fn layer(&self, kind: PurchaseCelebration, image: Arc<RenderImage>, viewport: gpui::Size<Pixels>) -> AnyElement {
        let id = ElementId::NamedInteger(
            SharedString::from(format!("purchase_celebration_upper_copy/{}", kind.animation())),
            image.id.0 as u64,
        );
        deferred(
            anchored().position(point(px(0.0), px(0.0))).child(
                div().w(viewport.width).h(self.cut).overflow_hidden().relative().child(
                    div()
                        .absolute()
                        .left(self.frame.left())
                        .top(self.frame.top())
                        .w(self.frame.size.width)
                        .h(self.frame.size.height)
                        .child(img(image).id(id).w(self.frame.size.width).h(self.frame.size.height)),
                ),
            ),
        )
        .with_priority(Self::LEVEL)
        .into_any_element()
    }
}
