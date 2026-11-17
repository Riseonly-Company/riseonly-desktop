use gpui::{App, Context, Entity};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::MEDIA_VIEWER_LOOP_ENABLED;

pub struct MediaViewerPlaybackPreferences {
    loops_video: bool,
}

impl MediaViewerPlaybackPreferences {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::new())
    }

    fn new() -> Self {
        Self {
            loops_video: UserDefaults::standard().bool(MEDIA_VIEWER_LOOP_ENABLED),
        }
    }

    pub fn loops_video(&self) -> bool {
        self.loops_video
    }

    pub fn set_loops_video(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.loops_video == enabled {
            return;
        }
        self.loops_video = enabled;
        UserDefaults::standard().set(MEDIA_VIEWER_LOOP_ENABLED, enabled);
        cx.notify();
    }
}
