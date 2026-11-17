use std::cmp::Ordering;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyWindowHandle, App, Bounds, ElementId, Entity, FocusHandle, Hsla, Pixels, Point,
    Role, Size, Task, Window,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::AppTheme;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::bottom_sheet_ui::bottom_sheet_axis_ownership::BottomSheetAxisOwnership;
use rise_ui::bottom_sheet_ui::bottom_sheet_backdrop::BottomSheetBackdrop;
use rise_ui::bottom_sheet_ui::bottom_sheet_interaction_policy::{
    BottomSheetInteractionStyle, BottomSheetReleaseDecision, BottomSheetReleasePolicy,
    BottomSheetSnapAnimationCurve,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::bottom_sheet_ui::bottom_sheet_motion_host::{
    BottomSheetMotionController, BottomSheetMotionHost,
};
use rise_ui::bottom_sheet_ui::modal_sheet_surface::{
    ModalSheetFrame, ModalSheetMetrics, ModalSheetStack, modal_sheet_centering, modal_sheet_chrome, modal_sheet_chrome_bare,
    modal_sheet_cycle_focus, modal_sheet_keep_focus_inside, modal_sheet_outline, modal_sheet_settle_pointer,
};
use rise_ui::keyboard_dismiss::SheetFocusKey;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::device_info::DeviceInfo;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_container::{
    BottomSheetContainer, BottomSheetContainerArgs, BottomSheetDragGesture, BottomSheetDragValue,
};

type SheetContent = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;
type BackdropTapInterceptor = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App) -> bool>;
type LiveSheetFrameObserver = Rc<dyn Fn(Option<Bounds<Pixels>>, &mut App)>;

const WINDOW_SAFE_INSET: f32 = 0.0;

const FOCUSED_FIELD_KEYBOARD_HEIGHT: f32 = 1.0;

const ESCAPE_KEY: &str = "escape";

#[derive(IntoElement, Clone)]
pub struct BottomSheetFullPageUi {
    id: ElementId,
    is_bottom_sheet: Binding<bool>,
    pub snap_fractions: Vec<f32>,
    pub content_is_edge_to_edge: bool,
    pub content_under_handle: bool,
    pub handle_over_content_is_light: bool,
    selected_snap_fraction: Option<Binding<f32>>,
    pub snap_animation_duration: f64,
    pub interaction_style: BottomSheetInteractionStyle,
    pub shows_drag_indicator: bool,
    pub full_page_hides_footer: bool,
    pub full_page_avoids_keyboard_when_footer_hidden: bool,
    pub keyboard_avoidance_height: Pixels,
    pub keyboard_footer_spacing: Pixels,
    pub footer_shows_top_border: bool,
    pub footer_overlays_content: bool,
    pub footer_horizontal_padding: Option<Pixels>,
    pub footer_top_padding: Option<Pixels>,
    pub content_scrollable: bool,
    pub full_screen_cover: bool,
    pub modal_sized_by_detent: bool,
    pub allows_interactive_dismiss: bool,
    pub dismisses_on_backdrop_tap: bool,
    pub dismisses_keyboard_before_backdrop_dismiss: bool,
    pub interaction_locked: bool,
    pub sheet_background: Option<Hsla>,
    pub footer_background: Option<Hsla>,
    pub surface_shadow_enabled: bool,
    pub modal_chrome_hidden: bool,
    modal_edge_underlay: Option<SheetContent>,
    pub backdrop_opacity: Option<f64>,
    pub backdrop_color: Option<Hsla>,
    pub backdrop_blur: Option<f32>,
    backdrop_tap_interceptor: Option<BackdropTapInterceptor>,
    pub uses_full_page_layout: bool,
    pub uses_liquid_glass_surface: bool,
    pub top_corner_radius: Option<Pixels>,
    pub appears_in_place: bool,
    on_live_sheet_frame: Option<LiveSheetFrameObserver>,
    on_dismiss: SheetAction,
    sheet_content: SheetContent,
    footer: SheetContent,
}

impl BottomSheetFullPageUi {
    /// a full screen cover stays one, every other sheet is a modal on desktop
    fn presents_as_modal(&self) -> bool {
        !self.full_screen_cover
    }

    pub fn new<C: IntoElement, F: IntoElement>(
        id: impl Into<ElementId>,
        is_bottom_sheet: Binding<bool>,
        sheet_content: impl Fn(&mut Window, &mut App) -> C + 'static,
        footer: impl Fn(&mut Window, &mut App) -> F + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            is_bottom_sheet,
            snap_fractions: vec![1.0],
            content_is_edge_to_edge: false,
            content_under_handle: false,
            handle_over_content_is_light: true,
            selected_snap_fraction: None,
            snap_animation_duration: BottomSheetMetrics::SNAP_DURATION,
            interaction_style: BottomSheetInteractionStyle::Standard,
            shows_drag_indicator: true,
            full_page_hides_footer: false,
            full_page_avoids_keyboard_when_footer_hidden: false,
            keyboard_avoidance_height: px(0.0),
            keyboard_footer_spacing: px(0.0),
            footer_shows_top_border: true,
            footer_overlays_content: false,
            footer_horizontal_padding: None,
            footer_top_padding: None,
            content_scrollable: true,
            full_screen_cover: false,
            modal_sized_by_detent: false,
            allows_interactive_dismiss: true,
            dismisses_on_backdrop_tap: true,
            dismisses_keyboard_before_backdrop_dismiss: false,
            interaction_locked: false,
            sheet_background: None,
            footer_background: None,
            surface_shadow_enabled: true,
            modal_chrome_hidden: false,
            modal_edge_underlay: None,
            backdrop_opacity: None,
            backdrop_color: None,
            backdrop_blur: None,
            backdrop_tap_interceptor: None,
            uses_full_page_layout: true,
            uses_liquid_glass_surface: false,
            top_corner_radius: None,
            appears_in_place: false,
            on_live_sheet_frame: None,
            on_dismiss: Rc::new(|_, _| {}),
            sheet_content: Rc::new(move |window, cx| sheet_content(window, cx).into_any_element()),
            footer: Rc::new(move |window, cx| footer(window, cx).into_any_element()),
        }
    }

    pub fn snap_fractions(mut self, snap_fractions: Vec<f32>) -> Self {
        self.snap_fractions = snap_fractions;
        self
    }

    pub fn content_is_edge_to_edge(mut self, edge_to_edge: bool) -> Self {
        self.content_is_edge_to_edge = edge_to_edge;
        self
    }

    pub fn content_under_handle(mut self, under_handle: bool) -> Self {
        self.content_under_handle = under_handle;
        self
    }

    pub fn handle_over_content_is_light(mut self, is_light: bool) -> Self {
        self.handle_over_content_is_light = is_light;
        self
    }

    pub fn selected_snap_fraction(mut self, selected: Option<Binding<f32>>) -> Self {
        self.selected_snap_fraction = selected;
        self
    }

    pub fn snap_animation_duration(mut self, duration: f64) -> Self {
        self.snap_animation_duration = duration;
        self
    }

    pub fn interaction_style(mut self, style: BottomSheetInteractionStyle) -> Self {
        self.interaction_style = style;
        self
    }

    pub fn shows_drag_indicator(mut self, shows: bool) -> Self {
        self.shows_drag_indicator = shows;
        self
    }

    pub fn full_page_hides_footer(mut self, hides: bool) -> Self {
        self.full_page_hides_footer = hides;
        self
    }

    pub fn full_page_avoids_keyboard_when_footer_hidden(mut self, avoids: bool) -> Self {
        self.full_page_avoids_keyboard_when_footer_hidden = avoids;
        self
    }

    pub fn keyboard_avoidance_height(mut self, height: Pixels) -> Self {
        self.keyboard_avoidance_height = height;
        self
    }

    pub fn keyboard_footer_spacing(mut self, spacing: Pixels) -> Self {
        self.keyboard_footer_spacing = spacing;
        self
    }

    pub fn footer_shows_top_border(mut self, shows: bool) -> Self {
        self.footer_shows_top_border = shows;
        self
    }

    pub fn footer_overlays_content(mut self, overlays: bool) -> Self {
        self.footer_overlays_content = overlays;
        self
    }

    pub fn footer_horizontal_padding(mut self, padding: Option<Pixels>) -> Self {
        self.footer_horizontal_padding = padding;
        self
    }

    pub fn footer_top_padding(mut self, padding: Option<Pixels>) -> Self {
        self.footer_top_padding = padding;
        self
    }

    pub fn content_scrollable(mut self, scrollable: bool) -> Self {
        self.content_scrollable = scrollable;
        self
    }

    pub fn full_screen_cover(mut self, cover: bool) -> Self {
        self.full_screen_cover = cover;
        self
    }

    pub fn modal_sized_by_detent(mut self, sized: bool) -> Self {
        self.modal_sized_by_detent = sized;
        self
    }

    pub fn allows_interactive_dismiss(mut self, allows: bool) -> Self {
        self.allows_interactive_dismiss = allows;
        self
    }

    pub fn dismisses_on_backdrop_tap(mut self, dismisses: bool) -> Self {
        self.dismisses_on_backdrop_tap = dismisses;
        self
    }

    pub fn dismisses_keyboard_before_backdrop_dismiss(mut self, dismisses: bool) -> Self {
        self.dismisses_keyboard_before_backdrop_dismiss = dismisses;
        self
    }

    pub fn interaction_locked(mut self, locked: bool) -> Self {
        self.interaction_locked = locked;
        self
    }

    pub fn sheet_background(mut self, background: Option<Hsla>) -> Self {
        self.sheet_background = background;
        self
    }

    pub fn footer_background(mut self, background: Option<Hsla>) -> Self {
        self.footer_background = background;
        self
    }

    pub fn surface_shadow_enabled(mut self, enabled: bool) -> Self {
        self.surface_shadow_enabled = enabled;
        self
    }

    pub fn modal_chrome_hidden(mut self, hidden: bool) -> Self {
        self.modal_chrome_hidden = hidden;
        self
    }

    pub fn modal_edge_underlay(mut self, underlay: Option<Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>>) -> Self {
        self.modal_edge_underlay = underlay;
        self
    }

    pub fn backdrop_opacity(mut self, opacity: Option<f64>) -> Self {
        self.backdrop_opacity = opacity;
        self
    }

    pub fn backdrop_color(mut self, color: Option<Hsla>) -> Self {
        self.backdrop_color = color;
        self
    }

    pub fn backdrop_blur(mut self, radius: Option<f32>) -> Self {
        self.backdrop_blur = radius;
        self
    }

    /// returns true when the caller took the click so it must not reach the sheet
    pub fn backdrop_tap_interceptor(
        mut self,
        interceptor: impl Fn(Point<Pixels>, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.backdrop_tap_interceptor = Some(Rc::new(interceptor));
        self
    }

    pub fn uses_full_page_layout(mut self, uses: bool) -> Self {
        self.uses_full_page_layout = uses;
        self
    }

    pub fn uses_liquid_glass_surface(mut self, uses: bool) -> Self {
        self.uses_liquid_glass_surface = uses;
        self
    }

    pub fn top_corner_radius(mut self, radius: Option<Pixels>) -> Self {
        self.top_corner_radius = radius;
        self
    }

    pub fn appears_in_place(mut self, in_place: bool) -> Self {
        self.appears_in_place = in_place;
        self
    }

    pub fn on_live_sheet_frame(
        mut self,
        observer: impl Fn(Option<Bounds<Pixels>>, &mut App) + 'static,
    ) -> Self {
        self.on_live_sheet_frame = Some(Rc::new(observer));
        self
    }

    pub fn on_dismiss(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Rc::new(action);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SheetDragAxis {
    Undetermined,
    Horizontal,
    Vertical,
}

struct BottomSheetFullPageState {
    selected_detent: usize,
    drag_base_height: Option<Pixels>,
    drag_activation_translation: Pixels,
    presentation_tap_guard_deadline: f64,
    motion_controller: BottomSheetMotionController,
    is_presented: bool,
    is_dismissing: bool,
    sheet_drag_axis: SheetDragAxis,
    geometry_size: Option<Size<Pixels>>,
    focus_handle: FocusHandle,
    focus_before_open: Option<FocusHandle>,
    modal_appearance: Animated<f32>,
    modal_dismissal: Option<Task<()>>,
}

impl BottomSheetFullPageState {
    fn new(cx: &mut App) -> Self {
        Self {
            selected_detent: 0,
            drag_base_height: None,
            drag_activation_translation: px(0.0),
            presentation_tap_guard_deadline: 0.0,
            motion_controller: BottomSheetMotionController::new(),
            is_presented: false,
            is_dismissing: false,
            sheet_drag_axis: SheetDragAxis::Undetermined,
            geometry_size: None,
            focus_handle: cx.focus_handle(),
            focus_before_open: None,
            modal_appearance: Animated::new(0.0),
            modal_dismissal: None,
        }
    }

    fn end_drag(&mut self) {
        self.drag_base_height = None;
        self.sheet_drag_axis = SheetDragAxis::Undetermined;
        self.drag_activation_translation = px(0.0);
    }
}

/// layout values one render computes, the hosted surface and its handlers read them
#[derive(Clone)]
struct SheetLayout {
    theme: AppTheme,
    sheet_background: Hsla,
    footer_background: Hsla,
    safe_bottom: f32,
    safe_top: f32,
    safe_horizontal: f32,
    effective_keyboard_height: f32,
    detents: Rc<[f32]>,
    maximum_height: f32,
    resting_height: f32,
    footer_bottom_padding: f32,
    expansion_distance: f32,
}

#[derive(Clone)]
struct FullPageWiring {
    ui: Rc<BottomSheetFullPageUi>,
    state: Entity<BottomSheetFullPageState>,
    reduce_motion: bool,
    window_handle: AnyWindowHandle,
}

impl RenderOnce for BottomSheetFullPageUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            BottomSheetFullPageState::new(cx)
        });
        let wiring = FullPageWiring {
            ui: Rc::new(self),
            state: state.clone(),
            reduce_motion: reduce_motion(cx),
            window_handle: window.window_handle(),
        };
        let id = wiring.ui.id.clone();

        let on_open = wiring.clone();
        on_change(
            (id.clone(), "is-bottom-sheet"),
            wiring.ui.is_bottom_sheet.get(cx),
            false,
            window,
            cx,
            move |_, open, window, cx| {
                tracing::warn!(target: "riseonly::X808", open = *open, dismissing = on_open.state.read(cx).is_dismissing, "full page flag changed");
                if *open {
                    on_open.begin_open(window, cx);
                } else if !on_open.state.read(cx).is_dismissing {
                    on_open.start_dismiss(window, cx);
                }
            },
        );

        let (is_presented, is_dismissing, focus_handle) = {
            let state = state.read(cx);
            (state.is_presented, state.is_dismissing, state.focus_handle.clone())
        };

        let mut root = div().id(id.clone()).absolute().inset_0().size_full();
        if is_presented {
            let on_escape = wiring.clone();
            root = root
                .track_focus(&focus_handle)
                .key_context(rise_ui::layer_keys::LAYER_KEY_CONTEXT)
                .on_key_down(move |event, window, cx| {
                    if event.keystroke.key == ESCAPE_KEY && on_escape.handle_escape(window, cx) {
                        cx.stop_propagation();
                    }
                })
                .child(env_scope::<SheetFocusKey>(Some(focus_handle.clone())).child(wiring.content(window, cx)));
            if wiring.ui.presents_as_modal() {
                let capture_focus = focus_handle.clone();
                let cycle_focus = focus_handle.clone();
                root = root
                    .capture_key_down(move |event, window, cx| {
                        modal_sheet_keep_focus_inside(event, &capture_focus, window, cx);
                    })
                    .on_key_down(move |event, window, cx| {
                        if modal_sheet_cycle_focus(event, &cycle_focus, window, cx) {
                            cx.stop_propagation();
                        }
                    });
            }
            if !is_dismissing {
                root = root.role(Role::Dialog);
                if wiring.ui.full_screen_cover {
                    root = root.occlude();
                }
            }
        }

        let on_appear = wiring;
        root.on_appear((id, "appear"), move |window, cx| {
            if on_appear.ui.is_bottom_sheet.get(cx) && !on_appear.state.read(cx).is_presented {
                on_appear.begin_open(window, cx);
            }
        })
    }
}

impl FullPageWiring {
    fn content(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let ui = &self.ui;
        let theme = rise_ui::theme(cx).clone();
        let sheet_background = ui.sheet_background.unwrap_or(theme.bg._200);
        let footer_background = ui.footer_background.unwrap_or(theme.bg._200);
        let (geometry, drag_base_height, is_presented, is_dismissing, motion_controller) = {
            let state = self.state.read(cx);
            (
                state.geometry_size.unwrap_or_else(|| window.viewport_size()),
                state.drag_base_height,
                state.is_presented,
                state.is_dismissing,
                state.motion_controller.clone(),
            )
        };
        let safe_bottom = WINDOW_SAFE_INSET;
        let safe_top = WINDOW_SAFE_INSET;
        let safe_horizontal = if ui.content_is_edge_to_edge { 0.0 } else { WINDOW_SAFE_INSET };
        let effective_keyboard_height = f32::from(ui.keyboard_avoidance_height).max(0.0);
        let height = f32::from(geometry.height);
        let keyboard_ceiling = if ui.full_screen_cover { height } else { (height - safe_top).max(0.0) };
        let detents: Rc<[f32]> = self
            .compute_detents(geometry, safe_top)
            .into_iter()
            .map(|detent| keyboard_ceiling.min(detent + effective_keyboard_height))
            .collect();
        let detent_index = self.clamped_detent_index(&detents, cx);
        let detent_height = detents.get(detent_index).copied().unwrap_or_default();
        let maximum_height = detents.last().copied().unwrap_or(detent_height);
        let controlled_height = self
            .controlled_detent_height(geometry, safe_top, cx)
            .map(|controlled| keyboard_ceiling.min(controlled + effective_keyboard_height));
        let resting_height = drag_base_height
            .map(f32::from)
            .or(controlled_height)
            .unwrap_or(detent_height);
        let footer_bottom_padding = if effective_keyboard_height > 0.0 {
            safe_bottom.max(effective_keyboard_height + f32::from(ui.keyboard_footer_spacing))
        } else {
            safe_bottom
        };
        let expansion_distance = (maximum_height - resting_height).max(0.0);
        let backdrop_visible = is_presented && !is_dismissing;

        let layout = SheetLayout {
            theme,
            sheet_background,
            footer_background,
            safe_bottom,
            safe_top,
            safe_horizontal,
            effective_keyboard_height,
            detents,
            maximum_height,
            resting_height,
            footer_bottom_padding,
            expansion_distance,
        };

        let measured = self.state.clone();
        let mut stack = div().absolute().inset_0().child(measure(
            (ui.id.clone(), "geometry"),
            move |bounds, _, cx| {
                measured.update(cx, |state, cx| {
                    if state.geometry_size != Some(bounds.size) {
                        state.geometry_size = Some(bounds.size);
                        cx.notify();
                    }
                });
            },
        ));

        if !ui.full_screen_cover {
            let on_tap = self.clone();
            let mut backdrop = BottomSheetBackdrop::new(backdrop_visible, move |point, window, cx| {
                on_tap.handle_backdrop_tap(point, window, cx);
            })
            .opacity(ui.backdrop_opacity.unwrap_or(BottomSheetMetrics::BACKDROP_OPACITY));
            if let Some(color) = ui.backdrop_color {
                backdrop = backdrop.color(color);
            }
            if let Some(radius) = ui.backdrop_blur {
                backdrop = backdrop.blur_radius(radius);
            }
            stack = stack.child(backdrop);
        }

        if ui.presents_as_modal() {
            return stack.child(self.modal_surface(&layout, geometry, window, cx)).into_any_element();
        }

        let surface = self.clone();
        let surface_layout = layout.clone();
        let pan_began = self.clone();
        let pan_began_layout = layout.clone();
        let pan_ended = self.clone();
        let pan_ended_layout = layout.clone();
        let mut host = BottomSheetMotionHost::new(
            (ui.id.clone(), "motion-host"),
            motion_controller,
            move |window, cx| surface.container(&surface_layout, window, cx),
        )
        .resting_height(px(layout.resting_height))
        .maximum_height(px(layout.maximum_height))
        .snap_animation_duration(self.resolved_snap_animation_duration())
        .snap_animation_curve(self.resolved_configuration_snap_curve())
        .allows_dismissal_preview(self.resolved_allows_dismissal_preview())
        .surface_corner_radius(px(self.resolved_surface_corner_radius()))
        .owns_content_pan(!ui.content_scrollable)
        .content_pan_locked(is_dismissing || ui.interaction_locked || !ui.allows_interactive_dismiss)
        .on_content_pan_began(move |window, cx| {
            pan_began.begin_drag_for_layout(&pan_began_layout, window, cx);
        })
        .on_content_pan_ended(move |translation_y, velocity_y, window, cx| {
            pan_ended.handle_release(
                f32::from(translation_y),
                f32::from(velocity_y),
                &pan_ended_layout.detents,
                pan_ended_layout.safe_bottom,
                pan_ended_layout.resting_height,
                window,
                cx,
            );
        });
        if let Some(observer) = ui.on_live_sheet_frame.clone() {
            host = host.live_frame_observer(move |frame, cx| observer(frame, cx));
        }
        if !ui.full_screen_cover && backdrop_visible {
            let on_tap = self.clone();
            host = host.on_backdrop_tap(move |point, window, cx| {
                on_tap.handle_backdrop_tap(point, window, cx);
            });
        }

        stack
            .child(div().absolute().inset_0().child(host))
            .into_any_element()
    }

    fn modal_surface(
        &self,
        layout: &SheetLayout,
        geometry: Size<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let ui = &self.ui;
        let frame = ModalSheetFrame::resolve(geometry, &layout.theme);
        let appearance = self.frame_modal_appearance(window, cx);
        let live_frame_id = (ui.id.clone(), "modal-live-frame");
        let live_frame_observer = ui.on_live_sheet_frame.clone();
        let stacked = ModalSheetStack::enter((ui.id.clone(), "modal-stack"), window, cx);
        let asked = Self::modal_height(ui.modal_sized_by_detent, &frame, layout);
        let height = if ui.modal_sized_by_detent { asked } else { stacked.raised(asked, &frame) };
        let underlay = ui.modal_edge_underlay.clone().filter(|_| !ui.modal_chrome_hidden);
        let surface = div()
            .id((ui.id.clone(), "modal-surface"))
            .relative()
            .w(px(frame.width))
            .h(px(height))
            .occlude()
            .when_some(underlay, |surface, underlay| {
                surface.child(div().absolute().inset_0().child(underlay(window, cx)))
            })
            .child(self.container(layout, window, cx))
            .child(stacked.measurer((ui.id.clone(), "modal-stack-measure")))
            .when_some(live_frame_observer, |surface, observer| {
                surface.child(measure(live_frame_id, move |bounds, _, cx| observer(Some(bounds), cx)))
            });
        let chrome = if ui.modal_chrome_hidden {
            modal_sheet_chrome_bare(surface, appearance)
        } else {
            modal_sheet_chrome(surface, layout.sheet_background, modal_sheet_outline(&layout.theme), appearance)
        };
        modal_sheet_centering().child(chrome).into_any_element()
    }

    fn modal_height(sized_by_detent: bool, frame: &ModalSheetFrame, layout: &SheetLayout) -> f32 {
        if sized_by_detent {
            layout.resting_height.min(frame.max_height).max(1.0)
        } else {
            frame.filled_height(layout.maximum_height)
        }
    }

    fn frame_modal_appearance(&self, window: &Window, cx: &mut App) -> f32 {
        let now = RiseClock::monotonic();
        self.state.update(cx, |state, cx| {
            let (target, duration) = if state.is_dismissing {
                (0.0, ModalSheetMetrics::DISMISS_DURATION)
            } else {
                (1.0, ModalSheetMetrics::PRESENT_DURATION)
            };
            state.modal_appearance.set_in(target, ease_out(duration), now, cx);
            state.modal_appearance.frame_value(now, window)
        })
    }

    fn container(&self, layout: &SheetLayout, _window: &mut Window, cx: &mut App) -> AnyElement {
        let ui = &self.ui;
        let is_modal = ui.presents_as_modal();
        let (is_dismissing, motion_controller) = {
            let state = self.state.read(cx);
            (state.is_dismissing, state.motion_controller.clone())
        };
        let drag_began = self.clone();
        let drag_began_layout = layout.clone();
        let pull = self.clone();
        let pull_layout = layout.clone();
        let hidden_footer_bottom_inset =
            if ui.full_page_hides_footer && ui.full_page_avoids_keyboard_when_footer_hidden {
                layout.effective_keyboard_height
            } else {
                0.0
            };
        BottomSheetContainer::new(BottomSheetContainerArgs {
            id: (ui.id.clone(), "container").into(),
            theme: layout.theme.clone(),
            full_page: ui.uses_full_page_layout,
            shows_handle: ui.shows_drag_indicator && !is_modal,
            presents_as_modal: is_modal,
            content_under_handle: ui.content_under_handle,
            handle_over_content_is_light: ui.handle_over_content_is_light,
            rounds_top_corners: !ui.full_screen_cover,
            full_page_hides_footer: ui.full_page_hides_footer,
            hidden_footer_bottom_inset: px(hidden_footer_bottom_inset),
            safe_top_inset: px(if ui.full_screen_cover { layout.safe_top } else { 0.0 }),
            horizontal_safe_inset: px(layout.safe_horizontal),
            sheet_background: layout.sheet_background,
            footer_background: layout.footer_background,
            surface_shadow_enabled: ui.surface_shadow_enabled && !is_modal,
            uses_liquid_glass_surface: ui.uses_liquid_glass_surface,
            top_corner_radius: if is_modal { px(0.0) } else { self.resolved_top_corner_radius() },
            footer_shows_top_border: ui.footer_shows_top_border,
            footer_overlays_content: ui.footer_overlays_content,
            footer_bottom_padding: px(layout.footer_bottom_padding),
            footer_horizontal_padding: ui.footer_horizontal_padding,
            footer_top_padding: ui.footer_top_padding,
            drag_gesture: self.drag_gesture(layout),
            host_owns_sheet_pan: is_modal || !ui.content_scrollable,
            motion_controller,
            is_sheet_interaction_locked: is_modal
                || is_dismissing
                || ui.interaction_locked
                || !ui.allows_interactive_dismiss,
            on_sheet_drag_began: Rc::new(move |window, cx| {
                drag_began.begin_drag_for_layout(&drag_began_layout, window, cx);
            }),
            on_sheet_pull_from_scroll_end: Rc::new(move |translation_y, velocity_y, window, cx| {
                pull.handle_release(
                    f32::from(translation_y),
                    f32::from(velocity_y),
                    &pull_layout.detents,
                    pull_layout.safe_bottom,
                    pull_layout.resting_height,
                    window,
                    cx,
                );
            }),
            can_expand_from_scroll_top: !is_modal && layout.expansion_distance > 0.5,
            sheet_expansion_distance: px(layout.expansion_distance),
            content_scrollable: ui.content_scrollable,
            sheet_content: ui.sheet_content.clone(),
            footer: ui.footer.clone(),
        })
        .into_any_element()
    }

    fn resolved_top_corner_radius(&self) -> Pixels {
        self.ui
            .top_corner_radius
            .unwrap_or(px(BottomSheetMetrics::TOP_CORNER_RADIUS))
    }

    fn handle_backdrop_tap(&self, point: Point<Pixels>, window: &mut Window, cx: &mut App) {
        if let Some(interceptor) = self.ui.backdrop_tap_interceptor.clone()
            && interceptor(point, window, cx)
        {
            return;
        }
        let now = RiseClock::monotonic().as_secs_f64();
        if now < self.state.read(cx).presentation_tap_guard_deadline {
            return;
        }
        let protection_deadline = 0.0;
        if BottomSheetDismissPolicy::should_dismiss_keyboard_first(
            self.ui.dismisses_keyboard_before_backdrop_dismiss,
            self.keyboard_height(window, cx),
            protection_deadline,
            now,
        ) {
            self.dismiss_keyboard(window, cx);
            return;
        }
        if !BottomSheetDismissPolicy::allows_backdrop_dismiss(
            self.ui.allows_interactive_dismiss,
            self.ui.dismisses_on_backdrop_tap,
            self.ui.interaction_locked,
        ) {
            return;
        }
        self.start_dismiss(window, cx);
    }

    /// escape is the desktop swipe down and follows the interactive dismiss rule
    /// closes the sheet straight from a focused field, the keyboard first step is a touch thing
    /// and on desktop it cost a second press
    fn handle_escape(&self, window: &mut Window, cx: &mut App) -> bool {
        let (is_presented, is_dismissing) = {
            let state = self.state.read(cx);
            (state.is_presented, state.is_dismissing)
        };
        if !is_presented || is_dismissing {
            return false;
        }
        if !self.resolved_allows_dismissal_preview() {
            return false;
        }
        self.start_dismiss(window, cx);
        true
    }

    fn keyboard_height(&self, window: &Window, cx: &App) -> f32 {
        let lift = f32::from(self.ui.keyboard_avoidance_height).max(0.0);
        if lift > 0.0 {
            return lift;
        }
        let focus_handle = &self.state.read(cx).focus_handle;
        if focus_handle.contains_focused(window, cx) && ScreenFocus::text_field_is_focused(window, cx) {
            FOCUSED_FIELD_KEYBOARD_HEIGHT
        } else {
            0.0
        }
    }

    /// why: keyboard dismiss blurs the whole window and that would take escape from the sheet
    /// moving focus to the sheet takes it from the field the same way
    fn dismiss_keyboard(&self, window: &mut Window, cx: &mut App) {
        let focus_handle = self.state.read(cx).focus_handle.clone();
        window.focus(&focus_handle, cx);
    }

    fn resolved_surface_corner_radius(&self) -> f32 {
        if self.ui.full_screen_cover {
            DeviceInfo::screen_corner_radius()
        } else {
            0.0
        }
    }

    fn compute_detents(&self, size: Size<Pixels>, safe_top: f32) -> Vec<f32> {
        let height = f32::from(size.height);
        let cap = if self.ui.full_screen_cover { height } else { (height - safe_top).max(0.0) };
        self.resolved_snap_fractions()
            .into_iter()
            .map(|fraction| cap.min(height * fraction))
            .collect()
    }

    fn controlled_detent_height(&self, size: Size<Pixels>, safe_top: f32, cx: &App) -> Option<f32> {
        let selected = self.ui.selected_snap_fraction.as_ref()?;
        let fraction = selected.get(cx).clamp(0.01, 1.0);
        let height = f32::from(size.height);
        let cap = if self.ui.full_screen_cover { height } else { (height - safe_top).max(0.0) };
        Some(cap.min(height * fraction))
    }

    fn clamped_detent_index(&self, detents: &[f32], cx: &App) -> usize {
        if detents.is_empty() {
            return 0;
        }
        if let Some(selected) = self.ui.selected_snap_fraction.as_ref() {
            let selected = selected.get(cx).clamp(0.01, 1.0);
            return nearest_index(&self.resolved_snap_fractions(), selected);
        }
        self.state.read(cx).selected_detent.min(detents.len() - 1)
    }

    fn begin_drag_for_layout(&self, layout: &SheetLayout, window: &mut Window, cx: &mut App) {
        self.begin_drag(
            layout.resting_height,
            layout.detents.first().copied().unwrap_or(layout.resting_height),
            layout.maximum_height,
            window,
            cx,
        );
    }

    fn begin_drag(
        &self,
        rest_height: f32,
        minimum_height: f32,
        maximum_height: f32,
        window: &mut Window,
        cx: &mut App,
    ) {
        let motion_controller = {
            let state = self.state.read(cx);
            if state.drag_base_height.is_some() {
                return;
            }
            state.motion_controller.clone()
        };
        if self.keyboard_height(window, cx) > 0.0 && rest_height >= maximum_height - 0.5 {
            self.dismiss_keyboard(window, cx);
        }
        let base = motion_controller.begin_interaction(
            px(rest_height),
            px(minimum_height),
            px(maximum_height),
            cx,
        );
        self.state.update(cx, |state, cx| {
            state.drag_base_height = Some(base);
            cx.notify();
        });
    }

    fn end_drag(&self, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.end_drag();
            cx.notify();
        });
    }

    fn drag_gesture(&self, layout: &SheetLayout) -> BottomSheetDragGesture {
        let changed = self.clone();
        let changed_layout = layout.clone();
        let ended = self.clone();
        let ended_layout = layout.clone();
        BottomSheetDragGesture {
            on_changed: Rc::new(move |value, window, cx| {
                changed.drag_changed(value, &changed_layout, window, cx);
            }),
            on_ended: Rc::new(move |value, window, cx| {
                ended.drag_ended(value, &ended_layout, window, cx);
            }),
        }
    }

    fn drag_changed(
        &self,
        value: &BottomSheetDragValue,
        layout: &SheetLayout,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (is_dismissing, axis, motion_controller) = {
            let state = self.state.read(cx);
            (state.is_dismissing, state.sheet_drag_axis, state.motion_controller.clone())
        };
        if is_dismissing
            || self.ui.interaction_locked
            || !self.ui.allows_interactive_dismiss
            || BottomSheetAxisOwnership::horizontal_is_driving(cx)
        {
            return;
        }

        let mut axis = axis;
        if axis == SheetDragAxis::Undetermined {
            let dx = f32::from(value.translation.width).abs();
            let dy = f32::from(value.translation.height).abs();
            if dx.max(dy) < 5.0 {
                return;
            }
            axis = if dx > dy * 1.08 { SheetDragAxis::Horizontal } else { SheetDragAxis::Vertical };
            let activation = value.translation.height;
            self.state.update(cx, |state, cx| {
                state.sheet_drag_axis = axis;
                state.drag_activation_translation = activation;
                cx.notify();
            });
        }

        if axis != SheetDragAxis::Vertical {
            return;
        }
        self.begin_drag_for_layout(layout, window, cx);
        let activation = self.state.read(cx).drag_activation_translation;
        motion_controller.update_interactive_translation(value.translation.height - activation, cx);
    }

    fn drag_ended(
        &self,
        value: &BottomSheetDragValue,
        layout: &SheetLayout,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (axis, activation, drag_base_height) = {
            let state = self.state.read(cx);
            (state.sheet_drag_axis, state.drag_activation_translation, state.drag_base_height)
        };
        if axis != SheetDragAxis::Vertical {
            self.end_drag(cx);
            return;
        }
        self.handle_release(
            f32::from(value.translation.height - activation),
            f32::from(value.velocity.height),
            &layout.detents,
            layout.safe_bottom,
            drag_base_height.map(f32::from).unwrap_or(layout.resting_height),
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_release(
        &self,
        translation_height: f32,
        velocity_height: f32,
        detents: &[f32],
        safe_bottom: f32,
        rest_height: f32,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (is_dismissing, drag_base_height, motion_controller) = {
            let state = self.state.read(cx);
            (state.is_dismissing, state.drag_base_height, state.motion_controller.clone())
        };
        if is_dismissing {
            return;
        }

        motion_controller.finish_interactive_updates(cx);
        let base_height = drag_base_height.map(f32::from).unwrap_or(rest_height);
        let current_index = nearest_index(detents, base_height);
        let decision = BottomSheetReleasePolicy::resolve(
            self.ui.interaction_style,
            detents,
            current_index,
            base_height,
            translation_height,
            velocity_height,
        );

        let new_index = match decision {
            BottomSheetReleaseDecision::Dismiss => {
                self.run_dismiss_animation(safe_bottom, window, cx);
                return;
            }
            BottomSheetReleaseDecision::Snap { index } => index,
        };
        let target_height = detents.get(new_index).copied().unwrap_or(base_height);

        self.state.update(cx, |state, cx| {
            state.selected_detent = new_index;
            cx.notify();
        });
        if let Some(fraction) = self.resolved_snap_fractions().get(new_index).copied()
            && let Some(selected) = self.ui.selected_snap_fraction.as_ref()
        {
            selected.set(fraction, window, cx);
        }
        self.end_drag(cx);

        motion_controller.animate_snap(
            px(target_height),
            self.resolved_release_snap_duration(current_index, new_index),
            self.resolved_release_snap_curve(
                current_index,
                new_index,
                base_height,
                target_height,
                velocity_height,
            ),
            |_| {},
            cx,
        );
    }

    fn begin_open(&self, window: &mut Window, cx: &mut App) {
        let selected_detent = match self.ui.selected_snap_fraction.as_ref() {
            Some(selected) => {
                let selected = selected.get(cx).clamp(0.01, 1.0);
                nearest_index(&self.resolved_snap_fractions(), selected)
            }
            None => 0,
        };
        let focused = window.focused(cx);
        let motion_controller = self.state.update(cx, |state, cx| {
            state.is_dismissing = false;
            state.modal_dismissal = None;
            state.selected_detent = selected_detent;
            state.end_drag();
            if !state.is_presented && focused.as_ref() != Some(&state.focus_handle) {
                state.focus_before_open = focused;
            }
            cx.notify();
            state.motion_controller.clone()
        });

        let guard_duration = if self.ui.presents_as_modal() {
            if self.ui.appears_in_place {
                self.state.update(cx, |state, _| state.modal_appearance.jump(1.0));
            }
            ModalSheetMetrics::PRESENT_DURATION
        } else {
            let present_duration = self.resolved_present_duration();
            let window_height = f32::from(window.viewport_size().height);
            motion_controller.prepare_presentation(
                px(if self.ui.appears_in_place { 0.0 } else { window_height }),
                if self.ui.appears_in_place { 0.01 } else { present_duration },
                cx,
            );
            present_duration.max(BottomSheetMetrics::PRESENT_DURATION)
        };
        let guard_deadline = if self.ui.appears_in_place {
            0.0
        } else {
            RiseClock::monotonic().as_secs_f64() + guard_duration
        };
        let focus_handle = self.state.update(cx, |state, cx| {
            state.presentation_tap_guard_deadline = guard_deadline;
            state.is_presented = true;
            cx.notify();
            state.focus_handle.clone()
        });
        ScreenFocus::register_sheet(&focus_handle, cx);
        window.focus(&focus_handle, cx);
        modal_sheet_settle_pointer(self.state.entity_id(), window);
    }

    fn start_dismiss(&self, window: &mut Window, cx: &mut App) {
        if self.state.read(cx).is_dismissing {
            return;
        }
        self.run_dismiss_animation(WINDOW_SAFE_INSET, window, cx);
    }

    fn run_dismiss_animation(&self, safe_bottom: f32, window: &mut Window, cx: &mut App) {
        let motion_controller = {
            let state = self.state.read(cx);
            if state.is_dismissing {
                return;
            }
            state.motion_controller.clone()
        };
        self.state.update(cx, |state, cx| {
            state.is_dismissing = true;
            cx.notify();
        });
        self.ui.is_bottom_sheet.set(false, window, cx);

        if self.ui.presents_as_modal() {
            let finish = self.clone();
            let delay = Duration::from_secs_f64(ModalSheetMetrics::DISMISS_DURATION);
            let dismissal = window.spawn(cx, async move |cx| {
                cx.background_executor().timer(delay).await;
                cx.update(|window, cx| finish.finish_dismiss(window, cx)).ok();
            });
            self.state.update(cx, |state, _| state.modal_dismissal = Some(dismissal));
            return;
        }

        let finish = self.clone();
        motion_controller.animate_dismiss(
            px(safe_bottom),
            self.resolved_dismiss_duration(),
            move |cx| {
                let window_handle = finish.window_handle;
                cx.defer(move |cx| {
                    window_handle
                        .update(cx, |_, window, cx| finish.finish_dismiss(window, cx))
                        .ok();
                });
            },
            cx,
        );
    }

    fn finish_dismiss(&self, window: &mut Window, cx: &mut App) {
        tracing::warn!(target: "riseonly::X808", reopened = self.ui.is_bottom_sheet.get(cx), "full page finish dismiss");
        if self.ui.is_bottom_sheet.get(cx) {
            return;
        }
        let (focus_handle, focus_before_open) = self.state.update(cx, |state, cx| {
            state.is_presented = false;
            state.is_dismissing = false;
            state.modal_dismissal = None;
            state.selected_detent = 0;
            state.end_drag();
            cx.notify();
            (state.focus_handle.clone(), state.focus_before_open.take())
        });
        if self.ui.presents_as_modal()
            && let Some(observer) = self.ui.on_live_sheet_frame.clone()
        {
            observer(None, cx);
        }
        if let Some(previous) = focus_before_open
            && (focus_handle.contains_focused(window, cx) || ScreenFocus::is_idle(window, cx))
        {
            window.focus(&previous, cx);
        }
        (self.ui.on_dismiss)(window, cx);
    }

    fn resolved_snap_fractions(&self) -> Vec<f32> {
        let mut values: Vec<f32> = self
            .ui
            .snap_fractions
            .iter()
            .copied()
            .filter(|fraction| *fraction > 0.0 && *fraction <= 1.0)
            .collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        if values.is_empty() { vec![1.0] } else { values }
    }

    fn resolved_snap_animation_duration(&self) -> f64 {
        if self.reduce_motion { 0.01 } else { self.ui.snap_animation_duration }
    }

    fn resolved_present_duration(&self) -> f64 {
        if self.reduce_motion { 0.01 } else { BottomSheetMetrics::PRESENT_DURATION }
    }

    fn resolved_dismiss_duration(&self) -> f64 {
        if self.reduce_motion { 0.01 } else { BottomSheetMetrics::DISMISS_DURATION }
    }

    /// without this the surface cant go below its smallest detent
    /// so a downward drag at the first detent shows no motion and the sheet only reacts on release
    fn resolved_allows_dismissal_preview(&self) -> bool {
        self.ui.allows_interactive_dismiss && !self.ui.interaction_locked
    }

    fn resolved_configuration_snap_curve(&self) -> BottomSheetSnapAnimationCurve {
        if self.reduce_motion {
            return BottomSheetSnapAnimationCurve::EaseOut;
        }
        if self.ui.interaction_style == BottomSheetInteractionStyle::TelegramWebApp {
            BottomSheetSnapAnimationCurve::TelegramSpring { initial_velocity: 0.0 }
        } else {
            BottomSheetSnapAnimationCurve::EaseOut
        }
    }

    fn resolved_release_snap_duration(&self, current_index: usize, target_index: usize) -> f64 {
        if self.reduce_motion {
            return 0.01;
        }
        if self.ui.interaction_style == BottomSheetInteractionStyle::TelegramWebApp
            && current_index == target_index
        {
            return 0.3;
        }
        self.ui.snap_animation_duration
    }

    fn resolved_release_snap_curve(
        &self,
        current_index: usize,
        target_index: usize,
        resting_height: f32,
        target_height: f32,
        velocity_y: f32,
    ) -> BottomSheetSnapAnimationCurve {
        if self.reduce_motion {
            return BottomSheetSnapAnimationCurve::EaseOut;
        }
        if self.ui.interaction_style != BottomSheetInteractionStyle::TelegramWebApp {
            return BottomSheetSnapAnimationCurve::EaseOut;
        }
        if current_index == target_index {
            return BottomSheetSnapAnimationCurve::EaseInOut;
        }
        let distance = (target_height - resting_height).abs().max(1.0);
        let initial_velocity = (velocity_y.abs() / distance).min(20.0);
        BottomSheetSnapAnimationCurve::TelegramSpring { initial_velocity }
    }
}

/// index of the value closest to target, the first one on a tie
fn nearest_index(values: &[f32], target: f32) -> usize {
    values
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (a.1 - target)
                .abs()
                .partial_cmp(&(b.1 - target).abs())
                .unwrap_or(Ordering::Equal)
        })
        .map_or(0, |(index, _)| index)
}

/// namespace of the dismiss rules
pub enum BottomSheetDismissPolicy {}

impl BottomSheetDismissPolicy {
    pub fn allows_backdrop_dismiss(
        allows_interactive_dismiss: bool,
        dismisses_on_backdrop_tap: bool,
        interaction_locked: bool,
    ) -> bool {
        allows_interactive_dismiss && dismisses_on_backdrop_tap && !interaction_locked
    }

    pub fn should_dismiss_keyboard_first(
        enabled: bool,
        keyboard_height: f32,
        protection_deadline: f64,
        now: f64,
    ) -> bool {
        enabled && (keyboard_height > 0.0 || now < protection_deadline)
    }
}
