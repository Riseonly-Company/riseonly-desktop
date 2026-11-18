use gpui::prelude::*;
use gpui::{App, Context, FontWeight, Hsla, SharedString, Window, div, px, white};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_theme::ui_color_system::UIColor;
use rise_i18n::tr;
use rise_ui::liquid_chrome::LiquidChrome;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::rise_symbol::RiseSymbol;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::{IconUi, MainText};

use crate::app::router::navigation_types::RiseRTCRoomTarget;
use crate::core::desktop_platform::tray_call_hooks::TrayCallState;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::rtc::engine::rise_rtc_call_system_hooks::RiseRTCCallSystemHooks;
use crate::modules::rtc::engine::rise_rtc_domain::RiseRTCDomain;
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCCallStateExt, RiseRTCConnectionState, RiseRTCParticipantRole, RiseRTCRoomMode,
};
use crate::modules::rtc::media::rise_rtc_audio_settings::RiseMicrophoneTestHush;
use crate::modules::rtc::media::rise_rtc_media_presentation::RiseRTCMediaPresentation;
use crate::modules::rtc::pages::rise_rtc_room_presentation_host::RiseRTCRoomPresentationHost;
use crate::modules::rtc::pages::rise_rtc_room_screen::RiseRTCRoomScreenView;
use crate::modules::rtc::shared::rise_rtc_device_refusal::RiseRTCDeviceRefusal;
use crate::modules::rtc::shared::rise_rtc_navigation_gate::RiseRTCNavigationGate;
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;
use crate::modules::rtc::system::rise_rtc_system_call_presentation::RiseRTCSystemCallPresentationStore;
use crate::modules::rtc::system::rise_rtc_system_models::{RiseRTCSystemCallDirection, RiseRTCSystemCallPhase};

const CLIENT_HANGUP_REASON: &str = "client_hangup";
const CARD_PADDING: f32 = 6.0;
const CARD_RADIUS: f32 = 12.0;
const BUTTON_SIDE: f32 = 20.0;
const ROW_GAP: f32 = 5.0;
const LIVE_DOT_SIDE: f32 = 7.0;
const FOOT_GAP: f32 = 4.0;

/// header line, room name once connected and the call window status before
#[derive(Clone, Copy, PartialEq)]
enum RiseRTCRailCallStatus {
    Connected,
    Connecting,
    Reconnecting,
    WaitingForNetwork,
    Failed,
}

/// what the card shows, compared before every notify since the room
/// store ticks with roster and presence changes the card doesnt draw
#[derive(Clone, PartialEq)]
struct RiseRTCRailCallState {
    target: RiseRTCRoomTarget,
    room_id: String,
    title: String,
    status: RiseRTCRailCallStatus,
    microphone_enabled: bool,
    deafened: bool,
    camera_enabled: bool,
    screen_share_enabled: bool,
    can_publish: bool,
    video_enabled: bool,
    server_silenced: bool,
    server_deafened: bool,
    camera_blocked: bool,
    screen_share_blocked: bool,
}

pub struct RiseRTCRailCallCard {
    interactions: gpui::Entity<RTCInteractionsStore>,
    state: Option<RiseRTCRailCallState>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl RiseRTCRailCallCard {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let rtc = RTCServicesStore::shared(cx);
        let interactions = RTCInteractionsStore::shared(cx);
        let system_calls = RiseRTCSystemCallPresentationStore::shared(cx);
        let socket = SaiWebSocketManager::shared(cx);
        let subscriptions = vec![
            cx.observe(&rtc, |this, _, cx| this.refresh(cx)),
            cx.observe(&system_calls, |this, _, cx| this.refresh(cx)),
            cx.observe(&socket, |this, _, cx| this.refresh(cx)),
            cx.observe_global::<RiseMicrophoneTestHush>(|_, cx| cx.notify()),
        ];
        let state = Self::resolve(cx);
        Self { interactions, state, _subscriptions: subscriptions }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let state = Self::resolve(cx);
        if state != self.state {
            self.state = state;
            cx.notify();
        }
    }

    fn resolve(cx: &mut App) -> Option<RiseRTCRailCallState> {
        let rtc = RTCServicesStore::shared(cx);
        let system_calls = RiseRTCSystemCallPresentationStore::shared(cx);
        let socket_connected = SaiWebSocketManager::shared(cx).read(cx).is_connected();
        let rtc = rtc.read(cx);
        let target = match rtc.room_presentation_target().cloned() {
            Some(target) => target,
            None => Self::active_direct_call_target(&system_calls, cx)?,
        };
        let direct = matches!(target, RiseRTCRoomTarget::DirectCall { .. });
        let room = rtc
            .active_room(cx)
            .filter(|room| !direct || room.config.mode == RiseRTCRoomMode::DirectCall);
        if room.is_none() && !direct {
            return None;
        }
        let room_id = room.map(|room| room.config.room_id.clone()).unwrap_or_default();
        let video_enabled = direct || room.is_some_and(|room| room.config.video_enabled);
        let participant = room.and_then(|room| room.self_participant.clone());
        let has_video = direct && Self::direct_call_has_video(&system_calls, &target, cx);
        let silenced = participant.as_ref().is_some_and(|participant| participant.is_server_silenced());
        let server_deafened = participant.as_ref().is_some_and(|participant| participant.server_deafened);
        let can_publish = !silenced
            && (direct
                || participant.as_ref().is_some_and(|participant| {
                    matches!(participant.role, RiseRTCParticipantRole::Speaker | RiseRTCParticipantRole::Moderator)
                }));
        let microphone_enabled = !silenced
            && participant.as_ref().map_or(direct, |participant| participant.microphone_enabled);
        let deafened = server_deafened || participant.as_ref().is_some_and(|participant| participant.deafened);
        let camera_enabled = participant.as_ref().map_or(has_video, |participant| participant.camera_enabled);
        let screen_share_enabled = participant.as_ref().is_some_and(|participant| participant.screen_share_enabled);
        let status = Self::status(room.is_some(), rtc.connection_state(cx), socket_connected);
        let title = Self::title(&target, cx);
        Some(RiseRTCRailCallState {
            target,
            room_id,
            title,
            status,
            microphone_enabled,
            deafened,
            camera_enabled,
            screen_share_enabled,
            can_publish,
            video_enabled,
            server_silenced: silenced,
            server_deafened,
            camera_blocked: participant.as_ref().is_some_and(|participant| participant.server_camera_blocked),
            screen_share_blocked: participant.as_ref().is_some_and(|participant| participant.server_screen_share_blocked),
        })
    }

    /// subtitle of the room screen, no socket while connecting means no network
    fn status(joined: bool, connection: RiseRTCConnectionState, socket_connected: bool) -> RiseRTCRailCallStatus {
        if connection == RiseRTCConnectionState::Failed {
            return RiseRTCRailCallStatus::Failed;
        }
        let connecting = !joined || matches!(connection, RiseRTCConnectionState::Idle | RiseRTCConnectionState::Connecting);
        let reconnecting = connection == RiseRTCConnectionState::Reconnecting;
        if (connecting || reconnecting) && !socket_connected {
            RiseRTCRailCallStatus::WaitingForNetwork
        } else if connecting {
            RiseRTCRailCallStatus::Connecting
        } else if reconnecting {
            RiseRTCRailCallStatus::Reconnecting
        } else {
            RiseRTCRailCallStatus::Connected
        }
    }

    fn header_line(state: &RiseRTCRailCallState, theme: &rise_theme::AppTheme) -> (String, Hsla) {
        let yellow = UIColor::system_yellow().resolved_color(theme.appearance);
        match state.status {
            RiseRTCRailCallStatus::Connected => (state.title.clone(), theme.semantic.success_100),
            RiseRTCRailCallStatus::Connecting => (tr("rtc_connecting"), yellow),
            RiseRTCRailCallStatus::Reconnecting => (tr("rtc_reconnecting"), yellow),
            RiseRTCRailCallStatus::WaitingForNetwork => (tr("rtc_waiting_for_network"), theme.semantic.error_100),
            RiseRTCRailCallStatus::Failed => (state.title.clone(), theme.semantic.error_100),
        }
    }

    fn active_direct_call_target(
        system_calls: &gpui::Entity<RiseRTCSystemCallPresentationStore>,
        cx: &App,
    ) -> Option<RiseRTCRoomTarget> {
        let system_calls = system_calls.read(cx);
        system_calls
            .calls()
            .iter()
            .filter(|call| !call.is_terminal() && !system_calls.dismissed_local_uuids().contains(&call.local_uuid))
            .filter(|call| {
                call.direction == RiseRTCSystemCallDirection::Outgoing || call.phase != RiseRTCSystemCallPhase::IncomingRinging
            })
            .find_map(|call| call.server_call_id.clone().filter(|call_id| !call_id.is_empty()))
            .map(|call_id| RiseRTCRoomTarget::DirectCall { call_id })
    }

    fn direct_call_has_video(
        system_calls: &gpui::Entity<RiseRTCSystemCallPresentationStore>,
        target: &RiseRTCRoomTarget,
        cx: &App,
    ) -> bool {
        let RiseRTCRoomTarget::DirectCall { call_id } = target else {
            return false;
        };
        system_calls
            .read(cx)
            .calls()
            .iter()
            .any(|call| call.server_call_id.as_deref() == Some(call_id.as_str()) && call.has_video)
    }

    /// names the room screen gives its header
    pub(crate) fn title(target: &RiseRTCRoomTarget, cx: &mut App) -> String {
        match target {
            RiseRTCRoomTarget::Topic { chat_id, topic_id } => ChatServicesStore::shared(cx)
                .update(cx, |store, cx| store.topic_from_any_cache(chat_id, topic_id, cx))
                .map(|topic| trimming_whitespaces_and_newlines(&topic.title).to_owned())
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| tr("rtc_voice_chat")),
            RiseRTCRoomTarget::Group { chat_id } => ChatServicesStore::shared(cx)
                .read(cx)
                .chat_info(chat_id, cx)
                .map(|chat| trimming_whitespaces_and_newlines(&chat.title).to_owned())
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| tr("rtc_group_voice")),
            RiseRTCRoomTarget::DirectCall { call_id } => RiseRTCSystemCallPresentationStore::shared(cx)
                .read(cx)
                .calls()
                .iter()
                .find(|call| call.server_call_id.as_deref() == Some(call_id.as_str()))
                .map(|call| trimming_whitespaces_and_newlines(&call.display_name).to_owned())
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| tr("rtc_direct_call")),
        }
    }

    fn direct_call_id(target: &RiseRTCRoomTarget) -> Option<String> {
        match target {
            RiseRTCRoomTarget::DirectCall { call_id } => Some(call_id.clone()),
            _ => None,
        }
    }

    /// call id for the microphone, only a call the rtc list holds goes through the system call hooks
    fn microphone_call_id(target: &RiseRTCRoomTarget, cx: &mut App) -> Option<String> {
        let rtc = RTCServicesStore::shared(cx);
        Self::direct_call_id(target).filter(|call_id| rtc.read(cx).call(call_id, cx).is_some())
    }

    pub(crate) fn tray_state(cx: &mut App) -> Option<TrayCallState> {
        let state = Self::resolve(cx)?;
        Some(TrayCallState {
            microphone_enabled: state.microphone_enabled,
            can_use_microphone: state.can_publish,
            deafened: state.deafened,
        })
    }

    pub(crate) fn tray_toggle_microphone(cx: &mut App) {
        if let Some(state) = Self::resolve(cx).filter(|state| state.can_publish) {
            Self::flip_microphone(&state, cx);
        }
    }

    pub(crate) fn tray_toggle_deafened(cx: &mut App) {
        if let Some(state) = Self::resolve(cx) {
            Self::flip_deafened(&state, cx);
        }
    }

    pub(crate) fn tray_leave(cx: &mut App) {
        if let Some(state) = Self::resolve(cx) {
            Self::leave_call(state, cx);
        }
    }

    fn flip_microphone(state: &RiseRTCRailCallState, cx: &mut App) {
        RTCInteractionsStore::shared(cx).update(cx, |interactions, _| interactions.forget_microphone_before_deafen());
        let call_id = Self::microphone_call_id(&state.target, cx);
        let task = RiseRTCRoomScreenView::apply_microphone(!state.microphone_enabled, call_id, cx);
        Self::settle(task, cx);
    }

    fn flip_deafened(state: &RiseRTCRailCallState, cx: &mut App) {
        let interactions = RTCInteractionsStore::shared(cx);
        let deafened = !state.deafened;
        let restores_microphone = interactions.update(cx, |interactions, cx| {
            if deafened {
                interactions.remember_microphone_before_deafen(state.microphone_enabled, cx);
                false
            } else {
                interactions.take_microphone_before_deafen(cx) == Some(true) && state.can_publish
            }
        });
        let call_id = Self::microphone_call_id(&state.target, cx);
        let set_deafened = interactions.update(cx, |interactions, cx| interactions.set_deafened(deafened, cx));
        let task = cx.spawn(async move |cx| {
            set_deafened.await?;
            if restores_microphone {
                cx.update(|cx| RiseRTCRoomScreenView::apply_microphone(true, call_id, cx)).await?;
            }
            Ok(())
        });
        Self::settle(task, cx);
    }

    fn open_call(target: RiseRTCRoomTarget, cx: &mut App) {
        if !RiseRTCRoomPresentationHost::bring_call_forward(cx)
            && !RTCServicesStore::shared(cx).read(cx).is_room_presentation_visible()
        {
            RTCInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.present_room(target, cx));
        }
        cx.defer(RiseRTCRoomPresentationHost::activate_call_window);
    }

    /// error path of the room dock toggles
    fn settle(task: gpui::Task<anyhow::Result<()>>, cx: &mut App) {
        cx.spawn(async move |cx| {
            if let Err(error) = task.await
                && !error.is::<rise_engine::CancellationError>()
            {
                cx.update(|cx| {
                    if !RiseRTCDeviceRefusal::present_settings_alert(&error, cx) {
                        Notifier::send(
                            RiseRTCDomain::desktop_failure_text(&error).unwrap_or_else(|| tr("rtc_action_failed")),
                            NotifierSendOptions {
                                r#type: NotifierType::Error,
                                position: NotifierPosition::Top,
                                ..NotifierSendOptions::default()
                            },
                            cx,
                        );
                    }
                });
            }
        })
        .detach();
    }

    fn toggle_microphone(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.state.clone() {
            Self::flip_microphone(&state, cx);
        }
    }

    fn toggle_deafened(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.state.clone() {
            Self::flip_deafened(&state, cx);
        }
    }

    fn toggle_camera(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let position = RiseRTCMediaPresentation::shared(cx).read(cx).local_camera_position();
        let task = self
            .interactions
            .update(cx, |interactions, cx| interactions.set_camera(!state.camera_enabled, position, cx));
        Self::settle(task, cx);
    }

    fn toggle_screen_share(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let task = self.interactions.update(cx, |interactions, cx| {
            let quality = interactions.remembered_stream_quality(cx);
            interactions.set_screen_share(!state.screen_share_enabled, true, quality, cx)
        });
        Self::settle(task, cx);
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.state.clone() {
            Self::leave_call(state, cx);
        }
    }

    /// close_room of the room screen, optimistic so the card and window go at once
    fn leave_call(state: RiseRTCRailCallState, cx: &mut App) {
        let interactions = RTCInteractionsStore::shared(cx);
        RiseRTCNavigationGate::mark_self_in_voice_channel(&state.target, false, cx);
        interactions.update(cx, |interactions, cx| interactions.dismiss_room_presentation(cx));
        let rtc = RTCServicesStore::shared(cx);
        let open_call = Self::direct_call_id(&state.target)
            .filter(|call_id| rtc.read(cx).call(call_id, cx).is_none_or(|call| !call.state.is_terminal()));
        let room_id = state.room_id;
        cx.spawn(async move |cx| match open_call {
            Some(call_id) => {
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
                    interactions.update(cx, |interactions, cx| interactions.leave_room_expected_room_id(&room_id, cx))
                });
                leave.await;
            }
        })
        .detach();
    }

    /// dock inks, primary for a moderator lock and red for silenced or leaving
    /// lit for a camera or share that is on
    #[allow(clippy::too_many_arguments)]
    fn control(
        &self,
        id: &'static str,
        system_image: &'static str,
        label: String,
        silenced: bool,
        imposed: bool,
        lit: bool,
        enabled: bool,
        theme: &rise_theme::AppTheme,
        action: fn(&mut Self, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tint: Option<Hsla> = if imposed {
            Some(theme.primary._100)
        } else if silenced {
            Some(theme.semantic.error_100)
        } else if lit {
            Some(theme.text.primary)
        } else {
            None
        };
        let glyph_color = if silenced || imposed {
            white()
        } else if lit {
            theme.bg._000
        } else {
            theme.text.primary
        };
        let this = cx.weak_entity();
        let button = LiquidGlassButton::new(SharedString::from(format!("rail-call-{id}")), LiquidGlassButtonShape::Circle)
            .size(BUTTON_SIDE)
            .tint(tint)
            .haptic_on_press(false)
            .disabled(!enabled)
            .on_press(move |_: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |card, cx| action(card, cx));
            })
            .child(
                div()
                    .size(px(BUTTON_SIDE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized(system_image, px(BUTTON_SIDE * 0.5), glyph_color, false)),
            );
        div()
            .id(SharedString::from(format!("rail-call-{id}-slot")))
            .aria_label(label.clone())
            .tooltip_label(label)
            .child(button)
    }
}

impl Render for RiseRTCRailCallCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(state) = self.state.clone() else {
            return div().into_any_element();
        };
        let theme = rise_ui::theme(cx as &App).clone();
        let open_target = state.target.clone();
        let (header_text, dot_color) = Self::header_line(&state, &theme);

        let header = plain("rail-call-open")
            .on_press(move |_: &mut Window, cx: &mut App| Self::open_call(open_target.clone(), cx))
            .w_full()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .w_full()
                    .min_w_0()
                    .child(div().flex_none().size(px(LIVE_DOT_SIDE)).rounded_full().bg(dot_color))
                    .child(
                        div().flex_1().min_w_0().child(
                            MainText::new(header_text)
                                .px(11.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .number_of_lines(1),
                        ),
                    ),
            );

        let hushed = RiseMicrophoneTestHush::is_active(cx);
        let microphone_enabled = state.microphone_enabled && !hushed;
        let deafened = state.deafened || hushed;
        let microphone = self.control(
            "microphone",
            if microphone_enabled { "mic.fill" } else { "mic.slash.fill" },
            tr("rtc_microphone"),
            !microphone_enabled,
            !microphone_enabled && state.server_silenced,
            false,
            state.can_publish,
            &theme,
            Self::toggle_microphone,
            cx,
        );
        let sound = self.control(
            "headphones",
            if deafened { RiseSymbol::DEAFENED } else { "headphones" },
            tr(if deafened { "rtc_undeafen" } else { "rtc_deafen" }),
            deafened,
            deafened && state.server_deafened,
            false,
            true,
            &theme,
            Self::toggle_deafened,
            cx,
        );
        let screen_share = self.control(
            "screen-share",
            if state.screen_share_enabled { "rectangle.on.rectangle.slash" } else { "rectangle.on.rectangle" },
            tr("rtc_screen_share"),
            false,
            false,
            state.screen_share_enabled,
            state.can_publish && (state.screen_share_enabled || !state.screen_share_blocked),
            &theme,
            Self::toggle_screen_share,
            cx,
        );
        let camera = self.control(
            "camera",
            if state.camera_enabled { "video.fill" } else { "video.slash.fill" },
            tr("rtc_camera"),
            false,
            !state.camera_enabled && state.camera_blocked,
            state.camera_enabled,
            state.can_publish && state.video_enabled && (state.camera_enabled || !state.camera_blocked),
            &theme,
            Self::toggle_camera,
            cx,
        );
        let leave = self.control(
            "leave",
            "phone.down.fill",
            tr("rtc_leave"),
            true,
            false,
            false,
            true,
            &theme,
            Self::leave,
            cx,
        );

        div()
            .w_full()
            .pb(px(FOOT_GAP))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .w_full()
                    .p(px(CARD_PADDING))
                    .rounded(px(CARD_RADIUS))
                    .bg(LiquidChrome::surface_fill(&theme, 1))
                    .child(header)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .w_full()
                            .child(microphone)
                            .child(sound)
                            .child(screen_share),
                    )
                    .child(div().flex().flex_row().justify_center().gap(px(ROW_GAP)).w_full().child(camera).child(leave)),
            )
            .into_any_element()
    }
}
