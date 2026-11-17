use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use rise_platform::{HostOs, PlatformSupport};
use thiserror::Error;

pub const CODE_PARTICIPANT_REMOVED: &str = "rtc_livekit_participant_removed";
pub const CODE_ROOM_DELETED: &str = "rtc_livekit_room_deleted";
pub const CODE_LINK_LOST: &str = "rtc_media_disconnected";
pub const CODE_TIMED_OUT: &str = "rtc_livekit_timeout";
pub const CODE_NETWORK_FAILED: &str = "rtc_livekit_network_failed";
pub const CODE_CONNECT_FAILED: &str = "rtc_livekit_connect_failed";
pub const CODE_JOIN_FAILED: &str = "rtc_livekit_join_failed";
pub const CODE_TOKEN_REJECTED: &str = "rtc_livekit_token_validation_failed";
pub const CODE_CANCELLED: &str = "rtc_livekit_cancelled";
pub const CODE_INVALID_URL: &str = "rtc_livekit_invalid_url";
pub const CODE_DUPLICATE_IDENTITY: &str = "rtc_livekit_duplicate_identity";
pub const CODE_INSUFFICIENT_PERMISSIONS: &str = "rtc_livekit_insufficient_permissions";
pub const CODE_AUDIO_FAILED: &str = "rtc_livekit_audio_failed";
pub const CODE_IDENTITY_MISMATCH: &str = "rtc_livekit_identity_mismatch";
pub const CODE_PARTICIPANT_IDENTITY_MISSING: &str = "rtc_livekit_participant_identity_missing";
pub const CODE_CONNECT_IN_PROGRESS: &str = "rtc_media_connect_in_progress";
pub const CODE_DATA_PUBLISH_DISABLED: &str = "rtc_livekit_data_publish_disabled";
pub const CODE_MISSING_ACTIVE_ROOM: &str = "missing_active_room";
pub const CODE_INVALID_JOIN_CREDENTIALS: &str = "rtc_invalid_join_credentials";
pub const CODE_JOIN_CREDENTIALS_EXPIRED: &str = "rtc_join_credentials_expired";

pub const METADATA_USER_ID_KEYS: [&str; 2] = ["userId", "user_id"];

pub const SPEAKER_FLUSH: Duration = Duration::from_millis(80);
pub const SPEAKER_FLUSH_UNDER_PRESSURE: Duration = Duration::from_millis(140);
pub const SPEAKER_SAMPLE_LIMIT: usize = 16;

pub const SCREEN_SHARE_FIRST_FRAME_WINDOW: Duration = Duration::from_secs(10);
pub const SCREEN_SHARE_TEARDOWN_ATTEMPTS: u32 = 30;

pub const FREE_MAX_HEIGHT: u32 = 720;
pub const FREE_MAX_FPS: u32 = 30;
pub const CAMERA_PREMIUM_MAX_KBPS: u64 = 4_500;
pub const SCREEN_SHARE_PREMIUM_MAX_KBPS: u64 = 4_500;
pub const SCREEN_SHARE_PREMIUM_1440_MAX_KBPS: u64 = 6_000;


/// which call an intent is about
/// every intent but connect carries it, so a late intent of an ended call never touches the next
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MediaSessionKey {
    pub room_id: String,
    pub connection_id: String,
    pub participant_identity: String,
}

/// what Room connect takes
/// the token never reaches a log since Debug redacts it
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectRequest {
    pub room_id: String,
    pub connection_id: String,
    pub server_url: String,
    pub access_token: String,
    pub participant_identity: String,
    pub video_ceiling: VideoCeiling,
    pub audio_bitrate_kbps: Option<i64>,
    pub end_to_end_encrypted: bool,
    pub expires_at_ms: i64,
}

impl ConnectRequest {
    pub fn session_key(&self) -> MediaSessionKey {
        MediaSessionKey {
            room_id: self.room_id.clone(),
            connection_id: self.connection_id.clone(),
            participant_identity: self.participant_identity.clone(),
        }
    }
}

/// mic track encoder ceiling in bits per second for a room bitrate in kbps
/// none for a room without one or a negative value, sdk default stays
pub fn microphone_max_bitrate(audio_bitrate_kbps: Option<i64>) -> Option<u64> {
    let kbps = u64::try_from(audio_bitrate_kbps?).ok()?;
    Some(kbps.saturating_mul(1_000))
}

impl fmt::Debug for ConnectRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectRequest")
            .field("room", &self.room_id)
            .field("server", &sanitized_endpoint(&self.server_url))
            .field("identity", &self.participant_identity)
            .field("ceiling", &self.video_ceiling)
            .field("audio_bitrate_kbps", &self.audio_bitrate_kbps)
            .field("encrypted", &self.end_to_end_encrypted)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("access_token", &"<redacted>")
            .finish()
    }
}

/// scheme://host[:port] of a server url for logs or invalid, path and query and credentials never pass
pub fn sanitized_endpoint(value: &str) -> String {
    let Some((scheme, rest)) = value.split_once("://") else {
        return "invalid".to_owned();
    };
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
        return "invalid".to_owned();
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, tail)| tail);
    let (host, port) = split_host_port(host_port);
    if host.is_empty() {
        return "invalid".to_owned();
    }
    match port {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    }
}

fn split_host_port(value: &str) -> (&str, Option<u16>) {
    if let Some(rest) = value.strip_prefix('[') {
        return match rest.split_once(']') {
            Some((_, tail)) => {
                let host = &value[..value.len() - tail.len()];
                (host, tail.strip_prefix(':').and_then(|port| port.parse().ok()))
            }
            None => ("", None),
        };
    }
    match value.rsplit_once(':') {
        Some((host, port)) => match port.parse::<u16>() {
            Ok(port) => (host, Some(port)),
            Err(_) => ("", None),
        },
        None => (value, None),
    }
}

/// sdk failure kinds the call tells apart, the branch maps the sdk value to one of these
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SdkFailure {
    Cancelled,
    TimedOut,
    InvalidUrl,
    Network,
    TokenRejected,
    DuplicateIdentity,
    ParticipantRemoved,
    RoomDeleted,
    JoinFailure,
    InsufficientPermissions,
    Audio,
    Other,
}

/// code a failed connect reports
pub const fn failure_code(failure: SdkFailure) -> &'static str {
    match failure {
        SdkFailure::Cancelled => CODE_CANCELLED,
        SdkFailure::TimedOut => CODE_TIMED_OUT,
        SdkFailure::InvalidUrl => CODE_INVALID_URL,
        SdkFailure::Network => CODE_NETWORK_FAILED,
        SdkFailure::TokenRejected => CODE_TOKEN_REJECTED,
        SdkFailure::DuplicateIdentity => CODE_DUPLICATE_IDENTITY,
        SdkFailure::ParticipantRemoved => CODE_PARTICIPANT_REMOVED,
        SdkFailure::RoomDeleted => CODE_ROOM_DELETED,
        SdkFailure::JoinFailure => CODE_JOIN_FAILED,
        SdkFailure::InsufficientPermissions => CODE_INSUFFICIENT_PERMISSIONS,
        SdkFailure::Audio => CODE_AUDIO_FAILED,
        SdkFailure::Other => CODE_CONNECT_FAILED,
    }
}

/// code a dropped room reports, no error at all means lost link
pub const fn disconnect_code(failure: Option<SdkFailure>) -> &'static str {
    match failure {
        Some(failure) => failure_code(failure),
        None => CODE_LINK_LOST,
    }
}

/// where a track comes from
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TrackSource {
    Microphone,
    Camera,
    ScreenShare,
    ScreenShareAudio,
    Unknown,
}

/// one publication of a participant as track flags read it, kind and source and muted
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PublishedTrack {
    pub audio: bool,
    pub source: TrackSource,
    pub muted: bool,
}

/// remote participant as the room reports it
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RemoteParticipant {
    pub identity: String,
    pub name: String,
    pub metadata: String,
    pub microphone_enabled: bool,
    pub camera_enabled: bool,
    pub screen_share_enabled: bool,
    pub joined_at_ms: Option<i64>,
}

impl RemoteParticipant {
    /// participant with the track flags its publications give
    pub fn new(
        identity: String,
        name: String,
        metadata: String,
        publications: impl IntoIterator<Item = PublishedTrack>,
        joined_at_ms: Option<i64>,
    ) -> Self {
        let mut participant = Self {
            identity,
            name,
            metadata,
            microphone_enabled: false,
            camera_enabled: false,
            screen_share_enabled: false,
            joined_at_ms,
        };
        for track in publications.into_iter().filter(|track| !track.muted) {
            if track.audio {
                participant.microphone_enabled = true;
            } else if track.source == TrackSource::Camera {
                participant.camera_enabled = true;
            } else if track.source == TrackSource::ScreenShare {
                participant.screen_share_enabled = true;
            }
        }
        participant
    }

    /// user behind this identity, metadata userId or user_id else the identity
    pub fn user_id(&self) -> Option<String> {
        user_id_for(&self.metadata, &self.identity)
    }
}

/// user id from metadata first, then a non empty identity
pub fn user_id_for(metadata: &str, identity: &str) -> Option<String> {
    if let Ok(serde_json::Value::Object(object)) = serde_json::from_str::<serde_json::Value>(metadata) {
        let found = METADATA_USER_ID_KEYS
            .iter()
            .filter_map(|key| object.get(*key).and_then(serde_json::Value::as_str))
            .find(|value| !value.is_empty());
        if let Some(value) = found {
            return Some(value.to_owned());
        }
    }
    (!identity.is_empty()).then(|| identity.to_owned())
}

/// what the sdk room event stream tells the controller, reduced to what the call acts on
#[derive(Clone, PartialEq, Debug)]
pub enum RoomCallback {
    Connected { local_identity: String },
    Reconnecting,
    Reconnected,
    ConnectFailed(Option<SdkFailure>),
    Disconnected(Option<SdkFailure>),
    ParticipantJoined(RemoteParticipant),
    ParticipantLeft {
        left: RemoteParticipant,
        remaining: Vec<RemoteParticipant>,
    },
    Data { topic: String, payload: Vec<u8> },
    TracksChanged,
    SpeakersChanged(Vec<(RemoteParticipant, f32)>),
    LocalTrackUnpublished(TrackSource),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionPhase {
    Connecting,
    Connected,
    Reconnecting,
    Ended,
}

/// what the call tells the domain after one callback
#[derive(Clone, PartialEq, Debug)]
pub enum SessionOutput {
    Connected,
    Reconnecting,
    Reconnected,
    Ended { code: &'static str, established: bool },
    ConnectFailureNoted(&'static str),
    ParticipantJoined { user_id: String, participant: RemoteParticipant },
    ParticipantLeft { user_id: String },
    Data { topic: String, payload: Vec<u8> },
    TracksChanged,
    Speakers(Vec<(String, f32)>),
    CheckScreenShareTeardown,
    Failure(&'static str),
}

/// one call media session state without the room object
/// the branch feeds it callbacks in order and acts on what comes out
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MediaSession {
    pub identity: String,
    pub phase: SessionPhase,
    pub did_emit_connected: bool,
}

impl MediaSession {
    pub fn new(request: &ConnectRequest) -> Self {
        Self {
            identity: request.participant_identity.clone(),
            phase: SessionPhase::Connecting,
            did_emit_connected: false,
        }
    }

    pub fn receive(&mut self, callback: RoomCallback) -> Vec<SessionOutput> {
        if self.phase == SessionPhase::Ended {
            return Vec::new();
        }
        match callback {
            RoomCallback::Connected { local_identity } => {
                if local_identity != self.identity {
                    return vec![self.end(CODE_IDENTITY_MISMATCH)];
                }
                if self.did_emit_connected {
                    return Vec::new();
                }
                self.did_emit_connected = true;
                self.phase = SessionPhase::Connected;
                vec![SessionOutput::Connected]
            }
            RoomCallback::Reconnecting => {
                if self.phase == SessionPhase::Reconnecting {
                    return Vec::new();
                }
                self.phase = SessionPhase::Reconnecting;
                vec![SessionOutput::Reconnecting]
            }
            RoomCallback::Reconnected => {
                if self.phase != SessionPhase::Reconnecting {
                    return Vec::new();
                }
                self.phase = SessionPhase::Connected;
                vec![SessionOutput::Reconnected]
            }
            RoomCallback::ConnectFailed(failure) => {
                let code = disconnect_code(failure);
                if self.phase == SessionPhase::Connecting && !self.did_emit_connected {
                    return vec![SessionOutput::ConnectFailureNoted(code)];
                }
                vec![self.end(code)]
            }
            RoomCallback::Disconnected(failure) => vec![self.end(disconnect_code(failure))],
            RoomCallback::ParticipantJoined(participant) => match participant.user_id() {
                Some(user_id) => vec![SessionOutput::ParticipantJoined { user_id, participant }],
                None => vec![SessionOutput::Failure(CODE_PARTICIPANT_IDENTITY_MISSING)],
            },
            RoomCallback::ParticipantLeft { left, remaining } => {
                let Some(user_id) = left.user_id() else {
                    return Vec::new();
                };
                let still_present = remaining
                    .iter()
                    .any(|other| other.identity != left.identity && other.user_id().as_deref() == Some(user_id.as_str()));
                if still_present {
                    return Vec::new();
                }
                vec![SessionOutput::ParticipantLeft { user_id }]
            }
            RoomCallback::Data { topic, payload } => vec![SessionOutput::Data { topic, payload }],
            RoomCallback::TracksChanged => vec![SessionOutput::TracksChanged],
            RoomCallback::SpeakersChanged(speakers) => {
                let samples = speakers
                    .into_iter()
                    .take(SPEAKER_SAMPLE_LIMIT)
                    .filter_map(|(participant, level)| participant.user_id().map(|id| (id, level.clamp(0.0, 1.0))))
                    .collect();
                vec![SessionOutput::Speakers(samples)]
            }
            RoomCallback::LocalTrackUnpublished(TrackSource::ScreenShare) => {
                vec![SessionOutput::CheckScreenShareTeardown]
            }
            RoomCallback::LocalTrackUnpublished(_) => Vec::new(),
        }
    }

    fn end(&mut self, code: &'static str) -> SessionOutput {
        let established = self.did_emit_connected;
        self.phase = SessionPhase::Ended;
        SessionOutput::Ended { code, established }
    }
}

/// how often speaker levels reach the screen
pub const fn speaker_flush_interval(under_pressure: bool) -> Duration {
    if under_pressure {
        SPEAKER_FLUSH_UNDER_PRESSURE
    } else {
        SPEAKER_FLUSH
    }
}

/// local playout of the room, self deafen and per person gain
/// the whole map is replaced not patched
/// reapplied on connect, reconnect and every track change since a new subscription starts at unity gain
#[derive(Clone, PartialEq, Debug, Default)]
pub struct RemotePlayout {
    pub deafened: bool,
    pub gains: HashMap<String, f64>,
}

impl RemotePlayout {
    /// volume a remote audio track of this user plays at
    /// deafen beats every gain, a person turned up to 200% must not be the one exception to i hear nobody
    pub fn volume_for(&self, user_id: Option<&str>) -> f64 {
        if self.deafened {
            return 0.0;
        }
        user_id
            .and_then(|id| self.gains.get(id))
            .copied()
            .filter(|gain| gain.is_finite() && *gain >= 0.0)
            .unwrap_or(1.0)
    }

    /// whether participant metadata has to be read at all, an ordinary room moves nobody
    pub fn resolves_per_listener(&self) -> bool {
        !self.deafened && !self.gains.is_empty()
    }
}

/// how large and how fast this publisher may send, from the join answer
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VideoCeiling {
    pub max_height: u32,
    pub max_fps: u32,
}

impl VideoCeiling {
    pub const FREE: Self = Self {
        max_height: FREE_MAX_HEIGHT,
        max_fps: FREE_MAX_FPS,
    };

    /// an older server that sends none or nonsense leaves the free edge
    pub fn from_wire(height: Option<u32>, fps: Option<u32>) -> Self {
        match (height, fps) {
            (Some(max_height), Some(max_fps)) if max_height > 0 && max_fps > 0 => Self { max_height, max_fps },
            _ => Self::FREE,
        }
    }

    pub const fn admits(self, height: u32, fps: u32) -> bool {
        height <= self.max_height && fps <= self.max_fps
    }

    /// under power or thermal pressure the frame rate drops to the free edge, resolution stays
    pub fn under_pressure(self, pressured: bool) -> Self {
        if !pressured {
            return self;
        }
        Self {
            max_height: self.max_height,
            max_fps: self.max_fps.min(FREE_MAX_FPS),
        }
    }
}

/// livekit video preset, 16:9 dimensions and the sdk default encoding for them
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VideoPreset {
    pub width: u32,
    pub height: u32,
    pub max_bitrate: u64,
    pub max_fps: u32,
}

pub const PRESET_H360: VideoPreset = VideoPreset { width: 640, height: 360, max_bitrate: 450_000, max_fps: 20 };
pub const PRESET_H540: VideoPreset = VideoPreset { width: 960, height: 540, max_bitrate: 800_000, max_fps: 25 };
pub const PRESET_H720: VideoPreset = VideoPreset { width: 1280, height: 720, max_bitrate: 1_700_000, max_fps: 30 };
pub const PRESET_H1080: VideoPreset = VideoPreset { width: 1920, height: 1080, max_bitrate: 3_000_000, max_fps: 30 };
pub const PRESET_H1440: VideoPreset = VideoPreset { width: 2560, height: 1440, max_bitrate: 5_000_000, max_fps: 30 };

pub const STANDARD_PRESETS: [VideoPreset; 3] = [PRESET_H720, PRESET_H540, PRESET_H360];
pub const CAMERA_PRESETS: [VideoPreset; 4] = [PRESET_H1080, PRESET_H720, PRESET_H540, PRESET_H360];

/// explicit encoder ceiling for a publication, without one the sdk picks its own
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VideoEncoding {
    pub max_bitrate: u64,
    pub max_fps: u32,
}

/// what a camera or screen share captures and how hard the encoder may push it
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VideoShelf {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub encoding: Option<VideoEncoding>,
}

fn tallest_under(presets: &[VideoPreset], ceiling: VideoCeiling) -> VideoPreset {
    presets
        .iter()
        .copied()
        .find(|preset| preset.height <= ceiling.max_height)
        .unwrap_or(PRESET_H360)
}

/// tallest camera preset under the ceiling
/// only 1080p runs above its preset frame rate and it needs an explicit encoder ceiling
pub fn camera_shelf(ceiling: VideoCeiling) -> VideoShelf {
    let preset = tallest_under(&CAMERA_PRESETS, ceiling);
    if preset != PRESET_H1080 || ceiling.max_fps <= preset.max_fps {
        return VideoShelf {
            width: preset.width,
            height: preset.height,
            fps: ceiling.max_fps.min(preset.max_fps),
            encoding: None,
        };
    }
    VideoShelf {
        width: preset.width,
        height: preset.height,
        fps: ceiling.max_fps,
        encoding: Some(VideoEncoding {
            max_bitrate: CAMERA_PREMIUM_MAX_KBPS * 1_000,
            max_fps: ceiling.max_fps,
        }),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScreenShareQuality {
    Standard,
    High,
}

/// screen share shelf by quality, ceiling and the captured display native pixel size
/// every shelf has an explicit encoder ceiling
/// left alone the sdk screen preset runs 720p at 5 fps
pub fn screen_share_shelf(quality: ScreenShareQuality, ceiling: VideoCeiling, display: (u32, u32)) -> VideoShelf {
    if quality == ScreenShareQuality::High {
        let long_side = display.0.max(display.1);
        let short_side = display.0.min(display.1);
        let wide = PRESET_H1440;
        if long_side >= wide.width && short_side >= wide.height && ceiling.admits(wide.height, wide.max_fps) {
            return VideoShelf {
                width: wide.width,
                height: wide.height,
                fps: wide.max_fps,
                encoding: Some(VideoEncoding {
                    max_bitrate: SCREEN_SHARE_PREMIUM_1440_MAX_KBPS * 1_000,
                    max_fps: wide.max_fps,
                }),
            };
        }
        if PRESET_H1080.height <= ceiling.max_height {
            return VideoShelf {
                width: PRESET_H1080.width,
                height: PRESET_H1080.height,
                fps: ceiling.max_fps,
                encoding: Some(VideoEncoding {
                    max_bitrate: SCREEN_SHARE_PREMIUM_MAX_KBPS * 1_000,
                    max_fps: ceiling.max_fps,
                }),
            };
        }
    }
    let standard = tallest_under(&STANDARD_PRESETS, ceiling);
    let fps = standard.max_fps.min(ceiling.max_fps);
    VideoShelf {
        width: standard.width,
        height: standard.height,
        fps,
        encoding: Some(VideoEncoding {
            max_bitrate: standard.max_bitrate,
            max_fps: fps,
        }),
    }
}

/// whether a published screen share is alive
/// the test is no frame at all and never no recent frame since a still screen sends nothing
/// caller passes the time since publish
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScreenShareLiveness {
    Alive,
    Waiting,
    DeadPipe,
}

pub fn screen_share_liveness(frames_seen: u64, since_publish: Duration) -> ScreenShareLiveness {
    if frames_seen > 0 {
        ScreenShareLiveness::Alive
    } else if since_publish < SCREEN_SHARE_FIRST_FRAME_WINDOW {
        ScreenShareLiveness::Waiting
    } else {
        ScreenShareLiveness::DeadPipe
    }
}

/// who runs a part of the voice processing chain
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessingMode {
    Platform,
    Software,
    Automatic,
}

/// stored voice settings
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AudioProcessingSettings {
    pub software_noise_suppression: bool,
    pub input_sensitivity: f64,
}

impl Default for AudioProcessingSettings {
    fn default() -> Self {
        Self {
            software_noise_suppression: false,
            input_sensitivity: 0.0,
        }
    }
}

/// what the sdk is asked to run on the microphone
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AudioProcessingOptions {
    pub echo_cancellation: bool,
    pub auto_gain_control: bool,
    pub noise_suppression: bool,
    pub highpass_filter: bool,
    pub echo_cancellation_mode: ProcessingMode,
    pub auto_gain_control_mode: ProcessingMode,
    pub noise_suppression_mode: ProcessingMode,
}

pub const fn has_platform_voice_chain(_host: HostOs) -> bool {
    false
}

impl AudioProcessingSettings {
    pub fn normalized(self) -> Self {
        Self {
            software_noise_suppression: self.software_noise_suppression,
            input_sensitivity: normalize_sensitivity(self.input_sensitivity),
        }
    }

    /// processing chain on this host
    /// the toggle picks whose chain runs, never whether one does
    pub fn options_for(self, host: HostOs) -> AudioProcessingOptions {
        let software = self.software_noise_suppression || !has_platform_voice_chain(host);
        let pair = if software { ProcessingMode::Software } else { ProcessingMode::Platform };
        AudioProcessingOptions {
            echo_cancellation: true,
            auto_gain_control: true,
            noise_suppression: true,
            highpass_filter: self.software_noise_suppression,
            echo_cancellation_mode: pair,
            auto_gain_control_mode: pair,
            noise_suppression_mode: if software { ProcessingMode::Software } else { ProcessingMode::Automatic },
        }
    }
}

/// slider value clamped to 0..=1, NaN is the open gate
pub fn normalize_sensitivity(value: f64) -> f64 {
    if value.is_nan() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

pub const SENSITIVITY_EPSILON: f64 = 0.001;

pub const GATE_ENGAGED_ABOVE: f64 = 0.005;
pub const GATE_FULL_SCALE: f32 = 32_768.0;
pub const GATE_QUIETEST_THRESHOLD_DB: f32 = -62.0;
pub const GATE_THRESHOLD_SPAN_DB: f32 = 47.0;
pub const GATE_HOLD_SECS: f32 = 0.25;
pub const GATE_RELEASE_SECS: f32 = 0.15;
pub const GATE_METER_FALL_DB_PER_SEC: f32 = 40.0;
pub const GATE_SPEAKING_FLOOR_DB: f32 = -50.0;
pub const GATE_STALE_SECS: f64 = 0.3;
pub const GATE_CLOSED_ABOVE: f64 = 0.995;

/// true if this slider position engages the gate
pub fn gate_engaged(sensitivity: f64) -> bool {
    sensitivity > GATE_ENGAGED_ABOVE
}

/// gate open threshold in dBFS for a slider position, none when the gate is off
/// right end of the slider is an infinite threshold so nothing passes
pub fn gate_threshold_db(sensitivity: f64) -> Option<f32> {
    let clamped = normalize_sensitivity(sensitivity);
    if clamped >= GATE_CLOSED_ABOVE {
        return Some(f32::INFINITY);
    }
    let position = clamped as f32;
    gate_engaged(clamped).then(|| GATE_QUIETEST_THRESHOLD_DB + position * GATE_THRESHOLD_SPAN_DB)
}

/// rms level of an i16 scale block in dBFS, floored at -180
pub fn level_db(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return -180.0;
    }
    let mean_square = samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32;
    let rms = mean_square.sqrt() / GATE_FULL_SCALE;
    20.0 * rms.max(1e-9).log10()
}

/// mic indicator level 0..=1 for a block
/// -60 dBFS and below is empty, 0 is full
pub fn indicator_level(samples: &[f32]) -> f32 {
    ((level_db(samples) + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// level gate on the published mic signal after webrtc capture processing
/// the branch runs it on every 10 ms block before it reaches the sdk audio source
#[derive(Clone, Debug)]
pub struct InputNoiseGate {
    threshold_db: Option<f32>,
    sample_rate: u32,
    is_open: bool,
    hold_remaining: f32,
    gain: f32,
    meter_db: f32,
    meter_at: Option<Instant>,
}

impl Default for InputNoiseGate {
    fn default() -> Self {
        Self {
            threshold_db: None,
            sample_rate: 0,
            is_open: false,
            hold_remaining: 0.0,
            gain: 0.0,
            meter_db: GATE_QUIETEST_THRESHOLD_DB,
            meter_at: None,
        }
    }
}

impl InputNoiseGate {
    pub fn new(sensitivity: f64) -> Self {
        let mut gate = Self::default();
        gate.update(sensitivity);
        gate
    }

    pub fn update(&mut self, sensitivity: f64) {
        self.threshold_db = gate_threshold_db(sensitivity);
    }

    pub fn initialize(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
        self.close();
    }

    pub fn release(&mut self) {
        self.close();
    }

    pub const fn is_open(&self) -> bool {
        self.is_open
    }

    pub const fn is_engaged(&self) -> bool {
        self.threshold_db.is_some()
    }

    fn close(&mut self) {
        self.is_open = false;
        self.hold_remaining = 0.0;
        self.gain = 0.0;
        self.meter_db = GATE_QUIETEST_THRESHOLD_DB;
        self.meter_at = None;
    }

    /// fall is counted from the wall clock
    /// a muted mic may stop the blocks and the last loud frame must not stay on the track
    fn meter(&self, now: Instant) -> f32 {
        let elapsed = self.meter_at.map_or(0.0, |at| now.saturating_duration_since(at).as_secs_f32());
        self.meter_db - elapsed * GATE_METER_FALL_DB_PER_SEC
    }

    pub fn meter_level(&self, now: Instant) -> f64 {
        f64::from(((self.meter(now) - GATE_QUIETEST_THRESHOLD_DB) / GATE_THRESHOLD_SPAN_DB).clamp(0.0, 1.0))
    }

    pub fn is_locally_speaking(&self, now: Instant) -> bool {
        let Some(at) = self.meter_at else {
            return false;
        };
        if now.saturating_duration_since(at).as_secs_f64() >= GATE_STALE_SECS {
            return false;
        }
        match self.threshold_db {
            Some(threshold) => self.is_open && threshold.is_finite(),
            None => self.meter(now) > GATE_SPEAKING_FLOOR_DB,
        }
    }

    /// gates one interleaved block in place, samples on the i16 scale
    pub fn process(&mut self, samples: &mut [f32], channels: usize) {
        if channels == 0 || samples.is_empty() {
            return;
        }
        let frames = samples.len() / channels;
        if frames == 0 {
            return;
        }
        let level = level_db(samples);
        let now = Instant::now();
        self.meter_db = level.max(self.meter(now));
        self.meter_at = Some(now);
        let Some(threshold) = self.threshold_db else {
            self.is_open = false;
            self.hold_remaining = 0.0;
            self.gain = 0.0;
            return;
        };
        let sample_rate = if self.sample_rate > 0 { self.sample_rate as f32 } else { frames as f32 * 100.0 };
        let seconds = frames as f32 / sample_rate;
        if level >= threshold {
            self.is_open = true;
            self.hold_remaining = GATE_HOLD_SECS;
        } else {
            self.hold_remaining = (self.hold_remaining - seconds).max(0.0);
            if self.hold_remaining == 0.0 {
                self.is_open = false;
            }
        }
        let start = self.gain;
        let end = if self.is_open { 1.0 } else { (start - seconds / GATE_RELEASE_SECS).max(0.0) };
        self.gain = end;
        if start >= 1.0 && end >= 1.0 {
            return;
        }
        if start == 0.0 && end == 0.0 {
            samples.fill(0.0);
            return;
        }
        let step = (end - start) / frames as f32;
        for (frame, block) in samples.chunks_mut(channels).enumerate() {
            let gain = start + step * frame as f32;
            for sample in block {
                *sample *= gain;
            }
        }
    }
}

/// where the mic test in audio settings stands
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MicrophoneTestState {
    Off,
    Starting,
    Running,
    Failed,
}

impl MicrophoneTestState {
    pub const fn raw(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Starting => 1,
            Self::Running => 2,
            Self::Failed => 3,
        }
    }

    pub const fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Starting,
            2 => Self::Running,
            3 => Self::Failed,
            _ => Self::Off,
        }
    }

    /// true when the test holds the mic, the screen keeps its meter and stop button
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }
}

pub const MICROPHONE_MONITOR_RATE: u32 = 48_000;

/// the mic as the test hears it, device blocks folded to one channel and resampled
/// to MICROPHONE_MONITOR_RATE and cut into the 10 ms frames the gate takes, i16 scale
#[derive(Clone, Debug)]
pub struct MicrophoneMonitor {
    channels: usize,
    step: f64,
    position: f64,
    source: std::collections::VecDeque<f32>,
    ready: std::collections::VecDeque<f32>,
}

impl MicrophoneMonitor {
    pub const FRAME_SAMPLES: usize = (MICROPHONE_MONITOR_RATE / 100) as usize;

    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            channels: usize::from(channels.max(1)),
            step: f64::from(sample_rate.max(1)) / f64::from(MICROPHONE_MONITOR_RATE),
            position: 0.0,
            source: std::collections::VecDeque::new(),
            ready: std::collections::VecDeque::new(),
        }
    }

    /// takes one interleaved device block
    pub fn push(&mut self, interleaved: &[f32]) {
        let channels = self.channels;
        for frame in interleaved.chunks_exact(channels) {
            self.source.push_back(frame.iter().sum::<f32>() / channels as f32);
        }
        loop {
            let index = self.position.floor() as usize;
            if index + 1 >= self.source.len() {
                break;
            }
            let fraction = (self.position - index as f64) as f32;
            let (from, to) = (self.source[index], self.source[index + 1]);
            self.ready.push_back(from + (to - from) * fraction);
            self.position += self.step;
        }
        let consumed = (self.position.floor() as usize).min(self.source.len().saturating_sub(1));
        self.source.drain(..consumed);
        self.position -= consumed as f64;
    }

    /// next whole 10 ms frame once one is waiting
    pub fn next_frame(&mut self) -> Option<Vec<f32>> {
        if self.ready.len() < Self::FRAME_SAMPLES {
            return None;
        }
        Some(self.ready.drain(..Self::FRAME_SAMPLES).collect())
    }

    /// one channel to the stereo the output mixer plays
    pub fn to_stereo(mono: &[i16]) -> Vec<i16> {
        mono.iter().flat_map(|sample| [*sample, *sample]).collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceDirection {
    Input,
    Output,
}

/// audio device as the os lists it
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub direction: DeviceDirection,
    pub is_default: bool,
}

/// device a call uses, the chosen one while plugged in
/// else the system default, else the first listed
/// none chosen means follow the system default
pub fn resolve_device<'a>(devices: &'a [AudioDevice], chosen: Option<&str>) -> Option<&'a AudioDevice> {
    chosen
        .and_then(|id| devices.iter().find(|device| device.id == id))
        .or_else(|| devices.iter().find(|device| device.is_default))
        .or_else(|| devices.first())
}

const SYSTEM_CHOICE_DEVICE_NAMES: [&str; 5] = ["default", "pipewire", "pulse", "jack", "oss"];

/// devices a person picks from
/// a call follows the system choice when none is picked
/// so a row for that is the caller job, under a localized title
pub fn pickable_devices(devices: &[AudioDevice]) -> Vec<&AudioDevice> {
    devices.iter().filter(|device| !SYSTEM_CHOICE_DEVICE_NAMES.iter().any(|name| *name == device.name)).collect()
}

/// what changes for a running call when the device list changes
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DeviceSwitch {
    Keep,
    SwitchTo(String),
    NoDevice,
}

pub fn device_switch(devices: &[AudioDevice], chosen: Option<&str>, current: Option<&str>) -> DeviceSwitch {
    match resolve_device(devices, chosen) {
        None => DeviceSwitch::NoDevice,
        Some(device) if Some(device.id.as_str()) == current => DeviceSwitch::Keep,
        Some(device) => DeviceSwitch::SwitchTo(device.id.clone()),
    }
}

/// what the domain asks the media layer, camera chosen by device id instead of front or back
/// every intent of a running call names its session
/// publish ceiling rides on the connect
#[derive(Clone, PartialEq, Debug)]
pub enum MediaIntent {
    Connect(ConnectRequest),
    Disconnect(MediaSessionKey),
    SetMicrophone { session: MediaSessionKey, enabled: bool },
    SetRemotePlayout { session: MediaSessionKey, playout: RemotePlayout },
    SetCamera { session: MediaSessionKey, enabled: bool, camera_id: Option<String> },
    SetEncryptionKey { session: MediaSessionKey, key: Option<String> },
    SetScreenShare {
        session: MediaSessionKey,
        enabled: bool,
        include_system_audio: bool,
        quality: ScreenShareQuality,
        display: (u32, u32),
    },
    SelectAudioInput { session: MediaSessionKey, id: Option<String> },
    SelectAudioOutput { session: MediaSessionKey, id: Option<String> },
    ApplyAudioProcessing(AudioProcessingSettings),
    SendData { session: MediaSessionKey, topic: String, payload: Vec<u8>, reliable: bool },
}

/// why an intent could not be carried out
/// each one is an answer the feature shows, not a silent no op
#[derive(Clone, Copy, PartialEq, Eq, Debug, Error)]
pub enum RtcUnavailable {
    #[error("call media has no runtime: {0}")]
    NoRuntime(&'static str),
    #[error("no call is connected")]
    NotConnected,
    #[error("{direction:?} audio device unavailable: {reason}")]
    Device { direction: DeviceDirection, reason: &'static str },
    #[error("call media refused: {0}")]
    Refused(&'static str),
    #[error("call media intent invalid: {0}")]
    Invalid(&'static str),
}

impl RtcUnavailable {
    /// code the domain maps, where there is one
    pub const fn code(self) -> Option<&'static str> {
        match self {
            Self::NotConnected => Some(CODE_MISSING_ACTIVE_ROOM),
            Self::Refused(code) | Self::Invalid(code) => Some(code),
            Self::NoRuntime(_) | Self::Device { .. } => None,
        }
    }
}

/// session the engine has a room for and how far that room got
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActiveSession<'a> {
    pub key: &'a MediaSessionKey,
    pub phase: SessionPhase,
}

/// an intent acts only on the session that is both intended and active
/// and not while its room is still connecting
pub fn require_session(
    intended: Option<&MediaSessionKey>,
    active: Option<ActiveSession<'_>>,
    key: &MediaSessionKey,
) -> Result<(), RtcUnavailable> {
    match active {
        Some(active)
            if active.key == key
                && intended == Some(key)
                && !matches!(active.phase, SessionPhase::Connecting | SessionPhase::Ended) =>
        {
            Ok(())
        }
        _ => Err(RtcUnavailable::NotConnected),
    }
}

/// guards of connect before any session step
/// every credential field present, then expiry still ahead of now_ms
pub fn validate_connect(request: &ConnectRequest, now_ms: i64) -> Result<(), RtcUnavailable> {
    let fields = [
        &request.room_id,
        &request.connection_id,
        &request.participant_identity,
        &request.server_url,
        &request.access_token,
    ];
    if fields.iter().any(|field| field.is_empty()) {
        return Err(RtcUnavailable::Invalid(CODE_INVALID_JOIN_CREDENTIALS));
    }
    if request.expires_at_ms <= now_ms {
        return Err(RtcUnavailable::Refused(CODE_JOIN_CREDENTIALS_EXPIRED));
    }
    Ok(())
}

/// what a connect does against the session the engine already holds
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnectStep {
    Start,
    AlreadyConnected,
    InProgress,
}

pub fn connect_step(intended: Option<&MediaSessionKey>, active: Option<ActiveSession<'_>>, key: &MediaSessionKey) -> ConnectStep {
    if intended != Some(key) {
        return ConnectStep::Start;
    }
    match active {
        None => ConnectStep::InProgress,
        Some(active) if active.key != key => ConnectStep::InProgress,
        Some(active) => match active.phase {
            SessionPhase::Connected | SessionPhase::Reconnecting => ConnectStep::AlreadyConnected,
            SessionPhase::Connecting => ConnectStep::InProgress,
            SessionPhase::Ended => ConnectStep::Start,
        },
    }
}

/// disconnect and audio route pick act only for the intended or active session
/// so a late intent of an ended call cant reach the next one
pub fn owns_session(intended: Option<&MediaSessionKey>, active: Option<&MediaSessionKey>, key: &MediaSessionKey) -> bool {
    intended == Some(key) || active == Some(key)
}

pub const NO_RUNTIME: &str = "the engine did not install its tokio runtime for calls";

/// os half of the seam
/// room and publishing and mixing are the same livekit code on every host
/// the branch names the audio host whose devices a call opens
pub trait RtcMediaEngine {
    fn audio_host_id(&self) -> cpal::HostId {
        cpal::default_host().id()
    }
}

/// intent name for traces, credentials and payloads stay out
pub const fn intent_name(intent: &MediaIntent) -> &'static str {
    match intent {
        MediaIntent::Connect(_) => "connect",
        MediaIntent::Disconnect(_) => "disconnect",
        MediaIntent::SetMicrophone { .. } => "set_microphone",
        MediaIntent::SetRemotePlayout { .. } => "set_remote_playout",
        MediaIntent::SetCamera { .. } => "set_camera",
        MediaIntent::SetEncryptionKey { .. } => "set_encryption_key",
        MediaIntent::SetScreenShare { .. } => "set_screen_share",
        MediaIntent::SelectAudioInput { .. } => "select_audio_input",
        MediaIntent::SelectAudioOutput { .. } => "select_audio_output",
        MediaIntent::ApplyAudioProcessing(_) => "apply_audio_processing",
        MediaIntent::SendData { .. } => "send_data",
    }
}

/// failure kind of a failed connect or publish, read from the error display text
/// livekit 0.7.32 hides EngineError and SignalError behind a private module so variants cant be matched
/// the thiserror messages stay stable so those get matched
pub fn classify_sdk_error(text: &str) -> SdkFailure {
    let lower = text.to_ascii_lowercase();
    if lower.contains("failed to parse the url") {
        return SdkFailure::InvalidUrl;
    }
    if lower.contains("access token has invalid characters")
        || lower.contains("client error: 401")
        || lower.contains("client error: 403")
    {
        return SdkFailure::TokenRejected;
    }
    if let Some(rest) = lower.split("reason=").nth(1) {
        let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if let Some(failure) = disconnect_failure(&name) {
            return failure;
        }
    }
    if lower.contains("notallowed") || lower.contains("not_allowed") || lower.contains("permission") {
        return SdkFailure::InsufficientPermissions;
    }
    if lower.contains("timeout") || lower.contains("timed out") {
        return SdkFailure::TimedOut;
    }
    if lower.contains("ws failure") || lower.contains("connection error") || lower.contains("signal failure") {
        return SdkFailure::Network;
    }
    if lower.contains("already closed") {
        return SdkFailure::Cancelled;
    }
    SdkFailure::Other
}

/// failure a server DisconnectReason carries, by protobuf name (DUPLICATE_IDENTITY) or rust name
/// none is an ordinary lost link
pub fn disconnect_failure(reason: &str) -> Option<SdkFailure> {
    let key: String = reason.chars().filter(|c| *c != '_').map(|c| c.to_ascii_lowercase()).collect();
    match key.as_str() {
        "clientinitiated" => Some(SdkFailure::Cancelled),
        "duplicateidentity" => Some(SdkFailure::DuplicateIdentity),
        "participantremoved" => Some(SdkFailure::ParticipantRemoved),
        "roomdeleted" | "roomclosed" => Some(SdkFailure::RoomDeleted),
        "joinfailure" => Some(SdkFailure::JoinFailure),
        _ => None,
    }
}

/// remote audio waiting for the output device, one queue per subscribed track
/// mixed at each person playout volume
/// samples are interleaved in the output device own rate and channel count, scale -1 to 1
#[derive(Debug, Default)]
pub struct PlayoutMixer {
    tracks: HashMap<String, MixerTrack>,
    capacity: usize,
}

#[derive(Debug, Default)]
struct MixerTrack {
    user_id: Option<String>,
    queue: std::collections::VecDeque<f32>,
}

impl PlayoutMixer {
    pub fn new(capacity: usize) -> Self {
        Self {
            tracks: HashMap::new(),
            capacity: capacity.max(1),
        }
    }

    /// queue bound for millis of audio in this format
    pub fn capacity_for(sample_rate: u32, channels: u16, millis: u32) -> usize {
        (sample_rate as usize * channels as usize * millis as usize) / 1000
    }

    pub fn add_track(&mut self, key: &str, user_id: Option<String>) {
        self.tracks.insert(
            key.to_owned(),
            MixerTrack {
                user_id,
                queue: std::collections::VecDeque::new(),
            },
        );
    }

    pub fn remove_track(&mut self, key: &str) {
        self.tracks.remove(key);
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn queued(&self, key: &str) -> usize {
        self.tracks.get(key).map_or(0, |track| track.queue.len())
    }

    /// queues i16 samples from the sdk for one track, a track the mixer doesnt know is dropped
    pub fn push_i16(&mut self, key: &str, samples: &[i16]) {
        let capacity = self.capacity;
        let Some(track) = self.tracks.get_mut(key) else {
            return;
        };
        track.queue.extend(samples.iter().map(|sample| f32::from(*sample) / GATE_FULL_SCALE));
        let excess = track.queue.len().saturating_sub(capacity);
        if excess > 0 {
            track.queue.drain(..excess);
        }
    }

    /// fills one output block, every track at its person volume summed and clipped
    /// deafen and per person gain apply here so a fresh subscription is never at unity by mistake
    pub fn mix_into(&mut self, out: &mut [f32], playout: &RemotePlayout) {
        out.fill(0.0);
        for track in self.tracks.values_mut() {
            let volume = playout.volume_for(track.user_id.as_deref()) as f32;
            let take = out.len().min(track.queue.len());
            for (slot, sample) in out.iter_mut().zip(track.queue.drain(..take)) {
                *slot += sample * volume;
            }
        }
        for slot in out.iter_mut() {
            *slot = slot.clamp(-1.0, 1.0);
        }
    }
}

/// whether this host can carry call media now
pub fn availability() -> Result<(), RtcUnavailable> {
    super::livekit_room::availability()
}

/// hands one intent to the room engine
pub fn perform(intent: &MediaIntent) -> Result<(), RtcUnavailable> {
    super::livekit_room::perform(&super::os::NativeRtcEngine, HostOs::current(), intent)
}

pub fn input_meter_level() -> f64 {
    super::livekit_room::input_meter_level()
}

pub fn is_locally_speaking() -> bool {
    super::livekit_room::is_locally_speaking()
}

pub fn start_microphone_test() -> Result<(), RtcUnavailable> {
    super::livekit_room::start_microphone_test(&super::os::NativeRtcEngine, HostOs::current())
}

pub fn stop_microphone_test() {
    super::livekit_room::stop_microphone_test();
}

pub fn microphone_test_state() -> MicrophoneTestState {
    super::livekit_room::microphone_test_state()
}

pub fn set_preferred_audio_devices(input: Option<String>, output: Option<String>) -> Result<(), RtcUnavailable> {
    super::livekit_room::set_preferred_audio_devices(input, output)
}

/// mics the host lists, system default marked
pub fn input_devices() -> Result<Vec<AudioDevice>, RtcUnavailable> {
    super::livekit_room::devices(&super::os::NativeRtcEngine, DeviceDirection::Input)
}

/// speakers and headsets the host lists, system default marked
pub fn output_devices() -> Result<Vec<AudioDevice>, RtcUnavailable> {
    super::livekit_room::devices(&super::os::NativeRtcEngine, DeviceDirection::Output)
}

pub fn support() -> PlatformSupport {
    match availability() {
        Ok(()) => PlatformSupport::Performed,
        Err(reason) => {
            tracing::debug!(%reason, "rtc media unavailable");
            PlatformSupport::Unsupported
        }
    }
}
