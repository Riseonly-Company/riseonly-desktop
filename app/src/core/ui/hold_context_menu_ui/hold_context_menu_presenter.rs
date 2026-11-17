use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, Bounds, Context, Entity, EntityId, IntoElement,
    ParentElement, Pixels, Point, Size, StyleRefinement, Styled, Window, div, point, px,
};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::hold_context_menu_ui::hold_context_menu_controller_preview::HoldContextMenuControllerPreview;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{
    HoldContextMenuAccessory, HoldContextMenuAction, HoldContextMenuAlignment, HoldSourceBubbleClip,
};
use rise_ui::swiftui::measure::measure;
use rise_ui::view_anchor_probe::{ViewAnchorProbe, ViewAnchorProbeReader};
use rise_ui::view_screen_snapshot::{ViewScreenImage, ViewScreenSource};

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter_hooks::hold_context_menu_presenter_hooks;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_scroll_lock::HoldContextMenuScrollLock;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_touch_tracker::HoldContextMenuTouchTracker;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;

const SOURCE_ATTACH_OPERATION: &str = "hold_menu.source_attach";
const SOURCE_RESTORE_OPERATION: &str = "hold_menu.source_restore";
const SOURCE_LIFT_THRESHOLD_MS: f64 = 2.0;
const PRESENT_EVENT: &str = "hold_menu.present";
const DISMISSED_EVENT: &str = "hold_menu.dismissed";
const PRESENT_RENDERER: &str = "core_animation_pages_v3";
const LOG_NAME: &str = "HoldMenu";
const AFTER_DISMISS_FRAMES: usize = 2;
const CLOSED_MOMENTS_AGO: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HoldContextMenuRequestID {
    pub value: Uuid,
}

impl HoldContextMenuRequestID {
    /// new value, with_value passes a given one
    pub fn new() -> Self {
        Self::with_value(Uuid::new_v4())
    }

    pub fn with_value(value: Uuid) -> Self {
        Self { value }
    }
}

impl Default for HoldContextMenuRequestID {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Source,
    Lifted,
    Restored,
}

/// a lifted source and the probe of its home slot, keyed by the source view
struct LiftedSource {
    home_size: Size<Pixels>,
    home_probe: ViewAnchorProbe,
}

thread_local! {
    static LIFTED_SOURCES: RefCell<HashMap<EntityId, LiftedSource>> = RefCell::new(HashMap::new());
}

/// the pressed row itself carried into the menu layer and handed back
/// the overlay draws the same view again
/// and the home slot draws a placeholder of the same size meanwhile
pub struct HoldContextMenuSourceLift {
    source_view: ViewScreenSource,
    source_bounds: Size<Pixels>,
    source_crop_rect: Option<Bounds<Pixels>>,
    home_probe: ViewAnchorProbe,
    host_probe: ViewAnchorProbe,
    phase: Cell<Phase>,
    content_growth: Rc<Cell<f32>>,
    on_content_growth: RefCell<Option<HoldContextMenuSourceGrowthHandler>>,
}

/// told the height the lifted source grew by over its home frame, after the frame that laid it out
pub type HoldContextMenuSourceGrowthHandler = Rc<dyn Fn(f32, &mut Window, &mut App)>;

impl HoldContextMenuSourceLift {
    /// none when the source isnt drawn in a window or the crop isnt inside it
    pub fn new(source_view: ViewScreenSource, source_crop_rect: Option<Bounds<Pixels>>) -> Option<Self> {
        let source_frame = source_view.window_frame()?;
        if let Some(crop) = source_crop_rect {
            let width = f32::from(crop.size.width);
            let height = f32::from(crop.size.height);
            let local = Bounds::new(point(px(0.0), px(0.0)), source_frame.size).dilate(px(0.5));
            if !(width > 1.0 && height > 1.0 && width.is_finite() && height.is_finite())
                || !crop.is_contained_within(&local)
            {
                return None;
            }
        }
        Some(Self {
            source_view,
            source_bounds: source_frame.size,
            source_crop_rect,
            home_probe: ViewAnchorProbe::new(),
            host_probe: ViewAnchorProbe::new(),
            phase: Cell::new(Phase::Source),
            content_growth: Rc::new(Cell::new(0.0)),
            on_content_growth: RefCell::new(None),
        })
    }

    pub fn clips_source_to_host_bounds(&self) -> bool {
        self.source_crop_rect.is_some()
    }

    /// visible bubble of a cropped source
    /// host masks to this fixed size and doesnt clip to its own bounds which may still grow on the
    /// first frames
    pub fn crop_size(&self) -> Option<Size<Pixels>> {
        self.source_crop_rect.map(|crop| crop.size)
    }

    /// how much taller the source laid out in the overlay than in its home slot
    /// the cropped bubble grows the same amount so its mask and the menu below follow
    pub fn content_growth(&self) -> f32 {
        self.content_growth.get()
    }

    pub fn set_on_content_growth(&self, handler: Option<HoldContextMenuSourceGrowthHandler>) {
        *self.on_content_growth.borrow_mut() = handler;
    }

    pub fn return_coordinate_view(&self) -> ViewAnchorProbe {
        if self.phase.get() == Phase::Lifted {
            self.home_probe.clone()
        } else {
            self.source_view.probe.clone()
        }
    }

    /// where the lifted source is on screen now, overlay scroll included
    pub fn lifted_center_in_screen(&self) -> Option<Point<Pixels>> {
        if self.phase.get() != Phase::Lifted {
            return None;
        }
        self.host_probe.window_frame().map(|frame| frame.center())
    }

    pub fn attach(&self, cx: &mut App) {
        if self.phase.get() == Phase::Restored {
            return;
        }
        let trace = ChatPerformanceTrace::shared();
        let token = trace.begin_operation(SOURCE_ATTACH_OPERATION, HashMap::new());
        let entity_id = self.source_view.view.entity_id();
        LIFTED_SOURCES.with(|lifted| {
            lifted.borrow_mut().insert(
                entity_id,
                LiftedSource { home_size: self.source_bounds, home_probe: self.home_probe.clone() },
            );
        });
        self.phase.set(Phase::Lifted);
        cx.notify(entity_id);
        trace.end_operation(token, SOURCE_LIFT_THRESHOLD_MS, HashMap::new());
    }

    /// places the source in a host of bounds size
    /// with a crop the cropped part sits at the host origin at crop size and the source keeps its own size
    /// without one the source fills the host
    pub fn layout(&self, bounds: Size<Pixels>) -> Option<AnyElement> {
        self.layout_replaying(bounds, false)
    }

    /// layout whose source fills a sized host with no crop
    /// and replays its last frame while it hasnt notified
    /// only when replays_source says the host stands still
    pub fn layout_replaying(&self, bounds: Size<Pixels>, replays_source: bool) -> Option<AnyElement> {
        if self.phase.get() != Phase::Lifted {
            return None;
        }
        let source: AnyView = self.source_view.view.clone();
        let placed = match self.source_crop_rect {
            Some(crop) => {
                let crop_center = crop.center();
                let home_height = f32::from(self.source_bounds.height);
                let growth = self.content_growth.clone();
                let on_growth = self.on_content_growth.borrow().clone();
                div()
                    .absolute()
                    .left(crop.size.width / 2.0 - crop_center.x)
                    .top(crop.size.height / 2.0 - crop_center.y)
                    .w(self.source_bounds.width)
                    .min_h(self.source_bounds.height)
                    .child(source)
                    .child(measure("hold-menu-lifted-source-natural", move |bounds, window, cx| {
                        let natural = f32::from(bounds.size.height);
                        if !natural.is_finite() {
                            return;
                        }
                        let grown = (natural - home_height).max(0.0);
                        if (grown - growth.get()).abs() <= 0.5 {
                            return;
                        }
                        growth.set(grown);
                        if let Some(on_growth) = on_growth {
                            on_growth(grown, window, cx);
                        }
                    }))
            }
            None if f32::from(bounds.width) < 1.0 || f32::from(bounds.height) < 1.0 => div()
                .absolute()
                .left_0()
                .top_0()
                .w(self.source_bounds.width)
                .h(self.source_bounds.height)
                .child(source),
            None if replays_source => div()
                .absolute()
                .left_0()
                .top_0()
                .w(bounds.width)
                .h(bounds.height)
                .child(source.cached(StyleRefinement::default().size_full()).replay_when_moved()),
            None => div().absolute().left_0().top_0().w(bounds.width).h(bounds.height).child(source),
        };
        let placed = match self.crop_size() {
            Some(crop) => div()
                .absolute()
                .left_0()
                .top_0()
                .w(crop.width)
                .h(crop.height + px(self.content_growth.get()))
                .overflow_hidden()
                .child(placed),
            None => placed,
        };
        let host = div().relative().flex_none().w(bounds.width).h(bounds.height);
        Some(
            host.child(placed)
                .child(ViewAnchorProbeReader::new(self.host_probe.clone()))
                .into_any_element(),
        )
    }

    pub fn restore(&self, cx: &mut App) {
        if self.phase.get() == Phase::Restored {
            return;
        }
        self.on_content_growth.borrow_mut().take();
        let trace = ChatPerformanceTrace::shared();
        let token = trace.begin_operation(SOURCE_RESTORE_OPERATION, HashMap::new());
        self.phase.set(Phase::Restored);
        let entity_id = self.source_view.view.entity_id();
        LIFTED_SOURCES.with(|lifted| {
            lifted.borrow_mut().remove(&entity_id);
        });
        cx.notify(entity_id);
        trace.end_operation(token, SOURCE_LIFT_THRESHOLD_MS, HashMap::new());
    }

    /// for the slot that normally draws view
    /// gives the element to draw while the view is lifted into the menu
    /// keeps the row size so the view is drawn once
    pub fn lifted_placeholder(view: &AnyView) -> Option<AnyElement> {
        LIFTED_SOURCES.with(|lifted| {
            let lifted = lifted.borrow();
            let source = lifted.get(&view.entity_id())?;
            Some(
                div()
                    .relative()
                    .flex_none()
                    .w(source.home_size.width)
                    .h(source.home_size.height)
                    .child(ViewAnchorProbeReader::new(source.home_probe.clone()))
                    .into_any_element(),
            )
        })
    }
}

pub type HoldContextMenuEmojiHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub type HoldContextMenuActionHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub type HoldContextMenuSourceHandler = Rc<dyn Fn(&mut App)>;
pub type HoldContextMenuContentBuilder = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
pub type HoldContextMenuLiveActions = Rc<dyn Fn(&App) -> Vec<HoldContextMenuAction>>;

/// defaults of the constructor, allows_custom_emoji false
/// expanded_emojis live_actions and controller_preview none
/// is_centered false, on_dismiss a no op and prefers_dark_chrome off
#[derive(Clone)]
pub struct HoldContextMenuRequest {
    pub id: HoldContextMenuRequestID,
    pub source_frame: Bounds<Pixels>,
    pub initial_source_center: Point<Pixels>,
    pub alignment: HoldContextMenuAlignment,
    pub show_emoji_list: bool,
    pub emojis: Vec<String>,
    pub allows_custom_emoji: bool,
    pub expanded_emojis: Option<Vec<String>>,
    pub actions: Vec<HoldContextMenuAction>,
    pub accessory: Option<HoldContextMenuAccessory>,
    pub on_emoji: HoldContextMenuEmojiHandler,
    pub on_action: HoldContextMenuActionHandler,
    pub on_lift_source: HoldContextMenuSourceHandler,
    pub on_restore_source: HoldContextMenuSourceHandler,
    pub source_lift: Option<Rc<HoldContextMenuSourceLift>>,
    pub snapshot: Option<ViewScreenImage>,
    pub source_bubble_clip: Option<HoldSourceBubbleClip>,
    pub initial_lift_scale: f32,
    pub uses_flat_chrome: bool,
    pub content: HoldContextMenuContentBuilder,
    pub is_centered: bool,
    pub live_actions: Option<HoldContextMenuLiveActions>,
    pub on_dismiss: HoldContextMenuSourceHandler,
    pub controller_preview: Option<Rc<RefCell<HoldContextMenuControllerPreview>>>,
    pub prefers_dark_chrome: bool,
    pub keeps_source_editing: bool,
    pub max_visible_rows: Option<i64>,
}

impl HoldContextMenuRequest {
    pub fn resolved_actions(&self, cx: &App) -> Vec<HoldContextMenuAction> {
        match &self.live_actions {
            Some(live_actions) => live_actions(cx),
            None => self.actions.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoldContextMenuReleaseEvent {
    pub id: Uuid,
    pub location: Option<Point<Pixels>>,
}

type ReturnCenterResolver = Rc<dyn Fn() -> Option<Point<Pixels>>>;

/// observable presenter, readers hold the entity and re render on notify
/// commands are associated functions on app so the request callbacks run outside the entity lease
/// and may read it
pub struct HoldContextMenuPresenter {
    active_request: Option<HoldContextMenuRequest>,
    close_began_at: Option<Duration>,
    touch_block_deadline: Duration,
    pub drag_location: Option<Point<Pixels>>,
    pub pending_release: Option<HoldContextMenuReleaseEvent>,
    dismissal_requested_id: Option<HoldContextMenuRequestID>,
    did_report_release: bool,
    did_perform_aimed_action: bool,
    touch_belongs_to_preview: bool,
    press_location: Option<Point<Pixels>>,
    menu_anchor: Option<Point<Pixels>>,
    resolve_return_center: Option<ReturnCenterResolver>,
    has_aimed: bool,
    after_dismiss_work: Vec<Box<dyn FnOnce(&mut App)>>,
    closes_without_motion: bool,
    source_restore_deferred: bool,
    source_restored_at: Duration,
}

impl HoldContextMenuPresenter {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            active_request: None,
            close_began_at: None,
            touch_block_deadline: Duration::ZERO,
            drag_location: None,
            pending_release: None,
            dismissal_requested_id: None,
            did_report_release: false,
            did_perform_aimed_action: false,
            touch_belongs_to_preview: false,
            press_location: None,
            menu_anchor: None,
            resolve_return_center: None,
            has_aimed: false,
            after_dismiss_work: Vec::new(),
            closes_without_motion: false,
            source_restore_deferred: false,
            source_restored_at: Duration::ZERO,
        }
    }

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    /// presentation is globally unique so this doubles as the canon fact hold_menu_opened
    /// reported by level from the one place that knows, never by whoever pressed
    /// a guide step waiting on it is satisfied by an already open menu
    pub fn active_request(&self) -> Option<&HoldContextMenuRequest> {
        self.active_request.as_ref()
    }

    /// pointer the open menu was confirmed at, in window coordinates
    pub fn menu_anchor(&self) -> Option<Point<Pixels>> {
        self.menu_anchor
    }

    pub fn has_active_request(cx: &mut App) -> bool {
        Self::shared(cx).read(cx).active_request.is_some()
    }

    fn set_active_request(&mut self, request: Option<HoldContextMenuRequest>, cx: &mut Context<Self>) {
        self.active_request = request;
        GuideFacts::set(RiseGuideSignal::HoldMenuOpened, self.active_request.is_some(), cx);
        cx.notify();
    }

    pub fn source_restored_at(&self) -> Duration {
        self.source_restored_at
    }

    pub fn closed_moments_ago(&self) -> bool {
        !self.source_restored_at.is_zero()
            && RiseClock::monotonic().saturating_sub(self.source_restored_at) < CLOSED_MOMENTS_AGO
    }

    /// runs work once no menu is open, right away if none is
    /// else two drawn frames after the lifted source is back in its row
    /// a row removed while its source is lifted leaves an empty slot
    pub fn after_dismiss(work: impl FnOnce(&mut App) + 'static, cx: &mut App) {
        let presenter = Self::shared(cx);
        if presenter.read(cx).active_request.is_none() {
            work(cx);
            return;
        }
        presenter.update(cx, |this, _| this.after_dismiss_work.push(Box::new(work)));
    }

    pub fn dismissal_requested_id(&self) -> Option<HoldContextMenuRequestID> {
        self.dismissal_requested_id
    }

    pub fn has_aimed(&self) -> bool {
        self.has_aimed
    }

    pub fn touch_block_deadline(&self) -> Duration {
        self.touch_block_deadline
    }

    pub fn note_dismiss_began(cx: &mut App) {
        GuideFacts::set(RiseGuideSignal::HoldMenuOpened, false, cx);
        Self::shared(cx).update(cx, |this, _| this.close_began_at = Some(RiseClock::monotonic()));
    }

    /// presses during the whole close are swallowed and not passed through to the source layer
    /// the source return outlasts the backdrop fade
    /// and a click passed through then would end after the request is gone
    pub fn swallows_touches_while_closing(&self) -> bool {
        let Some(close_began_at) = self.close_began_at else { return false };
        if self.active_request.is_none() {
            return false;
        }
        RiseClock::monotonic() < close_began_at + seconds(HoldContextMenuMetrics::CLOSE_TOUCH_BLOCK_LIMIT)
    }

    /// called before a pick runs, the pick may rebuild the source row and tear the menu down synchronously
    /// before the overlay starts its close, and the press block needs a start
    pub fn note_close_imminent(cx: &mut App) {
        Self::shared(cx).update(cx, |this, _| this.close_began_at = Some(RiseClock::monotonic()));
    }

    pub fn close_without_motion(cx: &mut App) {
        Self::shared(cx).update(cx, |this, _| {
            if this.active_request.is_some() {
                this.closes_without_motion = true;
            }
        });
    }

    pub fn closes_without_motion(&self) -> bool {
        self.closes_without_motion
    }

    pub fn defer_source_restore(cx: &mut App) {
        Self::shared(cx).update(cx, |this, _| {
            if this.active_request.is_some() {
                this.source_restore_deferred = true;
            }
        });
    }

    pub fn blocks_source_window_touches(&self) -> bool {
        if self.active_request.is_some() {
            return self.swallows_touches_while_closing();
        }
        RiseClock::monotonic() < self.touch_block_deadline
    }

    /// press_location is the pointer in window coordinates when the hold confirmed
    /// none for presentations with no press
    /// window is the window the source is drawn in
    pub fn present(
        request: HoldContextMenuRequest,
        scroll_lock_from: Option<&ViewScreenSource>,
        press_location: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let source_window: Option<AnyWindowHandle> = scroll_lock_from.map(|_| window.window_handle());
        let fields = HashMap::from([
            ("actions".to_owned(), request.actions.len().to_string()),
            ("renderer".to_owned(), PRESENT_RENDERER.to_owned()),
            ("emoji".to_owned(), flag(request.show_emoji_list).to_owned()),
            ("snapshot".to_owned(), flag(request.snapshot.is_some()).to_owned()),
            ("source_height".to_owned(), format!("{:.2}", f32::from(request.source_frame.size.height))),
            ("source_width".to_owned(), format!("{:.2}", f32::from(request.source_frame.size.width))),
        ]);
        ChatPerformanceTrace::shared().record(PRESENT_EVENT, fields);
        let presenter = Self::shared(cx);
        let keeps_source_editing = request.keeps_source_editing;
        let previous = presenter.read(cx).active_request.clone();
        let replacing = previous.is_some();
        let request_id = request.id.value;
        Sai::log(LOG_NAME, || format!("present id={request_id} replacing={replacing}"), SaiLogKind::Ui, cx);
        let _ = impact(ImpactStyle::Medium);
        if let Some(source_view) = scroll_lock_from {
            if let Some(source_frame) = source_view.window_frame() {
                HoldContextMenuScrollLock::lock(source_frame);
            }
            HoldContextMenuTouchTracker::install(window);
        }
        if let Some(previous) = previous {
            (previous.on_restore_source)(cx);
            (previous.on_dismiss)(cx);
        }
        let return_view = request
            .source_lift
            .as_ref()
            .map(|lift| lift.return_coordinate_view())
            .or_else(|| scroll_lock_from.map(|source| source.probe.clone()));
        let resolve_return_center: Option<ReturnCenterResolver> = match (return_view, source_window) {
            (Some(return_view), Some(_)) => return_view.window_frame().map(|frame| {
                let center = if request.controller_preview.is_some() {
                    request.initial_source_center
                } else {
                    request.source_frame.center()
                };
                let local_center = center - frame.origin;
                let resolver: ReturnCenterResolver =
                    Rc::new(move || return_view.window_frame().map(|frame| frame.origin + local_center));
                resolver
            }),
            _ => None,
        };
        presenter.update(cx, |this, cx| {
            this.resolve_return_center = resolve_return_center;
            this.close_began_at = None;
            this.closes_without_motion = false;
            this.source_restore_deferred = false;
            this.dismissal_requested_id = None;
            this.drag_location = None;
            this.pending_release = None;
            this.did_report_release = false;
            this.did_perform_aimed_action = false;
            this.touch_belongs_to_preview = false;
            this.press_location = press_location;
            this.menu_anchor = press_location;
            this.has_aimed = press_location.is_none();
            this.set_active_request(Some(request), cx);
        });
        if source_window.is_some() && !keeps_source_editing {
            ScreenFocus::release_text_field(window, cx);
        }
        if let Some(hooks) = hold_context_menu_presenter_hooks() {
            (hooks.show)(source_window, cx);
        }
    }

    pub fn perform_action(
        action_id: &str,
        if_active: HoldContextMenuRequestID,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let presenter = Self::shared(cx);
        let request = presenter.update(cx, |this, _| {
            let request = this.active_request.clone().filter(|request| request.id == if_active)?;
            if this.press_location.is_some() && this.has_aimed {
                if this.did_perform_aimed_action {
                    return None;
                }
                this.did_perform_aimed_action = true;
            }
            Some(request)
        });
        let Some(request) = request else { return false };
        if let Some(hooks) = hold_context_menu_presenter_hooks() {
            (hooks.restore_source_key_window)(window, cx);
        }
        (request.on_action)(action_id.to_owned(), window, cx);
        true
    }

    pub fn return_center(&self, if_active: HoldContextMenuRequestID) -> Option<Point<Pixels>> {
        if self.active_request.as_ref().map(|request| request.id) != Some(if_active) {
            return None;
        }
        self.resolve_return_center.as_ref().and_then(|resolve| resolve())
    }

    pub fn request_dismiss(if_active: HoldContextMenuRequestID, cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| {
            if this.active_request.as_ref().map(|request| request.id) != Some(if_active) {
                return;
            }
            this.dismissal_requested_id = Some(if_active);
            cx.notify();
        });
    }

    pub fn dismiss(cx: &mut App) {
        let presenter = Self::shared(cx);
        let request = presenter.read(cx).active_request.clone();
        if let Some(request) = &request {
            let fields = HashMap::from([("snapshot".to_owned(), flag(request.snapshot.is_some()).to_owned())]);
            ChatPerformanceTrace::shared().record(DISMISSED_EVENT, fields);
        }
        let request_id = request.as_ref().map(|request| request.id.value.to_string());
        Sai::log(
            LOG_NAME,
            || format!("dismiss id={}", request_id.unwrap_or_else(|| "none".to_owned())),
            SaiLogKind::Ui,
            cx,
        );
        HoldContextMenuScrollLock::unlock();
        let restored = request.is_some();
        let restore_deferred = presenter.read(cx).source_restore_deferred;
        if !restore_deferred {
            if let Some(request) = &request {
                (request.on_restore_source)(cx);
            }
        }
        let source_window = hold_context_menu_presenter_hooks().and_then(|hooks| (hooks.source_window)(cx));
        let on_dismiss = request.map(|request| request.on_dismiss);
        let (touch_block_deadline, after_dismiss_work) = presenter.update(cx, |this, cx| {
            if restored {
                this.source_restored_at = RiseClock::monotonic();
            }
            this.set_active_request(None, cx);
            this.dismissal_requested_id = None;
            this.closes_without_motion = false;
            this.source_restore_deferred = false;
            this.drag_location = None;
            this.pending_release = None;
            this.did_report_release = false;
            this.did_perform_aimed_action = false;
            this.touch_belongs_to_preview = false;
            this.press_location = None;
            this.menu_anchor = None;
            this.resolve_return_center = None;
            this.has_aimed = false;
            this.touch_block_deadline = this.close_began_at.map_or(Duration::ZERO, |close_began_at| {
                (close_began_at + seconds(HoldContextMenuMetrics::BACKDROP_FADE_OUT_DURATION))
                    .max(RiseClock::monotonic() + seconds(HoldContextMenuMetrics::CLOSED_TOUCH_BLOCK_TAIL))
                    .min(close_began_at + seconds(HoldContextMenuMetrics::CLOSE_TOUCH_BLOCK_LIMIT))
            });
            this.close_began_at = None;
            (this.touch_block_deadline, std::mem::take(&mut this.after_dismiss_work))
        });
        if !after_dismiss_work.is_empty() {
            HoldContextMenuRenderedFrameWait::run(AFTER_DISMISS_FRAMES, source_window, cx, move |cx| {
                for item in after_dismiss_work {
                    item(cx);
                }
            });
        }
        if let Some(hooks) = hold_context_menu_presenter_hooks() {
            (hooks.hide)(touch_block_deadline, cx);
        }
        if let Some(on_dismiss) = on_dismiss {
            on_dismiss(cx);
        }
    }

    pub fn reset_account_state(cx: &mut App) {
        Self::dismiss(cx);
    }

    /// dismisses the menu if the request is still the active one
    pub fn dismiss_if_active(if_active: HoldContextMenuRequestID, cx: &mut App) {
        let active = Self::shared(cx).read(cx).active_request.as_ref().map(|request| request.id);
        if active != Some(if_active) {
            let stale = if_active.value;
            Sai::log(LOG_NAME, || format!("stale dismiss ignored id={stale}"), SaiLogKind::Ui, cx);
            return;
        }
        Self::dismiss(cx);
    }

    pub fn restore_source(if_active: HoldContextMenuRequestID, cx: &mut App) {
        let request = Self::shared(cx)
            .read(cx)
            .active_request
            .clone()
            .filter(|request| request.id == if_active);
        if let Some(request) = request {
            (request.on_restore_source)(cx);
        }
    }

    pub fn note_touch_began(location: Point<Pixels>, allows_drag_selection: bool, cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| {
            let Some(request) = &this.active_request else { return };
            let on_preview = request
                .controller_preview
                .as_ref()
                .is_some_and(|preview| preview.borrow().contains_touch(location));
            this.press_location = Some(location);
            this.touch_belongs_to_preview = !allows_drag_selection || on_preview;
            this.has_aimed = false;
            this.drag_location = None;
            this.did_report_release = false;
            this.did_perform_aimed_action = false;
            cx.notify();
        });
    }

    pub fn update_drag(location: Point<Pixels>, cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| {
            if this.active_request.is_none() || this.touch_belongs_to_preview {
                return;
            }
            let was_aimed = this.has_aimed;
            if !this.note_aim(location) {
                return;
            }
            if this.drag_location == Some(location) {
                if !was_aimed {
                    cx.notify();
                }
                return;
            }
            this.drag_location = Some(location);
            cx.notify();
        });
    }

    /// latches once the pointer is SELECTION_AIM_TRAVEL away from where it went down
    /// both the moves and the release ask so a slide whose only report is the release still counts
    fn note_aim(&mut self, location: Point<Pixels>) -> bool {
        if self.has_aimed {
            return true;
        }
        let Some(press_location) = self.press_location else {
            self.has_aimed = true;
            return true;
        };
        let dx = f32::from(location.x - press_location.x);
        let dy = f32::from(location.y - press_location.y);
        let travel = HoldContextMenuMetrics::SELECTION_AIM_TRAVEL;
        if dx * dx + dy * dy <= travel * travel {
            return false;
        }
        self.has_aimed = true;
        true
    }

    /// two observers watch the same press, the plate and the window level tracker
    /// so the first release report wins, a second one would commit the action twice
    /// cancelled or never moved presses are dropped and the menu stays open
    pub fn release_at(location: Option<Point<Pixels>>, cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| {
            let Some(location) = location else { return };
            if this.active_request.is_none() || this.touch_belongs_to_preview || this.did_report_release {
                return;
            }
            let was_aimed = this.has_aimed;
            if !this.note_aim(location) {
                return;
            }
            if !was_aimed {
                cx.notify();
            }
            this.did_report_release = true;
            this.pending_release = Some(HoldContextMenuReleaseEvent { id: Uuid::new_v4(), location: Some(location) });
            cx.notify();
        });
    }

    pub fn clear_pending_release(cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| {
            if this.pending_release.take().is_some() {
                cx.notify();
            }
        });
    }
}

struct HoldContextMenuRenderedFrameWait;

impl HoldContextMenuRenderedFrameWait {
    fn run(
        frames: usize,
        window: Option<AnyWindowHandle>,
        cx: &mut App,
        work: impl FnOnce(&mut App) + 'static,
    ) {
        let work: Box<dyn FnOnce(&mut App)> = Box::new(work);
        let pending = Rc::new(RefCell::new(Some(work)));
        let scheduled = window.is_some_and(|window| {
            let pending = pending.clone();
            window
                .update(cx, |_, window, _| Self::tick(frames.max(1), pending, window))
                .is_ok()
        });
        if !scheduled {
            cx.defer(move |cx| {
                if let Some(work) = pending.borrow_mut().take() {
                    work(cx);
                }
            });
        }
    }

    fn tick(remaining: usize, pending: Rc<RefCell<Option<Box<dyn FnOnce(&mut App)>>>>, window: &mut Window) {
        window.on_next_frame(move |window, cx| {
            if remaining <= 1 {
                if let Some(work) = pending.borrow_mut().take() {
                    work(cx);
                }
            } else {
                Self::tick(remaining - 1, pending, window);
            }
        });
        window.refresh();
    }
}

fn flag(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value.max(0.0))
}

