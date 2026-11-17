use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Bounds, ElementId, Entity, Hsla, Pixels, Point, RenderOnce, Size, Task, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::ThemeStore;
use rise_ui::bottom_sheet_ui::bottom_sheet_interaction_policy::BottomSheetInteractionStyle;
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::{
    BottomSheetMeasuredFraction, BottomSheetMeasuredFractionModifiers,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationContentInteraction, PresentationDetent,
};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_full_page_ui::BottomSheetFullPageUi;
use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::ExperimentalBottomSheetPresentationHost;
use crate::core::ui::bottom_sheet_ui::sheet_floating_header::SheetSurfaceColorKey;

type SheetSlot = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;
type BackdropTapInterceptor = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App) -> bool>;
type LiveSheetFrame = Rc<dyn Fn(Option<Bounds<Pixels>>, &mut App)>;

#[derive(IntoElement)]
pub struct BottomSheetUi {
    id: ElementId,
    is_bottom_sheet: Binding<bool>,
    snap_fractions: Vec<f32>,
    selected_snap_fraction: Option<Binding<f32>>,
    snap_animation_duration: f64,
    interaction_style: BottomSheetInteractionStyle,
    full_page: bool,
    content_is_edge_to_edge: bool,
    content_under_handle: bool,
    handle_over_content_is_light: bool,
    experimental_bottom_sheet: bool,
    experimental_uses_liquid_glass_surface: bool,
    full_page_allows_swipe_dismiss: bool,
    dismisses_on_backdrop_tap: bool,
    dismisses_keyboard_before_backdrop_dismiss: bool,
    full_page_shows_drag_indicator: bool,
    full_page_hides_footer: bool,
    full_page_avoids_keyboard_when_footer_hidden: bool,
    keyboard_avoidance_height: f32,
    keyboard_footer_spacing: f32,
    footer_shows_top_border: bool,
    footer_overlays_content: Option<bool>,
    footer_horizontal_padding: Option<f32>,
    footer_top_padding: Option<f32>,
    content_scrollable: bool,
    fits_content: bool,
    fits_content_with_keyboard: bool,
    modal_sized_by_detent: bool,
    full_page_covers_full_screen: bool,
    full_page_interaction_locked: bool,
    sheet_background: Option<Hsla>,
    footer_background: Option<Hsla>,
    surface_shadow_enabled: bool,
    modal_chrome_hidden: bool,
    modal_edge_underlay: Option<SheetSlot>,
    backdrop_opacity: Option<f64>,
    backdrop_color: Option<Hsla>,
    backdrop_blur: Option<f32>,
    backdrop_tap_interceptor: Option<BackdropTapInterceptor>,
    on_live_sheet_frame: Option<LiveSheetFrame>,
    on_dismiss: SheetAction,
    on_did_dismiss: SheetAction,
    sheet_content: SheetSlot,
    footer: SheetSlot,
}

impl BottomSheetUi {
    pub fn new<C: IntoElement, F: IntoElement>(
        id: impl Into<ElementId>,
        is_bottom_sheet: Binding<bool>,
        sheet_content: impl Fn(&mut Window, &mut App) -> C + 'static,
        footer: impl Fn(&mut Window, &mut App) -> F + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            is_bottom_sheet,
            snap_fractions: vec![0.60, 0.93],
            selected_snap_fraction: None,
            snap_animation_duration: BottomSheetMetrics::SNAP_DURATION,
            interaction_style: BottomSheetInteractionStyle::Standard,
            full_page: false,
            content_is_edge_to_edge: false,
            content_under_handle: false,
            handle_over_content_is_light: true,
            experimental_bottom_sheet: true,
            experimental_uses_liquid_glass_surface: false,
            full_page_allows_swipe_dismiss: true,
            dismisses_on_backdrop_tap: true,
            dismisses_keyboard_before_backdrop_dismiss: false,
            full_page_shows_drag_indicator: true,
            full_page_hides_footer: false,
            full_page_avoids_keyboard_when_footer_hidden: false,
            keyboard_avoidance_height: 0.0,
            keyboard_footer_spacing: 0.0,
            footer_shows_top_border: true,
            footer_overlays_content: None,
            footer_horizontal_padding: None,
            footer_top_padding: None,
            content_scrollable: false,
            fits_content: false,
            fits_content_with_keyboard: false,
            modal_sized_by_detent: false,
            full_page_covers_full_screen: false,
            full_page_interaction_locked: false,
            sheet_background: None,
            footer_background: None,
            surface_shadow_enabled: true,
            modal_chrome_hidden: false,
            modal_edge_underlay: None,
            backdrop_opacity: None,
            backdrop_color: None,
            backdrop_blur: None,
            backdrop_tap_interceptor: None,
            on_live_sheet_frame: None,
            on_dismiss: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_did_dismiss: Rc::new(|_: &mut Window, _: &mut App| {}),
            sheet_content: Rc::new(move |window, cx| sheet_content(window, cx).into_any_element()),
            footer: Rc::new(move |window, cx| footer(window, cx).into_any_element()),
        }
    }

    pub fn snap_fractions(mut self, snap_fractions: Vec<f32>) -> Self {
        self.snap_fractions = snap_fractions;
        self
    }

    pub fn selected_snap_fraction(mut self, selected_snap_fraction: Option<Binding<f32>>) -> Self {
        self.selected_snap_fraction = selected_snap_fraction;
        self
    }

    pub fn snap_animation_duration(mut self, snap_animation_duration: f64) -> Self {
        self.snap_animation_duration = snap_animation_duration;
        self
    }

    pub fn interaction_style(mut self, interaction_style: BottomSheetInteractionStyle) -> Self {
        self.interaction_style = interaction_style;
        self
    }

    pub fn full_page(mut self, full_page: bool) -> Self {
        self.full_page = full_page;
        self
    }

    pub fn content_is_edge_to_edge(mut self, content_is_edge_to_edge: bool) -> Self {
        self.content_is_edge_to_edge = content_is_edge_to_edge;
        self
    }

    pub fn content_under_handle(mut self, content_under_handle: bool) -> Self {
        self.content_under_handle = content_under_handle;
        self
    }

    pub fn handle_over_content_is_light(mut self, is_light: bool) -> Self {
        self.handle_over_content_is_light = is_light;
        self
    }

    pub fn experimental_bottom_sheet(mut self, experimental_bottom_sheet: bool) -> Self {
        self.experimental_bottom_sheet = experimental_bottom_sheet;
        self
    }

    pub fn experimental_uses_liquid_glass_surface(mut self, uses: bool) -> Self {
        self.experimental_uses_liquid_glass_surface = uses;
        self
    }

    pub fn full_page_allows_swipe_dismiss(mut self, allows: bool) -> Self {
        self.full_page_allows_swipe_dismiss = allows;
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

    pub fn full_page_shows_drag_indicator(mut self, shows: bool) -> Self {
        self.full_page_shows_drag_indicator = shows;
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

    pub fn keyboard_avoidance_height(mut self, height: f32) -> Self {
        self.keyboard_avoidance_height = height;
        self
    }

    pub fn keyboard_footer_spacing(mut self, spacing: f32) -> Self {
        self.keyboard_footer_spacing = spacing;
        self
    }

    pub fn footer_shows_top_border(mut self, shows: bool) -> Self {
        self.footer_shows_top_border = shows;
        self
    }

    pub fn footer_overlays_content(mut self, overlays: Option<bool>) -> Self {
        self.footer_overlays_content = overlays;
        self
    }

    pub fn footer_horizontal_padding(mut self, padding: Option<f32>) -> Self {
        self.footer_horizontal_padding = padding;
        self
    }

    pub fn footer_top_padding(mut self, padding: Option<f32>) -> Self {
        self.footer_top_padding = padding;
        self
    }

    pub fn content_scrollable(mut self, content_scrollable: bool) -> Self {
        self.content_scrollable = content_scrollable;
        self
    }

    pub fn fits_content(mut self, fits: bool) -> Self {
        self.fits_content = fits;
        self
    }

    pub fn fits_content_with_keyboard(mut self, fits: bool) -> Self {
        self.fits_content_with_keyboard = fits;
        self
    }

    pub fn modal_sized_by_detent(mut self, sized: bool) -> Self {
        self.modal_sized_by_detent = sized;
        self
    }

    pub fn full_page_covers_full_screen(mut self, covers: bool) -> Self {
        self.full_page_covers_full_screen = covers;
        self
    }

    pub fn full_page_interaction_locked(mut self, locked: bool) -> Self {
        self.full_page_interaction_locked = locked;
        self
    }

    pub fn sheet_background(mut self, sheet_background: Option<Hsla>) -> Self {
        self.sheet_background = sheet_background;
        self
    }

    pub fn footer_background(mut self, footer_background: Option<Hsla>) -> Self {
        self.footer_background = footer_background;
        self
    }

    pub fn surface_shadow_enabled(mut self, enabled: bool) -> Self {
        self.surface_shadow_enabled = enabled;
        self
    }

    /// desktop modal only, no fill hairline or shadow on the dialog and the backdrop stays
    pub fn modal_chrome_hidden(mut self, hidden: bool) -> Self {
        self.modal_chrome_hidden = hidden;
        self
    }

    /// desktop modal only, drawn under the content across the dialog
    /// shows in the ring the content inset leaves so edge to edge chrome spans the full dialog width
    pub fn modal_edge_underlay<E: IntoElement>(mut self, underlay: impl Fn(&mut Window, &mut App) -> E + 'static) -> Self {
        self.modal_edge_underlay = Some(Rc::new(move |window: &mut Window, cx: &mut App| underlay(window, cx).into_any_element()));
        self
    }

    pub fn backdrop_opacity(mut self, backdrop_opacity: Option<f64>) -> Self {
        self.backdrop_opacity = backdrop_opacity;
        self
    }

    pub fn backdrop_color(mut self, backdrop_color: Option<Hsla>) -> Self {
        self.backdrop_color = backdrop_color;
        self
    }

    pub fn backdrop_blur(mut self, backdrop_blur: Option<f32>) -> Self {
        self.backdrop_blur = backdrop_blur;
        self
    }

    pub fn backdrop_tap_interceptor(
        mut self,
        interceptor: impl Fn(Point<Pixels>, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.backdrop_tap_interceptor = Some(Rc::new(interceptor));
        self
    }

    pub fn on_live_sheet_frame(
        mut self,
        action: impl Fn(Option<Bounds<Pixels>>, &mut App) + 'static,
    ) -> Self {
        self.on_live_sheet_frame = Some(Rc::new(action));
        self
    }

    pub fn on_dismiss(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Rc::new(action);
        self
    }

    pub fn on_did_dismiss(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_did_dismiss = Rc::new(action);
        self
    }

    pub fn on_presentation_refused(
        self,
        _action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self
    }
}

/// state of the fitted content, kept per sheet in element state
struct FittedState {
    fitted_content_height: Animated<f32>,
    fitted_footer_height: f32,
    fitted_last_size: Size<Pixels>,
    fitted_content_overflows: bool,
    fitted_overflow_width: f32,
    fitted_overflow_probed: bool,
    fitted_overflow_probe: Option<Task<()>>,
    fitted_presentation_id: u64,
    fitted_keyboard_shown: bool,
}

impl FittedState {
    fn new() -> Self {
        Self {
            fitted_content_height: Animated::new(0.0),
            fitted_footer_height: 0.0,
            fitted_last_size: Size::default(),
            fitted_content_overflows: false,
            fitted_overflow_width: 0.0,
            fitted_overflow_probed: false,
            fitted_overflow_probe: None,
            fitted_presentation_id: 0,
            fitted_keyboard_shown: false,
        }
    }
}

impl RenderOnce for BottomSheetUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.presentation_body(window, cx)
    }
}

impl BottomSheetUi {
    fn presentation_body(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let state = window.use_keyed_state((self.id.clone(), "fitted"), cx, |_, _| FittedState::new());
        if self.full_page {
            self.custom_sheet(&state, true, false, None, window, cx).into_any_element()
        } else if self.experimental_bottom_sheet {
            let reopen_state = state.clone();
            let reopen_shown = self.is_bottom_sheet.clone();
            let fits_content = self.fits_content;
            let fits_content_with_keyboard = self.fits_content_with_keyboard;
            on_change(
                (self.id.clone(), "fitted-is-presented"),
                self.is_bottom_sheet.get(cx),
                false,
                window,
                cx,
                move |_, shown, window, cx| {
                    if *shown && fits_content {
                        let last = reopen_state.read(cx).fitted_last_size;
                        Self::update_fitted_content_size(
                            &reopen_state,
                            last,
                            &reopen_shown,
                            fits_content_with_keyboard,
                            window,
                            cx,
                        );
                    }
                },
            );
            let on_did_dismiss = self.on_did_dismiss.clone();
            let reset_state = state.clone();
            let host_id = (self.id.clone(), "experimental-host");
            let is_presented = self.is_bottom_sheet.clone();
            let uses_liquid_glass_surface = self.experimental_uses_liquid_glass_surface;
            let sheet = Rc::new(self);
            ExperimentalBottomSheetPresentationHost::new(host_id, is_presented, move |window, cx| {
                sheet.custom_sheet(
                    &state,
                    false,
                    uses_liquid_glass_surface,
                    Some(BottomSheetMetrics::EXPERIMENTAL_TOP_CORNER_RADIUS),
                    window,
                    cx,
                )
            })
            .on_did_dismiss(move |window, cx| {
                Self::reset_fitted_content(&reset_state, cx);
                on_did_dismiss(window, cx);
            })
            .into_any_element()
        } else {
            self.native_sheet(window, cx)
        }
    }

    fn resolved_detents(&self) -> Vec<PresentationDetent> {
        let fractions = self.resolved_fractions();
        if let [only] = fractions.as_slice()
            && *only >= 0.99
        {
            return vec![PresentationDetent::Large];
        }
        fractions.into_iter().map(PresentationDetent::Fraction).collect()
    }

    fn resolved_fractions(&self) -> Vec<f32> {
        let mut fractions: Vec<f32> = self
            .snap_fractions
            .iter()
            .copied()
            .filter(|fraction| *fraction > 0.0 && *fraction <= 1.0)
            .collect();
        fractions.sort_by(f32::total_cmp);
        if fractions.is_empty() { vec![0.6, 0.92] } else { fractions }
    }

    fn presentation_detent_binding(&self, fraction: Binding<f32>) -> Binding<PresentationDetent> {
        let fractions = Rc::new(self.resolved_fractions());
        let get_fractions = fractions.clone();
        let get_fraction = fraction.clone();
        Binding::new(
            move |cx| {
                let wanted = get_fraction.get(cx);
                let closest = get_fractions
                    .iter()
                    .copied()
                    .min_by(|a, b| (a - wanted).abs().total_cmp(&(b - wanted).abs()))
                    .unwrap_or(1.0);
                if closest >= 0.99 {
                    PresentationDetent::Large
                } else {
                    PresentationDetent::Fraction(closest)
                }
            },
            move |detent, window, cx| {
                if detent == PresentationDetent::Large {
                    fraction.set(fractions.last().copied().unwrap_or(1.0), window, cx);
                    return;
                }
                if let Some(matched) = fractions
                    .iter()
                    .copied()
                    .find(|candidate| PresentationDetent::Fraction(*candidate) == detent)
                {
                    fraction.set(matched, window, cx);
                }
            },
        )
    }

    fn custom_sheet(
        &self,
        state: &Entity<FittedState>,
        uses_full_page_layout: bool,
        uses_liquid_glass_surface: bool,
        top_corner_radius: Option<f32>,
        window: &Window,
        cx: &App,
    ) -> BottomSheetFullPageUi {
        let snap_fractions =
            if self.fits_content { vec![self.fitted_fraction(state, window, cx)] } else { self.snap_fractions.clone() };
        let sheet_content = self.sheet_content.clone();
        let footer = self.footer.clone();
        let on_dismiss = self.on_dismiss.clone();
        let content_id = self.id.clone();
        let footer_id = self.id.clone();
        let content_state = state.clone();
        let footer_state = state.clone();
        let fits_content = self.fits_content;
        let fits_content_with_keyboard = self.fits_content_with_keyboard;
        let is_bottom_sheet = self.is_bottom_sheet.clone();
        let footer_is_bottom_sheet = self.is_bottom_sheet.clone();
        let mut sheet = BottomSheetFullPageUi::new(
            (self.id.clone(), "custom-sheet"),
            self.is_bottom_sheet.clone(),
            move |window, cx| {
                let content = sheet_content(window, cx);
                if !fits_content {
                    return content;
                }
                Self::fitted_content(
                    &content_id,
                    &content_state,
                    content,
                    &is_bottom_sheet,
                    fits_content_with_keyboard,
                    cx,
                )
            },
            move |window, cx| {
                let footer = footer(window, cx);
                if !fits_content {
                    return footer;
                }
                let state = footer_state.clone();
                let footer_is_bottom_sheet = footer_is_bottom_sheet.clone();
                footer
                    .on_sheet_part_height((footer_id.clone(), "fitted-footer"), move |height, window, cx| {
                        let was_latched = {
                            let current = state.read(cx);
                            if (current.fitted_footer_height - height).abs() <= 0.5 {
                                return;
                            }
                            current.fitted_content_overflows
                        };
                        state.update(cx, |state, cx| {
                            state.fitted_footer_height = height;
                            if was_latched {
                                state.fitted_content_overflows = false;
                                state.fitted_overflow_width = 0.0;
                            }
                            cx.notify();
                        });
                        if !was_latched {
                            let last = state.read(cx).fitted_last_size;
                            Self::update_fitted_content_size(
                                &state,
                                last,
                                &footer_is_bottom_sheet,
                                fits_content_with_keyboard,
                                window,
                                cx,
                            );
                        }
                    })
                    .into_any_element()
            },
        )
        .snap_fractions(snap_fractions)
        .content_is_edge_to_edge(self.content_is_edge_to_edge)
        .content_under_handle(self.content_under_handle)
        .handle_over_content_is_light(self.handle_over_content_is_light)
        .selected_snap_fraction(self.selected_snap_fraction.clone())
        .snap_animation_duration(self.snap_animation_duration)
        .interaction_style(self.interaction_style)
        .shows_drag_indicator(self.full_page_shows_drag_indicator)
        .full_page_hides_footer(self.full_page_hides_footer)
        .full_page_avoids_keyboard_when_footer_hidden(
            self.full_page_avoids_keyboard_when_footer_hidden,
        )
        .keyboard_avoidance_height(px(self.keyboard_avoidance_height))
        .keyboard_footer_spacing(px(self.keyboard_footer_spacing))
        .footer_shows_top_border(self.footer_shows_top_border)
        .footer_overlays_content(self.resolved_footer_overlays_content())
        .footer_horizontal_padding(self.resolved_footer_horizontal_padding().map(px))
        .footer_top_padding(self.resolved_footer_top_padding().map(px))
        .content_scrollable(self.content_scrollable)
        .full_screen_cover(self.full_page_covers_full_screen)
        .modal_sized_by_detent(self.modal_sized_by_detent || self.fits_content)
        .allows_interactive_dismiss(self.full_page_allows_swipe_dismiss)
        .dismisses_on_backdrop_tap(self.dismisses_on_backdrop_tap)
        .dismisses_keyboard_before_backdrop_dismiss(self.dismisses_keyboard_before_backdrop_dismiss)
        .interaction_locked(self.full_page_interaction_locked)
        .sheet_background(self.sheet_background)
        .footer_background(self.resolved_footer_background())
        .surface_shadow_enabled(self.surface_shadow_enabled)
        .modal_chrome_hidden(self.modal_chrome_hidden)
        .modal_edge_underlay(self.modal_edge_underlay.clone())
        .backdrop_opacity(self.backdrop_opacity)
        .backdrop_color(self.backdrop_color)
        .backdrop_blur(self.backdrop_blur)
        .uses_full_page_layout(uses_full_page_layout)
        .uses_liquid_glass_surface(uses_liquid_glass_surface)
        .top_corner_radius(top_corner_radius.map(px))
        .on_dismiss(move |window, cx| on_dismiss(window, cx));
        if let Some(interceptor) = self.backdrop_tap_interceptor.clone() {
            sheet = sheet.backdrop_tap_interceptor(move |point, window, cx| {
                interceptor(point, window, cx)
            });
        }
        if let Some(on_live_sheet_frame) = self.on_live_sheet_frame.clone() {
            sheet = sheet.on_live_sheet_frame(move |frame, cx| on_live_sheet_frame(frame, cx));
        }
        sheet
    }

    fn fitted_content(
        id: &ElementId,
        state: &Entity<FittedState>,
        content: AnyElement,
        is_bottom_sheet: &Binding<bool>,
        fits_content_with_keyboard: bool,
        cx: &App,
    ) -> AnyElement {
        let (overflows, keyboard_shown) = {
            let state = state.read(cx);
            (state.fitted_content_overflows, state.fitted_keyboard_shown)
        };
        let ideal_height = !overflows && (!keyboard_shown || fits_content_with_keyboard);
        let measured_state = state.clone();
        let is_bottom_sheet = is_bottom_sheet.clone();
        let measured = div()
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .when(ideal_height, |measured| measured.flex_none())
            .when(!ideal_height, |measured| measured.flex_1().min_h_0())
            .child(content)
            .child(measure((id.clone(), "fitted-size"), move |bounds, window, cx| {
                Self::update_fitted_content_size(
                    &measured_state,
                    bounds.size,
                    &is_bottom_sheet,
                    fits_content_with_keyboard,
                    window,
                    cx,
                );
            }));
        div().size_full().flex().flex_col().child(measured).into_any_element()
    }

    fn fitted_fraction(&self, state: &Entity<FittedState>, window: &Window, cx: &App) -> f32 {
        let state = state.read(cx);
        let content_height = if *state.fitted_content_height.target() > 0.0 {
            state.fitted_content_height.frame_value(RiseClock::monotonic(), window)
        } else {
            0.0
        };
        BottomSheetMeasuredFraction::attached(
            content_height,
            state.fitted_footer_height,
            FittedMeasurement::MINIMUM_FRACTION,
            window,
        )
        .unwrap_or_else(|| self.resolved_fractions()[0])
    }

    fn resolved_footer_overlays_content(&self) -> bool {
        self.footer_overlays_content.unwrap_or(false)
    }

    fn resolved_footer_horizontal_padding(&self) -> Option<f32> {
        self.footer_horizontal_padding
            .or(self.resolved_footer_overlays_content().then_some(14.0))
    }

    fn resolved_footer_top_padding(&self) -> Option<f32> {
        self.footer_top_padding
            .or(self.resolved_footer_overlays_content().then_some(8.0))
    }

    fn resolved_footer_background(&self) -> Option<Hsla> {
        self.footer_background
            .or(self.resolved_footer_overlays_content().then_some(gpui::transparent_black()))
    }

    fn native_sheet(self, _window: &mut Window, cx: &mut App) -> AnyElement {
        let theme_store = ThemeStore::shared(cx);
        let (theme_bg, border) = {
            let theme = theme_store.read(cx).current_theme();
            (theme.bg._200, theme.border._200)
        };
        let bg = self.sheet_background.unwrap_or(theme_bg);
        let footer_bg = self.footer_background.unwrap_or(theme_bg);

        let host_id = (self.id.clone(), "native-host");
        let is_presented = self.is_bottom_sheet.clone();
        let on_dismiss = self.on_dismiss.clone();
        let shows_drag_indicator = self.full_page_shows_drag_indicator;
        let interactive_dismiss_disabled =
            !self.full_page_allows_swipe_dismiss || self.full_page_interaction_locked;
        let detents = if self.fits_content { Vec::new() } else { self.resolved_detents() };
        let detent_selection = if self.fits_content {
            None
        } else {
            self.selected_snap_fraction
                .clone()
                .map(|fraction| self.presentation_detent_binding(fraction))
        };
        let content_interaction = if self.content_scrollable {
            PresentationContentInteraction::Scrolls
        } else {
            PresentationContentInteraction::Automatic
        };
        let sheet = Rc::new(self);

        let host = NativeBottomSheetPresentationHost::new(host_id, is_presented, move |window, cx| {
            sheet.native_presented_body(bg, footer_bg, border, window, cx)
        })
        .on_dismiss(move |window, cx| on_dismiss(window, cx))
        .presentation_drag_indicator(shows_drag_indicator)
        .presentation_background(bg)
        .presentation_corner_radius(BottomSheetMetrics::TOP_CORNER_RADIUS)
        .interactive_dismiss_disabled(interactive_dismiss_disabled);
        match detent_selection {
            Some(selection) => host.presentation_detents_selection(detents, selection),
            None => host.presentation_detents(detents),
        }
        .presentation_content_interaction(content_interaction)
        .into_any_element()
    }

    fn native_presented_body(
        &self,
        bg: Hsla,
        footer_bg: Hsla,
        border: Hsla,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let sheet_content = (self.sheet_content)(window, cx);
        let content = vstack(0.0)
            .w_full()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .child(env_scope::<SheetSurfaceColorKey>(Some(bg)).child(sheet_content)),
            )
            .when_some(self.native_footer_bar(border, footer_bg, window, cx), |content, bar| {
                content.child(bar)
            })
            .bg(bg);
        div()
            .w_full()
            .flex_1()
            .min_h_0()
            .size_full()
            .flex()
            .flex_col()
            .child(content)
            .into_any_element()
    }

    fn native_footer_bar(
        &self,
        border: Hsla,
        footer_bg: Hsla,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        if self.full_page_hides_footer {
            return None;
        }
        Some(
            vstack(0.0)
                .w_full()
                .when(self.footer_shows_top_border, |bar| {
                    bar.child(div().w_full().h(px(0.5)).bg(border.opacity(0.35)))
                })
                .child(div().w_full().child((self.footer)(window, cx)))
                .bg(footer_bg)
                .into_any_element(),
        )
    }

    /// a pass before the sheet has its real width wraps text into a narrow column
    /// and could latch a short form at full height
    /// such a pass is skipped, and a latch from another width is measured again
    fn update_fitted_content_size(
        state: &Entity<FittedState>,
        size: Size<Pixels>,
        is_bottom_sheet: &Binding<bool>,
        fits_content_with_keyboard: bool,
        window: &Window,
        cx: &mut App,
    ) {
        let height = f32::from(size.height);
        let width = f32::from(size.width);
        let keyboard_shown = {
            let state = state.read(cx);
            state.fitted_keyboard_shown
        };
        state.update(cx, |state, _| state.fitted_last_size = size);
        if !is_bottom_sheet.get(cx)
            || (keyboard_shown && !fits_content_with_keyboard)
            || !height.is_finite()
            || height <= 0.0
            || width < FittedMeasurement::MINIMUM_WIDTH
        {
            return;
        }
        let footer_height = state.read(cx).fitted_footer_height;
        let ceiling = BottomSheetMeasuredFraction::attached_content_ceiling(footer_height, window);
        let now = RiseClock::monotonic();
        let reduce_motion = cx.reduce_motion();
        state.update(cx, |state, cx| {
            if state.fitted_content_overflows {
                if (width - state.fitted_overflow_width).abs() <= 0.5 {
                    return;
                }
                state.fitted_content_overflows = false;
                cx.notify();
                return;
            }

            if height >= ceiling {
                state.fitted_content_overflows = true;
                state.fitted_overflow_width = width;
                state.fitted_content_height.set(
                    ceiling,
                    ease_out(BottomSheetMetrics::SNAP_DURATION),
                    now,
                    reduce_motion,
                );
                Self::probe_fitted_overflow_once(state, is_bottom_sheet, cx);
                cx.notify();
                return;
            }

            if (*state.fitted_content_height.target() - height).abs() <= 0.5 {
                return;
            }
            if *state.fitted_content_height.target() > 0.0 {
                state.fitted_content_height.set(
                    height,
                    ease_out(BottomSheetMetrics::SNAP_DURATION),
                    now,
                    reduce_motion,
                );
            } else {
                state.fitted_content_height.jump(height);
            }
            cx.notify();
        });
    }

    fn probe_fitted_overflow_once(
        state: &mut FittedState,
        is_bottom_sheet: &Binding<bool>,
        cx: &mut gpui::Context<FittedState>,
    ) {
        if state.fitted_overflow_probed {
            return;
        }
        state.fitted_overflow_probed = true;
        let presentation_id = state.fitted_presentation_id;
        let is_bottom_sheet = is_bottom_sheet.clone();
        let delay = Duration::from_secs_f64(BottomSheetMetrics::SNAP_DURATION + 0.12);
        state.fitted_overflow_probe = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |state, cx| {
                if !is_bottom_sheet.get(cx)
                    || state.fitted_presentation_id != presentation_id
                    || !state.fitted_content_overflows
                {
                    return;
                }
                state.fitted_overflow_probe = None;
                state.fitted_content_overflows = false;
                cx.notify();
            });
        }));
    }

    fn reset_fitted_content(state: &Entity<FittedState>, cx: &mut App) {
        state.update(cx, |state, cx| {
            state.fitted_overflow_probe = None;
            state.fitted_presentation_id = state.fitted_presentation_id.wrapping_add(1);
            state.fitted_content_overflows = false;
            state.fitted_overflow_width = 0.0;
            state.fitted_overflow_probed = false;
            state.fitted_keyboard_shown = false;
            cx.notify();
        });
    }
}

enum FittedMeasurement {}

impl FittedMeasurement {
    const MINIMUM_WIDTH: f32 = 100.0;
    const MINIMUM_FRACTION: f32 = 0.2;
}
