use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AnyView, App, AppContext as _, ClickEvent, Context, ElementId, Empty, Entity, FocusHandle, FontWeight, Hsla,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ObjectFit, ParentElement, Pixels, Point,
    Render, RenderOnce, SharedString, Size, StatefulInteractiveElement, Styled, StyledImage as _, Subscription, Task,
    StyleRefinement, TextRun, WeakEntity, Window, black, container_query, div, img, px, rgb, size, white,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::CancellationError;
use rise_i18n::{plural_word, tr};
use rise_platform::PlatformSupport;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::BottomSheetMeasuredFractionModifiers;
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::expandable_text_ui::EdgeInsets;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAlignment;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::TextAlignT;
use rise_ui::page_header_toolbar_button::PageHeaderToolbarSlot;
use rise_ui::rise_symbol::RiseSymbol;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::{BorderedStyle, plain, system_button};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::{env, reduce_motion};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{MotionSpec, ease_in_out, spring};
use rise_ui::swiftui::progress_view::{ControlSize, progress_view};
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::stacks::{HorizontalAlignment, StackAlign, hstack, spacer, vstack};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Edge, Presence, Transition, Transitioned, UnitPoint};
use rise_ui::stepped_slider_ui::SteppedSliderUi;
use rise_ui::switch_ui::SwitchUi;
use rise_ui::{IconUi, MainText};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{RiseRTCRoomTarget, RootRoute};
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::sai::Sai;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::{
    HoldContextMenuActionsProvider, HoldContextMenuContainer, HoldContextMenuSourceContent,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::forum::rise_forum_voice_presence::{
    RiseForumVoicePresenceSnapshot, RiseForumVoicePresentation,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatChannelKind, ChatPerm, ChatPermExt, VoiceChannelConfig, VoiceChannelParticipant, VoiceChannelState,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::EffectivePermissionsDTOExt;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::guides::stores::guides_services::guides_services::GuidesServicesStore;
use crate::modules::rtc::components::rise_rtc_floating_video_window::{
    RiseRTCFloatingVideoWindow, RiseRTCVideoSurface, VideoViewLayoutMode, VideoViewMirrorMode,
};
use crate::modules::rtc::components::rise_rtc_own_screen_share_wall::RiseRTCOwnScreenShareWall;
use crate::modules::rtc::components::rise_rtc_stage_roster::{RiseRTCStageListenerList, RiseRTCStageMetrics};
use crate::modules::rtc::components::rise_rtc_zoomable_video_surface::RiseRTCZoomableVideoSurface;
use crate::modules::rtc::engine::rise_rtc_call_system_hooks::RiseRTCCallSystemHooks;
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCCallState, RiseRTCCallStateExt, RiseRTCCameraPosition, RiseRTCConnectionState, RiseRTCDirectCall,
    RiseRTCModerationAction, RiseRTCParticipant, RiseRTCParticipantRole, RiseRTCRaisedHand, RiseRTCRoomConfig,
    RiseRTCRoomMode, RiseRTCScreenShareQuality, RiseRTCUserProfile,
};
use crate::modules::rtc::engine::rise_rtc_listener_volume_presentation::RiseRTCListenerVolume;
use crate::modules::rtc::engine::rise_rtc_presentation::RiseRTCRoomPresentation;
use crate::modules::rtc::media::rise_rtc_audio_settings::{RiseMicrophoneTestHush, RiseRTCAudioSettingsContent};
use crate::modules::rtc::media::rise_rtc_call_picture_in_picture::{
    RiseRTCCallPictureInPictureController, RiseRTCCallPictureInPictureControllerOwner, RiseRTCPictureInPictureLayerHost,
};
use crate::modules::rtc::media::rise_rtc_live_kit_audio_coordinator::RiseRTCLiveKitAudioCoordinator;
use crate::modules::rtc::media::rise_rtc_live_kit_media_controller::RiseRTCLiveKitMediaController;
use crate::modules::rtc::media::rise_rtc_media_presentation::{
    RiseRTCMediaPresentation, RiseRTCStreamPreviewStore, VideoTrack,
};
use crate::modules::rtc::pages::rise_rtc_direct_call_screen::RiseRTCDirectCallScreen;
use crate::modules::rtc::pages::rise_rtc_room_presentation_host::RiseRTCRoomPresentationHost;
use crate::modules::rtc::pages::rise_rtc_voice_invite_sheet::RiseRTCVoiceInviteBottomSheet;
use crate::modules::rtc::engine::rise_rtc_domain::RiseRTCDomain;
use crate::modules::rtc::shared::rise_rtc_device_refusal::RiseRTCDeviceRefusal;
use crate::modules::rtc::shared::rise_rtc_navigation_gate::{RiseRTCNavigationGate, RiseRTCRoomTargetSafeLogKind};
use crate::modules::rtc::shared::rise_rtc_room_locator::RiseRTCRoomTargetRoomID;
use crate::modules::rtc::shared::rise_rtc_room_menu::{
    RiseRTCRoomMenu, RiseRTCRoomMenuAction, RiseRTCRoomMenuCapabilities,
};
use crate::modules::rtc::shared::rise_rtc_screen_safe_area::RiseRTCScreenSafeArea;
use crate::modules::rtc::shared::rise_rtc_settings_accent::RiseRTCSettingsAccent;
use crate::modules::rtc::shared::rise_rtc_volume_menu_accessory::{
    RiseRTCListenerVolumeContent, RiseRTCVolumeMenuAccessory,
};
use crate::modules::rtc::shared::rtc_guide_sandbox_hooks::RtcGuideSandboxHooks;
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;
use crate::modules::user::shared::contact_device_book::RiseInstallationIdentity;

const GRID_FIT_VERTICAL_PADDING: f32 = 16.0;
const GRID_FIT_MIN_TILE: f32 = 160.0;
const CLIENT_HANGUP_REASON: &str = "client_hangup";
const OFFSCREEN_GRID_ROWS: f32 = 2.0;
const HEADER_PILL_SLOT_GAP: f32 = 8.0;

pub(crate) fn non_empty(value: &str) -> Option<String> {
    let value = trimming_whitespaces_and_newlines(value);
    if value.is_empty() { None } else { Some(value.to_owned()) }
}

fn tile_id(user_id: &str, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(ElementId::Name(user_id.to_owned().into())), name.into())
}

/// what the dock shows while a media command is in flight, so a toggle reads flipped on the next frame
/// the real participant clears the override once it agrees and a failure drops it
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PendingMediaToggles {
    microphone: Option<bool>,
    deafened: Option<bool>,
    camera: Option<bool>,
    screen_share: Option<bool>,
}

#[derive(Clone, Copy)]
struct CaptionImposed {
    muted: bool,
    deafened: bool,
    color: Hsla,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HeldSessionStart {
    room_id: String,
    self_joined_at_ms: i64,
    started_at_ms: i64,
}

/// values watched on the room body, compared once per render
/// a difference runs synchronize after the frame and never inside it
#[derive(Clone, Debug, Default, PartialEq)]
struct RoomChangeKeys {
    target_identity: Option<String>,
    picture_in_picture_source_identity: String,
    focused_stream_user_id: Option<String>,
    focused_stage_pair_identity: String,
    self_participant: Option<RiseRTCParticipant>,
    session_started_at_ms: Option<i64>,
    guides_running: bool,
    call_state: Option<RiseRTCCallState>,
    stage_capture: Option<(String, Option<String>)>,
}

/// room of a voice channel, group voice chat or direct call
/// mounted from navigation, from the room sheet or as the overlay of a running call
#[derive(IntoElement)]
pub struct RiseRTCRoomScreen {
    target: RiseRTCRoomTarget,
    presented_as_call_overlay: bool,
    presented_in_room_sheet: bool,
}

impl RiseRTCRoomScreen {
    pub fn new(target: RiseRTCRoomTarget, presented_in_room_sheet: bool) -> Self {
        Self { target, presented_as_call_overlay: false, presented_in_room_sheet }
    }

    pub fn presented_as_call_overlay(mut self, presented: bool) -> Self {
        self.presented_as_call_overlay = presented;
        self
    }
}

impl RenderOnce for RiseRTCRoomScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let navigation_store = env::<NavigationStoreKey>(cx);
        let initial_target = self.target.clone();
        let initial_navigation_store = navigation_store.clone();
        let overlay = self.presented_as_call_overlay;
        let sheet = self.presented_in_room_sheet;
        let view = window
            .use_keyed_state("rise-rtc-room-screen", cx, move |window, cx| RiseRTCRoomScreenHolder {
                view: cx.new(|cx| {
                    RiseRTCRoomScreenView::new(initial_target, overlay, sheet, initial_navigation_store, window, cx)
                }),
            })
            .read(cx)
            .view
            .clone();
        view.update(cx, |view, cx| view.configure(self.target, overlay, sheet, navigation_store, cx));
        div().size_full().child(view)
    }
}

struct RiseRTCRoomScreenHolder {
    view: Entity<RiseRTCRoomScreenView>,
}

/// state and body of the room screen
pub struct RiseRTCRoomScreenView {
    target: RiseRTCRoomTarget,
    presented_as_call_overlay: bool,
    presented_in_room_sheet: bool,
    navigation_store: Option<Entity<NavigationStore>>,
    this: WeakEntity<Self>,
    focus_handle: FocusHandle,

    rtc: Entity<RTCServicesStore>,
    interactions: Entity<RTCInteractionsStore>,
    media: Entity<RiseRTCMediaPresentation>,
    stream_previews: Entity<RiseRTCStreamPreviewStore>,
    audio: Entity<RiseRTCLiveKitAudioCoordinator>,
    chat_service: Entity<ChatServicesStore>,
    chat_interactions: Entity<ChatInteractionsStore>,
    forum_voice: Entity<RiseForumVoicePresentation>,
    auth: Entity<AuthServicesStore>,
    socket: Entity<SaiWebSocketManager>,
    guides: Entity<GuidesServicesStore>,
    hold_context_menu: Entity<HoldContextMenuPresenter>,

    joining: bool,
    action_ids: HashSet<String>,
    error_visible: bool,
    audio_routes_visible: bool,
    audio_inputs_visible: bool,
    stream_settings_visible: bool,
    voice_invite_visible: bool,
    swapped_stage_user_id: Option<String>,
    measured_dock_height: Animated<f32>,
    stage_inset_position: UnitPoint,
    audio_route_sheet_fraction: f32,
    audio_input_sheet_fraction: f32,
    top_safe_inset: f32,
    chrome_visible: bool,
    chrome_progress: Animated<f32>,
    selected_stream_user_id: Option<String>,
    stream_quality: RiseRTCScreenShareQuality,
    stream_includes_audio: bool,
    stream_audio_supported: bool,
    leave_committed: bool,
    pending_media: PendingMediaToggles,
    pending_room_cache: RiseRTCPendingRoomCache,
    held_session_start: Option<HeldSessionStart>,

    room: Option<Rc<RiseRTCRoomPresentation>>,
    displayed_room: Option<Rc<RiseRTCRoomPresentation>>,
    visible_participants: Rc<Vec<RiseRTCParticipant>>,

    applied_keys: RoomChangeKeys,
    synchronize_scheduled: bool,
    standard_body_mounted: bool,
    connect_task: Option<Task<()>>,
    session_timer_task: Option<Task<()>>,
    media_fingerprint: u64,
    captured_preview_tracks: HashMap<String, String>,
    grid_origin_y: f32,
    gear_menu_presented: bool,
    stage_controls: Option<Entity<RiseRTCStageControls>>,
    _subscriptions: Vec<Subscription>,
}

impl RiseRTCRoomScreenView {
    const KICK_REASON: &'static str = "moderator_removed";

    fn new(
        target: RiseRTCRoomTarget,
        presented_as_call_overlay: bool,
        presented_in_room_sheet: bool,
        navigation_store: Option<Entity<NavigationStore>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let rtc = RTCServicesStore::shared(cx);
        let interactions = RTCInteractionsStore::shared(cx);
        let media = RiseRTCMediaPresentation::shared(cx);
        let stream_previews = RiseRTCStreamPreviewStore::shared(cx);
        let audio = RiseRTCLiveKitAudioCoordinator::shared(cx);
        let chat_service = ChatServicesStore::shared(cx);
        let chat_interactions = ChatInteractionsStore::shared(cx);
        let forum_voice = RiseForumVoicePresentation::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let socket = SaiWebSocketManager::shared(cx);
        let guides = GuidesServicesStore::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);

        let subscriptions = vec![
            cx.observe(&rtc, |this, _, cx| this.room_inputs_changed(cx)),
            cx.observe(&chat_service, |this, _, cx| this.room_inputs_changed(cx)),
            cx.observe(&forum_voice, |this, _, cx| this.room_inputs_changed(cx)),
            cx.observe(&auth, |this, _, cx| this.room_inputs_changed(cx)),
            cx.observe(&media, |this, _, cx| {
                let fingerprint = this.current_media_fingerprint(cx);
                if fingerprint != this.media_fingerprint {
                    this.media_fingerprint = fingerprint;
                    cx.notify();
                }
            }),
            cx.observe(&stream_previews, |_, _, cx| cx.notify()),
            cx.observe(&audio, |_, _, cx| cx.notify()),
            cx.observe(&socket, |_, _, cx| cx.notify()),
            cx.observe_global::<RiseMicrophoneTestHush>(|_, cx| cx.notify()),
            cx.observe(&guides, |_, _, cx| cx.notify()),
            cx.observe(&hold_context_menu, |_, _, cx| cx.notify()),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() || this.room.is_none() || !this.standard_body_mounted {
                    return;
                }
                let interactions = this.interactions.clone();
                this.perform("refresh".to_owned(), move |cx| {
                    interactions.update(cx, |interactions, cx| interactions.refresh_active_room(cx))
                }, cx);
            }),
        ];

        cx.on_release(|this, cx| this.standard_body_disappeared(cx)).detach();
        cx.defer_in(window, |this, window, cx| window.focus(&this.focus_handle, cx));

        let mut view = Self {
            target,
            presented_as_call_overlay,
            presented_in_room_sheet,
            navigation_store,
            this: cx.weak_entity(),
            focus_handle: cx.focus_handle(),
            rtc,
            interactions,
            media,
            stream_previews,
            audio,
            chat_service,
            chat_interactions,
            forum_voice,
            auth,
            socket,
            guides,
            hold_context_menu,
            joining: false,
            action_ids: HashSet::new(),
            error_visible: false,
            audio_routes_visible: false,
            audio_inputs_visible: false,
            stream_settings_visible: false,
            voice_invite_visible: false,
            swapped_stage_user_id: None,
            measured_dock_height: Animated::new(75.0),
            stage_inset_position: UnitPoint::TOP_TRAILING,
            audio_route_sheet_fraction: 0.3,
            audio_input_sheet_fraction: 0.55,
            top_safe_inset: 0.0,
            chrome_visible: true,
            chrome_progress: Animated::new(1.0),
            selected_stream_user_id: None,
            stream_quality: RiseRTCScreenShareQuality::Balanced,
            stream_includes_audio: true,
            stream_audio_supported: matches!(rise_media::screen_capture::system_audio_support(), PlatformSupport::Performed),
            leave_committed: false,
            pending_media: PendingMediaToggles::default(),
            pending_room_cache: RiseRTCPendingRoomCache::default(),
            held_session_start: None,
            room: None,
            displayed_room: None,
            visible_participants: Rc::new(Vec::new()),
            applied_keys: RoomChangeKeys::default(),
            synchronize_scheduled: false,
            standard_body_mounted: false,
            connect_task: None,
            session_timer_task: None,
            media_fingerprint: 0,
            captured_preview_tracks: HashMap::new(),
            grid_origin_y: 0.0,
            gear_menu_presented: false,
            stage_controls: None,
            _subscriptions: subscriptions,
        };
        view.refresh_room(cx);
        view.media_fingerprint = view.current_media_fingerprint(cx);
        view
    }

    fn configure(
        &mut self,
        target: RiseRTCRoomTarget,
        presented_as_call_overlay: bool,
        presented_in_room_sheet: bool,
        navigation_store: Option<Entity<NavigationStore>>,
        cx: &mut Context<Self>,
    ) {
        self.presented_as_call_overlay = presented_as_call_overlay;
        self.presented_in_room_sheet = presented_in_room_sheet;
        self.navigation_store = navigation_store;
        if self.target != target {
            self.target = target;
            self.connect_task = None;
            self.joining = false;
            self.refresh_room(cx);
        }
    }

    fn room_inputs_changed(&mut self, cx: &mut Context<Self>) {
        self.refresh_room(cx);
        cx.notify();
    }

    fn refresh_room(&mut self, cx: &mut Context<Self>) {
        let expected = self.expected_room_id();
        let active = self
            .rtc
            .read(cx)
            .active_room(cx)
            .filter(|room| expected.as_deref() == Some(room.config.room_id.as_str()));
        let unchanged = matches!((active, &self.room), (Some(active), Some(cached)) if **cached == *active);
        if !unchanged {
            self.room = active.map(|active| Rc::new(active.clone()));
        }
        self.displayed_room = if self.room.is_some() {
            self.room.clone()
        } else if !self.error_visible && self.rtc.read(cx).connection_state(cx) != RiseRTCConnectionState::Failed {
            self.optimistic_room(cx)
        } else {
            None
        };
        let visible = self.compute_visible_participants(cx);
        if visible.iter().map(|row| (&row.user_id, &row.role)).ne(self.visible_participants.iter().map(|row| (&row.user_id, &row.role))) {
            tracing::warn!(
                target: "riseonly::X2163",
                "screen visible rows={:?} hands={:?} self={:?}",
                visible.iter().map(|row| (row.user_id.as_str(), &row.role)).collect::<Vec<_>>(),
                self.displayed_room.as_deref().map(|room| room.raised_hands.iter().map(|hand| hand.user_id.as_str()).collect::<Vec<_>>()),
                self.displayed_room.as_deref().and_then(|room| room.self_participant.as_ref()).map(|row| (row.user_id.as_str(), &row.role)),
            );
        }
        self.visible_participants = Rc::new(visible);
    }

    fn current_media_fingerprint(&self, cx: &App) -> u64 {
        let media = self.media.read(cx);
        let unordered = |values: &HashSet<String>| {
            values.iter().fold(0_u64, |sum, value| {
                let mut hasher = DefaultHasher::new();
                value.hash(&mut hasher);
                sum.wrapping_add(hasher.finish())
            })
        };
        let mut hasher = DefaultHasher::new();
        unordered(media.speaking_user_ids()).hash(&mut hasher);
        unordered(media.portrait_video_user_ids()).hash(&mut hasher);
        (media.local_camera_position() == RiseRTCCameraPosition::Front).hash(&mut hasher);
        for participant in self.visible_participants.iter() {
            media.camera_track(&participant.user_id).map(|track| track.track_key().to_owned()).hash(&mut hasher);
            media.screen_share_track(&participant.user_id).map(|track| track.track_key().to_owned()).hash(&mut hasher);
        }
        hasher.finish()
    }

    /// wraps a method into the click handler shape of the kit
    fn handler<F>(&self, action: F) -> impl Fn(&mut Window, &mut App) + 'static + use<F>
    where
        F: Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    {
        let this = self.this.clone();
        move |window: &mut Window, cx: &mut App| {
            let _ = this.update(cx, |view, cx| action(view, window, cx));
        }
    }

    /// silenced by a moderator, outranks the optimistic override
    /// the override is the members own intent and one the server already
    /// refused must not keep the dock lying while the toggle stays pending
    fn is_server_silenced(&self) -> bool {
        self.self_participant().is_some_and(RiseRTCParticipant::is_server_silenced)
    }

    fn is_server_deafened(&self) -> bool {
        self.self_participant().is_some_and(|participant| participant.server_deafened)
    }

    fn displayed_microphone_enabled(&self) -> bool {
        if self.is_server_silenced() {
            return false;
        }
        self.pending_media
            .microphone
            .unwrap_or_else(|| self.self_participant().is_some_and(|participant| participant.microphone_enabled))
    }

    /// a moderator deafen reads as deafened like the members own, the sfu already
    /// dropped the subscription so live headphones would claim audio that wasnt working
    fn displayed_deafened(&self) -> bool {
        if self.is_server_deafened() {
            return true;
        }
        self.pending_media.deafened.unwrap_or_else(|| self.self_participant().is_some_and(|participant| participant.deafened))
    }

    fn displayed_camera_enabled(&self) -> bool {
        self.pending_media.camera.unwrap_or_else(|| self.self_participant().is_some_and(|participant| participant.camera_enabled))
    }

    fn displayed_screen_share_enabled(&self) -> bool {
        self.pending_media
            .screen_share
            .unwrap_or_else(|| self.self_participant().is_some_and(|participant| participant.screen_share_enabled))
    }

    fn room(&self) -> Option<&RiseRTCRoomPresentation> {
        self.room.as_deref()
    }

    fn call(&self, cx: &App) -> Option<RiseRTCDirectCall> {
        let RiseRTCRoomTarget::DirectCall { call_id } = &self.target else {
            return None;
        };
        self.rtc.read(cx).call(call_id, cx)
    }

    fn expected_room_id(&self) -> Option<String> {
        self.target.room_id()
    }

    /// the voice chat a link would open, a forum topic or the groups own call
    fn voice_invite_chat_id(&self) -> Option<String> {
        match &self.target {
            RiseRTCRoomTarget::Topic { chat_id, .. } | RiseRTCRoomTarget::Group { chat_id } => Some(chat_id.clone()),
            RiseRTCRoomTarget::DirectCall { .. } => None,
        }
    }

    fn voice_invite_topic_id(&self) -> Option<String> {
        match &self.target {
            RiseRTCRoomTarget::Topic { topic_id, .. } => Some(topic_id.clone()),
            _ => None,
        }
    }

    /// the tile follows the chats own invite right, with it gone the server refuses anyway
    fn can_share_voice_invite(&self, cx: &mut App) -> bool {
        let Some(chat_id) = self.voice_invite_chat_id() else {
            return false;
        };
        let Some(chat) = self.chat_service.update(cx, |services, cx| services.chat_info(&chat_id, cx)) else {
            return false;
        };
        ChatProfilePermissions::distributes_invites(&chat, cx)
    }

    /// a voice chat or stream is reported as the chat or channel it belongs to
    /// nobody reports a chat they own and a direct call has no room to report
    fn reportable_chat_id(&self, cx: &mut App) -> Option<String> {
        let chat_id = self.voice_invite_chat_id().filter(|chat_id| !chat_id.is_empty())?;
        let chat = self.chat_service.update(cx, |services, cx| services.chat_info(&chat_id, cx));
        if chat.is_some_and(|chat| ChatProfilePermissions::is_owner(&chat)) {
            return None;
        }
        Some(chat_id)
    }

    /// the user closed this room, requests still in flight fail with it
    fn room_was_left(&self) -> bool {
        self.leave_committed
    }

    fn target_identity(&self) -> String {
        match &self.target {
            RiseRTCRoomTarget::Topic { chat_id, topic_id } => format!("topic:{chat_id}:{topic_id}"),
            RiseRTCRoomTarget::Group { chat_id } => format!("group:{chat_id}"),
            RiseRTCRoomTarget::DirectCall { call_id } => format!("call:{call_id}"),
        }
    }

    fn title(&self, cx: &mut App) -> String {
        match &self.target {
            RiseRTCRoomTarget::Topic { chat_id, topic_id } => self
                .chat_service
                .update(cx, |store, cx| store.topic_from_any_cache(chat_id, topic_id, cx))
                .and_then(|topic| non_empty(&topic.title))
                .unwrap_or_else(|| tr("rtc_voice_chat")),
            RiseRTCRoomTarget::Group { chat_id } => self
                .chat_service
                .read(cx)
                .chat_info(chat_id, cx)
                .and_then(|chat| non_empty(&chat.title))
                .unwrap_or_else(|| tr("rtc_group_voice")),
            RiseRTCRoomTarget::DirectCall { .. } => {
                let profile = self.counterpart_profile(cx);
                profile
                    .as_ref()
                    .and_then(|profile| non_empty(&profile.name))
                    .or_else(|| profile.as_ref().and_then(|profile| non_empty(&profile.tag)))
                    .unwrap_or_else(|| tr("rtc_direct_call"))
            }
        }
    }

    fn subtitle(&self, cx: &App) -> String {
        let state = self.rtc.read(cx).connection_state(cx);
        let connecting = self.joining || state == RiseRTCConnectionState::Connecting;
        if (connecting || state == RiseRTCConnectionState::Reconnecting) && !self.socket.read(cx).is_connected() {
            return tr("rtc_waiting_for_network");
        }
        if connecting {
            return tr("rtc_connecting");
        }
        if state == RiseRTCConnectionState::Reconnecting {
            return tr("rtc_reconnecting");
        }
        if let Some(room) = self.displayed_room.as_deref() {
            let one = tr("chat_count_member_one");
            let few = tr("chat_count_member_few");
            let many = tr("chat_count_member_many");
            return format!("{} {}", room.participant_count, plural_word(room.participant_count, &one, &few, &many));
        }
        tr("rtc_voice_chat")
    }

    fn roster_session_started_at_ms(&self, cx: &App) -> Option<i64> {
        if self.joining || self.rtc.read(cx).connection_state(cx) != RiseRTCConnectionState::Connected {
            return None;
        }
        let room = self.displayed_room.as_deref()?;
        let self_joined_at_ms = room.self_participant.as_ref()?.joined_at_ms;
        if self_joined_at_ms <= 0 {
            return None;
        }
        if room.config.mode == RiseRTCRoomMode::DirectCall {
            return Some(self_joined_at_ms);
        }
        let earliest = room
            .participants
            .iter()
            .map(|participant| participant.joined_at_ms)
            .filter(|joined_at_ms| *joined_at_ms > 0)
            .min()
            .unwrap_or(self_joined_at_ms);
        Some(earliest.min(self_joined_at_ms))
    }

    /// counted from the server joined_at not from screen appearance, the room
    /// is minimized and reopened a lot and a local stopwatch would restart
    /// none while connecting, reconnecting or after leaving
    fn session_started_at_ms(&self, cx: &App) -> Option<i64> {
        let roster = self.roster_session_started_at_ms(cx)?;
        let room_id = self.displayed_room.as_deref().map(|room| room.config.room_id.as_str());
        let self_joined_at_ms = self.self_participant().map(|participant| participant.joined_at_ms);
        match &self.held_session_start {
            Some(held) if Some(held.room_id.as_str()) == room_id && Some(held.self_joined_at_ms) == self_joined_at_ms => {
                Some(roster.min(held.started_at_ms))
            }
            _ => Some(roster),
        }
    }

    fn current_user_id(&self, cx: &App) -> String {
        self.auth.read(cx).effective_user_id_for_wire().unwrap_or_default()
    }

    fn self_participant(&self) -> Option<&RiseRTCParticipant> {
        self.displayed_room.as_deref()?.self_participant.as_ref()
    }

    fn counterpart_profile(&self, cx: &App) -> Option<RiseRTCUserProfile> {
        let rtc = self.rtc.read(cx);
        let incoming_caller = || rtc.incoming_call(cx).and_then(|incoming| incoming.caller.clone());
        let Some(call) = self.call(cx) else {
            return incoming_caller();
        };
        let current_user_id = self.current_user_id(cx);
        let counterpart_id = if call.caller_user_id == current_user_id { &call.callee_user_id } else { &call.caller_user_id };
        self.room()
            .and_then(|room| room.participants.iter().find(|participant| &participant.user_id == counterpart_id))
            .and_then(|participant| participant.profile.clone())
            .or_else(incoming_caller)
            .or_else(|| rtc.practice_call_counterpart(&call.call_id, cx))
    }

    fn room_mode(&self, cx: &mut App) -> RiseRTCRoomMode {
        if let Some(room) = self.room() {
            return room.config.mode;
        }
        match &self.target {
            RiseRTCRoomTarget::Topic { chat_id, topic_id } => {
                let kind = self
                    .chat_service
                    .update(cx, |store, cx| store.topic_from_any_cache(chat_id, topic_id, cx))
                    .map(|topic| topic.channel_kind);
                if kind == Some(ChatChannelKind::Stage) { RiseRTCRoomMode::Stage } else { RiseRTCRoomMode::Voice }
            }
            RiseRTCRoomTarget::Group { .. } => RiseRTCRoomMode::GroupVoice,
            RiseRTCRoomTarget::DirectCall { .. } => RiseRTCRoomMode::DirectCall,
        }
    }

    fn effective_permissions(&self, cx: &mut App) -> Option<i64> {
        match &self.target {
            RiseRTCRoomTarget::Topic { chat_id, topic_id } => self
                .chat_service
                .update(cx, |store, cx| store.effective_permissions(chat_id, Some(topic_id.as_str()), cx))
                .map(|permissions| permissions.mask()),
            RiseRTCRoomTarget::Group { chat_id } => self
                .chat_service
                .update(cx, |store, cx| store.effective_permissions(chat_id, None, cx))
                .map(|permissions| permissions.mask()),
            RiseRTCRoomTarget::DirectCall { .. } => None,
        }
    }

    fn has_publishing_role(&self) -> bool {
        self.self_participant().is_some_and(|participant| {
            matches!(participant.role, RiseRTCParticipantRole::Speaker | RiseRTCParticipantRole::Moderator)
        })
    }

    fn has_room_permission(&self, permission: i64, cx: &mut App) -> bool {
        if self.is_direct_call(cx) {
            return true;
        }
        self.effective_permissions(cx).is_some_and(|effective| ChatPerm::has(effective, permission))
    }

    fn can_publish_microphone(&self, cx: &mut App) -> bool {
        self.has_publishing_role() && !self.is_server_silenced() && self.has_room_permission(ChatPerm::SPEAK, cx)
    }

    fn is_server_camera_blocked(&self) -> bool {
        self.self_participant().is_some_and(|participant| participant.server_camera_blocked)
    }

    fn is_server_screen_share_blocked(&self) -> bool {
        self.self_participant().is_some_and(|participant| participant.server_screen_share_blocked)
    }

    fn can_publish_camera(&self, cx: &mut App) -> bool {
        self.has_publishing_role()
            && !self.is_server_camera_blocked()
            && self.displayed_room.as_deref().is_some_and(|room| room.config.video_enabled)
            && self.has_room_permission(ChatPerm::STREAM_VIDEO, cx)
    }

    fn can_publish_screen(&self, cx: &mut App) -> bool {
        self.has_publishing_role()
            && !self.is_server_screen_share_blocked()
            && self.displayed_room.as_deref().is_some_and(|room| room.config.screen_share_enabled)
            && self.has_room_permission(ChatPerm::SHARE_SCREEN, cx)
    }

    fn can_manage_stage(&self, cx: &mut App) -> bool {
        self.room_mode(cx) == RiseRTCRoomMode::Stage && self.has_room_permission(ChatPerm::MANAGE_STAGE, cx)
    }

    fn can_mute_participants(&self, cx: &mut App) -> bool {
        self.has_room_permission(ChatPerm::MUTE_VOICE_MEMBERS, cx)
    }

    fn can_deafen_participants(&self, cx: &mut App) -> bool {
        self.has_room_permission(ChatPerm::DEAFEN_VOICE_MEMBERS, cx)
    }

    fn can_block_participant_camera(&self, cx: &mut App) -> bool {
        self.has_room_permission(ChatPerm::BLOCK_VOICE_VIDEO, cx)
    }

    fn can_block_participant_screen_share(&self, cx: &mut App) -> bool {
        self.has_room_permission(ChatPerm::BLOCK_VOICE_SCREEN_SHARE, cx)
    }

    fn can_remove_participants(&self, cx: &mut App) -> bool {
        self.has_room_permission(ChatPerm::MOVE_VOICE_MEMBERS, cx)
    }

    fn can_request_to_speak(&self, cx: &mut App) -> bool {
        self.room_mode(cx) == RiseRTCRoomMode::Stage && self.has_room_permission(ChatPerm::REQUEST_TO_SPEAK, cx)
    }

    fn can_take_stage(&self, cx: &mut App) -> bool {
        self.room_mode(cx) == RiseRTCRoomMode::Stage && self.has_room_permission(ChatPerm::TAKE_STAGE, cx)
    }

    fn shows_take_stage_button(&self, cx: &mut App) -> bool {
        self.room_mode(cx) == RiseRTCRoomMode::Stage
            && self.self_participant().is_some_and(|participant| participant.role == RiseRTCParticipantRole::Listener)
            && !self.is_connecting_to_displayed_room()
            && self.can_take_stage(cx)
    }

    fn owns_navigation_room_session(&self) -> bool {
        if self.presented_as_call_overlay || self.presented_in_room_sheet {
            return false;
        }
        !matches!(self.target, RiseRTCRoomTarget::DirectCall { .. })
    }

    fn is_direct_call(&self, cx: &mut App) -> bool {
        self.room_mode(cx) == RiseRTCRoomMode::DirectCall
    }

    /// both halves of the stage split visible_participants and not the raw roster
    /// that list hoists this account to the front and folds in the optimistic self row
    fn speakers(&self) -> Vec<RiseRTCParticipant> {
        self.visible_participants
            .iter()
            .filter(|participant| {
                matches!(participant.role, RiseRTCParticipantRole::Speaker | RiseRTCParticipantRole::Moderator)
            })
            .cloned()
            .collect()
    }

    fn audience(&self) -> Vec<RiseRTCParticipant> {
        self.visible_participants
            .iter()
            .filter(|participant| {
                matches!(participant.role, RiseRTCParticipantRole::Listener | RiseRTCParticipantRole::Unspecified)
            })
            .cloned()
            .collect()
    }

    /// built from raised_hands not participant.hand_raised, only the hand has
    /// expires_at_ms so only it knows the hand already fell out of the queue
    /// the participant flag lags a purge by one snapshot
    fn raised_hand_user_ids(&self) -> HashSet<String> {
        self.room().map_or_else(HashSet::new, |room| room.raised_hands.iter().map(|hand| hand.user_id.clone()).collect())
    }

    fn self_hand_raised(&self, cx: &App) -> bool {
        self.raised_hand_user_ids().contains(&self.current_user_id(cx))
    }

    fn stage_audience_rows(&self) -> Vec<RiseRTCParticipant> {
        let audience = self.audience();
        let Some(hands) = self.room().map(|room| &room.raised_hands).filter(|hands| !hands.is_empty()) else {
            return audience;
        };
        let mut loaded: HashMap<&str, &RiseRTCParticipant> = HashMap::with_capacity(audience.len());
        for participant in &audience {
            loaded.entry(participant.user_id.as_str()).or_insert(participant);
        }
        let speakers = self.speakers();
        let speaker_ids: HashSet<&str> = speakers.iter().map(|speaker| speaker.user_id.as_str()).collect();
        let mut queued = Vec::with_capacity(hands.len());
        let mut placed: HashSet<&str> = HashSet::with_capacity(hands.len());
        for hand in hands {
            if speaker_ids.contains(hand.user_id.as_str()) || !placed.insert(hand.user_id.as_str()) {
                continue;
            }
            queued.push(
                loaded.get(hand.user_id.as_str()).map_or_else(|| Self::pending_listener(hand), |row| (*row).clone()),
            );
        }
        queued.extend(audience.iter().filter(|participant| !placed.contains(participant.user_id.as_str())).cloned());
        queued
    }

    fn pending_listener(hand: &RiseRTCRaisedHand) -> RiseRTCParticipant {
        RiseRTCParticipant {
            user_id: hand.user_id.clone(),
            role: RiseRTCParticipantRole::Listener,
            connected: true,
            microphone_enabled: false,
            camera_enabled: false,
            screen_share_enabled: false,
            server_muted: false,
            server_deafened: false,
            server_camera_blocked: false,
            server_screen_share_blocked: false,
            deafened: false,
            hand_raised: true,
            joined_at_ms: hand.requested_at_ms,
            updated_at_ms: hand.requested_at_ms,
            lease_expires_at_ms: 0,
            order_sequence: 0,
            profile: hand.profile.clone(),
        }
    }

    fn moderation_capabilities(&self, cx: &mut App) -> RiseRTCRoomMenuCapabilities {
        let chat_id = match &self.target {
            RiseRTCRoomTarget::Topic { chat_id, .. } | RiseRTCRoomTarget::Group { chat_id } => Some(chat_id.clone()),
            RiseRTCRoomTarget::DirectCall { .. } => None,
        };
        RiseRTCRoomMenuCapabilities {
            can_manage_stage: self.can_manage_stage(cx),
            can_mute: self.can_mute_participants(cx),
            can_deafen: self.can_deafen_participants(cx),
            can_block_camera: self.can_block_participant_camera(cx),
            can_block_screen_share: self.can_block_participant_screen_share(cx),
            can_remove: self.can_remove_participants(cx),
            chat_id,
        }
    }

    /// until the media session is live the room is built from the roster the topic list showed so
    /// the first frame isnt empty
    /// not a fallback for a failed join since the retry screen must stay reachable
    fn is_connecting_to_displayed_room(&self) -> bool {
        self.room.is_none()
    }

    fn optimistic_room(&mut self, cx: &mut App) -> Option<Rc<RiseRTCRoomPresentation>> {
        let seed = self.optimistic_room_seed(cx);
        let current_user_id = self.current_user_id(cx);
        self.pending_room_cache.room(seed, &current_user_id)
    }

    fn optimistic_room_seed(&self, cx: &mut App) -> Option<RiseRTCPendingRoomSeed> {
        let RiseRTCRoomTarget::Topic { chat_id, topic_id } = &self.target else {
            return None;
        };
        let room_id = self.expected_room_id()?;
        let topic = self.chat_service.update(cx, |store, cx| store.topic_from_any_cache(chat_id, topic_id, cx))?;
        if !topic.is_rtc_channel() {
            return None;
        }
        let state = self.forum_voice.read(cx).state(chat_id, topic_id).or_else(|| topic.voice_state.clone());
        Some(RiseRTCPendingRoomSeed {
            room_id,
            chat_id: chat_id.clone(),
            topic_id: topic_id.clone(),
            is_stage: topic.channel_kind == ChatChannelKind::Stage,
            config: topic.voice_config.clone(),
            state,
        })
    }

    fn compute_visible_participants(&self, cx: &App) -> Vec<RiseRTCParticipant> {
        let Some(room) = self.displayed_room.as_deref() else {
            return Vec::new();
        };
        let mut participants = room.participants.clone();
        if let Some(self_participant) = &room.self_participant
            && !participants.iter().any(|participant| participant.user_id == self_participant.user_id)
        {
            participants.insert(0, self_participant.clone());
        }
        let current_user_id = self.current_user_id(cx);
        match participants.iter().position(|participant| participant.user_id == current_user_id) {
            Some(index) if index != 0 => {
                let current = participants.remove(index);
                participants.insert(0, current);
                participants
            }
            _ => participants,
        }
    }

    /// fullscreen works for any published video not only a screen share,
    /// so a member (this one too) can open their own camera full bleed
    fn focused_stream_user_id(&self, cx: &App) -> Option<String> {
        let current_user_id = self.current_user_id(cx);
        let focused = self.rtc.read(cx).focused_stream_user_id();
        [self.selected_stream_user_id.as_deref(), focused].into_iter().flatten().find_map(|user_id| {
            (self.focusable_track(user_id, cx).is_some() || self.is_pending_surface(user_id, &current_user_id))
                .then(|| user_id.to_owned())
        })
    }

    fn is_pending_surface(&self, user_id: &str, current_user_id: &str) -> bool {
        if user_id == current_user_id {
            return self.displayed_screen_share_enabled() || self.displayed_camera_enabled();
        }
        self.visible_participants
            .iter()
            .find(|participant| participant.user_id == user_id)
            .is_some_and(|participant| participant.screen_share_enabled || participant.camera_enabled)
    }

    /// screen share wins when a member publishes both until the viewer swaps by clicking
    /// the corner inset, same promotion the direct call uses for the peer two surfaces
    fn focusable_track(&self, user_id: &str, cx: &App) -> Option<Rc<VideoTrack>> {
        let screen = self.screen_track(user_id, cx);
        if self.swapped_stage_user_id.as_deref() == Some(user_id)
            && screen.is_some()
            && let Some(camera) = self.camera_track(user_id, cx)
        {
            return Some(camera);
        }
        screen.or_else(|| self.camera_track(user_id, cx))
    }

    /// who is on stage and which of their two surfaces
    /// losing either one clears the manual swap
    fn focused_stage_pair_identity(&self, focused: Option<&str>, cx: &App) -> String {
        let Some(user_id) = focused else {
            return "none".to_owned();
        };
        format!(
            "{user_id}|{}|{}",
            self.screen_track(user_id, cx).is_some(),
            self.camera_track(user_id, cx).is_some()
        )
    }

    fn picture_in_picture_supported(&self) -> bool {
        RiseRTCCallPictureInPictureController::is_supported()
    }

    /// pip follows an actively watched remote stage, never ones own preview or the grid
    fn picture_in_picture_source(&self, cx: &App) -> Option<(String, bool, Rc<VideoTrack>)> {
        let user_id = self.focused_stream_user_id(cx)?;
        if user_id == self.current_user_id(cx) {
            return None;
        }
        let track = self.focusable_track(&user_id, cx)?;
        let is_screen = self.screen_track(&user_id, cx).is_some_and(|screen| Rc::ptr_eq(&screen, &track));
        Some((user_id, is_screen, track))
    }

    /// wire identity not object address, tracks are held
    /// weakly and a freed address gets reused by another track
    fn picture_in_picture_source_identity(&self, cx: &App) -> String {
        match self.picture_in_picture_source(cx) {
            Some((user_id, is_screen, track)) => format!("{user_id}|{is_screen}|{}", track.track_key()),
            None => "none".to_owned(),
        }
    }

    fn synchronize_picture_in_picture(&mut self, cx: &mut Context<Self>) {
        if !self.picture_in_picture_supported() {
            return;
        }
        let owner = RiseRTCCallPictureInPictureControllerOwner::VoiceRoom;
        let controller = RiseRTCCallPictureInPictureController::shared(cx);
        let source = self.picture_in_picture_source(cx);
        let identity = self.picture_in_picture_source_identity(cx);
        controller.update(cx, |controller, cx| match source {
            None => controller.disable(owner, cx),
            Some((_, _, track)) => {
                controller.enable(owner, cx);
                controller.present(Some(track.track_key().to_owned()), Some(identity), owner, cx);
            }
        });
    }

    fn on_change_keys(&self, cx: &App) -> RoomChangeKeys {
        let focused_stream_user_id = self.focused_stream_user_id(cx);
        let stage_capture = focused_stream_user_id.as_ref().map(|user_id| {
            let screen = self.screen_track(user_id, cx).map(|track| track.track_key().to_owned());
            (user_id.clone(), screen)
        });
        RoomChangeKeys {
            target_identity: Some(self.target_identity()),
            picture_in_picture_source_identity: self.picture_in_picture_source_identity(cx),
            focused_stage_pair_identity: self.focused_stage_pair_identity(focused_stream_user_id.as_deref(), cx),
            focused_stream_user_id,
            self_participant: self.self_participant().cloned(),
            session_started_at_ms: self.session_started_at_ms(cx),
            guides_running: self.guides.read(cx).is_running(cx),
            call_state: self.call(cx).map(|call| call.state),
            stage_capture,
        }
    }

    /// the on appear, task and on change chain of the room body
    fn synchronize(&mut self, cx: &mut Context<Self>) {
        let keys = self.on_change_keys(cx);
        let previous = std::mem::replace(&mut self.applied_keys, keys.clone());
        let appeared = !self.standard_body_mounted;
        self.standard_body_mounted = true;

        if appeared || keys.picture_in_picture_source_identity != previous.picture_in_picture_source_identity {
            self.synchronize_picture_in_picture(cx);
        }
        if keys.target_identity != previous.target_identity {
            self.start_room_task(cx);
        }
        if keys.focused_stream_user_id != previous.focused_stream_user_id {
            self.reconcile_focused_stream(cx);
        }
        if keys.focused_stage_pair_identity != previous.focused_stage_pair_identity {
            self.swapped_stage_user_id = None;
        }
        if keys.self_participant != previous.self_participant
            && let Some(participant) = &keys.self_participant
        {
            self.reconcile_pending_media(participant);
        }
        if (appeared || keys.session_started_at_ms != previous.session_started_at_ms)
            && let Some(started_at_ms) = keys.session_started_at_ms
            && let Some(room_id) = self.displayed_room.as_deref().map(|room| room.config.room_id.clone())
            && let Some(self_joined_at_ms) = self.self_participant().map(|participant| participant.joined_at_ms)
        {
            self.held_session_start = Some(HeldSessionStart { room_id, self_joined_at_ms, started_at_ms });
        }
        if keys.session_started_at_ms.is_some() != self.session_timer_task.is_some() {
            self.update_session_timer(keys.session_started_at_ms.is_some(), cx);
        }
        if keys.guides_running && !previous.guides_running && !self.chrome_visible {
            self.set_chrome_visible(true, cx);
        }
        if keys.call_state != previous.call_state
            && keys.call_state.is_some_and(RiseRTCCallStateExt::is_terminal)
            && !self.presented_as_call_overlay
        {
            if self.presented_in_room_sheet {
                self.interactions.update(cx, |interactions, cx| interactions.dismiss_room_presentation(cx));
            } else {
                self.pop_navigation(cx);
            }
        }
        if keys.stage_capture != previous.stage_capture {
            self.synchronize_stage_capture(previous.stage_capture.as_ref(), keys.stage_capture.as_ref(), cx);
        }
        cx.notify();
    }

    fn synchronize_stage_capture(
        &mut self,
        previous: Option<&(String, Option<String>)>,
        current: Option<&(String, Option<String>)>,
        cx: &mut Context<Self>,
    ) {
        if let Some((user_id, _)) = previous {
            self.stream_previews.update(cx, |previews, _| previews.end_capturing(user_id));
        }
        if let Some((user_id, Some(_))) = current
            && *user_id != self.current_user_id(cx)
            && let Some(track) = self.screen_track(user_id, cx)
        {
            self.stream_previews.update(cx, |previews, cx| previews.begin_capturing(user_id, &track, cx));
        }
    }

    fn update_session_timer(&mut self, running: bool, cx: &mut Context<Self>) {
        if !running {
            self.session_timer_task = None;
            return;
        }
        self.session_timer_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        }));
    }

    fn start_room_task(&mut self, cx: &mut Context<Self>) {
        let kind = self.target.safe_log_kind();
        Sai::log("RTC", move || format!("room screen mounted target={kind}"), SaiLogKind::Network, cx);
        let connect = self.connect_if_needed(false, cx);
        self.connect_task = Some(cx.spawn(async move |this, cx| {
            connect.await;
            let _ = this.update(cx, |this, cx| {
                if this.selected_stream_user_id.is_none() {
                    this.selected_stream_user_id = this.rtc.read(cx).focused_stream_user_id().map(str::to_owned);
                }
                this.room_inputs_changed(cx);
            });
        }));
    }

    /// room body disappeared, the pip owner
    /// the stage capture and the navigation owned session end with it
    fn standard_body_disappeared(&mut self, cx: &mut App) {
        if !self.standard_body_mounted {
            return;
        }
        self.standard_body_mounted = false;
        RiseRTCCallPictureInPictureController::shared(cx).update(cx, |controller, cx| {
            controller.disable(RiseRTCCallPictureInPictureControllerOwner::VoiceRoom, cx);
        });
        if let Some((user_id, _)) = self.applied_keys.stage_capture.take() {
            self.stream_previews.update(cx, |previews, _| previews.end_capturing(&user_id));
        }
        self.applied_keys = RoomChangeKeys::default();
        self.session_timer_task = None;
        self.release_navigation_room_if_detached(cx);
    }

    fn direct_call_replacement(&self) -> AnyElement {
        if self.presented_as_call_overlay {
            return Empty.into_any_element();
        }
        let Some(call_id) = self.target_direct_call_id() else {
            return Empty.into_any_element();
        };
        let in_sheet = self.presented_in_room_sheet;
        let on_minimize = self.handler(move |this, _, cx| {
            if in_sheet {
                this.interactions.update(cx, |interactions, cx| interactions.minimize_room(cx));
            } else {
                let target = this.target.clone();
                this.interactions.update(cx, |interactions, cx| {
                    interactions.present_room(target, cx);
                    interactions.minimize_room(cx);
                });
                this.pop_navigation(cx);
            }
        });
        let on_terminal_settled = self.handler(move |this, _, cx| {
            this.interactions.update(cx, |interactions, cx| interactions.dismiss_room_presentation(cx));
            if !in_sheet {
                this.pop_navigation(cx);
            }
        });
        RiseRTCDirectCallScreen::new(None, Some(call_id), on_minimize, on_terminal_settled).into_any_element()
    }

    fn target_direct_call_id(&self) -> Option<String> {
        match &self.target {
            RiseRTCRoomTarget::DirectCall { call_id } => Some(call_id.clone()),
            _ => None,
        }
    }

    fn pop_navigation(&self, cx: &mut Context<Self>) {
        if let Some(navigation_store) = &self.navigation_store {
            navigation_store.update(cx, |navigation_store, cx| navigation_store.pop(cx));
        }
    }

    fn standard_room_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let entity = cx.entity();
        let on_key = entity.clone();
        div()
            .id("rise-rtc-room-body")
            .track_focus(&self.focus_handle)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                on_key.update(cx, |this, cx| this.handle_key(event, cx));
            })
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.bg._000)
            .child(
                container_query(move |size: Size<Pixels>, window: &mut Window, cx: &mut App| {
                    entity.update(cx, |this, cx| this.room_content(size, window, cx))
                })
                .size_full(),
            )
            .when(self.picture_in_picture_supported(), |body| {
                body.child(div().absolute().top_0().left_0().w(px(160.0)).h(px(90.0)).opacity(0.001).child(RiseRTCPictureInPictureLayerHost))
            })
            .when(self.presented_in_room_sheet, |body| body.child(Self::window_mode_toggle(theme.text.primary, cx)))
            .when(self.audio_routes_visible, |body| body.child(self.audio_route_sheet(&theme, cx)))
            .when(self.audio_inputs_visible, |body| body.child(self.audio_input_sheet(&theme, cx)))
            .when(self.stream_settings_visible, |body| body.child(self.stream_settings_sheet(&theme, cx)))
            .children(self.voice_invite_sheet(cx))
            .into_any_element()
    }

    /// bottom trailing switch between a call in its own window and one inside the shell window
    pub(crate) fn window_mode_toggle(tint: Hsla, cx: &App) -> impl IntoElement + use<> {
        let separate = RiseRTCRoomPresentationHost::uses_separate_window(cx);
        let label = tr(if separate { "desktop_rtc_call_in_app" } else { "desktop_rtc_call_in_window" });
        div()
            .absolute()
            .right(px(16.0 + Self::horizontal_safe_inset()))
            .bottom(px(16.0 + Self::bottom_safe_inset()))
            .child(Self::glass_circle(
                "rtc-room-window-mode",
                label,
                36.0,
                move |_, cx| RiseRTCRoomPresentationHost::set_uses_separate_window(!separate, cx),
                IconUi::sized(if separate { "dock.rectangle" } else { "macwindow" }, px(16.0), tint, false),
            ))
    }

    fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.key == "escape" && !keystroke.modifiers.modified() {
            if self.gear_menu_presented {
                return;
            }
            cx.stop_propagation();
            if self.stream_settings_visible {
                self.stream_settings_visible = false;
            } else if self.audio_inputs_visible {
                self.audio_inputs_visible = false;
            } else if self.audio_routes_visible {
                self.audio_routes_visible = false;
            } else if self.focused_stream_user_id(cx).is_some() {
                self.show_participant_grid(cx);
            } else {
                self.minimize_or_close(cx);
            }
            cx.notify();
            return;
        }
        if !keystroke.modifiers.secondary() || !keystroke.modifiers.shift {
            return;
        }
        match keystroke.key.as_str() {
            "m" => {
                if self.can_publish_microphone(cx) || self.displayed_microphone_enabled() {
                    self.toggle_microphone(cx);
                }
            }
            "v" => {
                if self.can_publish_camera(cx) || self.displayed_camera_enabled() {
                    self.toggle_camera(cx);
                }
            }
            "e" => {
                if self.can_publish_screen(cx) || self.displayed_screen_share_enabled() {
                    self.toggle_screen_share(cx);
                }
            }
            "h" => {
                if self.shows_raise_hand_pill(cx) {
                    self.toggle_raised_hand(cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn room_content(&mut self, geometry: Size<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        if self.displayed_room.is_some() {
            return self.live_room_content(geometry, &theme, window, cx);
        }
        if self.error_visible || self.rtc.read(cx).connection_state(cx) == RiseRTCConnectionState::Failed {
            let in_sheet = self.presented_in_room_sheet;
            let retry = self.handler(|this, _, cx| {
                let task = this.connect_if_needed(true, cx);
                task.detach();
            });
            let close = self.handler(|this, _, cx| this.close_room(cx));
            let label = tr(if in_sheet { "rtc_minimize" } else { "common_back" });
            return div()
                .relative()
                .size_full()
                .child(
                    vstack(16.0)
                        .size_full()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized("exclamationmark.triangle.fill", px(34.0), theme.semantic.error_100, false))
                        .child(
                            MainText::new(tr("rtc_join_failed"))
                                .px(17.0)
                                .tac(TextAlignT::Center)
                                .font_weight(FontWeight::SEMIBOLD),
                        )
                        .child(
                            system_button("rtc-room-retry", BorderedStyle::Prominent)
                                .tint(theme.primary._100)
                                .on_press(retry)
                                .child(tr("rtc_retry")),
                        ),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(14.0 + RiseRTCScreenSafeArea::horizontal()))
                        .top(px((self.top_safe_inset() + 6.0).max(6.0)))
                        .child(Self::glass_circle(
                            "rtc-room-failed-close",
                            label,
                            44.0,
                            close,
                            IconUi::sized(
                                if in_sheet { "chevron.down" } else { "chevron.backward" },
                                px(17.0),
                                theme.text.primary,
                                false,
                            ),
                        )),
                )
                .into_any_element();
        }
        vstack(14.0)
            .size_full()
            .items_center()
            .justify_center()
            .child(progress_view().control_size(ControlSize::Large))
            .child(MainText::new(tr("rtc_connecting")).px(15.0).color(theme.text.secondary))
            .into_any_element()
    }

    fn live_room_content(
        &mut self,
        geometry: Size<Pixels>,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let width = f32::from(geometry.width);
        let height = f32::from(geometry.height);
        let focused = self.focused_stream_user_id(cx);
        let participant = focused
            .as_ref()
            .and_then(|user_id| self.visible_participants.iter().find(|participant| &participant.user_id == user_id).cloned());

        let primary = match (&focused, participant) {
            (Some(user_id), Some(participant)) => match self.focusable_track(user_id, cx) {
                Some(track) => self.stream_stage(track, &participant, theme, geometry, window, cx),
                None => div()
                    .id("rtc-stage-placeholder")
                    .size_full()
                    .on_click({
                        let toggle = self.handler(|this, _, cx| this.toggle_chrome(cx));
                        move |_: &ClickEvent, window, cx| toggle(window, cx)
                    })
                    .child(self.stream_stage_placeholder(&participant, theme, cx))
                    .into_any_element(),
            },
            _ => {
                let top = self.canvas_top_inset(focused.is_some(), window);
                let bottom = self.canvas_bottom_inset(focused.is_some(), window);
                let toggle = self.handler(|this, _, cx| this.toggle_chrome(cx));
                div()
                    .relative()
                    .size_full()
                    .child(div().id("rtc-room-canvas-tap").absolute().inset_0().on_click(move |_: &ClickEvent, window, cx| toggle(window, cx)))
                    .child(
                        div().absolute().left_0().right_0().top(px(top)).bottom(px(bottom)).block_mouse_except_scroll().child(self.participant_canvas(
                            theme,
                            width,
                            height - top - bottom,
                            window,
                            cx,
                        )),
                    )
                    .into_any_element()
            }
        };

        let animation = self.chrome_animation(cx);
        let visible = self.chrome_visible;
        let horizontal = 14.0 + Self::horizontal_safe_inset();
        let header = self.room_header(theme, focused.is_some(), (width - horizontal * 2.0).max(0.0), window, cx);
        let dock = self.control_dock(theme, width, focused.is_some(), cx);
        let header = Self::transitioned(
            "rtc-room-header-presence",
            visible,
            Transition::move_edge(Edge::Top).combined(Transition::opacity()),
            animation,
            window,
            cx,
            move || div().w_full().child(header),
        );
        let dock = Self::transitioned(
            "rtc-room-dock-presence",
            visible,
            Transition::move_edge(Edge::Bottom).combined(Transition::opacity()),
            animation,
            window,
            cx,
            move || div().w_full().child(dock),
        );

        div()
            .relative()
            .size_full()
            .child(div().absolute().inset_0().child(primary))
            .child(
                div()
                    .absolute()
                    .left(px(horizontal))
                    .right(px(horizontal))
                    .top(px((self.top_safe_inset() + 6.0).max(6.0)))
                    .block_mouse_except_scroll_when_present(visible)
                    .children(header),
            )
            .child(
                div()
                    .absolute()
                    .left(px(horizontal))
                    .right(px(horizontal))
                    .bottom(px(RiseRTCScreenSafeArea::dock_bottom_padding()))
                    .block_mouse_except_scroll_when_present(visible)
                    .children(dock),
            )
            .into_any_element()
    }

    fn canvas_top_inset(&self, has_focused_stream: bool, window: &Window) -> f32 {
        if has_focused_stream {
            return 0.0;
        }
        let top = self.top_safe_inset();
        top + 66.0 * self.chrome_fraction(window)
    }

    fn canvas_bottom_inset(&self, has_focused_stream: bool, window: &Window) -> f32 {
        if has_focused_stream {
            return 0.0;
        }
        let bottom = Self::bottom_safe_inset();
        bottom + (self.dock_reserved_height(window) - bottom) * self.chrome_fraction(window)
    }

    fn chrome_fraction(&self, window: &Window) -> f32 {
        self.chrome_progress.frame_value(RiseClock::monotonic(), window)
    }

    fn chrome_animation(&self, cx: &App) -> Option<MotionSpec> {
        if reduce_motion(cx) { None } else { Some(spring(0.34, 0.86)) }
    }

    fn set_chrome_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.chrome_visible = visible;
        let animation = self.chrome_animation(cx);
        self.chrome_progress.set_in(if visible { 1.0 } else { 0.0 }, animation, RiseClock::monotonic(), cx);
        cx.notify();
    }

    /// shows content with a transition under the callers animation,
    /// the state keeps the leaving copy alive until the transition ends
    fn transitioned<E: Styled + IntoElement + 'static>(
        id: impl Into<ElementId>,
        visible: bool,
        transition: Transition,
        animation: Option<MotionSpec>,
        window: &mut Window,
        cx: &mut App,
        content: impl FnOnce() -> E,
    ) -> Option<Transitioned> {
        let state = window.use_keyed_state(id.into(), cx, move |_, _| {
            Presence::new(visible.then_some(())).with_transition(transition)
        });
        let now = RiseClock::monotonic();
        state.update(cx, |presence, cx| presence.set_in(visible.then_some(()), animation, now, cx));
        state.read(cx).render_transitioned(now, window, move |_| content())
    }

    /// round glass button with its hover hint, the label is the only name an icon button has
    fn glass_circle(
        id: &'static str,
        label: String,
        size: f32,
        action: impl Fn(&mut Window, &mut App) + 'static,
        icon: Option<gpui::Svg>,
    ) -> impl IntoElement {
        div().id(SharedString::from(format!("{id}-hint"))).tooltip_label(label.clone()).child(
            LiquidGlassButton::new(id, LiquidGlassButtonShape::Circle)
                .size(size)
                .haptic_on_press(false)
                .accessibility_label(Some(label))
                .on_press(action)
                .children(icon),
        )
    }

    fn room_header(
        &mut self,
        theme: &AppTheme,
        has_focused_stream: bool,
        row_width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let back_label = tr(if has_focused_stream {
            "rtc_show_grid"
        } else if self.presented_in_room_sheet {
            "rtc_minimize"
        } else {
            "common_back"
        });
        let back_icon = if !has_focused_stream && self.presented_in_room_sheet { "chevron.down" } else { "chevron.backward" };
        let back = self.handler(move |this, _, cx| {
            if has_focused_stream {
                this.show_participant_grid(cx);
            } else {
                this.minimize_or_close(cx);
            }
        });
        let flip_camera = (self.displayed_camera_enabled() && self.media.read(cx).can_flip_camera()).then(|| {
            let flip = self.handler(|this, _, cx| {
                let next = if this.camera_position(cx) == RiseRTCCameraPosition::Front {
                    RiseRTCCameraPosition::Back
                } else {
                    RiseRTCCameraPosition::Front
                };
                let interactions = this.interactions.clone();
                this.perform("flip-camera".to_owned(), move |cx| {
                    interactions.update(cx, |interactions, cx| interactions.switch_camera(next, cx))
                }, cx);
            });
            div().pr(px(8.0)).child(Self::glass_circle(
                "rtc-room-flip-camera",
                tr("rtc_flip_camera"),
                44.0,
                flip,
                IconUi::sized("arrow.triangle.2.circlepath.camera.fill", px(15.0), theme.text.primary, false),
            ))
        });
        let pill = self.header_pill(theme, row_width, window, cx);

        div()
            .relative()
            .w_full()
            .h(px(45.0))
            .flex()
            .items_center()
            .justify_center()
            .child(pill)
            .child(
                hstack(0.0)
                    .absolute()
                    .inset_0()
                    .items_center()
                    .child(Self::glass_circle(
                        "rtc-room-back",
                        back_label,
                        44.0,
                        back,
                        IconUi::sized(back_icon, px(17.0), theme.text.primary, false),
                    ))
                    .child(spacer())
                    .children(flip_camera)
                    .child(self.gear_menu(theme, cx)),
            )
            .into_any_element()
    }

    fn gear_menu(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let mut items = vec![
            ContextMenuItem::new(
                0,
                tr("rtc_audio_output"),
                Some("speaker.wave.2.fill".to_owned()),
                "audio-output",
                self.handler(|this, _, cx| {
                    this.audio_routes_visible = true;
                    cx.notify();
                }),
            ),
            ContextMenuItem::new(
                1,
                tr("rtc_voice_settings"),
                Some("mic.and.signal.meter.fill".to_owned()),
                "voice-settings",
                self.handler(|this, _, cx| {
                    this.audio_inputs_visible = true;
                    cx.notify();
                }),
            ),
        ];
        let sharing = self.displayed_screen_share_enabled();
        if self.can_publish_screen(cx) || sharing {
            items.push(ContextMenuItem::new(
                2,
                tr(if sharing { "rtc_stop_stream" } else { "rtc_stream_settings" }),
                Some(if sharing { "rectangle.on.rectangle.slash" } else { "rectangle.on.rectangle" }.to_owned()),
                if sharing { "stop_stream" } else { "stream_settings" },
                self.handler(move |this, _, cx| {
                    if sharing {
                        this.stop_screen_share(cx);
                    } else {
                        this.open_stream_settings(cx);
                    }
                }),
            ));
        }
        if self.reportable_chat_id(cx).is_some() {
            items.push(
                ContextMenuItem::new(
                    3,
                    tr("ctx_report"),
                    Some("exclamationmark.bubble".to_owned()),
                    "report",
                    self.handler(|this, _, cx| {
                        if let Some(chat_id) = this.reportable_chat_id(cx) {
                            ReportSheetStore::shared(cx)
                                .update(cx, |store, cx| store.present(ReportTarget::Chat(chat_id), None, None, cx));
                        }
                    }),
                )
                .destructive(true),
            );
        }
        let this = self.this.clone();
        div().id("rtc-room-gear-hint").tooltip_label(tr("rtc_voice_settings")).child(
            ContextMenuUi::new(
                "rtc-room-gear",
                items,
                PageHeaderToolbarSlot::new(div().children(IconUi::sized("gearshape.fill", px(16.0), theme.text.primary, false))),
            )
            .shows_item_icons()
            .on_presented_change(move |presented, _, cx| {
                let _ = this.update(cx, |view, _| view.gear_menu_presented = presented);
            }),
        )
        .into_any_element()
    }

    fn header_pill(&self, theme: &AppTheme, row_width: f32, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut subtitle = self.subtitle(cx);
        if let Some(started_at_ms) = self.session_started_at_ms(cx) {
            let now_ms = (RiseClock::now().time_interval_since_1970() * 1000.0) as i64;
            let seconds = ((now_ms - started_at_ms).max(0) as f64) / 1000.0;
            subtitle = format!("{subtitle} \u{b7} {}", RiseFormat::duration_time_interval(seconds));
        }
        self.header_pill_body(theme, subtitle, row_width, window, cx)
    }

    fn header_pill_body(
        &self,
        theme: &AppTheme,
        subtitle: String,
        row_width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = self.title(cx);
        let width = self.room_header_pill_width(&title, &subtitle, row_width, window, cx);
        vstack(1.0)
            .items_center()
            .px(px(13.0))
            .py(px(7.0))
            .w(px(width))
            .child(
                MainText::new(title)
                    .px(15.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD)
                    .number_of_lines(1),
            )
            .child(
                MainText::new(subtitle)
                    .px(11.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary)
                    .number_of_lines(1)
                    .monospaced_digits(true),
            )
            .liquid_chrome_capsule()
    }

    fn text_width(text: &str, size: f32, weight: FontWeight, window: &Window) -> f32 {
        let mut font = window.text_style().font();
        font.weight = weight;
        let run = TextRun {
            len: text.len(),
            font,
            color: black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(text.to_owned().into(), px(size), &[run], None);
        f32::from(line.width)
    }

    fn room_header_pill_width(&self, title: &str, subtitle: &str, row_width: f32, window: &Window, cx: &mut Context<Self>) -> f32 {
        let delta = ThemeStore::shared(cx).read(cx).text_delta();
        let title_width = Self::text_width(title, (15.0 + delta).max(1.0), FontWeight::BOLD, window).ceil();
        let mask: String = subtitle.chars().map(|character| if character.is_numeric() { '0' } else { character }).collect();
        let subtitle_width = Self::text_width(&mask, (11.0 + delta).max(1.0), FontWeight::NORMAL, window).ceil();
        let trailing_slots = if self.displayed_camera_enabled() { 96.0 } else { 52.0 };
        let row_cap = row_width - 2.0 * (trailing_slots + HEADER_PILL_SLOT_GAP);
        let cap = (218.0_f32 - trailing_slots).max(row_cap).max(94.0);
        cap.min((title_width.max(subtitle_width) + 28.0).max(94.0))
    }

    fn participant_canvas(
        &mut self,
        theme: &AppTheme,
        width: f32,
        viewport_height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let has_more = self.displayed_room.as_deref().is_some_and(|room| room.has_more_participants);
        let body = if self.room_mode(cx) == RiseRTCRoomMode::Stage {
            self.stage_canvas(theme, width, window, cx)
        } else {
            let participants = self.visible_participants.clone();
            self.participant_grid(&participants, theme, width, true, Some(viewport_height), window, cx)
        };
        let padding = self.grid_horizontal_padding(window);
        let toggle = self.handler(|this, _, cx| this.toggle_chrome(cx));
        let load_more = has_more.then(|| {
            let press = self.handler(|this, _, cx| {
                let interactions = this.interactions.clone();
                this.perform("load-more".to_owned(), move |cx| {
                    interactions.update(cx, |interactions, cx| interactions.load_more_participants(cx))
                }, cx);
            });
            let label = if self.action_ids.contains("load-more") {
                progress_view().into_any_element()
            } else {
                MainText::new(tr("common_show_more"))
                    .px(14.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.primary._100)
                    .into_any_element()
            };
            plain("rtc-room-load-more")
                .on_press(press)
                .w_full()
                .flex()
                .justify_center()
                .py(px(14.0))
                .child(label)
                .liquid_chrome_rounded_rectangle_with(f32::from(theme.radius._300), 0.12, None, true)
        });
        let interactions = self.interactions.clone();
        scroll_view("rtc-room-canvas")
            .scroll_disabled(self.hold_context_menu.read(cx).active_request().is_some())
            .shows_indicators(false)
            .refreshable(move |_, cx| {
                let refresh = interactions.update(cx, |interactions, cx| interactions.refresh_active_room(cx));
                cx.spawn(async move |_| {
                    let _ = refresh.await;
                })
            })
            .child(
                div()
                    .relative()
                    .w_full()
                    .min_h(px(viewport_height.floor().max(0.0)))
                    .child(div().id("rtc-room-content-tap").absolute().inset_0().on_click(move |_: &ClickEvent, window, cx| toggle(window, cx)))
                    .child(
                        vstack(14.0)
                            .relative()
                            .w_full()
                            .px(px(padding))
                            .py(px(8.0))
                            .child(body)
                            .children(load_more),
                    ),
            )
            .into_any_element()
    }

    fn stream_stage(
        &mut self,
        track: Rc<VideoTrack>,
        participant: &RiseRTCParticipant,
        theme: &AppTheme,
        bounds: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let user_id = participant.user_id.clone();
        let screen = self.screen_track(&user_id, cx);
        let camera = self.camera_track(&user_id, cx);
        let inset_track = match (&screen, &camera) {
            (Some(screen), Some(camera)) => Some(if Rc::ptr_eq(&track, screen) { camera.clone() } else { screen.clone() }),
            _ => None,
        };
        let inset_is_screen = match (&inset_track, &screen) {
            (Some(inset), Some(screen)) => Rc::ptr_eq(inset, screen),
            _ => false,
        };
        let is_own_stage = user_id == self.current_user_id(cx);
        let primary_is_own_camera = is_own_stage && camera.as_ref().is_some_and(|camera| Rc::ptr_eq(&track, camera));
        let shows_identity_badge = !is_own_stage;
        let chrome_visible = self.chrome_visible;
        let top_safe = self.top_safe_inset();
        let horizontal_safe = Self::horizontal_safe_inset();
        let inset_top_padding = match (shows_identity_badge, chrome_visible) {
            (true, true) => top_safe + 70.0,
            (true, false) => 62.0,
            (false, true) => top_safe + 12.0,
            (false, false) => 12.0,
        };
        let animation = self.chrome_animation(cx);
        let toggle = self.handler(|this, _, cx| this.toggle_chrome(cx));

        let badge_source = self.participant_avatar_source(participant, cx);
        let badge_name = self.participant_avatar_name(participant, cx);
        let badge_user_id = user_id.clone();
        let badge = Self::transitioned(
            "rtc-stage-identity-badge",
            shows_identity_badge,
            Transition::scale_default().combined(Transition::opacity()),
            animation,
            window,
            cx,
            move || {
                div()
                    .absolute()
                    .top(px(12.0 + if chrome_visible { top_safe } else { 0.0 }))
                    .right(px(12.0))
                    .child(
                        UserLogoUi::new("rtc-stage-identity-logo")
                            .source(badge_source.map(SharedString::from))
                            .name(Some(SharedString::from(badge_name)))
                            .user_id(Some(SharedString::from(badge_user_id)))
                            .size(if chrome_visible { 44.0 } else { 34.0 })
                            .show_story_ring(false)
                            .image_cache_policy(CleverImageCachePolicy::MemoryOnly),
                    )
            },
        );

        let inset = inset_track.map(|inset_track| {
            let insets = EdgeInsets::new(
                inset_top_padding,
                12.0 + horizontal_safe,
                if chrome_visible { self.dock_reserved_height(window) } else { Self::bottom_safe_inset() + 12.0 },
                12.0 + horizontal_safe,
            );
            let on_position_change = self.handler_with_position();
            let swap_user_id = user_id.clone();
            let on_tap: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(self.handler(move |this, _, cx| {
                this.swapped_stage_user_id =
                    if this.swapped_stage_user_id.as_deref() == Some(swap_user_id.as_str()) { None } else { Some(swap_user_id.clone()) };
                cx.notify();
            }));
            let surface = if inset_is_screen && is_own_stage {
                div().size_full().child(RiseRTCOwnScreenShareWall::new("rtc-stage-inset-wall", theme.clone()).compact(true))
            } else {
                div().size_full().bg(theme.bg._300).child(RiseRTCVideoSurface::new(
                    "rtc-stage-inset-surface",
                    "stage-inset",
                    inset_track,
                    if inset_is_screen { VideoViewLayoutMode::Fit } else { VideoViewLayoutMode::Fill },
                    if !inset_is_screen && is_own_stage { VideoViewMirrorMode::Auto } else { VideoViewMirrorMode::Off },
                ))
            };
            RiseRTCFloatingVideoWindow::new(
                "rtc-stage-inset",
                self.stage_inset_position,
                size(108.0, 76.0),
                insets,
                size(f32::from(bounds.width), f32::from(bounds.height)),
                on_position_change,
                Some(on_tap),
                surface,
            )
        });

        let dock_reserved = self.dock_reserved_height(window);
        let caption = self.participant_caption(participant, cx);
        let caption = Self::transitioned(
            "rtc-stage-caption-presence",
            chrome_visible,
            Transition::move_edge(Edge::Bottom).combined(Transition::opacity()),
            animation,
            window,
            cx,
            move || {
                div()
                    .absolute()
                    .left(px(14.0 + horizontal_safe))
                    .right(px(14.0 + horizontal_safe))
                    .bottom(px(14.0 + dock_reserved))
                    .h(px(26.0))
                    .child(caption)
            },
        );

        let stage_controls = (!is_own_stage && chrome_visible).then(|| {
            let is_stream = screen.as_ref().is_some_and(|screen| Rc::ptr_eq(&track, screen));
            let controls = self.stage_controls_view(&user_id, is_stream, cx);
            AnyView::from(controls).cached(
                StyleRefinement::default()
                    .absolute()
                    .right(px(14.0 + horizontal_safe))
                    .bottom(px(14.0 + dock_reserved))
                    .w(px(RiseRTCStageControls::WIDTH))
                    .h(px(RiseRTCStageControls::HEIGHT)),
            )
        });
        let speaking = self.media.read(cx).is_speaking(&user_id);
        let radius = f32::from(theme.radius._300);
        let primary_is_own_screen = is_own_stage && screen.as_ref().is_some_and(|screen| Rc::ptr_eq(&track, screen));
        let primary = if primary_is_own_screen {
            let stop = self.handler(|this, _, cx| this.stop_screen_share(cx));
            div()
                .id("rtc-stage-own-share")
                .size_full()
                .on_click(move |_, window, cx| toggle(window, cx))
                .child(RiseRTCOwnScreenShareWall::new("rtc-stage-own-share-wall", theme.clone()).on_stop(stop))
                .into_any_element()
        } else {
            RiseRTCZoomableVideoSurface::new(
                "rtc-stage-surface",
                "stage",
                track,
                VideoViewLayoutMode::Fit,
                if primary_is_own_camera { VideoViewMirrorMode::Auto } else { VideoViewMirrorMode::Off },
                toggle,
            )
            .into_any_element()
        };
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.bg._000)
            .child(primary)
            .children(badge)
            .children(inset)
            .children(caption)
            .children(stage_controls)
            .when(speaking, |stage| {
                stage.child(
                    div()
                        .absolute()
                        .inset(px(4.0))
                        .child(centred_stroke(theme.semantic.success_100, 3.0, Shape::continuous(radius))),
                )
            })
            .into_any_element()
    }

    fn stage_controls_view(&mut self, user_id: &str, is_stream: bool, cx: &mut Context<Self>) -> Entity<RiseRTCStageControls> {
        if let Some(controls) = self.stage_controls.as_ref().filter(|controls| {
            let controls = controls.read(cx);
            controls.user_id == user_id && controls.is_stream == is_stream
        }) {
            return controls.clone();
        }
        let room = self.this.clone();
        let controls = cx.new(|_| RiseRTCStageControls {
            user_id: user_id.to_owned(),
            is_stream,
            volume_open: false,
            dragged_percent: None,
            room,
        });
        self.stage_controls = Some(controls.clone());
        controls
    }

    fn open_mini_player(&mut self, cx: &mut Context<Self>) {
        let Some((user_id, _, track)) = self.picture_in_picture_source(cx) else {
            return;
        };
        let identity = self.picture_in_picture_source_identity(cx);
        let track_key = track.track_key().to_owned();
        RiseRTCCallPictureInPictureController::shared(cx)
            .update(cx, |controller, cx| controller.float_stream(track_key, identity, user_id, cx));
        self.minimize_or_close(cx);
    }

    fn handler_with_position(&self) -> impl Fn(UnitPoint, &mut Window, &mut App) + 'static + use<> {
        let this = self.this.clone();
        move |position: UnitPoint, _: &mut Window, cx: &mut App| {
            let _ = this.update(cx, |view, cx| {
                view.stage_inset_position = position;
                cx.notify();
            });
        }
    }

    fn stream_stage_placeholder(&self, participant: &RiseRTCParticipant, theme: &AppTheme, cx: &App) -> impl IntoElement + use<> {
        let is_own = participant.user_id == self.current_user_id(cx);
        let is_screen_sharing = if is_own { self.displayed_screen_share_enabled() } else { participant.screen_share_enabled };
        div().size_full().bg(theme.bg._000).child(
            vstack(14.0)
                .size_full()
                .items_center()
                .justify_center()
                .children(IconUi::sized(
                    if is_screen_sharing { "rectangle.on.rectangle" } else { "video.fill" },
                    px(34.0),
                    theme.text.secondary,
                    false,
                ))
                .child(
                    MainText::new(tr(if is_screen_sharing { "rtc_screen_share" } else { "rtc_video" }))
                        .px(15.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::MEDIUM)
                        .color(theme.text.secondary),
                )
                .child(progress_view().tint(theme.text.secondary)),
        )
    }

    fn stage_canvas(&mut self, theme: &AppTheme, width: f32, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let speakers = self.speakers();
        let audience = self.stage_audience_rows();
        let hands = self.raised_hand_user_ids();
        let current_user_id = self.current_user_id(cx);
        let capabilities = self.moderation_capabilities(cx);
        let speakers_section = (!speakers.is_empty()).then(|| {
            let title = self.participant_section_title(tr("rtc_speakers"), speakers.len(), theme);
            let grid = self.participant_grid(&speakers, theme, width, false, None, window, cx);
            vstack(14.0).w_full().child(title).child(grid)
        });
        let audience_section = (!audience.is_empty()).then(|| {
            let title = self.participant_section_title(tr("rtc_audience"), audience.len(), theme);
            let name_user_id = current_user_id.clone();
            let on_action = self.this.clone();
            let on_tap = self.this.clone();
            vstack(14.0).w_full().child(title).child(RiseRTCStageListenerList::new(
                "rtc-stage-audience",
                audience,
                hands,
                current_user_id,
                capabilities,
                move |participant, _| {
                    Self::participant_name_user_id(&participant.user_id, participant.profile.as_ref(), &name_user_id)
                },
                move |action, participant, _, cx| {
                    let _ = on_action.update(cx, |view, cx| view.perform_menu_action(action, participant, cx));
                },
                move |participant, _, cx| {
                    let _ = on_tap.update(cx, |view, cx| view.open_participant_profile(participant, cx));
                },
            ))
        });
        let invite_row = self.can_share_voice_invite(cx).then(|| self.stage_invite_row(theme));
        vstack(14.0)
            .w_full()
            .children(speakers_section)
            .children(audience_section)
            .children(invite_row)
            .into_any_element()
    }

    fn stage_invite_row(&self, theme: &AppTheme) -> impl IntoElement + use<> {
        let open = self.handler(|this, _, cx| {
            this.voice_invite_visible = true;
            cx.notify();
        });
        div().id("rtc-stage-invite-hint").w_full().tooltip_label(tr("voice_invite_sheet_title")).child(
            plain("rtc-stage-invite")
                .on_press(open)
                .w_full()
                .child(
                    hstack(RiseRTCStageMetrics::ICON_SPACING)
                        .items_center()
                        .w_full()
                        .min_h(px(RiseRTCStageMetrics::ROW_HEIGHT))
                        .px(px(RiseRTCStageMetrics::HORIZONTAL_PADDING))
                        .rounded(px(GroupedCardMetrics::CORNER_RADIUS))
                        .border(px(1.5))
                        .border_dashed()
                        .border_color(theme.text.secondary.opacity(0.35))
                        .child(
                            div()
                                .size(px(RiseRTCStageMetrics::AVATAR_SIZE))
                                .rounded_full()
                                .bg(theme.bg._300)
                                .flex()
                                .items_center()
                                .justify_center()
                                .children(IconUi::sized("plus", px(16.0), theme.text.primary, false)),
                        )
                        .child(
                            MainText::new(tr("voice_invite_tile"))
                                .px(15.0)
                                .font_weight(FontWeight::MEDIUM)
                                .color(theme.text.secondary)
                                .number_of_lines(1),
                        )
                        .child(spacer()),
                ),
        )
    }

    fn voice_invite_tile(&self, theme: &AppTheme) -> impl IntoElement + use<> {
        let open = self.handler(|this, _, cx| {
            this.voice_invite_visible = true;
            cx.notify();
        });
        let radius = theme.radius._300;
        div().id("rtc-room-invite-hint").size_full().tooltip_label(tr("voice_invite_sheet_title")).child(
            plain("rtc-room-invite").on_press(open).size_full().child(
                vstack(10.0)
                    .size_full()
                    .items_center()
                    .justify_center()
                    .rounded(radius)
                    .border(px(1.5))
                    .border_dashed()
                    .border_color(theme.text.secondary.opacity(0.35))
                    .child(
                        div()
                            .size(px(56.0))
                            .rounded_full()
                            .bg(theme.bg._200)
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized("plus", px(26.0), theme.text.primary, false)),
                    )
                    .child(
                        MainText::new(tr("voice_invite_tile"))
                            .px(14.0)
                            .font_weight(FontWeight::MEDIUM)
                            .color(theme.text.secondary)
                            .number_of_lines(1),
                    ),
            ),
        )
    }

    fn voice_invite_sheet(&self, cx: &mut Context<Self>) -> Option<RiseRTCVoiceInviteBottomSheet> {
        if !self.voice_invite_visible {
            return None;
        }
        let chat_id = self.voice_invite_chat_id()?;
        let binding = Binding::entity(
            cx.entity(),
            |this: &Self| this.voice_invite_visible,
            |this, value, cx| {
                this.voice_invite_visible = value;
                cx.notify();
            },
        );
        Some(RiseRTCVoiceInviteBottomSheet::new(binding, chat_id, self.voice_invite_topic_id()))
    }

    /// one dispatcher for both menus so the grid tile and
    /// the roster row cant disagree about what an entry does
    fn perform_menu_action(&mut self, action: RiseRTCRoomMenuAction, participant: &RiseRTCParticipant, cx: &mut Context<Self>) {
        let user_id = participant.user_id.clone();
        let interactions = self.interactions.clone();
        match action {
            RiseRTCRoomMenuAction::ToggleMuteForMe => {
                let muted = !self.rtc.read(cx).is_muted_for_me(&user_id, cx);
                interactions.update(cx, |interactions, cx| interactions.set_muted_for_me(&user_id, muted, cx));
            }
            RiseRTCRoomMenuAction::ResetVolume => {
                interactions.update(cx, |interactions, cx| interactions.reset_listener_volume(&user_id, cx));
            }
            RiseRTCRoomMenuAction::PromoteToSpeaker => {
                self.perform(format!("promote-{user_id}"), move |cx| {
                    interactions.update(cx, |interactions, cx| interactions.promote_to_speaker(&user_id, cx))
                }, cx);
            }
            RiseRTCRoomMenuAction::MoveToAudience => {
                self.perform(format!("audience-{user_id}"), move |cx| {
                    interactions.update(cx, |interactions, cx| interactions.move_to_audience(&user_id, cx))
                }, cx);
            }
            RiseRTCRoomMenuAction::ToggleMute => {
                let moderation = if participant.server_muted {
                    RiseRTCModerationAction::AllowUnmute
                } else {
                    RiseRTCModerationAction::ForceMute
                };
                self.perform(format!("mute-{user_id}"), move |cx| {
                    interactions.update(cx, |interactions, cx| interactions.moderate(&user_id, moderation, None, cx))
                }, cx);
            }
            RiseRTCRoomMenuAction::ToggleDeafen => {
                let moderation = if participant.server_deafened {
                    RiseRTCModerationAction::AllowUndeafen
                } else {
                    RiseRTCModerationAction::ForceDeafen
                };
                self.perform(format!("deafen-{user_id}"), move |cx| {
                    interactions.update(cx, |interactions, cx| interactions.moderate(&user_id, moderation, None, cx))
                }, cx);
            }
            RiseRTCRoomMenuAction::ToggleCamera => {
                let moderation = if participant.server_camera_blocked {
                    RiseRTCModerationAction::AllowCamera
                } else {
                    RiseRTCModerationAction::BlockCamera
                };
                self.perform(format!("camera-block-{user_id}"), move |cx| {
                    interactions.update(cx, |interactions, cx| interactions.moderate(&user_id, moderation, None, cx))
                }, cx);
            }
            RiseRTCRoomMenuAction::ToggleScreenShare => {
                let moderation = if participant.server_screen_share_blocked {
                    RiseRTCModerationAction::AllowScreenShare
                } else {
                    RiseRTCModerationAction::BlockScreenShare
                };
                self.perform(format!("screen-share-block-{user_id}"), move |cx| {
                    interactions.update(cx, |interactions, cx| interactions.moderate(&user_id, moderation, None, cx))
                }, cx);
            }
            RiseRTCRoomMenuAction::Kick => {
                self.perform(format!("kick-{user_id}"), move |cx| {
                    interactions.update(cx, |interactions, cx| {
                        interactions.moderate(&user_id, RiseRTCModerationAction::Kick, Some(Self::KICK_REASON), cx)
                    })
                }, cx);
            }
        }
    }

    fn open_participant_profile(&mut self, participant: &RiseRTCParticipant, cx: &mut Context<Self>) {
        if participant.user_id == self.current_user_id(cx) || RtcGuideSandboxHooks::owns_user(&participant.user_id, cx) {
            return;
        }
        if self.presented_in_room_sheet {
            self.interactions.update(cx, |interactions, cx| interactions.minimize_room(cx));
        }
        let route = RootRoute::UserPage {
            user_id: Some(participant.user_id.clone()),
            tag: participant.profile.as_ref().and_then(|profile| non_empty(&profile.tag)),
        };
        if let Some(navigation_store) = &self.navigation_store {
            navigation_store.update(cx, |navigation_store, cx| {
                navigation_store.push(route, NavigationStoreSettlingPush::default(), cx);
            });
        }
    }

    fn grid_horizontal_padding(&self, window: &Window) -> f32 {
        5.0 + 9.0 * self.chrome_fraction(window) + Self::horizontal_safe_inset()
    }

    /// two lazy columns, tile height is known so only tiles near the window are built
    fn participant_grid(
        &mut self,
        participants: &[RiseRTCParticipant],
        theme: &AppTheme,
        width: f32,
        shows_invite: bool,
        fit_height: Option<f32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut portrait_user_ids = self.media.read(cx).portrait_video_user_ids().clone();
        for participant in participants {
            if !participant.screen_share_enabled && self.screen_track(&participant.user_id, cx).is_none() {
                continue;
            }
            if let Some(preview) = self.stream_previews.read(cx).image(&participant.user_id) {
                let preview_size = preview.size(0);
                if preview_size.width.0 > 0 && preview_size.width.0 < preview_size.height.0 {
                    portrait_user_ids.insert(participant.user_id.clone());
                }
            }
        }
        let current_user_id = self.current_user_id(cx);
        let first_guest_id = participants.iter().find(|participant| participant.user_id != current_user_id).map(|participant| participant.user_id.clone());
        let column_width = ((width - self.grid_horizontal_padding(window) * 2.0 - 10.0) / 2.0).max(1.0);
        let shows_invite = shows_invite && self.can_share_voice_invite(cx);
        let tile_width = fit_height.map_or(column_width, |fit_height| {
            let slots = participants
                .iter()
                .map(|participant| if portrait_user_ids.contains(&participant.user_id) { 2 } else { 1 })
                .sum::<usize>()
                + usize::from(shows_invite);
            let rows = slots.div_ceil(2).max(1) as f32;
            let available = fit_height - GRID_FIT_VERTICAL_PADDING;
            let fitted = ((available - 10.0 * (rows - 1.0)) / rows).floor();
            if fitted >= GRID_FIT_MIN_TILE { column_width.min(fitted) } else { column_width }
        });
        let leading = column_width - tile_width;
        let stride = tile_width + 10.0;
        let viewport_height = f32::from(window.viewport_size().height);
        let margin = OFFSCREEN_GRID_ROWS * stride;
        let origin_y = self.grid_origin_y;

        let mut occupied_rows = [0_usize, 0_usize];
        let mut grid = div().relative().w_full();
        for participant in participants {
            let column = if occupied_rows[0] <= occupied_rows[1] { 0 } else { 1 };
            let is_portrait = portrait_user_ids.contains(&participant.user_id);
            let top = occupied_rows[column] as f32 * stride;
            let height = if is_portrait { tile_width * 2.0 + 10.0 } else { tile_width };
            occupied_rows[column] += if is_portrait { 2 } else { 1 };
            let on_screen = origin_y + top + height > -margin && origin_y + top < viewport_height + margin;
            if !on_screen {
                continue;
            }
            let guide_anchor = (first_guest_id.as_ref() == Some(&participant.user_id)).then_some(RiseGuideAnchor::RtcRoomFirstGuestTile);
            let card = self.participant_card(participant, theme, guide_anchor, size(px(tile_width), px(height)), window, cx);
            grid = grid.child(
                div()
                    .absolute()
                    .left(px(leading + column as f32 * stride))
                    .top(px(top))
                    .w(px(tile_width))
                    .h(px(height))
                    .child(card),
            );
        }
        if shows_invite {
            let column = if occupied_rows[0] <= occupied_rows[1] { 0 } else { 1 };
            let top = occupied_rows[column] as f32 * stride;
            occupied_rows[column] += 1;
            grid = grid.child(
                div()
                    .absolute()
                    .left(px(leading + column as f32 * stride))
                    .top(px(top))
                    .w(px(tile_width))
                    .h(px(tile_width))
                    .child(self.voice_invite_tile(theme)),
            );
        }
        let total_rows = occupied_rows[0].max(occupied_rows[1]);
        let total_height = if total_rows == 0 { 0.0 } else { total_rows as f32 * stride - 10.0 };
        let this = self.this.clone();
        grid.h(px(total_height))
            .child(measure("rtc-room-grid-geometry", move |bounds, _, cx| {
                let origin_y = f32::from(bounds.origin.y);
                let _ = this.update(cx, |view, cx| {
                    let previous_band = (view.grid_origin_y / stride).floor();
                    view.grid_origin_y = origin_y;
                    if (origin_y / stride).floor() != previous_band {
                        cx.notify();
                    }
                });
            }))
            .into_any_element()
    }

    fn participant_card(
        &mut self,
        participant: &RiseRTCParticipant,
        theme: &AppTheme,
        guide_anchor: Option<RiseGuideAnchor>,
        tile_size: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let user_id = participant.user_id.clone();
        let screen_track = self.screen_track(&user_id, cx);
        let is_screen_sharing = participant.screen_share_enabled || screen_track.is_some();
        let is_remote_tile = user_id != self.current_user_id(cx);
        self.capture_tile_preview(&user_id, screen_track.as_ref(), is_remote_tile && is_screen_sharing, cx);

        let is_remote = user_id != self.current_user_id(cx);
        let toggle = self.handler(|this, _, cx| this.toggle_chrome(cx));

        if !is_remote {
            let tile = self.participant_tile(participant, theme, window, cx).guide_anchor(guide_anchor);
            let open_inputs = self.handler(|this, _, cx| {
                let _ = impact(ImpactStyle::Medium);
                this.audio_inputs_visible = true;
                cx.notify();
            });
            return div()
                .id(tile_id(&user_id, "self-tile"))
                .size_full()
                .block_mouse_except_scroll()
                .on_click(move |_: &ClickEvent, window, cx| toggle(window, cx))
                .on_mouse_down(MouseButton::Right, move |_: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    open_inputs(window, cx);
                })
                .child(tile)
                .into_any_element();
        }

        let rtc = self.rtc.read(cx);
        let accessory = RiseRTCVolumeMenuAccessory::configuration(
            participant,
            RiseRTCVolumeMenuAccessory::shows_blocked_hint(
                rtc.listener_volume_percent(&user_id, cx),
                rtc.has_explicit_listener_volume(&user_id, cx),
            ),
            cx,
        );
        let capabilities = if self.is_connecting_to_displayed_room() {
            RiseRTCRoomMenuCapabilities::default()
        } else {
            self.moderation_capabilities(cx)
        };
        let outranked = RiseRTCRoomMenu::outranked(participant, &capabilities, cx);
        let entity = cx.entity();
        let actions: HoldContextMenuActionsProvider = {
            let participant = participant.clone();
            Rc::new(move |cx: &App| RiseRTCRoomMenu::actions(&participant, &capabilities, outranked, cx))
        };
        let content: HoldContextMenuSourceContent = {
            let theme = theme.clone();
            let participant = participant.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let theme = theme.clone();
                let participant = participant.clone();
                let tile = entity.update(cx, |view, cx| {
                    view.participant_tile(&participant, &theme, window, cx).guide_anchor(guide_anchor).into_any_element()
                });
                div().w(tile_size.width).h(tile_size.height).child(tile).into_any_element()
            })
        };
        let on_action = self.this.clone();
        let action_participant = participant.clone();
        div()
            .id(tile_id(&user_id, "remote-tile"))
            .size_full()
            .block_mouse_except_scroll()
            .child(
                HoldContextMenuContainer::new(tile_id(&user_id, "hold-tile"), HoldContextMenuAlignment::Leading, actions, content)
                    .show_emoji_list(false)
                    .accessory(accessory)
                    .live_action_titles(true)
                    .source_identity(Some(user_id))
                    .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                        let Some(action) = RiseRTCRoomMenuAction::from_raw_value(&id) else {
                            return;
                        };
                        let _ = on_action.update(cx, |view, cx| view.perform_menu_action(action, &action_participant, cx));
                    }))
                    .on_tap(Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| toggle(window, cx))),
            )
            .into_any_element()
    }

    fn capture_tile_preview(
        &mut self,
        user_id: &str,
        screen_track: Option<&Rc<VideoTrack>>,
        is_screen_sharing: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(track) = screen_track.filter(|_| is_screen_sharing) else {
            self.captured_preview_tracks.remove(user_id);
            return;
        };
        if self.captured_preview_tracks.get(user_id).map(String::as_str) == Some(track.track_key()) {
            return;
        }
        self.captured_preview_tracks.insert(user_id.to_owned(), track.track_key().to_owned());
        let previews = self.stream_previews.clone();
        let user_id = user_id.to_owned();
        let track = track.clone();
        cx.defer(move |cx| previews.update(cx, |previews, cx| previews.capture_once(&user_id, &track, cx)));
    }

    fn avatar_logo(&self, participant: &RiseRTCParticipant, name: &'static str, side: f32, cx: &App) -> UserLogoUi {
        UserLogoUi::new(tile_id(&participant.user_id, name))
            .source(self.participant_avatar_source(participant, cx).map(SharedString::from))
            .name(Some(SharedString::from(self.participant_avatar_name(participant, cx))))
            .user_id(Some(SharedString::from(participant.user_id.clone())))
            .size(side)
            .show_story_ring(false)
            .image_cache_policy(CleverImageCachePolicy::MemoryOnly)
    }

    fn participant_tile(
        &mut self,
        participant: &RiseRTCParticipant,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let user_id = participant.user_id.clone();
        let screen_track = self.screen_track(&user_id, cx);
        let camera_track = self.camera_track(&user_id, cx);
        let is_screen_sharing = participant.screen_share_enabled || screen_track.is_some();
        let stream_preview = self.stream_previews.read(cx).image(&user_id);
        let is_speaking = self.media.read(cx).is_speaking(&user_id);
        let is_remote = user_id != self.current_user_id(cx);
        let offers_watch = is_screen_sharing || participant.camera_enabled || camera_track.is_some();
        let radius = f32::from(theme.radius._300);
        let inset_radius = f32::from(theme.radius._200);
        let centered = || div().absolute().inset_0().flex().items_center().justify_center();

        let mut tile = div().relative().size_full().bg(theme.bg._200).clip_shape(Shape::continuous(radius));

        if !is_remote && (screen_track.is_some() || camera_track.is_some()) {
            tile = tile.child(centered().child(self.avatar_logo(participant, "logo-under", 76.0, cx)));
        }

        if !is_remote && screen_track.is_some() {
            tile = tile.child(
                div()
                    .absolute()
                    .inset_0()
                    .child(RiseRTCOwnScreenShareWall::new(tile_id(&user_id, "tile-screen-wall"), theme.clone()).compact(true)),
            );
        } else if let (false, Some(track)) = (is_remote, &camera_track) {
            tile = tile.child(div().absolute().inset_0().child(RiseRTCVideoSurface::new(
                tile_id(&user_id, "tile-camera"),
                "tile-camera",
                track.clone(),
                VideoViewLayoutMode::Fill,
                VideoViewMirrorMode::Auto,
            )));
        } else if is_screen_sharing {
            tile = match stream_preview {
                Some(preview) => tile.child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(img(preview).size_full().rounded(px(radius)).object_fit(ObjectFit::Cover)),
                ),
                None => tile.child(
                    centered()
                        .bg(theme.bg._300)
                        .rounded(px(radius))
                        .children(IconUi::sized("rectangle.on.rectangle", px(30.0), theme.text.secondary, false)),
                ),
            };
            tile = tile.child(div().absolute().inset_0().rounded(px(radius)).bg(Hsla::from(rgb(0x8e8e93)).opacity(0.22)));
        } else {
            tile = tile.child(centered().child(self.avatar_logo(participant, "logo", 76.0, cx)));
        }

        let inset_frame = || {
            div()
                .absolute()
                .top(px(9.0))
                .right(px(9.0))
                .w(px(62.0))
                .h(px(44.0))
        };
        if !is_remote && is_screen_sharing && screen_track.is_none() && camera_track.is_some() {
            tile = tile.child(
                inset_frame()
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme.bg._300.opacity(0.9))
                            .clip_shape(Shape::continuous(inset_radius))
                            .children(IconUi::sized("tv", px(13.0), white(), false)),
                    )
                    .child(centred_stroke(white().opacity(0.5), 1.0, Shape::continuous(inset_radius))),
            );
        }

        if let (false, true, Some(track)) = (is_remote, screen_track.is_some(), &camera_track) {
            tile = tile.child(
                inset_frame()
                    .child(div().size_full().clip_shape(Shape::continuous(inset_radius)).child(RiseRTCVideoSurface::new(
                        tile_id(&user_id, "tile-camera-inset"),
                        "tile-camera-inset",
                        track.clone(),
                        VideoViewLayoutMode::Fill,
                        VideoViewMirrorMode::Auto,
                    )))
                    .child(centred_stroke(white().opacity(0.5), 1.0, Shape::continuous(inset_radius))),
            );
        }

        if offers_watch {
            let watch_user_id = user_id.clone();
            let watch = self.handler(move |this, _, cx| this.select_stream(&watch_user_id, cx));
            tile = tile.child(
                centered().child(
                    div().id(tile_id(&user_id, "watch-hint")).block_mouse_except_scroll().child(
                        LiquidGlassButton::new(tile_id(&user_id, "watch"), LiquidGlassButtonShape::Capsule)
                            .horizontal_padding(12.0)
                            .vertical_padding(8.0)
                            .haptic_on_press(false)
                            .flattened(true)
                            .accessibility_label(Some(tr("rtc_watch_stream")))
                            .on_press(watch)
                            .child(
                                hstack(6.0)
                                    .items_center()
                                    .children(IconUi::sized(
                                        if is_screen_sharing { "tv" } else { "video.fill" },
                                        px(13.0),
                                        white(),
                                        false,
                                    ))
                                    .child(
                                        MainText::new(tr("rtc_watch_stream"))
                                            .px(12.0)
                                            .font_weight(FontWeight::BOLD)
                                            .color(white())
                                            .number_of_lines(1),
                                    ),
                            ),
                    ),
                ),
            );
        }

        tile = tile.child(div().absolute().inset(px(9.0)).child(self.participant_caption(participant, cx)));

        if is_screen_sharing && camera_track.is_none() && screen_track.is_none() {
            tile = tile.child(
                div()
                    .absolute()
                    .top(px(9.0))
                    .right(px(9.0))
                    .child(self.avatar_logo(participant, "logo-corner", 38.0, cx)),
            );
        }

        let target = if is_speaking { 1.0_f32 } else { 0.0 };
        let ring = window.use_keyed_state(tile_id(&user_id, "speaking-ring"), cx, move |_, _| Animated::new(target));
        let now = RiseClock::monotonic();
        ring.update(cx, |ring, cx| ring.set_in(target, ease_in_out(0.18), now, cx));
        let progress = ring.read(cx).frame_value(now, window);
        let line_width = 0.75 + (3.0 - 0.75) * progress;

        div()
            .relative()
            .size_full()
            .child(tile)
            .child(centred_stroke(theme.border._200.opacity(0.45 * (1.0 - progress)), line_width, Shape::continuous(radius)))
            .child(centred_stroke(theme.semantic.success_100.opacity(progress), line_width, Shape::continuous(radius)))
            .into_any_element()
    }

    /// bottom leading capsule, the parent gives it the width it may take
    fn participant_caption(&self, participant: &RiseRTCParticipant, cx: &App) -> impl IntoElement + use<> {
        let name = self.participant_name(participant, cx);
        let is_self = participant.user_id == self.current_user_id(cx);
        let (muted, deafened) = if is_self {
            let hushed = RiseMicrophoneTestHush::is_active(cx);
            (!self.displayed_microphone_enabled() || hushed, self.displayed_deafened() || hushed)
        } else {
            (
                !participant.microphone_enabled || participant.is_server_silenced(),
                participant.deafened || participant.server_deafened,
            )
        };
        let imposed = CaptionImposed {
            muted: participant.is_server_silenced(),
            deafened: participant.server_deafened,
            color: rise_ui::theme(cx).primary._100,
        };
        div().size_full().child(RiseRTCListenerVolumeContent::new(
            participant.user_id.clone(),
            move |percent, _, _| {
                let volume = Self::listener_volume_caption(is_self, percent);
                match volume {
                    Some((true, Some(text))) => {
                        container_query(move |size: Size<Pixels>, window: &mut Window, cx: &mut App| {
                            let delta = ThemeStore::shared(cx).read(cx).text_delta();
                            let name_width = Self::text_width(&name, (12.0 + delta).max(1.0), FontWeight::SEMIBOLD, window);
                            let text_width = Self::text_width(&text, (11.0 + delta).max(1.0), FontWeight::SEMIBOLD, window);
                            let glyphs = if deafened { 3.0 } else { 2.0 };
                            let row_width = name_width + text_width + glyphs * 15.0 + 16.0;
                            let shows_text = row_width <= f32::from(size.width);
                            Self::caption_row(name, muted, deafened, imposed, Some((true, Some(text))), shows_text)
                        })
                        .size_full()
                        .into_any_element()
                    }
                    volume => Self::caption_row(name, muted, deafened, imposed, volume, true).into_any_element(),
                }
            },
        ))
    }

    fn caption_row(
        name: String,
        muted: bool,
        deafened: bool,
        imposed: CaptionImposed,
        volume: Option<(bool, Option<String>)>,
        shows_volume_text: bool,
    ) -> impl IntoElement {
        let glyph = |symbol: &str| IconUi::sized(symbol, px(10.0), white(), false);
        let tinted = |symbol: &str, imposed_state: bool| {
            IconUi::sized(symbol, px(10.0), if imposed_state { imposed.color } else { white() }, false)
        };
        let mut row = hstack(5.0)
            .items_center()
            .max_w_full()
            .px(px(8.0))
            .py(px(5.0))
            .rounded_full()
            .bg(black().opacity(0.58))
            .when(deafened, |row| row.children(tinted(RiseSymbol::DEAFENED, imposed.deafened)))
            .children(tinted(if muted { "mic.slash.fill" } else { "mic.fill" }, muted && imposed.muted))
            .child(
                div().flex_shrink_1().min_w_0().child(
                    MainText::new(name).px(12.0).font_weight(FontWeight::SEMIBOLD).color(white()).number_of_lines(1),
                ),
            );
        if let Some((is_muted, text)) = volume {
            row = row.children(glyph(if is_muted { "speaker.slash.fill" } else { "speaker.wave.2.fill" }));
            if let (true, Some(text)) = (shows_volume_text, text) {
                row = row.child(
                    div().flex_none().child(
                        MainText::new(text)
                            .px(11.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(white())
                            .number_of_lines(1)
                            .monospaced_digits(true),
                    ),
                );
            }
        }
        div().size_full().flex().items_end().justify_start().child(row)
    }

    fn listener_volume_caption(is_self: bool, percent: f64) -> Option<(bool, Option<String>)> {
        if is_self {
            return None;
        }
        if RiseRTCListenerVolume::is_muted(percent) {
            return Some((true, Some(tr("rtc_volume_muted_for_me"))));
        }
        if percent == RiseRTCListenerVolume::DEFAULT_PERCENT {
            return None;
        }
        Some((false, Some(RiseRTCListenerVolume::percent_label(percent))))
    }

    fn participant_section_title(&self, text: String, count: usize, theme: &AppTheme) -> impl IntoElement + use<> {
        hstack(None)
            .w_full()
            .items_center()
            .child(MainText::new(text).px(15.0).font_weight(FontWeight::BOLD))
            .child(spacer())
            .child(MainText::new(count.to_string()).px(12.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.secondary))
    }

    fn shows_raise_hand_pill(&self, cx: &mut App) -> bool {
        self.room_mode(cx) == RiseRTCRoomMode::Stage
            && self.self_participant().is_some_and(|participant| participant.role == RiseRTCParticipantRole::Listener)
            && (self.can_request_to_speak(cx) || self.self_hand_raised(cx))
    }

    fn shows_stage_pill_row(&self, cx: &mut App) -> bool {
        self.shows_raise_hand_pill(cx) || self.shows_take_stage_button(cx)
    }

    /// measured not guessed, dock height moves with compact size class
    /// stage mode pill and label band
    /// padding shrinks the scroll frame so an underestimate leaves content unreachable
    fn dock_reserved_height(&self, window: &Window) -> f32 {
        self.measured_dock_height.frame_value(RiseClock::monotonic(), window)
            + RiseRTCScreenSafeArea::dock_bottom_padding()
            + RiseRTCScreenSafeArea::DOCK_CONTENT_GAP
    }

    fn pill_label(icon: &str, title: String, theme: &AppTheme) -> impl IntoElement + use<> {
        hstack(6.0)
            .items_center()
            .children(IconUi::sized(icon, px(13.0), theme.primary._100, false))
            .child(MainText::new(title).px(13.0).font_weight(FontWeight::SEMIBOLD).color(theme.primary._100))
    }

    fn control_dock(&mut self, theme: &AppTheme, width: f32, has_focused_stream: bool, cx: &mut Context<Self>) -> AnyElement {
        let compact = width < 430.0;
        let dock_width = width - 2.0 * (14.0 + Self::horizontal_safe_inset());
        let sharing = self.displayed_screen_share_enabled();
        let shows_share = sharing || self.can_publish_screen(cx);
        let slot_count: f32 = if shows_share { 6.0 } else { 5.0 };
        let preferred_spacing = if compact { 10.0 } else { 13.0 };
        let preferred_size = if compact { 50.0 } else { 54.0 };
        let fits = slot_count * (preferred_size + 8.0) + (slot_count - 1.0) * preferred_spacing <= dock_width;
        let dock_spacing = if fits { preferred_spacing } else { 6.0 };
        let circle_size: f32 = if fits {
            preferred_size
        } else {
            (((dock_width - (slot_count - 1.0) * dock_spacing) / slot_count - 8.0).floor()).max(36.0)
        };

        let pills = self.shows_stage_pill_row(cx).then(|| {
            let take_stage = self.shows_take_stage_button(cx).then(|| {
                plain("rtc-room-take-stage")
                    .on_press(self.handler(|this, _, cx| this.take_stage(cx)))
                    .px(px(13.0))
                    .py(px(8.0))
                    .child(Self::pill_label("person.wave.2.fill", tr("rtc_take_stage"), theme))
                    .liquid_chrome_capsule()
            });
            let raise_hand = self.shows_raise_hand_pill(cx).then(|| {
                let raised = self.self_hand_raised(cx);
                let blocked = !raised && !self.interactions.read(cx).can_raise_hand(cx);
                plain("rtc-room-raise-hand")
                    .on_press(self.handler(|this, _, cx| this.toggle_raised_hand(cx)))
                    .disabled(blocked)
                    .opacity(if blocked { 0.45 } else { 1.0 })
                    .px(px(13.0))
                    .py(px(8.0))
                    .child(Self::pill_label(
                        if raised { "hand.raised.slash.fill" } else { "hand.raised.fill" },
                        tr(if raised { "rtc_lower_hand" } else { "rtc_raise_hand" }),
                        theme,
                    ))
                    .liquid_chrome_capsule()
            });
            hstack(8.0).items_center().children(take_stage).children(raise_hand)
        });

        let speaker_enabled = self.audio.read(cx).speaker_enabled();
        let camera_enabled = self.displayed_camera_enabled();
        let hushed = RiseMicrophoneTestHush::is_active(cx);
        let deafened = self.displayed_deafened() || hushed;
        let microphone_enabled = self.displayed_microphone_enabled() && !hushed;
        let can_camera = self.can_publish_camera(cx);
        let can_microphone = self.can_publish_microphone(cx);
        let is_direct_call = self.is_direct_call(cx);
        let over_stage = has_focused_stream;

        let mut row = hstack(dock_spacing)
            .items_start()
            .justify_center()
            .w_full()
            .child(self.dock_control(
                "audio-route",
                if speaker_enabled { "speaker.wave.3.fill" } else { "speaker.wave.2.fill" },
                tr("rtc_audio_speaker"),
                None,
                speaker_enabled,
                circle_size,
                true,
                false,
                None,
                theme,
                over_stage,
                self.handler(|this, _, cx| {
                    this.audio_routes_visible = true;
                    cx.notify();
                }),
            ))
            .child(self.dock_control(
                "camera",
                if camera_enabled { "video.fill" } else { "video.slash.fill" },
                tr("rtc_camera"),
                None,
                camera_enabled,
                circle_size,
                can_camera || camera_enabled,
                false,
                None,
                theme,
                over_stage,
                self.handler(|this, _, cx| this.toggle_camera(cx)),
            ))
            .child(self.dock_control(
                "headphones",
                if deafened { RiseSymbol::DEAFENED } else { "headphones" },
                tr("rtc_audio_route"),
                Some(tr(if deafened { "rtc_undeafen" } else { "rtc_deafen" })),
                deafened,
                circle_size,
                !self.is_server_deafened(),
                false,
                None,
                theme,
                over_stage,
                self.handler(|this, _, cx| this.toggle_deafened(cx)),
            ))
            .child(self.dock_control(
                "microphone",
                if microphone_enabled { "mic.fill" } else { "mic.slash.fill" },
                tr("rtc_microphone"),
                None,
                microphone_enabled,
                circle_size,
                can_microphone || microphone_enabled,
                false,
                None,
                theme,
                over_stage,
                self.handler(|this, _, cx| this.toggle_microphone(cx)),
            ));
        if shows_share {
            row = row.child(self.dock_control(
                "screen-share",
                if sharing { "rectangle.on.rectangle.slash" } else { "rectangle.on.rectangle" },
                tr("rtc_screen_share"),
                Some(tr(if sharing { "rtc_stop_stream" } else { "rtc_start_stream" })),
                sharing,
                circle_size,
                true,
                false,
                None,
                theme,
                over_stage,
                self.handler(|this, _, cx| this.toggle_screen_share(cx)),
            ));
        }
        row = if has_focused_stream {
            row.child(self.dock_control(
                "stop-watching",
                "square.grid.2x2.fill",
                tr("rtc_stop_watching_short"),
                Some(tr("rtc_show_grid")),
                false,
                circle_size,
                true,
                false,
                None,
                theme,
                over_stage,
                self.handler(|this, _, cx| this.show_participant_grid(cx)),
            ))
        } else {
            row.child(self.dock_control(
                "leave",
                "phone.down.fill",
                tr(if is_direct_call { "rtc_end_call" } else { "rtc_leave" }),
                None,
                true,
                circle_size,
                true,
                true,
                Some(RiseGuideAnchor::RtcDockLeave),
                theme,
                over_stage,
                self.handler(|this, _, cx| this.close_room(cx)),
            ))
        };

        let this = self.this.clone();
        vstack(10.0)
            .relative()
            .w_full()
            .items_center()
            .children(pills)
            .child(row)
            .child(measure("rtc-room-dock-geometry", move |bounds, _, cx| {
                let height = f32::from(bounds.size.height).ceil();
                let _ = this.update(cx, |view, cx| {
                    let now = RiseClock::monotonic();
                    if view.measured_dock_height.value(now) != height {
                        let animation = view.chrome_animation(cx);
                        view.measured_dock_height.set_in(height, animation, now, cx);
                        cx.notify();
                    }
                });
            }))
            .into_any_element()
    }

    /// title is printed under the circle and accessibility_title is the hover hint
    /// they differ where the phrase doesnt fit an 11pt slot so the label may abbreviate
    #[allow(clippy::too_many_arguments)]
    fn dock_control(
        &self,
        id: &'static str,
        system_image: &'static str,
        title: String,
        accessibility_title: Option<String>,
        is_on: bool,
        size: f32,
        enabled: bool,
        destructive: bool,
        guide_anchor: Option<RiseGuideAnchor>,
        theme: &AppTheme,
        over_stage: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let silenced = destructive || (id == "microphone" && !is_on) || (id == "headphones" && is_on);
        let lit = !silenced && is_on && id != "microphone";
        let imposed = (id == "microphone" && !is_on && self.is_server_silenced())
            || (id == "headphones" && is_on && self.is_server_deafened())
            || (id == "camera" && !is_on && self.is_server_camera_blocked());

        let tint = if imposed {
            Some(theme.primary._100)
        } else if silenced {
            Some(theme.semantic.error_100)
        } else if lit {
            Some(if over_stage { white() } else { theme.text.primary })
        } else {
            None
        };
        let glyph_color = if lit {
            if over_stage { black() } else { theme.bg._000 }
        } else if silenced || imposed || over_stage {
            white()
        } else {
            theme.text.primary
        };
        let label_color = if over_stage { white() } else { theme.text.primary };
        let busy = self.action_ids.contains(id);
        let label = accessibility_title.unwrap_or_else(|| title.clone());

        let glyph = if busy {
            progress_view().tint(glyph_color).into_any_element()
        } else {
            div().children(IconUi::sized(system_image, px(size * 0.34), glyph_color, false)).into_any_element()
        };
        let button = LiquidGlassButton::new(SharedString::from(format!("rtc-dock-{id}")), LiquidGlassButtonShape::Circle)
            .size(size)
            .tint(tint)
            .haptic_on_press(false)
            .disabled(!enabled || busy)
            .flattened(over_stage)
            .accessibility_label(Some(label.clone()))
            .on_press(action)
            .child(div().size(px(size)).flex().items_center().justify_center().child(glyph));

        vstack(8.0)
            .items_center()
            .opacity(if enabled { 1.0 } else { 0.42 })
            .child(
                div()
                    .id(SharedString::from(format!("rtc-dock-{id}-hint")))
                    .tooltip_label(label)
                    .child(button.guide_anchor(guide_anchor)),
            )
            .child(
                MainText::new(title)
                    .px(11.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::MEDIUM)
                    .color(label_color)
                    .number_of_lines(1)
                    .minimum_scale_factor(0.6)
                    .w(px(size + 8.0)),
            )
            .into_any_element()
    }

    /// all three sheets use the app own shell, it has no fitted
    /// mode so the fraction is measured from the content
    /// a fixed fraction left a short list floating over dead space
    fn sheet_shell(
        id: &'static str,
        is_bottom_sheet: Binding<bool>,
        fraction: f32,
        theme: &AppTheme,
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> BottomSheetUi {
        BottomSheetUi::new(id, is_bottom_sheet, content, |_: &mut Window, _: &mut App| Empty)
            .snap_fractions(vec![fraction])
            .dismisses_on_backdrop_tap(true)
            .full_page_shows_drag_indicator(true)
            .full_page_hides_footer(true)
            .footer_shows_top_border(false)
            .content_scrollable(true)
            .sheet_background(Some(theme.bg._100))
            .footer_background(Some(theme.bg._100))
            .surface_shadow_enabled(false)
    }

    fn audio_route_sheet(&self, theme: &AppTheme, cx: &mut Context<Self>) -> BottomSheetUi {
        let entity = cx.entity();
        let binding = Binding::entity(
            entity.clone(),
            |this: &Self| this.audio_routes_visible,
            |this, value, _| {
                this.audio_routes_visible = value;
            },
        );
        Self::sheet_shell("rtc-room-audio-route-sheet", binding, self.audio_route_sheet_fraction, theme, move |_, cx| {
            entity.update(cx, |this, cx| {
                let theme = rise_ui::theme(cx).clone();
                let items = this.audio_output_items(&theme, cx);
                let weak = this.this.clone();
                Self::sheet_body(tr("rtc_audio_output"), GroupedBtns::new("rtc-audio-outputs", items).into_any_element())
                    .measured_sheet_fraction("rtc-audio-route-fraction", 0.24, move |fraction, _, cx| {
                        let _ = weak.update(cx, |view, cx| {
                            view.audio_route_sheet_fraction = fraction;
                            cx.notify();
                        });
                    })
                    .into_any_element()
            })
        })
    }

    fn audio_input_sheet(&self, theme: &AppTheme, cx: &mut Context<Self>) -> BottomSheetUi {
        let entity = cx.entity();
        let binding = Binding::entity(
            entity.clone(),
            |this: &Self| this.audio_inputs_visible,
            |this, value, _| {
                this.audio_inputs_visible = value;
            },
        );
        Self::sheet_shell("rtc-room-audio-input-sheet", binding, self.audio_input_sheet_fraction, theme, move |_, cx| {
            entity.update(cx, |this, cx| {
                let theme = rise_ui::theme(cx).clone();
                let audio = this.audio.read(cx);
                let inputs = if audio.available_inputs().is_empty() && !audio.has_system_input_choice() {
                    div()
                        .w_full()
                        .px(px(18.0))
                        .py(px(14.0))
                        .child(MainText::new(tr("rtc_audio_input_unavailable")).px(14.0).color(theme.text.secondary))
                        .into_any_element()
                } else {
                    GroupedBtns::new("rtc-audio-inputs", this.audio_input_items(&theme, cx)).into_any_element()
                };
                let weak = this.this.clone();
                let content = vstack(12.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .min_w_0()
                    .child(inputs)
                    .child(div().w_full().min_w_0().pt(px(4.0)).child(RiseRTCAudioSettingsContent));
                scroll_view("rtc-room-audio-input-scroll")
                    .shows_indicators(false)
                    .child(
                        Self::sheet_body(tr("rtc_audio_input"), content.into_any_element())
                            .measured_sheet_fraction("rtc-audio-input-fraction", 0.34, move |fraction, _, cx| {
                                let _ = weak.update(cx, |view, cx| {
                                    if view.audio_input_sheet_fraction != fraction {
                                        view.audio_input_sheet_fraction = fraction;
                                        cx.notify();
                                    }
                                });
                            }),
                    )
                    .into_any_element()
            })
        })
    }

    fn sheet_body(title: String, content: AnyElement) -> impl IntoElement {
        vstack(12.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .min_w_0()
            .pb(px(12.0 + Self::sheet_bottom_safe_inset()))
            .child(div().px(px(18.0)).pt(px(10.0)).child(MainText::new(title).px(20.0).font_weight(FontWeight::BOLD)))
            .child(content)
    }

    /// chrome floats over a full bleed canvas so nothing insets it for
    /// the title bar, it does that itself like the direct call screen
    fn top_safe_inset(&self) -> f32 {
        self.top_safe_inset
    }

    fn bottom_safe_inset() -> f32 {
        Self::sheet_bottom_safe_inset()
    }

    fn horizontal_safe_inset() -> f32 {
        RiseRTCScreenSafeArea::horizontal()
    }

    /// the sheet shell only insets its footer and these sheets have none
    fn sheet_bottom_safe_inset() -> f32 {
        RiseRTCScreenSafeArea::insets().bottom
    }

    fn audio_output_items(&self, theme: &AppTheme, cx: &App) -> Vec<GroupedBtn> {
        let audio = self.audio.read(cx);
        let selected_output_id = audio.selected_output_id();
        let outputs = audio.available_outputs();
        let auto_selected = !outputs.iter().any(|route| Some(route.id.as_str()) == selected_output_id);
        let accent = RiseRTCSettingsAccent::output();
        let mut items = vec![self.audio_route_item(
            "audio-auto".to_owned(),
            tr("rtc_audio_automatic"),
            "speaker.wave.2.fill",
            auto_selected,
            accent,
            theme,
            |this, cx| {
                this.audio_routes_visible = false;
                let interactions = this.interactions.clone();
                this.perform("audio-auto".to_owned(), move |cx| {
                    interactions.update(cx, |interactions, cx| interactions.select_audio_output(None, cx))
                }, cx);
            },
        )];
        for route in outputs {
            let route_id = route.id.clone();
            let action_id = format!("audio-route-{}", route.id);
            let perform_id = action_id.clone();
            items.push(self.audio_route_item(
                action_id,
                route.name.clone(),
                "speaker.wave.2.fill",
                selected_output_id == Some(route.id.as_str()),
                accent,
                theme,
                move |this, cx| {
                    this.audio_routes_visible = false;
                    let interactions = this.interactions.clone();
                    let route_id = route_id.clone();
                    this.perform(perform_id.clone(), move |cx| {
                        interactions.update(cx, |interactions, cx| interactions.select_audio_output(Some(route_id), cx))
                    }, cx);
                },
            ));
        }
        items
    }

    fn audio_input_items(&self, theme: &AppTheme, cx: &App) -> Vec<GroupedBtn> {
        let audio = self.audio.read(cx);
        let selected_input_id = audio.selected_input_id();
        let system_choice = audio.has_system_input_choice().then(|| {
            self.audio_route_item(
                "audio-input-automatic".to_owned(),
                tr("rtc_audio_automatic"),
                "mic.fill",
                selected_input_id.is_none(),
                RiseRTCSettingsAccent::microphone(),
                theme,
                |this, cx| {
                    let interactions = this.interactions.clone();
                    this.perform("audio-input-automatic".to_owned(), move |cx| {
                        interactions.update(cx, |interactions, cx| interactions.select_audio_input(None, cx))
                    }, cx);
                },
            )
        });
        let devices = audio
            .available_inputs()
            .iter()
            .map(|route| {
                let route_id = route.id.clone();
                let action_id = format!("audio-input-{}", route.id);
                let perform_id = action_id.clone();
                self.audio_route_item(
                    action_id,
                    route.display_name(),
                    Self::audio_route_icon(&route.port_type),
                    selected_input_id == Some(route.id.as_str()),
                    RiseRTCSettingsAccent::microphone(),
                    theme,
                    move |this, cx| {
                        let interactions = this.interactions.clone();
                        let route_id = route_id.clone();
                        this.perform(perform_id.clone(), move |cx| {
                            interactions.update(cx, |interactions, cx| interactions.select_audio_input(Some(route_id), cx))
                        }, cx);
                    },
                )
            });
        system_choice.into_iter().chain(devices).collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn audio_route_item(
        &self,
        id: String,
        title: String,
        system_image: &'static str,
        selected: bool,
        accent: Hsla,
        theme: &AppTheme,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> GroupedBtn {
        let item = GroupedBtn::new(id, title)
            .icon(system_image)
            .icon_background(accent)
            .icon_foreground(white())
            .text_line_limit(1)
            .accessibility_selected(selected)
            .show_chevron(false)
            .callback(self.handler(move |this, _, cx| {
                action(this, cx);
                cx.notify();
            }));
        if selected {
            item.trailing_slot(div().children(IconUi::sized("checkmark", px(14.0), theme.primary._100, false)))
        } else {
            item
        }
    }

    fn stream_settings_sheet(&self, theme: &AppTheme, cx: &mut Context<Self>) -> BottomSheetUi {
        let entity = cx.entity();
        let binding = Binding::entity(
            entity.clone(),
            |this: &Self| this.stream_settings_visible,
            |this, value, _| {
                this.stream_settings_visible = value;
            },
        );
        Self::sheet_shell("rtc-room-stream-sheet", binding, 0.32, theme, move |_, cx| {
            let content_entity = entity.clone();
            content_entity.update(cx, |this, cx| {
                let theme = rise_ui::theme(cx).clone();
                let includes_audio = Binding::entity(
                    entity.clone(),
                    |this: &Self| this.stream_includes_audio,
                    |this, value, _| {
                        this.stream_includes_audio = value;
                    },
                );
                let audio_item = GroupedBtn::new("stream-audio", tr("rtc_stream_audio"))
                    .icon("speaker.wave.2.fill")
                    .icon_background(RiseRTCSettingsAccent::stream_audio())
                    .icon_foreground(white())
                    .subtitle(tr("rtc_stream_audio_description"))
                    .show_chevron(false)
                    .trailing_slot(SwitchUi::new("rtc-stream-audio-switch", includes_audio))
                    .callback(this.handler(|this, _, cx| {
                        this.stream_includes_audio = !this.stream_includes_audio;
                        cx.notify();
                    }));
                vstack(12.0)
                    .w_full()
                    .pb(px(12.0 + Self::sheet_bottom_safe_inset()))
                    .child(div().px(px(18.0)).pt(px(10.0)).child(MainText::new(tr("rtc_stream_settings")).px(21.0).font_weight(FontWeight::BOLD)))
                    .child(GroupedBtns::new("rtc-stream-qualities", this.stream_quality_items(&theme, cx)))
                    .when(this.stream_audio_supported, |stack| {
                        stack.child(GroupedBtns::new("rtc-stream-audio", vec![audio_item]))
                    })
                    .child(div().w_full().pt(px(4.0)).child(this.start_stream_button(&theme)))
                    .into_any_element()
            })
        })
        .fits_content(true)
    }

    fn start_stream_button(&self, theme: &AppTheme) -> impl IntoElement + use<> {
        let start = self.handler(|this, _, cx| {
            this.stream_settings_visible = false;
            let includes_audio = this.stream_includes_audio;
            let quality = this.stream_quality;
            let interactions = this.interactions.clone();
            this.perform_media(
                |pending| pending.screen_share = Some(true),
                move |cx| interactions.update(cx, |interactions, cx| interactions.set_screen_share(true, includes_audio, quality, cx)),
                cx,
            );
            let current_user_id = this.current_user_id(cx);
            this.select_stream(&current_user_id, cx);
        });
        div().w_full().px(px(16.0)).child(
            plain("rtc-room-start-stream")
                .on_press(start)
                .w_full()
                .h(px(50.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(theme.primary._100)
                .child(MainText::new(tr("rtc_start_stream")).px(16.0).tac(TextAlignT::Center).font_weight(FontWeight::BOLD).color(theme.content_on(theme.primary._100))),
        )
    }

    fn stream_quality_items(&self, theme: &AppTheme, cx: &mut Context<Self>) -> Vec<GroupedBtn> {
        let high_available = self.interactions.update(cx, |interactions, cx| interactions.is_high_stream_quality_available(cx));
        [
            (RiseRTCScreenShareQuality::Balanced, tr("rtc_stream_balanced"), tr("rtc_stream_balanced_description")),
            (RiseRTCScreenShareQuality::High, tr("rtc_stream_high"), tr("rtc_stream_high_description")),
        ]
        .into_iter()
        .map(|(quality, title, detail)| {
            let selected = self.stream_quality == quality;
            let locked = quality == RiseRTCScreenShareQuality::High && !high_available;
            let item = GroupedBtn::new(format!("stream-quality-{title}"), title)
                .icon(if quality == RiseRTCScreenShareQuality::High { "sparkles" } else { "gauge" })
                .icon_background(RiseRTCSettingsAccent::quality())
                .icon_foreground(white())
                .subtitle(detail)
                .accessibility_selected(selected)
                .show_chevron(false)
                .callback(self.handler(move |this, _, cx| {
                    let accepted = this.interactions.update(cx, |interactions, cx| interactions.select_stream_quality(quality, cx));
                    if accepted {
                        this.stream_quality = quality;
                        cx.notify();
                    }
                }));
            match Self::stream_quality_trailing(selected, locked, theme) {
                Some(trailing) => item.trailing_slot(trailing),
                None => item,
            }
        })
        .collect()
    }

    fn stream_quality_trailing(selected: bool, locked: bool, theme: &AppTheme) -> Option<AnyElement> {
        if locked {
            return Some(PremiumDiamond::new().size(20.0).looping(false).into_any_element());
        }
        selected.then(|| div().children(IconUi::sized("checkmark", px(15.0), theme.primary._100, false)).into_any_element())
    }

    /// a running guide points at the header and dock, hiding them on a
    /// tile click left the bubble asking for Leave over an empty band
    fn toggle_chrome(&mut self, cx: &mut Context<Self>) {
        let guide_running = self.guides.read(cx).is_running(cx);
        tracing::warn!(
            target: "riseonly::X1839",
            chrome_visible = self.chrome_visible,
            guide_running,
            audio_inputs = self.audio_inputs_visible,
            audio_routes = self.audio_routes_visible,
            "room chrome toggle"
        );
        if self.chrome_visible && guide_running {
            return;
        }
        self.set_chrome_visible(!self.chrome_visible, cx);
    }

    fn minimize_or_close(&mut self, cx: &mut Context<Self>) {
        if self.presented_in_room_sheet {
            self.interactions.update(cx, |interactions, cx| interactions.minimize_room(cx));
        } else {
            self.close_room(cx);
        }
    }

    fn select_stream(&mut self, user_id: &str, cx: &mut Context<Self>) {
        self.selected_stream_user_id = Some(user_id.to_owned());
        self.set_chrome_visible(true, cx);
        self.interactions.update(cx, |interactions, cx| interactions.focus_stream(Some(user_id), cx));
    }

    fn reconcile_focused_stream(&mut self, cx: &mut Context<Self>) {
        if self.focused_stream_user_id(cx).is_some() || self.selected_stream_user_id.is_none() {
            return;
        }
        self.selected_stream_user_id = None;
        self.interactions.update(cx, |interactions, cx| interactions.focus_stream(None, cx));
    }

    fn show_participant_grid(&mut self, cx: &mut Context<Self>) {
        self.selected_stream_user_id = None;
        self.interactions.update(cx, |interactions, cx| interactions.focus_stream(None, cx));
        cx.notify();
    }

    fn audio_route_icon(port_type: &str) -> &'static str {
        let normalized = port_type.to_lowercase();
        if normalized.contains("bluetooth") {
            return "airpodspro";
        }
        if normalized.contains("headphone") || normalized.contains("headset") || normalized.contains("usb") {
            return "headphones";
        }
        "mic.fill"
    }

    fn take_stage(&mut self, cx: &mut Context<Self>) {
        let current_user_id = self.current_user_id(cx);
        let interactions = self.interactions.clone();
        self.perform("take-stage".to_owned(), move |cx| {
            interactions.update(cx, |interactions, cx| interactions.take_stage(&current_user_id, cx))
        }, cx);
    }

    fn toggle_raised_hand(&mut self, cx: &mut Context<Self>) {
        let raised = self.self_hand_raised(cx);
        if !raised && !self.interactions.read(cx).can_raise_hand(cx) {
            return;
        }
        let interactions = self.interactions.clone();
        self.perform("hand".to_owned(), move |cx| {
            interactions.update(cx, |interactions, cx| {
                if raised { interactions.cancel_request_to_speak(cx) } else { interactions.request_to_speak(None, cx) }
            })
        }, cx);
    }

    fn toggle_camera(&mut self, cx: &mut Context<Self>) {
        let enabled = !self.displayed_camera_enabled();
        let position = self.camera_position(cx);
        let interactions = self.interactions.clone();
        self.perform_media(
            |pending| pending.camera = Some(enabled),
            move |cx| interactions.update(cx, |interactions, cx| interactions.set_camera(enabled, position, cx)),
            cx,
        );
    }

    fn toggle_deafened(&mut self, cx: &mut Context<Self>) {
        let deafened = !self.displayed_deafened();
        let microphone_enabled = self.displayed_microphone_enabled();
        let can_publish_microphone = self.can_publish_microphone(cx);
        let restores_microphone = self.interactions.update(cx, |interactions, cx| {
            if deafened {
                interactions.remember_microphone_before_deafen(microphone_enabled, cx);
                false
            } else {
                interactions.take_microphone_before_deafen(cx) == Some(true) && can_publish_microphone
            }
        });
        let interactions = self.interactions.clone();
        let call_id = self.direct_call_id_for_microphone(cx);
        self.perform_media(
            |pending| {
                pending.deafened = Some(deafened);
                if deafened {
                    pending.microphone = Some(false);
                }
                if restores_microphone {
                    pending.microphone = Some(true);
                }
            },
            move |cx| {
                let set_deafened = interactions.update(cx, |interactions, cx| interactions.set_deafened(deafened, cx));
                cx.spawn(async move |cx| {
                    set_deafened.await?;
                    if restores_microphone {
                        cx.update(|cx| Self::apply_microphone(true, call_id, cx)).await?;
                    }
                    Ok(())
                })
            },
            cx,
        );
    }

    fn toggle_microphone(&mut self, cx: &mut Context<Self>) {
        let enabled = !self.displayed_microphone_enabled();
        let lifts_deafen = enabled && self.displayed_deafened();
        self.interactions.update(cx, |interactions, _| interactions.forget_microphone_before_deafen());
        let call_id = self.direct_call_id_for_microphone(cx);
        self.perform_media(
            |pending| {
                pending.microphone = Some(enabled);
                if lifts_deafen {
                    pending.deafened = Some(false);
                }
            },
            move |cx| Self::apply_microphone(enabled, call_id, cx),
            cx,
        );
    }

    fn toggle_screen_share(&mut self, cx: &mut Context<Self>) {
        if self.displayed_screen_share_enabled() {
            self.stop_screen_share(cx);
        } else {
            self.open_stream_settings(cx);
        }
    }

    fn open_stream_settings(&mut self, cx: &mut Context<Self>) {
        self.stream_quality = self.interactions.update(cx, |interactions, cx| interactions.remembered_stream_quality(cx));
        self.stream_settings_visible = true;
        cx.notify();
    }

    fn stop_screen_share(&mut self, cx: &mut Context<Self>) {
        let includes_audio = self.stream_includes_audio;
        let quality = self.stream_quality;
        let interactions = self.interactions.clone();
        self.perform_media(
            |pending| pending.screen_share = Some(false),
            move |cx| interactions.update(cx, |interactions, cx| interactions.set_screen_share(false, includes_audio, quality, cx)),
            cx,
        );
    }

    fn connect_if_needed(&mut self, force: bool, cx: &mut Context<Self>) -> Task<()> {
        self.load_effective_room_permissions(cx);
        if self.joining {
            return Task::ready(());
        }
        let Some(expected_room_id) = self.expected_room_id() else {
            self.error_visible = true;
            self.room_inputs_changed(cx);
            return Task::ready(());
        };
        let rtc = self.rtc.read(cx);
        let active_is_expected = rtc.active_room(cx).is_some_and(|room| room.config.room_id == expected_room_id);
        if !force && active_is_expected && rtc.connection_state(cx) != RiseRTCConnectionState::Failed {
            let refresh = self.interactions.update(cx, |interactions, cx| interactions.refresh_active_room(cx));
            cx.spawn(async move |_, _| {
                let _ = refresh.await;
            })
            .detach();
            return Task::ready(());
        }

        self.joining = true;
        self.error_visible = false;
        self.room_inputs_changed(cx);
        let room_mode = self.room_mode(cx);
        let interactions = self.interactions.clone();
        let join: Task<anyhow::Result<()>> = match &self.target {
            RiseRTCRoomTarget::Topic { .. } => {
                let role = if room_mode == RiseRTCRoomMode::Stage {
                    RiseRTCParticipantRole::Listener
                } else {
                    RiseRTCParticipantRole::Speaker
                };
                interactions.update(cx, |interactions, cx| interactions.join_room(&expected_room_id, None, Some(role), cx))
            }
            RiseRTCRoomTarget::Group { .. } => interactions.update(cx, |interactions, cx| {
                interactions.join_room(&expected_room_id, None, Some(RiseRTCParticipantRole::Speaker), cx)
            }),
            RiseRTCRoomTarget::DirectCall { call_id } => {
                let installation_id = RiseInstallationIdentity::current().ok();
                let rejoin = interactions.update(cx, |interactions, cx| interactions.rejoin_call(call_id, installation_id, cx));
                cx.spawn(async move |_, _| rejoin.await.map(|_| ()))
            }
        };
        cx.spawn(async move |this, cx| {
            let result = join.await;
            let _ = this.update(cx, |this, cx| {
                this.joining = false;
                match result {
                    Ok(()) => this.release_navigation_room_if_detached(cx),
                    Err(error) if error.is::<CancellationError>() => this.release_navigation_room_if_detached(cx),
                    Err(_) => {
                        this.error_visible = true;
                        this.mark_self_in_forum_channel(false, cx);
                        Self::send_error(tr("rtc_join_failed"), cx);
                    }
                }
                this.room_inputs_changed(cx);
            });
        })
    }

    fn load_effective_room_permissions(&self, cx: &mut Context<Self>) {
        match &self.target {
            RiseRTCRoomTarget::Topic { chat_id, topic_id } => {
                self.chat_interactions.update(cx, |interactions, cx| {
                    interactions.load_effective_permissions(chat_id, Some(topic_id.as_str()), false, cx);
                });
            }
            RiseRTCRoomTarget::Group { chat_id } => {
                self.chat_interactions.update(cx, |interactions, cx| {
                    interactions.load_effective_permissions(chat_id, None, false, cx);
                });
            }
            RiseRTCRoomTarget::DirectCall { .. } => {}
        }
    }

    /// deferred turn so navigation settles first
    fn release_navigation_room_if_detached(&self, cx: &mut App) {
        if self.leave_committed || !self.owns_navigation_room_session() {
            return;
        }
        let Some(expected_room_id) = self.expected_room_id() else {
            return;
        };
        let Some(navigation_store) = self.navigation_store.clone() else {
            return;
        };
        let target = self.target.clone();
        let interactions = self.interactions.clone();
        cx.defer(move |cx| {
            let route = RootRoute::RtcRoom { target: target.clone() };
            if navigation_store.read(cx).path.contains(&route) {
                return;
            }
            RiseRTCNavigationGate::mark_self_in_voice_channel(&target, false, cx);
            interactions.update(cx, |interactions, cx| interactions.leave_room_expected_room_id(&expected_room_id, cx)).detach();
        });
    }

    fn close_room(&mut self, cx: &mut Context<Self>) {
        if self.leave_committed {
            return;
        }
        self.leave_committed = true;
        let closing_call = self.call(cx);
        let closing_room_id = self.expected_room_id();
        self.mark_self_in_forum_channel(false, cx);

        if self.presented_in_room_sheet {
            self.interactions.update(cx, |interactions, cx| interactions.dismiss_room_presentation(cx));
        } else if !self.presented_as_call_overlay {
            self.pop_navigation(cx);
        }

        let interactions = self.interactions.clone();
        cx.spawn(async move |_, cx| {
            match closing_call.filter(|call| !call.state.is_terminal()) {
                Some(call) => {
                    let call_id = call.call_id;
                    let request = cx.update(|cx| RiseRTCCallSystemHooks::request_end_server_call_id(call_id.clone(), cx));
                    let handled_by_system = request.await.unwrap_or(false);
                    if !handled_by_system {
                        let end = cx.update(|cx| {
                            interactions.update(cx, |interactions, cx| {
                                interactions.end_call(&call_id, Some(CLIENT_HANGUP_REASON.to_owned()), None, cx)
                            })
                        });
                        let _ = end.await;
                    }
                }
                None => {
                    let leave = cx.update(|cx| {
                        interactions.update(cx, |interactions, cx| match &closing_room_id {
                            Some(room_id) => interactions.leave_room_expected_room_id(room_id, cx),
                            None => interactions.leave_room(cx),
                        })
                    });
                    leave.await;
                }
            }
        })
        .detach();
    }

    /// mirrors this account presence into the forum topic list
    /// only channel rooms have a row there, group rooms and direct calls have none
    fn mark_self_in_forum_channel(&self, joined: bool, cx: &mut App) {
        RiseRTCNavigationGate::mark_self_in_voice_channel(&self.target, joined, cx);
    }

    /// media toggles never show a spinner, the button flips
    /// on the click frame and the request settles behind it
    /// re entrancy is guarded by the override not a busy id so a fast double click still lands
    fn perform_media(
        &mut self,
        apply: impl FnOnce(&mut PendingMediaToggles),
        operation: impl FnOnce(&mut App) -> Task<anyhow::Result<()>>,
        cx: &mut Context<Self>,
    ) {
        let mut next = self.pending_media;
        apply(&mut next);
        self.pending_media = next;
        cx.notify();

        if self.is_connecting_to_displayed_room() {
            return;
        }

        let task = operation(cx);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let left = this
                .update(cx, |this, cx| {
                    this.clear_pending_media(next, cx);
                    this.room_was_left()
                })
                .unwrap_or(true);
            if let Err(error) = result
                && !error.is::<CancellationError>()
                && !left
            {
                cx.update(|cx| {
                    if let Some(text) = RiseRTCDomain::desktop_failure_text(&error) {
                        Self::send_error(text, cx);
                    } else if !RiseRTCDeviceRefusal::present_settings_alert(&error, cx) {
                        Self::send_error(tr("rtc_action_failed"), cx);
                    }
                });
            }
        })
        .detach();
    }

    fn direct_call_id_for_microphone(&self, cx: &mut App) -> Option<String> {
        if !self.is_direct_call(cx) {
            return None;
        }
        self.call(cx).map(|call| call.call_id)
    }

    pub fn apply_microphone(enabled: bool, direct_call_id: Option<String>, cx: &mut App) -> Task<anyhow::Result<()>> {
        let interactions = RTCInteractionsStore::shared(cx);
        let Some(call_id) = direct_call_id else {
            return interactions.update(cx, |interactions, cx| interactions.set_microphone(enabled, cx));
        };
        let request = RiseRTCCallSystemHooks::request_muted_server_call_id(!enabled, call_id, cx);
        cx.spawn(async move |cx| {
            if request.await.unwrap_or(false) {
                return Ok(());
            }
            cx.update(|cx| interactions.update(cx, |interactions, cx| interactions.set_microphone(enabled, cx))).await
        })
    }

    /// drops exactly the fields this command claimed so a rollback
    /// cant undo a newer toggle made while the request was in flight
    fn clear_pending_media(&mut self, claimed: PendingMediaToggles, cx: &mut Context<Self>) {
        let mut next = self.pending_media;
        if claimed.microphone.is_some() && next.microphone == claimed.microphone {
            next.microphone = None;
        }
        if claimed.deafened.is_some() && next.deafened == claimed.deafened {
            next.deafened = None;
        }
        if claimed.camera.is_some() && next.camera == claimed.camera {
            next.camera = None;
        }
        if claimed.screen_share.is_some() && next.screen_share == claimed.screen_share {
            next.screen_share = None;
        }
        if next != self.pending_media {
            self.pending_media = next;
            cx.notify();
        }
    }

    fn reconcile_pending_media(&mut self, participant: &RiseRTCParticipant) {
        let pending = &mut self.pending_media;
        if pending.microphone == Some(participant.microphone_enabled) {
            pending.microphone = None;
        }
        if pending.deafened == Some(participant.deafened) {
            pending.deafened = None;
        }
        if pending.camera == Some(participant.camera_enabled) {
            pending.camera = None;
        }
        if pending.screen_share == Some(participant.screen_share_enabled) {
            pending.screen_share = None;
        }
    }

    fn perform(
        &mut self,
        id: String,
        operation: impl FnOnce(&mut App) -> Task<anyhow::Result<()>>,
        cx: &mut Context<Self>,
    ) {
        if !self.action_ids.insert(id.clone()) {
            return;
        }
        cx.notify();
        let task = operation(cx);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let left = this
                .update(cx, |this, cx| {
                    this.action_ids.remove(&id);
                    cx.notify();
                    this.room_was_left()
                })
                .unwrap_or(true);
            let Err(error) = result else {
                return;
            };
            if left && !error.is::<CancellationError>() {
                return;
            }
            cx.update(|cx| Self::report_action_failure(id, error, cx));
        })
        .detach();
    }

    pub(crate) fn report_action_failure(id: String, error: anyhow::Error, cx: &mut App) {
        if error.is::<CancellationError>() {
            return;
        }
        let summary = format!("{error:?}");
        Sai::log(
            "RTC",
            move || format!("room action failed action={id} error_value={summary}"),
            SaiLogKind::Network,
            cx,
        );
        let title = RiseRTCDomain::desktop_failure_text(&error)
            .or_else(|| CanonErrors::failure(&error).and_then(|failure| CanonErrors::text_for(&failure)))
            .unwrap_or_else(|| tr("rtc_action_failed"));
        Self::send_error(title, cx);
    }

    fn send_error(title: String, cx: &mut App) {
        Notifier::send(
            title,
            NotifierSendOptions { r#type: NotifierType::Error, position: NotifierPosition::Top, ..NotifierSendOptions::default() },
            cx,
        );
    }

    fn participant_name(&self, participant: &RiseRTCParticipant, cx: &App) -> String {
        Self::participant_name_user_id(&participant.user_id, participant.profile.as_ref(), &self.current_user_id(cx))
    }

    pub(crate) fn participant_name_user_id(user_id: &str, profile: Option<&RiseRTCUserProfile>, current_user_id: &str) -> String {
        if user_id == current_user_id {
            return tr("rtc_you");
        }
        profile
            .and_then(|profile| non_empty(&profile.name))
            .or_else(|| profile.and_then(|profile| non_empty(&profile.tag)))
            .unwrap_or_else(|| tr("rtc_participant"))
    }

    /// avatar letter without a photo comes from the real
    /// name, You is only the caption under ones own tile
    fn participant_avatar_name(&self, participant: &RiseRTCParticipant, cx: &App) -> String {
        if participant.user_id != self.current_user_id(cx) {
            return self.participant_name(participant, cx);
        }
        let profile = participant.profile.as_ref();
        profile
            .and_then(|profile| non_empty(&profile.name))
            .or_else(|| self.auth.read(cx).profile().and_then(|own| non_empty(&own.name)))
            .or_else(|| profile.and_then(|profile| non_empty(&profile.tag)))
            .unwrap_or_else(|| tr("rtc_you"))
    }

    fn participant_avatar_source(&self, participant: &RiseRTCParticipant, cx: &App) -> Option<String> {
        let source = participant.profile.as_ref().and_then(|profile| non_empty(&profile.avatar_url));
        if participant.user_id != self.current_user_id(cx) {
            return source;
        }
        source.or_else(|| {
            let own = self.auth.read(cx).profile()?;
            own.avatar_url.as_deref().and_then(non_empty).or_else(|| non_empty(&own.more.logo))
        })
    }

    fn camera_track(&self, user_id: &str, cx: &App) -> Option<Rc<VideoTrack>> {
        self.media.read(cx).camera_track(user_id)
    }

    fn screen_track(&self, user_id: &str, cx: &App) -> Option<Rc<VideoTrack>> {
        self.media.read(cx).screen_share_track(user_id)
    }

    /// owned by the media session not this screen
    /// the room is rebuilt while the session keeps filming, and a local copy reversed the flip button
    fn camera_position(&self, cx: &App) -> RiseRTCCameraPosition {
        self.media.read(cx).local_camera_position()
    }
}

impl Render for RiseRTCRoomScreenView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.top_safe_inset = RiseRTCScreenSafeArea::top(cx);
        if self.is_direct_call(cx) {
            if self.standard_body_mounted {
                cx.defer_in(window, |this, _, cx| this.standard_body_disappeared(cx));
            }
            return self.direct_call_replacement();
        }
        if !self.synchronize_scheduled && (!self.standard_body_mounted || self.on_change_keys(cx) != self.applied_keys) {
            self.synchronize_scheduled = true;
            cx.defer_in(window, |this, _, cx| {
                this.synchronize_scheduled = false;
                this.synchronize(cx);
            });
        }
        self.standard_room_body(cx)
    }
}

trait BlockMouseWhenPresent: InteractiveElement + Sized {
    /// live chrome takes the pointer and the canvas behind
    /// must not read the same press as a click on empty space
    /// hidden chrome lets everything through so a click brings it back
    fn block_mouse_except_scroll_when_present(self, present: bool) -> Self {
        if present { self.block_mouse_except_scroll() } else { self }
    }
}

impl<E: InteractiveElement> BlockMouseWhenPresent for E {}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseRTCPendingRoomSeed {
    pub room_id: String,
    pub chat_id: String,
    pub topic_id: String,
    pub is_stage: bool,
    pub config: Option<VoiceChannelConfig>,
    pub state: Option<VoiceChannelState>,
}

/// one entry memo for the placeholder room, rebuilding would map the
/// roster and allocate a presentation on main each time the seed is read
/// not observable, it only mirrors its seed
#[derive(Default)]
pub struct RiseRTCPendingRoomCache {
    seed: Option<RiseRTCPendingRoomSeed>,
    cached: Option<Rc<RiseRTCRoomPresentation>>,
}

impl RiseRTCPendingRoomCache {
    pub fn room(
        &mut self,
        seed: Option<RiseRTCPendingRoomSeed>,
        current_user_id: &str,
    ) -> Option<Rc<RiseRTCRoomPresentation>> {
        let Some(seed) = seed else {
            self.seed = None;
            self.cached = None;
            return None;
        };
        if self.seed.as_ref() == Some(&seed)
            && let Some(cached) = &self.cached
        {
            return Some(cached.clone());
        }

        let fallback_role = if seed.is_stage { RiseRTCParticipantRole::Listener } else { RiseRTCParticipantRole::Speaker };
        let participants: Vec<RiseRTCParticipant> = seed
            .state
            .as_ref()
            .map(|state| {
                state.participants.iter().map(|participant| participant.as_pending_rtc_participant(fallback_role)).collect()
            })
            .unwrap_or_default();
        let voice_config = seed.config.as_ref();
        let config = RiseRTCRoomConfig {
            room_id: seed.room_id.clone(),
            livekit_room_name: String::new(),
            chat_id: seed.chat_id.clone(),
            channel_id: Some(seed.topic_id.clone()),
            mode: if seed.is_stage { RiseRTCRoomMode::Stage } else { RiseRTCRoomMode::Voice },
            max_participants: voice_config.map_or(0, |config| config.user_limit),
            max_speakers: voice_config.map_or(0, |config| config.speaker_limit),
            video_enabled: voice_config.is_none_or(|config| config.video_enabled),
            screen_share_enabled: voice_config.is_none_or(|config| config.screen_share_enabled),
            audio_bitrate_kbps: voice_config.map_or(32, |config| config.audio_bitrate_kbps),
            region: voice_config.map_or_else(String::new, |config| config.region.clone()),
            revision: 0,
            created_at_ms: 0,
            updated_at_ms: 0,
        };
        let participant_count = seed.state.as_ref().map_or(0, |state| state.participant_count).max(participants.len() as i64);
        let room = Rc::new(RiseRTCRoomPresentation {
            config,
            self_participant: participants.iter().find(|participant| participant.user_id == current_user_id).cloned(),
            speaker_count: participants.len() as i64,
            participants,
            raised_hands: Vec::new(),
            participant_count,
            has_more_participants: false,
            next_participant_cursor: None,
            generated_at_ms: seed.state.as_ref().map_or(0, |state| state.generated_at_ms),
            profiles_complete: true,
            profile_error: None,
            is_refreshing: true,
        });
        self.seed = Some(seed);
        self.cached = Some(room.clone());
        Some(room)
    }
}

pub trait VoiceChannelParticipantPendingRTC {
    fn as_pending_rtc_participant(&self, fallback_role: RiseRTCParticipantRole) -> RiseRTCParticipant;
}

impl VoiceChannelParticipantPendingRTC for VoiceChannelParticipant {
    fn as_pending_rtc_participant(&self, fallback_role: RiseRTCParticipantRole) -> RiseRTCParticipant {
        RiseRTCParticipant {
            user_id: self.user_id.clone(),
            role: if self.role == RiseRTCParticipantRole::Unspecified { fallback_role } else { self.role },
            connected: self.connected,
            microphone_enabled: self.microphone_enabled,
            camera_enabled: self.camera_enabled,
            screen_share_enabled: self.screen_share_enabled,
            server_muted: self.server_muted,
            server_deafened: self.server_deafened,
            server_camera_blocked: false,
            server_screen_share_blocked: false,
            deafened: self.deafened,
            hand_raised: self.is_hand_raised(RiseForumVoicePresenceSnapshot::now_ms_for_display()),
            joined_at_ms: self.joined_at_ms,
            updated_at_ms: self.joined_at_ms,
            lease_expires_at_ms: 0,
            order_sequence: self.order_sequence,
            profile: Some(RiseRTCUserProfile {
                id: self.user_id.clone(),
                tag: self.tag.clone(),
                name: self.name.clone(),
                avatar_url: self.avatar_url.clone(),
                is_premium: self.is_premium,
                is_official: self.is_official,
                is_online: true,
                last_seen_ms: 0,
            }),
        }
    }
}

/// mini player and sound of the watched stage, bottom trailing over the picture
/// its own view so a slider drag redraws only this corner
pub struct RiseRTCStageControls {
    user_id: String,
    is_stream: bool,
    volume_open: bool,
    dragged_percent: Option<f64>,
    room: WeakEntity<RiseRTCRoomScreenView>,
}

impl RiseRTCStageControls {
    const WIDTH: f32 = 260.0;
    const HEIGHT: f32 = 120.0;
    const BUTTON_SIDE: f32 = 36.0;

    fn stored_percent(&self, cx: &mut App) -> f64 {
        if self.is_stream {
            RiseRTCLiveKitMediaController::shared().stream_volume(&self.user_id) * 100.0
        } else {
            RTCServicesStore::shared(cx).read(cx).listener_volume_percent(&self.user_id, cx)
        }
    }

    fn apply(&self, percent: f64, commit: bool, cx: &mut App) {
        if self.is_stream {
            RiseRTCLiveKitMediaController::shared().set_stream_volume(&self.user_id, percent / 100.0);
            return;
        }
        let user_id = self.user_id.clone();
        RTCInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            if commit {
                interactions.commit_listener_volume(&user_id, percent, cx);
            } else {
                interactions.preview_listener_volume(&user_id, percent, cx);
            }
        });
    }

    fn volume_panel(&self, percent: f64, theme: &AppTheme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let changed = cx.entity().downgrade();
        let committed = changed.clone();
        let slider = SteppedSliderUi::new(
            SharedString::from(format!("rtc-stage-volume-{}", self.user_id)),
            percent,
            RiseRTCListenerVolume::MINIMUM_PERCENT..=RiseRTCListenerVolume::MAXIMUM_PERCENT,
            RiseRTCListenerVolume::STEP_PERCENT,
            move |value, _: &mut Window, cx: &mut App| {
                let _ = changed.update(cx, |controls, cx| {
                    controls.dragged_percent = Some(value);
                    controls.apply(value, false, cx);
                    cx.notify();
                });
            },
            move |value, _: &mut Window, cx: &mut App| {
                let _ = committed.update(cx, |controls, cx| {
                    controls.dragged_percent = None;
                    controls.apply(value, true, cx);
                    cx.notify();
                });
            },
        )
        .breakpoints(RiseRTCListenerVolume::BREAKPOINTS.to_vec());
        let muted = RiseRTCListenerVolume::is_muted(percent);
        vstack(6.0)
            .w_full()
            .px(px(12.0))
            .py(px(10.0))
            .rounded(px(14.0))
            .bg(theme.bg._100)
            .border_1()
            .border_color(theme.border._100)
            .occlude()
            .child(
                hstack(8.0)
                    .w_full()
                    .items_center()
                    .children(IconUi::sized(
                        if muted { "speaker.slash.fill" } else { "speaker.wave.2.fill" },
                        px(13.0),
                        theme.text.primary,
                        true,
                    ))
                    .child(MainText::new(tr("rtc_volume_title")).px(14.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
                    .child(spacer())
                    .child(
                        MainText::new(RiseRTCListenerVolume::percent_label(percent))
                            .px(13.0)
                            .secondary(true)
                            .number_of_lines(1)
                            .monospaced_digits(true),
                    ),
            )
            .child(div().w_full().h(px(28.0)).child(slider))
    }
}

impl Render for RiseRTCStageControls {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let percent = match self.dragged_percent {
            Some(percent) => percent,
            None => self.stored_percent(cx),
        };
        let muted = RiseRTCListenerVolume::is_muted(percent);
        let room = self.room.clone();
        let toggled = cx.entity().downgrade();
        let panel = self.volume_open.then(|| self.volume_panel(percent, &theme, cx));
        let icon = |name: &'static str| IconUi::sized(name, px(15.0), white(), true);
        vstack(8.0)
            .size_full()
            .justify_end()
            .items_end()
            .children(panel)
            .child(
                hstack(8.0)
                    .items_center()
                    .child(RiseRTCRoomScreenView::glass_circle(
                        "rtc-stage-mini-player",
                        tr("desktop_rtc_mini_player"),
                        Self::BUTTON_SIDE,
                        move |_, cx| {
                            let _ = room.update(cx, |view, cx| view.open_mini_player(cx));
                        },
                        icon("pip.enter"),
                    ))
                    .child(RiseRTCRoomScreenView::glass_circle(
                        "rtc-stage-volume",
                        tr("rtc_volume_title"),
                        Self::BUTTON_SIDE,
                        move |_, cx| {
                            let _ = toggled.update(cx, |controls, cx| {
                                controls.volume_open = !controls.volume_open;
                                cx.notify();
                            });
                        },
                        icon(if muted { "speaker.slash.fill" } else { "speaker.wave.2.fill" }),
                    )),
            )
    }
}
