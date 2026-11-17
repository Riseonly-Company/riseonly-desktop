use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyElement, AnyView, AnyWindowHandle, App, Bounds, Pixels, Point, RenderImage, Size, Window};

use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentMode, SharedElementKey, SharedElementPart, SharedElementReturnScrollMathBand,
    SharedElementReturnScrollMathClamp, SharedElementSurface,
};
use crate::core::shared_element_transition::overlay::shared_element_overlay_contracts::SharedElementOverlayOrigin;
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementStackHost;

pub trait SharedElementOriginContainer {
    fn surface(&self) -> SharedElementSurface;
    fn host_view_controller(&self, cx: &App) -> Option<SharedElementStackHost>;
    fn host_view(&self, cx: &App) -> Option<AnyWindowHandle>;
    fn open_capture(&self, item_id: &str, cx: &mut App) -> Option<SharedElementCapture>;
    fn open_lands_on_same_picture(&self, item_id: &str, cx: &App) -> bool;
    /// pure read, no side effects
    fn can_return(&self, item_id: &str, cx: &App) -> bool;
    fn return_cover(&self, item_id: &str, cx: &App) -> Option<SharedElementCover>;
    fn prepare_return(&self, item_id: &str, epoch: u64, cx: &mut App) -> Option<SharedElementReturnPlan>;
    fn resolve_return(&self, epoch: u64, cx: &mut App) -> SharedElementTargetResolution;
    fn measured_target_frame(&self, item_id: &str, cx: &App) -> Option<Bounds<Pixels>>;
    fn cancel_return(&self, epoch: u64, cx: &mut App);
    fn finish_return(&self, epoch: u64, cx: &mut App);

    fn as_overlay_origin(self: Rc<Self>) -> Option<Rc<dyn SharedElementOverlayOrigin>> {
        None
    }
}

pub trait SharedElementDestinationContainer {
    fn surface(&self) -> SharedElementSurface;
    /// a list opened from scope X only returns to X
    fn origin_surface(&self) -> SharedElementSurface;
    fn host_view_controller(&self, cx: &App) -> Option<SharedElementStackHost>;
    fn host_view(&self, cx: &App) -> Option<AnyWindowHandle>;
    fn open_target(&self, item_id: &str, part: SharedElementPart, cx: &App) -> Option<Bounds<Pixels>>;
    fn open_landing_mode(
        &self,
        item_id: &str,
        content_aspect: f32,
        clip: Bounds<Pixels>,
        cx: &App,
    ) -> SharedElementContentMode;
    fn current_capture(&self, cx: &mut App) -> Option<SharedElementCapture>;
    fn can_begin_interactive_return(&self, cx: &App) -> bool;
    fn did_settle_open(&self, epoch: u64, cx: &mut App);

    fn lift_keys(&self, item_id: &str, part: SharedElementPart, _cx: &App) -> HashSet<SharedElementKey> {
        if part != SharedElementPart::Media {
            return HashSet::new();
        }
        HashSet::from([SharedElementKey::new(self.surface(), item_id, SharedElementPart::Media)])
    }

    fn open_landing_is_ready(&self, _item_id: &str, _cx: &App) -> Option<bool> {
        None
    }
}

pub type SharedElementReturnResumableHandler = Rc<dyn Fn(u64, &mut App)>;

/// implemented by the profile scroll coordinator
pub trait SharedElementReturnScroller {
    fn can_prepare_return(&self, tab_key: i64, cx: &App) -> bool;
    fn return_viewport_snapshot(&self, tab_key: i64, cx: &App) -> Option<SharedElementReturnViewport>;
    fn prepare_return(&self, request: SharedElementReturnRequest, cx: &mut App) -> Option<SharedElementReturnPlan>;
    fn resolve_return(
        &self,
        epoch: u64,
        item_content_rect: Bounds<Pixels>,
        cx: &mut App,
    ) -> SharedElementReturnResolution;
    fn cancel_return(&self, epoch: u64, cx: &mut App);
    fn finish_return(&self, epoch: u64, cx: &mut App);
    fn on_return_resumable(&self) -> Option<SharedElementReturnResumableHandler>;
    fn set_on_return_resumable(&self, handler: Option<SharedElementReturnResumableHandler>);

    fn cancel_live_return(&self, epoch: u64, cx: &mut App) {
        self.cancel_return(epoch, cx);
    }

    fn note_open_position(&self, _tab_key: i64, _cx: &mut App) {}

    fn reset_to_content_top(&self, _tab_key: i64, _cx: &mut App) {}
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SharedElementReturnRequest {
    pub tab_key: i64,
    pub item_content_rect: Bounds<Pixels>,
    pub band: SharedElementReturnScrollMathBand,
    pub epoch: u64,
    pub keeps_position: bool,
}

/// profile viewport numbers, H Y0 X0 inner width C and R
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SharedElementReturnViewport {
    pub height: f32,
    pub screen_origin: Point<Pixels>,
    pub inner_width: f32,
    pub collapse_range: f32,
    pub inner_range: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SharedElementReturnPlan {
    pub epoch: u64,
    pub predicted_frame: Bounds<Pixels>,
    pub clamp: SharedElementReturnScrollMathClamp,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SharedElementReturnResolution {
    Ready { predicted_frame: Bounds<Pixels> },
    Deferred,
    Unavailable,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SharedElementTargetResolution {
    Ready(Bounds<Pixels>),
    Deferred,
    Unavailable,
}

#[derive(Clone)]
pub enum SharedElementFlightContent {
    Bitmap(Arc<RenderImage>, f32),
    Replicant(AnyView, Size<Pixels>),
}

#[derive(Clone)]
pub struct SharedElementCapture {
    pub item_id: String,
    pub part: SharedElementPart,
    pub source_key: SharedElementKey,
    pub clip: Bounds<Pixels>,
    pub content: SharedElementFlightContent,
    pub content_rect: Bounds<Pixels>,
    pub mode: SharedElementContentMode,
    pub backdrop: Option<Arc<RenderImage>>,
    pub flying_identity: Option<String>,
    pub stop_frame: Option<Rc<SharedElementStopFrameRequest>>,
    pub live_readiness: Option<Rc<SharedElementLiveReadiness>>,
    pub placeholder: Option<Arc<RenderImage>>,
}

/// picture a cover shows
/// the element variant is a text cell drawn live in the cover frame
/// gpui cant rasterize a scene offscreen into a bitmap
#[derive(Clone)]
pub enum SharedElementCoverContent {
    Image(Arc<RenderImage>),
    Element(Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>),
}

#[derive(Clone)]
pub struct SharedElementCover {
    pub image: SharedElementCoverContent,
    pub aspect: f32,
    pub identity: String,
}

#[derive(Clone)]
pub struct SharedElementStopFrameResult {
    pub image: Arc<RenderImage>,
    pub aspect: f32,
    pub content_rect: Bounds<Pixels>,
}

#[derive(Clone)]
pub enum SharedElementStopFrameRequestState {
    Pending,
    Ready(SharedElementStopFrameResult),
    Missed,
}

type SharedElementStopFrameSettleHandler = Box<dyn FnOnce(Option<SharedElementStopFrameResult>)>;

/// one pending stop frame grab of a playing reel, no renderer types so no video dependency
/// grabber creates it, installs the cancel handler and settles it once
/// consumers read result and compare created_at with the budget
pub struct SharedElementStopFrameRequest {
    pub created_at: Duration,
    state: RefCell<SharedElementStopFrameRequestState>,
    settle_handlers: RefCell<Vec<SharedElementStopFrameSettleHandler>>,
    cancellation_handler: RefCell<Option<Box<dyn FnOnce()>>>,
}

impl SharedElementStopFrameRequest {
    pub fn new(created_at: Duration, cancellation_handler: Option<Box<dyn FnOnce()>>) -> Self {
        Self {
            created_at,
            state: RefCell::new(SharedElementStopFrameRequestState::Pending),
            settle_handlers: RefCell::new(Vec::new()),
            cancellation_handler: RefCell::new(cancellation_handler),
        }
    }

    pub fn state(&self) -> SharedElementStopFrameRequestState {
        self.state.borrow().clone()
    }

    pub fn result(&self) -> Option<SharedElementStopFrameResult> {
        match &*self.state.borrow() {
            SharedElementStopFrameRequestState::Ready(result) => Some(result.clone()),
            SharedElementStopFrameRequestState::Pending | SharedElementStopFrameRequestState::Missed => None,
        }
    }

    pub fn is_pending(&self) -> bool {
        matches!(&*self.state.borrow(), SharedElementStopFrameRequestState::Pending)
    }

    pub fn is_expired(&self, now: Duration, budget: f64) -> bool {
        now.saturating_sub(self.created_at).as_secs_f64() >= budget
    }

    pub fn set_cancellation_handler(&self, handler: Option<Box<dyn FnOnce()>>) {
        if !self.is_pending() {
            return;
        }
        *self.cancellation_handler.borrow_mut() = handler;
    }

    /// grabber side, ignored once settled
    pub fn resolve(&self, result: SharedElementStopFrameResult) {
        self.settle(SharedElementStopFrameRequestState::Ready(result));
    }

    pub fn miss(&self) {
        self.settle(SharedElementStopFrameRequestState::Missed);
    }

    /// consumer side, stop waiting and run the cancel handler once
    pub fn cancel(&self) {
        if !self.is_pending() {
            return;
        }
        let handler = self.cancellation_handler.borrow_mut().take();
        self.settle(SharedElementStopFrameRequestState::Missed);
        if let Some(handler) = handler {
            handler();
        }
    }

    /// runs right away if already settled, else once on settle
    pub fn on_settle(&self, handler: impl FnOnce(Option<SharedElementStopFrameResult>) + 'static) {
        if !self.is_pending() {
            handler(self.result());
            return;
        }
        self.settle_handlers.borrow_mut().push(Box::new(handler));
    }

    fn settle(&self, new_state: SharedElementStopFrameRequestState) {
        if !self.is_pending() {
            return;
        }
        *self.state.borrow_mut() = new_state;
        self.cancellation_handler.borrow_mut().take();
        let handlers = std::mem::take(&mut *self.settle_handlers.borrow_mut());
        let settled = self.result();
        for handler in handlers {
            handler(settled.clone());
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementLiveReadinessState {
    Pending,
    Ready,
    Missed,
}

type SharedElementLiveReadinessSettleHandler = Box<dyn FnOnce(bool)>;

/// one wait for a live video first displayed frame, the video view marks it ready
/// consumers compare created_at with LIVE_VIDEO_READY_BUDGET
/// fly the placeholder on a miss and cancel when they stop waiting
pub struct SharedElementLiveReadiness {
    pub created_at: Duration,
    state: Cell<SharedElementLiveReadinessState>,
    settle_handlers: RefCell<Vec<SharedElementLiveReadinessSettleHandler>>,
}

impl SharedElementLiveReadiness {
    pub fn new(created_at: Duration) -> Self {
        Self {
            created_at,
            state: Cell::new(SharedElementLiveReadinessState::Pending),
            settle_handlers: RefCell::new(Vec::new()),
        }
    }

    pub fn state(&self) -> SharedElementLiveReadinessState {
        self.state.get()
    }

    pub fn is_ready(&self) -> bool {
        self.state.get() == SharedElementLiveReadinessState::Ready
    }

    pub fn is_pending(&self) -> bool {
        self.state.get() == SharedElementLiveReadinessState::Pending
    }

    pub fn is_expired(&self, now: Duration) -> bool {
        now.saturating_sub(self.created_at).as_secs_f64() >= SharedElementMotion::LIVE_VIDEO_READY_BUDGET
    }

    /// video side, ignored once settled
    pub fn mark_ready(&self) {
        self.settle(SharedElementLiveReadinessState::Ready);
    }

    pub fn mark_missed(&self) {
        self.settle(SharedElementLiveReadinessState::Missed);
    }

    /// consumer side, stop waiting
    /// a pending wait settles as missed
    pub fn cancel(&self) {
        self.settle(SharedElementLiveReadinessState::Missed);
    }

    /// runs right away if already settled, else once on settle
    pub fn on_settle(&self, handler: impl FnOnce(bool) + 'static) {
        if !self.is_pending() {
            handler(self.is_ready());
            return;
        }
        self.settle_handlers.borrow_mut().push(Box::new(handler));
    }

    fn settle(&self, new_state: SharedElementLiveReadinessState) {
        if !self.is_pending() {
            return;
        }
        self.state.set(new_state);
        let handlers = std::mem::take(&mut *self.settle_handlers.borrow_mut());
        let ready = self.is_ready();
        for handler in handlers {
            handler(ready);
        }
    }
}
