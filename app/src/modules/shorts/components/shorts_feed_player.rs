use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, AppContext, Bounds, Context, ElementId, Entity, EntityId, EventEmitter, FontWeight,
    IntoElement,
    ParentElement, PathBuilder, Pixels, Render, RenderOnce, Size, StyleRefinement, Styled, Subscription, Task, WeakEntity, Window, black,
    canvas, div, point, px, size, white,
};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_media::video::av_player::{
    AVLayerVideoGravity, AVPlayer, AVPlayerActionAtItemEnd, AVPlayerItem, AVPlayerItemNotification, AVPlayerItemStatus,
    AVPlayerLayer, AVPlayerTimeControlStatus, AVPlayerTimeObserver,
};
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::shapes::{SwiftUiShapes, pass_through};
use rise_ui::{IconUi, MainText};

use crate::core::lib::rise_call_app_volume::RiseCallAppVolume;
use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheMaterializeOptions, RemoteMediaFileLease,
};
use crate::core::media::rise_media_foreground_gate::RiseMediaForegroundGate;
use crate::core::media::video_warm_start_prefetch::{VideoWarmPlayerBox, VideoWarmStartPrefetch};
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RISE_ENGINE_CONNECTION_READY;
use crate::core::shared_element_transition::renderer::shared_element_video_frame_grabber::SharedElementPlayerLayerView;
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi};
use crate::modules::analytics::stores::engagement::engagement_actions::engagement_types::EngagementSurface;
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;
use crate::modules::music::shared::utils::music::music_listening_interruption_gate::MusicListeningInterruptionGate;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::post::components::post::post_music_playback_controller::PostMusicPlaybackController;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

/// the one reel allowed to play across navigators of the window
/// the right panel stays drawn so a reel left there keeps playing
/// a starting reel claims focus, previous owner pauses
struct ShortsPlaybackFocus {
    owner: Option<EntityId>,
    waiting: Vec<EntityId>,
}

impl ShortsPlaybackFocus {
    fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { owner: None, waiting: Vec::new() }
    }

    fn owner(&self) -> Option<EntityId> {
        self.owner
    }

    fn claim(&mut self, id: EntityId, cx: &mut Context<Self>) {
        self.waiting.retain(|waiting| *waiting != id);
        if self.owner == Some(id) {
            return;
        }
        if let Some(previous) = self.owner.replace(id) {
            self.waiting.push(previous);
        }
        tracing::info!(target: "riseonly::X1637", ?id, waiting = self.waiting.len(), "reel focus claimed");
        cx.notify();
    }

    /// handover waits a turn so a pager swapping pages doesnt play the waiting reel for one frame
    fn release(&mut self, id: EntityId, cx: &mut Context<Self>) {
        self.waiting.retain(|waiting| *waiting != id);
        if self.owner != Some(id) {
            return;
        }
        self.owner = None;
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = weak.update(cx, |this, cx| {
                if this.owner.is_none() {
                    this.owner = this.waiting.pop();
                    if this.owner.is_some() {
                        tracing::info!(target: "riseonly::X1637", owner = ?this.owner, "reel focus handed back");
                        cx.notify();
                    }
                }
            });
        });
    }
}

#[derive(Default)]
struct PlaybackGate {
    should_play: bool,
    is_muted: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ShortsMusicOverlay {
    pub post_id: String,
    pub url: Url,
    pub track_id: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
}

impl GetPostFeedResponse {
    pub fn shorts_music_overlay(&self) -> Option<ShortsMusicOverlay> {
        if self.audio_baked {
            return None;
        }
        let url = self.music_audio_url()?;
        Some(ShortsMusicOverlay {
            post_id: self.id.clone(),
            url,
            track_id: self.track_id.clone(),
            start_ms: self.music_segment_start_ms(),
            end_ms: self.music_segment_end_ms(),
        })
    }
}

#[derive(Default)]
pub struct ShortsPlaybackController {
    current_time: f64,
    duration: f64,
    is_scrubbing: bool,
    player: Option<WeakEntity<AVPlayer>>,
    player_layer_view: Option<SharedElementPlayerLayerView>,
    is_held: bool,
}

/// event when set_held changed the hold
pub struct ShortsPlaybackHoldChanged;

impl EventEmitter<ShortsPlaybackHoldChanged> for ShortsPlaybackController {}

/// event when set_current_time moved the playhead
pub struct ShortsPlaybackTimeChanged;

impl EventEmitter<ShortsPlaybackTimeChanged> for ShortsPlaybackController {}

impl ShortsPlaybackController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current_time(&self) -> f64 {
        self.current_time
    }

    pub fn duration(&self) -> f64 {
        self.duration
    }

    pub fn is_scrubbing(&self) -> bool {
        self.is_scrubbing
    }

    pub fn player(&self) -> Option<Entity<AVPlayer>> {
        self.player.as_ref().and_then(WeakEntity::upgrade)
    }

    pub fn is_held(&self) -> bool {
        self.is_held
    }

    /// a held player pauses at once, the hidden page view wont be drawn to pass inactive
    pub fn set_held(&mut self, held: bool, cx: &mut Context<Self>) {
        if self.is_held != held {
            self.is_held = held;
            cx.emit(ShortsPlaybackHoldChanged);
        }
    }

    pub fn player_layer_view(&self) -> Option<SharedElementPlayerLayerView> {
        self.player_layer_view.clone()
    }

    pub fn set_player_layer_view(&mut self, view: Option<SharedElementPlayerLayerView>, cx: &mut Context<Self>) {
        let same = match (&self.player_layer_view, &view) {
            (Some(shown), Some(next)) => shown.same_layer(next),
            (None, None) => true,
            _ => false,
        };
        self.player_layer_view = view;
        if !same {
            cx.notify();
        }
    }

    pub fn set_current_time(&mut self, current_time: f64, cx: &mut Context<Self>) {
        if self.current_time != current_time {
            self.current_time = current_time;
            cx.emit(ShortsPlaybackTimeChanged);
        }
    }

    pub fn set_duration(&mut self, duration: f64, cx: &mut Context<Self>) {
        if self.duration != duration {
            self.duration = duration;
            cx.notify();
        }
    }

    pub fn set_is_scrubbing(&mut self, is_scrubbing: bool, cx: &mut Context<Self>) {
        if self.is_scrubbing != is_scrubbing {
            self.is_scrubbing = is_scrubbing;
            cx.notify();
        }
    }

    pub fn set_player(&mut self, player: Option<WeakEntity<AVPlayer>>, cx: &mut Context<Self>) {
        let changed = self.player.as_ref().map(|player| player.entity_id()) != player.as_ref().map(|player| player.entity_id());
        self.player = player;
        if changed {
            cx.notify();
        }
    }

    pub fn progress(&self) -> f64 {
        if self.duration <= 0.0 || !self.current_time.is_finite() {
            return 0.0;
        }
        (self.current_time / self.duration).clamp(0.0, 1.0)
    }

    pub fn seek(&mut self, to_fraction: f64, precise: bool, cx: &mut Context<Self>) {
        let Some(player) = self.player() else {
            return;
        };
        if self.duration <= 0.0 {
            return;
        }
        self.current_time = to_fraction.clamp(0.0, 1.0) * self.duration;
        let time = self.current_time;
        let tolerance = if precise { 0.0 } else { f64::INFINITY };
        player.update(cx, |player, cx| player.seek(time, tolerance, tolerance, None, cx));
        cx.notify();
    }
}

pub type ShortsFeedPlayerAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type ShortsFeedPlayerAspect = Rc<dyn Fn(f32, &mut Window, &mut App)>;

#[derive(Clone)]
struct ShortsFeedPlayerProps {
    url: String,
    thumbnail_url: Option<String>,
    is_active: bool,
    shows_thumbnail_while_loading: bool,
    post_id: Option<String>,
    author_id: Option<String>,
    is_muted: bool,
    paused: bool,
    playback: Option<Entity<ShortsPlaybackController>>,
    gravity_override: Option<AVLayerVideoGravity>,
    draws_opaque_background: bool,
    initial_aspect: Option<f32>,
    corner_radius: f32,
    loops: bool,
    on_ended: Option<ShortsFeedPlayerAction>,
    on_aspect: Option<ShortsFeedPlayerAspect>,
    on_poster_aspect: Option<ShortsFeedPlayerAspect>,
    music_overlay: Option<ShortsMusicOverlay>,
    persists_post_media_to_disk: bool,
    post_image_cache_policy: CleverImageCachePolicy,
    releases_player_on_disappear: bool,
    on_audio_declined: Option<ShortsFeedPlayerAction>,
}

#[derive(IntoElement)]
pub struct ShortsFeedPlayer {
    id: ElementId,
    props: ShortsFeedPlayerProps,
}

impl ShortsFeedPlayer {
    pub const FILL_ASPECT_LIMIT: f32 = 0.6;

    pub fn new(id: impl Into<ElementId>, url: impl Into<String>, thumbnail_url: Option<String>, is_active: bool) -> Self {
        Self {
            id: id.into(),
            props: ShortsFeedPlayerProps {
                url: url.into(),
                thumbnail_url,
                is_active,
                shows_thumbnail_while_loading: true,
                post_id: None,
                author_id: None,
                is_muted: false,
                paused: false,
                playback: None,
                gravity_override: None,
                draws_opaque_background: true,
                initial_aspect: None,
                corner_radius: 0.0,
                loops: true,
                on_ended: None,
                on_aspect: None,
                on_poster_aspect: None,
                music_overlay: None,
                persists_post_media_to_disk: true,
                post_image_cache_policy: CleverImageCachePolicy::Standard,
                releases_player_on_disappear: false,
                on_audio_declined: None,
            },
        }
    }

    pub fn gravity(for_aspect: f32) -> AVLayerVideoGravity {
        if for_aspect <= Self::FILL_ASPECT_LIMIT {
            AVLayerVideoGravity::ResizeAspectFill
        } else {
            AVLayerVideoGravity::ResizeAspect
        }
    }

    pub fn shows_thumbnail_while_loading(mut self, value: bool) -> Self {
        self.props.shows_thumbnail_while_loading = value;
        self
    }

    pub fn post_id(mut self, value: Option<String>) -> Self {
        self.props.post_id = value;
        self
    }

    pub fn author_id(mut self, value: Option<String>) -> Self {
        self.props.author_id = value;
        self
    }

    pub fn is_muted(mut self, value: bool) -> Self {
        self.props.is_muted = value;
        self
    }

    pub fn paused(mut self, value: bool) -> Self {
        self.props.paused = value;
        self
    }

    pub fn playback(mut self, value: Option<Entity<ShortsPlaybackController>>) -> Self {
        self.props.playback = value;
        self
    }

    pub fn gravity_override(mut self, value: Option<AVLayerVideoGravity>) -> Self {
        self.props.gravity_override = value;
        self
    }

    pub fn draws_opaque_background(mut self, value: bool) -> Self {
        self.props.draws_opaque_background = value;
        self
    }

    pub fn initial_aspect(mut self, value: Option<f32>) -> Self {
        self.props.initial_aspect = value;
        self
    }

    pub fn corner_radius(mut self, value: f32) -> Self {
        self.props.corner_radius = value;
        self
    }

    pub fn loops(mut self, value: bool) -> Self {
        self.props.loops = value;
        self
    }

    pub fn on_ended(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.props.on_ended = Some(Rc::new(action));
        self
    }

    pub fn on_aspect(mut self, action: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.props.on_aspect = Some(Rc::new(action));
        self
    }

    pub fn on_poster_aspect(mut self, action: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.props.on_poster_aspect = Some(Rc::new(action));
        self
    }

    pub fn music_overlay(mut self, value: Option<ShortsMusicOverlay>) -> Self {
        self.props.music_overlay = value;
        self
    }

    pub fn persists_post_media_to_disk(mut self, value: bool) -> Self {
        self.props.persists_post_media_to_disk = value;
        self
    }

    pub fn post_image_cache_policy(mut self, value: CleverImageCachePolicy) -> Self {
        self.props.post_image_cache_policy = value;
        self
    }

    pub fn releases_player_on_disappear(mut self, value: bool) -> Self {
        self.props.releases_player_on_disappear = value;
        self
    }

    pub fn on_audio_declined(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.props.on_audio_declined = Some(Rc::new(action));
        self
    }
}

impl RenderOnce for ShortsFeedPlayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let props = self.props;
        let initial = props.clone();
        let state = window.with_global_id(self.id, |global_id, window| {
            window.with_element_state(global_id, |state: Option<Entity<ShortsFeedPlayerState>>, _| {
                let state = state.unwrap_or_else(|| {
                    cx.new(move |cx: &mut Context<ShortsFeedPlayerState>| ShortsFeedPlayerState::new(initial, cx))
                });
                (state.clone(), state)
            })
        });
        let handle = window.window_handle();
        state.update(cx, |state, cx| {
            state.window = Some(handle);
            state.apply(props, cx);
        });
        div().size_full().child(AnyView::from(state).cached(StyleRefinement::default().size_full()).replay_when_moved())
    }
}

/// state of the player view, its own view since a playing layer redraws at display rate
struct ShortsFeedPlayerState {
    props: ShortsFeedPlayerProps,
    window: Option<AnyWindowHandle>,
    is_appeared: bool,
    player: Option<Entity<AVPlayer>>,
    media_lease: Option<RemoteMediaFileLease>,
    external_audio_owner: MusicExternalAudioOwner,
    end_observer: Option<Subscription>,
    player_observation: Option<Subscription>,
    time_observer: Option<AVPlayerTimeObserver>,
    music_time_observer: Option<AVPlayerTimeObserver>,
    watchdog: Option<Task<()>>,
    landing_task: Option<Task<()>>,
    video_aspect: Option<f32>,
    gate: PlaybackGate,
    replay_count: i64,
    watch_emitted: bool,
    is_preparing_player: bool,
    setup_generation: u64,
    setup_task: Option<Task<()>>,
    layer_ready: bool,
    poster_aspect: Option<f32>,
    unmuted_while_paused: bool,
    last_time_control_status: Option<AVPlayerTimeControlStatus>,
    retried_after_failure: bool,
    resume_time: Option<f64>,
    did_fail: bool,
    player_layer_view: Option<(SharedElementPlayerLayerView, AVLayerVideoGravity)>,
    hold_observer: Option<(EntityId, Subscription)>,
    preempted: bool,
    _focus_observer: Subscription,
    _foreground_observer: Subscription,
    _connection_ready_observer: RiseSubscription,
    _connection_ready_events: Task<()>,
    _release: Subscription,
}

impl ShortsFeedPlayerState {
    fn new(props: ShortsFeedPlayerProps, cx: &mut Context<Self>) -> Self {
        let foreground_gate = RiseMediaForegroundGate::shared(cx);
        let foreground_observer = cx.observe(&foreground_gate, |this, _, cx| {
            if this.is_appeared {
                this.sync_playback(cx);
            }
        });
        let focus = ShortsPlaybackFocus::shared(cx);
        let focus_observer = cx.observe(&focus, |this, focus, cx| this.focus_did_change(&focus, cx));
        let entity_id = cx.entity_id();

        let (sender, mut receiver) = mpsc::unbounded::<()>();
        let connection_ready_observer = RiseAppEvents::default_center().observe(RISE_ENGINE_CONNECTION_READY, move |_| {
            let _ = sender.unbounded_send(());
        });
        let connection_ready_events = cx.spawn(async move |this, cx| {
            while receiver.next().await.is_some() {
                if this.update(cx, |this, cx| this.rebuild_if_load_failed(true, cx)).is_err() {
                    return;
                }
            }
        });

        Self {
            props,
            window: None,
            is_appeared: false,
            player: None,
            media_lease: None,
            external_audio_owner: MusicExternalAudioOwner::unique("shorts"),
            end_observer: None,
            player_observation: None,
            time_observer: None,
            music_time_observer: None,
            watchdog: None,
            landing_task: None,
            video_aspect: None,
            gate: PlaybackGate::default(),
            replay_count: 0,
            watch_emitted: false,
            is_preparing_player: false,
            setup_generation: 0,
            setup_task: None,
            layer_ready: false,
            poster_aspect: None,
            unmuted_while_paused: false,
            last_time_control_status: None,
            retried_after_failure: false,
            resume_time: None,
            did_fail: false,
            player_layer_view: None,
            hold_observer: None,
            preempted: false,
            _focus_observer: focus_observer,
            _foreground_observer: foreground_observer,
            _connection_ready_observer: connection_ready_observer,
            _connection_ready_events: connection_ready_events,
            _release: cx.on_release(move |this, cx| {
                this.release_external_audio(cx);
                ShortsPlaybackFocus::shared(cx).update(cx, |focus, cx| focus.release(entity_id, cx));
            }),
        }
    }

    fn observe_hold(&mut self, cx: &mut Context<Self>) {
        let playback = self.props.playback.clone();
        let playback_id = playback.as_ref().map(Entity::entity_id);
        if self.hold_observer.as_ref().map(|(id, _)| *id) == playback_id {
            return;
        }
        self.hold_observer = playback.map(|playback| {
            let subscription = cx.subscribe(&playback, |this, _, _: &ShortsPlaybackHoldChanged, cx| {
                if this.is_appeared {
                    this.sync_playback(cx);
                }
            });
            (playback.entity_id(), subscription)
        });
    }

    /// runs on the props of each new frame
    fn apply(&mut self, props: ShortsFeedPlayerProps, cx: &mut Context<Self>) {
        let old_is_active = self.props.is_active;
        let old_paused = self.props.paused;
        let old_is_muted = self.props.is_muted;
        let url_changed = self.props.url != props.url;
        let layout_changed = self.props.gravity_override != props.gravity_override
            || self.props.draws_opaque_background != props.draws_opaque_background
            || self.props.corner_radius != props.corner_radius
            || self.props.initial_aspect != props.initial_aspect
            || self.props.shows_thumbnail_while_loading != props.shows_thumbnail_while_loading
            || self.props.thumbnail_url != props.thumbnail_url
            || self.props.post_image_cache_policy != props.post_image_cache_policy;
        self.props = props;
        self.observe_hold(cx);
        if layout_changed {
            cx.notify();
        }
        if !self.is_appeared {
            if url_changed {
                self.teardown(cx);
                self.poster_aspect = None;
            }
            return;
        }

        if old_is_active != self.props.is_active {
            let active = self.props.is_active;
            if self.props.is_active && !self.props.paused {
                self.claim_focus(cx);
            } else if !active {
                self.release_focus(cx);
            }
            if active {
                self.retried_after_failure = false;
                self.rebuild_if_load_failed(false, cx);
            }
            self.sync_playback(cx);
            if active {
                self.watch_emitted = false;
                self.replay_count = 0;
            } else {
                self.emit_video_watch_if_needed(cx);
            }
        }
        if old_paused != self.props.paused {
            if !self.props.paused && self.props.is_active {
                self.claim_focus(cx);
            }
            self.sync_playback(cx);
        }
        if url_changed {
            self.emit_video_watch_if_needed(cx);
            self.teardown(cx);
            if self.props.is_active && !self.props.paused {
                self.claim_focus(cx);
            }
            self.gate.should_play = self.should_play(cx);
            self.replay_count = 0;
            self.watch_emitted = false;
            self.poster_aspect = None;
            self.unmuted_while_paused = false;
            self.retried_after_failure = false;
            self.resume_time = None;
            self.setup_if_needed(cx);
        }
        if old_is_muted != self.props.is_muted {
            let muted = self.props.is_muted;
            self.gate.is_muted = muted;
            if let Some(player) = self.player.clone() {
                player.update(cx, |player, cx| player.set_is_muted(muted, cx));
            }
            let should_play = self.should_play(cx);
            self.unmuted_while_paused = !muted && !should_play;
            if !muted && should_play {
                self.activate_playback_audio(false, cx);
            }
            self.push_music_state(cx);
        }
    }

    /// backgrounded app plays nothing,
    /// music and calls keep the process alive so nothing else would stop a reel
    fn should_play(&self, cx: &mut App) -> bool {
        self.props.is_active
            && !self.props.paused
            && !self.props.playback.as_ref().is_some_and(|playback| playback.read(cx).is_held())
            && RiseMediaForegroundGate::shared(cx).read(cx).is_foreground()
            && !self.preempted
    }

    fn claim_focus(&mut self, cx: &mut Context<Self>) {
        self.preempted = false;
        let id = cx.entity_id();
        ShortsPlaybackFocus::shared(cx).update(cx, |focus, cx| focus.claim(id, cx));
    }

    fn release_focus(&mut self, cx: &mut Context<Self>) {
        self.preempted = false;
        let id = cx.entity_id();
        ShortsPlaybackFocus::shared(cx).update(cx, |focus, cx| focus.release(id, cx));
    }

    fn focus_did_change(&mut self, focus: &Entity<ShortsPlaybackFocus>, cx: &mut Context<Self>) {
        if !self.is_appeared {
            return;
        }
        let owner = focus.read(cx).owner();
        let me = cx.entity_id();
        let preempted = if owner == Some(me) {
            false
        } else {
            owner.is_some() && (self.preempted || self.gate.should_play)
        };
        if preempted != self.preempted {
            tracing::info!(target: "riseonly::X1637", ?me, preempted, "reel focus changed");
            self.preempted = preempted;
            self.sync_playback(cx);
        }
    }

    /// uncover only once the frame cant move anymore, layer has a picture and final gravity
    fn display_ready(&self) -> bool {
        self.player.is_some() && self.layer_ready && (self.props.gravity_override.is_some() || self.video_aspect.is_some())
    }

    /// aspect from the caller first,
    /// then the poster,
    /// else fill the page since most reels are 9:16
    fn current_gravity(&self) -> AVLayerVideoGravity {
        if let Some(gravity_override) = self.props.gravity_override {
            return gravity_override;
        }
        match self.video_aspect.or(self.props.initial_aspect).or(self.poster_aspect) {
            Some(aspect) => ShortsFeedPlayer::gravity(aspect),
            None if self.props.draws_opaque_background => AVLayerVideoGravity::ResizeAspectFill,
            None => AVLayerVideoGravity::ResizeAspect,
        }
    }

    /// runs the closure on the main thread after this update, with the window it asked from
    fn deliver(&self, action: impl FnOnce(&mut Window, &mut App) + 'static, cx: &mut Context<Self>) {
        let Some(handle) = self.window else {
            return;
        };
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| action(window, cx));
        });
    }

    fn appear(&mut self, cx: &mut Context<Self>) {
        self.is_appeared = true;
        if self.props.is_active && !self.props.paused {
            self.claim_focus(cx);
        }
        self.gate.should_play = self.should_play(cx);
        self.gate.is_muted = self.props.is_muted;
        self.setup_if_needed(cx);
        self.ensure_time_observer(cx);
        self.ensure_music_observer(cx);
        if self.gate.should_play {
            self.start_watchdog(cx);
            if self.player.is_some() {
                self.play(cx);
            }
        }
    }

    fn disappear(&mut self, cx: &mut Context<Self>) {
        self.is_appeared = false;
        self.release_focus(cx);
        self.gate.should_play = false;
        self.remove_time_observer();
        self.remove_music_observer();
        self.stop_music(cx);
        self.stop_watchdog();
        self.pause(cx);
        self.invalidate_external_audio(cx);
        self.emit_video_watch_if_needed(cx);
        if self.props.releases_player_on_disappear {
            self.teardown(cx);
            if let Some(playback) = self.props.playback.clone() {
                playback.update(cx, |playback, cx| {
                    playback.set_current_time(0.0, cx);
                    playback.set_duration(0.0, cx);
                });
            }
        }
    }

    fn invalidate_external_audio(&self, cx: &mut App) {
        let token = self.external_audio_owner.clone();
        MusicListeningInterruptionGate::shared(cx).update(cx, |gate, cx| gate.invalidate_prompt(token, cx));
        let owner = self.external_audio_owner.clone();
        MusicPlayerState::shared(cx).update(cx, |state, cx| state.resume_after_external_audio(owner, cx));
    }

    fn release_external_audio(&self, cx: &mut App) {
        if let Some(overlay) = &self.props.music_overlay {
            let post_id = overlay.post_id.clone();
            PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| controller.stop(&post_id, cx));
        }
        self.invalidate_external_audio(cx);
    }

    fn sync_playback(&mut self, cx: &mut Context<Self>) {
        self.gate.should_play = self.should_play(cx);
        if self.gate.should_play {
            self.play(cx);
            self.start_watchdog(cx);
        } else {
            self.stop_watchdog();
            self.pause(cx);
        }
        self.push_music_state(cx);
        let is_held = self.props.playback.as_ref().is_some_and(|playback| playback.read(cx).is_held());
        if !self.props.is_active || is_held {
            tracing::info!(target: "riseonly::X1715", owner = %self.external_audio_owner.raw_value, is_held, "riff released music");
            self.invalidate_external_audio(cx);
        }
    }

    fn rebuild_if_load_failed(&mut self, including_unready: bool, cx: &mut Context<Self>) {
        let Some(player) = self.player.clone() else {
            return;
        };
        let failed = {
            let player = player.read(cx);
            let status = player.current_item_status();
            player.error().is_some()
                || player.current_item().is_none()
                || status == AVPlayerItemStatus::Failed
                || (including_unready && status != AVPlayerItemStatus::ReadyToPlay)
        };
        if !failed {
            return;
        }
        self.retried_after_failure = false;
        self.teardown(cx);
        self.gate.should_play = self.should_play(cx);
        self.setup_if_needed(cx);
    }

    fn handle_playback_failure(&mut self, generation: u64, position: Option<f64>, cx: &mut Context<Self>) {
        if generation != self.setup_generation || self.player.is_none() {
            return;
        }
        if self.retried_after_failure {
            if !self.did_fail {
                self.did_fail = true;
                cx.notify();
            }
            return;
        }
        self.retried_after_failure = true;
        self.teardown(cx);
        self.gate.should_play = self.should_play(cx);
        self.resume_time = position.filter(|position| position.is_finite() && *position > 0.0);
        self.setup_if_needed(cx);
    }

    fn watch_stream_landing(
        &mut self,
        remote: Url,
        options: RemoteMediaDiskCacheMaterializeOptions,
        cx: &mut Context<Self>,
    ) {
        let generation = self.setup_generation;
        self.landing_task = Some(cx.spawn(async move |this, cx| {
            let mut waited = Duration::ZERO;
            while waited < Self::LANDING_JOIN {
                cx.background_executor().timer(Self::LANDING_POLL).await;
                waited += Self::LANDING_POLL;
                let waiting = |this: &Self| generation == this.setup_generation && !this.layer_ready && this.player.is_some();
                if !this.read_with(cx, |this, _| waiting(this)).unwrap_or(false) {
                    return;
                }
                let local = if waited >= Self::LANDING_JOIN {
                    RemoteMediaDiskCache::shared().ensure_cached(&remote, options.clone(), cx).await
                } else {
                    let remote = remote.clone();
                    let options = options.clone();
                    cx.background_executor().spawn(async move {
                        RemoteMediaDiskCache::shared().existing_local_url(
                            &remote,
                            options.cache_key.as_deref(),
                            options.expected_account_user_id.as_deref(),
                            options.expected_account_epoch.as_deref(),
                        )
                    })
                    .await
                };
                let Some(local) = local.as_deref().and_then(Url::from_file_path) else {
                    continue;
                };
                let _ = this.update(cx, |this, cx| {
                    if !waiting(this) {
                        return;
                    }
                    if let Some(task) = this.landing_task.take() {
                        task.detach();
                    }
                    this.teardown(cx);
                    this.gate.should_play = this.should_play(cx);
                    this.install_player(local, cx);
                });
                return;
            }
        }));
    }

    const LANDING_POLL: Duration = Duration::from_secs(1);
    const LANDING_JOIN: Duration = Duration::from_secs(6);

    fn setup_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.player.is_some() || self.is_preparing_player {
            return;
        }
        let Some(resolved) = RemoteMediaURL::optional_normalized(Some(self.props.url.as_str())) else {
            return;
        };
        let Some(video_url) = Url::from_string(&resolved) else {
            return;
        };
        if video_url.is_file_url() {
            self.install_player(video_url, cx);
            return;
        }

        self.is_preparing_player = true;
        self.setup_generation = self.setup_generation.wrapping_add(1);
        let generation = self.setup_generation;
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let authorization_token = AuthServicesStore::shared(cx).read(cx).tokens_and_other_data().access_token.clone();
        let persists_to_disk = self.props.persists_post_media_to_disk;
        let cache_key = self.props.post_id.as_ref().map(|post_id| format!("short-{post_id}"));
        let lookup = persists_to_disk.then(|| {
            let remote = video_url.clone();
            let cache_key = cache_key.clone();
            let account_user_id = account_user_id.clone();
            let account_epoch = account_epoch.clone();
            cx.background_spawn(async move {
                RemoteMediaDiskCache::shared().existing_local_url(
                    &remote,
                    cache_key.as_deref(),
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                )
            })
        });
        let options = RemoteMediaDiskCacheMaterializeOptions {
            cache_key,
            suggested_filename: Some(video_url.last_path_component()),
            authorization_token: Some(authorization_token),
            expected_account_user_id: account_user_id.clone(),
            expected_account_epoch: account_epoch.clone(),
            ..RemoteMediaDiskCacheMaterializeOptions::default()
        };
        self.setup_task = Some(cx.spawn(async move |this, cx| {
            let cached = match lookup {
                Some(lookup) => lookup.await,
                None => None,
            };
            let finished = this
                .update(cx, |this, cx| {
                    if generation != this.setup_generation {
                        return true;
                    }
                    let Some(cached) = cached.as_deref().and_then(Url::from_file_path) else {
                        return false;
                    };
                    this.finish_setup_task();
                    this.is_preparing_player = false;
                    this.install_player(cached, cx);
                    true
                })
                .unwrap_or(true);
            if finished {
                return;
            }

            let warmed =
                VideoWarmStartPrefetch::take_prepared_player(&resolved, account_user_id.clone(), account_epoch.clone())
                    .await;
            let installed = this
                .update(cx, |this, cx| {
                    if generation != this.setup_generation {
                        return false;
                    }
                    this.finish_setup_task();
                    this.is_preparing_player = false;
                    match warmed {
                        Some(warmed) => this.install_warm_player(warmed, cx),
                        None => {
                            this.install_player(video_url.clone(), cx);
                            if persists_to_disk {
                                this.watch_stream_landing(video_url.clone(), options.clone(), cx);
                            }
                        }
                    }
                    true
                })
                .unwrap_or(false);
            if !installed || !persists_to_disk {
                return;
            }
            cx.spawn(async move |cx| {
                let _ = RemoteMediaDiskCache::shared().ensure_cached(&video_url, options, cx).await;
            })
            .detach();
        }));
    }

    fn finish_setup_task(&mut self) {
        if let Some(task) = self.setup_task.take() {
            task.detach();
        }
    }

    fn install_warm_player(&mut self, warmed: VideoWarmPlayerBox, cx: &mut Context<Self>) {
        if self.player.is_some() {
            return;
        }
        self.media_lease = RemoteMediaDiskCache::lease(warmed.source.to_file_path().as_deref());
        let item = warmed.item.clone();
        warmed.player.update(cx, |player, cx| player.pause(cx));
        drop(warmed);
        let player = cx.new(AVPlayer::new);
        self.install(player, item, cx);
    }

    fn install_player(&mut self, video_url: Url, cx: &mut Context<Self>) {
        if self.player.is_some() {
            return;
        }
        self.media_lease = RemoteMediaDiskCache::lease(video_url.to_file_path().as_deref());
        let player = cx.new(AVPlayer::new);
        player.update(cx, |player, cx| player.set_automatically_waits_to_minimize_stalling(true, cx));
        self.install(player, AVPlayerItem::new(video_url), cx);
    }

    fn install(&mut self, player: Entity<AVPlayer>, item: AVPlayerItem, cx: &mut Context<Self>) {
        let item_url = item.url().clone();
        let is_muted = self.props.is_muted;
        let resume_time = self.resume_time.take();
        player.update(cx, |player, cx| {
            player.set_is_muted(is_muted, cx);
            player.set_action_at_item_end(AVPlayerActionAtItemEnd::None, cx);
            player.replace_current_item(Some(item), cx);
            if let Some(resume_time) = resume_time {
                player.seek(resume_time, f64::INFINITY, f64::INFINITY, None, cx);
            }
        });
        RiseCallAppVolume::follow(&player, cx);
        self.end_observer = Some(cx.subscribe(&player, move |this, player, notification: &AVPlayerItemNotification, cx| {
            let AVPlayerItemNotification::DidPlayToEndTime { asset_url } = notification;
            if *asset_url == item_url {
                this.item_did_play_to_end(&player, cx);
            }
        }));
        self.last_time_control_status = None;
        self.player_observation = Some(cx.observe(&player, |this, player, cx| this.player_did_change(&player, cx)));
        self.player = Some(player.clone());
        if let Some(playback) = self.props.playback.clone() {
            let weak = player.downgrade();
            playback.update(cx, |playback, cx| playback.set_player(Some(weak), cx));
        }
        self.ensure_time_observer(cx);
        self.ensure_music_observer(cx);
        if self.gate.should_play {
            self.start_watchdog(cx);
        }
        self.player_did_change(&player, cx);
        if self.gate.should_play {
            self.play(cx);
        }
        cx.notify();
    }

    fn item_did_play_to_end(&mut self, player: &Entity<AVPlayer>, cx: &mut Context<Self>) {
        if self.props.loops {
            self.replay_count += 1;
            let should_play = self.gate.should_play;
            player.update(cx, |player, cx| {
                player.seek(0.0, f64::INFINITY, f64::INFINITY, None, cx);
                if should_play {
                    player.play(cx);
                }
            });
        } else if let Some(on_ended) = self.props.on_ended.clone() {
            self.deliver(move |window, cx| on_ended(window, cx), cx);
        }
    }

    fn player_did_change(&mut self, player: &Entity<AVPlayer>, cx: &mut Context<Self>) {
        if self.player.as_ref().is_some_and(|current| current.entity_id() != player.entity_id()) {
            return;
        }
        let (ready, presentation_size, duration, time_control_status, failed_at) = {
            let player = player.read(cx);
            (
                player.is_ready_for_display(),
                player.presentation_size(),
                player.current_item_duration(),
                player.time_control_status(),
                (player.current_item_status() == AVPlayerItemStatus::Failed).then(|| player.current_time()),
            )
        };
        if let Some(position) = failed_at {
            let generation = self.setup_generation;
            self.handle_playback_failure(generation, position, cx);
            return;
        }

        if self.layer_ready != ready {
            self.layer_ready = ready;
            cx.notify();
        }

        let stalled = self.last_time_control_status == Some(AVPlayerTimeControlStatus::Playing)
            && time_control_status == AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate;
        self.last_time_control_status = Some(time_control_status);
        if stalled && self.gate.should_play {
            player.update(cx, |player, cx| player.play(cx));
        }

        if let (Some(duration), Some(playback)) = (duration, self.props.playback.clone())
            && duration.is_finite()
            && duration > 0.0
        {
            playback.update(cx, |playback, cx| playback.set_duration(duration, cx));
        }

        if self.video_aspect.is_none() && presentation_size.width > 0.0 && presentation_size.height > 0.0 {
            let aspect = presentation_size.width / presentation_size.height;
            self.video_aspect = Some(aspect);
            cx.notify();
            if let Some(on_aspect) = self.props.on_aspect.clone() {
                self.deliver(move |window, cx| on_aspect(aspect, window, cx), cx);
            }
        }
    }

    fn start_watchdog(&mut self, cx: &mut Context<Self>) {
        self.watchdog = None;
        if self.player.is_none() {
            return;
        }
        self.watchdog = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs_f64(0.8)).await;
                let alive = this.update(cx, |this, cx| this.watchdog_did_fire(cx));
                if alive.is_err() {
                    return;
                }
            }
        }));
    }

    fn watchdog_did_fire(&mut self, cx: &mut Context<Self>) {
        let Some(player) = self.player.clone() else {
            return;
        };
        if !self.gate.should_play {
            return;
        }
        let stuck = {
            let player = player.read(cx);
            player.error().is_none()
                && player.current_item_status() == AVPlayerItemStatus::ReadyToPlay
                && player.time_control_status() != AVPlayerTimeControlStatus::Playing
        };
        if stuck {
            player.update(cx, |player, cx| player.play(cx));
        }
    }

    fn stop_watchdog(&mut self) {
        self.watchdog = None;
    }

    fn ensure_time_observer(&mut self, cx: &mut Context<Self>) {
        if self.time_observer.is_some() {
            return;
        }
        let (Some(player), Some(controller)) = (self.player.clone(), self.props.playback.clone()) else {
            return;
        };
        let weak_player = player.downgrade();
        controller.update(cx, |controller, cx| controller.set_player(Some(weak_player), cx));
        let observed = player.downgrade();
        let block: Box<dyn Fn(f64, &mut App)> = Box::new(move |seconds, cx| {
            if controller.read(cx).is_scrubbing() {
                return;
            }
            let duration = observed.upgrade().and_then(|player| player.read(cx).current_item_duration());
            controller.update(cx, |controller, cx| {
                if seconds.is_finite() {
                    controller.set_current_time(seconds, cx);
                }
                if let Some(duration) = duration.filter(|duration| duration.is_finite() && *duration > 0.0) {
                    controller.set_duration(duration, cx);
                }
            });
        });
        self.time_observer =
            Some(player.update(cx, |player, cx| player.add_periodic_time_observer(Duration::from_secs_f64(0.2), block, cx)));
    }

    fn remove_time_observer(&mut self) {
        self.time_observer = None;
    }

    fn ensure_music_observer(&mut self, cx: &mut Context<Self>) {
        if self.music_time_observer.is_some() || self.props.music_overlay.is_none() {
            return;
        }
        let Some(player) = self.player.clone() else {
            return;
        };
        let weak = cx.weak_entity();
        let block: Box<dyn Fn(f64, &mut App)> = Box::new(move |seconds, cx| {
            if !seconds.is_finite() {
                return;
            }
            weak.update(cx, |this, cx| {
                let is_playing = this.gate.should_play && !this.gate.is_muted;
                this.sync_music(seconds, is_playing, cx);
            })
            .ok();
        });
        self.music_time_observer =
            Some(player.update(cx, |player, cx| player.add_periodic_time_observer(Duration::from_secs_f64(0.25), block, cx)));
    }

    fn remove_music_observer(&mut self) {
        self.music_time_observer = None;
    }

    fn sync_music(&mut self, video_time: f64, is_playing: bool, cx: &mut Context<Self>) {
        let Some(overlay) = self.props.music_overlay.clone() else {
            return;
        };
        let is_playing = is_playing
            && MusicListeningInterruptionGate::shared(cx).update(cx, |gate, cx| gate.allows_external_audio_now(cx));
        cx.defer(move |cx| {
            PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| {
                controller.sync_to_video(
                    &overlay.post_id,
                    &overlay.url,
                    overlay.track_id.as_deref(),
                    overlay.start_ms,
                    overlay.end_ms,
                    video_time,
                    is_playing,
                    1.0,
                    cx,
                );
            });
        });
    }

    fn push_music_state(&mut self, cx: &mut Context<Self>) {
        if self.props.music_overlay.is_none() {
            return;
        }
        let time = self.player.as_ref().and_then(|player| player.read(cx).current_time()).unwrap_or(0.0);
        let video_time = if time.is_finite() { time } else { 0.0 };
        let is_playing = self.should_play(cx) && !self.props.is_muted;
        self.sync_music(video_time, is_playing, cx);
    }

    fn stop_music(&self, cx: &mut Context<Self>) {
        let Some(overlay) = &self.props.music_overlay else {
            return;
        };
        let post_id = overlay.post_id.clone();
        cx.defer(move |cx| {
            PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| controller.stop(&post_id, cx));
        });
    }

    fn emit_video_watch_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.watch_emitted {
            return;
        }
        let (Some(post_id), Some(author_id), Some(controller)) =
            (self.props.post_id.clone(), self.props.author_id.clone(), self.props.playback.clone())
        else {
            return;
        };
        if post_id.is_empty() || author_id.is_empty() {
            return;
        }
        let (current_time, duration) = {
            let controller = controller.read(cx);
            (controller.current_time(), controller.duration())
        };
        if current_time <= 0.0 {
            return;
        }
        self.watch_emitted = true;
        let replays = self.replay_count;
        EngagementInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.video_watch(
                &post_id,
                &author_id,
                EngagementSurface::ShortsInline,
                -1,
                (current_time * 1000.0) as i64,
                (duration * 1000.0) as i64,
                replays,
                cx,
            );
        });
    }

    fn play(&mut self, cx: &mut Context<Self>) {
        if !self.props.is_muted {
            self.activate_playback_audio(!self.unmuted_while_paused, cx);
        }
        self.unmuted_while_paused = false;
        if let Some(player) = self.player.clone() {
            player.update(cx, |player, cx| player.play(cx));
        }
    }

    fn pause(&mut self, cx: &mut Context<Self>) {
        if let Some(player) = self.player.clone() {
            player.update(cx, |player, cx| player.pause(cx));
        }
    }

    fn set_player_muted(&self, muted: bool, cx: &mut App) {
        if let Some(player) = self.player.clone() {
            player.update(cx, |player, cx| player.set_is_muted(muted, cx));
        }
    }

    fn activate_playback_audio(&mut self, autoplay: bool, cx: &mut Context<Self>) {
        let retry: Rc<dyn Fn(&mut App)> = {
            let weak = cx.weak_entity();
            Rc::new(move |cx| {
                weak.update(cx, |this, cx| {
                    if !this.should_play(cx) || this.props.is_muted {
                        return;
                    }
                    this.activate_playback_audio(false, cx);
                    this.set_player_muted(this.props.is_muted, cx);
                })
                .ok();
            })
        };
        let on_cancel: Rc<dyn Fn(&mut App)> = {
            let weak = cx.weak_entity();
            Rc::new(move |cx| {
                weak.update(cx, |this, cx| {
                    this.set_player_muted(true, cx);
                    this.audio_declined(cx);
                })
                .ok();
            })
        };
        let token = self.external_audio_owner.clone();
        let allowed = MusicListeningInterruptionGate::shared(cx)
            .update(cx, |gate, cx| gate.request_external_playback(Some(token), autoplay, retry, Some(on_cancel), cx));
        if !allowed {
            self.set_player_muted(true, cx);
            if autoplay && MusicListeningInterruptionGate::shared(cx).read(cx).prompt().is_none() {
                self.audio_declined(cx);
            }
            return;
        }
        let owner = self.external_audio_owner.clone();
        MusicPlayerState::shared(cx).update(cx, |state, cx| state.suspend_for_external_audio(owner, true, cx));
    }

    fn audio_declined(&self, cx: &mut Context<Self>) {
        if let Some(on_audio_declined) = self.props.on_audio_declined.clone() {
            self.deliver(move |window, cx| on_audio_declined(window, cx), cx);
        }
    }

    fn teardown(&mut self, cx: &mut Context<Self>) {
        self.setup_generation = self.setup_generation.wrapping_add(1);
        self.setup_task = None;
        self.landing_task = None;
        self.is_preparing_player = false;
        self.did_fail = false;
        self.media_lease = None;
        self.end_observer = None;
        self.player_observation = None;
        self.remove_time_observer();
        self.remove_music_observer();
        self.stop_music(cx);
        self.invalidate_external_audio(cx);
        self.stop_watchdog();
        self.player_layer_view = None;
        if let Some(playback) = self.props.playback.clone() {
            playback.update(cx, |playback, cx| {
                playback.set_player(None, cx);
                playback.set_player_layer_view(None, cx);
            });
        }
        if let Some(player) = self.player.take() {
            player.update(cx, |player, cx| player.pause(cx));
        }
        self.video_aspect = None;
        self.layer_ready = false;
        self.last_time_control_status = None;
        cx.notify();
    }

    fn sync_player_layer_view(&mut self, cx: &mut Context<Self>) -> Option<SharedElementPlayerLayerView> {
        let gravity = self.current_gravity();
        let player = self.player.as_ref()?;
        if let Some((view, current)) = &self.player_layer_view
            && *current == gravity
            && view.player().is_some_and(|bound| bound.entity_id() == player.entity_id())
        {
            return Some(view.clone());
        }
        let view = SharedElementPlayerLayerView::new(player, gravity);
        self.player_layer_view = Some((view.clone(), gravity));
        if let Some(playback) = self.props.playback.clone() {
            let published = view.clone();
            playback.update(cx, |playback, cx| playback.set_player_layer_view(Some(published), cx));
        }
        Some(view)
    }

    fn poster_did_measure(&mut self, size: Size<f32>, window: &mut Window, cx: &mut Context<Self>) {
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        let aspect = size.width / size.height;
        if self.poster_aspect != Some(aspect) {
            self.poster_aspect = Some(aspect);
            cx.notify();
        }
        if let Some(on_poster_aspect) = self.props.on_poster_aspect.clone() {
            on_poster_aspect(aspect, window, cx);
        }
    }

    /// sits over the layer until display_ready, no fade out
    /// poster only in a certain gravity, else plain black
    fn cover(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let poster = RemoteMediaURL::display_source(&self.props.url, self.props.thumbnail_url.as_deref());
        let settled_gravity = self.props.gravity_override.or_else(|| {
            self.video_aspect.or(self.props.initial_aspect).or(self.poster_aspect).map(ShortsFeedPlayer::gravity)
        });
        let content_mode = if settled_gravity == Some(AVLayerVideoGravity::ResizeAspectFill) {
            ContentMode::Fill
        } else {
            ContentMode::Fit
        };
        let weak = cx.weak_entity();
        div()
            .absolute()
            .inset_0()
            .when(self.props.draws_opaque_background, |cover| cover.bg(black()))
            .when(!poster.is_empty(), |cover| {
                cover.child(
                    div()
                        .absolute()
                        .inset_0()
                        .overflow_hidden()
                        .opacity(if settled_gravity.is_none() { 0.0 } else { 1.0 })
                        .child(
                            CleverImageUi::new(poster)
                                .content_mode(content_mode)
                                .corner_radius(self.props.corner_radius)
                                .persist_to_disk(self.props.persists_post_media_to_disk)
                                .shows_progress_indicator(false)
                                .cache_policy(self.props.post_image_cache_policy)
                                .on_natural_size(move |size, window, cx| {
                                    weak.update(cx, |this, cx| this.poster_did_measure(size, window, cx)).ok();
                                }),
                        ),
                )
            })
    }
}

impl ShortsFeedPlayerState {
    fn failure_badge() -> AnyElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.0))
            .children(IconUi::sized("exclamationmark.triangle.fill", px(26.0), white(), true))
            .child(
                MainText::new(tr("media_viewer_video_load_failed"))
                    .px(13.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(white()),
            )
            .into_any_element()
    }
}

impl Render for ShortsFeedPlayerState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity_id = cx.entity_id();
        let display_ready = self.display_ready();
        let shows_thumbnail = self.props.shows_thumbnail_while_loading;
        let corner_radius = self.props.corner_radius;

        let player_layer_view = self.sync_player_layer_view(cx);
        let layer = self.player.clone().map(|player| {
            div()
                .absolute()
                .inset_0()
                .opacity(if shows_thumbnail || display_ready { 1.0 } else { 0.0 })
                .when(self.props.draws_opaque_background, |layer| layer.bg(black()))
                .child(AVPlayerLayer::for_player(Some(player)).video_gravity(self.current_gravity()))
                .children(player_layer_view.as_ref().map(|view| view.probe(("shorts-feed-player-layer", entity_id))))
        });
        let cover = (!display_ready && shows_thumbnail).then(|| self.cover(cx));
        let fitted_aspect = match self.current_gravity() {
            AVLayerVideoGravity::ResizeAspect => self.video_aspect.or(self.props.initial_aspect).or(self.poster_aspect),
            _ => None,
        };
        let corner_notches = (corner_radius > 0.0 && self.player.is_some()).then(|| {
            canvas(
                |_, _: &mut Window, _: &mut App| {},
                move |bounds, (), window: &mut Window, _: &mut App| {
                    paint_corner_notches(fitted_video_rect(bounds, fitted_aspect), corner_radius, window);
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        });
        let loader = (!display_ready).then(|| {
            pass_through()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(if self.did_fail { Self::failure_badge() } else { progress_view().tint(white()).into_any_element() })
        });

        let appearing = cx.weak_entity();
        let disappearing = cx.weak_entity();
        div()
            .relative()
            .size_full()
            .when(self.props.draws_opaque_background, |player| player.bg(black()))
            .when(corner_radius > 0.0, |player| player.overflow_hidden().continuous_corners(corner_radius))
            .children(layer)
            .children(corner_notches)
            .children(cover)
            .children(loader)
            .on_appear(("shorts-feed-player", entity_id), move |_, cx| {
                appearing.update(cx, |this, cx| this.appear(cx)).ok();
            })
            .and_on_disappear(move |_, cx| {
                disappearing.update(cx, |this, cx| this.disappear(cx)).ok();
            })
    }
}

fn fitted_video_rect(bounds: Bounds<Pixels>, aspect: Option<f32>) -> Bounds<Pixels> {
    let Some(aspect) = aspect.filter(|aspect| aspect.is_finite() && *aspect > 0.0) else {
        return bounds;
    };
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    if width <= 0.0 || height <= 0.0 {
        return bounds;
    }
    let (fitted_width, fitted_height) = if width / height > aspect { (height * aspect, height) } else { (width, width / aspect) };
    Bounds::new(
        point(bounds.origin.x + px((width - fitted_width) / 2.0), bounds.origin.y + px((height - fitted_height) / 2.0)),
        size(px(fitted_width), px(fitted_height)),
    )
}

fn paint_corner_notches(rect: Bounds<Pixels>, corner_radius: f32, window: &mut Window) {
    let left = f32::from(rect.origin.x);
    let top = f32::from(rect.origin.y);
    let right = left + f32::from(rect.size.width);
    let bottom = top + f32::from(rect.size.height);
    let radius = corner_radius.min(f32::from(rect.size.width) / 2.0).min(f32::from(rect.size.height) / 2.0);
    if radius <= 0.0 {
        return;
    }
    let corners = [
        ((left, top), (left, top + radius), (left + radius, top)),
        ((right, top), (right - radius, top), (right, top + radius)),
        ((right, bottom), (right, bottom - radius), (right - radius, bottom)),
        ((left, bottom), (left + radius, bottom), (left, bottom - radius)),
    ];
    for (corner, from, to) in corners {
        let mut builder = PathBuilder::fill();
        builder.move_to(point(px(corner.0), px(corner.1)));
        builder.line_to(point(px(from.0), px(from.1)));
        builder.arc_to(point(px(radius), px(radius)), px(0.0), false, true, point(px(to.0), px(to.1)));
        builder.close();
        if let Ok(path) = builder.build() {
            window.paint_path(path, black());
        }
    }
}
