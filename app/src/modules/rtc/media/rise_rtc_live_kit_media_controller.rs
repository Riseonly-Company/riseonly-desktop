use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures::future::{AbortHandle, Abortable, BoxFuture, FutureExt};
use gpui::{PromptButton, PromptLevel};
use rise_canon::rise_key::rtc::ROLE;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::RiseIsolated;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_wire::rise_wire::CancellationError;
use rise_i18n::tr;
use rise_media::camera_capture::call_camera::{CallCameraCapture, CallCameraEndSink, CallFrameSink};
use rise_media::camera_capture::CameraCaptureError;
use rise_media::rtc_media::livekit_room::{self, RemoteVideoFrame, RtcEvent};
use rise_media::rtc_media::{
    self as rtc_media, ConnectRequest, DeviceDirection, MediaIntent, MediaSessionKey, RemoteParticipant, RemotePlayout, RtcUnavailable,
    SessionOutput, TrackSource, VideoCeiling, VideoShelf, CODE_DATA_PUBLISH_DISABLED,
};
use rise_media::texture::native_surface::{self, HostNv12Frame};
use rise_media::screen_capture::{
    self as screen_capture, CaptureOptions, CaptureSource, CaptureSourceKind, PickerParent, ScreenCaptureBackend,
    ScreenAudioBlock, ScreenAudioSink, ScreenCaptureEnd, ScreenCaptureError, ScreenEndSink, ScreenFrame, ScreenFrameSink, ScreenShareCapture,
};
use rise_platform::capture_permission::{self, CaptureDecision, CaptureKind};
use rise_platform::power::system_low_power_mode;
use rise_platform::process_info::{self, ThermalState};

use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCCameraPosition, RiseRTCFlexibleWireEnum, RiseRTCJoinCredentials,
    RiseRTCMediaDriving, RiseRTCMediaEvent, RiseRTCMediaIntent, RiseRTCMediaSessionKey, RiseRTCParticipant,
    RiseRTCParticipantRole, RiseRTCRoomMode, RiseRTCScreenShareQuality, RiseRTCStateError, RiseRTCVideoCeiling,
};
use crate::modules::rtc::engine::rise_rtc_domain::RiseRTCDomain;
use crate::modules::rtc::engine::rise_rtc_media_boundary::RiseRTCMediaRuntime;
use crate::modules::rtc::media::rise_rtc_audio_settings::{RiseRTCAudioProcessingSettings, RiseRTCAudioSettingsStore};
use crate::modules::rtc::media::rise_rtc_call_picture_in_picture::RiseRTCCallPictureInPictureController;
use crate::modules::rtc::media::rise_rtc_live_kit_audio_coordinator::RiseRTCLiveKitAudioCoordinator;
use crate::modules::rtc::media::rise_rtc_media_presentation::{
    Dimensions, RiseRTCAudioLevelSample, RiseRTCMediaPresentation, RiseRTCParticipantVideo, RiseRTCRoomVideo,
    RiseRTCVideoPublication, VideoTrack,
};
use crate::modules::rtc::shared::rise_rtc_device_refusal::RiseRTCDeviceRefusal;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RiseRTCLiveKitMediaControllerSessionPhase {
    Connecting,
    Connected,
    Reconnecting,
}

#[derive(Clone)]
struct RiseRTCLiveKitMediaControllerActiveSession {
    key: RiseRTCMediaSessionKey,
    generation: u64,
    phase: RiseRTCLiveKitMediaControllerSessionPhase,
    did_emit_connected: bool,
}

#[derive(Clone)]
struct RiseRTCLiveKitMediaControllerTerminalOutcome {
    key: RiseRTCMediaSessionKey,
    generation: u64,
    code: String,
}

#[derive(Clone)]
struct RiseRTCLiveKitMediaControllerPendingSpeakerPresentation {
    session: RiseRTCMediaSessionKey,
    samples: Vec<RiseRTCAudioLevelSample>,
}

/// what one screen share keeps between publish and stop
struct RiseRTCLiveKitMediaControllerScreenShare {
    session: RiseRTCMediaSessionKey,
    source: screen_capture::CaptureSource,
    capture: Option<ScreenShareCapture>,
}

struct RiseRTCLiveKitMediaControllerState {
    active_session: Option<RiseRTCLiveKitMediaControllerActiveSession>,
    remote_playout_muted: bool,
    listener_gains: HashMap<String, f64>,
    stream_gains: HashMap<String, f64>,
    intended_session_key: Option<RiseRTCMediaSessionKey>,
    terminal_outcome: Option<RiseRTCLiveKitMediaControllerTerminalOutcome>,
    lifecycle_generation: u64,
    pending_speaker_presentation: Option<RiseRTCLiveKitMediaControllerPendingSpeakerPresentation>,
    speaker_presentation_task: Option<AbortHandle>,
    screen_share: Option<RiseRTCLiveKitMediaControllerScreenShare>,
    screen_share_mutation_in_flight: bool,
    background_muted_camera: bool,
    camera_position: RiseRTCCameraPosition,
    camera_enabled: bool,
    camera_capture: Option<CallCameraCapture>,
    camera_capture_epoch: u64,
    camera_capture_watchdog: Option<AbortHandle>,
    remote_participants: Vec<RemoteParticipant>,
    remote_speaker_samples: Vec<RiseRTCAudioLevelSample>,
    local_input_level: f32,
    application_state_observation: Vec<RiseSubscription>,
    continuations: HashMap<u64, UnboundedSender<RiseRTCMediaEvent>>,
    next_continuation_id: u64,
}

/// one video track the call draws, held on the main thread while the room publishes it
struct RiseRTCLiveKitMediaControllerVideoTrack {
    track: Rc<VideoTrack>,
    user_id: Option<String>,
    source: TrackSource,
}

/// what replace_tracks reads off the room as the controller last saw it
#[derive(Clone)]
struct RiseRTCLiveKitMediaControllerRoomSnapshot {
    session: RiseRTCMediaSessionKey,
    remote_participants: Vec<RemoteParticipant>,
    local_camera_position: Option<RiseRTCCameraPosition>,
}

/// main thread side of the room video, VideoTrack isnt Send
/// so tracks live here and the actor only posts what changed
#[derive(Default)]
struct RiseRTCLiveKitMediaControllerVideoTracks {
    room: Option<RiseRTCLiveKitMediaControllerRoomSnapshot>,
    tracks: HashMap<String, RiseRTCLiveKitMediaControllerVideoTrack>,
}

thread_local! {
    static VIDEO_TRACKS: RefCell<RiseRTCLiveKitMediaControllerVideoTracks> =
        RefCell::new(RiseRTCLiveKitMediaControllerVideoTracks::default());
}

pub struct RiseRTCLiveKitMediaController {
    state: RiseIsolated<RiseRTCLiveKitMediaControllerState>,
}

pub type RiseRTCLiveKitMediaControllerCameraShelf = VideoShelf;
pub type RiseRTCLiveKitMediaControllerScreenShareShelf = VideoShelf;

static SHARED: OnceLock<Arc<RiseRTCLiveKitMediaController>> = OnceLock::new();
static NEXT_SCREEN_SHARE_STREAM: AtomicU64 = AtomicU64::new(1);

const LOCAL_CAMERA_TRACK_KEY: &str = "local.camera";
const LOCAL_SCREEN_TRACK_KEY: &str = "local.screen";
const CAMERA_CAPTURE_WATCHDOG_DELAY: Duration = Duration::from_secs(3);
const CONNECT_SETTLE_POLL: Duration = Duration::from_millis(25);
const SCREEN_SHARE_NO_FRAMES_CODE: &str = "rtc_screen_share_no_frames";

impl RiseRTCLiveKitMediaController {
    pub fn shared() -> Arc<Self> {
        SHARED
            .get_or_init(|| {
                Arc::new(Self {
                    state: RiseIsolated::new(RiseRTCLiveKitMediaControllerState {
                        active_session: None,
                        remote_playout_muted: false,
                        listener_gains: HashMap::new(),
                        stream_gains: HashMap::new(),
                        intended_session_key: None,
                        terminal_outcome: None,
                        lifecycle_generation: 1,
                        pending_speaker_presentation: None,
                        speaker_presentation_task: None,
                        screen_share: None,
                        screen_share_mutation_in_flight: false,
                        background_muted_camera: false,
                        camera_position: RiseRTCCameraPosition::Front,
                        camera_enabled: false,
                        camera_capture: None,
                        camera_capture_epoch: 0,
                        camera_capture_watchdog: None,
                        remote_participants: Vec::new(),
                        remote_speaker_samples: Vec::new(),
                        local_input_level: 0.0,
                        application_state_observation: Vec::new(),
                        continuations: HashMap::new(),
                        next_continuation_id: 0,
                    }),
                })
            })
            .clone()
    }

    pub async fn perform(&self, intent: RiseRTCMediaIntent) -> anyhow::Result<()> {
        match intent {
            RiseRTCMediaIntent::Connect(credentials) => self.connect(credentials).await?,
            RiseRTCMediaIntent::Disconnect { session } => self.disconnect(&session, true),
            RiseRTCMediaIntent::SetMicrophone { session, enabled } => self.set_microphone(&session, enabled).await?,
            RiseRTCMediaIntent::SetRemotePlayoutMuted { session, muted } => self.set_remote_playout_muted(&session, muted)?,
            RiseRTCMediaIntent::SetListenerVolumes { session, gains } => self.set_listener_volumes(&session, gains)?,
            RiseRTCMediaIntent::SetCamera { session, enabled, position } => {
                self.set_camera(&session, enabled, position).await?
            }
            RiseRTCMediaIntent::SetEncryptionKey { session, key } => self.set_encryption_key(&session, key)?,
            RiseRTCMediaIntent::SetScreenShare { session, enabled, include_device_audio, quality } => {
                self.set_screen_share(&session, enabled, include_device_audio, quality).await?
            }
            RiseRTCMediaIntent::SwitchCamera { session, position } => self.switch_camera(&session, position).await?,
            RiseRTCMediaIntent::SelectAudioInput { session, id } => {
                Self::on_main_result(move |cx| {
                    RiseRTCLiveKitAudioCoordinator::shared(cx)
                        .update(cx, |coordinator, cx| coordinator.select_input(id, &session, cx))
                        .map_err(anyhow::Error::from)
                })
                .await?
            }
            RiseRTCMediaIntent::SelectAudioOutput { session, id } => {
                Self::on_main_result(move |cx| {
                    RiseRTCLiveKitAudioCoordinator::shared(cx)
                        .update(cx, |coordinator, cx| coordinator.select_output(id, &session, cx))
                        .map_err(anyhow::Error::from)
                })
                .await?
            }
            RiseRTCMediaIntent::SendData { .. } => {
                return Err(RiseRTCStateError::Remote(CODE_DATA_PUBLISH_DISABLED.to_owned()).into());
            }
        }
        Ok(())
    }

    pub async fn events(&self) -> UnboundedReceiver<RiseRTCMediaEvent> {
        let (sender, receiver) = mpsc::unbounded();
        self.state.with(|state| {
            state.continuations.retain(|_, continuation| !continuation.is_closed());
            let id = state.next_continuation_id;
            state.next_continuation_id = state.next_continuation_id.wrapping_add(1);
            state.continuations.insert(id, sender);
        });
        receiver
    }

    async fn connect(&self, credentials: RiseRTCJoinCredentials) -> anyhow::Result<()> {
        if credentials.room_id.is_empty()
            || credentials.connection_id.is_empty()
            || credentials.participant_identity.is_empty()
            || credentials.server_url.is_empty()
            || credentials.access_token.is_empty()
        {
            return Err(RiseRTCStateError::Validation(rtc_media::CODE_INVALID_JOIN_CREDENTIALS.to_owned()).into());
        }
        if credentials.expires_at_ms <= Self::now_milliseconds() {
            return Err(RiseRTCStateError::Remote(rtc_media::CODE_JOIN_CREDENTIALS_EXPIRED.to_owned()).into());
        }
        tracing::info!(
            room = %credentials.room_id,
            endpoint = %rtc_media::sanitized_endpoint(&credentials.server_url),
            "rtc connect start"
        );
        let key = credentials.session_key();
        let already = self.state.with(|state| {
            let active = state.active_session.as_ref().filter(|active| active.key == key)?;
            if state.intended_session_key.as_ref() != Some(&key) {
                return None;
            }
            Some(active.phase)
        });
        match already {
            Some(RiseRTCLiveKitMediaControllerSessionPhase::Connected | RiseRTCLiveKitMediaControllerSessionPhase::Reconnecting) => {
                return Ok(());
            }
            Some(RiseRTCLiveKitMediaControllerSessionPhase::Connecting) => {
                return Err(RiseRTCStateError::Remote(rtc_media::CODE_CONNECT_IN_PROGRESS.to_owned()).into());
            }
            None => {}
        }

        Self::request_room_microphone_access_if_undetermined(&credentials).await;

        let request = ConnectRequest {
            room_id: credentials.room_id.clone(),
            connection_id: credentials.connection_id.clone(),
            server_url: credentials.server_url.clone(),
            access_token: credentials.access_token.clone(),
            participant_identity: credentials.participant_identity.clone(),
            video_ceiling: Self::seam_ceiling(credentials.video_ceiling),
            audio_bitrate_kbps: credentials.audio_bitrate_kbps,
            end_to_end_encrypted: credentials.room_mode == RiseRTCRoomMode::DirectCall,
            expires_at_ms: credentials.expires_at_ms,
        };
        let (connect_generation, (previous_screen_share, previous_camera)) = self.state.with(|state| {
            let generation = Self::advance_generation(state);
            state.terminal_outcome = None;
            state.intended_session_key = Some(key.clone());
            state.active_session = Some(RiseRTCLiveKitMediaControllerActiveSession {
                key: key.clone(),
                generation,
                phase: RiseRTCLiveKitMediaControllerSessionPhase::Connecting,
                did_emit_connected: false,
            });
            (generation, Self::clear_session_media(state))
        });
        Self::stop_screen_share_capture(previous_screen_share);
        Self::stop_camera_capture(previous_camera);
        self.start_application_state_observation();
        let activated = key.clone();
        Self::on_main(move |cx| {
            Self::begin_video_tracks(&activated);
            RiseRTCMediaPresentation::shared(cx).update(cx, |presentation, cx| presentation.activate(&activated, cx));
        });

        let prepare_credentials = credentials.clone();
        let prepared = Self::on_main_result(move |cx| {
            RiseRTCLiveKitAudioCoordinator::shared(cx)
                .update(cx, |coordinator, cx| coordinator.prepare(&prepare_credentials, cx))
                .map_err(anyhow::Error::from)
        })
        .await;
        let intended = self.state.with(|state| state.intended_session_key.as_ref() == Some(&key));
        if let Err(error) = prepared {
            self.abandon_connect(&key);
            if intended {
                Self::enqueue_audio_cleanup(key.clone());
            }
            return Err(error);
        }
        tracing::info!(
            room = %credentials.room_id,
            endpoint = %rtc_media::sanitized_endpoint(&credentials.server_url),
            "rtc audio ready"
        );
        if !intended {
            Self::enqueue_audio_cleanup(key.clone());
            return Err(CancellationError.into());
        }

        if let Err(reason) = rtc_media::perform(&MediaIntent::Connect(request)) {
            let failed = self.state.with(|state| {
                let ours = state.active_session.as_ref().is_some_and(|active| active.key == key);
                if ours {
                    state.active_session = None;
                    state.intended_session_key = None;
                    Self::advance_generation(state);
                }
                ours
            });
            if failed {
                tracing::warn!(room = %key.room_id, %reason, "rtc connect refused by the media layer");
            }
            Self::enqueue_audio_cleanup(key.clone());
            return Err(Self::state_error(reason).into());
        }
        self.await_connect_settled(&key, connect_generation).await
    }

    async fn await_connect_settled(&self, key: &RiseRTCMediaSessionKey, generation: u64) -> anyhow::Result<()> {
        loop {
            let phase = self.state.with(|state| {
                let intended = state.intended_session_key.as_ref() == Some(key);
                state
                    .active_session
                    .as_ref()
                    .filter(|active| intended && active.generation == generation && &active.key == key)
                    .map(|active| active.phase)
            });
            match phase {
                Some(RiseRTCLiveKitMediaControllerSessionPhase::Connecting) => {
                    tokio::time::sleep(CONNECT_SETTLE_POLL).await;
                }
                Some(_) => return Ok(()),
                None => return Err(CancellationError.into()),
            }
        }
    }

    fn abandon_connect(&self, key: &RiseRTCMediaSessionKey) {
        self.state.with(|state| {
            if state.active_session.as_ref().is_some_and(|active| &active.key == key) {
                state.active_session = None;
                state.intended_session_key = None;
                Self::advance_generation(state);
            }
        });
    }

    /// queues audio cleanup, the main hop runs jobs in order so releases stay behind earlier ones
    fn enqueue_audio_cleanup(session: RiseRTCMediaSessionKey) {
        Self::on_main(move |cx| {
            Self::end_video_tracks(&session);
            RiseRTCMediaPresentation::shared(cx)
                .update(cx, |presentation, cx| presentation.reset_session(&session, cx));
            RiseRTCLiveKitAudioCoordinator::shared(cx)
                .update(cx, |coordinator, cx| coordinator.release(Some(&session), cx));
        });
    }

    fn disconnect(&self, key: &RiseRTCMediaSessionKey, emit_event: bool) {
        let (owned, (screen_share, camera)) = self.state.with(|state| {
            let owned = state.intended_session_key.as_ref() == Some(key)
                || state.active_session.as_ref().is_some_and(|active| &active.key == key);
            if !owned {
                return (false, (None, None));
            }
            Self::advance_generation(state);
            state.intended_session_key = None;
            state.terminal_outcome = None;
            let was_active = state.active_session.as_ref().is_some_and(|active| &active.key == key);
            if !was_active {
                return (false, (None, None));
            }
            state.active_session = None;
            (true, Self::clear_session_media(state))
        });
        Self::stop_screen_share_capture(screen_share);
        Self::stop_camera_capture(camera);
        if let Err(reason) = rtc_media::perform(&MediaIntent::Disconnect(Self::seam_key(key))) {
            tracing::debug!(%reason, "rtc disconnect");
        }
        if owned {
            Self::enqueue_audio_cleanup(key.clone());
        }
        if owned && emit_event {
            self.yield_event(RiseRTCMediaEvent::Disconnected { session: key.clone(), reason: None });
        }
    }

    async fn set_camera(
        &self,
        session: &RiseRTCMediaSessionKey,
        enabled: bool,
        position: RiseRTCCameraPosition,
    ) -> anyhow::Result<()> {
        self.require_session(session)?;
        if enabled {
            Self::ensure_camera_access().await?;
        }
        self.require_session(session)?;
        let camera_id = if enabled { Self::camera_device_id(position).await } else { None };
        self.state.with(|state| {
            state.camera_position = position;
            state.camera_enabled = enabled;
            if !enabled {
                state.background_muted_camera = false;
            }
        });
        rtc_media::perform(&MediaIntent::SetCamera { session: Self::seam_key(session), enabled, camera_id })
            .map_err(Self::state_error)?;
        Ok(())
    }

    async fn switch_camera(&self, session: &RiseRTCMediaSessionKey, position: RiseRTCCameraPosition) -> anyhow::Result<()> {
        let enabled = self.state.with(|state| state.camera_enabled);
        self.require_session(session)?;
        if !enabled {
            self.state.with(|state| state.camera_position = position);
            return Ok(());
        }
        self.set_camera(session, true, position).await
    }

    fn start_application_state_observation(&self) {
        let center = RiseAppEvents::default_center();
        let observation = vec![
            center.observe(UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION, |_| {
                Self::post_background_camera_policy(true);
            }),
            center.observe(UIApplication::WILL_ENTER_FOREGROUND_NOTIFICATION, |_| {
                Self::post_background_camera_policy(false);
            }),
        ];
        self.state.with(|state| state.application_state_observation = observation);
    }

    /// notification arrives on the posting thread, the policy reads the pip window on the main thread
    fn post_background_camera_policy(background: bool) {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        RiseAccountRuntime::current().spawn(async move {
            let applied = hop
                .run(move |mut cx| {
                    async move {
                        RiseRTCLiveKitMediaController::shared().apply_background_camera_policy(background, &mut cx).await;
                        Ok(())
                    }
                    .boxed_local()
                })
                .await;
            if let Err(error) = applied {
                tracing::debug!(%error, "rtc background camera policy");
            }
        });
    }

    pub async fn apply_background_camera_policy(&self, background: bool, cx: &mut gpui::AsyncApp) {
        let Some(current) = self.state.with(|state| state.active_session.clone()) else {
            return;
        };
        if background && cx.update(|cx| RiseRTCCallPictureInPictureController::shared(cx).read(cx).is_active()) {
            return;
        }
        if background {
            let paused = self.state.with(|state| {
                if !state.camera_enabled || state.background_muted_camera {
                    return false;
                }
                state.background_muted_camera = true;
                true
            });
            if !paused {
                return;
            }
            let intent = MediaIntent::SetCamera { session: Self::seam_key(&current.key), enabled: false, camera_id: None };
            if let Err(reason) = rtc_media::perform(&intent) {
                tracing::debug!(%reason, "rtc background camera pause");
            }
            self.yield_event(RiseRTCMediaEvent::LocalCameraPaused { session: current.key.clone() });
        } else {
            let (resume, position) = self.state.with(|state| {
                let resume = state.background_muted_camera;
                state.background_muted_camera = false;
                (resume, state.camera_position)
            });
            if !resume {
                return;
            }
            if !self.is_current(&current) {
                return;
            }
            let camera_id = Self::camera_device_id(position).await;
            let intent = MediaIntent::SetCamera { session: Self::seam_key(&current.key), enabled: true, camera_id };
            if rtc_media::perform(&intent).is_err() {
                self.yield_event(RiseRTCMediaEvent::LocalCameraResumeRequested { session: current.key.clone() });
                return;
            }
            if !self.is_current(&current) {
                return;
            }
            self.yield_event(RiseRTCMediaEvent::LocalCameraResumed { session: current.key.clone() });
        }
    }

    fn set_encryption_key(&self, session: &RiseRTCMediaSessionKey, key: Option<String>) -> anyhow::Result<()> {
        self.require_session(session)?;
        rtc_media::perform(&MediaIntent::SetEncryptionKey { session: Self::seam_key(session), key })
            .map_err(Self::state_error)?;
        Ok(())
    }

    fn set_remote_playout_muted(&self, session: &RiseRTCMediaSessionKey, muted: bool) -> anyhow::Result<()> {
        self.require_session(session)?;
        self.state.with(|state| state.remote_playout_muted = muted);
        self.apply_remote_playout_volume(session)
    }

    fn set_listener_volumes(&self, session: &RiseRTCMediaSessionKey, gains: HashMap<String, f64>) -> anyhow::Result<()> {
        self.require_session(session)?;
        self.state.with(|state| state.listener_gains = gains);
        self.apply_remote_playout_volume(session)
    }

    /// sound of one persons screen share on this machine only, 1 plays it as sent and 2 doubles it
    pub fn set_stream_volume(&self, user_id: &str, gain: f64) {
        let session = self.state.with(|state| {
            if (gain - 1.0).abs() < f64::EPSILON {
                state.stream_gains.remove(user_id);
            } else {
                state.stream_gains.insert(user_id.to_owned(), gain);
            }
            state.active_session.as_ref().map(|active| active.key.clone())
        });
        if let Some(session) = session
            && let Err(error) = self.apply_remote_playout_volume(&session)
        {
            tracing::debug!(%error, "rtc stream volume not applied");
        }
    }

    pub fn stream_volume(&self, user_id: &str) -> f64 {
        self.state.with(|state| state.stream_gains.get(user_id).copied().unwrap_or(1.0))
    }

    fn apply_remote_playout_volume(&self, session: &RiseRTCMediaSessionKey) -> anyhow::Result<()> {
        let playout = self.state.with(|state| RemotePlayout {
            deafened: state.remote_playout_muted,
            gains: state.listener_gains.clone(),
            stream_gains: state.stream_gains.clone(),
        });
        rtc_media::perform(&MediaIntent::SetRemotePlayout { session: Self::seam_key(session), playout })
            .map_err(Self::state_error)?;
        Ok(())
    }

    async fn set_microphone(&self, session: &RiseRTCMediaSessionKey, enabled: bool) -> anyhow::Result<()> {
        self.require_session(session)?;
        if enabled {
            Self::ensure_microphone_access().await?;
        }
        self.require_session(session)?;
        rtc_media::perform(&MediaIntent::SetMicrophone { session: Self::seam_key(session), enabled })
            .map_err(Self::state_error)?;
        Ok(())
    }

    async fn request_room_microphone_access_if_undetermined(credentials: &RiseRTCJoinCredentials) {
        if credentials.room_mode != RiseRTCRoomMode::Voice && credentials.room_mode != RiseRTCRoomMode::GroupVoice {
            return;
        }
        if let Err(reason) = capture_permission::ensure_access(CaptureKind::Microphone).await {
            tracing::debug!(%reason, "no microphone gate on this host");
        }
    }

    async fn ensure_microphone_access() -> Result<(), RiseRTCStateError> {
        match capture_permission::ensure_access(CaptureKind::Microphone).await {
            Ok(CaptureDecision::Granted) => Ok(()),
            Ok(CaptureDecision::Declined | CaptureDecision::Blocked) => {
                Err(RiseRTCStateError::Remote(RiseRTCDeviceRefusal::MICROPHONE.to_owned()))
            }
            Err(reason) => {
                tracing::debug!(%reason, "no microphone gate on this host");
                Ok(())
            }
        }
    }

    /// a refused camera must not reach livekit, the capturer would
    /// start anyway and send black frames with nothing saying why
    pub async fn ensure_camera_access() -> Result<(), RiseRTCStateError> {
        match capture_permission::ensure_access(CaptureKind::Camera).await {
            Ok(CaptureDecision::Granted) => Ok(()),
            Ok(CaptureDecision::Declined | CaptureDecision::Blocked) => {
                Err(RiseRTCStateError::Remote(RiseRTCDeviceRefusal::CAMERA.to_owned()))
            }
            Err(reason) => {
                tracing::debug!(%reason, "no camera gate on this host");
                Ok(())
            }
        }
    }

    async fn set_screen_share(
        &self,
        session: &RiseRTCMediaSessionKey,
        enabled: bool,
        include_device_audio: bool,
        quality: RiseRTCScreenShareQuality,
    ) -> anyhow::Result<()> {
        self.require_session(session)?;
        let claimed = self.state.with(|state| {
            if state.screen_share_mutation_in_flight {
                return false;
            }
            state.screen_share_mutation_in_flight = true;
            true
        });
        if !claimed {
            return Err(RiseRTCStateError::Remote(rtc_media::CODE_CONNECT_IN_PROGRESS.to_owned()).into());
        }
        let result = self.apply_screen_share(session, enabled, include_device_audio, quality).await;
        self.state.with(|state| state.screen_share_mutation_in_flight = false);
        result
    }

    async fn apply_screen_share(
        &self,
        session: &RiseRTCMediaSessionKey,
        enabled: bool,
        include_device_audio: bool,
        quality: RiseRTCScreenShareQuality,
    ) -> anyhow::Result<()> {
        if !enabled {
            let previous = self.state.with(|state| state.screen_share.take());
            Self::stop_screen_share_capture(previous);
            rtc_media::perform(&MediaIntent::SetScreenShare {
                session: Self::seam_key(session),
                enabled: false,
                include_system_audio: false,
                quality: Self::seam_quality(quality),
                display: (0, 0),
            })
            .map_err(Self::state_error)?;
            return Ok(());
        }
        let already = self.state.with(|state| state.screen_share.as_ref().is_some_and(|share| &share.session == session));
        if already {
            return Ok(());
        }
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Media, || "screen share: system picker requested".to_owned());
        let picked = RiseAccountRuntime::current().spawn_blocking(Self::pick_screen_share_source).await;
        let source = match picked {
            Ok(Ok(source)) => source,
            Ok(Err(ScreenCaptureError::PickerUnavailable)) => match self.choose_screen_share_source_in_app().await {
                Ok(Some(source)) => source,
                Ok(None) => return Err(CancellationError.into()),
                Err(error) => {
                    tracing::warn!(%error, "rtc screen share sources unavailable");
                    return Err(RiseRTCStateError::Remote(rtc_media::CODE_CONNECT_FAILED.to_owned()).into());
                }
            },
            Ok(Err(ScreenCaptureError::PortalMissing(reason))) => {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: {reason}"));
                return Err(RiseRTCStateError::Remote(RiseRTCDomain::SCREEN_SHARE_NO_PORTAL_CODE.to_owned()).into());
            }
            Ok(Err(error)) if error.is_user_choice() => {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Media, || "screen share: picker ended with no source".to_owned());
                return Err(CancellationError.into());
            }
            Ok(Err(error)) => {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: no source: {error}"));
                tracing::warn!(%error, "rtc screen share source unavailable");
                return Err(RiseRTCStateError::Remote(rtc_media::CODE_CONNECT_FAILED.to_owned()).into());
            }
            Err(error) => {
                tracing::warn!(%error, "rtc screen share picker task ended");
                return Err(RiseRTCStateError::Remote(rtc_media::CODE_CONNECT_FAILED.to_owned()).into());
            }
        };
        self.require_session(session)?;
        let display = source.pixel_size();
        self.state.with(|state| {
            state.screen_share = Some(RiseRTCLiveKitMediaControllerScreenShare {
                session: session.clone(),
                source,
                capture: None,
            });
        });
        let intent = MediaIntent::SetScreenShare {
            session: Self::seam_key(session),
            enabled: true,
            include_system_audio: include_device_audio,
            quality: Self::seam_quality(quality),
            display,
        };
        if let Err(reason) = rtc_media::perform(&intent) {
            let previous = self.state.with(|state| state.screen_share.take());
            Self::stop_screen_share_capture(previous);
            return Err(Self::state_error(reason).into());
        }
        Ok(())
    }

    fn pick_screen_share_source() -> Result<CaptureSource, ScreenCaptureError> {
        screen_capture::native().pick_source(&PickerParent::default())
    }

    fn list_screen_share_sources() -> Result<Vec<CaptureSource>, ScreenCaptureError> {
        screen_capture::native().sources()
    }

    async fn choose_screen_share_source_in_app(&self) -> anyhow::Result<Option<CaptureSource>> {
        let mut sources = RiseAccountRuntime::current().spawn_blocking(Self::list_screen_share_sources).await??;
        sources.sort_by_key(|source| source.kind != CaptureSourceKind::Display);
        if sources.is_empty() {
            return Err(ScreenCaptureError::SourceGone.into());
        }
        let labels: Vec<String> = sources
            .iter()
            .enumerate()
            .map(|(index, source)| Self::screen_share_source_label(source, index))
            .collect();
        let receiver = Self::on_main_result(move |cx| {
            let Some(host) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
                return Err(ScreenCaptureError::PickerUnavailable.into());
            };
            let mut buttons: Vec<PromptButton> = labels.into_iter().map(PromptButton::new).collect();
            buttons.push(PromptButton::cancel(tr("common_cancel")));
            host.update(cx, |_, window, cx| {
                window.prompt(PromptLevel::Info, &tr("perm_share_screen"), None, &buttons, cx)
            })
        })
        .await?;
        Ok(receiver.await.ok().and_then(|index| sources.get(index).cloned()))
    }

    fn screen_share_source_label(source: &CaptureSource, index: usize) -> String {
        if source.name.is_empty() {
            format!("{} {}", tr("rtc_screen_share"), index + 1)
        } else {
            source.name.clone()
        }
    }

    fn start_screen_share_capture(&self, shelf: VideoShelf, include_system_audio: bool) {
        let source = self.state.with(|state| state.screen_share.as_ref().map(|share| share.source.clone()));
        let Some(source) = source else {
            return;
        };
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Media, || "screen share: track published".to_owned());
        let stream_id = NEXT_SCREEN_SHARE_STREAM.fetch_add(1, Ordering::Relaxed);
        let generation = Arc::new(AtomicU64::new(0));
        let delivered = Arc::new(AtomicBool::new(false));
        let frames: ScreenFrameSink = Arc::new(move |frame: ScreenFrame| {
            let next = generation.fetch_add(1, Ordering::Relaxed) + 1;
            let dimensions = Dimensions { width: frame.width, height: frame.height };
            match livekit_room::push_screen_frame(frame, stream_id, next) {
                Ok(()) => {
                    if !delivered.swap(true, Ordering::Relaxed) {
                        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Media, || {
                            "screen share: first frame arrived".to_owned()
                        });
                        Self::on_main(move |cx| Self::show_local_screen_track(dimensions, cx));
                    }
                }
                Err(reason) => tracing::debug!(%reason, "rtc screen frame dropped"),
            }
        });
        let ended: ScreenEndSink = Arc::new(|end: ScreenCaptureEnd| {
            RiseAccountRuntime::current().spawn(async move {
                RiseRTCLiveKitMediaController::shared().handle_screen_capture_end(end).await;
            });
        });
        let backend = screen_capture::native();
        let options = CaptureOptions::from_shelf(&shelf);
        let capture = if include_system_audio {
            let audio: ScreenAudioSink = Arc::new(|block: ScreenAudioBlock| {
                if let Err(reason) = livekit_room::push_screen_audio(block.samples, block.sample_rate, block.channels) {
                    tracing::debug!(%reason, "rtc screen audio block dropped");
                }
            });
            ScreenShareCapture::start_with_system_audio(&backend, &source, options, frames, audio, ended)
        } else {
            ScreenShareCapture::start(&backend, &source, options, frames, ended)
        };
        match capture {
            Ok(capture) => {
                let orphan = self.state.with(|state| match state.screen_share.as_mut() {
                    Some(share) => share.capture.replace(capture),
                    None => Some(capture),
                });
                if let Some(mut orphan) = orphan {
                    orphan.stop();
                }
            }
            Err(error) => {
                tracing::warn!(%error, "rtc screen capture did not start");
            }
        }
    }

    fn end_lost_screen_share(&self, code: &str) {
        let Some(share) = self.state.with(|state| state.screen_share.take()) else {
            return;
        };
        let session = share.session.clone();
        Self::stop_screen_share_capture(Some(share));
        self.refresh_tracks(&session);
        self.yield_event(RiseRTCMediaEvent::LocalScreenShareEnded { session: session.clone() });
        self.yield_event(RiseRTCMediaEvent::Failure { session: Some(session), code: code.to_owned() });
    }

    async fn handle_screen_capture_end(&self, end: ScreenCaptureEnd) {
        let share = self.state.with(|state| state.screen_share.take());
        let Some(share) = share else {
            return;
        };
        tracing::info!(?end, "rtc screen capture ended by the system");
        let session = share.session.clone();
        Self::stop_screen_share_capture(Some(share));
        let intent = MediaIntent::SetScreenShare {
            session: Self::seam_key(&session),
            enabled: false,
            include_system_audio: false,
            quality: rtc_media::ScreenShareQuality::Standard,
            display: (0, 0),
        };
        if let Err(reason) = rtc_media::perform(&intent) {
            tracing::debug!(%reason, "rtc screen share unpublish after capture end");
        }
        if self.state.with(|state| state.active_session.as_ref().is_some_and(|active| active.key == session)) {
            self.yield_event(RiseRTCMediaEvent::LocalScreenShareEnded { session });
        }
    }

    fn stop_screen_share_capture(share: Option<RiseRTCLiveKitMediaControllerScreenShare>) {
        if let Some(mut capture) = share.and_then(|share| share.capture) {
            capture.stop();
            Self::on_main(|cx| Self::remove_video_track(LOCAL_SCREEN_TRACK_KEY, cx));
        }
    }

    fn show_local_screen_track(dimensions: Dimensions, cx: &mut gpui::App) {
        let created = VIDEO_TRACKS.with_borrow_mut(|tracks| {
            if tracks.room.is_none() || tracks.tracks.contains_key(LOCAL_SCREEN_TRACK_KEY) {
                return false;
            }
            let track = VideoTrack::new(LOCAL_SCREEN_TRACK_KEY.to_owned(), TrackSource::ScreenShare, Some(dimensions));
            tracks.tracks.insert(
                LOCAL_SCREEN_TRACK_KEY.to_owned(),
                RiseRTCLiveKitMediaControllerVideoTrack { track, user_id: None, source: TrackSource::ScreenShare },
            );
            true
        });
        if created {
            Self::present_room(cx);
        }
    }

    pub fn is_publish_under_pressure() -> bool {
        system_low_power_mode()
            || matches!(process_info::thermal_state(), ThermalState::Serious | ThermalState::Critical)
    }

    /// tallest camera preset under the server ceiling
    pub fn camera_shelf(ceiling: RiseRTCVideoCeiling) -> RiseRTCLiveKitMediaControllerCameraShelf {
        rtc_media::camera_shelf(Self::seam_ceiling(ceiling))
    }

    /// screen share shelf by quality, ceiling and the native pixel size of the captured display
    pub fn screen_share_shelf(
        quality: RiseRTCScreenShareQuality,
        ceiling: RiseRTCVideoCeiling,
        display: (u32, u32),
    ) -> RiseRTCLiveKitMediaControllerScreenShareShelf {
        rtc_media::screen_share_shelf(Self::seam_quality(quality), Self::seam_ceiling(ceiling), display)
    }

    pub async fn apply_audio_processing(
        &self,
        settings: RiseRTCAudioProcessingSettings,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<()> {
        let applied = cx.update(|cx| RiseRTCAudioSettingsStore::shared(cx).read(cx).apply_to_audio_engine());
        applied.map_err(Self::state_error)?;
        let _ = settings;
        Ok(())
    }

    async fn receive(&self, event: RtcEvent) {
        match event {
            RtcEvent::Session(output) => self.receive_session_output(output).await,
            RtcEvent::InputLevel(level) => {
                let Some(current) = self.state.with(|state| {
                    state.local_input_level = level.clamp(0.0, 1.0);
                    state.active_session.clone()
                }) else {
                    return;
                };
                self.schedule_speaker_presentation(&current);
            }
            RtcEvent::CaptureRefused { kind, decision } => {
                let Some(current) = self.state.with(|state| state.active_session.clone()) else {
                    return;
                };
                tracing::info!(?kind, ?decision, "rtc capture refused");
                let code = match kind {
                    CaptureKind::Camera => RiseRTCDeviceRefusal::CAMERA,
                    CaptureKind::Microphone => RiseRTCDeviceRefusal::MICROPHONE,
                };
                self.yield_event(RiseRTCMediaEvent::Failure { session: Some(current.key), code: code.to_owned() });
            }
            RtcEvent::LocalVideoWanted { source: TrackSource::ScreenShare, include_system_audio, shelf, .. } => {
                self.start_screen_share_capture(shelf, include_system_audio);
            }
            RtcEvent::LocalVideoWanted { source: TrackSource::Camera, device_id, shelf, .. } => {
                self.start_camera_capture(device_id, shelf).await;
            }
            RtcEvent::LocalVideoWanted { .. } => {}
            RtcEvent::LocalVideoStopped { source: TrackSource::Camera } => {
                let camera = self.state.with(Self::take_camera_capture);
                Self::stop_camera_capture(camera);
                Self::on_main(|cx| Self::remove_video_track(LOCAL_CAMERA_TRACK_KEY, cx));
            }
            RtcEvent::LocalVideoStopped { source: TrackSource::ScreenShare } => {
                let capture = self.state.with(|state| state.screen_share.as_mut().and_then(|share| share.capture.take()));
                if let Some(mut capture) = capture {
                    capture.stop();
                }
                Self::on_main(|cx| Self::remove_video_track(LOCAL_SCREEN_TRACK_KEY, cx));
            }
            RtcEvent::LocalVideoStopped { .. } => {}
            RtcEvent::ScreenShareDeadPipe => {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || {
                    "screen share: published but no frames arrived".to_owned()
                });
                tracing::warn!("screen share published but produced no frames; tearing down");
                self.end_lost_screen_share(SCREEN_SHARE_NO_FRAMES_CODE);
            }
            RtcEvent::ScreenSharePublishFailed { code } => {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: publish failed: {code}"));
                self.end_lost_screen_share(code);
            }
            RtcEvent::AudioDeviceFailed { direction, reason } => {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("call audio {direction:?} failed: {reason}"));
                let Some(current) = self.state.with(|state| state.active_session.clone()) else {
                    return;
                };
                let session = current.key;
                self.yield_event(match direction {
                    DeviceDirection::Input => RiseRTCMediaEvent::LocalMicrophoneLost { session },
                    DeviceDirection::Output => RiseRTCMediaEvent::LocalPlayoutLost { session },
                });
            }
            RtcEvent::RemoteVideo(frame) => {
                let dimensions = Self::frame_dimensions(&frame);
                Self::on_main(move |cx| Self::render_video_frame(frame, dimensions, cx));
            }
            RtcEvent::RemoteVideoEnded { track_key } => {
                Self::on_main(move |cx| Self::remove_video_track(&track_key, cx));
            }
        }
    }

    async fn receive_session_output(&self, output: SessionOutput) {
        let Some(current) = self.state.with(|state| {
            let active = state.active_session.clone()?;
            (state.intended_session_key.as_ref() == Some(&active.key)).then_some(active)
        }) else {
            if let SessionOutput::ParticipantJoined { user_id, .. } = &output {
                tracing::warn!(target: "riseonly::X1882", %user_id, "rtc participant joined dropped: no intended active session");
            }
            return;
        };
        let key = current.key.clone();
        match output {
            SessionOutput::Connected => {
                let emitted = self.state.with(|state| {
                    let Some(active) = state.active_session.as_mut().filter(|active| active.generation == current.generation)
                    else {
                        return false;
                    };
                    if active.did_emit_connected {
                        return false;
                    }
                    active.did_emit_connected = true;
                    active.phase = RiseRTCLiveKitMediaControllerSessionPhase::Connected;
                    true
                });
                if emitted {
                    self.refresh_tracks(&key);
                    self.yield_event(RiseRTCMediaEvent::Connected { session: key });
                }
            }
            SessionOutput::Reconnecting => {
                self.set_phase(&current, RiseRTCLiveKitMediaControllerSessionPhase::Reconnecting);
                self.yield_event(RiseRTCMediaEvent::Reconnecting { session: key });
            }
            SessionOutput::Reconnected => {
                self.set_phase(&current, RiseRTCLiveKitMediaControllerSessionPhase::Connected);
                if let Err(error) = self.apply_remote_playout_volume(&key) {
                    tracing::debug!(%error, "rtc playout after reconnect");
                }
                self.refresh_tracks(&key);
                if !self.is_current(&current) {
                    return;
                }
                self.yield_event(RiseRTCMediaEvent::Reconnected { session: key });
            }
            SessionOutput::ConnectFailureNoted(code) => {
                self.state.with(|state| {
                    state.terminal_outcome = Some(RiseRTCLiveKitMediaControllerTerminalOutcome {
                        key: key.clone(),
                        generation: current.generation,
                        code: code.to_owned(),
                    });
                });
            }
            SessionOutput::Ended { code, established } => {
                let code = self.state.with(|state| {
                    state
                        .terminal_outcome
                        .take()
                        .filter(|outcome| outcome.key == key && outcome.generation == current.generation)
                        .map_or_else(|| code.to_owned(), |outcome| outcome.code)
                });
                self.terminate(&current, code, established);
            }
            SessionOutput::ParticipantJoined { user_id, participant } => {
                tracing::warn!(target: "riseonly::X1882", %user_id, room = %key.room_id, "rtc participant joined");
                let model = Self::participant(user_id.clone(), &participant);
                self.state.with(|state| {
                    state.remote_participants.retain(|known| known.user_id().as_deref() != Some(user_id.as_str()));
                    state.remote_participants.push(participant);
                });
                self.refresh_tracks(&key);
                self.yield_event(RiseRTCMediaEvent::ParticipantJoined { session: key, participant: model });
            }
            SessionOutput::ParticipantLeft { user_id } => {
                self.state.with(|state| {
                    state.remote_participants.retain(|known| known.user_id().as_deref() != Some(user_id.as_str()));
                });
                self.refresh_tracks(&key);
                self.yield_event(RiseRTCMediaEvent::ParticipantLeft {
                    session: key,
                    user_id,
                    occurred_at_ms: Self::now_milliseconds(),
                });
            }
            SessionOutput::Data { topic, payload } => {
                self.yield_event(RiseRTCMediaEvent::Data { session: key, topic, payload });
            }
            SessionOutput::TracksChanged => {
                if let Err(error) = self.apply_remote_playout_volume(&key) {
                    tracing::debug!(%error, "rtc playout after track change");
                }
                self.refresh_tracks(&key);
            }
            SessionOutput::Speakers(samples) => self.refresh_speaking(samples, &current),
            SessionOutput::CheckScreenShareTeardown => {
                let had_share = self.state.with(|state| {
                    if state.screen_share_mutation_in_flight {
                        return None;
                    }
                    state.screen_share.take()
                });
                if let Some(share) = had_share {
                    Self::stop_screen_share_capture(Some(share));
                    self.yield_event(RiseRTCMediaEvent::LocalScreenShareEnded { session: key });
                }
            }
            SessionOutput::Failure(code) => {
                self.yield_event(RiseRTCMediaEvent::Failure { session: Some(key), code: code.to_owned() });
            }
        }
    }

    fn set_phase(&self, current: &RiseRTCLiveKitMediaControllerActiveSession, phase: RiseRTCLiveKitMediaControllerSessionPhase) {
        self.state.with(|state| {
            if let Some(active) = state.active_session.as_mut().filter(|active| active.generation == current.generation) {
                active.phase = phase;
            }
        });
    }

    fn refresh_tracks(&self, session: &RiseRTCMediaSessionKey) {
        let snapshot = self.state.with(|state| RiseRTCLiveKitMediaControllerRoomSnapshot {
            session: session.clone(),
            remote_participants: state.remote_participants.clone(),
            local_camera_position: state.camera_enabled.then_some(state.camera_position),
        });
        Self::on_main(move |cx| Self::replace_room(snapshot, cx));
    }

    fn refresh_speaking(&self, samples: Vec<(String, f32)>, session: &RiseRTCLiveKitMediaControllerActiveSession) {
        let local_identity = &session.key.participant_identity;
        let local_user_id = Self::local_speaker_user_id(local_identity);
        let samples = samples
            .into_iter()
            .filter(|(user_id, _)| *user_id != local_user_id && user_id != local_identity)
            .map(|(user_id, level)| RiseRTCAudioLevelSample { user_id, level })
            .collect();
        self.state.with(|state| state.remote_speaker_samples = samples);
        self.schedule_speaker_presentation(session);
    }

    fn speaker_samples(state: &RiseRTCLiveKitMediaControllerState, local_user_id: String) -> Vec<RiseRTCAudioLevelSample> {
        let mut samples = state.remote_speaker_samples.clone();
        if state.local_input_level > 0.0 {
            samples.push(RiseRTCAudioLevelSample {
                user_id: local_user_id,
                level: state.local_input_level,
            });
        }
        samples.sort_by(|left, right| right.level.total_cmp(&left.level));
        samples.truncate(16);
        samples
    }

    /// user behind this devices participant, read off the sdk participant in active speakers
    fn local_speaker_user_id(identity: &str) -> String {
        let local = RiseRTCParticipantVideo {
            identity: identity.to_owned(),
            metadata: livekit_room::local_metadata(identity).unwrap_or_default(),
            video_tracks: Vec::new(),
        };
        RiseRTCMediaPresentation::user_id(&local).unwrap_or_else(|| identity.to_owned())
    }

    fn schedule_speaker_presentation(&self, session: &RiseRTCLiveKitMediaControllerActiveSession) {
        let local_user_id = Self::local_speaker_user_id(&session.key.participant_identity);
        let schedule = self.state.with(|state| {
            let samples = Self::speaker_samples(state, local_user_id);
            state.pending_speaker_presentation =
                Some(RiseRTCLiveKitMediaControllerPendingSpeakerPresentation { session: session.key.clone(), samples });
            if state.speaker_presentation_task.is_some() {
                return None;
            }
            let (handle, registration) = AbortHandle::new_pair();
            state.speaker_presentation_task = Some(handle);
            Some(registration)
        });
        let Some(registration) = schedule else {
            return;
        };
        let interval = rtc_media::speaker_flush_interval(system_low_power_mode());
        let flush = Abortable::new(
            async move {
                tokio::time::sleep(interval).await;
                RiseRTCLiveKitMediaController::shared().flush_speaker_presentation();
            },
            registration,
        );
        RiseAccountRuntime::current().spawn(flush.map(|_| ()));
    }

    fn flush_speaker_presentation(&self) {
        let pending = self.state.with(|state| {
            state.speaker_presentation_task = None;
            let pending = state.pending_speaker_presentation.take()?;
            let current = state.active_session.as_ref().is_some_and(|active| active.key == pending.session);
            current.then_some(pending)
        });
        let Some(pending) = pending else {
            return;
        };
        Self::on_main(move |cx| {
            RiseRTCMediaPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.replace_speaking_samples(&pending.samples, &pending.session, cx)
            });
        });
    }

    fn cancel_speaker_presentation(state: &mut RiseRTCLiveKitMediaControllerState) {
        state.pending_speaker_presentation = None;
        if let Some(task) = state.speaker_presentation_task.take() {
            task.abort();
        }
    }

    fn terminate(&self, session: &RiseRTCLiveKitMediaControllerActiveSession, code: String, established: bool) {
        let screen_share = self.state.with(|state| {
            let current = state.active_session.as_ref().is_some_and(|active| active.generation == session.generation);
            if !current {
                return Err(());
            }
            Self::advance_generation(state);
            state.intended_session_key = None;
            state.active_session = None;
            Ok(Self::clear_session_media(state))
        });
        let Ok((screen_share, camera)) = screen_share else {
            return;
        };
        Self::stop_screen_share_capture(screen_share);
        Self::stop_camera_capture(camera);
        Self::enqueue_audio_cleanup(session.key.clone());
        if established {
            self.yield_event(RiseRTCMediaEvent::Disconnected { session: session.key.clone(), reason: Some(code) });
        } else {
            self.yield_event(RiseRTCMediaEvent::Failure { session: Some(session.key.clone()), code });
        }
    }

    fn require_session(&self, key: &RiseRTCMediaSessionKey) -> Result<RiseRTCLiveKitMediaControllerActiveSession, RiseRTCStateError> {
        self.state.with(|state| {
            let active = state.active_session.as_ref().filter(|active| {
                &active.key == key
                    && state.intended_session_key.as_ref() == Some(key)
                    && active.phase != RiseRTCLiveKitMediaControllerSessionPhase::Connecting
            });
            active.cloned().ok_or(RiseRTCStateError::MissingActiveRoom)
        })
    }

    fn is_current(&self, session: &RiseRTCLiveKitMediaControllerActiveSession) -> bool {
        self.state.with(|state| {
            state.active_session.as_ref().is_some_and(|active| active.generation == session.generation)
                && state.intended_session_key.as_ref() == Some(&session.key)
        })
    }

    fn advance_generation(state: &mut RiseRTCLiveKitMediaControllerState) -> u64 {
        state.lifecycle_generation = state.lifecycle_generation.wrapping_add(1);
        state.lifecycle_generation
    }

    fn yield_event(&self, event: RiseRTCMediaEvent) {
        self.state.with(|state| {
            state.continuations.retain(|_, continuation| continuation.unbounded_send(event.clone()).is_ok());
        });
    }

    fn participant(user_id: String, participant: &RemoteParticipant) -> RiseRTCParticipant {
        let now = Self::now_milliseconds();
        let role = serde_json::from_str::<serde_json::Value>(&participant.metadata)
            .ok()
            .and_then(|object| object.get(ROLE).cloned())
            .map_or(RiseRTCParticipantRole::Unspecified, |value| match value {
                serde_json::Value::String(text) => RiseRTCParticipantRole::parse(&text),
                serde_json::Value::Number(number) => {
                    number.as_i64().map_or(RiseRTCParticipantRole::Unspecified, RiseRTCParticipantRole::parse_int)
                }
                _ => RiseRTCParticipantRole::Unspecified,
            });
        RiseRTCParticipant {
            user_id,
            role,
            connected: true,
            microphone_enabled: participant.microphone_enabled,
            camera_enabled: participant.camera_enabled,
            screen_share_enabled: participant.screen_share_enabled,
            server_muted: false,
            server_deafened: false,
            server_camera_blocked: false,
            server_screen_share_blocked: false,
            deafened: false,
            hand_raised: false,
            joined_at_ms: participant.joined_at_ms.unwrap_or(now),
            updated_at_ms: now,
            lease_expires_at_ms: now + 120_000,
            order_sequence: now,
            profile: None,
        }
    }

    /// camera device for a position, the desktop picks the
    /// built in camera for front and the next one for back
    async fn camera_device_id(position: RiseRTCCameraPosition) -> Option<String> {
        let devices = RiseAccountRuntime::current().spawn_blocking(rise_media::camera_capture::devices).await;
        let devices = match devices {
            Ok(Ok(devices)) => devices,
            Ok(Err(error)) => {
                tracing::debug!(%error, "rtc camera list");
                return None;
            }
            Err(error) => {
                tracing::debug!(%error, "rtc camera list task ended");
                return None;
            }
        };
        let front = position == RiseRTCCameraPosition::Front;
        rise_media::camera_capture::pick_device(&devices, front).map(|device| device.id.clone())
    }

    /// runs work on the main thread from the controller tasks and returns the result
    async fn on_main_result<R: Send + 'static>(
        work: impl FnOnce(&mut gpui::App) -> anyhow::Result<R> + Send + 'static,
    ) -> anyhow::Result<R> {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return Err(CancellationError.into());
        };
        hop.run(move |cx| async move { cx.update(work) }.boxed_local()).await
    }

    fn on_main(work: impl FnOnce(&mut gpui::App) + Send + 'static) {
        match RiseEngineMainHop::installed() {
            Some(hop) => hop.post(work),
            None => tracing::debug!("rtc presentation update before the main hop started"),
        }
    }

    fn seam_key(key: &RiseRTCMediaSessionKey) -> MediaSessionKey {
        MediaSessionKey {
            room_id: key.room_id.clone(),
            connection_id: key.connection_id.clone(),
            participant_identity: key.participant_identity.clone(),
        }
    }

    fn seam_ceiling(ceiling: RiseRTCVideoCeiling) -> VideoCeiling {
        let ceiling = ceiling.under_pressure(Self::is_publish_under_pressure());
        let height = u32::try_from(ceiling.max_height).ok();
        let fps = u32::try_from(ceiling.max_fps).ok();
        VideoCeiling::from_wire(height, fps)
    }

    fn seam_quality(quality: RiseRTCScreenShareQuality) -> rtc_media::ScreenShareQuality {
        match quality {
            RiseRTCScreenShareQuality::Balanced => rtc_media::ScreenShareQuality::Standard,
            RiseRTCScreenShareQuality::High => rtc_media::ScreenShareQuality::High,
        }
    }

    fn state_error(reason: RtcUnavailable) -> RiseRTCStateError {
        match reason {
            RtcUnavailable::NotConnected => RiseRTCStateError::MissingActiveRoom,
            RtcUnavailable::Invalid(code) => RiseRTCStateError::Validation(code.to_owned()),
            RtcUnavailable::Refused(code) => RiseRTCStateError::Remote(code.to_owned()),
            RtcUnavailable::Device { .. } => RiseRTCStateError::Remote(rtc_media::CODE_AUDIO_FAILED.to_owned()),
            RtcUnavailable::NoRuntime(_) => RiseRTCStateError::Remote(rtc_media::CODE_CONNECT_FAILED.to_owned()),
        }
    }

    /// everything one media session kept beside the room, playout and background latch and camera
    /// speaker throttle and participants returns the captures to stop outside the actor
    fn clear_session_media(
        state: &mut RiseRTCLiveKitMediaControllerState,
    ) -> (Option<RiseRTCLiveKitMediaControllerScreenShare>, Option<CallCameraCapture>) {
        state.remote_playout_muted = false;
        state.listener_gains.clear();
        state.background_muted_camera = false;
        state.camera_enabled = false;
        state.remote_participants.clear();
        state.remote_speaker_samples.clear();
        state.local_input_level = 0.0;
        state.application_state_observation.clear();
        Self::cancel_speaker_presentation(state);
        let camera = Self::take_camera_capture(state);
        (state.screen_share.take(), camera)
    }

    /// cancels the camera capture watchdog and takes the capture it watched
    fn take_camera_capture(state: &mut RiseRTCLiveKitMediaControllerState) -> Option<CallCameraCapture> {
        state.camera_capture_epoch = state.camera_capture_epoch.wrapping_add(1);
        if let Some(watchdog) = state.camera_capture_watchdog.take() {
            watchdog.abort();
        }
        state.camera_capture.take()
    }

    fn stop_camera_capture(capture: Option<CallCameraCapture>) {
        if let Some(mut capture) = capture {
            capture.stop();
        }
    }

    async fn start_camera_capture(&self, device_id: Option<String>, shelf: VideoShelf) {
        let Some(current) = self.state.with(|state| state.active_session.clone()) else {
            return;
        };
        let local_user_id = current.key.participant_identity.clone();
        let mirror_preview = self.state.with(|state| state.camera_position) == RiseRTCCameraPosition::Front;
        let frames: CallFrameSink = Arc::new(move |frame: HostNv12Frame| {
            if let Err(reason) = livekit_room::push_local_frame(TrackSource::Camera, &frame) {
                tracing::debug!(%reason, "rtc camera frame dropped");
            }
            let dimensions = Dimensions { width: frame.width, height: frame.height };
            let preview_frame =
                if mirror_preview { native_surface::mirror_host_nv12_frame(&frame) } else { Some(frame) };
            let Some(preview_frame) = preview_frame else {
                tracing::debug!("rtc camera preview frame dropped: plane shorter than its rows");
                return;
            };
            match native_surface::prepare_host_frame(preview_frame) {
                Ok(prepared) => {
                    let preview = RemoteVideoFrame {
                        track_key: LOCAL_CAMERA_TRACK_KEY.to_owned(),
                        user_id: Some(local_user_id.clone()),
                        source: TrackSource::Camera,
                        rotation_degrees: 0,
                        frame: prepared,
                    };
                    Self::on_main(move |cx| Self::render_video_frame(preview, Some(dimensions), cx));
                }
                Err(error) => tracing::debug!(%error, "rtc camera preview frame dropped"),
            }
        });
        let ended: CallCameraEndSink = Arc::new(|error: CameraCaptureError| {
            RiseAccountRuntime::current().spawn(async move {
                RiseRTCLiveKitMediaController::shared().handle_camera_capture_end(error);
            });
        });
        let started = RiseAccountRuntime::current()
            .spawn_blocking(move || CallCameraCapture::start(device_id.as_deref(), &shelf, frames, ended))
            .await;
        let capture = match started {
            Ok(Ok(capture)) => capture,
            Ok(Err(error)) => {
                tracing::warn!(%error, "rtc camera capture did not start");
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "rtc camera capture task ended");
                return;
            }
        };
        let (orphans, epoch) = self.state.with(|state| {
            if !state.active_session.as_ref().is_some_and(|active| active.generation == current.generation) {
                return ((Some(capture), None), None);
            }
            let previous = Self::take_camera_capture(state);
            state.camera_capture = Some(capture);
            ((None, previous), Some(state.camera_capture_epoch))
        });
        Self::stop_camera_capture(orphans.0);
        Self::stop_camera_capture(orphans.1);
        if let Some(epoch) = epoch {
            self.schedule_camera_capture_watchdog(current.generation, epoch);
            self.refresh_tracks(&current.key);
        }
    }

    fn schedule_camera_capture_watchdog(&self, generation: u64, epoch: u64) {
        let (handle, registration) = AbortHandle::new_pair();
        let previous = self.state.with(|state| state.camera_capture_watchdog.replace(handle));
        if let Some(previous) = previous {
            previous.abort();
        }
        let watchdog = Abortable::new(
            async move {
                tokio::time::sleep(CAMERA_CAPTURE_WATCHDOG_DELAY).await;
                RiseRTCLiveKitMediaController::shared().restart_stalled_camera_capture(generation, epoch).await;
            },
            registration,
        );
        RiseAccountRuntime::current().spawn(watchdog.map(|_| ()));
    }

    async fn restart_stalled_camera_capture(&self, generation: u64, epoch: u64) {
        let stalled = self.state.with(|state| {
            if state.camera_capture_epoch != epoch {
                return None;
            }
            state.camera_capture_watchdog = None;
            let current = state.active_session.clone().filter(|active| active.generation == generation)?;
            if !state.camera_enabled || state.background_muted_camera {
                return None;
            }
            if state.camera_capture.as_ref().is_none_or(|capture| capture.dimensions().is_some()) {
                return None;
            }
            Some((current, state.camera_capture.take()?))
        });
        let Some((current, capture)) = stalled else {
            return;
        };
        tracing::info!(room = %current.key.room_id, "camera capture produced no frames; restarting");
        let restarted = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let result = capture.restart();
                (capture, result)
            })
            .await;
        let capture = match restarted {
            Ok((capture, Ok(()))) => capture,
            Ok((capture, Err(error))) => {
                tracing::debug!(%error, "rtc camera restart");
                capture
            }
            Err(error) => {
                tracing::debug!(%error, "rtc camera restart task ended");
                return;
            }
        };
        let orphan = self.state.with(|state| {
            if state.camera_capture_epoch != epoch || state.camera_capture.is_some() {
                return Some(capture);
            }
            state.camera_capture = Some(capture);
            None
        });
        Self::stop_camera_capture(orphan);
        if !self.is_current(&current) {
            return;
        }
        self.refresh_tracks(&current.key);
    }

    fn handle_camera_capture_end(&self, error: CameraCaptureError) {
        tracing::warn!(%error, "rtc camera capture ended");
        let camera = self.state.with(Self::take_camera_capture);
        Self::stop_camera_capture(camera);
        Self::on_main(|cx| Self::remove_video_track(LOCAL_CAMERA_TRACK_KEY, cx));
    }

    fn begin_video_tracks(session: &RiseRTCMediaSessionKey) {
        VIDEO_TRACKS.with_borrow_mut(|tracks| {
            tracks.tracks.clear();
            tracks.room = Some(RiseRTCLiveKitMediaControllerRoomSnapshot {
                session: session.clone(),
                remote_participants: Vec::new(),
                local_camera_position: None,
            });
        });
    }

    fn end_video_tracks(session: &RiseRTCMediaSessionKey) {
        VIDEO_TRACKS.with_borrow_mut(|tracks| {
            if tracks.room.as_ref().is_some_and(|room| &room.session == session) {
                tracks.room = None;
                tracks.tracks.clear();
            }
        });
    }

    fn replace_room(snapshot: RiseRTCLiveKitMediaControllerRoomSnapshot, cx: &mut gpui::App) {
        let dropped = VIDEO_TRACKS.with_borrow_mut(|tracks| {
            let current = tracks.room.as_ref().is_some_and(|room| room.session == snapshot.session);
            if current {
                tracks.room = Some(snapshot);
                None
            } else {
                Some(snapshot)
            }
        });
        if let Some(snapshot) = dropped {
            tracing::warn!(
                target: "riseonly::X1882",
                room = %snapshot.session.room_id,
                remotes = snapshot.remote_participants.len(),
                "rtc room snapshot dropped: video tracks hold another session"
            );
        } else {
            Self::present_room(cx);
        }
    }

    /// main thread, one picture of a track
    /// first seen track joins the room video and the presentation hears of it
    /// size comes from outside, a prepared macos surface reports none
    fn render_video_frame(frame: RemoteVideoFrame, dimensions: Option<Dimensions>, cx: &mut gpui::App) {
        let (track, created) = VIDEO_TRACKS.with_borrow_mut(|tracks| {
            tracks.room.as_ref()?;
            if let Some(known) = tracks.tracks.get(&frame.track_key) {
                return Some((known.track.clone(), false));
            }
            let track = VideoTrack::new(frame.track_key.clone(), frame.source, dimensions);
            tracks.tracks.insert(
                frame.track_key.clone(),
                RiseRTCLiveKitMediaControllerVideoTrack {
                    track: track.clone(),
                    user_id: frame.user_id.clone(),
                    source: frame.source,
                },
            );
            Some((track, true))
        })
        .unzip();
        let Some(track) = track else {
            return;
        };
        if created == Some(true) {
            Self::present_room(cx);
        }
        if dimensions.is_some() {
            track.update_dimensions(dimensions, cx);
        }
        RiseRTCCallPictureInPictureController::shared(cx).update(cx, |controller, cx| controller.render(&frame, cx));
        track.render(frame, cx);
    }

    /// main thread, a track the room no longer publishes
    fn remove_video_track(track_key: &str, cx: &mut gpui::App) {
        let removed = VIDEO_TRACKS.with_borrow_mut(|tracks| tracks.tracks.remove(track_key).is_some());
        if removed {
            Self::present_room(cx);
        }
    }

    /// main thread, replaces tracks from the room over the tracks held here
    fn present_room(cx: &mut gpui::App) {
        let Some((room, session)) = VIDEO_TRACKS.with_borrow(|tracks| {
            let snapshot = tracks.room.as_ref()?;
            let publications_of = |user_id: Option<&str>| -> Vec<RiseRTCVideoPublication> {
                tracks
                    .tracks
                    .iter()
                    .filter(|(key, held)| {
                        let local = matches!(key.as_str(), LOCAL_CAMERA_TRACK_KEY | LOCAL_SCREEN_TRACK_KEY);
                        match user_id {
                            None => local,
                            Some(user_id) => !local && held.user_id.as_deref() == Some(user_id),
                        }
                    })
                    .map(|(_, held)| RiseRTCVideoPublication {
                        source: held.source,
                        is_muted: false,
                        track: Some(held.track.clone()),
                        dimensions: held.track.dimensions(),
                    })
                    .collect()
            };
            let local_participant = RiseRTCParticipantVideo {
                identity: snapshot.session.participant_identity.clone(),
                metadata: livekit_room::local_metadata(&snapshot.session.participant_identity).unwrap_or_default(),
                video_tracks: publications_of(None),
            };
            let remote_participants = snapshot
                .remote_participants
                .iter()
                .map(|participant| RiseRTCParticipantVideo {
                    identity: participant.identity.clone(),
                    metadata: participant.metadata.clone(),
                    video_tracks: publications_of(participant.user_id().as_deref()),
                })
                .collect();
            let room = RiseRTCRoomVideo {
                local_participant,
                remote_participants,
                local_camera_position: snapshot.local_camera_position,
            };
            Some((room, snapshot.session.clone()))
        }) else {
            return;
        };
        RiseRTCMediaPresentation::shared(cx)
            .update(cx, |presentation, cx| presentation.replace_tracks(&room, &session, cx));
    }

    fn frame_dimensions(frame: &RemoteVideoFrame) -> Option<Dimensions> {
        let (width, height) = frame.frame.size();
        if frame.rotation_degrees % 180 == 90 {
            return Some(Dimensions { width: height, height: width });
        }
        Some(Dimensions { width, height })
    }

    fn now_milliseconds() -> i64 {
        (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64
    }
}

impl RiseRTCMediaDriving for RiseRTCLiveKitMediaController {
    fn perform(&self, intent: RiseRTCMediaIntent) -> BoxFuture<'_, anyhow::Result<()>> {
        RiseRTCLiveKitMediaController::perform(self, intent).boxed()
    }

    fn events(&self) -> BoxFuture<'_, UnboundedReceiver<RiseRTCMediaEvent>> {
        RiseRTCLiveKitMediaController::events(self).boxed()
    }
}

pub enum RiseRTCLiveKitBootstrap {}

impl RiseRTCLiveKitBootstrap {
    pub fn install(cx: &mut gpui::App) {
        RiseRTCLiveKitAudioCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.install_system_bridge(cx));
        RiseEngineMainHop::shared(cx);
        let runtime = RiseAccountRuntime::current();
        if let Some(mut receiver) = livekit_room::install(runtime.handle().clone()) {
            runtime.spawn(async move {
                let controller = RiseRTCLiveKitMediaController::shared();
                while let Some(event) = receiver.recv().await {
                    controller.receive(event).await;
                }
            });
        }
        RiseRTCMediaRuntime::shared()
            .install(Arc::new(|| RiseRTCLiveKitMediaController::shared() as Arc<dyn RiseRTCMediaDriving>));
    }
}
