use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{
    App, Bounds, DispatchPhase, Entity, EntityId, Global, IntoElement, Pixels, Point, RenderOnce, ScrollDelta,
    ScrollHandle, ScrollWheelEvent, Styled, TouchPhase, WeakEntity, Window, canvas, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::language_store::LanguageStore;
use rise_ui::hold_context_menu_ui::bubble_scroll_passthrough::BubbleScrollPassthrough;

use crate::app::router::navigation_store::NavigationStore;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::shared_element_transition::kernel::shared_element_types::SharedElementPopPanMode;
use crate::core::shared_element_transition::navigation::shared_element_navigation_driver::SharedElementNavigationDriver;
use crate::core::shared_element_transition::navigation::shared_element_navigation_transitioning::{
    SharedElementNavigationController, SharedElementPanGesture, SharedElementPanState, SharedElementStockPopTarget,
};
use crate::core::shared_element_transition::navigation::shared_element_pop_pan_router::SharedElementPopPanRouter;
use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::{
    PresentationChainTrace, SheetLayerStack,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet_store::StickerPackSheetStore;

const LOG_NAME: &str = "Navigation";
const NIL_DESCRIPTION: &str = "nil";
const STOCK_POP_COMPLETION_FRACTION: f32 = 0.5;
const STOCK_POP_COMPLETION_VELOCITY: f32 = 500.0;
const SIDEWAYS_RANGE_MINIMUM: f32 = 8.0;
const SIDEWAYS_OFFSET_TOLERANCE: f32 = 0.5;
const ROUTE_STACK_SWIPE_POPS: bool = false;

pub type NavigationInteractivePopGuardUnsavedChanges = Rc<dyn Fn(&mut Window, &mut App) -> bool>;
pub type NavigationInteractivePopGuardPagingRegion = Rc<dyn Fn(Point<Pixels>) -> bool>;

#[derive(Default)]
struct NavigationInteractivePopGuardState {
    is_media_editor_editing: bool,
    unsaved_changes_guard: Option<NavigationInteractivePopGuardUnsavedChanges>,
    profile_tab_paging_back_owners: HashMap<Uuid, Option<NavigationInteractivePopGuardPagingRegion>>,
    touched_sideways_scrolls: Option<(Point<Pixels>, Vec<ScrollHandle>)>,
}

impl Global for NavigationInteractivePopGuardState {}

pub enum NavigationInteractivePopGuard {}

impl NavigationInteractivePopGuard {
    pub fn is_media_editor_editing(cx: &App) -> bool {
        cx.try_global::<NavigationInteractivePopGuardState>()
            .is_some_and(|state| state.is_media_editor_editing)
    }

    /// screen with unsaved edits, back asks it first
    /// true means back doesnt run and the screen showed its own prompt, false means normal pop
    pub fn unsaved_changes_guard(cx: &App) -> Option<NavigationInteractivePopGuardUnsavedChanges> {
        cx.try_global::<NavigationInteractivePopGuardState>()
            .and_then(|state| state.unsaved_changes_guard.clone())
    }

    pub fn set_unsaved_changes_guard(guard: Option<NavigationInteractivePopGuardUnsavedChanges>, cx: &mut App) {
        cx.default_global::<NavigationInteractivePopGuardState>().unsaved_changes_guard = guard;
    }

    /// language decides and not the view
    /// a view built before a language switch keeps the old direction until next launch
    pub fn is_right_to_left(cx: &App) -> bool {
        LanguageStore::active(cx).is_some_and(LanguageStore::is_rtl)
    }

    pub fn is_profile_tab_paging_back(cx: &App) -> bool {
        cx.try_global::<NavigationInteractivePopGuardState>()
            .is_some_and(|state| !state.profile_tab_paging_back_owners.is_empty())
    }

    pub fn set_media_editor_editing(editing: bool, cx: &mut App) {
        cx.default_global::<NavigationInteractivePopGuardState>().is_media_editor_editing = editing;
    }

    /// an owner either takes the whole back swipe (none) or gives the region where it pages tabs
    /// outside it the screen still pops, the region check takes window coordinates
    pub fn set_profile_tab_paging_back(
        paging: bool,
        owner: Uuid,
        paging_region_contains: Option<NavigationInteractivePopGuardPagingRegion>,
        cx: &mut App,
    ) {
        let state = cx.default_global::<NavigationInteractivePopGuardState>();
        if paging {
            state.profile_tab_paging_back_owners.insert(owner, paging_region_contains);
        } else {
            state.profile_tab_paging_back_owners.remove(&owner);
        }
    }

    pub fn touch_down_point(pan: &dyn SharedElementPanGesture) -> Point<Pixels> {
        let current = pan.location();
        let travelled = pan.translation();
        Point::new(current.x - travelled.x, current.y - travelled.y)
    }

    pub fn blocks_interactive_pop(at_window_point: Point<Pixels>, cx: &App) -> bool {
        cx.try_global::<NavigationInteractivePopGuardState>().is_some_and(|state| {
            state.profile_tab_paging_back_owners.values().any(|region| match region {
                None => true,
                Some(region) => region(at_window_point),
            })
        })
    }

    fn note_touched_sideways_scroll(point: Point<Pixels>, scroll: ScrollHandle, cx: &mut App) {
        let state = cx.default_global::<NavigationInteractivePopGuardState>();
        let same_swipe = matches!(&state.touched_sideways_scrolls, Some((started_at, _)) if *started_at == point);
        if same_swipe {
            if let Some((_, scrolls)) = state.touched_sideways_scrolls.as_mut() {
                scrolls.push(scroll);
            }
        } else {
            state.touched_sideways_scrolls = Some((point, vec![scroll]));
        }
    }

    /// sideways scrolls under the swipe that started at point, innermost first
    fn touched_sideways_scrolls(point: Point<Pixels>, cx: &App) -> Vec<ScrollHandle> {
        cx.try_global::<NavigationInteractivePopGuardState>()
            .and_then(|state| state.touched_sideways_scrolls.as_ref())
            .filter(|(started_at, _)| *started_at == point)
            .map(|(_, scrolls)| scrolls.iter().rev().cloned().collect())
            .unwrap_or_default()
    }
}

/// sits behind every screen of the route stack, installs back handling once
/// also the view the back pan listens on
#[derive(IntoElement)]
pub struct UINavigationFullscreenPopInstaller {
    navigation_controller: Entity<NavigationStore>,
}

impl UINavigationFullscreenPopInstaller {
    pub fn new(navigation_controller: Entity<NavigationStore>) -> Self {
        Self { navigation_controller }
    }

    pub fn schedule_install(&self, cx: &mut App) {
        if NavigationFullscreenPop::is_installed(self.navigation_controller.entity_id(), cx) {
            return;
        }
        let navigation_controller = self.navigation_controller.clone();
        cx.defer(move |cx| {
            NavigationFullscreenPop::install_if_needed(&navigation_controller, cx);
            if NavigationFullscreenPop::is_installed(navigation_controller.entity_id(), cx) {
                cx.refresh_windows();
            }
        });
    }
}

impl RenderOnce for UINavigationFullscreenPopInstaller {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.schedule_install(cx);
        let delegate = NavigationFullscreenPop::delegate(self.navigation_controller.entity_id(), cx);
        canvas(
            |_, _, _| {},
            move |bounds, (), window, _cx| {
                let Some(delegate) = delegate else {
                    return;
                };
                if !ROUTE_STACK_SWIPE_POPS {
                    return;
                }
                let listener = delegate.claim_listener();
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || !delegate.is_current_listener(listener) {
                        return;
                    }
                    if event.touch_phase == TouchPhase::Started && !bounds.contains(&event.position) {
                        return;
                    }
                    delegate.drive_pop(event, window, cx);
                });
            },
        )
        .absolute()
        .inset_0()
    }
}

#[derive(IntoElement)]
pub struct NavigationSidewaysScrollWatch {
    scroll: ScrollHandle,
    is_scroll_enabled: bool,
}

impl NavigationSidewaysScrollWatch {
    pub fn new(scroll: ScrollHandle, is_scroll_enabled: bool) -> Self {
        Self { scroll, is_scroll_enabled }
    }
}

impl RenderOnce for NavigationSidewaysScrollWatch {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let scroll = self.scroll;
        let is_scroll_enabled = self.is_scroll_enabled;
        canvas(
            |_, _, _| {},
            move |_, (), window, _cx| {
                if !is_scroll_enabled {
                    return;
                }
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, _window, cx| {
                    if phase != DispatchPhase::Capture || event.touch_phase != TouchPhase::Started {
                        return;
                    }
                    if !scroll.bounds().contains(&event.position) {
                        return;
                    }
                    NavigationInteractivePopGuard::note_touched_sideways_scroll(event.position, scroll.clone(), cx);
                });
            },
        )
        .absolute()
        .inset_0()
    }
}

/// one trackpad swipe in window coordinates, from first move to end
pub struct NavigationFullscreenPopPan {
    state: Cell<SharedElementPanState>,
    translation: Cell<Point<Pixels>>,
    velocity: Cell<Point<Pixels>>,
    location: Cell<Point<Pixels>>,
    last_sample: Cell<Duration>,
    view_bounds: Option<Bounds<Pixels>>,
}

impl NavigationFullscreenPopPan {
    pub fn new(location: Point<Pixels>, view_bounds: Option<Bounds<Pixels>>) -> Self {
        Self {
            state: Cell::new(SharedElementPanState::Possible),
            translation: Cell::new(Point::default()),
            velocity: Cell::new(Point::default()),
            location: Cell::new(location),
            last_sample: Cell::new(RiseClock::monotonic()),
            view_bounds,
        }
    }

    pub fn set_state(&self, state: SharedElementPanState) {
        self.state.set(state);
    }

    /// one move of the swipe, delta is how far the fingers went since the last move
    pub fn add_translation(&self, delta: Point<Pixels>, location: Point<Pixels>) {
        let now = RiseClock::monotonic();
        let elapsed = now.saturating_sub(self.last_sample.get()).as_secs_f32();
        if elapsed > 0.0 {
            self.velocity.set(Point::new(px(delta.x.as_f32() / elapsed), px(delta.y.as_f32() / elapsed)));
        }
        self.last_sample.set(now);
        let translation = self.translation.get();
        self.translation.set(Point::new(translation.x + delta.x, translation.y + delta.y));
        self.location.set(location);
    }
}

impl SharedElementPanGesture for NavigationFullscreenPopPan {
    fn state(&self) -> SharedElementPanState {
        self.state.get()
    }

    fn translation(&self) -> Point<Pixels> {
        self.translation.get()
    }

    fn velocity(&self) -> Point<Pixels> {
        self.velocity.get()
    }

    fn location(&self) -> Point<Pixels> {
        self.location.get()
    }

    fn view_bounds(&self) -> Option<Bounds<Pixels>> {
        self.view_bounds
    }
}

/// one trackpad swipe as the back gesture sees it, first move to end
struct NavigationFullscreenPopPanSession {
    pan: Rc<NavigationFullscreenPopPan>,
    touched_sideways_scrolls: Vec<ScrollHandle>,
    decided: bool,
    began: bool,
}

pub struct NavigationFullscreenPopDelegate {
    navigation_controller: WeakEntity<NavigationStore>,
    router: RefCell<Weak<SharedElementPopPanRouter>>,
    session: RefCell<Option<NavigationFullscreenPopPanSession>>,
    swipe_started: Cell<bool>,
    listener: Cell<u64>,
}

impl NavigationFullscreenPopDelegate {
    pub fn new(navigation_controller: WeakEntity<NavigationStore>) -> Self {
        Self {
            navigation_controller,
            router: RefCell::new(Weak::new()),
            session: RefCell::new(None),
            swipe_started: Cell::new(false),
            listener: Cell::new(0),
        }
    }

    fn claim_listener(&self) -> u64 {
        let listener = self.listener.get().wrapping_add(1);
        self.listener.set(listener);
        listener
    }

    fn is_current_listener(&self, listener: u64) -> bool {
        self.listener.get() == listener
    }

    pub fn gesture_recognizer_should_begin(
        &self,
        pan: &Rc<NavigationFullscreenPopPan>,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(nav) = self.navigation_controller.upgrade() else {
            return false;
        };
        if nav.read(cx).path.is_empty() {
            return false;
        }
        if Self::has_presented_layer(&nav, cx) {
            Self::log_presented_refusal(cx);
            return false;
        }

        if HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some() {
            return self.refuse("hold menu open", cx);
        }

        if NavigationInteractivePopGuard::is_media_editor_editing(cx) {
            return self.refuse("media editor editing", cx);
        }

        if nav.read(cx).navigation_window_has_running_transition(cx) {
            return self.refuse("navigation transitioning", cx);
        }

        if BubbleScrollPassthrough::with_shared(|passthrough| passthrough.contains(pan.location())) {
            return self.refuse("bubble scroll passthrough", cx);
        }
        let translation = pan.translation();
        let multiplier = if NavigationInteractivePopGuard::is_right_to_left(cx) { -1.0 } else { 1.0 };
        if translation.x.as_f32() * multiplier <= 0.0 {
            return false;
        }
        if translation.x.as_f32().abs() <= translation.y.as_f32().abs() {
            return false;
        }
        if NavigationInteractivePopGuard::blocks_interactive_pop(
            NavigationInteractivePopGuard::touch_down_point(&**pan),
            cx,
        ) {
            return self.refuse("horizontal drag owner", cx);
        }
        let touched_sideways_scrolls = self
            .session
            .borrow()
            .as_ref()
            .map(|session| session.touched_sideways_scrolls.clone())
            .unwrap_or_default();
        if Self::horizontal_scroll_can_follow(translation.x, &touched_sideways_scrolls) {
            return self.refuse("horizontal scroll moves first", cx);
        }
        if let Some(unsaved_changes_guard) = NavigationInteractivePopGuard::unsaved_changes_guard(cx)
            && unsaved_changes_guard(window, cx)
        {
            return self.refuse("unsaved changes", cx);
        }
        let stack = nav.read(cx).navigation_window_stack();
        let gesture: Rc<dyn SharedElementPanGesture> = pan.clone();
        let mode = match stack {
            Some(stack) => SharedElementNavigationDriver::shared().pop_pan_mode(&stack, &gesture, cx),
            None => SharedElementPopPanMode::Stock,
        };
        let router = self.router.borrow().upgrade();
        if let Some(router) = router {
            router.latch(mode);
        }
        true
    }

    fn horizontal_scroll_can_follow(window_dx: Pixels, touched: &[ScrollHandle]) -> bool {
        let local_dx = window_dx.as_f32();
        touched.iter().any(|scroll| {
            let Some((lower_bound, upper_bound)) = Self::sideways_range(scroll) else {
                return false;
            };
            let offset = -scroll.offset().x.as_f32();
            (local_dx > 0.0 && offset > lower_bound + SIDEWAYS_OFFSET_TOLERANCE)
                || (local_dx < 0.0 && offset < upper_bound - SIDEWAYS_OFFSET_TOLERANCE)
        })
    }

    fn sideways_range(scroll: &ScrollHandle) -> Option<(f32, f32)> {
        let max_offset = scroll.max_offset();
        let min_x = 0.0;
        let max_x = max_offset.x.as_f32();
        let horizontal_range = max_x - min_x;
        let vertical_range = max_offset.y.as_f32().max(0.0);
        if horizontal_range <= SIDEWAYS_RANGE_MINIMUM || horizontal_range <= vertical_range {
            return None;
        }
        Some((min_x, max_x))
    }

    pub fn request_back(&self, window: &mut Window, cx: &mut App) -> bool {
        let Some(nav) = self.navigation_controller.upgrade() else {
            return false;
        };
        if nav.read(cx).path.is_empty() {
            return false;
        }
        if Self::has_presented_layer(&nav, cx) {
            Self::log_presented_refusal(cx);
            return false;
        }
        if HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some() {
            return self.refuse("hold menu open", cx);
        }
        if NavigationInteractivePopGuard::is_media_editor_editing(cx) {
            return self.refuse("media editor editing", cx);
        }
        if nav.read(cx).navigation_window_has_running_transition(cx) {
            return self.refuse("navigation transitioning", cx);
        }
        if let Some(unsaved_changes_guard) = NavigationInteractivePopGuard::unsaved_changes_guard(cx)
            && unsaved_changes_guard(window, cx)
        {
            return self.refuse("unsaved changes", cx);
        }
        let stack = nav.read(cx).navigation_window_stack();
        if let Some(stack) = stack {
            SharedElementNavigationDriver::arm_exit_for_back(&stack, cx);
        }
        nav.update(cx, |store, cx| store.pop(cx));
        true
    }

    /// true if the stack can pop, more than one screen and nothing presented over it
    pub fn can_pop(&self, cx: &App) -> bool {
        if !ROUTE_STACK_SWIPE_POPS {
            return false;
        }
        self.navigation_controller
            .upgrade()
            .is_some_and(|nav| !nav.read(cx).path.is_empty() && !Self::has_presented_layer(&nav, cx))
    }

    /// true when a sheet or a bot mini app is shown over the stack
    fn has_presented_layer(nav: &Entity<NavigationStore>, cx: &App) -> bool {
        !SheetLayerStack::top_is_root(cx) || nav.read(cx).bot_mini_app_presentation().is_some()
    }

    fn log_presented_refusal(cx: &mut App) {
        let pack_sheet = StickerPackSheetStore::shared(cx);
        let (is_presented, pending_link) = {
            let pack_sheet = pack_sheet.read(cx);
            (pack_sheet.is_presented(), pack_sheet.pending_link().map(str::to_owned))
        };
        let link = pending_link.unwrap_or_else(|| NIL_DESCRIPTION.to_owned());
        PresentationChainTrace::log(
            &format!("back swipe refused, pack sheet open={is_presented} link={link}"),
            cx,
        );
    }

    /// a back that never starts is invisible so the log names the reason
    fn refuse(&self, reason: &str, cx: &mut App) -> bool {
        let top = self
            .navigation_controller
            .upgrade()
            .and_then(|nav| nav.read(cx).path.last().map(|route| route.sai_active_screen().id))
            .unwrap_or_else(|| NIL_DESCRIPTION.to_owned());
        let line = format!("back swipe refused: {reason}, top={top}");
        Sai::log(LOG_NAME, move || line, SaiLogKind::Ui, cx);
        false
    }

    /// feeds one trackpad scroll event into the back gesture
    /// the first move decides if it begins, a refused swipe is ignored until its end
    pub fn drive_pop(&self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut App) {
        if !ROUTE_STACK_SWIPE_POPS {
            return;
        }
        let ScrollDelta::Pixels(delta) = event.delta else {
            return;
        };
        match event.touch_phase {
            TouchPhase::Started => {
                self.swipe_started.set(true);
                let view_bounds = Some(Bounds::new(Point::default(), window.viewport_size()));
                *self.session.borrow_mut() = Some(NavigationFullscreenPopPanSession {
                    pan: Rc::new(NavigationFullscreenPopPan::new(event.position, view_bounds)),
                    touched_sideways_scrolls: NavigationInteractivePopGuard::touched_sideways_scrolls(event.position, cx),
                    decided: false,
                    began: false,
                });
            }
            TouchPhase::Moved => {
                if !self.swipe_started.get() {
                    return;
                }
                let Some((pan, decided)) =
                    self.session.borrow().as_ref().map(|session| (session.pan.clone(), session.decided))
                else {
                    return;
                };
                pan.add_translation(delta, event.position);
                if !decided {
                    let begins = self.gesture_recognizer_should_begin(&pan, window, cx);
                    if let Some(session) = self.session.borrow_mut().as_mut() {
                        session.decided = true;
                        session.began = begins;
                    }
                    if !begins {
                        pan.set_state(SharedElementPanState::Failed);
                        return;
                    }
                    pan.set_state(SharedElementPanState::Began);
                } else {
                    let began = self.session.borrow().as_ref().is_some_and(|session| session.began);
                    if !began {
                        return;
                    }
                    pan.set_state(SharedElementPanState::Changed);
                }
                self.route(&pan, cx);
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.swipe_started.set(false);
                let Some(session) = self.session.borrow_mut().take() else {
                    return;
                };
                if !session.began {
                    return;
                }
                let state = if event.touch_phase == TouchPhase::Ended {
                    SharedElementPanState::Ended
                } else {
                    SharedElementPanState::Cancelled
                };
                session.pan.set_state(state);
                self.route(&session.pan, cx);
            }
        }
    }

    fn route(&self, pan: &Rc<NavigationFullscreenPopPan>, cx: &mut App) {
        let router = self.router.borrow().upgrade();
        if let Some(router) = router {
            let gesture: Rc<dyn SharedElementPanGesture> = pan.clone();
            router.handle(&gesture, cx);
        }
    }
}

struct NavigationFullscreenPopStockTarget {
    navigation_controller: WeakEntity<NavigationStore>,
}

impl SharedElementStockPopTarget for NavigationFullscreenPopStockTarget {
    fn handle_navigation_transition(&self, pan: &Rc<dyn SharedElementPanGesture>, cx: &mut App) {
        if pan.state() != SharedElementPanState::Ended {
            return;
        }
        let Some(nav) = self.navigation_controller.upgrade() else {
            return;
        };
        let multiplier = if NavigationInteractivePopGuard::is_right_to_left(cx) { -1.0 } else { 1.0 };
        let travelled = pan.translation().x.as_f32() * multiplier;
        let velocity = pan.velocity().x.as_f32() * multiplier;
        let width = pan.view_bounds().map_or(0.0, |bounds| bounds.size.width.as_f32());
        let past_half = width > 0.0 && travelled / width > STOCK_POP_COMPLETION_FRACTION;
        if past_half || velocity > STOCK_POP_COMPLETION_VELOCITY {
            nav.update(cx, |store, cx| store.pop(cx));
        }
    }
}

/// keeps the pan with its delegate and router attached to the stack
struct NavigationFullscreenPopInstallation {
    navigation_controller: EntityId,
    delegate: Rc<NavigationFullscreenPopDelegate>,
    router: Rc<SharedElementPopPanRouter>,
    _stock_target: Rc<dyn SharedElementStockPopTarget>,
}

#[derive(Default)]
struct NavigationFullscreenPopInstallations {
    installed: Vec<NavigationFullscreenPopInstallation>,
}

impl Global for NavigationFullscreenPopInstallations {}

pub enum NavigationFullscreenPop {}

impl NavigationFullscreenPop {
    pub fn install_if_needed(navigation_controller: &Entity<NavigationStore>, cx: &mut App) {
        let Some(stack) = navigation_controller.read(cx).navigation_window_stack() else {
            return;
        };
        SharedElementNavigationDriver::shared().attach(&stack, cx);
        if Self::is_installed(navigation_controller.entity_id(), cx) {
            return;
        }

        let delegate = Rc::new(NavigationFullscreenPopDelegate::new(navigation_controller.downgrade()));
        let stock_target: Rc<dyn SharedElementStockPopTarget> =
            Rc::new(NavigationFullscreenPopStockTarget { navigation_controller: navigation_controller.downgrade() });
        let router = SharedElementPopPanRouter::new(&stack, &stock_target);
        *delegate.router.borrow_mut() = Rc::downgrade(&router);

        let installations = cx.default_global::<NavigationFullscreenPopInstallations>();
        installations.installed.retain(|installation| installation.delegate.navigation_controller.upgrade().is_some());
        installations.installed.push(NavigationFullscreenPopInstallation {
            navigation_controller: navigation_controller.entity_id(),
            delegate,
            router,
            _stock_target: stock_target,
        });
    }

    pub fn is_installed(navigation_controller: EntityId, cx: &App) -> bool {
        cx.try_global::<NavigationFullscreenPopInstallations>().is_some_and(|installations| {
            installations
                .installed
                .iter()
                .any(|installation| installation.navigation_controller == navigation_controller)
        })
    }

    /// installed back handling of a stack, for hotkeys and trackpad
    pub fn delegate(navigation_controller: EntityId, cx: &App) -> Option<Rc<NavigationFullscreenPopDelegate>> {
        cx.try_global::<NavigationFullscreenPopInstallations>().and_then(|installations| {
            installations
                .installed
                .iter()
                .find(|installation| installation.navigation_controller == navigation_controller)
                .map(|installation| installation.delegate.clone())
        })
    }

    /// back handling of the last installed stack that is still alive
    pub fn active_delegate(cx: &App) -> Option<Rc<NavigationFullscreenPopDelegate>> {
        cx.try_global::<NavigationFullscreenPopInstallations>().and_then(|installations| {
            installations
                .installed
                .iter()
                .rev()
                .find(|installation| installation.delegate.navigation_controller.upgrade().is_some())
                .map(|installation| installation.delegate.clone())
        })
    }

    /// router the back swipe of a stack feeds
    pub fn router(navigation_controller: EntityId, cx: &App) -> Option<Rc<SharedElementPopPanRouter>> {
        cx.try_global::<NavigationFullscreenPopInstallations>().and_then(|installations| {
            installations
                .installed
                .iter()
                .find(|installation| installation.navigation_controller == navigation_controller)
                .map(|installation| installation.router.clone())
        })
    }
}
