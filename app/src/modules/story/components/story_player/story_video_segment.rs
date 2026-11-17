use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext, Context, Entity, ImageSource, IntoElement, ObjectFit, ParentElement, Pixels, Render, RenderOnce, Size,
    Styled, StyledImage, Subscription, Task, Window, black, div, img, px, white,
};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_media::video::av_player::{
    AVLayerVideoGravity, AVPlayer, AVPlayerItem, AVPlayerItemNotification, AVPlayerItemStatus, AVPlayerLayer,
    AVPlayerTimeObserver,
};
use rise_ui::IconUi;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::shapes::pass_through;

use crate::core::lib::rise_call_app_volume::RiseCallAppVolume;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheLimits, RemoteMediaDiskCacheMaterializeOptions, RemoteMediaFileLease,
};
use crate::core::media::rise_media_streaming_asset::RiseMediaStreamingAsset;
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;
use crate::modules::music::shared::utils::music::music_listening_interruption_gate::MusicListeningInterruptionGate;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::story::components::story_player::story_media_view::{StoryFrameAspect, StoryMediaPrefetcher};
use crate::modules::story::shared::story_poster_cache::StoryPosterCache;

pub type StoryVideoSegmentValue<T> = Rc<dyn Fn(T, &mut App)>;
pub type StoryVideoSegmentAction = Rc<dyn Fn(&mut App)>;

pub struct StoryVideoSegment {
    url: String,
    is_paused: bool,
    pub on_duration: StoryVideoSegmentValue<f64>,
    pub on_progress: StoryVideoSegmentValue<f64>,
    pub on_finished: StoryVideoSegmentAction,
    pub on_aspect: StoryVideoSegmentValue<f32>,
    pub on_player: Option<StoryVideoSegmentValue<Option<Entity<AVPlayer>>>>,
    pub on_ready_for_display: Option<StoryVideoSegmentValue<bool>>,
    pub loops_at_end: Rc<dyn Fn(&App) -> bool>,
    pub poster_story_id: i64,

    player: Option<Entity<AVPlayer>>,
    media_lease: Option<RemoteMediaFileLease>,
    streaming_asset: Option<RiseMediaStreamingAsset>,
    time_observer: Option<AVPlayerTimeObserver>,
    end_observer: Option<Subscription>,
    duration: f64,
    did_finish: bool,
    setup_generation: u64,
    setup_task: Option<Task<()>>,
    external_audio_owner: MusicExternalAudioOwner,
    video_aspect: Option<f32>,
    is_ready_for_display: bool,
    reported_ready_for_display: Option<bool>,
    status_observation: Option<Subscription>,
    observed_item_status: Option<AVPlayerItemStatus>,
    retried_after_failure: bool,
    did_fail: bool,
    paused_now: bool,
    landing_task: Option<Task<()>>,
    size: Size<Pixels>,
}

impl StoryVideoSegment {
    pub fn new(url: impl Into<String>, is_paused: bool, cx: &mut Context<Self>) -> Self {
        cx.on_release(|this: &mut Self, cx: &mut App| this.release_external_audio(cx)).detach();
        Self {
            url: url.into(),
            is_paused,
            on_duration: Rc::new(|_, _| {}),
            on_progress: Rc::new(|_, _| {}),
            on_finished: Rc::new(|_| {}),
            on_aspect: Rc::new(|_, _| {}),
            on_player: None,
            on_ready_for_display: None,
            loops_at_end: Rc::new(|_| false),
            poster_story_id: 0,
            player: None,
            media_lease: None,
            streaming_asset: None,
            time_observer: None,
            end_observer: None,
            duration: 0.0,
            did_finish: false,
            setup_generation: 0,
            setup_task: None,
            external_audio_owner: MusicExternalAudioOwner::unique("story"),
            video_aspect: None,
            is_ready_for_display: false,
            reported_ready_for_display: None,
            status_observation: None,
            observed_item_status: None,
            retried_after_failure: false,
            did_fail: false,
            paused_now: false,
            landing_task: None,
            size: Size::default(),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused
    }

    /// photo rule with slack, a clip a little wider than the box still fills
    /// near 9:16 has no hairline bars, clearly wider shows whole
    pub fn fills(aspect: Option<f32>, box_size: Size<f32>) -> bool {
        let Some(aspect) = aspect.filter(|aspect| *aspect > 0.0) else {
            return true;
        };
        if box_size.width <= 0.0 || box_size.height <= 0.0 {
            return true;
        }
        if StoryFrameAspect::is_story_frame(aspect) {
            return false;
        }
        aspect <= box_size.width / box_size.height * 1.25
    }

    pub fn set_is_paused(&mut self, paused: bool, cx: &mut Context<Self>) {
        if self.is_paused == paused {
            return;
        }
        self.is_paused = paused;
        self.paused_now = paused;
        tracing::warn!(target: "riseonly::X1291", paused, has_player = self.player.is_some(), "segment set paused");
        let Some(player) = self.player.clone() else {
            return;
        };
        if paused {
            player.update(cx, |player, cx| player.pause(cx));
        } else {
            self.activate_audio_session(cx);
            player.update(cx, |player, cx| player.play(cx));
        }
    }

    pub fn set_url(&mut self, url: impl Into<String>, cx: &mut Context<Self>) {
        let url = url.into();
        if self.url == url {
            return;
        }
        self.url = url;
        self.teardown(cx);
        self.retried_after_failure = false;
        self.setup(cx);
    }

    fn appear(&mut self, cx: &mut Context<Self>) {
        self.paused_now = self.is_paused;
        self.retried_after_failure = false;
        self.setup(cx);
    }

    fn handle_item_failure(&mut self, generation: u64, cx: &mut Context<Self>) {
        if generation != self.setup_generation || self.player.is_none() {
            return;
        }
        if self.retried_after_failure {
            self.did_fail = true;
            cx.notify();
            return;
        }
        self.retried_after_failure = true;
        self.teardown(cx);
        self.setup(cx);
    }

    fn setup(&mut self, cx: &mut Context<Self>) {
        let Some(u) = Url::from_string(&self.url) else {
            return;
        };
        self.setup_generation = self.setup_generation.wrapping_add(1);
        let generation = self.setup_generation;
        if u.is_file_url() {
            self.install_player_url(u, cx);
            return;
        }
        if let Some(prefetched) = StoryMediaPrefetcher::prefetched_video_url(&u) {
            self.install_player_url(prefetched, cx);
            return;
        }
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let cache_key = StoryMediaPrefetcher::video_cache_key(&u);
        let lookup = {
            let remote = u.clone();
            let cache_key = cache_key.clone();
            let account_user_id = account_user_id.clone();
            let account_epoch = account_epoch.clone();
            cx.background_spawn(async move {
                RemoteMediaDiskCache::shared().existing_local_url(
                    &remote,
                    Some(&cache_key),
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                )
            })
        };
        self.setup_task = Some(cx.spawn(async move |this, cx| {
            let cached = lookup.await;
            let installed_remote = this
                .update(cx, |this, cx| {
                    if generation != this.setup_generation {
                        return false;
                    }
                    if let Some(task) = this.setup_task.take() {
                        task.detach();
                    }
                    if let Some(cached) = cached.as_deref().and_then(Url::from_file_path) {
                        this.install_player_url(cached, cx);
                        return false;
                    }
                    if let Some(streaming) = RiseMediaStreamingAsset::make(
                        &u,
                        &cache_key,
                        RemoteMediaDiskCacheLimits::default_limits().maximum_file_bytes,
                        account_user_id.clone(),
                        account_epoch.clone(),
                    ) {
                        let item = AVPlayerItem::with_asset(streaming.asset.clone());
                        this.streaming_asset = Some(streaming);
                        this.install_player(item, None, cx);
                        this.watch_stream_landing(
                            u.clone(),
                            cache_key.clone(),
                            account_user_id.clone(),
                            account_epoch.clone(),
                            cx,
                        );
                        return false;
                    }
                    this.install_player_url(u.clone(), cx);
                    true
                })
                .unwrap_or(false);
            if !installed_remote {
                return;
            }
            let options = RemoteMediaDiskCacheMaterializeOptions {
                cache_key: Some(cache_key),
                suggested_filename: Some(u.last_path_component()),
                expected_account_user_id: account_user_id,
                expected_account_epoch: account_epoch,
                ..RemoteMediaDiskCacheMaterializeOptions::default()
            };
            let _ = RemoteMediaDiskCache::shared().ensure_cached(&u, options, cx).await;
        }));
    }

    fn install_player_url(&mut self, url: Url, cx: &mut Context<Self>) {
        self.install_player(AVPlayerItem::new(url.clone()), Some(url), cx);
    }

    fn watch_stream_landing(
        &mut self,
        remote: Url,
        cache_key: String,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let generation = self.setup_generation;
        self.landing_task = Some(cx.spawn(async move |this, cx| {
            let mut waited = Duration::ZERO;
            while waited < Self::LANDING_JOIN {
                cx.background_executor().timer(Self::LANDING_POLL).await;
                waited += Self::LANDING_POLL;
                let waiting = |this: &Self| generation == this.setup_generation && !this.is_ready_for_display;
                if !this.read_with(cx, |this, _| waiting(this)).unwrap_or(false) {
                    return;
                }
                let local = if waited >= Self::LANDING_JOIN {
                    let options = RemoteMediaDiskCacheMaterializeOptions {
                        cache_key: Some(cache_key.clone()),
                        suggested_filename: Some(remote.last_path_component()),
                        expected_account_user_id: account_user_id.clone(),
                        expected_account_epoch: account_epoch.clone(),
                        ..RemoteMediaDiskCacheMaterializeOptions::default()
                    };
                    RemoteMediaDiskCache::shared().ensure_cached(&remote, options, cx).await
                } else {
                    let remote = remote.clone();
                    let cache_key = cache_key.clone();
                    let account_user_id = account_user_id.clone();
                    let account_epoch = account_epoch.clone();
                    cx.background_executor()
                        .spawn(async move {
                            RemoteMediaDiskCache::shared().existing_local_url(
                                &remote,
                                Some(&cache_key),
                                account_user_id.as_deref(),
                                account_epoch.as_deref(),
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
                    tracing::warn!(target: "riseonly::X1291", waited = waited.as_secs(), "segment stream landed, swap to local");
                    this.teardown(cx);
                    this.install_player_url(local, cx);
                });
                return;
            }
        }));
    }

    const LANDING_POLL: Duration = Duration::from_secs(1);
    const LANDING_JOIN: Duration = Duration::from_secs(6);

    fn install_player(&mut self, item: AVPlayerItem, leasing: Option<Url>, cx: &mut Context<Self>) {
        if self.player.is_some() {
            return;
        }
        let leased_path = leasing.as_ref().and_then(Url::to_file_path);
        self.media_lease = RemoteMediaDiskCache::lease(leased_path.as_deref());
        let item_url = item.url().clone();
        tracing::warn!(target: "riseonly::X1291", local = leasing.is_some(), paused_now = self.paused_now, url = %self.url, "segment install player");
        let player = cx.new(AVPlayer::new);
        player.update(cx, |player, cx| {
            player.set_is_muted(false, cx);
            player.replace_current_item(Some(item), cx);
        });
        RiseCallAppVolume::follow(&player, cx);

        let weak = cx.weak_entity();
        let observed = player.downgrade();
        let block: Box<dyn Fn(f64, &mut App)> = Box::new(move |seconds, cx| {
            let Some(player) = observed.upgrade() else {
                return;
            };
            let item_duration = player.read(cx).current_item_duration();
            weak.update(cx, |this, cx| this.periodic_time_did_fire(seconds, item_duration, cx)).ok();
        });
        self.time_observer =
            Some(player.update(cx, |player, cx| player.add_periodic_time_observer(Duration::from_secs_f64(0.03), block, cx)));

        self.end_observer = Some(cx.subscribe(&player, move |this, player, notification: &AVPlayerItemNotification, cx| {
            let AVPlayerItemNotification::DidPlayToEndTime { asset_url } = notification;
            if *asset_url == item_url {
                this.item_did_play_to_end(&player, cx);
            }
        }));

        let generation = self.setup_generation;
        self.observed_item_status = None;
        self.reported_ready_for_display = None;
        self.status_observation = Some(cx.observe(&player, move |this, player, cx| {
            this.player_did_change(&player, generation, cx);
        }));

        if !self.paused_now {
            self.activate_audio_session(cx);
            player.update(cx, |player, cx| player.play(cx));
        }
        self.player = Some(player.clone());
        if let Some(on_player) = self.on_player.clone() {
            let installed = player.clone();
            cx.defer(move |cx| on_player(Some(installed), cx));
        }
        self.player_did_change(&player, generation, cx);
        cx.notify();
    }

    fn periodic_time_did_fire(&mut self, seconds: f64, item_duration: Option<f64>, cx: &mut Context<Self>) {
        if let Some(item_duration) = item_duration.filter(|value| value.is_finite() && *value > 0.0)
            && self.duration != item_duration
        {
            tracing::warn!(target: "riseonly::X1291", item_duration, seconds, "segment duration");
            self.duration = item_duration;
            let on_duration = self.on_duration.clone();
            cx.defer(move |cx| on_duration(item_duration, cx));
        }
        if self.duration > 0.0 {
            let progress = seconds / self.duration;
            let on_progress = self.on_progress.clone();
            cx.defer(move |cx| on_progress(progress, cx));
        }
    }

    fn item_did_play_to_end(&mut self, player: &Entity<AVPlayer>, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X1291", did_finish = self.did_finish, "segment played to end");
        if self.did_finish {
            return;
        }
        if (self.loops_at_end)(cx) {
            let paused_now = self.paused_now;
            player.update(cx, |player, cx| {
                player.seek(0.0, 0.0, 0.0, None, cx);
                if !paused_now {
                    player.play(cx);
                }
            });
            return;
        }
        self.did_finish = true;
        let on_progress = self.on_progress.clone();
        let on_finished = self.on_finished.clone();
        cx.defer(move |cx| {
            on_progress(1.0, cx);
            on_finished(cx);
        });
    }

    fn player_did_change(&mut self, player: &Entity<AVPlayer>, generation: u64, cx: &mut Context<Self>) {
        if self.player.as_ref().is_some_and(|current| current.entity_id() != player.entity_id()) {
            return;
        }
        let (status, ready, presentation_size) = {
            let player = player.read(cx);
            (player.current_item_status(), player.is_ready_for_display(), player.presentation_size())
        };

        if self.observed_item_status != Some(status) || self.reported_ready_for_display != Some(ready) {
            tracing::warn!(target: "riseonly::X1291", ?status, ready, w = presentation_size.width, h = presentation_size.height, "segment player change");
        }
        if self.observed_item_status != Some(status) {
            self.observed_item_status = Some(status);
            if status == AVPlayerItemStatus::Failed {
                let weak = cx.weak_entity();
                cx.defer(move |cx| {
                    weak.update(cx, |this, cx| this.handle_item_failure(generation, cx)).ok();
                });
            }
        }

        let ready_changed = self.reported_ready_for_display != Some(ready);
        self.reported_ready_for_display = Some(ready);
        if self.is_ready_for_display != ready {
            self.is_ready_for_display = ready;
            cx.notify();
        }
        if ready_changed && let Some(on_ready_for_display) = self.on_ready_for_display.clone() {
            cx.defer(move |cx| on_ready_for_display(ready, cx));
        }

        if self.video_aspect.is_none() && presentation_size.width > 0.0 && presentation_size.height > 0.0 {
            let aspect = presentation_size.width / presentation_size.height;
            self.video_aspect = Some(aspect);
            let on_aspect = self.on_aspect.clone();
            cx.defer(move |cx| on_aspect(aspect, cx));
            cx.notify();
        }
    }

    fn activate_audio_session(&mut self, cx: &mut Context<Self>) {
        let retry: Rc<dyn Fn(&mut App)> = {
            let weak = cx.weak_entity();
            Rc::new(move |cx| {
                weak.update(cx, |this, cx| {
                    if let Some(player) = this.player.clone() {
                        player.update(cx, |player, cx| player.set_is_muted(false, cx));
                    }
                    this.activate_audio_session(cx);
                })
                .ok();
            })
        };
        let on_cancel: Rc<dyn Fn(&mut App)> = {
            let weak = cx.weak_entity();
            Rc::new(move |cx| {
                weak.update(cx, |this, cx| {
                    if let Some(player) = this.player.clone() {
                        player.update(cx, |player, cx| player.set_is_muted(true, cx));
                    }
                })
                .ok();
            })
        };
        let token = self.external_audio_owner.clone();
        let allowed = MusicListeningInterruptionGate::shared(cx)
            .update(cx, |gate, cx| gate.request_external_playback(Some(token), false, retry, Some(on_cancel), cx));
        if !allowed {
            if let Some(player) = self.player.clone() {
                player.update(cx, |player, cx| player.set_is_muted(true, cx));
            }
            return;
        }
        let owner = self.external_audio_owner.clone();
        MusicPlayerState::shared(cx).update(cx, |state, cx| state.suspend_for_external_audio(owner, false, cx));
    }

    fn release_external_audio(&mut self, cx: &mut App) {
        if let Some(player) = self.player.take() {
            player.update(cx, |player, cx| player.pause(cx));
        }
        if let Some(streaming) = self.streaming_asset.take() {
            streaming.teardown();
        }
        let token = self.external_audio_owner.clone();
        MusicListeningInterruptionGate::shared(cx).update(cx, |gate, cx| gate.invalidate_prompt(token, cx));
        let owner = self.external_audio_owner.clone();
        MusicPlayerState::shared(cx).update(cx, |state, cx| state.resume_after_external_audio(owner, cx));
    }

    /// runs on disappear and on every restart of the clip
    fn teardown(&mut self, cx: &mut Context<Self>) {
        self.setup_generation = self.setup_generation.wrapping_add(1);
        self.setup_task = None;
        self.landing_task = None;
        if let (Some(observer), Some(player)) = (self.time_observer.take(), &self.player) {
            player.update(cx, |player, _| player.remove_time_observer(observer));
        }
        self.time_observer = None;
        self.end_observer = None;
        self.status_observation = None;
        self.observed_item_status = None;
        self.reported_ready_for_display = None;
        self.is_ready_for_display = false;
        self.did_fail = false;
        if let Some(player) = self.player.take() {
            player.update(cx, |player, cx| player.pause(cx));
            if let Some(on_player) = self.on_player.clone() {
                cx.defer(move |cx| on_player(None, cx));
            }
        }
        self.video_aspect = None;
        self.media_lease = None;
        if let Some(streaming) = self.streaming_asset.take() {
            streaming.teardown();
        }
        let token = self.external_audio_owner.clone();
        MusicListeningInterruptionGate::shared(cx).update(cx, |gate, cx| gate.invalidate_prompt(token, cx));
        let owner = self.external_audio_owner.clone();
        MusicPlayerState::shared(cx).update(cx, |state, cx| state.resume_after_external_audio(owner, cx));
        self.did_finish = false;
        self.duration = 0.0;
        cx.notify();
    }
}

impl Render for StoryVideoSegment {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity_id = cx.entity_id();
        let box_size = Size { width: f32::from(self.size.width), height: f32::from(self.size.height) };
        let fills = Self::fills(self.video_aspect, box_size);

        let content = match &self.player {
            Some(player) => StoryPlayerLayerView { player: player.clone(), fills }.into_any_element(),
            None => div().size_full().bg(black()).into_any_element(),
        };
        let poster = (!self.did_fail && (self.player.is_none() || !self.is_ready_for_display) && self.poster_story_id > 0)
            .then(|| StoryPosterCache::cached(self.poster_story_id))
            .flatten()
            .and_then(|poster| {
                let poster_size = poster.size(0);
                if poster_size.width.0 <= 0 || poster_size.height.0 <= 0 {
                    return None;
                }
                let aspect = poster_size.width.0 as f32 / poster_size.height.0 as f32;
                let fit = if Self::fills(Some(aspect), box_size) { ObjectFit::Cover } else { ObjectFit::Contain };
                Some(
                    pass_through()
                        .absolute()
                        .inset_0()
                        .overflow_hidden()
                        .child(img(ImageSource::Render(poster)).object_fit(fit).size_full()),
                )
            });
        let overlay = if self.did_fail {
            Some(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.0))
                    .children(IconUi::sized("exclamationmark.triangle.fill", px(26.0), white(), true))
                    .child(
                        MainText::new(tr("media_viewer_video_load_failed"))
                            .px(13.0)
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .color(white()),
                    )
                    .into_any_element(),
            )
        } else if self.player.is_none() || !self.is_ready_for_display {
            Some(LoaderUi::new().size(LoaderUiSize::Large).color(Some(white())).into_any_element())
        } else {
            None
        };

        let measured = cx.weak_entity();
        let appearing = cx.weak_entity();
        let disappearing = cx.weak_entity();
        div()
            .relative()
            .size_full()
            .child(content)
            .children(poster)
            .children(overlay.map(|overlay| {
                pass_through().absolute().inset_0().flex().items_center().justify_center().child(overlay)
            }))
            .child(measure(("story-video-segment-geometry", entity_id), move |bounds, _, cx| {
                measured
                    .update(cx, |this, cx| {
                        if this.size != bounds.size {
                            this.size = bounds.size;
                            cx.notify();
                        }
                    })
                    .ok();
            }))
            .on_appear(("story-video-segment", entity_id), move |_, cx| {
                appearing.update(cx, |this, cx| this.appear(cx)).ok();
            })
            .and_on_disappear(move |_, cx| {
                disappearing.update(cx, |this, cx| this.teardown(cx)).ok();
            })
    }
}

#[derive(IntoElement)]
struct StoryPlayerLayerView {
    player: Entity<AVPlayer>,
    fills: bool,
}

impl RenderOnce for StoryPlayerLayerView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let gravity = if self.fills { AVLayerVideoGravity::ResizeAspectFill } else { AVLayerVideoGravity::ResizeAspect };
        div().size_full().child(AVPlayerLayer::for_player(Some(self.player)).video_gravity(gravity))
    }
}
