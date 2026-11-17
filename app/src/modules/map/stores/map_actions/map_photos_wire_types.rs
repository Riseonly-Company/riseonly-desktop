use rise_canon::{rise_proto, rise_request};

use super::map_photos_types::{MapMediaKind, MapPhoto, MapPhotoDraft, MapPhotoGrid};

fn is_valid_coordinate(lat: f64, lng: f64) -> bool {
    lat.is_finite() && lng.is_finite() && lat.abs() <= 90.0 && lng.abs() <= 180.0
}

pub trait CreatePhotoExt {
    fn from_draft(draft: &MapPhotoDraft) -> Self;
}

impl CreatePhotoExt for rise_request::map::CreatePhoto {
    fn from_draft(draft: &MapPhotoDraft) -> Self {
        Self {
            caption: Some(draft.caption.clone()),
            duration_ms: Some(draft.duration_ms),
            grid_id: Some(draft.grid_id.clone()),
            height: Some(draft.height),
            id: draft.id.clone(),
            lat: draft.latitude,
            lng: draft.longitude,
            media_kind: Some(draft.media_kind.raw_value().to_owned()),
            taken_at: Some(draft.taken_at),
            thumbnail_url: Some(draft.thumbnail_url.clone()),
            url: Some(draft.url.clone()),
            width: Some(draft.width),
        }
    }
}

pub trait ProtoMapPhotoGridExt {
    fn model(&self) -> Option<MapPhotoGrid>;
}

impl ProtoMapPhotoGridExt for rise_proto::map::ProtoMapPhotoGrid {
    fn model(&self) -> Option<MapPhotoGrid> {
        if self.id.is_empty() || !is_valid_coordinate(self.lat, self.lng) {
            return None;
        }
        Some(MapPhotoGrid {
            id: self.id.clone(),
            owner_id: self.owner_id.clone(),
            latitude: self.lat,
            longitude: self.lng,
            photo_count: self.photo_count.max(0),
            cover_url: self.cover_url.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            is_pending: false,
            is_draft: false,
            title: self.title.clone().unwrap_or_default(),
            description: self.description.clone().unwrap_or_default(),
            cover_photo_id: self.cover_photo_id.clone(),
            sequence_number: (self.sequence_number > 0).then_some(self.sequence_number),
        })
    }
}

pub trait ProtoMapPhotoExt {
    fn model(&self) -> Option<MapPhoto>;
}

impl ProtoMapPhotoExt for rise_proto::map::ProtoMapPhoto {
    fn model(&self) -> Option<MapPhoto> {
        if self.id.is_empty() || self.url.is_empty() || !is_valid_coordinate(self.lat, self.lng) {
            return None;
        }
        Some(MapPhoto {
            id: self.id.clone(),
            owner_id: self.owner_id.clone(),
            url: self.url.clone(),
            thumbnail_url: self.thumbnail_url.clone(),
            latitude: self.lat,
            longitude: self.lng,
            width: self.width.max(0),
            height: self.height.max(0),
            taken_at: self.taken_at,
            created_at: self.created_at,
            caption: self.caption.clone(),
            grid_id: self.grid_id.clone(),
            media_kind: MapMediaKind::from_wire(&self.media_kind),
            duration_ms: self.duration_ms.max(0),
            is_pending: false,
        })
    }
}
