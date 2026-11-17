use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Context, Entity, Subscription, Task};
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_media::audio::decoder::AudioDecoder as _;
use rise_media::audio::symphonia_backend::SymphoniaDecoder;
use rise_media::video::av_player::{AVPlayer, AVPlayerItem, AVPlayerItemNotification};

use crate::core::lib::rise_audio_playback_work::RisePlaybackAudioSession;
use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription};
use crate::core::lib::rise_call_app_volume::RiseCallAppVolume;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_transport_status::{RiseTransportDropWatch, RiseTransportStatus};
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::modules::chat::shared::chat_music_hooks::ChatMusicHooks;
use crate::modules::chat::shared::utils::voice::voice_message_disk_cache::VoiceMessageDiskCache;
use crate::modules::chat::stores::message::message_actions::message_types::MessageMediaItem;
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;

const TRACE_THRESHOLD_MS: f64 = 1.0;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

enum VoicePlayerLog {}

impl VoicePlayerLog {
    fn kind() -> SaiLogKind {
        SaiLogKind::Custom("player".to_owned())
    }

    fn info(message: String, cx: &mut App) {
        tracing::warn!(target: "riseonly::X757", "voice {message}");
        Sai::log("🎵 Voice", || message, Self::kind(), cx);
    }

    fn fail(message: String, cx: &mut App) {
        tracing::warn!(target: "riseonly::X757", "voice fail {message}");
        Sai::log("🎵 Voice ❌", || message, Self::kind(), cx);
    }
}

pub enum ChatMediaPlaybackRate {}

impl ChatMediaPlaybackRate {
    pub const SUPPORTED: [f32; 3] = [1.0, 1.5, 2.0];

    pub fn next(after: f32) -> f32 {
        let Some(index) = Self::SUPPORTED.iter().position(|rate| (rate - after).abs() < 0.01) else {
            return Self::SUPPORTED[0];
        };
        Self::SUPPORTED[(index + 1) % Self::SUPPORTED.len()]
    }

    pub fn label(rate: f32) -> String {
        if (rate.round() - rate).abs() < 0.01 {
            return format!("{}x", rate.round() as i64);
        }
        format!("{rate:.1}x")
    }
}

/// file url for the player, it takes file:// urls
fn file_url(path: &Path) -> Option<Url> {
    Url::from_file_path(path)
}

fn load_duration_and_playable(url: &Url) -> Option<f64> {
    let bytes = std::fs::read(url.to_file_path()?).ok()?;
    let decoder = SymphoniaDecoder::open(bytes).ok()?;
    Some(decoder.duration_millis().map_or(0.0, |millis| millis as f64 / 1_000.0))
}

/// player with time domain pitch takes a new rate in place
/// the other kind ended the message as soon as the rate wasnt 1x so dont swap it back
struct VoiceFilePlaybackEngine {
    generation: u64,
    player: Option<Entity<AVPlayer>>,
    end_observers: Vec<Subscription>,
    known_duration: f64,
    rate: f32,
    on_finish: Option<Rc<dyn Fn(&mut App)>>,
}

impl VoiceFilePlaybackEngine {
    fn new() -> Self {
        Self { generation: 0, player: None, end_observers: Vec::new(), known_duration: 0.0, rate: 1.0, on_finish: None }
    }

    fn rate(&self) -> f32 {
        self.rate
    }

    fn is_playing(&self, cx: &App) -> bool {
        self.player.as_ref().is_some_and(|player| player.read(cx).rate() != 0.0)
    }

    fn current_time(&self, cx: &App) -> f64 {
        match self.player.as_ref().and_then(|player| player.read(cx).current_time()) {
            Some(seconds) if seconds.is_finite() => seconds.max(0.0),
            _ => 0.0,
        }
    }

    fn duration(&self) -> f64 {
        self.known_duration
    }

    fn owns(&self, player: &Entity<AVPlayer>) -> bool {
        self.player.as_ref().is_some_and(|current| current.entity_id() == player.entity_id())
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.remove_end_observers();
        if let Some(player) = self.player.take() {
            player.update(cx, |player, cx| {
                player.pause(cx);
                player.replace_current_item(None, cx);
            });
        }
        self.known_duration = 0.0;
    }

    fn pause(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(player) = &self.player {
            player.update(cx, |player, cx| player.pause(cx));
        }
    }

    fn apply_app_volume(&mut self, cx: &mut Context<Self>) {
        if let Some(player) = &self.player {
            let gain = RiseCallAppVolume::gain();
            player.update(cx, |player, cx| player.set_volume(gain, cx));
        }
    }

    fn set_rate(&mut self, value: f32, cx: &mut Context<Self>) {
        self.rate = value;
        let Some(player) = &self.player else {
            return;
        };
        if player.read(cx).rate() == 0.0 {
            return;
        }
        player.update(cx, |player, cx| player.set_rate(value, cx));
    }

    fn resume(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        let Some(player) = self.player.clone() else {
            return Task::ready(false);
        };
        let request = self.generation;
        cx.spawn(async move |this, cx| {
            let activated = RisePlaybackAudioSession::activate().await;
            let current = this
                .read_with(cx, |engine, _| engine.generation == request && engine.owns(&player))
                .unwrap_or(false);
            if !activated || !current {
                return false;
            }
            this.update(cx, |engine, cx| {
                let rate = engine.rate;
                let gain = RiseCallAppVolume::gain();
                player.update(cx, |player, cx| {
                    player.set_volume(gain, cx);
                    if player.rate() == 0.0 {
                        player.play_immediately(rate, cx);
                    }
                });
                player.read(cx).rate() != 0.0
            })
            .unwrap_or(false)
        })
    }

    fn play(&mut self, url: Url, rate: f32, cx: &mut Context<Self>) -> Task<bool> {
        self.stop(cx);
        let request = self.generation;
        cx.spawn(async move |this, cx| {
            let activated = RisePlaybackAudioSession::activate().await;
            let is_current =
                |cx: &mut gpui::AsyncApp| this.read_with(cx, |engine, _| engine.generation == request).unwrap_or(false);
            if !activated || !is_current(cx) {
                return false;
            }
            let trace = ChatPerformanceTrace::shared();
            let token = trace.begin_operation(
                "audio.voice.prepare",
                HashMap::from([
                    ("execution".to_owned(), "worker".to_owned()),
                    ("duration_kind".to_owned(), "wall".to_owned()),
                ]),
            );
            let asset_url = url.clone();
            let loaded = cx.background_executor().spawn(async move { load_duration_and_playable(&asset_url) }).await;
            trace.end_operation(token, TRACE_THRESHOLD_MS, HashMap::new());
            let Some(seconds) = loaded.filter(|_| is_current(cx)) else {
                let name = url.last_path_component();
                cx.update(|cx| VoicePlayerLog::fail(format!("asset not playable {name}"), cx));
                return false;
            };
            let total = if seconds.is_finite() && seconds > 0.0 { seconds } else { 0.0 };
            this.update(cx, |engine, cx| engine.commit(url, total, rate, cx)).unwrap_or(false)
        })
    }

    fn commit(&mut self, url: Url, total: f64, rate: f32, cx: &mut Context<Self>) -> bool {
        let trace = ChatPerformanceTrace::shared();
        let commit_token =
            trace.begin_operation("audio.voice.commit", HashMap::from([("execution".to_owned(), "main".to_owned())]));
        let player = cx.new(|cx| {
            let mut player = AVPlayer::new(cx);
            player.replace_current_item(Some(AVPlayerItem::new(url)), cx);
            player.set_volume(RiseCallAppVolume::gain(), cx);
            player
        });
        self.player = Some(player.clone());
        self.rate = rate;
        self.known_duration = total;
        self.observe_end(&player, cx);
        player.update(cx, |player, cx| player.play_immediately(rate, cx));
        let player_rate = if player.read(cx).error().is_some() { 0.0 } else { player.read(cx).rate() };
        VoicePlayerLog::info(format!("AVPlayer play rate={player_rate} dur={total:.1}s"), cx);
        let started = player_rate != 0.0;
        if !started {
            self.stop(cx);
        }
        trace.end_operation(commit_token, TRACE_THRESHOLD_MS, HashMap::new());
        started
    }

    fn observe_end(&mut self, player: &Entity<AVPlayer>, cx: &mut Context<Self>) {
        self.remove_end_observers();
        let subscription = cx.subscribe(player, |engine, player, event: &AVPlayerItemNotification, cx| {
            let AVPlayerItemNotification::DidPlayToEndTime { .. } = event;
            if !engine.owns(&player) {
                return;
            }
            if let Some(on_finish) = engine.on_finish.clone() {
                cx.defer(move |cx| on_finish(cx));
            }
        });
        self.end_observers = vec![subscription];
    }

    fn remove_end_observers(&mut self) {
        self.end_observers.clear();
    }
}

/// progress tick of the playing message, apart from the player
/// a view reading the player to know if a message plays must not rerender ten times a second
pub struct ChatVoicePlaybackTick;

pub struct ChatVoiceMessagePlayer {
    playing_message_id: Option<String>,
    progress: f64,
    is_playing: bool,
    is_loading: bool,
    active_chat_id: String,
    sender_name: String,
    elapsed_seconds: f64,
    duration_seconds: f64,
    playback_rate: f32,
    waiting_for_network_message_id: Option<String>,

    engine: Option<Entity<VoiceFilePlaybackEngine>>,
    progress_tick: Entity<ChatVoicePlaybackTick>,
    progress_timer: Option<Task<()>>,
    load_task: Option<Task<()>>,
    load_epoch: u64,
    playback_generation: i64,
    control_generation: i64,
    music_duck_generation: Option<i64>,
    connect_watch: Option<RiseTransportDropWatch>,
    _app_volume_observer: RiseSubscription,
    _app_volume_delivery: Task<()>,
}

impl ChatVoiceMessagePlayer {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let (sender, mut inbox) = mpsc::unbounded::<()>();
        let app_volume_observer =
            RiseAppEvents::default_center().observe(RiseCallAppVolume::DID_CHANGE_NOTIFICATION, move |_| {
                let _ = sender.unbounded_send(());
            });
        let app_volume_delivery = cx.spawn(async move |this, cx| {
            while inbox.next().await.is_some() {
                let updated = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                    if let Some(engine) = this.engine.clone() {
                        engine.update(cx, |engine, cx| engine.apply_app_volume(cx));
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        });
        Self {
            _app_volume_observer: app_volume_observer,
            _app_volume_delivery: app_volume_delivery,
            playing_message_id: None,
            progress: 0.0,
            is_playing: false,
            is_loading: false,
            active_chat_id: String::new(),
            sender_name: String::new(),
            elapsed_seconds: 0.0,
            duration_seconds: 0.0,
            playback_rate: 1.0,
            waiting_for_network_message_id: None,
            engine: None,
            progress_tick: cx.new(|_| ChatVoicePlaybackTick),
            progress_timer: None,
            load_task: None,
            load_epoch: 0,
            playback_generation: 0,
            control_generation: 0,
            music_duck_generation: None,
            connect_watch: None,
        }
    }

    pub fn playing_message_id(&self) -> Option<&str> {
        self.playing_message_id.as_deref()
    }

    /// notified on every progress tick, the player itself notifies only when its state changes
    pub fn progress_tick(&self) -> Entity<ChatVoicePlaybackTick> {
        self.progress_tick.clone()
    }

    pub fn progress(&self) -> f64 {
        self.progress
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    pub fn active_chat_id(&self) -> &str {
        &self.active_chat_id
    }

    pub fn sender_name(&self) -> &str {
        &self.sender_name
    }

    pub fn elapsed_seconds(&self) -> f64 {
        self.elapsed_seconds
    }

    pub fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }

    pub fn playback_rate(&self) -> f32 {
        self.playback_rate
    }

    pub fn waiting_for_network_message_id(&self) -> Option<&str> {
        self.waiting_for_network_message_id.as_deref()
    }

    pub fn is_playing_message_id(&self, message_id: &str) -> bool {
        self.playing_message_id.as_deref() == Some(message_id) && self.is_playing
    }

    pub fn is_loading_message_id(&self, message_id: &str) -> bool {
        self.playing_message_id.as_deref() == Some(message_id) && self.is_loading
    }

    pub fn is_waiting_for_network(&self, message_id: &str) -> bool {
        self.is_loading_message_id(message_id) && self.waiting_for_network_message_id.as_deref() == Some(message_id)
    }

    pub fn progress_for(&self, message_id: &str) -> f64 {
        if self.playing_message_id.as_deref() == Some(message_id) { self.progress } else { 0.0 }
    }

    pub fn is_active(&self) -> bool {
        self.playing_message_id.is_some() || self.is_loading || self.engine.is_some()
    }

    /// a message of some chat is playing
    /// a composer draft plays with no chat and never outlives its screen so it gets no bar outside the chat
    pub fn is_active_in_chat(&self) -> bool {
        self.is_active() && !self.active_chat_id.is_empty()
    }

    pub fn toggle(
        &mut self,
        message_id: String,
        chat_id: String,
        chat_name: String,
        sender_name: Option<String>,
        item: MessageMediaItem,
        cx: &mut Context<Self>,
    ) {
        VoicePlayerLog::info(
            format!(
                "toggle id={} playing={} loading={} active={}",
                prefix8(&message_id),
                self.is_playing,
                self.is_loading,
                self.playing_message_id.as_deref().map_or("nil", prefix8),
            ),
            cx,
        );

        let is_current = self.playing_message_id.as_deref() == Some(message_id.as_str());
        if is_current && self.is_playing {
            self.pause(cx);
            return;
        }
        if is_current && self.is_loading {
            VoicePlayerLog::info(format!("cancel load id={}", prefix8(&message_id)), cx);
            self.stop(cx);
            return;
        }
        let this = cx.weak_entity();
        let retry_args = (message_id.clone(), chat_id.clone(), chat_name.clone(), sender_name.clone(), item.clone());
        let retry = Rc::new(move |cx: &mut App| {
            let (message_id, chat_id, chat_name, sender_name, item) = retry_args.clone();
            if let Some(this) = this.upgrade() {
                this.update(cx, |player, cx| player.toggle(message_id, chat_id, chat_name, sender_name, item, cx));
            }
        });
        if !ChatMusicHooks::request_external_playback(None, false, retry, None, cx) {
            return;
        }
        if is_current && !self.is_playing && self.engine.is_some() && !self.is_loading {
            self.resume(cx).detach();
            return;
        }
        self.play(message_id, chat_id, chat_name, sender_name, item, cx);
    }

    pub fn play(
        &mut self,
        message_id: String,
        chat_id: String,
        chat_name: String,
        sender_name: Option<String>,
        item: MessageMediaItem,
        cx: &mut Context<Self>,
    ) {
        tracing::warn!(
            target: "riseonly::X757",
            "play id={} url={} local_preview={:?}",
            prefix8(&message_id),
            item.url.chars().take(80).collect::<String>(),
            item.local_preview_url
        );
        let Some(remote) = Self::playback_url(&item) else {
            VoicePlayerLog::fail(format!("play no URL id={}", prefix8(&message_id)), cx);
            return;
        };

        self.cancel_load_task();
        self.stop_connect_watch();
        self.playback_generation = self.playback_generation.wrapping_add(1);
        let generation = self.playback_generation;
        self.stop_playback_engine(cx);
        self.acquire_music_duck(generation, cx);
        VideoNotePlayerStore::shared(cx).update(cx, |store, cx| store.stop(cx));
        self.playing_message_id = Some(message_id.clone());
        self.progress = 0.0;
        self.is_playing = false;
        self.is_loading = true;
        self.active_chat_id = chat_id.clone();
        let resolved_sender_name =
            sender_name.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or_default().to_owned();
        self.sender_name = if resolved_sender_name.is_empty() { chat_name.clone() } else { resolved_sender_name };
        self.elapsed_seconds = 0.0;
        self.duration_seconds = item.duration.unwrap_or(0.0).max(0.0);
        cx.notify();

        if remote.is_file_url() {
            self.load_task = Some(cx.spawn(async move |this, cx| {
                this.update(cx, |player, cx| {
                    if generation != player.playback_generation {
                        return;
                    }
                    player.is_loading = false;
                    cx.notify();
                    player.start_local_playback(message_id, remote, generation, cx);
                })
                .ok();
            }));
            return;
        }

        let remote_label: String = remote.absolute_string().chars().take(100).collect();
        VoicePlayerLog::info(format!("play id={} remote={remote_label}", prefix8(&message_id)), cx);
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let local: Option<PathBuf>;
            if cx.update(RiseTransportStatus::is_connected) {
                let watch_args = (
                    this.clone(),
                    message_id.clone(),
                    chat_id.clone(),
                    chat_name.clone(),
                    sender_name.clone(),
                    item.clone(),
                );
                let drop_watch = cx.update(|cx| {
                    RiseTransportStatus::on_next_disconnect(
                        move |cx| {
                            let (this, message_id, chat_id, chat_name, sender_name, item) = watch_args;
                            let Some(this) = this.upgrade() else {
                                return;
                            };
                            this.update(cx, |player, cx| {
                                if generation != player.playback_generation
                                    || player.playing_message_id.as_deref() != Some(message_id.as_str())
                                    || !player.is_loading
                                {
                                    return;
                                }
                                VoicePlayerLog::fail(format!("link lost while loading id={}", prefix8(&message_id)), cx);
                                player.wait_for_network(message_id, chat_id, chat_name, sender_name, item, generation, cx);
                            });
                        },
                        cx,
                    )
                });
                local = VoiceMessageDiskCache::ensure_cached(
                    &remote,
                    &message_id,
                    &chat_id,
                    &chat_name,
                    item.file_size,
                    cx,
                )
                .await;
                drop_watch.stop();
            } else {
                let (cache_chat_id, cache_message_id) = (chat_id.clone(), message_id.clone());
                local = cx
                    .background_executor()
                    .spawn(async move { VoiceMessageDiskCache::cached_url(&cache_chat_id, &cache_message_id) })
                    .await;
                let current = this
                    .read_with(cx, |player, _| {
                        generation == player.playback_generation
                            && player.playing_message_id.as_deref() == Some(message_id.as_str())
                    })
                    .unwrap_or(false);
                if !current {
                    return;
                }
                if local.is_none() {
                    this.update(cx, |player, cx| {
                        player.wait_for_network(message_id, chat_id, chat_name, sender_name, item, generation, cx);
                    })
                    .ok();
                    return;
                }
            }
            this.update(cx, |player, cx| player.finish_remote_load(message_id, local, generation, cx)).ok();
        }));
    }

    fn finish_remote_load(&mut self, message_id: String, local: Option<PathBuf>, generation: i64, cx: &mut Context<Self>) {
        if generation != self.playback_generation || self.playing_message_id.as_deref() != Some(message_id.as_str()) {
            return;
        }
        self.is_loading = false;
        cx.notify();
        let Some(local) = local.as_deref().and_then(file_url) else {
            VoicePlayerLog::fail(format!("cache miss id={}", prefix8(&message_id)), cx);
            self.fail_playback(generation, cx);
            return;
        };
        VoicePlayerLog::info(
            format!("cached id={} path={}", prefix8(&message_id), local.last_path_component()),
            cx,
        );
        self.start_local_playback(message_id, local, generation, cx);
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.control_generation = self.control_generation.wrapping_add(1);
        self.cancel_load_task();
        self.stop_connect_watch();
        self.is_loading = false;
        if let Some(engine) = &self.engine {
            engine.update(cx, |engine, cx| engine.pause(cx));
        }
        self.is_playing = false;
        self.stop_progress_timer();
        let at = self.engine.as_ref().map_or(0.0, |engine| engine.read(cx).current_time(cx));
        VoicePlayerLog::info(
            format!("pause id={} t={at}", self.playing_message_id.as_deref().map_or("nil", prefix8)),
            cx,
        );
        cx.notify();
    }

    pub fn resume(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let Some(engine) = self.engine.clone() else {
            return Task::ready(());
        };
        if (engine.read(cx).rate() - self.playback_rate).abs() > 0.01 {
            let rate = self.playback_rate;
            engine.update(cx, |engine, cx| engine.set_rate(rate, cx));
        }
        let generation = self.playback_generation;
        let control = self.control_generation;
        self.acquire_music_duck(generation, cx);
        let resumed = engine.update(cx, |engine, cx| engine.resume(cx));
        cx.spawn(async move |this, cx| {
            let ok = resumed.await;
            this.update(cx, |player, cx| player.finish_resume(&engine, ok, generation, control, cx)).ok();
        })
    }

    fn finish_resume(
        &mut self,
        engine: &Entity<VoiceFilePlaybackEngine>,
        ok: bool,
        generation: i64,
        control: i64,
        cx: &mut Context<Self>,
    ) {
        if self.control_generation != control {
            return;
        }
        let owns_engine = self.engine.as_ref().is_some_and(|current| current.entity_id() == engine.entity_id());
        if generation != self.playback_generation || !owns_engine {
            self.release_music_duck_if_owned(Some(generation), cx);
            return;
        }
        if !ok || !engine.read(cx).is_playing(cx) {
            self.fail_playback(generation, cx);
            return;
        }
        self.is_playing = true;
        self.start_progress_timer(cx);
        VoicePlayerLog::info(
            format!(
                "resume id={} playing={}",
                self.playing_message_id.as_deref().map_or("nil", prefix8),
                self.is_playing
            ),
            cx,
        );
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.playback_generation = self.playback_generation.wrapping_add(1);
        self.cancel_load_task();
        self.stop_playback_engine(cx);
        self.clear_session_state(cx);
        self.release_music_duck_if_owned(None, cx);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.stop(cx);
        self.playback_rate = 1.0;
        cx.notify();
    }

    pub fn cycle_playback_rate(&mut self, cx: &mut Context<Self>) {
        self.playback_rate = ChatMediaPlaybackRate::next(self.playback_rate);
        if let Some(engine) = &self.engine {
            let rate = self.playback_rate;
            engine.update(cx, |engine, cx| engine.set_rate(rate, cx));
        }
        cx.notify();
    }

    fn cancel_load_task(&mut self) {
        self.load_task = None;
        self.load_epoch = self.load_epoch.wrapping_add(1);
    }

    fn start_local_playback(&mut self, message_id: String, url: Url, generation: i64, cx: &mut Context<Self>) {
        if generation != self.playback_generation || self.playing_message_id.as_deref() != Some(message_id.as_str()) {
            return;
        }
        let epoch = self.load_epoch;
        let playback = cx.new(|_| VoiceFilePlaybackEngine::new());
        let this = cx.weak_entity();
        playback.update(cx, |engine, _| {
            engine.on_finish = Some(Rc::new(move |cx: &mut App| {
                if let Some(this) = this.upgrade() {
                    this.update(cx, |player, cx| player.handle_playback_finished(cx));
                }
            }));
        });

        self.acquire_music_duck(generation, cx);
        let rate = self.playback_rate;
        let started = playback.update(cx, |engine, cx| engine.play(url, rate, cx));
        cx.spawn(async move |this, cx| {
            let ok = started.await;
            this.update(cx, |player, cx| player.finish_local_playback(&playback, &message_id, generation, epoch, ok, cx))
                .ok();
        })
        .detach();
    }

    fn finish_local_playback(
        &mut self,
        playback: &Entity<VoiceFilePlaybackEngine>,
        message_id: &str,
        generation: i64,
        epoch: u64,
        ok: bool,
        cx: &mut Context<Self>,
    ) {
        let is_current = generation == self.playback_generation && self.playing_message_id.as_deref() == Some(message_id);
        if !ok {
            if is_current {
                self.fail_playback(generation, cx);
            }
            return;
        }
        if epoch != self.load_epoch || !is_current {
            playback.update(cx, |engine, cx| engine.stop(cx));
            self.release_music_duck_if_owned(Some(generation), cx);
            return;
        }

        let (duration, current_time) = {
            let engine = playback.read(cx);
            (engine.duration(), engine.current_time(cx))
        };
        self.engine = Some(playback.clone());
        self.playing_message_id = Some(message_id.to_owned());
        self.is_playing = true;
        self.is_loading = false;
        self.duration_seconds = duration;
        self.elapsed_seconds = current_time;
        self.start_progress_timer(cx);
        VoicePlayerLog::info(format!("playing id={} dur={duration:.1}s", prefix8(message_id)), cx);
        cx.notify();
    }

    fn acquire_music_duck(&mut self, generation: i64, cx: &mut Context<Self>) {
        if self.music_duck_generation.is_none()
            || !ChatMusicHooks::is_external_audio_owner_active(MusicExternalAudioOwner::CHAT_VOICE_MESSAGE, cx)
        {
            ChatMusicHooks::suspend_for_external_audio(MusicExternalAudioOwner::CHAT_VOICE_MESSAGE, true, cx);
        }
        self.music_duck_generation = Some(generation);
    }

    fn release_music_duck_if_owned(&mut self, generation: Option<i64>, cx: &mut Context<Self>) {
        if let Some(generation) = generation
            && self.music_duck_generation != Some(generation)
        {
            return;
        }
        if self.music_duck_generation.is_none() {
            return;
        }
        self.music_duck_generation = None;
        ChatMusicHooks::resume_after_external_audio(MusicExternalAudioOwner::CHAT_VOICE_MESSAGE, cx);
    }

    fn handle_playback_finished(&mut self, cx: &mut Context<Self>) {
        VoicePlayerLog::info(
            format!("finished id={}", self.playing_message_id.as_deref().map_or("nil", prefix8)),
            cx,
        );
        self.stop(cx);
    }

    fn start_progress_timer(&mut self, cx: &mut Context<Self>) {
        self.stop_progress_timer();
        self.progress_timer = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PROGRESS_INTERVAL).await;
                if this.update(cx, |player, cx| player.tick_progress(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn stop_progress_timer(&mut self) {
        self.progress_timer = None;
    }

    fn tick_progress(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        if self.playing_message_id.is_none() {
            return;
        }
        let (total, current_time, is_playing) = {
            let engine = engine.read(cx);
            (engine.duration(), engine.current_time(cx), engine.is_playing(cx))
        };
        if total > 0.0 {
            self.progress = (current_time / total).max(0.0).min(1.0);
            self.elapsed_seconds = current_time;
            self.duration_seconds = total;
        }
        if self.is_playing != is_playing {
            self.is_playing = is_playing;
            cx.notify();
        }
        self.progress_tick.update(cx, |_, cx| cx.notify());
    }

    /// failure must be visible, the loader used to just go dark
    /// so a broken player looked like a failed download with no hint to the user or bug report
    fn fail_playback(&mut self, generation: i64, cx: &mut Context<Self>) {
        if generation != self.playback_generation {
            return;
        }
        self.stop_playback_engine(cx);
        self.clear_session_state(cx);
        self.release_music_duck_if_owned(Some(generation), cx);
        Notifier::send(
            tr("chat_media_download_failed"),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("waveform.slash".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    /// offline the note keeps its loading ring and starts by itself once the link is back, like a document
    #[allow(clippy::too_many_arguments)]
    fn wait_for_network(
        &mut self,
        message_id: String,
        chat_id: String,
        chat_name: String,
        sender_name: Option<String>,
        item: MessageMediaItem,
        generation: i64,
        cx: &mut Context<Self>,
    ) {
        if generation != self.playback_generation || self.playing_message_id.as_deref() != Some(message_id.as_str()) {
            return;
        }
        VoicePlayerLog::info(format!("waiting for link id={}", prefix8(&message_id)), cx);
        self.cancel_load_task();
        self.is_loading = true;
        self.stop_connect_watch();
        self.waiting_for_network_message_id = Some(message_id.clone());
        cx.notify();
        let this = cx.weak_entity();
        let resume_args = (message_id.clone(), chat_id.clone(), chat_name.clone(), sender_name.clone(), item.clone());
        self.connect_watch = Some(RiseTransportStatus::on_next_connect(
            move |cx| {
                let (message_id, chat_id, chat_name, sender_name, item) = resume_args;
                let Some(this) = this.upgrade() else {
                    return;
                };
                this.update(cx, |player, cx| {
                    player.resume_load(message_id, chat_id, chat_name, sender_name, item, generation, cx);
                });
            },
            cx,
        ));
        RiseTransportStatus::wake_if_offline(cx);
        if RiseTransportStatus::is_connected(cx) {
            self.resume_load(message_id, chat_id, chat_name, sender_name, item, generation, cx);
        }
    }

    /// resume closure of wait for network
    #[allow(clippy::too_many_arguments)]
    fn resume_load(
        &mut self,
        message_id: String,
        chat_id: String,
        chat_name: String,
        sender_name: Option<String>,
        item: MessageMediaItem,
        generation: i64,
        cx: &mut Context<Self>,
    ) {
        if generation != self.playback_generation
            || self.playing_message_id.as_deref() != Some(message_id.as_str())
            || !self.is_loading
        {
            return;
        }
        self.play(message_id, chat_id, chat_name, sender_name, item, cx);
    }

    fn stop_connect_watch(&mut self) {
        if let Some(watch) = self.connect_watch.take() {
            watch.stop();
        }
        self.waiting_for_network_message_id = None;
    }

    fn clear_session_state(&mut self, cx: &mut Context<Self>) {
        self.stop_connect_watch();
        self.playing_message_id = None;
        self.progress = 0.0;
        self.is_playing = false;
        self.is_loading = false;
        self.active_chat_id.clear();
        self.sender_name.clear();
        self.elapsed_seconds = 0.0;
        self.duration_seconds = 0.0;
        cx.notify();
    }

    fn stop_playback_engine(&mut self, cx: &mut Context<Self>) {
        self.stop_progress_timer();
        if let Some(engine) = self.engine.take() {
            engine.update(cx, |engine, cx| {
                engine.on_finish = None;
                engine.stop(cx);
            });
        }
    }

    fn playback_url(item: &MessageMediaItem) -> Option<Url> {
        let mut remote: Option<Url> = None;
        let mut local: Option<Url> = None;
        let candidates = [Some(item.url.as_str()), item.thumbnail_url.as_deref(), item.local_preview_url.as_deref()];
        for raw in candidates.into_iter().flatten().map(trimming_whitespaces_and_newlines) {
            if raw.is_empty() {
                continue;
            }
            let Some(parsed) = Self::parse_media_url(raw) else {
                continue;
            };
            if matches!(parsed.scheme(), Some("http" | "https")) {
                remote = remote.or(Some(parsed));
            } else if parsed.is_file_url() {
                local = local.or(Some(parsed));
            }
        }
        if let Some(local) = &local
            && local.to_file_path().is_some_and(|path| path.exists())
        {
            return Some(local.clone());
        }
        remote.or(local)
    }

    fn parse_media_url(raw: &str) -> Option<Url> {
        if raw.starts_with("file://") {
            return Url::from_string(raw);
        }
        if is_local_absolute_path(raw) {
            return file_url(Path::new(raw));
        }
        Url::from_string(raw)
    }
}

/// first 8 chars of an id for the log lines
fn prefix8(id: &str) -> &str {
    id.char_indices().nth(8).map_or(id, |(end, _)| &id[..end])
}
