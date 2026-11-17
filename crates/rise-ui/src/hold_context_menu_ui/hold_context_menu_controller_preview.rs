use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyView, App, Bounds, ElementId, InteractiveElement, IntoElement, MouseDownEvent, MouseUpEvent, ParentElement,
    Pixels, Point, RenderOnce, Size, StatefulInteractiveElement, Styled, Window, div, px, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::ThemeStore;
use rise_theme::ui_color_system::UIColor;

use crate::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use crate::swiftui::animated::{Animated, MotionTime};
use crate::swiftui::environment::reduce_motion;
use crate::swiftui::motion::{MotionSpec, Spring};
use crate::view_anchor_probe::{ViewAnchorProbe, ViewAnchorProbeExt};

pub type HoldContextMenuPreviewAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// a live view can leave the menu and become the destination itself
pub struct HoldContextMenuControllerPreview {
    pub controller: AnyView,
    size: Size<Pixels>,
    menu_maximum_height: Pixels,
    pub on_expand: HoldContextMenuPreviewAction,
    pub on_commit: Rc<dyn Fn(HoldContextMenuPreviewAction, &mut Window, &mut App)>,
    pub source_frame: Bounds<Pixels>,
    pub surface: ViewAnchorProbe,
    available_height: f32,
    pub layout_width: f32,
}

impl HoldContextMenuControllerPreview {
    const MAXIMUM_HEIGHT_FRACTION: f32 = 0.62;
    const MINIMUM_HEIGHT_FRACTION: f32 = 0.25;

    /// builds the preview, a caller with nothing to expand passes a no op on_expand
    pub fn new(
        controller: AnyView,
        available_size: Size<Pixels>,
        on_expand: HoldContextMenuPreviewAction,
        on_commit: Rc<dyn Fn(HoldContextMenuPreviewAction, &mut Window, &mut App)>,
    ) -> Self {
        let available_width = f32::from(available_size.width);
        let available_height = f32::max(1.0, f32::from(available_size.height));
        let width = f32::min(420.0, f32::max(1.0, available_width - 32.0));
        let height = f32::max(1.0, f32::from(available_size.height) * Self::MAXIMUM_HEIGHT_FRACTION);
        let menu_maximum_height =
            f32::max(1.0, f32::from(available_size.height) - height - HoldContextMenuMetrics::MENU_TO_SOURCE_GAP);
        Self {
            controller,
            size: size(px(width), px(height)),
            menu_maximum_height: px(menu_maximum_height),
            on_expand,
            on_commit,
            source_frame: Bounds::default(),
            surface: ViewAnchorProbe::new(),
            available_height,
            layout_width: f32::max(1.0, available_width),
        }
    }

    pub fn size(&self) -> Size<Pixels> {
        self.size
    }

    pub fn menu_maximum_height(&self) -> Pixels {
        self.menu_maximum_height
    }

    pub fn fit_menu(&mut self, estimated_height: f32) {
        let gap = HoldContextMenuMetrics::MENU_TO_SOURCE_GAP;
        let height = f32::min(
            self.available_height * Self::MAXIMUM_HEIGHT_FRACTION,
            f32::max(
                self.available_height * Self::MINIMUM_HEIGHT_FRACTION,
                self.available_height - estimated_height - gap,
            ),
        );
        self.size.height = px(f32::max(1.0, height));
        self.menu_maximum_height = px(f32::max(1.0, self.available_height - f32::from(self.size.height) - gap));
    }

    pub fn contains_touch(&self, point: Point<Pixels>) -> bool {
        self.surface.window_frame().is_some_and(|frame| frame.contains(&point))
    }
}

struct HoldContextMenuControllerPreviewHostController {
    frame: Animated<Bounds<Pixels>>,
    corner_radius: Animated<f32>,
    alpha: Animated<f32>,
    generation: u64,
    target_frame: Option<Bounds<Pixels>>,
    phase: i64,
    settle_generation: Option<u64>,
}

impl HoldContextMenuControllerPreviewHostController {
    fn new() -> Self {
        Self {
            frame: Animated::new(Bounds::default()),
            corner_radius: Animated::new(28.0),
            alpha: Animated::new(0.0),
            generation: 0,
            target_frame: None,
            phase: -1,
            settle_generation: None,
        }
    }

    fn update(&mut self, frame: Bounds<Pixels>, phase: i64, now: MotionTime, reduce_motion: bool) {
        if self.target_frame == Some(frame) && self.phase == phase {
            return;
        }
        let first = self.phase == -1;
        self.phase = phase;
        self.target_frame = Some(frame);
        self.generation += 1;
        let animated = !first && !reduce_motion;
        let corner_radius = if phase == 3 { 0.0 } else { 28.0 };
        let alpha = if phase == 0 || phase == 2 { 0.0 } else { 1.0 };
        if animated {
            let response = if phase == 3 { 0.46 } else { 0.36 };
            let spec = MotionSpec::spring(Spring::new(response, 0.88));
            self.frame.set(frame, spec, now, false);
            self.corner_radius.set(corner_radius, spec, now, false);
            self.alpha.set(alpha, spec, now, false);
        } else {
            self.frame.jump(frame);
            self.corner_radius.jump(corner_radius);
            self.alpha.jump(alpha);
        }
        self.settle_generation = (phase >= 2).then_some(self.generation);
    }

    fn cancel(&mut self) {
        self.generation += 1;
        self.settle_generation = None;
    }

    fn is_running(&self, now: MotionTime) -> bool {
        self.frame.is_running(now) || self.corner_radius.is_running(now) || self.alpha.is_running(now)
    }
}

/// phase 0 pressed row, 1 preview 2 returning 3 entering the destination
#[derive(IntoElement)]
pub struct HoldContextMenuControllerPreviewHost {
    id: ElementId,
    pub preview: Rc<RefCell<HoldContextMenuControllerPreview>>,
    pub frame: Bounds<Pixels>,
    pub phase: i64,
    pub on_commit: HoldContextMenuPreviewAction,
    pub on_settled: HoldContextMenuPreviewAction,
    pub on_backdrop_tap: HoldContextMenuPreviewAction,
}

impl HoldContextMenuControllerPreviewHost {
    /// id keys the surface animation, which lives as long as the host is drawn
    /// frame is in the host space which covers the window
    pub fn new(
        id: impl Into<ElementId>,
        preview: Rc<RefCell<HoldContextMenuControllerPreview>>,
        frame: Bounds<Pixels>,
        phase: i64,
        on_commit: HoldContextMenuPreviewAction,
        on_settled: HoldContextMenuPreviewAction,
        on_backdrop_tap: HoldContextMenuPreviewAction,
    ) -> Self {
        Self { id: id.into(), preview, frame, phase, on_commit, on_settled, on_backdrop_tap }
    }
}

impl RenderOnce for HoldContextMenuControllerPreviewHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let reduce_motion = reduce_motion(cx);
        let controller = window.use_keyed_state(self.id.clone(), cx, |_, _| {
            HoldContextMenuControllerPreviewHostController::new()
        });
        let (frame, corner_radius, alpha, settled, target_size) = controller.update(cx, |host, _| {
            host.update(self.frame, self.phase, now, reduce_motion);
            let settled = match host.settle_generation {
                Some(generation) if generation == host.generation && !host.is_running(now) => {
                    host.settle_generation = None;
                    true
                }
                _ => false,
            };
            (
                host.frame.frame_value(now, window),
                host.corner_radius.frame_value(now, window),
                host.alpha.frame_value(now, window),
                settled,
                host.target_frame.map_or(self.frame.size, |target| target.size),
            )
        });
        if settled {
            let on_settled = self.on_settled.clone();
            window.defer(cx, move |window, cx| on_settled(window, cx));
        }

        let (child, preview_size, probe) = {
            let preview = self.preview.borrow();
            (preview.controller.clone(), preview.size, preview.surface.clone())
        };
        let content_size = match self.phase {
            0 | 2 => preview_size,
            3 => target_size,
            _ => frame.size,
        };
        let page = div().relative().w(content_size.width).h(content_size.height).child(child);
        let appearance = ThemeStore::shared(cx).read(cx).current_theme().appearance;

        let phase = self.phase;
        let on_commit = self.on_commit.clone();
        let surface = div()
            .absolute()
            .left(frame.origin.x)
            .top(frame.origin.y)
            .w(frame.size.width)
            .h(frame.size.height)
            .rounded(px(corner_radius))
            .overflow_hidden()
            .bg(UIColor::system_background().resolved_color(appearance))
            .opacity(alpha)
            .child(page)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(px(corner_radius))
                    .border(px(1.0))
                    .border_color(white().opacity(HoldContextMenuMetrics::PANEL_STROKE_OPACITY as f32)),
            )
            .view_anchor_probe(&probe);

        let root = div().id(self.id).absolute().inset_0();
        if alpha <= 0.01 {
            return root.child(surface);
        }
        let surface = surface
            .capture_any_mouse_down(|_: &MouseDownEvent, _, cx| cx.stop_propagation())
            .capture_any_mouse_up(move |_: &MouseUpEvent, window, cx| {
                cx.stop_propagation();
                if phase == 1 {
                    on_commit(window, cx);
                }
            });
        let on_backdrop_tap = self.on_backdrop_tap.clone();
        root.on_click(move |_, window, cx| on_backdrop_tap(window, cx)).child(surface)
    }
}

impl Drop for HoldContextMenuControllerPreviewHostController {
    fn drop(&mut self) {
        self.cancel();
    }
}
