use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::channel::{mpsc, oneshot};
use futures::{FutureExt, StreamExt};
use futures::future::{LocalBoxFuture, Shared};
use image::imageops::{self, FilterType};
use image::{Frame, ImageBuffer, Rgba, RgbaImage};
use gpui::{App, AppContext, AsyncApp, Context, Entity, RenderImage, Subscription, Task, WeakEntity, WindowId};
use parking_lot::Mutex;
use rand::seq::{IndexedRandom, SliceRandom};
use rise_platform::process_activity::{ProcessActivityLease, begin_process_activity};
use rise_canon::rise_service_text::music_service;
use rise_canon::{RiseGuideSignal, RiseLimits};
use rise_canon::RiseIdentityCanon;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::core::engine_bridge::network_transport_config::NetworkClientSlot;
use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode, URLRequest};
use rise_i18n::{FormatArg, tr, tr_args};
use rise_media::video::av_player::{
    AVPlayer, AVPlayerError, AVPlayerItem, AVPlayerItemNotification, AVPlayerItemStatus, AVPlayerTimeObserver,
};
use rise_platform::file_manager::FileManager;
use rise_platform::app_info::AppInfo;
use rise_platform::host_os::HostOs;
use rise_platform::media_keys::{
    Artwork, CommandHandler, CommandOutcome, MediaCommand, MediaCommandEvent, MediaIdentity, MediaKeys, NowPlaying,
    PlaybackState, transport_needs_window,
};
use rise_platform::paths::temporary_directory;
use rise_ui::swiftui::RepeatingTimer;

use crate::app::aside::aside_navigator;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::desktop_platform::av_audio_session::{
    AVAudioSession, AVAudioSessionInterruption, AVAudioSessionInterruptionOptions, AVAudioSessionInterruptionType,
};
use crate::core::desktop_platform::av_audio_session_route::{AVAudioSessionRouteChange, AVAudioSessionRouteChangeReason};
use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication};
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::rise_audio_playback_work::{RiseAudioPlaybackWork, RisePlaybackAudioSession};
use crate::core::lib::rise_call_app_volume::RiseCallAppVolume;
use crate::modules::music::shared::utils::music::music_output_volume::MusicOutputVolume;
use crate::core::lib::rise_call_audio_ownership::RiseCallAudioOwnership;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDownloadProgressCoalescer, RemoteMediaDownloadRetryPolicy, RemoteMediaHttpResponse,
    RemoteMediaTransferError, RiseMediaRedirectSessionDelegate,
};
use crate::core::media::rise_media_transfer_hub::{RiseMediaCacheIdentity, RiseMediaTransferHub};
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapDecoder, CleverImageBitmapLoader, CleverImageRenderMemoryCache, CleverImageWorkLane,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType, NotifierUndoOptions,
};
use crate::modules::auth::services::auth_local_storage::AuthLocalStorage;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::{ChatPerformanceTrace, OperationToken};
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::modules::chat::components::chat::voice::chat_voice_message_player::ChatVoiceMessagePlayer;
use crate::modules::music::shared::utils::music::music_audio_effect_settings::{
    MusicAudioEffectKind, MusicAudioEffectKindExt, MusicAudioEffectSettings,
};
use crate::modules::music::shared::utils::music::music_audio_effect_store::MusicAudioEffectStore;
use crate::modules::music::shared::utils::music::music_effect_file_player::{
    MUSIC_EFFECT_FILE_PLAYER_DID_FINISH_PLAYING, MusicEffectFilePlayer, MusicEffectFilePlayerDidFinishPlaying,
    MusicEffectPlaybackLimits,
};
use crate::modules::music::shared::utils::music::music_effect_audio_graph::MusicEffectAudioFile;
use crate::modules::music::shared::utils::music::music_effect_thermal_guard::MusicEffectThermalGuard;
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;
use crate::modules::music::shared::utils::music::music_cover_letterbox::MusicCoverLetterbox;
use crate::modules::music::shared::utils::music::music_media_url::MusicMediaURL;
use crate::modules::music::shared::utils::music::music_local_state::MusicLocalState;
use crate::modules::music::shared::utils::music::music_player_rtc_hooks::music_player_rtc_hooks;
use crate::modules::music::shared::utils::music::music_models::{MusicRepeatMode, MusicRepeatModeExt, TrackModel};
use crate::modules::music::shared::utils::music::music_now_playing_artwork::MusicNowPlayingArtwork;
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;
use crate::modules::music::shared::utils::music::music_playback_url::MusicPlaybackUrl;
use crate::modules::music::shared::utils::music::music_playback_preferences::{
    MusicPlaybackPreferenceScope, MusicPlaybackPreferences,
};
use crate::modules::music::shared::utils::music::ro_music_file_player::{
    RO_MUSIC_FILE_PLAYER_DID_FINISH_PLAYING, ROMusicFilePlayer, ROMusicFilePlayerDidFinishPlaying,
    ROMusicFilePreparedPlayback,
};
use crate::modules::music::shared::utils::music::music_listening_interruption_gate::{
    MusicListeningAutoInterruptionAction, MusicListeningInterruptionCallback, MusicListeningInterruptionDecision,
    MusicListeningInterruptionGate, MusicListeningInterruptionPolicy, MusicListeningResyncAction,
};
use crate::modules::music::stores::music_actions::music_types::{
    MusicListeningParticipant, MusicListeningParticipantPermissions, MusicListeningSessionCommand,
    MusicListeningSessionTransition, MusicListeningSessionTransitionDecision, MusicListeningSessionTransitionPolicy,
    MusicListeningSessionUpdate, MusicPlaylistListeningSession, MusicUserReference,
};
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::stores::music_interactions::music_interactions::{
    MusicInteractionsStore, MusicPlaylistListeningSessionStateRequest,
};
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::user::engine::rise_meaningful_activity::RiseMeaningfulActivity;

enum MusicPlayerLog {}

impl MusicPlayerLog {
    fn kind() -> SaiLogKind {
        SaiLogKind::Custom("player".to_owned())
    }

    fn info(message: String, cx: &mut App) {
        Sai::log("🎵 Player", || message, Self::kind(), cx);
    }

    fn ok(message: String, cx: &mut App) {
        Sai::log("🎵 Player ✅", || message, Self::kind(), cx);
    }

    fn fail(message: String, cx: &mut App) {
        Sai::log("🎵 Player ❌", || message, Self::kind(), cx);
    }
}

pub type MusicListeningBlockedControl = rise_music::BlockedControl;

/// extension methods for the blocked control enum
pub trait MusicListeningBlockedControlExt {
    fn localized_name(&self) -> String;
}

impl MusicListeningBlockedControlExt for MusicListeningBlockedControl {
    fn localized_name(&self) -> String {
        tr(self.i18n_key())
    }
}

impl MusicListeningParticipantPermissions {
    pub fn allows(&self, control: MusicListeningBlockedControl) -> bool {
        match control {
            MusicListeningBlockedControl::Playback => self.can_control_playback,
            MusicListeningBlockedControl::Seek => self.can_seek,
            MusicListeningBlockedControl::ChangeTracks => self.can_change_tracks,
            MusicListeningBlockedControl::ApplyEffects => self.can_apply_effects,
        }
    }

    pub fn granted_controls(&self) -> Vec<MusicListeningBlockedControl> {
        MusicListeningBlockedControl::ALL
            .into_iter()
            .filter(|control| self.allows(*control))
            .collect()
    }

    pub fn localized_granted_list(&self) -> String {
        self.granted_controls()
            .iter()
            .map(MusicListeningBlockedControlExt::localized_name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicListeningPermissionRequestNotice {
    pub session_id: String,
    pub participant_user_id: String,
    pub permissions: MusicListeningParticipantPermissions,
}

impl MusicListeningPermissionRequestNotice {
    pub fn id(&self) -> String {
        format!("{}:{}", self.session_id, self.participant_user_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MusicListeningSessionTransitionPhase {
    AwaitingConfirmation,
    ClosingCurrent,
    JoiningTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MusicAudioDownloadProgress {
    downloaded_bytes: i64,
    total_bytes: i64,
}

impl MusicAudioDownloadProgress {
    pub fn new(downloaded_bytes: i64, total_bytes: i64) -> Self {
        Self {
            downloaded_bytes: downloaded_bytes.max(0),
            total_bytes: total_bytes.max(0),
        }
    }

    pub fn downloaded_bytes(&self) -> i64 {
        self.downloaded_bytes
    }

    pub fn total_bytes(&self) -> i64 {
        self.total_bytes
    }

    pub fn fraction(&self) -> Option<f64> {
        if self.total_bytes <= 0 {
            return None;
        }
        Some((self.downloaded_bytes as f64 / self.total_bytes as f64).clamp(0.0, 1.0))
    }

    pub fn byte_description(&self) -> String {
        let downloaded = Self::format(self.downloaded_bytes);
        if self.total_bytes <= 0 {
            return downloaded;
        }
        format!("{downloaded} / {}", Self::format(self.total_bytes))
    }

    fn format(byte_count: i64) -> String {
        RiseFormat::file_size(byte_count)
    }
}

#[derive(Clone, Debug, Default)]
pub struct MusicAudioDownloadProgressCoalescer {
    last_published_bytes: Option<i64>,
    last_published_total_bytes: i64,
    last_published_time: f64,
}

impl MusicAudioDownloadProgressCoalescer {
    pub const MINIMUM_PUBLISH_INTERVAL: f64 = 0.1;
    pub const MINIMUM_BYTE_DELTA: i64 = 16 * 1_024;
    pub const MAXIMUM_SILENCE_INTERVAL: f64 = 0.5;

    pub fn should_publish(&mut self, progress: MusicAudioDownloadProgress, now: f64) -> bool {
        let Some(previous_bytes) = self.last_published_bytes else {
            self.record(progress, now);
            return true;
        };

        let elapsed = (now - self.last_published_time).max(0.0);
        let byte_delta = (progress.downloaded_bytes - previous_bytes).max(0);
        let completed =
            progress.total_bytes > 0 && progress.downloaded_bytes >= progress.total_bytes;
        let total_changed = progress.total_bytes != self.last_published_total_bytes;
        let is_due = elapsed >= Self::MINIMUM_PUBLISH_INTERVAL
            && (byte_delta >= Self::MINIMUM_BYTE_DELTA
                || elapsed >= Self::MAXIMUM_SILENCE_INTERVAL
                || total_changed);
        if !(completed || is_due) {
            return false;
        }
        self.record(progress, now);
        true
    }

    fn record(&mut self, progress: MusicAudioDownloadProgress, now: f64) {
        self.last_published_bytes = Some(progress.downloaded_bytes);
        self.last_published_total_bytes = progress.total_bytes;
        self.last_published_time = now;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
enum MusicAudioDownloadOperationError {
    #[error("Audio file is too large")]
    PayloadTooLarge,
}

struct PlaybackCachePreparation {
    destination: Option<PathBuf>,
    cached_url: Option<PathBuf>,
    log_messages: Vec<String>,
}

struct PlaybackCacheCommit {
    url: Option<PathBuf>,
    error: Option<String>,
    log_messages: Vec<String>,
}

#[derive(Debug)]
struct MusicPlaybackDownloadCommitLease {
    valid: Mutex<bool>,
}

impl MusicPlaybackDownloadCommitLease {
    fn new() -> Arc<Self> {
        Arc::new(Self { valid: Mutex::new(true) })
    }

    fn is_valid(&self) -> bool {
        *self.valid.lock()
    }

    fn invalidate(&self) {
        *self.valid.lock() = false;
    }
}

/// progress callback of the playback download, gets progress and the retry attempt
type MusicPlaybackDownloadProgressHandler = Arc<dyn Fn(MusicAudioDownloadProgress, i64) + Send + Sync>;

struct MusicAudioDownloadTraceScope {
    started_at: Duration,
    is_completed: bool,
}

impl MusicAudioDownloadTraceScope {
    fn begin() -> Self {
        ChatPerformanceTrace::shared().record("audio.download.begin", HashMap::new());
        Self { started_at: RiseClock::monotonic(), is_completed: false }
    }
}

impl Drop for MusicAudioDownloadTraceScope {
    fn drop(&mut self) {
        let wall_ms = RiseClock::monotonic().saturating_sub(self.started_at).as_secs_f64() * 1_000.0;
        let cancelled = if self.is_completed { "0" } else { "1" };
        ChatPerformanceTrace::shared().record(
            "audio.download.end",
            HashMap::from([
                ("wall_ms".to_owned(), format!("{wall_ms:.2}")),
                ("cancelled".to_owned(), cancelled.to_owned()),
            ]),
        );
    }
}

/// cancels the transfer on the account runtime when the waiting download is dropped
struct MusicAudioDownloadAbortOnDrop(tokio::task::AbortHandle);

impl Drop for MusicAudioDownloadAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct MusicAudioDownloadTemporaryRemoval(PathBuf);

impl Drop for MusicAudioDownloadTemporaryRemoval {
    fn drop(&mut self) {
        let path = std::mem::take(&mut self.0);
        RiseAccountRuntime::current().spawn_blocking(move || {
            FileManager.remove_item(&path).ok();
        });
    }
}

/// prefetch job result, path or error text
type MusicPlaybackPrefetchResult = (Option<PathBuf>, Option<String>);

/// shared result of one prefetch job, every waiter gets the same one
#[derive(Clone)]
struct MusicPlaybackPrefetchJob {
    value: Shared<LocalBoxFuture<'static, MusicPlaybackPrefetchResult>>,
}

impl MusicPlaybackPrefetchJob {
    async fn value(&self) -> MusicPlaybackPrefetchResult {
        self.value.clone().await
    }
}

struct MusicPlaybackPrefetchEntry {
    job: MusicPlaybackPrefetchJob,
    generation: Uuid,
    _task: Task<()>,
}

thread_local! {
    static PREFETCH_JOBS: RefCell<HashMap<String, MusicPlaybackPrefetchEntry>> =
        RefCell::new(HashMap::new());
}

/// prefetch jobs live here keyed by track, not as children of one task
/// that task got cancelled on every play and killed the download of the next track
enum MusicPlaybackPrefetchRegistry {}

impl MusicPlaybackPrefetchRegistry {
    fn job(key: &str) -> Option<MusicPlaybackPrefetchJob> {
        PREFETCH_JOBS.with_borrow(|jobs| jobs.get(key).map(|entry| entry.job.clone()))
    }

    fn start(
        key: String,
        operation: impl Future<Output = MusicPlaybackPrefetchResult> + 'static,
        cx: &mut App,
    ) -> MusicPlaybackPrefetchJob {
        let generation = Uuid::new_v4();
        let (sender, receiver) = oneshot::channel::<MusicPlaybackPrefetchResult>();
        let job = MusicPlaybackPrefetchJob {
            value: receiver
                .map(|result| result.unwrap_or((None, None)))
                .boxed_local()
                .shared(),
        };
        let task_key = key.clone();
        let task = cx.spawn(async move |_cx| {
            let result = operation.await;
            let finished = PREFETCH_JOBS.with_borrow_mut(|jobs| {
                if jobs.get(&task_key).is_some_and(|entry| entry.generation == generation) {
                    jobs.remove(&task_key)
                } else {
                    None
                }
            });
            sender.send(result).ok();
            drop(finished);
        });
        let replaced = PREFETCH_JOBS.with_borrow_mut(|jobs| {
            jobs.insert(
                key,
                MusicPlaybackPrefetchEntry { job: job.clone(), generation, _task: task },
            )
        });
        drop(replaced);
        job
    }

    fn key(track_id: &str, owner_user_id: Option<&str>) -> String {
        format!("{}\u{0}{}", owner_user_id.unwrap_or(""), track_id)
    }

    fn cancel_jobs(keeping: &HashSet<String>) {
        let cancelled: Vec<MusicPlaybackPrefetchEntry> = PREFETCH_JOBS.with_borrow_mut(|jobs| {
            let keys: Vec<String> =
                jobs.keys().filter(|key| !keeping.contains(*key)).cloned().collect();
            keys.iter().filter_map(|key| jobs.remove(key)).collect()
        });
        drop(cancelled);
    }
}

struct MusicAudioDownloadTemporaryFile {
    path: PathBuf,
    is_owned: bool,
}

impl MusicAudioDownloadTemporaryFile {
    fn hand_over(mut self) -> PathBuf {
        self.is_owned = false;
        self.path.clone()
    }
}

impl Drop for MusicAudioDownloadTemporaryFile {
    fn drop(&mut self) {
        if self.is_owned {
            FileManager.remove_item(&self.path).ok();
        }
    }
}

static MUSIC_AUDIO_DOWNLOAD_SESSION: NetworkClientSlot = NetworkClientSlot::without_redirects();

type MusicAudioDownloadResult = (PathBuf, RemoteMediaHttpResponse);

struct MusicAudioDownloadOperation {
    maximum_byte_count: i64,
    on_progress: Arc<dyn Fn(MusicAudioDownloadProgress) + Send + Sync>,
    progress_coalescer: Mutex<MusicAudioDownloadProgressCoalescer>,
}

impl MusicAudioDownloadOperation {
    fn new(
        maximum_byte_count: i64,
        on_progress: impl Fn(MusicAudioDownloadProgress) + Send + Sync + 'static,
    ) -> Self {
        Self {
            maximum_byte_count: maximum_byte_count.max(1),
            on_progress: Arc::new(on_progress),
            progress_coalescer: Mutex::new(MusicAudioDownloadProgressCoalescer::default()),
        }
    }

    async fn start(&self, request: &URLRequest) -> anyhow::Result<MusicAudioDownloadResult> {
        let Some(session) = MUSIC_AUDIO_DOWNLOAD_SESSION.current() else {
            return Err(URLError::new(URLErrorCode::SecureConnectionFailed).into());
        };
        let session = &session;
        let (mut response, url) =
            RiseMediaRedirectSessionDelegate::send(session, request, None, |_, _| {}).await?;
        let head = RemoteMediaHttpResponse::from_reqwest(&response, url);
        let expected_bytes = head.expected_content_length().max(0);
        if expected_bytes > self.maximum_byte_count {
            return Err(MusicAudioDownloadOperationError::PayloadTooLarge.into());
        }

        let owned = MusicAudioDownloadTemporaryFile {
            path: temporary_directory()
                .join(format!("riseonly-audio-{}.download", Uuid::new_v4().uuid_string())),
            is_owned: true,
        };
        let mut file = std::fs::File::create(&owned.path)?;
        let mut total_bytes_written: i64 = 0;
        loop {
            let chunk = tokio::time::timeout(request.timeout_interval, response.chunk())
                .await
                .map_err(|_| URLError::new(URLErrorCode::TimedOut))?
                .map_err(|error| RemoteMediaTransferError::url_error(&error))?;
            let Some(bytes) = chunk else { break };
            file.write_all(&bytes)?;
            total_bytes_written =
                total_bytes_written.saturating_add(i64::try_from(bytes.len()).unwrap_or(i64::MAX));
            if total_bytes_written > self.maximum_byte_count {
                return Err(MusicAudioDownloadOperationError::PayloadTooLarge.into());
            }
            let progress = MusicAudioDownloadProgress::new(total_bytes_written, expected_bytes);
            let should_publish = self
                .progress_coalescer
                .lock()
                .should_publish(progress, RemoteMediaDownloadProgressCoalescer::uptime());
            if should_publish {
                (self.on_progress)(progress);
            }
        }
        file.flush()?;
        drop(file);

        let bytes = Self::file_byte_count(&owned.path);
        if bytes > self.maximum_byte_count {
            return Err(MusicAudioDownloadOperationError::PayloadTooLarge.into());
        }
        let expected = head.expected_content_length();
        let progress = MusicAudioDownloadProgress::new(
            bytes,
            if expected > 0 { bytes.max(expected) } else { bytes },
        );
        let should_publish = self
            .progress_coalescer
            .lock()
            .should_publish(progress, RemoteMediaDownloadProgressCoalescer::uptime());
        if should_publish {
            (self.on_progress)(progress);
        }
        Ok((owned.hand_over(), head))
    }

    fn file_byte_count(url: &Path) -> i64 {
        RemoteMediaDiskCache::file_byte_count(url)
    }
}

/// the room a follower left only because they couldnt pause it for an ad-hoc mp3
#[derive(Clone, Debug, PartialEq, Eq)]
struct MusicRoomLeftForAdHoc {
    account_user_id: String,
    playlist_id: String,
    host_user_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NowPlayingArtworkIdentity {
    track_id: String,
    cover_source: String,
    audio_source: String,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
}

impl NowPlayingArtworkIdentity {
    fn is_current_account(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(
            self.account_user_id.as_deref(),
            self.account_epoch.as_deref(),
        )
    }
}

struct MusicListeningPublishSuppressionReset(Rc<Cell<bool>>);

impl Drop for MusicListeningPublishSuppressionReset {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// play options past the track, with the defaults
pub struct MusicPlayOptions {
    pub queue: Vec<TrackModel>,
    pub scope_id: Option<String>,
    pub record_play_on_success: bool,
    pub preserve_queue: bool,
    pub confirm_call_interruption: bool,
    pub on_playback_accepted: Option<Box<dyn FnOnce(&mut App)>>,
}

impl Default for MusicPlayOptions {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            scope_id: None,
            record_play_on_success: false,
            preserve_queue: false,
            confirm_call_interruption: true,
            on_playback_accepted: None,
        }
    }
}

#[derive(Clone)]
struct MusicPlayRequest {
    track: TrackModel,
    queue: Vec<TrackModel>,
    scope_id: Option<String>,
    record_play_on_success: bool,
    preserve_queue: bool,
    on_playback_accepted: Rc<RefCell<Option<Box<dyn FnOnce(&mut App)>>>>,
}

impl MusicPlayRequest {
    fn on_playback_accepted(&self) -> Option<Box<dyn FnOnce(&mut App)>> {
        if self.on_playback_accepted.borrow().is_none() {
            return None;
        }
        let accepted = self.on_playback_accepted.clone();
        Some(Box::new(move |cx: &mut App| {
            let callback = accepted.borrow_mut().take();
            if let Some(callback) = callback {
                callback(cx);
            }
        }))
    }

    fn replay(&self, confirm_call_interruption: bool, player: &mut MusicPlayerState, cx: &mut Context<MusicPlayerState>) {
        player.play(
            self.track.clone(),
            MusicPlayOptions {
                queue: self.queue.clone(),
                scope_id: self.scope_id.clone(),
                record_play_on_success: self.record_play_on_success,
                preserve_queue: self.preserve_queue,
                confirm_call_interruption,
                on_playback_accepted: self.on_playback_accepted(),
            },
            cx,
        );
    }
}

struct MusicPlayDispatchTrace(Option<OperationToken>);

impl Drop for MusicPlayDispatchTrace {
    fn drop(&mut self) {
        ChatPerformanceTrace::shared().end_operation(self.0.take(), 1.0, HashMap::new());
    }
}

/// periodic position publication
/// notifying the whole player redrew everything twice a second
/// views showing position watch this instead
pub struct MusicPlayerPositionPublication;

pub struct MusicPlayerState {
    pub current_track: Option<TrackModel>,
    pub queue: Vec<TrackModel>,
    source_queue: Vec<TrackModel>,
    pub queue_index: usize,
    pub is_playing: bool,
    meaningful_activity_source: Uuid,
    pub progress: f64,
    pub current_time: f64,
    pub duration: f64,
    listening_shuffle_enabled: Option<bool>,
    music_shuffle_seed: i64,
    file_shuffle_seed: i64,
    listening_shuffle_seed: i64,
    listening_repeat_mode: Option<MusicRepeatMode>,
    show_now_playing: bool,
    pub show_queue_sheet: bool,
    pub show_playlist_listeners_sheet: bool,
    pub show_listening_session_sheet: bool,
    pub playback_scope_id: Option<String>,
    last_playback_error: Option<String>,
    playback_failure_serial: u64,
    pub is_preparing_playback: bool,
    playback_download_track_id: Option<String>,
    playback_download_progress: Option<MusicAudioDownloadProgress>,
    pub listening_session: Option<MusicPlaylistListeningSession>,
    departed_listening_users: HashMap<String, MusicUserReference>,
    pub listening_session_server_time_ms: i64,
    pub blocked_listening_control: Option<MusicListeningBlockedControl>,
    pub listening_permission_request_notice: Option<MusicListeningPermissionRequestNotice>,
    pub last_listening_error: Option<String>,
    pub pending_listening_session_transition: Option<MusicListeningSessionTransition>,
    is_listening_session_transitioning: bool,
    listening_session_transition_target_user_id: Option<String>,
    audio_effect_settings: MusicAudioEffectSettings,
    audio_effect_settings_before_reset: Option<MusicAudioEffectSettings>,
    show_audio_effects_sheet: bool,
    playback_ad_hoc_effects_key: Option<String>,

    ad_hoc_audio_effects: HashMap<String, MusicAudioEffectSettings>,
    ad_hoc_audio_effects_order: Vec<String>,

    listening_consent_episode_active: bool,
    session_paused_by_local_interruption: bool,
    room_left_for_ad_hoc: Option<MusicRoomLeftForAdHoc>,

    record_play_on_success: bool,
    playback_owner_user_id: Option<String>,
    playback_owner_account_epoch: Option<String>,
    now_playing_artwork_identity: Option<NowPlayingArtworkIdentity>,
    now_playing_artwork: Option<Artwork>,
    now_playing_artwork_task: Option<Task<()>>,
    now_playing_artwork_generation: Uuid,
    playback_download_candidate_id: Option<Uuid>,
    playback_download_retry_attempt: i64,
    playback_download_commit_lease: Option<Arc<MusicPlaybackDownloadCommitLease>>,
    background_task: Option<ProcessActivityLease>,
    prefetch_task: Option<Task<()>>,
    is_handling_track_end: bool,
    player: Option<Entity<AVPlayer>>,
    file_player: Option<Entity<ROMusicFilePlayer>>,
    effect_player: Option<Entity<MusicEffectFilePlayer>>,
    effect_finish_observer: Option<(RiseSubscription, Task<()>)>,
    effect_handoff_task: Option<Task<()>>,
    local_playback_url: Option<PathBuf>,
    progress_timer: Option<RepeatingTimer>,
    position_publication: Entity<MusicPlayerPositionPublication>,
    last_watchdog_playback_time: f64,
    time_observer: Option<AVPlayerTimeObserver>,
    item_status_observer: Option<Subscription>,
    end_observer_token: Option<Subscription>,
    file_finish_observer: Option<(RiseSubscription, Task<()>)>,
    prepare_task: Option<Task<()>>,
    prepare_generation: u64,
    call_playback_request_generation: u64,
    pending_playback_after_call: Option<Box<dyn FnOnce(&mut MusicPlayerState, &mut Context<MusicPlayerState>)>>,
    paused_for_call: bool,
    session_continuation_task: Option<Task<()>>,
    session_continuation_generation: u64,
    last_download_error: Option<String>,
    route_change_observer: Option<RiseSubscription>,
    interruption_observer: Option<RiseSubscription>,
    app_lifecycle_observers: Vec<RiseSubscription>,
    playback_signal_task: Option<Task<()>>,
    media_keys: Option<MediaKeys>,
    media_service_task: Option<Task<()>>,
    media_service_cancelled: bool,
    media_window: Option<WindowId>,
    playback_observers_installed: bool,
    audio_session_activated: bool,
    is_audio_session_interrupted: bool,
    was_playing_before_interruption: bool,
    background_keep_alive_task: Option<Task<()>>,
    listening_session_observer_id: Option<Uuid>,
    audio_effect_commit_tasks: HashMap<MusicAudioEffectKind, Task<()>>,
    audio_effect_actor_user_id: Option<String>,
    listening_state_publish_task: Option<Task<()>>,
    listening_apply_task: Option<Task<()>>,
    is_applying_listening_session: bool,
    last_applied_listening_generation: i64,
    last_applied_listening_revision: i64,
    presence_playlist_id: Option<String>,
    listening_observation_playlist_id: Option<String>,
    suppress_next_listening_playback_publish: Rc<Cell<bool>>,
    seek_operation_generation: u64,
    active_listening_session_transition: Option<MusicListeningSessionTransition>,
    listening_session_transition_phase: Option<MusicListeningSessionTransitionPhase>,
    listening_session_transition_success_callbacks:
        Vec<Box<dyn FnOnce(MusicPlaylistListeningSession, &mut App)>>,
    listening_session_transition_error_callbacks: Vec<Box<dyn FnOnce(String, &mut App)>>,
    listening_session_transition_cancelled_callbacks: Vec<Box<dyn FnOnce(&mut App)>>,
    is_listening_session_exit_in_flight: bool,
    listening_session_exit_session_id: Option<String>,

    external_duck_active: bool,
    external_resume_intent: bool,
    duck_origin_was_playing: bool,
    external_audio_owners: HashSet<MusicExternalAudioOwner>,
    suppress_external_resume_during_user_handoff: bool,
    resume_playback_when_foreground: bool,
    duck_fade_epoch: i64,
    av_player_fade_timer: Option<RepeatingTimer>,
    av_player_fade_start: Duration,
    av_player_fade_duration: f64,
    av_player_fade_start_volume: f32,
    av_player_fade_target_volume: f32,
    av_player_fade_completion: Option<Box<dyn FnOnce(&mut MusicPlayerState, &mut Context<MusicPlayerState>)>>,
}

/// remote transport command from media keys and the like
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MusicPlayerStateRemoteTransportCommand {
    Play,
    Pause,
    TogglePlayPause,
    Next,
    Previous,
    ChangePosition(f64),
}

/// what the playback observers hand to the main thread
#[derive(Clone, Copy, Debug)]
enum MusicPlaybackObserverSignal {
    RouteChange(AVAudioSessionRouteChange),
    Interruption(AVAudioSessionInterruption),
    WillResignActive,
    DidEnterBackground,
    DidBecomeActive,
    CallAppVolumeDidChange,
    RemoteCommand(MusicPlayerStateRemoteTransportCommand),
}

/// one pass of the background watchdog
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MusicBackgroundWatchdogStep {
    Stop,
    Skip,
    Proceed { app_is_active: bool },
    Recover { app_is_active: bool },
}

impl MusicPlayerState {
    const AD_HOC_AUDIO_EFFECTS_LIMIT: usize = 16;
    const DUCK_FADE_DURATION: f64 = 0.45;
    const EFFECT_HANDOFF_FADE_DURATION: f64 = 0.12;
    const SPATIAL_HANDOFF_FADE_DURATION: f64 = 0.55;
    const PLAYBACK_START_FADE_DURATION: f64 = 0.35;
    const EFFECT_PUBLISH_DEBOUNCE_MILLISECONDS: u64 = 300;
    const EFFECT_COMMIT_DEBOUNCE_MILLISECONDS: u64 = 160;
    const LISTENING_DRIFT_CORRECTION_MS: i64 = 1_200;
    const LISTENING_ALIGNMENT_SEEK_DRIFT: f64 = 0.45;
    const LISTENING_STATE_RETRY_ATTEMPTS: usize = 40;
    const LISTENING_STATE_RETRY_INTERVAL_MS: u64 = 125;
    const LISTENING_ENGINE_ALIGNMENT_ATTEMPTS: usize = 120;
    const LISTENING_ENGINE_ALIGNMENT_INTERVAL_MS: u64 = 100;

    fn now_playing_artwork_maximum_edge() -> i64 {
        RiseLimits::client_value("music_track_metadata_writer_maximum_artwork_edge")
    }

    fn playback_download_idle_timeout() -> f64 {
        RiseLimits::client_value("music_playback_download_idle_timeout_ms") as f64 / 1000.0
    }

    /// shared player if it exists, none means nothing has played yet so treat it as empty
    pub fn try_read(cx: &App) -> Option<&Self> {
        rise_ui::shared::try_read::<Self>(cx)
    }

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    /// notified on every position publication, which doesnt notify the player
    pub fn position_publication(&self) -> &Entity<MusicPlayerPositionPublication> {
        &self.position_publication
    }

    /// if only the playhead moved it notifies position alone
    /// a new duration changes what the whole player shows
    fn publish_position(&mut self, duration_before: f64, cx: &mut Context<Self>) {
        if self.duration == duration_before {
            self.position_publication.update(cx, |_, cx| cx.notify());
        } else {
            cx.notify();
        }
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let mut state = Self {
            current_track: None,
            queue: Vec::new(),
            source_queue: Vec::new(),
            queue_index: 0,
            is_playing: false,
            meaningful_activity_source: Uuid::new_v4(),
            progress: 0.0,
            current_time: 0.0,
            duration: 0.0,
            listening_shuffle_enabled: None,
            music_shuffle_seed: 0,
            file_shuffle_seed: 0,
            listening_shuffle_seed: 0,
            listening_repeat_mode: None,
            show_now_playing: false,
            show_queue_sheet: false,
            show_playlist_listeners_sheet: false,
            show_listening_session_sheet: false,
            playback_scope_id: None,
            last_playback_error: None,
            playback_failure_serial: 0,
            is_preparing_playback: false,
            playback_download_track_id: None,
            playback_download_progress: None,
            listening_session: None,
            departed_listening_users: HashMap::new(),
            listening_session_server_time_ms: 0,
            blocked_listening_control: None,
            listening_permission_request_notice: None,
            last_listening_error: None,
            pending_listening_session_transition: None,
            is_listening_session_transitioning: false,
            listening_session_transition_target_user_id: None,
            audio_effect_settings: MusicAudioEffectSettings::off(),
            audio_effect_settings_before_reset: None,
            show_audio_effects_sheet: false,
            playback_ad_hoc_effects_key: None,
            ad_hoc_audio_effects: HashMap::new(),
            ad_hoc_audio_effects_order: Vec::new(),
            listening_consent_episode_active: false,
            session_paused_by_local_interruption: false,
            room_left_for_ad_hoc: None,
            record_play_on_success: false,
            playback_owner_user_id: None,
            playback_owner_account_epoch: None,
            now_playing_artwork_identity: None,
            now_playing_artwork: None,
            now_playing_artwork_task: None,
            now_playing_artwork_generation: Uuid::new_v4(),
            playback_download_candidate_id: None,
            playback_download_retry_attempt: 0,
            playback_download_commit_lease: None,
            background_task: None,
            prefetch_task: None,
            is_handling_track_end: false,
            player: None,
            file_player: None,
            effect_player: None,
            effect_finish_observer: None,
            effect_handoff_task: None,
            local_playback_url: None,
            progress_timer: None,
            position_publication: cx.new(|_| MusicPlayerPositionPublication),
            last_watchdog_playback_time: -1.0,
            time_observer: None,
            item_status_observer: None,
            end_observer_token: None,
            file_finish_observer: None,
            prepare_task: None,
            prepare_generation: 0,
            call_playback_request_generation: 0,
            pending_playback_after_call: None,
            paused_for_call: false,
            session_continuation_task: None,
            session_continuation_generation: 0,
            last_download_error: None,
            route_change_observer: None,
            interruption_observer: None,
            app_lifecycle_observers: Vec::new(),
            playback_signal_task: None,
            media_keys: None,
            media_service_task: None,
            media_service_cancelled: false,
            media_window: None,
            playback_observers_installed: false,
            audio_session_activated: false,
            is_audio_session_interrupted: false,
            was_playing_before_interruption: false,
            background_keep_alive_task: None,
            listening_session_observer_id: None,
            audio_effect_commit_tasks: HashMap::new(),
            audio_effect_actor_user_id: None,
            listening_state_publish_task: None,
            listening_apply_task: None,
            is_applying_listening_session: false,
            last_applied_listening_generation: -1,
            last_applied_listening_revision: -1,
            presence_playlist_id: None,
            listening_observation_playlist_id: None,
            suppress_next_listening_playback_publish: Rc::new(Cell::new(false)),
            seek_operation_generation: 0,
            active_listening_session_transition: None,
            listening_session_transition_phase: None,
            listening_session_transition_success_callbacks: Vec::new(),
            listening_session_transition_error_callbacks: Vec::new(),
            listening_session_transition_cancelled_callbacks: Vec::new(),
            is_listening_session_exit_in_flight: false,
            listening_session_exit_session_id: None,
            external_duck_active: false,
            external_resume_intent: false,
            duck_origin_was_playing: false,
            external_audio_owners: HashSet::new(),
            suppress_external_resume_during_user_handoff: false,
            resume_playback_when_foreground: false,
            duck_fade_epoch: 0,
            av_player_fade_timer: None,
            av_player_fade_start: Duration::ZERO,
            av_player_fade_duration: 0.0,
            av_player_fade_start_volume: 0.0,
            av_player_fade_target_volume: 1.0,
            av_player_fade_completion: None,
        };
        state.install_playback_observers_if_needed(cx);
        cx.spawn(async move |this, cx| {
            this.update(cx, |this, cx| {
                let settings = MusicAudioEffectStore::shared(cx).read(cx).settings().clone();
                this.set_audio_effect_settings(settings, cx);
                this.install_listening_session_observer(cx);
            })
            .ok();
        })
        .detach();
        state
    }

    pub(crate) fn set_is_playing(&mut self, is_playing: bool, cx: &mut Context<Self>) {
        let old_value = self.is_playing;
        self.is_playing = is_playing;
        cx.notify();
        if old_value == is_playing {
            return;
        }
        let this = cx.weak_entity();
        let source = self.meaningful_activity_source;
        cx.defer(move |cx| {
            let Some(state) = this.upgrade() else { return };
            let active = state.read(cx).is_playing;
            let weak = state.downgrade();
            RiseMeaningfulActivity::shared(cx).update(cx, |activity, cx| {
                activity.set_source(
                    source,
                    active,
                    move |cx| {
                        weak.upgrade().is_some_and(|state| {
                            let state = state.read(cx);
                            state.is_playing && state.current_track.is_some()
                        })
                    },
                    cx,
                );
            });
        });
    }

    pub fn show_now_playing(&self) -> bool {
        self.show_now_playing
    }

    pub fn set_show_now_playing(&mut self, show_now_playing: bool, cx: &mut Context<Self>) {
        self.show_now_playing = show_now_playing;
        cx.notify();
        let open = show_now_playing;
        cx.defer(move |cx| {
            GuideFacts::set(RiseGuideSignal::NowPlayingOpened, open, cx);
            GuideFacts::set(RiseGuideSignal::NowPlayingClosed, !open, cx);
        });
    }

    pub fn last_playback_error(&self) -> Option<&str> {
        self.last_playback_error.as_deref()
    }

    pub fn set_last_playback_error(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        if error.is_some() {
            self.playback_failure_serial = self.playback_failure_serial.wrapping_add(1);
        }
        self.last_playback_error = error;
        cx.notify();
    }

    pub fn playback_failure_serial(&self) -> u64 {
        self.playback_failure_serial
    }

    pub fn source_queue(&self) -> &[TrackModel] {
        &self.source_queue
    }

    pub fn playback_download_track_id(&self) -> Option<&str> {
        self.playback_download_track_id.as_deref()
    }

    pub fn playback_download_progress(&self) -> Option<MusicAudioDownloadProgress> {
        self.playback_download_progress
    }

    pub fn is_listening_session_transitioning(&self) -> bool {
        self.is_listening_session_transitioning
    }

    pub fn listening_session_transition_target_user_id(&self) -> Option<&str> {
        self.listening_session_transition_target_user_id.as_deref()
    }

    pub fn set_listening_session(
        &mut self,
        listening_session: Option<MusicPlaylistListeningSession>,
        cx: &mut Context<Self>,
    ) {
        let old_value = std::mem::replace(&mut self.listening_session, listening_session);
        if self.listening_session.as_ref().map(|session| &session.session_id)
            != old_value.as_ref().map(|session| &session.session_id)
        {
            self.audio_effect_settings_before_reset = None;
            self.departed_listening_users.clear();
        }
        if let (Some(listening_session), Some(old_value)) = (&self.listening_session, &old_value)
            && listening_session.session_id == old_value.session_id
        {
            for participant in &old_value.participants {
                if !listening_session.participants.iter().any(|kept| kept.user_id == participant.user_id) {
                    self.departed_listening_users.insert(participant.user_id.clone(), participant.user.clone());
                }
            }
        }
        let active = self.listening_session.is_some();
        let granted = self.listening_session.as_ref().is_some_and(|session| {
            session
                .participants
                .iter()
                .any(|participant| !participant.permissions.granted_controls().is_empty())
        });
        cx.defer(move |cx| {
            GuideFacts::set(RiseGuideSignal::ListeningSessionActive, active, cx);
            GuideFacts::set(RiseGuideSignal::ListeningPermissionGranted, granted, cx);
        });
        if let (Some(listening_session), Some(old_value)) = (&self.listening_session, &old_value)
            && listening_session.session_id == old_value.session_id
        {
            let known: HashSet<&str> =
                old_value.participants.iter().map(|participant| participant.user_id.as_str()).collect();
            let host_id = listening_session.host_user_id.clone();
            if let Some(joined) = listening_session
                .participants
                .iter()
                .find(|participant| {
                    !known.contains(participant.user_id.as_str()) && participant.user_id != host_id
                })
                .cloned()
            {
                cx.defer(move |cx| Self::announce_joined_participant(&joined, &host_id, cx));
            }
        }
        cx.notify();
    }

    fn announce_joined_participant(joined: &MusicListeningParticipant, host_id: &str, cx: &mut App) {
        let viewer = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        if viewer.as_deref() != Some(host_id) {
            return;
        }
        let display_name = trimming_whitespaces_and_newlines(if joined.user.name.is_empty() {
            &joined.user.tag
        } else {
            &joined.user.name
        })
        .to_owned();
        let title = if display_name.is_empty() {
            tr("music_live_user_joined")
        } else {
            format!("{display_name} {}", tr("music_live_participant_joined"))
        };
        Notifier::send(
            title,
            NotifierSendOptions {
                r#type: NotifierType::Info,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("person.2.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn audio_effect_settings(&self) -> &MusicAudioEffectSettings {
        &self.audio_effect_settings
    }

    fn set_audio_effect_settings(
        &mut self,
        audio_effect_settings: MusicAudioEffectSettings,
        cx: &mut Context<Self>,
    ) {
        if audio_effect_settings != self.audio_effect_settings {
            self.audio_effect_settings_before_reset = None;
        }
        self.audio_effect_settings = audio_effect_settings;
        let active = self.audio_effect_settings.is_any_enabled();
        cx.defer(move |cx| GuideFacts::set(RiseGuideSignal::MusicEffectActive, active, cx));
        cx.notify();
    }

    pub fn audio_effect_settings_before_reset(&self) -> Option<&MusicAudioEffectSettings> {
        self.audio_effect_settings_before_reset.as_ref()
    }

    pub fn show_audio_effects_sheet(&self) -> bool {
        self.show_audio_effects_sheet
    }

    pub fn set_show_audio_effects_sheet(&mut self, show: bool, cx: &mut Context<Self>) {
        let old_value = self.show_audio_effects_sheet;
        self.show_audio_effects_sheet = show;
        let open = show;
        let closed = old_value && !open;
        cx.defer(move |cx| {
            GuideFacts::set(RiseGuideSignal::MusicEffectsSheetOpened, open, cx);
            if closed {
                GuideSignalBus::emit(RiseGuideSignal::MusicEffectsSheetClosed);
            }
        });
        cx.notify();
    }

    pub fn playback_ad_hoc_effects_key(&self) -> Option<&str> {
        self.playback_ad_hoc_effects_key.as_deref()
    }

    fn set_playback_ad_hoc_effects_key(&mut self, key: Option<String>, cx: &mut Context<Self>) {
        if key != self.playback_ad_hoc_effects_key {
            self.audio_effect_settings_before_reset = None;
        }
        self.playback_ad_hoc_effects_key = key;
        cx.notify();
    }

    pub fn listening_consent_episode_active(&self) -> bool {
        self.listening_consent_episode_active
    }

    pub fn audio_effect_actor_user_id(&self) -> Option<&str> {
        self.audio_effect_actor_user_id.as_deref()
    }

    pub fn is_shuffled(&self, cx: &App) -> bool {
        self.is_shuffled_ad_hoc(self.playback_ad_hoc_effects_key.is_some(), cx)
    }

    /// shuffle for an ad-hoc or music context, current or about to start
    /// ad-hoc keeps file prefs and never follows the room
    fn is_shuffled_ad_hoc(&self, ad_hoc: bool, cx: &App) -> bool {
        if self.listening_session.is_some()
            && !ad_hoc
            && let Some(listening_shuffle_enabled) = self.listening_shuffle_enabled
        {
            return listening_shuffle_enabled;
        }
        let scope = if ad_hoc {
            MusicPlaybackPreferenceScope::Files
        } else {
            MusicPlaybackPreferenceScope::Music
        };
        MusicPlaybackPreferences::try_read(cx).is_some_and(|preferences| preferences.is_shuffled(scope))
    }

    pub fn shuffle_seed(&self) -> i64 {
        self.shuffle_seed_ad_hoc(self.playback_ad_hoc_effects_key.is_some())
    }

    pub fn set_shuffle_seed(&mut self, seed: i64) {
        self.set_shuffle_seed_ad_hoc(seed, self.playback_ad_hoc_effects_key.is_some());
    }

    fn shuffle_seed_ad_hoc(&self, ad_hoc: bool) -> i64 {
        if self.listening_session.is_some() && !ad_hoc {
            return self.listening_shuffle_seed;
        }
        if ad_hoc { self.file_shuffle_seed } else { self.music_shuffle_seed }
    }

    fn set_shuffle_seed_ad_hoc(&mut self, seed: i64, ad_hoc: bool) {
        if self.listening_session.is_some() && !ad_hoc {
            self.listening_shuffle_seed = seed;
        } else if ad_hoc {
            self.file_shuffle_seed = seed;
        } else {
            self.music_shuffle_seed = seed;
        }
    }

    pub fn repeat_mode(&self, cx: &App) -> MusicRepeatMode {
        if self.listening_session.is_some()
            && !self.is_listening_playback_displaced_by_ad_hoc()
            && let Some(listening_repeat_mode) = self.listening_repeat_mode
        {
            return listening_repeat_mode;
        }
        MusicPlaybackPreferences::try_read(cx).map_or(MusicRepeatMode::Off, |preferences| {
            preferences.repeat_mode(self.playback_preference_scope())
        })
    }

    /// legacy tempo pair, still sent so old servers and clients stay in sync on that one effect
    pub fn audio_effect_enabled(&self) -> bool {
        self.audio_effect_settings.slowed_enabled()
    }

    pub fn audio_effect_rate(&self) -> f64 {
        self.audio_effect_settings.rate()
    }

    /// how fast media time moves against the wall clock for what this device actually renders
    /// thermal guard can hold effects back
    pub fn active_playback_rate_multiplier(&self, cx: &App) -> f64 {
        self.rendered_audio_effect_settings(cx).playback_rate().max(0.1)
    }

    /// true if any effect is on, player chrome lights up on this not tempo alone
    /// vinyl crackle counts too
    pub fn has_active_audio_effect(&self) -> bool {
        self.audio_effect_settings.is_any_enabled()
    }

    /// rate the graph really renders at
    /// now playing extrapolates time from it, so reporting 1.0 at 0.8 makes the counter run 25 % fast
    fn active_playback_rate(&self, cx: &App) -> f64 {
        let Some(effect_player) = &self.effect_player else { return 1.0 };
        effect_player.read(cx).settings().playback_rate()
    }

    fn effect_playback_rate_for_now_playing(&self, cx: &App) -> f64 {
        if self.is_playing { self.active_playback_rate(cx) } else { 0.0 }
    }

    pub fn is_listening_session_host(&self) -> bool {
        let Some(listening_session) = &self.listening_session else { return false };
        listening_session.host_user_id == Self::current_listening_user_id()
    }

    pub fn is_listening_session_follower(&self) -> bool {
        self.listening_session.is_some() && !self.is_listening_session_host()
    }

    pub fn is_listening_together(&self) -> bool {
        let Some(session) = &self.listening_session else { return false };
        if self.listening_session_exit_session_id.as_deref() == Some(session.session_id.as_str()) {
            return false;
        }
        if self.is_listening_session_host() { !session.participants.is_empty() } else { true }
    }

    /// nil on purpose while an ad-hoc mp3 displaces the session, its chrome must not wear the room ui
    pub fn listening_playlist_id(&self) -> Option<&str> {
        if !self.is_music_playback() {
            return None;
        }
        self.listening_session
            .as_ref()
            .map(|session| session.playlist_id.as_str())
            .or_else(|| self.playlist_id_from_playback_scope())
    }

    pub fn is_music_playback(&self) -> bool {
        let Some(current_track) = &self.current_track else { return false };
        !Self::is_ad_hoc_playback_context(self.playback_scope_id.as_deref(), current_track)
    }

    fn playback_preference_scope(&self) -> MusicPlaybackPreferenceScope {
        if self.playback_ad_hoc_effects_key.is_none() {
            MusicPlaybackPreferenceScope::Music
        } else {
            MusicPlaybackPreferenceScope::Files
        }
    }

    pub fn can_show_playlist_listeners(&self, cx: &mut App) -> bool {
        let Some(playlist_id) = self.listening_playlist_id() else { return false };
        if self.listening_session.is_some() {
            return true;
        }
        MusicServicesStore::shared(cx)
            .update(cx, |store, cx| store.playlist(playlist_id, cx))
            .is_some_and(|playlist| playlist.visibility.as_deref() == Some("local"))
    }

    /// session is attached but the player plays an ad-hoc mp3 the room never sees
    /// session state must not drive the engine and local playback must not publish
    pub fn is_listening_playback_displaced_by_ad_hoc(&self) -> bool {
        self.listening_session.is_some() && self.playback_ad_hoc_effects_key.is_some()
    }

    /// chat mp3s and profile featured tracks are ad-hoc, own effects and never join a session
    /// scope wins when present, chat tracks stay ad-hoc even without it
    pub fn is_ad_hoc_playback_context(scope_id: Option<&str>, track: &TrackModel) -> bool {
        if let Some(scope_id) = scope_id
            && !scope_id.is_empty()
        {
            return scope_id.starts_with("chat:") || scope_id.starts_with("profile-featured");
        }
        track.source_chat_id.is_some()
    }

    fn current_listening_user_id() -> String {
        AuthServicesStore::viewer_mirror().effective_user_id_for_wire.unwrap_or_default()
    }

    fn playlist_id_from_playback_scope(&self) -> Option<&str> {
        let id = self.playback_scope_id.as_deref()?.strip_prefix("playlist:")?;
        if id.is_empty() { None } else { Some(id) }
    }

    pub fn playback_scope_id(&self) -> Option<&str> {
        self.playback_scope_id.as_deref()
    }

    pub fn published_now_playing(&self) -> Option<rise_platform::media_keys::MediaSnapshot> {
        self.media_keys.as_ref().map(|media_keys| media_keys.published_snapshot())
    }

    pub fn listening_session(&self) -> Option<&MusicPlaylistListeningSession> {
        self.listening_session.as_ref()
    }

    fn install_listening_session_observer(&mut self, cx: &mut Context<Self>) {
        if self.listening_session_observer_id.is_some() {
            return;
        }
        let weak = cx.weak_entity();
        let observer_id = MusicServicesStore::shared(cx).update(cx, |store, _| {
            store.add_listening_session_update_observer(Box::new(
                move |update: &MusicListeningSessionUpdate, cx: &mut App| {
                    let update = update.clone();
                    let weak = weak.clone();
                    cx.defer(move |cx| {
                        weak.update(cx, |this, cx| this.handle_listening_session_update(update, cx)).ok();
                    });
                },
            ))
        });
        self.listening_session_observer_id = Some(observer_id);
    }

    fn handle_listening_session_update(&mut self, update: MusicListeningSessionUpdate, cx: &mut Context<Self>) {
        match update {
            MusicListeningSessionUpdate::Session { session, server_time_ms, actor_user_id } => {
                let viewer = Self::current_listening_user_id();
                let is_current_session = self.current_session_id() == Some(session.session_id.as_str());
                let is_viewer = session.host_user_id == viewer
                    || session.participants.iter().any(|participant| participant.user_id == viewer);
                let is_current_playlist =
                    self.playlist_id_from_playback_scope() == Some(session.playlist_id.as_str());
                if !(is_current_session || (is_viewer && is_current_playlist)) {
                    return;
                }
                self.attach_playlist_listening_session(session, server_time_ms, actor_user_id, cx);
            }

            MusicListeningSessionUpdate::Ended { session_id, reason } => {
                if self.current_session_id() != Some(session_id.as_str()) {
                    return;
                }
                let was_host = self.is_listening_session_host();
                if self.listening_session_exit_session_id.as_deref() == Some(session_id.as_str()) {
                    self.is_listening_session_exit_in_flight = false;
                    self.listening_session_exit_session_id = None;
                }
                self.detach_playlist_listening_session(&reason, !was_host, true, cx);
                self.continue_listening_session_transition_after_current_ended(&session_id, cx);
            }

            MusicListeningSessionUpdate::AccessRevoked { playlist_id, reason } => {
                if self.playlist_id_from_playback_scope() != Some(playlist_id.as_str()) {
                    return;
                }
                if self.listening_session.is_some() {
                    self.detach_playlist_listening_session("access_revoked", false, false, cx);
                }
                self.stop_and_clear(cx);
                if reason == music_service::PLAYLIST_MEMBER_EXIT_LEFT {
                    return;
                }
                let is_subscription_suspension = reason == music_service::PLAYLIST_ACCESS_PAUSED_REASON;
                let is_deleted = reason == music_service::PLAYLIST_REVOKED_REASON_DELETED;
                let title_key = if is_subscription_suspension {
                    "music_playlist_suspended_title"
                } else if is_deleted {
                    "music_playlist_deleted"
                } else {
                    "music_playlist_access_removed_title"
                };
                let content = if is_subscription_suspension {
                    Some(tr("music_playlist_suspended_hint"))
                } else if is_deleted {
                    None
                } else {
                    Some(tr("music_playlist_access_removed_hint"))
                };
                Notifier::send(
                    tr(title_key),
                    NotifierSendOptions {
                        content,
                        r#type: NotifierType::Error,
                        position: NotifierPosition::Top,
                        icon: NotifierIcon::System("lock.fill".to_owned()),
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            }

            MusicListeningSessionUpdate::AccessRestored => {
                Notifier::send(
                    tr("music_playlist_restored_title"),
                    NotifierSendOptions {
                        content: Some(tr("music_playlist_restored_hint")),
                        r#type: NotifierType::Success,
                        position: NotifierPosition::Top,
                        icon: NotifierIcon::System("lock.open.fill".to_owned()),
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            }

            MusicListeningSessionUpdate::PermissionRequested { session_id, participant_user_id, permissions } => {
                if self.current_session_id() != Some(session_id.as_str()) || !self.is_listening_session_host() {
                    return;
                }
                let requester = self
                    .listening_session
                    .as_ref()
                    .and_then(|session| {
                        session.participants.iter().find(|participant| participant.user_id == participant_user_id)
                    })
                    .map(|participant| {
                        trimming_whitespaces_and_newlines(if participant.user.name.is_empty() {
                            &participant.user.tag
                        } else {
                            &participant.user.name
                        })
                        .to_owned()
                    });
                self.listening_permission_request_notice = Some(MusicListeningPermissionRequestNotice {
                    session_id,
                    participant_user_id,
                    permissions,
                });
                cx.notify();
                let title = match requester.filter(|requester| !requester.is_empty()) {
                    Some(requester) => {
                        tr_args("music_live_permission_requested_by", &[FormatArg::Text(requester.into())])
                    }
                    None => tr("music_live_permission_requested"),
                };
                Notifier::send(
                    title,
                    NotifierSendOptions {
                        content: if permissions.granted_controls().is_empty() {
                            None
                        } else {
                            Some(permissions.localized_granted_list())
                        },
                        r#type: NotifierType::Info,
                        position: NotifierPosition::Top,
                        icon: NotifierIcon::System("person.badge.key.fill".to_owned()),
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            }

            MusicListeningSessionUpdate::ParticipantLeft { session_id, participant_user_id, participant_user } => {
                let Some(session) = self.listening_session.clone() else { return };
                if session.session_id != session_id || participant_user_id == Self::current_listening_user_id() {
                    return;
                }
                let cached_user = session
                    .participants
                    .iter()
                    .find(|participant| participant.user_id == participant_user_id)
                    .map(|participant| participant.user.clone());
                let departed_user = self.departed_listening_users.remove(&participant_user_id);
                self.set_listening_session(Some(session.removing_participant(&participant_user_id)), cx);
                let display_name = [participant_user, cached_user, departed_user].into_iter().flatten().find_map(|user| {
                    let name = trimming_whitespaces_and_newlines(&user.name);
                    let tag = trimming_whitespaces_and_newlines(&user.tag);
                    if !name.is_empty() {
                        Some(name.to_owned())
                    } else if tag.is_empty() {
                        None
                    } else {
                        Some(tag.to_owned())
                    }
                });
                let title = [display_name.filter(|name| !name.is_empty()), Some(tr("music_live_participant_left"))]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" ");
                Notifier::send(
                    title,
                    NotifierSendOptions {
                        r#type: NotifierType::Info,
                        position: NotifierPosition::Top,
                        icon: NotifierIcon::System("person.crop.circle.badge.minus".to_owned()),
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            }

            MusicListeningSessionUpdate::PermissionsUpdated { session_id, participant_user_id, permissions } => {
                let Some(session) = self.listening_session.clone() else { return };
                if session.session_id != session_id {
                    return;
                }
                let previous_permissions = session.viewer_permissions;
                let viewer = Self::current_listening_user_id();
                self.set_listening_session(
                    Some(session.updating_participant_permissions(&participant_user_id, permissions, &viewer)),
                    cx,
                );
                if participant_user_id != viewer {
                    return;
                }

                if let Some(blocked) = self.blocked_listening_control
                    && permissions.allows(blocked)
                {
                    self.blocked_listening_control = None;
                    cx.notify();
                }

                let granted: Vec<MusicListeningBlockedControl> = permissions
                    .granted_controls()
                    .into_iter()
                    .filter(|control| !previous_permissions.allows(*control))
                    .collect();
                let revoked: Vec<MusicListeningBlockedControl> = previous_permissions
                    .granted_controls()
                    .into_iter()
                    .filter(|control| !permissions.allows(*control))
                    .collect();
                if granted.is_empty() && revoked.is_empty() {
                    return;
                }

                let granted_notice = (!granted.is_empty()).then(|| {
                    tr_args(
                        "music_live_permissions_granted_toast",
                        &[FormatArg::Text(Self::localized_control_names(&granted).into())],
                    )
                });
                let title = if revoked.is_empty() {
                    granted_notice.clone().unwrap_or_default()
                } else {
                    tr("music_live_permissions_revoked_toast")
                };
                let content = (!revoked.is_empty()).then(|| {
                    [Some(Self::localized_control_names(&revoked)), granted_notice]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join("\n")
                });
                Notifier::send(
                    title,
                    NotifierSendOptions {
                        content,
                        r#type: if revoked.is_empty() { NotifierType::Success } else { NotifierType::Warning },
                        position: NotifierPosition::Top,
                        icon: NotifierIcon::System(
                            if revoked.is_empty() { "person.badge.key.fill" } else { "person.badge.minus" }.to_owned(),
                        ),
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            }
        }
    }

    fn current_session_id(&self) -> Option<&str> {
        self.listening_session.as_ref().map(|session| session.session_id.as_str())
    }

    fn localized_control_names(controls: &[MusicListeningBlockedControl]) -> String {
        controls
            .iter()
            .map(MusicListeningBlockedControlExt::localized_name)
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn is_stale_listening_session(&self, session: &MusicPlaylistListeningSession) -> bool {
        let Some(current) = &self.listening_session else { return false };
        if current.session_id != session.session_id {
            return false;
        }
        session.generation < current.generation
            || (session.generation == current.generation && session.revision < current.revision)
            || session.generation < self.last_applied_listening_generation
            || (session.generation == self.last_applied_listening_generation
                && session.revision < self.last_applied_listening_revision)
    }

    pub fn attach_playlist_listening_session(
        &mut self,
        session: MusicPlaylistListeningSession,
        server_time_ms: i64,
        actor_user_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.listening_session_exit_session_id.as_deref() == Some(session.session_id.as_str())
            || self.is_stale_listening_session(&session)
        {
            return;
        }
        let viewer = Self::current_listening_user_id();
        let previous_id = self.current_session_id().map(str::to_owned);
        let replacing_session = previous_id.as_deref() != Some(session.session_id.as_str());
        if replacing_session && let Some(previous_id) = previous_id {
            Self::music_interactions_deferred(cx, move |store, cx|
                store.stop_playlist_listening_session_sync(&previous_id, cx)
            );
            self.listening_apply_task = None;
            self.last_applied_listening_generation = -1;
            self.last_applied_listening_revision = -1;
        }
        let is_host_of_foreign_join = replacing_session
            && session.host_user_id == viewer
            && self.current_track.is_some()
            && actor_user_id.as_deref().is_some_and(|actor| !actor.is_empty() && actor != viewer);
        if replacing_session {
            self.listening_consent_episode_active = false;
            self.session_paused_by_local_interruption = false;
            if self.playback_ad_hoc_effects_key.is_some() && !is_host_of_foreign_join {
                self.clear_playback_engine_and_context(cx);
            }
        }

        let should_apply = replacing_session
            || session.generation > self.last_applied_listening_generation
            || (session.generation == self.last_applied_listening_generation
                && session.revision > self.last_applied_listening_revision);

        self.set_listening_session(Some(session.clone()), cx);
        self.listening_session_server_time_ms = server_time_ms;
        self.last_listening_error = None;
        cx.notify();
        let started_session_id = session.session_id.clone();
        Self::music_interactions_deferred(cx, move |store, cx|
            store.start_playlist_listening_session_sync(&started_session_id, cx)
        );

        if !should_apply {
            self.correct_listening_session_drift_if_needed(&session, cx);
            return;
        }
        self.last_applied_listening_generation = session.generation;
        self.last_applied_listening_revision = session.revision;

        if is_host_of_foreign_join {
            if !self.is_listening_playback_displaced_by_ad_hoc()
                && self.playlist_id_from_playback_scope() == Some(session.playlist_id.as_str())
                && let Some(track_id) = self.current_track.as_ref().map(|track| track.id.clone())
            {
                let command = if track_id == session.track_id {
                    MusicListeningSessionCommand::Playback
                } else {
                    MusicListeningSessionCommand::ChangeTrack
                };
                self.publish_listening_session_state(command, 0, None, cx);
                self.refresh_playlist_listening_presence(cx);
            }
            return;
        }

        if !Self::should_apply_accepted_listening_state(actor_user_id.as_deref(), &viewer) {
            self.correct_listening_session_drift_if_needed(&session, cx);
            return;
        }

        self.apply_listening_session_state(session, server_time_ms, actor_user_id.as_deref(), cx);
    }

    pub fn should_apply_accepted_listening_state(actor_user_id: Option<&str>, current_user_id: &str) -> bool {
        let Some(actor_user_id) = actor_user_id.filter(|actor| !actor.is_empty()) else { return true };
        actor_user_id != current_user_id
    }

    fn correct_listening_session_drift_if_needed(
        &mut self,
        session: &MusicPlaylistListeningSession,
        cx: &mut Context<Self>,
    ) {
        if self.is_listening_playback_displaced_by_ad_hoc() || self.is_preparing_playback {
            return;
        }
        let now_ms = RiseClock::now().ms();
        let services = MusicServicesStore::shared(cx);
        let (projected_ms, server_time_ms) = {
            let services = services.read(cx);
            (
                services.projected_position_ms_for_session(session, now_ms),
                services.estimated_playlist_server_time_ms(&session.playlist_id, now_ms),
            )
        };
        let drift_ms = ((self.current_time.max(0.0) * 1_000.0) as i64 - projected_ms).abs();
        let needs_correction = self.current_track.as_ref().map(|track| track.id.as_str())
            != Some(session.track_id.as_str())
            || self.is_playing != session.is_playing
            || self.is_shuffled(cx) != session.shuffle_enabled
            || (session.shuffle_enabled && self.shuffle_seed() != session.shuffle_seed)
            || self.repeat_mode_wire_value(cx) != session.repeat_mode
            || drift_ms > Self::LISTENING_DRIFT_CORRECTION_MS;
        if !needs_correction {
            return;
        }
        self.apply_listening_session_state(session.clone(), server_time_ms, None, cx);
    }

    fn apply_listening_session_state(
        &mut self,
        session: MusicPlaylistListeningSession,
        server_time_ms: i64,
        actor_user_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if self.is_listening_playback_displaced_by_ad_hoc() {
            return;
        }

        if let Some(actor_user_id) = actor_user_id
            && !actor_user_id.is_empty()
            && actor_user_id != Self::current_listening_user_id()
            && self.external_audio_owners.is_empty()
        {
            self.listening_consent_episode_active = false;
            self.session_paused_by_local_interruption = false;
        }

        let desired_position_ms = {
            let services = MusicServicesStore::shared(cx);
            services.read(cx).projected_position_ms_for_session(&session, RiseClock::now().ms())
        };
        let repeat_value = MusicRepeatMode::from_wire(&session.repeat_mode);

        self.adopt_session_audio_effect(&session, actor_user_id, cx);

        self.is_applying_listening_session = true;
        let shuffle_changed =
            self.is_shuffled(cx) != session.shuffle_enabled || self.shuffle_seed() != session.shuffle_seed;
        self.listening_shuffle_enabled = Some(session.shuffle_enabled);
        self.listening_shuffle_seed = if session.shuffle_enabled { session.shuffle_seed } else { 0 };
        self.listening_repeat_mode = Some(repeat_value);
        cx.notify();

        if self.current_track.as_ref().map(|track| track.id.as_str()) != Some(session.track_id.as_str()) {
            let tracks: Vec<TrackModel> = {
                MusicServicesStore::shared(cx)
                    .update(cx, |store, cx| store.playlist(&session.playlist_id, cx))
                    .and_then(|playlist| playlist.tracks)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(RiseStoredMusicTrack::as_track_model)
                    .collect()
            };
            let Some(track) = tracks.iter().find(|track| track.id == session.track_id).cloned() else {
                self.is_applying_listening_session = false;
                if self.is_playing || self.is_preparing_playback {
                    self.pause_playback_engine(cx);
                }
                let unseen_playlist_id = session.playlist_id.clone();
                Self::music_interactions_deferred(cx, move |store, cx|
                    store.open_playlist(&unseen_playlist_id, cx)
                );
                self.schedule_listening_state_retry(session, server_time_ms, cx);
                return;
            };

            self.suppress_next_listening_playback_publish.set(true);
            let scope_id = format!("playlist:{}", session.playlist_id);
            self.play(
                track,
                MusicPlayOptions {
                    queue: tracks,
                    scope_id: Some(scope_id),
                    preserve_queue: false,
                    ..MusicPlayOptions::default()
                },
                cx,
            );
            self.is_applying_listening_session = false;
            self.schedule_listening_engine_alignment(session, cx);
            return;
        }

        if shuffle_changed && let Some(track) = self.current_track.clone() {
            self.rebuild_playback_queue(&track, cx);
        }

        if self.is_preparing_playback {
            self.is_applying_listening_session = false;
            self.schedule_listening_engine_alignment(session, cx);
            return;
        }

        self.align_listening_engine(desired_position_ms, session.is_playing, cx);
        self.is_applying_listening_session = false;
        self.refresh_playlist_listening_presence(cx);
    }

    fn schedule_listening_state_retry(
        &mut self,
        session: MusicPlaylistListeningSession,
        server_time_ms: i64,
        cx: &mut Context<Self>,
    ) {
        self.listening_apply_task = None;
        self.listening_apply_task = Some(cx.spawn(async move |this, cx| {
            for _ in 0..Self::LISTENING_STATE_RETRY_ATTEMPTS {
                cx.background_executor()
                    .timer(Duration::from_millis(Self::LISTENING_STATE_RETRY_INTERVAL_MS))
                    .await;
                let outcome = this.update(cx, |this, cx| {
                    if this.current_session_id() != Some(session.session_id.as_str()) {
                        return ControlFlow::Break(());
                    }
                    let latest = this.listening_session.clone().unwrap_or_else(|| session.clone());
                    let has_track = MusicServicesStore::shared(cx)
                        .update(cx, |store, cx| store.playlist(&latest.playlist_id, cx))
                        .and_then(|playlist| playlist.tracks)
                        .is_some_and(|tracks| {
                            tracks.iter().any(|track| track.id.as_deref() == Some(latest.track_id.as_str()))
                        });
                    if !has_track {
                        return ControlFlow::Continue(());
                    }
                    this.apply_listening_session_state(latest, server_time_ms, None, cx);
                    ControlFlow::Break(())
                });
                if !matches!(outcome, Ok(ControlFlow::Continue(()))) {
                    return;
                }
            }

            this.update(cx, |this, cx| {
                if this.current_session_id() == Some(session.session_id.as_str()) {
                    this.report_listening_error(tr("music_live_track_unavailable"), cx);
                }
            })
            .ok();
        }));
    }

    fn schedule_listening_engine_alignment(&mut self, session: MusicPlaylistListeningSession, cx: &mut Context<Self>) {
        let reset = MusicListeningPublishSuppressionReset(self.suppress_next_listening_playback_publish.clone());
        self.listening_apply_task = None;
        self.listening_apply_task = Some(cx.spawn(async move |this, cx| {
            let _reset = reset;
            for _ in 0..Self::LISTENING_ENGINE_ALIGNMENT_ATTEMPTS {
                cx.background_executor()
                    .timer(Duration::from_millis(Self::LISTENING_ENGINE_ALIGNMENT_INTERVAL_MS))
                    .await;
                let outcome = this.update(cx, |this, cx| {
                    if this.current_session_id() != Some(session.session_id.as_str())
                        || this.current_track.as_ref().map(|track| track.id.as_str())
                            != Some(session.track_id.as_str())
                    {
                        return ControlFlow::Break(());
                    }
                    if this.is_preparing_playback {
                        return ControlFlow::Continue(());
                    }

                    let latest = this.listening_session.clone().unwrap_or_else(|| session.clone());
                    let aligned_position_ms = {
                        let services = MusicServicesStore::shared(cx);
                        services.read(cx).projected_position_ms_for_session(&latest, RiseClock::now().ms())
                    };
                    this.is_applying_listening_session = true;
                    this.align_listening_engine(aligned_position_ms, latest.is_playing, cx);
                    this.is_applying_listening_session = false;
                    this.refresh_playlist_listening_presence(cx);
                    ControlFlow::Break(())
                });
                if !matches!(outcome, Ok(ControlFlow::Continue(()))) {
                    return;
                }
            }
        }));
    }

    pub fn reset_or_restore_audio_effects(&mut self, cx: &mut Context<Self>) {
        if !self.audio_effect_settings.is_any_enabled() && self.audio_effect_settings_before_reset.is_none() {
            return;
        }
        if !self.allows_listening_control(MusicListeningBlockedControl::ApplyEffects, false, cx) {
            return;
        }

        if self.audio_effect_settings.is_any_enabled() {
            let previous = self.audio_effect_settings.clone();
            let mut cleared = previous.clone();
            for kind in MusicAudioEffectKind::all_cases() {
                cleared.set_enabled(*kind, false);
            }
            self.commit_audio_effect_settings(cleared, None, false, cx);
            self.audio_effect_settings_before_reset = Some(previous);
            cx.notify();
        } else if let Some(previous) = self.audio_effect_settings_before_reset.clone() {
            self.commit_audio_effect_settings(previous, None, false, cx);
        }
    }

    pub fn set_audio_effect(&mut self, kind: MusicAudioEffectKind, enabled: bool, cx: &mut Context<Self>) {
        if enabled == self.audio_effect_settings.is_enabled(kind) {
            return;
        }
        if !self.allows_listening_control(MusicListeningBlockedControl::ApplyEffects, false, cx) {
            return;
        }
        let mut next = self.audio_effect_settings.clone();
        next.set_enabled(kind, enabled);
        self.commit_audio_effect_settings(next, Some(kind), false, cx);
    }

    pub fn set_audio_effect_value(&mut self, kind: MusicAudioEffectKind, value: f64, cx: &mut Context<Self>) {
        let quantized = kind.quantize(value);
        if (quantized - self.audio_effect_settings.value(kind)).abs() <= 0.0001 {
            return;
        }
        if !self.allows_listening_control(MusicListeningBlockedControl::ApplyEffects, false, cx) {
            return;
        }
        let mut next = self.audio_effect_settings.clone();
        next.set_value(kind, quantized);
        self.commit_audio_effect_settings(next, Some(kind), true, cx);
    }

    /// shows the listening refusal to the user too
    /// last_listening_error is read by no view, so it failed silently
    pub fn report_listening_error(&mut self, error: String, cx: &mut Context<Self>) {
        let detail = trimming_whitespaces_and_newlines(&error).to_owned();
        self.last_listening_error = Some(error);
        cx.notify();
        Notifier::send(
            tr("music_live_action_failed"),
            NotifierSendOptions {
                content: if detail.is_empty() { None } else { Some(detail) },
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    /// read only check for the ui if a control is locked, never raises the alert
    pub fn is_listening_control_locked(&self, control: MusicListeningBlockedControl) -> bool {
        let Some(session) = &self.listening_session else { return false };
        if session.host_user_id == Self::current_listening_user_id() {
            return false;
        }
        match control {
            MusicListeningBlockedControl::Playback => !session.viewer_permissions.can_control_playback,
            MusicListeningBlockedControl::Seek => !session.viewer_permissions.can_seek,
            MusicListeningBlockedControl::ChangeTracks => !session.viewer_permissions.can_change_tracks,
            MusicListeningBlockedControl::ApplyEffects => !session.viewer_permissions.can_apply_effects,
        }
    }

    /// tapping a locked control, the only place a follower learns the right exists and can ask for it
    pub fn report_blocked_listening_control(&mut self, control: MusicListeningBlockedControl, cx: &mut Context<Self>) {
        if self.listening_session.is_none() {
            return;
        }
        self.blocked_listening_control = Some(control);
        cx.notify();
    }

    pub fn report_blocked_audio_effect_control(&mut self, cx: &mut Context<Self>) {
        self.report_blocked_listening_control(MusicListeningBlockedControl::ApplyEffects, cx);
    }

    pub fn set_audio_effect_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.set_audio_effect(MusicAudioEffectKind::Slowed, enabled, cx);
    }

    pub fn set_audio_effect_rate(&mut self, rate: f64, cx: &mut Context<Self>) {
        self.set_audio_effect_value(MusicAudioEffectKind::Slowed, rate, cx);
    }

    /// settings move at once so the slider follows the finger
    /// graph rebuild and session publish are held back per effect
    fn commit_audio_effect_settings(
        &mut self,
        next: MusicAudioEffectSettings,
        kind: Option<MusicAudioEffectKind>,
        debounced: bool,
        cx: &mut Context<Self>,
    ) {
        self.set_audio_effect_settings(next.clone(), cx);
        self.persist_active_effect_settings(next, cx);
        self.audio_effect_actor_user_id = Some(Self::current_listening_user_id());

        let (true, Some(kind)) = (debounced, kind) else {
            self.cancel_pending_audio_effect_commits();
            self.apply_audio_effect_to_engine(kind.is_none(), cx);
            self.publish_listening_session_state(MusicListeningSessionCommand::Effects, 0, None, cx);
            self.refresh_audio_effect_thermal_monitoring(cx);
            return;
        };

        let rendered = self.rendered_audio_effect_settings(cx);
        let applies_live = self
            .effect_player
            .as_ref()
            .is_some_and(|effect_player| !effect_player.read(cx).requires_rebuild(&rendered));
        if applies_live {
            self.apply_audio_effect_to_engine(false, cx);
        }

        let task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(Self::EFFECT_COMMIT_DEBOUNCE_MILLISECONDS))
                .await;
            this.update(cx, |this, cx| {
                if let Some(task) = this.audio_effect_commit_tasks.remove(&kind) {
                    task.detach();
                }
                this.apply_audio_effect_to_engine(false, cx);
                this.publish_listening_session_state(
                    MusicListeningSessionCommand::Effects,
                    Self::EFFECT_PUBLISH_DEBOUNCE_MILLISECONDS,
                    None,
                    cx,
                );
                this.refresh_audio_effect_thermal_monitoring(cx);
            })
            .ok();
        });
        self.audio_effect_commit_tasks.insert(kind, task);
    }

    fn adopt_session_audio_effect(
        &mut self,
        session: &MusicPlaylistListeningSession,
        actor_user_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let next = MusicAudioEffectSettings::from_wire_effects(&session.audio_effects, self.audio_effect_settings.clone());
        if next == self.audio_effect_settings {
            self.refresh_audio_effect_thermal_monitoring(cx);
            return;
        }
        self.audio_effect_actor_user_id = actor_user_id
            .map(str::to_owned)
            .or_else(|| self.audio_effect_actor_user_id.clone())
            .or_else(|| Some(session.host_user_id.clone()));

        self.cancel_pending_audio_effect_commits();
        self.set_audio_effect_settings(next.clone(), cx);
        MusicAudioEffectStore::shared(cx).update(cx, |store, cx| store.replace(next, cx));
        self.apply_audio_effect_to_engine(false, cx);
        self.refresh_audio_effect_thermal_monitoring(cx);
    }

    fn cancel_pending_audio_effect_commits(&mut self) {
        self.audio_effect_commit_tasks.clear();
    }

    /// puts a whole chain in place at once, same persistence as a switch flip so store and engine agree
    pub fn restore_audio_effect_settings(&mut self, settings: MusicAudioEffectSettings, cx: &mut Context<Self>) {
        self.cancel_pending_audio_effect_commits();
        self.set_audio_effect_settings(settings.clone(), cx);
        self.persist_active_effect_settings(settings, cx);
        self.apply_audio_effect_to_engine(false, cx);
        self.refresh_audio_effect_thermal_monitoring(cx);
    }

    /// library playback saves to the account store, an ad-hoc mp3 keeps its choice per track
    fn persist_active_effect_settings(&mut self, next: MusicAudioEffectSettings, cx: &mut Context<Self>) {
        let Some(key) = self.playback_ad_hoc_effects_key.clone() else {
            MusicAudioEffectStore::shared(cx).update(cx, |store, cx| store.replace(next, cx));
            return;
        };
        self.ad_hoc_audio_effects_order.retain(|existing| *existing != key);
        self.ad_hoc_audio_effects_order.push(key.clone());
        self.ad_hoc_audio_effects.insert(key, next);
        while self.ad_hoc_audio_effects_order.len() > Self::AD_HOC_AUDIO_EFFECTS_LIMIT {
            let evicted = self.ad_hoc_audio_effects_order.remove(0);
            self.ad_hoc_audio_effects.remove(&evicted);
        }
    }

    /// picks the effect set for the next engine start
    /// library set or per track set of an ad-hoc mp3 (clean by default)
    fn resolve_active_effect_settings(&mut self, scope_id: Option<&str>, track: &TrackModel, cx: &mut Context<Self>) {
        let ad_hoc = Self::is_ad_hoc_playback_context(scope_id, track);
        let next_key = ad_hoc.then(|| track.id.clone());
        let active = self.active_effect_settings(next_key.as_deref(), cx);
        if next_key == self.playback_ad_hoc_effects_key && self.audio_effect_settings == active {
            return;
        }

        self.cancel_pending_audio_effect_commits();
        let has_key = next_key.is_some();
        self.set_playback_ad_hoc_effects_key(next_key, cx);
        self.set_audio_effect_settings(active, cx);
        if has_key {
            self.audio_effect_actor_user_id = Some(Self::current_listening_user_id());
        }
        self.refresh_audio_effect_thermal_monitoring(cx);
    }

    fn active_effect_settings(&self, key: Option<&str>, cx: &mut App) -> MusicAudioEffectSettings {
        let Some(key) = key else {
            return MusicAudioEffectStore::shared(cx).read(cx).settings().clone();
        };
        self.ad_hoc_audio_effects.get(key).cloned().unwrap_or_else(MusicAudioEffectSettings::off)
    }

    pub fn clear_ad_hoc_effect_memory(&mut self, cx: &mut Context<Self>) {
        self.audio_effect_settings_before_reset = None;
        self.ad_hoc_audio_effects.clear();
        self.ad_hoc_audio_effects_order.clear();
        cx.notify();
    }

    /// what the graph really renders
    /// differs from settings only when the device is too hot for someone elses chain
    pub fn rendered_audio_effect_settings(&self, cx: &App) -> MusicAudioEffectSettings {
        MusicEffectThermalGuard::try_read(cx).map_or_else(
            || self.audio_effect_settings.clone(),
            |guard| guard.rendered(&self.audio_effect_settings),
        )
    }

    /// rerenders under the current thermal verdict without touching session settings
    /// so cooling down restores the full chain
    pub fn reapply_audio_effect_rendering(&mut self, cx: &mut Context<Self>) {
        self.apply_audio_effect_to_engine(false, cx);
    }

    fn apply_audio_effect_to_engine(&mut self, replacing_chain: bool, cx: &mut Context<Self>) {
        if self.is_audio_session_interrupted {
            return;
        }
        let rendered = self.rendered_audio_effect_settings(cx);
        let is_spatialised = self.effect_player.as_ref().is_some_and(|effect_player| effect_player.read(cx).is_spatialised());
        if replacing_chain && (self.effect_player.is_some() || self.file_player.is_some()) {
            let fade = if is_spatialised || rendered.is_enabled(MusicAudioEffectKind::Spin) {
                Self::SPATIAL_HANDOFF_FADE_DURATION
            } else {
                Self::EFFECT_HANDOFF_FADE_DURATION
            };
            self.handoff_local_engine(rendered.is_any_enabled(), fade, cx);
            return;
        }
        if let Some(effect_player) = self.effect_player.clone() {
            if !rendered.is_any_enabled() {
                let fade = if is_spatialised {
                    Self::SPATIAL_HANDOFF_FADE_DURATION
                } else {
                    Self::EFFECT_HANDOFF_FADE_DURATION
                };
                self.handoff_local_engine(false, fade, cx);
                return;
            }
            if effect_player.read(cx).requires_rebuild(&rendered) {
                self.handoff_local_engine(true, Self::SPATIAL_HANDOFF_FADE_DURATION, cx);
                return;
            }
            effect_player.update(cx, |effect_player, cx| effect_player.apply(rendered, cx));
            self.refresh_now_playing_playback_state(cx);
            return;
        }
        if !rendered.is_any_enabled() {
            return;
        }
        if self.file_player.is_none() {
            let label = self.active_engine_label();
            MusicPlayerLog::info(format!("EFFECT deferred \u{2014} active engine is {label}"), cx);
            return;
        }
        let fade = if rendered.is_enabled(MusicAudioEffectKind::Spin) {
            Self::SPATIAL_HANDOFF_FADE_DURATION
        } else {
            Self::EFFECT_HANDOFF_FADE_DURATION
        };
        self.handoff_local_engine(true, fade, cx);
    }

    /// swapping backends mid track costs a file open
    /// so the old engine ducks to silence and the new one fades up
    fn handoff_local_engine(&mut self, to_effect: bool, fade: f64, cx: &mut Context<Self>) {
        let Some(track) = self.current_track.clone() else { return };
        let Some(url) = self.local_engine_file_url(cx).or_else(|| self.local_playback_url.clone()) else {
            return;
        };

        let position = self.local_engine_current_time(cx).unwrap_or(self.current_time);
        let was_playing = self.is_playing;
        self.effect_handoff_task = None;
        self.local_engine_fade_out_and_pause(fade, cx);
        self.effect_handoff_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis((fade * 1_000.0) as u64))
                .await;

            let Ok(Some(start)) = this.update(cx, |this, cx| {
                if this.current_track.as_ref().is_none_or(|current| current.id != track.id) {
                    return None;
                }
                let resume_at = if was_playing {
                    position + fade * this.active_playback_rate_multiplier(cx)
                } else {
                    position
                };
                Some(if to_effect {
                    this.start_effect_file_player(url.clone(), track.clone(), resume_at, None, false, cx)
                } else {
                    this.start_local_file_player(url.clone(), track.clone(), resume_at, false, cx)
                })
            }) else {
                return;
            };
            let started = start.await;

            if !started {
                let Ok(Some((owns_settings, spin_recovery))) = this.update(cx, |this, cx| {
                    MusicPlayerLog::fail(
                        format!("EFFECT handoff failed toEffect={to_effect} \u{2014} restoring previous engine"),
                        cx,
                    );
                    if !to_effect {
                        return None;
                    }
                    let owns_settings =
                        this.listening_session.is_none() || this.playback_ad_hoc_effects_key.is_some();
                    let rendered = this.rendered_audio_effect_settings(cx);
                    if !rendered.is_enabled(MusicAudioEffectKind::Spin) {
                        return Some((owns_settings, None));
                    }
                    let mut recovered = rendered;
                    recovered.set_enabled(MusicAudioEffectKind::Spin, false);
                    if owns_settings {
                        this.set_audio_effect_settings(recovered.clone(), cx);
                        this.persist_active_effect_settings(recovered.clone(), cx);
                    }
                    let recovered_start = recovered.is_any_enabled().then(|| {
                        this.start_effect_file_player(url.clone(), track.clone(), position, Some(recovered), false, cx)
                    });
                    Some((owns_settings, Some(recovered_start)))
                }) else {
                    return;
                };

                if let Some(recovered_start) = spin_recovery {
                    let recovered_started = match recovered_start {
                        Some(task) => task.await,
                        None => false,
                    };
                    let Ok(handled) = this.update(cx, |this, cx| {
                        if this.current_track.as_ref().is_none_or(|current| current.id != track.id) {
                            return true;
                        }
                        if !recovered_started {
                            return false;
                        }
                        this.local_engine_fade_in_and_resume(fade, cx);
                        if !was_playing {
                            this.pause_local_engine(cx);
                            this.set_is_playing(false, cx);
                        }
                        this.refresh_now_playing_playback_state(cx);
                        true
                    }) else {
                        return;
                    };
                    if handled {
                        return;
                    }
                }

                let Ok(restore) = this.update(cx, |this, cx| {
                    if owns_settings {
                        this.set_audio_effect_settings(MusicAudioEffectSettings::off(), cx);
                        this.persist_active_effect_settings(MusicAudioEffectSettings::off(), cx);
                    }
                    this.start_local_file_player(url.clone(), track.clone(), position, false, cx)
                }) else {
                    return;
                };
                let restored = restore.await;
                this.update(cx, |this, cx| {
                    if this.current_track.as_ref().is_none_or(|current| current.id != track.id) {
                        return;
                    }
                    if restored && !was_playing {
                        this.pause_local_engine(cx);
                        this.set_is_playing(false, cx);
                        this.refresh_now_playing_playback_state(cx);
                    }
                })
                .ok();
                return;
            }

            this.update(cx, |this, cx| {
                this.local_engine_fade_in_and_resume(fade, cx);
                if !was_playing {
                    this.pause_local_engine(cx);
                    this.set_is_playing(false, cx);
                }
                this.refresh_now_playing_playback_state(cx);
            })
            .ok();
        }));
    }

    fn align_listening_engine(&mut self, position_ms: i64, should_play: bool, cx: &mut Context<Self>) {
        let target_time = (position_ms as f64 / 1_000.0).max(0.0);
        let drift = (self.current_time - target_time).abs();
        if drift > Self::LISTENING_ALIGNMENT_SEEK_DRIFT && self.duration > 0.0 {
            self.perform_seek((target_time / self.duration).min(1.0), None, cx);
        }

        if should_play {
            if !self.is_playing {
                self.resume_playback(false, None, cx);
            }
        } else if self.is_playing {
            self.pause_playback_engine(cx);
        }
    }

    fn repeat_mode_wire_value(&self, cx: &App) -> &'static str {
        self.repeat_mode(cx).wire()
    }

    fn publish_listening_session_state(
        &mut self,
        command: MusicListeningSessionCommand,
        debounce_milliseconds: u64,
        committed_position_ms: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        if self.is_listening_playback_displaced_by_ad_hoc()
            || self.is_applying_listening_session
            || self.is_listening_session_exit_in_flight
        {
            return;
        }
        let (Some(session), Some(track)) = (self.listening_session.clone(), self.current_track.clone()) else {
            return;
        };
        if session.host_user_id != Self::current_listening_user_id()
            && let Some(control) = Self::listening_control(command)
            && !session.viewer_permissions.allows(control)
        {
            return;
        }

        let publish = move |this: &mut Self, cx: &mut Context<Self>| {
            let Some(latest) = this.listening_session.clone() else { return };
            if this.is_listening_session_exit_in_flight
                || latest.session_id != session.session_id
                || this.current_track.as_ref().is_none_or(|current| current.id != track.id)
            {
                return;
            }

            let request = MusicPlaylistListeningSessionStateRequest {
                session_id: latest.session_id.clone(),
                command,
                track_id: track.id.clone(),
                position_ms: committed_position_ms
                    .unwrap_or_else(|| (this.current_time.max(0.0) * 1_000.0) as i64),
                is_playing: this.is_playing,
                shuffle_enabled: this.is_shuffled(cx),
                shuffle_seed: this.publishable_shuffle_seed(cx),
                repeat_mode: this.repeat_mode_wire_value(cx).to_owned(),
                audio_effect_enabled: this.audio_effect_enabled(),
                audio_effect_rate_milli: MusicEffectPlaybackLimits::rate_milli(this.audio_effect_rate()),
                audio_effects: this.audio_effect_settings.wire_effects(),
                expected_revision: latest.revision,
            };
            let on_success_target = cx.weak_entity();
            let on_error_target = cx.weak_entity();
            let session_id = session.session_id.clone();
            Self::music_interactions_deferred(cx, move |store, cx| {
                store.update_playlist_listening_session_state(
                    request,
                    Some(Box::new(move |updated: MusicPlaylistListeningSession, cx: &mut App| {
                        on_success_target
                            .update(cx, |this, cx| {
                                if this.is_listening_session_exit_in_flight
                                    || this.current_session_id() != Some(session_id.as_str())
                                    || this.is_stale_listening_session(&updated)
                                {
                                    return;
                                }
                                this.last_applied_listening_generation = updated.generation;
                                this.last_applied_listening_revision = updated.revision;
                                this.set_listening_session(Some(updated), cx);
                                this.last_listening_error = None;
                                cx.notify();
                            })
                            .ok();
                    })),
                    Some(Box::new(move |error: String, cx: &mut App| {
                        if command == MusicListeningSessionCommand::Heartbeat {
                            return;
                        }
                        on_error_target.update(cx, |this, cx| this.report_listening_error(error, cx)).ok();
                    })),
                    cx,
                );
            });
        };

        self.listening_state_publish_task = None;
        if debounce_milliseconds > 0 {
            self.listening_state_publish_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(debounce_milliseconds)).await;
                this.update(cx, |this, cx| publish(this, cx)).ok();
            }));
        } else {
            publish(self, cx);
        }
    }

    /// presence and room polling only run when someone else can listen along
    /// local playlist with other seats or an attached session
    fn listening_presence_playlist_id(&self, cx: &mut App) -> Option<String> {
        let playlist_id = self.playlist_id_from_playback_scope()?;
        let services = MusicServicesStore::shared(cx);
        let playlist = services.update(cx, |store, cx| store.playlist(playlist_id, cx))?;
        if playlist.visibility.as_deref() != Some("local") {
            return None;
        }
        if self.listening_session.is_some() {
            return Some(playlist_id.to_owned());
        }
        let is_solo = services
            .update(cx, |store, cx| store.playlists_mine(cx))
            .iter()
            .find(|playlist| playlist.id.as_deref() == Some(playlist_id))
            .is_some_and(|playlist| playlist.is_solo_playlist());
        if is_solo { None } else { Some(playlist_id.to_owned()) }
    }

    pub fn refresh_playlist_listening_presence_if_audience_changed(&mut self, cx: &mut Context<Self>) {
        if self.listening_presence_playlist_id(cx).is_some() == self.presence_playlist_id.is_some() {
            return;
        }
        self.refresh_playlist_listening_presence(cx);
    }

    pub fn refresh_playlist_listening_presence(&mut self, cx: &mut Context<Self>) {
        let (Some(playlist_id), Some(track_id)) = (
            self.listening_presence_playlist_id(cx),
            self.current_track.as_ref().map(|track| track.id.clone()),
        ) else {
            self.stop_playlist_listening_presence_if_needed(cx);
            return;
        };

        if let Some(previous) = self.presence_playlist_id.clone()
            && previous != playlist_id
        {
            cx.defer(move |cx| {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.stop_playlist_listening_presence(&previous, cx));
            });
        }
        if let Some(previous) = self.listening_observation_playlist_id.clone()
            && previous != playlist_id
        {
            cx.defer(move |cx| {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.stop_playlist_listening_observation(&previous, None, cx));
            });
        }
        self.presence_playlist_id = Some(playlist_id.clone());
        self.listening_observation_playlist_id = Some(playlist_id.clone());

        let position_ms = (self.current_time.max(0.0) * 1_000.0) as i64;
        let is_playing = self.is_playing;
        cx.defer(move |cx| {
            MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.start_playlist_listening_observation(&playlist_id, None, cx);
                store.update_playlist_listening_presence(&playlist_id, &track_id, position_ms, is_playing, cx);
            });
        });
    }

    fn stop_playlist_listening_presence_if_needed(&mut self, cx: &mut Context<Self>) {
        if let Some(presence_playlist_id) = self.presence_playlist_id.take() {
            cx.defer(move |cx| {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.stop_playlist_listening_presence(&presence_playlist_id, cx));
            });
        }
        if let Some(observation_playlist_id) = self.listening_observation_playlist_id.take() {
            cx.defer(move |cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.stop_playlist_listening_observation(&observation_playlist_id, None, cx)
                });
            });
        }
    }

    fn listening_control(command: MusicListeningSessionCommand) -> Option<MusicListeningBlockedControl> {
        match command {
            MusicListeningSessionCommand::Heartbeat => None,
            MusicListeningSessionCommand::Playback => Some(MusicListeningBlockedControl::Playback),
            MusicListeningSessionCommand::Seek => Some(MusicListeningBlockedControl::Seek),
            MusicListeningSessionCommand::ChangeTrack => Some(MusicListeningBlockedControl::ChangeTracks),
            MusicListeningSessionCommand::Effects => Some(MusicListeningBlockedControl::ApplyEffects),
        }
    }

    fn allows_listening_control(
        &mut self,
        control: MusicListeningBlockedControl,
        ignoring_ad_hoc_displacement: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if !ignoring_ad_hoc_displacement && self.is_listening_playback_displaced_by_ad_hoc() {
            return true;
        }
        if self.is_applying_listening_session {
            return true;
        }
        let Some(session) = &self.listening_session else { return true };
        if session.host_user_id == Self::current_listening_user_id() {
            return true;
        }

        let allowed = session.viewer_permissions.allows(control);
        if !allowed {
            let is_player_on_screen = self.show_now_playing
                || self.show_queue_sheet
                || self.show_audio_effects_sheet
                || aside_navigator::player_beside(cx);
            if is_player_on_screen {
                self.blocked_listening_control = Some(control);
                cx.notify();
            } else {
                let weak = cx.weak_entity();
                Notifier::send_undo(
                    tr("music_live_control_locked_title"),
                    tr("music_live_request_permissions"),
                    NotifierUndoOptions {
                        position: NotifierPosition::Bottom,
                        icon: NotifierIcon::System("lock.fill".to_owned()),
                        ..NotifierUndoOptions::default()
                    },
                    |_| {},
                    move |cx| {
                        weak.update(cx, |this, cx| this.request_listening_permission(control, cx)).ok();
                    },
                    cx,
                );
            }
        }
        allowed
    }

    pub fn request_blocked_listening_permission(&mut self, cx: &mut Context<Self>) {
        let Some(blocked_listening_control) = self.blocked_listening_control else { return };
        self.request_listening_permission(blocked_listening_control, cx);
    }

    pub fn request_listening_permission(
        &mut self,
        blocked_listening_control: MusicListeningBlockedControl,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = &self.listening_session else { return };

        let current = session.viewer_permissions;
        let requested = MusicListeningParticipantPermissions {
            can_control_playback: current.can_control_playback
                || blocked_listening_control == MusicListeningBlockedControl::Playback,
            can_seek: current.can_seek || blocked_listening_control == MusicListeningBlockedControl::Seek,
            can_change_tracks: current.can_change_tracks
                || blocked_listening_control == MusicListeningBlockedControl::ChangeTracks,
            can_apply_effects: current.can_apply_effects
                || blocked_listening_control == MusicListeningBlockedControl::ApplyEffects,
        };
        let session_id = session.session_id.clone();
        let on_success_target = cx.weak_entity();
        let on_error_target = cx.weak_entity();
        Self::music_interactions_deferred(cx, move |store, cx| {
            store.request_playlist_listening_permissions(
                &session_id,
                requested,
                Some(Box::new(move |cx: &mut App| {
                    on_success_target
                        .update(cx, |this, cx| {
                            this.blocked_listening_control = None;
                            cx.notify();
                        })
                        .ok();
                    Notifier::send(
                        tr("music_live_permission_request_sent"),
                        NotifierSendOptions {
                            r#type: NotifierType::Success,
                            position: NotifierPosition::Bottom,
                            icon: NotifierIcon::System("paperplane.fill".to_owned()),
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                })),
                Some(Box::new(move |error: String, cx: &mut App| {
                    on_error_target.update(cx, |this, cx| this.report_listening_error(error, cx)).ok();
                })),
                cx,
            );
        });
    }

    /// true when a transition started or joined a running one, callbacks answer later
    /// early refusal goes to on_error and returns false
    pub fn request_join_playlist_listening_session(
        &mut self,
        playlist_id: &str,
        target_user_id: &str,
        target_session_id: Option<&str>,
        on_success: Option<Box<dyn FnOnce(MusicPlaylistListeningSession, &mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        on_cancelled: Option<Box<dyn FnOnce(&mut App)>>,
        cx: &mut Context<Self>,
    ) -> bool {
        let current_user_id = Self::current_listening_user_id();
        if playlist_id.is_empty() || target_user_id.is_empty() || current_user_id.is_empty() {
            Self::defer_listening_error(on_error, tr("music_live_action_failed"), cx);
            return false;
        }
        if self.is_listening_session_exit_in_flight {
            Self::defer_listening_error(on_error, tr("music_live_transition_in_progress"), cx);
            return false;
        }
        if !MusicEffectThermalGuard::shared(cx).read(cx).allows_listening_session_join() {
            Self::defer_listening_error(on_error, tr("music_effects_overheat_cooldown"), cx);
            return false;
        }

        let transition = MusicListeningSessionTransition {
            playlist_id: playlist_id.to_owned(),
            target_user_id: target_user_id.to_owned(),
            target_session_id: target_session_id.map(str::to_owned),
            current_session_id: self.listening_session.as_ref().map(|session| session.session_id.clone()),
            participant_count: self.listening_session.as_ref().map_or(0, |session| session.participants.len() as i64),
        };

        if self.is_listening_session_transitioning {
            let joins_running = self
                .active_listening_session_transition
                .as_ref()
                .is_some_and(|active| active.targets(&transition));
            if !joins_running {
                Self::defer_listening_error(on_error, tr("music_live_transition_in_progress"), cx);
                return false;
            }
            self.append_listening_session_transition_callbacks(on_success, on_error, on_cancelled);
            return true;
        }

        let decision = MusicListeningSessionTransitionPolicy::decision(
            self.listening_session.as_ref(),
            &current_user_id,
            target_user_id,
            target_session_id,
        );
        if decision == MusicListeningSessionTransitionDecision::AlreadyJoined
            && let Some(listening_session) = self.listening_session.clone()
        {
            if self.is_listening_playback_displaced_by_ad_hoc() {
                self.restore_listening_session_after_ad_hoc_playback(cx);
            }
            self.last_listening_error = None;
            cx.notify();
            if let Some(on_success) = on_success {
                cx.defer(move |cx| on_success(listening_session, cx));
            }
            return false;
        }

        self.active_listening_session_transition = Some(transition.clone());
        self.listening_session_transition_target_user_id = Some(target_user_id.to_owned());
        self.is_listening_session_transitioning = true;
        self.append_listening_session_transition_callbacks(on_success, on_error, on_cancelled);
        cx.notify();

        match decision {
            MusicListeningSessionTransitionDecision::AlreadyJoined => {
                self.finish_listening_session_transition_error("Listening session is unavailable".to_owned(), cx);
                return false;
            }
            MusicListeningSessionTransitionDecision::Join => {
                self.begin_listening_session_transition_join(transition, cx);
            }
            MusicListeningSessionTransitionDecision::EndThenJoin => {
                self.begin_listening_session_transition_close(transition, true, cx);
            }
            MusicListeningSessionTransitionDecision::ConfirmEndThenJoin => {
                self.listening_session_transition_phase = Some(MusicListeningSessionTransitionPhase::AwaitingConfirmation);
                self.pending_listening_session_transition = Some(transition);
                cx.notify();
            }
            MusicListeningSessionTransitionDecision::LeaveThenJoin => {
                self.begin_listening_session_transition_close(transition, false, cx);
            }
        }
        true
    }

    fn defer_listening_error(on_error: Option<Box<dyn FnOnce(String, &mut App)>>, error: String, cx: &mut App) {
        if let Some(on_error) = on_error {
            cx.defer(move |cx| on_error(error, cx));
        }
    }

    pub fn confirm_pending_listening_session_transition(&mut self, cx: &mut Context<Self>) {
        let confirmed = self.pending_listening_session_transition.clone().filter(|transition| {
            self.active_listening_session_transition.as_ref() == Some(transition)
                && self.listening_session_transition_phase
                    == Some(MusicListeningSessionTransitionPhase::AwaitingConfirmation)
        });
        let Some(transition) = confirmed else {
            self.cancel_pending_listening_session_transition(cx);
            return;
        };

        self.pending_listening_session_transition = None;
        cx.notify();
        self.begin_listening_session_transition_close(transition, true, cx);
    }

    pub fn cancel_pending_listening_session_transition(&mut self, cx: &mut Context<Self>) {
        if self.pending_listening_session_transition.is_none() {
            return;
        }

        if self.listening_session_transition_phase != Some(MusicListeningSessionTransitionPhase::AwaitingConfirmation) {
            self.pending_listening_session_transition = None;
            cx.notify();
            return;
        }

        let callbacks = std::mem::take(&mut self.listening_session_transition_cancelled_callbacks);
        self.reset_listening_session_transition(cx);
        cx.defer(move |cx| {
            for callback in callbacks {
                callback(cx);
            }
        });
    }

    fn append_listening_session_transition_callbacks(
        &mut self,
        on_success: Option<Box<dyn FnOnce(MusicPlaylistListeningSession, &mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        on_cancelled: Option<Box<dyn FnOnce(&mut App)>>,
    ) {
        if let Some(on_success) = on_success {
            self.listening_session_transition_success_callbacks.push(on_success);
        }
        if let Some(on_error) = on_error {
            self.listening_session_transition_error_callbacks.push(on_error);
        }
        if let Some(on_cancelled) = on_cancelled {
            self.listening_session_transition_cancelled_callbacks.push(on_cancelled);
        }
    }

    fn begin_listening_session_transition_close(
        &mut self,
        transition: MusicListeningSessionTransition,
        end_owned_session: bool,
        cx: &mut Context<Self>,
    ) {
        if self.active_listening_session_transition.as_ref() != Some(&transition) {
            return;
        }

        self.listening_session_transition_phase = Some(MusicListeningSessionTransitionPhase::ClosingCurrent);
        let Some(current_session_id) = transition.current_session_id.clone() else {
            self.begin_listening_session_transition_join(transition, cx);
            return;
        };

        let success_target = cx.weak_entity();
        let success_transition = transition.clone();
        let on_success: Box<dyn FnOnce(&mut App)> = Box::new(move |cx: &mut App| {
            success_target
                .update(cx, |this, cx| {
                    this.detach_listening_session_for_transition_if_needed(&success_transition, cx);
                    this.begin_listening_session_transition_join(success_transition, cx);
                })
                .ok();
        });
        let error_target = cx.weak_entity();
        let on_error: Box<dyn FnOnce(String, &mut App)> = Box::new(move |error: String, cx: &mut App| {
            error_target
                .update(cx, |this, cx| {
                    if MusicListeningSessionTransitionPolicy::can_continue_after_close_error(&error, None) {
                        this.detach_listening_session_for_transition_if_needed(&transition, cx);
                        this.begin_listening_session_transition_join(transition, cx);
                    } else {
                        this.finish_listening_session_transition_error(error, cx);
                    }
                })
                .ok();
        });

        Self::music_interactions_deferred(cx, move |store, cx| {
            if end_owned_session {
                store.end_playlist_listening_session(&current_session_id, Some(on_success), Some(on_error), cx);
            } else {
                store.leave_playlist_listening_session(&current_session_id, Some(on_success), Some(on_error), cx);
            }
        });
    }

    fn detach_listening_session_for_transition_if_needed(
        &mut self,
        transition: &MusicListeningSessionTransition,
        cx: &mut Context<Self>,
    ) {
        let Some(current_session_id) = transition.current_session_id.as_deref() else {
            return;
        };
        if self.current_session_id() != Some(current_session_id) {
            return;
        }

        self.detach_playlist_listening_session("switched", false, true, cx);
    }

    fn begin_listening_session_transition_join(
        &mut self,
        transition: MusicListeningSessionTransition,
        cx: &mut Context<Self>,
    ) {
        if self.active_listening_session_transition.as_ref() != Some(&transition) {
            return;
        }

        self.listening_session_transition_phase = Some(MusicListeningSessionTransitionPhase::JoiningTarget);
        let success_target = cx.weak_entity();
        let success_transition = transition.clone();
        let on_success: Box<dyn FnOnce(MusicPlaylistListeningSession, &mut App)> =
            Box::new(move |session: MusicPlaylistListeningSession, cx: &mut App| {
                success_target
                    .update(cx, |this, cx| {
                        if this.active_listening_session_transition.as_ref() != Some(&success_transition) {
                            return;
                        }
                        this.attach_playlist_listening_session(session.clone(), RiseClock::now().ms(), None, cx);
                        this.finish_listening_session_transition_success(session, cx);
                    })
                    .ok();
            });
        let error_target = cx.weak_entity();
        let error_transition = transition.clone();
        let on_error: Box<dyn FnOnce(String, &mut App)> = Box::new(move |error: String, cx: &mut App| {
            error_target
                .update(cx, |this, cx| {
                    if this.active_listening_session_transition.as_ref() != Some(&error_transition) {
                        return;
                    }
                    this.finish_listening_session_transition_error(error, cx);
                })
                .ok();
        });

        Self::music_interactions_deferred(cx, move |store, cx| {
            store.join_playlist_listening_session(
                &transition.playlist_id,
                &transition.target_user_id,
                transition.target_session_id.as_deref(),
                Some(on_success),
                Some(on_error),
                cx,
            );
        });
    }

    fn continue_listening_session_transition_after_current_ended(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(transition) = self.active_listening_session_transition.clone() else {
            return;
        };
        if transition.current_session_id.as_deref() != Some(session_id)
            || self.listening_session_transition_phase
                != Some(MusicListeningSessionTransitionPhase::AwaitingConfirmation)
        {
            return;
        }

        self.pending_listening_session_transition = None;
        cx.notify();
        self.begin_listening_session_transition_join(transition, cx);
    }

    fn finish_listening_session_transition_success(
        &mut self,
        session: MusicPlaylistListeningSession,
        cx: &mut Context<Self>,
    ) {
        let callbacks = std::mem::take(&mut self.listening_session_transition_success_callbacks);
        self.reset_listening_session_transition(cx);
        self.last_listening_error = None;
        cx.defer(move |cx| {
            for callback in callbacks {
                callback(session.clone(), cx);
            }
        });
    }

    fn finish_listening_session_transition_error(&mut self, error: String, cx: &mut Context<Self>) {
        let callbacks = std::mem::take(&mut self.listening_session_transition_error_callbacks);
        self.reset_listening_session_transition(cx);
        self.last_listening_error = Some(error.clone());
        cx.defer(move |cx| {
            for callback in callbacks {
                callback(error.clone(), cx);
            }
        });
    }

    fn reset_listening_session_transition(&mut self, cx: &mut Context<Self>) {
        self.active_listening_session_transition = None;
        self.listening_session_transition_phase = None;
        self.pending_listening_session_transition = None;
        self.listening_session_transition_target_user_id = None;
        self.is_listening_session_transitioning = false;
        self.listening_session_transition_success_callbacks.clear();
        self.listening_session_transition_error_callbacks.clear();
        self.listening_session_transition_cancelled_callbacks.clear();
        cx.notify();
    }

    pub fn leave_current_playlist_listening_session(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.listening_session.clone() else {
            return;
        };
        if self.is_listening_session_transitioning || self.is_listening_session_exit_in_flight {
            return;
        }

        self.is_listening_session_exit_in_flight = true;
        self.listening_session_exit_session_id = Some(session.session_id.clone());
        self.listening_state_publish_task = None;
        self.listening_apply_task = None;
        let stopped_session_id = session.session_id.clone();
        Self::music_interactions_deferred(cx, move |store, cx|
            store.stop_playlist_listening_session_sync(&stopped_session_id, cx)
        );

        let success_target = cx.weak_entity();
        let success_session_id = session.session_id.clone();
        let on_success: Box<dyn FnOnce(&mut App)> = Box::new(move |cx: &mut App| {
            success_target
                .update(cx, |this, cx| {
                    if this.listening_session_exit_session_id.as_deref() != Some(success_session_id.as_str()) {
                        return;
                    }
                    this.is_listening_session_exit_in_flight = false;
                    this.listening_session_exit_session_id = None;
                    if this.current_session_id() == Some(success_session_id.as_str()) {
                        let reason = if this.is_listening_session_host() { "host_ended" } else { "left" };
                        this.detach_playlist_listening_session(reason, false, true, cx);
                    }
                })
                .ok();
        });
        let error_target = cx.weak_entity();
        let error_session_id = session.session_id.clone();
        let on_error: Box<dyn FnOnce(String, &mut App)> = Box::new(move |error: String, cx: &mut App| {
            error_target
                .update(cx, |this, cx| {
                    if this.listening_session_exit_session_id.as_deref() != Some(error_session_id.as_str()) {
                        return;
                    }
                    this.is_listening_session_exit_in_flight = false;
                    this.listening_session_exit_session_id = None;
                    let still_attached = this.current_session_id() == Some(error_session_id.as_str());
                    if MusicListeningSessionTransitionPolicy::can_continue_after_close_error(&error, None) {
                        if still_attached {
                            this.detach_playlist_listening_session("closed", false, true, cx);
                        }
                    } else {
                        if still_attached {
                            let restarted_session_id = error_session_id.clone();
                            Self::music_interactions_deferred(cx, move |store, cx| {
                                store.start_playlist_listening_session_sync(&restarted_session_id, cx);
                            });
                        }
                        this.report_listening_error(error, cx);
                    }
                })
                .ok();
        });

        let is_host = self.is_listening_session_host();
        Self::music_interactions_deferred(cx, move |store, cx| {
            if is_host {
                store.end_playlist_listening_session(&session.session_id, Some(on_success), Some(on_error), cx);
            } else {
                store.leave_playlist_listening_session(&session.session_id, Some(on_success), Some(on_error), cx);
            }
        });
    }

    pub fn apply_consented_listening_interruption(
        &mut self,
        decision: MusicListeningInterruptionDecision,
        cx: &mut Context<Self>,
    ) {
        if !self.is_listening_together() {
            return;
        }
        match decision {
            MusicListeningInterruptionDecision::ConfirmPauseAll => {
                self.listening_consent_episode_active = true;
            }
            MusicListeningInterruptionDecision::ConfirmLeave => {
                let current_user_id = Self::current_listening_user_id();
                if let Some(session) = self.listening_session.as_ref()
                    && session.host_user_id != current_user_id
                {
                    self.room_left_for_ad_hoc = Some(MusicRoomLeftForAdHoc {
                        account_user_id: current_user_id,
                        playlist_id: session.playlist_id.clone(),
                        host_user_id: session.host_user_id.clone(),
                    });
                }
                self.leave_listening_session_for_local_interruption("displaced_by_media", cx);
            }
            MusicListeningInterruptionDecision::Proceed => {}
        }
    }

    /// audio vanished with no choice, call or recorder
    /// a participant who can pause pauses the room for all, one who cant is removed instead of drifting
    fn enforce_listening_session_for_audio_loss(&mut self, local_was_playing: bool, cx: &mut Context<Self>) {
        if self.is_listening_session_exit_in_flight {
            return;
        }
        let Some(session) = self.listening_session.as_ref() else {
            return;
        };
        let action = MusicListeningInterruptionPolicy::auto_action(
            true,
            self.is_listening_session_host(),
            session.viewer_permissions.can_control_playback,
            session.is_playing,
            local_was_playing,
            self.session_paused_by_local_interruption,
        );
        match action {
            MusicListeningAutoInterruptionAction::None => {}
            MusicListeningAutoInterruptionAction::PauseAll => {
                self.pause_listening_session_for_local_interruption(cx);
            }
            MusicListeningAutoInterruptionAction::Leave => {
                self.leave_listening_session_for_local_interruption("audio_interrupted", cx);
            }
        }
    }

    fn pause_listening_session_for_local_interruption(&mut self, cx: &mut Context<Self>) {
        if self.listening_session.is_none()
            || self.current_track.is_none()
            || self.is_listening_playback_displaced_by_ad_hoc()
        {
            return;
        }
        self.session_paused_by_local_interruption = true;
        self.publish_listening_session_state(MusicListeningSessionCommand::Playback, 0, None, cx);
    }

    /// detach locally first so nothing hits the permission gate or publishes into the room
    /// then tell the server, host never takes this path
    fn leave_listening_session_for_local_interruption(&mut self, reason: &str, cx: &mut Context<Self>) {
        let Some(session_id) = self.current_session_id().map(str::to_owned) else {
            return;
        };
        MusicPlayerLog::info(format!("LIVE leaving session reason={reason}"), cx);

        self.detach_playlist_listening_session(reason, false, true, cx);
        let left_session_id = session_id.clone();
        Self::music_interactions_deferred(cx, move |store, cx| {
            store.leave_playlist_listening_session(&left_session_id, None, None, cx);
        });
        Notifier::send(
            tr("music_live_interrupt_left_toast"),
            NotifierSendOptions {
                r#type: NotifierType::Info,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("person.2.slash".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn enforce_listening_session_for_ad_hoc_takeover(&mut self, cx: &mut Context<Self>) {
        let Some(session_is_playing) = self.listening_session.as_ref().map(|session| session.is_playing) else {
            return;
        };
        if self.is_listening_playback_displaced_by_ad_hoc() || !(session_is_playing || self.is_playing) {
            return;
        }
        if self.is_playing {
            self.pause_playback_engine(cx);
        }
        self.session_paused_by_local_interruption = true;
        self.publish_listening_session_state(MusicListeningSessionCommand::Playback, 0, None, cx);
    }

    /// ad-hoc queue ended with a session still attached, give the player back to the room as it is now
    /// room is not auto resumed, play is an explicit press
    fn restore_listening_session_after_ad_hoc_playback(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.listening_session.clone() else {
            return;
        };
        self.clear_playback_engine_and_context(cx);
        self.listening_consent_episode_active = false;
        self.session_paused_by_local_interruption = false;
        let server_time_ms = self.listening_session_server_time_ms;
        self.apply_listening_session_state(session, server_time_ms, None, cx);
    }

    /// true while rendering an effect chain someone else chose
    /// only then thermal eviction makes sense, own effects heat is theirs to turn off
    pub fn is_carrying_foreign_audio_effects(&self) -> bool {
        if self.listening_session.is_none() || !self.audio_effect_settings.is_any_enabled() {
            return false;
        }
        let Some(actor) = self.audio_effect_actor_user_id.as_deref().filter(|actor| !actor.is_empty()) else {
            return false;
        };

        actor != Self::current_listening_user_id()
    }

    fn refresh_audio_effect_thermal_monitoring(&mut self, cx: &mut Context<Self>) {
        let carries_foreign_effects = self.is_carrying_foreign_audio_effects();
        MusicEffectThermalGuard::shared(cx).update(cx, |guard, cx| {
            guard.refresh_monitoring(carries_foreign_effects, cx);
        });
    }

    pub fn evict_from_listening_session_for_overheating(
        &mut self,
        report_sent: Option<oneshot::Receiver<()>>,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.current_session_id().map(str::to_owned) else {
            return;
        };
        let was_host = self.is_listening_session_host();

        self.listening_session_exit_session_id = Some(session_id.clone());
        self.detach_playlist_listening_session("overheated", false, true, cx);
        self.pause(cx);
        self.cancel_pending_audio_effect_commits();

        let mut cleared = self.audio_effect_settings.clone();
        for kind in MusicAudioEffectKind::all_cases() {
            cleared.set_enabled(*kind, false);
        }
        self.set_audio_effect_settings(cleared.clone(), cx);
        self.audio_effect_actor_user_id = None;
        MusicAudioEffectStore::shared(cx).update(cx, |store, cx| store.replace(cleared, cx));
        self.apply_audio_effect_to_engine(false, cx);
        self.refresh_audio_effect_thermal_monitoring(cx);

        let Some(report_sent) = report_sent else {
            Self::leave_overheated_session_on_server(session_id, was_host, cx);
            return;
        };
        cx.spawn(async move |this, cx| {
            Self::await_overheat_report(report_sent, Duration::from_secs(5), cx).await;
            this.update(cx, |_, cx| Self::leave_overheated_session_on_server(session_id, was_host, cx))
                .ok();
        })
        .detach();
    }

    async fn await_overheat_report(report: oneshot::Receiver<()>, at_most: Duration, cx: &mut AsyncApp) {
        let timer = cx.background_executor().timer(at_most);
        futures::pin_mut!(timer);
        let _ = futures::future::select(report, timer).await;
    }

    fn leave_overheated_session_on_server(session_id: String, was_host: bool, cx: &mut Context<Self>) {
        let settle = |target: WeakEntity<Self>, session_id: String| {
            move |cx: &mut App| {
                target
                    .update(cx, |state, _| {
                        if state.listening_session_exit_session_id.as_deref() == Some(session_id.as_str()) {
                            state.listening_session_exit_session_id = None;
                        }
                    })
                    .ok();
            }
        };
        let on_success_settle = settle(cx.weak_entity(), session_id.clone());
        let on_error_settle = settle(cx.weak_entity(), session_id.clone());
        let on_success: Box<dyn FnOnce(&mut App)> = Box::new(on_success_settle);
        let on_error: Box<dyn FnOnce(String, &mut App)> = Box::new(move |_: String, cx: &mut App| on_error_settle(cx));
        Self::music_interactions_deferred(cx, move |store, cx| {
            if was_host {
                store.end_playlist_listening_session(&session_id, Some(on_success), Some(on_error), cx);
            } else {
                store.leave_playlist_listening_session(&session_id, Some(on_success), Some(on_error), cx);
            }
        });
    }

    fn music_interactions_deferred<R: 'static>(
        cx: &mut App,
        f: impl FnOnce(&mut MusicInteractionsStore, &mut Context<MusicInteractionsStore>) -> R + 'static,
    ) {
        cx.defer(move |cx| {
            MusicInteractionsStore::shared(cx).update(cx, f);
        });
    }

    pub fn detach_listening_session_locally(&mut self, cx: &mut Context<Self>) {
        self.detach_playlist_listening_session("guide_sandbox", false, false, cx);
    }

    fn detach_playlist_listening_session(
        &mut self,
        reason: &str,
        notify: bool,
        refresh_presence: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.current_session_id().map(str::to_owned) else {
            return;
        };
        MusicPlayerLog::info(format!("LIVE detach session reason={reason}"), cx);

        let detached_session_id = session_id.clone();
        Self::music_interactions_deferred(cx, move |store, cx| {
            store.stop_playlist_listening_session_sync(&detached_session_id, cx);
        });
        self.listening_state_publish_task = None;
        self.listening_apply_task = None;
        self.set_listening_session(None, cx);
        self.listening_repeat_mode = None;
        self.listening_shuffle_enabled = None;
        self.listening_shuffle_seed = 0;
        self.listening_session_server_time_ms = 0;
        self.blocked_listening_control = None;
        self.listening_permission_request_notice = None;
        self.last_applied_listening_generation = -1;
        self.last_applied_listening_revision = -1;
        self.is_applying_listening_session = false;
        self.suppress_next_listening_playback_publish.set(false);
        self.listening_consent_episode_active = false;
        self.session_paused_by_local_interruption = false;
        self.audio_effect_actor_user_id = None;
        cx.notify();
        self.cancel_pending_audio_effect_commits();
        self.refresh_audio_effect_thermal_monitoring(cx);
        if self.is_music_playback()
            && let Some(track) = self.current_track.clone()
        {
            self.rebuild_playback_queue(&track, cx);
        }
        if refresh_presence {
            self.refresh_playlist_listening_presence(cx);
        }

        if notify {
            Notifier::send(
                tr("music_live_session_closed"),
                NotifierSendOptions {
                    r#type: NotifierType::Info,
                    position: NotifierPosition::Top,
                    icon: NotifierIcon::System("person.2.slash".to_owned()),
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        }
    }

    fn start_media_service(&mut self, cx: &mut Context<Self>) {
        let Some(media_keys) = &self.media_keys else { return };
        let service = media_keys.service();
        if transport_needs_window(HostOs::current()) {
            self.media_service_task = Some(cx.spawn(async move |_, cx| {
                if let Err(error) = service.run().await {
                    cx.update(|cx| {
                        MusicPlayerLog::fail(format!("INIT media transport stopped: {error}"), cx);
                    });
                }
            }));
            return;
        }
        let job = RiseAccountRuntime::current().spawn(service.run());
        let abort_on_drop = MusicAudioDownloadAbortOnDrop(job.abort_handle());
        self.media_service_cancelled = false;
        self.media_service_task = Some(cx.spawn(async move |this, cx| {
            let _abort_on_drop = abort_on_drop;
            match job.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    cx.update(|cx| {
                        MusicPlayerLog::fail(format!("INIT media transport stopped: {error}"), cx);
                    });
                }
                Err(error) if error.is_cancelled() => {
                    let _ = this.update(cx, |this, _| this.media_service_cancelled = true);
                }
                Err(error) => {
                    cx.update(|cx| {
                        MusicPlayerLog::fail(format!("INIT media transport stopped: {error}"), cx);
                    });
                }
            }
        }));
    }

    fn bind_media_window(&mut self, cx: &mut Context<Self>) {
        let windows = cx.windows();
        if self.media_window.is_some_and(|id| windows.iter().any(|handle| handle.window_id() == id)) {
            return;
        }
        let Some(handle) = cx.active_window().or_else(|| windows.first().copied()) else { return };
        let Some(media_keys) = &self.media_keys else { return };
        if handle.update(cx, |_, window, _| media_keys.bind_window(window)).is_ok() {
            self.media_window = Some(handle.window_id());
        }
    }

    fn install_playback_observers_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.playback_observers_installed {
            return;
        }
        self.playback_observers_installed = true;

        let (sender, mut inbox) = mpsc::unbounded::<MusicPlaybackObserverSignal>();
        let center = RiseAppEvents::default_center();

        let route_sender = sender.clone();
        self.route_change_observer = Some(center.observe(AVAudioSession::ROUTE_CHANGE_NOTIFICATION, move |note| {
            if let Some(change) = note.payload::<AVAudioSessionRouteChange>() {
                let _ = route_sender.unbounded_send(MusicPlaybackObserverSignal::RouteChange(*change));
            }
        }));

        let remote_sender = sender.clone();
        let handler: CommandHandler = Arc::new(move |event: MediaCommandEvent| {
            let command = match event.command {
                MediaCommand::Play => MusicPlayerStateRemoteTransportCommand::Play,
                MediaCommand::Pause => MusicPlayerStateRemoteTransportCommand::Pause,
                MediaCommand::Toggle => MusicPlayerStateRemoteTransportCommand::TogglePlayPause,
                MediaCommand::Next => MusicPlayerStateRemoteTransportCommand::Next,
                MediaCommand::Previous => MusicPlayerStateRemoteTransportCommand::Previous,
                MediaCommand::Seek => {
                    let Some(position) = event.position else {
                        return CommandOutcome::Rejected;
                    };
                    MusicPlayerStateRemoteTransportCommand::ChangePosition(position.as_secs_f64())
                }
            };
            match remote_sender.unbounded_send(MusicPlaybackObserverSignal::RemoteCommand(command)) {
                Ok(()) => CommandOutcome::Handled,
                Err(_) => CommandOutcome::Rejected,
            }
        });
        if let Some(info) = AppInfo::current(AppEnvironment::compiled().as_str()) {
            let identity = MediaIdentity::new(info.display_name, info.bundle_identifier);
            match MediaKeys::install(&identity, handler) {
                Ok(media_keys) => {
                    self.media_keys = Some(media_keys);
                    self.start_media_service(cx);
                }
                Err(error) => {
                    MusicPlayerLog::fail(format!("INIT media transport unavailable: {error}"), cx);
                }
            }
        }

        let interruption_sender = sender.clone();
        self.interruption_observer = Some(center.observe(AVAudioSession::INTERRUPTION_NOTIFICATION, move |note| {
            if let Some(interruption) = note.payload::<AVAudioSessionInterruption>() {
                let _ = interruption_sender.unbounded_send(MusicPlaybackObserverSignal::Interruption(*interruption));
            }
        }));

        let observe_lifecycle = |name: RiseNotificationName, signal: MusicPlaybackObserverSignal| {
            let sender = sender.clone();
            center.observe(name, move |_| {
                let _ = sender.unbounded_send(signal);
            })
        };
        self.app_lifecycle_observers.push(observe_lifecycle(
            RiseCallAppVolume::DID_CHANGE_NOTIFICATION,
            MusicPlaybackObserverSignal::CallAppVolumeDidChange,
        ));
        self.app_lifecycle_observers.push(observe_lifecycle(
            UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION,
            MusicPlaybackObserverSignal::WillResignActive,
        ));
        self.app_lifecycle_observers.push(observe_lifecycle(
            UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION,
            MusicPlaybackObserverSignal::DidEnterBackground,
        ));
        self.app_lifecycle_observers.push(observe_lifecycle(
            UIApplication::DID_BECOME_ACTIVE_NOTIFICATION,
            MusicPlaybackObserverSignal::DidBecomeActive,
        ));

        self.playback_signal_task = Some(cx.spawn(async move |this, cx| {
            while let Some(signal) = inbox.next().await {
                let updated = this.update(cx, |this, cx| match signal {
                    MusicPlaybackObserverSignal::RouteChange(change) => this.handle_audio_route_change(change, cx),
                    MusicPlaybackObserverSignal::Interruption(interruption) => {
                        this.handle_audio_session_interruption(interruption, cx);
                    }
                    MusicPlaybackObserverSignal::WillResignActive => this.handle_app_will_resign_active(cx),
                    MusicPlaybackObserverSignal::DidEnterBackground => this.handle_app_did_enter_background(cx),
                    MusicPlaybackObserverSignal::DidBecomeActive => this.handle_app_did_become_active(cx),
                    MusicPlaybackObserverSignal::CallAppVolumeDidChange => this.apply_call_app_volume(cx),
                    MusicPlaybackObserverSignal::RemoteCommand(command) => {
                        this.handle_remote_command(command, cx);
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        }));
    }

    /// raw application state number as the diagnostics print it
    fn application_state_raw_value(cx: &App) -> i64 {
        match RiseAppLifecycle::scene_phase(cx) {
            ScenePhase::Active => 0,
            ScenePhase::Inactive => 1,
            ScenePhase::Background => 2,
        }
    }

    fn is_application_active(cx: &App) -> bool {
        RiseAppLifecycle::scene_phase(cx) == ScenePhase::Active
    }

    fn handle_audio_session_interruption(&mut self, interruption: AVAudioSessionInterruption, cx: &mut Context<Self>) {
        let app_state = Self::application_state_raw_value(cx);
        match interruption.kind {
            AVAudioSessionInterruptionType::Began => {
                if self.is_audio_session_interrupted {
                    return;
                }
                let should_resume = self.is_playing
                    || self.is_preparing_playback
                    || self.resume_playback_when_foreground
                    || (self.external_duck_active && self.external_resume_intent);
                self.is_audio_session_interrupted = true;
                self.audio_session_activated = false;
                self.pause_playback_engine(cx);
                self.was_playing_before_interruption = should_resume;
                MusicPlayerLog::info(
                    format!(
                        "SESSION interruption began suspended=false state={app_state} wasPlaying={}",
                        self.was_playing_before_interruption
                    ),
                    cx,
                );
                self.enforce_listening_session_for_audio_loss(self.was_playing_before_interruption, cx);
            }
            AVAudioSessionInterruptionType::Ended => {
                if !self.is_audio_session_interrupted {
                    return;
                }
                self.is_audio_session_interrupted = false;
                let should_resume = interruption.options.contains(AVAudioSessionInterruptionOptions::SHOULD_RESUME);
                MusicPlayerLog::info(
                    format!(
                        "SESSION interruption ended suspended=false shouldResume={should_resume} wasPlaying={} state={app_state}",
                        self.was_playing_before_interruption
                    ),
                    cx,
                );
                let resume_interrupted_playback = should_resume
                    && self.was_playing_before_interruption
                    && !self.paused_for_call
                    && !RiseCallAudioOwnership::is_call_active();
                self.was_playing_before_interruption = false;
                if resume_interrupted_playback {
                    if self.external_duck_active {
                        self.external_resume_intent = true;
                    } else {
                        self.resume_playback_after_interruption(cx);
                    }
                }
            }
        }
    }

    fn handle_app_will_resign_active(&mut self, cx: &mut Context<Self>) {
        if self.current_track.is_none() || !self.is_playing {
            return;
        }
        MusicPlayerLog::info("APP willResignActive \u{2014} lock session before suspend".to_owned(), cx);
        self.prepare_audio_session_for_background(cx);
    }

    fn handle_app_did_enter_background(&mut self, cx: &mut Context<Self>) {
        if self.current_track.is_none() || !self.is_playing {
            return;
        }
        MusicPlayerLog::info("APP background \u{2014} maintain player only (no session churn)".to_owned(), cx);
        self.begin_background_task_if_needed();
        self.ensure_player_engine_running(cx);
        self.refresh_now_playing_playback_state(cx);
        self.schedule_background_playback_watchdog(cx);
    }

    fn handle_app_did_become_active(&mut self, cx: &mut Context<Self>) {
        self.background_keep_alive_task = None;
        self.end_background_task();
        if self.current_track.is_none() || self.is_audio_session_interrupted {
            return;
        }
        let should_resume_deferred = self.resume_playback_when_foreground;
        self.resume_playback_when_foreground = false;
        if should_resume_deferred && !self.is_playing {
            if self.external_duck_active {
                self.resume_playback_when_foreground = true;
                return;
            }
            MusicPlayerLog::info(
                "APP active \u{2014} resuming playback deferred from background interruption".to_owned(),
                cx,
            );
            self.resume_playback_after_interruption(cx);
            return;
        }
        if self.is_playing {
            MusicPlayerLog::info("APP active \u{2014} re-activate audio session".to_owned(), cx);
            self.perform_after_session_activation(
                true,
                None,
                |this, cx| {
                    if !this.is_playing {
                        return;
                    }
                    this.ensure_player_engine_running(cx);
                    this.refresh_now_playing_playback_state(cx);
                },
                cx,
            );
        }
        self.resync_listening_session_after_stall(cx);
    }

    fn prepare_audio_session_for_background(&mut self, cx: &mut Context<Self>) {
        self.begin_background_task_if_needed();
        self.perform_after_session_activation(
            false,
            None,
            |this, cx| {
                if this.has_local_engine() {
                    if this.local_engine_is_playing(cx) {
                        MusicPlayerLog::info(
                            "APP prepareBackground \u{2192} already playing, skip resume()".to_owned(),
                            cx,
                        );
                    } else {
                        this.resume_local_engine(cx);
                        MusicPlayerLog::info(
                            format!("APP prepareBackground \u{2192} {}.resume() (was paused)", this.active_engine_label()),
                            cx,
                        );
                    }
                } else {
                    this.ensure_av_player_running(cx);
                }
                this.log_playback_diagnostics("prepareBackground", cx);
                this.refresh_now_playing_playback_state(cx);
            },
            cx,
        );
    }

    fn ensure_av_player_running(&mut self, cx: &mut Context<Self>) {
        if self.is_audio_session_interrupted {
            return;
        }
        let Some(player) = self.player.clone() else {
            return;
        };
        let rate = self.remote_playback_rate();
        player.update(cx, |player, cx| {
            player.set_automatically_waits_to_minimize_stalling(false, cx);
            if player.rate() == 0.0 {
                player.play_immediately(rate, cx);
            }
        });
    }

    fn ensure_player_engine_running(&mut self, cx: &mut Context<Self>) {
        if self.is_audio_session_interrupted {
            return;
        }
        if self.has_local_engine() {
            if !self.local_engine_is_playing(cx) {
                self.resume_local_engine(cx);
            }
            return;
        }
        self.ensure_av_player_running(cx);
    }

    fn begin_background_task_if_needed(&mut self) {
        if self.background_task.is_none() {
            self.background_task = Some(begin_process_activity("MusicPlayback"));
        }
    }

    fn end_background_task(&mut self) {
        self.background_task = None;
    }

    fn schedule_background_playback_watchdog(&mut self, cx: &mut Context<Self>) {
        self.background_keep_alive_task = None;
        self.last_watchdog_playback_time = self
            .local_engine_current_time(cx)
            .or_else(|| self.player.as_ref().and_then(|player| player.read(cx).current_time()))
            .unwrap_or(-1.0);
        self.background_keep_alive_task = Some(cx.spawn(async move |this, cx| {
            let mut last_recovery_attempt: i64 = -4;
            for attempt in 1..=60_i64 {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let Ok(step) = this.update(cx, |this, cx| {
                    this.background_watchdog_check(attempt, &mut last_recovery_attempt, cx)
                }) else {
                    return;
                };
                let app_is_active = match step {
                    MusicBackgroundWatchdogStep::Stop => return,
                    MusicBackgroundWatchdogStep::Skip => continue,
                    MusicBackgroundWatchdogStep::Proceed { app_is_active } => app_is_active,
                    MusicBackgroundWatchdogStep::Recover { app_is_active } => {
                        let Ok(session) = this.update(cx, |this, cx| {
                            this.audio_session_activated = false;
                            this.configure_session(true, cx)
                        }) else {
                            return;
                        };
                        let session_ok = session.await;
                        let Ok(recovered) = this.update(cx, |this, cx| {
                            this.background_watchdog_recover(attempt, session_ok, cx)
                        }) else {
                            return;
                        };
                        if !recovered {
                            continue;
                        }
                        app_is_active
                    }
                };
                if app_is_active {
                    let _ = this.update(cx, |this, _| this.end_background_task());
                    return;
                }
            }
        }));
    }

    fn background_watchdog_check(
        &mut self,
        attempt: i64,
        last_recovery_attempt: &mut i64,
        cx: &mut Context<Self>,
    ) -> MusicBackgroundWatchdogStep {
        if self.is_audio_session_interrupted || !self.is_playing || self.current_track.is_none() {
            return MusicBackgroundWatchdogStep::Stop;
        }
        let app_state = Self::application_state_raw_value(cx);
        let app_is_active = Self::is_application_active(cx);
        if self.has_local_engine() {
            let engine_is_playing = self.local_engine_is_playing(cx);
            let engine_duration = self.local_engine_duration(cx).unwrap_or(0.0);
            let t = self.local_engine_current_time(cx).unwrap_or(0.0);
            let prev = self.last_watchdog_playback_time;
            let stall_threshold = 0.12 * self.active_playback_rate(cx);
            let stalled = prev >= 0.0 && t < prev + stall_threshold;
            self.last_watchdog_playback_time = t;
            if self.is_handling_track_end {
                return MusicBackgroundWatchdogStep::Skip;
            }
            if engine_duration > 1.0 && t >= engine_duration - 1.5 {
                MusicPlayerLog::info(
                    format!(
                        "APP watchdog[{attempt}] skip \u{2014} natural track end t={t:.2} dur={engine_duration:.2}"
                    ),
                    cx,
                );
                return MusicBackgroundWatchdogStep::Skip;
            }
            if !engine_is_playing || stalled {
                MusicPlayerLog::fail(
                    format!(
                        "APP watchdog[{attempt}] STALL engine={} isPlaying={engine_is_playing} rate={} stalled={stalled} t={t:.2} prev={prev:.2}",
                        self.active_engine_label(),
                        self.local_engine_rate(cx)
                    ),
                    cx,
                );
                if attempt - *last_recovery_attempt < 2 {
                    MusicPlayerLog::info(
                        format!(
                            "APP watchdog[{attempt}] recovery cooldown (gap={})",
                            attempt - *last_recovery_attempt
                        ),
                        cx,
                    );
                    return MusicBackgroundWatchdogStep::Skip;
                }
                *last_recovery_attempt = attempt;
                return MusicBackgroundWatchdogStep::Recover { app_is_active };
            }
            MusicPlayerLog::info(
                format!("APP watchdog[{attempt}] ok isPlaying={engine_is_playing} t={t:.2} state={app_state}"),
                cx,
            );
        } else if let Some(player) = self.player.clone()
            && player.read(cx).rate() == 0.0
        {
            MusicPlayerLog::fail(
                format!("APP watchdog[{attempt}] AVPlayer rate=0 state={app_state} \u{2192} playImmediately"),
                cx,
            );
            let rate = self.remote_playback_rate();
            player.update(cx, |player, cx| player.play_immediately(rate, cx));
        } else {
            MusicPlayerLog::info(
                format!("APP watchdog[{attempt}] ok engine={} state={app_state}", self.active_engine_label()),
                cx,
            );
        }
        MusicBackgroundWatchdogStep::Proceed { app_is_active }
    }

    fn background_watchdog_recover(&mut self, attempt: i64, session_ok: bool, cx: &mut Context<Self>) -> bool {
        if !session_ok || !self.is_playing {
            return false;
        }
        MusicPlayerLog::info(format!("APP watchdog[{attempt}] sessionReset={session_ok}"), cx);
        let recovered = self.recover_local_engine_if_stalled(cx);
        MusicPlayerLog::info(
            format!(
                "APP watchdog[{attempt}] recoverIfStalled={recovered} stallCount={}",
                self.local_engine_stall_recovery_count(cx)
            ),
            cx,
        );
        if !recovered {
            MusicPlayerLog::fail(
                format!(
                    "APP watchdog[{attempt}] recovery exhausted stallCount={}",
                    self.local_engine_stall_recovery_count(cx)
                ),
                cx,
            );
        }
        self.refresh_now_playing_playback_state(cx);
        true
    }

    fn log_playback_diagnostics(&self, prefix: &str, cx: &mut App) {
        let state_label = match RiseAppLifecycle::scene_phase(cx) {
            ScenePhase::Active => "active",
            ScenePhase::Inactive => "inactive",
            ScenePhase::Background => "background",
        };
        MusicPlayerLog::info(
            format!(
                "APP {prefix} state={state_label} engine={} isPlaying={}",
                self.active_engine_label(),
                self.is_playing
            ),
            cx,
        );
        if self.has_local_engine() {
            let message = format!(
                "APP {prefix} {} isPlaying={} rate={} t={:.1}",
                self.active_engine_label(),
                self.local_engine_is_playing(cx),
                self.local_engine_rate(cx),
                self.local_engine_current_time(cx).unwrap_or(0.0)
            );
            MusicPlayerLog::info(message, cx);
        }
        if let Some(player) = &self.player {
            let message = format!("APP {prefix} AVPlayer rate={}", player.read(cx).rate());
            MusicPlayerLog::info(message, cx);
        }
    }

    fn active_engine_label(&self) -> &'static str {
        if self.effect_player.is_some() {
            return "MusicEffectFilePlayer";
        }
        if self.file_player.is_some() {
            return "ROMusicFilePlayer";
        }
        if self.player.is_some() {
            return "AVPlayer";
        }
        "none"
    }

    /// what the media card, media keys and headphones ask for
    /// debug panel uses the same entry
    pub fn handle_remote_command(
        &mut self,
        command: MusicPlayerStateRemoteTransportCommand,
        cx: &mut Context<Self>,
    ) -> CommandOutcome {
        match command {
            MusicPlayerStateRemoteTransportCommand::Play => {
                if self.current_track.is_none() {
                    return CommandOutcome::Rejected;
                }
                if self.is_backend_silent_while_playing(cx) {
                    self.restart_silent_playback_from_remote(cx);
                } else if !self.is_playing {
                    self.resume_playback_from_user(cx);
                }
            }
            MusicPlayerStateRemoteTransportCommand::Pause => {
                if !(self.is_playing || self.was_playing_before_interruption || self.resume_playback_when_foreground) {
                    return CommandOutcome::Rejected;
                }
                self.pause(cx);
            }
            MusicPlayerStateRemoteTransportCommand::TogglePlayPause => {
                if self.is_backend_silent_while_playing(cx) {
                    self.restart_silent_playback_from_remote(cx);
                } else {
                    self.toggle_play_pause(cx);
                }
            }
            MusicPlayerStateRemoteTransportCommand::Next => self.next(cx),
            MusicPlayerStateRemoteTransportCommand::Previous => self.previous(cx),
            MusicPlayerStateRemoteTransportCommand::ChangePosition(seconds) => {
                if self.current_track.is_none() || self.duration <= 0.0 {
                    return CommandOutcome::Rejected;
                }
                self.seek((seconds / self.duration).clamp(0.0, 1.0), cx);
                self.refresh_now_playing_playback_state(cx);
            }
        }
        CommandOutcome::Handled
    }

    /// media card derives play and pause from real output not is_playing
    /// so it can offer play for a backend the system stopped
    fn is_backend_silent_while_playing(&self, cx: &App) -> bool {
        if !self.is_playing
            || self.is_preparing_playback
            || self.is_handling_track_end
            || self.session_continuation_task.is_some()
            || !self.external_audio_owners.is_empty()
            || self.is_audio_session_interrupted
        {
            return false;
        }
        if self.has_local_engine() {
            return !self.local_engine_is_playing(cx);
        }
        self.player.as_ref().map_or(0.0, |player| player.read(cx).rate()) == 0.0
    }

    fn restart_silent_playback_from_remote(&mut self, cx: &mut Context<Self>) {
        MusicPlayerLog::info(
            format!("REMOTE play while {} is silent \u{2192} restart", self.active_engine_label()),
            cx,
        );
        self.audio_session_activated = false;
        self.resume_playback_from_user(cx);
    }

    fn handle_audio_route_change(&mut self, change: AVAudioSessionRouteChange, cx: &mut Context<Self>) {
        let reason = change.reason;
        let app_state = Self::application_state_raw_value(cx);
        let app_is_active = Self::is_application_active(cx);
        let reason_label = match reason {
            AVAudioSessionRouteChangeReason::NewDeviceAvailable => "newDeviceAvailable".to_owned(),
            AVAudioSessionRouteChangeReason::OldDeviceUnavailable => "oldDeviceUnavailable".to_owned(),
            AVAudioSessionRouteChangeReason::CategoryChange => "categoryChange".to_owned(),
            AVAudioSessionRouteChangeReason::Override => "override".to_owned(),
            AVAudioSessionRouteChangeReason::WakeFromSleep => "wakeFromSleep".to_owned(),
            AVAudioSessionRouteChangeReason::NoSuitableRouteForCategory => "noSuitableRoute".to_owned(),
            AVAudioSessionRouteChangeReason::RouteConfigurationChange => "routeConfigChange".to_owned(),
            AVAudioSessionRouteChangeReason::Unknown => format!("unknown({})", change.reason_raw_value()),
        };
        MusicPlayerLog::info(format!("ROUTE {reason_label} state={app_state}"), cx);

        match reason {
            AVAudioSessionRouteChangeReason::OldDeviceUnavailable => {
                if app_is_active || self.is_playing {
                    MusicPlayerLog::info(
                        format!("ROUTE oldDeviceUnavailable state={app_state} \u{2192} pause"),
                        cx,
                    );
                    self.pause_playback_engine(cx);
                    self.refresh_playlist_listening_presence(cx);
                }
            }
            AVAudioSessionRouteChangeReason::NewDeviceAvailable
            | AVAudioSessionRouteChangeReason::RouteConfigurationChange => {
                if !self.is_playing || self.current_track.is_none() {
                    return;
                }
                if !app_is_active {
                    MusicPlayerLog::info(
                        "ROUTE new output in background \u{2014} reconnecting player".to_owned(),
                        cx,
                    );
                    if self.has_local_engine() {
                        self.resume_local_engine(cx);
                    } else {
                        self.ensure_av_player_running(cx);
                    }
                    self.refresh_now_playing_playback_state(cx);
                }
            }
            AVAudioSessionRouteChangeReason::CategoryChange
            | AVAudioSessionRouteChangeReason::Override
            | AVAudioSessionRouteChangeReason::WakeFromSleep
            | AVAudioSessionRouteChangeReason::NoSuitableRouteForCategory
            | AVAudioSessionRouteChangeReason::Unknown => {}
        }
    }

    fn can_complete_audio_preparation(&self, generation: u64) -> bool {
        self.prepare_generation == generation
            && !self.is_audio_session_interrupted
            && !RiseCallAudioOwnership::blocks_music()
            && self.external_audio_owners.is_empty()
            && self.playback_owner_user_id == SaiAccountCacheScope::active_user_id()
            && self.playback_owner_account_epoch == SaiAccountCacheScope::active_epoch()
    }

    /// resolves to whether the session may play now, dropping the task cancels the activation
    fn configure_session(&mut self, force_activate: bool, cx: &mut Context<Self>) -> Task<bool> {
        self.install_playback_observers_if_needed(cx);
        let generation = self.prepare_generation;
        if !self.can_complete_audio_preparation(generation) {
            return Task::ready(false);
        }
        if self.audio_session_activated && !force_activate {
            return Task::ready(true);
        }
        cx.spawn(async move |this, cx| {
            let activated = RisePlaybackAudioSession::activate().await;
            this.update(cx, |this, cx| {
                if !this.can_complete_audio_preparation(generation) {
                    return false;
                }
                this.audio_session_activated = activated;
                if !activated {
                    this.set_last_playback_error(Some("Audio session unavailable".to_owned()), cx);
                }
                activated
            })
            .unwrap_or(false)
        })
    }

    fn cancel_session_continuation(&mut self) {
        self.session_continuation_generation = self.session_continuation_generation.wrapping_add(1);
        self.session_continuation_task = None;
    }

    /// keeps sync transport controls responsive while waiting for hardware
    /// pause, track changes and external owners invalidate the continuation
    fn perform_after_session_activation(
        &mut self,
        force: bool,
        on_skipped: Option<Box<dyn FnOnce(&mut Self, &mut Context<Self>)>>,
        operation: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.cancel_session_continuation();
        let generation = self.prepare_generation;
        let request = self.session_continuation_generation;
        if !self.can_complete_audio_preparation(generation) {
            if let Some(on_skipped) = on_skipped {
                on_skipped(self, cx);
            }
            return;
        }
        if self.audio_session_activated && !force {
            operation(self, cx);
            return;
        }
        let activation = self.configure_session(force, cx);
        self.session_continuation_task = Some(cx.spawn(async move |this, cx| {
            let activated = activation.await;
            this.update(cx, |this, cx| {
                let is_current = this.session_continuation_generation == request;
                if is_current && let Some(task) = this.session_continuation_task.take() {
                    task.detach();
                }
                if !activated || !is_current || !this.can_complete_audio_preparation(generation) {
                    if let Some(on_skipped) = on_skipped {
                        on_skipped(this, cx);
                    }
                    return;
                }
                operation(this, cx);
            })
            .ok();
        }));
    }

    fn resume_playback_after_interruption(&mut self, cx: &mut Context<Self>) {
        if self.is_audio_session_interrupted || self.current_track.is_none() {
            return;
        }
        if !self.external_audio_owners.is_empty() {
            MusicPlayerLog::info(
                format!(
                    "INTERRUPTION resume deferred \u{2014} external owners={}",
                    self.external_audio_owners.len()
                ),
                cx,
            );
            self.refresh_now_playing_playback_state(cx);
            return;
        }
        self.audio_session_activated = false;
        self.resume_playback_when_foreground = true;
        self.local_engine_restore_full_volume(cx);
        if let Some(player) = self.player.clone() {
            player.update(cx, |player, cx| player.set_volume(MusicOutputVolume::gain(), cx));
        }
        self.resume_playback(
            false,
            Some(Box::new(|this: &mut Self, cx: &mut Context<Self>| {
                this.refresh_now_playing_playback_state(cx);
                this.resync_listening_session_after_stall(cx);
            })),
            cx,
        );
    }

    /// checks drift after a stall, host owns the clock and republishes
    /// except when this device paused and someone else resumed, room wins
    fn resync_listening_session_after_stall(&mut self, cx: &mut Context<Self>) {
        if self.is_audio_session_interrupted || !self.external_audio_owners.is_empty() {
            return;
        }
        let Some(session) = self.listening_session.clone() else {
            return;
        };
        if self.is_listening_playback_displaced_by_ad_hoc()
            || self.current_track.as_ref().is_none_or(|track| track.id != session.track_id)
            || self.is_preparing_playback
        {
            return;
        }

        let action = MusicListeningInterruptionPolicy::resync_action(
            self.is_listening_session_host(),
            self.session_paused_by_local_interruption,
            session.is_playing,
        );
        match action {
            MusicListeningResyncAction::Republish => {
                if self.is_playing {
                    self.session_paused_by_local_interruption = false;
                }
                self.publish_listening_session_state(MusicListeningSessionCommand::Playback, 0, None, cx);
            }
            MusicListeningResyncAction::Align => {
                self.session_paused_by_local_interruption = false;
                let services = MusicServicesStore::shared(cx);
                let projected_ms = services.read(cx).projected_position_ms_for_session(&session, RiseClock::now().ms());
                self.is_applying_listening_session = true;
                self.align_listening_engine(projected_ms, session.is_playing, cx);
                self.is_applying_listening_session = false;
                self.refresh_playlist_listening_presence(cx);
            }
        }
    }

    fn resume_playback(
        &mut self,
        publish_playback_state: bool,
        completion: Option<Box<dyn FnOnce(&mut Self, &mut Context<Self>)>>,
        cx: &mut Context<Self>,
    ) {
        if self.is_audio_session_interrupted {
            return;
        }
        let Some(track) = self.current_track.clone() else {
            return;
        };
        if !self.external_audio_owners.is_empty() {
            MusicPlayerLog::info(
                format!(
                    "PLAY resume deferred \u{2014} external owners={}",
                    self.external_audio_owners.len()
                ),
                cx,
            );
            return;
        }
        if self.is_preparing_playback {
            return;
        }
        let needs_engine_reload = !self.has_local_engine()
            && (self.last_playback_error.is_some()
                || self.player.as_ref().is_none_or(|player| {
                    let player = player.read(cx);
                    player.current_item().is_none() || player.current_item_status() == AVPlayerItemStatus::Failed
                }));
        if needs_engine_reload {
            self.play(
                track,
                MusicPlayOptions {
                    preserve_queue: true,
                    confirm_call_interruption: false,
                    ..MusicPlayOptions::default()
                },
                cx,
            );
            return;
        }
        self.perform_after_session_activation(
            false,
            None,
            move |this, cx| {
                if this.has_local_engine() {
                    this.resume_local_engine(cx);
                    if this.progress_timer.is_none() {
                        this.install_progress_timer(cx);
                    }
                } else if let Some(player) = this.player.clone() {
                    let rate = this.remote_playback_rate();
                    player.update(cx, |player, cx| player.play_immediately(rate, cx));
                }
                let is_playing = if this.has_local_engine() {
                    this.local_engine_is_playing(cx)
                } else {
                    this.player.is_some()
                };
                this.set_is_playing(is_playing, cx);
                if this.is_playing {
                    this.resume_playback_when_foreground = false;
                }
                if this.is_playing && !Self::is_application_active(cx) {
                    this.begin_background_task_if_needed();
                    this.schedule_background_playback_watchdog(cx);
                }
                this.refresh_now_playing_playback_state(cx);
                if publish_playback_state {
                    this.refresh_playlist_listening_presence(cx);
                    this.publish_listening_session_state(MusicListeningSessionCommand::Playback, 0, None, cx);
                }
                if this.is_playing
                    && let Some(completion) = completion
                {
                    completion(this, cx);
                }
            },
            cx,
        );
    }

    /// emergency fallback player cant render effects but must keep the session tempo
    /// else a 1.0x fallback gets dragged back on every drift check
    fn remote_playback_rate(&self) -> f32 {
        let Some(session) = &self.listening_session else {
            return 1.0;
        };
        session.playback_rate_multiplier().clamp(0.5, 2.0) as f32
    }

    pub fn is_suspended_for_external_audio(&self) -> bool {
        !self.external_audio_owners.is_empty()
    }

    pub fn is_external_audio_owner_active(&self, owner: &MusicExternalAudioOwner) -> bool {
        self.external_audio_owners.contains(owner)
    }

    pub fn suspend_for_external_audio_legacy(&mut self, resume_when_finished: bool, cx: &mut Context<Self>) {
        self.suspend_for_external_audio(MusicExternalAudioOwner::LEGACY, resume_when_finished, cx);
    }

    pub fn suspend_for_external_audio(
        &mut self,
        owner: MusicExternalAudioOwner,
        resume_when_finished: bool,
        cx: &mut Context<Self>,
    ) {
        let was_inactive = self.external_audio_owners.is_empty();
        self.external_audio_owners.insert(owner.clone());
        self.cancel_session_continuation();
        self.prepare_task = None;
        self.effect_handoff_task = None;
        if owner == MusicExternalAudioOwner::RTC {
            self.paused_for_call = true;
        }

        if !was_inactive {
            self.external_resume_intent = !self.paused_for_call
                && (self.external_resume_intent || (resume_when_finished && self.duck_origin_was_playing));
            self.external_duck_active = true;
            MusicPlayerLog::info(
                format!(
                    "DUCK retain owner={} owners={} resumeIntent={}",
                    owner.raw_value,
                    self.external_audio_owners.len(),
                    self.external_resume_intent
                ),
                cx,
            );
            if owner != MusicExternalAudioOwner::POST_AMBIENT {
                self.enforce_listening_session_for_audio_loss(self.duck_origin_was_playing, cx);
            }
            return;
        }

        let was_playing = self.is_playing;
        self.duck_origin_was_playing = was_playing;
        self.external_resume_intent = resume_when_finished && self.duck_origin_was_playing && !self.paused_for_call;
        self.external_duck_active = true;
        self.resume_playback_when_foreground = false;
        MusicPlayerLog::info(
            format!("DUCK begin owner={} resumeIntent={}", owner.raw_value, self.external_resume_intent),
            cx,
        );
        if was_playing && self.current_track.is_some() {
            let duration = if owner == MusicExternalAudioOwner::RTC { 0.0 } else { Self::DUCK_FADE_DURATION };
            self.fade_out_and_pause_for_duck(duration, cx);
        }
        if owner != MusicExternalAudioOwner::POST_AMBIENT {
            self.enforce_listening_session_for_audio_loss(was_playing, cx);
        }
    }

    pub fn resume_after_external_audio_legacy(&mut self, cx: &mut Context<Self>) {
        self.resume_after_external_audio(MusicExternalAudioOwner::LEGACY, cx);
    }

    pub fn resume_after_external_audio(&mut self, owner: MusicExternalAudioOwner, cx: &mut Context<Self>) {
        if !self.external_audio_owners.remove(&owner) {
            if owner == MusicExternalAudioOwner::RTC
                && self.is_playing
                && self.current_track.is_some()
                && self.external_audio_owners.is_empty()
            {
                self.resume_playback_after_interruption(cx);
            }
            return;
        }
        if owner == MusicExternalAudioOwner::RTC
            && let Some(pending) = self.pending_playback_after_call.take()
        {
            self.audio_session_activated = false;
            self.external_resume_intent = false;
            self.was_playing_before_interruption = false;
            pending(self, cx);
            return;
        }
        if !self.external_audio_owners.is_empty() {
            MusicPlayerLog::info(
                format!("DUCK release owner={} owners={}", owner.raw_value, self.external_audio_owners.len()),
                cx,
            );
            return;
        }

        self.external_duck_active = false;
        self.audio_session_activated = false;
        let resumes_room_after_call = owner == MusicExternalAudioOwner::RTC
            && self.paused_for_call
            && self.duck_origin_was_playing
            && self.session_paused_by_local_interruption
            && self.listening_session.as_ref().is_some_and(|session| !session.is_playing)
            && !self.is_listening_playback_displaced_by_ad_hoc();
        if resumes_room_after_call {
            self.paused_for_call = false;
        }
        let should_resume = resumes_room_after_call
            || (self.external_resume_intent && !self.suppress_external_resume_during_user_handoff && !self.paused_for_call);
        self.external_resume_intent = false;
        self.duck_origin_was_playing = false;
        self.resume_playback_when_foreground = false;
        MusicPlayerLog::info(format!("DUCK end owner={} resume={}", owner.raw_value, should_resume), cx);
        if should_resume && self.current_track.is_some() && !self.is_playing {
            self.fade_in_and_resume_from_duck(|this, cx| this.resync_listening_session_after_stall(cx), cx);
            return;
        }
        self.resync_listening_session_after_stall(cx);
    }

    pub fn take_over_external_chat_playback_for_user_control(
        &mut self,
        recover_from_interruption: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if RiseCallAudioOwnership::blocks_music() {
            return false;
        }
        if self.is_audio_session_interrupted && !recover_from_interruption {
            return false;
        }
        self.suppress_external_resume_during_user_handoff = true;
        let voice_player = ChatVoiceMessagePlayer::shared(cx);
        let video_note_player = VideoNotePlayerStore::shared(cx);
        let stops_voice = voice_player.read(cx).is_active();
        let stops_video_note = video_note_player.read(cx).is_active();
        if stops_voice || stops_video_note {
            cx.defer(move |cx| {
                if stops_voice {
                    voice_player.update(cx, |player, cx| player.stop(cx));
                }
                if stops_video_note {
                    video_note_player.update(cx, |player, cx| player.stop(cx));
                }
            });
        }
        self.external_audio_owners.remove(&MusicExternalAudioOwner::CHAT_VOICE_MESSAGE);
        self.external_audio_owners.remove(&MusicExternalAudioOwner::CHAT_VIDEO_NOTE);
        self.suppress_external_resume_during_user_handoff = false;

        if !self.external_audio_owners.is_empty() {
            let owners = self
                .external_audio_owners
                .iter()
                .map(|owner| owner.raw_value.as_ref())
                .collect::<Vec<_>>()
                .join(",");
            tracing::warn!(target: "riseonly", %owners, "music play blocked by external audio owners");
            MusicPlayerLog::info(
                format!("PLAY blocked \u{2014} non-chat external owners={}", self.external_audio_owners.len()),
                cx,
            );
            return false;
        }

        if self.is_audio_session_interrupted {
            MusicPlayerLog::info("PLAY user retry after interruption \u{2014} re-activate session".to_owned(), cx);
            self.is_audio_session_interrupted = false;
            self.audio_session_activated = false;
        }
        self.was_playing_before_interruption = false;
        self.paused_for_call = false;
        self.external_duck_active = false;
        self.external_resume_intent = false;
        self.duck_origin_was_playing = false;
        self.resume_playback_when_foreground = false;
        self.duck_fade_epoch = self.duck_fade_epoch.wrapping_add(1);
        if let Some(mut timer) = self.av_player_fade_timer.take() {
            timer.invalidate();
        }
        self.av_player_fade_completion = None;
        self.local_engine_restore_full_volume(cx);
        if let Some(player) = &self.player {
            player.update(cx, |player, cx| player.set_volume(MusicOutputVolume::gain(), cx));
        }
        true
    }

    fn fade_out_and_pause_for_duck(&mut self, duration: f64, cx: &mut Context<Self>) {
        self.duck_fade_epoch = self.duck_fade_epoch.wrapping_add(1);
        if let Some(mut timer) = self.av_player_fade_timer.take() {
            timer.invalidate();
        }
        if self.has_local_engine() {
            self.local_engine_fade_out_and_pause(duration, cx);
            self.set_is_playing(false, cx);
            self.refresh_now_playing_playback_state(cx);
            return;
        }
        if let Some(player) = self.player.clone() {
            let epoch = self.duck_fade_epoch;
            self.set_is_playing(false, cx);
            self.refresh_now_playing_playback_state(cx);
            self.fade_av_player_volume(
                0.0,
                duration,
                Some(Box::new(move |this: &mut Self, cx: &mut Context<Self>| {
                    if this.duck_fade_epoch != epoch {
                        return;
                    }
                    player.update(cx, |player, cx| player.pause(cx));
                })),
                cx,
            );
            return;
        }
        self.set_is_playing(false, cx);
    }

    fn fade_in_and_resume_from_duck(
        &mut self,
        completion: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.is_audio_session_interrupted {
            completion(self, cx);
            return;
        }
        self.duck_fade_epoch = self.duck_fade_epoch.wrapping_add(1);
        if let Some(mut timer) = self.av_player_fade_timer.take() {
            timer.invalidate();
        }
        if self.current_track.is_none() {
            completion(self, cx);
            return;
        }
        let completion: Rc<Cell<Option<Box<dyn FnOnce(&mut Self, &mut Context<Self>)>>>> =
            Rc::new(Cell::new(Some(Box::new(completion))));
        let on_skipped = completion.clone();
        self.perform_after_session_activation(
            false,
            Some(Box::new(move |this: &mut Self, cx: &mut Context<Self>| {
                if let Some(completion) = on_skipped.take() {
                    completion(this, cx);
                }
            })),
            move |this, cx| {
                let duration = Self::DUCK_FADE_DURATION;
                if this.has_local_engine() {
                    this.local_engine_fade_in_and_resume(duration, cx);
                    this.set_is_playing(true, cx);
                    if this.progress_timer.is_none() {
                        this.install_progress_timer(cx);
                    }
                    this.refresh_now_playing_playback_state(cx);
                } else if let Some(player) = this.player.clone() {
                    let rate = this.remote_playback_rate();
                    player.update(cx, |player, cx| {
                        player.set_volume(0.0, cx);
                        player.play_immediately(rate, cx);
                    });
                    this.set_is_playing(true, cx);
                    this.fade_av_player_volume(MusicOutputVolume::gain(), duration, None, cx);
                    this.refresh_now_playing_playback_state(cx);
                } else if let Some(track) = this.current_track.clone() {
                    this.play(
                        track,
                        MusicPlayOptions {
                            preserve_queue: true,
                            confirm_call_interruption: false,
                            ..MusicPlayOptions::default()
                        },
                        cx,
                    );
                }
                if let Some(completion) = completion.take() {
                    completion(this, cx);
                }
            },
            cx,
        );
    }

    fn fade_av_player_volume(
        &mut self,
        target: f32,
        duration: f64,
        completion: Option<Box<dyn FnOnce(&mut Self, &mut Context<Self>)>>,
        cx: &mut Context<Self>,
    ) {
        if let Some(mut timer) = self.av_player_fade_timer.take() {
            timer.invalidate();
        }
        let Some(player) = &self.player else {
            if let Some(completion) = completion {
                completion(self, cx);
            }
            return;
        };
        self.av_player_fade_start = RiseClock::monotonic();
        self.av_player_fade_duration = duration.max(0.01);
        self.av_player_fade_start_volume = player.read(cx).volume();
        self.av_player_fade_target_volume = target;
        self.av_player_fade_completion = completion;
        let fade_epoch = self.duck_fade_epoch;
        let timer = RepeatingTimer::scheduled_timer(Duration::from_secs_f64(1.0 / 60.0), true, cx, move |this, cx| {
            if this.duck_fade_epoch != fade_epoch {
                return;
            }
            this.tick_av_player_fade(cx);
        });
        self.av_player_fade_timer = Some(timer);
    }

    fn tick_av_player_fade(&mut self, cx: &mut Context<Self>) {
        let Some(player) = self.player.clone() else {
            if let Some(mut timer) = self.av_player_fade_timer.take() {
                timer.invalidate();
            }
            if let Some(done) = self.av_player_fade_completion.take() {
                done(self, cx);
            }
            return;
        };
        let elapsed = RiseClock::monotonic().saturating_sub(self.av_player_fade_start).as_secs_f64();
        let progress = (elapsed / self.av_player_fade_duration).clamp(0.0, 1.0) as f32;
        let start = self.av_player_fade_start_volume;
        let target = self.av_player_fade_target_volume;
        player.update(cx, |player, cx| player.set_volume(start + (target - start) * progress, cx));
        if progress >= 1.0 {
            player.update(cx, |player, cx| player.set_volume(target, cx));
            if let Some(mut timer) = self.av_player_fade_timer.take() {
                timer.invalidate();
            }
            if let Some(done) = self.av_player_fade_completion.take() {
                done(self, cx);
            }
        }
    }

    pub fn stop_and_clear(&mut self, cx: &mut Context<Self>) {
        MusicPlayerLog::info("STOP and clear \u{2014} user dismissed mini player".to_owned(), cx);
        if self.show_now_playing {
            self.set_show_now_playing(false, cx);
        }
        if self.show_queue_sheet {
            self.show_queue_sheet = false;
        }
        if self.show_audio_effects_sheet {
            self.set_show_audio_effects_sheet(false, cx);
        }
        if self.listening_session.is_some() {
            self.leave_current_playlist_listening_session(cx);
            self.detach_playlist_listening_session("cleared", false, true, cx);
        }
        self.stop_playlist_listening_presence_if_needed(cx);
        self.clear_playback_engine_and_context(cx);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        if self.show_now_playing {
            self.set_show_now_playing(false, cx);
        }
        if self.show_queue_sheet {
            self.show_queue_sheet = false;
        }
        if self.show_audio_effects_sheet {
            self.set_show_audio_effects_sheet(false, cx);
        }
        if self.listening_session.is_some() {
            self.detach_playlist_listening_session("account_changed", false, false, cx);
        }
        self.reset_listening_session_transition(cx);
        self.is_listening_session_exit_in_flight = false;
        self.listening_session_exit_session_id = None;
        self.presence_playlist_id = None;
        self.listening_observation_playlist_id = None;
        self.show_playlist_listeners_sheet = false;
        self.show_listening_session_sheet = false;
        self.last_listening_error = None;
        self.clear_playback_engine_and_context(cx);
    }

    /// drops the outgoing audio without leaving the session
    /// clearing only the ad-hoc key lets a pending load finish while the session track resolves
    fn clear_playback_engine_and_context(&mut self, cx: &mut Context<Self>) {
        self.call_playback_request_generation = self.call_playback_request_generation.wrapping_add(1);
        self.pending_playback_after_call = None;
        self.paused_for_call = false;
        self.audio_effect_settings_before_reset = None;
        self.was_playing_before_interruption = false;
        self.cancel_pending_audio_effect_commits();
        self.external_resume_intent = false;
        self.duck_origin_was_playing = false;
        self.resume_playback_when_foreground = false;
        self.duck_fade_epoch = self.duck_fade_epoch.wrapping_add(1);
        if let Some(mut timer) = self.av_player_fade_timer.take() {
            timer.invalidate();
        }
        self.av_player_fade_completion = None;
        self.background_keep_alive_task = None;
        self.end_background_task();
        self.cancel_session_continuation();
        self.prepare_generation = self.prepare_generation.wrapping_add(1);
        self.prepare_task = None;
        self.prefetch_task = None;
        MusicPlaybackPrefetchRegistry::cancel_jobs(&HashSet::new());
        self.is_handling_track_end = false;
        self.is_preparing_playback = false;
        self.stop_file_player(cx);
        self.release_av_player(cx);
        self.set_is_playing(false, cx);
        self.progress = 0.0;
        self.current_time = 0.0;
        self.duration = 0.0;
        self.current_track = None;
        self.queue = Vec::new();
        self.source_queue = Vec::new();
        self.queue_index = 0;
        self.playback_scope_id = None;
        if self.playback_ad_hoc_effects_key.is_some() {
            self.set_playback_ad_hoc_effects_key(None, cx);
            let settings = MusicAudioEffectStore::shared(cx).read(cx).settings().clone();
            self.set_audio_effect_settings(settings, cx);
        }
        self.playback_owner_user_id = None;
        self.playback_owner_account_epoch = None;
        self.clear_playback_download_progress(cx);
        self.set_last_playback_error(None, cx);
        self.cancel_now_playing_artwork();
        if let Some(media_keys) = &mut self.media_keys {
            media_keys.clear();
        }
        cx.notify();
    }

    pub fn remove_permanently_deleted_tracks(&mut self, track_ids: &HashSet<String>, cx: &mut Context<Self>) {
        let ids: HashSet<String> = track_ids
            .iter()
            .map(|raw| trimming_whitespaces_and_newlines(raw))
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect();
        if ids.is_empty() {
            return;
        }

        let user_id = self.playback_owner_user_id.clone().or_else(SaiAccountCacheScope::active_user_id);
        for id in &ids {
            MusicPlaybackDiskCache::clear_track(id, user_id.as_deref());
        }

        if let Some(current_track) = &self.current_track
            && ids.contains(&current_track.id)
        {
            MusicPlayerLog::info(format!("STOP permanently deleted track id={}", current_track.id), cx);
            self.stop_and_clear(cx);
            return;
        }

        self.source_queue.retain(|track| !ids.contains(&track.id));
        self.queue.retain(|track| !ids.contains(&track.id));
        let current_index = self
            .current_track
            .as_ref()
            .and_then(|current_track| self.queue.iter().position(|track| track.id == current_track.id));
        self.queue_index = match current_index {
            Some(current_index) => current_index,
            None => self.queue_index.min(self.queue.len().saturating_sub(1)),
        };
        cx.notify();
    }

    fn artwork_identity(&self, track: &TrackModel) -> NowPlayingArtworkIdentity {
        NowPlayingArtworkIdentity {
            track_id: track.id.clone(),
            cover_source: track.cover_image_url.clone(),
            audio_source: track.stable_audio_url(),
            account_user_id: self.playback_owner_user_id.clone(),
            account_epoch: self.playback_owner_account_epoch.clone(),
        }
    }

    fn cancel_now_playing_artwork(&mut self) {
        self.now_playing_artwork_task = None;
        self.now_playing_artwork_generation = Uuid::new_v4();
        self.now_playing_artwork_identity = None;
        self.now_playing_artwork = None;
    }

    fn prefetch_track_cover(&mut self, track: &TrackModel, cx: &mut Context<Self>) {
        let identity = self.artwork_identity(track);
        let current_identity = self.current_track.as_ref().map(|current| self.artwork_identity(current));
        if !identity.is_current_account() || current_identity.as_ref() != Some(&identity) {
            return;
        }
        if self.now_playing_artwork_identity.as_ref() == Some(&identity)
            && (self.now_playing_artwork.is_some() || self.now_playing_artwork_task.is_some())
        {
            return;
        }
        self.cancel_now_playing_artwork();
        self.now_playing_artwork_identity = Some(identity.clone());
        let generation = self.now_playing_artwork_generation;
        self.now_playing_artwork_task = Some(cx.spawn(async move |this, cx| {
            let image = Self::prepare_now_playing_artwork(identity.clone(), cx).await;
            let artwork = match image {
                Some(image) => {
                    cx.background_executor()
                        .spawn(async move { MusicNowPlayingArtwork::make(&image) })
                        .await
                }
                None => None,
            };
            let _ = this.update(cx, |this, cx| {
                if this.now_playing_artwork_generation != generation
                    || this.now_playing_artwork_identity.as_ref() != Some(&identity)
                {
                    return;
                }
                let Some(current) = this.current_track.clone() else { return };
                if this.artwork_identity(&current) != identity || !identity.is_current_account() {
                    return;
                }
                if let Some(task) = this.now_playing_artwork_task.take() {
                    task.detach();
                }
                let Some(artwork) = artwork else { return };
                this.now_playing_artwork = Some(artwork);
                this.update_now_playing_info(&current, cx);
            });
        }));
    }

    async fn prepare_now_playing_artwork(identity: NowPlayingArtworkIdentity, cx: &AsyncApp) -> Option<CleverImage> {
        if !identity.is_current_account() {
            return None;
        }
        let maximum_edge = Self::now_playing_artwork_maximum_edge();
        let sources = MusicMediaURL::cover_url_strings_for_fetch(Some(&identity.cover_source));
        for source in &sources {
            let image = CleverImageRenderMemoryCache::image(
                source,
                maximum_edge,
                identity.account_user_id.as_deref(),
                identity.account_epoch.as_deref(),
            )
            .or_else(|| {
                CleverImageBitmapLoader::memory_image(
                    source,
                    false,
                    identity.account_user_id.as_deref(),
                    identity.account_epoch.as_deref(),
                    None,
                    CleverImageCachePolicy::Standard,
                    None,
                    false,
                )
            });
            if let Some(image) = image {
                return Self::prepare_now_playing_bitmap(image, identity, cx).await;
            }
        }
        for source in &sources {
            if !identity.is_current_account() {
                return None;
            }
            let image = match Url::from_string(source).filter(Url::is_file_url) {
                Some(url) => {
                    let guard = identity.clone();
                    let path = url.to_file_path();
                    CleverImageBitmapLoader::prepare_bitmap(
                        move || {
                            if !guard.is_current_account() {
                                return None;
                            }
                            let data = std::fs::read(path.as_ref()?).ok()?;
                            CleverImageBitmapDecoder::decode(&data, Some(maximum_edge))
                        },
                        cx,
                    )
                    .await
                }
                None => {
                    CleverImageBitmapLoader::load(
                        source,
                        true,
                        true,
                        identity.account_user_id.clone(),
                        identity.account_epoch.clone(),
                        Some(maximum_edge),
                        CleverImageCachePolicy::Standard,
                        CleverImageWorkLane::Visible,
                        cx,
                    )
                    .await
                }
            };
            if let Some(image) = image {
                return Self::prepare_now_playing_bitmap(image, identity, cx).await;
            }
        }
        None
    }

    async fn prepare_now_playing_bitmap(
        image: CleverImage,
        identity: NowPlayingArtworkIdentity,
        cx: &AsyncApp,
    ) -> Option<CleverImage> {
        let maximum_edge = Self::now_playing_artwork_maximum_edge();
        CleverImageBitmapLoader::prepare_bitmap(
            move || {
                if !identity.is_current_account() {
                    return None;
                }
                let trimmed = MusicCoverLetterbox::trimmed(&image);
                let size = trimmed.size(0);
                let width = f64::from(size.width.0);
                let height = f64::from(size.height.0);
                let edge = width.max(height);
                if edge <= 0.0 {
                    return None;
                }
                let scale = (maximum_edge as f64 / edge).min(1.0);
                let target_width = ((width * scale).round() as u32).max(1);
                let target_height = ((height * scale).round() as u32).max(1);
                let source_width = u32::try_from(size.width.0).ok()?;
                let source_height = u32::try_from(size.height.0).ok()?;
                let bytes = trimmed.as_bytes(0)?;
                let mut drawn: RgbaImage = if (target_width, target_height) == (source_width, source_height) {
                    RgbaImage::from_raw(source_width, source_height, bytes.to_vec())?
                } else {
                    let source: ImageBuffer<Rgba<u8>, &[u8]> = ImageBuffer::from_raw(source_width, source_height, bytes)?;
                    imageops::resize(&source, target_width, target_height, FilterType::Triangle)
                };
                for pixel in drawn.pixels_mut() {
                    let alpha = u16::from(pixel.0[3]);
                    for channel in &mut pixel.0[..3] {
                        *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
                    }
                    pixel.0[3] = u8::MAX;
                }
                Some(Arc::new(RenderImage::new(vec![Frame::from_parts(drawn, 0, 0, trimmed.delay(0))])))
            },
            cx,
        )
        .await
    }

    fn update_now_playing_info(&mut self, track: &TrackModel, cx: &mut Context<Self>) {
        let Some(current) = self.current_track.clone() else { return };
        if current.id != track.id || !self.artwork_identity(&current).is_current_account() {
            return;
        }
        let track = current;
        self.prefetch_track_cover(&track, cx);
        let state = if self.effect_playback_rate_for_now_playing(cx) > 0.0 {
            PlaybackState::Playing
        } else {
            PlaybackState::Paused
        };
        let mut info = NowPlaying::new(track.title.clone(), track.artist.clone())
            .with_album(track.album_title.clone())
            .at(Self::now_playing_seconds(self.current_time), self.active_playback_rate(cx));
        let duration = if self.duration > 0.0 { self.duration } else { track.duration_seconds as f64 };
        if duration > 0.0 {
            info = info.with_duration(Self::now_playing_seconds(duration));
        }
        if self.now_playing_artwork_identity.as_ref() == Some(&self.artwork_identity(&track))
            && let Some(artwork) = &self.now_playing_artwork
        {
            info = info.with_artwork(artwork.clone());
        }
        if self.media_service_cancelled {
            self.start_media_service(cx);
        }
        self.bind_media_window(cx);
        if let Some(media_keys) = &mut self.media_keys {
            media_keys.set_now_playing(&info, state);
        }
    }

    fn now_playing_seconds(seconds: f64) -> Duration {
        if seconds.is_finite() && seconds > 0.0 { Duration::from_secs_f64(seconds) } else { Duration::ZERO }
    }

    fn refresh_now_playing_playback_state(&mut self, cx: &mut Context<Self>) {
        let Some(track) = self.current_track.clone() else {
            if let Some(media_keys) = &mut self.media_keys {
                media_keys.clear();
            }
            return;
        };
        self.update_now_playing_info(&track, cx);
    }

    fn disk_playback_url(track_id: &str, ext: &str, user_id: Option<&str>) -> Option<PathBuf> {
        MusicPlaybackDiskCache::file_url(track_id, ext, user_id).ok()
    }

    fn prepare_playback_cache(
        track_id: &str,
        fallback_extension: &str,
        owner_user_id: Option<&str>,
        skip_cache: bool,
    ) -> PlaybackCachePreparation {
        let mut log_messages = Vec::new();
        if !skip_cache
            && let Some(adopted) =
                MusicPlaybackDiskCache::adopt_legacy_temp_file_if_needed(track_id, fallback_extension, owner_user_id)
        {
            log_messages.push(format!("DOWNLOAD migrated legacy temp → {}", Self::last_path_component(&adopted)));
        }
        let Some(mut destination) = Self::disk_playback_url(track_id, fallback_extension, owner_user_id) else {
            return PlaybackCachePreparation { destination: None, cached_url: None, log_messages };
        };

        if !skip_cache
            && let Some(cached) = MusicPlaybackDiskCache::find_cached_url(track_id, owner_user_id)
            && let Ok(attributes) = FileManager.attributes_of_item(&cached)
            && attributes.size > 1_024
            && attributes.size <= MusicPlaybackDiskCache::MAXIMUM_FILE_BYTE_COUNT
        {
            let size = attributes.size;
            destination = cached;
            let head = Self::read_file_head(&destination, 16).unwrap_or_default();
            let format = Self::audio_format_label(&head);
            let valid = Self::is_likely_audio_file(&head);
            log_messages.push(format!(
                "DOWNLOAD cache hit trackId={track_id} bytes={size} format={format} magicOK={valid} path={}",
                Self::last_path_component(&destination)
            ));
            if valid || size > 8_192 {
                if !valid {
                    log_messages.push(format!("DOWNLOAD accept cached file without magic header (size={size})"));
                }
                destination = MusicPlaybackDiskCache::relocate_to_match_format_if_needed(track_id, &destination, format);
                return PlaybackCachePreparation {
                    destination: Some(destination.clone()),
                    cached_url: Some(destination),
                    log_messages,
                };
            }
            MusicPlaybackDiskCache::remove_file(&destination);
        }
        MusicPlaybackDiskCache::remove_file(&destination);
        PlaybackCachePreparation { destination: Some(destination), cached_url: None, log_messages }
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_playback_download(
        temporary_url: &Path,
        initial_destination: &Path,
        track_id: &str,
        fallback_extension: &str,
        content_type: &str,
        owner_user_id: Option<&str>,
        owner_account_epoch: Option<&str>,
        commit_lease: Option<&MusicPlaybackDownloadCommitLease>,
    ) -> PlaybackCacheCommit {
        if !SaiAccountCacheScope::is_current_account_epoch(owner_user_id, owner_account_epoch) {
            return PlaybackCacheCommit { url: None, error: Some("Cancelled".to_owned()), log_messages: Vec::new() };
        }

        let staging_url = initial_destination
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(format!(".incoming-{}", Uuid::new_v4().uuid_string().to_lowercase()));
        let commit = Self::commit_staged_playback_download(
            temporary_url,
            &staging_url,
            initial_destination,
            track_id,
            fallback_extension,
            content_type,
            owner_user_id,
            owner_account_epoch,
            commit_lease,
        );
        let _ = FileManager.remove_item(&staging_url);
        match commit {
            Ok(commit) => commit,
            Err(error) => PlaybackCacheCommit { url: None, error: Some(error), log_messages: Vec::new() },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_staged_playback_download(
        temporary_url: &Path,
        staging_url: &Path,
        initial_destination: &Path,
        track_id: &str,
        fallback_extension: &str,
        content_type: &str,
        owner_user_id: Option<&str>,
        owner_account_epoch: Option<&str>,
        commit_lease: Option<&MusicPlaybackDownloadCommitLease>,
    ) -> Result<PlaybackCacheCommit, String> {
        let mut destination = initial_destination.to_path_buf();
        FileManager.move_item(temporary_url, staging_url).map_err(|error| error.to_string())?;
        let size = FileManager.attributes_of_item(staging_url).map(|attributes| attributes.size).unwrap_or(0);
        let head = Self::read_file_head(staging_url, 16).unwrap_or_default();
        let format = Self::audio_format_label(&head);
        let valid = Self::is_likely_audio_file(&head);
        let audio_content =
            content_type.starts_with("audio/") || content_type.contains("mpeg") || content_type.contains("mp4");
        if size <= 1_024
            || size > MusicPlaybackDiskCache::MAXIMUM_FILE_BYTE_COUNT
            || (!valid && !(audio_content && size > 8_192))
        {
            let head_hex = Self::file_head_hex(&head);
            return Ok(PlaybackCacheCommit {
                url: None,
                error: Some("Downloaded file is not a valid audio".to_owned()),
                log_messages: vec![format!(
                    "DOWNLOAD magic check failed trackId={track_id} bytes={size} head={head_hex} format={format}"
                )],
            });
        }
        let correct_extension = Self::file_extension_for_format(format, fallback_extension);
        if let Some(final_destination) = Self::disk_playback_url(track_id, correct_extension, owner_user_id) {
            destination = final_destination;
        }
        if !SaiAccountCacheScope::is_current_account_epoch(owner_user_id, owner_account_epoch) {
            return Ok(PlaybackCacheCommit { url: None, error: Some("Cancelled".to_owned()), log_messages: Vec::new() });
        }
        let install = || -> Result<bool, String> {
            if !SaiAccountCacheScope::is_current_account_epoch(owner_user_id, owner_account_epoch) {
                return Ok(false);
            }
            if FileManager.file_exists(&destination) {
                std::fs::rename(staging_url, &destination).map_err(|error| error.to_string())?;
            } else {
                FileManager.move_item(staging_url, &destination).map_err(|error| error.to_string())?;
            }
            if initial_destination != destination {
                MusicPlaybackDiskCache::remove_file(initial_destination);
            }
            MusicPlaybackDiskCache::register_saved_file(track_id, &Self::last_path_component(&destination), owner_user_id);
            Ok(true)
        };
        let lease_is_current = commit_lease.is_none_or(MusicPlaybackDownloadCommitLease::is_valid);
        let committed = install()?;
        if !committed {
            return Ok(PlaybackCacheCommit { url: None, error: Some("Cancelled".to_owned()), log_messages: Vec::new() });
        }
        if !lease_is_current {
            return Ok(PlaybackCacheCommit {
                url: None,
                error: Some("Cancelled".to_owned()),
                log_messages: vec![format!(
                    "DOWNLOAD cached superseded bytes trackId={track_id} file={}",
                    Self::last_path_component(&destination)
                )],
            });
        }
        if !FileManager.file_exists(&destination) {
            return Ok(PlaybackCacheCommit {
                url: None,
                error: Some("Audio cache is unavailable".to_owned()),
                log_messages: Vec::new(),
            });
        }
        let mut log_messages = Vec::new();
        if !valid {
            log_messages.push(format!(
                "DOWNLOAD accept by Content-Type/size trackId={track_id} bytes={size} ctype={content_type}"
            ));
        }
        log_messages.push(format!(
            "DOWNLOAD saved trackId={track_id} bytes={size} format={format} file={} head={}",
            Self::last_path_component(&destination),
            Self::file_head_hex(&head)
        ));
        Ok(PlaybackCacheCommit { url: Some(destination), error: None, log_messages })
    }

    fn last_path_component(url: &Path) -> String {
        url.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn read_file_head(url: &Path, count: usize) -> Option<Vec<u8>> {
        use std::io::Read;
        let file = std::fs::File::open(url).ok()?;
        let mut head = Vec::with_capacity(count);
        file.take(count as u64).read_to_end(&mut head).ok()?;
        Some(head)
    }

    fn audio_format_label(head: &[u8]) -> &'static str {
        if head.len() >= 3 && head.starts_with(&[0x49, 0x44, 0x33]) {
            return "mp3-id3";
        }
        if head.len() >= 2 && head[0] == 0xFF && (head[1] & 0xE0) == 0xE0 {
            return "mp3-frame";
        }
        if head.len() >= 8 && head[4..8] == [0x66, 0x74, 0x79, 0x70] {
            return "m4a/mp4";
        }
        if head.starts_with(&[0x66, 0x4C, 0x61, 0x43]) {
            return "flac";
        }
        if head.starts_with(&[0x4F, 0x67, 0x67, 0x53]) {
            return "ogg";
        }
        if head.starts_with(&[0x52, 0x49, 0x46, 0x46]) {
            return "wav";
        }
        "unknown"
    }

    fn file_extension_for_format<'a>(format: &str, fallback: &'a str) -> &'a str {
        match format {
            "m4a/mp4" => "m4a",
            "flac" => "flac",
            "ogg" => "ogg",
            "wav" => "wav",
            "mp3-id3" | "mp3-frame" => "mp3",
            _ if fallback.is_empty() => "mp3",
            _ => fallback,
        }
    }

    fn make_player_item(url: &Url) -> AVPlayerItem {
        AVPlayerItem::new(url.clone())
    }

    fn log_av_player_failure(error: Option<&AVPlayerError>, track_id: &str, cx: &mut App) {
        if let Some(error) = error {
            MusicPlayerLog::fail(format!("AVPlayer error id={track_id} {error}"), cx);
        }
    }

    fn clear_disk_playback_cache(track_id: &str, user_id: Option<&str>) {
        MusicPlaybackDiskCache::clear_track(track_id, user_id);
    }

    fn file_head_hex(head: &[u8]) -> String {
        if head.is_empty() {
            return "?".to_owned();
        }
        head.iter().map(|byte| format!("{byte:02X}")).collect::<Vec<_>>().join(" ")
    }

    fn is_likely_audio_file(head: &[u8]) -> bool {
        if head.len() < 3 {
            return false;
        }
        Self::audio_format_label(head) != "unknown"
    }

    /// downloads or finds the audio for playback and gives a local file
    pub async fn materialize_audio_for_playback(
        remote: Url,
        track_id: String,
        owner_user_id: Option<String>,
        owner_account_epoch: Option<String>,
        authorization_token: Option<String>,
        cx: &mut AsyncApp,
    ) -> Option<Url> {
        let (url, _) = Self::download_audio_for_playback(
            remote,
            track_id,
            false,
            0,
            owner_user_id,
            owner_account_epoch,
            authorization_token,
            None,
            None,
            cx.clone(),
        )
        .await;
        url.and_then(|path| Url::from_file_path(&path))
    }

    #[allow(clippy::too_many_arguments)]
    fn download_audio_for_playback(
        raw_remote: Url,
        track_id: String,
        skip_cache: bool,
        retry_attempt: i64,
        owner_user_id: Option<String>,
        owner_account_epoch: Option<String>,
        authorization_token: Option<String>,
        commit_lease: Option<Arc<MusicPlaybackDownloadCommitLease>>,
        on_progress: Option<MusicPlaybackDownloadProgressHandler>,
        cx: AsyncApp,
    ) -> LocalBoxFuture<'static, MusicPlaybackPrefetchResult> {
        async move {
            let mut trace = MusicAudioDownloadTraceScope::begin();
            let outcome = Self::download_audio_for_playback_attempt(
                raw_remote,
                track_id,
                skip_cache,
                retry_attempt,
                owner_user_id,
                owner_account_epoch,
                authorization_token,
                commit_lease,
                on_progress,
                cx,
            )
            .await;
            trace.is_completed = true;
            outcome
        }
        .boxed_local()
    }

    #[allow(clippy::too_many_arguments)]
    async fn download_audio_for_playback_attempt(
        raw_remote: Url,
        track_id: String,
        skip_cache: bool,
        retry_attempt: i64,
        owner_user_id: Option<String>,
        owner_account_epoch: Option<String>,
        authorization_token: Option<String>,
        commit_lease: Option<Arc<MusicPlaybackDownloadCommitLease>>,
        on_progress: Option<MusicPlaybackDownloadProgressHandler>,
        cx: AsyncApp,
    ) -> MusicPlaybackPrefetchResult {
        let cancelled = || (None, Some("Cancelled".to_owned()));
        let lease_is_valid = |lease: &Option<Arc<MusicPlaybackDownloadCommitLease>>| {
            lease.as_deref().is_none_or(MusicPlaybackDownloadCommitLease::is_valid)
        };
        if !lease_is_valid(&commit_lease) {
            return cancelled();
        }
        if !SaiAccountCacheScope::is_current_account_epoch(owner_user_id.as_deref(), owner_account_epoch.as_deref()) {
            return cancelled();
        }
        let remote = if raw_remote.scheme().is_none() {
            MusicPlaybackUrl::playback_url(raw_remote.absolute_string())
        } else {
            Some(raw_remote)
        };
        let Some(remote) = remote else {
            return (None, Some("Invalid URL".to_owned()));
        };
        let request_access_token = authorization_token.unwrap_or_else(|| {
            cx.update(|cx| AuthServicesStore::shared(cx).read(cx).tokens_and_other_data().access_token.clone())
        });
        let path_extension = remote.path_extension();
        let fallback_ext = if path_extension.is_empty() { "mp3".to_owned() } else { path_extension };
        let PlaybackCachePreparation { destination, cached_url, log_messages } = {
            let track_id = track_id.clone();
            let fallback_ext = fallback_ext.clone();
            let owner_user_id = owner_user_id.clone();
            cx.background_executor()
                .spawn(async move {
                    Self::prepare_playback_cache(&track_id, &fallback_ext, owner_user_id.as_deref(), skip_cache)
                })
                .await
        };
        cx.update(|cx| {
            for message in log_messages {
                MusicPlayerLog::info(message, cx);
            }
        });
        if !lease_is_valid(&commit_lease)
            || !SaiAccountCacheScope::is_current_account_epoch(owner_user_id.as_deref(), owner_account_epoch.as_deref())
        {
            return cancelled();
        }
        let transfer_key = RiseMediaCacheIdentity::key(&remote);
        if let Some(cached_url) = cached_url {
            RiseAccountRuntime::current().spawn(async move {
                RiseMediaTransferHub::shared().ingest(transfer_key, 0, 0, true).await;
            });
            return (Some(cached_url), None);
        }
        let Some(destination) = destination else {
            return (None, Some("Cache path unavailable".to_owned()));
        };

        cx.update(|cx| {
            MusicPlayerLog::info(
                format!("DOWNLOAD start trackId={track_id} url={}", remote.absolute_string()),
                cx,
            );
        });

        let mut request = URLRequest::new(remote.clone());
        request.timeout_interval = Duration::try_from_secs_f64(Self::playback_download_idle_timeout())
            .unwrap_or(URLRequest::DEFAULT_TIMEOUT_INTERVAL);
        request.http_method = "GET".to_owned();
        request.set_value("no-store", reqwest::header::CACHE_CONTROL.as_str());
        if let Some(token) = RemoteMediaDiskCache::bearer_token(&remote, Some(&request_access_token), &cx).await {
            request.set_value(&format!("Bearer {token}"), reqwest::header::AUTHORIZATION.as_str());
        }

        let operation = {
            let on_progress = on_progress.clone();
            let transfer_key = transfer_key.clone();
            MusicAudioDownloadOperation::new(MusicPlaybackDiskCache::MAXIMUM_FILE_BYTE_COUNT, move |progress| {
                if let Some(on_progress) = &on_progress {
                    on_progress(progress, retry_attempt);
                }
                let cache_key = transfer_key.clone();
                RiseAccountRuntime::current().spawn(async move {
                    RiseMediaTransferHub::shared()
                        .ingest(cache_key, progress.downloaded_bytes(), progress.total_bytes(), false)
                        .await;
                });
            })
        };
        let download = RiseAccountRuntime::current().spawn(async move { operation.start(&request).await });
        let _abort_on_drop = MusicAudioDownloadAbortOnDrop(download.abort_handle());
        let Ok(downloaded) = download.await else {
            return cancelled();
        };

        match downloaded {
            Ok((temp_url, response)) => {
                let _temporary_removal = MusicAudioDownloadTemporaryRemoval(temp_url.clone());
                let still_wanted = lease_is_valid(&commit_lease);
                if !SaiAccountCacheScope::is_current_account_epoch(
                    owner_user_id.as_deref(),
                    owner_account_epoch.as_deref(),
                ) {
                    return cancelled();
                }
                let content_type = response
                    .value(reqwest::header::CONTENT_TYPE.as_str())
                    .map(str::to_lowercase)
                    .unwrap_or_default();
                let status_code = response.status_code;
                cx.update(|cx| {
                    MusicPlayerLog::info(
                        format!(
                            "DOWNLOAD response trackId={track_id} HTTP={status_code} Content-Type={}",
                            if content_type.is_empty() { "?" } else { content_type.as_str() }
                        ),
                        cx,
                    );
                });
                if content_type.contains("text/html")
                    || content_type.contains("application/json")
                    || content_type.contains("application/xml")
                {
                    cx.update(|cx| {
                        MusicPlayerLog::fail(
                            format!(
                                "DOWNLOAD not audio (HTML/XML) trackId={track_id} url={}",
                                remote.absolute_string()
                            ),
                            cx,
                        );
                    });
                    return (None, Some("Invalid audio URL (not a media file)".to_owned()));
                }
                if !(200..=299).contains(&status_code) {
                    cx.update(|cx| {
                        MusicPlayerLog::fail(format!("DOWNLOAD HTTP {status_code} trackId={track_id}"), cx);
                    });
                    return (None, Some(format!("Audio download failed (HTTP {status_code})")));
                }
                let expected_content_length = response.expected_content_length();
                if expected_content_length > 0
                    && expected_content_length > MusicPlaybackDiskCache::MAXIMUM_FILE_BYTE_COUNT
                {
                    return (None, Some("Audio file is too large".to_owned()));
                }
                let commit = {
                    let temp_url = temp_url.clone();
                    let track_id = track_id.clone();
                    let fallback_ext = fallback_ext.clone();
                    let content_type = content_type.clone();
                    let owner_user_id = owner_user_id.clone();
                    let owner_account_epoch = owner_account_epoch.clone();
                    let commit_lease = commit_lease.clone();
                    cx.background_executor()
                        .spawn(async move {
                            Self::commit_playback_download(
                                &temp_url,
                                &destination,
                                &track_id,
                                &fallback_ext,
                                &content_type,
                                owner_user_id.as_deref(),
                                owner_account_epoch.as_deref(),
                                commit_lease.as_deref(),
                            )
                        })
                        .await
                };
                let PlaybackCacheCommit { url: committed_url, error: commit_error, log_messages } = commit;
                cx.update(|cx| {
                    for message in log_messages {
                        if commit_error.is_some() {
                            MusicPlayerLog::fail(message, cx);
                        } else if message.contains("DOWNLOAD saved") {
                            MusicPlayerLog::ok(message, cx);
                        } else {
                            MusicPlayerLog::info(message, cx);
                        }
                    }
                });
                if !still_wanted || !lease_is_valid(&commit_lease) {
                    return cancelled();
                }
                if let Some(error) = commit_error {
                    return (None, Some(error));
                }
                let Some(committed_url) = committed_url else {
                    return (None, Some("Audio cache is unavailable".to_owned()));
                };
                RiseAccountRuntime::current()
                    .spawn(async move {
                        RiseMediaTransferHub::shared().ingest(transfer_key, 0, 0, true).await;
                    })
                    .await
                    .ok();
                (Some(committed_url), None)
            }
            Err(error) => {
                if !lease_is_valid(&commit_lease) {
                    return cancelled();
                }
                if error.downcast_ref::<MusicAudioDownloadOperationError>().is_some() {
                    return (None, Some("Audio file is too large".to_owned()));
                }
                let error = RemoteMediaTransferError::from_anyhow(error);
                let timed_out =
                    matches!(&error, RemoteMediaTransferError::Url(url_error) if url_error.code == URLErrorCode::TimedOut);
                if !timed_out && RemoteMediaDownloadRetryPolicy::should_retry(&error, retry_attempt) {
                    let next_attempt = retry_attempt + 1;
                    let code = match &error {
                        RemoteMediaTransferError::Url(url_error) => format!("{:?}", url_error.code),
                        other => other.to_string(),
                    };
                    cx.update(|cx| {
                        MusicPlayerLog::info(
                            format!(
                                "DOWNLOAD transient error trackId={track_id} retry={next_attempt}/{} code={code}",
                                RemoteMediaDownloadRetryPolicy::maximum_retries()
                            ),
                            cx,
                        );
                    });
                    if let Some(on_progress) = &on_progress {
                        on_progress(MusicAudioDownloadProgress::new(0, 0), next_attempt);
                    }
                    cx.background_executor().timer(RemoteMediaDownloadRetryPolicy::delay(next_attempt)).await;
                    if !lease_is_valid(&commit_lease) {
                        return cancelled();
                    }
                    return Self::download_audio_for_playback(
                        remote,
                        track_id,
                        skip_cache,
                        next_attempt,
                        owner_user_id,
                        owner_account_epoch,
                        Some(request_access_token),
                        commit_lease,
                        on_progress,
                        cx,
                    )
                    .await;
                }
                let description = error.to_string();
                cx.update(|cx| {
                    MusicPlayerLog::fail(format!("DOWNLOAD network error trackId={track_id}: {description}"), cx);
                });
                (None, Some(description))
            }
        }
    }

    async fn wait_until_app_is_active(timeout_ms: i64, cx: &AsyncApp) {
        let deadline = RiseClock::now().adding_time_interval(timeout_ms as f64 / 1000.0);
        while !cx.update(|cx| Self::is_application_active(cx)) && deadline.time_interval_since_now() > 0.0 {
            cx.background_executor().timer(Duration::from_millis(50)).await;
        }
    }

    pub fn play_and_wait(
        &mut self,
        track: TrackModel,
        queue: Vec<TrackModel>,
        cx: &mut Context<Self>,
    ) -> Task<Option<String>> {
        cx.spawn(async move |this, cx| {
            Self::wait_until_app_is_active(4_000, cx).await;
            if this
                .update(cx, |this, cx| {
                    this.play(track, MusicPlayOptions { queue, ..MusicPlayOptions::default() }, cx);
                })
                .is_err()
            {
                return None;
            }
            let poll = Duration::from_millis(100);
            for _ in 0..150 {
                let Ok((error, is_playing, is_preparing)) = this.read_with(cx, |this, _| {
                    (this.last_playback_error().map(str::to_owned), this.is_playing, this.is_preparing_playback)
                }) else {
                    return None;
                };
                if error.is_some() {
                    return error;
                }
                if is_playing {
                    return None;
                }
                if !is_preparing {
                    cx.background_executor().timer(poll).await;
                    let Ok((error, is_playing)) = this.read_with(cx, |this, _| {
                        (this.last_playback_error().map(str::to_owned), this.is_playing)
                    }) else {
                        return None;
                    };
                    if is_playing {
                        return None;
                    }
                    if error.is_some() {
                        return error;
                    }
                }
                cx.background_executor().timer(poll).await;
            }
            this.read_with(cx, |this, _| {
                Some(this.last_playback_error().map_or_else(|| "Playback timed out".to_owned(), str::to_owned))
            })
            .ok()
            .flatten()
        })
    }

    pub fn play(&mut self, track: TrackModel, options: MusicPlayOptions, cx: &mut Context<Self>) {
        let MusicPlayOptions {
            queue: new_queue,
            scope_id,
            record_play_on_success,
            preserve_queue,
            confirm_call_interruption,
            on_playback_accepted,
        } = options;
        let request = MusicPlayRequest {
            track: track.clone(),
            queue: new_queue.clone(),
            scope_id: scope_id.clone(),
            record_play_on_success,
            preserve_queue,
            on_playback_accepted: Rc::new(RefCell::new(on_playback_accepted)),
        };
        if confirm_call_interruption && !self.is_applying_listening_session {
            let retry = request.clone();
            if !self.request_playback_after_leaving_call(
                Box::new(move |this: &mut MusicPlayerState, cx: &mut Context<MusicPlayerState>| {
                    retry.replay(true, this, cx)
                }),
                cx,
            ) {
                return;
            }
            self.call_playback_request_generation = self.call_playback_request_generation.wrapping_add(1);
        }
        let trace = ChatPerformanceTrace::shared();
        trace.record("audio.play.request", HashMap::new());
        let _dispatch = MusicPlayDispatchTrace(trace.begin_operation(
            "audio.play.dispatch",
            HashMap::from([("execution".to_owned(), "main".to_owned())]),
        ));
        let effective_scope_id =
            scope_id.clone().or_else(|| if preserve_queue { self.playback_scope_id.clone() } else { None });
        let incoming_ad_hoc = Self::is_ad_hoc_playback_context(effective_scope_id.as_deref(), &track);

        let previous_track_id = self.current_track.as_ref().map(|current| current.id.clone());
        if incoming_ad_hoc && self.listening_session.is_some() && !self.is_applying_listening_session {
            if self.listening_interruption_decision() != MusicListeningInterruptionDecision::Proceed {
                let retry_request = request.clone();
                let retry: MusicListeningInterruptionCallback = Rc::new(move |cx: &mut App| {
                    let retry_request = retry_request.clone();
                    MusicPlayerState::shared(cx)
                        .update(cx, move |player, cx| retry_request.replay(true, player, cx));
                });
                cx.defer(move |cx| {
                    MusicListeningInterruptionGate::shared(cx).update(cx, |gate, cx| {
                        gate.request_external_playback(None, false, retry, None, cx);
                    });
                });
                return;
            }
        } else {
            if !incoming_ad_hoc
                && !self.is_applying_listening_session
                && let Some(left) = self.room_left_for_ad_hoc.take()
                && self.listening_session.is_none()
                && left.account_user_id == Self::current_listening_user_id()
                && effective_scope_id.as_deref() == Some(format!("playlist:{}", left.playlist_id).as_str())
            {
                let rejoin_request = request.clone();
                self.request_join_playlist_listening_session(
                    &left.playlist_id,
                    &left.host_user_id,
                    None,
                    None,
                    Some(Box::new(move |_error: String, cx: &mut App| {
                        MusicPlayerState::shared(cx).update(cx, move |player, cx| {
                            if player.listening_session.is_some() {
                                return;
                            }
                            player.last_listening_error = None;
                            rejoin_request.replay(false, player, cx);
                        });
                    })),
                    None,
                    cx,
                );
                return;
            }
            if self.is_listening_playback_displaced_by_ad_hoc()
                && self.listening_session.as_ref().is_some_and(|session| track.id == session.track_id)
            {
                self.restore_listening_session_after_ad_hoc_playback(cx);
                return;
            }
            let required_control = if previous_track_id.as_deref() == Some(track.id.as_str()) {
                MusicListeningBlockedControl::Playback
            } else {
                MusicListeningBlockedControl::ChangeTracks
            };
            if !self.allows_listening_control(required_control, true, cx) {
                return;
            }
        }
        if !self.take_over_external_chat_playback_for_user_control(!self.is_applying_listening_session, cx) {
            return;
        }
        if incoming_ad_hoc && self.listening_session.is_some() && !self.is_applying_listening_session {
            self.enforce_listening_session_for_ad_hoc_takeover(cx);
        }
        self.resolve_active_effect_settings(effective_scope_id.as_deref(), &track, cx);

        let candidates = MusicPlaybackUrl::playback_url_candidates_for(&track);
        let raw_audio_url = if track.audio_url.is_empty() { "(empty)" } else { track.audio_url.as_str() };
        MusicPlayerLog::info(
            format!(
                "PLAY tap id={} title=\"{}\" recordPlay={record_play_on_success} preserveQueue={preserve_queue} queueLen={} rawAudioUrl={raw_audio_url} candidates={}",
                track.id,
                track.title,
                new_queue.len(),
                candidates.len()
            ),
            cx,
        );
        for (index, url) in candidates.iter().enumerate() {
            MusicPlayerLog::info(format!("PLAY candidate[{index}] {}", url.absolute_string()), cx);
        }

        self.prepare_task = None;
        self.cancel_session_continuation();
        self.prepare_generation = self.prepare_generation.wrapping_add(1);
        let generation = self.prepare_generation;
        self.reset_playback_ui(&track, cx);
        if let Some(on_playback_accepted) = request.on_playback_accepted() {
            cx.defer(on_playback_accepted);
        }
        self.set_last_playback_error(None, cx);
        self.is_preparing_playback = true;
        self.record_play_on_success = record_play_on_success;
        self.playback_owner_user_id = SaiAccountCacheScope::active_user_id();
        self.playback_owner_account_epoch = SaiAccountCacheScope::active_epoch();
        let owner_user_id = self.playback_owner_user_id.clone();
        let authorization_token = self.playback_owner_access_token(cx);

        if !preserve_queue {
            let in_source_queue = self.source_queue.iter().any(|queued| queued.id == track.id);
            if let Some(scope_id) = scope_id {
                self.playback_scope_id = Some(scope_id);
            } else if !new_queue.is_empty() || !in_source_queue {
                self.playback_scope_id = None;
            }
            if !new_queue.is_empty() {
                self.set_playback_context(new_queue, &track, cx);
            } else if in_source_queue {
                self.rebuild_playback_queue(&track, cx);
            } else {
                self.set_playback_context(vec![track.clone()], &track, cx);
            }
        } else {
            self.queue_index =
                self.queue.iter().position(|queued| queued.id == track.id).unwrap_or(self.queue_index);
        }
        self.refresh_playlist_listening_presence(cx);
        let command = if previous_track_id.as_deref() == Some(track.id.as_str()) {
            MusicListeningSessionCommand::Playback
        } else {
            MusicListeningSessionCommand::ChangeTrack
        };
        self.publish_listening_session_state(command, 0, None, cx);
        if candidates.is_empty() {
            self.set_last_playback_error(Some("Invalid audio URL".to_owned()), cx);
            self.is_preparing_playback = false;
            self.clear_playback_download_progress(cx);
            MusicPlayerLog::fail("PLAY abort - no playable URL (check track.audioUrl from API)".to_owned(), cx);
            cx.notify();
            return;
        }

        self.prefetch_track_cover(&track, cx);
        self.prepare_task =
            Some(self.load_and_play(track.clone(), generation, owner_user_id, authorization_token, false, cx));
        MusicLocalState::shared(cx).update(cx, |state, cx| state.mark_played(&track, cx));
        cx.notify();
    }

    fn listening_interruption_decision(&self) -> MusicListeningInterruptionDecision {
        if !self.is_listening_together() || self.is_listening_playback_displaced_by_ad_hoc() {
            return MusicListeningInterruptionDecision::Proceed;
        }
        let Some(session) = self.listening_session.as_ref() else {
            return MusicListeningInterruptionDecision::Proceed;
        };
        MusicListeningInterruptionPolicy::decision(
            true,
            self.is_listening_session_host(),
            session.viewer_permissions.can_control_playback,
            session.is_playing,
            self.is_playing,
            self.listening_consent_episode_active,
        )
    }

    pub fn refresh_chat_track_metadata(&mut self, updated: TrackModel, cx: &mut Context<Self>) {
        if updated.source_chat_id.is_none() {
            return;
        }
        let matches = |existing: &TrackModel| -> bool {
            if existing.source_chat_id != updated.source_chat_id {
                return false;
            }
            if existing.source_message_id.is_some() || updated.source_message_id.is_some() {
                return existing.source_message_id == updated.source_message_id;
            }
            existing.title == updated.title && existing.artist == updated.artist
        };
        let previous_track = self.current_track.clone();
        let matches_current = previous_track.as_ref().is_some_and(matches);
        let matches_source_queue = self.source_queue.iter().any(matches);
        let matches_queue = self.queue.iter().any(matches);
        if !matches_current && !matches_source_queue && !matches_queue {
            return;
        }
        let previous_url = previous_track
            .as_ref()
            .map(|previous| trimming_whitespaces_and_newlines(&previous.stable_audio_url()).to_owned())
            .unwrap_or_default();
        let updated_url = trimming_whitespaces_and_newlines(&updated.stable_audio_url()).to_owned();
        let should_restart = Self::should_restart_after_chat_metadata_update(
            matches_current,
            &previous_url,
            &updated_url,
            self.is_playing,
            self.is_preparing_playback,
            self.last_playback_error.is_some(),
        );

        let current_metadata_changed = previous_track.as_ref().is_none_or(|previous| {
            previous.id != updated.id
                || previous.title != updated.title
                || previous.artist != updated.artist
                || previous.album_title != updated.album_title
                || previous.duration_seconds != updated.duration_seconds
                || previous.cover_image_url != updated.cover_image_url
                || previous.stable_audio_url() != updated.stable_audio_url()
        });
        if matches_current && current_metadata_changed {
            self.current_track = Some(updated.clone());
            self.update_now_playing_info(&updated, cx);
        }
        if matches_source_queue {
            let refreshed: Vec<TrackModel> = self
                .source_queue
                .iter()
                .map(|queued| if matches(queued) { updated.clone() } else { queued.clone() })
                .collect();
            if refreshed != self.source_queue {
                self.source_queue = refreshed;
            }
        }
        if matches_queue {
            let refreshed: Vec<TrackModel> = self
                .queue
                .iter()
                .map(|queued| if matches(queued) { updated.clone() } else { queued.clone() })
                .collect();
            if refreshed != self.queue {
                self.queue = refreshed;
            }
        }
        cx.notify();

        if should_restart && !self.is_audio_session_interrupted {
            MusicPlayerLog::info(format!("PLAY metadata URL changed - restarting id={}", updated.id), cx);
            self.play(
                updated,
                MusicPlayOptions { preserve_queue: true, confirm_call_interruption: false, ..MusicPlayOptions::default() },
                cx,
            );
        }
    }

    pub fn refresh_track_metadata(&mut self, updated: &TrackModel, cx: &mut Context<Self>) {
        let matches = |existing: &TrackModel| existing.id == updated.id;
        let matches_current = self.current_track.as_ref().is_some_and(matches);
        let matches_source_queue = self.source_queue.iter().any(matches);
        let matches_queue = self.queue.iter().any(matches);
        if !matches_current && !matches_source_queue && !matches_queue {
            return;
        }

        if matches_current {
            self.current_track = Some(updated.clone());
            self.update_now_playing_info(updated, cx);
        }
        if matches_source_queue {
            for queued in self.source_queue.iter_mut().filter(|queued| queued.id == updated.id) {
                *queued = updated.clone();
            }
        }
        if matches_queue {
            for queued in self.queue.iter_mut().filter(|queued| queued.id == updated.id) {
                *queued = updated.clone();
            }
        }
        cx.notify();
    }

    pub fn adopt_uploaded_track(&mut self, temp_id: &str, uploaded: &TrackModel, cx: &mut Context<Self>) {
        if temp_id.is_empty() || temp_id == uploaded.id {
            return;
        }
        let matches = |existing: &TrackModel| existing.id == temp_id;
        let matches_current = self.current_track.as_ref().is_some_and(matches);
        let matches_source_queue = self.source_queue.iter().any(matches);
        let matches_queue = self.queue.iter().any(matches);
        if !matches_current && !matches_source_queue && !matches_queue {
            return;
        }
        if matches_current {
            self.current_track = Some(uploaded.clone());
            self.update_now_playing_info(uploaded, cx);
        }
        for queued in self.source_queue.iter_mut().filter(|queued| queued.id == temp_id) {
            *queued = uploaded.clone();
        }
        for queued in self.queue.iter_mut().filter(|queued| queued.id == temp_id) {
            *queued = uploaded.clone();
        }
        cx.notify();
    }

    pub fn should_restart_after_chat_metadata_update(
        matches_current: bool,
        previous_url: &str,
        updated_url: &str,
        is_playing: bool,
        is_preparing: bool,
        has_playback_error: bool,
    ) -> bool {
        matches_current
            && !is_playing
            && previous_url != updated_url
            && !updated_url.is_empty()
            && (is_preparing || has_playback_error)
    }

    pub fn current_track_download_progress(&self) -> Option<MusicAudioDownloadProgress> {
        if !self.is_preparing_playback
            || self.playback_download_track_id.as_deref() != self.current_track.as_ref().map(|track| track.id.as_str())
        {
            return None;
        }
        self.playback_download_progress
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_playback_download_progress(
        &mut self,
        track_id: &str,
        generation: u64,
        owner_user_id: Option<&str>,
        owner_account_epoch: Option<&str>,
        candidate_id: Uuid,
        commit_lease: Arc<MusicPlaybackDownloadCommitLease>,
        cx: &mut Context<Self>,
    ) {
        if self.prepare_generation != generation
            || self.current_track.as_ref().map(|track| track.id.as_str()) != Some(track_id)
            || self.playback_owner_user_id.as_deref() != owner_user_id
            || self.playback_owner_account_epoch.as_deref() != owner_account_epoch
            || !SaiAccountCacheScope::is_current_account_epoch(owner_user_id, owner_account_epoch)
        {
            commit_lease.invalidate();
            return;
        }
        if let Some(previous) = self.playback_download_commit_lease.take() {
            previous.invalidate();
        }
        self.playback_download_track_id = Some(track_id.to_owned());
        self.playback_download_candidate_id = Some(candidate_id);
        self.playback_download_retry_attempt = 0;
        self.playback_download_commit_lease = Some(commit_lease);
        self.playback_download_progress = Some(MusicAudioDownloadProgress::new(0, 0));
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_playback_download_progress(
        &mut self,
        progress: MusicAudioDownloadProgress,
        track_id: &str,
        generation: u64,
        owner_user_id: Option<&str>,
        owner_account_epoch: Option<&str>,
        candidate_id: Uuid,
        retry_attempt: i64,
        cx: &mut Context<Self>,
    ) {
        if !self.is_preparing_playback
            || self.prepare_generation != generation
            || self.current_track.as_ref().map(|track| track.id.as_str()) != Some(track_id)
            || self.playback_owner_user_id.as_deref() != owner_user_id
            || self.playback_owner_account_epoch.as_deref() != owner_account_epoch
            || self.playback_download_candidate_id != Some(candidate_id)
            || !SaiAccountCacheScope::is_current_account_epoch(owner_user_id, owner_account_epoch)
        {
            return;
        }
        if retry_attempt < self.playback_download_retry_attempt {
            return;
        }
        if retry_attempt == self.playback_download_retry_attempt
            && let Some(current) = self.playback_download_progress
            && progress.downloaded_bytes() < current.downloaded_bytes()
        {
            return;
        }
        self.playback_download_retry_attempt = retry_attempt;
        self.playback_download_track_id = Some(track_id.to_owned());
        self.playback_download_progress = Some(progress);
        cx.notify();
    }

    fn clear_playback_download_progress(&mut self, cx: &mut Context<Self>) {
        if let Some(lease) = self.playback_download_commit_lease.take() {
            lease.invalidate();
        }
        self.playback_download_track_id = None;
        self.playback_download_progress = None;
        self.playback_download_candidate_id = None;
        self.playback_download_retry_attempt = 0;
        cx.notify();
    }

    fn reset_playback_ui(&mut self, track: &TrackModel, cx: &mut Context<Self>) {
        self.cancel_now_playing_artwork();
        self.stop_file_player(cx);
        self.release_av_player(cx);
        self.set_is_playing(false, cx);
        self.progress = 0.0;
        self.clear_playback_download_progress(cx);
        self.current_time = 0.0;
        self.duration = if track.duration_seconds > 0 { track.duration_seconds as f64 } else { 0.0 };
        self.current_track = Some(track.clone());
        cx.notify();
    }

    fn set_playback_context(&mut self, incoming: Vec<TrackModel>, starting_track: &TrackModel, cx: &mut Context<Self>) {
        let tracks = if incoming.is_empty() { vec![starting_track.clone()] } else { incoming };
        let same_ids = tracks.len() == self.source_queue.len()
            && tracks.iter().zip(&self.source_queue).all(|(incoming, queued)| incoming.id == queued.id);
        if !same_ids {
            self.source_queue = tracks;
        }
        self.rebuild_playback_queue(starting_track, cx);
    }

    /// shuffle can be on before a session starts and seed 0 lets every device pick its own order
    /// mint one lazily so the room shares a permutation
    fn publishable_shuffle_seed(&mut self, cx: &App) -> i64 {
        if !self.is_shuffled(cx) {
            return 0;
        }
        if self.shuffle_seed() == 0 {
            self.set_shuffle_seed(Self::random_shuffle_seed());
        }
        self.shuffle_seed()
    }

    /// random shuffle seed between 1 and i32 max
    fn random_shuffle_seed() -> i64 {
        rand::random_range(1..=i64::from(i32::MAX))
    }

    pub fn collection_start_track(
        &mut self,
        tracks: &[TrackModel],
        scope_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<TrackModel> {
        let first = tracks.first()?;
        let ad_hoc = Self::is_ad_hoc_playback_context(scope_id, first);
        if tracks.len() <= 1 || !self.is_shuffled_ad_hoc(ad_hoc, cx) {
            return Some(first.clone());
        }

        if self.listening_session.is_none() || ad_hoc {
            self.set_shuffle_seed_ad_hoc(Self::random_shuffle_seed(), ad_hoc);
            cx.notify();
        }
        Some(Self::shuffled_head(tracks, self.shuffle_seed_ad_hoc(ad_hoc)).unwrap_or_else(|| first.clone()))
    }

    pub fn shuffled_head(tracks: &[TrackModel], seed: i64) -> Option<TrackModel> {
        if seed == 0 {
            return tracks.choose(&mut rand::rng()).cloned();
        }
        let mut head: Option<(u64, &TrackModel)> = None;
        for track in tracks {
            let hash = Self::stable_hash(seed, &track.id);
            let precedes = match head {
                Some((current_hash, current)) => {
                    hash < current_hash || (hash == current_hash && track.id < current.id)
                }
                None => true,
            };
            if precedes {
                head = Some((hash, track));
            }
        }
        head.map(|(_, track)| track.clone())
    }

    /// deterministic shuffle, same seed and tracks give the same order on every device
    /// nothing goes on the wire, seed 0 means plain random shuffle
    pub fn shuffled(tracks: &[TrackModel], seed: i64) -> Vec<TrackModel> {
        if seed == 0 {
            let mut shuffled = tracks.to_vec();
            shuffled.shuffle(&mut rand::rng());
            return shuffled;
        }

        let mut ranked: Vec<(u64, &TrackModel)> =
            tracks.iter().map(|track| (Self::stable_hash(seed, &track.id), track)).collect();
        ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.id.cmp(&right.1.id)));
        ranked.into_iter().map(|(_, track)| track.clone()).collect()
    }

    /// fnv-1a hash
    /// a per process seeded hasher would give a different order every launch and device
    fn stable_hash(seed: i64, value: &str) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let prime: u64 = 0x0000_0100_0000_01B3;
        let mut mix = u64::from_ne_bytes(seed.to_ne_bytes());
        for _ in 0..8 {
            hash = (hash ^ (mix & 0xFF)).wrapping_mul(prime);
            mix >>= 8;
        }
        for byte in value.bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(prime);
        }
        hash
    }

    fn rebuild_playback_queue(&mut self, anchor: &TrackModel, cx: &mut Context<Self>) {
        if self.source_queue.is_empty() {
            self.source_queue = vec![anchor.clone()];
        }
        let is_shuffled = self.is_shuffled(cx);
        if is_shuffled {
            let rest: Vec<TrackModel> =
                self.source_queue.iter().filter(|queued| queued.id != anchor.id).cloned().collect();
            let seed = self.publishable_shuffle_seed(cx);
            let mut queue = vec![anchor.clone()];
            queue.extend(Self::shuffled(&rest, seed));
            self.queue = queue;
        } else {
            self.queue = self.source_queue.clone();
        }
        self.queue_index = self.queue.iter().position(|queued| queued.id == anchor.id).unwrap_or(0);
        MusicPlayerLog::info(
            format!(
                "QUEUE rebuilt shuffle={is_shuffled} count={} index={} anchor={}",
                self.queue.len(),
                self.queue_index,
                anchor.id
            ),
            cx,
        );
        cx.notify();
    }

    pub fn insert_next(&mut self, track: TrackModel, cx: &mut Context<Self>) -> bool {
        if !self.allows_listening_control(MusicListeningBlockedControl::ChangeTracks, false, cx)
            || self.current_track.as_ref().is_some_and(|current| current.id == track.id)
        {
            return false;
        }
        self.detach_from_queue(&track.id);
        let insert_at = (self.queue_index + 1).min(self.queue.len());
        self.queue.insert(insert_at, track.clone());
        let source_at = self
            .current_track
            .as_ref()
            .and_then(|current| self.source_queue.iter().position(|queued| queued.id == current.id))
            .map(|index| index + 1)
            .unwrap_or_else(|| insert_at.min(self.source_queue.len()));
        MusicPlayerLog::info(format!("QUEUE insertNext id={} at={insert_at}", track.id), cx);
        self.source_queue.insert(source_at, track);
        cx.notify();
        true
    }

    pub fn append_to_queue(&mut self, track: TrackModel, cx: &mut Context<Self>) -> bool {
        if !self.allows_listening_control(MusicListeningBlockedControl::ChangeTracks, false, cx)
            || self.current_track.as_ref().is_some_and(|current| current.id == track.id)
        {
            return false;
        }
        self.detach_from_queue(&track.id);
        self.queue.push(track.clone());
        MusicPlayerLog::info(format!("QUEUE append id={} total={}", track.id, self.queue.len()), cx);
        self.source_queue.push(track);
        cx.notify();
        true
    }

    /// swaps an ad-hoc chat queue for a fuller list of the same chat while the current track plays
    /// tracks must contain it, with shuffle on the dealt order stays
    pub fn replace_ad_hoc_queue(&mut self, scope_id: &str, tracks: Vec<TrackModel>, cx: &mut Context<Self>) -> bool {
        if self.listening_session.is_some() || self.playback_scope_id.as_deref() != Some(scope_id) {
            return false;
        }
        let Some(current) = self.current_track.clone() else {
            return false;
        };
        if !tracks.iter().any(|track| track.id == current.id) {
            return false;
        }
        let same_ids = tracks.len() == self.source_queue.len()
            && tracks.iter().zip(&self.source_queue).all(|(track, queued)| track.id == queued.id);
        if same_ids {
            return false;
        }

        if self.is_shuffled(cx) {
            let kept: HashSet<&str> = tracks.iter().map(|track| track.id.as_str()).collect();
            let dealt: Vec<TrackModel> =
                self.queue.iter().filter(|queued| kept.contains(queued.id.as_str())).cloned().collect();
            let dealt_ids: HashSet<&str> = dealt.iter().map(|track| track.id.as_str()).collect();
            let added: Vec<TrackModel> =
                tracks.iter().filter(|track| !dealt_ids.contains(track.id.as_str())).cloned().collect();
            self.source_queue = tracks;
            let seed = self.publishable_shuffle_seed(cx);
            let mut queue = dealt;
            queue.extend(Self::shuffled(&added, seed));
            self.queue = queue;
            self.queue_index = self.queue.iter().position(|queued| queued.id == current.id).unwrap_or(0);
        } else {
            self.source_queue = tracks;
            self.rebuild_playback_queue(&current, cx);
        }
        MusicPlayerLog::info(
            format!("QUEUE ad-hoc grown scope={scope_id} count={} index={}", self.queue.len(), self.queue_index),
            cx,
        );
        cx.notify();
        true
    }

    fn detach_from_queue(&mut self, track_id: &str) {
        if let Some(index) = self.queue.iter().position(|queued| queued.id == track_id) {
            self.queue.remove(index);
            if index < self.queue_index {
                self.queue_index -= 1;
            }
        }
        self.source_queue.retain(|queued| queued.id != track_id);
    }

    pub fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        if !self.allows_listening_control(MusicListeningBlockedControl::ChangeTracks, false, cx) {
            return;
        }

        let next = !self.is_shuffled(cx);
        let scope = self.playback_preference_scope();
        MusicPlaybackPreferences::shared(cx).update(cx, |preferences, cx| preferences.set_shuffled(next, scope, cx));
        if self.listening_session.is_some() && !self.is_listening_playback_displaced_by_ad_hoc() {
            self.listening_shuffle_enabled = Some(next);
        }
        self.set_shuffle_seed(if next { Self::random_shuffle_seed() } else { 0 });
        if let Some(track) = self.current_track.clone() {
            self.rebuild_playback_queue(&track, cx);
        } else if let Some(first) = self.queue.first().cloned() {
            self.rebuild_playback_queue(&first, cx);
        }
        self.publish_listening_session_state(MusicListeningSessionCommand::ChangeTrack, 0, None, cx);
        cx.notify();
    }

    pub fn cycle_repeat_mode(&mut self, cx: &mut Context<Self>) {
        if !self.allows_listening_control(MusicListeningBlockedControl::ChangeTracks, false, cx) {
            return;
        }

        let next = self.repeat_mode(cx).next();
        let scope = self.playback_preference_scope();
        MusicPlaybackPreferences::shared(cx)
            .update(cx, |preferences, cx| preferences.set_repeat_mode(next, scope, cx));
        if self.listening_session.is_some() && !self.is_listening_playback_displaced_by_ad_hoc() {
            self.listening_repeat_mode = Some(next);
        }
        self.publish_listening_session_state(MusicListeningSessionCommand::ChangeTrack, 0, None, cx);
        cx.notify();
    }

    pub fn play_queue_item(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.allows_listening_control(MusicListeningBlockedControl::ChangeTracks, false, cx) {
            return;
        }
        let Some(track) = self.queue.get(index).cloned() else {
            return;
        };
        self.play(track, MusicPlayOptions { preserve_queue: true, ..MusicPlayOptions::default() }, cx);
    }

    fn prefetch_upcoming_tracks(&mut self, cx: &mut Context<Self>) {
        self.prefetch_task = None;
        let owner_user_id = self.playback_owner_user_id.clone();

        let mut indices: Vec<usize> = Vec::new();
        if self.queue.len() > 1 {
            let next = self.queue_index + 1;
            if next < self.queue.len() {
                indices.push(next);
                if next + 1 < self.queue.len() {
                    indices.push(next + 1);
                }
            } else if self.repeat_mode(cx) == MusicRepeatMode::All && !self.queue.is_empty() {
                indices.push(0);
            }
        }

        let mut wanted: HashSet<String> = indices
            .iter()
            .map(|&index| {
                MusicPlaybackPrefetchRegistry::key(&self.queue[index].playback_cache_key(), owner_user_id.as_deref())
            })
            .collect();
        if let Some(current) = self.current_track.as_ref() {
            wanted.insert(MusicPlaybackPrefetchRegistry::key(
                &current.playback_cache_key(),
                owner_user_id.as_deref(),
            ));
        }
        MusicPlaybackPrefetchRegistry::cancel_jobs(&wanted);
        if indices.is_empty() {
            return;
        }

        let owner_account_epoch = self.playback_owner_account_epoch.clone();
        let authorization_token = self.playback_owner_access_token(cx);

        let upcoming: Vec<TrackModel> = indices.iter().map(|&index| self.queue[index].clone()).collect();
        let generation = self.prepare_generation;
        self.prefetch_task = Some(cx.spawn(async move |this, cx| {
            for track in upcoming {
                let Ok(current_generation) = this.read_with(cx, |this, _| this.prepare_generation) else {
                    return;
                };
                if current_generation != generation {
                    return;
                }
                let Some(remote) =
                    MusicPlaybackUrl::playback_url_candidates_for(&track).into_iter().find(|url| !url.is_file_url())
                else {
                    continue;
                };
                let cache_key = track.playback_cache_key();
                let key = MusicPlaybackPrefetchRegistry::key(&cache_key, owner_user_id.as_deref());
                if MusicPlaybackPrefetchRegistry::job(&key).is_some() {
                    continue;
                }
                let operation = Self::download_audio_for_playback(
                    remote,
                    cache_key,
                    false,
                    0,
                    owner_user_id.clone(),
                    owner_account_epoch.clone(),
                    Some(authorization_token.clone()),
                    None,
                    None,
                    cx.clone(),
                );
                let job = cx.update(|cx| {
                    MusicPlayerLog::info(format!("PREFETCH next id={}", track.id), cx);
                    MusicPlaybackPrefetchRegistry::start(key, operation, cx)
                });
                job.value().await;
            }
        }));
    }

    fn notify_playback_started(&mut self, track: &TrackModel, cx: &mut Context<Self>) {
        if !self.record_play_on_success {
            return;
        }
        self.record_play_on_success = false;
        if !SaiAccountCacheScope::is_current_account(self.playback_owner_user_id.as_deref()) {
            return;
        }
        let track_id = track.id.clone();
        cx.defer(move |cx| {
            MusicActionsStore::shared(cx).update(cx, |store, cx| store.record_play_action(&track_id, None, cx));
        });
    }

    fn playback_owner_access_token(&self, cx: &App) -> String {
        let Some(owner_user_id) = self.playback_owner_user_id.as_deref() else {
            return String::new();
        };
        if SaiAccountCacheScope::is_current_account(Some(owner_user_id)) {
            return AuthServicesStore::try_read(cx)
                .map(|store| store.tokens_and_other_data().access_token.clone())
                .unwrap_or_default();
        }
        AuthLocalStorage::load_account(owner_user_id).map(|account| account.tokens.access_token).unwrap_or_default()
    }

    fn request_playback_after_leaving_call(
        &mut self,
        retry: Box<dyn FnOnce(&mut MusicPlayerState, &mut Context<MusicPlayerState>)>,
        cx: &mut Context<Self>,
    ) -> bool {
        if RiseCallAudioOwnership::is_call_active() {
            drop(retry);
            self.join_call_with_music(cx);
            return true;
        }
        let generation = self.call_playback_request_generation;
        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let retry = Rc::new(RefCell::new(Some(retry)));
        let make_resume = Rc::new(move || -> Box<dyn FnOnce(&mut MusicPlayerState, &mut Context<MusicPlayerState>)> {
            let retry = retry.clone();
            let account_id = account_id.clone();
            let account_epoch = account_epoch.clone();
            Box::new(move |this: &mut MusicPlayerState, cx: &mut Context<MusicPlayerState>| {
                if this.call_playback_request_generation != generation
                    || !SaiAccountCacheScope::is_current_account_epoch(account_id.as_deref(), account_epoch.as_deref())
                {
                    return;
                }
                let retry = retry.borrow_mut().take();
                if let Some(retry) = retry {
                    retry(this, cx);
                }
            })
        });
        let player = cx.weak_entity();
        let hook_resume = make_resume.clone();
        let allowed = (music_player_rtc_hooks().request_music_playback)(
            Box::new(move |cx: &mut App| {
                cx.defer(move |cx| {
                    let _ = player.update(cx, |this, cx| {
                        let resume = hook_resume();
                        if this.external_audio_owners.contains(&MusicExternalAudioOwner::RTC) {
                            this.pending_playback_after_call = Some(resume);
                        } else {
                            resume(this, cx);
                        }
                    });
                });
            }),
            cx,
        );
        if !allowed {
            return false;
        }
        if self.external_audio_owners.contains(&MusicExternalAudioOwner::RTC) {
            self.pending_playback_after_call = Some(make_resume());
            return false;
        }
        true
    }

    fn load_and_play(
        &mut self,
        track: TrackModel,
        generation: u64,
        owner_user_id: Option<String>,
        authorization_token: String,
        force_refresh_download: bool,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let owner_account_epoch = self.playback_owner_account_epoch.clone();
        cx.spawn(async move |this, cx| {
            Self::load_and_play_body(
                this.clone(),
                &track,
                generation,
                owner_user_id,
                owner_account_epoch,
                authorization_token,
                force_refresh_download,
                cx,
            )
            .await;
            let _ = this.update(cx, |this, cx| {
                if this.prepare_generation != generation {
                    return;
                }
                this.is_preparing_playback = false;
                this.clear_playback_download_progress(cx);
                MusicPlayerLog::info(
                    format!(
                        "PLAY prepare finished id={} isPlaying={} error={}",
                        track.id,
                        this.is_playing,
                        this.last_playback_error.as_deref().unwrap_or("none")
                    ),
                    cx,
                );
                cx.notify();
                this.prepare_task = None;
            });
        })
    }

    fn is_preparation_current(this: &WeakEntity<Self>, generation: u64, cx: &AsyncApp) -> bool {
        this.read_with(cx, |this, _| this.prepare_generation == generation).unwrap_or(false)
    }

    #[allow(clippy::too_many_arguments)]
    async fn load_and_play_body(
        this: WeakEntity<Self>,
        track: &TrackModel,
        generation: u64,
        owner_user_id: Option<String>,
        owner_account_epoch: Option<String>,
        authorization_token: String,
        force_refresh_download: bool,
        cx: &mut AsyncApp,
    ) {
        if !Self::is_preparation_current(&this, generation, cx) {
            cx.update(|cx| MusicPlayerLog::info(format!("PLAY cancelled before load id={}", track.id), cx));
            return;
        }
        let candidates = MusicPlaybackUrl::playback_url_candidates_for(track);
        if candidates.is_empty() {
            let _ = this.update(cx, |this, cx| {
                this.set_last_playback_error(Some("Invalid audio URL".to_owned()), cx);
                MusicPlayerLog::fail("PLAY load abort - no URL candidates".to_owned(), cx);
            });
            return;
        }

        if force_refresh_download {
            let cache_key = track.playback_cache_key();
            let user_id = owner_user_id.clone();
            cx.background_executor()
                .spawn(async move { Self::clear_disk_playback_cache(&cache_key, user_id.as_deref()) })
                .await;
            cx.update(|cx| MusicPlayerLog::info(format!("PLAY force re-download id={}", track.id), cx));
        }

        let started = this.update(cx, |this, cx| {
            MusicPlayerLog::info(
                format!("PLAY load start id={} trying {} URL(s)", track.id, candidates.len()),
                cx,
            );
            this.prefetch_track_cover(track, cx);
            this.last_download_error = None;
        });
        if started.is_err() {
            return;
        }
        let mut play_url: Option<Url> = None;
        let mut last_error: Option<String> = None;
        let mut joined_prefetch = false;
        for (index, remote_url) in candidates.iter().enumerate() {
            if remote_url.is_file_url() {
                play_url = Some(remote_url.clone());
                last_error = None;
                let _ = this.update(cx, |this, cx| {
                    this.adopt_local_playback_file_into_cache(track, remote_url, owner_user_id.as_deref(), cx);
                });
                break;
            }
            if !force_refresh_download
                && !joined_prefetch
                && let Some(prefetching) = MusicPlaybackPrefetchRegistry::job(&MusicPlaybackPrefetchRegistry::key(
                    &track.playback_cache_key(),
                    owner_user_id.as_deref(),
                ))
            {
                joined_prefetch = true;
                cx.update(|cx| MusicPlayerLog::info(format!("PLAY joining in-flight prefetch id={}", track.id), cx));
                let (joined_url, _) = prefetching.value().await;
                if !Self::is_preparation_current(&this, generation, cx) {
                    cx.update(|cx| {
                        MusicPlayerLog::info(format!("PLAY cancelled while joining prefetch id={}", track.id), cx);
                    });
                    return;
                }
                if let Some(joined_url) = joined_url.and_then(|path| Url::from_file_path(&path)) {
                    play_url = Some(joined_url);
                    last_error = None;
                    break;
                }
            }
            let candidate_id = Uuid::new_v4();
            let commit_lease = MusicPlaybackDownloadCommitLease::new();
            let began = this.update(cx, |this, cx| {
                this.begin_playback_download_progress(
                    &track.id,
                    generation,
                    owner_user_id.as_deref(),
                    owner_account_epoch.as_deref(),
                    candidate_id,
                    commit_lease.clone(),
                    cx,
                );
                MusicPlayerLog::info(format!("PLAY try download [{}/{}]", index + 1, candidates.len()), cx);
            });
            if began.is_err() {
                return;
            }
            let (progress_sender, mut progress_inbox) = mpsc::unbounded::<(MusicAudioDownloadProgress, i64)>();
            let on_progress: MusicPlaybackDownloadProgressHandler = Arc::new(move |progress, retry_attempt| {
                let _ = progress_sender.unbounded_send((progress, retry_attempt));
            });
            let progress_task = cx.spawn({
                let this = this.clone();
                let track_id = track.id.clone();
                let owner_user_id = owner_user_id.clone();
                let owner_account_epoch = owner_account_epoch.clone();
                async move |cx| {
                    while let Some((progress, retry_attempt)) = progress_inbox.next().await {
                        let applied = this.update(cx, |this, cx| {
                            this.apply_playback_download_progress(
                                progress,
                                &track_id,
                                generation,
                                owner_user_id.as_deref(),
                                owner_account_epoch.as_deref(),
                                candidate_id,
                                retry_attempt,
                                cx,
                            );
                        });
                        if applied.is_err() {
                            return;
                        }
                    }
                }
            });
            let (downloaded, error) = Self::download_audio_for_playback(
                remote_url.clone(),
                track.playback_cache_key(),
                force_refresh_download,
                0,
                owner_user_id.clone(),
                owner_account_epoch.clone(),
                Some(authorization_token.clone()),
                Some(commit_lease),
                Some(on_progress),
                cx.clone(),
            )
            .await;
            drop(progress_task);
            if !Self::is_preparation_current(&this, generation, cx) {
                cx.update(|cx| MusicPlayerLog::info(format!("PLAY cancelled during download id={}", track.id), cx));
                return;
            }
            if let Some(local) = downloaded.and_then(|path| Url::from_file_path(&path)) {
                play_url = Some(local);
                last_error = None;
                break;
            }
            cx.update(|cx| {
                MusicPlayerLog::fail(
                    format!("PLAY candidate [{}] failed: {}", index + 1, error.as_deref().unwrap_or("?")),
                    cx,
                );
            });
            last_error = error;
        }
        let Some(play_url) = play_url else {
            let _ = this.update(cx, |this, cx| {
                let error = last_error.unwrap_or_else(|| "Audio download failed".to_owned());
                MusicPlayerLog::fail(format!("PLAY all downloads failed id={} lastError={error}", track.id), cx);
                this.set_last_playback_error(Some(error), cx);
            });
            return;
        };

        if !Self::is_preparation_current(&this, generation, cx) {
            cx.update(|cx| MusicPlayerLog::info(format!("PLAY cancelled after download id={}", track.id), cx));
            return;
        }
        let remote_fallback =
            candidates.iter().find(|candidate| !candidate.is_file_url()).map(|candidate| candidate.absolute_string().to_owned());
        let start = this.update(cx, |this, cx| {
            MusicPlayerLog::ok(
                format!(
                    "PLAY starting engine file={} path={}",
                    play_url.is_file_url(),
                    play_url.last_path_component()
                ),
                cx,
            );
            this.start_player_item(play_url, track.clone(), remote_fallback, !force_refresh_download, cx)
        });
        if let Ok(start) = start {
            start.await;
        }
    }

    /// starts a player item with optional remote fallback and cache recovery retry
    fn start_player_item(
        &mut self,
        url: Url,
        track: TrackModel,
        remote_fallback: Option<String>,
        allow_cache_recovery_retry: bool,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        if self.current_track.as_ref().map(|current| current.id.as_str()) != Some(track.id.as_str()) {
            return Task::ready(());
        }
        let generation = self.prepare_generation;
        let session = self.configure_session(false, cx);
        cx.spawn(async move |this, cx| {
            if !session.await {
                return;
            }
            let local_engine = this.update(cx, |this, cx| {
                if !this.can_complete_audio_preparation(generation) {
                    return None;
                }
                this.remove_item_status_observer();
                this.remove_end_observer();
                this.remove_time_observer(cx);
                this.stop_file_player(cx);
                this.release_av_player(cx);
                Some(url.to_file_path().map(|path| this.start_local_engine(path, track.clone(), cx)))
            });
            let Ok(Some(local_engine)) = local_engine else {
                return;
            };

            let Some(local_engine) = local_engine else {
                let av_player = this.update(cx, |this, cx| {
                    this.start_av_player_item(url, track, false, None, false, cx)
                });
                if let Ok(av_player) = av_player {
                    av_player.await;
                }
                return;
            };
            let started = local_engine.await;
            let fallback = this.update(cx, |this, cx| {
                if started {
                    this.apply_playback_start_fade(cx);
                    return None;
                }
                if !this.can_complete_audio_preparation(generation) {
                    return None;
                }
                MusicPlayerLog::info("ENGINE ROMusicFilePlayer unavailable -> AVPlayer (local file)".to_owned(), cx);
                Some(this.start_av_player_item(url, track, true, remote_fallback, allow_cache_recovery_retry, cx))
            });
            if let Ok(Some(fallback)) = fallback {
                fallback.await;
            }
        })
    }

    fn adopt_local_playback_file_into_cache(
        &self,
        track: &TrackModel,
        local_url: &Url,
        owner_user_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if track.transfer_phase.is_some() {
            return;
        }
        let track_id = trimming_whitespaces_and_newlines(&track.playback_cache_key()).to_owned();
        if track_id.is_empty() || Self::is_temporary_playback_cache_key(&track_id) {
            return;
        }
        let Some(source_path) = local_url.to_file_path() else {
            return;
        };
        let owner_user_id = owner_user_id.map(str::to_owned);
        cx.background_spawn(async move {
            MusicPlaybackDiskCache::adopt_external_file(&track_id, &source_path, None, owner_user_id.as_deref());
        })
        .detach();
    }

    fn is_temporary_playback_cache_key(key: &str) -> bool {
        key.contains(RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// starts the av player item for a url with optional remote fallback and cache retry
    fn start_av_player_item(
        &mut self,
        url: Url,
        track: TrackModel,
        invalidate_temp_cache_on_failure: bool,
        remote_fallback: Option<String>,
        allow_cache_recovery_retry: bool,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        if self.current_track.as_ref().map(|current| current.id.as_str()) != Some(track.id.as_str()) {
            return Task::ready(());
        }
        let origin = if url.is_file_url() { "(local)" } else { "(remote)" };
        MusicPlayerLog::info(format!("ENGINE AVPlayer {origin} {}", url.absolute_string()), cx);
        let generation = self.prepare_generation;
        let item = Self::make_player_item(&url);
        if !self.can_complete_audio_preparation(generation) {
            return Task::ready(());
        }
        let player = match self.player.clone() {
            Some(player) => {
                player.update(cx, |player, cx| player.replace_current_item(Some(item.clone()), cx));
                player
            }
            None => {
                let first_item = item.clone();
                let player = cx.new(|cx| {
                    let mut player = AVPlayer::new(cx);
                    player.replace_current_item(Some(first_item), cx);
                    player
                });
                self.player = Some(player.clone());
                player
            }
        };
        player.update(cx, |player, cx| player.set_automatically_waits_to_minimize_stalling(false, cx));

        self.remove_time_observer(cx);
        self.add_time_observer(cx);
        self.set_is_playing(false, cx);

        let on_status = {
            let item = item.clone();
            let url = url.clone();
            let track = track.clone();
            move |cx: &mut Context<Self>| {
                let item = item.clone();
                let url = url.clone();
                let track = track.clone();
                let remote_fallback = remote_fallback.clone();
                cx.spawn(async move |this, cx| {
                    let _ = this.update(cx, |this, cx| {
                        let Some(player) = this.player.clone() else {
                            return;
                        };
                        if player.read(cx).current_item() != Some(&item)
                            || this.current_track.as_ref().map(|current| current.id.as_str()) != Some(track.id.as_str())
                        {
                            return;
                        }
                        match player.read(cx).current_item_status() {
                            AVPlayerItemStatus::ReadyToPlay => {
                                if this.is_audio_session_interrupted {
                                    return;
                                }
                                this.set_last_playback_error(None, cx);
                                let session = this.configure_session(false, cx);
                                cx.spawn(async move |this, cx| {
                                    if !session.await {
                                        return;
                                    }
                                    let _ = this.update(cx, |this, cx| {
                                        let Some(player) = this.player.clone() else {
                                            return;
                                        };
                                        if !this.can_complete_audio_preparation(generation)
                                            || player.read(cx).current_item() != Some(&item)
                                        {
                                            return;
                                        }
                                        let rate = this.remote_playback_rate();
                                        player.update(cx, |player, cx| player.play_immediately(rate, cx));
                                        this.set_is_playing(true, cx);
                                        this.apply_playback_start_fade(cx);
                                        MusicPlayerLog::ok(format!("AVPlayer readyToPlay id={}", track.id), cx);
                                        this.update_now_playing_info(&track, cx);
                                        this.notify_playback_started(&track, cx);
                                        this.prefetch_upcoming_tracks(cx);
                                        this.refresh_playlist_listening_presence(cx);
                                        if !this.suppress_next_listening_playback_publish.get() {
                                            this.publish_listening_session_state(
                                                MusicListeningSessionCommand::Playback,
                                                0,
                                                None,
                                                cx,
                                            );
                                        }
                                    });
                                })
                                .detach();
                            }
                            AVPlayerItemStatus::Failed => {
                                this.set_is_playing(false, cx);
                                let error = player.read(cx).error().cloned();
                                Self::log_av_player_failure(error.as_ref(), &track.id, cx);
                                let message = match &this.last_download_error {
                                    Some(download_error) => download_error.clone(),
                                    None => error.map(|error| error.to_string()).unwrap_or_else(|| "Playback failed".to_owned()),
                                };
                                this.set_last_playback_error(Some(message.clone()), cx);
                                MusicPlayerLog::fail(format!("AVPlayer failed id={}: {message}", track.id), cx);
                                let remote_url = remote_fallback
                                    .as_deref()
                                    .and_then(|remote| MusicPlaybackUrl::playback_url_candidates_from(remote).into_iter().next());
                                if url.is_file_url()
                                    && invalidate_temp_cache_on_failure
                                    && allow_cache_recovery_retry
                                    && let Some(remote_url) = remote_url
                                {
                                    this.invalidate_disk_playback_cache(&url, cx);
                                    MusicPlayerLog::info(format!("PLAY fallback -> re-download remote id={}", track.id), cx);
                                    this.restart_preparation_from_remote(track.clone(), remote_url, cx);
                                } else if invalidate_temp_cache_on_failure {
                                    this.invalidate_disk_playback_cache(&url, cx);
                                }
                            }
                            AVPlayerItemStatus::Unknown => {
                                MusicPlayerLog::info(format!("AVPlayer status=unknown id={}", track.id), cx);
                            }
                        }
                    });
                })
                .detach();
            }
        };
        let initial_status = player.read(cx).current_item_status();
        let mut last_status = initial_status;
        on_status(cx);
        self.item_status_observer = Some(cx.observe(&player, move |_, player, cx| {
            let status = player.read(cx).current_item_status();
            if status == last_status {
                return;
            }
            last_status = status;
            on_status(cx);
        }));

        let end_item = item;
        let end_track_id = track.id;
        self.end_observer_token = Some(cx.subscribe(&player, move |_, _, event: &AVPlayerItemNotification, cx| {
            let AVPlayerItemNotification::DidPlayToEndTime { asset_url } = event;
            if asset_url != end_item.url() {
                return;
            }
            let end_item = end_item.clone();
            let track_id = end_track_id.clone();
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |this, cx| {
                    let Some(player) = this.player.clone() else {
                        return;
                    };
                    if player.read(cx).current_item() != Some(&end_item)
                        || this.current_track.as_ref().map(|current| current.id.as_str()) != Some(track_id.as_str())
                    {
                        return;
                    }
                    this.handle_track_end(cx);
                });
            })
            .detach();
        }));
        Task::ready(())
    }

    fn restart_preparation_from_remote(&mut self, track: TrackModel, remote: Url, cx: &mut Context<Self>) {
        self.prepare_task = None;
        self.cancel_session_continuation();
        self.prepare_generation = self.prepare_generation.wrapping_add(1);
        let generation = self.prepare_generation;
        self.is_preparing_playback = true;
        cx.notify();
        let owner_user_id = self.playback_owner_user_id.clone();
        let owner_account_epoch = self.playback_owner_account_epoch.clone();
        let authorization_token = self.playback_owner_access_token(cx);
        self.prepare_task = Some(cx.spawn(async move |this, cx| {
            let (downloaded, error) = Self::download_audio_for_playback(
                remote,
                track.playback_cache_key(),
                true,
                0,
                owner_user_id,
                owner_account_epoch,
                Some(authorization_token),
                None,
                None,
                cx.clone(),
            )
            .await;
            let start = this.update(cx, |this, cx| {
                if this.prepare_generation != generation
                    || this.current_track.as_ref().map(|current| current.id.as_str()) != Some(track.id.as_str())
                {
                    return None;
                }
                let Some(recovered) = downloaded.and_then(|path| Url::from_file_path(&path)) else {
                    this.set_last_playback_error(Some(error.unwrap_or_else(|| "Audio download failed".to_owned())), cx);
                    MusicPlayerLog::fail(format!("PLAY recovery download failed id={}", track.id), cx);
                    return None;
                };
                Some(this.start_player_item(recovered, track.clone(), None, false, cx))
            });
            if let Ok(Some(start)) = start {
                start.await;
            }
            let _ = this.update(cx, |this, cx| {
                if this.prepare_generation != generation {
                    return;
                }
                this.is_preparing_playback = false;
                this.clear_playback_download_progress(cx);
                cx.notify();
                this.prepare_task = None;
            });
        }));
    }

    fn invalidate_disk_playback_cache(&self, url: &Url, cx: &mut Context<Self>) {
        let Some(path) = url.to_file_path() else {
            return;
        };
        let name = url.last_path_component();
        if !name.starts_with("music-") {
            return;
        }
        cx.spawn(async move |_, cx| {
            let decodable = cx
                .background_executor()
                .spawn(async move {
                    let decodable = Self::is_decodable_audio_file(&path);
                    if !decodable {
                        MusicPlaybackDiskCache::remove_file(&path);
                    }
                    decodable
                })
                .await;
            cx.update(|cx| {
                let message = if decodable {
                    format!("DOWNLOAD keep cache - file still decodes {name}")
                } else {
                    format!("DOWNLOAD deleted bad cache {name}")
                };
                MusicPlayerLog::info(message, cx);
            });
        })
        .detach();
    }

    fn is_decodable_audio_file(path: &Path) -> bool {
        let Ok(file) = MusicEffectAudioFile::for_reading(path) else {
            return false;
        };
        file.length() > 0 && file.processing_format().sample_rate > 0
    }

    fn remove_item_status_observer(&mut self) {
        self.item_status_observer = None;
    }

    fn remove_end_observer(&mut self) {
        self.end_observer_token = None;
    }

    fn start_local_engine(&mut self, url: PathBuf, track: TrackModel, cx: &mut Context<Self>) -> Task<bool> {
        if self.current_track.as_ref().map(|current| current.id.as_str()) != Some(track.id.as_str()) {
            return Task::ready(false);
        }
        let generation = self.prepare_generation;
        let effect = self
            .rendered_audio_effect_settings(cx)
            .is_any_enabled()
            .then(|| self.start_effect_file_player(url.clone(), track.clone(), 0.0, None, true, cx));
        cx.spawn(async move |this, cx| {
            if let Some(effect) = effect
                && effect.await
            {
                return true;
            }
            let local = this.update(cx, |this, cx| {
                if !this.can_complete_audio_preparation(generation) {
                    return None;
                }
                Some(this.start_local_file_player(url, track, 0.0, true, cx))
            });
            match local {
                Ok(Some(local)) => local.await,
                _ => false,
            }
        })
    }

    fn start_effect_file_player(
        &mut self,
        url: PathBuf,
        track: TrackModel,
        start_at: f64,
        settings: Option<MusicAudioEffectSettings>,
        publish_playback_state: bool,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if self.current_track.as_ref().map(|current| current.id.as_str()) != Some(track.id.as_str()) {
            return Task::ready(false);
        }
        let generation = self.prepare_generation;
        let session = self.configure_session(false, cx);
        cx.spawn(async move |this, cx| {
            if !session.await {
                return false;
            }
            let trace = ChatPerformanceTrace::shared();
            let prepared = this.update(cx, |this, cx| {
                if !this.can_complete_audio_preparation(generation) {
                    return None;
                }
                let effect_settings = settings.unwrap_or_else(|| this.rendered_audio_effect_settings(cx));
                let expected_duration = this.expected_track_duration(&track);
                let ep = cx.new(MusicEffectFilePlayer::new);
                let token = trace.begin_operation(
                    "audio.effect.prepare",
                    HashMap::from([
                        ("execution".to_owned(), "worker".to_owned()),
                        ("duration_kind".to_owned(), "wall".to_owned()),
                    ]),
                );
                let prepare = ep.update(cx, |ep, cx| {
                    ep.prepare_file_url(url.clone(), effect_settings.clone(), expected_duration, cx)
                });
                Some((ep, prepare, token, effect_settings))
            });
            let Ok(Some((ep, prepare, token, effect_settings))) = prepared else {
                return false;
            };
            let prepared = prepare.await;
            trace.end_operation(token, 1.0, HashMap::new());
            this.update(cx, |this, cx| {
                if !this.can_complete_audio_preparation(generation)
                    || this.current_track.as_ref().map(|current| current.id.as_str()) != Some(track.id.as_str())
                    || !prepared
                {
                    ep.update(cx, |ep, _| ep.stop());
                    return false;
                }
                this.stop_file_player_cancelling_handoff(false, cx);
                this.release_av_player(cx);
                this.local_playback_url = Some(url.clone());
                let start_token = trace.begin_operation(
                    "audio.effect.start",
                    HashMap::from([("execution".to_owned(), "main".to_owned())]),
                );
                let started = ep.update(cx, |ep, _| ep.play_prepared(start_at));
                trace.end_operation(start_token, 1.0, HashMap::new());
                if !started {
                    ep.update(cx, |ep, _| ep.stop());
                    return false;
                }
                let publish_token = trace.begin_operation(
                    "audio.music.publish",
                    HashMap::from([("execution".to_owned(), "main".to_owned())]),
                );
                let effect_duration = ep.read(cx).duration();
                this.effect_player = Some(ep.clone());
                ep.update(cx, |ep, _| ep.set_output_gain(MusicOutputVolume::gain()));
                this.install_effect_finish_observer(&ep, cx);
                this.duration = effect_duration;
                this.current_time = start_at;
                this.progress = if this.duration > 0.0 { start_at / this.duration } else { 0.0 };
                cx.notify();
                this.set_is_playing(true, cx);
                this.is_handling_track_end = false;
                this.set_last_playback_error(None, cx);
                MusicPlayerLog::ok(
                    format!(
                        "EFFECT playing id={} rate={:.2} duration={:.1}s",
                        track.id,
                        effect_settings.playback_rate(),
                        effect_duration
                    ),
                    cx,
                );
                this.update_now_playing_info(&track, cx);
                this.notify_playback_started(&track, cx);
                this.install_progress_timer(cx);
                this.prefetch_upcoming_tracks(cx);
                this.refresh_playlist_listening_presence(cx);
                if publish_playback_state && !this.suppress_next_listening_playback_publish.get() {
                    this.publish_listening_session_state(MusicListeningSessionCommand::Playback, 0, None, cx);
                }
                trace.end_operation(publish_token, 1.0, HashMap::new());
                true
            })
            .unwrap_or(false)
        })
    }

    fn expected_track_duration(&self, track: &TrackModel) -> f64 {
        let known = self.duration;
        if known > 1.0 && self.current_track.as_ref().is_some_and(|current| current.id == track.id) {
            return known;
        }
        0.0
    }

    fn start_local_file_player(
        &mut self,
        url: PathBuf,
        track: TrackModel,
        start_at: f64,
        publish_playback_state: bool,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if self.current_track.as_ref().map(|current| current.id.as_str()) != Some(track.id.as_str()) {
            return Task::ready(false);
        }
        let generation = self.prepare_generation;
        let session = self.configure_session(false, cx);
        cx.spawn(async move |this, cx| {
            if !session.await {
                return false;
            }
            let can_complete = |cx: &AsyncApp| {
                this.read_with(cx, |this, _| this.can_complete_audio_preparation(generation)).unwrap_or(false)
            };
            let still_current = |cx: &AsyncApp| {
                this.read_with(cx, |this, _| {
                    this.can_complete_audio_preparation(generation)
                        && this.current_track.as_ref().is_some_and(|current| current.id == track.id)
                })
                .unwrap_or(false)
            };
            if !can_complete(cx) {
                return false;
            }
            let trace = ChatPerformanceTrace::shared();
            let token = trace.begin_operation(
                "audio.file.prepare",
                HashMap::from([
                    ("execution".to_owned(), "worker".to_owned()),
                    ("duration_kind".to_owned(), "wall".to_owned()),
                ]),
            );
            let prepare_path = url.clone();
            let prepared = RiseAudioPlaybackWork::perform(
                |candidate: ROMusicFilePreparedPlayback| drop(candidate),
                move |_| ROMusicFilePreparedPlayback::prepare_file_url(&prepare_path, start_at).ok_or(CancellationError),
            )
            .await
            .ok();
            trace.end_operation(token, 1.0, HashMap::new());
            let Some(fp) = prepared else {
                return false;
            };
            if !still_current(cx) {
                RiseAudioPlaybackWork::dispose(move || drop(fp));
                return false;
            }
            let start_token = trace.begin_operation(
                "audio.file.start",
                HashMap::from([
                    ("execution".to_owned(), "worker".to_owned()),
                    ("duration_kind".to_owned(), "wall".to_owned()),
                ]),
            );
            let silent = RiseAudioPlaybackWork::perform(
                |(candidate, _): (ROMusicFilePreparedPlayback, bool)| drop(candidate),
                move |_| {
                    let mut fp = fp;
                    let started = !RiseCallAudioOwnership::blocks_music() && fp.start_prepared_silently();
                    Ok::<_, CancellationError>((fp, started))
                },
            )
            .await;
            trace.end_operation(start_token, 1.0, HashMap::new());
            let Ok((mut fp, mut started)) = silent else {
                return false;
            };
            if !still_current(cx) {
                RiseAudioPlaybackWork::dispose(move || drop(fp));
                return false;
            }
            if !started {
                if !RisePlaybackAudioSession::activate().await || !can_complete(cx) {
                    RiseAudioPlaybackWork::dispose(move || drop(fp));
                    return false;
                }
                let retried = RiseAudioPlaybackWork::perform(
                    |(candidate, _): (ROMusicFilePreparedPlayback, bool)| drop(candidate),
                    move |_| {
                        let mut fp = fp;
                        let started = !RiseCallAudioOwnership::blocks_music() && fp.start_prepared_silently();
                        Ok::<_, CancellationError>((fp, started))
                    },
                )
                .await;
                let Ok((retried, retried_started)) = retried else {
                    return false;
                };
                fp = retried;
                started = retried_started;
            }
            if !started || !still_current(cx) {
                RiseAudioPlaybackWork::dispose(move || drop(fp));
                return false;
            }
            let handed_over = this.update(cx, |this, cx| {
                this.stop_file_player_cancelling_handoff(false, cx);
                this.release_av_player(cx);
                this.local_playback_url = Some(url.clone());
            });
            if handed_over.is_err() {
                RiseAudioPlaybackWork::dispose(move || drop(fp));
                return false;
            }
            let mut committed = false;
            for attempt in 0..3 {
                if !still_current(cx) {
                    break;
                }
                let commit_token = trace.begin_operation(
                    "audio.file.commit",
                    HashMap::from([("execution".to_owned(), "main".to_owned())]),
                );
                committed = fp.commit_prepared_playback();
                trace.end_operation(commit_token, 1.0, HashMap::new());
                if committed || attempt == 2 {
                    break;
                }
                let reset_token = trace.begin_operation(
                    "audio.file.reposition",
                    HashMap::from([
                        ("execution".to_owned(), "worker".to_owned()),
                        ("duration_kind".to_owned(), "wall".to_owned()),
                    ]),
                );
                let reset = RiseAudioPlaybackWork::perform(
                    |candidate: ROMusicFilePreparedPlayback| drop(candidate),
                    move |_| {
                        let mut fp = fp;
                        fp.reset_prepared_playback_position();
                        Ok::<_, CancellationError>(fp)
                    },
                )
                .await;
                trace.end_operation(reset_token, 1.0, HashMap::new());
                let Ok(reset) = reset else {
                    return false;
                };
                fp = reset;
            }
            if !committed {
                RiseAudioPlaybackWork::dispose(move || drop(fp));
                return false;
            }
            this.update(cx, |this, cx| {
                let publish_token = trace.begin_operation(
                    "audio.music.publish",
                    HashMap::from([("execution".to_owned(), "main".to_owned())]),
                );
                let file_player = cx.new(|cx| ROMusicFilePlayer::new(fp, cx));
                let file_duration = file_player.read(cx).duration();
                this.file_player = Some(file_player.clone());
                file_player.update(cx, |file_player, _| file_player.set_output_gain(MusicOutputVolume::gain()));
                this.install_file_finish_observer(&file_player, cx);
                this.duration = file_duration;
                this.current_time = start_at;
                this.progress = if this.duration > 0.0 { start_at / this.duration } else { 0.0 };
                cx.notify();
                this.set_is_playing(true, cx);
                this.is_handling_track_end = false;
                this.set_last_playback_error(None, cx);
                MusicPlayerLog::ok(
                    format!("ROMusicFilePlayer playing id={} duration={:.1}s", track.id, file_duration),
                    cx,
                );
                this.update_now_playing_info(&track, cx);
                this.notify_playback_started(&track, cx);
                this.install_progress_timer(cx);
                this.prefetch_upcoming_tracks(cx);
                this.refresh_playlist_listening_presence(cx);
                if publish_playback_state && !this.suppress_next_listening_playback_publish.get() {
                    this.publish_listening_session_state(MusicListeningSessionCommand::Playback, 0, None, cx);
                }
                trace.end_operation(publish_token, 1.0, HashMap::new());
                true
            })
            .unwrap_or(false)
        })
    }

    fn install_effect_finish_observer(&mut self, ep: &Entity<MusicEffectFilePlayer>, cx: &mut Context<Self>) {
        self.remove_effect_finish_observer();
        let object = ep.entity_id();
        let (sender, mut inbox) = mpsc::unbounded::<()>();
        let subscription =
            RiseAppEvents::default_center().observe(MUSIC_EFFECT_FILE_PLAYER_DID_FINISH_PLAYING, move |note| {
                if note.payload::<MusicEffectFilePlayerDidFinishPlaying>().is_some_and(|finished| finished.object == object) {
                    let _ = sender.unbounded_send(());
                }
            });
        let delivery = cx.spawn(async move |this, cx| {
            while inbox.next().await.is_some() {
                if this.update(cx, |this, cx| this.on_local_file_playback_finished(true, cx)).is_err() {
                    break;
                }
            }
        });
        self.effect_finish_observer = Some((subscription, delivery));
    }

    fn remove_effect_finish_observer(&mut self) {
        self.effect_finish_observer = None;
    }

    fn install_file_finish_observer(&mut self, fp: &Entity<ROMusicFilePlayer>, cx: &mut Context<Self>) {
        self.remove_file_finish_observer();
        let object = fp.entity_id();
        let (sender, mut inbox) = mpsc::unbounded::<bool>();
        let subscription =
            RiseAppEvents::default_center().observe(RO_MUSIC_FILE_PLAYER_DID_FINISH_PLAYING, move |note| {
                if let Some(finished) = note.payload::<ROMusicFilePlayerDidFinishPlaying>()
                    && finished.object == object
                {
                    let _ = sender.unbounded_send(finished.successfully);
                }
            });
        let delivery = cx.spawn(async move |this, cx| {
            while let Some(successfully) = inbox.next().await {
                if this.update(cx, |this, cx| this.on_local_file_playback_finished(successfully, cx)).is_err() {
                    break;
                }
            }
        });
        self.file_finish_observer = Some((subscription, delivery));
    }

    fn remove_file_finish_observer(&mut self) {
        self.file_finish_observer = None;
    }

    fn on_local_file_playback_finished(&mut self, successfully: bool, cx: &mut Context<Self>) {
        if !self.has_local_engine() || self.current_track.is_none() {
            return;
        }
        if !self.is_playing || self.is_handling_track_end {
            MusicPlayerLog::info(
                format!(
                    "TRACK finish notification ignored isPlaying={} handling={}",
                    self.is_playing, self.is_handling_track_end
                ),
                cx,
            );
            return;
        }
        let current_id = self.current_track.as_ref().map(|current| current.id.clone()).unwrap_or_else(|| "?".to_owned());
        if !successfully {
            MusicPlayerLog::fail(format!("TRACK local playback failed id={current_id}"), cx);
            self.stop_file_player(cx);
            self.set_is_playing(false, cx);
            self.set_last_playback_error(Some("Playback failed".to_owned()), cx);
            return;
        }
        MusicPlayerLog::info(format!("TRACK finish notification id={current_id}"), cx);
        if let Some(mut timer) = self.progress_timer.take() {
            timer.invalidate();
        }
        self.set_is_playing(false, cx);
        self.handle_track_end(cx);
    }

    fn install_progress_timer(&mut self, cx: &mut Context<Self>) {
        if let Some(mut timer) = self.progress_timer.take() {
            timer.invalidate();
        }
        let timer = RepeatingTimer::scheduled_timer(Duration::from_millis(400), true, cx, |this, cx| {
            if !this.has_local_engine() {
                return;
            }
            let (Some(engine_time), Some(engine_duration)) =
                (this.local_engine_current_time(cx), this.local_engine_duration(cx))
            else {
                return;
            };
            let engine_is_playing = this.local_engine_is_playing(cx);
            let duration_before = this.duration;
            let (time_before, progress_before) = (this.current_time, this.progress);
            this.current_time = engine_time;
            this.duration = engine_duration.max(1.0);
            this.progress = if this.duration > 0.0 { engine_time / this.duration } else { 0.0 };
            let moved = this.current_time != time_before || this.progress != progress_before;
            if moved || this.duration != duration_before {
                this.publish_position(duration_before, cx);
            }
            if this.is_playing
                && let Some(track) = this.current_track.clone()
            {
                this.update_now_playing_info(&track, cx);
            }
            if !this.is_playing || this.is_handling_track_end || engine_duration <= 0.0 {
                return;
            }
            let near_end =
                engine_time >= engine_duration - 1.5 || (engine_duration > 0.0 && engine_time / engine_duration >= 0.97);
            if !engine_is_playing && near_end {
                let current_id =
                    this.current_track.as_ref().map(|current| current.id.clone()).unwrap_or_else(|| "?".to_owned());
                MusicPlayerLog::info(
                    format!("TRACK end (timer) id={current_id} t={engine_time:.1} dur={engine_duration:.1}"),
                    cx,
                );
                if let Some(mut timer) = this.progress_timer.take() {
                    timer.invalidate();
                }
                this.set_is_playing(false, cx);
                this.handle_track_end(cx);
            }
        });
        self.progress_timer = Some(timer);
    }

    fn stop_file_player(&mut self, cx: &mut Context<Self>) {
        self.stop_file_player_cancelling_handoff(true, cx);
    }

    /// stops the file player, can cancel a pending handoff too
    fn stop_file_player_cancelling_handoff(&mut self, cancel_handoff: bool, cx: &mut Context<Self>) {
        if let Some(mut timer) = self.progress_timer.take() {
            timer.invalidate();
        }
        self.remove_file_finish_observer();
        if let Some(file_player) = self.file_player.take() {
            let trace = ChatPerformanceTrace::shared();
            let token =
                trace.begin_operation("audio.file.stop", HashMap::from([("execution".to_owned(), "main".to_owned())]));
            file_player.update(cx, |file_player, _| file_player.stop());
            trace.end_operation(token, 1.0, HashMap::new());
        }
        self.stop_effect_player(cancel_handoff, cx);
        self.local_playback_url = None;
    }

    fn stop_effect_player(&mut self, cancel_handoff: bool, cx: &mut Context<Self>) {
        if cancel_handoff {
            self.effect_handoff_task = None;
        }
        self.remove_effect_finish_observer();
        if let Some(effect_player) = self.effect_player.take() {
            effect_player.update(cx, |effect_player, _| effect_player.stop());
        }
    }

    /// both local backends look the same to the player, only one is attached at a time
    fn has_local_engine(&self) -> bool {
        self.effect_player.is_some() || self.file_player.is_some()
    }

    fn local_engine_current_time(&self, cx: &App) -> Option<f64> {
        if let Some(effect_player) = &self.effect_player {
            return Some(effect_player.read(cx).current_time());
        }
        self.file_player.as_ref().map(|file_player| file_player.read(cx).current_time())
    }

    fn local_engine_duration(&self, cx: &App) -> Option<f64> {
        if let Some(effect_player) = &self.effect_player {
            return Some(effect_player.read(cx).duration());
        }
        self.file_player.as_ref().map(|file_player| file_player.read(cx).duration())
    }

    fn local_engine_is_playing(&self, cx: &App) -> bool {
        if let Some(effect_player) = &self.effect_player {
            return effect_player.read(cx).is_playing();
        }
        self.file_player.as_ref().is_some_and(|file_player| file_player.read(cx).is_playing())
    }

    fn local_engine_rate(&self, cx: &App) -> f32 {
        if let Some(effect_player) = &self.effect_player {
            return effect_player.read(cx).rate();
        }
        self.file_player.as_ref().map_or(0.0, |file_player| file_player.read(cx).rate())
    }

    fn local_engine_file_url(&self, cx: &App) -> Option<PathBuf> {
        if let Some(effect_player) = &self.effect_player {
            return effect_player.read(cx).file_url().cloned();
        }
        self.file_player.as_ref().and_then(|file_player| file_player.read(cx).file_url().cloned())
    }

    fn resume_local_engine(&mut self, cx: &mut Context<Self>) {
        if self.is_audio_session_interrupted || RiseCallAudioOwnership::blocks_music() {
            return;
        }
        if let Some(effect_player) = &self.effect_player {
            effect_player.update(cx, |effect_player, cx| effect_player.resume(cx));
            return;
        }
        if let Some(file_player) = &self.file_player {
            file_player.update(cx, |file_player, _| file_player.resume());
        }
    }

    fn pause_local_engine(&mut self, cx: &mut Context<Self>) {
        if let Some(effect_player) = &self.effect_player {
            effect_player.update(cx, |effect_player, _| effect_player.pause());
            return;
        }
        if let Some(file_player) = &self.file_player {
            file_player.update(cx, |file_player, _| file_player.pause());
        }
    }

    fn seek_local_engine(&mut self, seconds: f64, cx: &mut Context<Self>) {
        if let Some(effect_player) = &self.effect_player {
            effect_player.update(cx, |effect_player, _| effect_player.seek(seconds));
            return;
        }
        if let Some(file_player) = &self.file_player {
            file_player.update(cx, |file_player, _| file_player.seek(seconds));
        }
    }

    fn recover_local_engine_if_stalled(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(effect_player) = &self.effect_player {
            return effect_player.update(cx, |effect_player, cx| effect_player.recover_if_stalled(cx));
        }
        self.file_player
            .as_ref()
            .is_some_and(|file_player| file_player.update(cx, |file_player, _| file_player.recover_if_stalled()))
    }

    fn local_engine_stall_recovery_count(&self, cx: &App) -> i64 {
        if let Some(effect_player) = &self.effect_player {
            return effect_player.read(cx).stall_recovery_count();
        }
        self.file_player.as_ref().map_or(0, |file_player| file_player.read(cx).stall_recovery_count())
    }

    fn local_engine_restore_full_volume(&mut self, cx: &mut Context<Self>) {
        if let Some(effect_player) = &self.effect_player {
            effect_player.update(cx, |effect_player, _| effect_player.restore_full_volume());
        }
        if let Some(file_player) = &self.file_player {
            file_player.update(cx, |file_player, _| file_player.restore_full_volume());
        }
    }

    fn join_call_with_music(&mut self, cx: &mut Context<Self>) {
        RiseCallAudioOwnership::allow_music_in_call();
        self.pending_playback_after_call = None;
        self.paused_for_call = false;
        if !self.external_audio_owners.remove(&MusicExternalAudioOwner::RTC) {
            return;
        }
        self.audio_session_activated = false;
        if !self.external_audio_owners.is_empty() {
            return;
        }
        self.external_duck_active = false;
        self.external_resume_intent = false;
        self.duck_origin_was_playing = false;
        MusicPlayerLog::info("DUCK end owner=rtc \u{2014} user plays music in the call".to_owned(), cx);
    }

    /// applies the call app volume to the playing engine, fades and ducks keep their own level
    /// also multiplied by the bar music volume
    pub(crate) fn apply_call_app_volume(&mut self, cx: &mut Context<Self>) {
        let gain = MusicOutputVolume::gain();
        if let Some(effect_player) = self.effect_player.clone() {
            effect_player.update(cx, |effect_player, _| effect_player.set_output_gain(gain));
        }
        if let Some(file_player) = self.file_player.clone() {
            file_player.update(cx, |file_player, _| file_player.set_output_gain(gain));
        }
        let Some(player) = self.player.clone() else {
            return;
        };
        if self.av_player_fade_timer.is_some() || !self.is_playing || self.has_local_engine() {
            return;
        }
        player.update(cx, |player, cx| player.set_volume(gain, cx));
    }

    fn local_engine_set_volume(&mut self, volume: f32, fade: f64, cx: &mut Context<Self>) {
        if let Some(effect_player) = &self.effect_player {
            effect_player.update(cx, |effect_player, cx| effect_player.set_volume(volume, fade, cx));
            return;
        }
        if let Some(file_player) = &self.file_player {
            file_player.update(cx, |file_player, cx| file_player.set_volume(volume, fade, cx));
        }
    }

    /// ramps a fresh track up from silence
    /// skipped while something else owns the level so it cant fight a duck
    fn apply_playback_start_fade(&mut self, cx: &mut Context<Self>) {
        if !self.external_audio_owners.is_empty() || self.external_duck_active {
            return;
        }
        if self.has_local_engine() {
            self.local_engine_set_volume(0.0, 0.0, cx);
            self.local_engine_set_volume(1.0, Self::PLAYBACK_START_FADE_DURATION, cx);
            return;
        }
        let Some(player) = self.player.clone() else {
            return;
        };
        player.update(cx, |player, cx| player.set_volume(0.0, cx));
        self.fade_av_player_volume(MusicOutputVolume::gain(), Self::PLAYBACK_START_FADE_DURATION, None, cx);
    }

    fn local_engine_fade_out_and_pause(&mut self, duration: f64, cx: &mut Context<Self>) {
        if let Some(effect_player) = &self.effect_player {
            effect_player.update(cx, |effect_player, cx| effect_player.fade_out_and_pause(duration, cx));
            return;
        }
        if let Some(file_player) = &self.file_player {
            file_player.update(cx, |file_player, cx| file_player.fade_out_and_pause(duration, cx));
        }
    }

    fn local_engine_fade_in_and_resume(&mut self, duration: f64, cx: &mut Context<Self>) {
        if let Some(effect_player) = &self.effect_player {
            effect_player.update(cx, |effect_player, cx| effect_player.fade_in_and_resume(duration, cx));
            return;
        }
        if let Some(file_player) = &self.file_player {
            file_player.update(cx, |file_player, cx| file_player.fade_in_and_resume(duration, cx));
        }
    }

    fn release_av_player(&mut self, cx: &mut Context<Self>) {
        self.remove_item_status_observer();
        self.remove_end_observer();
        self.remove_time_observer(cx);
        if let Some(player) = self.player.take() {
            player.update(cx, |player, cx| {
                player.pause(cx);
                player.replace_current_item(None, cx);
            });
        }
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        if !self.allows_listening_control(MusicListeningBlockedControl::Playback, false, cx) {
            return;
        }

        self.call_playback_request_generation = self.call_playback_request_generation.wrapping_add(1);
        self.pending_playback_after_call = None;
        self.clear_listening_interruption_flags_for_user_control();
        self.pause_playback_engine(cx);
        self.refresh_playlist_listening_presence(cx);
        self.publish_listening_session_state(MusicListeningSessionCommand::Playback, 0, None, cx);
    }

    /// user touched the session controls so any remembered interruption pause and pause consent are stale
    /// ad-hoc mp3 controls dont count
    fn clear_listening_interruption_flags_for_user_control(&mut self) {
        if self.listening_session.is_none() || self.is_listening_playback_displaced_by_ad_hoc() {
            return;
        }
        self.listening_consent_episode_active = false;
        self.session_paused_by_local_interruption = false;
    }

    fn pause_playback_engine(&mut self, cx: &mut Context<Self>) {
        self.was_playing_before_interruption = false;
        self.resume_playback_when_foreground = false;
        self.effect_handoff_task = None;
        self.duck_fade_epoch = self.duck_fade_epoch.wrapping_add(1);
        if let Some(mut timer) = self.av_player_fade_timer.take() {
            timer.invalidate();
        }
        self.av_player_fade_completion = None;
        if !self.external_audio_owners.is_empty() {
            self.external_resume_intent = false;
            self.duck_origin_was_playing = false;
        }
        self.background_keep_alive_task = None;
        self.end_background_task();
        self.prepare_task = None;
        self.cancel_session_continuation();
        self.prepare_generation = self.prepare_generation.wrapping_add(1);
        self.is_preparing_playback = false;
        self.clear_playback_download_progress(cx);
        self.pause_local_engine(cx);
        if let Some(player) = &self.player {
            player.update(cx, |player, cx| player.pause(cx));
        }
        self.set_is_playing(false, cx);
        self.refresh_now_playing_playback_state(cx);
        cx.notify();
    }

    fn resume_playback_from_user(&mut self, cx: &mut Context<Self>) {
        if self.current_track.is_none() {
            return;
        }
        if !self.request_playback_after_leaving_call(
            Box::new(|this: &mut MusicPlayerState, cx: &mut Context<MusicPlayerState>| {
                this.resume_playback_from_user(cx);
            }),
            cx,
        ) {
            return;
        }
        self.call_playback_request_generation = self.call_playback_request_generation.wrapping_add(1);
        if !self.allows_listening_control(MusicListeningBlockedControl::Playback, false, cx) {
            return;
        }
        if !self.take_over_external_chat_playback_for_user_control(false, cx) {
            return;
        }

        self.clear_listening_interruption_flags_for_user_control();
        self.resume_playback(true, None, cx);
    }

    pub fn toggle_play_pause(&mut self, cx: &mut Context<Self>) {
        if self.is_backend_silent_while_playing(cx) {
            MusicPlayerLog::info("PLAY toggle on a silent backend \u{2014} resume instead of pause".to_owned(), cx);
            self.set_is_playing(false, cx);
            self.audio_session_activated = false;
            self.resume_playback_from_user(cx);
            return;
        }
        if !self.is_playing && self.session_continuation_task.is_none() && !self.is_preparing_playback {
            self.resume_playback_from_user(cx);
            return;
        }
        if !self.allows_listening_control(MusicListeningBlockedControl::Playback, false, cx) {
            return;
        }

        self.call_playback_request_generation = self.call_playback_request_generation.wrapping_add(1);
        self.pending_playback_after_call = None;
        self.clear_listening_interruption_flags_for_user_control();
        self.pause_playback_engine(cx);
        self.refresh_playlist_listening_presence(cx);
        self.publish_listening_session_state(MusicListeningSessionCommand::Playback, 0, None, cx);
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        if !self.allows_listening_control(MusicListeningBlockedControl::ChangeTracks, false, cx) {
            return;
        }
        if self.queue.is_empty() {
            return;
        }
        if self.queue_index + 1 < self.queue.len() {
            let track = self.queue[self.queue_index + 1].clone();
            self.play(track, MusicPlayOptions { preserve_queue: true, ..MusicPlayOptions::default() }, cx);
            return;
        }
        if self.repeat_mode(cx) == MusicRepeatMode::All {
            let track = self.queue[0].clone();
            self.play(track, MusicPlayOptions { preserve_queue: true, ..MusicPlayOptions::default() }, cx);
            return;
        }
        self.stop_at_end_of_queue(cx);
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) {
        if self.current_time > 3.0 {
            if !self.allows_listening_control(MusicListeningBlockedControl::Seek, false, cx) {
                return;
            }
            self.seek(0.0, cx);
            return;
        }
        if !self.allows_listening_control(MusicListeningBlockedControl::ChangeTracks, false, cx) {
            return;
        }
        if self.queue.is_empty() {
            return;
        }
        if self.queue_index > 0 {
            let track = self.queue[self.queue_index - 1].clone();
            self.play(track, MusicPlayOptions { preserve_queue: true, ..MusicPlayOptions::default() }, cx);
            return;
        }
        if self.repeat_mode(cx) == MusicRepeatMode::All {
            let track = self.queue[self.queue.len() - 1].clone();
            self.play(track, MusicPlayOptions { preserve_queue: true, ..MusicPlayOptions::default() }, cx);
        }
    }

    fn stop_at_end_of_queue(&mut self, cx: &mut Context<Self>) {
        if self.is_listening_playback_displaced_by_ad_hoc() {
            MusicPlayerLog::info("QUEUE end - ad-hoc playback over, returning to the session".to_owned(), cx);
            self.pause_playback_engine(cx);
            self.restore_listening_session_after_ad_hoc_playback(cx);
            return;
        }
        MusicPlayerLog::info("QUEUE end - stopping (repeat=none)".to_owned(), cx);
        self.pause(cx);
    }

    pub fn seek(&mut self, fraction: f64, cx: &mut Context<Self>) {
        if !self.allows_listening_control(MusicListeningBlockedControl::Seek, false, cx) {
            return;
        }

        let track_id = self.current_track.as_ref().map(|current| current.id.clone());
        let session_id = self.listening_session.as_ref().map(|session| session.session_id.clone());
        self.perform_seek(
            fraction,
            Some(Box::new(move |this: &mut MusicPlayerState, committed_time: f64, cx: &mut Context<MusicPlayerState>| {
                if this.current_track.as_ref().map(|current| current.id.clone()) != track_id
                    || this.listening_session.as_ref().map(|session| session.session_id.clone()) != session_id
                {
                    return;
                }

                let committed_position_ms = (committed_time.max(0.0) * 1_000.0) as i64;
                this.refresh_playlist_listening_presence(cx);
                this.publish_listening_session_state(
                    MusicListeningSessionCommand::Seek,
                    0,
                    Some(committed_position_ms),
                    cx,
                );
            })),
            cx,
        );
    }

    fn perform_seek(
        &mut self,
        fraction: f64,
        completion: Option<Box<dyn FnOnce(&mut MusicPlayerState, f64, &mut Context<MusicPlayerState>)>>,
        cx: &mut Context<Self>,
    ) {
        self.seek_operation_generation = self.seek_operation_generation.wrapping_add(1);
        let seek_generation = self.seek_operation_generation;
        let fraction = fraction.clamp(0.0, 1.0);
        if self.has_local_engine()
            && let Some(engine_duration) = self.local_engine_duration(cx)
            && engine_duration > 0.0
        {
            let t = engine_duration * fraction;
            self.seek_local_engine(t, cx);
            self.progress = fraction;
            self.current_time = t;
            cx.notify();
            if let Some(completion) = completion {
                completion(self, t, cx);
            }
            return;
        }
        let Some(player) = self.player.clone() else {
            return;
        };
        let (Some(item), Some(track_id)) =
            (player.read(cx).current_item().cloned(), self.current_track.as_ref().map(|current| current.id.clone()))
        else {
            return;
        };
        let Some(duration_seconds) = player.read(cx).current_item_duration() else {
            return;
        };
        if duration_seconds.is_nan() || duration_seconds <= 0.0 {
            return;
        }
        let target_seconds = duration_seconds * fraction;
        self.progress = fraction;
        self.current_time = target_seconds;
        cx.notify();
        let this = cx.weak_entity();
        player.update(cx, |player, cx| {
            player.seek(
                target_seconds,
                0.0,
                0.0,
                Some(Box::new(move |finished: bool, cx: &mut App| {
                    if !finished {
                        return;
                    }
                    let _ = this.update(cx, |this, cx| {
                        if this.seek_operation_generation != seek_generation
                            || this.player.as_ref().and_then(|player| player.read(cx).current_item().cloned()).as_ref()
                                != Some(&item)
                            || this.current_track.as_ref().map(|current| current.id.as_str()) != Some(track_id.as_str())
                        {
                            return;
                        }
                        this.current_time = target_seconds;
                        this.progress = fraction;
                        cx.notify();
                        if let Some(completion) = completion {
                            completion(this, target_seconds, cx);
                        }
                    });
                })),
                cx,
            );
        });
    }

    fn add_time_observer(&mut self, cx: &mut Context<Self>) {
        let Some(player) = self.player.clone() else {
            self.time_observer = None;
            return;
        };
        let this = cx.weak_entity();
        let observer = player.update(cx, |player, cx| {
            player.add_periodic_time_observer(
                Duration::from_millis(500),
                Box::new(move |t: f64, cx: &mut App| {
                    let _ = this.update(cx, |this, cx| {
                        let duration_before = this.duration;
                        let (time_before, progress_before) = (this.current_time, this.progress);
                        this.current_time = t;
                        if let Some(duration) = this.player.as_ref().and_then(|player| player.read(cx).current_item_duration())
                            && !duration.is_nan()
                            && duration > 0.0
                        {
                            this.duration = duration;
                            this.progress = t / duration;
                        }
                        if this.current_time != time_before
                            || this.progress != progress_before
                            || this.duration != duration_before
                        {
                            this.publish_position(duration_before, cx);
                        }
                        if this.is_playing
                            && let Some(track) = this.current_track.clone()
                        {
                            this.update_now_playing_info(&track, cx);
                        }
                    });
                }),
                cx,
            )
        });
        self.time_observer = Some(observer);
    }

    fn remove_time_observer(&mut self, cx: &mut Context<Self>) {
        if let Some(observer) = self.time_observer.take() {
            match &self.player {
                Some(player) => player.update(cx, |player, _| player.remove_time_observer(observer)),
                None => drop(observer),
            }
        }
    }

    pub fn handle_track_end(&mut self, cx: &mut Context<Self>) {
        if self.is_audio_session_interrupted {
            return;
        }
        if self.is_handling_track_end {
            MusicPlayerLog::info("TRACK end ignored - already handling".to_owned(), cx);
            return;
        }
        if self.is_listening_session_follower() && !self.is_listening_playback_displaced_by_ad_hoc() {
            self.set_is_playing(false, cx);
            self.current_time = self.current_time.max(self.duration);
            self.progress = 1.0;
            cx.notify();
            self.refresh_now_playing_playback_state(cx);
            self.refresh_playlist_listening_presence(cx);
            return;
        }
        self.is_handling_track_end = true;
        let track_id = self.current_track.as_ref().map(|current| current.id.clone()).unwrap_or_else(|| "?".to_owned());
        let repeat_mode = self.repeat_mode(cx);
        MusicPlayerLog::info(
            format!(
                "TRACK end id={track_id} repeat={} queueIdx={}/{}",
                match repeat_mode {
                    MusicRepeatMode::One => "one",
                    MusicRepeatMode::All => "all",
                    MusicRepeatMode::Off => "none",
                },
                self.queue_index,
                self.queue.len().saturating_sub(1)
            ),
            cx,
        );

        match repeat_mode {
            MusicRepeatMode::One => {
                self.is_handling_track_end = false;
                self.perform_seek(
                    0.0,
                    Some(Box::new(move |this: &mut MusicPlayerState, _: f64, cx: &mut Context<MusicPlayerState>| {
                        if this.is_audio_session_interrupted
                            || this.current_track.as_ref().map(|current| current.id.as_str()) != Some(track_id.as_str())
                        {
                            return;
                        }
                        if this.has_local_engine() {
                            this.resume_local_engine(cx);
                            this.set_is_playing(true, cx);
                            this.install_progress_timer(cx);
                        } else {
                            let rate = this.remote_playback_rate();
                            if let Some(player) = &this.player {
                                player.update(cx, |player, cx| player.play_immediately(rate, cx));
                            }
                            this.set_is_playing(true, cx);
                        }
                        this.refresh_now_playing_playback_state(cx);
                        this.refresh_playlist_listening_presence(cx);
                        this.publish_listening_session_state(MusicListeningSessionCommand::Seek, 0, Some(0), cx);
                        MusicPlayerLog::info(format!("TRACK repeat-one restart id={track_id}"), cx);
                    })),
                    cx,
                );
            }
            MusicRepeatMode::All => self.advance_after_track_ended(true, cx),
            MusicRepeatMode::Off => self.advance_after_track_ended(false, cx),
        }
    }

    fn advance_after_track_ended(&mut self, wrap: bool, cx: &mut Context<Self>) {
        self.background_keep_alive_task = None;
        if let Some(mut timer) = self.progress_timer.take() {
            timer.invalidate();
        }
        if let Some(file_player) = self.file_player.take() {
            file_player.update(cx, |file_player, _| file_player.stop());
        }
        self.stop_effect_player(true, cx);
        self.remove_file_finish_observer();
        self.local_playback_url = None;
        self.set_is_playing(false, cx);

        if self.queue.is_empty() {
            self.is_handling_track_end = false;
            self.stop_at_end_of_queue(cx);
            return;
        }
        if self.queue_index + 1 < self.queue.len() {
            self.queue_index += 1;
            let next = self.queue[self.queue_index].clone();
            MusicPlayerLog::info(format!("TRACK advance idx={} id={} wrap=false", self.queue_index, next.id), cx);
            self.play_next_in_queue_after_end(next, cx);
            return;
        }
        if wrap {
            self.queue_index = 0;
            let next = self.queue[0].clone();
            MusicPlayerLog::info(format!("TRACK advance idx=0 id={} wrap=true", next.id), cx);
            self.play_next_in_queue_after_end(next, cx);
        } else {
            self.is_handling_track_end = false;
            self.stop_at_end_of_queue(cx);
        }
    }

    fn play_next_in_queue_after_end(&mut self, track: TrackModel, cx: &mut Context<Self>) {
        let candidates = MusicPlaybackUrl::playback_url_candidates_for(&track);
        if candidates.is_empty() {
            self.is_handling_track_end = false;
            self.set_last_playback_error(Some("Invalid audio URL".to_owned()), cx);
            MusicPlayerLog::fail(format!("TRACK advance abort - no URL id={}", track.id), cx);
            return;
        }

        let scope_id = self.playback_scope_id.clone();
        self.resolve_active_effect_settings(scope_id.as_deref(), &track, cx);
        self.reset_playback_ui(&track, cx);
        self.set_last_playback_error(None, cx);
        self.is_preparing_playback = true;
        self.record_play_on_success = false;
        self.prefetch_track_cover(&track, cx);
        self.refresh_playlist_listening_presence(cx);
        self.publish_listening_session_state(MusicListeningSessionCommand::ChangeTrack, 0, None, cx);

        self.prepare_task = None;
        self.cancel_session_continuation();
        self.prepare_generation = self.prepare_generation.wrapping_add(1);
        let generation = self.prepare_generation;
        let owner_user_id = self.playback_owner_user_id.clone();
        let authorization_token = self.playback_owner_access_token(cx);
        let owner_account_epoch = self.playback_owner_account_epoch.clone();
        self.prepare_task = Some(cx.spawn(async move |this, cx| {
            Self::load_and_play_body(
                this.clone(),
                &track,
                generation,
                owner_user_id,
                owner_account_epoch,
                authorization_token,
                false,
                cx,
            )
            .await;
            let _ = this.update(cx, |this, cx| {
                if this.prepare_generation != generation {
                    return;
                }
                this.is_preparing_playback = false;
                this.clear_playback_download_progress(cx);
                MusicPlayerLog::info(
                    format!(
                        "PLAY prepare finished id={} isPlaying={} error={}",
                        track.id,
                        this.is_playing,
                        this.last_playback_error.as_deref().unwrap_or("none")
                    ),
                    cx,
                );
                this.is_handling_track_end = false;
                cx.notify();
                this.prepare_task = None;
            });
        }));
        cx.notify();
    }
}
