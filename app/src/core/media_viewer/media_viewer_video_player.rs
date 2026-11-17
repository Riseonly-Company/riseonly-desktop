use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, ElementId, Entity, FontWeight, Global, Hsla,
    InteractiveElement, IntoElement, KeyBinding, MouseButton, MouseDownEvent, MouseUpEvent, ParentElement, Pixels, Point,
    Render, RenderOnce, Size, StatefulInteractiveElement, Styled, Subscription, Task, Window, actions,
    black, canvas, div, fill, point, px, size, white,
};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_media::video::av_player::{
    AVLayerVideoGravity, AVPlayer, AVPlayerActionAtItemEnd, AVPlayerItem, AVPlayerItemNotification, AVPlayerItemStatus,
    AVPlayerLayer, AVPlayerTimeControlStatus, AVPlayerTimeObserver, CMTimeRange,
};
use rise_platform::haptics::selection_changed;
use rise_ui::IconUi;
use rise_ui::liquid_glass_button::LiquidGlassPressMetrics;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::environment::LayoutDirection;
use rise_ui::swiftui::lifecycle::on_change;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::progress_view::{ControlSize, progress_view};

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::lib::rise_call_app_volume::RiseCallAppVolume;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{RemoteMediaDiskCache, RemoteMediaDiskCacheLimits, RemoteMediaFileLease};
use crate::core::media::rise_media_streaming_asset::RiseMediaStreamingAsset;
use crate::core::media::rise_media_transfer_hub::{
    RiseMediaCacheIdentity, RiseMediaTransferHub, RiseMediaTransferRequest, RiseMediaTransferRequestArgs,
};
use crate::core::media::rise_secure_media_location::RiseSecureMediaLocation;
use crate::core::media::video_warm_start_prefetch::{VideoWarmStartPolicy, VideoWarmStartPrefetch};
use crate::core::media_viewer::media_viewer_host::{NextPage, PreviousPage};
use crate::core::media_viewer::media_viewer_types::MediaViewerVideoTiming;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::media_viewer::media_zoomable_image_view::{
    MediaZoomChanged, MediaZoomContainerView, MediaZoomSingleTap, MediaZoomViewBuilder,
};
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;
use crate::modules::music::shared::utils::music::music_listening_interruption_gate::MusicListeningInterruptionGate;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaViewerVideoBufferMetrics {
    pub buffered_until: TimeInterval,
    pub buffered_ahead: TimeInterval,
    pub fraction: Option<f64>,
}

impl MediaViewerVideoBufferMetrics {
    pub fn resolve(
        loaded_time_ranges: &[CMTimeRange],
        playhead: TimeInterval,
        duration: TimeInterval,
    ) -> MediaViewerVideoBufferMetrics {
        let safe_playhead = if playhead.is_finite() { playhead.max(0.0) } else { 0.0 };
        let mut ranges: Vec<(f64, f64)> = loaded_time_ranges
            .iter()
            .filter_map(|range| {
                let start = range.start;
                let end = range.end();
                if !start.is_finite() || !end.is_finite() || end < start {
                    return None;
                }
                Some((start.max(0.0), end.max(0.0)))
            })
            .collect();
        ranges.sort_by(|lhs, rhs| lhs.0.total_cmp(&rhs.0));

        let adjacency_tolerance: TimeInterval = 0.1;
        let mut buffered_until = safe_playhead;
        let mut found_current_range = false;
        for (start, end) in ranges {
            if end + adjacency_tolerance < safe_playhead {
                continue;
            }
            if !found_current_range {
                if start > safe_playhead + adjacency_tolerance {
                    break;
                }
                found_current_range = true;
                buffered_until = buffered_until.max(end);
                continue;
            }
            if start > buffered_until + adjacency_tolerance {
                break;
            }
            buffered_until = buffered_until.max(end);
        }

        let safe_duration = if duration.is_finite() { duration.max(0.0) } else { 0.0 };
        let fraction = if safe_duration > 0.0 && found_current_range {
            Some((buffered_until / safe_duration).clamp(0.0, 1.0))
        } else {
            None
        };
        MediaViewerVideoBufferMetrics {
            buffered_until,
            buffered_ahead: if found_current_range { (buffered_until - safe_playhead).max(0.0) } else { 0.0 },
            fraction,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MediaViewerVideoControllerPlaybackStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed,
}

/// playback owner a callback was armed for, a callback from an older
/// load or another account must not publish over the current one
#[derive(Clone, Debug, PartialEq, Eq)]
struct MediaViewerVideoPlaybackOwner {
    generation: u64,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
}

/// what the observers last delivered so a notification republishes only what moved
#[derive(Default)]
struct MediaViewerVideoObservedPlayer {
    item_status: Option<AVPlayerItemStatus>,
    time_control_status: Option<AVPlayerTimeControlStatus>,
    loaded_time_ranges: Option<Vec<CMTimeRange>>,
}

/// notified on each playhead and buffer step, only the seek bar observes it
/// a controller notify rerenders the viewer and everything up to the root, ten times a second
pub struct MediaViewerVideoTimeline;

/// what the screen and player page read from the controller
/// an equal step reaches only the timeline
#[derive(PartialEq)]
struct MediaViewerVideoDiscreteState {
    status: MediaViewerVideoControllerPlaybackStatus,
    is_playing: bool,
    is_buffering: bool,
    is_scrubbing: bool,
    duration: TimeInterval,
    buffering_fraction: Option<f64>,
}

pub struct MediaViewerVideoController {
    timeline: Entity<MediaViewerVideoTimeline>,
    player: Option<Entity<AVPlayer>>,
    source_url: String,
    status: MediaViewerVideoControllerPlaybackStatus,
    is_playing: bool,
    progress: TimeInterval,
    duration: TimeInterval,
    buffered_ahead: TimeInterval,
    buffered_fraction: Option<f64>,
    is_buffering: bool,
    playback_rate: f32,
    is_muted: bool,
    is_scrubbing: bool,

    scrub_resumes_playback: bool,
    pending_scrub_seconds: Option<TimeInterval>,
    is_scrub_seek_in_flight: bool,
    loops_playback: bool,
    loop_restart_pending: bool,
    scrub_resume_pending: bool,

    time_observer_token: Option<AVPlayerTimeObserver>,
    player_observation: Option<Subscription>,
    observed_player: MediaViewerVideoObservedPlayer,
    end_observer: Option<Subscription>,
    load_task: Option<Task<()>>,
    load_timeout_task: Option<Task<()>>,
    media_lease: Option<RemoteMediaFileLease>,
    streaming_asset: Option<RiseMediaStreamingAsset>,
    streaming_fallback_source: Option<Url>,
    playback_generation: u64,
    playback_owner_user_id: Option<String>,
    playback_owner_account_epoch: Option<String>,
    external_audio_owner: MusicExternalAudioOwner,
    holds_music_duck: bool,
    _background_observer: RiseSubscription,
    _background_pump: Task<()>,
}

impl MediaViewerVideoController {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (sender, mut inbox) = mpsc::unbounded::<()>();
        let background_observer = RiseAppEvents::default_center()
            .observe(UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION, move |_| {
                let _ = sender.unbounded_send(());
            });
        let background_pump = cx.spawn(async move |this, cx| {
            while inbox.next().await.is_some() {
                if this.update(cx, |this: &mut Self, cx: &mut Context<Self>| this.did_enter_background(cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            timeline: cx.new(|_| MediaViewerVideoTimeline),
            player: None,
            source_url: String::new(),
            status: MediaViewerVideoControllerPlaybackStatus::Idle,
            is_playing: false,
            progress: 0.0,
            duration: 0.0,
            buffered_ahead: 0.0,
            buffered_fraction: None,
            is_buffering: false,
            playback_rate: 1.0,
            is_muted: false,
            is_scrubbing: false,
            scrub_resumes_playback: false,
            pending_scrub_seconds: None,
            is_scrub_seek_in_flight: false,
            loops_playback: false,
            loop_restart_pending: false,
            scrub_resume_pending: false,
            time_observer_token: None,
            player_observation: None,
            observed_player: MediaViewerVideoObservedPlayer::default(),
            end_observer: None,
            load_task: None,
            load_timeout_task: None,
            media_lease: None,
            streaming_asset: None,
            streaming_fallback_source: None,
            playback_generation: 0,
            playback_owner_user_id: None,
            playback_owner_account_epoch: None,
            external_audio_owner: MusicExternalAudioOwner::unique("media-viewer"),
            holds_music_duck: false,
            _background_observer: background_observer,
            _background_pump: background_pump,
        }
    }

    fn did_enter_background(&mut self, cx: &mut Context<Self>) {
        self.scrub_resumes_playback = false;
        if self.is_playing || self.loop_restart_pending || self.scrub_resume_pending {
            tracing::info!(target: "riseonly::X1519", "media viewer video paused: app entered background");
            self.pause(cx);
        }
    }

    pub fn player(&self) -> Option<&Entity<AVPlayer>> {
        self.player.as_ref()
    }

    pub fn source_url(&self) -> &str {
        &self.source_url
    }

    pub fn status(&self) -> MediaViewerVideoControllerPlaybackStatus {
        self.status
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    pub fn progress(&self) -> TimeInterval {
        self.progress
    }

    pub fn duration(&self) -> TimeInterval {
        self.duration
    }

    pub fn buffered_ahead(&self) -> TimeInterval {
        self.buffered_ahead
    }

    pub fn buffered_fraction(&self) -> Option<f64> {
        self.buffered_fraction
    }

    pub fn is_buffering(&self) -> bool {
        self.is_buffering
    }

    pub fn playback_rate(&self) -> f32 {
        self.playback_rate
    }

    pub fn is_muted(&self) -> bool {
        self.is_muted
    }

    pub fn is_scrubbing(&self) -> bool {
        self.is_scrubbing
    }

    /// entity notified on playhead and buffer steps
    pub fn timeline(&self) -> &Entity<MediaViewerVideoTimeline> {
        &self.timeline
    }

    fn discrete_state(&self) -> MediaViewerVideoDiscreteState {
        MediaViewerVideoDiscreteState {
            status: self.status,
            is_playing: self.is_playing,
            is_buffering: self.is_buffering,
            is_scrubbing: self.is_scrubbing,
            duration: self.duration,
            buffering_fraction: self.buffered_fraction.filter(|_| self.is_buffering),
        }
    }

    /// notifies the controller only when what its readers show changed and the timeline always
    fn publish_step(&mut self, before: MediaViewerVideoDiscreteState, cx: &mut Context<Self>) {
        if self.discrete_state() != before {
            cx.notify();
        }
        self.timeline.update(cx, |_, cx| cx.notify());
    }

    pub fn fraction_complete(&self) -> f64 {
        if self.duration <= 0.0 {
            return 0.0;
        }
        (self.progress / self.duration).clamp(0.0, 1.0)
    }

    pub fn remaining(&self) -> TimeInterval {
        (self.duration - self.progress).max(0.0)
    }

    /// loads a video url, defaults are auto play true and persist true and no cache key
    pub fn load(
        &mut self,
        url: &str,
        auto_play: bool,
        persist_to_disk: bool,
        cache_key: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let normalized = RemoteMediaURL::optional_normalized(Some(url)).unwrap_or_else(|| url.to_owned());
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        if normalized.is_empty() {
            self.reset(cx);
            self.status = MediaViewerVideoControllerPlaybackStatus::Failed;
            cx.notify();
            return;
        }
        if self.source_url == normalized
            && self.player.is_some()
            && self.status != MediaViewerVideoControllerPlaybackStatus::Failed
            && self.playback_owner_user_id == account_user_id
            && self.playback_owner_account_epoch == account_epoch
            && SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
        {
            if auto_play && !self.is_playing {
                self.play(cx);
            }
            return;
        }

        self.reset(cx);
        let Some(source) = Url::from_string(&normalized) else {
            self.source_url = normalized;
            self.playback_owner_user_id = account_user_id;
            self.playback_owner_account_epoch = account_epoch;
            self.status = MediaViewerVideoControllerPlaybackStatus::Failed;
            cx.notify();
            return;
        };

        self.source_url = normalized.clone();
        self.status = MediaViewerVideoControllerPlaybackStatus::Loading;
        self.playback_owner_user_id = account_user_id.clone();
        self.playback_owner_account_epoch = account_epoch.clone();
        self.playback_generation = self.playback_generation.wrapping_add(1);
        let owner = MediaViewerVideoPlaybackOwner {
            generation: self.playback_generation,
            account_user_id: account_user_id.clone(),
            account_epoch: account_epoch.clone(),
        };
        cx.notify();
        self.arm_load_timeout(owner.clone(), cx);

        if source.is_file_url() {
            self.install_player_item(source, auto_play, &owner, cx);
            return;
        }
        if !persist_to_disk {
            self.install_remote_player_item(source, auto_play, owner, cx);
            return;
        }

        self.load_task = Some(cx.spawn(async move |this, cx| {
            let resolved_cache_key = cache_key.unwrap_or_else(|| RiseMediaCacheIdentity::key(&source));
            let lookup_source = source.clone();
            let lookup_key = resolved_cache_key.clone();
            let lookup_user_id = account_user_id.clone();
            let lookup_epoch = account_epoch.clone();
            let cached = cx
                .background_spawn(async move {
                    RemoteMediaDiskCache::shared().existing_local_url(
                        &lookup_source,
                        Some(lookup_key.as_str()),
                        lookup_user_id.as_deref(),
                        lookup_epoch.as_deref(),
                    )
                })
                .await;
            let current = this.update(cx, |this, cx| this.ensure_current_playback(&owner, cx));
            if !matches!(current, Ok(true)) {
                return;
            }
            if let Some(cached) = cached.and_then(|path| Url::from_file_path(&path)) {
                let installed = this.update(cx, |this, cx| {
                    this.load_task = None;
                    this.install_player_item(cached, auto_play, &owner, cx);
                });
                if installed.is_err() {
                    return;
                }
                RiseMediaTransferHub::shared()
                    .note(resolved_cache_key, 0, account_user_id.clone(), account_epoch.clone())
                    .await;
                return;
            }

            let offline = this.update(cx, |this, cx| {
                if !RiseTransportStatus::is_waiting_for_network(cx) {
                    return false;
                }
                tracing::warn!(target: "riseonly::X1531", "viewer video uncached while waiting for network: failed");
                this.load_task = None;
                this.load_timeout_task = None;
                this.status = MediaViewerVideoControllerPlaybackStatus::Failed;
                this.is_playing = false;
                this.is_buffering = false;
                cx.notify();
                true
            });
            if !matches!(offline, Ok(false)) {
                return;
            }

            let warmed =
                VideoWarmStartPrefetch::take_prepared_player(&normalized, account_user_id.clone(), account_epoch.clone())
                    .await;
            let installed = this.update(cx, |this, cx| {
                if !this.ensure_current_playback(&owner, cx) {
                    return false;
                }
                this.load_task = None;
                match warmed {
                    Some(warmed) => {
                        let item = warmed.item.clone();
                        warmed.player.update(cx, |player, cx| player.pause(cx));
                        drop(warmed);
                        let player = cx.new(AVPlayer::new);
                        this.install_player(player, item, auto_play, &owner, cx);
                    }
                    None => {
                        this.install_streaming_item(source.clone(), &resolved_cache_key, auto_play, &owner, cx);
                    }
                }
                true
            });
            if !matches!(installed, Ok(true)) {
                return;
            }
            let request = RiseMediaTransferRequest::new(
                source.clone(),
                RiseMediaTransferRequestArgs {
                    cache_key: Some(resolved_cache_key.clone()),
                    suggested_filename: Some(source.last_path_component()),
                    is_streamable: true,
                    account_user_id: account_user_id.clone(),
                    account_epoch: account_epoch.clone(),
                    ..RiseMediaTransferRequestArgs::default()
                },
            );
            cx.update(|cx| RiseMediaTransferHub::shared().start(request, false, cx));
        }));
    }

    fn install_streaming_item(
        &mut self,
        source: Url,
        cache_key: &str,
        auto_play: bool,
        owner: &MediaViewerVideoPlaybackOwner,
        cx: &mut Context<Self>,
    ) {
        let Some(streaming) = RiseMediaStreamingAsset::make(
            &source,
            cache_key,
            RemoteMediaDiskCacheLimits::default_limits().maximum_file_bytes,
            owner.account_user_id.clone(),
            owner.account_epoch.clone(),
        ) else {
            self.install_player_item(source, auto_play, owner, cx);
            return;
        };
        self.streaming_fallback_source = Some(source);
        let item = AVPlayerItem::with_asset(streaming.asset.clone());
        self.streaming_asset = Some(streaming);
        let player = cx.new(AVPlayer::new);
        self.install_player(player, item, auto_play, owner, cx);
    }

    fn fall_back_from_streaming(
        &mut self,
        owner: &MediaViewerVideoPlaybackOwner,
        auto_play: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(source) = self.streaming_fallback_source.take() else {
            return false;
        };
        if let Some(streaming) = self.streaming_asset.take() {
            streaming.teardown();
        }
        if !self.ensure_current_playback(owner, cx) {
            return false;
        }
        self.install_remote_player_item(source, auto_play, owner.clone(), cx);
        true
    }

    /// plays a remote source straight off the network, for media that must not hit the disk cache
    /// also the fallback from the streaming loader
    /// protected route gives 401 to a bare item so the signed address is resolved first
    fn install_remote_player_item(
        &mut self,
        source: Url,
        auto_play: bool,
        owner: MediaViewerVideoPlaybackOwner,
        cx: &mut Context<Self>,
    ) {
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let playable = RiseSecureMediaLocation::resolve(&source, cx).await.unwrap_or(source);
            this.update(cx, |this, cx| {
                if !this.ensure_current_playback(&owner, cx) {
                    return;
                }
                this.load_task = None;
                this.install_player_item(playable, auto_play, &owner, cx);
            })
            .ok();
        }));
    }

    fn install_player_item(
        &mut self,
        source: Url,
        auto_play: bool,
        owner: &MediaViewerVideoPlaybackOwner,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_current_playback(owner, cx) {
            return;
        }
        if let Some(streaming) = self.streaming_asset.take() {
            streaming.teardown();
        }
        let local_path = source.is_file_url().then(|| source.to_file_path()).flatten();
        self.media_lease = RemoteMediaDiskCache::lease(local_path.as_deref());
        let item = AVPlayerItem::new(source);
        let player = cx.new(AVPlayer::new);
        self.install_player(player, item, auto_play, owner, cx);
    }

    fn install_player(
        &mut self,
        player: Entity<AVPlayer>,
        mut item: AVPlayerItem,
        auto_play: bool,
        owner: &MediaViewerVideoPlaybackOwner,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_current_playback(owner, cx) {
            player.update(cx, |player, cx| player.pause(cx));
            return;
        }
        item.set_preferred_forward_buffer_duration(VideoWarmStartPolicy::PREFERRED_BUFFER_DURATION);
        let is_muted = self.is_muted;
        let playback_rate = self.playback_rate;
        let action_at_item_end =
            if self.loops_playback { AVPlayerActionAtItemEnd::None } else { AVPlayerActionAtItemEnd::Pause };
        let item_url = item.url().clone();
        player.update(cx, |player, cx| {
            player.set_automatically_waits_to_minimize_stalling(true, cx);
            player.set_is_muted(is_muted, cx);
        });
        RiseCallAppVolume::follow(&player, cx);
        player.update(cx, |player, cx| {
            player.set_default_rate(playback_rate, cx);
            player.set_action_at_item_end(action_at_item_end, cx);
            player.replace_current_item(Some(item), cx);
        });
        self.player = Some(player.clone());
        self.observed_player = MediaViewerVideoObservedPlayer::default();

        {
            let owner = owner.clone();
            self.player_observation = Some(cx.observe(&player, move |this, player, cx| {
                this.player_did_change(&player, &owner, auto_play, cx);
            }));
        }
        self.player_did_change(&player, owner, auto_play, cx);

        let interval = Duration::from_secs_f64(MediaViewerVideoTiming::PROGRESS_PUBLICATION);
        let weak = cx.weak_entity();
        let observed_player = player.downgrade();
        let time_owner = owner.clone();
        let block: Box<dyn Fn(f64, &mut App)> = Box::new(move |seconds, cx| {
            let Some(player) = observed_player.upgrade() else {
                return;
            };
            weak.update(cx, |this, cx| this.periodic_time_did_fire(&player, seconds, &time_owner, cx)).ok();
        });
        self.time_observer_token =
            Some(player.update(cx, |player, cx| player.add_periodic_time_observer(interval, block, cx)));

        let end_owner = owner.clone();
        self.end_observer = Some(cx.subscribe(&player, move |this, _, notification: &AVPlayerItemNotification, cx| {
            let AVPlayerItemNotification::DidPlayToEndTime { asset_url } = notification;
            if *asset_url == item_url {
                this.item_did_play_to_end(&end_owner, cx);
            }
        }));
        cx.notify();
    }

    fn player_did_change(
        &mut self,
        player: &Entity<AVPlayer>,
        owner: &MediaViewerVideoPlaybackOwner,
        auto_play: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_current_playback(owner, cx) {
            return;
        }
        let before = self.discrete_state();
        let (item_status, time_control_status, loaded_time_ranges, item_duration) = {
            let player = player.read(cx);
            (
                player.current_item_status(),
                player.time_control_status(),
                player.loaded_time_ranges(),
                player.current_item_duration(),
            )
        };

        if self.observed_player.item_status != Some(item_status) {
            self.observed_player.item_status = Some(item_status);
            match item_status {
                AVPlayerItemStatus::ReadyToPlay => {
                    self.status = MediaViewerVideoControllerPlaybackStatus::Ready;
                    if let Some(seconds) = item_duration.filter(|seconds| seconds.is_finite()) {
                        self.duration = seconds.max(0.0);
                    }
                    self.refresh_buffer_metrics(&loaded_time_ranges);
                    if auto_play {
                        self.play(cx);
                    }
                }
                AVPlayerItemStatus::Failed => {
                    if self.fall_back_from_streaming(owner, auto_play, cx) {
                        cx.notify();
                        return;
                    }
                    self.status = MediaViewerVideoControllerPlaybackStatus::Failed;
                    self.is_playing = false;
                    self.is_buffering = false;
                }
                AVPlayerItemStatus::Unknown => {
                    if self.status != MediaViewerVideoControllerPlaybackStatus::Failed {
                        self.status = MediaViewerVideoControllerPlaybackStatus::Loading;
                    }
                }
            }
        }

        if self.observed_player.loaded_time_ranges.as_ref() != Some(&loaded_time_ranges) {
            self.refresh_buffer_metrics(&loaded_time_ranges);
            self.observed_player.loaded_time_ranges = Some(loaded_time_ranges);
        }

        if self.observed_player.time_control_status != Some(time_control_status) {
            self.observed_player.time_control_status = Some(time_control_status);
            if !self.is_scrubbing && !self.scrub_resume_pending {
                self.is_buffering = time_control_status == AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate;
                self.is_playing = time_control_status == AVPlayerTimeControlStatus::Playing;
            }
        }
        self.publish_step(before, cx);
    }

    fn periodic_time_did_fire(
        &mut self,
        player: &Entity<AVPlayer>,
        seconds: f64,
        owner: &MediaViewerVideoPlaybackOwner,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_current_playback(owner, cx) {
            return;
        }
        let before = self.discrete_state();
        let (item_duration, loaded_time_ranges, time_control_status) = {
            let player = player.read(cx);
            (player.current_item_duration(), player.loaded_time_ranges(), player.time_control_status())
        };
        if let Some(item_duration) = item_duration.filter(|seconds| seconds.is_finite()) {
            self.duration = item_duration.max(0.0);
        }
        if self.is_scrubbing {
            self.publish_step(before, cx);
            return;
        }
        if seconds.is_finite() {
            self.progress = seconds.max(0.0);
        }
        self.refresh_buffer_metrics(&loaded_time_ranges);
        if !self.scrub_resume_pending {
            self.is_playing = time_control_status == AVPlayerTimeControlStatus::Playing;
        }
        self.publish_step(before, cx);
    }

    fn item_did_play_to_end(&mut self, owner: &MediaViewerVideoPlaybackOwner, cx: &mut Context<Self>) {
        if !self.ensure_current_playback(owner, cx) {
            return;
        }
        if self.is_scrubbing {
            return;
        }
        if self.loops_playback {
            self.restart_loop(owner, cx);
            return;
        }
        self.is_playing = false;
        self.is_buffering = false;
        self.progress = self.duration;
        self.release_music_duck(cx);
        cx.notify();
    }

    pub fn set_loops_playback(&mut self, loops: bool, cx: &mut Context<Self>) {
        self.loops_playback = loops;
        if let Some(player) = &self.player {
            let action = if loops { AVPlayerActionAtItemEnd::None } else { AVPlayerActionAtItemEnd::Pause };
            player.update(cx, |player, cx| player.set_action_at_item_end(action, cx));
        }
    }

    fn restart_loop(&mut self, owner: &MediaViewerVideoPlaybackOwner, cx: &mut Context<Self>) {
        let Some(player) = self.player.clone() else {
            return;
        };
        self.progress = 0.0;
        self.loop_restart_pending = true;
        cx.notify();
        let weak = cx.weak_entity();
        let owner = owner.clone();
        let completion: Box<dyn FnOnce(bool, &mut App)> = Box::new(move |finished, cx| {
            weak.update(cx, |this, cx| {
                let resumes = this.loop_restart_pending;
                this.loop_restart_pending = false;
                if !resumes || !finished || !this.ensure_current_playback(&owner, cx) || this.is_scrubbing {
                    return;
                }
                let Some(player) = this.player.clone() else {
                    return;
                };
                let rate = this.playback_rate;
                player.update(cx, |player, cx| player.play_immediately(rate, cx));
                this.is_playing = true;
                cx.notify();
            })
            .ok();
        });
        player.update(cx, |player, cx| player.seek(0.0, 0.0, 0.0, Some(completion), cx));
    }

    fn refresh_buffer_metrics(&mut self, loaded_time_ranges: &[CMTimeRange]) {
        let metrics = MediaViewerVideoBufferMetrics::resolve(loaded_time_ranges, self.progress, self.duration);
        self.buffered_ahead = metrics.buffered_ahead;
        match (self.buffered_fraction, metrics.fraction) {
            (None, None) => {}
            (Some(old), Some(new)) if (old - new).abs() < 0.005 => {}
            _ => self.buffered_fraction = metrics.fraction,
        }
    }

    fn arm_load_timeout(&mut self, owner: MediaViewerVideoPlaybackOwner, cx: &mut Context<Self>) {
        self.load_timeout_task = None;
        let buffered_mark = self.loaded_seconds(cx);
        self.load_timeout_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(MediaViewerVideoTiming::LOAD_TIMEOUT))
                .await;
            this.update(cx, |this, cx| {
                if !this.ensure_current_playback(&owner, cx)
                    || this.status != MediaViewerVideoControllerPlaybackStatus::Loading
                {
                    return;
                }
                let waiting_for_network = RiseTransportStatus::is_waiting_for_network(cx);
                let has_open_requests = !waiting_for_network
                    && this.streaming_asset.as_ref().is_some_and(|asset| asset.has_open_requests());
                if has_open_requests || this.loaded_seconds(cx) > buffered_mark {
                    this.arm_load_timeout(owner.clone(), cx);
                    return;
                }
                this.status = MediaViewerVideoControllerPlaybackStatus::Failed;
                this.is_buffering = false;
                cx.notify();
            })
            .ok();
        }));
    }

    fn loaded_seconds(&self, cx: &App) -> f64 {
        let Some(player) = &self.player else {
            return 0.0;
        };
        let player = player.read(cx);
        if player.current_item().is_none() {
            return 0.0;
        }
        player.loaded_time_ranges().iter().fold(0.0, |total, range| {
            let seconds = range.duration;
            if seconds.is_finite() { total + seconds.max(0.0) } else { total }
        })
    }

    fn ensure_current_playback(&mut self, owner: &MediaViewerVideoPlaybackOwner, cx: &mut Context<Self>) -> bool {
        if owner.generation != self.playback_generation {
            return false;
        }
        if self.playback_owner_user_id != owner.account_user_id
            || self.playback_owner_account_epoch != owner.account_epoch
        {
            return false;
        }
        if !SaiAccountCacheScope::is_current_account_epoch(
            owner.account_user_id.as_deref(),
            owner.account_epoch.as_deref(),
        ) {
            self.reset(cx);
            return false;
        }
        true
    }

    fn current_owner(&self) -> MediaViewerVideoPlaybackOwner {
        MediaViewerVideoPlaybackOwner {
            generation: self.playback_generation,
            account_user_id: self.playback_owner_user_id.clone(),
            account_epoch: self.playback_owner_account_epoch.clone(),
        }
    }

    pub fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        if self.is_playing {
            self.pause(cx);
        } else {
            self.play(cx);
        }
    }

    pub fn play(&mut self, cx: &mut Context<Self>) {
        let Some(player) = self.player.clone() else {
            return;
        };
        if self.status != MediaViewerVideoControllerPlaybackStatus::Ready {
            return;
        }
        if RiseAppLifecycle::scene_phase(cx) == ScenePhase::Background {
            return;
        }
        if !self.is_muted {
            let weak = cx.weak_entity();
            let retry: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
                weak.update(cx, |this, cx| this.play(cx)).ok();
            });
            let token = self.external_audio_owner.clone();
            let allowed = MusicListeningInterruptionGate::shared(cx)
                .update(cx, |gate, cx| gate.request_external_playback(Some(token), false, retry, None, cx));
            if !allowed {
                return;
            }
            self.acquire_music_duck(cx);
        }
        if self.duration > 0.0 && self.progress >= self.duration - 0.05 {
            player.update(cx, |player, cx| player.seek(0.0, 0.0, 0.0, None, cx));
            self.progress = 0.0;
        }
        let rate = self.playback_rate;
        player.update(cx, |player, cx| player.play_immediately(rate, cx));
        self.is_playing = true;
        cx.notify();
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.loop_restart_pending = false;
        self.scrub_resume_pending = false;
        if let Some(player) = &self.player {
            player.update(cx, |player, cx| player.pause(cx));
        }
        self.is_playing = false;
        self.is_buffering = false;
        self.release_music_duck(cx);
        cx.notify();
    }

    fn acquire_music_duck(&mut self, cx: &mut Context<Self>) {
        if self.holds_music_duck {
            return;
        }
        self.holds_music_duck = true;
        let owner = self.external_audio_owner.clone();
        MusicPlayerState::shared(cx).update(cx, |state, cx| state.suspend_for_external_audio(owner, true, cx));
    }

    fn release_music_duck(&mut self, cx: &mut Context<Self>) {
        if !self.holds_music_duck {
            return;
        }
        self.holds_music_duck = false;
        let owner = self.external_audio_owner.clone();
        MusicPlayerState::shared(cx).update(cx, |state, cx| state.resume_after_external_audio(owner, cx));
    }

    pub fn begin_scrubbing(&mut self, cx: &mut Context<Self>) {
        let Some(player) = self.player.clone() else {
            return;
        };
        if self.duration <= 0.0 || self.is_scrubbing {
            return;
        }
        self.is_scrubbing = true;
        self.scrub_resumes_playback = self.is_playing;
        self.scrub_resume_pending = false;
        self.pending_scrub_seconds = None;
        player.update(cx, |player, cx| player.set_rate(0.0, cx));
        cx.notify();
    }

    pub fn scrub(&mut self, to_fraction: f64, cx: &mut Context<Self>) {
        if !self.is_scrubbing || self.duration <= 0.0 {
            return;
        }
        let target = self.duration * to_fraction.clamp(0.0, 1.0);
        self.progress = target;
        self.pending_scrub_seconds = Some(target);
        self.drive_scrub_seek(cx);
        cx.notify();
    }

    pub fn end_scrubbing(&mut self, at_fraction: f64, cx: &mut Context<Self>) {
        if !self.is_scrubbing {
            return;
        }
        self.is_scrubbing = false;
        self.pending_scrub_seconds = None;
        self.scrub_resume_pending = self.scrub_resumes_playback;
        self.scrub_resumes_playback = false;
        let Some(player) = self.player.clone().filter(|_| self.duration > 0.0) else {
            self.scrub_resume_pending = false;
            cx.notify();
            return;
        };

        let target = self.duration * at_fraction.clamp(0.0, 1.0);
        self.progress = target;
        let owner = self.current_owner();
        let weak = cx.weak_entity();
        let completion: Box<dyn FnOnce(bool, &mut App)> = Box::new(move |_, cx| {
            weak.update(cx, |this, cx| {
                this.is_scrub_seek_in_flight = false;
                let resumes = this.scrub_resume_pending;
                this.scrub_resume_pending = false;
                if !this.ensure_current_playback(&owner, cx) {
                    return;
                }
                if this.is_scrubbing {
                    return;
                }
                if let Some(player) = this.player.clone() {
                    let loaded_time_ranges = player.read(cx).loaded_time_ranges();
                    this.refresh_buffer_metrics(&loaded_time_ranges);
                }
                let Some(player) = this.player.clone().filter(|_| resumes) else {
                    cx.notify();
                    return;
                };
                let rate = this.playback_rate;
                player.update(cx, |player, cx| player.play_immediately(rate, cx));
                this.is_playing = true;
                cx.notify();
            })
            .ok();
        });
        player.update(cx, |player, cx| player.seek(target, 0.0, 0.0, Some(completion), cx));
        cx.notify();
    }

    /// one seek at a time, whatever the pointer reached meanwhile becomes the next target
    /// so no queue of stale positions is left after release
    fn drive_scrub_seek(&mut self, cx: &mut Context<Self>) {
        if !self.is_scrubbing || self.is_scrub_seek_in_flight {
            return;
        }
        let Some(player) = self.player.clone() else {
            return;
        };
        let Some(target) = self.pending_scrub_seconds.take() else {
            return;
        };
        self.is_scrub_seek_in_flight = true;
        let tolerance = MediaViewerVideoTiming::SCRUB_SEEK_TOLERANCE;
        let owner = self.current_owner();
        let weak = cx.weak_entity();
        let completion: Box<dyn FnOnce(bool, &mut App)> = Box::new(move |_, cx| {
            weak.update(cx, |this, cx| {
                this.is_scrub_seek_in_flight = false;
                if !this.ensure_current_playback(&owner, cx) {
                    return;
                }
                this.drive_scrub_seek(cx);
            })
            .ok();
        });
        player.update(cx, |player, cx| player.seek(target, tolerance, tolerance, Some(completion), cx));
    }

    /// keyboard seek with the arrow keys, one press and release of the seek bar
    pub fn step_seek(&mut self, by_seconds: TimeInterval, cx: &mut Context<Self>) {
        if self.duration <= 0.0 {
            return;
        }
        let target = (self.progress + by_seconds) / self.duration;
        self.begin_scrubbing(cx);
        self.end_scrubbing(target.clamp(0.0, 1.0), cx);
    }

    pub fn set_playback_rate(&mut self, rate: f32, cx: &mut Context<Self>) {
        let resolved = rate.clamp(0.5, 2.0);
        self.playback_rate = resolved;
        let is_playing = self.is_playing;
        if let Some(player) = &self.player {
            player.update(cx, |player, cx| {
                player.set_default_rate(resolved, cx);
                if is_playing {
                    player.set_rate(resolved, cx);
                }
            });
        }
        cx.notify();
    }

    pub fn toggle_muted(&mut self, cx: &mut Context<Self>) {
        if self.is_muted && self.is_playing {
            let weak = cx.weak_entity();
            let retry: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
                weak.update(cx, |this, cx| this.toggle_muted(cx)).ok();
            });
            let token = self.external_audio_owner.clone();
            let allowed = MusicListeningInterruptionGate::shared(cx)
                .update(cx, |gate, cx| gate.request_external_playback(Some(token), false, retry, None, cx));
            if !allowed {
                return;
            }
        }
        self.is_muted = !self.is_muted;
        let is_muted = self.is_muted;
        if let Some(player) = &self.player {
            player.update(cx, |player, cx| player.set_is_muted(is_muted, cx));
        }
        cx.notify();
        if !self.is_playing {
            return;
        }
        if self.is_muted {
            self.release_music_duck(cx);
        } else {
            self.acquire_music_duck(cx);
        }
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        let token = self.external_audio_owner.clone();
        MusicListeningInterruptionGate::shared(cx).update(cx, |gate, cx| gate.invalidate_prompt(token, cx));
        self.release_music_duck(cx);
        self.playback_generation = self.playback_generation.wrapping_add(1);
        self.load_task = None;
        self.load_timeout_task = None;
        if let (Some(token), Some(player)) = (self.time_observer_token.take(), &self.player) {
            player.update(cx, |player, _| player.remove_time_observer(token));
        }
        self.end_observer = None;
        if let Some(player) = self.player.take() {
            player.update(cx, |player, cx| player.pause(cx));
        }
        self.media_lease = None;
        if let Some(streaming) = self.streaming_asset.take() {
            streaming.teardown();
        }
        self.streaming_fallback_source = None;
        self.source_url = String::new();
        self.playback_owner_user_id = None;
        self.playback_owner_account_epoch = None;
        self.status = MediaViewerVideoControllerPlaybackStatus::Idle;
        self.is_playing = false;
        self.is_buffering = false;
        self.is_scrubbing = false;
        self.is_scrub_seek_in_flight = false;
        self.pending_scrub_seconds = None;
        self.scrub_resumes_playback = false;
        self.scrub_resume_pending = false;
        self.loop_restart_pending = false;
        self.progress = 0.0;
        self.duration = 0.0;
        self.buffered_ahead = 0.0;
        self.buffered_fraction = None;
        self.time_observer_token = None;
        self.player_observation = None;
        self.observed_player = MediaViewerVideoObservedPlayer::default();
        cx.notify();
    }
}

/// toggle for the center playback control
pub type MediaViewerPlaybackToggle = Rc<dyn Fn(&mut Window, &mut App)>;

pub const KEY_CONTEXT: &str = "MediaViewerVideoPlayer";

const KEYBOARD_SEEK_STEP_SECONDS: TimeInterval = 5.0;

actions!(media_viewer_video_player, [TogglePlayback, SeekBackward, SeekForward]);

struct MediaViewerVideoPlayerKeyBindingsInstalled;

impl Global for MediaViewerVideoPlayerKeyBindingsInstalled {}

/// safe to call twice, every render of the video player already calls it
pub fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<MediaViewerVideoPlayerKeyBindingsInstalled>() {
        return;
    }
    cx.set_global(MediaViewerVideoPlayerKeyBindingsInstalled);
    let context = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("space", TogglePlayback, context),
        KeyBinding::new("left", SeekBackward, context),
        KeyBinding::new("right", SeekForward, context),
    ]);
}

#[derive(IntoElement)]
pub struct MediaViewerVideoPlayer {
    pub id: ElementId,
    pub url: String,
    pub thumbnail_url: Option<String>,
    pub aspect_ratio: f32,
    pub is_current: bool,
    pub fills_screen: bool,
    pub controller: Entity<MediaViewerVideoController>,
    pub persist_poster_to_disk: bool,
    pub poster_cache_policy: CleverImageCachePolicy,
    pub playback_control: Option<MediaZoomViewBuilder>,
    pub playback_control_label: String,
    pub on_playback_toggle: MediaViewerPlaybackToggle,
    pub on_zoom_changed: MediaZoomChanged,
    pub on_single_tap: MediaZoomSingleTap,
    pub has_previous_page: bool,
    pub has_next_page: bool,
}

impl MediaViewerVideoPlayer {
    pub const PLAYBACK_CONTROL_SIZE: f32 = 96.0;

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }

    fn poster(&self) -> AnyElement {
        let source = RemoteMediaURL::display_source(&self.url, self.thumbnail_url.as_deref());
        if source.is_empty() {
            return div().absolute().inset_0().bg(black()).into_any_element();
        }
        let content_mode = if self.fills_screen { ContentMode::Fill } else { ContentMode::Fit };
        let aspect_ratio = if self.aspect_ratio > 0.0 { self.aspect_ratio } else { 16.0 / 9.0 };
        let ratio_box = match content_mode {
            ContentMode::Fit => div().max_w_full().max_h_full().w_full(),
            ContentMode::Fill => div().min_w_full().min_h_full().h_full(),
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .child(
                ratio_box
                    .flex_none()
                    .aspect_ratio(aspect_ratio)
                    .overflow_hidden()
                    .child(
                        CleverImageUi::new(source)
                            .id(self.child_id("media-viewer-video-poster"))
                            .content_mode(content_mode)
                            .corner_radius(0.0)
                            .persist_to_disk(self.persist_poster_to_disk)
                            .maximum_decode_pixel_dimension(2_048_i64)
                            .cache_policy(self.poster_cache_policy)
                            .placeholder_color(black()),
                    ),
            )
            .into_any_element()
    }

    fn buffering_indicator(fraction: Option<f64>) -> AnyElement {
        let indicator = match fraction {
            Some(fraction) => {
                let percent = (fraction * 100.0).round() as i64;
                div()
                    .id("media-viewer-video-buffering")
                    .size(px(30.0))
                    .aria_label(format!("{}, {percent}%", tr("media_viewer_video_buffering")))
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, (), window, _| paint_buffer_ring(bounds, fraction as f32, window),
                        )
                        .size_full(),
                    )
                    .into_any_element()
            }
            None => div()
                .id("media-viewer-video-buffering")
                .aria_label(tr("media_viewer_video_buffering"))
                .child(progress_view().tint(white()).control_size(ControlSize::Large))
                .into_any_element(),
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(px(64.0))
                    .rounded_full()
                    .bg(black().opacity(0.45))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(indicator),
            )
            .into_any_element()
    }

    fn failure_badge() -> AnyElement {
        let white = white();
        let icon = match IconUi::sized("exclamationmark.triangle.fill", px(26.0), white, true) {
            Some(icon) => icon.into_any_element(),
            None => div().size(px(26.0)).into_any_element(),
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(18.0))
                    .py(px(14.0))
                    .rounded(px(18.0))
                    .bg(black().opacity(0.45))
                    .text_color(white)
                    .child(icon)
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr("media_viewer_video_load_failed")),
                    ),
            )
            .into_any_element()
    }
}

impl RenderOnce for MediaViewerVideoPlayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let normalized_url = RemoteMediaURL::optional_normalized(Some(self.url.as_str())).unwrap_or_else(|| self.url.clone());
        let (owns_playback, player, status, is_buffering, buffered_fraction) = {
            let controller = self.controller.read(cx);
            let owns_playback = self.is_current && controller.source_url() == normalized_url;
            (
                owns_playback,
                controller.player().cloned(),
                controller.status(),
                controller.is_buffering(),
                controller.buffered_fraction(),
            )
        };

        install_key_bindings(cx);
        let focus_handle = window
            .use_keyed_state(self.child_id("media-viewer-video-focus"), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        {
            let focus_handle = focus_handle.clone();
            on_change(self.child_id("media-viewer-video-focus-current"), self.is_current, true, window, cx, move |_, current, window, cx| {
                if *current {
                    window.focus(&focus_handle, cx);
                }
            });
        }

        {
            let on_zoom_changed = Rc::clone(&self.on_zoom_changed);
            on_change(self.child_id("media-viewer-video-current"), self.is_current, false, window, cx, move |_, current, _, cx| {
                if !*current {
                    on_zoom_changed(false, cx);
                }
            });
        }

        let surface = MediaViewerZoomableVideoSurface {
            id: self.child_id("media-viewer-zoomable-video-surface"),
            player: if owns_playback { player } else { None },
            aspect_ratio: self.aspect_ratio,
            fills_screen: self.fills_screen,
            is_current: self.is_current,
            playback_control: self.playback_control.clone(),
            playback_control_label: self.playback_control_label.clone(),
            on_playback_toggle: Rc::clone(&self.on_playback_toggle),
            on_zoom_changed: Rc::clone(&self.on_zoom_changed),
            on_single_tap: Rc::clone(&self.on_single_tap),
        };

        let shows_buffering = owns_playback
            && (status == MediaViewerVideoControllerPlaybackStatus::Loading || is_buffering);
        let shows_failure = owns_playback && status == MediaViewerVideoControllerPlaybackStatus::Failed;

        let on_playback_toggle = Rc::clone(&self.on_playback_toggle);
        let seek_back_controller = self.controller.clone();
        let seek_forward_controller = self.controller.clone();
        let (has_previous_page, has_next_page) = if LayoutDirection::current().is_right_to_left() {
            (self.has_next_page, self.has_previous_page)
        } else {
            (self.has_previous_page, self.has_next_page)
        };

        div()
            .id(self.child_id("media-viewer-video-player"))
            .key_context(KEY_CONTEXT)
            .track_focus(&focus_handle)
            .when(owns_playback, |this| {
                this.on_action(move |_: &TogglePlayback, window, cx| on_playback_toggle(window, cx))
                    .on_action(move |_: &SeekBackward, window, cx| {
                        if has_previous_page {
                            window.dispatch_action(Box::new(PreviousPage), cx);
                            return;
                        }
                        seek_back_controller.update(cx, |controller, cx| controller.step_seek(-KEYBOARD_SEEK_STEP_SECONDS, cx));
                    })
                    .on_action(move |_: &SeekForward, window, cx| {
                        if has_next_page {
                            window.dispatch_action(Box::new(NextPage), cx);
                            return;
                        }
                        seek_forward_controller.update(cx, |controller, cx| controller.step_seek(KEYBOARD_SEEK_STEP_SECONDS, cx));
                    })
            })
            .relative()
            .size_full()
            .child(self.poster())
            .child(div().absolute().inset_0().child(surface))
            .when(shows_buffering, |this| this.child(Self::buffering_indicator(buffered_fraction)))
            .when(shows_failure, |this| this.child(Self::failure_badge()))
    }
}

fn paint_buffer_ring(bounds: Bounds<Pixels>, fraction: f32, window: &mut Window) {
    const LINE_WIDTH: f32 = 3.0;
    let side = f32::from(bounds.size.width.min(bounds.size.height));
    let radius = (side - LINE_WIDTH) / 2.0;
    let center = bounds.center();
    let track = white().opacity(0.28);
    let head = white();
    stroke_arc(&arc_points(center, radius, 0.0, 2.0 * PI), LINE_WIDTH, true, track, window);
    let sweep = 2.0 * PI * fraction.clamp(0.0, 1.0);
    if sweep <= 0.0 {
        return;
    }
    let start = -PI / 2.0;
    let points = arc_points(center, radius, start, sweep);
    stroke_arc(&points, LINE_WIDTH, false, head, window);
    if let (Some(first), Some(last)) = (points.first(), points.last()) {
        round_cap(*first, LINE_WIDTH, head, window);
        round_cap(*last, LINE_WIDTH, head, window);
    }
}

/// points along a circle from start through sweep radians, clockwise on a y down screen
fn arc_points(center: Point<Pixels>, radius: f32, start: f32, sweep: f32) -> Vec<Point<Pixels>> {
    let steps = ((sweep.abs() / (2.0 * PI)) * 96.0).ceil().max(2.0) as usize;
    (0..=steps)
        .map(|step| {
            let angle = start + sweep * step as f32 / steps as f32;
            point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
        })
        .collect()
}

fn stroke_arc(points: &[Point<Pixels>], line_width: f32, closed: bool, color: Hsla, window: &mut Window) {
    let mut builder = gpui::PathBuilder::stroke(px(line_width));
    builder.add_polygon(points, closed);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// round line cap, gpui strokes have butt ends so each end gets a dot of the line width
fn round_cap(at: Point<Pixels>, line_width: f32, color: Hsla, window: &mut Window) {
    let half = line_width / 2.0;
    let bounds = Bounds::new(point(at.x - px(half), at.y - px(half)), size(px(line_width), px(line_width)));
    window.paint_quad(fill(bounds, color).corner_radii(px(half)));
}

#[derive(IntoElement)]
struct MediaViewerZoomableVideoSurface {
    id: ElementId,
    player: Option<Entity<AVPlayer>>,
    aspect_ratio: f32,
    fills_screen: bool,
    is_current: bool,
    playback_control: Option<MediaZoomViewBuilder>,
    playback_control_label: String,
    on_playback_toggle: MediaViewerPlaybackToggle,
    on_zoom_changed: MediaZoomChanged,
    on_single_tap: MediaZoomSingleTap,
}

impl MediaViewerZoomableVideoSurface {
    fn make_ui_view(id: ElementId, window: &mut Window, cx: &mut App) -> Entity<MediaViewerVideoZoomContainerView> {
        window.use_keyed_state(id, cx, |_, cx| MediaViewerVideoZoomContainerView::new(cx))
    }

    fn update_ui_view(self, ui_view: &Entity<MediaViewerVideoZoomContainerView>, cx: &mut App) {
        ui_view.update(cx, |ui_view, cx| {
            let container = ui_view.container.clone();
            container.update(cx, |container, _| {
                container.on_zoom_changed = Some(Rc::clone(&self.on_zoom_changed));
                container.on_single_tap = Some(Rc::clone(&self.on_single_tap));
            });
            ui_view.configure_playback_control(
                self.playback_control,
                self.playback_control_label,
                self.on_playback_toggle,
                cx,
            );
            let animated = ui_view.has_configured;
            ui_view.has_configured = true;
            ui_view.configure(self.player, self.aspect_ratio, self.fills_screen, animated, cx);
            if !self.is_current {
                container.update(cx, |container, cx| container.reset_zoom(false, cx));
            }
        });
    }
}

impl RenderOnce for MediaViewerZoomableVideoSurface {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ui_view = Self::make_ui_view(self.id.clone(), window, cx);
        self.update_ui_view(&ui_view, cx);
        div().size_full().child(ui_view)
    }
}

struct MediaViewerVideoZoomContainerView {
    container: Entity<MediaZoomContainerView>,
    playback_button: Entity<MediaViewerPinchPlaybackButton>,
    player: Option<Entity<AVPlayer>>,
    video_gravity: AVLayerVideoGravity,
    presentation_size_observation: Option<Subscription>,
    presentation_size: Size<f32>,
    fallback_aspect: f32,
    fills_screen: bool,
    applied_natural_size: Size<f32>,
    applied_fills_screen: bool,
    fit_fill_landing: Option<Task<()>>,
    has_configured: bool,
}

impl MediaViewerVideoZoomContainerView {
    const FIT_FILL_DURATION: TimeInterval = 0.2;

    fn new(cx: &mut Context<Self>) -> Self {
        let player_view = cx.weak_entity();
        let zoomed_view: MediaZoomViewBuilder = Rc::new(move |_: &mut Window, cx: &mut App| {
            let Some(view) = player_view.upgrade() else {
                return div().into_any_element();
            };
            let (player, gravity) = {
                let view = view.read(cx);
                (view.player.clone(), view.video_gravity)
            };
            AVPlayerLayer::for_player(player)
                .video_gravity(gravity)
                .into_any_element()
        });
        let playback_button = cx.new(MediaViewerPinchPlaybackButton::new);
        let container = cx.new(|_| MediaZoomContainerView::new(zoomed_view));
        let control_button = playback_button.clone();
        let control: MediaZoomViewBuilder =
            Rc::new(move |_: &mut Window, _: &mut App| control_button.clone().into_any_element());
        container.update(cx, |container, cx| container.install_viewport_control(control, cx));
        Self {
            container,
            playback_button,
            player: None,
            video_gravity: AVLayerVideoGravity::ResizeAspect,
            presentation_size_observation: None,
            presentation_size: Size::default(),
            fallback_aspect: 16.0 / 9.0,
            fills_screen: false,
            applied_natural_size: Size::default(),
            applied_fills_screen: false,
            fit_fill_landing: None,
            has_configured: false,
        }
    }

    fn configure_playback_control(
        &mut self,
        content: Option<MediaZoomViewBuilder>,
        label: String,
        action: MediaViewerPlaybackToggle,
        cx: &mut Context<Self>,
    ) {
        self.playback_button
            .update(cx, |button, cx| button.configure(content, label, action, cx));
    }

    fn configure(
        &mut self,
        player: Option<Entity<AVPlayer>>,
        aspect_ratio: f32,
        fills_screen: bool,
        animated: bool,
        cx: &mut Context<Self>,
    ) {
        let same_player = match (&self.player, &player) {
            (Some(current), Some(next)) => current.entity_id() == next.entity_id(),
            (None, None) => true,
            _ => false,
        };
        if !same_player {
            self.player = player.clone();
            self.observe_presentation_size(player, cx);
        }
        self.fallback_aspect = if aspect_ratio > 0.0 { aspect_ratio } else { 16.0 / 9.0 };
        self.fills_screen = fills_screen;
        self.apply_geometry(animated, cx);
    }

    fn observe_presentation_size(&mut self, player: Option<Entity<AVPlayer>>, cx: &mut Context<Self>) {
        self.presentation_size_observation = None;
        self.presentation_size = Size::default();
        let Some(player) = player else {
            return;
        };
        let player_id = player.entity_id();
        self.presentation_size_observation = Some(cx.observe(&player, move |this, player, cx| {
            let is_current = this.player.as_ref().is_some_and(|current| current.entity_id() == player_id);
            if !is_current {
                return;
            }
            let size = player.read(cx).presentation_size();
            if size == this.presentation_size {
                return;
            }
            this.presentation_size = size;
            this.apply_geometry(false, cx);
        }));
        self.presentation_size = player.read(cx).presentation_size();
    }

    /// applied only on a real change, every reset returns the zoom to rest
    /// the host calls configure on each pass of the page
    fn apply_geometry(&mut self, animated: bool, cx: &mut Context<Self>) {
        let natural = if self.presentation_size.width > 1.0 && self.presentation_size.height > 1.0 {
            self.presentation_size
        } else {
            Size { width: self.fallback_aspect, height: 1.0 }
        };
        let mode_changed = self.applied_fills_screen != self.fills_screen;
        if natural == self.applied_natural_size && !mode_changed {
            return;
        }
        self.applied_natural_size = natural;
        self.applied_fills_screen = self.fills_screen;
        let fills_screen = self.fills_screen;
        self.container
            .update(cx, |container, cx| container.set_content_geometry(natural, fills_screen, cx));

        if !animated || !mode_changed {
            self.fit_fill_landing = None;
            self.video_gravity =
                if fills_screen { AVLayerVideoGravity::ResizeAspectFill } else { AVLayerVideoGravity::ResizeAspect };
            cx.notify();
            return;
        }
        self.video_gravity = AVLayerVideoGravity::ResizeAspectFill;
        cx.notify();
        self.fit_fill_landing = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(Self::FIT_FILL_DURATION))
                .await;
            this.update(cx, |this, cx| {
                this.fit_fill_landing = None;
                if this.applied_fills_screen {
                    return;
                }
                this.video_gravity = AVLayerVideoGravity::ResizeAspect;
                cx.notify();
            })
            .ok();
        }));
    }
}

impl Render for MediaViewerVideoZoomContainerView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.container.clone())
    }
}

/// playback button owned by the zoom container so its
/// pinch reaches the container even from the button
/// the press is cancelled when zoom takes over
struct MediaViewerPinchPlaybackButton {
    hosted_content: Option<MediaZoomViewBuilder>,
    action: Option<MediaViewerPlaybackToggle>,
    accessibility_label: String,
    is_hidden: bool,
    is_highlighted: bool,
    press_scale: Animated<f32>,
    bounds: Bounds<Pixels>,
}

impl MediaViewerPinchPlaybackButton {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            hosted_content: None,
            action: None,
            accessibility_label: String::new(),
            is_hidden: true,
            is_highlighted: false,
            press_scale: Animated::new(1.0),
            bounds: Bounds::default(),
        }
    }

    fn configure(
        &mut self,
        content: Option<MediaZoomViewBuilder>,
        label: String,
        action: MediaViewerPlaybackToggle,
        cx: &mut Context<Self>,
    ) {
        let is_hidden = content.is_none();
        let changed = is_hidden != self.is_hidden || label != self.accessibility_label;
        self.action = content.as_ref().map(|_| action);
        self.accessibility_label = label;
        self.is_hidden = is_hidden;
        if is_hidden {
            self.set_highlighted(false, cx);
        }
        self.hosted_content = content;
        if changed {
            cx.notify();
        }
    }

    fn set_highlighted(&mut self, highlighted: bool, cx: &mut Context<Self>) {
        if self.is_highlighted == highlighted {
            return;
        }
        self.is_highlighted = highlighted;
        if highlighted {
            selection_changed();
        }
        let (target, spec) = if highlighted {
            (LiquidGlassPressMetrics::PRESS_SCALE, ease_in_out(LiquidGlassPressMetrics::PRESS_IN_RESPONSE))
        } else {
            (1.0, ease_in_out(LiquidGlassPressMetrics::RELEASE_RESPONSE))
        };
        self.press_scale.set_in(target, spec, RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn activate_playback(&mut self, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_hidden {
            return;
        }
        if let ClickEvent::Mouse(mouse) = event
            && !Self::point(self.bounds, mouse.up.position)
        {
            return;
        }
        if let Some(action) = self.action.clone() {
            action(window, cx);
        }
    }

    /// hit test of the oval, a press in the corners of the square frame misses
    fn point(bounds: Bounds<Pixels>, inside: Point<Pixels>) -> bool {
        let center = bounds.center();
        let radius_x = f32::from(bounds.size.width) / 2.0;
        let radius_y = f32::from(bounds.size.height) / 2.0;
        if radius_x <= 0.0 || radius_y <= 0.0 {
            return false;
        }
        let dx = f32::from(inside.x - center.x) / radius_x;
        let dy = f32::from(inside.y - center.y) / radius_y;
        dx * dx + dy * dy <= 1.0
    }
}

impl Render for MediaViewerPinchPlaybackButton {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let side = px(MediaViewerVideoPlayer::PLAYBACK_CONTROL_SIZE);
        if self.is_hidden {
            return div().into_any_element();
        }
        let scale = self.press_scale.frame_value(RiseClock::monotonic(), window);
        let content = self
            .hosted_content
            .clone()
            .map(|content| content(window, cx))
            .unwrap_or_else(|| div().into_any_element());
        let scaled_side = side * scale;
        let this_view = cx.weak_entity();
        div()
            .id("media-viewer-center-playback")
            .aria_label(self.accessibility_label.clone())
            .size(side)
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .child(
                div()
                    .size(scaled_side)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(content),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| this.set_highlighted(true, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.set_highlighted(false, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.set_highlighted(false, cx)),
            )
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                this.activate_playback(event, window, cx);
            }))
            .child(measure("media-viewer-center-playback-bounds", move |bounds, _, cx| {
                let _ = this_view.update(cx, |this, _| this.bounds = bounds);
            }))
            .into_any_element()
    }
}

