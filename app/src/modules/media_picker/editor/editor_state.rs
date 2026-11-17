use gpui::{Bounds, Hsla, Point, Size};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use serde::{Deserialize, Serialize};

use crate::modules::media_picker::editor::editor_crop_auto_fit::EditorCropAutoFitPlanner;
use crate::modules::media_picker::editor::editor_image_layout::EditorImageLayout;
use crate::modules::media_picker::editor::editor_link_entity::EditorLinkEntity;
use crate::modules::media_picker::editor::editor_sticker_entity::EditorStickerEntity;
use crate::modules::media_picker::editor::story_canvas_model::EditorStoryCanvas;
use crate::modules::media_picker::editor::editor_state_codable::rgba_color;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditorDrawStroke {
    pub tool: EditorDrawStrokeTool,
    #[serde(with = "rgba_color")]
    pub color: Hsla,
    pub width: f32,
    pub points: Vec<Point<f32>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditorDrawStrokeTool {
    Pen,
    Marker,
    Neon,
    Pencil,
    Arrow,
    Blur,
    Eraser,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorTextEntity {
    pub id: Uuid,
    pub text: String,
    pub font_size: f32,
    #[serde(with = "rgba_color")]
    pub color: Hsla,
    pub center: Point<f32>,
    pub rotation: f32,
    pub scale: f32,
    pub style_mode: EditorTextEntityStyleMode,
    pub alignment: EditorTextEntityTextAlignment,
}

/// style mode of a text entity, stored as an int on disk
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "i64", try_from = "i64")]
pub enum EditorTextEntityStyleMode {
    Background,
    TranslucentBg,
    TextOnly,
}

/// text alignment of a text entity, stored as an int on disk
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "i64", try_from = "i64")]
pub enum EditorTextEntityTextAlignment {
    Leading,
    Center,
    Trailing,
}

/// track card over the media
/// center is 0..1 of the canvas, base width a fraction of the canvas short side
/// not baked into the export, every client renders it live so the equalizer animates
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorMusicPlateEntity {
    pub id: Uuid,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub cover_url: String,
    pub is_dark: bool,
    pub center: Point<f32>,
    pub rotation: f32,
    pub scale: f32,
}

/// person tag over the frame
/// center is 0..1 of the canvas
/// not baked into export, ships with the post as a live layer so the bubble can open the profile
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorPersonTagEntity {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    pub center: Point<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_dark: Option<bool>,
}

impl EditorPersonTagEntity {
    pub fn uses_dark_style(&self) -> bool {
        self.is_dark.unwrap_or(false)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EditorImageAdjustments {
    pub enhance: f64,
    pub brightness: f64,
    pub contrast: f64,
    pub saturation: f64,
    pub temperature: f64,
    pub highlights: f64,
    pub shadows: f64,
    pub vignette: f64,
}

impl EditorImageAdjustments {
    pub fn is_identity(&self) -> bool {
        self.enhance == 0.0
            && self.brightness == 0.0
            && self.contrast == 0.0
            && self.saturation == 0.0
            && self.temperature == 0.0
            && self.highlights == 0.0
            && self.shadows == 0.0
            && self.vignette == 0.0
    }
}

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EditorCropAspect {
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "freeform")]
    Freeform,
    #[serde(rename = "square")]
    Square,
    #[serde(rename = "r3_2")]
    R3_2,
    #[serde(rename = "r4_3")]
    R4_3,
    #[serde(rename = "r5_4")]
    R5_4,
    #[serde(rename = "r3_5")]
    R3_5,
    #[serde(rename = "r4_5")]
    R4_5,
    #[serde(rename = "r5_7")]
    R5_7,
    #[serde(rename = "r9_16")]
    R9_16,
    #[serde(rename = "r16_9")]
    R16_9,
}

impl EditorCropAspect {
    /// raw string value, same strings the draft codec writes
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Freeform => "freeform",
            Self::Square => "square",
            Self::R3_2 => "r3_2",
            Self::R4_3 => "r4_3",
            Self::R5_4 => "r5_4",
            Self::R3_5 => "r3_5",
            Self::R4_5 => "r4_5",
            Self::R5_7 => "r5_7",
            Self::R9_16 => "r9_16",
            Self::R16_9 => "r16_9",
        }
    }

    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|aspect| aspect.raw_value() == raw_value)
    }

    pub const ALL_CASES: [Self; 11] = [
        Self::Original,
        Self::Freeform,
        Self::Square,
        Self::R3_2,
        Self::R4_3,
        Self::R5_4,
        Self::R3_5,
        Self::R4_5,
        Self::R5_7,
        Self::R9_16,
        Self::R16_9,
    ];

    pub fn ratio(self, original_ratio: f32) -> Option<f32> {
        match self {
            Self::Original => Some(original_ratio),
            Self::Freeform => None,
            Self::Square => Some(1.0),
            Self::R3_2 => Some(3.0 / 2.0),
            Self::R4_3 => Some(4.0 / 3.0),
            Self::R5_4 => Some(5.0 / 4.0),
            Self::R3_5 => Some(3.0 / 5.0),
            Self::R4_5 => Some(4.0 / 5.0),
            Self::R5_7 => Some(5.0 / 7.0),
            Self::R9_16 => Some(9.0 / 16.0),
            Self::R16_9 => Some(16.0 / 9.0),
        }
    }

    pub fn localized_title(self) -> String {
        match self {
            Self::Original => tr("editor_crop_original"),
            Self::Freeform => tr("editor_crop_freeform"),
            Self::Square => tr("editor_crop_square"),
            Self::R3_2 => "3:2".to_string(),
            Self::R4_3 => "4:3".to_string(),
            Self::R5_4 => "5:4".to_string(),
            Self::R3_5 => "3:5".to_string(),
            Self::R4_5 => "4:5".to_string(),
            Self::R5_7 => "5:7".to_string(),
            Self::R9_16 => "9:16".to_string(),
            Self::R16_9 => "16:9".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorOverlayCoordinateSpace {
    pub crop_rect_relative: Bounds<f32>,
    pub rotation_steps: i64,
    pub free_rotation_degrees: f64,
    pub is_mirrored: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorImageState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_asset_id: Option<String>,
    pub source_pixel_size: Size<f32>,
    pub rotation_steps: i64,
    pub free_rotation_degrees: f64,
    pub is_mirrored: bool,
    pub aspect: EditorCropAspect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop_rect_relative: Option<Bounds<f32>>,
    pub crop_baked_into_canvas: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay_coordinate_space: Option<EditorOverlayCoordinateSpace>,
    pub strokes: Vec<EditorDrawStroke>,
    pub texts: Vec<EditorTextEntity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music_plate: Option<EditorMusicPlateEntity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person_tags: Option<Vec<EditorPersonTagEntity>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stickers: Option<Vec<EditorStickerEntity>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub story_canvas: Option<EditorStoryCanvas>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<EditorLinkEntity>>,
    pub adjustments: EditorImageAdjustments,
    pub caption: String,
    pub was_edited: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_order: Option<Vec<Uuid>>,
}

impl EditorImageState {
    /// texts, stickers and links in the type order export falls back to
    fn stacked_entity_ids(&self) -> Vec<Uuid> {
        self.texts
            .iter()
            .map(|text| text.id)
            .chain(self.stickers.iter().flatten().map(|sticker| sticker.id))
            .chain(self.links.iter().flatten().map(|link| link.id))
            .collect()
    }

    /// a text, sticker or link not in previous goes above all the others
    /// after that the last touched one rises
    /// without an order yet type order put a new text under an older sticker
    pub fn raise_added_entities(&mut self, previous: &EditorImageState) {
        let known = previous.stacked_entity_ids();
        let current = self.stacked_entity_ids();
        let added: Vec<Uuid> = current.iter().copied().filter(|id| !known.contains(id)).collect();
        if added.is_empty() {
            return;
        }
        let mut order = self
            .entity_order
            .take()
            .unwrap_or_else(|| current.iter().copied().filter(|id| known.contains(id)).collect());
        order.retain(|id| !added.contains(id));
        order.extend(added);
        self.entity_order = Some(order);
    }

    /// true if edits change frame pixels
    /// person tags arent baked so they must not force an untouched photo through png export
    pub fn has_baked_edits(&self) -> bool {
        self.crop_baked_into_canvas
            || !self.strokes.is_empty()
            || !self.texts.is_empty()
            || self.stickers.as_ref().is_some_and(|stickers| !stickers.is_empty())
            || self.links.as_ref().is_some_and(|links| !links.is_empty())
            || self.story_canvas.is_some()
            || !self.adjustments.is_identity()
            || !EditorCropAutoFitPlanner::is_effectively_full(self.crop_rect_relative)
            || self.rotation_steps % 4 != 0
            || self.is_mirrored
            || self.free_rotation_degrees.abs() > EditorImageLayout::GEOMETRY_EPSILON_DEGREES
    }

    pub fn has_meaningful_edits(&self) -> bool {
        self.has_baked_edits() || self.person_tags.as_ref().is_some_and(|tags| !tags.is_empty())
    }

    /// true if the user changed something, meaningful edits minus the story canvas it opened with
    /// drives the edit badge and whether a closed editor keeps an edit, export still uses has_baked_edits
    pub fn has_user_edits(&self) -> bool {
        if !self.story_canvas.as_ref().is_some_and(EditorStoryCanvas::is_initial_layout) {
            return self.has_meaningful_edits();
        }
        let mut flat = self.clone();
        flat.story_canvas = None;
        flat.has_meaningful_edits()
    }

    pub fn synchronize_edit_flag(&mut self) {
        self.was_edited = self.has_meaningful_edits();
    }

    pub fn empty(source_asset_id: Option<String>, source_pixel_size: Size<f32>) -> EditorImageState {
        EditorImageState {
            source_asset_id,
            source_pixel_size,
            rotation_steps: 0,
            free_rotation_degrees: 0.0,
            is_mirrored: false,
            aspect: EditorCropAspect::Freeform,
            crop_rect_relative: None,
            crop_baked_into_canvas: false,
            overlay_coordinate_space: None,
            strokes: Vec::new(),
            texts: Vec::new(),
            music_plate: None,
            person_tags: None,
            stickers: None,
            story_canvas: None,
            links: None,
            adjustments: EditorImageAdjustments::default(),
            caption: String::new(),
            was_edited: false,
            entity_order: None,
        }
    }
}

/// every field decodes with a fallback
/// volumes came later and drafts persist this, a strict decoder would reject old records
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorVideoState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_asset_id: Option<String>,
    #[serde(default)]
    pub start_seconds: f64,
    #[serde(default)]
    pub end_seconds: f64,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub is_muted: bool,
    #[serde(default)]
    pub caption: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial_edit: Option<EditorImageState>,
    #[serde(default)]
    pub was_edited: bool,
    #[serde(default = "EditorVideoState::full_volume")]
    pub video_volume: f64,
    #[serde(default = "EditorVideoState::full_volume")]
    pub music_volume: f64,
    #[serde(default)]
    pub music_offset_seconds: f64,
}

impl EditorVideoState {
    fn full_volume() -> f64 {
        1.0
    }

    /// plain constructor with defaults, volumes 1 and music offset 0
    /// set the fields after to override
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source_asset_id: Option<String>,
        start_seconds: f64,
        end_seconds: f64,
        duration: f64,
        is_muted: bool,
        caption: String,
        spatial_edit: Option<EditorImageState>,
        was_edited: bool,
    ) -> Self {
        Self {
            source_asset_id,
            start_seconds,
            end_seconds,
            duration,
            is_muted,
            caption,
            spatial_edit,
            was_edited,
            video_volume: Self::full_volume(),
            music_volume: Self::full_volume(),
            music_offset_seconds: 0.0,
        }
    }

    pub fn effective_video_volume(&self) -> f64 {
        if self.is_muted { 0.0 } else { self.video_volume.clamp(0.0, 1.0) }
    }

    pub fn effective_music_volume(&self) -> f64 {
        self.music_volume.clamp(0.0, 1.0)
    }

    /// true if edits need re-encoding the clip
    /// person tags arent baked so they dont push an untouched video through the exporter
    pub fn has_baked_edits(&self) -> bool {
        self.is_muted
            || self.start_seconds > 0.01
            || (self.duration > 0.0 && self.end_seconds < self.duration - 0.01)
            || self.video_volume < 0.999
            || self.music_volume < 0.999
            || self.music_offset_seconds > 0.01
            || self.spatial_edit.as_ref().is_some_and(EditorImageState::has_baked_edits)
    }

    pub fn has_meaningful_edits(&self) -> bool {
        self.has_baked_edits()
            || self
                .spatial_edit
                .as_ref()
                .and_then(|edit| edit.person_tags.as_ref())
                .is_some_and(|tags| !tags.is_empty())
    }

    pub fn empty(source_asset_id: Option<String>, duration: f64) -> EditorVideoState {
        EditorVideoState::new(source_asset_id, 0.0, duration, duration, false, String::new(), None, false)
    }
}
