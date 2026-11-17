#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, BoxShadow, Context, ElementId, Entity, FontWeight, Hsla, SharedString, Subscription, Task, Window, black,
    point, rgb, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::file_dialog::{self, FileFilter, SaveOutcome, SaveRequest};
use rise_platform::haptics::{ImpactStyle, impact};
use rise_platform::reveal_in_file_manager::reveal;
use rise_ui::IconUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::motion::{MotionSpec, ease_in, ease_out, spring_blended};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_debugger::sai_debugger_overlay_state::SaiFloatingDebugMotionPolicy;
use crate::core::lib::sai::modules::sai_debugger::sai_performance_monitor::{
    SaiPerformanceLiveStats, SaiPerformanceMonitor,
};
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::navigations::main_tab_bar::MainTabBarMetrics;

const HUD_WIDTH: f32 = 232.;
const HUD_HEIGHT: f32 = 154.;
const HUD_RADIUS: f32 = 22.;
const SYSTEM_RED: u32 = 0xff3b30;
const SYSTEM_GREEN: u32 = 0x34c759;
const SYSTEM_ORANGE: u32 = 0xff9500;
const FPS_GOOD: u32 = 0x4ade80;
const FPS_FAIR: u32 = 0xfacc15;
const FPS_POOR: u32 = 0xfb7185;
const LOG_NAME: &str = "SaiPerformanceHUD";

type DismissAction = Rc<dyn Fn(&mut Window, &mut App)>;
type ShareAction = Rc<dyn Fn(PathBuf, &mut App)>;

/// initial placement of the floating debug surface
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaiFloatingDebugInitialPlacement {
    TopLeading,
    BottomTrailing,
    Center,
    CenterTrailing,
    BottomLeading,
}

fn named(base: &ElementId, part: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("{base}-{part}")))
}

fn stop_recording_and_share(monitor: &Entity<SaiPerformanceMonitor>, reason: &str, on_share: ShareAction, cx: &mut App) {
    monitor.update(cx, |monitor, cx| {
        monitor.stop_recording(
            reason,
            Some(Box::new(move |url: Option<PathBuf>, cx: &mut App| {
                if let Some(url) = url {
                    on_share(url, cx);
                }
            })),
            cx,
        );
    });
}

struct SaiPerformanceHudState {
    monitor: Entity<SaiPerformanceMonitor>,
    _monitor_changes: Subscription,
}

impl SaiPerformanceHudState {
    fn new(cx: &mut Context<Self>) -> Self {
        let monitor = SaiPerformanceMonitor::shared(cx);
        let monitor_changes = cx.observe(&monitor, |_, _, cx| cx.notify());
        Self { monitor, _monitor_changes: monitor_changes }
    }
}

/// performance hud, the share callback takes only the app
/// the monitor finishes a recording off any window so it has none to pass
#[derive(IntoElement)]
pub struct SaiPerformanceHUD {
    id: ElementId,
    on_dismiss: DismissAction,
    on_share: ShareAction,
}

impl SaiPerformanceHUD {
    pub fn new(
        id: impl Into<ElementId>,
        on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
        on_share: impl Fn(PathBuf, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), on_dismiss: Rc::new(on_dismiss), on_share: Rc::new(on_share) }
    }

    fn hud_content(&self, monitor: &Entity<SaiPerformanceMonitor>, cx: &App) -> Div {
        let current = monitor.read(cx);
        let stats = &current.stats;
        let is_recording = current.is_recording;
        let share_url = current
            .latest_trace_url
            .clone()
            .filter(|_| !current.is_recording && !current.is_preparing_recording && !current.is_finishing);
        let share = share_url.map(|url| {
            let on_share = self.on_share.clone();
            plain(named(&self.id, "share"))
                .on_press(move |_window, cx| on_share(url.clone(), cx))
                .child(
                    div()
                        .id(named(&self.id, "share-label"))
                        .tooltip_label("Share performance trace")
                        .flex()
                        .items_center()
                        .justify_center()
                        .frame_size(28., 28.)
                        .children(IconUi::sized("square.and.arrow.up", px(13.), white().opacity(0.86), false)),
                )
        });

        let top = hstack(8.)
            .w_full()
            .child(
                hstack(6.)
                    .child(
                        div()
                            .frame_size(7., 7.)
                            .bg(rgb(if is_recording { SYSTEM_RED } else { SYSTEM_GREEN }))
                            .rounded_full(),
                    )
                    .child(
                        div()
                            .child(if is_recording { Self::elapsed(stats.recording_duration) } else { "PERF".to_owned() })
                            .swiftui_font(Font::system(11., FontWeight::BOLD, FontDesign::Monospaced))
                            .text_color(white().opacity(0.84)),
                    ),
            )
            .child(spacer_min_w(0.))
            .children(share)
            .child(self.record_button(monitor, cx));

        let fps = hstack(7.)
            .align(VerticalAlignment::FirstTextBaseline)
            .w_full()
            .child(
                div()
                    .child(format!("{:.0}", stats.fps.round()))
                    .swiftui_font(Font::system(35., FontWeight::BLACK, FontDesign::Rounded))
                    .text_color(Self::fps_color(stats)),
            )
            .child(
                div()
                    .child("FPS")
                    .swiftui_font(Font::system(12., FontWeight::BOLD, FontDesign::Rounded))
                    .text_color(white().opacity(0.64)),
            )
            .child(spacer_min_w(0.))
            .child(
                div()
                    .child(format!("{} Hz", stats.maximum_fps))
                    .swiftui_font(Font::system(11., FontWeight::SEMIBOLD, FontDesign::Monospaced))
                    .text_color(white().opacity(0.5)),
            );

        let metrics = hstack(5.)
            .w_full()
            .child(Self::metric("DROP", stats.missed_frames.to_string()))
            .child(Self::metric("P95", format!("{}ms", stats.p95_frame_ms.round() as i64)))
            .child(Self::metric("WORST", format!("{}ms", stats.worst_frame_ms.round() as i64)));

        let footer_color = white().opacity(0.58);
        let footer = hstack(7.)
            .w_full()
            .swiftui_font(Font::system(9., FontWeight::SEMIBOLD, FontDesign::Monospaced))
            .text_color(footer_color)
            .child(Self::label(format!("{} MB", stats.memory_mb), "memorychip", footer_color))
            .child(spacer_min_w(0.))
            .child(Self::label(stats.thermal_state.to_uppercase(), "thermometer.medium", footer_color))
            .when(stats.slow_main_thread_turns > 0, |footer| {
                footer.child(
                    div().child(format!("MAIN {}", stats.slow_main_thread_turns)).text_color(rgb(SYSTEM_ORANGE)),
                )
            });

        let (stroke_color, stroke_width): (Hsla, f32) = if is_recording {
            (Hsla::from(rgb(SYSTEM_RED)).opacity(0.65), 1.5)
        } else {
            (white().opacity(0.14), 1.)
        };

        div()
            .relative()
            .frame_size(HUD_WIDTH, HUD_HEIGHT)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(black().opacity(0.91))
                    .clip_shape(Shape::continuous(HUD_RADIUS))
                    .shadow(vec![BoxShadow {
                        color: black().opacity(0.42),
                        offset: point(px(0.), px(8.)),
                        blur_radius: px(16.),
                        spread_radius: px(0.),
                        inset: false,
                    }]),
            )
            .child(centred_stroke(stroke_color, stroke_width, Shape::continuous(HUD_RADIUS)))
            .child(
                vstack(10.)
                    .relative()
                    .size_full()
                    .p(px(13.))
                    .child(top)
                    .child(fps)
                    .child(metrics)
                    .child(footer),
            )
    }

    fn record_button(&self, monitor: &Entity<SaiPerformanceMonitor>, cx: &App) -> AnyElement {
        let current = monitor.read(cx);
        if current.is_preparing_recording || current.is_finishing {
            return div()
                .id(named(&self.id, "processing"))
                .tooltip_label("Processing performance trace")
                .flex()
                .items_center()
                .justify_center()
                .frame_size(30., 30.)
                .child(progress_view().control_size(ControlSize::Small).tint(white()))
                .into_any_element();
        }
        let is_recording = current.is_recording;
        let inner = if is_recording {
            div().frame_size(11., 11.).bg(rgb(SYSTEM_RED)).clip_shape(Shape::continuous(3.))
        } else {
            div().frame_size(13., 13.).bg(rgb(SYSTEM_RED)).rounded_full()
        };
        let toggle_monitor = monitor.clone();
        let on_share = self.on_share.clone();
        plain(named(&self.id, "record"))
            .on_press(move |window, cx| Self::toggle_recording(&toggle_monitor, on_share.clone(), window, cx))
            .child(
                div()
                    .id(named(&self.id, "record-label"))
                    .tooltip_label(if is_recording {
                        "Stop performance recording"
                    } else {
                        "Start performance recording"
                    })
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .frame_size(30., 30.)
                    .rounded_full()
                    .child(centred_stroke(white().opacity(0.22), 1., Shape::Circle))
                    .child(inner),
            )
            .into_any_element()
    }

    fn metric(title: &'static str, value: String) -> Div {
        vstack(2.)
            .flex_1()
            .child(
                div()
                    .child(title)
                    .swiftui_font(Font::system(8., FontWeight::BOLD, FontDesign::Monospaced))
                    .text_color(white().opacity(0.42)),
            )
            .child(
                div()
                    .child(value)
                    .swiftui_font(Font::system(11., FontWeight::BOLD, FontDesign::Monospaced))
                    .text_color(white().opacity(0.88)),
            )
            .py(px(5.))
            .bg(white().opacity(0.07))
            .clip_shape(Shape::continuous(8.))
    }

    fn label(text: String, symbol: &str, color: Hsla) -> Div {
        hstack(4.).children(IconUi::sized(symbol, px(9.), color, false)).child(div().child(text))
    }

    fn fps_color(stats: &SaiPerformanceLiveStats) -> Hsla {
        if stats.fps <= 0. {
            return white().opacity(0.72);
        }
        let ratio = stats.fps / stats.maximum_fps.max(1) as f64;
        if ratio >= 0.90 {
            return rgb(FPS_GOOD).into();
        }
        if ratio >= 0.70 {
            return rgb(FPS_FAIR).into();
        }
        rgb(FPS_POOR).into()
    }

    fn toggle_recording(
        monitor: &Entity<SaiPerformanceMonitor>,
        on_share: ShareAction,
        window: &mut Window,
        cx: &mut App,
    ) {
        let _ = impact(ImpactStyle::Medium);
        if monitor.read(cx).is_recording {
            stop_recording_and_share(monitor, "manual", on_share, cx);
        } else {
            monitor.update(cx, |monitor, cx| monitor.start_recording(window, cx));
        }
    }

    fn elapsed(interval: f64) -> String {
        let seconds = interval.floor().max(0.) as i64;
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

impl RenderOnce for SaiPerformanceHUD {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(named(&self.id, "state"), cx, |_, cx| SaiPerformanceHudState::new(cx));
        let monitor = state.read(cx).monitor.clone();

        let appear_monitor = monitor.clone();
        let disappear_monitor = monitor.clone();
        let content = self
            .hud_content(&monitor, cx)
            .on_appear(named(&self.id, "appear"), move |window, cx| {
                appear_monitor.update(cx, |monitor, cx| monitor.hud_did_appear(window, cx));
            })
            .on_disappear(named(&self.id, "disappear"), move |_window, cx| {
                disappear_monitor.update(cx, |monitor, cx| monitor.hud_did_disappear(cx));
            });

        let on_dismiss = self.on_dismiss.clone();
        let on_share = self.on_share.clone();
        SaiFloatingDebugSurface::new(
            named(&self.id, "surface"),
            (HUD_WIDTH, HUD_HEIGHT),
            SaiFloatingDebugInitialPlacement::TopLeading,
            HUD_RADIUS,
            move |window, cx| {
                let current = monitor.read(cx);
                if current.is_preparing_recording || current.is_recording {
                    stop_recording_and_share(&monitor, "hud.dismissed", on_share.clone(), cx);
                }
                on_dismiss(window, cx);
            },
            content,
        )
    }
}

struct SaiFloatingDebugSurfaceState {
    has_center: bool,
    center_x: Animated<f32>,
    center_y: Animated<f32>,
    drag_origin: (f32, f32),
    drag_width: Animated<f32>,
    is_dragging: bool,
    is_dismissing: bool,
    container: (f32, f32),
    size: (f32, f32),
    initial_placement: SaiFloatingDebugInitialPlacement,
    _dismiss_task: Option<Task<()>>,
}

impl SaiFloatingDebugSurfaceState {
    fn new(size: (f32, f32), initial_placement: SaiFloatingDebugInitialPlacement) -> Self {
        Self {
            has_center: false,
            center_x: Animated::new(0.),
            center_y: Animated::new(0.),
            drag_origin: (0., 0.),
            drag_width: Animated::new(0.),
            is_dragging: false,
            is_dismissing: false,
            container: (0., 0.),
            size,
            initial_placement,
            _dismiss_task: None,
        }
    }

    fn resolved_center(&self) -> (f32, f32) {
        if self.has_center {
            let now = RiseClock::monotonic();
            (self.center_x.value(now), self.center_y.value(now))
        } else {
            self.initial_center()
        }
    }

    fn initial_center(&self) -> (f32, f32) {
        let (width, height) = self.size;
        let (container_width, container_height) = self.container;
        match self.initial_placement {
            SaiFloatingDebugInitialPlacement::TopLeading => (width / 2. + 14., height / 2. + 14.),
            SaiFloatingDebugInitialPlacement::BottomTrailing => {
                (container_width - width / 2. - 18., container_height - height / 2. - 24.)
            }
            SaiFloatingDebugInitialPlacement::Center => (container_width / 2., container_height / 2.),
            SaiFloatingDebugInitialPlacement::CenterTrailing => {
                (container_width - width / 2. - 18., container_height / 2.)
            }
            SaiFloatingDebugInitialPlacement::BottomLeading => {
                let rail_outer_inset = 6.;
                let foot = MainTabBarMetrics::RAIL_ITEM_HEIGHT * 2. + MainTabBarMetrics::INSET * 3.;
                (51., container_height - rail_outer_inset - foot - MainTabBarMetrics::INSET * 2. - height / 2.)
            }
        }
    }

    fn clamped(&self, (x, y): (f32, f32)) -> (f32, f32) {
        let (width, height) = self.size;
        let (container_width, container_height) = self.container;
        let min_x = width / 2. + 8.;
        let max_x = container_width - width / 2. - 8.;
        let min_y = height / 2. + 8.;
        let max_y = container_height - height / 2. - 8.;
        (x.max(min_x).min(min_x.max(max_x)), y.max(min_y).min(min_y.max(max_y)))
    }

    fn rubber_band(&self, (x, y): (f32, f32)) -> (f32, f32) {
        let (width, height) = self.size;
        let (container_width, container_height) = self.container;
        (
            x.max(-width * 0.35).min(container_width + width * 0.35),
            y.max(height / 2.).min(container_height - height / 2.),
        )
    }

    fn jump_center(&mut self, (x, y): (f32, f32)) {
        self.has_center = true;
        self.center_x.jump(x);
        self.center_y.jump(y);
    }

    fn note_container(&mut self, container: (f32, f32)) -> bool {
        if self.container == container {
            return false;
        }
        self.container = container;
        if self.has_center {
            let center = self.clamped(self.resolved_center());
            self.jump_center(center);
        }
        true
    }
}

/// floating debug surface, content comes built as one element per frame
#[derive(IntoElement)]
pub struct SaiFloatingDebugSurface {
    id: ElementId,
    size: (f32, f32),
    initial_placement: SaiFloatingDebugInitialPlacement,
    corner_radius: f32,
    on_dismiss: DismissAction,
    content: AnyElement,
}

impl SaiFloatingDebugSurface {
    pub fn new(
        id: impl Into<ElementId>,
        size: (f32, f32),
        initial_placement: SaiFloatingDebugInitialPlacement,
        corner_radius: f32,
        on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
        content: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            size,
            initial_placement,
            corner_radius,
            on_dismiss: Rc::new(on_dismiss),
            content: content.into_any_element(),
        }
    }

    fn drag_gesture(state: Entity<SaiFloatingDebugSurfaceState>, on_dismiss: DismissAction) -> DragGesture {
        let changed_state = state.clone();
        DragGesture::new(3.)
            .on_changed(move |value, _window, cx| {
                changed_state.update(cx, |state, cx| {
                    if state.is_dismissing {
                        return;
                    }
                    if !state.is_dragging {
                        state.is_dragging = true;
                        state.drag_origin = state.resolved_center();
                    }
                    let width = f32::from(value.translation.width);
                    let height = f32::from(value.translation.height);
                    let raw_center = (state.drag_origin.0 + width, state.drag_origin.1 + height);
                    let center = state.rubber_band(raw_center);
                    state.jump_center(center);
                    state.drag_width.jump(width);
                    cx.notify();
                });
            })
            .on_ended(move |value, window, cx| {
                let reduce = reduce_motion(cx);
                let predicted_width = f32::from(value.predicted_end_translation.width);
                let predicted_height = f32::from(value.predicted_end_translation.height);
                let current = state.read(cx);
                if current.is_dismissing {
                    return;
                }
                let current_center = current.resolved_center();
                let predicted_center =
                    (current.drag_origin.0 + predicted_width, current.drag_origin.1 + predicted_height);
                let container_width = current.container.0;
                let surface_width = current.size.0;
                let should_dismiss = SaiFloatingDebugMotionPolicy::should_dismiss(
                    point(px(current_center.0), px(current_center.1)),
                    point(px(predicted_center.0), px(predicted_center.1)),
                    px(container_width),
                    px(surface_width),
                );
                if should_dismiss {
                    Self::dismiss(
                        &state,
                        predicted_center.0 < container_width / 2.,
                        reduce,
                        on_dismiss.clone(),
                        window,
                        cx,
                    );
                    return;
                }
                let inertial_center = if reduce {
                    current_center
                } else {
                    (
                        current_center.0 + (predicted_center.0 - current_center.0) * 0.34,
                        current_center.1 + (predicted_center.1 - current_center.1) * 0.24,
                    )
                };
                let spec: MotionSpec =
                    if reduce { ease_out(0.12) } else { spring_blended(0.56, 0.67, 0.18) };
                state.update(cx, |state, cx| {
                    state.is_dragging = false;
                    let now = RiseClock::monotonic();
                    let (x, y) = state.clamped(inertial_center);
                    state.center_x.set(x, spec, now, false);
                    state.center_y.set(y, spec, now, false);
                    state.drag_width.set(0., spec, now, false);
                    cx.notify();
                });
            })
    }

    fn dismiss(
        state: &Entity<SaiFloatingDebugSurfaceState>,
        toward_left: bool,
        reduce: bool,
        on_dismiss: DismissAction,
        window: &mut Window,
        cx: &mut App,
    ) {
        let spec: MotionSpec = if reduce { ease_in(0.12) } else { spring_blended(0.38, 0.72, 0.12) };
        let delay = Duration::from_secs_f64(if reduce { 0.13 } else { 0.38 });
        let task = window.spawn(cx, async move |cx| {
            cx.background_executor().timer(delay).await;
            let _ = cx.update(|window, cx| on_dismiss(window, cx));
        });
        state.update(cx, |state, cx| {
            state.is_dragging = false;
            state.is_dismissing = true;
            let center = state.resolved_center();
            state.jump_center(center);
            let target_x = if toward_left { -state.size.0 } else { state.container.0 + state.size.0 };
            let now = RiseClock::monotonic();
            state.center_x.set(target_x, spec, now, false);
            state.drag_width.set(if toward_left { -260. } else { 260. }, spec, now, false);
            state._dismiss_task = Some(task);
            cx.notify();
        });
    }
}

impl RenderOnce for SaiFloatingDebugSurface {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let size = self.size;
        let placement = self.initial_placement;
        let state = window.use_keyed_state(named(&self.id, "state"), cx, move |_, _| {
            SaiFloatingDebugSurfaceState::new(size, placement)
        });
        let reduce = reduce_motion(cx);
        let now = RiseClock::monotonic();

        let measure_state = state.clone();
        let reader = measure(named(&self.id, "geometry"), move |bounds, _window, cx| {
            let container = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            measure_state.update(cx, |state, cx| {
                if state.note_container(container) {
                    cx.notify();
                }
            });
        });

        let current = state.read(cx);
        let is_measured = current.container != (0., 0.);
        let center = if current.has_center {
            (current.center_x.frame_value(now, window), current.center_y.frame_value(now, window))
        } else {
            current.initial_center()
        };
        let drag_width = current.drag_width.frame_value(now, window);
        let blocks_presses = current.is_dragging || current.is_dismissing;

        let (scale_x, scale_y, degrees) = if reduce {
            (1., 1., 0.)
        } else {
            (
                1. + 0.065_f32.min(drag_width.abs() / 1_700.),
                1. - 0.045_f32.min(drag_width.abs() / 2_000.),
                drag_width / 45.,
            )
        };

        let content = div()
            .relative()
            .frame_size(size.0, size.1)
            .child(self.content)
            .when(blocks_presses, |content| {
                content.child(hit_area(Shape::continuous(self.corner_radius)).occlude())
            })
            .rotation_effect(degrees)
            .stretched(scale_x, scale_y)
            .gesture(named(&self.id, "drag"), Self::drag_gesture(state.clone(), self.on_dismiss.clone()));

        pass_through().absolute().inset_0().child(reader).when(is_measured, |surface| {
            surface.child(
                div()
                    .absolute()
                    .left(px(center.0 - size.0 / 2.))
                    .top(px(center.1 - size.1 / 2.))
                    .child(content),
            )
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiPerformanceShareItem {
    pub id: Uuid,
    pub url: PathBuf,
}

impl SaiPerformanceShareItem {
    pub fn new(url: PathBuf) -> Self {
        Self { id: Uuid::new_v4(), url }
    }
}

/// performance share sheet, the activity sheet is the os save panel here
/// the saved report is then shown in the file manager
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiPerformanceShareSheet {
    pub url: PathBuf,
}

impl SaiPerformanceShareSheet {
    pub fn new(url: PathBuf) -> Self {
        Self { url }
    }

    fn is_same_file(source: &Path, destination: &Path) -> bool {
        match (source.canonicalize(), destination.canonicalize()) {
            (Ok(source), Ok(destination)) => source == destination,
            _ => false,
        }
    }

    pub fn present(&self, cx: &mut App) {
        let url = self.url.clone();
        let directory = url.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut request = SaveRequest::new(directory, FileFilter::any());
        if let Some(name) = url.file_name().and_then(|name| name.to_str()) {
            request = request.with_suggested_name(name);
        }
        let outcome = file_dialog::save(cx, &request);
        cx.spawn(async move |cx| {
            let target = match outcome.await {
                SaveOutcome::Chosen(target) => target,
                SaveOutcome::Cancelled => return,
                SaveOutcome::Failed(error) => {
                    let message = error.to_string();
                    let _ = cx.update(|cx| Sai::log(LOG_NAME, move || message, SaiLogKind::Debug, cx));
                    return;
                }
            };
            let destination = target.clone();
            let copied = cx
                .background_executor()
                .spawn(async move {
                    if Self::is_same_file(&url, &destination) {
                        return Ok(0);
                    }
                    std::fs::copy(&url, &destination)
                })
                .await;
            let _ = cx.update(|cx| match copied {
                Ok(_) => {
                    if let Err(error) = reveal(cx, &target) {
                        let message = error.to_string();
                        Sai::log(LOG_NAME, move || message, SaiLogKind::Debug, cx);
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    Sai::log(LOG_NAME, move || message, SaiLogKind::Debug, cx);
                }
            });
        })
        .detach();
    }
}
