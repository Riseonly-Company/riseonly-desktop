use std::collections::HashMap;
use std::sync::Arc;

use gpui::Point;
use parking_lot::Mutex;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::Date;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::map::engine::core::map_track_heat::{MapHeatPolicy, MapHeatStretch, MapTrackHeat};
use crate::modules::map::recorder::map_track_codec::{MapTrackCodec, MapTrackCodecPass, MapTrackGeometry};
use crate::modules::map::stores::map_actions::map_tracks_types::{MapTileKey, MapTrackBand};

/// one unprojected polyline of a tile
/// bounds let a child tile take only the lines crossing it
#[derive(Clone, PartialEq, Debug)]
pub struct MapTrackLine {
    pub day: i32,
    pub mode: i32,
    pub line_id: Option<u32>,
    pub passes: Vec<MapTrackCodecPass>,
    pub cells: Vec<Point<i32>>,
    pub coordinates: Vec<f64>,
    pub min_latitude: f64,
    pub max_latitude: f64,
    pub min_longitude: f64,
    pub max_longitude: f64,
}

impl MapTrackLine {
    pub fn intersects(&self, min_lat: f64, max_lat: f64, min_lng: f64, max_lng: f64) -> bool {
        self.max_latitude >= min_lat
            && self.min_latitude <= max_lat
            && self.max_longitude >= min_lng
            && self.min_longitude <= max_lng
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct MapDecodedTile {
    pub key: MapTileKey,
    pub revision: i64,
    pub since_day: i32,
    pub extent: f64,
    pub lines: Arc<[MapTrackLine]>,
}

/// lines already cut into stretches of one temperature
/// valid for one track revision and one cooling period and one hour of the clock
#[derive(Clone)]
struct MapHeatedTile {
    revision: i64,
    since_day: i32,
    cooling_days: i64,
    heat_walks: i64,
    hour: i64,
    stretches: Arc<[MapHeatStretch]>,
}

pub struct MapTrackTileStoreLookup {
    pub tile: Option<Arc<MapDecodedTile>>,
    pub stretches: Arc<[MapHeatStretch]>,
    pub needs_fetch: bool,
}

struct MapTrackTileStoreState {
    bands: Vec<MapTrackBand>,
    tiles: HashMap<MapTileKey, Arc<MapDecodedTile>>,
    heated: HashMap<MapTileKey, MapHeatedTile>,
    cooling_days: i64,
    heat_walks: i64,
    recency: Vec<MapTileKey>,
    current_revision: i64,
    wanted_since_day: i32,
}

/// decoded tiles of one owner's tracks, readable synchronously
/// the map asks from its own background thread
/// and wants the answer in the same call which an actor cant give
pub struct MapTrackTileStore {
    state: Mutex<MapTrackTileStoreState>,
}

impl Default for MapTrackTileStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MapTrackTileStore {
    fn capacity() -> usize {
        usize::try_from(RiseLimits::client_value("map_track_tiles_memory")).unwrap_or(0)
    }

    fn maximum_parts() -> i64 {
        RiseLimits::service_value("map_max_overview_parts_per_tile")
            .max(RiseLimits::service_value("map_max_detail_parts_per_tile"))
    }

    fn maximum_vertices() -> i64 {
        RiseLimits::service_value("map_max_segment_points")
    }

    fn maximum_passes() -> i64 {
        RiseLimits::service_value("map_max_heat_passes_per_line")
    }

    fn default_cooling_days() -> i64 {
        RiseLimits::service_value("map_track_cooling_days_default")
    }

    fn default_heat_walks() -> i64 {
        RiseLimits::service_value("map_track_heat_walks_default")
    }

    pub fn new() -> Self {
        Self {
            state: Mutex::new(MapTrackTileStoreState {
                bands: Vec::new(),
                tiles: HashMap::new(),
                heated: HashMap::new(),
                cooling_days: Self::default_cooling_days(),
                heat_walks: Self::default_heat_walks(),
                recency: Vec::new(),
                current_revision: 0,
                wanted_since_day: 0,
            }),
        }
    }

    pub fn configure(&self, bands: Vec<MapTrackBand>, revision: i64, wanted_since_day: i32) {
        let mut state = self.state.lock();
        state.bands = bands;
        state.current_revision = revision;
        state.wanted_since_day = wanted_since_day;
    }

    pub fn band(&self, zoom: i64) -> Option<MapTrackBand> {
        let state = self.state.lock();
        state
            .bands
            .iter()
            .find(|band| zoom >= band.min_zoom && zoom <= band.max_zoom)
            .or_else(|| state.bands.last().filter(|band| zoom > band.max_zoom))
            .cloned()
    }

    pub fn fallback_bands(&self, requested: &MapTrackBand) -> Vec<MapTrackBand> {
        let state = self.state.lock();
        let mut bands: Vec<MapTrackBand> =
            state.bands.iter().filter(|band| band.band != requested.band).cloned().collect();
        bands.sort_by(|left, right| {
            let left_distance = (left.index_zoom - requested.index_zoom).abs();
            let right_distance = (right.index_zoom - requested.index_zoom).abs();
            left_distance.cmp(&right_distance).then(left.index_zoom.cmp(&right.index_zoom))
        });
        bands
    }

    /// the detail band is the pyramid's last
    /// only one the server keeps by day so only one a period can make too shallow
    pub fn is_detail_band(&self, band: i64) -> bool {
        self.state.lock().bands.last().is_some_and(|last| last.band == band)
    }

    /// warmth is computed outside the lock
    /// its the slow step and the repository writes tiles through the same lock
    pub fn lookup(&self, key: MapTileKey, is_detail: bool) -> MapTrackTileStoreLookup {
        let now = Date::now();
        let hour = (now.time_interval_since_1970() / 3_600.0) as i64;

        let (tile, needs_fetch, cooling, walks, held, band) = {
            let mut state = self.state.lock();
            let Some(tile) = state.tiles.get(&key).cloned() else {
                return MapTrackTileStoreLookup { tile: None, stretches: Arc::from([]), needs_fetch: true };
            };
            if let Some(index) = state.recency.iter().rposition(|held| *held == key)
                && index != state.recency.len() - 1
            {
                state.recency.remove(index);
                state.recency.push(key);
            }
            let needs_fetch = tile.revision != state.current_revision
                || (is_detail && tile.since_day > state.wanted_since_day);
            let held = state.heated.get(&key).cloned();
            let band = state.bands.iter().find(|band| band.band == key.band).cloned();
            (tile, needs_fetch, state.cooling_days, state.heat_walks, held, band)
        };

        if let Some(held) = held
            && held.revision == tile.revision
            && held.since_day == tile.since_day
            && held.cooling_days == cooling
            && held.heat_walks == walks
            && held.hour == hour
        {
            return MapTrackTileStoreLookup { tile: Some(tile), stretches: held.stretches, needs_fetch };
        }

        let offset_seconds = f64::from(TimeZone::current().seconds_from_gmt(&now));
        let Some(band) = band else {
            return MapTrackTileStoreLookup { tile: Some(tile), stretches: Arc::from([]), needs_fetch };
        };
        let stretches: Arc<[MapHeatStretch]> = MapTrackHeat::stretches(
            &tile,
            MapHeatPolicy::new(band.index_zoom, band.extent_bits),
            cooling,
            walks,
            (now.time_interval_since_1970() + offset_seconds) / 86_400.0,
        )
        .into();
        {
            let mut state = self.state.lock();
            let current = state.tiles.get(&key).is_some_and(|held| held.revision == tile.revision);
            if current && state.cooling_days == cooling && state.heat_walks == walks {
                state.heated.insert(
                    key,
                    MapHeatedTile {
                        revision: tile.revision,
                        since_day: tile.since_day,
                        cooling_days: cooling,
                        heat_walks: walks,
                        hour,
                        stretches: stretches.clone(),
                    },
                );
            }
        }
        MapTrackTileStoreLookup { tile: Some(tile), stretches, needs_fetch }
    }

    /// colouring from the owner's settings or a friend's tiles, zero leaves a value as is
    /// true when something changed and the map must redraw
    pub fn set_colouring(&self, days: i64, walks: i64) -> bool {
        let mut state = self.state.lock();
        let next_days = if days > 0 { days } else { state.cooling_days };
        let next_walks = if walks > 0 { walks } else { state.heat_walks };
        if next_days == state.cooling_days && next_walks == state.heat_walks {
            return false;
        }
        state.cooling_days = next_days;
        state.heat_walks = next_walks;
        state.heated.clear();
        true
    }

    pub fn put(&self, tile: MapDecodedTile) {
        let capacity = Self::capacity();
        let mut state = self.state.lock();
        let key = tile.key;
        if state.tiles.insert(key, Arc::new(tile)).is_none() {
            state.recency.push(key);
        }
        state.heated.remove(&key);
        while state.recency.len() > capacity {
            let evicted = state.recency.remove(0);
            state.tiles.remove(&evicted);
            state.heated.remove(&evicted);
        }
    }

    /// server said these are still current so only their revision moves
    pub fn confirm(&self, keys: &[MapTileKey], revision: i64) {
        let mut state = self.state.lock();
        for key in keys {
            let Some(tile) = state.tiles.get(key).cloned() else {
                continue;
            };
            state.tiles.insert(
                *key,
                Arc::new(MapDecodedTile {
                    key: *key,
                    revision,
                    since_day: tile.since_day,
                    extent: tile.extent,
                    lines: tile.lines.clone(),
                }),
            );
            if let Some(held) = state.heated.get_mut(key) {
                held.revision = revision;
            }
        }
    }

    pub fn snapshot(&self, key: MapTileKey) -> Option<(i64, i32)> {
        self.state.lock().tiles.get(&key).map(|tile| (tile.revision, tile.since_day))
    }

    pub fn remove_all(&self) {
        let mut state = self.state.lock();
        state.tiles.clear();
        state.heated.clear();
        state.cooling_days = Self::default_cooling_days();
        state.heat_walks = Self::default_heat_walks();
        state.recency.clear();
        state.bands.clear();
        state.current_revision = 0;
    }

    /// unprojects a payload into lines
    /// pure, runs wherever the caller is
    pub fn decode(
        key: MapTileKey,
        band: &MapTrackBand,
        payload: &[u8],
        revision: i64,
        since_day: i32,
        part_lines: &[u32],
    ) -> Option<MapDecodedTile> {
        let parts = MapTrackCodec::decode_tile(
            payload,
            Self::maximum_parts(),
            Self::maximum_vertices(),
            Self::maximum_passes(),
        )?;

        let extent = (1_i64 << band.extent_bits) as f64;
        let world = (1_i64 << (band.index_zoom + band.extent_bits)) as f64;
        let origin_x = key.x as f64 * extent;
        let origin_y = key.y as f64 * extent;
        let line_ids: &[u32] = if part_lines.len() == parts.len() { part_lines } else { &[] };
        let mut lines = Vec::with_capacity(parts.len());
        for (index, part) in parts.into_iter().enumerate() {
            if part.vertices.is_empty() {
                continue;
            }
            let mut coordinates = Vec::with_capacity(part.vertices.len() * 2);
            let (mut min_lat, mut max_lat, mut min_lng, mut max_lng) = (90.0_f64, -90.0_f64, 180.0_f64, -180.0_f64);
            for vertex in &part.vertices {
                let (latitude, longitude) = MapTrackGeometry::coordinate(
                    origin_x + f64::from(vertex.x) + 0.5,
                    origin_y + f64::from(vertex.y) + 0.5,
                    world,
                );
                coordinates.push(latitude);
                coordinates.push(longitude);
                min_lat = min_lat.min(latitude);
                max_lat = max_lat.max(latitude);
                min_lng = min_lng.min(longitude);
                max_lng = max_lng.max(longitude);
            }
            lines.push(MapTrackLine {
                day: part.day,
                mode: part.mode,
                line_id: line_ids.get(index).copied(),
                passes: part.passes,
                cells: part.vertices,
                coordinates,
                min_latitude: min_lat,
                max_latitude: max_lat,
                min_longitude: min_lng,
                max_longitude: max_lng,
            });
        }
        Some(MapDecodedTile { key, revision, since_day, extent, lines: lines.into() })
    }
}
