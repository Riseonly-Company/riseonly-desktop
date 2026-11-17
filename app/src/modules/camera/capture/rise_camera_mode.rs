use gpui::App;
use rise_core::foundation::foundation_date::TimeInterval;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys;

use crate::app::router::navigation_types::CreatePostIntent;
use crate::modules::camera::capture::post_author_premium_hooks::post_author_is_premium;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::short_video_limits::ShortVideoLimits;

/// what a shot from the feed camera becomes
/// the strip lists them in this order, camera opens on the last used one and on story the first time
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RiseCameraMode {
    Publication,
    Story,
    Riffs,
}

impl RiseCameraMode {
    pub const ALL_CASES: [RiseCameraMode; 3] = [Self::Publication, Self::Story, Self::Riffs];

    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::Publication => "publication",
            Self::Story => "story",
            Self::Riffs => "riffs",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|mode| mode.raw_value() == raw)
    }

    pub const fn title_key(self) -> &'static str {
        match self {
            Self::Publication => "create_post_kind_publication",
            Self::Story => "create_menu_story",
            Self::Riffs => "riffs",
        }
    }

    /// a riff is a video so a shutter tap only explains how to record one
    pub fn takes_photos(self) -> bool {
        self != Self::Riffs
    }

    /// same preset the gallery path of this mode uses so the recording limit matches the composer
    pub fn configuration(self, cx: &App) -> MediaPickerConfiguration {
        match self {
            Self::Story => MediaPickerConfiguration::story(),
            Self::Publication | Self::Riffs => {
                MediaPickerConfiguration::post_media_for_tier(post_author_is_premium(cx))
            }
        }
    }

    pub fn post_intent(self) -> Option<CreatePostIntent> {
        match self {
            Self::Publication => Some(CreatePostIntent::Publication),
            Self::Riffs => Some(CreatePostIntent::Reel),
            Self::Story => None,
        }
    }

    pub fn max_recording_seconds(self, cx: &App) -> TimeInterval {
        let limit = self
            .configuration(cx)
            .max_video_duration_seconds
            .unwrap_or(ShortVideoLimits::MAX_DURATION_SECONDS);
        limit.max(1) as TimeInterval
    }
}

/// device preference, the mode the camera last left on
pub enum RiseCameraModePreference {}

impl RiseCameraModePreference {
    pub fn mode() -> RiseCameraMode {
        UserDefaults::standard()
            .string(user_defaults_keys::CAMERA_FEED_LAST_MODE)
            .and_then(|raw| RiseCameraMode::from_raw_value(&raw))
            .unwrap_or(RiseCameraMode::Story)
    }

    pub fn set_mode(new_value: RiseCameraMode) {
        UserDefaults::standard().set(user_defaults_keys::CAMERA_FEED_LAST_MODE, new_value.raw_value());
    }
}
