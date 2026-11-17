use rise_limits::limit::{CHAT_VIDEO_NOTE_DURATION, SHORT_VIDEO_DURATION};

/// canon short video durations in whole seconds
pub enum ShortVideoLimits {}

impl ShortVideoLimits {
    pub const MAX_DURATION_SECONDS: i64 = (SHORT_VIDEO_DURATION / 1000) as i64;
    pub const CHAT_VIDEO_NOTE_MAX_DURATION_SECONDS: i64 = (CHAT_VIDEO_NOTE_DURATION / 1000) as i64;
}
