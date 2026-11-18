use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use rise_canon::rise_key::{message as message_key, message_location as location_key};
use rise_canon::RiseLimits;
use rise_chat::{
    HistoryDirection as ChatHistoryDirection, LinkPreviewSize as ChatLinkPreviewSize,
    MediaKind as ChatMediaKind, MessageContentType as ChatMessageContentType,
    StickerKind as ChatStickerKind, SystemKind as ChatSystemKind,
};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_i18n::tr;
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;
use rise_deeplink::ENVIRONMENTS;
use rise_rtc::{CallHistoryState as RiseRTCCallHistoryState, CallMediaMode as RiseRTCCallMediaMode, RtcWireEnum};
use serde_json::{Map, Number, Value};

pub use crate::modules::chat::components::chat::composer::chat_composer_types::MessageCustomEmojiEntity;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;
pub use crate::modules::chat::components::chat::composer::remote_media_url_type_hooks::RemoteMediaURL;
use crate::modules::chat::stores::message::message_actions::message_types_auth_hooks::MessageTypesAuthHooks;
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageEntitiesCodec;
pub use crate::modules::media_picker::api::message_attachment_types_hooks::MessageLocationInfo;
use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, AuthorInfoMore, GetPostFeedResponse};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageDeleteError {
    UnresolvedServerId,
}

impl MessageDeleteError {
    pub fn error_description(&self) -> Option<String> {
        Some("Message server ID is not available".to_owned())
    }
}

impl fmt::Display for MessageDeleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for MessageDeleteError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageForwardError {
    InvalidRequest,
    InvalidResponse,
}

impl MessageForwardError {
    pub fn error_description(&self) -> Option<String> {
        match self {
            Self::InvalidRequest => Some("Invalid message forward request".to_owned()),
            Self::InvalidResponse => Some("Invalid message forward response".to_owned()),
        }
    }
}

impl fmt::Display for MessageForwardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for MessageForwardError {}

static MESSAGE_FORWARD_MAX_MESSAGES: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("max_forward_messages"));
static MESSAGE_FORWARD_MAX_DESTINATIONS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("max_forward_destinations"));
static MESSAGE_FORWARD_MAX_COPIES: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("max_forward_copies"));

pub enum MessageForwardLimits {}

impl MessageForwardLimits {
    pub fn max_messages() -> i64 {
        *MESSAGE_FORWARD_MAX_MESSAGES
    }

    pub fn max_destinations() -> i64 {
        *MESSAGE_FORWARD_MAX_DESTINATIONS
    }

    pub fn max_copies() -> i64 {
        *MESSAGE_FORWARD_MAX_COPIES
    }
}

pub enum MessageHistoryPaginationDispatch {}

impl MessageHistoryPaginationDispatch {
    pub fn perform(
        shift_cached_window: impl FnOnce() -> bool,
        request_remote_page: impl FnOnce(),
    ) -> bool {
        let did_shift_cached_window = shift_cached_window();
        request_remote_page();
        did_shift_cached_window
    }
}

static MESSAGE_DELETE_MAX_MESSAGES_PER_REQUEST: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("max_delete_messages"));

pub enum MessageDeleteBatchPlanner {}

impl MessageDeleteBatchPlanner {
    pub fn max_messages_per_request() -> i64 {
        *MESSAGE_DELETE_MAX_MESSAGES_PER_REQUEST
    }

    pub fn chunks(message_ids: &[String]) -> Vec<Vec<String>> {
        let mut seen: HashSet<String> = HashSet::new();
        let normalized: Vec<String> = message_ids
            .iter()
            .filter_map(|raw| {
                let value = trimming_whitespaces_and_newlines(raw);
                if value.is_empty() || !seen.insert(value.to_owned()) {
                    return None;
                }
                Some(value.to_owned())
            })
            .collect();
        let step = usize::try_from(Self::max_messages_per_request()).unwrap_or(1).max(1);
        normalized.chunks(step).map(<[String]>::to_vec).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum MessageContentType {
    #[default]
    Text,
    Image,
    Video,
    Animation,
    Voice,
    Audio,
    Document,
    Sticker,
    VideoNote,
    Contact,
    Location,
    Poll,
    Call,
    System,
    PostShort,
    Unknown(String),
}

impl MessageContentType {
    pub fn from_raw_value(raw_value: &str) -> Self {
        let value = raw_value.to_lowercase();
        let value = value.as_str();
        if value == ChatMessageContentType::Text.wire() {
            Self::Text
        } else if value == ChatMediaKind::Image.wire() || value == "photo" {
            Self::Image
        } else if value == ChatMediaKind::Video.wire() {
            Self::Video
        } else if value == ChatMediaKind::Animation.wire() || value == ChatMediaKind::Gif.wire() {
            Self::Animation
        } else if value == ChatMediaKind::Voice.wire() {
            Self::Voice
        } else if value == ChatMediaKind::Audio.wire() || value == "music" {
            Self::Audio
        } else if value == ChatMediaKind::Document.wire() || value == "file" {
            Self::Document
        } else if value == ChatMediaKind::Sticker.wire() {
            Self::Sticker
        } else if value == ChatMediaKind::VideoNote.wire() {
            Self::VideoNote
        } else if value == ChatMessageContentType::Contact.wire() {
            Self::Contact
        } else if value == ChatMessageContentType::Location.wire() || value == "geo" {
            Self::Location
        } else if value == ChatMessageContentType::Poll.wire() {
            Self::Poll
        } else if value == ChatMessageContentType::Call.wire() {
            Self::Call
        } else if value == ChatMessageContentType::System.wire() {
            Self::System
        } else if value == ChatMessageContentType::PostShort.wire()
            || value == "post"
            || value == "short"
            || value == message_key::SHARED_POST
        {
            Self::PostShort
        } else {
            Self::Unknown(raw_value.to_owned())
        }
    }

    pub fn raw_value(&self) -> &str {
        match self {
            Self::Text => ChatMessageContentType::Text.wire(),
            Self::Image => ChatMediaKind::Image.wire(),
            Self::Video => ChatMediaKind::Video.wire(),
            Self::Animation => ChatMediaKind::Animation.wire(),
            Self::Voice => ChatMediaKind::Voice.wire(),
            Self::Audio => ChatMediaKind::Audio.wire(),
            Self::Document => ChatMediaKind::Document.wire(),
            Self::Sticker => ChatMediaKind::Sticker.wire(),
            Self::VideoNote => ChatMediaKind::VideoNote.wire(),
            Self::Contact => ChatMessageContentType::Contact.wire(),
            Self::Location => ChatMessageContentType::Location.wire(),
            Self::Poll => ChatMessageContentType::Poll.wire(),
            Self::Call => ChatMessageContentType::Call.wire(),
            Self::System => ChatMessageContentType::System.wire(),
            Self::PostShort => ChatMessageContentType::PostShort.wire(),
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageMediaKind {
    Image,
    Video,
    Animation,
    Audio,
    Voice,
    Document,
    Sticker,
    VideoNote,
    Gif,
}

impl MessageMediaKind {
    pub const ALL: [MessageMediaKind; 9] = [
        Self::Image,
        Self::Video,
        Self::Animation,
        Self::Audio,
        Self::Voice,
        Self::Document,
        Self::Sticker,
        Self::VideoNote,
        Self::Gif,
    ];

    pub fn raw_value(self) -> &'static str {
        self.canon().wire()
    }

    pub fn from_raw(raw_value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.raw_value() == raw_value)
    }

    fn canon(self) -> ChatMediaKind {
        match self {
            Self::Image => ChatMediaKind::Image,
            Self::Video => ChatMediaKind::Video,
            Self::Animation => ChatMediaKind::Animation,
            Self::Audio => ChatMediaKind::Audio,
            Self::Voice => ChatMediaKind::Voice,
            Self::Document => ChatMediaKind::Document,
            Self::Sticker => ChatMediaKind::Sticker,
            Self::VideoNote => ChatMediaKind::VideoNote,
            Self::Gif => ChatMediaKind::Gif,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageStickerKind {
    Static,
    Animated,
    Lottie,
}

impl MessageStickerKind {
    pub const ALL: [MessageStickerKind; 3] = [Self::Static, Self::Animated, Self::Lottie];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Static => ChatStickerKind::StaticImage.wire(),
            Self::Animated => ChatStickerKind::Animated.wire(),
            Self::Lottie => ChatStickerKind::Lottie.wire(),
        }
    }

    pub fn from_raw(raw_value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.raw_value() == raw_value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct MessageMediaVariant {
    pub url: String,
    pub width: i64,
    pub height: i64,
    pub size: Option<i64>,
    pub mime: Option<String>,
}

const LOCAL_PREVIEW_PREFIXES: [&str; 4] = ["file://", "/", "content://", "ph://"];
const GENERATED_CHAT_IMAGE_PREFIX: &str = "ro-chat-img-";
const VIDEO_FILE_EXTENSIONS: [&str; 3] = [".mp4", ".mov", ".m4v"];
const DECODABLE_IMAGE_EXTENSIONS: [&str; 10] =
    ["jpg", "jpeg", "png", "heic", "heif", "webp", "gif", "bmp", "tif", "tiff"];
const MP3_MIME_TYPES: [&str; 3] = ["audio/mpeg", "audio/mp3", "audio/x-mp3"];
const MP3_EXTENSION: &str = "mp3";
const UPLOAD_OVERLAY_DONE_PERCENT: f64 = 99.5;

fn ns_path_extension(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_owned()
}

fn path_before_query(trimmed: &str) -> &str {
    trimmed.split('?').next().unwrap_or(trimmed)
}

fn standardized_file_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// how the recipient may open a view once or timed photo or video
/// the server keeps the window per viewer, first open starts it and reopening doesnt extend it
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageMediaViewLimit {
    pub mode: String,
    pub seconds: Option<i64>,
    pub expires_at_ms: Option<i64>,
    pub state: Option<String>,
}

static MESSAGE_MEDIA_VIEW_MODES: LazyLock<&'static [&'static str]> =
    LazyLock::new(|| RiseFieldRule::new("message_create_message", "/media_items/*/view_mode").allowed_values());

impl MessageMediaViewLimit {
    pub const STATE_CONSUMED: &'static str = "consumed";
    pub const STATE_OPENED: &'static str = "opened";

    /// the server destroyed the file, a view once was closed or a timed window ran out
    pub fn is_destroyed_on_server(&self) -> bool {
        self.state.as_deref() == Some(Self::STATE_CONSUMED)
    }

    pub fn is_timed(&self) -> bool {
        let modes = *MESSAGE_MEDIA_VIEW_MODES;
        modes.len() > 1 && self.mode == modes[1]
    }

    /// the window ran out for this viewer so the file is gone for them
    pub fn is_consumed(&self, now: Date) -> bool {
        if self.is_destroyed_on_server() {
            return true;
        }
        let Some(expires_at_ms) = self.expires_at_ms else {
            return false;
        };
        (now.time_interval_since_1970() * MILLISECONDS_PER_SECOND) as i64 >= expires_at_ms
    }

    /// sender copy of a timed photo the peer opened, the end of the peer countdown
    /// server stamps the peer window end which includes download grace, none until the peer opens it
    pub fn sender_countdown_deadline_ms(&self) -> Option<i64> {
        if !self.is_timed() || self.is_destroyed_on_server() {
            return None;
        }
        let expires_at_ms = self.expires_at_ms?;
        self.seconds.filter(|seconds| *seconds > 0)?;
        Some(expires_at_ms - *MESSAGE_VIEW_GRACE_MS)
    }
}

static MESSAGE_VIEW_GRACE_MS: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("message_view_grace_ms"));

/// what a view limited tile shows in the bubble, derived per viewer
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageMediaViewPhase {
    Sealed,
    Consumed,
    Sent { viewed: bool },
    Countdown { deadline_ms: i64, seconds: i64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageMediaItem {
    pub id: String,
    pub kind: MessageMediaKind,
    pub url: String,
    pub thumbnail_url: Option<String>,
    pub width: f32,
    pub height: f32,
    pub duration: Option<f64>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub mime_type: Option<String>,
    pub bitrate: Option<i64>,
    pub fps: Option<i64>,
    pub codec: Option<String>,
    pub variants: Vec<MessageMediaVariant>,
    pub waveform: Vec<f32>,
    pub sticker_kind: Option<MessageStickerKind>,
    pub sticker_emojis: Vec<String>,
    pub pack_id: Option<String>,
    pub has_spoiler: bool,
    pub view_limit: Option<MessageMediaViewLimit>,
    pub view_phase: Option<MessageMediaViewPhase>,

    pub is_uploading: bool,
    pub is_compressing: bool,
    pub upload_progress: f64,
    pub compression_progress: f64,
    pub uploaded_bytes: i64,
    pub total_bytes: i64,
    pub local_preview_url: Option<String>,
    pub is_cancelled: bool,
    pub local_slot_index: i64,
}

impl Eq for MessageMediaItem {}

impl std::hash::Hash for MessageMediaItem {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        use std::hash::Hash;
        self.id.hash(state);
        self.kind.hash(state);
        self.url.hash(state);
        self.thumbnail_url.hash(state);
        (self.width + 0.0).to_bits().hash(state);
        (self.height + 0.0).to_bits().hash(state);
        self.duration.map(|duration| (duration + 0.0).to_bits()).hash(state);
        self.file_name.hash(state);
        self.file_size.hash(state);
        self.mime_type.hash(state);
        self.bitrate.hash(state);
        self.fps.hash(state);
        self.codec.hash(state);
        self.variants.hash(state);
        self.waveform.len().hash(state);
        for sample in &self.waveform {
            (sample + 0.0).to_bits().hash(state);
        }
        self.sticker_kind.hash(state);
        self.sticker_emojis.hash(state);
        self.pack_id.hash(state);
        self.has_spoiler.hash(state);
        self.view_limit.hash(state);
        self.view_phase.hash(state);
        self.is_uploading.hash(state);
        self.is_compressing.hash(state);
        (self.upload_progress + 0.0).to_bits().hash(state);
        (self.compression_progress + 0.0).to_bits().hash(state);
        self.uploaded_bytes.hash(state);
        self.total_bytes.hash(state);
        self.local_preview_url.hash(state);
        self.is_cancelled.hash(state);
        self.local_slot_index.hash(state);
    }
}

impl MessageMediaItem {
    pub fn new(id: String, kind: MessageMediaKind, url: String) -> Self {
        Self {
            id,
            kind,
            url,
            thumbnail_url: None,
            width: 0.0,
            height: 0.0,
            duration: None,
            file_name: None,
            file_size: None,
            mime_type: None,
            bitrate: None,
            fps: None,
            codec: None,
            variants: Vec::new(),
            waveform: Vec::new(),
            sticker_kind: None,
            sticker_emojis: Vec::new(),
            pack_id: None,
            has_spoiler: false,
            view_limit: None,
            view_phase: None,
            is_uploading: false,
            is_compressing: false,
            upload_progress: 0.0,
            compression_progress: 0.0,
            uploaded_bytes: 0,
            total_bytes: 0,
            local_preview_url: None,
            is_cancelled: false,
            local_slot_index: 0,
        }
    }

    pub fn stable_render_id(&self, owner_local_id: &str) -> String {
        Self::stable_render_id_for_slot(owner_local_id, self.local_slot_index).unwrap_or_else(|| self.id.clone())
    }

    pub fn stable_render_id_for_slot(owner_local_id: &str, slot_index: i64) -> Option<String> {
        let owner = trimming_whitespaces_and_newlines(owner_local_id);
        if owner.is_empty() {
            return None;
        }
        Some(format!("{}:{}:media:{}", owner.len(), owner, slot_index.max(0)))
    }

    pub fn aspect_ratio(&self) -> f32 {
        if self.width <= 0.0 || self.height <= 0.0 {
            return 1.0;
        }
        self.width / self.height
    }

    pub fn resolved_local_preview_url(&self) -> Option<String> {
        if let Some(local) = self.local_preview_url.as_deref().map(trimming_whitespaces_and_newlines) {
            if !local.is_empty()
                && Self::is_local_preview_url(local)
                && Self::is_available_local_preview_url(local)
                && !Self::is_local_video_file(local)
            {
                return Some(local.to_owned());
            }
        }
        if Self::is_local_preview_url(&self.url)
            && Self::is_available_local_preview_url(&self.url)
            && !Self::is_local_video_file(&self.url)
        {
            return Some(self.url.clone());
        }
        if let Some(thumb) = self.thumbnail_url.as_deref() {
            if Self::is_local_preview_url(thumb)
                && Self::is_available_local_preview_url(thumb)
                && !Self::is_local_video_file(thumb)
            {
                return Some(thumb.to_owned());
            }
        }
        None
    }

    pub fn display_source(&self) -> String {
        if !self.is_uploading && !self.is_compressing {
            match self.kind {
                MessageMediaKind::Image
                | MessageMediaKind::Video
                | MessageMediaKind::VideoNote
                | MessageMediaKind::Animation
                | MessageMediaKind::Gif => {
                    for candidate in [self.thumbnail_url.as_deref(), Some(self.url.as_str())] {
                        if let Some(remote) = RemoteMediaURL::optional_normalized(candidate) {
                            let lowered = remote.to_lowercase();
                            if lowered.starts_with("http://") || lowered.starts_with("https://") {
                                return remote;
                            }
                        }
                    }
                }
                MessageMediaKind::Audio
                | MessageMediaKind::Voice
                | MessageMediaKind::Document
                | MessageMediaKind::Sticker => {}
            }
        }
        if let Some(local) = self.resolved_local_preview_url() {
            return local;
        }
        let remote = RemoteMediaURL::display_source(&self.url, self.thumbnail_url.as_deref());
        if Self::is_local_video_file(&remote) {
            return String::new();
        }
        remote
    }

    pub fn is_local_preview_url(raw: &str) -> bool {
        let t = trimming_whitespaces_and_newlines(raw).to_lowercase();
        LOCAL_PREVIEW_PREFIXES.iter().any(|prefix| t.starts_with(prefix)) || is_local_absolute_path(&t)
    }

    pub fn is_available_local_preview_url(raw: &str) -> bool {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        let path = if trimmed.to_lowercase().starts_with("file://") {
            let Some(url) = Url::from_string(trimmed) else {
                return true;
            };
            if !url.is_file_url() {
                return true;
            }
            let Some(path) = url.to_file_path() else {
                return true;
            };
            path
        } else if trimmed.starts_with('/') || is_local_absolute_path(trimmed) {
            PathBuf::from(trimmed)
        } else {
            return true;
        };
        let standardized = standardized_file_path(&path);
        let is_generated_chat_image = standardized
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(GENERATED_CHAT_IMAGE_PREFIX))
            && standardized.parent() == Some(standardized_file_path(&temporary_directory()).as_path());
        !is_generated_chat_image || FileManager.file_exists(&standardized)
    }

    pub fn is_local_video_file(raw: &str) -> bool {
        let t = trimming_whitespaces_and_newlines(raw).to_lowercase();
        if !Self::is_local_preview_url(&t) || t.starts_with("ph://") {
            return false;
        }
        Self::has_video_file_extension(&t)
    }

    pub fn has_video_file_extension(raw: &str) -> bool {
        let trimmed = trimming_whitespaces_and_newlines(raw).to_lowercase();
        let path = path_before_query(&trimmed);
        VIDEO_FILE_EXTENSIONS.iter().any(|extension| path.ends_with(extension))
    }

    pub fn has_image_file_extension(raw: &str) -> bool {
        let trimmed = trimming_whitespaces_and_newlines(raw).to_lowercase();
        let path = path_before_query(&trimmed);
        DECODABLE_IMAGE_EXTENSIONS.contains(&ns_path_extension(path).as_str())
    }

    pub fn is_decodable_image_source(raw: &str) -> bool {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        if trimmed.is_empty() || Self::has_video_file_extension(trimmed) {
            return false;
        }
        if !Self::is_local_preview_url(trimmed) {
            return true;
        }
        Self::has_image_file_extension(trimmed)
    }

    pub fn is_upload_queued(&self) -> bool {
        if !self.is_uploading || self.is_compressing {
            return false;
        }
        self.upload_progress <= 0.0 && self.uploaded_bytes <= 0 && self.compression_progress <= 0.0
    }

    pub fn has_active_upload_work(&self) -> bool {
        self.is_compressing || self.upload_progress > 0.0 || self.uploaded_bytes > 0
    }

    pub fn shows_upload_overlay(&self) -> bool {
        if self.is_upload_queued() {
            return true;
        }
        if self.is_compressing {
            return true;
        }
        if !self.is_uploading {
            return false;
        }
        self.upload_progress < UPLOAD_OVERLAY_DONE_PERCENT
    }

    pub fn shows_detailed_upload_overlay(&self) -> bool {
        self.shows_upload_overlay() && !self.is_upload_queued() && self.has_active_upload_work()
    }

    pub fn effective_total_bytes(&self) -> i64 {
        if self.total_bytes > 0 {
            return self.total_bytes;
        }
        self.file_size.unwrap_or(0).max(0)
    }

    pub fn clamped_uploaded_bytes(&self) -> i64 {
        let total = self.effective_total_bytes();
        if total <= 0 {
            return self.uploaded_bytes.max(0);
        }
        total.min(self.uploaded_bytes.max(0))
    }

    pub fn remaining_upload_bytes(&self) -> i64 {
        (self.effective_total_bytes() - self.clamped_uploaded_bytes()).max(0)
    }

    pub fn normalized_transfer_progress(&self) -> f64 {
        if self.is_compressing {
            return (self.compression_progress / 100.0).clamp(0.0, 1.0);
        }
        let total = self.effective_total_bytes();
        if total > 0 {
            return (self.clamped_uploaded_bytes() as f64 / total as f64).clamp(0.0, 1.0);
        }
        (self.upload_progress / 100.0).clamp(0.0, 1.0)
    }

    pub fn is_importable_mp3(&self) -> bool {
        if self.kind != MessageMediaKind::Audio && self.kind != MessageMediaKind::Document {
            return false;
        }
        self.has_mp3_signal()
    }

    pub fn has_mp3_signal(&self) -> bool {
        let normalized_mime = self.mime_type.as_deref().and_then(|mime| {
            mime.split(';')
                .next()
                .map(|first| trimming_whitespaces_and_newlines(first).to_lowercase())
        });
        let has_mp3_mime = normalized_mime
            .as_deref()
            .is_some_and(|mime| MP3_MIME_TYPES.contains(&mime));

        has_mp3_mime
            || Self::has_mp3_extension(self.file_name.as_deref())
            || Self::has_mp3_extension(Some(self.url.as_str()))
            || Self::has_mp3_extension(self.local_preview_url.as_deref())
    }

    fn has_mp3_extension(raw: Option<&str>) -> bool {
        let Some(raw) = raw else {
            return false;
        };
        let trimmed = trimming_whitespaces_and_newlines(raw);
        if trimmed.is_empty() {
            return false;
        }

        let path = match UrlComponents::from_string(trimmed).map(|components| components.path()) {
            Some(path) if !path.is_empty() => path,
            _ => path_before_query(trimmed).to_owned(),
        };
        ns_path_extension(&path).to_lowercase() == MP3_EXTENSION
    }

    pub fn is_grid_media(&self) -> bool {
        matches!(
            self.kind,
            MessageMediaKind::Image | MessageMediaKind::Video | MessageMediaKind::Animation | MessageMediaKind::Gif
        )
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MessageReplyTo {
    pub message_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub content_type: MessageContentType,
    pub content: String,
    pub caption: Option<String>,
    pub media_items: Vec<MessageMediaItem>,
    pub created_at: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageForwardInfo {
    pub from_chat_id: Option<String>,
    pub from_message_id: Option<String>,
    pub original_message_id: Option<String>,
    pub sender_id: Option<String>,
    pub sender_tag: Option<String>,
    pub sender_name: String,
    pub sender_logo: Option<String>,
    pub from_chat_title: Option<String>,
    pub from_chat_logo: Option<String>,
    pub date: f64,
    pub forward_chain_count: i64,
}

impl Default for MessageForwardInfo {
    fn default() -> Self {
        Self {
            from_chat_id: None,
            from_message_id: None,
            original_message_id: None,
            sender_id: None,
            sender_tag: None,
            sender_name: String::new(),
            sender_logo: None,
            from_chat_title: None,
            from_chat_logo: None,
            date: 0.0,
            forward_chain_count: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum MessageLinkPreviewSize {
    #[default]
    Small,
    Big,
}

impl MessageLinkPreviewSize {
    pub const ALL: [MessageLinkPreviewSize; 2] = [Self::Small, Self::Big];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Small => ChatLinkPreviewSize::Small.wire(),
            Self::Big => ChatLinkPreviewSize::Big.wire(),
        }
    }

    pub fn from_raw(raw_value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|size| size.raw_value() == raw_value)
    }
}

const LINK_PREVIEW_DEFAULT_IMAGE_ASPECT: f32 = 1.91;
const SITE_STOCK_IMAGE_PATH: &str = "/og-image.png";
const SITE_TITLE_BRAND_SUFFIX: &str = " on Riseonly";
const WEB_SCHEME_PREFIX: &str = "https://";

#[derive(Clone, Debug, PartialEq)]
pub struct MessageLinkPreview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    pub image_url: Option<String>,
    pub image_aspect: f32,
    pub size: MessageLinkPreviewSize,
    pub is_above_text: bool,
    pub feed_owns_voice_card: bool,
    pub voice_card_pending: bool,
}

impl Default for MessageLinkPreview {
    fn default() -> Self {
        Self {
            url: String::new(),
            title: None,
            description: None,
            site_name: None,
            image_url: None,
            image_aspect: LINK_PREVIEW_DEFAULT_IMAGE_ASPECT,
            size: MessageLinkPreviewSize::Small,
            is_above_text: false,
            feed_owns_voice_card: false,
            voice_card_pending: false,
        }
    }
}

impl MessageLinkPreview {
    pub fn localized_for_own_link(&self) -> MessageLinkPreview {
        let raw = if self.url.contains("://") {
            self.url.clone()
        } else {
            format!("{WEB_SCHEME_PREFIX}{}", self.url)
        };
        let Some(link) = Url::from_string(&raw) else {
            return self.clone();
        };
        let Some(deep_link) = UniversalLinkParser::parse(&link, AppEnvironment::current()) else {
            return self.clone();
        };
        let Some(kind) = Self::kind_title(&deep_link) else {
            return self.clone();
        };
        let mut copy = self.clone();
        let is_stock_image = self
            .image_url
            .as_deref()
            .and_then(Url::from_string)
            .is_some_and(|image| {
                let host = image.host();
                image.path() == SITE_STOCK_IMAGE_PATH
                    && (host.is_none() || host == link.host() || Self::is_site_host(host.as_deref()))
            });
        if is_stock_image {
            if matches!(deep_link, PushDeepLink::PublicTag { .. }) {
                return self.clone();
            }
            copy.title = Some(kind);
            copy.description = None;
            copy.image_url = None;
            return copy;
        }
        if let Some(title) = self.title.as_deref() {
            let suffix_length = SITE_TITLE_BRAND_SUFFIX.len();
            if title.len() > suffix_length {
                let cut = title.len() - suffix_length;
                if title.is_char_boundary(cut) && title[cut..].eq_ignore_ascii_case(SITE_TITLE_BRAND_SUFFIX) {
                    copy.title = Some(title[..cut].to_owned());
                }
            }
        }
        copy
    }

    fn is_site_host(host: Option<&str>) -> bool {
        let Some(host) = host.map(str::to_lowercase) else {
            return false;
        };
        ENVIRONMENTS.iter().any(|environment| environment.accepts_web_host(&host))
    }

    fn kind_title(link: &PushDeepLink) -> Option<String> {
        match link {
            PushDeepLink::Post { .. } => Some(tr("report_type_post")),
            PushDeepLink::PostComment { .. } => Some(tr("report_type_comment")),
            PushDeepLink::Profile { .. } | PushDeepLink::PublicTag { .. } => Some(tr("navbtn_profile")),
            PushDeepLink::Story { .. } => Some(tr("report_type_story")),
            PushDeepLink::Highlight { .. } => Some(tr("highlight_share_text")),
            PushDeepLink::Chat { .. } | PushDeepLink::ChatTopic { .. } | PushDeepLink::ChatMessage { .. } => {
                Some(tr("report_type_chat"))
            }
            PushDeepLink::BotChat { .. } | PushDeepLink::BotMiniApp { .. } => Some(tr("report_type_bot")),
            PushDeepLink::MusicAlbum { .. } => Some(tr("music_track_album")),
            PushDeepLink::MusicTrack { .. }
            | PushDeepLink::MusicReleaseTrack { .. }
            | PushDeepLink::PlaylistTrack { .. } => Some(tr("report_type_music_track")),
            PushDeepLink::AddStickerPack { .. } => Some(tr("sticker_pack_share_title")),
            PushDeepLink::JoinChat { .. } => Some(tr("voice_invite_preview_title")),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct MessageContactInfo {
    pub name: String,
    pub phone: String,
    pub user_id: Option<String>,
    pub avatar_url: Option<String>,
}

const MILLISECONDS_PER_SECOND: f64 = 1_000.0;
const LATITUDE_RANGE: std::ops::RangeInclusive<f64> = -90.0..=90.0;
const LONGITUDE_RANGE: std::ops::RangeInclusive<f64> = -180.0..=180.0;

pub enum MessageLocationCodec {}

impl MessageLocationCodec {
    pub fn encode(location: &MessageLocationInfo) -> Option<String> {
        let mut object = Map::new();
        object.insert(location_key::LATITUDE.to_owned(), Value::Number(Number::from_f64(location.latitude)?));
        object.insert(location_key::LONGITUDE.to_owned(), Value::Number(Number::from_f64(location.longitude)?));
        if let Some(address) = location.address.as_deref().filter(|address| !address.is_empty()) {
            object.insert(location_key::ADDRESS.to_owned(), Value::String(address.to_owned()));
        }
        if let (Some(period), Some(until)) = (location.live_period, location.live_until) {
            object.insert(location_key::LIVE_PERIOD.to_owned(), Value::from(period as i64));
            object.insert(
                location_key::LIVE_UNTIL.to_owned(),
                Value::from((until.time_interval_since_1970() * MILLISECONDS_PER_SECOND) as i64),
            );
        }
        let mut sorted: Vec<(String, Value)> = object.into_iter().collect();
        sorted.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));
        let object: Map<String, Value> = sorted.into_iter().collect();
        serde_json::to_string(&Value::Object(object)).ok()
    }

    pub fn decode(content: &str) -> Option<MessageLocationInfo> {
        let Value::Object(object) = serde_json::from_str::<Value>(content).ok()? else {
            return None;
        };
        let latitude = object.get(location_key::LATITUDE).and_then(Value::as_f64)?;
        let longitude = object.get(location_key::LONGITUDE).and_then(Value::as_f64)?;
        if !LATITUDE_RANGE.contains(&latitude) || !LONGITUDE_RANGE.contains(&longitude) {
            return None;
        }
        let address = object
            .get(location_key::ADDRESS)
            .and_then(Value::as_str)
            .map(|address| trimming_whitespaces_and_newlines(address).to_owned());
        let period = object.get(location_key::LIVE_PERIOD).and_then(Value::as_f64);
        let until_ms = object.get(location_key::LIVE_UNTIL).and_then(Value::as_f64);
        let is_live = period.unwrap_or(0.0) > 0.0 && until_ms.unwrap_or(0.0) > 0.0;
        Some(MessageLocationInfo {
            latitude,
            longitude,
            title: None,
            address: address.filter(|address| !address.is_empty()),
            live_period: if is_live { period } else { None },
            live_until: if is_live {
                until_ms.map(|ms| Date::from_time_interval_since_1970(ms / MILLISECONDS_PER_SECOND))
            } else {
                None
            },
        })
    }
}

impl GetMessageMessage {
    pub fn adopt_location_payload(&mut self) {
        if self.content_type != MessageContentType::Location || self.location.is_some() {
            return;
        }
        let Some(decoded) = MessageLocationCodec::decode(&self.text) else {
            return;
        };
        let is_live = decoded.live();
        self.location = Some(decoded);
        self.text = String::new();
        if is_live {
            self.edited_at = None;
        }
    }

    pub fn apply_edited_content(&mut self, content: Option<&str>, edited_at: f64) {
        if let Some(content) = content {
            if self.original_text.is_none() {
                self.original_text = Some(self.text.clone());
            }
            self.text = content.to_owned();
        }
        self.edited_at = Some(edited_at);
        if self.content_type != MessageContentType::Location || content.is_none() {
            return;
        }
        self.location = None;
        self.adopt_location_payload();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct MessagePollOption {
    pub id: String,
    pub text: String,
    pub voter_count: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessagePollInfo {
    pub question: String,
    pub options: Vec<MessagePollOption>,
    pub total_voters: i64,
    pub is_anonymous: bool,
    pub allows_multiple: bool,
    pub quiz_correct_id: Option<String>,
    pub closed: bool,
    pub chosen_option_ids: Vec<String>,
}

impl Default for MessagePollInfo {
    fn default() -> Self {
        Self {
            question: String::new(),
            options: Vec::new(),
            total_voters: 0,
            is_anonymous: true,
            allows_multiple: false,
            quiz_correct_id: None,
            closed: false,
            chosen_option_ids: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct MessageBotKeyboardButton {
    pub text: String,
    pub callback_data: Option<String>,
    pub url: Option<String>,
    pub web_app_url: Option<String>,
    pub bot_id: Option<String>,
    pub mini_app_id: Option<String>,
    pub switch_inline_query: Option<String>,
    pub switch_inline_query_current_chat: Option<String>,
    pub is_pay: bool,
    pub is_game: bool,
}

impl MessageBotKeyboardButton {
    pub fn is_plain_text(&self) -> bool {
        self.callback_data.is_none()
            && self.url.is_none()
            && self.web_app_url.is_none()
            && self.switch_inline_query.is_none()
            && self.switch_inline_query_current_chat.is_none()
            && !self.is_pay
            && !self.is_game
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct MessageBotReplyMarkup {
    pub inline_keyboard: Vec<Vec<MessageBotKeyboardButton>>,
    pub keyboard: Vec<Vec<MessageBotKeyboardButton>>,
    pub resize_keyboard: bool,
    pub one_time_keyboard: bool,
    pub selective: bool,
    pub input_field_placeholder: Option<String>,
    pub remove_keyboard: bool,
    pub force_reply: bool,
}

impl MessageBotReplyMarkup {
    pub fn has_inline_keyboard(&self) -> bool {
        self.inline_keyboard.iter().any(|row| !row.is_empty())
    }

    pub fn has_reply_keyboard(&self) -> bool {
        self.keyboard.iter().any(|row| !row.is_empty())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct MessageBotWebAppData {
    pub data: String,
    pub button_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct MessageSharedPostAuthor {
    pub id: String,
    pub tag: String,
    pub name: String,
    pub logo: String,
    pub is_premium: bool,
    pub is_official: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageSharedPost {
    pub post_id: String,
    pub is_short: bool,
    pub is_riff: bool,
    pub show_in_publications: bool,
    pub title: String,
    pub snippet: String,
    pub images: Vec<String>,
    pub video: String,
    pub video_thumbnail: String,
    pub likes_count: i64,
    pub comments_count: i64,
    pub author: MessageSharedPostAuthor,
}

impl Default for MessageSharedPost {
    fn default() -> Self {
        Self {
            post_id: String::new(),
            is_short: false,
            is_riff: false,
            show_in_publications: true,
            title: String::new(),
            snippet: String::new(),
            images: Vec::new(),
            video: String::new(),
            video_thumbnail: String::new(),
            likes_count: 0,
            comments_count: 0,
            author: MessageSharedPostAuthor::default(),
        }
    }
}

impl MessageSharedPost {
    pub fn display_thumbnail(&self) -> Option<String> {
        let thumb = trimming_whitespaces_and_newlines(&self.video_thumbnail);
        if !thumb.is_empty() {
            return Some(thumb.to_owned());
        }
        self.images
            .iter()
            .find(|image| !trimming_whitespaces_and_newlines(image).is_empty())
            .cloned()
    }

    pub fn as_feed_post(&self) -> GetPostFeedResponse {
        GetPostFeedResponse {
            id: self.post_id.clone(),
            title: self.title.clone(),
            content: self.snippet.clone(),
            images: self.images.clone(),
            video: (!self.video.is_empty()).then(|| self.video.clone()),
            video_thumbnail: (!self.video_thumbnail.is_empty()).then(|| self.video_thumbnail.clone()),
            is_short: self.is_short,
            is_riff: self.is_riff,
            show_in_publications: self.show_in_publications,
            likes_count: self.likes_count,
            comments_count: self.comments_count,
            author: AuthorInfo {
                id: Some(self.author.id.clone()),
                name: self.author.name.clone(),
                tag: self.author.tag.clone(),
                more: AuthorInfoMore {
                    logo: self.author.logo.clone(),
                    is_premium: self.author.is_premium,
                    is_official: self.author.is_official,
                    ..AuthorInfoMore::default()
                },
                ..AuthorInfo::default()
            },
            ..GetPostFeedResponse::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct MessageStoryReplyAuthor {
    pub id: String,
    pub tag: String,
    pub name: String,
    pub logo: String,
    pub is_premium: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct MessageStoryReply {
    pub story_id: i64,
    pub media_url: String,
    pub is_video: bool,
    pub caption: String,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub post_to_profile: Option<bool>,
    pub author: MessageStoryReplyAuthor,
    pub is_known_gone: bool,
}

impl MessageStoryReply {
    pub fn is_past_display_window(&self) -> bool {
        if self.expires_at_ms <= 0 {
            return false;
        }
        RiseClock::now().time_interval_since_1970() * MILLISECONDS_PER_SECOND >= self.expires_at_ms as f64
    }

    pub fn is_unavailable(&self) -> bool {
        self.is_known_gone || (self.is_past_display_window() && self.post_to_profile == Some(false))
    }

    pub fn thumbnail_source(&self) -> Option<String> {
        if self.is_video || self.is_unavailable() {
            return None;
        }
        RemoteMediaURL::optional_normalized(Some(self.media_url.as_str()))
    }

    pub fn video_poster_source(&self) -> Option<String> {
        if !self.is_video || self.is_unavailable() || self.story_id <= 0 {
            return None;
        }
        RemoteMediaURL::optional_normalized(Some(self.media_url.as_str()))
    }

    pub fn is_openable(&self) -> bool {
        self.story_id > 0 && !self.is_unavailable()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum MessageSystemKind {
    UserJoined,
    UserLeft,
    UserAdded,
    UserRemoved,
    MemberJoined,
    MemberLeft,
    MemberKicked,
    MemberBanned,
    ChatCreated,
    ChatRenamed,
    ChatPhotoChanged,
    ChatPhotoRemoved,
    ChatWallpaperChanged,
    ChatWallpaperRemoved,
    MessagePinned,
    MessageUnpinned,
    VoiceCallStarted,
    VoiceCallEnded,
    VideoCallStarted,
    VideoCallEnded,
    SecretChatStarted,
    SecretChatTtlChanged,
    ChannelCreated,
    MigratedFromGroup,
    ScoreReached,
    TopicCreated,
    ContactJoined,
    PremiumGifted,
    GiftSent,
    #[default]
    Generic,
}

impl MessageSystemKind {
    pub const ALL: [MessageSystemKind; 30] = [
        Self::UserJoined,
        Self::UserLeft,
        Self::UserAdded,
        Self::UserRemoved,
        Self::MemberJoined,
        Self::MemberLeft,
        Self::MemberKicked,
        Self::MemberBanned,
        Self::ChatCreated,
        Self::ChatRenamed,
        Self::ChatPhotoChanged,
        Self::ChatPhotoRemoved,
        Self::ChatWallpaperChanged,
        Self::ChatWallpaperRemoved,
        Self::MessagePinned,
        Self::MessageUnpinned,
        Self::VoiceCallStarted,
        Self::VoiceCallEnded,
        Self::VideoCallStarted,
        Self::VideoCallEnded,
        Self::SecretChatStarted,
        Self::SecretChatTtlChanged,
        Self::ChannelCreated,
        Self::MigratedFromGroup,
        Self::ScoreReached,
        Self::TopicCreated,
        Self::ContactJoined,
        Self::PremiumGifted,
        Self::GiftSent,
        Self::Generic,
    ];

    pub fn raw_value(self) -> &'static str {
        self.canon().wire()
    }

    pub fn from_raw(raw_value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.raw_value() == raw_value)
    }

    fn canon(self) -> ChatSystemKind {
        match self {
            Self::UserJoined => ChatSystemKind::UserJoined,
            Self::UserLeft => ChatSystemKind::UserLeft,
            Self::UserAdded => ChatSystemKind::UserAdded,
            Self::UserRemoved => ChatSystemKind::UserRemoved,
            Self::MemberJoined => ChatSystemKind::MemberJoined,
            Self::MemberLeft => ChatSystemKind::MemberLeft,
            Self::MemberKicked => ChatSystemKind::MemberKicked,
            Self::MemberBanned => ChatSystemKind::MemberBanned,
            Self::ChatCreated => ChatSystemKind::ChatCreated,
            Self::ChatRenamed => ChatSystemKind::ChatRenamed,
            Self::ChatPhotoChanged => ChatSystemKind::ChatPhotoChanged,
            Self::ChatPhotoRemoved => ChatSystemKind::ChatPhotoRemoved,
            Self::ChatWallpaperChanged => ChatSystemKind::ChatWallpaperChanged,
            Self::ChatWallpaperRemoved => ChatSystemKind::ChatWallpaperRemoved,
            Self::MessagePinned => ChatSystemKind::MessagePinned,
            Self::MessageUnpinned => ChatSystemKind::MessageUnpinned,
            Self::VoiceCallStarted => ChatSystemKind::VoiceCallStarted,
            Self::VoiceCallEnded => ChatSystemKind::VoiceCallEnded,
            Self::VideoCallStarted => ChatSystemKind::VideoCallStarted,
            Self::VideoCallEnded => ChatSystemKind::VideoCallEnded,
            Self::SecretChatStarted => ChatSystemKind::SecretChatStarted,
            Self::SecretChatTtlChanged => ChatSystemKind::SecretChatTtlChanged,
            Self::ChannelCreated => ChatSystemKind::ChannelCreated,
            Self::MigratedFromGroup => ChatSystemKind::MigratedFromGroup,
            Self::ScoreReached => ChatSystemKind::ScoreReached,
            Self::TopicCreated => ChatSystemKind::TopicCreated,
            Self::ContactJoined => ChatSystemKind::ContactJoined,
            Self::PremiumGifted => ChatSystemKind::PremiumGifted,
            Self::GiftSent => ChatSystemKind::GiftSent,
            Self::Generic => ChatSystemKind::Generic,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageSystemWallpaperPreview {
    pub image_url: String,
    pub look: ChatWallpaperLook,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MessageSystemGiftItem {
    pub instance_id: String,
    pub name: String,
    pub poster_url: String,
    pub animation_url: String,
    pub recipient_id: String,
    pub caption: String,
    pub anonymous: bool,
    pub serial: i64,
    pub supply_total: i64,
    pub rarity: String,
    pub backdrop: HashMap<String, String>,
    pub item_kind: String,
    pub gift_id: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MessageSystemInfo {
    pub kind: MessageSystemKind,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub target_id: Option<String>,
    pub target_name: Option<String>,
    pub text: Option<String>,
    pub wallpaper_preview: Option<MessageSystemWallpaperPreview>,
    pub gift_months: Option<i64>,
    pub gift_recipient_id: Option<String>,
    pub gift_item: Option<MessageSystemGiftItem>,
}

const CALL_MEDIA_MODE_LEGACY_VOICE: &str = "voice";
const CALL_MEDIA_MODE_LEGACY_AUDIO: &str = "call_media_mode_audio";
const CALL_MEDIA_MODE_LEGACY_VIDEO: &str = "call_media_mode_video";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MessageCallMediaMode {
    Audio,
    Video,
    Unknown(String),
}

impl MessageCallMediaMode {
    pub fn from_raw_value(raw_value: &str) -> Self {
        let value = trimming_whitespaces_and_newlines(raw_value).to_lowercase();
        let value = value.as_str();
        if value == RiseRTCCallMediaMode::Audio.wire()
            || value == CALL_MEDIA_MODE_LEGACY_VOICE
            || value == CALL_MEDIA_MODE_LEGACY_AUDIO
        {
            Self::Audio
        } else if value == RiseRTCCallMediaMode::Video.wire() || value == CALL_MEDIA_MODE_LEGACY_VIDEO {
            Self::Video
        } else {
            Self::Unknown(raw_value.to_owned())
        }
    }

    pub fn raw_value(&self) -> &str {
        match self {
            Self::Audio => RiseRTCCallMediaMode::Audio.wire(),
            Self::Video => RiseRTCCallMediaMode::Video.wire(),
            Self::Unknown(value) => value,
        }
    }

    pub fn is_video(&self) -> bool {
        *self == Self::Video
    }
}

const CALL_STATE_LEGACY_CANCELED: &str = "canceled";
const CALL_STATE_LEGACY_CANCELLED: &str = "call_state_cancelled";
const CALL_STATE_LEGACY_DECLINED: &str = "call_state_declined";
const CALL_STATE_LEGACY_TIMEOUT: &str = "timeout";
const CALL_STATE_LEGACY_TIMED_OUT: &str = "call_state_timed_out";
const CALL_STATE_LEGACY_ENDED: &str = "call_state_ended";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MessageCallTerminalState {
    Cancelled,
    Declined,
    TimedOut,
    Ended,
    Unknown(String),
}

impl MessageCallTerminalState {
    pub fn from_raw_value(raw_value: &str) -> Self {
        let value = trimming_whitespaces_and_newlines(raw_value).to_lowercase();
        let value = value.as_str();
        if value == RiseRTCCallHistoryState::Cancelled.wire()
            || value == CALL_STATE_LEGACY_CANCELED
            || value == CALL_STATE_LEGACY_CANCELLED
        {
            Self::Cancelled
        } else if value == RiseRTCCallHistoryState::Declined.wire() || value == CALL_STATE_LEGACY_DECLINED {
            Self::Declined
        } else if value == RiseRTCCallHistoryState::TimedOut.wire()
            || value == CALL_STATE_LEGACY_TIMEOUT
            || value == CALL_STATE_LEGACY_TIMED_OUT
        {
            Self::TimedOut
        } else if value == RiseRTCCallHistoryState::Ended.wire() || value == CALL_STATE_LEGACY_ENDED {
            Self::Ended
        } else {
            Self::Unknown(raw_value.to_owned())
        }
    }

    pub fn raw_value(&self) -> &str {
        match self {
            Self::Cancelled => RiseRTCCallHistoryState::Cancelled.wire(),
            Self::Declined => RiseRTCCallHistoryState::Declined.wire(),
            Self::TimedOut => RiseRTCCallHistoryState::TimedOut.wire(),
            Self::Ended => RiseRTCCallHistoryState::Ended.wire(),
            Self::Unknown(value) => value,
        }
    }

    pub fn is_terminal(&self) -> bool {
        match self {
            Self::Cancelled | Self::Declined | Self::TimedOut | Self::Ended => true,
            Self::Unknown(_) => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageCallInfo {
    pub schema_version: i64,
    pub call_id: String,
    pub media_mode: MessageCallMediaMode,
    pub terminal_state: MessageCallTerminalState,
    pub caller_user_id: String,
    pub callee_user_id: String,
    pub ended_by_user_id: Option<String>,
    pub created_at_ms: i64,
    pub answered_at_ms: Option<i64>,
    pub ended_at_ms: i64,
    pub duration_ms: i64,
    pub end_reason: Option<String>,
}

impl MessageCallInfo {
    pub fn resolved_duration_ms(&self) -> i64 {
        if self.duration_ms > 0 {
            return self.duration_ms;
        }
        let Some(answered_at_ms) = self.answered_at_ms else {
            return 0;
        };
        (self.ended_at_ms - answered_at_ms).max(0)
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ReactionReactor {
    pub user_id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub created_at: f64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MessageReaction {
    pub emoji: String,
    pub count: i64,
    pub reacted_by_me: bool,
    pub reactors: Vec<ReactionReactor>,
}

impl MessageReaction {
    pub const CUSTOM_EMOJI_PREFIX: &'static str = rise_canon::RiseChatCustomEmoji::PREFIX;

    pub fn id(&self) -> &str {
        &self.emoji
    }

    pub fn is_custom_emoji(&self) -> bool {
        self.emoji.starts_with(Self::CUSTOM_EMOJI_PREFIX)
    }

    pub fn custom_emoji_id(&self) -> Option<String> {
        let id = self.emoji.strip_prefix(Self::CUSTOM_EMOJI_PREFIX)?;
        if id.is_empty() {
            return None;
        }
        Some(id.to_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct MessageRoleMentionEntity {
    pub offset: i64,
    pub length: i64,
    pub role_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct MessageTopicMentionEntity {
    pub offset: i64,
    pub length: i64,
    pub topic_id: String,
    pub chat_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageTextLinkEntity {
    pub offset: i64,
    pub length: i64,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct MessageNormalizedEntities {
    pub custom_emojis: Vec<MessageCustomEmojiEntity>,
    pub role_mentions: Vec<MessageRoleMentionEntity>,
    pub topic_mentions: Vec<MessageTopicMentionEntity>,
    pub text_links: Vec<MessageTextLinkEntity>,
}

static EMPTY_NORMALIZED_ENTITIES: MessageNormalizedEntities = MessageNormalizedEntities::EMPTY;

impl MessageNormalizedEntities {
    pub const EMPTY: MessageNormalizedEntities = MessageNormalizedEntities {
        custom_emojis: Vec::new(),
        role_mentions: Vec::new(),
        topic_mentions: Vec::new(),
        text_links: Vec::new(),
    };

    pub fn parse(entities_json: Option<&str>) -> Self {
        let parsed = MessageEntitiesCodec::parse(entities_json);
        Self {
            custom_emojis: parsed
                .custom_emojis
                .iter()
                .map(|entity| MessageCustomEmojiEntity {
                    offset: entity.offset,
                    length: entity.length,
                    custom_emoji_id: entity.custom_emoji_id.clone(),
                    file_url: entity.file_url.clone(),
                    asset_type: entity.asset_type.clone(),
                    width: entity.width,
                    height: entity.height,
                    pack_id: entity.pack_id.clone(),
                })
                .collect(),
            role_mentions: parsed
                .role_mentions
                .iter()
                .map(|entity| MessageRoleMentionEntity {
                    offset: entity.offset,
                    length: entity.length,
                    role_id: entity.role_id.clone(),
                })
                .collect(),
            topic_mentions: parsed
                .topic_mentions
                .iter()
                .map(|entity| MessageTopicMentionEntity {
                    offset: entity.offset,
                    length: entity.length,
                    topic_id: entity.topic_id.clone(),
                    chat_id: entity.chat_id.clone(),
                })
                .collect(),
            text_links: parsed
                .text_links
                .iter()
                .map(|entity| MessageTextLinkEntity {
                    offset: entity.offset,
                    length: entity.length,
                    url: entity.url.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum MessageDeliveryStatus {
    Sending,
    Failed,
    #[default]
    Sent,
    Delivered,
    Read,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageEncryptionInfo {
    pub is_encrypted: bool,
    pub encrypted_for: Vec<String>,
    pub sender_key_id: Option<String>,
    pub encryption_version: i64,
}

impl Default for MessageEncryptionInfo {
    fn default() -> Self {
        Self { is_encrypted: false, encrypted_for: Vec::new(), sender_key_id: None, encryption_version: 1 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct MessageCommenterPreview {
    pub logo: String,
    pub name: String,
}

const MODERATION_STATUS_DISABLED: &str = "disabled";

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GetMessageMessage {
    pub id: String,
    pub server_id: Option<String>,
    pub chat_id: String,

    pub sender_id: String,
    pub sender_name: String,
    pub sender_tag: String,
    pub sender_logo: Option<String>,
    pub sender_is_official: bool,
    pub sender_is_deleted: bool,

    pub content_type: MessageContentType,
    pub text: String,
    pub original_text: Option<String>,
    pub edited_at: Option<f64>,

    pub entities: Option<String>,
    pub normalized_entities: Option<MessageNormalizedEntities>,

    pub created_at: f64,

    pub media_items: Vec<MessageMediaItem>,
    pub media_group_id: Option<String>,

    pub reply_to: Option<MessageReplyTo>,
    pub forward_info: Option<MessageForwardInfo>,
    pub link_preview: Option<MessageLinkPreview>,

    pub contact: Option<MessageContactInfo>,
    pub location: Option<MessageLocationInfo>,
    pub poll: Option<MessagePollInfo>,
    pub call_info: Option<MessageCallInfo>,
    pub system: Option<MessageSystemInfo>,
    pub shared_post: Option<MessageSharedPost>,
    pub story_reply: Option<MessageStoryReply>,
    pub document_raw: Option<String>,
    pub moderation_status: String,

    pub encryption: Option<MessageEncryptionInfo>,

    pub reactions: Vec<MessageReaction>,
    pub is_read: bool,
    pub played_by: Vec<String>,
    pub is_temp: bool,
    pub delivery_status: MessageDeliveryStatus,
    pub can_retry_send: bool,
    pub send_failure_code: Option<String>,
    pub is_pinned: bool,
    pub is_mentioned_you: bool,
    pub is_replied_you: bool,
    pub has_unread_reactions: bool,
    pub has_protected_content: bool,
    pub view_count: Option<i64>,

    pub topic_id: Option<String>,

    pub discussion_chat_id: Option<String>,
    pub discussion_message_id: Option<String>,
    pub comments_count: Option<i64>,
    pub recent_commenters: Vec<MessageCommenterPreview>,
    pub sparks_total: i64,
    pub my_sparks: i64,

    pub author_signature: Option<String>,

    pub reply_markup: Option<MessageBotReplyMarkup>,
    pub is_bot_message: bool,
    pub via_bot: Option<String>,
    pub via_bot_tag: Option<String>,
    pub web_app_data: Option<MessageBotWebAppData>,
}

impl GetMessageMessage {
    pub fn new(id: String) -> Self {
        Self { id, ..Self::default() }
    }

    pub fn is_disabled_by_moderation(&self) -> bool {
        self.moderation_status == MODERATION_STATUS_DISABLED
    }

    pub fn has_reaction_row(&self) -> bool {
        !self.reactions.is_empty() || self.sparks_total > 0
    }

    pub fn is_system(&self) -> bool {
        self.content_type == MessageContentType::System || self.system.is_some()
    }

    /// tag shown as via @tag when a bot composed the message for its sender
    /// a bot own messages already name it so no label
    pub fn via_bot_label_tag(&self) -> Option<String> {
        let tag = trimming_whitespaces_and_newlines(self.via_bot_tag.as_deref()?);
        if tag.is_empty() {
            return None;
        }
        let via_bot = trimming_whitespaces_and_newlines(self.via_bot.as_deref()?);
        if via_bot.is_empty() || via_bot == self.sender_id {
            return None;
        }
        let bare = tag.strip_prefix('@').unwrap_or(tag);
        (!bare.is_empty()).then(|| bare.to_owned())
    }

    pub fn is_pending_outgoing(&self) -> bool {
        self.delivery_status == MessageDeliveryStatus::Sending || self.is_temp
    }

    pub fn sai_server_id(&self) -> String {
        let resolved_server_id = self.server_id.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if !resolved_server_id.is_empty() {
            return resolved_server_id.to_owned();
        }
        trimming_whitespaces_and_newlines(&self.id).to_owned()
    }

    pub fn matches_message_id(&self, other: &str) -> bool {
        self.id == other || self.server_id.as_deref() == Some(other)
    }

    pub fn has_media(&self) -> bool {
        !self.media_items.is_empty()
    }

    pub fn has_text(&self) -> bool {
        !trimming_whitespaces_and_newlines(&self.text).is_empty()
    }

    pub fn grid_media_items(&self) -> Vec<&MessageMediaItem> {
        self.media_items.iter().filter(|item| item.is_grid_media()).collect()
    }

    /// view once or timed media is protected like a no forward chat
    /// no forward, share save or copy and the server refuses edits of it
    pub fn has_view_limited_media(&self) -> bool {
        self.media_items.iter().any(|item| item.view_limit.is_some())
    }

    /// protected content as the client enforces it, the server flag or view limited media
    pub fn protects_content_locally(&self) -> bool {
        self.has_protected_content || self.has_view_limited_media()
    }

    pub fn first_grid_media_item(&self) -> Option<&MessageMediaItem> {
        self.media_items.iter().find(|item| item.is_grid_media())
    }

    pub fn first_voice_item(&self) -> Option<&MessageMediaItem> {
        if self.content_type == MessageContentType::Voice {
            return self
                .media_items
                .iter()
                .find(|item| item.kind == MessageMediaKind::Voice || item.kind == MessageMediaKind::Audio);
        }
        self.media_items.iter().find(|item| item.kind == MessageMediaKind::Voice)
    }

    pub fn first_audio_item(&self) -> Option<&MessageMediaItem> {
        if self.content_type == MessageContentType::Voice {
            return None;
        }
        if self.content_type == MessageContentType::Audio {
            return self
                .media_items
                .iter()
                .find(|item| item.kind == MessageMediaKind::Audio || item.kind == MessageMediaKind::Voice);
        }
        self.media_items.iter().find(|item| item.kind == MessageMediaKind::Audio)
    }

    pub fn first_importable_mp3_item(&self) -> Option<&MessageMediaItem> {
        if self.content_type == MessageContentType::Voice {
            return None;
        }
        let owner_allows_legacy_kind =
            self.content_type == MessageContentType::Audio || self.content_type == MessageContentType::Document;
        self.media_items.iter().find(|item| {
            if !item.has_mp3_signal() {
                return false;
            }
            owner_allows_legacy_kind || item.kind == MessageMediaKind::Audio || item.kind == MessageMediaKind::Document
        })
    }

    pub fn first_document_item(&self) -> Option<&MessageMediaItem> {
        if self.content_type == MessageContentType::Document {
            return self.media_items.first();
        }
        self.media_items.iter().find(|item| item.kind == MessageMediaKind::Document)
    }

    pub fn document_items(&self) -> Vec<&MessageMediaItem> {
        if self.content_type == MessageContentType::Document {
            return self.media_items.iter().collect();
        }
        self.media_items.iter().filter(|item| item.kind == MessageMediaKind::Document).collect()
    }

    pub fn first_sticker_item(&self) -> Option<&MessageMediaItem> {
        self.media_items.iter().find(|item| item.kind == MessageMediaKind::Sticker)
    }

    pub fn lottie_sticker_prefetch_url(&self) -> Option<String> {
        let item = self.first_sticker_item()?;
        if item.sticker_kind != Some(MessageStickerKind::Lottie) {
            return None;
        }
        let url = trimming_whitespaces_and_newlines(&item.url);
        if url.is_empty() {
            return None;
        }
        Some(url.to_owned())
    }

    pub fn resolved_entities(&self) -> &MessageNormalizedEntities {
        self.normalized_entities.as_ref().unwrap_or(&EMPTY_NORMALIZED_ENTITIES)
    }

    pub fn custom_emoji_entities(&self) -> &[MessageCustomEmojiEntity] {
        &self.resolved_entities().custom_emojis
    }

    pub fn has_custom_emoji_entities(&self) -> bool {
        !self.resolved_entities().custom_emojis.is_empty()
    }
}

const MILLISECOND_TIMESTAMP_THRESHOLD: f64 = 1_000_000_000_000.0;

#[derive(Clone, Debug, PartialEq)]
pub struct MessageThreadOrderKey {
    pub timestamp: f64,
    pub id: String,
}

pub enum MessageThreadOrder {}

impl MessageThreadOrder {
    pub fn chronological(messages: Vec<GetMessageMessage>) -> Vec<GetMessageMessage> {
        if messages.len() <= 1 {
            return messages;
        }
        let mut keyed: Vec<(MessageThreadOrderKey, GetMessageMessage)> =
            messages.into_iter().map(|message| (Self::key(&message), message)).collect();
        keyed.sort_by(|lhs, rhs| {
            if Self::precedes(&lhs.0, &rhs.0) {
                Ordering::Less
            } else if Self::precedes(&rhs.0, &lhs.0) {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        });
        keyed.into_iter().map(|(_, message)| message).collect()
    }

    pub fn precedes(lhs: &MessageThreadOrderKey, rhs: &MessageThreadOrderKey) -> bool {
        if lhs.timestamp != rhs.timestamp {
            return lhs.timestamp < rhs.timestamp;
        }
        lhs.id > rhs.id
    }

    fn key(message: &GetMessageMessage) -> MessageThreadOrderKey {
        let server_id = message
            .server_id
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|server_id| !server_id.is_empty());
        MessageThreadOrderKey {
            timestamp: message.created_at,
            id: server_id.map(str::to_owned).unwrap_or_else(|| message.id.clone()),
        }
    }

    pub fn timestamp_seconds(row: &Map<String, Value>) -> Option<f64> {
        for key in [message_key::CREATED_AT, message_key::TIMESTAMP] {
            if let Some(raw) = Self::numeric_timestamp(row.get(key)) {
                return Some(Self::normalize(raw));
            }
        }
        None
    }

    pub fn normalize(raw: f64) -> f64 {
        if raw > MILLISECOND_TIMESTAMP_THRESHOLD { raw / 1000.0 } else { raw }
    }

    fn numeric_timestamp(value: Option<&Value>) -> Option<f64> {
        match value? {
            Value::Number(number) => number.as_f64().filter(|value| *value > 0.0),
            Value::String(text) => {
                let trimmed = trimming_whitespaces_and_newlines(text);
                trimmed.parse::<f64>().ok().filter(|value| *value > 0.0)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageHistoryDirection {
    Older,
    Newer,
}

impl MessageHistoryDirection {
    pub const ALL_CASES: [MessageHistoryDirection; 2] = [Self::Older, Self::Newer];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Older => ChatHistoryDirection::Older.wire(),
            Self::Newer => ChatHistoryDirection::Newer.wire(),
        }
    }

    pub fn from_raw(raw_value: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|direction| direction.raw_value() == raw_value)
    }

    pub fn wire_up(self) -> bool {
        self == Self::Older
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageHistoryPageState {
    pub relative_id: Option<String>,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageHistoryLane {
    pub chat_id: String,
    pub direction: MessageHistoryDirection,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MessageHistoryLaneState {
    pub is_loading: bool,
    pub retry_after: f64,
}

const DEBUG_UPLOAD_PREVIEW_URL: &str = "https://picsum.photos/seed/upload-debug/1080/1440";
const DEBUG_UPLOAD_SENDER_ID: &str = "debug-me";

pub enum MessageUploadPreviewDebug {}

impl MessageUploadPreviewDebug {
    pub const IS_ENABLED: bool = false;
    pub const MESSAGE_ID: &'static str = "temp_debug_upload_preview";

    pub fn appending_to_thread(messages: Vec<GetMessageMessage>, chat_id: &str) -> Vec<GetMessageMessage> {
        if !cfg!(debug_assertions) || !Self::IS_ENABLED || chat_id.is_empty() {
            return messages;
        }
        if messages.iter().any(|message| message.id == Self::MESSAGE_ID) {
            return messages;
        }

        let sender_id = MessageTypesAuthHooks::message_author_user_id_for_outgoing();
        let sender_name = MessageTypesAuthHooks::profile_name().unwrap_or_else(|| tr("me_sender_fallback"));

        let mut media = MessageMediaItem::new(
            format!("{}-media", Self::MESSAGE_ID),
            MessageMediaKind::Image,
            DEBUG_UPLOAD_PREVIEW_URL.to_owned(),
        );
        media.thumbnail_url = Some(DEBUG_UPLOAD_PREVIEW_URL.to_owned());
        media.width = 1080.0;
        media.height = 1440.0;
        media.is_uploading = true;
        media.upload_progress = 47.0;
        media.uploaded_bytes = 840_320;
        media.total_bytes = 1_790_000;

        let anchor_ts = messages
            .last()
            .map(|message| message.created_at)
            .unwrap_or_else(|| RiseClock::now().time_interval_since_1970());
        let message = GetMessageMessage {
            id: Self::MESSAGE_ID.to_owned(),
            chat_id: chat_id.to_owned(),
            sender_id: if sender_id.is_empty() { DEBUG_UPLOAD_SENDER_ID.to_owned() } else { sender_id },
            sender_name,
            sender_logo: MessageTypesAuthHooks::profile_logo(),
            content_type: MessageContentType::Image,
            text: String::new(),
            created_at: anchor_ts + 1.0,
            media_items: vec![media],
            delivery_status: MessageDeliveryStatus::Sending,
            is_read: false,
            ..GetMessageMessage::default()
        };

        let mut messages = messages;
        messages.push(message);
        messages
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatTypingPeer {
    pub user_id: String,
    pub name: String,
    pub action: Option<String>,
    pub status_text: Option<String>,
    pub at: f64,
}

#[derive(Clone, Debug)]
pub struct MessageEditMediaSnapshot {
    pub media_items: Vec<Map<String, Value>>,
    pub content: Option<String>,
    pub edit_date: Option<Value>,
    pub original_content: Option<String>,
}
