use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{AnyElement, App, Window};
use rise_canon::{RiseLimits, RiseUploadFormat};
use rise_i18n::tr;
use rise_limits::MediaPickerPreset as RiseMediaPickerPreset;
use rise_limits::limit::{BUG_REPORT_SCREENSHOTS, POST_IMAGES, POST_IMAGES_PREMIUM};

use crate::core::lib::canon_rules::{RiseLimitsCanonRules, RiseMediaPickerPresetCanonRules};
use crate::core::rise_engine_bridge::rise_canon_fields::ValidationSchema;
use crate::modules::media_picker::editor::editor_link_entity::EditorLinkEntity;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::PackKind;

pub type MediaPickerMediaKind = rise_limits::MediaPickerKind;
pub type MediaPickerSendMode = rise_limits::MediaPickerSendMode;
pub type MediaPickerSourceTab = rise_limits::MediaPickerSourceTab;

pub type MediaPickerCustomTopOverlay = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingShortUpload {
    pub temp_id: String,
    pub local_video_url: PathBuf,
    pub thumbnail_url: Option<String>,
    pub caption: String,
    pub sender_name: String,
    pub sender_tag: String,
    pub sender_logo: Option<String>,
    pub sender_id: String,
    pub is_uploading: bool,
}

impl PendingShortUpload {
    /// plain init with is_uploading defaulting to true
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        temp_id: String,
        local_video_url: PathBuf,
        thumbnail_url: Option<String>,
        caption: String,
        sender_name: String,
        sender_tag: String,
        sender_logo: Option<String>,
        sender_id: String,
    ) -> Self {
        Self {
            temp_id,
            local_video_url,
            thumbnail_url,
            caption,
            sender_name,
            sender_tag,
            sender_logo,
            sender_id,
            is_uploading: true,
        }
    }
}

const POST_MEDIA_TIER_CEILINGS: (i64, i64) = (
    rise_limits::signed(POST_IMAGES),
    rise_limits::signed(POST_IMAGES_PREMIUM),
);

#[derive(Clone)]
pub struct MediaPickerConfiguration {
    pub kind: MediaPickerMediaKind,
    pub send_mode: MediaPickerSendMode,
    pub max_selection_count: i64,
    pub allows_caption: bool,
    pub enforces_single_kind: bool,
    pub max_video_count: Option<i64>,
    pub max_image_count: Option<i64>,
    pub max_video_duration_seconds: Option<i64>,
    pub untrimmed_long_video_message_key: Option<String>,
    pub premium_selection_ceiling: Option<i64>,
    pub selection_limit_message_key: Option<String>,
    pub source_tabs: Vec<MediaPickerSourceTab>,

    pub allows_editing: bool,
    pub title: String,
    pub confirm_button_title: String,
    pub confirm_button_system_image: String,
    pub shows_export_progress_overlay: bool,
    pub custom_top_overlay: Option<MediaPickerCustomTopOverlay>,
    pub shows_selection_controls: bool,
    pub shows_bottom_bar: bool,
    pub header_confirms_selection: bool,
    pub auto_confirm_on_select: bool,
    pub auto_confirm_after_editing: bool,
    pub defers_video_processing_to_consumer: bool,
    pub consumer_mixes_attached_track: bool,
    pub keeps_animated_images: bool,
    pub shows_camera_tile: bool,
    pub allows_media_spoiler: bool,
    pub allows_view_once: bool,
    pub allowed_file_extensions: Option<HashSet<String>>,
    pub offers_file_chooser: bool,
    pub allows_music_selection: bool,
    pub allows_person_tagging: bool,
    pub uses_story_canvas: bool,
    pub allows_media_links: bool,
    pub media_links_max: i64,
    pub media_links_shared_across_selection: bool,
    pub media_links_elsewhere: i64,
    pub destination_title: Option<String>,
    pub allows_poll: bool,
    pub caption_rule: Option<ValidationSchema>,
}

impl MediaPickerConfiguration {
    /// private init with defaults
    /// the canon preset fixes kind and send mode and counts and source tabs and video duration
    /// titles and overlays and export flags stay local and each preset overrides what it needs
    fn new(preset: &RiseMediaPickerPreset, title: String, shows_export_progress_overlay: bool) -> Self {
        Self {
            kind: preset.kind,
            send_mode: preset.send_mode,
            max_selection_count: rise_limits::signed(preset.max_selection_count),
            allows_caption: preset.allows_caption,
            enforces_single_kind: preset.single_kind,
            max_video_count: preset.max_video_count.map(rise_limits::signed),
            max_image_count: None,
            max_video_duration_seconds: preset.max_video_duration_seconds(),
            untrimmed_long_video_message_key: None,
            premium_selection_ceiling: None,
            selection_limit_message_key: None,
            source_tabs: preset.source_tabs.to_vec(),
            allows_editing: true,
            title,
            confirm_button_title: tr("media_picker_done"),
            confirm_button_system_image: "checkmark".to_string(),
            shows_export_progress_overlay,
            custom_top_overlay: None,
            shows_selection_controls: true,
            shows_bottom_bar: true,
            header_confirms_selection: false,
            auto_confirm_on_select: false,
            auto_confirm_after_editing: false,
            defers_video_processing_to_consumer: false,
            consumer_mixes_attached_track: false,
            keeps_animated_images: false,
            shows_camera_tile: false,
            allows_media_spoiler: false,
            allows_view_once: false,
            allowed_file_extensions: None,
            offers_file_chooser: false,
            allows_music_selection: false,
            allows_person_tagging: false,
            uses_story_canvas: false,
            allows_media_links: false,
            media_links_max: EditorLinkEntity::max_count(),
            media_links_shared_across_selection: false,
            media_links_elsewhere: 0,
            destination_title: None,
            allows_poll: false,
            caption_rule: None,
        }
    }

    pub fn chat_attachments() -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("chat_attachments"),
            tr("media_picker_title"),
            false,
        );
        configuration.defers_video_processing_to_consumer = true;
        configuration.keeps_animated_images = true;
        configuration.shows_camera_tile = true;
        configuration.allows_media_spoiler = true;
        configuration.caption_rule = Some(rise_validate::MESSAGE_CAPTION);
        configuration.selection_limit_message_key = Some("share_error_too_many_items".to_string());
        configuration
    }

    pub fn chat_video_note() -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("chat_video_note"),
            tr("media_picker_video_note_title"),
            false,
        );
        configuration.defers_video_processing_to_consumer = true;
        configuration
    }

    pub fn track_cover() -> Self {
        Self::new(
            RiseLimits::media_picker_preset("track_cover"),
            tr("music_upload_cover"),
            true,
        )
    }

    pub fn track_cover_inside_editor() -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("track_cover"),
            tr("music_upload_cover"),
            true,
        );
        configuration.allows_editing = false;
        configuration.auto_confirm_on_select = true;
        configuration
    }

    /// desktop only
    /// an image preset with no file source takes a file from the system dialog or a drop
    /// only formats the consumer can preview and label, no heic or gif
    pub fn offering_file_chooser(mut self) -> Self {
        if self.kind != MediaPickerMediaKind::Images || self.source_tabs.contains(&MediaPickerSourceTab::File) {
            return self;
        }
        self.offers_file_chooser = true;
        self.allowed_file_extensions.get_or_insert_with(|| {
            [RiseUploadFormat::IMAGE_PNG, RiseUploadFormat::IMAGE_JPEG, RiseUploadFormat::IMAGE_WEBP]
                .into_iter()
                .flat_map(|format| format.disk_extensions.iter())
                .map(|disk_extension| (*disk_extension).to_owned())
                .collect()
        });
        self
    }

    pub fn chat_avatar() -> Self {
        Self::new(
            RiseLimits::media_picker_preset("chat_avatar"),
            tr("create_chat_avatar_picker_title"),
            true,
        )
    }

    pub fn company_logo() -> Self {
        Self::new(
            RiseLimits::media_picker_preset("chat_avatar"),
            tr("vacancy_experience_logo"),
            true,
        )
    }

    /// one photo, used as the background of every chat
    pub fn chat_wallpaper() -> Self {
        Self::new(
            RiseLimits::media_picker_preset("chat_wallpaper"),
            tr("chat_wallpaper_picker_title"),
            true,
        )
    }

    pub fn profile_banner() -> Self {
        Self::new(
            RiseLimits::media_picker_preset("chat_wallpaper"),
            tr("profile_banner_picker_title"),
            true,
        )
    }

    pub fn profile_avatar() -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("profile_avatar"),
            tr("select_avatar_text"),
            true,
        );
        configuration.shows_selection_controls = false;
        configuration.shows_bottom_bar = false;
        configuration
    }

    fn profile_avatar_album_base() -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("profile_avatar_album"),
            tr("select_avatar_text"),
            true,
        );
        configuration.shows_bottom_bar = false;
        configuration.header_confirms_selection = true;
        configuration
    }

    pub fn profile_avatar_album(remaining_slots: i64) -> Self {
        let mut configuration = Self::profile_avatar_album_base();
        configuration.max_selection_count = 1_i64.max(configuration.max_selection_count.min(remaining_slots));
        configuration
    }

    pub fn post_images() -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("post_images"),
            tr("create_post_add_photos"),
            true,
        );
        configuration.allows_music_selection = true;
        configuration.allows_person_tagging = true;
        configuration
    }

    pub fn post_media() -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("post_media"),
            tr("create_post_add_media"),
            true,
        );
        configuration.allows_music_selection = true;
        configuration.allows_person_tagging = true;
        configuration.untrimmed_long_video_message_key = Some("create_post_short_too_long".to_string());
        configuration
    }

    pub fn post_media_ceiling(premium: bool) -> i64 {
        if premium {
            POST_MEDIA_TIER_CEILINGS.1
        } else {
            POST_MEDIA_TIER_CEILINGS.0
        }
    }

    /// composer media step, post_media with the count raised to the author's tier edge
    /// the preset count is the free edge so a subscriber would stop short otherwise
    pub fn post_media_for_tier(premium: bool) -> Self {
        let mut configuration = Self::post_media();
        configuration.max_selection_count = Self::post_media_ceiling(premium);
        if !premium && POST_MEDIA_TIER_CEILINGS.1 > POST_MEDIA_TIER_CEILINGS.0 {
            configuration.premium_selection_ceiling = Some(POST_MEDIA_TIER_CEILINGS.1);
        }
        configuration
    }

    pub fn post_media_appending(remaining_slots: i64, remaining_videos: i64, premium: bool) -> Self {
        let mut configuration = Self::post_media_for_tier(premium);
        configuration.max_selection_count = 1_i64.max(configuration.max_selection_count.min(remaining_slots));
        let video_ceiling = configuration.max_video_count.unwrap_or(remaining_videos);
        configuration.max_video_count = Some(0_i64.max(video_ceiling.min(remaining_videos)));
        configuration
    }

    pub fn post_short_video() -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("post_short_video"),
            tr("create_post_add_short"),
            true,
        );
        configuration.allows_music_selection = true;
        configuration.allows_person_tagging = true;
        configuration
    }

    pub fn story() -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("story"),
            tr("story_create_title"),
            false,
        );
        configuration.confirm_button_title = tr("story_publish");
        configuration.shows_selection_controls = false;
        configuration.shows_bottom_bar = false;
        configuration.allows_music_selection = true;
        configuration.consumer_mixes_attached_track = true;
        configuration.uses_story_canvas = true;
        configuration.allows_media_links = true;
        configuration
    }

    pub fn story_canvas_add(remaining_slots: i64) -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("post_images"),
            tr("create_post_add_photos"),
            true,
        );
        configuration.allows_editing = false;
        configuration.max_selection_count = 1_i64.max(configuration.max_selection_count.min(remaining_slots));
        configuration
    }

    pub fn comment_photo() -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("comment_photo"),
            tr("media_picker_title"),
            false,
        );
        configuration.keeps_animated_images = true;
        configuration
    }

    pub fn map_grid() -> Self {
        Self::new(
            RiseLimits::media_picker_preset("map_grid"),
            tr("map_photo_add"),
            true,
        )
    }

    pub fn bug_report() -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("bug_report"),
            tr("bug_report_attachments_section"),
            true,
        );
        let screenshots = rise_limits::signed(BUG_REPORT_SCREENSHOTS);
        configuration.max_image_count = Some(screenshots);
        configuration.max_video_count = Some(0_i64.max(configuration.max_selection_count - screenshots));
        configuration
    }

    pub fn bug_report_appending(existing_screenshots: i64, has_video: bool) -> Self {
        let mut configuration = Self::bug_report();
        let screenshots = 0_i64.max(configuration.max_image_count.unwrap_or(0) - existing_screenshots);
        let videos = if has_video { 0 } else { configuration.max_video_count.unwrap_or(0) };
        configuration.max_image_count = Some(screenshots);
        configuration.max_video_count = Some(videos);
        configuration.max_selection_count = 1_i64.max(configuration.max_selection_count.min(screenshots + videos));
        configuration
    }

    pub fn report_evidence() -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("report_evidence"),
            tr("report_evidence_section"),
            true,
        );
        configuration.untrimmed_long_video_message_key = Some("bug_report_video_too_long".to_string());
        configuration.selection_limit_message_key = Some("report_evidence_limit_reached".to_string());
        configuration
    }

    /// evidence picker capped to the slots the draft still has so nothing picked is dropped
    pub fn report_evidence_slots_left(slots_left: i64) -> Self {
        let mut configuration = Self::report_evidence();
        let slots = 1_i64.max(configuration.max_selection_count.min(slots_left));
        configuration.max_selection_count = slots;
        configuration.max_image_count = configuration.max_image_count.map(|count| count.min(slots));
        configuration.max_video_count = configuration.max_video_count.map(|count| count.min(slots));
        configuration
    }

    pub fn sticker_asset(kind: PackKind) -> Self {
        let mut configuration = Self::new(
            RiseLimits::media_picker_preset("sticker_asset"),
            tr(if kind == PackKind::Emoji {
                "emoji_pack_media_picker_title"
            } else {
                "sticker_pack_media_picker_title"
            }),
            false,
        );
        configuration.shows_selection_controls = false;
        configuration.auto_confirm_after_editing = true;
        configuration.defers_video_processing_to_consumer = true;
        configuration.allowed_file_extensions = Some(
            [
                "jpg", "jpeg", "png", "gif", "heic", "heif", "webp", "webm", "mp4", "mov", "m4v", "json",
                "lottie", "tgs", "ros",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        );
        configuration.untrimmed_long_video_message_key = Some("sticker_pack_media_too_long".to_string());
        configuration.keeps_animated_images = true;
        configuration
    }
}
