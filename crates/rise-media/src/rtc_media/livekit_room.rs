use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;
use futures::StreamExt;
use livekit::e2ee::key_provider::{KeyProvider, KeyProviderOptions};
use livekit::e2ee::{E2eeOptions, EncryptionType};
use livekit::options::{
    AudioEncoding as SdkAudioEncoding, TrackPublishOptions, VideoCodec as SdkVideoCodec, VideoEncoding as SdkVideoEncoding,
};
use livekit::prelude::{
    LocalAudioTrack, LocalTrack, LocalVideoTrack, RemoteTrack, Room, RoomEvent, RoomOptions, TrackSid,
};
use livekit::track::{TrackKind as SdkTrackKind, TrackSource as SdkTrackSource};
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::native::apm::AudioProcessingModule;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use livekit::webrtc::native::yuv_helper;
use livekit::webrtc::video_frame::{NV12Buffer, VideoBuffer, VideoFrame, VideoRotation};
use livekit::webrtc::video_source::native::NativeVideoSource;
use livekit::webrtc::video_source::{RtcVideoSource, VideoResolution};
use livekit::webrtc::video_stream::native::NativeVideoStream;
use parking_lot::Mutex;
use rise_platform::capture_permission::{self, CaptureDecision, CaptureKind};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::HostOs;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::rtc_media::*;
use crate::screen_capture::{ScreenCaptureError, ScreenFrame, ScreenPixels};
use crate::texture::color::{ColorDescription, ColorRange, ColorSpace};
use crate::texture::frame::FrameFormat;
use crate::texture::native_surface::{self, HostNv12Frame, PreparedHostFrame};

const SOURCE_QUEUE_MS: u32 = 50;
const PLAYOUT_QUEUE_MS: u32 = 200;
const PLAYOUT_RATE: u32 = 48_000;
const PLAYOUT_CHANNELS: u16 = 2;
const INPUT_BLOCKS: usize = 32;
const TESTER_INPUT_TICK: std::time::Duration = std::time::Duration::from_millis(10);
const LEVEL_EVERY_BLOCKS: u32 = 8;
const MICROPHONE_TEST_QUEUE_MS: u32 = 120;
const MICROPHONE_TEST_TRACK: &str = "microphone-test";

static MICROPHONE_TEST_HUSHES_CALL: AtomicBool = AtomicBool::new(false);

/// what the call tells the screen and the domain, in order
/// comes on the receiver install returns
#[derive(Clone, Debug)]
pub enum RtcEvent {
    Session(SessionOutput),
    InputLevel(f32),
    CaptureRefused { kind: CaptureKind, decision: CaptureDecision },
    LocalVideoWanted { source: TrackSource, device_id: Option<String>, include_system_audio: bool, shelf: VideoShelf },
    LocalVideoStopped { source: TrackSource },
    ScreenShareDeadPipe,
    RemoteVideo(RemoteVideoFrame),
    RemoteVideoEnded { track_key: String },
}

#[derive(Clone, Debug)]
pub struct RemoteVideoFrame {
    pub track_key: String,
    pub user_id: Option<String>,
    pub source: TrackSource,
    pub rotation_degrees: u16,
    pub frame: PreparedHostFrame,
}

/// room engine state for the process, one call at a time
struct Engine {
    runtime: Handle,
    events: mpsc::UnboundedSender<RtcEvent>,
    settings: Mutex<Settings>,
    intended: Mutex<Option<MediaSessionKey>>,
    call: Mutex<Option<Arc<Call>>>,
    generation: AtomicU64,
    microphone_test: Mutex<Option<MicrophoneTest>>,
    microphone_test_generation: AtomicU64,
    microphone_test_state: AtomicU8,
}

/// microphone test of the audio settings screen
/// own mic through the gate and the call processing chain, played on the picked output
/// dropping it closes the mic first which ends the worker, then the output
struct MicrophoneTest {
    gate: Arc<Mutex<InputNoiseGate>>,
    input: Option<DeviceThread>,
    worker: Option<std::thread::JoinHandle<()>>,
    output: Option<DeviceThread>,
}

impl Drop for MicrophoneTest {
    fn drop(&mut self) {
        self.input.take();
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                tracing::warn!("rtc microphone test worker panicked");
            }
        }
        self.output.take();
    }
}

/// who an output device plays for, the call goes quiet while the mic test runs
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PlayoutRole {
    Call,
    MicrophoneTest,
}

/// what holds across calls, audio settings and the devices picked
#[derive(Clone, Default)]
struct Settings {
    processing: AudioProcessingSettings,
    input: Option<String>,
    output: Option<String>,
}

/// frame cryptor of a direct call
#[derive(Clone, PartialEq, Eq)]
enum Encryption {
    Pending,
    Keyed(String),
    Disabled,
}

struct Call {
    key: MediaSessionKey,
    ceiling: VideoCeiling,
    audio_max_bitrate: Option<u64>,
    encryption: Option<Mutex<Encryption>>,
    room: Room,
    host_id: cpal::HostId,
    host: HostOs,
    session: Mutex<MediaSession>,
    mixer: Arc<Mutex<PlayoutMixer>>,
    playout: Arc<Mutex<RemotePlayout>>,
    output: Mutex<Option<DeviceThread>>,
    microphone: Mutex<Option<Microphone>>,
    microphone_wanted: AtomicBool,
    gate: Arc<Mutex<InputNoiseGate>>,
    video: Mutex<HashMap<TrackSource, LocalVideo>>,
    screen_audio: Mutex<Option<ScreenAudio>>,
    remote: Mutex<HashMap<String, JoinHandle<()>>>,
}

struct Microphone {
    sid: TrackSid,
    source: NativeAudioSource,
    _device: DeviceThread,
    pump: JoinHandle<()>,
}

/// way into the share sound track
/// dropping it ends the pump and unpublishes the track
/// so a stop that lands mid publish leaves nothing behind
struct ScreenAudio {
    blocks: mpsc::Sender<Vec<i16>>,
    sample_rate: u32,
    channels: u16,
}

struct LocalVideo {
    sid: TrackSid,
    source: NativeVideoSource,
    frames: Arc<AtomicU64>,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();
static NEXT_STREAM: AtomicU64 = AtomicU64::new(1);

pub fn install(runtime: Handle) -> Option<mpsc::UnboundedReceiver<RtcEvent>> {
    let (events, receiver) = mpsc::unbounded_channel();
    let engine = Engine {
        runtime,
        events,
        settings: Mutex::new(Settings::default()),
        intended: Mutex::new(None),
        call: Mutex::new(None),
        generation: AtomicU64::new(0),
        microphone_test: Mutex::new(None),
        microphone_test_generation: AtomicU64::new(0),
        microphone_test_state: AtomicU8::new(MicrophoneTestState::Off.raw()),
    };
    ENGINE.set(engine).ok().map(|()| receiver)
}

fn engine() -> Result<&'static Engine, RtcUnavailable> {
    ENGINE.get().ok_or(RtcUnavailable::NoRuntime(NO_RUNTIME))
}

pub fn availability() -> Result<(), RtcUnavailable> {
    engine().map(|_| ())
}

fn emit(engine: &Engine, event: RtcEvent) {
    if engine.events.send(event).is_err() {
        tracing::debug!("rtc event receiver is gone");
    }
}

fn current_call(engine: &Engine) -> Option<Arc<Call>> {
    engine.call.lock().clone()
}

pub fn input_meter_level() -> f64 {
    let Ok(engine) = engine() else {
        return 0.0;
    };
    let now = std::time::Instant::now();
    let tested = engine.microphone_test.lock().as_ref().map(|test| test.gate.lock().meter_level(now));
    if let Some(level) = tested {
        return level;
    }
    current_call(engine).map_or(0.0, |call| call.gate.lock().meter_level(now))
}

/// metadata the server made for this device participant, none unless the room local identity matches
/// identity is a per join uuid so the user behind the local tiles is read from here
pub fn local_metadata(identity: &str) -> Option<String> {
    let call = engine().ok().and_then(current_call)?;
    let local = call.room.local_participant();
    (local.identity().0 == identity).then(|| local.metadata())
}

/// invariant: only voice the call really sends counts, never a muted or test hushed mic
pub fn is_locally_speaking() -> bool {
    if MICROPHONE_TEST_HUSHES_CALL.load(Ordering::SeqCst) {
        return false;
    }
    engine().ok().and_then(current_call).is_some_and(|call| {
        call.microphone_wanted.load(Ordering::SeqCst)
            && call.microphone.lock().is_some()
            && call.gate.lock().is_locally_speaking(std::time::Instant::now())
    })
}

/// call this intent may act on, same guard as requireSession
fn require_call(engine: &Engine, key: &MediaSessionKey) -> Result<Arc<Call>, RtcUnavailable> {
    let intended = engine.intended.lock().clone();
    let call = current_call(engine);
    let phase = call.as_ref().map(|call| call.session.lock().phase);
    let active = call.as_ref().zip(phase).map(|(call, phase)| ActiveSession { key: &call.key, phase });
    require_session(intended.as_ref(), active, key)?;
    call.ok_or(RtcUnavailable::NotConnected)
}

/// wall clock milliseconds since 1970
fn now_ms() -> i64 {
    (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64
}

/// true if this session is the intended or the active one
fn owns(engine: &Engine, key: &MediaSessionKey) -> bool {
    let intended = engine.intended.lock().clone();
    let call = current_call(engine);
    owns_session(intended.as_ref(), call.as_ref().map(|call| &call.key), key)
}

/// live call of this session if its room is up already
fn call_of(engine: &Engine, key: &MediaSessionKey) -> Option<Arc<Call>> {
    current_call(engine).filter(|call| &call.key == key)
}

pub fn perform(os: &dyn RtcMediaEngine, host: HostOs, intent: &MediaIntent) -> Result<(), RtcUnavailable> {
    let engine = engine()?;
    tracing::debug!(intent = intent_name(intent), "rtc media intent");
    match intent {
        MediaIntent::Connect(request) => {
            validate_connect(request, now_ms())?;
            let key = request.session_key();
            let step = {
                let mut intended = engine.intended.lock();
                let call = current_call(engine);
                let phase = call.as_ref().map(|call| call.session.lock().phase);
                let active = call.as_ref().zip(phase).map(|(call, phase)| ActiveSession { key: &call.key, phase });
                let step = connect_step(intended.as_ref(), active, &key);
                if step == ConnectStep::Start {
                    *intended = Some(key);
                }
                step
            };
            match step {
                ConnectStep::AlreadyConnected => Ok(()),
                ConnectStep::InProgress => Err(RtcUnavailable::Refused(CODE_CONNECT_IN_PROGRESS)),
                ConnectStep::Start => {
                    let generation = engine.generation.fetch_add(1, Ordering::SeqCst) + 1;
                    let request = request.clone();
                    let host_id = os.audio_host_id();
                    engine.runtime.spawn(run_call(engine, request, generation, host_id, host));
                    Ok(())
                }
            }
        }
        MediaIntent::Disconnect(key) => {
            if !owns(engine, key) {
                tracing::debug!("rtc disconnect of another session ignored");
                return Ok(());
            }
            engine.generation.fetch_add(1, Ordering::SeqCst);
            *engine.intended.lock() = None;
            let call = {
                let mut slot = engine.call.lock();
                let ours = slot.as_ref().is_some_and(|call| &call.key == key);
                if ours {
                    slot.take()
                } else {
                    None
                }
            };
            if let Some(call) = call {
                engine.runtime.spawn(async move { teardown(engine, call, true).await });
            }
            Ok(())
        }
        MediaIntent::SetMicrophone { session, enabled } => {
            let call = require_call(engine, session)?;
            let enabled = *enabled;
            call.microphone_wanted.store(enabled, Ordering::SeqCst);
            engine.runtime.spawn(async move { set_microphone(engine, call, enabled).await });
            Ok(())
        }
        MediaIntent::SetRemotePlayout { session, playout } => {
            let call = require_call(engine, session)?;
            *call.playout.lock() = playout.clone();
            Ok(())
        }
        MediaIntent::SetCamera { session, enabled, camera_id } => {
            let call = require_call(engine, session)?;
            let shelf = camera_shelf(call.ceiling);
            let (enabled, camera_id) = (*enabled, camera_id.clone());
            engine.runtime.spawn(async move {
                if enabled {
                    publish_video(engine, call, TrackSource::Camera, shelf, PublishedVideoCodec::Vp8, camera_id, false).await;
                } else {
                    unpublish_video(engine, &call, TrackSource::Camera).await;
                }
            });
            Ok(())
        }
        MediaIntent::SetEncryptionKey { session, key } => {
            let call = require_call(engine, session)?;
            if let Some(state) = call.encryption.as_ref() {
                *state.lock() = key.clone().map_or(Encryption::Disabled, Encryption::Keyed);
            }
            apply_encryption(&call);
            Ok(())
        }
        MediaIntent::SetScreenShare { session, enabled, include_system_audio, quality, display } => {
            let call = require_call(engine, session)?;
            let shelf = screen_share_shelf(*quality, call.ceiling, *display);
            let (enabled, system_audio) = (*enabled, *include_system_audio);
            let codec = os.screen_share_codec();
            engine.runtime.spawn(async move {
                if enabled {
                    publish_video(engine, call, TrackSource::ScreenShare, shelf, codec, None, system_audio).await;
                } else {
                    unpublish_video(engine, &call, TrackSource::ScreenShare).await;
                }
            });
            Ok(())
        }
        MediaIntent::SelectAudioInput { session, id } => {
            if !owns(engine, session) {
                return Err(RtcUnavailable::NotConnected);
            }
            engine.settings.lock().input = id.clone();
            if let Some(call) = call_of(engine, session) {
                let restart = call.microphone.lock().is_some();
                if restart {
                    engine.runtime.spawn(async move {
                        set_microphone(engine, call.clone(), false).await;
                        set_microphone(engine, call, true).await;
                    });
                }
            }
            Ok(())
        }
        MediaIntent::SelectAudioOutput { session, id } => {
            if !owns(engine, session) {
                return Err(RtcUnavailable::NotConnected);
            }
            engine.settings.lock().output = id.clone();
            if let Some(call) = call_of(engine, session) {
                engine.runtime.spawn(open_output_off_thread(engine, call));
            }
            Ok(())
        }
        MediaIntent::ApplyAudioProcessing(settings) => {
            let settings = settings.normalized();
            engine.settings.lock().processing = settings;
            if let Some(test) = engine.microphone_test.lock().as_ref() {
                test.gate.lock().update(settings.input_sensitivity);
            }
            if let Some(call) = current_call(engine) {
                call.gate.lock().update(settings.input_sensitivity);
                if let Some(microphone) = call.microphone.lock().as_ref() {
                    microphone.source.set_audio_options(source_options(settings.options_for(call.host)));
                }
            }
            Ok(())
        }
        MediaIntent::SendData { .. } => Err(RtcUnavailable::Refused(CODE_DATA_PUBLISH_DISABLED)),
    }
}

fn source_options(options: AudioProcessingOptions) -> AudioSourceOptions {
    AudioSourceOptions {
        echo_cancellation: options.echo_cancellation,
        noise_suppression: options.noise_suppression,
        auto_gain_control: options.auto_gain_control,
    }
}

/// applies call encryption state to every frame cryptor the room holds
/// rerun on track change and reconnect, new cryptors get armed at creation
/// livekit 0.7.32 set_enabled skips its flag so each is switched alone
fn apply_encryption(call: &Call) {
    let Some(state) = call.encryption.as_ref() else {
        return;
    };
    let state = state.lock().clone();
    let manager = call.room.e2ee_manager();
    let enabled = match state {
        Encryption::Pending => return,
        Encryption::Keyed(key) => {
            match manager.key_provider() {
                Some(provider) => provider.set_shared_key(key.into_bytes(), 0),
                None => {
                    tracing::warn!("rtc room has no key provider for an encrypted call");
                    return;
                }
            }
            true
        }
        Encryption::Disabled => false,
    };
    for cryptor in manager.frame_cryptors().values() {
        cryptor.set_enabled(enabled);
    }
}

/// key nobody else holds, for the window before the agreed one arrives
/// rust sdk has no discard when cryptor not ready so that window encrypts under random bytes
/// per process, the far side cant decrypt them
fn pending_key() -> Vec<u8> {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = Vec::with_capacity(32);
    for round in 0u64..4 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(round);
        bytes.extend_from_slice(&hasher.finish().to_le_bytes());
    }
    bytes
}

fn room_options(request: &ConnectRequest) -> RoomOptions {
    let mut options = RoomOptions::default();
    options.auto_subscribe = true;
    options.adaptive_stream = true;
    options.dynacast = true;
    options.encryption = request.end_to_end_encrypted.then(|| E2eeOptions {
        encryption_type: EncryptionType::Gcm,
        key_provider: KeyProvider::with_shared_key(KeyProviderOptions::default(), pending_key()),
    });
    options
}

const SDK_JOINED_AT_MS: Option<i64> = None;

fn from_remote(participant: &livekit::prelude::RemoteParticipant) -> RemoteParticipant {
    let publications = participant.track_publications().into_values().map(|publication| PublishedTrack {
        audio: publication.kind() == SdkTrackKind::Audio,
        source: seam_source(publication.source()),
        muted: publication.is_muted(),
    });
    RemoteParticipant::new(
        participant.identity().0,
        participant.name(),
        participant.metadata(),
        publications,
        SDK_JOINED_AT_MS,
    )
}

const fn seam_source(source: SdkTrackSource) -> TrackSource {
    match source {
        SdkTrackSource::Microphone => TrackSource::Microphone,
        SdkTrackSource::Camera => TrackSource::Camera,
        SdkTrackSource::Screenshare => TrackSource::ScreenShare,
        SdkTrackSource::ScreenshareAudio => TrackSource::ScreenShareAudio,
        SdkTrackSource::Unknown => TrackSource::Unknown,
    }
}

const fn sdk_source(source: TrackSource) -> SdkTrackSource {
    match source {
        TrackSource::Microphone => SdkTrackSource::Microphone,
        TrackSource::Camera => SdkTrackSource::Camera,
        TrackSource::ScreenShare => SdkTrackSource::Screenshare,
        TrackSource::ScreenShareAudio => SdkTrackSource::ScreenshareAudio,
        TrackSource::Unknown => SdkTrackSource::Unknown,
    }
}

/// feeds one callback to the session and acts on what it says before the domain hears it
fn receive(engine: &Engine, call: &Call, callback: RoomCallback) -> bool {
    let outputs = call.session.lock().receive(callback);
    let mut ended = false;
    for output in outputs {
        match output {
            SessionOutput::Ended { .. } => ended = true,
            SessionOutput::TracksChanged | SessionOutput::Reconnected => apply_encryption(call),
            _ => {}
        }
        emit(engine, RtcEvent::Session(output));
    }
    ended
}

async fn run_call(engine: &'static Engine, request: ConnectRequest, generation: u64, host_id: cpal::HostId, host: HostOs) {
    let previous = engine.call.lock().take();
    if let Some(previous) = previous {
        teardown(engine, previous, true).await;
    }
    let settings = engine.settings.lock().clone();
    let key = request.session_key();
    let mut session = MediaSession::new(&request);
    tracing::info!(endpoint = %sanitized_endpoint(&request.server_url), "rtc connect");
    let connected = Room::connect(&request.server_url, &request.access_token, room_options(&request)).await;
    let (room, mut room_events) = match connected {
        Ok(pair) => pair,
        Err(error) => {
            let failure = classify_sdk_error(&error.to_string());
            tracing::warn!(code = failure_code(failure), "rtc connect failed");
            if engine.generation.load(Ordering::SeqCst) == generation {
                release_intended(engine, &key);
            }
            for output in session.receive(RoomCallback::ConnectFailed(Some(failure))) {
                emit(engine, RtcEvent::Session(output));
            }
            return;
        }
    };
    let local_identity = room.local_participant().identity().0;
    let call = Arc::new(Call {
        key: key.clone(),
        ceiling: request.video_ceiling,
        audio_max_bitrate: microphone_max_bitrate(request.audio_bitrate_kbps),
        encryption: request.end_to_end_encrypted.then(|| Mutex::new(Encryption::Pending)),
        room,
        host_id,
        host,
        session: Mutex::new(session),
        mixer: Arc::new(Mutex::new(PlayoutMixer::new(PlayoutMixer::capacity_for(
            PLAYOUT_RATE,
            PLAYOUT_CHANNELS,
            PLAYOUT_QUEUE_MS,
        )))),
        playout: Arc::new(Mutex::new(RemotePlayout::default())),
        output: Mutex::new(None),
        microphone: Mutex::new(None),
        microphone_wanted: AtomicBool::new(false),
        screen_audio: Mutex::new(None),
        gate: Arc::new(Mutex::new(InputNoiseGate::new(settings.processing.input_sensitivity))),
        video: Mutex::new(HashMap::new()),
        remote: Mutex::new(HashMap::new()),
    });
    let current = {
        let mut slot = engine.call.lock();
        let current = engine.generation.load(Ordering::SeqCst) == generation;
        if current {
            *slot = Some(call.clone());
        }
        current
    };
    if !current {
        if let Err(error) = call.room.close().await {
            tracing::debug!(%error, "stale rtc room close");
        }
        return;
    }
    if receive(engine, &call, RoomCallback::Connected { local_identity }) {
        if take_if_current(engine, &call) {
            release_intended(engine, &key);
        }
        teardown(engine, call, true).await;
        return;
    }
    let present: Vec<_> = call.room.remote_participants().values().map(from_remote).collect();
    tracing::warn!(
        target: "riseonly::X1882",
        present = present.len(),
        users = ?present.iter().map(RemoteParticipant::user_id).collect::<Vec<_>>(),
        "rtc room connected"
    );
    for participant in present {
        if receive(engine, &call, RoomCallback::ParticipantJoined(participant)) {
            if take_if_current(engine, &call) {
                release_intended(engine, &key);
            }
            teardown(engine, call, true).await;
            return;
        }
    }
    open_output_off_thread(engine, call.clone()).await;

    while let Some(event) = room_events.recv().await {
        if engine.generation.load(Ordering::SeqCst) != generation {
            break;
        }
        let speakers_after_mic_drop = drops_microphone(&event).then(|| live_speakers(&call));
        if let Some(callback) = translate(engine, &call, event) {
            if receive(engine, &call, callback) {
                break;
            }
        }
        if let Some(callback) = speakers_after_mic_drop {
            if receive(engine, &call, callback) {
                break;
            }
        }
    }
    if take_if_current(engine, &call) {
        release_intended(engine, &key);
        teardown(engine, call, false).await;
    }
}

/// takes this call out of the engine slot if its still the one there
fn take_if_current(engine: &Engine, call: &Arc<Call>) -> bool {
    let mut slot = engine.call.lock();
    let ours = slot.as_ref().is_some_and(|current| Arc::ptr_eq(current, call));
    if ours {
        slot.take();
    }
    ours
}

/// session that ended by itself is no longer intended, unless the domain already moved on
fn release_intended(engine: &Engine, key: &MediaSessionKey) {
    let mut intended = engine.intended.lock();
    if intended.as_ref() == Some(key) {
        *intended = None;
    }
}

fn drops_microphone(event: &RoomEvent) -> bool {
    match event {
        RoomEvent::TrackMuted { publication, .. } => publication.source() == SdkTrackSource::Microphone,
        RoomEvent::TrackUnpublished { publication, .. } | RoomEvent::TrackUnsubscribed { publication, .. } => {
            publication.source() == SdkTrackSource::Microphone
        }
        _ => false,
    }
}

/// remote speakers whose mic is still live, loudest first
fn live_speakers(call: &Call) -> RoomCallback {
    let mut speakers: Vec<(RemoteParticipant, f32)> = call
        .room
        .remote_participants()
        .values()
        .filter(|participant| participant.is_speaking())
        .filter(|participant| {
            participant
                .track_publications()
                .values()
                .any(|publication| publication.source() == SdkTrackSource::Microphone && !publication.is_muted())
        })
        .map(|participant| (from_remote(participant), participant.audio_level()))
        .collect();
    speakers.sort_by(|left, right| right.1.total_cmp(&left.1));
    RoomCallback::SpeakersChanged(speakers)
}

/// turns one sdk event into the seam callback, media side effects like subscriptions happen here
fn translate(engine: &'static Engine, call: &Arc<Call>, event: RoomEvent) -> Option<RoomCallback> {
    match event {
        RoomEvent::ParticipantConnected(participant) => {
            let joined = from_remote(&participant);
            tracing::warn!(target: "riseonly::X1882", user = ?joined.user_id(), "rtc sdk participant connected");
            Some(RoomCallback::ParticipantJoined(joined))
        }
        RoomEvent::ParticipantDisconnected(participant) => {
            let remaining = call.room.remote_participants().values().map(from_remote).collect();
            Some(RoomCallback::ParticipantLeft {
                left: from_remote(&participant),
                remaining,
            })
        }
        RoomEvent::TrackSubscribed { track, publication, participant } => {
            let user_id = from_remote(&participant).user_id();
            let key = publication.sid().to_string();
            let source = seam_source(publication.source());
            start_remote(engine, call, key, user_id, source, track);
            Some(RoomCallback::TracksChanged)
        }
        RoomEvent::TrackUnsubscribed { publication, .. } => {
            let key = publication.sid().to_string();
            stop_remote(engine, call, &key);
            Some(RoomCallback::TracksChanged)
        }
        RoomEvent::TrackPublished { .. }
        | RoomEvent::LocalTrackPublished { .. }
        | RoomEvent::TrackUnpublished { .. }
        | RoomEvent::TrackMuted { .. }
        | RoomEvent::TrackUnmuted { .. }
        | RoomEvent::ParticipantMetadataChanged { .. } => Some(RoomCallback::TracksChanged),
        RoomEvent::ActiveSpeakersChanged { .. } => Some(live_speakers(call)),
        RoomEvent::DataReceived { payload, topic, .. } => Some(RoomCallback::Data {
            topic: topic.unwrap_or_default(),
            payload: payload.as_ref().clone(),
        }),
        RoomEvent::Reconnecting => Some(RoomCallback::Reconnecting),
        RoomEvent::Reconnected => Some(RoomCallback::Reconnected),
        RoomEvent::Disconnected { reason } => Some(RoomCallback::Disconnected(disconnect_failure(reason.as_str_name()))),
        RoomEvent::LocalTrackUnpublished { publication, .. } => {
            let source = seam_source(publication.source());
            let ours = call.video.lock().remove(&source).is_some();
            if ours {
                stop_screen_audio(call, source);
                emit(engine, RtcEvent::LocalVideoStopped { source });
            }
            Some(RoomCallback::LocalTrackUnpublished(source))
        }
        _ => None,
    }
}

fn start_remote(engine: &'static Engine, call: &Arc<Call>, key: String, user_id: Option<String>, source: TrackSource, track: RemoteTrack) {
    let task = match track {
        RemoteTrack::Audio(track) => {
            call.mixer.lock().add_track(&key, user_id);
            let mixer = call.mixer.clone();
            let task_key = key.clone();
            engine.runtime.spawn(async move {
                let mut stream = NativeAudioStream::new(track.rtc_track(), PLAYOUT_RATE as i32, i32::from(PLAYOUT_CHANNELS));
                while let Some(frame) = stream.next().await {
                    mixer.lock().push_i16(&task_key, &frame.data);
                }
            })
        }
        RemoteTrack::Video(track) => {
            let task_key = key.clone();
            let stream_id = NEXT_STREAM.fetch_add(1, Ordering::Relaxed);
            engine.runtime.spawn(async move {
                let mut stream = NativeVideoStream::new(track.rtc_track());
                let mut generation = 0u64;
                while let Some(frame) = stream.next().await {
                    generation += 1;
                    let picture = nv12_from_buffer(&*frame.buffer, stream_id, generation);
                    let picture = match native_surface::prepare_host_frame(picture) {
                        Ok(prepared) => prepared,
                        Err(error) => {
                            tracing::debug!(%error, "rtc remote video frame dropped");
                            continue;
                        }
                    };
                    let rotation = rotation_degrees(frame.rotation);
                    if let Err(error) = native_surface::prepare_host_frame_rotation(&picture, rotation) {
                        tracing::debug!(%error, "rtc remote video frame not turned");
                    }
                    emit(
                        engine,
                        RtcEvent::RemoteVideo(RemoteVideoFrame {
                            track_key: task_key.clone(),
                            user_id: user_id.clone(),
                            source,
                            rotation_degrees: rotation,
                            frame: picture,
                        }),
                    );
                }
            })
        }
    };
    if let Some(previous) = call.remote.lock().insert(key, task) {
        previous.abort();
    }
}

fn stop_remote(engine: &Engine, call: &Call, key: &str) {
    if let Some(task) = call.remote.lock().remove(key) {
        task.abort();
    }
    call.mixer.lock().remove_track(key);
    emit(engine, RtcEvent::RemoteVideoEnded { track_key: key.to_owned() });
}

const fn rotation_degrees(rotation: VideoRotation) -> u16 {
    match rotation {
        VideoRotation::VideoRotation0 => 0,
        VideoRotation::VideoRotation90 => 90,
        VideoRotation::VideoRotation180 => 180,
        VideoRotation::VideoRotation270 => 270,
    }
}

fn nv12_from_buffer(buffer: &dyn VideoBuffer, stream_id: u64, generation: u64) -> HostNv12Frame {
    let i420 = buffer.to_i420();
    let (width, height) = (i420.width(), i420.height());
    let (stride_y, stride_u, stride_v) = i420.strides();
    let (y, u, v) = i420.data();
    let chroma_stride = width.div_ceil(2) * 2;
    let mut luma = vec![0u8; (width * height) as usize];
    let mut chroma = vec![0u8; (chroma_stride * height.div_ceil(2)) as usize];
    yuv_helper::i420_to_nv12(
        y,
        stride_y,
        u,
        stride_u,
        v,
        stride_v,
        &mut luma,
        width,
        &mut chroma,
        chroma_stride,
        width as i32,
        height as i32,
    );
    let description = ColorDescription {
        space: ColorSpace::Bt601,
        range: ColorRange::Limited,
    };
    HostNv12Frame {
        stream_id,
        generation,
        width,
        height,
        luma: luma.into(),
        luma_stride: width,
        chroma: chroma.into(),
        chroma_stride,
        conversion: description.params(FrameFormat::Nv12),
    }
}

pub fn push_screen_audio(samples: Vec<i16>, sample_rate: u32, channels: u16) -> Result<(), RtcUnavailable> {
    let engine = engine()?;
    let call = current_call(engine).ok_or(RtcUnavailable::NotConnected)?;
    let video = call.video.lock();
    if !video.contains_key(&TrackSource::ScreenShare) {
        return Err(RtcUnavailable::NotConnected);
    }
    if sample_rate < 100 || channels == 0 {
        return Ok(());
    }
    let mut lane = call.screen_audio.lock();
    let is_current = lane.as_ref().is_some_and(|audio| audio.sample_rate == sample_rate && audio.channels == channels);
    if !is_current {
        let (blocks, receiver) = mpsc::channel::<Vec<i16>>(INPUT_BLOCKS);
        engine.runtime.spawn(pump_screen_audio(call.clone(), receiver, sample_rate, channels));
        *lane = Some(ScreenAudio { blocks, sample_rate, channels });
    }
    if let Some(audio) = lane.as_ref()
        && audio.blocks.try_send(samples).is_err()
    {
        tracing::debug!("rtc screen audio block dropped");
    }
    drop(video);
    Ok(())
}

/// hands one captured picture from camera or screen capture to its published track
pub fn push_local_frame(source: TrackSource, frame: &HostNv12Frame) -> Result<(), RtcUnavailable> {
    let engine = engine()?;
    let call = current_call(engine).ok_or(RtcUnavailable::NotConnected)?;
    let video = call.video.lock();
    let Some(local) = video.get(&source) else {
        return Err(RtcUnavailable::NotConnected);
    };
    let mut buffer = NV12Buffer::with_strides(frame.width, frame.height, frame.luma_stride, frame.chroma_stride);
    let (luma, chroma) = buffer.data_mut();
    let luma_len = luma.len().min(frame.luma.len());
    luma[..luma_len].copy_from_slice(&frame.luma[..luma_len]);
    let chroma_len = chroma.len().min(frame.chroma.len());
    chroma[..chroma_len].copy_from_slice(&frame.chroma[..chroma_len]);
    local.source.capture_frame(&VideoFrame {
        rotation: VideoRotation::VideoRotation0,
        timestamp_us: 0,
        buffer,
    });
    local.frames.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

/// why a captured screen picture did not reach the published track
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ScreenFrameDropped {
    #[error(transparent)]
    Unreadable(#[from] ScreenCaptureError),
    #[error(transparent)]
    Unavailable(#[from] RtcUnavailable),
}

/// hands one captured screen picture to the published screen track
/// a surface goes to the encoder as is, planes are copied into a track buffer
pub fn push_screen_frame(frame: ScreenFrame, stream_id: u64, generation: u64) -> Result<(), ScreenFrameDropped> {
    let ScreenPixels::Surface(surface) = &frame.pixels else {
        let host_frame = frame.into_host_frame(stream_id, generation)?;
        return Ok(push_local_frame(TrackSource::ScreenShare, &host_frame)?);
    };
    let engine = engine()?;
    let call = current_call(engine).ok_or(RtcUnavailable::NotConnected)?;
    let video = call.video.lock();
    let Some(local) = video.get(&TrackSource::ScreenShare) else {
        return Err(RtcUnavailable::NotConnected.into());
    };
    if !super::os::capture_surface(&local.source, surface) {
        return Err(ScreenCaptureError::BadFrame("a surface this host does not encode").into());
    }
    local.frames.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

const fn sdk_codec(codec: PublishedVideoCodec) -> SdkVideoCodec {
    match codec {
        PublishedVideoCodec::Vp8 => SdkVideoCodec::VP8,
        PublishedVideoCodec::H264 => SdkVideoCodec::H264,
    }
}

async fn publish_video(
    engine: &'static Engine,
    call: Arc<Call>,
    source: TrackSource,
    shelf: VideoShelf,
    codec: PublishedVideoCodec,
    device_id: Option<String>,
    include_system_audio: bool,
) {
    let kind = match source {
        TrackSource::Camera => Some(CaptureKind::Camera),
        _ => None,
    };
    if let Some(kind) = kind {
        if !capture_allowed(engine, kind).await {
            return;
        }
    }
    unpublish_video(engine, &call, source).await;
    let screencast = source == TrackSource::ScreenShare;
    let native = NativeVideoSource::new(
        VideoResolution {
            width: shelf.width,
            height: shelf.height,
        },
        screencast,
    );
    let name = if screencast { "screen" } else { "camera" };
    let track = LocalVideoTrack::create_video_track(name, RtcVideoSource::Native(native.clone()));
    let options = TrackPublishOptions {
        source: sdk_source(source),
        video_encoding: shelf.encoding.map(|encoding| SdkVideoEncoding {
            max_bitrate: encoding.max_bitrate,
            max_framerate: f64::from(encoding.max_fps),
        }),
        video_codec: sdk_codec(codec),
        simulcast: !screencast,
        ..Default::default()
    };
    let published = call.room.local_participant().publish_track(LocalTrack::Video(track), options).await;
    let publication = match published {
        Ok(publication) => publication,
        Err(error) => {
            let failure = classify_sdk_error(&error.to_string());
            tracing::warn!(code = failure_code(failure), name, "rtc video publish failed");
            emit(engine, RtcEvent::Session(SessionOutput::Failure(failure_code(failure))));
            return;
        }
    };
    let frames = Arc::new(AtomicU64::new(0));
    call.video.lock().insert(
        source,
        LocalVideo {
            sid: publication.sid(),
            source: native,
            frames: frames.clone(),
        },
    );
    emit(engine, RtcEvent::LocalVideoWanted { source, device_id, include_system_audio, shelf });
    if screencast {
        tokio::time::sleep(SCREEN_SHARE_FIRST_FRAME_WINDOW).await;
        let still_ours = call.video.lock().get(&source).is_some_and(|local| Arc::ptr_eq(&local.frames, &frames));
        let liveness = screen_share_liveness(frames.load(Ordering::Relaxed), SCREEN_SHARE_FIRST_FRAME_WINDOW);
        if still_ours && liveness == ScreenShareLiveness::DeadPipe {
            tracing::warn!("rtc screen share carried no frame in its first window");
            unpublish_video(engine, &call, source).await;
            emit(engine, RtcEvent::ScreenShareDeadPipe);
        }
    }
}

async fn unpublish_video(engine: &'static Engine, call: &Call, source: TrackSource) {
    let Some(local) = call.video.lock().remove(&source) else {
        return;
    };
    stop_screen_audio(call, source);
    emit(engine, RtcEvent::LocalVideoStopped { source });
    if let Err(error) = call.room.local_participant().unpublish_track(&local.sid).await {
        tracing::debug!(%error, "rtc video unpublish");
    }
}

/// asks the capture permission seam
/// a host without an app level gate goes on, the device open reports a refusal itself
async fn capture_allowed(engine: &'static Engine, kind: CaptureKind) -> bool {
    match capture_permission::ensure_access(kind).await {
        Ok(CaptureDecision::Granted) => true,
        Ok(decision) => {
            emit(engine, RtcEvent::CaptureRefused { kind, decision });
            false
        }
        Err(reason) => {
            tracing::debug!(%reason, "no capture gate on this host");
            true
        }
    }
}

/// the sound goes with the picture of the share
fn stop_screen_audio(call: &Call, source: TrackSource) {
    if source == TrackSource::ScreenShare {
        call.screen_audio.lock().take();
    }
}

async fn pump_screen_audio(call: Arc<Call>, mut blocks: mpsc::Receiver<Vec<i16>>, rate: u32, channels: u16) {
    let options = AudioSourceOptions { echo_cancellation: false, noise_suppression: false, auto_gain_control: false };
    let source = NativeAudioSource::new(options, rate, u32::from(channels), SOURCE_QUEUE_MS);
    let track = LocalAudioTrack::create_audio_track("screen_audio", RtcAudioSource::Native(source.clone()));
    let publish = TrackPublishOptions {
        source: SdkTrackSource::ScreenshareAudio,
        audio_encoding: call.audio_max_bitrate.map(|max_bitrate| SdkAudioEncoding { max_bitrate }),
        ..Default::default()
    };
    let sid = match call.room.local_participant().publish_track(LocalTrack::Audio(track), publish).await {
        Ok(publication) => publication.sid(),
        Err(error) => {
            tracing::warn!(%error, "rtc screen audio publish failed");
            return;
        }
    };
    let per_channel = rate / 100;
    let chunk = per_channel as usize * usize::from(channels);
    let mut pending: Vec<i16> = Vec::with_capacity(chunk * 2);
    while let Some(block) = blocks.recv().await {
        pending.extend_from_slice(&block);
        while pending.len() >= chunk {
            let data: Vec<i16> = pending.drain(..chunk).collect();
            let frame = AudioFrame {
                data: data.into(),
                sample_rate: rate,
                num_channels: u32::from(channels),
                samples_per_channel: per_channel,
            };
            if let Err(error) = source.capture_frame(&frame).await {
                tracing::debug!(%error, "rtc screen audio frame was refused");
            }
        }
    }
    if let Err(error) = call.room.local_participant().unpublish_track(&sid).await {
        tracing::debug!(%error, "rtc screen audio unpublish");
    }
}

async fn set_microphone(engine: &'static Engine, call: Arc<Call>, enabled: bool) {
    let previous = call.microphone.lock().take();
    if let Some(previous) = previous {
        previous.pump.abort();
        if let Err(error) = call.room.local_participant().unpublish_track(&previous.sid).await {
            tracing::debug!(%error, "rtc microphone unpublish");
        }
        call.gate.lock().release();
        emit(engine, RtcEvent::InputLevel(0.0));
    }
    if !enabled || !call.microphone_wanted.load(Ordering::SeqCst) || !capture_allowed(engine, CaptureKind::Microphone).await {
        return;
    }
    let settings = engine.settings.lock().clone();
    let (blocks, receiver) = mpsc::channel::<Vec<f32>>(INPUT_BLOCKS);
    let (host_id, input) = (call.host_id, settings.input.clone());
    let opened = engine.runtime.spawn_blocking(move || DeviceThread::open_input(host_id, input, blocks)).await;
    let opened = opened.unwrap_or(Err(RtcUnavailable::Device {
        direction: DeviceDirection::Input,
        reason: "the microphone opener did not finish",
    }));
    let (device, rate, channels) = match opened {
        Ok(opened) => opened,
        Err(reason) => {
            tracing::warn!(%reason, "rtc microphone did not open");
            emit(engine, RtcEvent::Session(SessionOutput::Failure(CODE_AUDIO_FAILED)));
            return;
        }
    };
    let options = source_options(settings.processing.options_for(call.host));
    let source = NativeAudioSource::new(options, rate, u32::from(channels), SOURCE_QUEUE_MS);
    let track = LocalAudioTrack::create_audio_track("microphone", RtcAudioSource::Native(source.clone()));
    let publish = TrackPublishOptions {
        source: SdkTrackSource::Microphone,
        audio_encoding: call.audio_max_bitrate.map(|max_bitrate| SdkAudioEncoding { max_bitrate }),
        ..Default::default()
    };
    let publication = match call.room.local_participant().publish_track(LocalTrack::Audio(track), publish).await {
        Ok(publication) => publication,
        Err(error) => {
            let failure = classify_sdk_error(&error.to_string());
            tracing::warn!(code = failure_code(failure), "rtc microphone publish failed");
            emit(engine, RtcEvent::Session(SessionOutput::Failure(CODE_AUDIO_FAILED)));
            return;
        }
    };
    let sid = publication.sid();
    let stale = {
        let mut slot = call.microphone.lock();
        // why: a mute or a second enable that landed while this one opened the device wins
        let stale = slot.is_some() || !call.microphone_wanted.load(Ordering::SeqCst);
        if !stale {
            call.gate.lock().initialize(rate);
            let pump = engine.runtime.spawn(pump_microphone(engine, receiver, source.clone(), rate, channels, call.gate.clone()));
            *slot = Some(Microphone {
                sid: sid.clone(),
                source,
                _device: device,
                pump,
            });
        }
        stale
    };
    if stale {
        if let Err(error) = call.room.local_participant().unpublish_track(&sid).await {
            tracing::debug!(%error, "stale rtc microphone unpublish");
        }
    }
}

async fn pump_microphone(
    engine: &'static Engine,
    mut blocks: mpsc::Receiver<Vec<f32>>,
    source: NativeAudioSource,
    rate: u32,
    channels: u16,
    gate: Arc<Mutex<InputNoiseGate>>,
) {
    let per_channel = rate / 100;
    let chunk = per_channel as usize * usize::from(channels);
    let mut pending: Vec<f32> = Vec::with_capacity(chunk * 2);
    let mut counted = 0u32;
    while let Some(block) = blocks.recv().await {
        pending.extend_from_slice(&block);
        while pending.len() >= chunk {
            let mut samples: Vec<f32> = pending.drain(..chunk).collect();
            gate.lock().process(&mut samples, usize::from(channels));
            if MICROPHONE_TEST_HUSHES_CALL.load(Ordering::Relaxed) {
                samples.fill(0.0);
            }
            counted += 1;
            if counted % LEVEL_EVERY_BLOCKS == 0 {
                emit(engine, RtcEvent::InputLevel(indicator_level(&samples)));
            }
            let data: Vec<i16> = samples.iter().map(|sample| sample.clamp(-GATE_FULL_SCALE, GATE_FULL_SCALE - 1.0) as i16).collect();
            let frame = AudioFrame {
                data: data.into(),
                sample_rate: rate,
                num_channels: u32::from(channels),
                samples_per_channel: per_channel,
            };
            if let Err(error) = source.capture_frame(&frame).await {
                tracing::warn!(%error, "rtc microphone frame was refused");
            }
        }
    }
}

/// opening a device waits for its thread, so it runs on the blocking pool
/// never on the main thread or a runtime worker
async fn open_output_off_thread(engine: &'static Engine, call: Arc<Call>) {
    let opened = engine.runtime.spawn_blocking(move || start_output(engine, &call)).await;
    let failed = match opened {
        Ok(Ok(())) => None,
        Ok(Err(reason)) => Some(reason.to_string()),
        Err(error) => Some(error.to_string()),
    };
    if let Some(reason) = failed {
        tracing::warn!(%reason, "rtc playout device did not open");
        emit(engine, RtcEvent::Session(SessionOutput::Failure(CODE_AUDIO_FAILED)));
    }
}

fn start_output(engine: &Engine, call: &Call) -> Result<(), RtcUnavailable> {
    let chosen = engine.settings.lock().output.clone();
    call.output.lock().take();
    let device = DeviceThread::open_output(call.host_id, chosen, call.mixer.clone(), call.playout.clone(), PlayoutRole::Call)?;
    *call.output.lock() = Some(device);
    Ok(())
}

async fn teardown(engine: &'static Engine, call: Arc<Call>, close: bool) {
    let sources: Vec<TrackSource> = call.video.lock().drain().map(|(source, _)| source).collect();
    call.screen_audio.lock().take();
    for source in sources {
        emit(engine, RtcEvent::LocalVideoStopped { source });
    }
    if let Some(microphone) = call.microphone.lock().take() {
        microphone.pump.abort();
    }
    let keys: Vec<String> = call.remote.lock().keys().cloned().collect();
    for key in keys {
        stop_remote(engine, &call, &key);
    }
    call.output.lock().take();
    call.mixer.lock().clear();
    if close {
        if let Err(error) = call.room.close().await {
            tracing::debug!(%error, "rtc room close");
        }
    }
}

pub fn set_preferred_audio_devices(input: Option<String>, output: Option<String>) -> Result<(), RtcUnavailable> {
    let engine = engine()?;
    let (input_changed, output_changed) = {
        let mut settings = engine.settings.lock();
        let changed = (settings.input != input, settings.output != output);
        settings.input = input;
        settings.output = output;
        changed
    };
    if let Some(call) = current_call(engine) {
        if input_changed && call.microphone.lock().is_some() {
            let call = call.clone();
            engine.runtime.spawn(async move {
                set_microphone(engine, call.clone(), false).await;
                set_microphone(engine, call, true).await;
            });
        }
        if output_changed {
            engine.runtime.spawn(open_output_off_thread(engine, call));
        }
    }
    Ok(())
}

pub fn microphone_test_state() -> MicrophoneTestState {
    engine().map_or(MicrophoneTestState::Off, |engine| {
        MicrophoneTestState::from_raw(engine.microphone_test_state.load(Ordering::SeqCst))
    })
}

pub fn start_microphone_test(os: &dyn RtcMediaEngine, host: HostOs) -> Result<(), RtcUnavailable> {
    let engine = engine()?;
    let (generation, previous) = {
        let mut slot = engine.microphone_test.lock();
        let generation = engine.microphone_test_generation.fetch_add(1, Ordering::SeqCst) + 1;
        engine.microphone_test_state.store(MicrophoneTestState::Starting.raw(), Ordering::SeqCst);
        MICROPHONE_TEST_HUSHES_CALL.store(true, Ordering::SeqCst);
        (generation, slot.take())
    };
    if let Some(previous) = previous {
        engine.runtime.spawn_blocking(move || drop(previous));
    }
    let host_id = os.audio_host_id();
    engine.runtime.spawn(async move {
        let failed = |engine: &Engine| {
            let _slot = engine.microphone_test.lock();
            if engine.microphone_test_generation.load(Ordering::SeqCst) == generation {
                engine.microphone_test_state.store(MicrophoneTestState::Failed.raw(), Ordering::SeqCst);
                MICROPHONE_TEST_HUSHES_CALL.store(false, Ordering::SeqCst);
            }
        };
        if !capture_allowed(engine, CaptureKind::Microphone).await {
            failed(engine);
            return;
        }
        let opened = engine.runtime.spawn_blocking(move || open_microphone_test(engine, host_id, host)).await;
        let test = match opened {
            Ok(Ok(test)) => test,
            Ok(Err(reason)) => {
                tracing::warn!(%reason, "rtc microphone test did not open");
                failed(engine);
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "rtc microphone test opener did not finish");
                failed(engine);
                return;
            }
        };
        let stale = {
            let mut slot = engine.microphone_test.lock();
            if engine.microphone_test_generation.load(Ordering::SeqCst) == generation {
                *slot = Some(test);
                engine.microphone_test_state.store(MicrophoneTestState::Running.raw(), Ordering::SeqCst);
                MICROPHONE_TEST_HUSHES_CALL.store(true, Ordering::SeqCst);
                None
            } else {
                Some(test)
            }
        };
        if let Some(stale) = stale {
            engine.runtime.spawn_blocking(move || drop(stale));
        }
    });
    Ok(())
}

pub fn stop_microphone_test() {
    let Ok(engine) = engine() else {
        return;
    };
    let test = {
        let mut slot = engine.microphone_test.lock();
        engine.microphone_test_generation.fetch_add(1, Ordering::SeqCst);
        engine.microphone_test_state.store(MicrophoneTestState::Off.raw(), Ordering::SeqCst);
        slot.take()
    };
    MICROPHONE_TEST_HUSHES_CALL.store(false, Ordering::SeqCst);
    if let Some(test) = test {
        engine.runtime.spawn_blocking(move || drop(test));
    }
}

fn open_microphone_test(engine: &'static Engine, host_id: cpal::HostId, host: HostOs) -> Result<MicrophoneTest, RtcUnavailable> {
    let settings = engine.settings.lock().clone();
    let capacity = PlayoutMixer::capacity_for(PLAYOUT_RATE, PLAYOUT_CHANNELS, MICROPHONE_TEST_QUEUE_MS);
    let mixer = Arc::new(Mutex::new(PlayoutMixer::new(capacity)));
    mixer.lock().add_track(MICROPHONE_TEST_TRACK, None);
    let playout = Arc::new(Mutex::new(RemotePlayout::default()));
    let output = DeviceThread::open_output(host_id, settings.output.clone(), mixer.clone(), playout, PlayoutRole::MicrophoneTest)?;
    let (blocks, receiver) = mpsc::channel::<Vec<f32>>(INPUT_BLOCKS);
    let (input, rate, channels) = DeviceThread::open_input(host_id, settings.input.clone(), blocks)?;
    let gate = Arc::new(Mutex::new(InputNoiseGate::new(settings.processing.input_sensitivity)));
    gate.lock().initialize(MICROPHONE_MONITOR_RATE);
    let worker_gate = gate.clone();
    let worker = std::thread::Builder::new()
        .name("rtc-microphone-test".to_owned())
        .spawn(move || run_microphone_test(engine, receiver, rate, channels, worker_gate, mixer, host))
        .map_err(|_| RtcUnavailable::Device {
            direction: DeviceDirection::Input,
            reason: "the microphone test thread did not start",
        })?;
    Ok(MicrophoneTest {
        gate,
        input: Some(input),
        worker: Some(worker),
        output: Some(output),
    })
}

fn run_microphone_test(
    engine: &'static Engine,
    mut blocks: mpsc::Receiver<Vec<f32>>,
    rate: u32,
    channels: u16,
    gate: Arc<Mutex<InputNoiseGate>>,
    mixer: Arc<Mutex<PlayoutMixer>>,
    host: HostOs,
) {
    let monitor_rate = MICROPHONE_MONITOR_RATE as i32;
    let mut monitor = MicrophoneMonitor::new(rate, channels);
    let mut chain: Option<(AudioProcessingOptions, AudioProcessingModule)> = None;
    let mut frame: Vec<i16> = Vec::with_capacity(MicrophoneMonitor::FRAME_SAMPLES);
    let mut played: Vec<i16> = Vec::with_capacity(MicrophoneMonitor::FRAME_SAMPLES);
    while let Some(block) = blocks.blocking_recv() {
        monitor.push(&block);
        while let Some(mut samples) = monitor.next_frame() {
            gate.lock().process(&mut samples, 1);
            let options = engine.settings.lock().processing.options_for(host);
            if chain.as_ref().is_none_or(|(applied, _)| *applied != options) {
                let module = AudioProcessingModule::new(
                    options.echo_cancellation,
                    options.auto_gain_control,
                    options.highpass_filter,
                    options.noise_suppression,
                );
                chain = Some((options, module));
                played.clear();
            }
            frame.clear();
            frame.extend(samples.iter().map(|sample| sample.clamp(-GATE_FULL_SCALE, GATE_FULL_SCALE - 1.0) as i16));
            if let Some((_, module)) = chain.as_mut() {
                if !played.is_empty() {
                    if let Err(error) = module.process_reverse_stream(&mut played, monitor_rate, 1) {
                        tracing::debug!(%error, "rtc microphone test far end was refused");
                    }
                }
                if let Err(error) = module.process_stream(&mut frame, monitor_rate, 1) {
                    tracing::debug!(%error, "rtc microphone test frame was refused");
                }
            }
            played.clear();
            played.extend_from_slice(&frame);
            let stereo = MicrophoneMonitor::to_stereo(&frame);
            mixer.lock().push_i16(MICROPHONE_TEST_TRACK, &stereo);
        }
    }
}

/// devices of one direction on the branch audio host
/// cpal 0.16 has no stable device id so the name is the id
/// a renamed device reads as unplugged and the call falls back to default
pub fn devices(os: &dyn RtcMediaEngine, direction: DeviceDirection) -> Result<Vec<AudioDevice>, RtcUnavailable> {
    let host = cpal::host_from_id(os.audio_host_id()).map_err(|_| RtcUnavailable::Device {
        direction,
        reason: "the audio host is not available on this machine",
    })?;
    let (_, described) = listed(&host, direction).ok_or(RtcUnavailable::Device {
        direction,
        reason: "the audio host did not list its devices",
    })?;
    Ok(described)
}

/// host devices of one direction, plus the same list as the seam describes it
fn listed(host: &cpal::Host, direction: DeviceDirection) -> Option<(Vec<cpal::Device>, Vec<AudioDevice>)> {
    let (devices, default): (Vec<cpal::Device>, Option<cpal::Device>) = match direction {
        DeviceDirection::Input => (host.input_devices().ok()?.collect(), host.default_input_device()),
        DeviceDirection::Output => (host.output_devices().ok()?.collect(), host.default_output_device()),
    };
    let default_name = default.and_then(|device| device.name().ok());
    let mut kept = Vec::with_capacity(devices.len());
    let mut described = Vec::with_capacity(devices.len());
    for device in devices {
        let Ok(name) = device.name() else {
            continue;
        };
        described.push(AudioDevice {
            id: name.clone(),
            is_default: default_name.as_deref() == Some(name.as_str()),
            name,
            direction,
        });
        kept.push(device);
    }
    Some((kept, described))
}

fn pick_device(host: &cpal::Host, direction: DeviceDirection, chosen: Option<&str>) -> Option<cpal::Device> {
    let (devices, described) = listed(host, direction)?;
    let id = resolve_device(&described, chosen)?.id.clone();
    let index = described.iter().position(|device| device.id == id)?;
    devices.into_iter().nth(index)
}

/// cpal stream lives on its own thread since some hosts dont let it leave the thread that built it
/// dropping this stops the stream and joins the thread
struct DeviceThread {
    stop: std::sync::mpsc::Sender<()>,
    join: Option<std::thread::JoinHandle<()>>,
    sample_rate: u32,
    channels: u16,
}

impl Drop for DeviceThread {
    fn drop(&mut self) {
        if self.stop.send(()).is_err() {
            tracing::debug!("rtc audio device thread already stopped");
        }
        if let Some(join) = self.join.take() {
            if join.join().is_err() {
                tracing::warn!("rtc audio device thread panicked");
            }
        }
    }
}

type Opened = Result<(u32, u16), RtcUnavailable>;

impl DeviceThread {
    fn spawn<F>(name: &str, direction: DeviceDirection, build: F) -> Result<Self, RtcUnavailable>
    where
        F: FnOnce() -> Result<(cpal::Stream, u32, u16), RtcUnavailable> + Send + 'static,
    {
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let (ready, opened) = std::sync::mpsc::sync_channel::<Opened>(1);
        let join = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let built = build();
                let stream = match built {
                    Ok((stream, rate, channels)) => {
                        if ready.send(Ok((rate, channels))).is_err() {
                            return;
                        }
                        stream
                    }
                    Err(reason) => {
                        if ready.send(Err(reason)).is_err() {
                            tracing::debug!("rtc audio device opener is gone");
                        }
                        return;
                    }
                };
                if stopped.recv().is_err() {
                    tracing::debug!("rtc audio device owner is gone");
                }
                drop(stream);
            })
            .map_err(|_| RtcUnavailable::Device {
                direction,
                reason: "the audio device thread did not start",
            })?;
        let answer = opened.recv().unwrap_or(Err(RtcUnavailable::Device {
            direction,
            reason: "the audio device thread ended before it opened the stream",
        }));
        match answer {
            Ok((sample_rate, channels)) => Ok(Self {
                stop,
                join: Some(join),
                sample_rate,
                channels,
            }),
            Err(reason) => {
                if join.join().is_err() {
                    tracing::warn!("rtc audio device thread panicked");
                }
                Err(reason)
            }
        }
    }

    fn open_input(host_id: cpal::HostId, chosen: Option<String>, blocks: mpsc::Sender<Vec<f32>>) -> Result<(Self, u32, u16), RtcUnavailable> {
        if crate::audio_capture::uses_tester_signal() {
            return Self::open_tester_input(blocks);
        }
        let thread = Self::spawn("rtc-microphone", DeviceDirection::Input, move || {
            let direction = DeviceDirection::Input;
            let host = cpal::host_from_id(host_id).map_err(|_| RtcUnavailable::Device {
                direction,
                reason: "the audio host is not available on this machine",
            })?;
            let device = pick_device(&host, direction, chosen.as_deref()).ok_or(RtcUnavailable::Device {
                direction,
                reason: "no microphone is connected",
            })?;
            let supported = device.default_input_config().map_err(|_| RtcUnavailable::Device {
                direction,
                reason: "the microphone has no usable format",
            })?;
            let config = supported.config();
            let stream = match supported.sample_format() {
                cpal::SampleFormat::F32 => build_input::<f32>(&device, &config, blocks),
                cpal::SampleFormat::I16 => build_input::<i16>(&device, &config, blocks),
                cpal::SampleFormat::I32 => build_input::<i32>(&device, &config, blocks),
                _ => Err(cpal::BuildStreamError::StreamConfigNotSupported),
            }
            .map_err(|_| RtcUnavailable::Device {
                direction,
                reason: "the microphone stream did not open",
            })?;
            stream.play().map_err(|_| RtcUnavailable::Device {
                direction,
                reason: "the microphone stream did not start",
            })?;
            Ok((stream, config.sample_rate.0, config.channels))
        })?;
        let (rate, channels) = (thread.sample_rate, thread.channels);
        Ok((thread, rate, channels))
    }

    fn open_tester_input(blocks: mpsc::Sender<Vec<f32>>) -> Result<(Self, u32, u16), RtcUnavailable> {
        let format = crate::audio_capture::TESTER_SIGNAL_FORMAT;
        let (rate, channels) = (format.sample_rate, format.channels);
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let join = std::thread::Builder::new()
            .name("rtc-tester-microphone".to_owned())
            .spawn(move || {
                let started = RiseClock::monotonic();
                let mut written: u64 = 0;
                while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = stopped.recv_timeout(TESTER_INPUT_TICK) {
                    let elapsed = RiseClock::monotonic().saturating_sub(started);
                    let due = (elapsed.as_secs_f64() * f64::from(rate)) as u64 * u64::from(channels);
                    if due <= written {
                        continue;
                    }
                    let block: Vec<f32> = (written..due)
                        .map(|index| crate::audio_capture::tester_signal_sample(index / u64::from(channels)) * GATE_FULL_SCALE)
                        .collect();
                    written = due;
                    if let Err(mpsc::error::TrySendError::Closed(_)) = blocks.try_send(block) {
                        return;
                    }
                }
            })
            .map_err(|_| RtcUnavailable::Device {
                direction: DeviceDirection::Input,
                reason: "the tester microphone thread did not start",
            })?;
        Ok((
            Self {
                stop,
                join: Some(join),
                sample_rate: rate,
                channels,
            },
            rate,
            channels,
        ))
    }

    fn open_output(
        host_id: cpal::HostId,
        chosen: Option<String>,
        mixer: Arc<Mutex<PlayoutMixer>>,
        playout: Arc<Mutex<RemotePlayout>>,
        role: PlayoutRole,
    ) -> Result<Self, RtcUnavailable> {
        Self::spawn("rtc-playout", DeviceDirection::Output, move || {
            let direction = DeviceDirection::Output;
            let host = cpal::host_from_id(host_id).map_err(|_| RtcUnavailable::Device {
                direction,
                reason: "the audio host is not available on this machine",
            })?;
            let device = pick_device(&host, direction, chosen.as_deref()).ok_or(RtcUnavailable::Device {
                direction,
                reason: "no speaker or headset is connected",
            })?;
            let format = device
                .supported_output_configs()
                .map_err(|_| RtcUnavailable::Device {
                    direction,
                    reason: "the output device did not list its formats",
                })?
                .filter(|range| range.channels() == PLAYOUT_CHANNELS)
                .filter(|range| range.min_sample_rate().0 <= PLAYOUT_RATE && PLAYOUT_RATE <= range.max_sample_rate().0)
                .map(|range| range.sample_format())
                .find(|format| matches!(format, cpal::SampleFormat::F32 | cpal::SampleFormat::I16 | cpal::SampleFormat::I32))
                .ok_or(RtcUnavailable::Device {
                    direction,
                    reason: "the output device does not play 48 kHz stereo",
                })?;
            let config = cpal::StreamConfig {
                channels: PLAYOUT_CHANNELS,
                sample_rate: cpal::SampleRate(PLAYOUT_RATE),
                buffer_size: cpal::BufferSize::Default,
            };
            let stream = match format {
                cpal::SampleFormat::F32 => build_output::<f32>(&device, &config, mixer, playout, role),
                cpal::SampleFormat::I16 => build_output::<i16>(&device, &config, mixer, playout, role),
                cpal::SampleFormat::I32 => build_output::<i32>(&device, &config, mixer, playout, role),
                _ => Err(cpal::BuildStreamError::StreamConfigNotSupported),
            }
            .map_err(|_| RtcUnavailable::Device {
                direction,
                reason: "the output stream did not open",
            })?;
            stream.play().map_err(|_| RtcUnavailable::Device {
                direction,
                reason: "the output stream did not start",
            })?;
            Ok((stream, config.sample_rate.0, config.channels))
        })
    }
}

/// copies device blocks out of the callback on the i16 scale the gate and sdk use
/// a block the publish task has no room for is counted and dropped, never waited on
fn build_input<T>(device: &cpal::Device, config: &cpal::StreamConfig, blocks: mpsc::Sender<Vec<f32>>) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let dropped = Arc::new(AtomicU64::new(0));
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let block: Vec<f32> = data.iter().map(|sample| sample.to_sample::<f32>() * GATE_FULL_SCALE).collect();
            if blocks.try_send(block).is_err() {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
        },
        |error| tracing::warn!(%error, "rtc microphone stream failed"),
        None,
    )
}

/// output callback mixes under try_lock
/// a block that would wait plays silence instead of stalling the device thread
/// call still drains its mixer while the mic test hushes it so people come back without a backlog
fn build_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mixer: Arc<Mutex<PlayoutMixer>>,
    playout: Arc<Mutex<RemotePlayout>>,
    role: PlayoutRole,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let mut scratch: Vec<f32> = Vec::new();
    device.build_output_stream(
        config,
        move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
            scratch.resize(out.len(), 0.0);
            match (mixer.try_lock(), playout.try_lock()) {
                (Some(mut mixer), Some(playout)) => mixer.mix_into(&mut scratch, &playout),
                _ => scratch.fill(0.0),
            }
            if role == PlayoutRole::Call && MICROPHONE_TEST_HUSHES_CALL.load(Ordering::Relaxed) {
                scratch.fill(0.0);
            }
            crate::audio::sound_mute::silence_if_muted(&mut scratch);
            for (slot, sample) in out.iter_mut().zip(scratch.iter()) {
                *slot = T::from_sample(*sample);
            }
        },
        |error| tracing::warn!(%error, "rtc playout stream failed"),
        None,
    )
}
