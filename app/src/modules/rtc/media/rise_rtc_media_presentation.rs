use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext, Context, Entity, RenderImage, Task, WeakEntity};
use image::imageops::FilterType;
use image::{Frame, RgbaImage};
use rise_canon::rise_key;
use rise_core::foundation::foundation_date::RiseClock;
use rise_media::camera_capture;
use rise_media::rtc_media::TrackSource;
use rise_media::rtc_media::livekit_room::RemoteVideoFrame;
use rise_media::texture::native_surface::{HostBgraFrame, HostNv12Frame, PreparedHostFrame};
use rise_platform::display_sleep::{self, CALL_IN_PROGRESS_REASON, DisplaySleepGuard};
use smallvec::SmallVec;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::rtc::engine::rise_rtc_engine_models::{RiseRTCCameraPosition, RiseRTCMediaSessionKey};
use crate::modules::rtc::media::rise_rtc_audio_settings::RiseRTCInputNoiseGate;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;

/// livekit dimensions of a video track
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

/// livekit track delegate, the part the presentation listens to
pub trait TrackDelegate {
    fn track_did_update_dimensions(&self, track: &Rc<VideoTrack>, dimensions: Option<Dimensions>, cx: &mut App);
}

/// livekit video renderer, sees every picture the track draws on the main thread
pub trait VideoRenderer {
    fn render(&self, frame: &RemoteVideoFrame, cx: &mut App);
}

/// livekit video track as the desktop call holds it
/// the media controller owns each track as Rc while the room publishes it and feeds pictures in
/// screens and the presentation keep only weak refs
pub struct VideoTrack {
    track_key: String,
    source: TrackSource,
    dimensions: Cell<Option<Dimensions>>,
    delegates: RefCell<Vec<Weak<dyn TrackDelegate>>>,
    renderers: RefCell<Vec<Rc<dyn VideoRenderer>>>,
    latest_frame: RefCell<Option<RemoteVideoFrame>>,
}

impl VideoTrack {
    pub fn new(track_key: String, source: TrackSource, dimensions: Option<Dimensions>) -> Rc<Self> {
        Rc::new(Self {
            track_key,
            source,
            dimensions: Cell::new(dimensions),
            delegates: RefCell::new(Vec::new()),
            renderers: RefCell::new(Vec::new()),
            latest_frame: RefCell::new(None),
        })
    }

    pub fn track_key(&self) -> &str {
        &self.track_key
    }

    pub fn source(&self) -> TrackSource {
        self.source
    }

    pub fn dimensions(&self) -> Option<Dimensions> {
        self.dimensions.get()
    }

    /// newest picture for the view that draws this track
    pub fn latest_frame(&self) -> Option<RemoteVideoFrame> {
        self.latest_frame.borrow().clone()
    }

    pub fn add_delegate(&self, delegate: Weak<dyn TrackDelegate>) {
        self.delegates.borrow_mut().push(delegate);
    }

    pub fn remove_delegate(&self, delegate: &Weak<dyn TrackDelegate>) {
        self.delegates
            .borrow_mut()
            .retain(|existing| existing.strong_count() > 0 && !Weak::ptr_eq(existing, delegate));
    }

    pub fn add_video_renderer(&self, renderer: Rc<dyn VideoRenderer>) {
        self.renderers.borrow_mut().push(renderer);
    }

    pub fn remove_video_renderer(&self, renderer: &Rc<dyn VideoRenderer>) {
        self.renderers.borrow_mut().retain(|existing| !Rc::ptr_eq(existing, renderer));
    }

    pub fn update_dimensions(self: &Rc<Self>, dimensions: Option<Dimensions>, cx: &mut App) {
        if self.dimensions.get() == dimensions {
            return;
        }
        self.dimensions.set(dimensions);
        let delegates: Vec<Rc<dyn TrackDelegate>> = self.delegates.borrow().iter().filter_map(Weak::upgrade).collect();
        for delegate in delegates {
            delegate.track_did_update_dimensions(self, dimensions, cx);
        }
    }

    pub fn render(&self, frame: RemoteVideoFrame, cx: &mut App) {
        let renderers: Vec<Rc<dyn VideoRenderer>> = self.renderers.borrow().clone();
        for renderer in renderers {
            renderer.render(&frame, cx);
        }
        self.latest_frame.replace(Some(frame));
    }
}

struct WeakTrack {
    value: Weak<VideoTrack>,
    is_portrait: Cell<bool>,
    on_orientation_change: Rc<dyn Fn(&mut App)>,
}

impl WeakTrack {
    fn new(value: &Rc<VideoTrack>, dimensions: Option<Dimensions>, on_orientation_change: Rc<dyn Fn(&mut App)>) -> Rc<Self> {
        let dimensions = value.dimensions().or(dimensions);
        let is_portrait = dimensions.is_some_and(|dimensions| dimensions.width > 0 && dimensions.width < dimensions.height);
        let track = Rc::new(Self { value: Rc::downgrade(value), is_portrait: Cell::new(is_portrait), on_orientation_change });
        let delegate: Rc<dyn TrackDelegate> = track.clone();
        value.add_delegate(Rc::downgrade(&delegate));
        track
    }

    fn value(&self) -> Option<Rc<VideoTrack>> {
        self.value.upgrade()
    }

    fn is_portrait(&self) -> bool {
        self.is_portrait.get()
    }
}

impl TrackDelegate for WeakTrack {
    fn track_did_update_dimensions(&self, track: &Rc<VideoTrack>, dimensions: Option<Dimensions>, cx: &mut App) {
        let Some(dimensions) = dimensions.filter(|dimensions| dimensions.width > 0 && dimensions.height > 0) else {
            return;
        };
        let is_portrait = dimensions.width < dimensions.height;
        let same_track = self.value.upgrade().is_some_and(|value| Rc::ptr_eq(&value, track));
        if !same_track || self.is_portrait.get() == is_portrait {
            return;
        }
        self.is_portrait.set(is_portrait);
        let on_orientation_change = self.on_orientation_change.clone();
        cx.defer(move |cx| on_orientation_change(cx));
    }
}

impl Drop for WeakTrack {
    fn drop(&mut self) {
        if let Some(value) = self.value.upgrade() {
            value.delegates.borrow_mut().retain(|existing| existing.strong_count() > 0);
        }
    }
}

/// one video publication of a participant as the room reports it
#[derive(Clone)]
pub struct RiseRTCVideoPublication {
    pub source: TrackSource,
    pub is_muted: bool,
    pub track: Option<Rc<VideoTrack>>,
    pub dimensions: Option<Dimensions>,
}

/// participant of the room with identity, metadata and the video it publishes
#[derive(Clone)]
pub struct RiseRTCParticipantVideo {
    pub identity: String,
    pub metadata: String,
    pub video_tracks: Vec<RiseRTCVideoPublication>,
}

/// what replace_tracks reads off the livekit room
#[derive(Clone)]
pub struct RiseRTCRoomVideo {
    pub local_participant: RiseRTCParticipantVideo,
    pub remote_participants: Vec<RiseRTCParticipantVideo>,
    pub local_camera_position: Option<RiseRTCCameraPosition>,
}

pub struct RiseRTCMediaPresentation {
    revision: u64,
    active_session: Option<RiseRTCMediaSessionKey>,
    idle_timer_guard: Option<DisplaySleepGuard>,
    local_speaking: bool,
    local_speaking_task: Option<Task<()>>,
    local_user_id: Option<String>,
    camera_tracks: HashMap<String, Rc<WeakTrack>>,
    screen_share_tracks: HashMap<String, Rc<WeakTrack>>,
    portrait_video_user_ids: HashSet<String>,
    remote_user_ids: HashSet<String>,
    speaking_user_ids: HashSet<String>,
    audio_levels: HashMap<String, f32>,
    active_speaker_user_id: Option<String>,
    last_speaker_user_id: Option<String>,
    local_camera_position: RiseRTCCameraPosition,
    can_flip_camera: bool,
    camera_device_watch: Option<Task<()>>,
}

impl RiseRTCMediaPresentation {
    const AUDIO_LEVEL_EPSILON: f32 = 0.006;
    const LOCAL_SPEAKING_POLL_INTERVAL: Duration = Duration::from_millis(80);
    const CAMERA_DEVICE_WATCH_INTERVAL: Duration = Duration::from_secs(5);

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            revision: 0,
            active_session: None,
            idle_timer_guard: None,
            local_speaking: false,
            local_speaking_task: None,
            local_user_id: None,
            camera_tracks: HashMap::new(),
            screen_share_tracks: HashMap::new(),
            portrait_video_user_ids: HashSet::new(),
            remote_user_ids: HashSet::new(),
            speaking_user_ids: HashSet::new(),
            audio_levels: HashMap::new(),
            active_speaker_user_id: None,
            last_speaker_user_id: None,
            local_camera_position: RiseRTCCameraPosition::Front,
            can_flip_camera: false,
            camera_device_watch: None,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn portrait_video_user_ids(&self) -> &HashSet<String> {
        &self.portrait_video_user_ids
    }

    pub fn remote_user_ids(&self) -> &HashSet<String> {
        &self.remote_user_ids
    }

    pub fn speaking_user_ids(&self) -> &HashSet<String> {
        &self.speaking_user_ids
    }

    pub fn active_speaker_user_id(&self) -> Option<&str> {
        self.active_speaker_user_id.as_deref()
    }

    pub fn last_speaker_user_id(&self) -> Option<&str> {
        self.last_speaker_user_id.as_deref()
    }

    pub fn local_camera_position(&self) -> RiseRTCCameraPosition {
        self.local_camera_position
    }

    /// true when the flip button shows, this device has a second camera to flip to
    pub fn can_flip_camera(&self) -> bool {
        self.can_flip_camera
    }

    pub fn is_idle_timer_disabled(&self) -> bool {
        self.idle_timer_guard.is_some()
    }

    /// display stays awake exactly while a session exists
    /// session scoped reset stops stale cleanup from releasing a new call
    fn set_active_session(&mut self, session: Option<RiseRTCMediaSessionKey>, cx: &mut Context<Self>) {
        let changed = self.active_session.is_some() != session.is_some();
        self.active_session = session;
        if !changed {
            return;
        }
        self.idle_timer_guard =
            self.active_session.as_ref().map(|_| display_sleep::acquire(CALL_IN_PROGRESS_REASON));
        if self.active_session.is_some() {
            self.start_local_speaking_watch(cx);
        } else {
            self.local_speaking_task = None;
            self.local_speaking = false;
        }
    }

    fn start_local_speaking_watch(&mut self, cx: &mut Context<Self>) {
        self.local_speaking_task = Some(cx.spawn(async move |this, cx| {
            loop {
                let speaking = RiseRTCInputNoiseGate::is_locally_speaking();
                let alive = this.update(cx, |this, cx| {
                    let auth = AuthServicesStore::shared(cx);
                    this.local_user_id =
                        auth.read(cx).active_account_id().filter(|id| !id.is_empty()).map(str::to_owned);
                    let speaking = speaking && Self::local_voice_reaches_call(cx);
                    if this.local_speaking != speaking {
                        this.local_speaking = speaking;
                        this.revision = this.revision.wrapping_add(1);
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
                cx.background_executor().timer(Self::LOCAL_SPEAKING_POLL_INTERVAL).await;
            }
        }));
    }

    /// invariant: own speaking shows only while the room lets our mic through
    fn local_voice_reaches_call(cx: &mut App) -> bool {
        let rtc = RTCServicesStore::shared(cx);
        rtc.read(cx).active_room(cx).and_then(|room| room.self_participant.as_ref()).is_none_or(|participant| {
            participant.microphone_enabled
                && !participant.deafened
                && !participant.server_deafened
                && !participant.is_server_silenced()
        })
    }

    pub fn camera_track(&self, user_id: &str) -> Option<Rc<VideoTrack>> {
        self.camera_tracks.get(user_id).and_then(|track| track.value())
    }

    pub fn screen_share_track(&self, user_id: &str) -> Option<Rc<VideoTrack>> {
        self.screen_share_tracks.get(user_id).and_then(|track| track.value())
    }

    pub fn is_speaking(&self, user_id: &str) -> bool {
        if self.local_user_id.as_deref() == Some(user_id) {
            return self.local_speaking;
        }
        self.speaking_user_ids.contains(user_id)
    }

    pub fn audio_level(&self, user_id: &str) -> f32 {
        self.audio_levels.get(user_id).copied().unwrap_or(0.0)
    }

    pub fn screen_share_aspect_ratio(&self, user_id: &str) -> f32 {
        match self.screen_share_track(user_id).and_then(|track| track.dimensions()) {
            Some(dimensions) if dimensions.width > 0 && dimensions.height > 0 => {
                dimensions.width as f32 / dimensions.height as f32
            }
            _ => 16.0 / 9.0,
        }
    }

    pub fn activate(&mut self, session: &RiseRTCMediaSessionKey, cx: &mut Context<Self>) {
        if self.active_session.as_ref() == Some(session) {
            return;
        }
        RiseRTCStreamPreviewStore::shared(cx).update(cx, |store, cx| store.reset(cx));
        self.set_active_session(Some(session.clone()), cx);
        self.camera_tracks = HashMap::new();
        self.screen_share_tracks = HashMap::new();
        self.portrait_video_user_ids = HashSet::new();
        self.remote_user_ids = HashSet::new();
        self.speaking_user_ids = HashSet::new();
        self.audio_levels = HashMap::new();
        self.active_speaker_user_id = None;
        self.last_speaker_user_id = None;
        self.local_camera_position = RiseRTCCameraPosition::Front;
        self.bump(cx);
    }

    pub fn replace_tracks(&mut self, room: &RiseRTCRoomVideo, session: &RiseRTCMediaSessionKey, cx: &mut Context<Self>) {
        if self.active_session.as_ref() != Some(session) {
            tracing::warn!(
                target: "riseonly::X1882",
                room = %session.room_id,
                active = ?self.active_session.as_ref().map(|active| active.room_id.as_str()),
                remotes = room.remote_participants.len(),
                "rtc tracks dropped: presentation holds another session"
            );
            return;
        }
        let mut cameras = HashMap::new();
        let mut screens = HashMap::new();
        let mut remotes = HashSet::new();
        let presentation = cx.weak_entity();
        self.install_tracks(&room.local_participant, &mut cameras, &mut screens, &presentation);
        for participant in &room.remote_participants {
            self.install_tracks(participant, &mut cameras, &mut screens, &presentation);
            if let Some(user_id) = Self::user_id(participant) {
                remotes.insert(user_id);
            }
        }
        self.camera_tracks = cameras;
        self.screen_share_tracks = screens;
        self.update_portrait_video_users();
        if self.remote_user_ids != remotes {
            tracing::warn!(target: "riseonly::X1882", room = %session.room_id, ?remotes, "rtc presence changed");
        }
        self.remote_user_ids = remotes;
        if let Some(position) = room.local_camera_position
            && position != self.local_camera_position
        {
            self.local_camera_position = position;
        }
        self.bump(cx);
    }

    pub fn is_present(&self, user_id: &str) -> bool {
        self.remote_user_ids.contains(user_id)
    }

    pub fn install_practice(
        &mut self,
        session: &RiseRTCMediaSessionKey,
        cameras: &HashMap<String, Rc<VideoTrack>>,
        screens: &HashMap<String, Rc<VideoTrack>>,
        remotes: HashSet<String>,
        camera_position: RiseRTCCameraPosition,
        cx: &mut Context<Self>,
    ) {
        if self.active_session.as_ref() != Some(session) {
            self.activate(session, cx);
        }
        let presentation = cx.weak_entity();
        let camera_tracks = cameras
            .iter()
            .map(|(user_id, track)| {
                (user_id.clone(), Self::practice_track(track, self.camera_tracks.get(user_id), &presentation))
            })
            .collect();
        let screen_share_tracks = screens
            .iter()
            .map(|(user_id, track)| {
                (user_id.clone(), Self::practice_track(track, self.screen_share_tracks.get(user_id), &presentation))
            })
            .collect();
        self.camera_tracks = camera_tracks;
        self.screen_share_tracks = screen_share_tracks;
        self.update_portrait_video_users();
        if self.remote_user_ids != remotes {
            self.remote_user_ids = remotes;
        }
        if self.local_camera_position != camera_position {
            self.local_camera_position = camera_position;
        }
        self.bump(cx);
    }

    fn practice_track(
        track: &Rc<VideoTrack>,
        previous: Option<&Rc<WeakTrack>>,
        presentation: &WeakEntity<Self>,
    ) -> Rc<WeakTrack> {
        if let Some(previous) = previous
            && previous.value().is_some_and(|value| Rc::ptr_eq(&value, track))
        {
            return previous.clone();
        }
        WeakTrack::new(track, track.dimensions(), Self::orientation_callback(presentation.clone()))
    }

    pub fn replace_speaking_samples(
        &mut self,
        samples: &[RiseRTCAudioLevelSample],
        session: &RiseRTCMediaSessionKey,
        cx: &mut Context<Self>,
    ) {
        if self.active_session.as_ref() != Some(session) {
            return;
        }
        let bounded_samples = &samples[..samples.len().min(16)];
        let ordered_user_ids: Vec<&str> = bounded_samples.iter().map(|sample| sample.user_id.as_str()).collect();
        let user_ids: HashSet<String> = ordered_user_ids.iter().map(|user_id| (*user_id).to_owned()).collect();
        let active_user_id = ordered_user_ids.first().map(|user_id| (*user_id).to_owned());
        let mut next_levels = HashMap::with_capacity(bounded_samples.len());
        for sample in bounded_samples {
            let level = sample.level.clamp(0.0, 1.0);
            let previous = self.audio_levels.get(&sample.user_id).copied().unwrap_or(0.0);
            next_levels.insert(sample.user_id.clone(), (previous * 0.8) + (level * 0.2));
        }
        let levels_changed = Self::audio_levels_changed(&self.audio_levels, bounded_samples);
        if user_ids == self.speaking_user_ids && active_user_id == self.active_speaker_user_id && !levels_changed {
            return;
        }
        if user_ids != self.speaking_user_ids {
            self.speaking_user_ids = user_ids;
        }
        self.audio_levels = next_levels;
        if active_user_id != self.active_speaker_user_id {
            self.active_speaker_user_id = active_user_id.clone();
        }
        if let Some(active_user_id) = active_user_id {
            self.last_speaker_user_id = Some(active_user_id);
        }
        self.bump(cx);
    }

    pub fn reset_session(&mut self, session: &RiseRTCMediaSessionKey, cx: &mut Context<Self>) {
        if self.active_session.as_ref() != Some(session) {
            return;
        }
        self.reset(cx);
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        let had_state = self.active_session.is_some()
            || !self.camera_tracks.is_empty()
            || !self.screen_share_tracks.is_empty()
            || !self.remote_user_ids.is_empty()
            || !self.speaking_user_ids.is_empty()
            || !self.audio_levels.is_empty()
            || self.active_speaker_user_id.is_some()
            || self.last_speaker_user_id.is_some()
            || self.local_camera_position != RiseRTCCameraPosition::Front;
        if !had_state {
            return;
        }
        RiseRTCStreamPreviewStore::shared(cx).update(cx, |store, cx| store.reset(cx));
        self.set_active_session(None, cx);
        self.camera_tracks = HashMap::new();
        self.screen_share_tracks = HashMap::new();
        self.portrait_video_user_ids = HashSet::new();
        self.remote_user_ids = HashSet::new();
        self.speaking_user_ids = HashSet::new();
        self.audio_levels = HashMap::new();
        self.active_speaker_user_id = None;
        self.last_speaker_user_id = None;
        self.local_camera_position = RiseRTCCameraPosition::Front;
        self.bump(cx);
    }

    fn install_tracks(
        &self,
        participant: &RiseRTCParticipantVideo,
        cameras: &mut HashMap<String, Rc<WeakTrack>>,
        screens: &mut HashMap<String, Rc<WeakTrack>>,
        presentation: &WeakEntity<Self>,
    ) {
        let Some(user_id) = Self::user_id(participant) else {
            return;
        };
        for publication in &participant.video_tracks {
            let Some(track) = publication.track.as_ref().filter(|_| !publication.is_muted) else {
                continue;
            };
            match publication.source {
                TrackSource::Camera => {
                    let observed =
                        Self::observed_track(track, publication, self.camera_tracks.get(&user_id), presentation);
                    cameras.insert(user_id.clone(), observed);
                }
                TrackSource::ScreenShare => {
                    let observed =
                        Self::observed_track(track, publication, self.screen_share_tracks.get(&user_id), presentation);
                    screens.insert(user_id.clone(), observed);
                }
                TrackSource::Microphone | TrackSource::ScreenShareAudio | TrackSource::Unknown => {}
            }
        }
    }

    fn observed_track(
        track: &Rc<VideoTrack>,
        publication: &RiseRTCVideoPublication,
        previous: Option<&Rc<WeakTrack>>,
        presentation: &WeakEntity<Self>,
    ) -> Rc<WeakTrack> {
        if let Some(previous) = previous
            && previous.value().is_some_and(|value| Rc::ptr_eq(&value, track))
        {
            return previous.clone();
        }
        WeakTrack::new(track, publication.dimensions, Self::orientation_callback(presentation.clone()))
    }

    fn orientation_callback(presentation: WeakEntity<Self>) -> Rc<dyn Fn(&mut App)> {
        Rc::new(move |cx: &mut App| {
            presentation
                .update(cx, |presentation, cx| {
                    if presentation.update_portrait_video_users() {
                        cx.notify();
                    }
                })
                .ok();
        })
    }

    /// true when the set changed so the caller knows to tell the screens
    fn update_portrait_video_users(&mut self) -> bool {
        let mut users = HashSet::new();
        for (user_id, track) in &self.camera_tracks {
            if !self.screen_share_tracks.contains_key(user_id) && track.is_portrait() {
                users.insert(user_id.clone());
            }
        }
        for (user_id, track) in &self.screen_share_tracks {
            if track.is_portrait() {
                users.insert(user_id.clone());
            }
        }
        if users == self.portrait_video_user_ids {
            return false;
        }
        self.portrait_video_user_ids = users;
        true
    }

    /// metadata user id first then a non empty identity
    pub fn user_id(participant: &RiseRTCParticipantVideo) -> Option<String> {
        if let Ok(serde_json::Value::Object(object)) = serde_json::from_str::<serde_json::Value>(&participant.metadata) {
            let candidates = [
                object.get(rise_key::rtc::camel::USER_ID).and_then(serde_json::Value::as_str),
                object.get(rise_key::otp::USER_ID).and_then(serde_json::Value::as_str),
            ];
            if let Some(value) = candidates.into_iter().flatten().find(|value| !value.is_empty()) {
                return Some(value.to_owned());
            }
        }
        if participant.identity.is_empty() {
            return None;
        }
        Some(participant.identity.clone())
    }

    fn audio_levels_changed(old_value: &HashMap<String, f32>, samples: &[RiseRTCAudioLevelSample]) -> bool {
        if old_value.len() != samples.len() {
            return true;
        }
        samples.iter().any(|sample| {
            let Some(previous) = old_value.get(&sample.user_id) else {
                return true;
            };
            let level = sample.level.clamp(0.0, 1.0);
            (previous - level).abs() >= Self::AUDIO_LEVEL_EPSILON
        })
    }

    fn bump(&mut self, cx: &mut Context<Self>) {
        self.revision = self.revision.wrapping_add(1);
        self.sync_camera_device_watch(cx);
        cx.notify();
    }

    fn sync_camera_device_watch(&mut self, cx: &mut Context<Self>) {
        let wanted = self.active_session.is_some();
        if wanted == self.camera_device_watch.is_some() {
            return;
        }
        if !wanted {
            self.camera_device_watch = None;
            return;
        }
        self.camera_device_watch = Some(cx.spawn(async move |this, cx| {
            loop {
                let devices = cx.background_spawn(async { camera_capture::devices() }).await;
                let can_flip = match devices {
                    Ok(devices) => camera_capture::can_flip(&devices),
                    Err(error) => {
                        tracing::debug!(%error, "rtc camera list");
                        false
                    }
                };
                let delivered = this.update(cx, |presentation, cx| {
                    if presentation.can_flip_camera != can_flip {
                        presentation.can_flip_camera = can_flip;
                        presentation.bump(cx);
                    }
                });
                if delivered.is_err() {
                    return;
                }
                cx.background_executor().timer(Self::CAMERA_DEVICE_WATCH_INTERVAL).await;
            }
        }));
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseRTCAudioLevelSample {
    pub user_id: String,
    pub level: f32,
}

struct CaptureSession {
    track: Rc<VideoTrack>,
    renderer: Rc<dyn VideoRenderer>,
    is_one_shot: bool,
}

pub struct RiseRTCStreamPreviewStore {
    revision: u64,
    previews: HashMap<String, Arc<RenderImage>>,
    preview_order: Vec<String>,
    captures: HashMap<String, CaptureSession>,
}

impl RiseRTCStreamPreviewStore {
    const PREVIEW_LIMIT: usize = 24;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { revision: 0, previews: HashMap::new(), preview_order: Vec::new(), captures: HashMap::new() }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn image(&self, user_id: &str) -> Option<Arc<RenderImage>> {
        self.previews.get(user_id).cloned()
    }

    pub fn begin_capturing(&mut self, user_id: &str, track: &Rc<VideoTrack>, cx: &mut Context<Self>) {
        if let Some(current) = self.captures.get(user_id)
            && Rc::ptr_eq(&current.track, track)
            && !current.is_one_shot
        {
            return;
        }
        self.end_capturing(user_id);
        let store = cx.weak_entity();
        let renderer: Rc<dyn VideoRenderer> = RiseRTCStreamPreviewRenderer::new(
            user_id.to_owned(),
            Rc::new(move |user_id: String, image: Arc<RenderImage>, cx: &mut App| {
                store.update(cx, |store, cx| store.store(image, &user_id, cx)).ok();
            }),
        );
        self.captures.insert(
            user_id.to_owned(),
            CaptureSession { track: track.clone(), renderer: renderer.clone(), is_one_shot: false },
        );
        track.add_video_renderer(renderer);
    }

    pub fn capture_once(&mut self, user_id: &str, track: &Rc<VideoTrack>, cx: &mut Context<Self>) {
        if self.previews.contains_key(user_id) || self.captures.contains_key(user_id) {
            return;
        }
        let store = cx.weak_entity();
        let renderer: Rc<dyn VideoRenderer> = RiseRTCStreamPreviewRenderer::new(
            user_id.to_owned(),
            Rc::new(move |user_id: String, image: Arc<RenderImage>, cx: &mut App| {
                store
                    .update(cx, |store, cx| {
                        store.store(image, &user_id, cx);
                        store.end_capturing(&user_id);
                    })
                    .ok();
            }),
        );
        self.captures.insert(
            user_id.to_owned(),
            CaptureSession { track: track.clone(), renderer: renderer.clone(), is_one_shot: true },
        );
        track.add_video_renderer(renderer);
    }

    pub fn end_capturing(&mut self, user_id: &str) {
        let Some(capture) = self.captures.remove(user_id) else {
            return;
        };
        capture.track.remove_video_renderer(&capture.renderer);
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        let current_captures = std::mem::take(&mut self.captures);
        for capture in current_captures.into_values() {
            capture.track.remove_video_renderer(&capture.renderer);
        }
        self.previews = HashMap::new();
        self.preview_order = Vec::new();
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
    }

    fn store(&mut self, image: Arc<RenderImage>, user_id: &str, cx: &mut Context<Self>) {
        self.previews.insert(user_id.to_owned(), image);
        self.preview_order.retain(|existing| existing != user_id);
        self.preview_order.push(user_id.to_owned());
        while self.preview_order.len() > Self::PREVIEW_LIMIT {
            let oldest = self.preview_order.remove(0);
            self.previews.remove(&oldest);
        }
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
    }
}

type PreviewDelivery = Rc<dyn Fn(String, Arc<RenderImage>, &mut App)>;

/// turns at most one picture a second into a small still, off the main thread
struct RiseRTCStreamPreviewRenderer {
    user_id: String,
    deliver: PreviewDelivery,
    last_capture_time: Cell<Option<Duration>>,
    render_pending: Rc<Cell<bool>>,
}

impl RiseRTCStreamPreviewRenderer {
    const CAPTURE_INTERVAL: Duration = Duration::from_secs(1);
    const MAXIMUM_DIMENSION: u32 = 480;

    fn new(user_id: String, deliver: PreviewDelivery) -> Rc<Self> {
        Rc::new(Self { user_id, deliver, last_capture_time: Cell::new(None), render_pending: Rc::new(Cell::new(false)) })
    }

    fn preview_image(frame: &PreparedHostFrame, rotation_degrees: u16) -> Option<Arc<RenderImage>> {
        let source = match frame {
            PreparedHostFrame::Planes(planes) => Self::rgba_from_nv12(planes)?,
            PreparedHostFrame::Bgra(pixels) => Self::rgba_from_bgra(pixels)?,
            PreparedHostFrame::Surface { .. } => return None,
        };
        let (width, height) = source.dimensions();
        let maximum_dimension = width.max(height).max(1);
        let scaled = if maximum_dimension > Self::MAXIMUM_DIMENSION {
            let scale = f64::from(Self::MAXIMUM_DIMENSION) / f64::from(maximum_dimension);
            let target_width = ((f64::from(width) * scale).round() as u32).max(1);
            let target_height = ((f64::from(height) * scale).round() as u32).max(1);
            image::imageops::resize(&source, target_width, target_height, FilterType::Triangle)
        } else {
            source
        };
        let mut oriented = match rotation_degrees {
            90 => image::imageops::rotate90(&scaled),
            180 => image::imageops::rotate180(&scaled),
            270 => image::imageops::rotate270(&scaled),
            _ => scaled,
        };
        for pixel in oriented.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(oriented)]);
        Some(rise_ui::image_ui::render_image_atlas::RenderImageAtlas::tracked(Arc::new(RenderImage::new(frames))))
    }

    fn rgba_from_bgra(frame: &HostBgraFrame) -> Option<RgbaImage> {
        if !frame.rows_fit() {
            return None;
        }
        let mut image = RgbaImage::new(frame.width, frame.height);
        for (y, row) in image.rows_mut().enumerate() {
            let start = y * frame.stride as usize;
            let source = frame.pixels.get(start..start + frame.width as usize * 4)?;
            for (pixel, bgra) in row.zip(source.chunks_exact(4)) {
                pixel.0 = [bgra[2], bgra[1], bgra[0], 255];
            }
        }
        Some(image)
    }

    fn rgba_from_nv12(frame: &HostNv12Frame) -> Option<RgbaImage> {
        if frame.width == 0 || frame.height == 0 {
            return None;
        }
        let matrix = frame.conversion.matrix;
        let offset = frame.conversion.offset;
        let mut image = RgbaImage::new(frame.width, frame.height);
        for (y, row) in image.rows_mut().enumerate() {
            let luma_row = frame.luma.get(y * frame.luma_stride as usize..)?;
            let chroma_row = frame.chroma.get((y / 2) * frame.chroma_stride as usize..)?;
            for (x, pixel) in row.enumerate() {
                let luma = f32::from(*luma_row.get(x)?) / 255.0;
                let pair = (x / 2) * 2;
                let cb = f32::from(*chroma_row.get(pair)?) / 255.0;
                let cr = f32::from(*chroma_row.get(pair + 1)?) / 255.0;
                let yuv = [luma - offset[0], cb - offset[1], cr - offset[2]];
                let channel = |row: [f32; 3]| {
                    let value = row[0] * yuv[0] + row[1] * yuv[1] + row[2] * yuv[2];
                    (value.clamp(0.0, 1.0) * 255.0).round() as u8
                };
                pixel.0 = [channel(matrix[0]), channel(matrix[1]), channel(matrix[2]), 255];
            }
        }
        Some(image)
    }

    fn finish_pending_render(&self) {
        self.render_pending.set(false);
    }
}

impl VideoRenderer for RiseRTCStreamPreviewRenderer {
    fn render(&self, frame: &RemoteVideoFrame, cx: &mut App) {
        let now = RiseClock::monotonic();
        let due = self.last_capture_time.get().is_none_or(|last| now.saturating_sub(last) >= Self::CAPTURE_INTERVAL);
        if self.render_pending.get() || !due {
            return;
        }
        self.render_pending.set(true);
        self.last_capture_time.set(Some(now));

        let prepared = frame.frame.clone();
        let rotation = frame.rotation_degrees;
        let user_id = self.user_id.clone();
        let deliver = self.deliver.clone();
        let pending = self.render_pending.clone();
        let rendering = cx.background_spawn(async move { Self::preview_image(&prepared, rotation) });
        cx.spawn(async move |cx| {
            let preview = rendering.await;
            pending.set(false);
            if let Some(preview) = preview {
                cx.update(|cx| deliver(user_id, preview, cx));
            }
        })
        .detach();
    }
}

impl Drop for RiseRTCStreamPreviewRenderer {
    fn drop(&mut self) {
        self.finish_pending_render();
    }
}
