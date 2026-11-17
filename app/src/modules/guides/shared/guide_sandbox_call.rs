use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{App, AssetSource, AsyncApp, Context, SMOOTH_SVG_SCALE_FACTOR, SvgRenderer, Task};
use image::imageops::FilterType;
use image::{ImageFormat, Rgba, RgbaImage};
use rise_canon::RiseGuideSignal;
use rise_canon::rise_crypto::CALL_VERIFICATION_EMOJIS;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::camera_capture::call_camera::{CallCameraCapture, CallCameraEndSink, CallFrameSink};
use rise_media::camera_capture::{self, CameraCaptureError};
use rise_media::rtc_media::livekit_room::RemoteVideoFrame;
use rise_media::rtc_media::{TrackSource, VideoCeiling, camera_shelf};
use rise_media::texture::native_surface::{self, HostBgraFrame, HostNv12Frame, PreparedHostFrame};

use crate::app::router::navigation_types::RiseRTCRoomTarget;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::guides::engine::rise_guides_models::RiseGuide;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::rtc::engine::core::rise_rtc_store_layout::{RiseRTCRoomMeta, RiseRTCRoomRepositorySnapshot};
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCCallMediaMode, RiseRTCCallState, RiseRTCCameraPosition, RiseRTCConnectionState, RiseRTCDirectCall,
    RiseRTCMediaSessionKey, RiseRTCParticipant, RiseRTCParticipantRole, RiseRTCRoomConfig, RiseRTCRoomMode,
    RiseRTCUserProfile,
};
use crate::modules::rtc::engine::rise_rtc_presentation::RiseRTCPresentation;
use crate::modules::rtc::media::rise_rtc_call_picture_in_picture::{
    RiseRTCCallPictureInPictureController, RiseRTCCallPictureInPictureControllerOwner,
};
use crate::modules::rtc::media::rise_rtc_live_kit_media_controller::RiseRTCLiveKitMediaController;
use crate::modules::rtc::media::rise_rtc_media_presentation::{Dimensions, RiseRTCMediaPresentation, VideoTrack};
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;

const PANDA_ANSWER_MS: u64 = 1_500;
const ENDED_CALL_LINGER_MS: u64 = 1_200;
const RINGING_WINDOW_MS: i64 = 60_000;
const PANDA_FRAME_INTERVAL: Duration = Duration::from_millis(100);
const SCREEN_FRAME_INTERVAL: Duration = Duration::from_millis(500);
const LOCAL_CAMERA_TRACK_KEY: &str = "guide.practice.camera";
const PANDA_CAMERA_TRACK_KEY: &str = "guide.practice.panda";
const SCREEN_TRACK_KEY: &str = "guide.practice.screen";
const APP_LOGO_ASSET: &str = "images/AppLogo.png";
const PRACTICE_VERIFICATION_INDEXES: [usize; 4] = [3, 17, 29, 41];
static PRACTICE_STREAM_IDS: AtomicU64 = AtomicU64::new(0x0067_0000_0000_0000);

/// the call lessons, a practice call with panda on the real call screen
/// no server and no livekit room, the sandbox publishes a direct call room
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideSandboxCallScene {
    pub id: Uuid,
    pub started_at_ms: i64,
    pub answered_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
    pub self_microphone: bool,
    pub self_deafened: bool,
    pub self_camera: bool,
    pub self_screen_share: bool,
    pub panda_camera: bool,
    pub camera_position: RiseRTCCameraPosition,
    pub revision: i64,
    pub viewer_user_id: String,
}

impl GuideSandboxCallScene {
    pub fn new(started_at_ms: i64, viewer_user_id: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            started_at_ms,
            answered_at_ms: None,
            ended_at_ms: None,
            self_microphone: true,
            self_deafened: false,
            self_camera: false,
            self_screen_share: false,
            panda_camera: false,
            camera_position: RiseRTCCameraPosition::Front,
            revision: 1,
            viewer_user_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GuideSandboxCallCameraRequest {
    scene_id: Uuid,
    request_id: Uuid,
    account_id: String,
    account_epoch: String,
}

/// read off the lesson steps, a video lesson meets panda with camera on so theres a picture to swap
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuideSandboxCallLesson {
    pub panda_shows_camera: bool,
}

impl GuideSandboxCallLesson {
    pub fn new(guide: &RiseGuide) -> Self {
        let signals: HashSet<RiseGuideSignal> = guide.steps.iter().filter_map(|step| step.advance_signal).collect();
        Self {
            panda_shows_camera: signals.contains(&RiseGuideSignal::CallCameraOn)
                || signals.contains(&RiseGuideSignal::CallWindowsSwapped),
        }
    }
}

/// reader's camera while filming for the practice call
/// feed tells this capture's frames from a replaced one
struct GuideSandboxCallCamera {
    feed: Uuid,
    track: Rc<VideoTrack>,
    capture: CallCameraCapture,
}

/// practice call tracks and the loops that draw into them
/// the screen holds tracks weakly so this holds them
pub struct GuideSandboxCallMedia {
    camera_request_id: Uuid,
    local_camera: Option<GuideSandboxCallCamera>,
    pub(crate) panda_camera: Option<Rc<VideoTrack>>,
    pub(crate) screen: Option<Rc<VideoTrack>>,
    pub(crate) panda_frames: Option<Task<()>>,
    pub(crate) screen_frames: Option<Task<()>>,
}

impl Default for GuideSandboxCallMedia {
    fn default() -> Self {
        Self::new()
    }
}

impl GuideSandboxCallMedia {
    pub fn new() -> Self {
        Self {
            camera_request_id: Uuid::new_v4(),
            local_camera: None,
            panda_camera: None,
            screen: None,
            panda_frames: None,
            screen_frames: None,
        }
    }

    pub fn begin_camera_request(&mut self) -> Uuid {
        self.camera_request_id = Uuid::new_v4();
        self.camera_request_id
    }

    pub fn stop_all(&mut self) {
        self.begin_camera_request();
        self.panda_frames = None;
        self.screen_frames = None;
        self.panda_camera = None;
        self.screen = None;
        if let Some(camera) = self.local_camera.take() {
            Self::stop_capture(camera.capture);
        }
    }

    fn local_camera_track(&self) -> Option<Rc<VideoTrack>> {
        self.local_camera.as_ref().map(|camera| camera.track.clone())
    }

    /// takes over a started capture as the reader's camera, the old one is stopped
    fn install_local_camera(&mut self, feed: Uuid, capture: CallCameraCapture) {
        let track = VideoTrack::new(LOCAL_CAMERA_TRACK_KEY.to_owned(), TrackSource::Camera, None);
        let previous = self.local_camera.replace(GuideSandboxCallCamera { feed, track, capture });
        if let Some(previous) = previous {
            Self::stop_capture(previous.capture);
        }
    }

    fn remove_local_camera(&mut self) {
        if let Some(camera) = self.local_camera.take() {
            Self::stop_capture(camera.capture);
        }
    }

    fn stop_capture(mut capture: CallCameraCapture) {
        drop(RiseAccountRuntime::current().spawn_blocking(move || capture.stop()));
    }
}

impl GuideSandbox {
    fn practice_media_session() -> RiseRTCMediaSessionKey {
        RiseRTCMediaSessionKey {
            room_id: Self::practice_call_room_id(),
            connection_id: Self::call_scene_id(),
            participant_identity: Self::panda_user_id(),
        }
    }

    fn practice_call_target() -> RiseRTCRoomTarget {
        RiseRTCRoomTarget::DirectCall { call_id: Self::practice_call_id() }
    }

    pub fn owns_call_scene(&self) -> bool {
        self.is_armed && self.scenes.contains(&Self::call_scene_id())
    }

    pub fn owns_call_room_id(&self, id: Option<&str>) -> bool {
        self.is_armed && self.call_scene.is_some() && id == Some(Self::practice_call_room_id().as_str())
    }

    pub fn owns_call_id(&self, id: Option<&str>) -> bool {
        self.is_armed && self.call_scene.is_some() && id == Some(Self::practice_call_id().as_str())
    }

    /// answers the profile call button when the callee is panda
    /// returns the call id or none if the call isnt the lesson's
    pub fn start_practice_call(&mut self, callee_id: &str, cx: &mut Context<Self>) -> Option<Uuid> {
        if !self.owns_call_scene() || callee_id != Self::panda_user_id() {
            return None;
        }
        let presentation = RiseRTCPresentation::shared(cx);
        if !presentation.read(cx).current_account_id().is_some_and(|account_id| !account_id.is_empty()) {
            return None;
        }
        if self.call_scene.as_ref().is_some_and(|scene| scene.ended_at_ms.is_none()) {
            RTCServicesStore::shared(cx).update(cx, |store, cx| store.present_room(Self::practice_call_target(), cx));
            return Some(Uuid::new_v4());
        }
        self.call_media.stop_all();
        let viewer_user_id = self.viewer_profile(cx).id;
        self.call_scene = Some(GuideSandboxCallScene::new(self.now_ms(), viewer_user_id));
        presentation.update(cx, |presentation, cx| presentation.select_room(Some(Self::practice_call_room_id()), cx));
        self.publish_practice_call_room(cx);
        presentation.update(cx, |presentation, cx| {
            presentation.publish_connection_state(RiseRTCConnectionState::Connected, cx);
        });
        self.refresh_practice_call_media(cx);
        RTCServicesStore::shared(cx).update(cx, |store, cx| store.present_room(Self::practice_call_target(), cx));
        self.schedule(PANDA_ANSWER_MS, cx, |sandbox, cx| sandbox.answer_practice_call(cx));
        cx.notify();
        Some(Uuid::new_v4())
    }

    fn answer_practice_call(&mut self, cx: &mut Context<Self>) {
        let now = self.now_ms();
        let shows_camera = self.call_lesson.is_some_and(|lesson| lesson.panda_shows_camera);
        let Some(scene) = self.call_scene.as_mut().filter(|scene| scene.ended_at_ms.is_none()) else {
            return;
        };
        scene.answered_at_ms = Some(now);
        scene.panda_camera = shows_camera;
        if shows_camera {
            self.start_panda_camera(cx);
        }
        self.publish_practice_call_room(cx);
        self.refresh_practice_call_media(cx);
        let emojis = Self::practice_verification_emojis();
        RiseRTCPresentation::shared(cx).update(cx, |presentation, cx| {
            presentation.publish_verification_emojis(emojis, &Self::practice_call_id(), cx);
        });
        cx.notify();
    }

    /// ends the call like the server would, then removes the room after the screen showed the ending
    pub fn end_practice_call(&mut self, cx: &mut Context<Self>) -> Option<RiseRTCDirectCall> {
        let now = self.now_ms();
        let scene = self.call_scene.as_mut()?;
        if scene.ended_at_ms.is_none() {
            scene.ended_at_ms = Some(now);
            scene.self_camera = false;
            scene.self_screen_share = false;
            self.call_media.stop_all();
            self.publish_practice_call_room(cx);
            self.refresh_practice_call_media(cx);
            GuideSignalBus::emit(RiseGuideSignal::CallEnded);
            self.schedule(ENDED_CALL_LINGER_MS, cx, |sandbox, cx| {
                if sandbox.call_scene.as_ref().is_some_and(|scene| scene.ended_at_ms.is_some()) {
                    sandbox.tear_down_practice_call(cx);
                }
            });
            cx.notify();
        }
        self.practice_call(&Self::practice_call_id())
    }

    pub fn withdraw_call(&mut self, cx: &mut Context<Self>) {
        if self.call_scene.is_none() {
            return;
        }
        self.tear_down_practice_call(cx);
        self.call_scene = None;
        cx.notify();
    }

    fn tear_down_practice_call(&mut self, cx: &mut Context<Self>) {
        self.call_media.stop_all();
        RiseRTCCallPictureInPictureController::shared(cx).update(cx, |picture_in_picture, cx| {
            picture_in_picture.disable(RiseRTCCallPictureInPictureControllerOwner::DirectCall, cx);
        });
        let session = Self::practice_media_session();
        RiseRTCMediaPresentation::shared(cx).update(cx, |media, cx| media.reset_session(&session, cx));
        let services = RTCServicesStore::shared(cx);
        let shows_practice_call = matches!(
            services.read(cx).room_presentation_target(),
            Some(RiseRTCRoomTarget::DirectCall { call_id }) if *call_id == Self::practice_call_id()
        );
        if shows_practice_call {
            services.update(cx, |store, cx| store.dismiss_room_presentation(cx));
        }
        let presentation = RiseRTCPresentation::shared(cx);
        let holds_practice_room = presentation
            .read(cx)
            .current_active_room()
            .is_some_and(|room| room.config.room_id == Self::practice_call_room_id());
        if holds_practice_room {
            presentation.update(cx, |presentation, cx| {
                presentation.select_room(None, cx);
                presentation.publish_connection_state(RiseRTCConnectionState::Idle, cx);
            });
        }
        GuideFacts::set(RiseGuideSignal::CallConnected, false, cx);
        GuideFacts::set(RiseGuideSignal::CallCameraOn, false, cx);
        GuideFacts::set(RiseGuideSignal::CallScreenShareOn, false, cx);
    }

    pub fn practice_call(&self, call_id: &str) -> Option<RiseRTCDirectCall> {
        if !self.owns_call_id(Some(call_id)) {
            return None;
        }
        let scene = self.call_scene.as_ref()?;
        let state = if scene.ended_at_ms.is_some() {
            RiseRTCCallState::Ended
        } else if scene.answered_at_ms.is_some() {
            RiseRTCCallState::Accepted
        } else {
            RiseRTCCallState::Ringing
        };
        let ended = scene.ended_at_ms.is_some();
        Some(RiseRTCDirectCall {
            call_id: Self::practice_call_id(),
            room_id: Self::practice_call_room_id(),
            chat_id: Self::chat_id(),
            caller_user_id: scene.viewer_user_id.clone(),
            callee_user_id: Self::panda_user_id(),
            media_mode: RiseRTCCallMediaMode::Audio,
            state,
            created_at_ms: scene.started_at_ms,
            updated_at_ms: scene.ended_at_ms.or(scene.answered_at_ms).unwrap_or(scene.started_at_ms),
            expires_at_ms: scene.started_at_ms + RINGING_WINDOW_MS,
            answered_at_ms: scene.answered_at_ms,
            ended_at_ms: scene.ended_at_ms,
            ended_by_user_id: ended.then(|| scene.viewer_user_id.clone()),
            end_reason: ended.then(|| "client_hangup".to_owned()),
            caller_e2ee_public_key: None,
            callee_e2ee_public_key: None,
            recording_state: None,
            recording_started_by_user_id: None,
            recording_path: None,
        })
    }

    /// panda, for the screen to name before she answers
    pub fn practice_call_counterpart(&self, call_id: &str) -> Option<RiseRTCUserProfile> {
        self.owns_call_id(Some(call_id)).then(|| self.panda_profile())
    }

    fn practice_verification_emojis() -> Vec<String> {
        let pool = &CALL_VERIFICATION_EMOJIS;
        PRACTICE_VERIFICATION_INDEXES.iter().map(|index| pool[index % pool.len()].to_owned()).collect()
    }

    fn publish_practice_call_room(&mut self, cx: &mut Context<Self>) {
        if self.call_scene.is_none() {
            return;
        }
        let presentation = RiseRTCPresentation::shared(cx);
        let Some(account_id) =
            presentation.read(cx).current_account_id().filter(|account_id| !account_id.is_empty()).map(str::to_owned)
        else {
            return;
        };
        let Some(scene) = self.call_scene.as_mut() else {
            return;
        };
        scene.revision += 1;
        let scene = scene.clone();
        let now = self.now_ms();
        let lease_ms = self.lease_ms();
        let viewer = self.viewer_profile(cx);
        let mut participants = vec![RiseRTCParticipant {
            user_id: viewer.id.clone(),
            role: RiseRTCParticipantRole::Speaker,
            connected: true,
            microphone_enabled: scene.self_microphone && !scene.self_deafened,
            camera_enabled: scene.self_camera,
            screen_share_enabled: scene.self_screen_share,
            server_muted: false,
            server_deafened: false,
            deafened: scene.self_deafened,
            hand_raised: false,
            joined_at_ms: scene.started_at_ms,
            updated_at_ms: now,
            lease_expires_at_ms: now + lease_ms,
            order_sequence: 1,
            profile: Some(viewer),
        }];
        if let Some(answered_at_ms) = scene.answered_at_ms
            && scene.ended_at_ms.is_none()
        {
            participants.push(RiseRTCParticipant {
                user_id: Self::panda_user_id(),
                role: RiseRTCParticipantRole::Speaker,
                connected: true,
                microphone_enabled: true,
                camera_enabled: scene.panda_camera,
                screen_share_enabled: false,
                server_muted: false,
                server_deafened: false,
                deafened: false,
                hand_raised: false,
                joined_at_ms: answered_at_ms,
                updated_at_ms: now,
                lease_expires_at_ms: now + lease_ms,
                order_sequence: 2,
                profile: Some(self.panda_profile()),
            });
        }
        let config = RiseRTCRoomConfig {
            room_id: Self::practice_call_room_id(),
            livekit_room_name: String::new(),
            chat_id: Self::chat_id(),
            channel_id: None,
            mode: RiseRTCRoomMode::DirectCall,
            max_participants: 2,
            max_speakers: 2,
            video_enabled: true,
            screen_share_enabled: true,
            audio_bitrate_kbps: 0,
            region: String::new(),
            revision: scene.revision,
            created_at_ms: scene.started_at_ms,
            updated_at_ms: now,
        };
        let participant_count = i64::try_from(participants.len()).unwrap_or(i64::MAX);
        let meta = RiseRTCRoomMeta {
            participant_count,
            speaker_count: participant_count,
            generated_at_ms: now,
            profiles_complete: true,
            profile_error: None,
            has_more_participants: false,
            next_participant_cursor: None,
            has_more_raised_hands: false,
            next_raised_hand_cursor: None,
        };
        let snapshot = RiseRTCRoomRepositorySnapshot {
            account_id,
            revision: scene.revision,
            room: Some(config),
            participants,
            raised_hands: Vec::new(),
            meta: Some(meta),
        };
        presentation.update(cx, |presentation, cx| presentation.publish_room(snapshot, cx));
    }

    fn refresh_practice_call_media(&mut self, cx: &mut Context<Self>) {
        let Some(scene) = self.call_scene.clone() else {
            return;
        };
        let viewer_id = self.viewer_profile(cx).id;
        let mut cameras: HashMap<String, Rc<VideoTrack>> = HashMap::new();
        let mut screens: HashMap<String, Rc<VideoTrack>> = HashMap::new();
        if scene.self_camera
            && let Some(track) = self.call_media.local_camera_track()
        {
            cameras.insert(viewer_id.clone(), track);
        }
        if scene.panda_camera
            && let Some(track) = self.call_media.panda_camera.clone()
        {
            cameras.insert(Self::panda_user_id(), track);
        }
        if scene.self_screen_share
            && let Some(track) = self.call_media.screen.clone()
        {
            screens.insert(viewer_id, track);
        }
        let present = scene.answered_at_ms.is_some() && scene.ended_at_ms.is_none();
        let remotes: HashSet<String> = if present { HashSet::from([Self::panda_user_id()]) } else { HashSet::new() };
        let session = Self::practice_media_session();
        RiseRTCMediaPresentation::shared(cx).update(cx, |media, cx| {
            media.install_practice(&session, &cameras, &screens, remotes, scene.camera_position, cx);
        });
    }

    pub fn set_practice_call_microphone(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let Some(scene) = self.call_scene.as_mut() else {
            return;
        };
        scene.self_microphone = enabled;
        self.publish_practice_call_room(cx);
        cx.notify();
    }

    pub fn set_practice_call_deafened(&mut self, deafened: bool, cx: &mut Context<Self>) {
        let Some(scene) = self.call_scene.as_mut() else {
            return;
        };
        scene.self_deafened = deafened;
        self.publish_practice_call_room(cx);
        cx.notify();
    }

    /// reader's real camera filming into a track nobody receives
    /// refused permission fails the switch like a real call
    pub fn set_practice_call_camera(
        &mut self,
        enabled: bool,
        position: RiseRTCCameraPosition,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        let Some(request) = self.begin_practice_camera_request(cx) else {
            return Task::ready(Ok(()));
        };
        let opens_camera = enabled && self.call_media.local_camera.is_none();
        cx.spawn(async move |this, cx| {
            let mut started = None;
            if opens_camera {
                let access = RiseRTCLiveKitMediaController::ensure_camera_access().await;
                let current =
                    this.update(cx, |sandbox, cx| sandbox.is_current_practice_camera_request(&request, cx)).unwrap_or(false);
                if !current {
                    return Ok(());
                }
                if let Err(error) = access {
                    return Err(anyhow::Error::from(error));
                }
                match Self::start_practice_capture(position).await {
                    Ok(capture) => started = Some(capture),
                    Err(error) => {
                        let current = this
                            .update(cx, |sandbox, cx| sandbox.is_current_practice_camera_request(&request, cx))
                            .unwrap_or(false);
                        return if current { Err(error) } else { Ok(()) };
                    }
                }
            }
            if let Err(error) = this.update(cx, |sandbox, cx| {
                sandbox.apply_practice_camera(&request, enabled, position, started, cx);
            }) {
                tracing::debug!(%error, "guide practice camera outlived the sandbox");
            }
            Ok(())
        })
    }

    fn apply_practice_camera(
        &mut self,
        request: &GuideSandboxCallCameraRequest,
        enabled: bool,
        position: RiseRTCCameraPosition,
        started: Option<(Uuid, CallCameraCapture)>,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current_practice_camera_request(request, cx) || self.call_scene.is_none() {
            if let Some((_, capture)) = started {
                GuideSandboxCallMedia::stop_capture(capture);
            }
            return;
        }
        if let Some((feed, capture)) = started {
            self.call_media.install_local_camera(feed, capture);
        } else if !enabled {
            self.call_media.remove_local_camera();
        }
        if let Some(scene) = self.call_scene.as_mut() {
            scene.self_camera = enabled;
            scene.camera_position = position;
        }
        self.publish_practice_call_room(cx);
        self.refresh_practice_call_media(cx);
        cx.notify();
    }

    /// flips the filming camera to the other side
    /// a computer has no front and back so the built in is front and the next is back
    pub fn switch_practice_camera(
        &mut self,
        position: RiseRTCCameraPosition,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        let Some(request) = self.begin_practice_camera_request(cx) else {
            return Task::ready(Ok(()));
        };
        let filming = self.call_media.local_camera.is_some();
        cx.spawn(async move |this, cx| {
            let mut started = None;
            if filming {
                match Self::start_practice_capture(position).await {
                    Ok(capture) => started = Some(capture),
                    Err(error) => {
                        let current = this
                            .update(cx, |sandbox, cx| sandbox.is_current_practice_camera_request(&request, cx))
                            .unwrap_or(false);
                        return if current { Err(error) } else { Ok(()) };
                    }
                }
            }
            if let Err(error) = this.update(cx, |sandbox, cx| {
                sandbox.apply_practice_camera_switch(&request, position, started, cx);
            }) {
                tracing::debug!(%error, "guide practice camera outlived the sandbox");
            }
            Ok(())
        })
    }

    fn apply_practice_camera_switch(
        &mut self,
        request: &GuideSandboxCallCameraRequest,
        position: RiseRTCCameraPosition,
        started: Option<(Uuid, CallCameraCapture)>,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current_practice_camera_request(request, cx) || self.call_scene.is_none() {
            if let Some((_, capture)) = started {
                GuideSandboxCallMedia::stop_capture(capture);
            }
            return;
        }
        if let Some((feed, capture)) = started {
            self.call_media.install_local_camera(feed, capture);
        }
        if let Some(scene) = self.call_scene.as_mut() {
            scene.camera_position = position;
        }
        self.refresh_practice_call_media(cx);
        cx.notify();
    }

    fn begin_practice_camera_request(&mut self, cx: &mut Context<Self>) -> Option<GuideSandboxCallCameraRequest> {
        if !self.owns_call_scene() {
            return None;
        }
        let scene_id = self.call_scene.as_ref().filter(|scene| scene.ended_at_ms.is_none())?.id;
        let account_id = RiseRTCPresentation::shared(cx)
            .read(cx)
            .current_account_id()
            .filter(|account_id| !account_id.is_empty())?
            .to_owned();
        let account_epoch = SaiAccountCacheScope::active_epoch()?;
        if !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&account_epoch)) {
            return None;
        }
        Some(GuideSandboxCallCameraRequest {
            scene_id,
            request_id: self.call_media.begin_camera_request(),
            account_id,
            account_epoch,
        })
    }

    fn is_current_practice_camera_request(&self, request: &GuideSandboxCallCameraRequest, cx: &mut App) -> bool {
        self.owns_call_scene()
            && self.call_scene.as_ref().is_some_and(|scene| scene.id == request.scene_id && scene.ended_at_ms.is_none())
            && self.call_media.camera_request_id == request.request_id
            && RiseRTCPresentation::shared(cx).read(cx).current_account_id() == Some(request.account_id.as_str())
            && SaiAccountCacheScope::is_current_account_epoch(Some(&request.account_id), Some(&request.account_epoch))
    }

    /// opens the camera for position and starts frames toward the main thread
    /// frames are dropped until the capture is installed
    async fn start_practice_capture(position: RiseRTCCameraPosition) -> anyhow::Result<(Uuid, CallCameraCapture)> {
        let runtime = RiseAccountRuntime::current();
        let devices = runtime.spawn_blocking(camera_capture::devices).await??;
        let front = position == RiseRTCCameraPosition::Front;
        let device_id = camera_capture::pick_device(&devices, front).map(|device| device.id.clone());
        let feed = Uuid::new_v4();
        let shelf = camera_shelf(VideoCeiling::FREE);
        let frames: CallFrameSink = Arc::new(move |frame: HostNv12Frame| {
            let dimensions = Dimensions { width: frame.width, height: frame.height };
            match native_surface::prepare_host_frame(frame) {
                Ok(prepared) => match RiseEngineMainHop::installed() {
                    Some(hop) => hop.post(move |cx| Self::render_practice_camera_frame(feed, dimensions, prepared, cx)),
                    None => tracing::debug!("guide practice camera frame before the main hop started"),
                },
                Err(error) => tracing::debug!(%error, "guide practice camera frame dropped"),
            }
        });
        let ended: CallCameraEndSink = Arc::new(|error: CameraCaptureError| {
            tracing::debug!(%error, "guide practice camera ended");
        });
        let capture = runtime
            .spawn_blocking(move || CallCameraCapture::start(device_id.as_deref(), &shelf, frames, ended))
            .await??;
        Ok((feed, capture))
    }

    fn render_practice_camera_frame(feed: Uuid, dimensions: Dimensions, frame: PreparedHostFrame, cx: &mut App) {
        let sandbox = Self::shared(cx);
        let track = sandbox
            .read(cx)
            .call_media
            .local_camera
            .as_ref()
            .filter(|camera| camera.feed == feed)
            .map(|camera| camera.track.clone());
        let Some(track) = track else {
            return;
        };
        let user_id = sandbox.read(cx).call_scene.as_ref().map(|scene| scene.viewer_user_id.clone());
        track.update_dimensions(Some(dimensions), cx);
        track.render(
            RemoteVideoFrame {
                track_key: LOCAL_CAMERA_TRACK_KEY.to_owned(),
                user_id,
                source: TrackSource::Camera,
                rotation_degrees: 0,
                frame,
            },
            cx,
        );
    }

    /// share a real call would ask the system for, here the picture is this app itself
    pub fn set_practice_screen_share(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.call_scene.is_none() {
            return;
        }
        if enabled && self.call_media.screen.is_none() {
            self.start_practice_screen(cx);
        } else if !enabled {
            self.call_media.screen_frames = None;
            self.call_media.screen = None;
        }
        if let Some(scene) = self.call_scene.as_mut() {
            scene.self_screen_share = enabled;
        }
        self.publish_practice_call_room(cx);
        self.refresh_practice_call_media(cx);
        cx.notify();
    }

    fn start_panda_camera(&mut self, cx: &mut Context<Self>) {
        if self.call_media.panda_camera.is_some() {
            return;
        }
        let track = VideoTrack::new(PANDA_CAMERA_TRACK_KEY.to_owned(), TrackSource::Camera, None);
        let weak_track = Rc::downgrade(&track);
        self.call_media.panda_camera = Some(track);
        let renderer = SvgRenderer::new(Arc::clone(cx.asset_source()));
        let stream_id = PRACTICE_STREAM_IDS.fetch_add(1, Ordering::Relaxed);
        self.call_media.panda_frames = Some(cx.spawn(async move |this, cx| {
            let frame = cx
                .background_executor()
                .spawn(async move { GuideSandboxCallPictures::panda_frame(&renderer, stream_id) })
                .await;
            let Some((dimensions, frame)) = frame else {
                if let Err(error) = this.update(cx, |sandbox, cx| {
                    let ours = sandbox
                        .call_media
                        .panda_camera
                        .as_ref()
                        .is_some_and(|track| weak_track.upgrade().is_some_and(|weak| Rc::ptr_eq(track, &weak)));
                    if ours {
                        sandbox.call_media.panda_camera = None;
                        sandbox.refresh_practice_call_media(cx);
                    }
                }) {
                    tracing::debug!(%error, "guide practice panda camera outlived the sandbox");
                }
                return;
            };
            let user_id = Self::panda_user_id();
            loop {
                if !Self::render_drawn_frame(&weak_track, PANDA_CAMERA_TRACK_KEY, &user_id, dimensions, &frame, cx) {
                    return;
                }
                cx.background_executor().timer(PANDA_FRAME_INTERVAL).await;
            }
        }));
    }

    fn start_practice_screen(&mut self, cx: &mut Context<Self>) {
        let track = VideoTrack::new(SCREEN_TRACK_KEY.to_owned(), TrackSource::ScreenShare, None);
        let weak_track = Rc::downgrade(&track);
        self.call_media.screen = Some(track);
        let assets = Arc::clone(cx.asset_source());
        let user_id = self.viewer_profile(cx).id;
        let stream_id = PRACTICE_STREAM_IDS.fetch_add(1, Ordering::Relaxed);
        self.call_media.screen_frames = Some(cx.spawn(async move |_, cx| {
            let logo = cx
                .background_executor()
                .spawn(async move { GuideSandboxCallPictures::app_logo(assets.as_ref()).map(Arc::new) })
                .await;
            let mut generation: u64 = 0;
            while weak_track.strong_count() > 0 {
                if let Some((width, height)) = cx.update(GuideSandboxCallPictures::screen_size) {
                    generation = generation.wrapping_add(1);
                    let logo = logo.clone();
                    let frame = cx
                        .background_executor()
                        .spawn(async move {
                            GuideSandboxCallPictures::screen_frame(logo.as_deref(), width, height, stream_id, generation)
                        })
                        .await;
                    if let Some(frame) = frame {
                        let dimensions = Dimensions { width, height };
                        if !Self::render_drawn_frame(&weak_track, SCREEN_TRACK_KEY, &user_id, dimensions, &frame, cx) {
                            return;
                        }
                    }
                }
                cx.background_executor().timer(SCREEN_FRAME_INTERVAL).await;
            }
        }));
    }

    /// hands one drawn picture to its track, false once the track is gone
    fn render_drawn_frame(
        track: &Weak<VideoTrack>,
        track_key: &str,
        user_id: &str,
        dimensions: Dimensions,
        frame: &PreparedHostFrame,
        cx: &mut AsyncApp,
    ) -> bool {
        let Some(track) = track.upgrade() else {
            return false;
        };
        let source = track.source();
        let picture = RemoteVideoFrame {
            track_key: track_key.to_owned(),
            user_id: Some(user_id.to_owned()),
            source,
            rotation_degrees: 0,
            frame: frame.clone(),
        };
        cx.update(|cx| {
            track.update_dimensions(Some(dimensions), cx);
            track.render(picture, cx);
        });
        true
    }
}

/// pictures for the drawn tracks, BGRA and even sizes or the encoder skips them
pub enum GuideSandboxCallPictures {}

impl GuideSandboxCallPictures {
    const PANDA_WIDTH: u32 = 540;
    const PANDA_HEIGHT: u32 = 960;
    const ENCODER_ALIGNMENT: u32 = 16;

    pub fn panda_frame(renderer: &SvgRenderer, stream_id: u64) -> Option<(Dimensions, PreparedHostFrame)> {
        let width = Self::PANDA_WIDTH;
        let height = Self::PANDA_HEIGHT;
        let document = format!(
            concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">"#,
                r##"<defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1">"##,
                r##"<stop offset="0" stop-color="#FADB9E"/><stop offset="1" stop-color="#8CC79E"/>"##,
                r##"</linearGradient></defs><rect width="{w}" height="{h}" fill="url(#g)"/>"##,
                r#"<text x="{cx}" y="{cy}" font-size="300" text-anchor="middle" dominant-baseline="central">{panda}</text>"#,
                r#"<text x="40" y="{by}" font-size="120" dominant-baseline="text-before-edge">{bamboo}</text>"#,
                "</svg>"
            ),
            w = width,
            h = height,
            cx = width / 2,
            cy = height / 2 - 40,
            by = height - 220,
            panda = "\u{1F43C}",
            bamboo = "\u{1F38B}",
        );
        let image = renderer.render_single_frame(document.as_bytes(), 1.0 / SMOOTH_SVG_SCALE_FACTOR).ok()?;
        let size = image.size(0);
        let width = u32::try_from(size.width.0).ok()?;
        let height = u32::try_from(size.height.0).ok()?;
        let pixels = image.as_bytes(0)?;
        let frame = HostBgraFrame {
            stream_id,
            generation: 1,
            width,
            height,
            stride: width.checked_mul(4)?,
            pixels: Arc::from(pixels),
        };
        let prepared = native_surface::prepare_host_bgra_frame(frame).ok()?;
        Some((Dimensions { width, height }, prepared))
    }

    /// window size now at half, rounded to sizes the encoder accepts
    pub fn screen_size(cx: &mut App) -> Option<(u32, u32)> {
        let window = cx.active_window()?;
        let viewport = window.update(cx, |_, window, _| window.viewport_size()).ok()?;
        let width_points = f32::from(viewport.width);
        let height_points = f32::from(viewport.height);
        if width_points <= 0.0 || height_points <= 0.0 {
            return None;
        }
        let width = ((width_points / 2.0) as u32 / Self::ENCODER_ALIGNMENT) * Self::ENCODER_ALIGNMENT;
        let height = ((height_points / 2.0) as u32 / Self::ENCODER_ALIGNMENT) * Self::ENCODER_ALIGNMENT;
        (width >= Self::ENCODER_ALIGNMENT && height >= Self::ENCODER_ALIGNMENT).then_some((width, height))
    }

    pub fn app_logo(assets: &dyn AssetSource) -> Option<RgbaImage> {
        let bytes = assets.load(APP_LOGO_ASSET).ok().flatten()?;
        Some(image::load_from_memory_with_format(&bytes, ImageFormat::Png).ok()?.to_rgba8())
    }

    /// this app at the window size
    /// gpui has no render to texture so the picture is the app mark on black
    pub fn screen_frame(
        logo: Option<&RgbaImage>,
        width: u32,
        height: u32,
        stream_id: u64,
        generation: u64,
    ) -> Option<PreparedHostFrame> {
        let mut canvas = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));
        if let Some(logo) = logo {
            let side = (width.min(height) / 3).max(1);
            let scaled = image::imageops::resize(logo, side, side, FilterType::Triangle);
            let left = i64::from((width - side) / 2);
            let top = i64::from((height - side) / 2);
            image::imageops::overlay(&mut canvas, &scaled, left, top);
        }
        let mut pixels = canvas.into_raw();
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let frame = HostBgraFrame {
            stream_id,
            generation,
            width,
            height,
            stride: width.checked_mul(4)?,
            pixels: Arc::from(pixels),
        };
        native_surface::prepare_host_bgra_frame(frame).ok()
    }
}
