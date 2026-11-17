use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, Bounds, Context, ElementId, Entity, Hsla, IntoElement, ParentElement, Pixels, Point,
    Render, RenderOnce, Size, Styled, Window, div, point, prelude::FluentBuilder, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::hold_context_menu_ui::hold_context_menu_controller_preview::HoldContextMenuControllerPreview;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::{
    HoldContextMenuMetrics,
};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{
    HoldContextMenuAccessory, HoldContextMenuAction, HoldContextMenuAlignment, HoldContextMenuDefaults,
    HoldSourceBubbleClip,
};
use rise_ui::hold_context_menu_ui::rise_strict_tap_gesture_recognizer::RiseTapPolicy;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::lifecycle::on_change;
use rise_ui::swiftui::motion::{MotionSpec, Spring};
use rise_ui::swiftui::transition::TransitionModifiers;
use rise_ui::view_anchor_probe::{ViewAnchorProbe, ViewAnchorProbeExt};
use rise_ui::view_screen_snapshot::{
    HoldContextMenuSnapshotRenderRoot, ViewScreenSource,
};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_overlay::HoldContextMenuOverflowPreviewPolicy;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::{
    HoldContextMenuPresenter, HoldContextMenuRequest, HoldContextMenuRequestID, HoldContextMenuSourceLift,
};
use crate::core::ui::hold_context_menu_ui::hold_gesture_plate::HoldGesturePlate;

pub type HoldContextMenuSourceContent = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
pub type HoldContextMenuActionsProvider = Rc<dyn Fn(&App) -> Vec<HoldContextMenuAction>>;
pub type HoldContextMenuStringHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub type HoldContextMenuPointHandler = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;
pub type HoldContextMenuCanDoubleTap = Rc<dyn Fn(&App) -> bool>;
pub type HoldContextMenuControllerPreviewFactory =
    Rc<dyn Fn(Size<Pixels>, &mut Window, &mut App) -> HoldContextMenuControllerPreview>;

const LOG_NAME: &str = "HoldMenu";
const CONTROLLER_PREVIEW_VERTICAL_MARGIN: f32 = 16.0;

/// modifier local state keeps invalidation to the pressed source
/// observing the global presenter here would rebuild every mounted source whenever any menu
/// opened or closed
struct HoldContextMenuSourcePresentationState {
    press_scale: Animated<f32>,
    is_lifted: bool,
    request_id: Option<HoldContextMenuRequestID>,
    probe: ViewAnchorProbe,
    source_view: Entity<HoldContextMenuSourceView>,
}

impl HoldContextMenuSourcePresentationState {
    fn new(content: HoldContextMenuSourceContent, cx: &mut Context<Self>) -> Self {
        Self {
            press_scale: Animated::new(1.0),
            is_lifted: false,
            request_id: None,
            probe: ViewAnchorProbe::new(),
            source_view: cx.new(|_| HoldContextMenuSourceView::new(content)),
        }
    }

    fn reset(&mut self) {
        self.press_scale.jump(1.0);
        self.is_lifted = false;
        self.request_id = None;
    }

    fn source(&self) -> ViewScreenSource {
        ViewScreenSource::new(self.source_view.clone(), self.probe.clone())
    }
}

/// a source subtree as a view so it can be drawn again outside its slot
pub struct HoldContextMenuSourceView {
    content: HoldContextMenuSourceContent,
}

impl HoldContextMenuSourceView {
    pub fn new(content: HoldContextMenuSourceContent) -> Self {
        Self { content }
    }

    pub fn set_content(&mut self, content: HoldContextMenuSourceContent) {
        self.content = content;
    }
}

impl Render for HoldContextMenuSourceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        (self.content)(window, cx)
    }
}

struct HoldLiftedRowCard {
    color: Option<Hsla>,
    source_min_x: f32,
    source_max_x: f32,
    screen_width: f32,
}

impl HoldLiftedRowCard {
    const CORNER_RADIUS: f32 = 24.0;
    const HORIZONTAL_BLEED: f32 = 10.0;

    /// leading and trailing padding of the card, negative bleeds past the row
    fn insets(&self) -> (f32, f32) {
        let edge = HoldContextMenuMetrics::EDGE_INSET;
        if self.source_max_x < self.screen_width - 1.0 {
            return (-Self::HORIZONTAL_BLEED, -Self::HORIZONTAL_BLEED);
        }
        ((edge - self.source_min_x).max(0.0), edge)
    }

    fn body(&self, content: AnyElement) -> AnyElement {
        match self.color {
            Some(color) => div()
                .relative()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(self.insets().0))
                        .right(px(self.insets().1))
                        .rounded(px(Self::CORNER_RADIUS))
                        .bg(color),
                )
                .child(content)
                .into_any_element(),
            None => content,
        }
    }
}

pub enum HoldContextMenuSnapshotCapturePolicy {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationGeometry {
    pub source_frame: Bounds<Pixels>,
    pub initial_source_center: Point<Pixels>,
}

/// rect intersection for the policy checks, none is the null rect
fn intersection(a: Bounds<Pixels>, b: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
    let min_x = f32::max(f32::from(a.origin.x), f32::from(b.origin.x));
    let min_y = f32::max(f32::from(a.origin.y), f32::from(b.origin.y));
    let max_x = f32::min(f32::from(a.right()), f32::from(b.right()));
    let max_y = f32::min(f32::from(a.bottom()), f32::from(b.bottom()));
    (max_x >= min_x && max_y >= min_y).then(|| Bounds::new(point(px(min_x), px(min_y)), size(px(max_x - min_x), px(max_y - min_y))))
}

fn is_empty(rect: Bounds<Pixels>) -> bool {
    f32::from(rect.size.width) <= 0.0 || f32::from(rect.size.height) <= 0.0
}

impl HoldContextMenuSnapshotCapturePolicy {
    pub fn visible_coverage(expected_frame: Bounds<Pixels>, captured_frame: Option<Bounds<Pixels>>) -> f32 {
        let (expected_width, expected_height) =
            (f32::from(expected_frame.size.width), f32::from(expected_frame.size.height));
        if !(expected_width > 0.0 && expected_height > 0.0) {
            return 0.0;
        }
        let Some(captured_frame) = captured_frame.filter(|captured| !is_empty(*captured)) else {
            return 0.0;
        };
        let expected_area = expected_width * expected_height;
        let Some(intersection) = intersection(expected_frame, captured_frame).filter(|rect| !is_empty(*rect)) else {
            return 0.0;
        };
        let area = f32::from(intersection.size.width) * f32::from(intersection.size.height);
        f32::min(1.0, f32::max(0.0, area / expected_area))
    }

    pub fn presentation_geometry(
        full_source_frame: Bounds<Pixels>,
        captured_frame: Option<Bounds<Pixels>>,
        initial_scale: f32,
    ) -> Option<PresentationGeometry> {
        let full_width = f32::from(full_source_frame.size.width);
        let full_height = f32::from(full_source_frame.size.height);
        let captured_frame = captured_frame?;
        if !(full_width > 1.0
            && full_height > 1.0
            && f32::from(captured_frame.size.width) > 1.0
            && f32::from(captured_frame.size.height) > 1.0
            && initial_scale.is_finite()
            && initial_scale > 0.0)
        {
            return None;
        }

        let center = full_source_frame.center();
        let (mid_x, mid_y) = (f32::from(center.x), f32::from(center.y));
        let expected_scaled_frame = Bounds::new(
            point(px(mid_x - full_width * initial_scale / 2.0), px(mid_y - full_height * initial_scale / 2.0)),
            size(px(full_width * initial_scale), px(full_height * initial_scale)),
        );
        let visible_frame = intersection(expected_scaled_frame, captured_frame)
            .filter(|rect| f32::from(rect.size.width) > 1.0 && f32::from(rect.size.height) > 1.0)?;

        let unscaled_frame = Bounds::new(
            point(
                px(mid_x + (f32::from(visible_frame.origin.x) - mid_x) / initial_scale),
                px(mid_y + (f32::from(visible_frame.origin.y) - mid_y) / initial_scale),
            ),
            size(
                px(f32::from(visible_frame.size.width) / initial_scale),
                px(f32::from(visible_frame.size.height) / initial_scale),
            ),
        );
        let unscaled_frame = intersection(unscaled_frame, full_source_frame)
            .filter(|rect| f32::from(rect.size.width) > 1.0 && f32::from(rect.size.height) > 1.0)?;

        Some(PresentationGeometry {
            source_frame: unscaled_frame,
            initial_source_center: visible_frame.center(),
        })
    }
}

#[derive(IntoElement)]
pub struct HoldContextMenuContainer {
    id: ElementId,
    pub alignment: HoldContextMenuAlignment,
    pub show_emoji_list: bool,
    pub emojis: Vec<String>,
    pub allows_custom_emoji: bool,
    pub expanded_emojis: Option<Vec<String>>,
    actions_provider: HoldContextMenuActionsProvider,
    pub source_bubble_clip: Option<HoldSourceBubbleClip>,
    pub accessory: Option<HoldContextMenuAccessory>,
    pub on_emoji: HoldContextMenuStringHandler,
    pub on_action: HoldContextMenuStringHandler,
    pub on_tap: HoldContextMenuPointHandler,
    pub can_double_tap: HoldContextMenuCanDoubleTap,
    pub on_double_tap: Option<HoldContextMenuPointHandler>,
    pub tap_movement_tolerance: f32,
    pub squish_amount: f32,
    pub capture_snapshot: bool,
    pub lifts_live_source: bool,
    pub disabled: bool,
    pub uses_flat_chrome: bool,
    pub lifted_background: Option<Hsla>,
    pub live_action_titles: bool,
    pub lift_inset: f32,
    pub source_identity: Option<String>,
    pub active_request_cell: Option<Rc<Cell<Option<HoldContextMenuRequestID>>>>,
    pub controller_preview: Option<HoldContextMenuControllerPreviewFactory>,
    pub live_source_roots: Vec<Rc<dyn HoldContextMenuSnapshotRenderRoot>>,
    pub context_click_across_row: bool,
    pub content: HoldContextMenuSourceContent,
}

impl HoldContextMenuContainer {
    /// constructor with every other label at its default, the builders below set them
    /// id keeps the press state of this slot across frames
    pub fn new(
        id: impl Into<ElementId>,
        alignment: HoldContextMenuAlignment,
        actions: HoldContextMenuActionsProvider,
        content: HoldContextMenuSourceContent,
    ) -> Self {
        Self {
            id: id.into(),
            alignment,
            show_emoji_list: true,
            emojis: HoldContextMenuDefaults::EMOJIS.iter().map(|emoji| (*emoji).to_owned()).collect(),
            allows_custom_emoji: false,
            expanded_emojis: None,
            actions_provider: actions,
            source_bubble_clip: None,
            accessory: None,
            on_emoji: Rc::new(|_: String, _: &mut Window, _: &mut App| {}),
            on_action: Rc::new(|_: String, _: &mut Window, _: &mut App| {}),
            on_tap: Rc::new(|_: Point<Pixels>, _: &mut Window, _: &mut App| {}),
            can_double_tap: Rc::new(|_: &App| true),
            on_double_tap: None,
            tap_movement_tolerance: RiseTapPolicy::MOVEMENT_SLOP,
            squish_amount: HoldContextMenuMetrics::PRESS_SQUISH_AMOUNT,
            capture_snapshot: false,
            lifts_live_source: false,
            disabled: false,
            uses_flat_chrome: false,
            lifted_background: None,
            live_action_titles: false,
            lift_inset: 0.0,
            source_identity: None,
            active_request_cell: None,
            controller_preview: None,
            live_source_roots: Vec::new(),
            context_click_across_row: false,
            content,
        }
    }

    pub fn context_click_across_row(mut self, context_click_across_row: bool) -> Self {
        self.context_click_across_row = context_click_across_row;
        self
    }

    pub fn show_emoji_list(mut self, show_emoji_list: bool) -> Self {
        self.show_emoji_list = show_emoji_list;
        self
    }

    pub fn emojis(mut self, emojis: Vec<String>) -> Self {
        self.emojis = emojis;
        self
    }

    pub fn allows_custom_emoji(mut self, allows_custom_emoji: bool) -> Self {
        self.allows_custom_emoji = allows_custom_emoji;
        self
    }

    pub fn expanded_emojis(mut self, expanded_emojis: Option<Vec<String>>) -> Self {
        self.expanded_emojis = expanded_emojis;
        self
    }

    pub fn source_bubble_clip(mut self, source_bubble_clip: Option<HoldSourceBubbleClip>) -> Self {
        self.source_bubble_clip = source_bubble_clip;
        self
    }

    pub fn accessory(mut self, accessory: Option<HoldContextMenuAccessory>) -> Self {
        self.accessory = accessory;
        self
    }

    pub fn tap_movement_tolerance(mut self, tap_movement_tolerance: f32) -> Self {
        self.tap_movement_tolerance = tap_movement_tolerance;
        self
    }

    pub fn squish_amount(mut self, squish_amount: f32) -> Self {
        self.squish_amount = squish_amount;
        self
    }

    pub fn capture_snapshot(mut self, capture_snapshot: bool) -> Self {
        self.capture_snapshot = capture_snapshot;
        self
    }

    pub fn lifts_live_source(mut self, lifts_live_source: bool) -> Self {
        self.lifts_live_source = lifts_live_source;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn uses_flat_chrome(mut self, uses_flat_chrome: bool) -> Self {
        self.uses_flat_chrome = uses_flat_chrome;
        self
    }

    pub fn lifted_background(mut self, lifted_background: Option<Hsla>) -> Self {
        self.lifted_background = lifted_background;
        self
    }

    pub fn live_action_titles(mut self, live_action_titles: bool) -> Self {
        self.live_action_titles = live_action_titles;
        self
    }

    pub fn lift_inset(mut self, lift_inset: f32) -> Self {
        self.lift_inset = lift_inset;
        self
    }

    pub fn active_request_cell(mut self, cell: Rc<Cell<Option<HoldContextMenuRequestID>>>) -> Self {
        self.active_request_cell = Some(cell);
        self
    }

    pub fn source_identity(mut self, source_identity: Option<String>) -> Self {
        self.source_identity = source_identity;
        self
    }

    pub fn controller_preview(mut self, controller_preview: Option<HoldContextMenuControllerPreviewFactory>) -> Self {
        self.controller_preview = controller_preview;
        self
    }

    pub fn live_source_roots(mut self, live_source_roots: Vec<Rc<dyn HoldContextMenuSnapshotRenderRoot>>) -> Self {
        self.live_source_roots = live_source_roots;
        self
    }

    pub fn on_emoji(mut self, on_emoji: HoldContextMenuStringHandler) -> Self {
        self.on_emoji = on_emoji;
        self
    }

    pub fn on_action(mut self, on_action: HoldContextMenuStringHandler) -> Self {
        self.on_action = on_action;
        self
    }

    pub fn on_tap(mut self, on_tap: HoldContextMenuPointHandler) -> Self {
        self.on_tap = on_tap;
        self
    }

    pub fn can_double_tap(mut self, can_double_tap: HoldContextMenuCanDoubleTap) -> Self {
        self.can_double_tap = can_double_tap;
        self
    }

    pub fn on_double_tap(mut self, on_double_tap: Option<HoldContextMenuPointHandler>) -> Self {
        self.on_double_tap = on_double_tap;
        self
    }

    /// a press with nothing to offer neither squishes nor lifts the source over an empty menu strip
    /// an uploading grid cell or a row in reorder mode passes no actions
    fn offers_menu(&self, cx: &App) -> bool {
        self.show_emoji_list
            || self.accessory.is_some()
            || self.controller_preview.is_some()
            || !(self.actions_provider)(cx).is_empty()
    }

    fn begin_progressive_press(&self, state: &Entity<HoldContextMenuSourcePresentationState>, cx: &mut App) {
        let is_lifted = state.read(cx).is_lifted;
        let active = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some();
        let offers_menu = self.offers_menu(cx);
        if is_lifted || active || !offers_menu {
            Sai::log(
                LOG_NAME,
                || format!("press skipped lifted={is_lifted} active={active} menu={offers_menu}"),
                SaiLogKind::Ui,
                cx,
            );
            return;
        }
        if let Some(accessory) = self.accessory.as_ref() {
            (accessory.on_prefetch)(cx);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn confirm(
        &self,
        state: &Entity<HoldContextMenuSourcePresentationState>,
        press_location: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let active = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some();
        let offers_menu = self.offers_menu(cx);
        if active || !offers_menu {
            Sai::log(LOG_NAME, || format!("confirm skipped active={active} menu={offers_menu}"), SaiLogKind::Ui, cx);
            return;
        }
        let now = RiseClock::monotonic();
        let (full_frame, press_scale) = {
            let state = state.read(cx);
            (state.probe.window_frame(), state.press_scale.value(now))
        };
        let Some(full_frame) = full_frame else {
            Sai::log(LOG_NAME, || "confirm skipped no window".to_owned(), SaiLogKind::Ui, cx);
            return;
        };
        let full_size = full_frame.size;
        let center = full_frame.center();
        let (mid_x, mid_y) = (f32::from(center.x), f32::from(center.y));
        let (full_width, full_height) = (f32::from(full_size.width), f32::from(full_size.height));
        let (scaled_width, scaled_height) = (full_width * press_scale, full_height * press_scale);
        let scaled_rect = Bounds::new(
            point(px(mid_x - scaled_width / 2.0), px(mid_y - scaled_height / 2.0)),
            size(px(scaled_width), px(scaled_height)),
        );
        if !(scaled_width > 1.0 && scaled_height > 1.0 && full_width > 1.0 && full_height > 1.0) {
            Sai::log(
                LOG_NAME,
                || format!("confirm skipped size scaled={scaled_width}x{scaled_height} full={full_width}x{full_height}"),
                SaiLogKind::Ui,
                cx,
            );
            return;
        }
        let source_frame = Bounds::new(point(px(mid_x - full_width / 2.0), px(mid_y - full_height / 2.0)), full_size);
        let initial_lift_scale = f32::max(0.5, f32::min(1.0, scaled_width / full_width));
        let scroll_lock_from = state.read(cx).source();

        let wants_peek = window.modifiers().alt;
        let peeks = wants_peek && self.controller_preview.is_some();
        if !peeks && !self.show_emoji_list && self.accessory.is_none() && (self.actions_provider)(cx).is_empty() {
            Sai::log(LOG_NAME, || "confirm skipped no actions".to_owned(), SaiLogKind::Ui, cx);
            return;
        }
        if let Some(controller_preview) = self.controller_preview.as_ref().filter(|_| wants_peek) {
            let viewport = window.viewport_size();
            let available = size(viewport.width, viewport.height - px(CONTROLLER_PREVIEW_VERTICAL_MARGIN));
            let mut preview = controller_preview(available, window, cx);
            let actions = (self.actions_provider)(cx);
            let accessory_height = 0.0;
            preview.fit_menu(HoldContextMenuOverflowPreviewPolicy::estimated_menu_height(&actions, accessory_height));
            preview.source_frame = source_frame;
            let preview_size = preview.size();
            let id = HoldContextMenuRequestID::new();
            if let Some(cell) = &self.active_request_cell {
                cell.set(Some(id));
            }
            state.update(cx, |state, _| state.request_id = Some(id));
            let lifted = state.clone();
            let restored = state.clone();
            HoldContextMenuPresenter::present(
                HoldContextMenuRequest {
                    id,
                    source_frame: Bounds::new(source_frame.origin, preview_size),
                    initial_source_center: source_frame.center(),
                    alignment: HoldContextMenuAlignment::Trailing,
                    show_emoji_list: false,
                    emojis: Vec::new(),
                    allows_custom_emoji: false,
                    expanded_emojis: None,
                    actions,
                    accessory: None,
                    on_emoji: Rc::new(|_: String, _: &mut Window, _: &mut App| {}),
                    on_action: self.on_action.clone(),
                    on_lift_source: Rc::new(move |cx: &mut App| {
                        lifted.update(cx, |state, cx| {
                            if state.request_id != Some(id) {
                                return;
                            }
                            state.press_scale.jump(1.0);
                            cx.notify();
                        });
                    }),
                    on_restore_source: Rc::new(move |cx: &mut App| {
                        restored.update(cx, |state, cx| {
                            if state.request_id != Some(id) {
                                return;
                            }
                            state.reset();
                            cx.notify();
                        });
                    }),
                    source_lift: None,
                    snapshot: None,
                    source_bubble_clip: None,
                    initial_lift_scale,
                    uses_flat_chrome: false,
                    content: Rc::new(|_: &mut Window, _: &mut App| div().into_any_element()),
                    is_centered: true,
                    live_actions: None,
                    on_dismiss: Rc::new(|_: &mut App| {}),
                    controller_preview: Some(Rc::new(RefCell::new(preview))),
                    prefers_dark_chrome: false,
                    keeps_source_editing: false,
                    max_visible_rows: None,
                },
                Some(&scroll_lock_from),
                Some(press_location),
                window,
                cx,
            );
            return;
        }

        let source_lift: Option<Rc<HoldContextMenuSourceLift>> = None;

        let captured_content = self.content.clone();
        let captured_actions = (self.actions_provider)(cx);
        let live_actions: Option<Rc<dyn Fn(&App) -> Vec<HoldContextMenuAction>>> = if self.live_action_titles {
            let actions_provider = self.actions_provider.clone();
            let captured = captured_actions.clone();
            Some(Rc::new(move |cx: &App| {
                let fresh = actions_provider(cx);
                captured
                    .iter()
                    .map(|captured| fresh.iter().find(|action| action.id == captured.id).cloned().unwrap_or_else(|| captured.clone()))
                    .collect()
            }))
        } else {
            None
        };
        let captured_emojis = Self::emojis_fitting_screen(&self.emojis, window);
        let captured_lifted_background = self.lifted_background;

        let id = HoldContextMenuRequestID::new();
        if let Some(cell) = &self.active_request_cell {
            cell.set(Some(id));
        }
        state.update(cx, |state, _| state.request_id = Some(id));

        let applied_lift_inset = if self.lift_inset > 0.0 {
            self.lift_inset.min(f32::from(source_frame.size.width) / 4.0)
        } else {
            0.0
        };
        let presented_source_frame = Bounds::new(
            point(source_frame.origin.x + px(applied_lift_inset), source_frame.origin.y),
            size(source_frame.size.width - px(applied_lift_inset * 2.0), source_frame.size.height),
        );
        let screen_width = f32::from(window.viewport_size().width);
        let presented_source_center = scaled_rect.center();

        let lifted = state.clone();
        let restored = state.clone();
        let restored_lift = source_lift.clone();
        HoldContextMenuPresenter::present(
            HoldContextMenuRequest {
                id,
                source_frame: presented_source_frame,
                initial_source_center: presented_source_center,
                alignment: self.alignment,
                show_emoji_list: self.show_emoji_list,
                emojis: captured_emojis,
                allows_custom_emoji: self.allows_custom_emoji,
                expanded_emojis: self.expanded_emojis.clone(),
                actions: captured_actions,
                accessory: self.accessory.clone(),
                on_emoji: self.on_emoji.clone(),
                on_action: self.on_action.clone(),
                on_lift_source: Rc::new(move |cx: &mut App| {
                    lifted.update(cx, |state, cx| {
                        if state.request_id != Some(id) {
                            return;
                        }
                        if state.is_lifted || *state.press_scale.target() != 1.0 {
                            state.press_scale.jump(1.0);
                            state.is_lifted = false;
                            cx.notify();
                        }
                    });
                }),
                on_restore_source: Rc::new(move |cx: &mut App| {
                    if let Some(lift) = restored_lift.as_ref() {
                        lift.restore(cx);
                    }
                    restored.update(cx, |state, cx| {
                        if state.request_id != Some(id) {
                            return;
                        }
                        let changed = state.is_lifted || *state.press_scale.target() != 1.0;
                        state.reset();
                        if changed {
                            cx.notify();
                        }
                    });
                }),
                source_lift,
                snapshot: None,
                source_bubble_clip: self.source_bubble_clip,
                initial_lift_scale,
                uses_flat_chrome: self.uses_flat_chrome,
                content: Rc::new(move |window: &mut Window, cx: &mut App| {
                    let content = div()
                        .mx(px(-applied_lift_inset))
                        .child(captured_content(window, cx))
                        .into_any_element();
                    HoldLiftedRowCard {
                        color: captured_lifted_background,
                        source_min_x: f32::from(presented_source_frame.origin.x),
                        source_max_x: f32::from(presented_source_frame.right()),
                        screen_width,
                    }
                    .body(content)
                }),
                is_centered: false,
                live_actions,
                on_dismiss: Rc::new(|_: &mut App| {}),
                controller_preview: None,
                prefers_dark_chrome: false,
                keeps_source_editing: false,
                max_visible_rows: None,
            },
            Some(&scroll_lock_from),
            Some(press_location),
            window,
            cx,
        );
    }

    fn emojis_fitting_screen(list: &[String], window: &Window) -> Vec<String> {
        let screen_width = f32::from(window.viewport_size().width);
        let slot = HoldContextMenuMetrics::EMOJI_SLOT_WIDTH;
        let available = screen_width
            - HoldContextMenuMetrics::EDGE_INSET * 2.0
            - HoldContextMenuMetrics::EMOJI_BAR_HORIZONTAL_PADDING * 2.0
            - slot;
        let max_count = usize::max(1, f32::max(0.0, available / slot) as usize);
        list.iter().take(max_count).cloned().collect()
    }

    fn cancel(state: &Entity<HoldContextMenuSourcePresentationState>, cx: &mut App) {
        let is_lifted = state.read(cx).is_lifted;
        if is_lifted || HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some() {
            return;
        }
        Self::release_press_scale(state, cx);
    }

    fn release_press_scale(state: &Entity<HoldContextMenuSourcePresentationState>, cx: &mut App) {
        let reduces_motion = reduce_motion(cx);
        state.update(cx, |state, cx| {
            if *state.press_scale.target() == 1.0 {
                return;
            }
            let spring = Spring::new(
                HoldContextMenuMetrics::PRESS_RELEASE_RESPONSE,
                HoldContextMenuMetrics::PRESS_RELEASE_DAMPING,
            );
            state.press_scale.set(1.0, MotionSpec::spring(spring), RiseClock::monotonic(), reduces_motion);
            cx.notify();
        });
    }
}

impl RenderOnce for HoldContextMenuContainer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let content = self.content.clone();
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            HoldContextMenuSourcePresentationState::new(content.clone(), cx)
        });
        let source_view = state.read(cx).source_view.clone();
        let latest_content = self.content.clone();
        source_view.update(cx, |view, _| view.set_content(latest_content));

        let reset = state.clone();
        on_change(
            (self.id.clone(), "source-identity"),
            self.source_identity.clone(),
            false,
            window,
            cx,
            move |_, _, _, cx| {
                let request_id = reset.update(cx, |state, cx| {
                    let request_id = state.request_id.take();
                    state.reset();
                    cx.notify();
                    request_id
                });
                if let Some(request_id) = request_id {
                    HoldContextMenuPresenter::dismiss_if_active(request_id, cx);
                }
            },
        );

        let now = RiseClock::monotonic();
        let (press_scale, is_lifted, probe) = {
            let state = state.read(cx);
            (state.press_scale.frame_value(now, window), state.is_lifted, state.probe.clone())
        };
        let disabled = self.disabled;
        let element = (self.content)(window, cx);
        let this = Rc::new(self);

        let source = div()
            .relative()
            .child(element)
            .view_anchor_probe(&probe)
            .when(!disabled, |source| source.child(Self::gesture_plate(&this, &state)))
            .opacity(if disabled || !is_lifted { 1.0 } else { 0.0 });
        source.scale_effect(if disabled { 1.0 } else { press_scale })
    }
}

impl HoldContextMenuContainer {
    fn gesture_plate(this: &Rc<Self>, state: &Entity<HoldContextMenuSourcePresentationState>) -> HoldGesturePlate {
        let (anticipating, anticipated) = (this.clone(), state.clone());
        let (confirming, confirmed) = (this.clone(), state.clone());
        let cancelled = state.clone();
        HoldGesturePlate::new((this.id.clone(), "plate"))
            .tap_movement_tolerance(this.tap_movement_tolerance)
            .context_click_across_row(this.context_click_across_row)
            .on_touch_down(Rc::new(|_: &mut Window, _: &mut App| {}))
            .on_anticipate(Rc::new(move |_plate: &ViewAnchorProbe, _: &mut Window, cx: &mut App| {
                anticipating.begin_progressive_press(&anticipated, cx);
            }))
            .on_confirm(Rc::new(
                move |_plate: &ViewAnchorProbe, location: Point<Pixels>, window: &mut Window, cx: &mut App| {
                    confirming.confirm(&confirmed, location, window, cx);
                },
            ))
            .on_update(Rc::new(|location: Point<Pixels>, _: &mut Window, cx: &mut App| {
                HoldContextMenuPresenter::update_drag(location, cx);
            }))
            .on_release(Rc::new(|location: Option<Point<Pixels>>, _: &mut Window, cx: &mut App| {
                HoldContextMenuPresenter::release_at(location, cx);
            }))
            .on_cancel(Rc::new(move |_: &mut Window, cx: &mut App| Self::cancel(&cancelled, cx)))
            .on_tap(this.on_tap.clone())
            .can_double_tap(this.can_double_tap.clone())
            .on_double_tap(this.on_double_tap.clone())
    }
}
