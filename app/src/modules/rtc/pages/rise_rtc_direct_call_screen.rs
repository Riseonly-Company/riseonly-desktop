use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, Context, ElementId, Empty, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, Render, Rgba,
    SharedString, Size, Subscription, Task, WeakEntity, Window, black, size, white,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_rtc::{CallState, CameraPosition, ConnectionState, RoomMode, ScreenShareQuality};
use rise_theme::AppTheme;
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::BottomSheetMeasuredFractionModifiers;
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::expandable_text_ui::EdgeInsets;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape, LiquidGlassMenuLabel, LiquidPressSettlesAtOnceKey};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::rise_symbol::RiseSymbol;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::linear_gradient::{GradientStop, LinearGradient};
use rise_ui::swiftui::motion::{MotionSpec, ease_in_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::text::TruncationMode;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition, UnitPoint};
use rise_ui::{IconUi, theme};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::sai::Sai;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::core::widgets::overlay::premium_upsell_host::PremiumUpsellSheet;
use crate::core::widgets::overlay::premium_upsell_store::{PremiumUpsellInteractionsStore, PremiumUpsellStore};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::stores::guides_services::guides_services::GuidesServicesStore;
use crate::modules::rtc::pages::rise_rtc_room_screen::RiseRTCRoomScreenView;
use crate::modules::rtc::components::rise_rtc_floating_video_window::{
    RiseRTCFloatingVideoWindow, RiseRTCFloatingVideoWindowModifiers, RiseRTCVideoSurface, VideoViewLayoutMode,
    VideoViewMirrorMode,
};
use crate::modules::rtc::components::rise_rtc_own_screen_share_wall::RiseRTCOwnScreenShareWall;
use crate::modules::rtc::components::rise_rtc_speaking_ring::RiseRTCSpeakingRing;
use crate::modules::rtc::components::rise_rtc_zoomable_video_surface::RiseRTCZoomableVideoSurface;
use crate::modules::rtc::engine::rise_rtc_call_trace::{RiseRTCCallTrace, RiseRTCCallTraceControl};
use crate::modules::rtc::engine::rise_rtc_connection_wait_policy::RiseRTCConnectionWaitPolicy;
use crate::modules::rtc::engine::rise_rtc_domain::{RiseRTCDomain, RiseRTCHangupAction};
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCCallStateExt as _, RiseRTCDirectCall, RiseRTCParticipant, RiseRTCStateError, RiseRTCUserProfile,
};
use crate::modules::rtc::engine::rise_rtc_presentation::RiseRTCRoomPresentation;
use crate::modules::rtc::media::rise_rtc_audio_settings::{RiseMicrophoneTestHush, RiseRTCAudioSettingsContent};
use crate::modules::rtc::media::rise_rtc_call_picture_in_picture::{
    RiseRTCCallPictureInPictureController, RiseRTCCallPictureInPictureControllerOwner, RiseRTCPictureInPictureFace,
    RiseRTCPictureInPictureLayerHost,
};
use crate::modules::rtc::media::rise_rtc_end_tone_player::RiseRTCEndTonePlayer;
use crate::modules::rtc::media::rise_rtc_live_kit_audio_coordinator::RiseRTCLiveKitAudioCoordinator;
use crate::modules::rtc::media::rise_rtc_media_presentation::{RiseRTCMediaPresentation, VideoTrack};
use crate::modules::rtc::shared::rise_rtc_device_refusal::RiseRTCDeviceRefusal;
use crate::modules::rtc::shared::rise_rtc_screen_safe_area::RiseRTCScreenSafeArea;
use crate::modules::rtc::shared::rise_rtc_settings_accent::RiseRTCSettingsAccent;
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;
use crate::modules::rtc::system::rise_rtc_system_call_presentation::{
    RiseRTCSystemCallPresentation, RiseRTCSystemCallPresentationStore,
};
use crate::modules::rtc::system::rise_rtc_system_models::{
    RiseRTCSystemCallDirection, RiseRTCSystemCallPhase, RiseRTCSystemCallTerminalOutcome,
};
use crate::modules::user::shared::contact_device_book::RiseInstallationIdentity;

/// how long the call screen stays after the call ends
/// an outcome the user didnt choose stays long enough to
/// be read, direct screen and its host share these values
pub enum RiseRTCTerminalHold {}

impl RiseRTCTerminalHold {
    pub const CHOSEN: Duration = Duration::from_millis(760);
    pub const CANCELLED: Duration = Duration::from_millis(1400);
    pub const NOT_CHOSEN: Duration = Duration::from_millis(2600);
    pub const FAILURE: Duration = Duration::from_millis(3200);
}

const REMOTE_SURFACE_ARRIVAL_WINDOW: Duration = Duration::from_secs(15);

const SILENCED_NOTICE_HEIGHT: f32 = 30.0;

const INSTANT_MEDIA_ACTION_IDS: [&str; 2] = ["microphone", "headphones"];

const ESCAPE_KEY: &str = "escape";
const MICROPHONE_SHORTCUT_KEY: &str = "m";
const CAMERA_SHORTCUT_KEY: &str = "v";
const SCREEN_SHARE_SHORTCUT_KEY: &str = "e";

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

fn interactions(cx: &mut App) -> Entity<RTCInteractionsStore> {
    RTCInteractionsStore::shared(cx)
}

fn normalized(value: Option<&str>) -> Option<String> {
    let value = trimming_whitespaces_and_newlines(value?);
    (!value.is_empty()).then(|| value.to_owned())
}

fn connected_green() -> Hsla {
    Rgba { r: 0.45, g: 1.0, b: 0.62, a: 1.0 }.into()
}

fn symbol(name: &str, side: f32, color: Hsla) -> Option<gpui::Svg> {
    IconUi::sized(name, px(side), color, name.ends_with(".fill"))
}

/// gpui fires every hovered click handler so a control over the picture must occlude the zoom surface
/// otherwise one press also toggles the chrome
/// icon buttons name themselves on hover here
fn glass_control(id: ElementId, label: impl Into<SharedString>, control: impl IntoElement) -> impl IntoElement {
    div().id(id).flex_none().occlude().tooltip_label(label).child(control)
}

#[derive(Clone, Copy, Default, PartialEq)]
struct PendingMediaToggles {
    microphone: Option<bool>,
    camera: Option<bool>,
    screen_share: Option<bool>,
    deafened: Option<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum RemoteSurfacePlaceholder {
    ScreenShare,
    Camera,
}

impl RemoteSurfacePlaceholder {
    fn system_image(self) -> &'static str {
        match self {
            Self::ScreenShare => "display",
            Self::Camera => "video.fill",
        }
    }

    fn identity(self) -> &'static str {
        match self {
            Self::ScreenShare => "screen",
            Self::Camera => "camera",
        }
    }

    fn title(self) -> String {
        match self {
            Self::ScreenShare => tr("rtc_screen_share_loading"),
            Self::Camera => tr("rtc_video_loading"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum VisualState {
    Request,
    Ringing,
    Incoming,
    Connecting,
    Connected(Date),
    Reconnecting,
    Ending,
    Ended,
    Declined,
    Cancelled,
    TimedOut,
    Failed,
    AwaitingPeer,
}

impl VisualState {
    fn is_connected(self) -> bool {
        matches!(self, Self::Connected(_))
    }

    fn is_terminal(self) -> bool {
        matches!(self, Self::Ended | Self::Declined | Self::Cancelled | Self::TimedOut | Self::Failed)
    }

    fn terminal_hold(self) -> Duration {
        match self {
            Self::Failed => RiseRTCTerminalHold::FAILURE,
            Self::Declined | Self::TimedOut => RiseRTCTerminalHold::NOT_CHOSEN,
            Self::Cancelled => RiseRTCTerminalHold::CANCELLED,
            Self::Request
            | Self::Ringing
            | Self::Incoming
            | Self::Connecting
            | Self::Connected(_)
            | Self::Reconnecting
            | Self::Ending
            | Self::Ended
            | Self::AwaitingPeer => RiseRTCTerminalHold::CHOSEN,
        }
    }

    fn task_identity(self) -> String {
        match self {
            Self::Request => "request".to_owned(),
            Self::Ringing => "ringing".to_owned(),
            Self::Incoming => "incoming".to_owned(),
            Self::Connecting => "connecting".to_owned(),
            Self::Connected(date) => format!("connected-{}", date.time_interval_since_1970()),
            Self::Reconnecting => "reconnecting".to_owned(),
            Self::Ending => "ending".to_owned(),
            Self::Ended => "ended".to_owned(),
            Self::Declined => "declined".to_owned(),
            Self::Cancelled => "cancelled".to_owned(),
            Self::TimedOut => "timed-out".to_owned(),
            Self::Failed => "failed".to_owned(),
            Self::AwaitingPeer => "awaiting-peer".to_owned(),
        }
    }
}

/// template split around {name} so only the name truncates and the action stays whole
#[derive(Clone, PartialEq)]
struct SilencedNotice {
    lead: String,
    name: Option<String>,
    tail: String,
}

impl SilencedNotice {
    fn new(template: String, name: Option<String>) -> Self {
        const NAME_PLACEHOLDER: &str = "{name}";
        if let Some(name) = name
            && let Some((lead, tail)) = template.split_once(NAME_PLACEHOLDER)
        {
            return Self { lead: lead.to_owned(), name: Some(name), tail: tail.to_owned() };
        }
        Self { lead: template, name: None, tail: String::new() }
    }
}

/// what a guide step can wait on here, folded into one observed value
#[derive(Clone, Copy, Default, PartialEq)]
struct GuideCallFacts {
    connected: bool,
    camera_on: bool,
    screen_share_on: bool,
    windows_swapped: bool,
    stream_settings_open: bool,
    encryption_open: bool,
}

#[derive(Clone)]
struct LocalEndPresentation {
    identity: String,
    account_id: String,
    account_epoch: Option<String>,
    state: VisualState,
}

#[derive(Clone, Copy, PartialEq)]
struct PrimarySurface {
    surface: RemoteSurfacePlaceholder,
    is_local: bool,
}

/// one if of floating windows with its transition, a slot leaves with the value it had
struct FloatingWindowSlot {
    shown: Option<RemoteSurfacePlaceholder>,
    presence: Presence<RemoteSurfacePlaceholder>,
}

impl FloatingWindowSlot {
    fn new() -> Self {
        let mut presence = Presence::new(None);
        presence.set_transition(Transition::scale(0.9).combined(Transition::opacity()));
        Self { shown: None, presence }
    }

    fn sync(&mut self, target: Option<RemoteSurfacePlaceholder>, motion: Option<MotionSpec>, now: Duration, cx: &App) {
        if self.shown != target {
            self.shown = target;
            self.presence.set_in(target, motion, now, cx);
        }
    }
}

type ScreenCallback = Rc<dyn Fn(&mut Window, &mut App)>;

/// phases of a personal call
/// the call rides in its own dark window on desktop, the host
/// decides where this is mounted and closing it only minimizes
#[derive(IntoElement)]
pub struct RiseRTCDirectCallScreen {
    id: ElementId,
    preferred_call_uuid: Option<Uuid>,
    server_call_id: Option<String>,
    on_minimize: ScreenCallback,
    on_terminal_settled: ScreenCallback,
}

impl RiseRTCDirectCallScreen {
    pub fn new(
        preferred_call_uuid: Option<Uuid>,
        server_call_id: Option<String>,
        on_minimize: impl Fn(&mut Window, &mut App) + 'static,
        on_terminal_settled: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        let identity = preferred_call_uuid.map(|uuid| uuid.uuid_string()).unwrap_or_default();
        Self {
            id: ElementId::Name(format!("rtc-direct-call-{identity}").into()),
            preferred_call_uuid,
            server_call_id,
            on_minimize: Rc::new(on_minimize),
            on_terminal_settled: Rc::new(on_terminal_settled),
        }
    }
}

impl RenderOnce for RiseRTCDirectCallScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        let state = window.use_keyed_state(child_id(&id, "state"), cx, |_, cx| RiseRTCDirectCallScreenState::new(id, cx));
        state.update(cx, |state, _| {
            state.preferred_call_uuid = self.preferred_call_uuid;
            state.server_call_id = self.server_call_id;
            state.on_minimize = Some(self.on_minimize);
            state.on_terminal_settled = Some(self.on_terminal_settled);
        });
        div().size_full().child(state).on_disappear(child_id(&self.id, "disappear"), |_, cx| {
            RiseRTCCallPictureInPictureController::shared(cx).update(cx, |controller, cx| {
                controller.disable(RiseRTCCallPictureInPictureControllerOwner::DirectCall, cx);
            });
            RiseRTCDirectCallScreenState::report_guide_facts(GuideCallFacts::default(), cx);
        })
    }
}

/// state of the screen, observers live here so it
/// reacts to the stores and not to the parent frames
pub struct RiseRTCDirectCallScreenState {
    id: ElementId,
    this: WeakEntity<Self>,
    window_handle: Option<AnyWindowHandle>,
    focus_handle: FocusHandle,
    preferred_call_uuid: Option<Uuid>,
    server_call_id: Option<String>,
    on_minimize: Option<ScreenCallback>,
    on_terminal_settled: Option<ScreenCallback>,
    auth: Entity<AuthServicesStore>,
    upsell: Entity<PremiumUpsellStore>,
    rtc: Entity<RTCServicesStore>,
    media: Entity<RiseRTCMediaPresentation>,
    system_calls: Entity<RiseRTCSystemCallPresentationStore>,
    audio: Entity<RiseRTCLiveKitAudioCoordinator>,
    socket: Entity<SaiWebSocketManager>,
    pending_media: PendingMediaToggles,
    media_action_ids: HashSet<String>,
    audio_routes_visible: bool,
    audio_inputs_visible: bool,
    stream_settings_visible: bool,
    encryption_info_visible: bool,
    audio_route_sheet_fraction: f32,
    audio_input_sheet_fraction: f32,
    stream_settings_sheet_fraction: f32,
    encryption_sheet_fraction: f32,
    stream_quality: ScreenShareQuality,
    stream_includes_audio: bool,
    pending_screen_share_intent: Option<bool>,
    is_ending: bool,
    microphone_shown_at_hang_up: Option<bool>,
    local_end_presentation: Option<LocalEndPresentation>,
    recording_busy: bool,
    is_answering: bool,
    terminal_dismiss_scheduled: bool,
    terminal_hold_owned_by_state: bool,
    terminal_hold_task: Option<Task<()>>,
    terminal_state_task: Option<Task<()>>,
    peer_first_joined_at: Option<Date>,
    remote_media_swapped: bool,
    selected_local_surface: Option<RemoteSurfacePlaceholder>,
    remote_window_position: UnitPoint,
    local_window_position: UnitPoint,
    local_screen_window_position: UnitPoint,
    abandoned_remote_surfaces: HashSet<RemoteSurfacePlaceholder>,
    top_safe_inset: f32,
    chrome_hidden: bool,
    chrome_animation: MotionSpec,
    microphone_refused: bool,
    transaction_motion: Option<MotionSpec>,
    geometry_size: Size<f32>,
    controls_height: f32,
    media_fingerprint: String,
    primary_surface_shown: Option<PrimarySurface>,
    primary_surface_presence: Presence<PrimarySurface>,
    readability_gradient_opacity: Animated<f32>,
    silenced_notice_shown: Option<SilencedNotice>,
    silenced_notice_presence: Presence<SilencedNotice>,
    remote_window_slot: FloatingWindowSlot,
    displaced_screen_window_slot: FloatingWindowSlot,
    local_screen_window_slot: FloatingWindowSlot,
    displaced_camera_window_slot: FloatingWindowSlot,
    local_camera_window_slot: FloatingWindowSlot,
    _subscriptions: Vec<Subscription>,
}

impl RiseRTCDirectCallScreenState {
    fn new(id: ElementId, cx: &mut Context<Self>) -> Self {
        let auth = AuthServicesStore::shared(cx);
        let upsell = PremiumUpsellStore::shared(cx);
        let rtc = RTCServicesStore::shared(cx);
        let media = RiseRTCMediaPresentation::shared(cx);
        let system_calls = RiseRTCSystemCallPresentationStore::shared(cx);
        let audio = RiseRTCLiveKitAudioCoordinator::shared(cx);
        let socket = SaiWebSocketManager::shared(cx);
        let subscriptions = vec![
            cx.observe(&auth, |_, _, cx| cx.notify()),
            cx.observe(&upsell, |_, _, cx| cx.notify()),
            cx.observe(&rtc, |_, _, cx| cx.notify()),
            cx.observe(&system_calls, |_, _, cx| cx.notify()),
            cx.observe(&audio, |_, _, cx| cx.notify()),
            cx.observe(&socket, |_, _, cx| cx.notify()),
            cx.observe_global::<RiseMicrophoneTestHush>(|_, cx| cx.notify()),
            cx.observe(&media, |this, _, cx| {
                let fingerprint = this.current_media_fingerprint(cx);
                if fingerprint != this.media_fingerprint {
                    this.media_fingerprint = fingerprint;
                    cx.notify();
                }
            }),
        ];
        let mut primary_surface_presence = Presence::new(None);
        primary_surface_presence.set_transition(Transition::opacity());
        let mut silenced_notice_presence = Presence::new(None);
        silenced_notice_presence.set_transition(Transition::opacity().combined(Transition::scale(0.94)));
        Self {
            id,
            this: cx.weak_entity(),
            window_handle: None,
            focus_handle: cx.focus_handle(),
            preferred_call_uuid: None,
            server_call_id: None,
            on_minimize: None,
            on_terminal_settled: None,
            auth,
            upsell,
            rtc,
            media,
            system_calls,
            audio,
            socket,
            pending_media: PendingMediaToggles::default(),
            media_action_ids: HashSet::new(),
            audio_routes_visible: false,
            audio_inputs_visible: false,
            stream_settings_visible: false,
            encryption_info_visible: false,
            audio_route_sheet_fraction: 0.34,
            audio_input_sheet_fraction: 0.55,
            stream_settings_sheet_fraction: 0.5,
            encryption_sheet_fraction: 0.46,
            stream_quality: ScreenShareQuality::Balanced,
            stream_includes_audio: false,
            pending_screen_share_intent: None,
            is_ending: false,
            microphone_shown_at_hang_up: None,
            local_end_presentation: None,
            recording_busy: false,
            is_answering: false,
            terminal_dismiss_scheduled: false,
            terminal_hold_owned_by_state: false,
            terminal_hold_task: None,
            terminal_state_task: None,
            peer_first_joined_at: None,
            remote_media_swapped: false,
            selected_local_surface: None,
            remote_window_position: UnitPoint::TOP_TRAILING,
            local_window_position: UnitPoint::BOTTOM_TRAILING,
            local_screen_window_position: UnitPoint::BOTTOM_LEADING,
            abandoned_remote_surfaces: HashSet::new(),
            top_safe_inset: 0.0,
            chrome_hidden: false,
            chrome_animation: ease_in_out(0.26),
            microphone_refused: false,
            transaction_motion: None,
            geometry_size: Size::default(),
            controls_height: 0.0,
            media_fingerprint: String::new(),
            primary_surface_shown: None,
            primary_surface_presence,
            readability_gradient_opacity: Animated::new(1.0),
            silenced_notice_shown: None,
            silenced_notice_presence,
            remote_window_slot: FloatingWindowSlot::new(),
            displaced_screen_window_slot: FloatingWindowSlot::new(),
            local_screen_window_slot: FloatingWindowSlot::new(),
            displaced_camera_window_slot: FloatingWindowSlot::new(),
            local_camera_window_slot: FloatingWindowSlot::new(),
            _subscriptions: subscriptions,
        }
    }

    fn scoped_id(&self, name: impl Into<SharedString>) -> ElementId {
        child_id(&self.id, name)
    }

    /// press handler bound to this state, a press after the screen went away does nothing
    fn action(
        this: WeakEntity<Self>,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl Fn(&mut Window, &mut App) + 'static {
        move |_, cx| {
            this.update(cx, |this, cx| action(this, cx)).ok();
        }
    }

    fn with_animation(&mut self, motion: MotionSpec) {
        self.transaction_motion = Some(motion);
    }

    fn minimize_action(this: WeakEntity<Self>) -> impl Fn(&mut Window, &mut App) + 'static {
        move |window, cx| {
            let Ok(Some(on_minimize)) = this.read_with(cx, |this, _| this.on_minimize.clone()) else {
                return;
            };
            on_minimize(window, cx);
        }
    }

    fn system_call(&self, cx: &App) -> Option<RiseRTCSystemCallPresentation> {
        let calls = self.system_calls.read(cx).calls();
        if let Some(preferred_call_uuid) = self.preferred_call_uuid
            && let Some(call) = calls.iter().find(|call| call.local_uuid == preferred_call_uuid)
        {
            return Some(call.clone());
        }
        let resolved_server_call_id = normalized(self.server_call_id.as_deref())?;
        calls.iter().find(|call| call.server_call_id.as_deref() == Some(resolved_server_call_id.as_str())).cloned()
    }

    fn resolved_server_call_id(&self, cx: &App) -> Option<String> {
        normalized(self.system_call(cx).and_then(|call| call.server_call_id).as_deref())
            .or_else(|| normalized(self.server_call_id.as_deref()))
    }

    fn call(&self, cx: &App) -> Option<RiseRTCDirectCall> {
        let resolved_server_call_id = self.resolved_server_call_id(cx)?;
        self.rtc.read(cx).call(&resolved_server_call_id, cx)
    }

    fn room<'a>(&self, cx: &'a App) -> Option<&'a RiseRTCRoomPresentation> {
        let active_room = self.rtc.read(cx).active_room(cx)?;
        if active_room.config.mode != RoomMode::DirectCall {
            return None;
        }
        match self.call(cx) {
            Some(call) => (active_room.config.room_id == call.room_id).then_some(active_room),
            None => Some(active_room),
        }
    }

    fn current_user_id(&self, cx: &App) -> String {
        self.auth.read(cx).effective_user_id_for_wire().unwrap_or_default()
    }

    fn counterpart_user_id(&self, cx: &App) -> Option<String> {
        if let Some(value) = normalized(self.system_call(cx).and_then(|call| call.remote_user_id).as_deref()) {
            return Some(value);
        }
        let call = self.call(cx)?;
        Some(if call.caller_user_id == self.current_user_id(cx) { call.callee_user_id } else { call.caller_user_id })
    }

    fn counterpart_participant<'a>(&self, cx: &'a App) -> Option<&'a RiseRTCParticipant> {
        let counterpart_user_id = self.counterpart_user_id(cx)?;
        self.room(cx)?.participants.iter().find(|participant| participant.user_id == counterpart_user_id)
    }

    fn counterpart_profile(&self, cx: &App) -> Option<RiseRTCUserProfile> {
        self.counterpart_participant(cx)
            .and_then(|participant| participant.profile.clone())
            .or_else(|| self.rtc.read(cx).incoming_call(cx).and_then(|incoming| incoming.caller.clone()))
            .or_else(|| {
                let call_id = self.resolved_server_call_id(cx)?;
                self.rtc.read(cx).practice_call_counterpart(&call_id, cx)
            })
    }

    fn counterpart_name(&self, cx: &App) -> String {
        let profile = self.counterpart_profile(cx);
        normalized(profile.as_ref().map(|profile| profile.name.as_str()))
            .or_else(|| normalized(profile.as_ref().map(|profile| profile.tag.as_str())))
            .or_else(|| normalized(self.system_call(cx).map(|call| call.display_name).as_deref()))
            .unwrap_or_else(|| tr("rtc_direct_call"))
    }

    fn counterpart_avatar_url(&self, cx: &App) -> Option<String> {
        let profile = self.counterpart_profile(cx);
        normalized(profile.as_ref().map(|profile| profile.avatar_url.as_str()))
            .or_else(|| normalized(self.system_call(cx).and_then(|call| call.avatar_url).as_deref()))
    }

    fn self_participant<'a>(&self, cx: &'a App) -> Option<&'a RiseRTCParticipant> {
        let room = self.room(cx)?;
        room.self_participant.as_ref().or_else(|| {
            let current_user_id = self.current_user_id(cx);
            room.participants.iter().find(|participant| participant.user_id == current_user_id)
        })
    }

    fn displayed_microphone_enabled(&self, cx: &App) -> bool {
        if let Some(shown) = self.microphone_shown_at_hang_up
            && (self.is_ending || self.visual_state(cx).is_terminal())
        {
            return shown;
        }
        self.pending_media
            .microphone
            .or_else(|| self.self_participant(cx).map(|participant| participant.microphone_enabled))
            .unwrap_or(true)
    }

    fn displayed_camera_enabled(&self, cx: &App) -> bool {
        self.pending_media
            .camera
            .or_else(|| self.self_participant(cx).map(|participant| participant.camera_enabled))
            .unwrap_or_else(|| self.system_call(cx).is_some_and(|call| call.has_video))
    }

    fn displayed_screen_share_enabled(&self, cx: &App) -> bool {
        self.pending_media
            .screen_share
            .or_else(|| self.self_participant(cx).map(|participant| participant.screen_share_enabled))
            .unwrap_or(false)
    }

    fn displayed_deafened(&self, cx: &App) -> bool {
        self.pending_media.deafened.unwrap_or_else(|| self.self_participant(cx).is_some_and(|p| p.deafened))
    }

    /// who to name in the banner and what they silenced, own state wins over the peers
    fn silenced_notice(&self, cx: &App) -> Option<SilencedNotice> {
        if self.is_ending || self.visual_state(cx).is_terminal() {
            return None;
        }
        if self.displayed_deafened(cx) {
            return Some(SilencedNotice::new(tr("rtc_self_deafened"), None));
        }
        if !self.displayed_microphone_enabled(cx) {
            let template =
                if self.microphone_refused { tr("rtc_media_microphone_failed") } else { tr("rtc_self_muted_microphone") };
            return Some(SilencedNotice::new(template, None));
        }
        let counterpart = self.counterpart_participant(cx)?;
        if !self.counterpart_is_present(cx) {
            return None;
        }
        if counterpart.deafened {
            return Some(SilencedNotice::new(tr("rtc_peer_deafened"), Some(self.counterpart_name(cx))));
        }
        if !counterpart.microphone_enabled {
            return Some(SilencedNotice::new(tr("rtc_peer_muted_microphone"), Some(self.counterpart_name(cx))));
        }
        None
    }

    fn media_ready(&self, cx: &App) -> bool {
        let connection_state = self.rtc.read(cx).connection_state(cx);
        self.self_participant(cx).is_some()
            && connection_state != ConnectionState::Idle
            && connection_state != ConnectionState::Connecting
            && connection_state != ConnectionState::Failed
    }

    fn remote_screen_track(&self, cx: &App) -> Option<Rc<VideoTrack>> {
        self.media.read(cx).screen_share_track(&self.counterpart_user_id(cx)?)
    }

    fn remote_camera_track(&self, cx: &App) -> Option<Rc<VideoTrack>> {
        self.media.read(cx).camera_track(&self.counterpart_user_id(cx)?)
    }

    fn local_camera_track(&self, cx: &App) -> Option<Rc<VideoTrack>> {
        let current_user_id = self.current_user_id(cx);
        if current_user_id.is_empty() {
            return None;
        }
        self.media.read(cx).camera_track(&current_user_id)
    }

    /// owned by the media session not this screen, the screen is rebuilt
    /// on every minimize and reopen while the capturer keeps its position
    fn camera_position(&self, cx: &App) -> CameraPosition {
        self.media.read(cx).local_camera_position()
    }

    /// local capture as it leaves this device
    /// not the sfu echo, so the sharer sees the share started
    fn local_screen_track(&self, cx: &App) -> Option<Rc<VideoTrack>> {
        let current_user_id = self.current_user_id(cx);
        if current_user_id.is_empty() {
            return None;
        }
        self.media.read(cx).screen_share_track(&current_user_id)
    }

    /// announcement lands seconds before the media so its held apart from the track
    /// an announced surface keeps its slot with a loader instead of reflowing when frames arrive
    fn counterpart_announced_screen_share(&self, cx: &App) -> bool {
        self.counterpart_participant(cx).is_some_and(|participant| participant.screen_share_enabled)
    }

    fn counterpart_announced_camera(&self, cx: &App) -> bool {
        self.counterpart_participant(cx).is_some_and(|participant| participant.camera_enabled)
    }

    /// peer surfaces in display order, head fills the screen and the rest ride in floating windows
    /// a tap on a window rotates the order
    fn remote_surfaces(&self, cx: &App) -> Vec<RemoteSurfacePlaceholder> {
        let mut surfaces = Vec::with_capacity(2);
        if self.remote_screen_track(cx).is_some()
            || (self.counterpart_announced_screen_share(cx)
                && !self.abandoned_remote_surfaces.contains(&RemoteSurfacePlaceholder::ScreenShare))
        {
            surfaces.push(RemoteSurfacePlaceholder::ScreenShare);
        }
        if self.remote_camera_track(cx).is_some()
            || (self.counterpart_announced_camera(cx)
                && !self.abandoned_remote_surfaces.contains(&RemoteSurfacePlaceholder::Camera))
        {
            surfaces.push(RemoteSurfacePlaceholder::Camera);
        }
        if self.remote_media_swapped {
            surfaces.reverse();
        }
        surfaces
    }

    /// announced by the server but not carried by the sfu yet
    fn pending_remote_surfaces(&self, cx: &App) -> HashSet<RemoteSurfacePlaceholder> {
        let mut pending = HashSet::new();
        if self.counterpart_announced_screen_share(cx) && self.remote_screen_track(cx).is_none() {
            pending.insert(RemoteSurfacePlaceholder::ScreenShare);
        }
        if self.counterpart_announced_camera(cx) && self.remote_camera_track(cx).is_none() {
            pending.insert(RemoteSurfacePlaceholder::Camera);
        }
        pending.retain(|surface| !self.abandoned_remote_surfaces.contains(surface));
        pending
    }

    fn pending_remote_surface_identity(&self, cx: &App) -> String {
        let mut identities: Vec<&str> =
            self.pending_remote_surfaces(cx).iter().map(|surface| surface.identity()).collect();
        identities.sort_unstable();
        identities.join("+")
    }

    /// folded into one on_change key
    fn remote_surface_evidence(&self, cx: &App) -> String {
        format!(
            "{}|{}|{}|{}",
            self.remote_screen_track(cx).is_some(),
            self.remote_camera_track(cx).is_some(),
            self.counterpart_announced_screen_share(cx),
            self.counterpart_announced_camera(cx),
        )
    }

    /// everything this screen reads from the media store, without audio levels
    fn current_media_fingerprint(&self, cx: &App) -> String {
        let track_key = |track: Option<Rc<VideoTrack>>| track.map(|track| track.track_key().to_owned());
        format!(
            "{:?}|{:?}|{:?}|{:?}|{}|{:?}",
            track_key(self.remote_screen_track(cx)),
            track_key(self.remote_camera_track(cx)),
            track_key(self.local_camera_track(cx)),
            track_key(self.local_screen_track(cx)),
            self.counterpart_is_present(cx),
            self.camera_position(cx),
        )
    }

    fn reconcile_abandoned_remote_surfaces(&mut self, cx: &mut Context<Self>) {
        if self.remote_screen_track(cx).is_some() || !self.counterpart_announced_screen_share(cx) {
            self.abandoned_remote_surfaces.remove(&RemoteSurfacePlaceholder::ScreenShare);
        }
        if self.remote_camera_track(cx).is_some() || !self.counterpart_announced_camera(cx) {
            self.abandoned_remote_surfaces.remove(&RemoteSurfacePlaceholder::Camera);
        }
        if self.chrome_hidden && !self.has_primary_surface(cx) {
            self.chrome_animation = ease_in_out(0.2);
            self.chrome_hidden = false;
        }
        cx.notify();
    }

    fn remote_track(&self, surface: RemoteSurfacePlaceholder, cx: &App) -> Option<Rc<VideoTrack>> {
        match surface {
            RemoteSurfacePlaceholder::ScreenShare => self.remote_screen_track(cx),
            RemoteSurfacePlaceholder::Camera => self.remote_camera_track(cx),
        }
    }

    fn remote_primary_surface(&self, cx: &App) -> Option<RemoteSurfacePlaceholder> {
        self.remote_surfaces(cx).first().copied()
    }

    fn remote_secondary_surface(&self, cx: &App) -> Option<RemoteSurfacePlaceholder> {
        self.remote_surfaces(cx).get(1).copied()
    }

    fn local_primary_surface(&self, cx: &App) -> Option<RemoteSurfacePlaceholder> {
        if self.visual_state(cx).is_terminal() {
            return None;
        }
        match self.selected_local_surface {
            Some(RemoteSurfacePlaceholder::Camera)
                if self.displayed_camera_enabled(cx) && self.has_local_camera_evidence(cx) =>
            {
                Some(RemoteSurfacePlaceholder::Camera)
            }
            Some(RemoteSurfacePlaceholder::ScreenShare) if self.displayed_screen_share_enabled(cx) => {
                Some(RemoteSurfacePlaceholder::ScreenShare)
            }
            _ => None,
        }
    }

    fn has_primary_surface(&self, cx: &App) -> bool {
        self.local_primary_surface(cx).is_some() || self.remote_primary_surface(cx).is_some()
    }

    fn header_flattened(&self, cx: &App) -> bool {
        self.has_primary_surface(cx)
    }

    fn select_local_surface(&mut self, surface: Option<RemoteSurfacePlaceholder>, cx: &mut Context<Self>) {
        self.with_animation(spring(0.42, 0.86));
        self.selected_local_surface = surface;
        cx.notify();
    }

    fn picture_in_picture_supported(&self) -> bool {
        RiseRTCCallPictureInPictureController::is_supported()
    }

    /// wire identity of the source not the object address, a freed address
    /// gets reused by another track so pointer identity misses the swap
    fn picture_in_picture_source_id(&self, cx: &App) -> Option<String> {
        if self.local_primary_surface(cx).is_some() {
            return None;
        }
        let surface = self.remote_primary_surface(cx)?;
        let track = self.remote_track(surface, cx)?;
        let user_id = self.counterpart_user_id(cx)?;
        Some(format!("{user_id}|{}|{}", surface.identity(), track.track_key()))
    }

    /// one observed key instead of four on_change
    fn picture_in_picture_source_identity(&self, cx: &App) -> String {
        let state = self.visual_state(cx);
        format!(
            "{}|{}|{}",
            state.is_terminal(),
            state.is_connected(),
            self.picture_in_picture_source_id(cx).unwrap_or_else(|| "none".to_owned()),
        )
    }

    fn picture_in_picture_track(&self, cx: &App) -> Option<Rc<VideoTrack>> {
        if self.local_primary_surface(cx).is_some() {
            return None;
        }
        self.remote_track(self.remote_primary_surface(cx)?, cx)
    }

    fn synchronize_picture_in_picture(&self, state: VisualState, cx: &mut App) {
        if !self.picture_in_picture_supported() {
            return;
        }
        let owner = RiseRTCCallPictureInPictureControllerOwner::DirectCall;
        let controller = RiseRTCCallPictureInPictureController::shared(cx);
        let track = if state.is_terminal() { None } else { self.picture_in_picture_track(cx) };
        let Some(track) = track else {
            let is_terminal = state.is_terminal();
            let face = RiseRTCPictureInPictureFace {
                user_id: self.counterpart_user_id(cx),
                name: self.counterpart_name(cx),
                avatar_url: self.counterpart_avatar_url(cx),
            };
            controller.update(cx, |controller, cx| {
                if is_terminal || !controller.hold_without_source(owner, face, cx) {
                    controller.disable(owner, cx);
                }
            });
            return;
        };
        let source_id = self.picture_in_picture_source_id(cx);
        controller.update(cx, |controller, cx| {
            controller.enable(owner, cx);
            controller.present(Some(track.track_key().to_owned()), source_id, owner, cx);
        });
    }

    fn counterpart_is_present(&self, cx: &App) -> bool {
        self.counterpart_user_id(cx).is_some_and(|user_id| self.media.read(cx).is_present(&user_id))
    }

    fn visual_state(&self, cx: &App) -> VisualState {
        let system_call = self.system_call(cx);
        let call = self.call(cx);
        let system_phase = system_call.as_ref().map(|call| call.phase);
        let connection_state = self.rtc.read(cx).connection_state(cx);
        if system_phase == Some(RiseRTCSystemCallPhase::Ended) {
            return match system_call.as_ref().and_then(|call| call.terminal_outcome) {
                Some(RiseRTCSystemCallTerminalOutcome::Failed) => VisualState::Failed,
                Some(RiseRTCSystemCallTerminalOutcome::Cancelled) => VisualState::Cancelled,
                Some(RiseRTCSystemCallTerminalOutcome::Declined) => VisualState::Declined,
                Some(RiseRTCSystemCallTerminalOutcome::TimedOut) => VisualState::TimedOut,
                Some(RiseRTCSystemCallTerminalOutcome::AnsweredElsewhere)
                | Some(RiseRTCSystemCallTerminalOutcome::Ended)
                | None => VisualState::Ended,
            };
        }
        if let Some(call) = &call
            && call.state.is_terminal()
        {
            match call.state {
                CallState::Declined => return VisualState::Declined,
                CallState::Cancelled => return VisualState::Cancelled,
                CallState::TimedOut => return VisualState::TimedOut,
                CallState::Ended => return VisualState::Ended,
                CallState::Unspecified | CallState::Ringing | CallState::Accepted => {}
            }
        }
        if let Some(local_end_presentation) = &self.local_end_presentation
            && local_end_presentation.identity == self.presentation_identity(cx)
            && local_end_presentation.account_id == self.current_user_id(cx)
            && local_end_presentation.account_epoch == normalized(SaiAccountCacheScope::active_epoch().as_deref())
        {
            return local_end_presentation.state;
        }
        if self.is_ending || system_phase == Some(RiseRTCSystemCallPhase::Ending) {
            return VisualState::Ending;
        }
        let call_accepted = call.as_ref().is_some_and(|call| call.state == CallState::Accepted);
        if connection_state == ConnectionState::Failed {
            if system_phase == Some(RiseRTCSystemCallPhase::Active) {
                return VisualState::Failed;
            }
            if system_call.is_none() && call_accepted {
                return VisualState::Failed;
            }
        }
        if connection_state == ConnectionState::Reconnecting {
            return VisualState::Reconnecting;
        }
        if call_accepted || system_phase == Some(RiseRTCSystemCallPhase::Active) {
            if connection_state != ConnectionState::Connected
                || self.socket.read(cx).is_network_path_down()
                || !(system_call.is_none() || system_phase == Some(RiseRTCSystemCallPhase::Active))
            {
                return if self.peer_first_joined_at.is_none() {
                    VisualState::Connecting
                } else {
                    VisualState::Reconnecting
                };
            }
            if !self.counterpart_is_present(cx) {
                return VisualState::AwaitingPeer;
            }
            return VisualState::Connected(self.connected_at(cx));
        }
        match system_phase {
            Some(RiseRTCSystemCallPhase::IncomingRinging) => VisualState::Incoming,
            Some(RiseRTCSystemCallPhase::OutgoingRequested | RiseRTCSystemCallPhase::OutgoingStarting) => {
                if system_call.as_ref().is_some_and(|call| call.awaiting_connection) {
                    VisualState::Connecting
                } else {
                    VisualState::Request
                }
            }
            Some(RiseRTCSystemCallPhase::OutgoingRinging) => {
                if self.counterpart_is_present(cx) || call.as_ref().is_some_and(|call| call.answered_at_ms.is_some()) {
                    VisualState::Connecting
                } else {
                    VisualState::Ringing
                }
            }
            Some(RiseRTCSystemCallPhase::Answering) => VisualState::Connecting,
            Some(RiseRTCSystemCallPhase::Active) => {
                if connection_state == ConnectionState::Connected {
                    VisualState::Connected(self.connected_at(cx))
                } else {
                    VisualState::Connecting
                }
            }
            Some(RiseRTCSystemCallPhase::Ending) => VisualState::Ending,
            Some(RiseRTCSystemCallPhase::Ended) => VisualState::Ended,
            None => {
                if let Some(call) = &call
                    && call.state == CallState::Ringing
                {
                    return if call.caller_user_id == self.current_user_id(cx) {
                        VisualState::Ringing
                    } else {
                        VisualState::Incoming
                    };
                }
                if connection_state == ConnectionState::Reconnecting {
                    VisualState::Reconnecting
                } else {
                    VisualState::Connecting
                }
            }
        }
    }

    /// when the conversation really began
    /// answer timestamp is when this side committed, often seconds before the peer joins
    /// recorded once so a peer who drops and rejoins resumes the same duration
    fn connected_at(&self, cx: &App) -> Date {
        if let Some(peer_first_joined_at) = self.peer_first_joined_at {
            return peer_first_joined_at;
        }
        if let Some(date) = self.system_call(cx).and_then(|call| call.connected_at) {
            return date;
        }
        let call = self.call(cx);
        if let Some(milliseconds) = call.as_ref().and_then(|call| call.answered_at_ms) {
            return Date::from_time_interval_since_1970(milliseconds as TimeInterval / 1_000.0);
        }
        Date::from_time_interval_since_1970(call.map_or(0, |call| call.updated_at_ms) as TimeInterval / 1_000.0)
    }

    fn presentation_identity(&self, cx: &App) -> String {
        self.system_call(cx)
            .map(|call| call.local_uuid.uuid_string())
            .or_else(|| self.resolved_server_call_id(cx))
            .unwrap_or_else(|| "direct-call".to_owned())
    }

    fn install_observers(&mut self, state: VisualState, window: &mut Window, cx: &mut Context<Self>) {
        let this = self.this.clone();

        let self_participant = self.self_participant(cx).map(|participant| {
            (
                participant.microphone_enabled,
                participant.camera_enabled,
                participant.screen_share_enabled,
                participant.deafened,
            )
        });
        let weak = this.clone();
        on_change(self.scoped_id("self-participant"), self_participant, false, window, cx, move |_, _, _, cx| {
            weak.update(cx, |this, cx| {
                this.reconcile_pending_media(cx);
                this.flush_pending_media(cx);
            })
            .ok();
        });

        let phase = self.system_call(cx).map(|call| call.phase);
        let weak = this.clone();
        on_change(self.scoped_id("system-phase"), phase, false, window, cx, move |_, phase, _, cx| {
            let is_active = *phase == Some(RiseRTCSystemCallPhase::Active);
            weak.update(cx, |this, cx| {
                this.flush_pending_media(cx);
                if is_active {
                    this.synchronize_call_kit_mute_state(cx);
                }
            })
            .ok();
        });

        let connection_state = self.rtc.read(cx).connection_state(cx);
        let weak = this.clone();
        on_change(self.scoped_id("connection-state"), connection_state, false, window, cx, move |_, _, _, cx| {
            weak.update(cx, |this, cx| this.flush_pending_media(cx)).ok();
        });

        let awaiting_probe = (
            self.counterpart_user_id(cx),
            matches!(self.visual_state(cx), VisualState::AwaitingPeer),
        );
        on_change(self.scoped_id("x1882-awaiting-probe"), awaiting_probe, true, window, cx, |_, probe, _, cx| {
            let remotes = RiseRTCMediaPresentation::shared(cx).read(cx).remote_user_ids().clone();
            tracing::warn!(
                target: "riseonly::X1882",
                counterpart = ?probe.0,
                awaiting = probe.1,
                ?remotes,
                "direct call presence probe"
            );
        });

        let counterpart_is_present = self.counterpart_is_present(cx);
        let weak = this.clone();
        on_change(self.scoped_id("counterpart-present"), counterpart_is_present, false, window, cx, move |_, is_present, _, cx| {
            if !*is_present {
                return;
            }
            weak.update(cx, |this, cx| {
                if this.peer_first_joined_at.is_none() {
                    this.peer_first_joined_at = Some(Date::now());
                    cx.notify();
                }
            })
            .ok();
        });

        let resolved_server_call_id = self.resolved_server_call_id(cx);
        let weak = this.clone();
        on_change(self.scoped_id("server-call-id"), resolved_server_call_id, false, window, cx, move |_, _, _, cx| {
            weak.update(cx, |this, cx| {
                this.peer_first_joined_at = None;
                this.abandoned_remote_surfaces.clear();
                this.selected_local_surface = None;
                cx.notify();
            })
            .ok();
        });

        let local_primary_surface = self.local_primary_surface(cx);
        let weak = this.clone();
        on_change(self.scoped_id("local-primary"), local_primary_surface, false, window, cx, move |_, surface, _, cx| {
            let cleared = surface.is_none();
            weak.update(cx, |this, cx| {
                if cleared {
                    this.selected_local_surface = None;
                }
                if !this.has_primary_surface(cx) {
                    this.chrome_hidden = false;
                }
                cx.notify();
            })
            .ok();
        });

        let remote_surface_evidence = self.remote_surface_evidence(cx);
        let weak = this.clone();
        on_change(self.scoped_id("surface-evidence"), remote_surface_evidence, false, window, cx, move |_, _, _, cx| {
            weak.update(cx, |this, cx| this.reconcile_abandoned_remote_surfaces(cx)).ok();
        });

        let pending = self.pending_remote_surfaces(cx);
        let weak = this.clone();
        use_task(
            self.scoped_id("expire-surfaces"),
            self.pending_remote_surface_identity(cx),
            window,
            cx,
            async move |cx| {
                if pending.is_empty() {
                    return;
                }
                cx.background_executor().timer(REMOTE_SURFACE_ARRIVAL_WINDOW).await;
                weak.update(cx, |this, cx| {
                    for surface in pending {
                        if this.remote_track(surface, cx).is_none() {
                            this.abandoned_remote_surfaces.insert(surface);
                        }
                    }
                    cx.notify();
                })
                .ok();
            },
        );

        let weak = this.clone();
        let stranded_intent = self.pending_screen_share_intent;
        use_task(self.scoped_id("stranded-intent"), stranded_intent, window, cx, async move |cx| {
            if stranded_intent.is_none() {
                return;
            }
            cx.background_executor().timer(Duration::from_secs(1)).await;
            weak.update(cx, |this, cx| this.apply_pending_screen_share_intent(cx)).ok();
        });

        let last_error_code = self.rtc.read(cx).last_error_code(cx).map(str::to_owned);
        let weak = this.clone();
        on_change(self.scoped_id("last-error"), last_error_code, false, window, cx, move |_, code, _, cx| {
            let code = code.clone();
            weak.update(cx, |this, cx| this.announce_media_failure(code.as_deref(), cx)).ok();
        });

        let displayed_microphone_enabled = self.displayed_microphone_enabled(cx);
        let weak = this.clone();
        on_change(self.scoped_id("microphone-shown"), displayed_microphone_enabled, false, window, cx, move |_, enabled, _, cx| {
            if !*enabled {
                return;
            }
            weak.update(cx, |this, cx| {
                this.microphone_refused = false;
                cx.notify();
            })
            .ok();
        });

        on_change(self.scoped_id("guide-facts"), self.guide_facts(cx), true, window, cx, |_, facts, _, cx| {
            Self::report_guide_facts(*facts, cx);
        });

        let picture_in_picture_source_identity = self.picture_in_picture_source_identity(cx);
        let weak = this.clone();
        on_change(self.scoped_id("pip-source"), picture_in_picture_source_identity, true, window, cx, move |_, _, _, cx| {
            weak.update(cx, |this, cx| this.synchronize_picture_in_picture(state, cx)).ok();
        });

        let weak = this.clone();
        use_task(
            self.scoped_id("restored-connection"),
            self.restored_connection_task_identity(cx),
            window,
            cx,
            async move |cx| {
                let task = weak.update(cx, |this, cx| this.connect_restored_call_if_needed(cx)).ok().flatten();
                let Some(task) = task else {
                    return;
                };
                let Err(error) = task.await else {
                    return;
                };
                if error.is::<CancellationError>() || RiseRTCConnectionWaitPolicy::signalling_failure(&error).is_some() {
                    return;
                }
                weak.update(cx, |this, cx| this.show_action_failure(None, cx)).ok();
            },
        );

        let weak = this;
        on_change(self.scoped_id("task-identity"), state.task_identity(), true, window, cx, move |_, _, _, cx| {
            weak.update(cx, |this, cx| this.visual_state_task(state, cx)).ok();
        });

        let any_sheet_visible = self.any_sheet_visible();
        let weak = self.this.clone();
        on_change(self.scoped_id("focus"), any_sheet_visible, true, window, cx, move |_, visible, window, cx| {
            if *visible {
                return;
            }
            weak.update(cx, |this, cx| window.focus(&this.focus_handle, cx)).ok();
        });
    }

    fn visual_state_task(&mut self, state: VisualState, cx: &mut Context<Self>) {
        self.terminal_state_task = None;
        if self.terminal_hold_owned_by_state {
            self.terminal_hold_task = None;
            self.terminal_dismiss_scheduled = false;
            self.terminal_hold_owned_by_state = false;
        }
        if !state.is_terminal() {
            return;
        }
        self.terminal_state_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(120)).await;
            this.update(cx, |this, cx| {
                let identity = this.presentation_identity(cx);
                Self::play_end_tone(&identity, true, cx);
                this.settle_terminal_presentation(state.terminal_hold(), true, cx);
            })
            .ok();
        }));
    }

    fn any_sheet_visible(&self) -> bool {
        self.audio_routes_visible
            || self.audio_inputs_visible
            || self.stream_settings_visible
            || self.encryption_info_visible
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.key == ESCAPE_KEY {
            if !self.any_sheet_visible() {
                if let Some(on_minimize) = self.on_minimize.clone() {
                    on_minimize(window, cx);
                }
                cx.stop_propagation();
            }
            return;
        }
        if !(keystroke.modifiers.secondary() && keystroke.modifiers.shift) {
            return;
        }
        let state = self.visual_state(cx);
        if state.is_terminal() || self.is_ending {
            return;
        }
        let key = keystroke.key.to_ascii_lowercase();
        if key == MICROPHONE_SHORTCUT_KEY {
            self.toggle_microphone(cx);
        } else if key == CAMERA_SHORTCUT_KEY {
            self.toggle_camera(cx);
        } else if key == SCREEN_SHARE_SHORTCUT_KEY {
            let sharing = self.displayed_screen_share_enabled(cx);
            self.toggle_stream(sharing, cx);
        } else {
            return;
        }
        cx.stop_propagation();
    }

    fn call_background(&self, state: VisualState) -> RiseRTCAmbientCallBackground {
        let mode = match state {
            VisualState::Connected(_) => RiseRTCAmbientCallBackgroundMode::Active,
            VisualState::Reconnecting => RiseRTCAmbientCallBackgroundMode::WeakConnection,
            VisualState::Ending
            | VisualState::Ended
            | VisualState::Declined
            | VisualState::Cancelled
            | VisualState::TimedOut
            | VisualState::Failed => RiseRTCAmbientCallBackgroundMode::Terminal,
            VisualState::Request
            | VisualState::Ringing
            | VisualState::Incoming
            | VisualState::Connecting
            | VisualState::AwaitingPeer => RiseRTCAmbientCallBackgroundMode::Preconnect,
        };
        RiseRTCAmbientCallBackground::new(self.scoped_id("background"), mode)
    }

    fn remote_media_background(&self, primary: PrimarySurface, cx: &App) -> Div {
        let PrimarySurface { surface, is_local } = primary;
        let track = if is_local {
            if surface == RemoteSurfacePlaceholder::Camera {
                self.local_camera_track(cx)
            } else {
                self.local_screen_track(cx)
            }
        } else {
            self.remote_track(surface, cx)
        };
        let toggle = Self::action(self.this.clone(), |this, cx| this.toggle_chrome(cx));
        let content = if is_local && surface == RemoteSurfacePlaceholder::ScreenShare && track.is_some() {
            let stop = Self::action(self.this.clone(), |this, cx| this.stop_own_screen_share(cx));
            div()
                .id(self.scoped_id("own-share-switch"))
                .size_full()
                .on_click(move |_, window, cx| toggle(window, cx))
                .child(
                    RiseRTCOwnScreenShareWall::new(self.scoped_id("own-share-wall"), theme(cx).clone())
                        .on_stop(move |window, cx| stop(window, cx)),
                )
                .into_any_element()
        } else if let Some(track) = track {
            RiseRTCZoomableVideoSurface::new(
                self.scoped_id("primary-surface"),
                if is_local { "local-primary" } else { "remote-primary" },
                track,
                if surface == RemoteSurfacePlaceholder::ScreenShare {
                    VideoViewLayoutMode::Fit
                } else {
                    VideoViewLayoutMode::Fill
                },
                if is_local && surface == RemoteSurfacePlaceholder::Camera {
                    VideoViewMirrorMode::Auto
                } else {
                    VideoViewMirrorMode::Off
                },
                toggle,
            )
            .into_any_element()
        } else {
            let legend = if is_local && surface == RemoteSurfacePlaceholder::ScreenShare {
                self.local_screen_placeholder(false).into_any_element()
            } else {
                self.surface_loader(surface, false).into_any_element()
            };
            div()
                .id(self.scoped_id("primary-switch"))
                .flex()
                .flex_col()
                .size_full()
                .on_click(move |_, window, cx| toggle(window, cx))
                .child(div().w_full().flex_1().child(legend))
                .child(div().w_full().flex_1().bg(black()))
                .into_any_element()
        };
        div()
            .absolute()
            .inset_0()
            .bg(black())
            .child(content)
            .child(self.media_readability_gradient())
    }

    fn is_chrome_hidden(&self, state: VisualState, cx: &App) -> bool {
        if !self.chrome_hidden || !self.has_primary_surface(cx) {
            return false;
        }
        if state.is_terminal() {
            return false;
        }
        if state == VisualState::Incoming && self.offers_in_app_answer(cx) {
            return false;
        }
        true
    }

    /// a running guide points at the dock so a click on the video must not hide it under the bubble
    fn toggle_chrome(&mut self, cx: &mut Context<Self>) {
        if !self.chrome_hidden && GuidesServicesStore::shared(cx).read(cx).is_running(cx) {
            return;
        }
        self.chrome_animation = ease_in_out(0.26);
        self.chrome_hidden = !self.chrome_hidden;
        cx.notify();
    }

    /// black with a legend, an in flight surface must look different from nothing switched on
    fn surface_loader(&self, surface: RemoteSurfacePlaceholder, compact: bool) -> Div {
        let tint = white().opacity(0.72);
        div().size_full().flex().items_center().justify_center().when(!compact, |fill| fill.bg(black())).child(
            vstack(if compact { 8.0 } else { 14.0 })
                .p(px(if compact { 6.0 } else { 0.0 }))
                .children(symbol(surface.system_image(), if compact { 20.0 } else { 34.0 }, tint))
                .when(!compact, |stack| {
                    stack.child(
                        MainText::new(surface.title())
                            .px(15.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::MEDIUM)
                            .color(tint),
                    )
                })
                .child(progress_view().tint(tint).control_size(if compact { ControlSize::Small } else { ControlSize::Regular })),
        )
    }

    fn media_readability_gradient(&self) -> impl IntoElement {
        let now = RiseClock::monotonic();
        div().absolute().inset_0().opacity(self.readability_gradient_opacity.value(now)).child(LinearGradient::new(
            vec![
                GradientStop::new(black().opacity(0.28), 0.0),
                GradientStop::new(black().opacity(0.0), 0.5),
                GradientStop::new(black().opacity(0.62), 1.0),
            ],
            UnitPoint::TOP,
            UnitPoint::BOTTOM,
        ))
    }

    fn header(&self, cx: &App) -> Div {
        let flattened = self.header_flattened(cx);
        let this = self.this.clone();
        let minimize = Self::minimize_action(this.clone());
        hstack(0.0)
            .w_full()
            .h(px(46.0))
            .relative()
            .child(env_scope::<LiquidPressSettlesAtOnceKey>(true).child(glass_control(
                self.scoped_id("minimize-control"),
                tr("rtc_minimize"),
                LiquidGlassButton::new(self.scoped_id("minimize"), LiquidGlassButtonShape::Circle)
                    .size(44.0)
                    .haptic_on_press(false)
                    .flattened(flattened)
                    .accessibility_label(Some(tr("rtc_minimize")))
                    .on_press(minimize)
                    .child(div().size(px(44.0)).flex().items_center().justify_center().children(symbol("xmark", 16.0, white()))),
            )))
            .child(spacer_min_w(8.0))
            .child(RiseRTCCallTraceControl)
            .when(self.displayed_camera_enabled(cx) && self.media.read(cx).can_flip_camera(), |row| {
                row.child(
                    div()
                        .mr(px(8.0))
                        .child(
                            glass_control(
                                self.scoped_id("flip-control"),
                                tr("rtc_flip_camera"),
                                LiquidGlassButton::new(self.scoped_id("flip"), LiquidGlassButtonShape::Circle)
                                    .size(44.0)
                                    .haptic_on_press(false)
                                    .flattened(flattened)
                                    .accessibility_label(Some(tr("rtc_flip_camera")))
                                    .on_press(Self::action(this.clone(), |this, cx| this.flip_camera(cx)))
                                    .child(
                                        div()
                                            .size(px(44.0))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .children(symbol("arrow.triangle.2.circlepath.camera.fill", 15.0, white())),
                                    ),
                            )
                            .guide_anchor(RiseGuideAnchor::CallFlipCamera),
                        ),
                )
            })
            .child(self.settings_menu(flattened, cx).guide_anchor(RiseGuideAnchor::CallSettingsMenu))
            .children(self.verification_emojis(cx).map(|emojis| {
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(self.encryption_badge(&emojis, flattened))
            }))
    }

    fn settings_menu(&self, flattened: bool, cx: &App) -> impl IntoElement {
        let this = self.this.clone();
        let mut items = vec![
            ContextMenuItem::new(
                0,
                tr("rtc_audio_output"),
                Some("speaker.wave.2.fill".to_owned()),
                "audio-output",
                Self::action(this.clone(), |this, cx| {
                    this.audio_routes_visible = true;
                    cx.notify();
                }),
            ),
            ContextMenuItem::new(
                1,
                tr("rtc_voice_settings"),
                Some("mic.and.signal.meter.fill".to_owned()),
                "voice-settings",
                Self::action(this.clone(), |this, cx| {
                    this.audio_inputs_visible = true;
                    cx.notify();
                }),
            ),
        ];
        let sharing = self.displayed_screen_share_enabled(cx);
        items.push(ContextMenuItem::new(
            2,
            tr(if sharing { "rtc_stop_stream" } else { "rtc_stream_settings" }),
            Some((if sharing { "rectangle.on.rectangle.slash" } else { "rectangle.on.rectangle" }).to_owned()),
            if sharing { "stop_stream" } else { "stream_settings" },
            Self::action(this.clone(), move |this, cx| this.toggle_stream(sharing, cx)),
        ));
        if !self.call(cx).is_some_and(|call| call.is_recording()) {
            let accepted = self.call(cx).is_some_and(|call| call.state == CallState::Accepted);
            items.push(
                ContextMenuItem::new(
                    3,
                    tr("rtc_record_call"),
                    Some("record.circle".to_owned()),
                    "record",
                    Self::action(this, |this, cx| {
                        let accepted = this.call(cx).is_some_and(|call| call.state == CallState::Accepted);
                        if this.is_ending || this.recording_busy || !accepted {
                            return;
                        }
                        this.toggle_recording(cx);
                    }),
                )
                .disabled(self.is_ending || self.recording_busy || !accepted),
            );
        }
        let label = LiquidGlassMenuLabel::new()
            .shape(LiquidGlassButtonShape::Circle)
            .size(44.0)
            .flattened(flattened)
            .children(symbol("gearshape.fill", 16.0, white()));
        glass_control(
            self.scoped_id("settings-label"),
            tr("rtc_call_settings"),
            ContextMenuUi::new(self.scoped_id("settings"), items, label).shows_item_icons(),
        )
    }

    fn encryption_badge(&self, emojis: &[String], flattened: bool) -> impl IntoElement {
        glass_control(
            self.scoped_id("encryption-control"),
            tr("rtc_encryption_title"),
            LiquidGlassButton::new(self.scoped_id("encryption"), LiquidGlassButtonShape::Capsule)
                .size(40.0)
                .horizontal_padding(12.0)
                .vertical_padding(6.0)
                .haptic_on_press(false)
                .flattened(flattened)
                .accessibility_label(Some(tr("rtc_encryption_title")))
                .on_press(Self::action(self.this.clone(), |this, cx| {
                    this.encryption_info_visible = true;
                    cx.notify();
                }))
                .child(hstack(3.0).h(px(40.0)).children(emojis.iter().map(|emoji| MainText::new(emoji.clone()).px(16.0)))),
        )
        .guide_anchor(RiseGuideAnchor::CallEncryption)
    }

    fn guide_facts(&self, cx: &App) -> GuideCallFacts {
        let self_participant = self.self_participant(cx);
        GuideCallFacts {
            connected: self.visual_state(cx).is_connected(),
            camera_on: self_participant.is_some_and(|participant| participant.camera_enabled),
            screen_share_on: self_participant.is_some_and(|participant| participant.screen_share_enabled),
            windows_swapped: self.remote_media_swapped || self.selected_local_surface.is_some(),
            stream_settings_open: self.stream_settings_visible,
            encryption_open: self.encryption_info_visible,
        }
    }

    fn report_guide_facts(facts: GuideCallFacts, cx: &mut App) {
        GuideFacts::set(RiseGuideSignal::CallConnected, facts.connected, cx);
        GuideFacts::set(RiseGuideSignal::CallCameraOn, facts.camera_on, cx);
        GuideFacts::set(RiseGuideSignal::CallScreenShareOn, facts.screen_share_on, cx);
        GuideFacts::set(RiseGuideSignal::CallWindowsSwapped, facts.windows_swapped, cx);
        GuideFacts::set(RiseGuideSignal::CallStreamSettingsOpen, facts.stream_settings_open, cx);
        GuideFacts::set(RiseGuideSignal::CallEncryptionOpen, facts.encryption_open, cx);
    }

    /// present only once a key exists
    /// no placeholder padlock, a badge before the key would claim more than the call can back
    fn verification_emojis(&self, cx: &App) -> Option<Vec<String>> {
        let call_id = self.resolved_server_call_id(cx);
        self.rtc.read(cx).verification_emojis(call_id.as_deref(), cx).map(<[String]>::to_vec)
    }

    fn avatar_content(&self, geometry: Size<f32>, state: VisualState, cx: &App) -> impl IntoElement {
        let avatar_size = (geometry.width * 0.43).min(geometry.height * 0.23).clamp(126.0, 184.0);
        let counterpart_name = self.counterpart_name(cx);
        vstack(22.0)
            .child(RiseRTCAudioReactiveAvatarRing {
                id: self.scoped_id("avatar-ring"),
                avatar_url: self.counterpart_avatar_url(cx),
                name: counterpart_name.clone(),
                user_id: self.counterpart_user_id(cx),
                avatar_size,
                connected: state.is_connected(),
            })
            .child(
                vstack(8.0)
                    .child(
                        MainText::new(counterpart_name)
                            .px((geometry.width * 0.08).clamp(27.0, 34.0))
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::MEDIUM)
                            .color(white())
                            .number_of_lines(1)
                            .truncation(TruncationMode::Tail),
                    )
                    .child(self.status_view(state, false, cx)),
            )
    }

    fn remote_media_identity(&self, state: VisualState, cx: &App) -> impl IntoElement {
        let on_live_surface = self.has_primary_surface(cx);
        let text = if self.local_primary_surface(cx).is_none() { self.counterpart_name(cx) } else { tr("rtc_you") };
        vstack(4.0)
            .child(Self::chrome_pill(text, 13.0, FontWeight::SEMIBOLD, 172.0, on_live_surface))
            .child(
                div()
                    .flex()
                    .px(px(11.0))
                    .py(px(5.0))
                    .child(self.status_view(state, true, cx))
                    .call_chrome_with(on_live_surface, 0.16),
            )
    }

    fn chrome_pill(text: String, text_px: f32, weight: FontWeight, limit: f32, on_live_surface: bool) -> impl IntoElement {
        const HORIZONTAL_PADDING: f32 = 11.0;
        const VERTICAL_PADDING: f32 = 5.0;
        let max_width = px(limit + HORIZONTAL_PADDING * 2.0);
        div().flex().max_w(max_width).child(
            div()
                .flex()
                .min_w_0()
                .max_w(max_width)
                .px(px(HORIZONTAL_PADDING))
                .py(px(VERTICAL_PADDING))
                .child(
                    MainText::new(text)
                        .px(text_px)
                        .tac(TextAlignT::Center)
                        .font_weight(weight)
                        .color(white())
                        .number_of_lines(1)
                        .truncation(TruncationMode::Tail)
                        .min_w_0(),
                )
                .call_chrome_with(on_live_surface, 0.16),
        )
    }

    fn silenced_pill(notice: &SilencedNotice, on_live_surface: bool) -> impl IntoElement {
        const HORIZONTAL_PADDING: f32 = 13.0;
        let part = |text: String| {
            MainText::new(text)
                .px(13.0)
                .tac(TextAlignT::Center)
                .font_weight(FontWeight::MEDIUM)
                .color(white())
                .number_of_lines(1)
        };
        let mut row = div().flex().flex_row().items_center().min_w_0();
        if notice.name.is_none() {
            row = row.child(part(notice.lead.clone()).truncation(TruncationMode::Tail).min_w_0());
        } else if !notice.lead.is_empty() {
            row = row.child(part(notice.lead.clone()).flex_none());
        }
        if let Some(name) = &notice.name {
            row = row.child(part(name.clone()).truncation(TruncationMode::Tail).min_w_0());
        }
        if !notice.tail.is_empty() {
            row = row.child(part(notice.tail.clone()).flex_none());
        }
        let max_width = px(260.0 + HORIZONTAL_PADDING * 2.0);
        div().flex().max_w(max_width).child(
            div()
                .flex()
                .min_w_0()
                .max_w(max_width)
                .px(px(HORIZONTAL_PADDING))
                .py(px(7.0))
                .child(row)
                .call_chrome_with(on_live_surface, 0.16),
        )
    }

    fn status_view(&self, state: VisualState, compact: bool, cx: &App) -> AnyElement {
        let size = if compact { 13.0 } else { 17.0 };
        if let VisualState::Connected(started_at) = state {
            return RiseRTCCallElapsedStatus {
                id: self.scoped_id(if compact { "elapsed-compact" } else { "elapsed" }),
                started_at,
                size,
                compact,
            }
            .into_any_element();
        }
        MainText::new(self.status_text(state, cx))
            .px(size)
            .tac(TextAlignT::Center)
            .font_weight(FontWeight::MEDIUM)
            .color(Self::status_color(state))
            .into_any_element()
    }

    /// in app answer, the system doesnt always answer so without this the only path
    /// to answer_call is a system action that never arrives for an unreported call
    fn offers_in_app_answer(&self, cx: &App) -> bool {
        if self.is_ending || self.is_answering {
            return false;
        }
        if let Some(system_call) = self.system_call(cx) {
            return system_call.phase == RiseRTCSystemCallPhase::IncomingRinging && system_call.requires_in_app_answer_ui;
        }
        self.call(cx).is_some_and(|call| call.state == CallState::Ringing && call.callee_user_id == self.current_user_id(cx))
    }

    fn controls(&self, geometry: Size<f32>, state: VisualState, window: &Window, cx: &App) -> AnyElement {
        if state == VisualState::Incoming && self.offers_in_app_answer(cx) {
            self.incoming_controls(geometry, cx).into_any_element()
        } else {
            self.active_controls(geometry, state, window, cx).into_any_element()
        }
    }

    fn incoming_controls(&self, geometry: Size<f32>, cx: &App) -> impl IntoElement {
        let compact = geometry.width < 430.0;
        let circle_size = if compact { 64.0 } else { 72.0 };
        let disabled = self.is_ending || self.is_answering;
        hstack(if compact { 44.0 } else { 60.0 })
            .items_start()
            .w_full()
            .justify_center()
            .child(self.call_control(
                "decline",
                "phone.down.fill",
                tr("rtc_decline_call"),
                circle_size,
                Some(theme(cx).semantic.error_100),
                disabled,
                Self::action(self.this.clone(), |this, cx| this.end_call(cx)),
                cx,
            ))
            .child(self.call_control(
                "answer",
                "phone.fill",
                tr("rtc_answer_call"),
                circle_size,
                Some(theme(cx).semantic.success_100),
                disabled,
                Self::action(self.this.clone(), |this, cx| this.answer_call(cx)),
                cx,
            ))
    }

    fn active_controls(&self, geometry: Size<f32>, state: VisualState, window: &Window, cx: &App) -> impl IntoElement {
        let compact = geometry.width < 430.0;
        let this = self.this.clone();
        let call = self.call(cx);
        let is_recording = call.as_ref().is_some_and(|call| call.is_recording());
        let dock_width = geometry.width - 2.0 * (14.0 + self.horizontal_safe_inset());
        let slot_count: f32 = if is_recording { 6.0 } else { 5.0 };
        let preferred_spacing = if compact { 10.0 } else { 13.0 };
        let preferred_size = if compact { 50.0 } else { 54.0 };
        let fits = slot_count * (preferred_size + 8.0) + (slot_count - 1.0) * preferred_spacing <= dock_width;
        let spacing = if fits { preferred_spacing } else { 6.0 };
        let circle_size: f32 = if fits {
            preferred_size
        } else {
            (((dock_width - (slot_count - 1.0) * spacing) / slot_count - 8.0).floor()).max(36.0)
        };
        let controls_disabled = state.is_terminal() || self.is_ending;
        let call_accepted = call.as_ref().is_some_and(|call| call.state == CallState::Accepted);
        let speaker_enabled = self.audio.read(cx).speaker_enabled();
        let camera_enabled = self.displayed_camera_enabled(cx);
        let hushed = RiseMicrophoneTestHush::is_active(cx);
        let deafened = self.displayed_deafened(cx) || hushed;
        let microphone_enabled = self.displayed_microphone_enabled(cx) && !hushed;
        let now = RiseClock::monotonic();

        let mut row = hstack(spacing)
            .items_start()
            .w_full()
            .justify_center()
            .relative()
            .child(self.dock_control(
                "audio-route",
                if speaker_enabled { "speaker.wave.3.fill" } else { "speaker.wave.2.fill" },
                tr("rtc_audio_speaker"),
                speaker_enabled,
                circle_size,
                !controls_disabled,
                false,
                Self::action(this.clone(), |this, cx| {
                    this.audio_routes_visible = true;
                    cx.notify();
                }),
                cx,
            ))
            .child(self.dock_control(
                "camera",
                if camera_enabled { "video.fill" } else { "video.slash.fill" },
                tr("rtc_video"),
                camera_enabled,
                circle_size,
                !controls_disabled,
                false,
                Self::action(this.clone(), |this, cx| this.toggle_camera(cx)),
                cx,
            ));
        if is_recording {
            row = row.child(self.dock_control(
                "record",
                "record.circle.fill",
                tr("rtc_recording_stop"),
                true,
                circle_size,
                !controls_disabled && !self.recording_busy && call_accepted,
                true,
                Self::action(this.clone(), |this, cx| this.toggle_recording(cx)),
                cx,
            ));
        }
        let silenced_notice = self.silenced_notice_presence.render_transitioned(now, window, |notice| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(-(SILENCED_NOTICE_HEIGHT + 12.0)))
                .flex()
                .justify_center()
                .child(Self::silenced_pill(notice, self.has_primary_surface(cx)))
        });
        row.child(self.dock_control(
            "headphones",
            if deafened { RiseSymbol::DEAFENED } else { "headphones" },
            tr("rtc_audio_route"),
            deafened,
            circle_size,
            !controls_disabled,
            false,
            Self::action(this.clone(), |this, cx| this.toggle_headphones(cx)),
            cx,
        ))
        .child(self.dock_control(
            "microphone",
            if microphone_enabled { "mic.fill" } else { "mic.slash.fill" },
            tr("rtc_microphone"),
            microphone_enabled,
            circle_size,
            !controls_disabled,
            false,
            Self::action(this.clone(), |this, cx| this.toggle_microphone(cx)),
            cx,
        ))
        .child(self.dock_control(
            "end",
            "phone.down.fill",
            tr("rtc_end_call"),
            true,
            circle_size,
            !controls_disabled,
            true,
            Self::action(this, |this, cx| this.end_call(cx)),
            cx,
        ))
        .children(silenced_notice)
    }

    fn toggle_camera(&mut self, cx: &mut Context<Self>) {
        self.pending_media.camera = Some(!self.displayed_camera_enabled(cx));
        self.flush_pending_media(cx);
        cx.notify();
    }

    fn toggle_headphones(&mut self, cx: &mut Context<Self>) {
        let deafened = !self.displayed_deafened(cx);
        self.pending_media.deafened = Some(deafened);
        if deafened {
            let enabled = self.displayed_microphone_enabled(cx);
            interactions(cx).update(cx, |store, cx| store.remember_microphone_before_deafen(enabled, cx));
            self.pending_media.microphone = Some(false);
        } else {
            let restored = interactions(cx).update(cx, |store, cx| store.take_microphone_before_deafen(cx));
            self.pending_media.microphone = Some(restored.unwrap_or(false));
        }
        self.flush_pending_media(cx);
        cx.notify();
    }

    fn toggle_microphone(&mut self, cx: &mut Context<Self>) {
        let enabled = !self.displayed_microphone_enabled(cx);
        self.pending_media.microphone = Some(enabled);
        if enabled && self.displayed_deafened(cx) {
            self.pending_media.deafened = Some(false);
            interactions(cx).update(cx, |store, _| store.forget_microphone_before_deafen());
        }
        self.flush_pending_media(cx);
        cx.notify();
    }

    /// sharing is what the control said when it was offered
    fn toggle_stream(&mut self, sharing: bool, cx: &mut Context<Self>) {
        if sharing {
            self.stop_own_screen_share(cx);
        } else {
            self.stream_quality = interactions(cx).update(cx, |store, cx| store.remembered_stream_quality(cx));
            self.stream_settings_visible = true;
        }
        cx.notify();
    }

    /// same path as the stop item of the settings menu
    fn stop_own_screen_share(&mut self, cx: &mut Context<Self>) {
        self.pending_media.screen_share = Some(false);
        self.flush_pending_media(cx);
        cx.notify();
    }

    fn flip_camera(&mut self, cx: &mut Context<Self>) {
        let position =
            if self.camera_position(cx) == CameraPosition::Front { CameraPosition::Back } else { CameraPosition::Front };
        let claimed = self.pending_media;
        self.run_media_action(
            "camera-position",
            claimed,
            false,
            None,
            move |cx| interactions(cx).update(cx, |store, cx| store.switch_camera(position, cx)),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn dock_control(
        &self,
        id: &'static str,
        system_image: &'static str,
        title: String,
        is_on: bool,
        size: f32,
        enabled: bool,
        destructive: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
        cx: &App,
    ) -> impl IntoElement {
        let silenced = destructive || (id == "microphone" && !is_on) || (id == "headphones" && is_on);
        let lit = !silenced && is_on && id != "microphone";
        let tint = if silenced {
            Some(theme(cx).semantic.error_100)
        } else if lit {
            Some(white())
        } else {
            None
        };
        let glyph_color = if lit { black() } else { white() };
        let busy = self.media_action_ids.contains(id) && !INSTANT_MEDIA_ACTION_IDS.contains(&id);
        let anchor = match id {
            "camera" => Some(RiseGuideAnchor::CallDockCamera),
            "end" => Some(RiseGuideAnchor::CallDockEnd),
            _ => None,
        };
        let glyph = if busy {
            progress_view().tint(glyph_color).into_any_element()
        } else {
            div().children(symbol(system_image, size * 0.34, glyph_color)).into_any_element()
        };
        let state_text = Self::silenced_state_text(id, silenced, destructive);
        let hint = if state_text.is_empty() { title.clone() } else { format!("{title}: {state_text}") };
        vstack(8.0)
            .opacity(if enabled { 1.0 } else { 0.42 })
            .child(
                glass_control(
                    self.scoped_id(format!("{id}-control")),
                    hint,
                    LiquidGlassButton::new(self.scoped_id(id), LiquidGlassButtonShape::Circle)
                        .size(size)
                        .tint(tint)
                        .haptic_on_press(false)
                        .disabled(!enabled || busy)
                        .flattened(self.header_flattened(cx))
                        .accessibility_label(Some(title.clone()))
                        .on_press(action)
                        .child(div().size(px(size)).flex().items_center().justify_center().child(glyph)),
                )
                .guide_anchor(anchor),
            )
            .child(
                MainText::new(title)
                    .px(11.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::MEDIUM)
                    .color(white())
                    .number_of_lines(1)
                    .minimum_scale_factor(0.6)
                    .w(px(size + 8.0)),
            )
    }

    fn silenced_state_text(id: &str, silenced: bool, destructive: bool) -> String {
        if !silenced || destructive {
            return String::new();
        }
        match id {
            "headphones" => tr("rtc_deafened"),
            "microphone" => tr("rtc_muted"),
            _ => String::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn call_control(
        &self,
        id: &'static str,
        icon: &'static str,
        title: String,
        circle_size: f32,
        tint: Option<Hsla>,
        disabled: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
        cx: &App,
    ) -> impl IntoElement {
        vstack(8.0)
            .opacity(if disabled { 0.52 } else { 1.0 })
            .child(glass_control(
                self.scoped_id(format!("{id}-control")),
                title.clone(),
                LiquidGlassButton::new(self.scoped_id(id), LiquidGlassButtonShape::Circle)
                    .size(circle_size)
                    .tint(tint)
                    .haptic_on_press(false)
                    .disabled(disabled)
                    .flattened(self.header_flattened(cx))
                    .accessibility_label(Some(title.clone()))
                    .on_press(action)
                    .child(
                        div()
                            .size(px(circle_size))
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(symbol(icon, circle_size * 0.34, white())),
                    ),
            ))
            .child(
                MainText::new(title)
                    .px(11.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::MEDIUM)
                    .color(white())
                    .number_of_lines(1)
                    .minimum_scale_factor(0.72)
                    .w(px(circle_size + 8.0)),
            )
    }

    fn sync_transitions(&mut self, state: VisualState, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        let motion = self.transaction_motion.take();
        let local_primary = self.local_primary_surface(cx);
        let screen_share_enabled = self.displayed_screen_share_enabled(cx);
        let camera_enabled = self.displayed_camera_enabled(cx);
        let has_camera_evidence = self.has_local_camera_evidence(cx);
        let remote_secondary = self.remote_secondary_surface(cx);
        let remote_primary = self.remote_primary_surface(cx);

        let primary = local_primary
            .map(|surface| PrimarySurface { surface, is_local: true })
            .or(remote_primary.map(|surface| PrimarySurface { surface, is_local: false }));
        if self.primary_surface_shown != primary {
            self.primary_surface_shown = primary;
            self.primary_surface_presence.set_in(primary, motion, now, cx);
        }

        self.remote_window_slot.sync(remote_secondary, motion, now, cx);
        let displaced_screen = local_primary == Some(RemoteSurfacePlaceholder::ScreenShare);
        self.displaced_screen_window_slot.sync(
            displaced_screen.then_some(RemoteSurfacePlaceholder::ScreenShare),
            motion,
            now,
            cx,
        );
        self.local_screen_window_slot.sync(
            (!displaced_screen && screen_share_enabled && !state.is_terminal())
                .then_some(RemoteSurfacePlaceholder::ScreenShare),
            motion,
            now,
            cx,
        );
        let displaced_camera = local_primary == Some(RemoteSurfacePlaceholder::Camera);
        self.displaced_camera_window_slot.sync(
            displaced_camera.then_some(RemoteSurfacePlaceholder::Camera),
            motion,
            now,
            cx,
        );
        self.local_camera_window_slot.sync(
            (!displaced_camera && camera_enabled && !state.is_terminal() && has_camera_evidence)
                .then_some(RemoteSurfacePlaceholder::Camera),
            motion,
            now,
            cx,
        );

        let silenced_notice = self.silenced_notice(cx);
        if self.silenced_notice_shown != silenced_notice {
            self.silenced_notice_shown = silenced_notice.clone();
            self.silenced_notice_presence.set_in(silenced_notice, ease_in_out(0.22), now, cx);
        }

        let gradient_opacity = if self.chrome_hidden { 0.0 } else { 1.0 };
        if *self.readability_gradient_opacity.target() != gradient_opacity {
            self.readability_gradient_opacity.set_in(gradient_opacity, self.chrome_animation, now, cx);
        }
    }

    fn floating_windows(&self, geometry: Size<f32>, window: &Window, cx: &App) -> Vec<AnyElement> {
        let now = RiseClock::monotonic();
        let layer = |element: AnyElement| div().absolute().inset_0().child(element);
        let mut windows = Vec::with_capacity(3);
        windows.extend(self.remote_window_slot.presence.render_transitioned(now, window, |surface| {
            layer(self.remote_floating_window(*surface, geometry, cx))
        }));
        windows.extend(self.displaced_screen_window_slot.presence.render_transitioned(now, window, |surface| {
            layer(self.displaced_peer_window(geometry, *surface, cx))
        }));
        windows.extend(self.local_screen_window_slot.presence.render_transitioned(now, window, |_| {
            layer(self.local_screen_floating_window(geometry, cx))
        }));
        windows.extend(self.displaced_camera_window_slot.presence.render_transitioned(now, window, |surface| {
            layer(self.displaced_peer_window(geometry, *surface, cx))
        }));
        windows.extend(self.local_camera_window_slot.presence.render_transitioned(now, window, |_| {
            layer(self.local_camera_floating_window(geometry, cx))
        }));
        windows.into_iter().map(IntoElement::into_any_element).collect()
    }

    /// displayed_camera_enabled falls back to the system
    /// call has_video which is true already while ringing
    /// without evidence of a capture that fallback painted a spinner over the ring
    fn has_local_camera_evidence(&self, cx: &App) -> bool {
        self.local_camera_track(cx).is_some()
            || self.pending_media.camera == Some(true)
            || self.self_participant(cx).is_some_and(|participant| participant.camera_enabled)
    }

    fn floating_window_size(&self, geometry: Size<f32>) -> Size<f32> {
        let width = (geometry.width * 0.29).min(112.0);
        size(width, width * 1.34)
    }

    fn bottom_floating_window_reserve(&self, geometry: Size<f32>) -> f32 {
        const NAME_CLEARANCE: f32 = 8.0;
        let resting_low = |slot: &FloatingWindowSlot, position: UnitPoint| slot.shown.is_some() && position.y > 0.5;
        let any_low = resting_low(&self.remote_window_slot, self.remote_window_position)
            || resting_low(&self.displaced_screen_window_slot, self.local_screen_window_position)
            || resting_low(&self.local_screen_window_slot, self.local_screen_window_position)
            || resting_low(&self.displaced_camera_window_slot, self.local_window_position)
            || resting_low(&self.local_camera_window_slot, self.local_window_position);
        if !any_low {
            return 0.0;
        }
        let window_top_from_bottom = self.floating_window_insets().bottom + self.floating_window_size(geometry).height;
        (window_top_from_bottom + NAME_CLEARANCE - RiseRTCScreenSafeArea::dock_bottom_padding() - self.controls_height).max(0.0)
    }

    fn floating_window_insets(&self) -> EdgeInsets {
        EdgeInsets {
            top: self.top_safe_inset() + if self.chrome_hidden { 12.0 } else { 72.0 },
            leading: 16.0 + self.horizontal_safe_inset(),
            bottom: self.bottom_safe_inset() + if self.chrome_hidden { 12.0 } else { 132.0 },
            trailing: 16.0 + self.horizontal_safe_inset(),
        }
    }

    fn sheet(
        &self,
        name: &'static str,
        is_presented: Binding<bool>,
        fraction: f32,
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
        cx: &App,
    ) -> BottomSheetUi {
        let background = theme(cx).bg._100;
        BottomSheetUi::new(self.scoped_id(name), is_presented, content, |_, _| Empty)
            .snap_fractions(vec![fraction])
            .dismisses_on_backdrop_tap(true)
            .full_page_shows_drag_indicator(true)
            .full_page_hides_footer(true)
            .footer_shows_top_border(false)
            .content_scrollable(true)
            .modal_sized_by_detent(true)
            .sheet_background(Some(background))
            .footer_background(Some(background))
            .surface_shadow_enabled(false)
    }

    /// one sheet per concern behind the gear menu, like the voice room
    fn audio_input_sheet(&self, cx: &mut Context<Self>) -> BottomSheetUi {
        let entity = cx.entity();
        let reader = entity.clone();
        self.sheet(
            "audio-input-sheet",
            Binding::entity(entity, |this| this.audio_inputs_visible, |this, visible, cx| {
                this.audio_inputs_visible = visible;
                cx.notify();
            }),
            self.audio_input_sheet_fraction,
            move |_, cx| reader.read(cx).audio_input_sheet_content(cx),
            cx,
        )
    }

    fn audio_input_sheet_content(&self, cx: &App) -> AnyElement {
        let theme = theme(cx);
        let audio = self.audio.read(cx);
        let this = self.this.clone();
        let content = vstack(12.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .min_w_0()
            .pb(px((self.bottom_safe_inset() + 8.0).max(14.0)))
            .child(
                div()
                    .px(px(18.0))
                    .pt(px(10.0))
                    .child(MainText::new(tr("rtc_audio_input")).px(21.0).font_weight(FontWeight::BOLD)),
            )
            .child(if audio.available_inputs().is_empty() && !audio.has_system_input_choice() {
                div()
                    .w_full()
                    .px(px(18.0))
                    .child(MainText::new(tr("rtc_audio_input_unavailable")).px(14.0).color(theme.text.secondary))
                    .into_any_element()
            } else {
                GroupedBtns::new(self.scoped_id("audio-inputs"), self.audio_input_items(theme, cx)).into_any_element()
            })
            .child(RiseRTCAudioSettingsContent)
            .measured_sheet_fraction(self.scoped_id("audio-input-fraction"), 0.34, move |fraction, _, cx| {
                this.update(cx, |this, cx| {
                    if this.audio_input_sheet_fraction != fraction {
                        this.audio_input_sheet_fraction = fraction;
                        cx.notify();
                    }
                })
                .ok();
            });
        scroll_view(self.scoped_id("audio-input-scroll")).shows_indicators(false).child(content).into_any_element()
    }

    fn premium_upsell_sheet(&self, cx: &mut Context<Self>) -> Option<PremiumUpsellSheet> {
        let prompt = self.upsell.read(cx).prompt().cloned()?;
        let upsell = self.upsell.clone();
        let navigation: Option<Entity<NavigationStore>> = env::<NavigationStoreKey>(cx);
        let minimize = Self::minimize_action(self.this.clone());
        Some(PremiumUpsellSheet::new(
            self.scoped_id("upsell"),
            Binding::new(
                move |cx| upsell.read(cx).prompt().is_some(),
                |presented, _, cx| {
                    if !presented {
                        PremiumUpsellInteractionsStore::shared(cx).update(cx, |store, cx| store.dismiss(cx));
                    }
                },
            ),
            Some(prompt),
            |_, cx| PremiumUpsellInteractionsStore::shared(cx).update(cx, |store, cx| store.dismiss(cx)),
            move |window, cx| {
                PremiumUpsellInteractionsStore::shared(cx).update(cx, |store, cx| store.proceed_to_paywall(cx));
                minimize(window, cx);
                if let Some(navigation) = &navigation {
                    navigation.update(cx, |navigation, cx| {
                        navigation.push(RootRoute::SubscriptionSettings, Default::default(), cx);
                    });
                }
            },
        ))
    }

    fn stream_settings_sheet(&self, cx: &mut Context<Self>) -> BottomSheetUi {
        let entity = cx.entity();
        let reader = entity.clone();
        self.sheet(
            "stream-settings-sheet",
            Binding::entity(entity, |this| this.stream_settings_visible, |this, visible, cx| {
                this.stream_settings_visible = visible;
                cx.notify();
            }),
            self.stream_settings_sheet_fraction,
            move |_, cx| {
                let high_available =
                    interactions(cx).update(cx, |store, cx| store.is_high_stream_quality_available(cx));
                reader.read(cx).stream_settings_sheet_content(high_available, cx)
            },
            cx,
        )
        .on_did_dismiss(Self::action(self.this.clone(), |this, cx| this.apply_pending_screen_share_intent(cx)))
    }

    fn stream_settings_sheet_content(&self, high_available: bool, cx: &App) -> AnyElement {
        let theme = theme(cx);
        let this = self.this.clone();
        vstack(12.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .pb(px((self.bottom_safe_inset() + 8.0).max(14.0)))
            .child(
                div()
                    .px(px(18.0))
                    .pt(px(10.0))
                    .child(MainText::new(tr("rtc_stream_settings")).px(21.0).font_weight(FontWeight::BOLD)),
            )
            .child(
                GroupedBtns::new(self.scoped_id("screen-share-items"), self.screen_share_items(theme, cx))
                    .guide_anchor(RiseGuideAnchor::CallStreamStart),
            )
            .when(!self.displayed_screen_share_enabled(cx), |stack| {
                stack.child(
                    GroupedBtns::new(
                        self.scoped_id("stream-quality-items"),
                        self.stream_quality_items(high_available, theme),
                    )
                    .guide_anchor(RiseGuideAnchor::CallStreamQuality),
                )
            })
            .measured_sheet_fraction(self.scoped_id("stream-settings-fraction"), 0.32, move |fraction, _, cx| {
                this.update(cx, |this, cx| {
                    this.stream_settings_sheet_fraction = fraction;
                    cx.notify();
                })
                .ok();
            })
            .into_any_element()
    }

    fn encryption_sheet(&self, emojis: Vec<String>, cx: &mut Context<Self>) -> BottomSheetUi {
        let entity = cx.entity();
        let reader = entity.clone();
        self.sheet(
            "encryption-sheet",
            Binding::entity(entity, |this| this.encryption_info_visible, |this, visible, cx| {
                this.encryption_info_visible = visible;
                cx.notify();
            }),
            self.encryption_sheet_fraction,
            move |_, cx| reader.read(cx).encryption_sheet_content(&emojis, cx),
            cx,
        )
    }

    fn encryption_sheet_content(&self, emojis: &[String], cx: &App) -> AnyElement {
        let secondary = theme(cx).text.secondary;
        let this = self.this.clone();
        let body = |key: &str| {
            MainText::new(tr(key)).px(15.0).tac(TextAlignT::Center).font_weight(FontWeight::MEDIUM).color(secondary)
        };
        vstack(16.0)
            .w_full()
            .px(px(22.0))
            .pb(px((self.bottom_safe_inset() + 10.0).max(18.0)))
            .child(hstack(10.0).pt(px(16.0)).children(emojis.iter().map(|emoji| MainText::new(emoji.clone()).px(38.0))))
            .child(MainText::new(tr("rtc_encryption_title")).px(21.0).tac(TextAlignT::Center).font_weight(FontWeight::BOLD))
            .child(body("rtc_encryption_body"))
            .child(body("rtc_encryption_compare"))
            .guide_anchor(RiseGuideAnchor::CallEncryptionSheet)
            .measured_sheet_fraction(self.scoped_id("encryption-fraction"), 0.3, move |fraction, _, cx| {
                this.update(cx, |this, cx| {
                    this.encryption_sheet_fraction = fraction;
                    cx.notify();
                })
                .ok();
            })
            .into_any_element()
    }

    fn screen_share_items(&self, theme: &AppTheme, cx: &App) -> Vec<GroupedBtn> {
        let enabled = self.displayed_screen_share_enabled(cx);
        vec![
            GroupedBtn::new("screen-share", tr(if enabled { "rtc_stop_stream" } else { "rtc_start_stream" }))
                .icon(if enabled { "rectangle.on.rectangle.slash" } else { "rectangle.on.rectangle" })
                .icon_background(if enabled { theme.semantic.error_100 } else { RiseRTCSettingsAccent::screen() })
                .icon_foreground(white())
                .show_chevron(false)
                .callback(Self::action(self.this.clone(), |this, cx| {
                    this.pending_screen_share_intent = Some(!this.displayed_screen_share_enabled(cx));
                    this.stream_settings_visible = false;
                    cx.notify();
                })),
        ]
    }

    fn stream_quality_items(&self, high_available: bool, theme: &AppTheme) -> Vec<GroupedBtn> {
        [
            (ScreenShareQuality::Balanced, tr("rtc_stream_balanced"), tr("rtc_stream_balanced_description")),
            (ScreenShareQuality::High, tr("rtc_stream_high"), tr("rtc_stream_high_description")),
        ]
        .into_iter()
        .map(|(quality, title, detail)| {
            let selected = self.stream_quality == quality;
            let locked = quality == ScreenShareQuality::High && !high_available;
            let mut item = GroupedBtn::new(format!("quality-{title}"), title)
                .icon(if quality == ScreenShareQuality::High { "sparkles" } else { "gauge" })
                .icon_background(RiseRTCSettingsAccent::quality())
                .icon_foreground(white())
                .subtitle(detail)
                .accessibility_selected(selected)
                .show_chevron(false)
                .callback(Self::action(self.this.clone(), move |this, cx| {
                    if interactions(cx).update(cx, |store, cx| store.select_stream_quality(quality, cx)) {
                        this.stream_quality = quality;
                        cx.notify();
                    }
                }));
            if let Some(trailing) = Self::stream_quality_trailing(selected, locked, theme) {
                item = item.trailing_slot(trailing);
            }
            item
        })
        .collect()
    }

    fn stream_quality_trailing(selected: bool, locked: bool, theme: &AppTheme) -> Option<AnyElement> {
        if locked {
            return Some(PremiumDiamond::new().size(20.0).looping(false).into_any_element());
        }
        if !selected {
            return None;
        }
        Some(div().children(symbol("checkmark", 15.0, theme.primary._100)).into_any_element())
    }

    fn audio_input_items(&self, theme: &AppTheme, cx: &App) -> Vec<GroupedBtn> {
        let audio = self.audio.read(cx);
        let system_choice = audio.has_system_input_choice().then(|| {
            let selected = audio.selected_input_id().is_none();
            let mut item = GroupedBtn::new("audio-input-automatic".to_owned(), tr("rtc_audio_automatic"))
                .icon("mic.fill")
                .icon_background(RiseRTCSettingsAccent::microphone())
                .icon_foreground(white())
                .accessibility_selected(selected)
                .show_chevron(false)
                .callback(Self::action(self.this.clone(), |this, cx| this.select_audio_input(None, cx)));
            if selected {
                item = item.trailing_slot(div().children(symbol("checkmark", 15.0, theme.primary._100)));
            }
            item
        });
        let devices = audio
            .available_inputs()
            .iter()
            .map(|route| {
                let selected = audio.selected_input_id() == Some(route.id.as_str());
                let route_id = route.id.clone();
                let mut item = GroupedBtn::new(format!("audio-input-{}", route.id), route.display_name())
                    .icon(Self::audio_route_icon(&route.port_type, "mic.fill"))
                    .icon_background(RiseRTCSettingsAccent::microphone())
                    .icon_foreground(white())
                    .accessibility_selected(selected)
                    .show_chevron(false)
                    .callback(Self::action(self.this.clone(), move |this, cx| {
                        this.select_audio_input(Some(route_id.clone()), cx);
                    }));
                if selected {
                    item = item.trailing_slot(div().children(symbol("checkmark", 15.0, theme.primary._100)));
                }
                item
            });
        system_choice.into_iter().chain(devices).collect()
    }

    fn audio_route_sheet(&self, cx: &mut Context<Self>) -> BottomSheetUi {
        let entity = cx.entity();
        let reader = entity.clone();
        self.sheet(
            "audio-route-sheet",
            Binding::entity(entity, |this| this.audio_routes_visible, |this, visible, cx| {
                this.audio_routes_visible = visible;
                cx.notify();
            }),
            self.audio_route_sheet_fraction,
            move |_, cx| reader.read(cx).audio_route_sheet_content(cx),
            cx,
        )
    }

    fn audio_route_sheet_content(&self, cx: &App) -> AnyElement {
        let theme = theme(cx);
        let audio = self.audio.read(cx);
        let selected_output_id = audio.selected_output_id();
        let outputs = audio.available_outputs();
        let this = self.this.clone();
        let device_selected = !outputs.iter().any(|route| Some(route.id.as_str()) == selected_output_id);
        let mut routes = vstack(0.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .bg(theme.bg._200)
            .clip_shape(Shape::continuous(18.0))
            .child(self.audio_route_button(
                "audio-route-device",
                tr("rtc_audio_automatic"),
                "speaker.wave.2.fill",
                device_selected,
                Self::action(this.clone(), |this, cx| this.select_audio_output(None, cx)),
                theme,
            ));
        for route in outputs {
            let route_id = route.id.clone();
            routes = routes.child(Self::audio_route_divider(theme)).child(self.audio_route_button(
                format!("audio-route-{}", route.id),
                route.name.clone(),
                "speaker.wave.2.fill",
                selected_output_id == Some(route.id.as_str()),
                Self::action(this.clone(), move |this, cx| this.select_audio_output(Some(route_id.clone()), cx)),
                theme,
            ));
        }
        vstack(10.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .pb(px((self.bottom_safe_inset() + 8.0).max(14.0)))
            .child(
                div()
                    .px(px(18.0))
                    .pt(px(10.0))
                    .child(MainText::new(tr("rtc_audio_output")).px(21.0).font_weight(FontWeight::BOLD)),
            )
            .child(div().w_full().px(px(14.0)).child(routes))
            .measured_sheet_fraction(self.scoped_id("audio-route-fraction"), 0.24, move |fraction, _, cx| {
                this.update(cx, |this, cx| {
                    this.audio_route_sheet_fraction = fraction;
                    cx.notify();
                })
                .ok();
            })
            .into_any_element()
    }

    fn floating_window(
        &self,
        id: &'static str,
        position: UnitPoint,
        geometry: Size<f32>,
        on_position_change: impl Fn(&mut Self, UnitPoint) + 'static,
        on_tap: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        content: impl IntoElement,
    ) -> AnyElement {
        let this = self.this.clone();
        RiseRTCFloatingVideoWindow::new(
            self.scoped_id(id),
            position,
            self.floating_window_size(geometry),
            self.floating_window_insets(),
            geometry,
            move |position, _, cx| {
                this.update(cx, |this, cx| {
                    on_position_change(this, position);
                    cx.notify();
                })
                .ok();
            },
            Some(Rc::new(Self::action(self.this.clone(), on_tap))),
            content,
        )
        .into_any_element()
    }

    fn remote_floating_window(&self, surface: RemoteSurfacePlaceholder, geometry: Size<f32>, cx: &App) -> AnyElement {
        let content = match self.remote_track(surface, cx) {
            Some(track) => RiseRTCVideoSurface::new(
                self.scoped_id("remote-window-surface"),
                "remote-window",
                track,
                if surface == RemoteSurfacePlaceholder::ScreenShare {
                    VideoViewLayoutMode::Fit
                } else {
                    VideoViewLayoutMode::Fill
                },
                VideoViewMirrorMode::Off,
            )
            .into_any_element(),
            None => self.surface_loader(surface, true).into_any_element(),
        };
        self.floating_window(
            "remote-window",
            self.remote_window_position,
            geometry,
            |this, position| this.remote_window_position = position,
            |this, cx| {
                this.with_animation(spring(0.42, 0.86));
                this.remote_media_swapped = !this.remote_media_swapped;
                this.selected_local_surface = None;
                cx.notify();
            },
            div().size_full().child(content),
        )
    }

    /// the peer occupies the window vacated by a promoted local publication
    fn displaced_peer_window(
        &self,
        geometry: Size<f32>,
        local_surface: RemoteSurfacePlaceholder,
        cx: &App,
    ) -> AnyElement {
        let is_camera = local_surface == RemoteSurfacePlaceholder::Camera;
        let content = match self.remote_primary_surface(cx) {
            Some(surface) => match self.remote_track(surface, cx) {
                Some(track) => RiseRTCVideoSurface::new(
                    self.scoped_id("displaced-surface"),
                    "remote-displaced",
                    track,
                    if surface == RemoteSurfacePlaceholder::ScreenShare {
                        VideoViewLayoutMode::Fit
                    } else {
                        VideoViewLayoutMode::Fill
                    },
                    VideoViewMirrorMode::Off,
                )
                .into_any_element(),
                None => self.surface_loader(surface, true).into_any_element(),
            },
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    UserLogoUi::new(self.scoped_id("displaced-logo"))
                        .source(self.counterpart_avatar_url(cx).map(SharedString::from))
                        .name(Some(SharedString::from(self.counterpart_name(cx))))
                        .user_id(self.counterpart_user_id(cx).map(SharedString::from))
                        .size(64.0)
                        .show_story_ring(false)
                        .image_cache_policy(CleverImageCachePolicy::MemoryOnly),
                )
                .into_any_element(),
        };
        self.floating_window(
            if is_camera { "displaced-camera-window" } else { "displaced-screen-window" },
            if is_camera { self.local_window_position } else { self.local_screen_window_position },
            geometry,
            move |this, position| {
                if is_camera {
                    this.local_window_position = position;
                } else {
                    this.local_screen_window_position = position;
                }
            },
            |this, cx| this.select_local_surface(None, cx),
            div().size_full().child(content),
        )
    }

    fn local_screen_floating_window(&self, geometry: Size<f32>, cx: &App) -> AnyElement {
        let content = match self.local_screen_track(cx) {
            Some(_) => RiseRTCOwnScreenShareWall::new(self.scoped_id("local-screen-wall"), theme(cx).clone())
                .compact(true)
                .into_any_element(),
            None => self.local_screen_placeholder(true).into_any_element(),
        };
        self.floating_window(
            "local-screen-window",
            self.local_screen_window_position,
            geometry,
            |this, position| this.local_screen_window_position = position,
            |this, cx| this.select_local_surface(Some(RemoteSurfacePlaceholder::ScreenShare), cx),
            div()
                .relative()
                .size_full()
                .child(content)
                .child(
                    div()
                        .absolute()
                        .top(px(6.0))
                        .left(px(6.0))
                        .p(px(5.0))
                        .rounded_full()
                        .bg(black().opacity(0.46))
                        .children(symbol("display", 11.0, white())),
                )
                .guide_anchor(RiseGuideAnchor::CallLocalScreenWindow),
        )
    }

    /// compact but not silent
    /// unlike the peer loader (surface promised within 15s) this covers a wait on the system picker
    /// up to a minute outside the app, and a window with only a spinner got reported as a hang
    fn local_screen_placeholder(&self, compact: bool) -> Div {
        let tint = white().opacity(0.72);
        let title =
            if self.awaiting_broadcast_confirmation() { tr("rtc_screen_share_confirm") } else { tr("rtc_screen_share_loading") };
        div().size_full().flex().items_center().justify_center().when(!compact, |fill| fill.bg(black())).child(
            vstack(if compact { 7.0 } else { 14.0 })
                .w_full()
                .p(px(if compact { 6.0 } else { 0.0 }))
                .children(symbol("display", if compact { 18.0 } else { 34.0 }, tint))
                .child(
                    div().w_full().child(
                        MainText::new(title)
                            .px(if compact { 11.0 } else { 15.0 })
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::MEDIUM)
                            .color(tint)
                            .number_of_lines(if compact { 2 } else { 0 }),
                    ),
                )
                .child(progress_view().tint(tint).control_size(if compact { ControlSize::Small } else { ControlSize::Regular })),
        )
    }

    fn awaiting_broadcast_confirmation(&self) -> bool {
        self.pending_media.screen_share == Some(true)
    }

    fn local_camera_floating_window(&self, geometry: Size<f32>, cx: &App) -> AnyElement {
        let content = match self.local_camera_track(cx) {
            Some(track) => RiseRTCVideoSurface::new(
                self.scoped_id("local-camera-surface"),
                "local-camera",
                track,
                VideoViewLayoutMode::Fill,
                VideoViewMirrorMode::Auto,
            )
            .into_any_element(),
            None => self.surface_loader(RemoteSurfacePlaceholder::Camera, true).into_any_element(),
        };
        self.floating_window(
            "local-camera-window",
            self.local_window_position,
            geometry,
            |this, position| this.local_window_position = position,
            |this, cx| this.select_local_surface(Some(RemoteSurfacePlaceholder::Camera), cx),
            div().size_full().child(content).guide_anchor(RiseGuideAnchor::CallLocalWindow),
        )
    }

    fn audio_route_button(
        &self,
        id: impl Into<SharedString>,
        title: String,
        icon: &'static str,
        selected: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
        theme: &AppTheme,
    ) -> impl IntoElement {
        plain(self.scoped_id(id)).w_full().on_press(action).child(
            hstack(12.0)
                .w_full()
                .h(px(54.0))
                .px(px(15.0))
                .child(
                    div()
                        .w(px(28.0))
                        .flex()
                        .justify_center()
                        .children(symbol(icon, 17.0, theme.text.primary)),
                )
                .child(MainText::new(title).px(16.0).font_weight(FontWeight::MEDIUM).number_of_lines(1))
                .child(spacer_min_w(8.0))
                .when(selected, |row| row.children(symbol("checkmark", 14.0, theme.primary._100))),
        )
    }

    fn audio_route_divider(theme: &AppTheme) -> impl IntoElement {
        div().w_full().pl(px(55.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.45)))
    }

    fn select_audio_output(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.audio_routes_visible = false;
        cx.notify();
        let task = interactions(cx).update(cx, |store, cx| store.select_audio_output(id, cx));
        self.report_route_failure(task, cx);
    }

    fn select_audio_input(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.audio_routes_visible = false;
        cx.notify();
        let task = interactions(cx).update(cx, |store, cx| store.select_audio_input(id, cx));
        self.report_route_failure(task, cx);
    }

    fn report_route_failure(&self, task: Task<anyhow::Result<()>>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let Err(error) = task.await else {
                return;
            };
            if error.is::<CancellationError>() {
                return;
            }
            this.update(cx, |this, cx| this.show_action_failure(None, cx)).ok();
        })
        .detach();
    }

    fn audio_route_icon(port_type: &str, fallback: &'static str) -> &'static str {
        let normalized = port_type.to_lowercase();
        if normalized.contains("bluetooth") {
            return "airpodspro";
        }
        if normalized.contains("headphone") || normalized.contains("headset") || normalized.contains("usb") {
            return "headphones";
        }
        fallback
    }

    fn flush_pending_media(&mut self, cx: &mut Context<Self>) {
        if !self.media_ready(cx) {
            return;
        }

        if let Some(enabled) = self.pending_media.microphone
            && !self.media_action_ids.contains("microphone")
        {
            let claimed = self.pending_media;
            let system_call = self.system_call(cx);
            let call_accepted = self.call(cx).is_some_and(|call| call.state == CallState::Accepted);
            let resolved_server_call_id = self.resolved_server_call_id(cx);
            self.run_media_action(
                "microphone",
                claimed,
                !enabled,
                None,
                move |cx| {
                    cx.spawn(async move |_, cx| {
                        if let Some(system_call) = &system_call
                            && system_call.phase == RiseRTCSystemCallPhase::Active
                        {
                            let call_uuid = system_call.local_uuid;
                            let request = cx.update(|cx| {
                                interactions(cx)
                                    .update(cx, |store, cx| store.request_system_call_muted_call_uuid(!enabled, call_uuid, cx))
                            });
                            match request.await {
                                Ok(()) => return Ok(()),
                                Err(error) if error.is::<CancellationError>() => return Err(error),
                                Err(_) => {}
                            }
                        }
                        if system_call.is_none()
                            && call_accepted
                            && let Some(server_call_id) = &resolved_server_call_id
                        {
                            let request = cx.update(|cx| {
                                interactions(cx).update(cx, |store, cx| {
                                    store.request_system_call_muted_server_call_id(!enabled, server_call_id, cx)
                                })
                            });
                            match request.await {
                                Ok(true) => return Ok(()),
                                Ok(false) => {}
                                Err(error) => return Err(error),
                            }
                        }
                        let request =
                            cx.update(|cx| interactions(cx).update(cx, |store, cx| store.set_microphone(enabled, cx)));
                        request.await
                    })
                },
                cx,
            );
        }

        if let Some(enabled) = self.pending_media.camera
            && !self.media_action_ids.contains("camera")
        {
            let claimed = self.pending_media;
            let position = self.camera_position(cx);
            self.run_media_action(
                "camera",
                claimed,
                !enabled,
                None,
                move |cx| interactions(cx).update(cx, |store, cx| store.set_camera(enabled, position, cx)),
                cx,
            );
        }

        if let Some(enabled) = self.pending_media.screen_share
            && !self.media_action_ids.contains("screen-share")
        {
            let claimed = self.pending_media;
            let includes_audio = self.stream_includes_audio;
            let quality = self.stream_quality;
            self.run_media_action(
                "screen-share",
                claimed,
                !enabled,
                Some(tr("rtc_media_screen_share_failed")),
                move |cx| interactions(cx).update(cx, |store, cx| store.set_screen_share(enabled, includes_audio, quality, cx)),
                cx,
            );
        }

        if let Some(deafened) = self.pending_media.deafened
            && !self.media_action_ids.contains("headphones")
        {
            let claimed = self.pending_media;
            self.run_media_action(
                "headphones",
                claimed,
                deafened,
                None,
                move |cx| interactions(cx).update(cx, |store, cx| store.set_deafened(deafened, cx)),
                cx,
            );
        }
    }

    fn synchronize_call_kit_mute_state(&mut self, cx: &mut Context<Self>) {
        let Some(system_call) = self.system_call(cx) else {
            return;
        };
        if system_call.phase != RiseRTCSystemCallPhase::Active || self.self_participant(cx).is_none() {
            return;
        }
        let is_muted = !self.displayed_microphone_enabled(cx);
        interactions(cx)
            .update(cx, |store, cx| store.request_system_call_muted_call_uuid(is_muted, system_call.local_uuid, cx))
            .detach();
    }

    fn run_media_action(
        &mut self,
        id: &'static str,
        claimed: PendingMediaToggles,
        keeps_optimistic_on_failure: bool,
        failure_title: Option<String>,
        operation: impl FnOnce(&mut Context<Self>) -> Task<anyhow::Result<()>>,
        cx: &mut Context<Self>,
    ) {
        if !self.media_action_ids.insert(id.to_owned()) {
            return;
        }
        RiseRTCCallTrace::note_media_mutation(id, !keeps_optimistic_on_failure);
        cx.notify();
        let operation = operation(cx);
        cx.spawn(async move |this, cx| {
            let result = operation.await;
            this.update(cx, |this, cx| {
                let mut settled = true;
                match result {
                    Ok(()) => this.clear_pending_media(claimed),
                    Err(error) if error.is::<CancellationError>() => this.clear_pending_media(claimed),
                    Err(error) => {
                        if keeps_optimistic_on_failure {
                            settled = false;
                        } else {
                            this.clear_pending_media(claimed);
                        }
                        if let Some(RiseRTCStateError::Remote(code)) = error.downcast_ref::<RiseRTCStateError>()
                            && (code == RiseRTCDeviceRefusal::MICROPHONE
                                || code == RiseRTCDeviceRefusal::MICROPHONE_RESTRICTED)
                        {
                            this.microphone_refused = true;
                        }
                        if let Some(text) = RiseRTCDomain::desktop_failure_text(&error) {
                            this.show_action_failure(Some(text), cx);
                        } else if !keeps_optimistic_on_failure && !RiseRTCDeviceRefusal::present_settings_alert(&error, cx) {
                            this.show_action_failure(failure_title, cx);
                        }
                    }
                }
                this.media_action_ids.remove(id);
                if settled {
                    this.flush_pending_media(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn clear_pending_media(&mut self, claimed: PendingMediaToggles) {
        if claimed.microphone.is_some() && self.pending_media.microphone == claimed.microphone {
            self.pending_media.microphone = None;
        }
        if claimed.camera.is_some() && self.pending_media.camera == claimed.camera {
            self.pending_media.camera = None;
        }
        if claimed.screen_share.is_some() && self.pending_media.screen_share == claimed.screen_share {
            self.pending_media.screen_share = None;
        }
        if claimed.deafened.is_some() && self.pending_media.deafened == claimed.deafened {
            self.pending_media.deafened = None;
        }
    }

    fn reconcile_pending_media(&mut self, cx: &mut Context<Self>) {
        let Some(participant) = self.self_participant(cx).cloned() else {
            return;
        };
        if self.pending_media.microphone == Some(participant.microphone_enabled) {
            self.pending_media.microphone = None;
        }
        if self.pending_media.camera == Some(participant.camera_enabled) {
            self.pending_media.camera = None;
        }
        if self.pending_media.screen_share == Some(participant.screen_share_enabled) {
            self.pending_media.screen_share = None;
        }
        if self.pending_media.deafened == Some(participant.deafened) {
            self.pending_media.deafened = None;
        }
        cx.notify();
    }

    fn toggle_recording(&mut self, cx: &mut Context<Self>) {
        let Some(call) = self.call(cx) else {
            return;
        };
        if self.recording_busy {
            return;
        }
        let turn_on = !call.is_recording();
        if turn_on && !self.auth.read(cx).profile().is_some_and(|profile| profile.more.is_premium) {
            self.upsell.update(cx, |upsell, cx| {
                upsell.offer_premium_feature(tr("premium_upsell_reason_call_recording"), "call_recording", false, cx);
            });
            return;
        }
        self.recording_busy = true;
        cx.notify();
        let _ = impact(ImpactStyle::Medium);
        let request = interactions(cx).update(cx, |store, cx| store.set_call_recording(&call.call_id, turn_on, cx));
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |this, cx| {
                this.recording_busy = false;
                cx.notify();
                let Err(error) = result else {
                    return;
                };
                if error.is::<CancellationError>() {
                    return;
                }
                let message = format!("call recording toggle failed: {error}");
                Sai::log("RTC", move || message, SaiLogKind::Network, cx);
                if this.upsell.read(cx).prompt().is_none() {
                    this.show_action_failure(None, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn end_call(&mut self, cx: &mut Context<Self>) {
        if self.is_ending || self.visual_state(cx).is_terminal() {
            return;
        }
        let expected_identity = self.presentation_identity(cx);
        let expected_account_id = self.current_user_id(cx);
        let expected_account_epoch = normalized(SaiAccountCacheScope::active_epoch().as_deref());
        let system_call = self.system_call(cx);
        let call = self.call(cx);
        let was_accepted = call.as_ref().is_some_and(|call| call.state == CallState::Accepted)
            || system_call.as_ref().is_some_and(|call| call.connected_at.is_some())
            || self.peer_first_joined_at.is_some();
        let was_incoming = system_call.as_ref().is_some_and(|call| call.direction == RiseRTCSystemCallDirection::Incoming)
            || call.as_ref().is_some_and(|call| call.callee_user_id == expected_account_id);
        let completed_state = if was_accepted {
            VisualState::Ended
        } else if was_incoming {
            VisualState::Declined
        } else {
            VisualState::Cancelled
        };
        self.microphone_shown_at_hang_up = Some(self.displayed_microphone_enabled(cx));
        self.is_ending = true;
        self.pending_media.microphone = Some(false);
        cx.notify();

        let call_uuid = system_call.map(|call| call.local_uuid);
        let resolved_server_call_id = self.resolved_server_call_id(cx);
        cx.spawn(async move |this, cx| {
            let result = Self::perform_hang_up(
                &this,
                call_uuid,
                resolved_server_call_id,
                &expected_identity,
                was_accepted,
                was_incoming,
                cx,
            )
            .await;
            this.update(cx, |this, cx| {
                if this.presentation_identity(cx) != expected_identity
                    || this.current_user_id(cx) != expected_account_id
                    || normalized(SaiAccountCacheScope::active_epoch().as_deref()) != expected_account_epoch
                {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.local_end_presentation = Some(LocalEndPresentation {
                            identity: expected_identity,
                            account_id: expected_account_id,
                            account_epoch: expected_account_epoch,
                            state: completed_state,
                        });
                        this.is_ending = false;
                        cx.notify();
                        this.settle_terminal_presentation(completed_state.terminal_hold(), false, cx);
                    }
                    Err(error) if error.is::<CancellationError>() => this.restore_microphone_after_failed_hang_up(cx),
                    Err(_) => {
                        this.restore_microphone_after_failed_hang_up(cx);
                        this.show_action_failure(None, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    async fn perform_hang_up(
        this: &WeakEntity<Self>,
        call_uuid: Option<Uuid>,
        resolved_server_call_id: Option<String>,
        identity: &str,
        was_accepted: bool,
        was_incoming: bool,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<()> {
        if let Some(call_uuid) = call_uuid {
            let request =
                cx.update(|cx| interactions(cx).update(cx, |store, cx| store.request_system_call_end_call_uuid(call_uuid, cx)));
            match request.await {
                Ok(()) => Self::play_end_tone_after_system_release(identity, cx).await,
                Err(error) if error.is::<CancellationError>() => return Err(error),
                Err(error) => {
                    let Some(server_call_id) = resolved_server_call_id else {
                        return Err(error);
                    };
                    cx.update(|cx| Self::play_end_tone(identity, false, cx));
                    Self::end_domain_call(this, &server_call_id, was_accepted, was_incoming, cx).await?;
                }
            }
        } else if let Some(server_call_id) = resolved_server_call_id {
            let request = cx.update(|cx| {
                interactions(cx).update(cx, |store, cx| store.request_system_call_end_server_call_id(&server_call_id, cx))
            });
            if request.await? {
                Self::play_end_tone_after_system_release(identity, cx).await;
            } else {
                cx.update(|cx| Self::play_end_tone(identity, false, cx));
                Self::end_domain_call(this, &server_call_id, was_accepted, was_incoming, cx).await?;
            }
        } else {
            cx.update(|cx| Self::play_end_tone(identity, false, cx));
        }
        Ok(())
    }

    fn play_end_tone(identity: &str, activates_audio_session: bool, cx: &mut App) {
        RiseRTCEndTonePlayer::shared(cx).update(cx, |player, cx| player.play_once(identity, activates_audio_session, cx));
    }

    /// hanging up silences the mic first, if the call stays up the
    /// user never touched the switch so it goes back to what it showed
    fn restore_microphone_after_failed_hang_up(&mut self, cx: &mut Context<Self>) {
        let shown = self.microphone_shown_at_hang_up.take();
        self.is_ending = false;
        cx.notify();
        let Some(shown) = shown else {
            return;
        };
        if self.pending_media.microphone != Some(false) || self.visual_state(cx).is_terminal() {
            return;
        }
        self.pending_media.microphone = Some(shown);
        self.flush_pending_media(cx);
    }

    fn answer_call(&mut self, cx: &mut Context<Self>) {
        if self.is_answering || self.is_ending {
            return;
        }
        let Some(resolved_server_call_id) = self.resolved_server_call_id(cx) else {
            return;
        };
        self.is_answering = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<()> = async {
                let request = cx.update(|cx| {
                    interactions(cx).update(cx, |store, cx| store.request_system_call_answer(&resolved_server_call_id, cx))
                });
                if request.await? {
                    return Ok(());
                }
                let installation_id = RiseInstallationIdentity::current()?;
                let request = cx.update(|cx| {
                    interactions(cx)
                        .update(cx, |store, cx| store.answer_call(&resolved_server_call_id, None, &installation_id, cx))
                });
                request.await?;
                Ok::<(), anyhow::Error>(())
            }
            .await;
            this.update(cx, |this, cx| {
                this.is_answering = false;
                cx.notify();
                if let Err(error) = result
                    && !error.is::<CancellationError>()
                {
                    this.show_action_failure(None, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    async fn play_end_tone_after_system_release(identity: &str, cx: &mut gpui::AsyncApp) {
        cx.background_executor().timer(Duration::from_millis(120)).await;
        cx.update(|cx| Self::play_end_tone(identity, true, cx));
    }

    /// before the call record has loaded what the screen saw decides
    /// unanswered incoming is declined and unanswered outgoing cancelled, server wont end a ringing call
    /// without a connection the screen still closes and the hangup is sent on reconnect
    async fn end_domain_call(
        this: &WeakEntity<Self>,
        call_id: &str,
        was_accepted: bool,
        was_incoming: bool,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<()> {
        let (call, current_user_id) = this.update(cx, |this, cx| (this.call(cx), this.current_user_id(cx)))?;
        let ringing = call.as_ref().map_or(!was_accepted, |call| call.state == CallState::Ringing);
        let is_caller = call.as_ref().map_or(!was_incoming, |call| call.caller_user_id == current_user_id);
        let action = if !ringing {
            RiseRTCHangupAction::End
        } else if is_caller {
            RiseRTCHangupAction::Cancel
        } else {
            RiseRTCHangupAction::Decline
        };
        let reason = match action {
            RiseRTCHangupAction::End => "client_hangup",
            RiseRTCHangupAction::Cancel => "client_cancelled",
            RiseRTCHangupAction::Decline => "client_declined",
        };
        let idempotency_key = Uuid::new_v4().to_string().to_lowercase();
        let request = cx.update(|cx| {
            interactions(cx).update(cx, |store, cx| {
                let reason = Some(reason.to_owned());
                let key = Some(idempotency_key.clone());
                match action {
                    RiseRTCHangupAction::End => store.end_call(call_id, reason, key, cx),
                    RiseRTCHangupAction::Cancel => store.cancel_call(call_id, reason, key, cx),
                    RiseRTCHangupAction::Decline => store.decline_call(call_id, reason, key, cx),
                }
            })
        });
        match request.await {
            Ok(_) => Ok(()),
            Err(error) if RiseRTCConnectionWaitPolicy::signalling_failure(&error).is_some() => {
                let deliver = cx.update(|cx| {
                    interactions(cx).update(cx, |store, cx| {
                        store.deliver_hangup_when_online(action, call_id, reason.to_owned(), idempotency_key.clone(), cx)
                    })
                });
                deliver.await
            }
            Err(error) => Err(error),
        }
    }

    fn restored_connection_task_identity(&self, cx: &App) -> String {
        let owner = if self.system_call(cx).is_none() { "restored" } else { "system" };
        let link = if self.is_waiting_for_network(cx) { "offline" } else { "online" };
        format!("{}|{owner}|{link}", self.resolved_server_call_id(cx).unwrap_or_else(|| "none".to_owned()))
    }

    fn connect_restored_call_if_needed(&mut self, cx: &mut Context<Self>) -> Option<Task<anyhow::Result<RiseRTCDirectCall>>> {
        if self.system_call(cx).is_some() {
            return None;
        }
        let resolved_server_call_id = self.resolved_server_call_id(cx)?;
        let call = self.call(cx);
        let connection_state = self.rtc.read(cx).connection_state(cx);
        if call.as_ref().is_some_and(|call| call.state.is_terminal())
            || connection_state == ConnectionState::Connecting
            || connection_state == ConnectionState::Reconnecting
        {
            return None;
        }
        if let Some(call) = &call
            && self.room(cx).is_some_and(|room| room.config.room_id == call.room_id)
            && connection_state != ConnectionState::Failed
            && connection_state != ConnectionState::Idle
        {
            return None;
        }
        let installation_id = RiseInstallationIdentity::current().ok();
        Some(interactions(cx).update(cx, |store, cx| store.rejoin_call(&resolved_server_call_id, installation_id, cx)))
    }

    fn settle_terminal_presentation(&mut self, hold: Duration, owned_by_state: bool, cx: &mut Context<Self>) {
        if self.terminal_dismiss_scheduled {
            return;
        }
        self.terminal_dismiss_scheduled = true;
        self.terminal_hold_owned_by_state = owned_by_state;
        self.terminal_hold_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(hold).await;
            let Ok((Some(on_terminal_settled), Some(window_handle))) =
                this.read_with(cx, |this, _| (this.on_terminal_settled.clone(), this.window_handle))
            else {
                return;
            };
            cx.update_window(window_handle, |_, window, cx| on_terminal_settled(window, cx)).ok();
        }));
    }

    fn announce_media_failure(&mut self, code: Option<&str>, cx: &mut Context<Self>) {
        let Some(code) = code else {
            return;
        };
        if code == RiseRTCDomain::CAMERA_PUBLISH_FAILED_CODE {
            self.show_action_failure(Some(tr("rtc_media_camera_failed")), cx);
        } else if code == RiseRTCDomain::MICROPHONE_PUBLISH_FAILED_CODE {
            self.show_action_failure(Some(tr("rtc_media_microphone_failed")), cx);
        } else if code == RiseRTCDeviceRefusal::MICROPHONE || code == RiseRTCDeviceRefusal::MICROPHONE_RESTRICTED {
            self.microphone_refused = true;
            cx.notify();
            let error = anyhow::Error::new(RiseRTCStateError::Remote(code.to_owned()));
            RiseRTCDeviceRefusal::present_settings_alert(&error, cx);
        }
    }

    fn show_action_failure(&self, title: Option<String>, cx: &mut App) {
        Notifier::send(
            title.unwrap_or_else(|| tr("rtc_action_failed")),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..Default::default()
            },
            cx,
        );
    }

    fn apply_pending_screen_share_intent(&mut self, cx: &mut Context<Self>) {
        let Some(intent) = self.pending_screen_share_intent.take() else {
            return;
        };
        self.pending_media.screen_share = Some(intent);
        self.flush_pending_media(cx);
        cx.notify();
    }

    /// no socket means no signalling and basically no network,
    /// connecting would promise progress that isnt happening
    fn is_waiting_for_network(&self, cx: &App) -> bool {
        !self.socket.read(cx).is_connected()
    }

    fn status_text(&self, state: VisualState, cx: &App) -> String {
        match state {
            VisualState::Request => tr("rtc_call_request"),
            VisualState::Ringing => tr("rtc_call_ringing"),
            VisualState::Incoming => tr("rtc_call_incoming"),
            VisualState::Connecting => {
                if self.is_waiting_for_network(cx) {
                    tr("rtc_waiting_for_network")
                } else {
                    tr("rtc_connecting")
                }
            }
            VisualState::Connected(_) => tr("rtc_call_connected"),
            VisualState::Reconnecting => {
                if self.is_waiting_for_network(cx) {
                    tr("rtc_waiting_for_network")
                } else {
                    tr("rtc_reconnecting")
                }
            }
            VisualState::Ending => tr("rtc_call_ending"),
            VisualState::Ended => tr("rtc_call_ended"),
            VisualState::Declined => tr("rtc_call_declined"),
            VisualState::Cancelled => tr("rtc_call_cancelled"),
            VisualState::TimedOut => tr("rtc_call_timed_out"),
            VisualState::Failed => tr("rtc_call_failed"),
            VisualState::AwaitingPeer => tr("rtc_call_awaiting_peer"),
        }
    }

    fn status_color(state: VisualState) -> Hsla {
        match state {
            VisualState::Connected(_) => connected_green(),
            VisualState::Failed | VisualState::Declined | VisualState::Cancelled | VisualState::TimedOut => {
                white().opacity(0.78)
            }
            VisualState::Request
            | VisualState::Ringing
            | VisualState::Incoming
            | VisualState::Connecting
            | VisualState::Reconnecting
            | VisualState::Ending
            | VisualState::Ended
            | VisualState::AwaitingPeer => white().opacity(0.9),
        }
    }

    fn bottom_safe_inset(&self) -> f32 {
        RiseRTCScreenSafeArea::insets().bottom
    }

    fn top_safe_inset(&self) -> f32 {
        self.top_safe_inset
    }

    fn horizontal_safe_inset(&self) -> f32 {
        RiseRTCScreenSafeArea::horizontal()
    }
}

impl Render for RiseRTCDirectCallScreenState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.window_handle = Some(window.window_handle());
        self.top_safe_inset = RiseRTCScreenSafeArea::top(cx);
        let state = self.visual_state(cx);
        let chrome_off = self.is_chrome_hidden(state, cx);
        RiseRTCCallTrace::note_body_pass(
            &state.task_identity(),
            self.remote_primary_surface(cx).is_some(),
            self.remote_surfaces(cx).len(),
        );
        self.media_fingerprint = self.current_media_fingerprint(cx);
        self.sync_transitions(state, cx);
        self.install_observers(state, window, cx);

        let now = RiseClock::monotonic();
        if self.readability_gradient_opacity.is_running(now) {
            window.request_animation_frame();
        }
        if self.geometry_size.width <= 0.0 {
            let viewport = window.viewport_size();
            self.geometry_size = size(f32::from(viewport.width), f32::from(viewport.height));
        }
        let geometry = self.geometry_size;
        let has_primary_surface = self.has_primary_surface(cx);
        let weak = self.this.clone();
        let measured = measure(self.scoped_id("geometry"), move |bounds, _, cx| {
            let measured = size(f32::from(bounds.size.width), f32::from(bounds.size.height));
            weak.update(cx, |this, cx| {
                if this.geometry_size != measured {
                    this.geometry_size = measured;
                    cx.notify();
                }
            })
            .ok();
        });

        let weak = self.this.clone();
        let controls_measured = measure(self.scoped_id("controls-geometry"), move |bounds, _, cx| {
            let measured = f32::from(bounds.size.height);
            weak.update(cx, |this, cx| {
                if (this.controls_height - measured).abs() > 0.5 {
                    this.controls_height = measured;
                    cx.notify();
                }
            })
            .ok();
        });
        let lower_spacer_min = RiseRTCScreenSafeArea::DOCK_CONTENT_GAP.max(self.bottom_floating_window_reserve(geometry));

        let media_background = self
            .primary_surface_presence
            .render_transitioned(now, window, |primary| self.remote_media_background(*primary, cx));

        let chrome_animation = self.chrome_animation;
        let content = vstack(0.0)
            .absolute()
            .inset_0()
            .px(px(14.0 + self.horizontal_safe_inset()))
            .pt(px((self.top_safe_inset() + 8.0).max(8.0)))
            .pb(px(RiseRTCScreenSafeArea::dock_bottom_padding()))
            .child(div().w_full().child(self.header(cx).call_chrome_visibility(
                self.scoped_id("header-visibility"),
                chrome_off,
                -(self.top_safe_inset() + 84.0),
                false,
                chrome_animation,
            )))
            .child(spacer_min_h(12.0))
            .child(if has_primary_surface {
                self.remote_media_identity(state, cx).call_chrome_visibility(
                    self.scoped_id("identity-visibility"),
                    chrome_off,
                    0.0,
                    true,
                    chrome_animation,
                )
            } else {
                self.avatar_content(geometry, state, cx).into_any_element()
            })
            .child(spacer_min_h(lower_spacer_min))
            .child(div().relative().w_full().child(div().absolute().inset_0().child(controls_measured)).child(self.controls(geometry, state, window, cx).call_chrome_visibility(
                self.scoped_id("controls-visibility"),
                chrome_off,
                self.bottom_safe_inset() + 168.0,
                false,
                chrome_animation,
            )));

        let emojis = self.verification_emojis(cx);
        let encryption_sheet = emojis.map(|emojis| self.encryption_sheet(emojis, cx));
        let audio_route_sheet = self.audio_route_sheet(cx);
        let audio_input_sheet = self.audio_input_sheet(cx);
        let stream_settings_sheet = self.stream_settings_sheet(cx);
        let premium_upsell_sheet = self.premium_upsell_sheet(cx);

        div()
            .id(self.scoped_id("root"))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(black())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| this.handle_key_down(event, window, cx)))
            .child(div().absolute().size_0().track_focus(&self.focus_handle))
            .child(div().absolute().inset_0().child(measured))
            .when(!has_primary_surface, |root| root.child(self.call_background(state)))
            .when(self.picture_in_picture_supported(), |root| {
                root.child(div().absolute().w(px(160.0)).h(px(90.0)).opacity(0.001).child(RiseRTCPictureInPictureLayerHost))
            })
            .children(media_background)
            .when(has_primary_surface, |root| {
                root.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(div().size(px(200.0)).guide_anchor(RiseGuideAnchor::CallMainVideo)),
                )
            })
            .child(content)
            .children(self.floating_windows(geometry, window, cx))
            .child(RiseRTCRoomScreenView::window_mode_toggle(white(), cx))
            .child(div().absolute().inset_0().child(audio_route_sheet))
            .child(div().absolute().inset_0().child(audio_input_sheet))
            .children(encryption_sheet.map(|sheet| div().absolute().inset_0().child(sheet)))
            .child(div().absolute().inset_0().child(stream_settings_sheet))
            .children(premium_upsell_sheet.map(|sheet| div().absolute().inset_0().child(sheet)))
    }
}

#[derive(IntoElement)]
struct RiseRTCCallElapsedStatus {
    id: ElementId,
    started_at: Date,
    size: f32,
    compact: bool,
}

impl RenderOnce for RiseRTCCallElapsedStatus {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tick = window.use_keyed_state(child_id(&self.id, "tick"), cx, |_, _| 0_u64);
        use_task(child_id(&self.id, "timeline"), self.started_at, window, cx, async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                tick.update(cx, |tick, cx| {
                    *tick = tick.wrapping_add(1);
                    cx.notify();
                });
            }
        });
        let elapsed = Date::now().time_interval_since(&self.started_at).max(0.0);
        let just_connected = elapsed < 1.5;
        MainText::new(if just_connected { tr("rtc_call_connected") } else { RiseFormat::duration_time_interval(elapsed) })
            .px(self.size)
            .tac(TextAlignT::Center)
            .font_weight(if just_connected && !self.compact { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
            .color(if just_connected { connected_green() } else { white().opacity(0.92) })
    }
}

#[derive(Clone, Copy, PartialEq)]
enum RiseRTCAmbientCallBackgroundMode {
    Preconnect,
    Active,
    WeakConnection,
    Terminal,
}

/// still gradient, only crossfades on call state changes
#[derive(IntoElement)]
struct RiseRTCAmbientCallBackground {
    id: ElementId,
    mode: RiseRTCAmbientCallBackgroundMode,
}

impl RiseRTCAmbientCallBackground {
    fn new(id: ElementId, mode: RiseRTCAmbientCallBackgroundMode) -> Self {
        Self { id, mode }
    }

    fn palette(mode: RiseRTCAmbientCallBackgroundMode) -> [Hsla; 4] {
        let rgb = |r: f32, g: f32, b: f32| -> Hsla { Rgba { r, g, b, a: 1.0 }.into() };
        let bytes = |r: f32, g: f32, b: f32| rgb(r / 255.0, g / 255.0, b / 255.0);
        match mode {
            RiseRTCAmbientCallBackgroundMode::Preconnect => {
                [bytes(232.0, 72.0, 60.0), bytes(255.0, 98.0, 74.0), bytes(255.0, 142.0, 74.0), bytes(214.0, 84.0, 48.0)]
            }
            RiseRTCAmbientCallBackgroundMode::Active => {
                [bytes(172.0, 189.0, 101.0), bytes(69.0, 159.0, 141.0), bytes(83.0, 164.0, 209.0), bytes(62.0, 145.0, 122.0)]
            }
            RiseRTCAmbientCallBackgroundMode::WeakConnection => {
                [rgb(0.72, 0.36, 0.48), rgb(0.82, 0.48, 0.25), rgb(0.56, 0.34, 0.67), rgb(0.44, 0.31, 0.68)]
            }
            RiseRTCAmbientCallBackgroundMode::Terminal => {
                [rgb(0.55, 0.28, 0.24), rgb(0.48, 0.30, 0.22), rgb(0.58, 0.35, 0.24), rgb(0.42, 0.25, 0.20)]
            }
        }
    }
}

impl RenderOnce for RiseRTCAmbientCallBackground {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let target = Self::palette(self.mode);
        let palette = window.use_keyed_state(child_id(&self.id, "palette"), cx, |_, _| target.map(Animated::new));
        let now = RiseClock::monotonic();
        palette.update(cx, |palette, cx| {
            for (animated, color) in palette.iter_mut().zip(target) {
                if *animated.target() != color {
                    animated.set_in(color, ease_in_out(0.42), now, cx);
                }
            }
        });
        let palette = palette.read(cx);
        if palette.iter().any(|animated| animated.is_running(now)) {
            window.request_animation_frame();
        }
        div()
            .absolute()
            .inset_0()
            .child(
                div().absolute().inset_0().child(LinearGradient::new(
                    vec![
                        GradientStop::new(palette[0].value(now), 0.0),
                        GradientStop::new(palette[1].value(now), 0.34),
                        GradientStop::new(palette[2].value(now), 0.69),
                        GradientStop::new(palette[3].value(now), 1.0),
                    ],
                    UnitPoint::TOP_LEADING,
                    UnitPoint::BOTTOM_TRAILING,
                )),
            )
            .child(
                div().absolute().inset_0().child(LinearGradient::new(
                    vec![
                        GradientStop::new(black().opacity(0.03), 0.0),
                        GradientStop::new(black().opacity(0.0), 0.5),
                        GradientStop::new(black().opacity(0.11), 1.0),
                    ],
                    UnitPoint::TOP,
                    UnitPoint::BOTTOM,
                )),
            )
    }
}

#[derive(IntoElement)]
struct RiseRTCAudioReactiveAvatarRing {
    id: ElementId,
    avatar_url: Option<String>,
    name: String,
    user_id: Option<String>,
    avatar_size: f32,
    connected: bool,
}

impl RenderOnce for RiseRTCAudioReactiveAvatarRing {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        let ring = window.use_keyed_state(child_id(&id, "ring"), cx, |_, cx| RiseRTCAudioReactiveAvatarRingState::new(cx));
        ring.update(cx, |ring, _| {
            ring.user_id = self.user_id.clone();
            ring.connected = self.connected;
        });
        let avatar_size = self.avatar_size;
        let frame = avatar_size + 28.0;
        let stroke = avatar_size + 12.0;
        div()
            .relative()
            .size(px(frame))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .child(
                div().size(px(avatar_size)).child(
                    UserLogoUi::new(child_id(&id, "logo"))
                        .source(self.avatar_url.map(SharedString::from))
                        .name(Some(SharedString::from(self.name)))
                        .user_id(self.user_id.map(SharedString::from))
                        .size(avatar_size)
                        .show_border(false)
                        .show_story_ring(false)
                        .image_cache_policy(CleverImageCachePolicy::MemoryOnly),
                ),
            )
            .child(
                div()
                    .absolute()
                    .left(px((frame - stroke) / 2.0))
                    .top(px((frame - stroke) / 2.0))
                    .size(px(stroke))
                    .child(ring),
            )
    }
}

struct RiseRTCAudioReactiveAvatarRingState {
    media: Entity<RiseRTCMediaPresentation>,
    user_id: Option<String>,
    connected: bool,
    ring_opacity: Animated<f32>,
    _subscription: Subscription,
}

impl RiseRTCAudioReactiveAvatarRingState {
    fn new(cx: &mut Context<Self>) -> Self {
        let media = RiseRTCMediaPresentation::shared(cx);
        let subscription = cx.observe(&media, |this, _, cx| {
            let target = if this.is_speaking(cx) { 1.0 } else { 0.0 };
            if *this.ring_opacity.target() != target {
                this.ring_opacity.set_in(target, ease_in_out(0.18), RiseClock::monotonic(), cx);
                cx.notify();
            }
        });
        Self { media, user_id: None, connected: false, ring_opacity: Animated::new(0.0), _subscription: subscription }
    }

    /// the servers voice activity decision with no amplitude threshold on
    /// top, a threshold made the ring depend on loudness not on speaking
    fn is_speaking(&self, cx: &App) -> bool {
        self.connected && self.user_id.as_deref().is_some_and(|user_id| self.media.read(cx).is_speaking(user_id))
    }
}

impl Render for RiseRTCAudioReactiveAvatarRingState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let target = if self.is_speaking(cx) { 1.0 } else { 0.0 };
        if *self.ring_opacity.target() != target {
            self.ring_opacity.set_in(target, ease_in_out(0.18), now, cx);
        }
        if self.ring_opacity.is_running(now) {
            window.request_animation_frame();
        }
        div()
            .relative()
            .size_full()
            .opacity(self.ring_opacity.value(now))
            .child(centred_stroke(RiseRTCSpeakingRing::color(), 4.0, Shape::Circle))
    }
}
