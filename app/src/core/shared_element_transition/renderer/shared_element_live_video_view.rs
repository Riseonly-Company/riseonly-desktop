use std::rc::{Rc, Weak};

use gpui::{
    App, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Subscription, WeakEntity, Window, div,
};
use rise_media::video::av_player::{AVLayerVideoGravity, AVPlayer, AVPlayerLayer};

use crate::core::shared_element_transition::runtime::shared_element_containers::SharedElementLiveReadiness;

pub struct SharedElementLiveVideoView {
    player: Option<WeakEntity<AVPlayer>>,
    video_gravity: AVLayerVideoGravity,
    on_ready_for_display: Option<Rc<dyn Fn(bool, &mut App)>>,
    readiness: Option<Weak<SharedElementLiveReadiness>>,
    last_ready_for_display: bool,
    _ready_observation: Option<Subscription>,
}

impl SharedElementLiveVideoView {
    pub fn new(player: &Entity<AVPlayer>, video_gravity: AVLayerVideoGravity, cx: &mut Context<Self>) -> Self {
        let observation = cx.observe(player, |this, player, cx| {
            let ready = player.read(cx).is_ready_for_display();
            this.ready_for_display_did_change(ready, cx);
        });
        Self {
            player: Some(player.downgrade()),
            video_gravity,
            on_ready_for_display: None,
            readiness: None,
            last_ready_for_display: false,
            _ready_observation: Some(observation),
        }
    }

    pub fn video_gravity(&self) -> AVLayerVideoGravity {
        self.video_gravity
    }

    pub fn set_video_gravity(&mut self, video_gravity: AVLayerVideoGravity, cx: &mut Context<Self>) {
        self.video_gravity = video_gravity;
        cx.notify();
    }

    pub fn set_on_ready_for_display(&mut self, handler: Option<Rc<dyn Fn(bool, &mut App)>>) {
        self.on_ready_for_display = handler;
    }

    /// marked ready on the first displayed frame, or right away if one is already shown
    pub fn set_readiness(&mut self, readiness: Option<Weak<SharedElementLiveReadiness>>, cx: &App) {
        self.readiness = readiness;
        if self.is_ready_for_display(cx)
            && let Some(readiness) = self.readiness.as_ref().and_then(Weak::upgrade)
        {
            readiness.mark_ready();
        }
    }

    pub fn is_ready_for_display(&self, cx: &App) -> bool {
        self.player
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .is_some_and(|player| player.read(cx).is_ready_for_display())
    }

    pub fn detach_player(&mut self, cx: &mut Context<Self>) {
        self.player = None;
        self._ready_observation = None;
        cx.notify();
    }

    fn ready_for_display_did_change(&mut self, ready: bool, cx: &mut Context<Self>) {
        if ready == self.last_ready_for_display {
            return;
        }
        self.last_ready_for_display = ready;
        if let Some(handler) = self.on_ready_for_display.clone() {
            handler(ready, &mut **cx);
        }
        if ready
            && self.is_ready_for_display(cx)
            && let Some(readiness) = self.readiness.as_ref().and_then(Weak::upgrade)
        {
            readiness.mark_ready();
        }
    }
}

impl Render for SharedElementLiveVideoView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let player = self.player.as_ref().and_then(WeakEntity::upgrade);
        div().size_full().child(AVPlayerLayer::for_player(player).video_gravity(self.video_gravity))
    }
}
