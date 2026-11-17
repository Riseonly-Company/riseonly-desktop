use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui::{App, Pixels, px};
use rise_core::foundation::foundation_date::TimeInterval;
use rise_ui::message_bubble_geometry::MessageBubbleRadii;
use rise_ui::message_media_tail_presentation::MessageMediaTailPresentation;
use rise_ui::swiftui::motion::{MotionSpec, timing_curve};

use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;

pub enum MediaViewerFrameKeys {}

impl MediaViewerFrameKeys {
    pub fn chat_bubble(media_item_id: &str) -> String {
        format!("media-{media_item_id}")
    }

    pub fn profile_grid(entry_id: &str) -> String {
        format!("profile-media-{entry_id}")
    }

    pub fn picker_asset(local_id: &str) -> String {
        format!("picker-{local_id}")
    }

    pub fn post_image(post_id: &str, index: usize) -> String {
        format!("post-image-{post_id}-{index}")
    }

    pub fn chat_avatar(chat_id: &str) -> String {
        format!("chat-avatar-{chat_id}")
    }

    pub fn music_cover(id: &str) -> String {
        format!("music-cover-{id}")
    }

    pub fn comment_media(comment_id: &str) -> String {
        format!("comment-media-{comment_id}")
    }

    pub fn user_avatar(user_id: &str) -> String {
        format!("user-avatar-{user_id}")
    }
}

pub enum MediaViewerTransition {}

impl MediaViewerTransition {
    pub const MOVE_DURATION: f64 = 0.42;

    pub fn move_animation() -> MotionSpec {
        timing_curve(0.22, 0.82, 0.12, 1.0, Self::MOVE_DURATION)
    }
}

/// playback timing of the viewer video chrome
/// local and not canon because canon needs a server pair per entry and a redraw cadence has none
pub enum MediaViewerVideoTiming {}

impl MediaViewerVideoTiming {
    pub const PROGRESS_PUBLICATION: TimeInterval = 0.1;
    pub const SCRUB_SEEK_TOLERANCE: TimeInterval = 0.2;
    pub const LOAD_TIMEOUT: TimeInterval = 20.0;
}

/// geometry of the thumbnail rail under the photo, local chrome and not canon
/// decode size follows the 3x of rendered size rule
pub enum MediaViewerFilmstripMetrics {}

impl MediaViewerFilmstripMetrics {
    pub const SLOT_WIDTH: Pixels = px(30.0);
    pub const HEIGHT: Pixels = px(38.0);
    pub const RESTING_HEIGHT: Pixels = px(24.0);
    pub const SPACING: Pixels = px(2.0);
    pub const CORNER_RADIUS: Pixels = px(5.0);
    pub const DECODE_PIXEL_DIMENSION: i64 = 120;
}

pub type ChatMediaHeroTransition = MediaViewerTransition;

#[derive(Clone, Debug, PartialEq)]
pub struct MediaViewerHeroSkin {
    pub display_source: String,
    pub uses_fit_foreground: bool,
    pub media_tail: Option<MessageMediaTailPresentation>,
    pub source_media_radii: Option<MessageBubbleRadii>,
}

impl MediaViewerHeroSkin {
    /// plain init with media tail and source media radii left empty
    pub fn new(display_source: impl Into<String>, uses_fit_foreground: bool) -> Self {
        Self {
            display_source: display_source.into(),
            uses_fit_foreground,
            media_tail: None,
            source_media_radii: None,
        }
    }
}

pub type ChatMediaHeroSkin = MediaViewerHeroSkin;

/// how much of the presentation the rail draws, the pager still spans every item
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaViewerFilmstripScope {
    WholeGallery,
    CurrentMessage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaViewerChrome {
    pub shows_bottom_bar: bool,
    pub can_report: bool,
    pub is_own_profile_album: bool,
    pub filmstrip_scope: MediaViewerFilmstripScope,
}

impl MediaViewerChrome {
    const DEFAULT: Self = Self {
        shows_bottom_bar: true,
        can_report: false,
        is_own_profile_album: false,
        filmstrip_scope: MediaViewerFilmstripScope::WholeGallery,
    };

    pub const CHAT: Self = Self {
        shows_bottom_bar: true,
        can_report: false,
        filmstrip_scope: MediaViewerFilmstripScope::CurrentMessage,
        ..Self::DEFAULT
    };
    pub const POST: Self = Self {
        shows_bottom_bar: true,
        can_report: true,
        ..Self::DEFAULT
    };
    pub const PROFILE_ALBUM: Self = Self {
        shows_bottom_bar: false,
        can_report: false,
        ..Self::DEFAULT
    };
    pub const OWN_PROFILE_ALBUM: Self = Self {
        shows_bottom_bar: true,
        can_report: false,
        is_own_profile_album: true,
        ..Self::DEFAULT
    };
}

impl Default for MediaViewerChrome {
    fn default() -> Self {
        Self::DEFAULT
    }
}

pub type MediaViewerAccessoryApplies = Rc<dyn Fn(&MediaViewerGalleryItem, &App) -> bool>;
pub type MediaViewerAccessoryPerform = Rc<dyn Fn(&MediaViewerGalleryItem, &mut App)>;

/// one action of the screen that opened the viewer
/// drawn at the end of the bottom bar where delete would be
/// the map uses it to move a draft into a grid
#[derive(Clone)]
pub struct MediaViewerAccessoryAction {
    pub system_image: String,
    pub accessibility_label: String,
    pub applies: MediaViewerAccessoryApplies,
    pub perform: MediaViewerAccessoryPerform,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaViewerDeleteMode {
    ForMe,
    ForEveryone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaViewerGalleryItemKind {
    Image,
    Video,
    Animation,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostDownloadRef {
    pub post_id: String,
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaViewerGalleryItem {
    pub id: String,
    pub kind: MediaViewerGalleryItemKind,
    pub url: String,
    pub thumbnail_url: Option<String>,
    pub width: f32,
    pub height: f32,
    pub duration: Option<TimeInterval>,

    pub sender_id: String,
    pub sender_name: String,
    pub sender_logo: Option<String>,
    pub created_at: TimeInterval,
    pub caption_title: String,
    pub caption: String,

    pub message_id: String,
    pub index_in_message: usize,
    pub media_item_id: String,
    pub uses_fit_letterbox: bool,
    pub frame_registry_id: String,
    pub is_own: bool,
    pub can_delete: bool,
    pub can_save: bool,
    pub can_forward: bool,
    pub can_reply: bool,
    pub can_show_in_context: bool,
    pub can_delete_for_me: bool,
    pub can_delete_for_everyone: bool,
    pub deletes_from_favourites: bool,
    pub can_set_as_main: bool,
    pub is_album_row: bool,
    pub display_source_override: Option<String>,
    pub persist_media_to_disk: bool,
    pub image_cache_policy: CleverImageCachePolicy,
    pub post_download: Option<PostDownloadRef>,
}

impl Eq for MediaViewerGalleryItem {}

impl Hash for MediaViewerGalleryItem {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl MediaViewerGalleryItem {
    pub fn cell_registry_id(&self) -> &str {
        &self.frame_registry_id
    }

    pub fn display_source(&self) -> String {
        if let Some(source) = RemoteMediaURL::optional_normalized(self.display_source_override.as_deref()) {
            return source;
        }
        RemoteMediaURL::display_source(&self.url, self.thumbnail_url.as_deref())
    }

    pub fn aspect_ratio(&self) -> f32 {
        if self.width > 0.0 && self.height > 0.0 {
            self.width / self.height
        } else {
            1.0
        }
    }
}

pub type ChatMediaGalleryItem = MediaViewerGalleryItem;
