use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, Bounds, Context, Div, ElementId, Entity, Hsla, InteractiveElement, IntoElement,
    ParentElement, Pixels, RenderOnce, ScrollHandle, SharedString, Size, Stateful, StatefulInteractiveElement, Styled,
    Window, black, canvas, div, prelude::FluentBuilder, px, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::hold_context_menu_ui::hold_context_menu_glass::{HoldContextMenuGlass, HoldContextMenuShadow};
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::environment::{EnvKey, env, env_scope, reduce_motion};
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{MotionSpec, Spring};
use rise_ui::swiftui::shapes::{Shape, centred_stroke};

use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;

pub type HoldContextMenuPageContent = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
pub type HoldContextMenuPageSizeChanged = Rc<dyn Fn(Size<Pixels>, &mut Window, &mut App)>;

const MEASURE_PAGE_OPERATION: &str = "hold_menu.measure_page";
const MEASURE_PAGE_THRESHOLD_MS: f64 = 2.0;
const SAFE_BOTTOM_MARGIN: f32 = 6.0;
const OUTGOING_SHIFT_FRACTION: f32 = 0.3;
const PAGE_SPRING: Spring = Spring::new(HoldContextMenuMetrics::OPEN_RESPONSE, HoldContextMenuMetrics::OPEN_DAMPING);

/// one stationary card, two pages only while navigating
/// top edge stays fixed, clip bounds animate and the page contents slide sideways
#[derive(IntoElement)]
pub struct HoldContextMenuPageHost {
    id: ElementId,
    pub page_id: SharedString,
    pub depth: i64,
    pub trailing: bool,
    pub is_revealed: bool,
    pub solid_color: Option<Hsla>,
    pub on_size_changed: HoldContextMenuPageSizeChanged,
    pub content: HoldContextMenuPageContent,
}

impl HoldContextMenuPageHost {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        page_id: impl Into<SharedString>,
        depth: i64,
        trailing: bool,
        is_revealed: bool,
        solid_color: Option<Hsla>,
        on_size_changed: HoldContextMenuPageSizeChanged,
        content: HoldContextMenuPageContent,
    ) -> Self {
        Self {
            id: id.into(),
            page_id: page_id.into(),
            depth,
            trailing,
            is_revealed,
            solid_color,
            on_size_changed,
            content,
        }
    }
}

struct HoldContextMenuPage {
    page_id: SharedString,
    content: HoldContextMenuPageContent,
    measured: Option<Size<Pixels>>,
    x: Animated<f32>,
    opacity: Animated<f32>,
}

impl HoldContextMenuPage {
    fn new(page_id: SharedString, content: HoldContextMenuPageContent) -> Self {
        Self {
            page_id,
            content,
            measured: None,
            x: Animated::new(0.0),
            opacity: Animated::new(1.0),
        }
    }
}

pub struct HoldContextMenuPageHostController {
    current: Option<HoldContextMenuPage>,
    outgoing: Option<HoldContextMenuPage>,
    depth: i64,
    revealed: Option<bool>,
    reveal: Animated<f32>,
    generation: u64,
    cancelled: bool,
    on_size_changed: Option<HoldContextMenuPageSizeChanged>,
    content_size: Size<Pixels>,
    clip_size: Animated<Size<Pixels>>,
    scroll_overflow_model: Entity<HoldContextMenuScrollOverflow>,
    scroll_overflow_page_id: Option<SharedString>,
    scroll_overflow: f32,
    scroll_overflow_queued: bool,
    scroll_clip_height: Option<f32>,
}

impl HoldContextMenuPageHostController {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            current: None,
            outgoing: None,
            depth: 0,
            revealed: None,
            reveal: Animated::new(0.0),
            generation: 0,
            cancelled: false,
            on_size_changed: None,
            content_size: Size::default(),
            clip_size: Animated::new(Size::default()),
            scroll_overflow_model: cx.new(|_| HoldContextMenuScrollOverflow::new()),
            scroll_overflow_page_id: None,
            scroll_overflow: 0.0,
            scroll_overflow_queued: false,
            scroll_clip_height: None,
        }
    }

    pub fn content_size(&self) -> Size<Pixels> {
        self.content_size
    }

    /// each page owns its value so an outgoing page keeps its scroll range while it slides out
    pub fn scroll_overflow(&mut self, page_id: &SharedString, cx: &mut Context<Self>) -> Entity<HoldContextMenuScrollOverflow> {
        if self.scroll_overflow_page_id.as_ref() != Some(page_id) {
            self.scroll_overflow_page_id = Some(page_id.clone());
            self.scroll_overflow_model = cx.new(|_| HoldContextMenuScrollOverflow::new());
            self.scroll_overflow = 0.0;
        }
        self.scroll_overflow_model.clone()
    }

    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        page_id: SharedString,
        depth: i64,
        revealed: bool,
        on_size_changed: HoldContextMenuPageSizeChanged,
        content: HoldContextMenuPageContent,
        now: MotionTime,
        reduce_motion: bool,
    ) {
        self.on_size_changed = Some(on_size_changed);
        let changed_page = self.current.as_ref().is_none_or(|current| current.page_id != page_id);
        let previous_depth = self.depth;
        if changed_page {
            self.generation += 1;
            self.unbind_scroll_view();
            self.outgoing = self.current.take();
            let mut host = HoldContextMenuPage::new(page_id, content);
            self.depth = depth;
            let animate = self.outgoing.is_some() && !reduce_motion;
            match self.outgoing.as_mut() {
                Some(outgoing) if animate => {
                    let forward = depth >= previous_depth;
                    let outgoing_width = outgoing.measured.map_or(0.0, |measured| f32::from(measured.width));
                    let width = f32::max(f32::from(self.content_size.width), outgoing_width);
                    let shift = width * OUTGOING_SHIFT_FRACTION;
                    host.x = Animated::new(if forward { width } else { -shift });
                    host.x.set(0.0, MotionSpec::spring(PAGE_SPRING), now, false);
                    outgoing.x.set(if forward { -shift } else { width }, MotionSpec::spring(PAGE_SPRING), now, false);
                    outgoing.opacity.set(0.0, MotionSpec::spring(PAGE_SPRING), now, false);
                }
                _ => self.outgoing = None,
            }
            self.current = Some(host);
        } else if let Some(current) = self.current.as_mut() {
            current.content = content;
        }

        if self.revealed != Some(revealed) || changed_page {
            let animated = self.revealed.is_some();
            self.revealed = Some(revealed);
            let target = if revealed { 1.0 } else { 0.0 };
            if animated {
                self.reveal.set(target, MotionSpec::spring(PAGE_SPRING), now, reduce_motion);
            } else {
                self.reveal.jump(target);
            }
        }
    }

    /// a page reported its laid out size, the clip glass stroke and shadow follow it
    fn measure(&mut self, page_id: &SharedString, measured: Size<Pixels>, now: MotionTime, reduce_motion: bool) -> Option<Size<Pixels>> {
        let token = ChatPerformanceTrace::shared().begin_operation(MEASURE_PAGE_OPERATION, HashMap::new());
        let resized = self.apply_measured(page_id, measured, now, reduce_motion);
        ChatPerformanceTrace::shared().end_operation(token, MEASURE_PAGE_THRESHOLD_MS, HashMap::new());
        resized
    }

    fn apply_measured(&mut self, page_id: &SharedString, measured: Size<Pixels>, now: MotionTime, reduce_motion: bool) -> Option<Size<Pixels>> {
        if self.cancelled {
            return None;
        }
        let current = self.current.as_mut().filter(|current| &current.page_id == page_id)?;
        let (width, height) = (f32::from(measured.width), f32::from(measured.height));
        if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
            return None;
        }
        let size = size(px(width.ceil()), px(height.ceil()));
        current.measured = Some(size);
        if size == self.content_size {
            return None;
        }
        let animated = self.revealed == Some(true) && self.content_size != Size::default() && !reduce_motion;
        self.content_size = size;
        if animated {
            self.clip_size.set(size, MotionSpec::spring(PAGE_SPRING), now, false);
        } else {
            self.clip_size.jump(size);
        }
        self.scroll_clip_height = None;
        Some(size)
    }

    /// only the explicitly marked list takes part, its handle is read and its rows stay untouched
    fn update_scroll_geometry(&mut self, page_bounds: Bounds<Pixels>, viewport: Size<Pixels>, cx: &mut Context<Self>) {
        let content_height = f32::from(self.content_size.height);
        if self.cancelled || self.current.is_none() || content_height <= 0.0 {
            return;
        }
        let (registered, scroll_handle) = {
            let model = self.scroll_overflow_model.read(cx);
            (model.registered, model.scroll_handle.clone())
        };
        let list_bounds = scroll_handle.bounds();
        let list_height = f32::from(list_bounds.size.height);
        let max_offset = f32::from(scroll_handle.max_offset().y);
        if !registered || list_height <= 0.0 || list_height + max_offset <= 0.0 {
            return;
        }

        let page_top = f32::from(page_bounds.origin.y);
        let safe_bottom = f32::from(viewport.height) - SAFE_BOTTOM_MARGIN;
        let visible_bottom = safe_bottom - page_top;
        let overflow = f32::min(f32::max(0.0, list_height - 1.0), f32::max(0.0, content_height - visible_bottom));
        if (self.scroll_overflow - overflow).abs() > 0.5 {
            self.set_scroll_overflow(overflow, cx);
        }

        let applied_overflow = self.scroll_overflow_model.read(cx).value;
        let list_content_height = list_height + max_offset - applied_overflow;
        let scrolled = -f32::from(scroll_handle.offset().y);
        let remaining_content = list_content_height - scrolled;
        let list_min_y = f32::from(list_bounds.origin.y) - page_top;
        let list_max_y = list_min_y + list_height;
        let tail = list_min_y + remaining_content + f32::max(0.0, content_height - list_max_y);
        let height = f32::min(content_height, f32::max(content_height - overflow, tail));
        if !(height < content_height || self.scroll_clip_height.is_some()) {
            return;
        }
        if self.scroll_clip_height == Some(height) {
            return;
        }
        self.scroll_clip_height = Some(height);
        cx.notify();
    }

    fn set_scroll_overflow(&mut self, overflow: f32, cx: &mut Context<Self>) {
        self.scroll_overflow = overflow;
        if self.scroll_overflow_queued {
            return;
        }
        self.scroll_overflow_queued = true;
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            this.update(cx, |this, cx| {
                this.scroll_overflow_queued = false;
                if this.cancelled {
                    return;
                }
                let value = this.scroll_overflow;
                this.scroll_overflow_model.update(cx, |model, cx| {
                    if model.value != value {
                        model.value = value;
                        cx.notify();
                    }
                });
            })
            .ok();
        });
    }

    fn unbind_scroll_view(&mut self) {
        self.scroll_clip_height = None;
    }

    fn settle_outgoing(&mut self, now: MotionTime) {
        let settled = self
            .outgoing
            .as_ref()
            .is_some_and(|outgoing| !outgoing.x.is_running(now) && !outgoing.opacity.is_running(now));
        if settled {
            self.outgoing = None;
        }
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.unbind_scroll_view();
        self.on_size_changed = None;
        self.generation += 1;
        self.outgoing = None;
    }
}

impl RenderOnce for HoldContextMenuPageHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let reduces_motion = reduce_motion(cx);
        let controller = window.use_keyed_state(self.id.clone(), cx, |_, cx| HoldContextMenuPageHostController::new(cx));
        let overflow = controller.update(cx, |controller, cx| {
            let overflow = controller.scroll_overflow(&self.page_id, cx);
            controller.update(
                self.page_id.clone(),
                self.depth,
                self.is_revealed,
                self.on_size_changed.clone(),
                self.content.clone(),
                now,
                reduces_motion,
            );
            controller.settle_outgoing(now);
            overflow
        });

        let trailing = self.trailing;
        let solid_color = self.solid_color;
        let radius = HoldContextMenuMetrics::MENU_CORNER_RADIUS;
        let state = controller.read(cx);
        let content_size = state.content_size;
        let reveal = state.reveal.frame_value(now, window);
        let mut clip_size = state.clip_size.frame_value(now, window);
        if let Some(height) = state.scroll_clip_height {
            clip_size.height = px(height);
        }
        let current = state.current.as_ref().map(|page| {
            (
                page.page_id.clone(),
                page.content.clone(),
                page.x.frame_value(now, window),
            )
        });
        let outgoing = state.outgoing.as_ref().map(|page| {
            (
                page.content.clone(),
                page.x.frame_value(now, window),
                page.opacity.frame_value(now, window),
            )
        });

        let layer = |base: Div| -> Div {
            let base = base.absolute().top_0().w(clip_size.width).h(clip_size.height);
            if trailing { base.right_0() } else { base.left_0() }
        };

        let mut clip = layer(div()).overflow_hidden().rounded(px(radius)).opacity(reveal);
        if let Some((content, x, opacity)) = outgoing {
            clip = clip.child(
                div()
                    .absolute()
                    .top_0()
                    .left(px(x))
                    .opacity(opacity)
                    .child(content(window, cx))
                    .child(div().id("hold-context-menu-page-outgoing").absolute().inset_0().occlude()),
            );
        }
        if let Some((page_id, content, x)) = current {
            let measured = controller.clone();
            let measure_page_id = page_id.clone();
            clip = clip.child(
                div()
                    .absolute()
                    .top_0()
                    .left(px(x))
                    .max_w(px(
                        HoldContextMenuMetrics::MENU_TITLE_MAX_WIDTH + HoldContextMenuMetrics::MENU_DESKTOP_WIDTH_SLACK,
                    ))
                    .child(
                        env_scope::<HoldContextMenuScrollOverflowKey>(Some(overflow))
                            .child(content(window, cx)),
                    )
                    .child(measure(
                        ElementId::from((self.id.clone(), page_id)),
                        move |bounds, window, cx| {
                            let now = RiseClock::monotonic();
                            let reduces_motion = reduce_motion(cx);
                            let resized = measured.update(cx, |controller, cx| {
                                let resized = controller.measure(&measure_page_id, bounds.size, now, reduces_motion);
                                if resized.is_some() {
                                    cx.notify();
                                }
                                resized.and_then(|size| controller.on_size_changed.clone().map(|callback| (size, callback)))
                            });
                            if let Some((size, on_size_changed)) = resized {
                                on_size_changed(size, window, cx);
                            }
                        },
                    )),
            );
        }

        let geometry = controller.clone();
        let cancelled = controller.clone();
        let stroke_path = Shape::continuous(radius);
        div()
            .id(self.id.clone())
            .relative()
            .flex_none()
            .w(content_size.width)
            .h(content_size.height)
            .child(
                layer(div())
                    .opacity(reveal)
                    .child(HoldContextMenuShadow::new(radius).radius(22.0).opacity(0.28).offset_y(12.0)),
            )
            .when(solid_color.is_none(), |host| {
                host.child(
                    layer(div()).child(
                        HoldContextMenuGlass::new((self.id.clone(), "glass"))
                            .tint(black().opacity(0.12))
                            .corner_radius(radius)
                            .is_revealed(self.is_revealed)
                            .opaque(true),
                    ),
                )
            })
            .when_some(solid_color, |host, color| {
                host.child(layer(div()).rounded(px(radius)).bg(color).opacity(reveal))
            })
            .child(clip)
            .child(layer(div()).opacity(reveal).child(centred_stroke(
                white().opacity(HoldContextMenuMetrics::PANEL_STROKE_OPACITY as f32),
                1.0,
                stroke_path,
            )))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let viewport = window.viewport_size();
                        geometry.update(cx, |controller, cx| controller.update_scroll_geometry(bounds, viewport, cx));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .on_disappear((self.id.clone(), "dismantle"), move |_, cx| {
                cancelled.update(cx, |controller, _| controller.cancel());
            })
    }
}

/// scroll range for the part of a menu page below the window bottom edge
/// with the handle of the one list that carries the scroll content marker
pub struct HoldContextMenuScrollOverflow {
    pub value: f32,
    pub scroll_handle: ScrollHandle,
    registered: bool,
}

impl HoldContextMenuScrollOverflow {
    pub fn new() -> Self {
        Self {
            value: 0.0,
            scroll_handle: ScrollHandle::new(),
            registered: false,
        }
    }
}

impl Default for HoldContextMenuScrollOverflow {
    fn default() -> Self {
        Self::new()
    }
}

pub struct HoldContextMenuScrollOverflowKey;

impl EnvKey for HoldContextMenuScrollOverflowKey {
    type Value = Option<Entity<HoldContextMenuScrollOverflow>>;

    fn default_value() -> Self::Value {
        None
    }
}

/// scroll overflow of the menu from the environment
pub fn hold_context_menu_scroll_overflow(cx: &App) -> Option<Entity<HoldContextMenuScrollOverflow>> {
    env::<HoldContextMenuScrollOverflowKey>(cx)
}

pub trait HoldContextMenuScrollViewportModifiers: Sized {
    fn hold_context_menu_scroll_viewport(self, cx: &App) -> Self;
}

impl HoldContextMenuScrollViewportModifiers for Stateful<Div> {
    fn hold_context_menu_scroll_viewport(self, cx: &App) -> Self {
        match hold_context_menu_scroll_overflow(cx) {
            Some(overflow) => {
                let overflow = overflow.read(cx);
                self.pb(px(overflow.value)).track_scroll(&overflow.scroll_handle)
            }
            None => self.pb(px(0.0)),
        }
    }
}

/// place inside a menu scroll content
/// registers the viewport once drawn including a list that arrives after loading
/// takes no clicks, no hitbox
#[derive(IntoElement, Default)]
pub struct HoldContextMenuScrollContent;

impl HoldContextMenuScrollContent {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for HoldContextMenuScrollContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let overflow = hold_context_menu_scroll_overflow(cx);
        canvas(
            move |_, _, cx| {
                if let Some(overflow) = overflow {
                    overflow.update(cx, |overflow, _| overflow.registered = true);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
}
