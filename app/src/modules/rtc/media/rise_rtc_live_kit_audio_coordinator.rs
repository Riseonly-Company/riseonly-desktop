use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext, Context, Entity, Task, WeakEntity};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::RiseSubscription;
use rise_i18n::tr;
use rise_media::rtc_media::{self, AudioDevice, MediaIntent, MediaSessionKey, RtcUnavailable};

use crate::core::desktop_platform::av_audio_session::AVAudioSession;
use crate::core::desktop_platform::av_audio_session_route::{AVAudioSessionRouteChange, AVAudioSessionRouteChangeReason};
use crate::core::desktop_platform::rise_app_events::RiseAppEvents;
use crate::core::lib::rise_call_audio_ownership::RiseCallAudioOwnership;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::modules::chat::components::chat::voice::chat_voice_message_player::ChatVoiceMessagePlayer;
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::rtc::engine::rise_rtc_call_system_hooks::RiseRTCCallSystemHooks;
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCAudioRoutingPolicy, RiseRTCJoinCredentials, RiseRTCMediaSessionKey, RiseRTCRoomMode, RiseRTCRoomModeExt,
    RiseRTCStateError,
};
use crate::modules::rtc::media::rise_rtc_audio_settings::RiseRTCAudioSettingsStore;
use crate::modules::rtc::shared::rise_rtc_room_locator::RiseRTCRoomLocator;
use crate::modules::rtc::system::rise_rtc_system_bridges::{
    RiseRTCMediaSystemBridge, RiseRTCMediaSystemBridgeAudioOperation, RiseRTCMediaSystemBridgeResetOperation,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCAudioInputRoute {
    pub id: String,
    pub name: String,
    pub port_type: String,
}

impl RiseRTCAudioInputRoute {
    pub const BUILT_IN_MIC_PORT_TYPE: &'static str = "MicrophoneBuiltIn";

    /// the built in microphone is the iPhone row like in the output
    /// list, its port name is the raw MicrophoneBuiltIn on the simulator
    pub fn display_name(&self) -> String {
        if self.port_type == Self::BUILT_IN_MIC_PORT_TYPE { tr("rtc_audio_iphone") } else { self.name.clone() }
    }

    fn from_device(device: &AudioDevice) -> Self {
        Self { id: device.id.clone(), name: device.name.clone(), port_type: String::new() }
    }
}

/// why a route pick did not land, a newer pick or another session took over
/// or the media layer refused it
/// also a speaker or headset the audio host lists as the output sheet shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCAudioOutputRoute {
    pub id: String,
    pub name: String,
}

impl RiseRTCAudioOutputRoute {
    fn from_device(device: &AudioDevice) -> Self {
        Self { id: device.id.clone(), name: device.name.clone() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseRTCAudioRouteError {
    #[error("audio route pick cancelled")]
    Cancelled,
    #[error(transparent)]
    State(#[from] RiseRTCStateError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OutputPreference {
    session: RiseRTCMediaSessionKey,
    speaker: bool,
    device: Option<String>,
}

pub struct RiseRTCLiveKitAudioCoordinator {
    available_inputs: Vec<RiseRTCAudioInputRoute>,
    available_outputs: Vec<RiseRTCAudioOutputRoute>,
    selected_input_id: Option<String>,
    has_system_input_choice: bool,
    speaker_enabled: bool,
    is_active: bool,

    bridge_owner_id: Uuid,
    active_mode: Option<RiseRTCRoomMode>,
    active_session_uses_call_kit: bool,
    route_observer: Option<RiseSubscription>,
    route_task: Option<Task<()>>,
    bridge_installed: bool,
    call_kit_audio_is_active: bool,
    restore_after_call_kit_deactivation: bool,
    route_selection_generation: u64,
    routes_generation: u64,
    routes_task: Option<Task<()>>,
    active_session_key: Option<RiseRTCMediaSessionKey>,
    output_preference: Option<OutputPreference>,
    preferred_input: Option<String>,
    speaker_reassertions: Vec<Duration>,
}

impl RiseRTCLiveKitAudioCoordinator {
    const SPEAKER_REASSERTION_LIMIT: usize = 3;
    const SPEAKER_REASSERTION_PERIOD: Duration = Duration::from_secs(5);

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let (changes, mut inbox) = mpsc::unbounded::<Option<AVAudioSessionRouteChangeReason>>();
        let route_observer = RiseAppEvents::default_center().observe(AVAudioSession::ROUTE_CHANGE_NOTIFICATION, move |notification| {
            let reason = notification.payload::<AVAudioSessionRouteChange>().map(|change| change.reason);
            let _ = changes.unbounded_send(reason);
        });
        let route_task = cx.spawn(async move |this: WeakEntity<Self>, cx| {
            while let Some(reason) = inbox.next().await {
                if this.update(cx, |coordinator, cx| coordinator.route_did_change(reason, cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            available_inputs: Vec::new(),
            available_outputs: Vec::new(),
            selected_input_id: None,
            has_system_input_choice: false,
            speaker_enabled: false,
            is_active: false,
            bridge_owner_id: Uuid::new_v4(),
            active_mode: None,
            active_session_uses_call_kit: false,
            route_observer: Some(route_observer),
            route_task: Some(route_task),
            bridge_installed: false,
            call_kit_audio_is_active: false,
            restore_after_call_kit_deactivation: false,
            route_selection_generation: 0,
            routes_generation: 0,
            routes_task: None,
            active_session_key: None,
            output_preference: None,
            preferred_input: None,
            speaker_reassertions: Vec::new(),
        }
    }

    pub fn available_inputs(&self) -> &[RiseRTCAudioInputRoute] {
        &self.available_inputs
    }

    pub fn available_outputs(&self) -> &[RiseRTCAudioOutputRoute] {
        &self.available_outputs
    }

    /// output device the person picked, none follows the system choice
    pub fn selected_output_id(&self) -> Option<&str> {
        self.output_preference.as_ref().and_then(|preference| preference.device.as_deref())
    }

    /// the host lists the systems own choice under a raw name (ALSA), the
    /// sheet shows one row for it selected while selected_input_id is none
    pub fn has_system_input_choice(&self) -> bool {
        self.has_system_input_choice
    }

    pub fn selected_input_id(&self) -> Option<&str> {
        self.selected_input_id.as_deref()
    }

    pub fn speaker_enabled(&self) -> bool {
        self.speaker_enabled
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn install_system_bridge(&mut self, cx: &mut Context<Self>) {
        if self.bridge_installed {
            return;
        }
        self.bridge_installed = true;
        let owner_id = self.bridge_owner_id;
        let activate_target = cx.weak_entity();
        let deactivate_target = cx.weak_entity();
        let reset_target = cx.weak_entity();
        let activate: RiseRTCMediaSystemBridgeAudioOperation = std::rc::Rc::new(move |cx: &mut App| {
            activate_target.update(cx, |coordinator, cx| coordinator.call_kit_did_activate(cx)).ok();
        });
        let deactivate: RiseRTCMediaSystemBridgeAudioOperation = std::rc::Rc::new(move |cx: &mut App| {
            deactivate_target.update(cx, |coordinator, cx| coordinator.call_kit_did_deactivate(cx)).ok();
        });
        let reset: RiseRTCMediaSystemBridgeResetOperation = std::rc::Rc::new(move |cx: &mut App| {
            reset_target.update(cx, |coordinator, cx| coordinator.release(None, cx)).ok();
        });
        RiseRTCMediaSystemBridge::shared(cx).update(cx, |bridge, _| {
            bridge.install(owner_id, activate, deactivate, reset);
        });
    }

    pub fn prepare(&mut self, credentials: &RiseRTCJoinCredentials, cx: &mut Context<Self>) -> Result<(), RiseRTCStateError> {
        self.route_selection_generation = self.route_selection_generation.wrapping_add(1);
        self.prepare_session(credentials, cx)
    }

    fn prepare_session(&mut self, credentials: &RiseRTCJoinCredentials, cx: &mut Context<Self>) -> Result<(), RiseRTCStateError> {
        self.install_system_bridge(cx);
        let session_key = credentials.session_key();
        if self.output_preference.as_ref().map(|preference| &preference.session) != Some(&session_key) {
            self.output_preference = None;
        }
        let is_new_session = self.active_session_key.as_ref() != Some(&session_key);
        self.active_session_key = Some(session_key);
        self.active_mode = Some(credentials.room_mode);
        self.active_session_uses_call_kit = Self::system_audio_owner(credentials, cx);
        self.restore_after_call_kit_deactivation = false;
        RiseCallAudioOwnership::set_call_active(true);
        SaiWs::set_transport_hold("rtc.media", true, cx);
        MusicPlayerState::shared(cx).update(cx, |state, cx| {
            state.suspend_for_external_audio(MusicExternalAudioOwner::RTC, true, cx);
        });
        ChatVoiceMessagePlayer::shared(cx).update(cx, |player, cx| player.stop(cx));
        VideoNotePlayerStore::shared(cx).update(cx, |store, cx| store.stop(cx));

        let applied = RiseRTCAudioSettingsStore::shared(cx).read(cx).apply_to_audio_engine();
        if let Err(error) = applied {
            Sai::log(
                "RTC",
                move || format!("audio processing prepare skipped error_type=RtcUnavailable error_value={error:?}"),
                SaiLogKind::Network,
                cx,
            );
        }
        if is_new_session {
            self.preferred_input = Self::hand_settings_devices_to_media(cx);
        }
        let routing = RiseRTCAudioRoutingPolicy::decision(credentials.room_mode, credentials.call_media_mode);
        if self.output_preference.is_none() {
            self.speaker_enabled = routing.prefers_speaker;
        }

        if self.active_session_uses_call_kit {
            self.is_active = self.call_kit_audio_is_active;
        } else {
            self.is_active = true;
        }
        cx.notify();
        self.refresh_routes(cx);
        self.reapply_output_preference(cx);
        Ok(())
    }

    pub fn select_input(
        &mut self,
        id: Option<String>,
        session: &RiseRTCMediaSessionKey,
        cx: &mut Context<Self>,
    ) -> Result<(), RiseRTCAudioRouteError> {
        self.route_selection_generation = self.route_selection_generation.wrapping_add(1);
        if self.output_preference.as_ref().is_some_and(|preference| &preference.session == session) {
            self.output_preference = None;
        }
        if self.active_session_key.as_ref() != Some(session) {
            return Err(RiseRTCAudioRouteError::Cancelled);
        }
        if let Err(error) = self.apply_input(id.as_deref(), session) {
            self.refresh_routes(cx);
            return Err(error.into());
        }
        self.refresh_routes(cx);
        Ok(())
    }

    pub fn select_output(
        &mut self,
        id: Option<String>,
        session: &RiseRTCMediaSessionKey,
        cx: &mut Context<Self>,
    ) -> Result<(), RiseRTCAudioRouteError> {
        self.route_selection_generation = self.route_selection_generation.wrapping_add(1);
        let speaker = id.as_deref() == Some("speaker");
        let preference = OutputPreference {
            session: session.clone(),
            speaker,
            device: if speaker { None } else { id },
        };
        self.output_preference = Some(preference.clone());
        if self.active_session_key.as_ref() != Some(session) {
            return Ok(());
        }
        if let Err(error) = Self::apply_output(&preference) {
            self.refresh_routes(cx);
            return Err(error.into());
        }
        self.speaker_enabled = preference.speaker;
        cx.notify();
        self.refresh_routes(cx);
        Ok(())
    }

    fn apply_input(&mut self, id: Option<&str>, session: &RiseRTCMediaSessionKey) -> Result<(), RiseRTCStateError> {
        let chosen = match id.filter(|id| !id.is_empty()) {
            None => None,
            Some(id) => {
                if !self.available_inputs.iter().any(|input| input.id == id) {
                    return Err(RiseRTCStateError::Validation("rtc_audio_input_unavailable".to_owned()));
                }
                Some(id.to_owned())
            }
        };
        rtc_media::perform(&MediaIntent::SelectAudioInput { session: Self::media_key(session), id: chosen.clone() })
            .map_err(Self::state_error)?;
        self.preferred_input = chosen;
        Ok(())
    }

    pub fn adopt_settings_devices(&mut self, input: Option<String>, cx: &mut Context<Self>) {
        if self.active_session_key.is_none() {
            return;
        }
        self.route_selection_generation = self.route_selection_generation.wrapping_add(1);
        self.preferred_input = input;
        if self.output_preference.as_ref().map(|preference| &preference.session) == self.active_session_key.as_ref() {
            self.output_preference = None;
        }
        cx.notify();
        self.refresh_routes(cx);
    }

    /// hands the devices picked in audio settings to the media layer and answers the microphone
    fn hand_settings_devices_to_media(cx: &mut App) -> Option<String> {
        let store = RiseRTCAudioSettingsStore::shared(cx);
        let (input, output) = {
            let store = store.read(cx);
            (store.input_device().map(str::to_owned), store.output_device().map(str::to_owned))
        };
        if let Err(error) = rtc_media::set_preferred_audio_devices(input.clone(), output) {
            Sai::log("RTC", move || format!("audio device preference skipped: {error}"), SaiLogKind::Network, cx);
        }
        input
    }

    fn apply_output(preference: &OutputPreference) -> Result<(), RiseRTCStateError> {
        rtc_media::perform(&MediaIntent::SelectAudioOutput {
            session: Self::media_key(&preference.session),
            id: preference.device.clone(),
        })
        .map_err(Self::state_error)
    }

    pub fn release(&mut self, session: Option<&RiseRTCMediaSessionKey>, cx: &mut Context<Self>) {
        if let Some(session) = session
            && self.active_session_key.as_ref() != Some(session)
        {
            return;
        }
        self.route_selection_generation = self.route_selection_generation.wrapping_add(1);
        self.release_session(cx);
    }

    fn release_session(&mut self, cx: &mut Context<Self>) {
        if self.output_preference.as_ref().map(|preference| &preference.session) == self.active_session_key.as_ref() {
            self.output_preference = None;
        }
        self.speaker_reassertions.clear();
        self.active_session_key = None;
        RiseCallAudioOwnership::set_call_active(false);
        let _ = Self::hand_settings_devices_to_media(cx);
        SaiWs::set_transport_hold("rtc.media", false, cx);
        if self.active_mode.is_none() {
            return;
        }
        let used_call_kit = self.active_session_uses_call_kit;
        self.active_mode = None;
        self.active_session_uses_call_kit = false;
        self.is_active = false;
        self.preferred_input = None;
        self.routes_generation = self.routes_generation.wrapping_add(1);
        drop(self.routes_task.take());
        self.available_inputs.clear();
        self.available_outputs.clear();
        self.selected_input_id = None;
        self.has_system_input_choice = false;
        self.speaker_enabled = false;
        cx.notify();

        if used_call_kit {
            if self.call_kit_audio_is_active {
                self.restore_after_call_kit_deactivation = true;
            } else {
                Self::restore_playback_category(cx);
            }
        } else {
            Self::restore_playback_category(cx);
        }
    }

    fn call_kit_did_activate(&mut self, cx: &mut Context<Self>) {
        self.call_kit_audio_is_active = true;
        if !self.active_session_uses_call_kit {
            return;
        }
        self.is_active = true;
        cx.notify();
        self.refresh_routes(cx);
        self.reapply_output_preference(cx);
    }

    fn call_kit_did_deactivate(&mut self, cx: &mut Context<Self>) {
        self.call_kit_audio_is_active = false;
        if self.restore_after_call_kit_deactivation {
            self.restore_after_call_kit_deactivation = false;
            self.is_active = false;
            cx.notify();
            Self::restore_playback_category(cx);
            return;
        }
        if !self.active_session_uses_call_kit {
            return;
        }
        self.is_active = false;
        cx.notify();
        self.refresh_routes(cx);
    }

    fn system_audio_owner(credentials: &RiseRTCJoinCredentials, cx: &App) -> bool {
        if !credentials.room_mode.uses_call_kit_audio_session() {
            return false;
        }
        let Some(call_id) = RiseRTCRoomLocator::direct_call_id(&credentials.room_id) else {
            return false;
        };
        RiseRTCCallSystemHooks::drives_system_audio(&call_id, cx)
    }

    fn restore_playback_category(cx: &mut App) {
        MusicPlayerState::shared(cx).update(cx, |state, cx| {
            state.resume_after_external_audio(MusicExternalAudioOwner::RTC, cx);
        });
    }

    fn reapply_output_preference(&mut self, cx: &mut Context<Self>) {
        let Some(preference) = self.output_preference.clone() else {
            return;
        };
        if self.active_session_key.as_ref() != Some(&preference.session) {
            return;
        }
        if let Err(error) = Self::apply_output(&preference) {
            Sai::log("RTC", move || format!("audio output reapply failed: {error}"), SaiLogKind::Network, cx);
        }
        self.speaker_enabled = preference.speaker;
        cx.notify();
        self.refresh_routes(cx);
    }

    fn route_did_change(&mut self, reason: Option<AVAudioSessionRouteChangeReason>, cx: &mut Context<Self>) {
        self.refresh_routes(cx);
        let Some(session) = self.active_session_key.clone() else {
            return;
        };
        if self.active_mode.is_none() {
            return;
        }
        let preference = self
            .output_preference
            .clone()
            .filter(|preference| preference.session == session)
            .unwrap_or(OutputPreference { session, speaker: self.speaker_enabled, device: None });
        let now = RiseClock::monotonic();
        self.speaker_reassertions
            .retain(|applied| now.saturating_sub(*applied) <= Self::SPEAKER_REASSERTION_PERIOD);
        if self.speaker_reassertions.len() >= Self::SPEAKER_REASSERTION_LIMIT {
            return;
        }
        self.speaker_reassertions.push(now);
        let raw_reason = reason.map_or_else(|| "nil".to_owned(), |reason| reason.raw_value().to_string());
        Sai::log(
            "RTC",
            move || format!("audio output moved reason={raw_reason}, reopening"),
            SaiLogKind::Network,
            cx,
        );
        if let Err(error) = Self::apply_output(&preference) {
            Sai::log("RTC", move || format!("audio output reopen failed: {error}"), SaiLogKind::Network, cx);
        }
    }

    /// device lists come from the audio host off the main thread
    /// and the main thread only commits the newest answer
    fn refresh_routes(&mut self, cx: &mut Context<Self>) {
        self.routes_generation = self.routes_generation.wrapping_add(1);
        let generation = self.routes_generation;
        let preferred_input = self.preferred_input.clone();
        let listing = cx.background_spawn(async move { (rtc_media::input_devices(), rtc_media::output_devices()) });
        self.routes_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let (inputs, outputs) = listing.await;
            this.update(cx, |coordinator, cx| {
                if coordinator.routes_generation != generation {
                    return;
                }
                coordinator.routes_task = None;
                let devices = inputs.unwrap_or_default();
                let pickable = rtc_media::pickable_devices(&devices);
                coordinator.has_system_input_choice = pickable.len() != devices.len();
                coordinator.selected_input_id = rtc_media::resolve_device(&devices, preferred_input.as_deref())
                    .filter(|device| pickable.iter().any(|shown| shown.id == device.id))
                    .map(|device| device.id.clone());
                coordinator.available_inputs = pickable.into_iter().map(RiseRTCAudioInputRoute::from_device).collect();
                let outputs = outputs.unwrap_or_default();
                coordinator.available_outputs =
                    rtc_media::pickable_devices(&outputs).into_iter().map(RiseRTCAudioOutputRoute::from_device).collect();
                cx.notify();
            })
            .ok();
        }));
    }

    fn media_key(session: &RiseRTCMediaSessionKey) -> MediaSessionKey {
        MediaSessionKey {
            room_id: session.room_id.clone(),
            connection_id: session.connection_id.clone(),
            participant_identity: session.participant_identity.clone(),
        }
    }

    fn state_error(error: RtcUnavailable) -> RiseRTCStateError {
        match error {
            RtcUnavailable::NotConnected => RiseRTCStateError::MissingActiveRoom,
            RtcUnavailable::Invalid(code) => RiseRTCStateError::Validation(code.to_owned()),
            RtcUnavailable::Refused(code) => RiseRTCStateError::Remote(code.to_owned()),
            other @ (RtcUnavailable::NoRuntime(_) | RtcUnavailable::Device { .. }) => {
                RiseRTCStateError::Remote(other.to_string())
            }
        }
    }
}

impl Drop for RiseRTCLiveKitAudioCoordinator {
    fn drop(&mut self) {
        self.route_observer.take();
        self.route_task.take();
    }
}
