use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext as _, Context, Entity, Subscription, Task};
use rise_core::foundation::foundation_url::Url;
use rise_media::video::av_player::{AVPlayer, AVPlayerItem, AVPlayerItemNotification, AVPlayerTimeObserver};

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName};
use crate::core::lib::rise_audio_playback_work::RisePlaybackAudioSession;
use crate::core::lib::rise_call_app_volume::RiseCallAppVolume;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{RemoteMediaDiskCache, RemoteMediaDiskCacheLimits, RemoteMediaFileLease};
use crate::core::media::rise_media_streaming_asset::RiseMediaStreamingAsset;
use crate::core::media::rise_media_transfer_hub::{
    RiseMediaCacheIdentity, RiseMediaTransferHub, RiseMediaTransferRequest, RiseMediaTransferRequestArgs,
};
use crate::modules::chat::components::chat::voice::chat_voice_message_player::{
    ChatMediaPlaybackRate, ChatVoiceMessagePlayer,
};
use crate::modules::chat::shared::chat_music_hooks::ChatMusicHooks;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;

const TIME_OBSERVER_INTERVAL: Duration = Duration::from_millis(100);
const ENDED_STOP_DELAY: Duration = Duration::from_millis(600);

/// the one video note that plays
/// inline, expanded or floating
pub struct VideoNotePlayerStore {
    is_active: bool,
    is_playing: bool,
    progress: f32,
    elapsed_seconds: f64,
    duration_seconds: f64,
    sender_name: String,
    message_id: String,
    active_chat_id: String,
    thumbnail_url: Option<String>,
    is_floating: bool,
    playback_rate: f32,

    display_epoch: i64,

    player: Entity<AVPlayer>,

    time_observer: Option<AVPlayerTimeObserver>,
    end_observer: Option<Subscription>,
    current_item: Option<AVPlayerItem>,
    playback_generation: i64,
    load_task: Option<Task<()>>,
    resume_task: Option<Task<()>>,
    resume_generation: i64,
    media_lease: Option<RemoteMediaFileLease>,
    streaming_asset: Option<RiseMediaStreamingAsset>,
}

impl VideoNotePlayerStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let player = cx.new(AVPlayer::new);
        RiseCallAppVolume::follow(&player, cx);
        Self {
            is_active: false,
            is_playing: false,
            progress: 0.0,
            elapsed_seconds: 0.0,
            duration_seconds: 0.0,
            sender_name: String::new(),
            message_id: String::new(),
            active_chat_id: String::new(),
            thumbnail_url: None,
            is_floating: false,
            playback_rate: 1.0,
            display_epoch: 0,
            player,
            time_observer: None,
            end_observer: None,
            current_item: None,
            playback_generation: 0,
            load_task: None,
            resume_task: None,
            resume_generation: 0,
            media_lease: None,
            streaming_asset: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    pub fn progress(&self) -> f32 {
        self.progress
    }

    pub fn elapsed_seconds(&self) -> f64 {
        self.elapsed_seconds
    }

    pub fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }

    pub fn sender_name(&self) -> &str {
        &self.sender_name
    }

    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    pub fn active_chat_id(&self) -> &str {
        &self.active_chat_id
    }

    pub fn thumbnail_url(&self) -> Option<&str> {
        self.thumbnail_url.as_deref()
    }

    pub fn is_floating(&self) -> bool {
        self.is_floating
    }

    pub fn playback_rate(&self) -> f32 {
        self.playback_rate
    }

    pub fn display_epoch(&self) -> i64 {
        self.display_epoch
    }

    pub fn player(&self) -> Entity<AVPlayer> {
        self.player.clone()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn play(
        &mut self,
        url: &str,
        message_id: &str,
        chat_id: &str,
        sender_name: &str,
        duration_seconds: f64,
        thumbnail_url: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let Some(video_url) = RemoteMediaURL::optional_normalized(Some(url)).and_then(|resolved| Url::from_string(&resolved))
        else {
            return;
        };

        let weak = cx.weak_entity();
        let retry_url = url.to_owned();
        let retry_message_id = message_id.to_owned();
        let retry_chat_id = chat_id.to_owned();
        let retry_sender_name = sender_name.to_owned();
        let retry_thumbnail_url = thumbnail_url.map(str::to_owned);
        let retry = Rc::new(move |cx: &mut App| {
            weak.update(cx, |store, cx| {
                store.play(
                    &retry_url,
                    &retry_message_id,
                    &retry_chat_id,
                    &retry_sender_name,
                    duration_seconds,
                    retry_thumbnail_url.as_deref(),
                    cx,
                );
            })
            .ok();
        });
        if !ChatMusicHooks::request_external_playback(None, false, retry, None, cx) {
            return;
        }

        ChatMusicHooks::suspend_for_external_audio(MusicExternalAudioOwner::CHAT_VIDEO_NOTE, true, cx);
        ChatVoiceMessagePlayer::shared(cx).update(cx, |player, cx| player.stop(cx));
        self.resume_generation = self.resume_generation.wrapping_add(1);
        self.resume_task = None;
        self.load_task = None;
        self.teardown_observers();
        self.playback_generation = self.playback_generation.wrapping_add(1);
        let generation = self.playback_generation;

        self.sender_name = sender_name.to_owned();
        self.message_id = message_id.to_owned();
        self.active_chat_id = chat_id.to_owned();
        self.duration_seconds = duration_seconds;
        self.thumbnail_url = thumbnail_url.map(str::to_owned);
        self.progress = 0.0;
        self.elapsed_seconds = 0.0;
        self.is_floating = false;
        self.is_active = true;
        self.is_playing = true;
        Self::post_playback_target_did_change();
        cx.notify();

        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let cache_key = RiseMediaCacheIdentity::key(&video_url);
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let activated = RisePlaybackAudioSession::activate().await;
            let current = this.update(cx, |store, cx| {
                let current = generation == store.playback_generation;
                if !activated && current {
                    store.stop(cx);
                }
                current
            });
            if !activated || !matches!(current, Ok(true)) {
                return;
            }
            let lookup_url = video_url.clone();
            let lookup_key = cache_key.clone();
            let lookup_user_id = account_user_id.clone();
            let lookup_epoch = account_epoch.clone();
            let cached = cx
                .background_spawn(async move {
                    RemoteMediaDiskCache::shared().existing_local_url(
                        &lookup_url,
                        Some(lookup_key.as_str()),
                        lookup_user_id.as_deref(),
                        lookup_epoch.as_deref(),
                    )
                })
                .await;
            this.update(cx, |store, cx| {
                if generation != store.playback_generation
                    || account_user_id != SaiAccountCacheScope::active_user_id()
                    || account_epoch != SaiAccountCacheScope::active_epoch()
                {
                    return;
                }
                if let Some(cached) = cached {
                    store.load_task = None;
                    if let Some(file) = file_url(&cached) {
                        store.install_player_item(file, generation, duration_seconds, cx);
                    }
                    return;
                }
                store.install_streaming_item(
                    &video_url,
                    &cache_key,
                    generation,
                    duration_seconds,
                    account_user_id.clone(),
                    account_epoch.clone(),
                    cx,
                );
                let request = RiseMediaTransferRequest::new(
                    video_url.clone(),
                    RiseMediaTransferRequestArgs {
                        cache_key: Some(cache_key.clone()),
                        suggested_filename: Some(video_url.last_path_component()),
                        is_streamable: true,
                        account_user_id: account_user_id.clone(),
                        account_epoch: account_epoch.clone(),
                        ..RiseMediaTransferRequestArgs::default()
                    },
                );
                RiseMediaTransferHub::shared().start(request, false, cx);
                store.load_task = None;
            })
            .ok();
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn install_streaming_item(
        &mut self,
        remote: &Url,
        cache_key: &str,
        generation: i64,
        fallback_duration: f64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.playback_generation {
            return;
        }
        let Some(streaming) = RiseMediaStreamingAsset::make(
            remote,
            cache_key,
            RemoteMediaDiskCacheLimits::default_limits().maximum_file_bytes,
            account_user_id,
            account_epoch,
        ) else {
            self.install_player_item(remote.clone(), generation, fallback_duration, cx);
            return;
        };
        let item = AVPlayerItem::with_asset(streaming.asset.clone());
        self.streaming_asset = Some(streaming);
        self.media_lease = None;
        self.current_item = Some(item.clone());
        let playback_rate = self.playback_rate;
        let is_playing = self.is_playing;
        self.player.update(cx, |player, cx| {
            player.replace_current_item(Some(item.clone()), cx);
            if is_playing {
                player.play_immediately(playback_rate, cx);
            }
        });
        self.observe_item(&item, generation, fallback_duration, cx);
    }

    fn install_player_item(&mut self, url: Url, generation: i64, fallback_duration: f64, cx: &mut Context<Self>) {
        if generation != self.playback_generation {
            return;
        }
        if let Some(streaming) = self.streaming_asset.take() {
            streaming.teardown();
        }
        let local_path = url.is_file_url().then(|| url.to_file_path()).flatten();
        self.media_lease = RemoteMediaDiskCache::lease(local_path.as_deref());
        let item = AVPlayerItem::new(url);
        self.current_item = Some(item.clone());
        let playback_rate = self.playback_rate;
        let is_playing = self.is_playing;
        self.player.update(cx, |player, cx| {
            player.replace_current_item(Some(item.clone()), cx);
            if is_playing {
                player.play_immediately(playback_rate, cx);
            }
        });

        self.observe_item(&item, generation, fallback_duration, cx);
    }

    fn observe_item(&mut self, item: &AVPlayerItem, generation: i64, fallback_duration: f64, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let observed_player = self.player.downgrade();
        let block: Box<dyn Fn(f64, &mut App)> = Box::new(move |elapsed, cx| {
            let Ok(item_duration) = observed_player.read_with(&*cx, |player, _| player.current_item_duration()) else {
                return;
            };
            let duration = item_duration.unwrap_or(fallback_duration);
            if duration <= 0.0 || duration.is_nan() {
                return;
            }
            weak.update(cx, |store, cx| {
                if generation != store.playback_generation {
                    return;
                }
                store.elapsed_seconds = elapsed;
                store.duration_seconds = duration;
                store.progress = (elapsed / duration).min(1.0) as f32;
                cx.notify();
            })
            .ok();
        });
        self.time_observer =
            Some(self.player.update(cx, |player, cx| player.add_periodic_time_observer(TIME_OBSERVER_INTERVAL, block, cx)));

        let item_url = item.url().clone();
        self.end_observer = Some(cx.subscribe(&self.player, move |store, _, notification: &AVPlayerItemNotification, cx| {
            let AVPlayerItemNotification::DidPlayToEndTime { asset_url } = notification;
            if *asset_url == item_url {
                store.on_playback_ended(generation, cx);
            }
        }));
    }

    pub fn pause_or_resume(&mut self, cx: &mut Context<Self>) {
        if !self.is_active {
            return;
        }
        if self.is_playing || self.resume_task.is_some() {
            self.resume_generation = self.resume_generation.wrapping_add(1);
            self.resume_task = None;
            self.player.update(cx, |player, cx| player.pause(cx));
            self.is_playing = false;
            cx.notify();
        } else {
            let weak = cx.weak_entity();
            let retry = Rc::new(move |cx: &mut App| {
                weak.update(cx, |store, cx| store.pause_or_resume(cx)).ok();
            });
            if !ChatMusicHooks::request_external_playback(None, false, retry, None, cx) {
                return;
            }
            ChatMusicHooks::suspend_for_external_audio(MusicExternalAudioOwner::CHAT_VIDEO_NOTE, true, cx);
            self.resume_generation = self.resume_generation.wrapping_add(1);
            let request = self.resume_generation;
            let generation = self.playback_generation;
            self.resume_task = Some(cx.spawn(async move |this, cx| {
                let activated = RisePlaybackAudioSession::activate().await;
                this.update(cx, |store, cx| {
                    if store.resume_generation != request {
                        return;
                    }
                    store.resume_task = None;
                    if !activated || store.playback_generation != generation || !store.is_active {
                        return;
                    }
                    let playback_rate = store.playback_rate;
                    store.player.update(cx, |player, cx| player.play_immediately(playback_rate, cx));
                    store.is_playing = true;
                    cx.notify();
                })
                .ok();
            }));
        }
    }

    pub fn cycle_playback_rate(&mut self, cx: &mut Context<Self>) {
        self.playback_rate = ChatMediaPlaybackRate::next(self.playback_rate);
        if self.is_playing {
            let playback_rate = self.playback_rate;
            self.player.update(cx, |player, cx| player.set_rate(playback_rate, cx));
        }
        cx.notify();
    }

    pub fn bump_display(&mut self, cx: &mut Context<Self>) {
        self.display_epoch = self.display_epoch.wrapping_add(1);
        cx.notify();
    }

    pub fn enter_floating(&mut self, cx: &mut Context<Self>) {
        if !self.is_active {
            return;
        }
        self.is_floating = true;
        cx.notify();
    }

    pub fn exit_floating(&mut self, cx: &mut Context<Self>) {
        self.is_floating = false;
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.playback_generation = self.playback_generation.wrapping_add(1);
        self.resume_generation = self.resume_generation.wrapping_add(1);
        self.resume_task = None;
        self.load_task = None;
        self.teardown_observers();
        self.player.update(cx, |player, cx| player.replace_current_item(None, cx));
        self.current_item = None;
        self.media_lease = None;
        if let Some(streaming) = self.streaming_asset.take() {
            streaming.teardown();
        }
        ChatMusicHooks::resume_after_external_audio(MusicExternalAudioOwner::CHAT_VIDEO_NOTE, cx);
        self.is_active = false;
        self.is_playing = false;
        self.is_floating = false;
        self.progress = 0.0;
        self.elapsed_seconds = 0.0;
        self.duration_seconds = 0.0;
        self.sender_name = String::new();
        self.message_id = String::new();
        self.active_chat_id = String::new();
        self.thumbnail_url = None;
        Self::post_playback_target_did_change();
        cx.notify();
    }

    pub const PLAYBACK_TARGET_DID_CHANGE: RiseNotificationName =
        RiseNotificationName::new("VideoNotePlayerStore.playbackTargetDidChange");

    fn post_playback_target_did_change() {
        RiseAppEvents::default_center().post(Self::PLAYBACK_TARGET_DID_CHANGE);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.stop(cx);
        self.playback_rate = 1.0;
        cx.notify();
    }

    fn on_playback_ended(&mut self, generation: i64, cx: &mut Context<Self>) {
        if generation != self.playback_generation {
            return;
        }
        self.is_playing = false;
        self.progress = 1.0;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ENDED_STOP_DELAY).await;
            this.update(cx, |store, cx| {
                if generation == store.playback_generation {
                    store.stop(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn teardown_observers(&mut self) {
        self.time_observer = None;
        self.end_observer = None;
    }
}

/// file url of a cached file, the form the player opens
fn file_url(path: &Path) -> Option<Url> {
    Url::from_file_path(path)
}
