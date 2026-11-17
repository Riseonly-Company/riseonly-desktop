use std::path::PathBuf;

use gpui::{
    Bounds, DisplayId, Pixels, WindowBackgroundAppearance, WindowBounds, WindowDecorations,
    WindowKind, WindowOptions, point, px, size,
};
use parking_lot::Mutex;
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::notifications::{
    LocalNotification, NotificationAction, NotificationPresenter, NotificationTag,
};

pub const FLOATING_WINDOW_WIDTH: f32 = 320.0;
pub const FLOATING_WINDOW_HEIGHT: f32 = 180.0;
pub const SCREEN_EDGE_INSET: f32 = 16.0;
pub const INCOMING_WINDOW_WIDTH: f32 = 360.0;
pub const INCOMING_WINDOW_HEIGHT: f32 = 112.0;

pub const CALL_TAG_KEY_PREFIX: &str = "call:";

/// why a call window couldnt get what the os layer was asked for, the call goes on either way
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum CallWindowUnavailable {
    #[error("call windows are not written for {0:?}")]
    BranchNotWritten(HostOs),
    #[error("call window requests must run on the main thread")]
    NotMainThread,
    #[error("no native window handle: {0}")]
    NoNativeWindow(String),
    #[error("the window system does not allow it: {0}")]
    WindowSystemLimit(&'static str),
    #[error("the system refused the call window request: {0}")]
    SystemRefused(String),
}

/// default answer of an os layer that doesnt override request_attention
pub const fn unavailable_by_default(host: HostOs) -> CallWindowUnavailable {
    CallWindowUnavailable::BranchNotWritten(host)
}

/// performed when the os layer did what was asked
pub fn support_of<T>(outcome: &Result<T, CallWindowUnavailable>) -> PlatformSupport {
    if outcome.is_ok() {
        PlatformSupport::Performed
    } else {
        PlatformSupport::Unsupported
    }
}

/// screen corner the floating video window sits in
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum FloatingCorner {
    TopLeading,
    TopTrailing,
    BottomLeading,
    #[default]
    BottomTrailing,
}

impl FloatingCorner {
    pub const ALL: [Self; 4] = [
        Self::TopLeading,
        Self::TopTrailing,
        Self::BottomLeading,
        Self::BottomTrailing,
    ];

    const fn is_top(self) -> bool {
        matches!(self, Self::TopLeading | Self::TopTrailing)
    }

    const fn is_leading(self) -> bool {
        matches!(self, Self::TopLeading | Self::BottomLeading)
    }
}

pub fn floating_window_size() -> gpui::Size<Pixels> {
    size(px(FLOATING_WINDOW_WIDTH), px(FLOATING_WINDOW_HEIGHT))
}

pub fn incoming_window_size() -> gpui::Size<Pixels> {
    size(px(INCOMING_WINDOW_WIDTH), px(INCOMING_WINDOW_HEIGHT))
}

/// window of window_size in a corner of visible (display area without dock
/// menu bar or taskbar), inset from both edges
/// a display smaller than the window keeps it at the display origin instead of pushing it off screen
pub fn corner_bounds(
    visible: Bounds<Pixels>,
    window_size: gpui::Size<Pixels>,
    corner: FloatingCorner,
) -> Bounds<Pixels> {
    let inset = px(SCREEN_EDGE_INSET);
    let left = visible.origin.x + inset;
    let top = visible.origin.y + inset;
    let right = (visible.right() - inset - window_size.width).max(visible.origin.x);
    let bottom = (visible.bottom() - inset - window_size.height).max(visible.origin.y);
    let x = if corner.is_leading() { left.min(right) } else { right };
    let y = if corner.is_top() { top.min(bottom) } else { bottom };
    Bounds::new(point(x, y), window_size)
}

/// where the floating video window opens in a corner of visible
pub fn floating_window_bounds(visible: Bounds<Pixels>, corner: FloatingCorner) -> Bounds<Pixels> {
    corner_bounds(visible, floating_window_size(), corner)
}

/// corner a dragged window belongs to, the quarter of visible its center is in
/// so it opens next time where the user left it
pub fn nearest_corner(visible: Bounds<Pixels>, window: Bounds<Pixels>) -> FloatingCorner {
    let screen_center = visible.center();
    let window_center = window.center();
    let top = window_center.y < screen_center.y;
    let leading = window_center.x < screen_center.x;
    match (top, leading) {
        (true, true) => FloatingCorner::TopLeading,
        (true, false) => FloatingCorner::TopTrailing,
        (false, true) => FloatingCorner::BottomLeading,
        (false, false) => FloatingCorner::BottomTrailing,
    }
}

static REMEMBERED_CORNER: Mutex<Option<FloatingCorner>> = Mutex::new(None);

/// corner the floating window was last left in during this run
pub fn remembered_floating_corner() -> FloatingCorner {
    let remembered: Option<FloatingCorner> = *REMEMBERED_CORNER.lock();
    remembered.unwrap_or_default()
}

/// keeps the corner of a floating window the user moved
/// called on close or move with its bounds and the visible area of its display
pub fn remember_floating_corner(visible: Bounds<Pixels>, window: Bounds<Pixels>) -> FloatingCorner {
    let corner = nearest_corner(visible, window);
    *REMEMBERED_CORNER.lock() = Some(corner);
    corner
}

/// gpui kind of a call window on a host
/// popup on macos and windows, normal window on linux
pub const fn call_window_kind(host: HostOs) -> WindowKind {
    match host {
        HostOs::MacOs | HostOs::Windows => WindowKind::PopUp,
        HostOs::Linux => WindowKind::Normal,
    }
}

fn call_window_options(
    host: HostOs,
    bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
    focus: bool,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus,
        show: true,
        kind: call_window_kind(host),
        is_movable: true,
        app_owns_titlebar_drag: true,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        window_background: WindowBackgroundAppearance::Opaque,
        window_decorations: Some(WindowDecorations::Client),
        ..WindowOptions::default()
    }
}

/// floating video window in a corner of the visible area of the display the main window is on
/// it opens without focus so typing in another app isnt interrupted
pub fn floating_window_options_for(
    host: HostOs,
    visible: Bounds<Pixels>,
    display_id: Option<DisplayId>,
    corner: FloatingCorner,
) -> WindowOptions {
    call_window_options(host, floating_window_bounds(visible, corner), display_id, false)
}

/// floating_window_options_for on this host in the remembered corner
pub fn floating_window_options(
    visible: Bounds<Pixels>,
    display_id: Option<DisplayId>,
) -> WindowOptions {
    floating_window_options_for(
        HostOs::current(),
        visible,
        display_id,
        remembered_floating_corner(),
    )
}

/// true when the incoming call window takes the keyboard as it opens
/// so enter and escape answer and decline at once
/// on macos its a non activating panel and typing from another app would land here
pub const fn incoming_window_takes_focus(host: HostOs) -> bool {
    match host {
        HostOs::MacOs | HostOs::Windows | HostOs::Linux => false,
    }
}

/// incoming call window, top trailing like a system banner
pub fn incoming_window_options_for(
    host: HostOs,
    visible: Bounds<Pixels>,
    display_id: Option<DisplayId>,
) -> WindowOptions {
    let bounds = corner_bounds(visible, incoming_window_size(), FloatingCorner::TopTrailing);
    call_window_options(host, bounds, display_id, incoming_window_takes_focus(host))
}

pub fn incoming_window_options(
    visible: Bounds<Pixels>,
    display_id: Option<DisplayId>,
) -> WindowOptions {
    incoming_window_options_for(HostOs::current(), visible, display_id)
}

/// what the user answered an incoming call with, a window button and a key or a notification button
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IncomingCallResponse {
    Answer,
    Decline,
}

impl IncomingCallResponse {
    /// enter answers and escape declines (gpui key names)
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "enter" => Some(Self::Answer),
            "escape" => Some(Self::Decline),
            _ => None,
        }
    }

    /// response a notification button carries
    pub fn from_action_id(action_id: &str) -> Option<Self> {
        match action_id {
            CALL_ANSWER_ACTION_ID => Some(Self::Answer),
            CALL_DECLINE_ACTION_ID => Some(Self::Decline),
            _ => None,
        }
    }

    pub const fn action_id(self) -> &'static str {
        match self {
            Self::Answer => CALL_ANSWER_ACTION_ID,
            Self::Decline => CALL_DECLINE_ACTION_ID,
        }
    }
}

pub const CALL_ANSWER_ACTION_ID: &str = "CALL_ANSWER_ACTION";
pub const CALL_DECLINE_ACTION_ID: &str = "CALL_DECLINE_ACTION";

/// tag of the notification of a call
/// a second post for the same call replaces the first and the tag retracts it
pub fn call_notification_tag(call_id: &str) -> NotificationTag {
    NotificationTag::unique(&format!("{CALL_TAG_KEY_PREFIX}{call_id}"))
}

/// call id a notification tag carries, none for any other notification
pub fn call_id_from_notification_tag(tag: &NotificationTag) -> Option<&str> {
    let prefix = call_notification_tag("");
    tag.as_str()
        .strip_prefix(prefix.as_str())
        .filter(|call_id| !call_id.is_empty())
}

/// text and buttons of an incoming call notification, labels arrive already translated
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct IncomingCallNotice {
    pub call_id: String,
    pub caller_name: String,
    pub body: String,
    pub answer_label: String,
    pub decline_label: String,
    pub avatar: Option<PathBuf>,
}

impl IncomingCallNotice {
    /// notification with answer and decline buttons under the call tag
    pub fn notification(&self) -> LocalNotification {
        let notification = LocalNotification::new(
            call_notification_tag(&self.call_id),
            self.caller_name.clone(),
            self.body.clone(),
        )
        .with_action(NotificationAction::new(
            CALL_ANSWER_ACTION_ID,
            self.answer_label.clone(),
        ))
        .with_action(NotificationAction::new(
            CALL_DECLINE_ACTION_ID,
            self.decline_label.clone(),
        ));
        match &self.avatar {
            Some(path) => notification.with_avatar(path.clone()),
            None => notification,
        }
    }
}

pub fn present_incoming_call(
    presenter: &dyn NotificationPresenter,
    notice: &IncomingCallNotice,
) -> PlatformSupport {
    presenter.present(&notice.notification())
}

/// retracts the notification of call_id, called on answer from anywhere (window and key and button
/// another device) and when the call ends
pub fn retract_call_notification(
    presenter: &dyn NotificationPresenter,
    call_id: &str,
) -> PlatformSupport {
    presenter.dismiss(&call_notification_tag(call_id))
}

/// how hard the os is asked to draw the user to the app
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AttentionLevel {
    Informational,
    Critical,
}

/// one attention request the os holds for the app, dropping it cancels the request
/// not send, appkit cancels only on the main thread where it was asked
#[must_use = "dropping the request cancels it"]
pub struct AttentionRequest {
    cancel: Option<Box<dyn FnOnce()>>,
}

impl AttentionRequest {
    /// cancel gives the request back to the system once, on drop
    pub fn new(cancel: impl FnOnce() + 'static) -> Self {
        Self {
            cancel: Some(Box::new(cancel)),
        }
    }
}

impl Drop for AttentionRequest {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

impl std::fmt::Debug for AttentionRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AttentionRequest")
    }
}

pub trait CallWindowLayer {
    fn keep_above(&self, window: &gpui::Window) -> Result<(), CallWindowUnavailable> {
        let _ = window;
        Ok(())
    }

    fn request_attention(
        &self,
        window: &gpui::Window,
        level: AttentionLevel,
    ) -> Result<AttentionRequest, CallWindowUnavailable> {
        let _ = (window, level);
        Err(unavailable_by_default(HostOs::current()))
    }

    fn start_move(&self, window: &gpui::Window) -> Result<(), CallWindowUnavailable> {
        window.start_window_move();
        Ok(())
    }
}

/// called by the window right after it opened (open_window callback or its first render)
pub fn keep_call_window_above(window: &gpui::Window) -> Result<(), CallWindowUnavailable> {
    let outcome = super::os::NativeCallWindow.keep_above(window);
    if let Err(reason) = &outcome {
        tracing::info!(target: "riseonly", %reason, "the call window is an ordinary window here");
    }
    outcome
}

/// called from a mouse move handler while the left button is held on a call window body
/// windows runs the move in a modal loop so no mouse up arrives and a second mouse down
/// comes at the start so dont read the end of a move from those, compare the window origin
pub fn start_call_window_move(window: &gpui::Window) -> PlatformSupport {
    let outcome = super::os::NativeCallWindow.start_move(window);
    if let Err(reason) = &outcome {
        tracing::info!(target: "riseonly", %reason, "the call window does not move here");
    }
    support_of(&outcome)
}

/// screen point as the lparam of a win32 mouse message, x in the low word and y in the high
/// each a signed 16 bit value so monitors left or above the primary cut the words, no clamping
pub const fn screen_point_lparam(x: i32, y: i32) -> isize {
    let low = x as u16 as u32;
    let high = y as u16 as u32;
    ((high << 16) | low) as isize
}

/// one attention request for the ringing call, same call again keeps the running one (no second bounce)
/// a new call replaces the old one, answer or decline or end drops it and that cancels the request
#[derive(Default)]
pub struct CallAttention {
    active: Option<(String, AttentionRequest)>,
    failure: Option<CallWindowUnavailable>,
}

impl CallAttention {
    pub fn new() -> Self {
        Self::default()
    }

    /// asks the os layer for critical attention to the window for this call
    pub fn begin(&mut self, window: &gpui::Window, call_id: &str) -> PlatformSupport {
        self.begin_with(call_id, || {
            super::os::NativeCallWindow.request_attention(window, AttentionLevel::Critical)
        })
    }

    /// begin with the request made by a closure, tests pass a fake system here
    pub fn begin_with(
        &mut self,
        call_id: &str,
        request: impl FnOnce() -> Result<AttentionRequest, CallWindowUnavailable>,
    ) -> PlatformSupport {
        if self.is_requesting(call_id) {
            return PlatformSupport::Performed;
        }
        self.active = None;
        match request() {
            Ok(held) => {
                self.active = Some((call_id.to_owned(), held));
                self.failure = None;
                PlatformSupport::Performed
            }
            Err(failure) => {
                tracing::info!(target: "riseonly", %failure, "no attention request for the incoming call");
                self.failure = Some(failure);
                PlatformSupport::Unsupported
            }
        }
    }

    /// cancels the request of call_id, a request of another call keeps running
    pub fn end(&mut self, call_id: &str) {
        if self.is_requesting(call_id) {
            self.active = None;
        }
        self.failure = None;
    }

    /// cancels whatever request is held
    pub fn end_all(&mut self) {
        self.active = None;
        self.failure = None;
    }

    pub fn is_requesting(&self, call_id: &str) -> bool {
        self.active
            .as_ref()
            .is_some_and(|(active_id, _)| active_id == call_id)
    }

    pub fn requesting_call_id(&self) -> Option<&str> {
        self.active.as_ref().map(|(call_id, _)| call_id.as_str())
    }

    /// why the last request was refused, until it ends
    pub fn failure(&self) -> Option<&CallWindowUnavailable> {
        self.failure.as_ref()
    }
}

impl std::fmt::Debug for CallAttention {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CallAttention")
            .field("call_id", &self.requesting_call_id())
            .field("failure", &self.failure)
            .finish()
    }
}
