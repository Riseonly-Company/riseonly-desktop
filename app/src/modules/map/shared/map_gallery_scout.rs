use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use parking_lot::Mutex;
use rise_canon::RiseLimits;
use rise_platform::photo_library::{
    self, ChangeWatch, LibraryAssetDetails, LibraryAssetKind, LibraryMediaKind, PhotoLibraryAccessLevel,
};
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::map::stores::map_actions::map_photos_types::MapMediaKind;

/// geotagged photo or short video from the local gallery, a grid candidate
/// nothing is uploaded until the user picks it
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapGalleryDraft {
    pub id: String,
    pub latitude: f64,
    pub longitude: f64,
    pub taken_at: i64,
    pub media_kind: MapMediaKind,
    pub duration_ms: i64,
    pub width: i64,
    pub height: i64,
}

impl Eq for MapGalleryDraft {}

impl Hash for MapGalleryDraft {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.latitude.to_bits().hash(state);
        self.longitude.to_bits().hash(state);
        self.taken_at.hash(state);
        self.media_kind.hash(state);
        self.duration_ms.hash(state);
        self.width.hash(state);
        self.height.hash(state);
    }
}

impl MapGalleryDraft {
    /// what the image loader and media viewer open a gallery asset by
    pub fn display_source(&self) -> String {
        format!("ph://{}", self.id)
    }
}

static MAXIMUM_DRAFTS: LazyLock<i64> = LazyLock::new(|| RiseLimits::client_value("map_photo_drafts_max"));
static LOOKBACK_MS: LazyLock<i64> = LazyLock::new(|| RiseLimits::client_value("map_gallery_lookback_ms"));
static MAXIMUM_VIDEO_MS: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_max_photo_video_ms"));

/// finds shots taken outside the app
/// gallery stores place and time so one scan at map open catches all since the last one, even offline
/// needs full library access
pub enum MapGalleryScout {}

impl MapGalleryScout {
    pub fn maximum_drafts() -> i64 {
        *MAXIMUM_DRAFTS
    }

    pub fn lookback_ms() -> i64 {
        *LOOKBACK_MS
    }

    pub fn has_full_access() -> bool {
        matches!(photo_library::access_level(), Ok(PhotoLibraryAccessLevel::Authorized))
    }

    /// newest first
    /// runs a gallery fetch so call it off the main thread
    pub fn scan(since_ms: i64, now_ms: i64, excluding: &HashSet<String>) -> Vec<MapGalleryDraft> {
        if !Self::has_full_access() {
            return Vec::new();
        }
        let since = since_ms.max(now_ms.saturating_sub(Self::lookback_ms()));
        let maximum_drafts = usize::try_from(Self::maximum_drafts()).unwrap_or(0);
        let fetched = match photo_library::assets_created_after(
            LibraryMediaKind::All,
            since,
            maximum_drafts.saturating_mul(4),
        ) {
            Ok(fetched) => fetched,
            Err(reason) => {
                tracing::debug!(target: "riseonly", %reason, "gallery scan unavailable");
                return Vec::new();
            }
        };

        let mut drafts: Vec<MapGalleryDraft> = Vec::new();
        for details in &fetched {
            if drafts.len() >= maximum_drafts {
                break;
            }
            if excluding.contains(&details.asset.id) {
                continue;
            }
            if let Some(draft) = Self::draft(details) {
                drafts.push(draft);
            }
        }
        drafts
    }

    fn draft(from: &LibraryAssetDetails) -> Option<MapGalleryDraft> {
        let coordinate = from.place.as_ref()?;
        let is_valid = (-90.0..=90.0).contains(&coordinate.latitude) && (-180.0..=180.0).contains(&coordinate.longitude);
        if !is_valid || (coordinate.latitude == 0.0 && coordinate.longitude == 0.0) || from.is_screenshot {
            return None;
        }
        let shot_at = from.asset.created_at_ms?;

        let is_video = from.asset.kind == LibraryAssetKind::Video;
        let duration_ms = if is_video { (from.asset.duration_secs * 1_000.0).round() as i64 } else { 0 };
        if duration_ms > *MAXIMUM_VIDEO_MS {
            return None;
        }
        Some(MapGalleryDraft {
            id: from.asset.id.clone(),
            latitude: coordinate.latitude,
            longitude: coordinate.longitude,
            taken_at: shot_at,
            media_kind: if is_video { MapMediaKind::Video } else { MapMediaKind::Image },
            duration_ms,
            width: i64::from(from.asset.pixel_width),
            height: i64::from(from.asset.pixel_height),
        })
    }
}

/// the library delivers changes on its own thread
/// ownership and coalescing stay in the domain
pub struct MapGalleryObserver {
    watch: Mutex<Option<ChangeWatch>>,
}

impl MapGalleryObserver {
    pub fn new(on_change: impl Fn() + Send + Sync + 'static) -> Self {
        let watch = match photo_library::watch_changes(on_change) {
            Ok(watch) => Some(watch),
            Err(reason) => {
                tracing::debug!(target: "riseonly", %reason, "gallery changes not observed");
                None
            }
        };
        Self { watch: Mutex::new(watch) }
    }

    pub fn stop(&self) {
        self.watch.lock().take();
    }
}
