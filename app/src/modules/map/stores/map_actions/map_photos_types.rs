use std::hash::{Hash, Hasher};

use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_media::map_view::LatLng;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum MapMediaKind {
    #[serde(rename = "IMAGE")]
    Image,
    #[serde(rename = "VIDEO")]
    Video,
}

impl MapMediaKind {
    pub const fn raw_value(self) -> &'static str {
        match self {
            MapMediaKind::Image => "IMAGE",
            MapMediaKind::Video => "VIDEO",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        [MapMediaKind::Image, MapMediaKind::Video].into_iter().find(|kind| kind.raw_value() == raw)
    }

    pub fn from_wire(wire: &str) -> Self {
        Self::from_raw_value(wire).unwrap_or(MapMediaKind::Image)
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapPhotoGrid {
    pub id: String,
    #[serde(rename = "ownerID")]
    pub owner_id: String,
    pub latitude: f64,
    pub longitude: f64,
    pub photo_count: i64,
    #[serde(rename = "coverURL")]
    pub cover_url: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub is_pending: bool,
    pub is_draft: bool,
    pub title: String,
    pub description: String,
    #[serde(rename = "coverPhotoID", default, skip_serializing_if = "Option::is_none")]
    pub cover_photo_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<i64>,
}

impl Eq for MapPhotoGrid {}

impl Hash for MapPhotoGrid {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.owner_id.hash(state);
        self.latitude.to_bits().hash(state);
        self.longitude.to_bits().hash(state);
        self.photo_count.hash(state);
        self.cover_url.hash(state);
        self.created_at.hash(state);
        self.updated_at.hash(state);
        self.is_pending.hash(state);
        self.is_draft.hash(state);
        self.title.hash(state);
        self.description.hash(state);
        self.cover_photo_id.hash(state);
        self.sequence_number.hash(state);
    }
}

impl MapPhotoGrid {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        owner_id: String,
        latitude: f64,
        longitude: f64,
        photo_count: i64,
        cover_url: String,
        created_at: i64,
        updated_at: i64,
    ) -> Self {
        Self {
            id,
            owner_id,
            latitude,
            longitude,
            photo_count,
            cover_url,
            created_at,
            updated_at,
            is_pending: false,
            is_draft: false,
            title: String::new(),
            description: String::new(),
            cover_photo_id: None,
            sequence_number: None,
        }
    }

    pub fn display_title(&self) -> String {
        if !self.title.is_empty() {
            return self.title.clone();
        }
        match self.sequence_number {
            Some(sequence_number) if sequence_number > 0 => {
                tr_args("map_grid_numbered_title", &[FormatArg::Int(sequence_number)])
            }
            _ => tr("map_grid_default_title"),
        }
    }

    pub fn coordinate(&self) -> LatLng {
        LatLng::new(self.latitude, self.longitude)
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapPhoto {
    pub id: String,
    #[serde(rename = "ownerID")]
    pub owner_id: String,
    pub url: String,
    #[serde(rename = "thumbnailURL")]
    pub thumbnail_url: String,
    pub latitude: f64,
    pub longitude: f64,
    pub width: i64,
    pub height: i64,
    pub taken_at: i64,
    pub created_at: i64,
    pub caption: String,
    #[serde(rename = "gridID")]
    pub grid_id: String,
    pub media_kind: MapMediaKind,
    pub duration_ms: i64,
    pub is_pending: bool,
}

impl Eq for MapPhoto {}

impl Hash for MapPhoto {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.owner_id.hash(state);
        self.url.hash(state);
        self.thumbnail_url.hash(state);
        self.latitude.to_bits().hash(state);
        self.longitude.to_bits().hash(state);
        self.width.hash(state);
        self.height.hash(state);
        self.taken_at.hash(state);
        self.created_at.hash(state);
        self.caption.hash(state);
        self.grid_id.hash(state);
        self.media_kind.hash(state);
        self.duration_ms.hash(state);
        self.is_pending.hash(state);
    }
}

impl MapPhoto {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        owner_id: String,
        url: String,
        thumbnail_url: String,
        latitude: f64,
        longitude: f64,
        width: i64,
        height: i64,
        taken_at: i64,
        created_at: i64,
        caption: String,
    ) -> Self {
        Self {
            id,
            owner_id,
            url,
            thumbnail_url,
            latitude,
            longitude,
            width,
            height,
            taken_at,
            created_at,
            caption,
            grid_id: String::new(),
            media_kind: MapMediaKind::Image,
            duration_ms: 0,
            is_pending: false,
        }
    }

    pub fn display_url(&self) -> &str {
        if self.thumbnail_url.is_empty() { &self.url } else { &self.thumbnail_url }
    }

    pub fn coordinate(&self) -> LatLng {
        LatLng::new(self.latitude, self.longitude)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct MapPhotoDraft {
    pub id: String,
    pub url: String,
    pub thumbnail_url: String,
    pub latitude: f64,
    pub longitude: f64,
    pub width: i64,
    pub height: i64,
    pub taken_at: i64,
    pub caption: String,
    pub grid_id: String,
    pub media_kind: MapMediaKind,
    pub duration_ms: i64,
}

impl MapPhotoDraft {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        url: String,
        latitude: f64,
        longitude: f64,
        width: i64,
        height: i64,
        taken_at: i64,
        caption: String,
    ) -> Self {
        Self {
            id,
            url,
            thumbnail_url: String::new(),
            latitude,
            longitude,
            width,
            height,
            taken_at,
            caption,
            grid_id: Uuid::new_v4().uuid_string().to_lowercase(),
            media_kind: MapMediaKind::Image,
            duration_ms: 0,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct MapPhotoPlacement {
    pub photo: MapPhoto,
    pub grid: MapPhotoGrid,
}

#[derive(Clone, PartialEq, Debug)]
pub struct MapPhotoRemoval {
    pub grid: Option<MapPhotoGrid>,
    pub removed_grid_id: Option<String>,
}
