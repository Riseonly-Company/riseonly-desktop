use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::LazyLock;

use gpui::{Bounds, Size, bounds, point, size};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_canon::RiseLimits;
use rise_ui::rich_text::document::rise_document::RiseDocument;

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;

use crate::modules::chat::stores::message::message_actions::message_poll_codec::MessagePollDraft;
use crate::modules::chat::stores::message::message_actions::message_types::MessageLocationInfo;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::editor::editor_crop_auto_fit::EditorCropAutoFitPlanner;
use crate::modules::media_picker::editor::editor_image_layout::EditorImageLayout;
use crate::modules::media_picker::editor::editor_state::{EditorImageState, EditorPersonTagEntity, EditorVideoState};
use crate::modules::media_picker::editor::editor_state_codable::EditorStateCodec;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaPickerItemKind {
    Image,
    Video,
    File,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaPickerEditedImage {
    pub data: Vec<u8>,
    pub was_edited: bool,
    pub pixel_size: Size<f32>,
    pub caption: String,
    pub editor_state_data: Option<Vec<u8>>,
    pub original_image_url: Option<Url>,
    pub deferred_edit_state: Option<EditorImageState>,
    pub is_processing_deferred: bool,
    pub optimistic_preview_url: Option<Url>,
    pub person_tags: Vec<EditorPersonTagEntity>,
    pub metadata_source_url: Option<Url>,
}

impl MediaPickerEditedImage {
    /// plain init with defaults for the optional fields, no editor data or deferred state or tags
    /// set them afterwards to override
    pub fn new(data: Vec<u8>, was_edited: bool, pixel_size: Size<f32>, caption: String) -> Self {
        Self {
            data,
            was_edited,
            pixel_size,
            caption,
            editor_state_data: None,
            original_image_url: None,
            deferred_edit_state: None,
            is_processing_deferred: false,
            optimistic_preview_url: None,
            person_tags: Vec::new(),
            metadata_source_url: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaPickerEditedVideo {
    pub url: Url,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub duration: f64,
    pub is_muted: bool,
    pub caption: String,
    pub pixel_size: Size<f32>,
    pub poster_url: Option<Url>,
    pub optimistic_poster_url: Option<Url>,
    pub spatial_edit: Option<EditorImageState>,
    pub is_processing_deferred: bool,
    pub audio_pre_baked: bool,
}

impl MediaPickerEditedVideo {
    /// plain init with defaults for the optional fields, zero pixel size and no posters or spatial edit
    /// not deferred and not pre baked
    /// set them afterwards to override
    pub fn new(
        url: Url,
        start_seconds: f64,
        end_seconds: f64,
        duration: f64,
        is_muted: bool,
        caption: String,
    ) -> Self {
        Self {
            url,
            start_seconds,
            end_seconds,
            duration,
            is_muted,
            caption,
            pixel_size: Size::default(),
            poster_url: None,
            optimistic_poster_url: None,
            spatial_edit: None,
            is_processing_deferred: false,
            audio_pre_baked: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaPickerAudioPayload {
    pub title: String,
    pub artist: String,
    pub duration: f64,
    pub cover_url: Option<Url>,
}

/// how the recipient may open a photo or video sent from the chat picker
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MediaPickerViewMode {
    Once,
    Timed { seconds: i64 },
}

static MEDIA_PICKER_VIEW_MODE_WIRE_MODES: LazyLock<&'static [&'static str]> =
    LazyLock::new(|| RiseFieldRule::new("message_create_message", "/media_items/*/view_mode").allowed_values());

static MEDIA_PICKER_VIEW_MODE_TIMER_PRESETS: LazyLock<Vec<i64>> = LazyLock::new(|| {
    (1..=4)
        .filter_map(|index| {
            let ms = RiseLimits::service_value(&format!("message_view_timed_preset_{index}_ms"));
            (ms > 0).then_some(ms / 1000)
        })
        .collect()
});

impl MediaPickerViewMode {
    pub fn wire_mode(&self) -> String {
        let modes = *MEDIA_PICKER_VIEW_MODE_WIRE_MODES;
        match self {
            Self::Once => modes.first().map_or_else(String::new, |mode| (*mode).to_string()),
            Self::Timed { .. } => modes.get(1).map_or_else(String::new, |mode| (*mode).to_string()),
        }
    }

    pub fn timer_presets() -> Vec<i64> {
        MEDIA_PICKER_VIEW_MODE_TIMER_PRESETS.clone()
    }

    pub fn seconds(&self) -> Option<i64> {
        match self {
            Self::Timed { seconds } => Some(*seconds),
            Self::Once => None,
        }
    }
}

/// identity is id alone for hash and equality
#[derive(Clone, Debug)]
pub struct MediaPickerItem {
    pub id: String,
    pub kind: MediaPickerItemKind,
    pub url: Url,
    pub suggested_file_name: String,
    pub asset_identifier: Option<String>,
    pub image_payload: Option<MediaPickerEditedImage>,
    pub video_payload: Option<MediaPickerEditedVideo>,
    pub audio_payload: Option<MediaPickerAudioPayload>,
    pub is_file: bool,
    pub has_spoiler: bool,
    pub view_mode: Option<MediaPickerViewMode>,
}

impl PartialEq for MediaPickerItem {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for MediaPickerItem {}

impl Hash for MediaPickerItem {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

fn standardized_intersection(rect: Bounds<f32>, other: Bounds<f32>) -> Bounds<f32> {
    let min_x = rect.origin.x.min(rect.origin.x + rect.size.width);
    let max_x = rect.origin.x.max(rect.origin.x + rect.size.width);
    let min_y = rect.origin.y.min(rect.origin.y + rect.size.height);
    let max_y = rect.origin.y.max(rect.origin.y + rect.size.height);
    let left = min_x.max(other.origin.x);
    let right = max_x.min(other.origin.x + other.size.width);
    let top = min_y.max(other.origin.y);
    let bottom = max_y.min(other.origin.y + other.size.height);
    if right <= left || bottom <= top {
        return Bounds::default();
    }
    bounds(point(left, top), size(right - left, bottom - top))
}

impl MediaPickerItem {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        kind: MediaPickerItemKind,
        url: Url,
        suggested_file_name: String,
        asset_identifier: Option<String>,
        image_payload: Option<MediaPickerEditedImage>,
        video_payload: Option<MediaPickerEditedVideo>,
        audio_payload: Option<MediaPickerAudioPayload>,
        is_file: bool,
    ) -> Self {
        Self {
            id,
            kind,
            url,
            suggested_file_name,
            asset_identifier,
            image_payload,
            video_payload,
            audio_payload,
            is_file,
            has_spoiler: false,
            view_mode: None,
        }
    }

    /// name an audio attachment travels under, the chat bubble parses artist and title back out of it
    /// extension must match the real bytes
    /// an m4a announced as mp3 is refused by the upload policy
    pub fn audio_file_name(&self) -> Option<String> {
        let audio_payload = self.audio_payload.as_ref()?;
        let suggested = trimming_whitespaces_and_newlines(&self.suggested_file_name);
        let has_extension = Path::new(suggested).extension().is_some_and(|extension| !extension.is_empty());
        if !suggested.is_empty() && has_extension {
            return Some(suggested.to_string());
        }
        let url_extension = self.url.path_extension();
        let file_extension = if url_extension.is_empty() { "mp3".to_string() } else { url_extension.to_lowercase() };
        Some(format!("{} - {}.{file_extension}", audio_payload.artist, audio_payload.title))
    }

    /// none only when the identifier doesnt form a url
    pub fn deferred_photo_library_image(
        asset_identifier: &str,
        pixel_size: Size<f32>,
        edit_state: EditorImageState,
        optimistic_preview_url: Option<Url>,
    ) -> Option<MediaPickerItem> {
        let source_url = MediaPickerPhotoAssetSource::url(asset_identifier)?;
        let payload = MediaPickerEditedImage {
            data: Vec::new(),
            was_edited: edit_state.was_edited,
            pixel_size: Self::deferred_image_pixel_size(pixel_size, &edit_state),
            caption: edit_state.caption.clone(),
            editor_state_data: EditorStateCodec::encode_image(&edit_state),
            original_image_url: None,
            person_tags: edit_state.person_tags.clone().unwrap_or_default(),
            deferred_edit_state: Some(edit_state),
            is_processing_deferred: true,
            optimistic_preview_url,
            metadata_source_url: None,
        };
        Some(MediaPickerItem::new(
            format!("ph-image:{asset_identifier}"),
            MediaPickerItemKind::Image,
            source_url,
            MediaPickerPhotoAssetSource::suggested_image_file_name(asset_identifier),
            Some(asset_identifier.to_string()),
            Some(payload),
            None,
            None,
            false,
        ))
    }

    /// none only when the identifier doesnt form a url
    pub fn deferred_photo_library_video(
        asset_identifier: &str,
        pixel_size: Size<f32>,
        edit_state: EditorVideoState,
        optimistic_poster_url: Option<Url>,
    ) -> Option<MediaPickerItem> {
        let source_url = MediaPickerPhotoAssetSource::url(asset_identifier)?;
        let duration = (edit_state.end_seconds - edit_state.start_seconds).max(0.0);
        let payload = MediaPickerEditedVideo {
            url: source_url.clone(),
            start_seconds: edit_state.start_seconds,
            end_seconds: edit_state.end_seconds,
            duration,
            is_muted: edit_state.is_muted,
            caption: edit_state.caption.clone(),
            pixel_size: Self::deferred_video_pixel_size(pixel_size, edit_state.spatial_edit.as_ref()),
            poster_url: Some(source_url.clone()),
            optimistic_poster_url,
            spatial_edit: edit_state.spatial_edit,
            is_processing_deferred: true,
            audio_pre_baked: false,
        };
        Some(MediaPickerItem::new(
            format!("ph-video:{asset_identifier}"),
            MediaPickerItemKind::Video,
            source_url,
            MediaPickerPhotoAssetSource::suggested_video_file_name(asset_identifier),
            Some(asset_identifier.to_string()),
            None,
            Some(payload),
            None,
            false,
        ))
    }

    pub fn deferred_external_video(
        source_url: Url,
        suggested_file_name: String,
        pixel_size: Size<f32>,
        edit_state: EditorVideoState,
        optimistic_poster_url: Option<Url>,
    ) -> MediaPickerItem {
        let duration = (edit_state.end_seconds - edit_state.start_seconds).max(0.0);
        let payload = MediaPickerEditedVideo {
            url: source_url.clone(),
            start_seconds: edit_state.start_seconds,
            end_seconds: edit_state.end_seconds,
            duration,
            is_muted: edit_state.is_muted,
            caption: edit_state.caption.clone(),
            pixel_size: Self::deferred_video_pixel_size(pixel_size, edit_state.spatial_edit.as_ref()),
            poster_url: None,
            optimistic_poster_url,
            spatial_edit: edit_state.spatial_edit,
            is_processing_deferred: true,
            audio_pre_baked: false,
        };
        MediaPickerItem::new(
            format!("external-video:{}", Uuid::new_v4().uuid_string()),
            MediaPickerItemKind::Video,
            source_url,
            suggested_file_name,
            None,
            None,
            Some(payload),
            None,
            true,
        )
    }

    fn has_photo_asset_scheme(&self) -> bool {
        self.url.scheme().is_some_and(|scheme| scheme.to_lowercase() == MediaPickerPhotoAssetSource::SCHEME)
    }

    pub fn deferred_photo_library_image_asset_identifier(&self) -> Option<String> {
        let payload = self.image_payload.as_ref()?;
        if self.kind != MediaPickerItemKind::Image
            || !payload.is_processing_deferred
            || payload.deferred_edit_state.is_none()
            || !self.has_photo_asset_scheme()
        {
            return None;
        }
        self.asset_identifier.clone().filter(|identifier| !identifier.is_empty())
    }

    pub fn deferred_photo_library_video_asset_identifier(&self) -> Option<String> {
        let payload = self.video_payload.as_ref()?;
        if self.kind != MediaPickerItemKind::Video || !payload.is_processing_deferred || !self.has_photo_asset_scheme() {
            return None;
        }
        self.asset_identifier.clone().filter(|identifier| !identifier.is_empty())
    }

    /// optimistic row box for a deferred video, same math as the render size
    /// rotate then crop
    /// otherwise the bubble keeps the source aspect and a cropped video shows uncropped
    fn deferred_video_pixel_size(source_pixel_size: Size<f32>, spatial_edit: Option<&EditorImageState>) -> Size<f32> {
        let source = size(source_pixel_size.width.max(1.0), source_pixel_size.height.max(1.0));
        let Some(spatial_edit) = spatial_edit else {
            return source;
        };
        let oriented = EditorImageLayout::oriented_display_size(
            source,
            spatial_edit.rotation_steps,
            spatial_edit.free_rotation_degrees,
        );
        let Some(crop) = spatial_edit.crop_rect_relative else {
            return oriented;
        };
        if EditorCropAutoFitPlanner::is_effectively_full(Some(crop)) {
            return oriented;
        }
        let relative = standardized_intersection(crop, bounds(point(0.0, 0.0), size(1.0, 1.0)));
        let passes = relative.size.width > 0.0 && relative.size.height > 0.0;
        if !passes {
            return oriented;
        }
        size(
            (oriented.width * relative.size.width).floor().max(1.0),
            (oriented.height * relative.size.height).floor().max(1.0),
        )
    }

    fn deferred_image_pixel_size(source_pixel_size: Size<f32>, edit_state: &EditorImageState) -> Size<f32> {
        let width = source_pixel_size.width.max(1.0);
        let height = source_pixel_size.height.max(1.0);
        let longest_side = width.max(height);
        let source_scale = (MediaCompressionConfiguration::source_pixel_max_side() as f32 / longest_side).min(1.0);
        let request_size = size(width * source_scale, height * source_scale);
        let oriented = EditorImageLayout::oriented_display_size(
            request_size,
            edit_state.rotation_steps,
            edit_state.free_rotation_degrees,
        );
        let crop = edit_state.crop_rect_relative.unwrap_or(bounds(point(0.0, 0.0), size(1.0, 1.0)));
        size(
            (oriented.width * crop.size.width.max(0.0)).floor().max(1.0),
            (oriented.height * crop.size.height.max(0.0)).floor().max(1.0),
        )
    }
}

pub enum MediaPickerPhotoAssetSource {}

impl MediaPickerPhotoAssetSource {
    pub const SCHEME: &'static str = "ph";

    /// photo library local ids look like UUID/L0/001
    /// the id goes into the url as is so image caches resolve the same source early
    /// none if it doesnt form a url
    pub fn url(asset_identifier: &str) -> Option<Url> {
        Url::from_string(&format!("{}://{asset_identifier}", Self::SCHEME))
    }

    pub fn suggested_video_file_name(asset_identifier: &str) -> String {
        match Self::picked_file_stem(asset_identifier) {
            Some(stem) => format!("{stem}.mp4"),
            None => format!("video-{}.mp4", Self::stable_stem(asset_identifier)),
        }
    }

    pub fn suggested_image_file_name(asset_identifier: &str) -> String {
        match Self::picked_file_stem(asset_identifier) {
            Some(stem) => format!("{stem}.jpg"),
            None => format!("image-{}.jpg", Self::stable_stem(asset_identifier)),
        }
    }

    fn picked_file_stem(asset_identifier: &str) -> Option<String> {
        let bytes = asset_identifier.as_bytes();
        let is_windows_drive = bytes.len() > 2 && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/');
        if !(asset_identifier.starts_with('/') || asset_identifier.starts_with("\\\\") || is_windows_drive) {
            return None;
        }
        let leaf = asset_identifier.rsplit(['/', '\\']).next()?;
        let stem = match leaf.rfind('.') {
            Some(dot) if dot > 0 => &leaf[..dot],
            _ => leaf,
        };
        let stem = trimming_whitespaces_and_newlines(stem);
        (!stem.is_empty()).then(|| stem.to_owned())
    }

    fn stable_stem(asset_identifier: &str) -> String {
        asset_identifier.split('/').find(|part| !part.is_empty()).unwrap_or("asset").to_string()
    }
}

/// one track per post on the wire
/// so the selection hangs off the picker session and not any single asset's edit state
#[derive(Clone, Debug, PartialEq)]
pub struct MediaPickerTrackSelection {
    pub track: RiseStoredMusicTrack,
    pub start_ms: i64,
    pub end_ms: i64,
    pub plate: Option<MediaPickerTrackPlate>,
}

/// transform and theme of the on media track card
/// normalized to the media frame, center in 0..1 and base width 0.72 of the short side at scale 1
/// so every client can render it live
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaPickerTrackPlate {
    pub is_dark: bool,
    pub center_x: f64,
    pub center_y: f64,
    pub rotation: f64,
    pub scale: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaPickerResult {
    pub items: Vec<MediaPickerItem>,
    pub was_cancelled: bool,
    pub attached_track: Option<MediaPickerTrackSelection>,
    pub document: Option<RiseDocument>,
    pub document_entities: Option<String>,
    pub location: Option<MessageLocationInfo>,
    pub poll: Option<MessagePollDraft>,
}

impl MediaPickerResult {
    pub const CANCELLED: MediaPickerResult = MediaPickerResult::new(Vec::new(), true);

    /// plain init with defaults, no track or document or location or poll
    pub const fn new(items: Vec<MediaPickerItem>, was_cancelled: bool) -> Self {
        Self {
            items,
            was_cancelled,
            attached_track: None,
            document: None,
            document_entities: None,
            location: None,
            poll: None,
        }
    }

    /// defaults are empty items and no entities
    pub fn document(document: RiseDocument, items: Vec<MediaPickerItem>, entities: Option<String>) -> MediaPickerResult {
        MediaPickerResult { document: Some(document), document_entities: entities, ..MediaPickerResult::new(items, false) }
    }

    pub fn location(location: MessageLocationInfo) -> MediaPickerResult {
        MediaPickerResult { location: Some(location), ..MediaPickerResult::new(Vec::new(), false) }
    }

    pub fn poll(poll: MessagePollDraft) -> MediaPickerResult {
        MediaPickerResult { poll: Some(poll), ..MediaPickerResult::new(Vec::new(), false) }
    }
}
