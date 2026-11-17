use gpui::{App, Context, ElementId, Entity, Render, RenderOnce, Window};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_media::video::av_player::{
    AVLayerVideoGravity, AVPlayerActionAtItemEnd, AVPlayerItem, AVPlayerLayer, AVPlayerLooper, AVQueuePlayer,
};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::shapes::pass_through;

use crate::modules::notify::engine::core::rise_release_media_warmer::RiseReleaseMediaWarmer;

/// clip of a release slide, silent and looping with no controls
/// muted and never takes media keys
/// so it wont interrupt music or a call
#[derive(IntoElement)]
pub struct ReleaseSlideVideo {
    id: ElementId,
    remote_url: String,
    is_active: bool,
}

impl ReleaseSlideVideo {
    /// id keys the clips view so neighbour pages of one deck never share a player
    pub fn new(id: impl Into<ElementId>, remote_url: impl Into<String>, is_active: bool) -> Self {
        Self { id: id.into(), remote_url: remote_url.into(), is_active }
    }
}

impl RenderOnce for ReleaseSlideVideo {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = window.use_keyed_state(self.id, cx, |_, _| LoopingVideoView::new());
        view.update(cx, |view, cx| {
            view.configure(&self.remote_url, cx);
            view.set_active(self.is_active, cx);
        });
        div().size_full().child(view)
    }
}

pub struct LoopingVideoView {
    queue_player: Option<Entity<AVQueuePlayer>>,
    looper: Option<AVPlayerLooper>,
    loaded_url: Option<String>,
    is_active: bool,
}

impl LoopingVideoView {
    pub fn new() -> Self {
        Self { queue_player: None, looper: None, loaded_url: None, is_active: false }
    }

    pub fn configure(&mut self, remote_url: &str, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(remote_url).to_string();
        if trimmed.is_empty() || self.loaded_url.as_deref() == Some(trimmed.as_str()) {
            return;
        }
        self.teardown(cx);
        self.loaded_url = Some(trimmed.clone());

        let source = RiseReleaseMediaWarmer::local_video_url(&trimmed)
            .and_then(|path| Url::from_file_path(&path))
            .or_else(|| Url::from_string(&trimmed));
        let Some(source) = source else {
            return;
        };

        let item = AVPlayerItem::new(source);
        let player = cx.new(|cx| {
            let mut player = AVQueuePlayer::new(item.clone(), cx);
            player.set_is_muted(true);
            player.set_action_at_item_end(AVPlayerActionAtItemEnd::Advance);
            player.set_automatically_waits_to_minimize_stalling(true);
            player
        });

        self.looper = Some(AVPlayerLooper::new(&player, &item, cx));
        self.queue_player = Some(player);
        self.apply_active_state(cx);
        cx.notify();
    }

    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.is_active == active {
            return;
        }
        self.is_active = active;
        self.apply_active_state(cx);
    }

    fn apply_active_state(&self, cx: &mut Context<Self>) {
        let Some(queue_player) = &self.queue_player else {
            return;
        };
        let is_active = self.is_active;
        queue_player.update(cx, |player, cx| {
            if is_active {
                player.play(cx);
            } else {
                player.pause(cx);
            }
        });
    }

    pub fn teardown(&mut self, cx: &mut Context<Self>) {
        if let Some(queue_player) = &self.queue_player {
            queue_player.update(cx, |player, cx| player.pause(cx));
        }
        self.looper = None;
        self.queue_player = None;
        self.loaded_url = None;
        cx.notify();
    }
}

impl Default for LoopingVideoView {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LoopingVideoView {
    fn drop(&mut self) {
        self.looper = None;
        self.queue_player = None;
    }
}

impl Render for LoopingVideoView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        pass_through().size_full().child(
            AVPlayerLayer::new(self.queue_player.clone()).video_gravity(AVLayerVideoGravity::ResizeAspectFill),
        )
    }
}
