use gpui::{App, Entity, IntoElement, ParentElement as _, RenderOnce, Styled as _, Window, div};
use rise_media::video::av_player::{AVLayerVideoGravity, AVPlayer, AVPlayerLayer};

/// the note picture filling its parent, aspect fill
/// caller clips it to the circle, desktop pulls the newest picture every frame
/// so epoch only keys the view
#[derive(IntoElement)]
pub struct VideoNotePlayerLayerView {
    pub player: Entity<AVPlayer>,
    pub epoch: i64,
}

impl VideoNotePlayerLayerView {
    pub fn new(player: Entity<AVPlayer>, epoch: i64) -> Self {
        Self { player, epoch }
    }
}

impl RenderOnce for VideoNotePlayerLayerView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        VideoNotePlayerUIView::new(self.player)
    }
}

#[derive(IntoElement)]
pub struct VideoNotePlayerUIView {
    player_layer: AVPlayerLayer,
}

impl VideoNotePlayerUIView {
    pub fn new(player: Entity<AVPlayer>) -> Self {
        Self {
            player_layer: AVPlayerLayer::for_player(Some(player)).video_gravity(AVLayerVideoGravity::ResizeAspectFill),
        }
    }
}

impl RenderOnce for VideoNotePlayerUIView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().size_full().child(self.player_layer)
    }
}
