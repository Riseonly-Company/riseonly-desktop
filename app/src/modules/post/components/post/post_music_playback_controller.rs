use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext, Context, ElementId, Entity, IntoElement, RenderOnce, SharedString, Subscription, Task, Window,
};
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_media::video::av_player::{
    AVPlayer, AVPlayerItem, AVPlayerItemNotification, AVPlayerItemStatus, AVPlayerTimeControlStatus,
    AVPlayerTimeObserver,
};
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::measure::measure;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::rise_media_foreground_gate::RiseMediaForegroundGate;
use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::components::shorts_inline_feed_cell::ShortsFeedFocusCoordinator;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

const BOUNDARY_INTERVAL: Duration = Duration::from_millis(50);

pub struct PostMusicPlaybackController {
    active_post_id: Option<String>,
    player: Entity<AVPlayer>,
    boundary_observer: Option<AVPlayerTimeObserver>,
    status_observation: Option<Subscription>,
    item_end_observer: Option<Subscription>,
    current_item: Option<u64>,
    item_generation: u64,
    current_url: Option<Url>,
    current_track_cache_key: Option<String>,
    start_ms: i64,
    end_ms: i64,
    loop_: bool,
    sync_mode: bool,
    last_position_ms: HashMap<String, i64>,
    cache_tasks: HashMap<String, Task<Option<Url>>>,
    _foreground_observer: Subscription,
}

impl PostMusicPlaybackController {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let gate = RiseMediaForegroundGate::shared(cx);
        let foreground_observer = cx.observe(&gate, |this: &mut Self, gate, cx| {
            if gate.read(cx).is_foreground() {
                return;
            }
            this.pause_for_background(cx);
        });
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            if let Some(controller) = weak.upgrade() {
                AccountStateResetRegistry::register(&controller, cx);
            }
        });
        Self {
            active_post_id: None,
            player: cx.new(|cx| AVPlayer::new(cx)),
            boundary_observer: None,
            status_observation: None,
            item_end_observer: None,
            current_item: None,
            item_generation: 0,
            current_url: None,
            current_track_cache_key: None,
            start_ms: 0,
            end_ms: 0,
            loop_: false,
            sync_mode: false,
            last_position_ms: HashMap::new(),
            cache_tasks: HashMap::new(),
            _foreground_observer: foreground_observer,
        }
    }

    pub fn active_post_id(&self) -> Option<&str> {
        self.active_post_id.as_deref()
    }

    fn pause_for_background(&mut self, cx: &mut Context<Self>) {
        if self.active_post_id.is_none() || self.is_paused(cx) {
            return;
        }
        self.player.update(cx, |player, cx| player.pause(cx));
    }

    fn is_paused(&self, cx: &App) -> bool {
        self.player.read(cx).time_control_status() == AVPlayerTimeControlStatus::Paused
    }

    fn set_player_volume(&self, volume: f32, cx: &mut Context<Self>) {
        self.player.update(cx, |player, cx| player.set_volume(volume, cx));
    }

    /// returns whether sound actually started
    /// caller must not render playback when the player refused
    #[allow(clippy::too_many_arguments)]
    pub fn play_ambient(
        &mut self,
        post_id: &str,
        url: &Url,
        track_id: Option<&str>,
        start_ms: i64,
        end_ms: i64,
        loop_: bool,
        resumes_last_position: bool,
        consented_over_listening_session: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if end_ms <= start_ms {
            return false;
        }
        if !consented_over_listening_session && MusicPlayerState::shared(cx).read(cx).listening_session.is_some() {
            return false;
        }
        let cache_key = Self::resolved_cache_key(track_id, url);

        if self.is_current(post_id, url, &cache_key, start_ms, end_ms) && !self.sync_mode {
            self.set_player_volume(1.0, cx);
            if self.player.read(cx).time_control_status() != AVPlayerTimeControlStatus::Playing {
                self.player.update(cx, |player, cx| player.play(cx));
            }
            return true;
        }

        self.activate_ambient_audio_session(cx);
        self.set_player_volume(1.0, cx);
        let playback_url = Self::cached_url(&cache_key).unwrap_or_else(|| url.clone());
        self.begin_item(
            post_id,
            url,
            playback_url,
            &cache_key,
            start_ms,
            end_ms,
            loop_,
            false,
            if resumes_last_position { None } else { Some(start_ms) },
            cx,
        );
        self.cache_if_needed(url, &cache_key, cx);
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sync_to_video(
        &mut self,
        post_id: &str,
        url: &Url,
        track_id: Option<&str>,
        start_ms: i64,
        end_ms: i64,
        video_time: f64,
        is_playing: bool,
        volume: f32,
        cx: &mut Context<Self>,
    ) {
        if end_ms <= start_ms {
            return;
        }
        if MusicPlayerState::shared(cx).read(cx).listening_session.is_some() {
            return;
        }
        let cache_key = Self::resolved_cache_key(track_id, url);
        let clamped_volume = volume.clamp(0.0, 1.0);

        let safe_video_time = if video_time.is_finite() { video_time.max(0.0) } else { 0.0 };
        let expected_ms = start_ms + (safe_video_time * 1000.0).round() as i64;
        let is_current = self.is_current(post_id, url, &cache_key, start_ms, end_ms) && self.sync_mode;

        if !is_current {
            if !is_playing || expected_ms >= end_ms {
                if self.active_post_id.as_deref() == Some(post_id) && !self.is_paused(cx) {
                    self.player.update(cx, |player, cx| player.pause(cx));
                }
                return;
            }
            self.activate_ambient_audio_session(cx);
            self.set_player_volume(clamped_volume, cx);
            let playback_url = Self::cached_url(&cache_key).unwrap_or_else(|| url.clone());
            self.begin_item(
                post_id,
                url,
                playback_url,
                &cache_key,
                start_ms,
                end_ms,
                false,
                true,
                Some(expected_ms),
                cx,
            );
            self.cache_if_needed(url, &cache_key, cx);
            return;
        }

        if self.player.read(cx).volume() != clamped_volume {
            self.set_player_volume(clamped_volume, cx);
        }

        if !is_playing || expected_ms >= end_ms {
            if !self.is_paused(cx) {
                self.player.update(cx, |player, cx| player.pause(cx));
            }
            return;
        }

        if let Some(actual) = self.player.read(cx).current_time().filter(|actual| actual.is_finite()) {
            let actual_ms = (actual * 1000.0).round() as i64;
            if (expected_ms - actual_ms).abs() > 300 {
                self.player.update(cx, |player, cx| player.seek(Self::time(expected_ms), 0.0, 0.0, None, cx));
            }
        }
        if self.is_paused(cx) {
            self.player.update(cx, |player, cx| player.play(cx));
        }
    }

    fn is_current(&self, post_id: &str, url: &Url, cache_key: &str, start_ms: i64, end_ms: i64) -> bool {
        self.active_post_id.as_deref() == Some(post_id)
            && self.current_url.as_ref() == Some(url)
            && self.current_track_cache_key.as_deref() == Some(cache_key)
            && self.start_ms == start_ms
            && self.end_ms == end_ms
    }

    pub fn stop(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if self.active_post_id.as_deref() != Some(post_id) {
            return;
        }
        self.stop_all(cx);
    }

    pub fn stop_all(&mut self, cx: &mut Context<Self>) {
        self.capture_position(cx);
        self.teardown_item(cx);
        self.set_player_volume(1.0, cx);
        MusicPlayerState::shared(cx).update(cx, |state, cx| {
            state.resume_after_external_audio(MusicExternalAudioOwner::POST_AMBIENT, cx);
        });
        self.active_post_id = None;
        self.current_url = None;
        self.current_track_cache_key = None;
        self.start_ms = 0;
        self.end_ms = 0;
        self.loop_ = false;
        self.sync_mode = false;
        cx.notify();
    }

    fn capture_position(&mut self, cx: &App) {
        if self.sync_mode || self.end_ms <= self.start_ms {
            return;
        }
        let Some(post_id) = self.active_post_id.clone() else {
            return;
        };
        let Some(seconds) = self.player.read(cx).current_time().filter(|seconds| seconds.is_finite()) else {
            return;
        };
        let mut ms = (seconds * 1000.0).round() as i64;
        if ms >= self.end_ms - 250 {
            ms = self.start_ms;
        }
        self.last_position_ms.insert(post_id, ms.max(self.start_ms).min(self.end_ms - 1));
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_item(
        &mut self,
        post_id: &str,
        source_url: &Url,
        playback_url: Url,
        cache_key: &str,
        start_ms: i64,
        end_ms: i64,
        loop_: bool,
        sync_mode: bool,
        initial_ms: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        self.teardown_item(cx);

        self.active_post_id = Some(post_id.to_owned());
        self.current_url = Some(source_url.clone());
        self.current_track_cache_key = Some(cache_key.to_owned());
        self.start_ms = start_ms;
        self.end_ms = end_ms;
        self.loop_ = loop_;
        self.sync_mode = sync_mode;
        cx.notify();

        self.item_generation = self.item_generation.wrapping_add(1);
        let item = self.item_generation;
        self.current_item = Some(item);
        self.player.update(cx, |player, cx| player.replace_current_item(Some(AVPlayerItem::new(playback_url)), cx));
        let player = self.player.clone();
        self.item_end_observer = Some(cx.subscribe(
            &player,
            move |this: &mut Self, _, _: &AVPlayerItemNotification, cx: &mut Context<Self>| {
                if this.current_item != Some(item) {
                    return;
                }
                let ended_post_id = this.active_post_id.clone();
                let segment_start_ms = this.start_ms;
                this.handle_segment_boundary(cx);
                if let Some(ended_post_id) = ended_post_id
                    && this.active_post_id.is_none()
                {
                    this.last_position_ms.insert(ended_post_id, segment_start_ms);
                }
            },
        ));
        let post_id = post_id.to_owned();
        let mut last_status = AVPlayerItemStatus::Unknown;
        self.status_observation = Some(cx.observe(&player, move |this: &mut Self, player, cx| {
            let status = player.read(cx).current_item_status();
            if status == last_status {
                return;
            }
            last_status = status;
            if this.current_item != Some(item) {
                return;
            }
            match status {
                AVPlayerItemStatus::Failed => {
                    if !this.sync_mode {
                        this.stop_all(cx);
                    }
                }
                AVPlayerItemStatus::ReadyToPlay => {
                    let seek_ms = match initial_ms {
                        Some(initial_ms) => initial_ms.max(start_ms).min(start_ms.max(end_ms - 1)),
                        None => this.resume_start_ms(&post_id),
                    };
                    player.update(cx, |player, cx| player.seek(Self::time(seek_ms), 0.0, 0.0, None, cx));
                    this.install_boundary_observer(cx);
                    player.update(cx, |player, cx| player.play(cx));
                }
                AVPlayerItemStatus::Unknown => {}
            }
        }));
    }

    fn resume_start_ms(&self, post_id: &str) -> i64 {
        if self.sync_mode {
            return self.start_ms;
        }
        match self.last_position_ms.get(post_id) {
            Some(&saved) if saved >= self.start_ms && saved < self.end_ms => saved,
            _ => self.start_ms,
        }
    }

    fn install_boundary_observer(&mut self, cx: &mut Context<Self>) {
        self.remove_boundary_observer(cx);
        let this = cx.weak_entity();
        let end_ms = self.end_ms;
        let armed = Cell::new(true);
        let observer = self.player.update(cx, |player, cx| {
            player.add_periodic_time_observer(
                BOUNDARY_INTERVAL,
                Box::new(move |seconds: f64, cx: &mut App| {
                    if ((seconds * 1000.0).round() as i64) < end_ms {
                        armed.set(true);
                        return;
                    }
                    if armed.replace(false) {
                        let _ = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                            this.handle_segment_boundary(cx);
                        });
                    }
                }),
                cx,
            )
        });
        self.boundary_observer = Some(observer);
    }

    fn handle_segment_boundary(&mut self, cx: &mut Context<Self>) {
        if self.sync_mode {
            self.player.update(cx, |player, cx| player.pause(cx));
            return;
        }
        if self.loop_ {
            if let Some(post_id) = self.active_post_id.clone() {
                self.last_position_ms.insert(post_id, self.start_ms);
            }
            let start = Self::time(self.start_ms);
            self.player.update(cx, |player, cx| {
                player.seek(start, 0.0, 0.0, None, cx);
                player.play(cx);
            });
        } else {
            self.stop_all(cx);
        }
    }

    fn teardown_item(&mut self, cx: &mut Context<Self>) {
        self.remove_boundary_observer(cx);
        self.status_observation = None;
        self.item_end_observer = None;
        self.player.update(cx, |player, cx| {
            player.pause(cx);
            player.replace_current_item(None, cx);
        });
        self.current_item = None;
    }

    fn remove_boundary_observer(&mut self, cx: &mut Context<Self>) {
        if let Some(boundary_observer) = self.boundary_observer.take() {
            self.player.update(cx, |player, _| player.remove_time_observer(boundary_observer));
        }
    }

    fn activate_ambient_audio_session(&self, cx: &mut Context<Self>) {
        MusicPlayerState::shared(cx).update(cx, |state, cx| {
            state.suspend_for_external_audio(MusicExternalAudioOwner::POST_AMBIENT, false, cx);
        });
    }

    /// players own copy wins if the user already keeps the track
    /// what this controller downloads goes to the ambient lane whose small budget cant evict a
    /// playlist track
    fn cached_url(cache_key: &str) -> Option<Url> {
        let user_id = SaiAccountCacheScope::active_user_id();
        let user_id = user_id.as_deref();
        MusicPlaybackDiskCache::find_cached_url(cache_key, user_id)
            .or_else(|| MusicPlaybackDiskCache::find_cached_url(&MusicPlaybackDiskCache::ambient_key(cache_key), user_id))
            .and_then(|path| Url::from_file_path(&path))
    }

    fn cache_if_needed(&mut self, remote_url: &Url, cache_key: &str, cx: &mut Context<Self>) {
        if Self::cached_url(cache_key).is_some() || self.cache_tasks.contains_key(cache_key) {
            return;
        }
        let owner_user_id = SaiAccountCacheScope::active_user_id();
        let owner_account_epoch = SaiAccountCacheScope::active_epoch();
        let authorization_token = AuthServicesStore::shared(cx).read(cx).tokens_and_other_data().access_token.clone();
        let remote_url = remote_url.clone();
        let task_key = cache_key.to_owned();
        let task = cx.spawn(async move |this, cx| {
            let local_url = MusicPlayerState::materialize_audio_for_playback(
                remote_url,
                MusicPlaybackDiskCache::ambient_key(&task_key),
                owner_user_id,
                owner_account_epoch,
                Some(authorization_token),
                cx,
            )
            .await;
            let _ = this.update(cx, |this: &mut Self, _: &mut Context<Self>| {
                this.cache_tasks.remove(&task_key);
            });
            local_url
        });
        self.cache_tasks.insert(cache_key.to_owned(), task);
    }

    fn resolved_cache_key(track_id: Option<&str>, url: &Url) -> String {
        let track_id = trimming_whitespaces_and_newlines(track_id.unwrap_or_default());
        if !track_id.is_empty() {
            return track_id.to_owned();
        }
        format!("ambient-{}", sha256_hex(url.absolute_string().as_bytes()))
    }

    fn time(ms: i64) -> f64 {
        ms as f64 / 1000.0
    }
}

impl AccountResettable for PostMusicPlaybackController {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.stop_all(cx);
        self.cache_tasks.clear();
        self.last_position_ms.clear();
    }
}

struct PostAmbientMusicDriverState {
    post: Rc<GetPostFeedResponse>,
    focus: Entity<ShortsFeedFocusCoordinator>,
    focus_token: String,
    shorts_store: Entity<ShortsServicesStore>,
    foreground_gate: Entity<RiseMediaForegroundGate>,
    last_should_play: bool,
    _observers: Vec<Subscription>,
    _release: Subscription,
}

impl PostAmbientMusicDriverState {
    fn new(post: Rc<GetPostFeedResponse>, cx: &mut Context<Self>) -> Self {
        let focus = ShortsFeedFocusCoordinator::shared(cx);
        let shorts_store = ShortsServicesStore::shared(cx);
        let foreground_gate = RiseMediaForegroundGate::shared(cx);
        let observers = vec![
            cx.observe(&focus, |this: &mut Self, _, cx| this.should_play_changed(cx)),
            cx.observe(&shorts_store, |this: &mut Self, _, cx| this.should_play_changed(cx)),
            cx.observe(&foreground_gate, |this: &mut Self, _, cx| {
                this.last_should_play = this.should_play(cx);
                this.apply_should_play(this.last_should_play, cx);
            }),
        ];
        Self {
            post,
            focus,
            focus_token: Uuid::new_v4().uuid_string(),
            shorts_store,
            foreground_gate,
            last_should_play: false,
            _observers: observers,
            _release: cx.on_release(|this: &mut Self, cx: &mut App| {
                let post_id = this.post.id.clone();
                PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| controller.stop(&post_id, cx));
            }),
        }
    }

    fn should_play(&self, cx: &App) -> bool {
        let shorts_store = self.shorts_store.read(cx);
        self.focus.read(cx).active_token() == Some(self.focus_token.as_str())
            && shorts_store.inline_feed_playback_enabled()
            && !shorts_store.feed_audio_muted
            && self.foreground_gate.read(cx).is_foreground()
            && self.post.music_audio_url().is_some()
    }

    fn should_play_changed(&mut self, cx: &mut Context<Self>) {
        let play = self.should_play(cx);
        if play == self.last_should_play {
            return;
        }
        self.last_should_play = play;
        self.apply_should_play(play, cx);
    }

    fn apply_should_play(&self, play: bool, cx: &mut Context<Self>) {
        if play {
            self.start_ambient(cx);
        } else {
            let post_id = self.post.id.clone();
            PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| controller.stop(&post_id, cx));
        }
    }

    fn start_ambient(&self, cx: &mut Context<Self>) {
        let Some(url) = self.post.music_audio_url() else {
            return;
        };
        let post = &self.post;
        PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| {
            controller.play_ambient(
                &post.id,
                &url,
                post.track_id.as_deref(),
                post.music_segment_start_ms(),
                post.music_segment_end_ms(),
                true,
                true,
                false,
                cx,
            );
        });
    }
}

/// draws nothing, a layer over its parent that reports where the post sits
/// parent must be relative like for any measure layer
#[derive(IntoElement)]
pub struct PostAmbientMusicDriver {
    id: SharedString,
    post: Rc<GetPostFeedResponse>,
}

impl PostAmbientMusicDriver {
    /// id stands for the state of this instance, a post shown twice keeps two drivers
    pub fn new(id: impl Into<SharedString>, post: impl Into<Rc<GetPostFeedResponse>>) -> Self {
        Self { id: id.into(), post: post.into() }
    }
}

impl RenderOnce for PostAmbientMusicDriver {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = |part: &str| -> ElementId {
            ElementId::Name(SharedString::from(format!("post-ambient-music-{part}-{}", self.id)))
        };
        let initial_post = self.post.clone();
        let state = window.use_keyed_state(key("state"), cx, move |_, cx| {
            PostAmbientMusicDriverState::new(initial_post, cx)
        });
        if state.read(cx).post != self.post {
            let state = state.clone();
            let post = self.post.clone();
            window.defer(cx, move |_, cx| {
                state.update(cx, |state, cx| {
                    state.post = post;
                    state.should_play_changed(cx);
                });
            });
        }

        let report_state = state.clone();
        let appear_state = state.clone();
        let measure_key = key("frame");
        let appear_key = key("appear");
        measure(measure_key, move |frame, _, cx| {
            let (focus, token) = {
                let state = report_state.read(cx);
                (state.focus.clone(), state.focus_token.clone())
            };
            focus.update(cx, |focus, cx| focus.report(&token, frame, cx));
        })
        .on_appear(appear_key, move |_, cx| {
            appear_state.update(cx, |state, cx| {
                if state.should_play(cx) {
                    state.start_ambient(cx);
                }
            });
        })
        .and_on_disappear(move |_, cx| {
            let (focus, token, post_id) = {
                let state = state.read(cx);
                (state.focus.clone(), state.focus_token.clone(), state.post.id.clone())
            };
            focus.update(cx, |focus, cx| focus.remove(&token, cx));
            PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| controller.stop(&post_id, cx));
        })
    }
}
