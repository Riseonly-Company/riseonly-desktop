use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use gpui::Point;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_media::map_view::LatLng;
use serde::{Deserialize, Deserializer, Serialize};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapUser {
    pub id: String,
    pub tag: String,
    pub name: String,
    pub logo: String,
    pub is_premium: bool,
    pub is_official: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub premium_badge_emoji: Option<String>,
    pub is_deleted: bool,
}

impl MapUser {
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() { &self.tag } else { &self.name }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapSettings {
    pub sharing_enabled: bool,
    pub tracks_enabled: bool,
    pub battery_visible: bool,
    pub frozen_until: i64,
    pub track_cooling_days: i64,
    pub track_heat_walks: i64,
    pub home_track_exclusion_radius_m: i64,
    pub track_merge_enabled: bool,
    pub track_merge_radius_m: i64,
    pub track_merge_lanes: i64,
    pub track_precise_recording: bool,
}

impl MapSettings {
    pub fn default_cooling_days() -> i64 {
        RiseLimits::service_value("map_track_cooling_days_default")
    }

    pub fn default_heat_walks() -> i64 {
        RiseLimits::service_value("map_track_heat_walks_default")
    }

    pub fn minimum_home_track_radius() -> i64 {
        RiseLimits::service_value("map_home_track_exclusion_radius_min_m")
    }

    pub fn maximum_home_track_radius() -> i64 {
        RiseLimits::service_value("map_home_track_exclusion_radius_max_m")
    }

    pub fn default_home_track_radius() -> i64 {
        RiseLimits::service_value("map_home_track_exclusion_radius_default_m")
    }

    pub fn clamped_home_track_radius(value: i64) -> i64 {
        Self::maximum_home_track_radius().min(Self::minimum_home_track_radius().max(value))
    }

    pub fn minimum_merge_radius() -> i64 {
        RiseLimits::service_value("map_track_merge_radius_min_m")
    }

    pub fn maximum_merge_radius() -> i64 {
        RiseLimits::service_value("map_track_merge_radius_max_m")
    }

    pub fn default_merge_radius() -> i64 {
        RiseLimits::service_value("map_track_merge_radius_default_m")
    }

    pub fn minimum_merge_lanes() -> i64 {
        RiseLimits::service_value("map_track_merge_lanes_min")
    }

    pub fn maximum_merge_lanes() -> i64 {
        RiseLimits::service_value("map_track_merge_lanes_max")
    }

    pub fn default_merge_lanes() -> i64 {
        RiseLimits::service_value("map_track_merge_lanes_default")
    }

    pub fn clamped_merge_radius(value: i64) -> i64 {
        Self::maximum_merge_radius().min(Self::minimum_merge_radius().max(value))
    }

    pub fn clamped_merge_lanes(value: i64) -> i64 {
        Self::maximum_merge_lanes().min(Self::minimum_merge_lanes().max(value))
    }

    pub fn initial() -> MapSettings {
        MapSettings::with_defaults(true, true, true, 0, Self::default_cooling_days(), Self::default_heat_walks())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sharing_enabled: bool,
        tracks_enabled: bool,
        battery_visible: bool,
        frozen_until: i64,
        track_cooling_days: i64,
        track_heat_walks: i64,
        home_track_exclusion_radius_m: i64,
        track_merge_enabled: bool,
        track_merge_radius_m: i64,
        track_merge_lanes: i64,
        track_precise_recording: bool,
    ) -> Self {
        Self {
            sharing_enabled,
            tracks_enabled,
            battery_visible,
            frozen_until,
            track_cooling_days,
            track_heat_walks,
            home_track_exclusion_radius_m: Self::clamped_home_track_radius(home_track_exclusion_radius_m),
            track_merge_enabled,
            track_merge_radius_m: Self::clamped_merge_radius(track_merge_radius_m),
            track_merge_lanes: Self::clamped_merge_lanes(track_merge_lanes),
            track_precise_recording,
        }
    }

    pub fn with_defaults(
        sharing_enabled: bool,
        tracks_enabled: bool,
        battery_visible: bool,
        frozen_until: i64,
        track_cooling_days: i64,
        track_heat_walks: i64,
    ) -> Self {
        Self::new(
            sharing_enabled,
            tracks_enabled,
            battery_visible,
            frozen_until,
            track_cooling_days,
            track_heat_walks,
            Self::default_home_track_radius(),
            true,
            Self::default_merge_radius(),
            Self::default_merge_lanes(),
            false,
        )
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MapSettingsStored {
    sharing_enabled: bool,
    tracks_enabled: bool,
    battery_visible: bool,
    frozen_until: i64,
    #[serde(default)]
    track_cooling_days: Option<i64>,
    #[serde(default)]
    track_heat_walks: Option<i64>,
    #[serde(default)]
    home_track_exclusion_radius_m: Option<i64>,
    #[serde(default)]
    track_merge_enabled: Option<bool>,
    #[serde(default)]
    track_merge_radius_m: Option<i64>,
    #[serde(default)]
    track_merge_lanes: Option<i64>,
    #[serde(default)]
    track_precise_recording: Option<bool>,
}

impl<'de> Deserialize<'de> for MapSettings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored = MapSettingsStored::deserialize(deserializer)?;
        Ok(MapSettings {
            sharing_enabled: stored.sharing_enabled,
            tracks_enabled: stored.tracks_enabled,
            battery_visible: stored.battery_visible,
            frozen_until: stored.frozen_until,
            track_cooling_days: stored.track_cooling_days.unwrap_or_else(MapSettings::default_cooling_days),
            track_heat_walks: stored.track_heat_walks.unwrap_or_else(MapSettings::default_heat_walks),
            home_track_exclusion_radius_m: MapSettings::clamped_home_track_radius(
                stored.home_track_exclusion_radius_m.unwrap_or_else(MapSettings::default_home_track_radius),
            ),
            track_merge_enabled: stored.track_merge_enabled.unwrap_or(true),
            track_merge_radius_m: MapSettings::clamped_merge_radius(
                stored.track_merge_radius_m.unwrap_or_else(MapSettings::default_merge_radius),
            ),
            track_merge_lanes: MapSettings::clamped_merge_lanes(
                stored.track_merge_lanes.unwrap_or_else(MapSettings::default_merge_lanes),
            ),
            track_precise_recording: stored.track_precise_recording.unwrap_or(false),
        })
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapState {
    pub is_member: bool,
    pub joined_at: i64,
    pub settings: MapSettings,
    pub friends_count: i64,
    pub incoming_requests_count: i64,
}

impl MapState {
    pub fn outsider() -> MapState {
        MapState {
            is_member: false,
            joined_at: 0,
            settings: MapSettings::initial(),
            friends_count: 0,
            incoming_requests_count: 0,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct MapSettingsPatch {
    pub sharing_enabled: Option<bool>,
    pub tracks_enabled: Option<bool>,
    pub battery_visible: Option<bool>,
    pub frozen_until: Option<i64>,
    pub track_cooling_days: Option<i64>,
    pub track_heat_walks: Option<i64>,
    pub home_track_exclusion_radius_m: Option<i64>,
    pub track_merge_enabled: Option<bool>,
    pub track_merge_radius_m: Option<i64>,
    pub track_merge_lanes: Option<i64>,
    pub track_precise_recording: Option<bool>,
}

impl MapSettingsPatch {
    pub fn redraws_lines(&self) -> bool {
        self.track_merge_enabled.is_some() || self.track_merge_radius_m.is_some() || self.track_merge_lanes.is_some()
    }

    pub fn applied(&self, settings: &MapSettings) -> MapSettings {
        MapSettings::new(
            self.sharing_enabled.unwrap_or(settings.sharing_enabled),
            self.tracks_enabled.unwrap_or(settings.tracks_enabled),
            self.battery_visible.unwrap_or(settings.battery_visible),
            self.frozen_until.unwrap_or(settings.frozen_until),
            self.track_cooling_days.unwrap_or(settings.track_cooling_days),
            self.track_heat_walks.unwrap_or(settings.track_heat_walks),
            self.home_track_exclusion_radius_m.unwrap_or(settings.home_track_exclusion_radius_m),
            self.track_merge_enabled.unwrap_or(settings.track_merge_enabled),
            self.track_merge_radius_m.unwrap_or(settings.track_merge_radius_m),
            self.track_merge_lanes.unwrap_or(settings.track_merge_lanes),
            self.track_precise_recording.unwrap_or(settings.track_precise_recording),
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MapMergePreviewSource {
    Sample,
    Mine,
}

impl MapMergePreviewSource {
    pub const ALL: [MapMergePreviewSource; 2] = [MapMergePreviewSource::Sample, MapMergePreviewSource::Mine];

    pub const fn raw_value(self) -> &'static str {
        match self {
            MapMergePreviewSource::Sample => "SAMPLE",
            MapMergePreviewSource::Mine => "MINE",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.raw_value() == raw)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct MapMergePreviewRequest {
    pub source: MapMergePreviewSource,
    pub radius_m: i64,
    pub lanes: i64,
    pub centre: Option<MapCoordinate>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapCoordinate {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Clone, PartialEq, Debug)]
pub struct MapMergePreviewStroke {
    pub points: Vec<Point<f64>>,
    pub line_id: Option<u32>,
    pub passes: i64,
}

#[derive(Clone, PartialEq, Debug)]
pub struct MapMergePreview {
    pub walks: Vec<MapMergePreviewStroke>,
    pub lines: Vec<MapMergePreviewStroke>,
    pub radius_m: i64,
    pub lanes: i64,
    pub window_meters: f64,
    pub truncated: bool,
    pub walk_count: i64,
    pub line_count: i64,
}

#[derive(Clone, PartialEq, Debug)]
pub enum MapMergePreviewState {
    Idle,
    Loading(Option<MapMergePreview>),
    Loaded(MapMergePreview),
    Failed(Option<String>),
}

impl MapMergePreviewState {
    pub fn preview(&self) -> Option<&MapMergePreview> {
        match self {
            MapMergePreviewState::Loading(preview) => preview.as_ref(),
            MapMergePreviewState::Loaded(preview) => Some(preview),
            MapMergePreviewState::Idle | MapMergePreviewState::Failed(_) => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, MapMergePreviewState::Loading(_))
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapFriend {
    pub user: MapUser,
    pub since: i64,
}

impl MapFriend {
    pub fn id(&self) -> &str {
        &self.user.id
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum MapRequestStatus {
    #[serde(rename = "PENDING")]
    Pending,
    #[serde(rename = "ACCEPTED")]
    Accepted,
    #[serde(rename = "DECLINED")]
    Declined,
    #[serde(rename = "CANCELLED")]
    Cancelled,
    #[serde(rename = "")]
    Unknown,
}

impl MapRequestStatus {
    pub const ALL: [MapRequestStatus; 5] = [
        MapRequestStatus::Pending,
        MapRequestStatus::Accepted,
        MapRequestStatus::Declined,
        MapRequestStatus::Cancelled,
        MapRequestStatus::Unknown,
    ];

    pub const fn raw_value(self) -> &'static str {
        match self {
            MapRequestStatus::Pending => "PENDING",
            MapRequestStatus::Accepted => "ACCEPTED",
            MapRequestStatus::Declined => "DECLINED",
            MapRequestStatus::Cancelled => "CANCELLED",
            MapRequestStatus::Unknown => "",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.raw_value() == raw)
    }

    pub fn from_wire(wire: &str) -> Self {
        Self::from_raw_value(wire).unwrap_or(MapRequestStatus::Unknown)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MapRequestDirection {
    Incoming,
    Outgoing,
}

impl MapRequestDirection {
    pub const ALL: [MapRequestDirection; 2] = [MapRequestDirection::Incoming, MapRequestDirection::Outgoing];

    pub const fn raw_value(self) -> &'static str {
        match self {
            MapRequestDirection::Incoming => "incoming",
            MapRequestDirection::Outgoing => "outgoing",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapFriendRequest {
    pub id: String,
    pub from_user: MapUser,
    pub to_user: MapUser,
    pub status: MapRequestStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum MapMotion {
    #[serde(rename = "UNKNOWN")]
    Unknown,
    #[serde(rename = "STILL")]
    Still,
    #[serde(rename = "WALK")]
    Walk,
    #[serde(rename = "RUN")]
    Run,
    #[serde(rename = "CYCLE")]
    Cycle,
    #[serde(rename = "DRIVE")]
    Drive,
    #[serde(rename = "FLY")]
    Fly,
}

impl MapMotion {
    pub const ALL: [MapMotion; 7] = [
        MapMotion::Unknown,
        MapMotion::Still,
        MapMotion::Walk,
        MapMotion::Run,
        MapMotion::Cycle,
        MapMotion::Drive,
        MapMotion::Fly,
    ];

    pub const fn raw_value(self) -> &'static str {
        match self {
            MapMotion::Unknown => "UNKNOWN",
            MapMotion::Still => "STILL",
            MapMotion::Walk => "WALK",
            MapMotion::Run => "RUN",
            MapMotion::Cycle => "CYCLE",
            MapMotion::Drive => "DRIVE",
            MapMotion::Fly => "FLY",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|motion| motion.raw_value() == raw)
    }

    pub fn from_wire(wire: &str) -> Self {
        Self::from_raw_value(wire).unwrap_or(MapMotion::Unknown)
    }

    pub fn is_moving(self) -> bool {
        match self {
            MapMotion::Walk | MapMotion::Run | MapMotion::Cycle | MapMotion::Drive | MapMotion::Fly => true,
            MapMotion::Unknown | MapMotion::Still => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum MapVenueKind {
    #[serde(rename = "FOOD")]
    Food,
    #[serde(rename = "CAFE")]
    Cafe,
    #[serde(rename = "BAR")]
    Bar,
    #[serde(rename = "SPORT")]
    Sport,
    #[serde(rename = "SHOP")]
    Shop,
    #[serde(rename = "STUDY")]
    Study,
    #[serde(rename = "HEALTH")]
    Health,
    #[serde(rename = "PARK")]
    Park,
    #[serde(rename = "FUN")]
    Fun,
    #[serde(rename = "STAY")]
    Stay,
    #[serde(rename = "TRANSIT")]
    Transit,
}

static MAP_VENUE_KIND_BY_POI_VALUE: LazyLock<HashMap<&'static str, MapVenueKind>> = LazyLock::new(|| {
    let table: [(MapVenueKind, &[&'static str]); 11] = [
        (MapVenueKind::Food, &["restaurant", "fast_food", "food_court", "ice_cream", "bakery"]),
        (MapVenueKind::Cafe, &["cafe"]),
        (MapVenueKind::Bar, &["bar", "pub", "beer", "biergarten", "nightclub"]),
        (
            MapVenueKind::Sport,
            &[
                "fitness_centre",
                "sports_centre",
                "sports_hall",
                "stadium",
                "swimming",
                "swimming_pool",
                "pitch",
                "golf",
                "golf_course",
                "ice_rink",
                "track",
                "fitness_station",
            ],
        ),
        (
            MapVenueKind::Shop,
            &[
                "shop",
                "grocery",
                "supermarket",
                "convenience",
                "mall",
                "department_store",
                "clothing_store",
                "clothes",
                "marketplace",
                "alcohol_shop",
            ],
        ),
        (MapVenueKind::Study, &["school", "college", "university", "library", "kindergarten"]),
        (MapVenueKind::Health, &["hospital", "clinic", "doctors", "dentist", "pharmacy"]),
        (MapVenueKind::Park, &["park", "garden", "dog_park", "playground"]),
        (
            MapVenueKind::Fun,
            &[
                "cinema",
                "theatre",
                "museum",
                "art_gallery",
                "attraction",
                "zoo",
                "aquarium",
                "music",
                "castle",
                "theme_park",
                "water_park",
                "bowling_alley",
            ],
        ),
        (MapVenueKind::Stay, &["lodging", "hotel", "hostel", "motel", "guest_house"]),
        (
            MapVenueKind::Transit,
            &["railway", "station", "bus", "bus_station", "aerialway", "ferry_terminal", "harbor"],
        ),
    ];
    let mut flat: HashMap<&'static str, MapVenueKind> = HashMap::new();
    for (kind, names) in table {
        for name in names {
            flat.insert(*name, kind);
        }
    }
    flat
});

impl MapVenueKind {
    pub const ALL: [MapVenueKind; 11] = [
        MapVenueKind::Food,
        MapVenueKind::Cafe,
        MapVenueKind::Bar,
        MapVenueKind::Sport,
        MapVenueKind::Shop,
        MapVenueKind::Study,
        MapVenueKind::Health,
        MapVenueKind::Park,
        MapVenueKind::Fun,
        MapVenueKind::Stay,
        MapVenueKind::Transit,
    ];

    pub const fn raw_value(self) -> &'static str {
        match self {
            MapVenueKind::Food => "FOOD",
            MapVenueKind::Cafe => "CAFE",
            MapVenueKind::Bar => "BAR",
            MapVenueKind::Sport => "SPORT",
            MapVenueKind::Shop => "SHOP",
            MapVenueKind::Study => "STUDY",
            MapVenueKind::Health => "HEALTH",
            MapVenueKind::Park => "PARK",
            MapVenueKind::Fun => "FUN",
            MapVenueKind::Stay => "STAY",
            MapVenueKind::Transit => "TRANSIT",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.raw_value() == raw)
    }

    pub const fn emoji(self) -> &'static str {
        match self {
            MapVenueKind::Food => "\u{1F37D}\u{FE0F}",
            MapVenueKind::Cafe => "\u{2615}\u{FE0F}",
            MapVenueKind::Bar => "\u{1F37B}",
            MapVenueKind::Sport => "\u{1F3CB}\u{FE0F}",
            MapVenueKind::Shop => "\u{1F6CD}\u{FE0F}",
            MapVenueKind::Study => "\u{1F393}",
            MapVenueKind::Health => "\u{1F3E5}",
            MapVenueKind::Park => "\u{1F333}",
            MapVenueKind::Fun => "\u{1F3AC}",
            MapVenueKind::Stay => "\u{1F3E8}",
            MapVenueKind::Transit => "\u{1F689}",
        }
    }

    pub fn from_poi(poi_class: &str, subclass: &str) -> Option<Self> {
        MAP_VENUE_KIND_BY_POI_VALUE
            .get(subclass)
            .or_else(|| MAP_VENUE_KIND_BY_POI_VALUE.get(poi_class))
            .copied()
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapFix {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy_meters: f64,
    pub speed_meters_per_second: f64,
    pub heading_degrees: f64,
    pub battery_level: i64,
    pub is_charging: bool,
    pub motion: MapMotion,
    pub recorded_at: i64,
    pub stay_since: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place_kind: Option<MapVenueKind>,
}

impl Eq for MapFix {}

impl Hash for MapFix {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.latitude.to_bits().hash(state);
        self.longitude.to_bits().hash(state);
        self.accuracy_meters.to_bits().hash(state);
        self.speed_meters_per_second.to_bits().hash(state);
        self.heading_degrees.to_bits().hash(state);
        self.battery_level.hash(state);
        self.is_charging.hash(state);
        self.motion.hash(state);
        self.recorded_at.hash(state);
        self.stay_since.hash(state);
        self.place_kind.hash(state);
    }
}

impl MapFix {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        latitude: f64,
        longitude: f64,
        accuracy_meters: f64,
        speed_meters_per_second: f64,
        heading_degrees: f64,
        battery_level: i64,
        is_charging: bool,
        motion: MapMotion,
        recorded_at: i64,
        stay_since: i64,
    ) -> Self {
        Self {
            latitude,
            longitude,
            accuracy_meters,
            speed_meters_per_second,
            heading_degrees,
            battery_level,
            is_charging,
            motion,
            recorded_at,
            stay_since,
            place_kind: None,
        }
    }

    pub fn coordinate(&self) -> LatLng {
        LatLng::new(self.latitude, self.longitude)
    }

    pub fn has_battery(&self) -> bool {
        self.battery_level >= 0
    }

    pub fn has_heading(&self) -> bool {
        self.heading_degrees >= 0.0
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapFriendPosition {
    #[serde(rename = "userID")]
    pub user_id: String,
    pub fix: MapFix,
    pub at_home: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_unavailable_since: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability_recorded_at: Option<i64>,
}

impl MapFriendPosition {
    pub fn new(user_id: String, fix: MapFix, at_home: bool) -> Self {
        Self { user_id, fix, at_home, location_unavailable_since: None, availability_recorded_at: None }
    }

    pub fn updated_at(&self) -> i64 {
        self.fix.recorded_at.max(self.availability_recorded_at.unwrap_or(0))
    }

    pub fn unavailable_since_ms(&self) -> Option<i64> {
        self.location_unavailable_since.filter(|since| *since > 0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MapLocationAvailability {
    pub available: bool,
    pub recorded_at: i64,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapLiveFriend {
    pub user: MapUser,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<MapFriendPosition>,
}

impl MapLiveFriend {
    pub fn id(&self) -> &str {
        &self.user.id
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapInvite {
    pub token: String,
    pub created_at: i64,
    pub uses: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum MapInviteRelation {
    #[serde(rename = "SELF")]
    Own,
    #[serde(rename = "FRIENDS")]
    Friends,
    #[serde(rename = "REQUESTED")]
    Requested,
    #[serde(rename = "INCOMING")]
    Incoming,
    #[serde(rename = "NONE")]
    None,
}

impl MapInviteRelation {
    pub const ALL: [MapInviteRelation; 5] = [
        MapInviteRelation::Own,
        MapInviteRelation::Friends,
        MapInviteRelation::Requested,
        MapInviteRelation::Incoming,
        MapInviteRelation::None,
    ];

    pub const fn raw_value(self) -> &'static str {
        match self {
            MapInviteRelation::Own => "SELF",
            MapInviteRelation::Friends => "FRIENDS",
            MapInviteRelation::Requested => "REQUESTED",
            MapInviteRelation::Incoming => "INCOMING",
            MapInviteRelation::None => "NONE",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|relation| relation.raw_value() == raw)
    }

    pub fn from_wire(wire: &str) -> Self {
        Self::from_raw_value(wire).unwrap_or(MapInviteRelation::None)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MapInvitePreview {
    pub owner: MapUser,
    pub relation: MapInviteRelation,
    pub viewer_is_member: bool,
    pub request_id: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapFriendRequestOutcome {
    pub request: MapFriendRequest,
    pub friend: Option<MapFriend>,
    pub target_is_member: bool,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MapReaction {
    pub id: Uuid,
    pub from_user: MapUser,
    pub emoji: String,
    pub sent_at: i64,
    pub count: i64,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapFocusRequest {
    pub id: Uuid,
    pub user_id: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MapInviteSheetStatePhase {
    Loading,
    Preview(MapInvitePreview),
    Failed(Option<String>),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapInviteSheetState {
    pub token: String,
    pub phase: MapInviteSheetStatePhase,
    pub is_submitting: bool,
    pub submission_error: Option<String>,
}

impl MapInviteSheetState {
    pub fn new(token: String, phase: MapInviteSheetStatePhase) -> Self {
        Self { token, phase, is_submitting: false, submission_error: None }
    }
}
