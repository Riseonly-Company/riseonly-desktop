use std::collections::{HashSet, VecDeque};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, App, AppContext, AsyncApp, Context, Entity, EventEmitter, FocusHandle, Global, InteractiveElement,
    IntoElement, ParentElement, Render, StyleRefinement, Styled, Subscription, Task, TitlebarOptions, WeakEntity, Window,
    WindowAppearance, WindowBounds, WindowHandle, WindowOptions, black, deferred, div, px, size,
};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::window_chrome::set_system_controls_appearance;
use rise_ui::swiftui::environment::{env, env_scope};
use rise_ui::window_cover::WindowCoverFrame;
use rise_ui::guide_overlay_ui::guide_overlay_window::GuideOverlayWindow;
use rise_theme::AppTheme;
use rise_widgets::window_controls_band;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::app::router::navigation_types::RiseRTCRoomTarget;
use crate::app::shell::handover_hooks::handover_hooks;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_host::HoldContextMenuHost;
use crate::core::widgets::notifier::notifier_host::NotifierHost;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCCallStateExt, RiseRTCDirectCall, RiseRTCEvictionReason, RiseRTCRoomEviction, RiseRTCRoomMode,
};
use crate::modules::rtc::media::rise_rtc_end_tone_player::RiseRTCEndTonePlayer;
use crate::modules::rtc::pages::rise_rtc_call_window_chrome_hooks::rise_rtc_call_window_chrome_hooks;
use crate::modules::rtc::pages::rise_rtc_direct_call_screen::{RiseRTCDirectCallScreen, RiseRTCTerminalHold};
use crate::modules::rtc::pages::rise_rtc_room_screen::RiseRTCRoomScreen;
use crate::modules::rtc::pages::shell_window_actions_hooks::{CloseWindow, KEY_CONTEXT};
use crate::modules::rtc::shared::rise_rtc_screen_safe_area::RiseRTCScreenTopInsetKey;
use crate::modules::rtc::shared::rise_rtc_navigation_gate::{RiseRTCNavigationGate, RiseRTCRoomTargetSafeLogKind};
use crate::modules::rtc::shared::rise_rtc_room_locator::RiseRTCRoomTargetRoomID;
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;
use crate::modules::rtc::system::rise_rtc_call_window_hooks::rise_rtc_call_window_hooks;
use crate::modules::rtc::system::rise_rtc_system_call_presentation::{
    RiseRTCSystemCallPresentation, RiseRTCSystemCallPresentationStore,
};
use crate::modules::rtc::system::rise_rtc_system_models::{
    RiseRTCSystemCallDirection, RiseRTCSystemCallPhase, RiseRTCSystemCallTerminalOutcome,
};

const MAX_REMEMBERED_SYSTEM_CALLS: usize = 8;
const TERMINAL_SYSTEM_CALL_MAX_AGE_SECONDS: f64 = 4.0;
const END_TONE_HANDOFF: Duration = Duration::from_millis(120);
const ENDING_SYSTEM_CALL_DEADLINE: Duration = Duration::from_millis(760);

const DIRECT_CALL_WINDOW_SIZE: (f32, f32) = (440.0, 780.0);
const ROOM_WINDOW_SIZE: (f32, f32) = (960.0, 720.0);
const CALL_WINDOW_MIN_SIZE: (f32, f32) = (360.0, 480.0);

pub struct RiseRTCRoomPresentationHost {
    rtc: Entity<RTCServicesStore>,
    system_calls: Entity<RiseRTCSystemCallPresentationStore>,
    interactions: Entity<RTCInteractionsStore>,
    navigation_store: Option<Entity<NavigationStore>>,
    call_window: Option<WindowHandle<RiseRTCCallWindow>>,
    call_window_opening: bool,
    call_window_is_active: bool,
    minimized_direct_call_uuid: Option<Uuid>,
    dismissed_terminal_call_uuid: Option<Uuid>,
    presented_system_call_uuids: HashSet<Uuid>,
    presented_system_call_order: VecDeque<Uuid>,
    is_mounted_in_window: bool,
    did_synchronize: bool,
    last_target: Option<RiseRTCRoomTarget>,
    seated_voice_room: Option<(RiseRTCRoomTarget, String)>,
    last_is_visible: bool,
    last_eviction: Option<RiseRTCRoomEviction>,
    last_account_id: Option<String>,
    last_displayed_call_uuid: Option<Uuid>,
    last_active_server_call_id: Option<String>,
    last_displayed_server_call_id: Option<String>,
    terminal_task_identity: Option<Uuid>,
    ending_task_identity: Option<Uuid>,
    terminal_task: Option<Task<()>>,
    ending_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// the mounted host, so the rail call card can reopen a call the host minimized
struct RiseRTCRoomPresentationHostHandle(WeakEntity<RiseRTCRoomPresentationHost>);

impl Global for RiseRTCRoomPresentationHostHandle {}

impl RiseRTCRoomPresentationHost {
    pub(crate) fn bring_call_forward(cx: &mut App) -> bool {
        let Some(host) = cx.try_global::<RiseRTCRoomPresentationHostHandle>().and_then(|handle| handle.0.upgrade()) else {
            return false;
        };
        host.update(cx, |host, cx| host.bring_forward(cx));
        true
    }

    fn bring_forward(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.call_window
            && self.is_call_window_behind()
        {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            return;
        }
        if let Some(local_uuid) = self.displayed_system_call(cx).map(|call| call.local_uuid) {
            if self.minimized_direct_call_uuid == Some(local_uuid) {
                self.minimized_direct_call_uuid = None;
                self.set_needs_synchronize(cx);
            }
            return;
        }
        let target = self.rtc.read(cx).room_presentation_target().cloned();
        if let Some(target) = target
            && !self.rtc.read(cx).is_room_presentation_visible()
        {
            self.interactions.update(cx, |interactions, cx| interactions.present_room(target, cx));
        }
    }

    /// brings an open call window to the front
    /// opening the room it already shows changes no state
    /// so without this the window stays behind the main one
    pub fn activate_call_window(cx: &mut App) {
        for window in cx.windows() {
            if let Some(handle) = window.downcast::<RiseRTCCallWindow>() {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            }
        }
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        let rtc = RTCServicesStore::shared(cx);
        let system_calls = RiseRTCSystemCallPresentationStore::shared(cx);
        let interactions = RTCInteractionsStore::shared(cx);

        let subscriptions = vec![
            cx.observe(&rtc, |this, _, cx| this.synchronize(cx)),
            cx.observe(&system_calls, |this, _, cx| this.synchronize(cx)),
        ];

        Sai::log("RTC", || "room presentation host mounted".to_owned(), SaiLogKind::Network, cx);
        let handle = cx.weak_entity();
        cx.set_global(RiseRTCRoomPresentationHostHandle(handle));

        let rtc_store = rtc.read(cx);
        let last_target = rtc_store.room_presentation_target().cloned();
        let last_is_visible = rtc_store.is_room_presentation_visible();
        let last_account_id = rtc_store.account_id(cx).map(str::to_owned);

        Self {
            rtc,
            system_calls,
            interactions,
            navigation_store: None,
            call_window: None,
            call_window_opening: false,
            call_window_is_active: false,
            minimized_direct_call_uuid: None,
            dismissed_terminal_call_uuid: None,
            presented_system_call_uuids: HashSet::new(),
            presented_system_call_order: VecDeque::new(),
            is_mounted_in_window: false,
            did_synchronize: false,
            last_target,
            seated_voice_room: None,
            last_is_visible,
            last_eviction: None,
            last_account_id,
            last_displayed_call_uuid: None,
            last_active_server_call_id: None,
            last_displayed_server_call_id: None,
            terminal_task_identity: None,
            ending_task_identity: None,
            terminal_task: None,
            ending_task: None,
            _subscriptions: subscriptions,
        }
    }

    /// everything the host body on_change and task handlers did
    /// handlers of the call window run with that window leased and closing
    /// it there would fail, so they come here through set_needs_synchronize
    fn synchronize(&mut self, cx: &mut Context<Self>) {
        self.synchronize_room_presentation(cx);
        self.synchronize_system_call_presentation(cx);
        self.synchronize_settling_tasks(cx);
        self.synchronize_call_window(cx);
        self.did_synchronize = true;
        cx.emit(RiseRTCCallWindowContentChanged);
    }

    fn set_needs_synchronize(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = this.update(cx, |host, cx| host.synchronize(cx));
        });
    }

    fn synchronize_room_presentation(&mut self, cx: &mut Context<Self>) {
        let rtc = self.rtc.read(cx);
        let target = rtc.room_presentation_target().cloned();
        let is_visible = rtc.is_room_presentation_visible();
        let eviction = rtc.room_eviction(cx).cloned();
        let account_id = rtc.account_id(cx).map(str::to_owned);
        let active_room_id = rtc.active_room(cx).map(|room| room.config.room_id.clone());
        self.settle_switched_voice_seat(target.as_ref(), active_room_id, cx);

        if target != self.last_target {
            let kind = target.as_ref().map_or("none", |target| target.safe_log_kind()).to_owned();
            Sai::log("RTC", move || format!("room presentation target={kind}"), SaiLogKind::Network, cx);
            self.last_target = target;
        }
        if is_visible != self.last_is_visible {
            Sai::log("RTC", move || format!("room presentation visible={is_visible}"), SaiLogKind::Network, cx);
            self.last_is_visible = is_visible;
        }
        if eviction != self.last_eviction {
            self.last_eviction = eviction.clone();
            if let Some(eviction) = eviction {
                self.settle_room_eviction(&eviction, cx);
            }
        }
        if account_id != self.last_account_id {
            let signed_out = account_id.is_none();
            self.last_account_id = account_id;
            if signed_out {
                self.interactions.update(cx, |interactions, cx| interactions.dismiss_room_presentation(cx));
                self.minimized_direct_call_uuid = None;
                self.dismissed_terminal_call_uuid = None;
                self.presented_system_call_uuids.clear();
                self.presented_system_call_order.clear();
                self.system_calls.update(cx, |store, cx| store.reset_dismissed_presentations(cx));
            }
        }
    }

    fn settle_switched_voice_seat(
        &mut self,
        target: Option<&RiseRTCRoomTarget>,
        active_room_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let switched = matches!(
            (&self.seated_voice_room, active_room_id.as_deref()),
            (Some((_, seated)), Some(active)) if seated.as_str() != active
        );
        if switched && let Some((seated, _)) = self.seated_voice_room.take() {
            tracing::warn!(target: "riseonly::X1858", kind = seated.safe_log_kind(), "voice seat released by room switch");
            RiseRTCNavigationGate::mark_self_in_voice_channel(&seated, false, cx);
            if target == Some(&seated) {
                self.interactions.update(cx, |interactions, cx| interactions.dismiss_room_presentation(cx));
            }
        }
        match (target, active_room_id) {
            (Some(target), Some(active))
                if !matches!(target, RiseRTCRoomTarget::DirectCall { .. })
                    && target.room_id().as_deref() == Some(active.as_str()) =>
            {
                self.seated_voice_room = Some((target.clone(), active));
            }
            (None, None) => self.seated_voice_room = None,
            _ => {}
        }
    }

    fn synchronize_system_call_presentation(&mut self, cx: &mut Context<Self>) {
        let displayed_uuid = self.displayed_system_call(cx).map(|call| call.local_uuid);
        if let Some(new_value) = displayed_uuid {
            self.remember_presented_system_call(new_value);
        }
        if self.last_displayed_call_uuid != displayed_uuid {
            self.last_displayed_call_uuid = displayed_uuid;
            if displayed_uuid != self.minimized_direct_call_uuid {
                self.minimized_direct_call_uuid = None;
            }
            if displayed_uuid.is_some() && displayed_uuid != self.dismissed_terminal_call_uuid {
                self.dismissed_terminal_call_uuid = None;
            }
        }

        let active_server_call_id = self.active_server_direct_call(cx).map(|call| call.call_id.clone());
        let displayed_server_call_id =
            self.displayed_system_call(cx).and_then(|call| call.server_call_id.clone());
        if !self.did_synchronize
            || active_server_call_id != self.last_active_server_call_id
            || displayed_server_call_id != self.last_displayed_server_call_id
        {
            self.last_active_server_call_id = active_server_call_id;
            self.last_displayed_server_call_id = displayed_server_call_id;
            self.synchronize_server_direct_call_presentation(cx);
        }
    }

    fn synchronize_settling_tasks(&mut self, cx: &mut Context<Self>) {
        let terminal_identity = self.terminal_system_call_task_identity(cx);
        if terminal_identity != self.terminal_task_identity {
            self.terminal_task_identity = terminal_identity;
            self.terminal_task = self.settle_terminal_system_call_if_needed(cx);
        }
        let ending_identity = self.ending_system_call_task_identity(cx);
        if ending_identity != self.ending_task_identity {
            self.ending_task_identity = ending_identity;
            self.ending_task = self.settle_ending_system_call_if_needed(cx);
        }
    }

    fn synchronize_call_window(&mut self, cx: &mut Context<Self>) {
        if !self.is_mounted_in_window {
            return;
        }
        let should_be_open = self.is_presentation_shown(cx);
        if should_be_open && self.call_window.is_none() && !self.call_window_opening {
            self.open_call_window(cx);
        } else if !should_be_open && let Some(handle) = self.call_window.take() {
            self.call_window_is_active = false;
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    fn open_call_window(&mut self, cx: &mut Context<Self>) {
        let host = cx.entity();
        let navigation_store = self.navigation_store.clone();
        let (width, height) =
            if self.is_direct_call_presentation(cx) { DIRECT_CALL_WINDOW_SIZE } else { ROOM_WINDOW_SIZE };
        let bounds = gpui::Bounds::centered(None, size(px(width), px(height)), cx);
        let traffic_lights = rise_ui::theme(cx).shell.traffic_light_origin();
        self.call_window_opening = true;
        cx.defer(move |cx| {
            let opened = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(CALL_WINDOW_MIN_SIZE.0), px(CALL_WINDOW_MIN_SIZE.1))),
                    app_id: rise_rtc_call_window_hooks().and_then(|hooks| (hooks.app_id)()),
                    titlebar: Some(TitlebarOptions {
                        title: Some(tr("rtc_voice_chat").into()),
                        appears_transparent: true,
                        traffic_light_position: Some(traffic_lights),
                    }),
                    ..Default::default()
                },
                {
                    let host = host.clone();
                    move |window, cx| cx.new(|cx| RiseRTCCallWindow::new(host, navigation_store, window, cx))
                },
            );
            host.update(cx, |this, cx| {
                this.call_window_opening = false;
                match opened {
                    Ok(handle) => {
                        this.call_window = Some(handle);
                        this.call_window_is_active = true;
                    }
                    Err(error) => {
                        Sai::log("RTC", move || format!("call window failed: {error}"), SaiLogKind::Network, cx);
                        this.minimize_active_presentation(cx);
                    }
                }
            });
        });
    }

    fn set_call_window_active(&mut self, active: bool) {
        self.call_window_is_active = active;
    }

    fn is_call_window_behind(&self) -> bool {
        self.call_window.is_some() && !self.call_window_is_active
    }

    fn settle_room_eviction(&mut self, eviction: &RiseRTCRoomEviction, cx: &mut Context<Self>) {
        self.interactions.update(cx, |interactions, cx| interactions.acknowledge_room_eviction(cx));
        let target = self.rtc.read(cx).room_presentation_target().cloned();
        if let Some(target) = target
            && target.room_id().as_deref() == Some(eviction.room_id.as_str())
        {
            RiseRTCNavigationGate::mark_self_in_voice_channel(&target, false, cx);
            self.interactions.update(cx, |interactions, cx| interactions.dismiss_room_presentation(cx));
        }
        tracing::warn!(
            target: "riseonly::X1850",
            reason = eviction.reason.raw_value(),
            key = eviction.reason.message_key(),
            "room eviction toast"
        );
        Notifier::send(
            tr(eviction.reason.message_key()),
            NotifierSendOptions {
                r#type: eviction.reason.notifier_type(),
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn is_presentation_shown(&self, cx: &App) -> bool {
        if self.displayed_system_call(cx).is_some() {
            return self.is_system_direct_call_visible(cx);
        }
        let rtc = self.rtc.read(cx);
        rtc.room_presentation_target().is_some() && rtc.is_room_presentation_visible()
    }

    fn displayed_system_call<'a>(&self, cx: &'a App) -> Option<&'a RiseRTCSystemCallPresentation> {
        let system_calls = self.system_calls.read(cx);
        if let Some(active) = system_calls.calls().iter().find(|call| {
            !call.is_terminal()
                && !system_calls.dismissed_local_uuids().contains(&call.local_uuid)
                && Self::should_open_system_call(call)
        }) {
            return Some(active);
        }
        let latest = system_calls.calls().first()?;
        (latest.is_terminal()
            && Some(latest.local_uuid) != self.dismissed_terminal_call_uuid
            && !system_calls.dismissed_local_uuids().contains(&latest.local_uuid)
            && self.should_show_terminal_system_call(latest))
        .then_some(latest)
    }

    fn should_open_system_call(call: &RiseRTCSystemCallPresentation) -> bool {
        call.direction == RiseRTCSystemCallDirection::Outgoing
            || call.phase != RiseRTCSystemCallPhase::IncomingRinging
            || call.requires_in_app_answer_ui
    }

    fn should_show_terminal_system_call(&self, call: &RiseRTCSystemCallPresentation) -> bool {
        if self.presented_system_call_uuids.contains(&call.local_uuid) {
            return true;
        }
        if call.direction != RiseRTCSystemCallDirection::Outgoing {
            return false;
        }
        let Some(terminal_at) = &call.terminal_at else {
            return false;
        };
        Date::now().time_interval_since(terminal_at).max(0.0) <= TERMINAL_SYSTEM_CALL_MAX_AGE_SECONDS
    }

    fn terminal_system_call_task_identity(&self, cx: &App) -> Option<Uuid> {
        self.displayed_system_call(cx).filter(|call| call.is_terminal()).map(|call| call.local_uuid)
    }

    fn ending_system_call_task_identity(&self, cx: &App) -> Option<Uuid> {
        self.displayed_system_call(cx)
            .filter(|call| call.phase == RiseRTCSystemCallPhase::Ending)
            .map(|call| call.local_uuid)
    }

    fn settle_terminal_system_call_if_needed(&mut self, cx: &mut Context<Self>) -> Option<Task<()>> {
        let call = self.displayed_system_call(cx).filter(|call| call.is_terminal())?;
        if Some(call.local_uuid) == self.dismissed_terminal_call_uuid {
            return None;
        }
        let call_uuid = call.local_uuid;
        let server_call_id = call.server_call_id.clone();
        let hold = Self::terminal_hold(call.terminal_outcome);
        let call_identity = call_uuid.uuid_string();

        Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(END_TONE_HANDOFF).await;
            cx.update(|cx| {
                RiseRTCEndTonePlayer::shared(cx).update(cx, |player, cx| player.play_once(&call_identity, true, cx));
            });
            cx.background_executor().timer(hold).await;
            let _ = this.update(cx, |host, cx| {
                let still_terminal = host
                    .system_calls
                    .read(cx)
                    .calls()
                    .iter()
                    .any(|call| call.local_uuid == call_uuid && call.is_terminal());
                if still_terminal {
                    host.dismiss_system_call_presentation(call_uuid, server_call_id, cx);
                }
            });
        }))
    }

    fn terminal_hold(outcome: Option<RiseRTCSystemCallTerminalOutcome>) -> Duration {
        match outcome {
            Some(RiseRTCSystemCallTerminalOutcome::Failed) => RiseRTCTerminalHold::FAILURE,
            Some(RiseRTCSystemCallTerminalOutcome::Declined | RiseRTCSystemCallTerminalOutcome::TimedOut) => {
                RiseRTCTerminalHold::NOT_CHOSEN
            }
            Some(RiseRTCSystemCallTerminalOutcome::Cancelled) => RiseRTCTerminalHold::CANCELLED,
            Some(RiseRTCSystemCallTerminalOutcome::Ended | RiseRTCSystemCallTerminalOutcome::AnsweredElsewhere)
            | None => RiseRTCTerminalHold::CHOSEN,
        }
    }

    fn settle_ending_system_call_if_needed(&mut self, cx: &mut Context<Self>) -> Option<Task<()>> {
        let call = self.displayed_system_call(cx).filter(|call| call.phase == RiseRTCSystemCallPhase::Ending)?;
        let call_uuid = call.local_uuid;
        let server_call_id = call.server_call_id.clone();

        Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(ENDING_SYSTEM_CALL_DEADLINE).await;
            let is_visible = this.read_with(cx, |host, cx| host.is_system_direct_call_visible(cx)).unwrap_or(false);
            if is_visible {
                cx.background_executor().timer(RiseRTCTerminalHold::NOT_CHOSEN).await;
            }
            let _ = this.update(cx, |host, cx| {
                let still_ending = host
                    .system_calls
                    .read(cx)
                    .calls()
                    .iter()
                    .any(|call| call.local_uuid == call_uuid && call.phase == RiseRTCSystemCallPhase::Ending);
                if still_ending {
                    host.dismiss_system_call_presentation(call_uuid, server_call_id, cx);
                }
            });
        }))
    }

    fn remember_presented_system_call(&mut self, call_uuid: Uuid) {
        if !self.presented_system_call_uuids.insert(call_uuid) {
            return;
        }
        self.presented_system_call_order.push_back(call_uuid);
        while self.presented_system_call_order.len() > MAX_REMEMBERED_SYSTEM_CALLS {
            if let Some(oldest) = self.presented_system_call_order.pop_front() {
                self.presented_system_call_uuids.remove(&oldest);
            }
        }
    }

    fn active_server_direct_call<'a>(&self, cx: &'a App) -> Option<&'a RiseRTCDirectCall> {
        let rtc = self.rtc.read(cx);
        let room = rtc.active_room(cx)?;
        if room.config.mode != RiseRTCRoomMode::DirectCall {
            return None;
        }
        rtc.recent_calls(cx).iter().find(|call| call.room_id == room.config.room_id && !call.state.is_terminal())
    }

    fn is_system_direct_call_visible(&self, cx: &App) -> bool {
        let Some(call) = self.displayed_system_call(cx) else {
            return false;
        };
        self.minimized_direct_call_uuid != Some(call.local_uuid)
            && self.dismissed_terminal_call_uuid != Some(call.local_uuid)
    }

    fn is_direct_call_presentation(&self, cx: &App) -> bool {
        self.displayed_system_call(cx).is_some() || self.server_direct_call_id(cx).is_some()
    }

    fn minimize_active_presentation(&mut self, cx: &mut Context<Self>) {
        if let Some(local_uuid) = self.displayed_system_call(cx).map(|call| call.local_uuid) {
            self.minimized_direct_call_uuid = Some(local_uuid);
            self.set_needs_synchronize(cx);
        } else {
            self.interactions.update(cx, |interactions, cx| interactions.minimize_room(cx));
        }
    }

    fn dismiss_server_direct_call_presentation(&mut self, cx: &mut Context<Self>) {
        if self.server_direct_call_id(cx).is_none() {
            return;
        }
        self.interactions.update(cx, |interactions, cx| interactions.dismiss_room_presentation(cx));
    }

    fn dismiss_system_call_presentation(
        &mut self,
        local_uuid: Uuid,
        server_call_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let current_server_call_id = self
            .system_calls
            .read(cx)
            .calls()
            .iter()
            .find(|call| call.local_uuid == local_uuid)
            .and_then(|call| call.server_call_id.clone())
            .or(server_call_id);
        self.system_calls.update(cx, |store, cx| {
            store.dismiss_presentation(local_uuid, current_server_call_id.as_deref(), cx);
        });
        self.dismissed_terminal_call_uuid = Some(local_uuid);
        if self.minimized_direct_call_uuid == Some(local_uuid) {
            self.minimized_direct_call_uuid = None;
        }
        self.dismiss_server_direct_call_presentation(cx);
        self.set_needs_synchronize(cx);
    }

    fn server_direct_call_id<'a>(&self, cx: &'a App) -> Option<&'a str> {
        match self.rtc.read(cx).room_presentation_target() {
            Some(RiseRTCRoomTarget::DirectCall { call_id }) => Some(call_id.as_str()),
            _ => None,
        }
    }

    fn synchronize_server_direct_call_presentation(&mut self, cx: &mut Context<Self>) {
        if self.displayed_system_call(cx).is_some() {
            return;
        }
        let Some(call_id) = self.active_server_direct_call(cx).map(|call| call.call_id.clone()) else {
            return;
        };
        if self.system_calls.read(cx).dismissed_server_call_ids().contains(&call_id) {
            return;
        }
        let target = RiseRTCRoomTarget::DirectCall { call_id };
        if self.rtc.read(cx).room_presentation_target() == Some(&target) {
            return;
        }
        self.interactions.update(cx, |interactions, cx| interactions.present_room(target, cx));
    }

    fn call_window_content(&self, host: WeakEntity<Self>, cx: &App) -> AnyElement {
        if let Some(call) = self.displayed_system_call(cx) {
            let local_uuid = call.local_uuid;
            let server_call_id = call.server_call_id.clone();
            let minimized = host.clone();
            let settled = host;
            let settled_server_call_id = server_call_id.clone();
            return RiseRTCDirectCallScreen::new(
                Some(local_uuid),
                server_call_id,
                move |_: &mut Window, cx: &mut App| {
                    let _ = minimized.update(cx, |host, cx| {
                        host.minimized_direct_call_uuid = Some(local_uuid);
                        host.set_needs_synchronize(cx);
                    });
                },
                move |_: &mut Window, cx: &mut App| {
                    let server_call_id = settled_server_call_id.clone();
                    let _ = settled.update(cx, |host, cx| {
                        host.dismiss_system_call_presentation(local_uuid, server_call_id, cx);
                    });
                },
            )
            .into_any_element();
        }
        if let Some(call_id) = self.server_direct_call_id(cx).map(str::to_owned) {
            let minimized = self.interactions.clone();
            let settled = self.interactions.clone();
            let rtc = self.rtc.clone();
            let target = RiseRTCRoomTarget::DirectCall { call_id: call_id.clone() };
            return RiseRTCDirectCallScreen::new(
                None,
                Some(call_id),
                move |_: &mut Window, cx: &mut App| {
                    minimized.update(cx, |interactions, cx| interactions.minimize_room(cx));
                },
                move |_: &mut Window, cx: &mut App| {
                    if rtc.read(cx).room_presentation_target() == Some(&target) {
                        settled.update(cx, |interactions, cx| interactions.dismiss_room_presentation(cx));
                    }
                },
            )
            .into_any_element();
        }
        if let Some(target) = self.rtc.read(cx).room_presentation_target().cloned() {
            return RiseRTCRoomScreen::new(target, true).into_any_element();
        }
        div().size_full().into_any_element()
    }
}

/// a synchronize pass ran, the call window content reads host state that may have changed
pub struct RiseRTCCallWindowContentChanged;

impl EventEmitter<RiseRTCCallWindowContentChanged> for RiseRTCRoomPresentationHost {}

impl Render for RiseRTCRoomPresentationHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.navigation_store = env::<NavigationStoreKey>(cx);
        if !self.is_mounted_in_window {
            self.is_mounted_in_window = true;
            self.set_needs_synchronize(cx);
        }
        div()
    }
}

/// root of the call window, the desktop place of the full page bottom sheet
pub struct RiseRTCCallWindow {
    host: Entity<RiseRTCRoomPresentationHost>,
    navigation_store: Option<Entity<NavigationStore>>,
    applied_direct_call_controls: Option<bool>,
    focus_handle: FocusHandle,
    title_bar: Option<AnyView>,
    hold_context_menu_host: Entity<HoldContextMenuHost>,
    notifier_host: Entity<NotifierHost>,
    _host_changes: Vec<Subscription>,
}

impl RiseRTCCallWindow {
    fn new(
        host: Entity<RiseRTCRoomPresentationHost>,
        navigation_store: Option<Entity<NavigationStore>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let closing = host.clone();
        window.on_window_should_close(cx, move |_, cx| {
            Self::minimize_call(&closing, cx);
            true
        });
        let activation_host = host.downgrade();
        let host_changes = vec![
            cx.observe(&host, |_, _, cx| cx.notify()),
            cx.subscribe(&host, |_, _, _: &RiseRTCCallWindowContentChanged, cx| cx.notify()),
            cx.observe_window_activation(window, move |_, window, cx| {
                let active = window.is_window_active();
                let _ = activation_host.update(cx, |host, _| host.set_call_window_active(active));
            }),
        ];
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let closing = host.clone();
        let title_bar = rise_rtc_call_window_chrome_hooks().map(|hooks| {
            (hooks.title_bar)(
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    Self::minimize_call(&closing, cx);
                    window.remove_window();
                }),
                window,
                cx,
            )
        });
        let notifier_host = cx.new(NotifierHost::window_scoped);
        Self {
            host,
            navigation_store,
            applied_direct_call_controls: None,
            focus_handle,
            title_bar,
            hold_context_menu_host: cx.new(|cx| HoldContextMenuHost::new(window, cx)),
            notifier_host,
            _host_changes: host_changes,
        }
    }

    /// closing the window minimizes the call, never hangs up
    /// call card lives in a shell window, with none left the process quits and ends the call
    /// so a shell opens first and sync, gpui checks for quit once the window is gone
    fn minimize_call(host: &Entity<RiseRTCRoomPresentationHost>, cx: &mut App) {
        host.update(cx, |host, cx| {
            host.call_window = None;
            host.call_window_is_active = false;
            host.minimize_active_presentation(cx);
        });
        if let Some(hooks) = handover_hooks()
            && !cx.windows().into_iter().any(|window| (hooks.is_shell)(window))
        {
            (hooks.open_shell)(cx);
        }
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        Self::minimize_call(&self.host, cx);
        window.remove_window();
    }
}

impl Render for RiseRTCCallWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_direct = self.host.read(cx).is_direct_call_presentation(cx);
        if self.applied_direct_call_controls != Some(is_direct) {
            self.applied_direct_call_controls = Some(is_direct);
            set_system_controls_appearance(window, is_direct.then_some(WindowAppearance::Dark));
            if let Some(hooks) = rise_rtc_call_window_chrome_hooks()
                && let Some(title_bar) = &self.title_bar
            {
                (hooks.set_title_bar_controls_theme)(title_bar, is_direct.then(AppTheme::dark), cx);
            }
        }
        let content = self.host.read(cx).call_window_content(self.host.downgrade(), cx);
        let background = if is_direct { black() } else { rise_ui::theme(cx).bg._000 };
        let band = window_controls_band(window, cx);
        let top_inset = f32::from(WindowCoverFrame::resolve(window, cx).safe_top);
        let body = div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close_window))
            .size_full()
            .relative()
            .bg(background)
            .pt(band)
            .child(
                env_scope::<NavigationStoreKey>(self.navigation_store.clone())
                    .child(env_scope::<RiseRTCScreenTopInsetKey>(top_inset).child(content)),
            )
            .children((band > px(0.0)).then(|| self.title_bar.clone()).flatten())
            .child(
                deferred(
                    AnyView::from(self.hold_context_menu_host.clone())
                        .cached(StyleRefinement::default().absolute().top_0().left_0().size_full()),
                )
                .with_priority(HoldContextMenuHost::LAYER_PRIORITY),
            )
            .child(
                deferred(
                    AnyView::from(self.notifier_host.clone())
                        .cached(StyleRefinement::default().absolute().top_0().left_0().size_full()),
                )
                .with_priority(NotifierHost::LAYER_PRIORITY),
            )
            .child(GuideOverlayWindow::new());
        match rise_rtc_call_window_chrome_hooks() {
            Some(hooks) => (hooks.window_frame)(body.into_any_element(), window, cx),
            None => body.into_any_element(),
        }
    }
}

trait RiseRTCEvictionReasonPresentation {
    fn message_key(self) -> &'static str;
    fn notifier_type(self) -> NotifierType;
}

impl RiseRTCEvictionReasonPresentation for RiseRTCEvictionReason {
    fn message_key(self) -> &'static str {
        match self {
            Self::Removed => "rtc_removed_from_voice",
            Self::RoomClosed => "rtc_voice_chat_closed",
        }
    }

    fn notifier_type(self) -> NotifierType {
        match self {
            Self::Removed => NotifierType::Error,
            Self::RoomClosed => NotifierType::Info,
        }
    }
}
