use rise_limits::OutgoingMediaProfile;

use crate::modules::share_extension::shared::rise_outgoing_media::{
    outgoing_media_largest_image_side, outgoing_media_profile,
};

const IMAGE_TEMP_SUFFIX: &str = "-img";
const VIDEO_TEMP_SUFFIX: &str = "-vid";

#[derive(Clone, Debug, PartialEq)]
pub struct MediaCompressionConfiguration {
    pub profile: &'static OutgoingMediaProfile,
    pub image_temp_filename_prefix: String,
    pub video_temp_filename_prefix: String,
    profile_name: &'static str,
}

impl MediaCompressionConfiguration {
    fn new(profile_name: &'static str, temporary_file_stem: &str, premium: bool) -> Self {
        Self {
            profile_name,
            profile: outgoing_media_profile(profile_name, premium),
            image_temp_filename_prefix: format!("{temporary_file_stem}{IMAGE_TEMP_SUFFIX}"),
            video_temp_filename_prefix: format!("{temporary_file_stem}{VIDEO_TEMP_SUFFIX}"),
        }
    }

    /// same profile resolved for the account tier
    /// static configs are free tier since they outlive the session
    /// tier is read at send time
    pub fn for_tier(&self, premium: bool) -> MediaCompressionConfiguration {
        if !premium {
            return self.clone();
        }
        let stem = self
            .image_temp_filename_prefix
            .strip_suffix(IMAGE_TEMP_SUFFIX)
            .unwrap_or(&self.image_temp_filename_prefix);
        MediaCompressionConfiguration::new(self.profile_name, stem, true)
    }

    /// max side a picked image is materialised at before compression sees it
    /// first and hardest stage, an image cut here cant grow back under any byte budget
    /// equals the largest canon profile
    pub fn source_pixel_max_side() -> f64 {
        outgoing_media_largest_image_side() as f64
    }

    /// message attachment and comment photo and media replacement on edit
    pub fn chat() -> Self {
        Self::new("chat", "ro-chat", false)
    }

    /// post and riff and story
    pub fn publication() -> Self {
        Self::new("publication", "ro-post", false)
    }

    /// avatar and header of profile and chat and bot, contact photo and resume logo
    pub fn avatar() -> Self {
        Self::new("avatar", "ro-avatar", false)
    }

    /// track and playlist cover
    pub fn cover() -> Self {
        Self::new("cover", "ro-cover", false)
    }

    pub fn wallpaper() -> Self {
        Self::new("wallpaper", "ro-wallpaper", false)
    }

    /// screenshots and screen recordings for bug reports and report evidence
    pub fn evidence() -> Self {
        Self::new("evidence", "ro-bug", false)
    }

    pub fn temporary_file_prefixes() -> Vec<String> {
        [
            Self::chat(),
            Self::publication(),
            Self::avatar(),
            Self::cover(),
            Self::wallpaper(),
            Self::evidence(),
        ]
        .iter()
        .flat_map(|configuration| {
            [
                format!("{}-", configuration.image_temp_filename_prefix),
                format!("{}-", configuration.video_temp_filename_prefix),
            ]
        })
        .collect()
    }
}
