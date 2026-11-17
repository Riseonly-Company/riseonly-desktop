use std::hash::{Hash, Hasher};

use rise_canon::RiseLimits;
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{StickerAssetType, StickerItem};

pub type MapTrackPeriod = rise_catalog::MapTrackPeriod;

pub trait MapTrackPeriodIdentifiable {
    fn id(self) -> &'static str;
}

impl MapTrackPeriodIdentifiable for MapTrackPeriod {
    fn id(self) -> &'static str {
        self.wire()
    }
}

pub mod map_track_period_codable {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::MapTrackPeriod;

    pub fn serialize<S: Serializer>(period: &MapTrackPeriod, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(period.wire())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<MapTrackPeriod, D::Error> {
        let raw = String::deserialize(deserializer)?;
        MapTrackPeriod::ALL
            .iter()
            .copied()
            .find(|period| period.wire() == raw)
            .ok_or_else(|| D::Error::unknown_variant(&raw, MapTrackPeriod::WIRES))
    }
}

pub mod map_place_emoji_asset_type_codable {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::StickerAssetType;

    pub fn serialize<S: Serializer>(asset_type: &StickerAssetType, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(asset_type.wire())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<StickerAssetType, D::Error> {
        let raw = String::deserialize(deserializer)?;
        StickerAssetType::from_wire(&raw)
            .ok_or_else(|| D::Error::custom(format!("Invalid sticker asset type: {raw}")))
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapTrackPeriodStats {
    #[serde(with = "map_track_period_codable")]
    pub period: MapTrackPeriod,
    pub since_day: i32,
    pub distance_meters: i64,
    pub duration_seconds: i64,
    pub active_days: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walk_run_distance_meters: Option<i64>,
}

impl MapTrackPeriodStats {
    pub fn new(
        period: MapTrackPeriod,
        since_day: i32,
        distance_meters: i64,
        duration_seconds: i64,
        active_days: i64,
    ) -> Self {
        Self { period, since_day, distance_meters, duration_seconds, active_days, walk_run_distance_meters: None }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapTrackBand {
    pub band: i64,
    pub index_zoom: i64,
    pub min_zoom: i64,
    pub max_zoom: i64,
    pub extent_bits: i64,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapTrackSummary {
    pub revision: i64,
    pub periods: Vec<MapTrackPeriodStats>,
    pub bands: Vec<MapTrackBand>,
    pub first_day: i32,
    pub last_day: i32,
}

impl MapTrackSummary {
    pub fn is_empty(&self) -> bool {
        self.last_day <= 0 || self.periods.iter().all(|stats| stats.active_days == 0)
    }

    pub fn stats(&self, period: MapTrackPeriod) -> Option<&MapTrackPeriodStats> {
        self.periods.iter().find(|stats| stats.period == period)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct MapTileKey {
    pub band: i64,
    pub x: i64,
    pub y: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum MapPlaceKind {
    #[serde(rename = "HOME")]
    Home,
    #[serde(rename = "WORK")]
    Work,
    #[serde(rename = "CUSTOM")]
    Custom,
}

impl MapPlaceKind {
    pub const fn raw_value(self) -> &'static str {
        match self {
            MapPlaceKind::Home => "HOME",
            MapPlaceKind::Work => "WORK",
            MapPlaceKind::Custom => "CUSTOM",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        [MapPlaceKind::Home, MapPlaceKind::Work, MapPlaceKind::Custom].into_iter().find(|kind| kind.raw_value() == raw)
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapPlace {
    pub id: String,
    pub kind: MapPlaceKind,
    pub is_inferred: bool,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub radius_meters: i64,
    pub description: String,
    pub emoji: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_emoji: Option<MapPlaceEmoji>,
    pub record_tracks: bool,
    pub track_exclusion_radius_meters: i64,
}

impl Eq for MapPlace {}

impl Hash for MapPlace {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.kind.hash(state);
        self.is_inferred.hash(state);
        self.name.hash(state);
        self.latitude.to_bits().hash(state);
        self.longitude.to_bits().hash(state);
        self.radius_meters.hash(state);
        self.description.hash(state);
        self.emoji.hash(state);
        self.custom_emoji.hash(state);
        self.record_tracks.hash(state);
        self.track_exclusion_radius_meters.hash(state);
    }
}

impl MapPlace {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        kind: MapPlaceKind,
        is_inferred: bool,
        name: String,
        latitude: f64,
        longitude: f64,
        radius_meters: i64,
    ) -> Self {
        Self {
            id,
            kind,
            is_inferred,
            name,
            latitude,
            longitude,
            radius_meters,
            description: String::new(),
            emoji: "\u{1F4CD}".to_owned(),
            custom_emoji: None,
            record_tracks: false,
            track_exclusion_radius_meters: MapPlacePolicy::default_radius(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct MapPlaceEmoji {
    pub id: String,
    #[serde(rename = "packID")]
    pub pack_id: String,
    #[serde(rename = "fileURL")]
    pub file_url: String,
    #[serde(rename = "thumbnailURL", default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    pub width: i64,
    pub height: i64,
    #[serde(rename = "assetType", with = "map_place_emoji_asset_type_codable")]
    pub asset_type: StickerAssetType,
    #[serde(rename = "mimeType", default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
}

impl MapPlaceEmoji {
    pub fn new(item: &StickerItem) -> Self {
        Self {
            id: item.id.clone(),
            pack_id: item.pack_id.clone(),
            file_url: item.file_url.clone(),
            thumbnail_url: item.thumbnail_url.clone(),
            width: item.width,
            height: item.height,
            asset_type: item.asset_type,
            mime_type: item.mime_type.clone(),
            codec: item.codec.clone(),
        }
    }

    pub fn item(&self) -> StickerItem {
        StickerItem::new(
            self.id.clone(),
            self.pack_id.clone(),
            self.file_url.clone(),
            self.thumbnail_url.clone(),
            self.width,
            self.height,
            None,
            0,
            0,
            Vec::new(),
            self.asset_type,
            self.mime_type.clone(),
            self.codec.clone(),
        )
    }
}

pub enum MapPlacePolicy {}

impl MapPlacePolicy {
    pub const QUICK_EMOJIS: [&'static str; 6] = [
        "\u{1F333}",
        "\u{1F3E0}",
        "\u{1F9D1}",
        "\u{1F4BC}",
        "\u{2615}\u{FE0F}",
        "\u{2764}\u{FE0F}",
    ];

    pub fn minimum_radius() -> i64 {
        RiseLimits::service_value("map_place_track_radius_min_m")
    }

    pub fn maximum_radius() -> i64 {
        RiseLimits::service_value("map_place_track_radius_max_m")
    }

    pub fn default_radius() -> i64 {
        RiseLimits::service_value("map_place_track_radius_default_m")
    }

    pub fn maximum_places() -> i64 {
        RiseLimits::service_value("map_max_custom_places")
    }

    pub fn name_rule() -> RiseFieldRule {
        RiseFieldRule::new("map_set_place", "/name")
    }

    pub fn description_rule() -> RiseFieldRule {
        RiseFieldRule::new("map_set_place", "/description")
    }

    pub fn radius(value: i64) -> i64 {
        Self::maximum_radius().min(Self::minimum_radius().max(value))
    }
}
