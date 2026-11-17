use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gpui::{App, Entity, WeakEntity};
use rise_core::foundation::foundation_url::Url;

use crate::modules::media_picker::editor::editor_music_track_lane::EditorMusicLaneUIView;
use crate::modules::media_picker::editor::editor_video_trim_view::{
    EditorVideoPlayerUIView, EditorVideoTimelineUIView,
};
use crate::modules::post::components::post::post_music_playback_controller::PostMusicPlaybackController;

/// attached track playing in the editor preview
#[derive(Clone, Debug, PartialEq)]
pub struct EditorVideoPlaybackControllerMusicPreview {
    pub session_id: String,
    pub url: Url,
    pub track_id: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub volume: f32,
    pub offset_seconds: f64,
}

/// held as Rc, nothing observes it and the player calls back into it from inside its own seek
/// a gpui entity lease would re-enter
pub struct EditorVideoPlaybackController {
    pub player_view: RefCell<Option<WeakEntity<EditorVideoPlayerUIView>>>,
    pub timeline_view: RefCell<Option<WeakEntity<EditorVideoTimelineUIView>>>,
    pub music_lane_view: RefCell<Option<WeakEntity<EditorMusicLaneUIView>>>,

    pub pending_seek_seconds: Cell<Option<f64>>,
    pub suppress_swift_ui_trim_publish: Cell<bool>,
    pub video_window: Cell<(f64, f64)>,
    pub source_duration_seconds: Cell<f64>,

    music_preview: RefCell<Option<EditorVideoPlaybackControllerMusicPreview>>,
    is_preview_playing: Cell<bool>,
}

impl EditorVideoPlaybackController {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            player_view: RefCell::new(None),
            timeline_view: RefCell::new(None),
            music_lane_view: RefCell::new(None),
            pending_seek_seconds: Cell::new(None),
            suppress_swift_ui_trim_publish: Cell::new(false),
            video_window: Cell::new((0.0, 0.0)),
            source_duration_seconds: Cell::new(0.0),
            music_preview: RefCell::new(None),
            is_preview_playing: Cell::new(false),
        })
    }

    pub fn wire(
        self: &Rc<Self>,
        player: &Entity<EditorVideoPlayerUIView>,
        timeline: &Entity<EditorVideoTimelineUIView>,
        cx: &mut App,
    ) {
        self.player_view.replace(Some(player.downgrade()));
        self.timeline_view.replace(Some(timeline.downgrade()));

        let this = Rc::downgrade(self);
        player.update(cx, |player, _| {
            player.on_time_update = Some(Rc::new(move |seconds: f64, cx: &mut App| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                this.update_timeline_playhead(seconds, cx);
                this.update_music_lane_playhead(this.source_fraction(seconds), cx);
                this.sync_music(seconds, cx);
            }));
        });

        let this = Rc::downgrade(self);
        timeline.update(cx, |timeline, cx| {
            timeline.follow_playhead(
                Rc::new(move |cx: &App| this.upgrade().and_then(|this| this.live_playback_seconds(cx))),
                cx,
            );
        });
        let lane = self.music_lane_view.borrow().as_ref().and_then(WeakEntity::upgrade);
        if let Some(lane) = lane {
            self.follow_lane_playhead(&lane, cx);
        }
    }

    pub fn attach_music_lane(self: &Rc<Self>, lane: &Entity<EditorMusicLaneUIView>, cx: &mut App) {
        self.music_lane_view.replace(Some(lane.downgrade()));
        self.follow_lane_playhead(lane, cx);
    }

    pub fn request_seek(&self, seconds: f64, pause: bool, cx: &mut App) {
        self.pending_seek_seconds.set(Some(seconds));
        let player = self.player_view.borrow().as_ref().and_then(WeakEntity::upgrade);
        if let Some(player) = player {
            player.update(cx, |player, cx| player.seek(seconds, pause, cx));
        }
        self.update_timeline_playhead(seconds, cx);
        self.update_music_lane_playhead(self.source_fraction(seconds), cx);
        self.pending_seek_seconds.set(None);
        self.sync_music(seconds, cx);
    }

    pub fn set_music_preview(
        &self,
        preview: Option<EditorVideoPlaybackControllerMusicPreview>,
        cx: &mut App,
    ) {
        let has_preview = preview.is_some();
        let previous = self.music_preview.replace(preview);
        if !has_preview {
            if let Some(previous) = previous {
                Self::stop_post_music(previous.session_id, cx);
            }
            return;
        }
        self.sync_music(self.clock_seconds(cx), cx);
    }

    pub fn set_preview_playing(&self, playing: bool, cx: &mut App) {
        if self.is_preview_playing.get() == playing {
            return;
        }
        self.is_preview_playing.set(playing);
        self.sync_music(self.clock_seconds(cx), cx);
    }

    pub fn stop_music_preview(&self, cx: &mut App) {
        self.is_preview_playing.set(false);
        if let Some(previous) = self.music_preview.replace(None) {
            Self::stop_post_music(previous.session_id, cx);
        }
    }

    /// track follows the video clock not its own so preview matches the muxed file
    /// position counts from the trim start, music begins with the selected range
    fn sync_music(&self, video_seconds: f64, cx: &mut App) {
        let Some(music) = self.music_preview.borrow().clone() else {
            return;
        };
        let offset = video_seconds - music.offset_seconds;
        let (window_start, window_end) = self.video_window.get();
        let within_window = window_end <= window_start || video_seconds <= window_end + 0.01;
        let playing = self.is_preview_playing.get() && offset >= 0.0 && within_window;
        cx.defer(move |cx| {
            PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| {
                controller.sync_to_video(
                    &music.session_id,
                    &music.url,
                    music.track_id.as_deref(),
                    music.start_ms,
                    music.end_ms,
                    offset.max(0.0),
                    playing,
                    music.volume,
                    cx,
                );
            });
        });
    }

    fn source_fraction(&self, seconds: f64) -> f64 {
        let duration = self.source_duration_seconds.get();
        if duration <= 0.01 {
            return 0.0;
        }
        (seconds / duration).clamp(0.0, 1.0)
    }

    fn stop_post_music(session_id: String, cx: &mut App) {
        cx.defer(move |cx| {
            PostMusicPlaybackController::shared(cx)
                .update(cx, |controller, cx| controller.stop(&session_id, cx));
        });
    }

    fn live_playback_seconds(&self, cx: &App) -> Option<f64> {
        let player = self.player_view.borrow().as_ref().and_then(WeakEntity::upgrade)?;
        player.read(cx).live_playback_seconds(cx)
    }

    /// player live seconds or the video window start
    fn clock_seconds(&self, cx: &App) -> f64 {
        self.live_playback_seconds(cx).unwrap_or(self.video_window.get().0)
    }

    fn update_timeline_playhead(&self, seconds: f64, cx: &mut App) {
        let timeline = self.timeline_view.borrow().as_ref().and_then(WeakEntity::upgrade);
        if let Some(timeline) = timeline {
            timeline.update(cx, |timeline, cx| timeline.update_playhead(seconds, cx));
        }
    }

    fn update_music_lane_playhead(&self, fraction: f64, cx: &mut App) {
        let lane = self.music_lane_view.borrow().as_ref().and_then(WeakEntity::upgrade);
        if let Some(lane) = lane {
            lane.update(cx, |lane, cx| lane.update_playhead(fraction, cx));
        }
    }

    fn follow_lane_playhead(self: &Rc<Self>, lane: &Entity<EditorMusicLaneUIView>, cx: &mut App) {
        let this: Weak<Self> = Rc::downgrade(self);
        lane.update(cx, |lane, cx| {
            lane.follow_playhead(
                Rc::new(move |cx: &App| {
                    let this = this.upgrade()?;
                    let seconds = this.live_playback_seconds(cx)?;
                    Some(this.source_fraction(seconds))
                }),
                cx,
            );
        });
    }
}
