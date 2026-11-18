use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyWindowHandle, App, AppContext, AsyncApp, Bounds, ClickEvent, Context, CursorStyle, DisplayId, DispatchPhase,
    Entity, EntityId, IntoElement, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, RenderOnce,
    SharedString, Subscription, Task, WeakEntity, Window, WindowHandle, black, canvas, div, px, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_media::rtc_media::livekit_room::RemoteVideoFrame;
use rise_media::texture::native_surface::{PreparedHostFrame, present_host_frame_rotated};
use rise_i18n::tr;
use rise_platform::call_window::{
    CallWindowResize, FloatingResizeCorner, floating_window_options, keep_call_window_above, remember_floating_corner,
    set_call_window_frame, start_call_window_move, start_call_window_resize,
};
use rise_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use crate::modules::rtc::media::rise_rtc_live_kit_media_controller::RiseRTCLiveKitMediaController;
use crate::modules::rtc::system::rise_rtc_call_window_hooks::rise_rtc_call_window_hooks;

const CONTROLS_VISIBLE_FOR: Duration = Duration::from_secs(3);
const CONTROL_SIDE: f32 = 36.0;
const CORNER_GRIP_SIDE: f32 = 14.0;

/// current window owner
/// two screens compete (direct call and voice room) and can both be mounted
/// without an owner a leaving screen would kill the others live window
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RiseRTCCallPictureInPictureControllerOwner {
    DirectCall,
    DirectCallWidget,
    VoiceRoom,
    StreamMiniPlayer,
}

impl RiseRTCCallPictureInPictureControllerOwner {
    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::DirectCall => "directCall",
            Self::DirectCallWidget => "directCallWidget",
            Self::VoiceRoom => "voiceRoom",
            Self::StreamMiniPlayer => "streamMiniPlayer",
        }
    }

    /// one on one outranks a room, with both open the window belongs to the call
    /// the open call screen outranks its own widget, a mini player the person asked for outranks all
    pub const fn rank(self) -> i32 {
        match self {
            Self::StreamMiniPlayer => 3,
            Self::DirectCall => 2,
            Self::DirectCallWidget => 1,
            Self::VoiceRoom => 0,
        }
    }
}

/// picture in picture for a call
/// stand in on desktop is a floating call window above all, no taskbar button
/// opens when the main window goes inactive, closes on return
pub struct RiseRTCCallPictureInPictureController {
    is_active: bool,
    feed: RiseRTCPictureInPictureFeed,
    controller: Option<WindowHandle<RiseRTCPictureInPictureWindow>>,
    controller_generation: u64,
    attached_track: Option<String>,
    attached_source_id: Option<String>,
    owner: Option<RiseRTCCallPictureInPictureControllerOwner>,
    is_enabled: bool,
    main_window: Option<AnyWindowHandle>,
    widget_standby: Option<RiseRTCPictureInPictureStandby>,
    held_face: Option<RiseRTCPictureInPictureFace>,
    stream_user_id: Option<String>,
    _lifecycle_observers: Vec<RiseSubscription>,
    _lifecycle_pump: Task<()>,
}

/// counterpart the floating window stands for when no live picture is left
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseRTCPictureInPictureFace {
    pub user_id: Option<String>,
    pub name: String,
    pub avatar_url: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct RiseRTCPictureInPictureStandby {
    holder: EntityId,
    track: String,
    source_id: Option<String>,
}

impl RiseRTCCallPictureInPictureController {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let (sender, mut inbox) = mpsc::unbounded::<bool>();
        let events = RiseAppEvents::default_center();
        let lifecycle_observers = [
            (UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION, true),
            (UIApplication::WILL_ENTER_FOREGROUND_NOTIFICATION, false),
            (UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, false),
        ]
        .into_iter()
        .map(|(name, background)| {
            let sender = sender.clone();
            events.observe(name, move |_| {
                let _ = sender.unbounded_send(background);
            })
        })
        .collect();
        let lifecycle_pump = cx.spawn(async move |this, cx| {
            while let Some(background) = inbox.next().await {
                if this
                    .update(cx, |this: &mut Self, cx: &mut Context<Self>| this.app_background_changed(background, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            is_active: false,
            feed: RiseRTCPictureInPictureFeed::new(),
            controller: None,
            controller_generation: 0,
            attached_track: None,
            attached_source_id: None,
            owner: None,
            is_enabled: false,
            main_window: None,
            widget_standby: None,
            held_face: None,
            stream_user_id: None,
            _lifecycle_observers: lifecycle_observers,
            _lifecycle_pump: lifecycle_pump,
        }
    }

    /// every desktop host can open the floating window, where the os
    /// cant keep it above others (wayland) its just a normal window
    pub fn is_supported() -> bool {
        true
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    /// floats the watched stream because the person asked, it stays above every window until closed
    /// or the call opens again, focus changes do not touch it
    pub fn float_stream(&mut self, track: String, source_id: String, user_id: String, cx: &mut Context<Self>) {
        let owner = RiseRTCCallPictureInPictureControllerOwner::StreamMiniPlayer;
        if let Some(stale) = self.owner.filter(|current| *current != owner) {
            self.disable(stale, cx);
        }
        self.enable(owner, cx);
        self.present(Some(track), Some(source_id), owner, cx);
        self.stream_user_id = Some(user_id);
        if self.controller.is_some() {
            return;
        }
        let main_window = rise_rtc_call_window_hooks()
            .and_then(|hooks| cx.windows().into_iter().find(|window| (hooks.is_main_window)(*window)));
        self.main_window = main_window;
        let display = main_window
            .and_then(|window| window.update(cx, |_, window, cx| window.display(cx)).ok().flatten())
            .or_else(|| cx.primary_display());
        let Some(display) = display else {
            return;
        };
        self.start_picture_in_picture(display.visible_bounds(), Some(display.id()), cx);
    }

    pub fn is_floating_stream(&self) -> bool {
        self.owner == Some(RiseRTCCallPictureInPictureControllerOwner::StreamMiniPlayer)
    }

    pub fn end_stream_float(&mut self, cx: &mut Context<Self>) {
        self.stream_user_id = None;
        self.disable(RiseRTCCallPictureInPictureControllerOwner::StreamMiniPlayer, cx);
    }

    pub fn enable(&mut self, requested_owner: RiseRTCCallPictureInPictureControllerOwner, cx: &mut Context<Self>) {
        if !Self::is_supported() || !self.may_claim(requested_owner) {
            return;
        }
        self.owner = Some(requested_owner);
        self.feed.set_active(self.is_active);
        if self.is_enabled {
            return;
        }
        self.is_enabled = true;
        cx.notify();
    }

    pub fn disable(&mut self, requested_owner: RiseRTCCallPictureInPictureControllerOwner, cx: &mut Context<Self>) {
        if self.owner != Some(requested_owner) {
            return;
        }
        self.owner = None;
        if !self.is_enabled {
            return;
        }
        self.is_enabled = false;
        self.stop_picture_in_picture(cx);
        self.controller = None;
        self.is_active = false;
        self.feed.set_active(false);
        self.detach_track();
        self.feed.flush();
        self.held_face = None;
        cx.notify();
    }

    /// picture source for the window, a remote screen share beats the remote camera
    /// source_id is the wire identity (user and track kind and sid), not the track key
    /// a key can come back for a new track
    pub fn present(
        &mut self,
        track: Option<String>,
        source_id: Option<String>,
        requested_owner: RiseRTCCallPictureInPictureControllerOwner,
        cx: &mut Context<Self>,
    ) {
        if !self.is_enabled || self.owner != Some(requested_owner) {
            return;
        }
        if self.attached_source_id == source_id && self.attached_track == track {
            return;
        }
        self.detach_track();
        let Some(track) = track else {
            self.feed.flush();
            self.held_face = None;
            cx.notify();
            return;
        };
        self.attached_track = Some(track);
        self.attached_source_id = source_id;
        cx.notify();
    }

    /// source went away while the floating window is open, eg the other side turned the camera off
    /// window stays and shows face until the next present
    /// returns false when nothing floats
    pub fn hold_without_source(
        &mut self,
        requested_owner: RiseRTCCallPictureInPictureControllerOwner,
        face: RiseRTCPictureInPictureFace,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_enabled || self.owner != Some(requested_owner) || self.controller.is_none() {
            return false;
        }
        let changed =
            self.attached_track.is_some() || self.feed.frame.is_some() || self.held_face.as_ref() != Some(&face);
        if changed {
            self.detach_track();
            self.feed.flush();
            self.held_face = Some(face);
            cx.notify();
        }
        true
    }

    pub fn render(&mut self, frame: &RemoteVideoFrame, cx: &mut Context<Self>) {
        let is_attached = self.attached_track.as_deref() == Some(frame.track_key.as_str());
        let is_standby = self.attached_track.is_none()
            && self.widget_standby.as_ref().is_some_and(|standby| standby.track == frame.track_key);
        if !is_attached && !is_standby {
            return;
        }
        if self.feed.render(frame, RiseClock::monotonic()) {
            cx.notify();
        }
    }

    pub fn set_widget_standby(
        &mut self,
        holder: EntityId,
        track: Option<String>,
        source_id: Option<String>,
        face: RiseRTCPictureInPictureFace,
        cx: &mut Context<Self>,
    ) {
        let standby = track.map(|track| RiseRTCPictureInPictureStandby { holder, track, source_id });
        if standby.is_none() && self.widget_standby.as_ref().is_some_and(|current| current.holder != holder) {
            return;
        }
        if self.widget_standby == standby {
            return;
        }
        self.widget_standby = standby.clone();
        let owner = RiseRTCCallPictureInPictureControllerOwner::DirectCallWidget;
        if self.owner != Some(owner) {
            return;
        }
        match standby {
            Some(standby) => self.present(Some(standby.track), standby.source_id, owner, cx),
            None => {
                if !self.hold_without_source(owner, face, cx) {
                    self.disable(owner, cx);
                }
            }
        }
    }

    pub fn release_widget_standby(&mut self, holder: EntityId, cx: &mut Context<Self>) {
        if self.widget_standby.as_ref().is_some_and(|current| current.holder != holder) {
            return;
        }
        self.widget_standby = None;
        let owner = RiseRTCCallPictureInPictureControllerOwner::DirectCallWidget;
        if self.owner == Some(owner) {
            self.disable(owner, cx);
        }
    }

    fn app_background_changed(&mut self, background: bool, cx: &mut Context<Self>) {
        let owner = RiseRTCCallPictureInPictureControllerOwner::DirectCallWidget;
        if !background {
            if self.owner == Some(owner) {
                self.disable(owner, cx);
            }
            return;
        }
        let Some(standby) = self.widget_standby.clone() else {
            return;
        };
        if self.controller.is_some() {
            return;
        }
        if let Some(stale) = self.owner.filter(|current| *current != owner) {
            self.disable(stale, cx);
        }
        self.enable(owner, cx);
        self.present(Some(standby.track), standby.source_id, owner, cx);
        if self.owner != Some(owner) {
            return;
        }
        let main_window = rise_rtc_call_window_hooks()
            .and_then(|hooks| cx.windows().into_iter().find(|window| (hooks.is_main_window)(*window)));
        self.main_window = main_window;
        let display = main_window
            .and_then(|window| window.update(cx, |_, window, cx| window.display(cx)).ok().flatten())
            .or_else(|| cx.primary_display());
        let Some(display) = display else {
            return;
        };
        tracing::warn!(target: "riseonly::X1892", "widget call floats on app background");
        self.start_picture_in_picture(display.visible_bounds(), Some(display.id()), cx);
    }

    fn floating_window_restored(&mut self, cx: &mut Context<Self>) {
        let owner = RiseRTCCallPictureInPictureControllerOwner::DirectCallWidget;
        if self.owner == Some(owner) {
            self.disable(owner, cx);
        }
    }

    /// only a direct call floats by itself, a room stream floats when the person asks for the mini player
    pub fn main_window_activation_changed(&mut self, is_main_active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.owner != Some(RiseRTCCallPictureInPictureControllerOwner::DirectCall) {
            return;
        }
        self.main_window = Some(window.window_handle());
        if is_main_active {
            self.stop_picture_in_picture(cx);
            return;
        }
        if !self.is_enabled || self.controller.is_some() || self.attached_track.is_none() {
            return;
        }
        let Some(display) = window.display(cx) else {
            return;
        };
        self.start_picture_in_picture(display.visible_bounds(), Some(display.id()), cx);
    }

    fn start_picture_in_picture(
        &mut self,
        visible: Bounds<Pixels>,
        display_id: Option<DisplayId>,
        cx: &mut Context<Self>,
    ) {
        self.controller_generation += 1;
        let generation = self.controller_generation;
        let controller = cx.entity();
        let options = gpui::WindowOptions {
            app_id: rise_rtc_call_window_hooks().and_then(|hooks| (hooks.app_id)()),
            ..floating_window_options(visible, display_id)
        };
        cx.defer(move |cx| {
            let opened = cx.open_window(options, {
                let controller = controller.clone();
                move |window, cx| {
                    let _ = keep_call_window_above(window);
                    cx.new(|cx| RiseRTCPictureInPictureWindow::new(controller, generation, visible, window, cx))
                }
            });
            controller.update(cx, |this, cx| match opened {
                Ok(handle) if this.controller_generation == generation => {
                    this.controller = Some(handle);
                    this.picture_in_picture_controller_did_start_picture_in_picture(generation, cx);
                }
                Ok(handle) => {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
                Err(_) => this.picture_in_picture_controller_failed_to_start(generation, cx),
            });
        });
    }

    fn stop_picture_in_picture(&mut self, cx: &mut Context<Self>) {
        let Some(handle) = self.controller.take() else {
            return;
        };
        let generation = self.controller_generation;
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        self.picture_in_picture_controller_did_stop_picture_in_picture(generation, cx);
    }

    fn picture_in_picture_controller_did_start_picture_in_picture(&mut self, generation: u64, cx: &mut Context<Self>) {
        if !self.is_enabled || self.controller_generation != generation {
            return;
        }
        self.is_active = true;
        self.feed.set_active(true);
        cx.notify();
    }

    fn picture_in_picture_controller_did_stop_picture_in_picture(&mut self, generation: u64, cx: &mut Context<Self>) {
        if self.controller_generation != generation {
            return;
        }
        self.is_active = false;
        self.feed.set_active(false);
        cx.notify();
    }

    fn picture_in_picture_controller_failed_to_start(&mut self, generation: u64, cx: &mut Context<Self>) {
        if self.controller_generation != generation {
            return;
        }
        self.controller = None;
        self.is_active = false;
        self.feed.set_active(false);
        cx.notify();
    }

    fn floating_window_closed(&mut self, generation: u64, cx: &mut Context<Self>) {
        if self.controller_generation != generation || self.controller.is_none() {
            return;
        }
        self.controller = None;
        self.picture_in_picture_controller_did_stop_picture_in_picture(generation, cx);
        if self.is_floating_stream() {
            self.end_stream_float(cx);
        }
    }

    /// the second click on the floating window or its open button, back to the call itself
    fn open_call(controller: &Entity<Self>, cx: &mut App) {
        if controller.read(cx).is_floating_stream() {
            controller.update(cx, |controller, cx| controller.end_stream_float(cx));
            if let Some(hooks) = rise_rtc_call_window_hooks() {
                (hooks.present_call)(cx);
            }
            return;
        }
        let main_window = controller.read(cx).main_window;
        Self::restore_main_window(main_window, cx);
        let controller = controller.clone();
        cx.defer(move |cx| {
            controller.update(cx, |controller, cx| controller.floating_window_restored(cx));
        });
    }

    fn close_floating(controller: &Entity<Self>, cx: &mut App) {
        controller.update(cx, |controller, cx| {
            if controller.is_floating_stream() {
                controller.end_stream_float(cx);
            } else if let Some(owner) = controller.owner {
                controller.disable(owner, cx);
            }
        });
    }

    fn toggle_stream_sound(&self) {
        let Some(user_id) = self.stream_user_id.as_deref() else {
            return;
        };
        let media = RiseRTCLiveKitMediaController::shared();
        let muted = media.stream_volume(user_id) <= f64::EPSILON;
        media.set_stream_volume(user_id, if muted { 1.0 } else { 0.0 });
    }

    fn restore_main_window(main_window: Option<AnyWindowHandle>, cx: &mut App) {
        if let Some(main_window) = main_window {
            let _ = main_window.update(cx, |_, window, _| {
                let _ = rise_platform::window_minimise::show_window(&*window);
                window.activate_window();
                #[cfg(feature = "dev-drive")]
                if rise_platform::window_minimise::minimised_for_dev_drive(&*window) == Some(true) {
                    let _ = rise_platform::window_minimise::report_for_dev_drive(&*window, false);
                }
            });
        }
    }

    fn may_claim(&self, candidate: RiseRTCCallPictureInPictureControllerOwner) -> bool {
        let Some(owner) = self.owner else {
            return true;
        };
        owner == candidate || candidate.rank() > owner.rank()
    }

    fn detach_track(&mut self) {
        if self.attached_track.is_none() {
            return;
        }
        self.attached_track = None;
        self.attached_source_id = None;
    }
}

/// last picture of the attached track for the floating window
/// desktop seam already prepares and turns every remote picture off the main thread
/// so this only keeps the newest one
struct RiseRTCPictureInPictureFeed {
    is_active: bool,
    last_accepted_at: Option<Duration>,
    frame: Option<RiseRTCPictureInPictureFrame>,
}

#[derive(Clone)]
struct RiseRTCPictureInPictureFrame {
    picture: PreparedHostFrame,
    rotation_degrees: u16,
}

impl RiseRTCPictureInPictureFeed {
    const WARM_FRAME_INTERVAL: Duration = Duration::from_millis(333);

    fn new() -> Self {
        Self { is_active: false, last_accepted_at: None, frame: None }
    }

    fn set_active(&mut self, active: bool) {
        self.is_active = active;
    }

    fn flush(&mut self) {
        self.frame = None;
    }

    fn render(&mut self, frame: &RemoteVideoFrame, now: Duration) -> bool {
        if !self.is_active
            && self
                .last_accepted_at
                .is_some_and(|accepted| now.saturating_sub(accepted) < Self::WARM_FRAME_INTERVAL)
        {
            return false;
        }
        self.last_accepted_at = Some(now);
        self.frame = Some(RiseRTCPictureInPictureFrame {
            picture: frame.frame.clone(),
            rotation_degrees: frame.rotation_degrees,
        });
        self.is_active
    }
}

/// view of the floating window
/// newest picture aspect fit on black, moved by its body and resized by its corners
/// the first click shows its buttons, the next one opens the call
pub struct RiseRTCPictureInPictureWindow {
    controller: Entity<RiseRTCCallPictureInPictureController>,
    is_moving: bool,
    is_armed: bool,
    press_origin: Option<Point<Pixels>>,
    controls_visible: bool,
    controls_hide: Option<Task<()>>,
    resize: Option<(FloatingResizeCorner, Bounds<Pixels>)>,
    _controller_changes: Subscription,
    _window_bounds_changes: Subscription,
}

impl RiseRTCPictureInPictureWindow {
    fn new(
        controller: Entity<RiseRTCCallPictureInPictureController>,
        generation: u64,
        visible: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let controller_changes = cx.observe(&controller, |_, _, cx| cx.notify());
        let closing = controller.clone();
        window.on_window_should_close(cx, move |window, cx| {
            remember_floating_corner(visible, window.bounds());
            closing.update(cx, |controller, cx| controller.floating_window_closed(generation, cx));
            true
        });
        let window_bounds_changes = cx.observe_window_bounds(window, move |view: &mut Self, window, _| {
            let bounds = window.bounds();
            if view.press_origin.is_some_and(|origin| origin != bounds.origin) {
                remember_floating_corner(visible, bounds);
            }
        });
        Self {
            controller,
            is_moving: false,
            is_armed: false,
            press_origin: None,
            controls_visible: false,
            controls_hide: None,
            resize: None,
            _controller_changes: controller_changes,
            _window_bounds_changes: window_bounds_changes,
        }
    }

    fn show_controls(&mut self, cx: &mut Context<Self>) {
        self.controls_visible = true;
        self.controls_hide = Some(cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(CONTROLS_VISIBLE_FOR).await;
            let _ = view.update(cx, |view, cx| {
                view.controls_visible = false;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn control(
        id: &'static str,
        symbol: &'static str,
        label: String,
        action: impl Fn(&mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .size(px(CONTROL_SIDE))
            .rounded_full()
            .bg(white().opacity(0.18))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .tooltip_label(label)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                action(cx);
            })
            .children(IconUi::sized(symbol, px(15.0), white(), true))
    }

    fn controls(&self, cx: &App) -> impl IntoElement + use<> {
        let controller = self.controller.read(cx);
        let open = self.controller.clone();
        let close = self.controller.clone();
        let sound = controller.stream_user_id.as_deref().map(|user_id| {
            let muted = RiseRTCLiveKitMediaController::shared().stream_volume(user_id) <= f64::EPSILON;
            let toggled = self.controller.clone();
            Self::control(
                "rtc-picture-in-picture-sound",
                if muted { "speaker.slash.fill" } else { "speaker.wave.2.fill" },
                tr(if muted { "feed_sound_unmute" } else { "feed_sound_mute" }),
                move |cx| {
                    toggled.read(cx).toggle_stream_sound();
                    toggled.update(cx, |_, cx| cx.notify());
                },
            )
        });
        div()
            .absolute()
            .inset_0()
            .bg(black().opacity(0.35))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .child(Self::control(
                "rtc-picture-in-picture-open",
                "arrow.up.left.and.arrow.down.right",
                tr("desktop_rtc_mini_player_open_call"),
                move |cx| RiseRTCCallPictureInPictureController::open_call(&open, cx),
            ))
            .children(sound)
            .child(Self::control("rtc-picture-in-picture-close", "xmark", tr("close"), move |cx| {
                RiseRTCCallPictureInPictureController::close_floating(&close, cx)
            }))
    }

    fn corner_grip(&self, corner: FloatingResizeCorner, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let side = px(CORNER_GRIP_SIDE);
        let cursor = if corner.is_left() == corner.is_top() {
            CursorStyle::ResizeUpLeftDownRight
        } else {
            CursorStyle::ResizeUpRightDownLeft
        };
        let grip = div().id(SharedString::from(format!("rtc-picture-in-picture-grip-{corner:?}"))).absolute().size(side);
        let grip = if corner.is_top() { grip.top_0() } else { grip.bottom_0() };
        let grip = if corner.is_left() { grip.left_0() } else { grip.right_0() };
        grip.cursor(cursor).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |view, _, window, cx| {
                cx.stop_propagation();
                view.is_armed = false;
                if start_call_window_resize(window, corner) == CallWindowResize::App {
                    view.resize = Some((corner, window.bounds()));
                    cx.notify();
                }
            }),
        )
    }

    /// ends an app driven resize, on release or on a move with the button already up
    fn end_resize(&mut self, cx: &mut Context<Self>) {
        if self.resize.take().is_some() {
            cx.notify();
        }
    }

    fn resize_driver(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (corner, start) = self.resize?;
        let view = cx.entity().downgrade();
        Some(
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let lost = view.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture {
                            return;
                        }
                        if event.pressed_button != Some(MouseButton::Left) {
                            let _ = lost.update(cx, |view, cx| view.end_resize(cx));
                            return;
                        }
                        let pointer = window.bounds().origin + event.position;
                        let _ = set_call_window_frame(window, corner.dragged_frame(start, pointer));
                    });
                    let released = view.clone();
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                        if phase != DispatchPhase::Capture {
                            return;
                        }
                        let _ = released.update(cx, |view, cx| view.end_resize(cx));
                    });
                },
            )
            .absolute()
            .size_0(),
        )
    }
}

impl Render for RiseRTCPictureInPictureWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let controller = self.controller.read(cx);
        let frame = controller.feed.frame.clone();
        let picture =
            frame.and_then(|frame| present_host_frame_rotated(&frame.picture, frame.rotation_degrees).ok());
        let face = if picture.is_none() { controller.held_face.clone() } else { None };
        let face = face.map(|face| {
            vstack(8.0)
                .w_full()
                .px(px(16.0))
                .child(
                    UserLogoUi::new("rtc-picture-in-picture-face")
                        .source(face.avatar_url.map(SharedString::from))
                        .name(SharedString::from(face.name.clone()))
                        .user_id(face.user_id.map(SharedString::from))
                        .size(96.0)
                        .show_border(false)
                        .show_story_ring(false)
                        .image_cache_policy(CleverImageCachePolicy::MemoryOnly),
                )
                .child(
                    MainText::new(face.name)
                        .w_full()
                        .px(15.0)
                        .tac(TextAlignT::Center)
                        .color(gpui::white())
                        .number_of_lines(1),
                )
        });
        let controls = self.controls_visible.then(|| self.controls(cx));
        let grips: Vec<_> = FloatingResizeCorner::ALL.into_iter().map(|corner| self.corner_grip(corner, cx)).collect();
        let resize_driver = self.resize_driver(cx);

        div()
            .id("rtc-picture-in-picture")
            .relative()
            .size_full()
            .bg(black())
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    view.end_resize(cx);
                    view.is_moving = false;
                    view.is_armed = true;
                    view.press_origin = Some(window.bounds().origin);
                }),
            )
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, window, _| {
                if event.pressed_button == Some(MouseButton::Left)
                    && view.is_armed
                    && !view.is_moving
                    && view.resize.is_none()
                {
                    view.is_moving = true;
                    let _ = start_call_window_move(window);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    view.is_moving = false;
                    view.is_armed = false;
                    view.end_resize(cx);
                }),
            )
            .on_click(cx.listener(|view, _: &ClickEvent, window, cx| {
                if view.is_moving || view.resize.is_some() || view.press_origin != Some(window.bounds().origin) {
                    return;
                }
                if !view.controls_visible {
                    view.show_controls(cx);
                    return;
                }
                view.controls_visible = false;
                view.controls_hide = None;
                let controller = view.controller.clone();
                cx.defer(move |cx| RiseRTCCallPictureInPictureController::open_call(&controller, cx));
            }))
            .children(picture)
            .children(face)
            .children(controls)
            .children(grips)
            .children(resize_driver)
    }
}

/// mounted by the call screens, only watches main window activation
/// keyed state so a second host reuses it
#[derive(IntoElement)]
pub struct RiseRTCPictureInPictureLayerHost;

struct RiseRTCPictureInPictureLayerWatch {
    _activation: Subscription,
}

impl RenderOnce for RiseRTCPictureInPictureLayerHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let controller = RiseRTCCallPictureInPictureController::shared(cx);
        let _watch = window.use_keyed_state("rtc-picture-in-picture-layer-host", cx, move |window, cx| {
            let activation = cx.observe_window_activation(window, move |_, window, cx| {
                let is_main_active = window.is_window_active();
                controller.update(cx, |controller, cx| {
                    controller.main_window_activation_changed(is_main_active, window, cx);
                });
            });
            RiseRTCPictureInPictureLayerWatch { _activation: activation }
        });
        div().size_0()
    }
}
