use std::collections::HashSet;
use std::ops::RangeInclusive;

use gpui::{App, Context, Entity};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::CHAT_PROFILE_MEDIA_COLUMNS;

use crate::modules::chat::stores::message::message_actions::message_types::MessageMediaKind;

/// media grid settings of the chat profile
/// column count is kept on this device, the photo and video filters per chat for the session
pub struct ChatProfileMediaGridPrefs {
    columns: i64,
    hides_photos: HashSet<String>,
    hides_videos: HashSet<String>,
}

impl ChatProfileMediaGridPrefs {
    pub const COLUMN_RANGE: RangeInclusive<i64> = 3..=6;
    const COLUMNS_DEFAULTS_KEY: &'static str = CHAT_PROFILE_MEDIA_COLUMNS;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        let stored = UserDefaults::standard().integer(Self::COLUMNS_DEFAULTS_KEY);
        let columns = if Self::COLUMN_RANGE.contains(&stored) { stored } else { *Self::COLUMN_RANGE.start() };
        Self { columns, hides_photos: HashSet::new(), hides_videos: HashSet::new() }
    }

    pub fn columns(&self) -> i64 {
        self.columns
    }

    pub fn can_zoom_in(&self) -> bool {
        self.columns > *Self::COLUMN_RANGE.start()
    }

    pub fn can_zoom_out(&self) -> bool {
        self.columns < *Self::COLUMN_RANGE.end()
    }

    pub fn zoom_in(&mut self, cx: &mut Context<Self>) {
        self.set_columns(self.columns - 1, cx);
    }

    /// more, smaller cells
    pub fn zoom_out(&mut self, cx: &mut Context<Self>) {
        self.set_columns(self.columns + 1, cx);
    }

    pub fn set_columns(&mut self, value: i64, cx: &mut Context<Self>) {
        let next = (*Self::COLUMN_RANGE.end()).min((*Self::COLUMN_RANGE.start()).max(value));
        if next == self.columns {
            return;
        }
        self.columns = next;
        UserDefaults::standard().set(Self::COLUMNS_DEFAULTS_KEY, next);
        cx.notify();
    }

    pub fn shows_photos(&self, scope: &str) -> bool {
        !self.hides_photos.contains(scope)
    }

    pub fn shows_videos(&self, scope: &str) -> bool {
        !self.hides_videos.contains(scope)
    }

    pub fn toggle_photos(&mut self, scope: &str, cx: &mut Context<Self>) {
        if self.hides_photos.contains(scope) {
            self.hides_photos.remove(scope);
        } else if self.shows_videos(scope) {
            self.hides_photos.insert(scope.to_owned());
        } else {
            return;
        }
        cx.notify();
    }

    pub fn toggle_videos(&mut self, scope: &str, cx: &mut Context<Self>) {
        if self.hides_videos.contains(scope) {
            self.hides_videos.remove(scope);
        } else if self.shows_photos(scope) {
            self.hides_videos.insert(scope.to_owned());
        } else {
            return;
        }
        cx.notify();
    }

    /// a gif counts as a video here, the filter only has two kinds
    pub fn shows(&self, kind: MessageMediaKind, scope: &str) -> bool {
        match kind {
            MessageMediaKind::Video | MessageMediaKind::Animation | MessageMediaKind::Gif | MessageMediaKind::VideoNote => {
                self.shows_videos(scope)
            }
            _ => self.shows_photos(scope),
        }
    }
}
