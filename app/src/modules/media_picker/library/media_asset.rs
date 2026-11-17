use std::hash::{Hash, Hasher};

use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_platform::photo_library::{LibraryAsset, LibraryAssetKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaAssetMediaType {
    Image,
    Video,
}

/// one library asset as the picker grid reads it
#[derive(Clone, Debug, PartialEq)]
pub struct MediaAsset {
    pub id: String,
    pub media_type: MediaAssetMediaType,
    pub pixel_width: i64,
    pub pixel_height: i64,
    pub duration: TimeInterval,
    pub creation_date: Option<Date>,
}

impl Eq for MediaAsset {}

impl Hash for MediaAsset {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.media_type.hash(state);
        self.pixel_width.hash(state);
        self.pixel_height.hash(state);
        self.duration.to_bits().hash(state);
        self.creation_date.hash(state);
    }
}

impl MediaAsset {
    /// video is video, anything else is an image
    pub fn from_asset(asset: &LibraryAsset) -> Self {
        Self {
            id: asset.id.clone(),
            media_type: match asset.kind {
                LibraryAssetKind::Video => MediaAssetMediaType::Video,
                LibraryAssetKind::Image => MediaAssetMediaType::Image,
            },
            pixel_width: i64::from(asset.pixel_width),
            pixel_height: i64::from(asset.pixel_height),
            duration: asset.duration_secs,
            creation_date: asset.created_at_ms.map(Date::from_ms),
        }
    }

    /// plain constructor, duration defaults to 0 and creation date to none
    pub fn new(
        id: String,
        media_type: MediaAssetMediaType,
        pixel_width: i64,
        pixel_height: i64,
        duration: TimeInterval,
        creation_date: Option<Date>,
    ) -> Self {
        Self {
            id,
            media_type,
            pixel_width,
            pixel_height,
            duration,
            creation_date,
        }
    }
}
