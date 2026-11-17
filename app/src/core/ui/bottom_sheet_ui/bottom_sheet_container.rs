use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, BoxShadow, DispatchPhase, ElementId, Hitbox, HitboxBehavior, Hsla,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    RenderOnce, Size, Window, black, canvas, point, px, size,
};
use rise_theme::AppTheme;
use rise_ui::bottom_sheet_ui::bottom_sheet_handle::BottomSheetHandle;
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::BottomSheetHorizontalSafeInsetKey;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::bottom_sheet_ui::bottom_sheet_motion_host::BottomSheetMotionController;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui_kit_scroll::{
    BottomSheetUIKitScroll, BottomSheetUIKitScrollAction, BottomSheetUIKitScrollContent,
    BottomSheetUIKitScrollPullEnd,
};
use crate::core::ui::bottom_sheet_ui::sheet_floating_header::SheetSurfaceColorKey;

const VELOCITY_WINDOW: Duration = Duration::from_millis(100);

/// drag translation and velocity in global space
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BottomSheetDragValue {
    pub translation: Size<Pixels>,
    pub velocity: Size<Pixels>,
}

pub type BottomSheetDragGestureHandler = Rc<dyn Fn(&BottomSheetDragValue, &mut Window, &mut App)>;

/// drag gesture with min distance 0 in global space and its changed and ended hooks
/// driven on desktop by a left button drag
#[derive(Clone)]
pub struct BottomSheetDragGesture {
    pub on_changed: BottomSheetDragGestureHandler,
    pub on_ended: BottomSheetDragGestureHandler,
}

impl BottomSheetDragGesture {
    pub fn new(
        on_changed: impl Fn(&BottomSheetDragValue, &mut Window, &mut App) + 'static,
        on_ended: impl Fn(&BottomSheetDragValue, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { on_changed: Rc::new(on_changed), on_ended: Rc::new(on_ended) }
    }
}

struct DragTracking {
    origin: Point<Pixels>,
    location: Point<Pixels>,
    samples: Vec<(Instant, Point<Pixels>)>,
}

impl DragTracking {
    fn value(&self) -> BottomSheetDragValue {
        let translation = self.location - self.origin;
        let velocity = match (self.samples.first(), self.samples.last()) {
            (Some(first), Some(last)) => {
                let seconds = last.0.saturating_duration_since(first.0).as_secs_f32();
                if seconds <= f32::EPSILON {
                    size(px(0.0), px(0.0))
                } else {
                    size(
                        px(f32::from(last.1.x - first.1.x) / seconds),
                        px(f32::from(last.1.y - first.1.y) / seconds),
                    )
                }
            }
            _ => size(px(0.0), px(0.0)),
        };
        BottomSheetDragValue {
            translation: size(translation.x, translation.y),
            velocity,
        }
    }

    fn record(&mut self, location: Point<Pixels>, now: Instant) {
        self.location = location;
        self.samples.push((now, location));
        self.samples
            .retain(|(time, _)| now.saturating_duration_since(*time) <= VELOCITY_WINDOW);
    }
}

type DragTrackingCell = Rc<RefCell<Option<DragTracking>>>;

fn drag_gesture_overlay(gesture: BottomSheetDragGesture, tracking: DragTrackingCell) -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |_, hitbox: Hitbox, window, _| {
            let down = tracking.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                let now = cx.background_executor().now();
                *down.borrow_mut() = Some(DragTracking {
                    origin: event.position,
                    location: event.position,
                    samples: vec![(now, event.position)],
                });
            });

            let moved = tracking.clone();
            let moved_gesture = gesture.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || moved.borrow().is_none() {
                    return;
                }
                if event.pressed_button != Some(MouseButton::Left) {
                    let Some(ended) = moved.borrow_mut().take() else {
                        return;
                    };
                    (moved_gesture.on_ended)(&ended.value(), window, cx);
                    return;
                }
                let now = cx.background_executor().now();
                let value = {
                    let mut tracking = moved.borrow_mut();
                    let Some(tracking) = tracking.as_mut() else {
                        return;
                    };
                    tracking.record(event.position, now);
                    tracking.value()
                };
                (moved_gesture.on_changed)(&value, window, cx);
            });

            let up = tracking;
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                    return;
                }
                let Some(mut ended) = up.borrow_mut().take() else {
                    return;
                };
                let now = cx.background_executor().now();
                ended.record(event.position, now);
                (gesture.on_ended)(&ended.value(), window, cx);
            });
        },
    )
    .absolute()
    .inset_0()
}

pub type BottomSheetContainerContent = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// the sheet surface, every parameter is a public field filled in by the caller through the args
/// id keys the drag state of the surface
#[derive(IntoElement)]
pub struct BottomSheetContainer {
    pub id: ElementId,
    pub theme: AppTheme,
    pub full_page: bool,
    pub shows_handle: bool,
    pub presents_as_modal: bool,
    pub content_under_handle: bool,
    pub handle_over_content_is_light: bool,
    pub rounds_top_corners: bool,
    pub full_page_hides_footer: bool,
    pub hidden_footer_bottom_inset: Pixels,
    pub safe_top_inset: Pixels,
    pub horizontal_safe_inset: Pixels,
    pub sheet_background: Hsla,
    pub footer_background: Hsla,
    pub surface_shadow_enabled: bool,
    pub uses_liquid_glass_surface: bool,
    pub top_corner_radius: Pixels,
    pub footer_shows_top_border: bool,
    pub footer_overlays_content: bool,
    pub footer_bottom_padding: Pixels,
    pub footer_horizontal_padding: Option<Pixels>,
    pub footer_top_padding: Option<Pixels>,
    pub drag_gesture: BottomSheetDragGesture,
    pub host_owns_sheet_pan: bool,
    pub motion_controller: BottomSheetMotionController,
    pub is_sheet_interaction_locked: bool,
    pub on_sheet_drag_began: BottomSheetUIKitScrollAction,
    pub on_sheet_pull_from_scroll_end: BottomSheetUIKitScrollPullEnd,
    pub can_expand_from_scroll_top: bool,
    pub sheet_expansion_distance: Pixels,
    pub content_scrollable: bool,
    pub sheet_content: BottomSheetUIKitScrollContent,
    pub footer: BottomSheetContainerContent,
}

/// argument list of the constructor, the struct names every field
pub type BottomSheetContainerArgs = BottomSheetContainer;

impl BottomSheetContainer {
    pub fn new(args: BottomSheetContainerArgs) -> Self {
        args
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }

    fn standard_shadow_enabled(&self) -> bool {
        self.surface_shadow_enabled && !self.full_page && !self.uses_liquid_glass_surface
    }

    /// square top corners only for a full page sheet that doesnt ask for rounding
    fn rounds_surface(&self) -> bool {
        !(self.full_page && !self.rounds_top_corners)
    }

    fn full_page_body(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let main = self.inset_main_content(self.sheet_main_content(window, cx), true);
        let mut body = zstack(Alignment::Top).size_full().child(main);

        if !self.full_page_hides_footer {
            body = body.child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_stretch()
                    .child(spacer_min_h(0.0))
                    .child(self.footer_bar(window, cx)),
            );
        }

        let handle = div()
            .w_full()
            .pt(self.safe_top_inset)
            .opacity(if self.shows_handle { 1.0 } else { 0.0 })
            .child(BottomSheetHandle::new(true));
        body.child(self.handle(handle.into_any_element(), window, cx)).into_any_element()
    }

    fn handle(&self, view: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
        if self.host_owns_sheet_pan {
            return view;
        }
        let tracking = window
            .use_keyed_state(self.child_id("handle_drag"), cx, |_, _| -> DragTrackingCell {
                Rc::new(RefCell::new(None))
            })
            .read(cx)
            .clone();
        div()
            .relative()
            .w_full()
            .child(view)
            .child(drag_gesture_overlay(self.drag_gesture.clone(), tracking))
            .into_any_element()
    }

    fn standard_body(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let main = self.standard_main_content(window, cx);
        if self.footer_overlays_content {
            let mut body = div().relative().size_full().flex().flex_col().child(main);
            if !self.full_page_hides_footer {
                body = body.child(div().absolute().left_0().right_0().bottom_0().w_full().child(self.footer_bar(window, cx)));
            }
            return body.into_any_element();
        }

        let mut body = vstack(0.0).size_full().child(main);
        if !self.full_page_hides_footer {
            body = body.child(self.footer_bar(window, cx));
        }
        body.into_any_element()
    }

    fn standard_main_content(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        if self.content_under_handle {
            let main = self.inset_main_content(self.sheet_main_content(window, cx), true);
            let stack = zstack(Alignment::Top).size_full().child(main);
            if self.presents_as_modal {
                return stack.into_any_element();
            }
            let handle = self.handle(
                BottomSheetHandle::new(self.handle_over_content_is_light).into_any_element(),
                window,
                cx,
            );
            return stack.child(handle).into_any_element();
        }

        let mut column = vstack(0.0).w_full().flex_1().min_h_0();
        if !self.presents_as_modal {
            column = column.child(self.handle(BottomSheetHandle::new(false).into_any_element(), window, cx));
        }
        let main = self.inset_main_content(self.sheet_main_content(window, cx), false);
        column.child(main).into_any_element()
    }

    /// insets the main content for the hidden footer and horizontal safe area
    /// fills a stack cell or sizes to content and pins to the top
    fn inset_main_content(&self, content: AnyElement, fills_cell: bool) -> AnyElement {
        let frame = div()
            .flex()
            .flex_col()
            .pb(self.hidden_footer_bottom_inset)
            .px(self.horizontal_safe_inset)
            .child(env_scope::<BottomSheetHorizontalSafeInsetKey>(f32::from(self.horizontal_safe_inset)).child(content));
        let frame = if fills_cell { frame.size_full() } else { frame.w_full().flex_1().min_h_0() };
        frame.into_any_element()
    }

    fn sheet_main_content(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        if self.content_scrollable {
            let on_sheet_pull_end = self.on_sheet_pull_from_scroll_end.clone();
            let sheet_content = self.sheet_content.clone();
            return BottomSheetUIKitScroll::new(
                self.child_id("scroll"),
                self.motion_controller.clone(),
                self.can_expand_from_scroll_top,
                move |translation, velocity, window, cx| on_sheet_pull_end(translation, velocity, window, cx),
                move |window, cx| sheet_content(window, cx),
            )
            .is_sheet_interaction_locked(self.is_sheet_interaction_locked)
            .on_sheet_drag_began(self.on_sheet_drag_began.clone())
            .sheet_expansion_distance(self.sheet_expansion_distance)
            .into_any_element();
        }

        let content = (self.sheet_content)(window, cx);
        if self.host_owns_sheet_pan {
            return div().size_full().child(content).into_any_element();
        }

        let tracking = window
            .use_keyed_state(self.child_id("content_drag"), cx, |_, _| -> DragTrackingCell {
                Rc::new(RefCell::new(None))
            })
            .read(cx)
            .clone();
        div()
            .relative()
            .size_full()
            .child(content)
            .child(drag_gesture_overlay(self.drag_gesture.clone(), tracking))
            .into_any_element()
    }

    fn footer_bar(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let full_page = self.full_page;
        let horizontal = self.footer_horizontal_padding.unwrap_or(px(if full_page { 0.0 } else { 10.0 }));
        let top = self
            .footer_top_padding
            .unwrap_or(px(if full_page { 0.0 } else { BottomSheetMetrics::ATTACHED_FOOTER_TOP_PADDING }));
        let stack = self.footer_stack(window, cx);
        if full_page || self.footer_overlays_content {
            return div()
                .w_full()
                .pt(top)
                .child(
                    div()
                        .w_full()
                        .occlude()
                        .bg(self.footer_background)
                        .px(horizontal)
                        .pb(self.footer_bottom_padding)
                        .child(stack),
                )
                .into_any_element();
        }
        div()
            .w_full()
            .px(horizontal)
            .pt(top)
            .pb(self.footer_bottom_padding)
            .child(div().w_full().bg(self.footer_background).child(stack))
            .into_any_element()
    }

    fn footer_stack(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let mut stack = vstack(0.0).w_full().items_stretch();
        if self.footer_shows_top_border {
            stack = stack.child(div().w_full().h(px(0.5)).bg(self.theme.border._200.opacity(0.35)));
        }
        stack
            .child(
                div()
                    .w_full()
                    .px(self.horizontal_safe_inset)
                    .flex()
                    .flex_col()
                    .items_stretch()
                    .child((self.footer)(window, cx).frame_max_width(Alignment::Center)),
            )
            .into_any_element()
    }
}

impl RenderOnce for BottomSheetContainer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let body = if self.full_page {
            self.full_page_body(window, cx)
        } else {
            self.standard_body(window, cx)
        };

        let radius = self.top_corner_radius;
        let rounds_surface = self.rounds_surface();
        let mut surface = div().relative().size_full().overflow_hidden();
        if rounds_surface {
            surface = surface.rounded_tl(radius).rounded_tr(radius);
        }
        if self.uses_liquid_glass_surface {
            surface = surface.child(ExperimentalBottomSheetGlassSurface::new(
                self.sheet_background,
                if rounds_surface { radius } else { px(0.0) },
            ));
        } else {
            surface = surface.bg(self.sheet_background);
        }
        if self.standard_shadow_enabled() {
            surface = surface.shadow(vec![BoxShadow {
                color: black().opacity(BottomSheetMetrics::SHADOW_OPACITY as f32),
                offset: point(px(0.0), px(BottomSheetMetrics::SHADOW_OFFSET_Y)),
                blur_radius: px(BottomSheetMetrics::SHADOW_RADIUS),
                spread_radius: px(0.0),
                inset: false,
            }]);
        }

        env_scope::<SheetSurfaceColorKey>(Some(self.sheet_background))
            .child(surface.child(div().relative().size_full().child(body)))
    }
}

#[derive(IntoElement)]
struct ExperimentalBottomSheetGlassSurface {
    tint: Hsla,
    top_corner_radius: Pixels,
}

impl ExperimentalBottomSheetGlassSurface {
    fn new(tint: Hsla, top_corner_radius: Pixels) -> Self {
        Self { tint, top_corner_radius }
    }
}

impl RenderOnce for ExperimentalBottomSheetGlassSurface {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let fill = div()
            .w_full()
            .h(window.viewport_size().height)
            .bg(self.tint.opacity(0.12));
        let radius = f32::from(self.top_corner_radius);
        pass_through()
            .absolute()
            .inset_0()
            .child(fill.liquid_chrome_uneven_rounded_rectangle(radius, 0.0, 0.0, radius, 0.14, true, None, false))
    }
}
