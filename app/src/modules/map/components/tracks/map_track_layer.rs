use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui::{App, AppContext, AsyncApp, Task};
use parking_lot::Mutex;
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::Date;
use rise_media::map_view::{ComputedTiles, LatLng, TileId};
use serde_json::{Value, json};

use crate::modules::map::components::rise_map_host::{RiseMapHost, WeakRiseMapHost};
use crate::modules::map::engine::core::map_track_heat::MapHeatStretch;
use crate::modules::map::engine::core::map_track_tile_store::MapTrackTileStore;
use crate::modules::map::engine::map_track_erasure_geometry::{
    MapEraseStroke, MapErasureMask, MapErasureMaskPreparedStroke,
};
use crate::modules::map::engine::rise_map_tracks_repository::RiseMapTrackTilesTick;
use crate::modules::map::recorder::map_track_codec::MapTrackGeometry;
use crate::modules::map::stores::map_actions::map_tracks_types::{
    MapPlace, MapPlacePolicy, MapTileKey, MapTrackBand,
};
use crate::modules::map::stores::map_actions::map_types::MapSettings;
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;

static METERS_PER_DEGREE: LazyLock<f64> = LazyLock::new(|| MapTrackGeometry::haversine_meters(0.0, 0.0, 1.0, 0.0));

/// south west and north east corners of a region
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapTrackBounds {
    pub sw: LatLng,
    pub ne: LatLng,
}

impl MapTrackBounds {
    /// the whole mercator world, what a full invalidation covers
    pub fn world() -> Self {
        Self {
            sw: LatLng::new(-MapTrackGeometry::MAXIMUM_MERCATOR_LATITUDE, -180.0),
            ne: LatLng::new(MapTrackGeometry::MAXIMUM_MERCATOR_LATITUDE, 180.0),
        }
    }

    pub fn intersects(&self, other: &MapTrackBounds) -> bool {
        self.ne.latitude >= other.sw.latitude
            && self.sw.latitude <= other.ne.latitude
            && self.ne.longitude >= other.sw.longitude
            && self.sw.longitude <= other.ne.longitude
    }

    fn union(&self, other: &MapTrackBounds) -> MapTrackBounds {
        MapTrackBounds {
            sw: LatLng::new(self.sw.latitude.min(other.sw.latitude), self.sw.longitude.min(other.sw.longitude)),
            ne: LatLng::new(self.ne.latitude.max(other.ne.latitude), self.ne.longitude.max(other.ne.longitude)),
        }
    }
}

/// display cutout around the owner's home
/// split crossing edges too and not just inside vertices
/// or a sparse route could draw straight through the house
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapTrackHomeExclusion {
    pub latitude: f64,
    pub longitude: f64,
    pub radius: f64,
}

impl MapTrackHomeExclusion {
    pub fn new(home: Option<&MapPlace>, radius_meters: i64) -> Option<Self> {
        let home = home?;
        Some(Self {
            latitude: home.latitude,
            longitude: home.longitude,
            radius: MapSettings::clamped_home_track_radius(radius_meters) as f64,
        })
    }

    pub fn from_place(place: &MapPlace) -> Self {
        Self {
            latitude: place.latitude,
            longitude: place.longitude,
            radius: MapPlacePolicy::radius(place.track_exclusion_radius_meters) as f64,
        }
    }

    pub fn clip(&self, coordinates: &[LatLng]) -> Vec<Vec<LatLng>> {
        let Some(first) = coordinates.first() else { return Vec::new() };
        let meters_per_degree = *METERS_PER_DEGREE;
        let scale_x = meters_per_degree * 0.000_001_f64.max((self.latitude * std::f64::consts::PI / 180.0).cos());
        let radius = self.radius;
        let radius_squared = radius * radius;
        let longitude_offset = |value: f64| -> f64 {
            let mut delta = (value - self.longitude) % 360.0;
            if delta > 180.0 {
                delta -= 360.0;
            }
            if delta < -180.0 {
                delta += 360.0;
            }
            delta
        };
        let point = |coordinate: &LatLng| -> (f64, f64) {
            (
                longitude_offset(coordinate.longitude) * scale_x,
                (coordinate.latitude - self.latitude) * meters_per_degree,
            )
        };
        if coordinates.len() == 1 {
            let p = point(first);
            return if p.0 * p.0 + p.1 * p.1 < radius_squared { Vec::new() } else { vec![coordinates.to_vec()] };
        }
        let mut lines: Vec<Vec<LatLng>> = Vec::new();
        let mut line: Vec<LatLng> = Vec::new();
        let finish = |line: &mut Vec<LatLng>, lines: &mut Vec<Vec<LatLng>>| {
            if line.len() >= 2 {
                lines.push(line.clone());
            }
            line.clear();
        };
        for index in 1..coordinates.len() {
            let start = coordinates[index - 1];
            let end = coordinates[index];
            let a = point(&start);
            let mut b = point(&end);
            let half_world = 180.0 * scale_x;
            if b.0 - a.0 > half_world {
                b.0 -= 2.0 * half_world;
            }
            if b.0 - a.0 < -half_world {
                b.0 += 2.0 * half_world;
            }
            if a.0.min(b.0) > radius || a.0.max(b.0) < -radius || a.1.min(b.1) > radius || a.1.max(b.1) < -radius {
                if line.is_empty() {
                    line.push(start);
                }
                line.push(end);
                continue;
            }
            let dx = b.0 - a.0;
            let dy = b.1 - a.1;
            let length_squared = dx * dx + dy * dy;
            let dot = a.0 * dx + a.1 * dy;
            let constant = a.0 * a.0 + a.1 * a.1 - radius_squared;
            let discriminant = dot * dot - length_squared * constant;
            let mut entry = 0.0;
            let mut exit = 0.0;
            if length_squared > 0.0 && discriminant > 0.0 {
                let root = discriminant.sqrt();
                entry = 0.0_f64.max((-dot - root) / length_squared);
                exit = 1.0_f64.min((-dot + root) / length_squared);
            } else if length_squared == 0.0 && constant < 0.0 {
                finish(&mut line, &mut lines);
                continue;
            }
            if exit <= entry {
                if line.is_empty() {
                    line.push(start);
                }
                line.push(end);
                continue;
            }
            let boundary = |t: f64| -> LatLng {
                let mut lng = self.longitude + (a.0 + t * dx) / scale_x;
                if lng > 180.0 {
                    lng -= 360.0;
                }
                if lng < -180.0 {
                    lng += 360.0;
                }
                LatLng::new(self.latitude + (a.1 + t * dy) / meters_per_degree, lng)
            };
            if entry > 0.0 {
                if line.is_empty() {
                    line.push(start);
                }
                line.push(boundary(entry));
            }
            finish(&mut line, &mut lines);
            if exit < 1.0 {
                line = vec![boundary(exit), end];
            }
        }
        finish(&mut line, &mut lines);
        lines
    }
}

/// what a computed pass over the tiles in view produced
/// geojson features and the index tiles the store doesnt hold yet
#[derive(Clone, Debug, Default)]
pub struct MapTrackTileFeatures {
    pub features: Vec<Value>,
    pub misses: Vec<MapTileKey>,
}

#[derive(Default)]
struct MapTrackTileProviderState {
    store: Option<Arc<MapTrackTileStore>>,
    home_exclusion: Option<MapTrackHomeExclusion>,
    place_exclusions: Vec<MapTrackHomeExclusion>,
    erasures: Vec<MapEraseStroke>,
    erasure_generation: u64,
    prepared_erasures: Option<(u64, Arc<MapErasureMask>)>,
    prepared_strokes: HashMap<String, MapErasureMaskPreparedStroke>,
}

/// serves track tiles from the synchronous tile store
/// runs on a background executor so all state is behind a lock and nothing blocks
/// held tiles are returned and missing ones are reported
#[derive(Default)]
pub struct MapTrackTileProvider {
    state: Mutex<MapTrackTileProviderState>,
}

impl MapTrackTileProvider {
    const MAXIMUM_SPAN: i64 = 4;

    pub fn configure(&self, store: Option<Arc<MapTrackTileStore>>) {
        self.state.lock().store = store;
    }

    pub fn set_home_exclusion(&self, exclusion: Option<MapTrackHomeExclusion>) {
        self.state.lock().home_exclusion = exclusion;
    }

    pub fn set_place_exclusions(&self, exclusions: Vec<MapTrackHomeExclusion>) {
        self.state.lock().place_exclusions = exclusions;
    }

    pub fn set_erasures(&self, strokes: Vec<MapEraseStroke>) {
        let mut state = self.state.lock();
        state.erasures = strokes;
        state.erasure_generation = state.erasure_generation.wrapping_add(1);
        state.prepared_erasures = None;
    }

    pub fn erasure_mask(&self) -> Arc<MapErasureMask> {
        let (generation, strokes, held) = {
            let state = self.state.lock();
            if let Some((generation, mask)) = &state.prepared_erasures
                && *generation == state.erasure_generation
            {
                return mask.clone();
            }
            (state.erasure_generation, state.erasures.clone(), state.prepared_strokes.clone())
        };
        let prepared: Vec<MapErasureMaskPreparedStroke> = strokes
            .into_iter()
            .map(|stroke| match held.get(&stroke.id) {
                Some(cached) if cached.stroke == stroke => cached.clone(),
                _ => MapErasureMaskPreparedStroke::new(stroke),
            })
            .collect();
        let mask = Arc::new(MapErasureMask::new(prepared.clone()));
        let mut state = self.state.lock();
        if generation == state.erasure_generation {
            state.prepared_strokes = prepared.into_iter().map(|stroke| (stroke.stroke.id.clone(), stroke)).collect();
            state.prepared_erasures = Some((generation, mask.clone()));
        }
        mask
    }

    pub fn features_in_view(&self, tiles: &[TileId]) -> MapTrackTileFeatures {
        let (store, home_exclusion, place_exclusions) = {
            let state = self.state.lock();
            (state.store.clone(), state.home_exclusion, state.place_exclusions.clone())
        };
        let Some(store) = store else { return MapTrackTileFeatures::default() };
        let Some(first) = tiles.first() else { return MapTrackTileFeatures::default() };
        let erasures = self.erasure_mask();
        let view = tiles.iter().skip(1).fold(
            Self::bounds(i64::from(first.x), i64::from(first.y), i64::from(first.zoom), 0.1),
            |view, tile| view.union(&Self::bounds(i64::from(tile.x), i64::from(tile.y), i64::from(tile.zoom), 0.1)),
        );
        let mut seen: HashSet<MapTileKey> = HashSet::new();
        let mut result = MapTrackTileFeatures::default();
        for tile in tiles {
            let (x, y, zoom) = (i64::from(tile.x), i64::from(tile.y), i64::from(tile.zoom));
            let Some(band) = store.band(zoom) else { continue };
            let keys = Self::index_tiles(x, y, zoom, &band);
            if keys.is_empty() {
                continue;
            }
            let is_detail = store.is_detail_band(band.band);
            for key in keys {
                if !seen.insert(key) {
                    continue;
                }
                let lookup = store.lookup(key, is_detail);
                if lookup.needs_fetch {
                    result.misses.push(key);
                }
                let stretches: Vec<MapHeatStretch> = if lookup.tile.is_none() {
                    Self::cached_fallback(&store, x, y, zoom, &band, &mut seen)
                } else {
                    lookup.stretches.to_vec()
                };
                for stretch in stretches.iter().filter(|stretch| {
                    stretch.intersects(view.sw.latitude, view.ne.latitude, view.sw.longitude, view.ne.longitude)
                }) {
                    result.features.extend(Self::features(stretch, home_exclusion.as_ref(), &place_exclusions, &erasures));
                }
            }
        }
        result
    }

    fn cached_fallback(
        store: &MapTrackTileStore,
        x: i64,
        y: i64,
        zoom: i64,
        band: &MapTrackBand,
        seen: &mut HashSet<MapTileKey>,
    ) -> Vec<MapHeatStretch> {
        let bounds = Self::bounds(x, y, zoom, 0.1);
        for candidate in store.fallback_bands(band) {
            let keys = Self::index_tiles(x, y, zoom, &candidate);
            if keys.is_empty() {
                continue;
            }
            let detail = store.is_detail_band(candidate.band);
            let held: Vec<(MapTileKey, Vec<MapHeatStretch>)> = keys
                .iter()
                .map(|key| {
                    let stretches = store
                        .lookup(*key, detail)
                        .stretches
                        .iter()
                        .filter(|stretch| {
                            stretch.intersects(
                                bounds.sw.latitude,
                                bounds.ne.latitude,
                                bounds.sw.longitude,
                                bounds.ne.longitude,
                            )
                        })
                        .cloned()
                        .collect();
                    (*key, stretches)
                })
                .collect();
            if held.iter().all(|(_, stretches)| stretches.is_empty()) {
                continue;
            }
            return held
                .into_iter()
                .filter(|(key, _)| seen.insert(*key))
                .flat_map(|(_, stretches)| stretches)
                .collect();
        }
        Vec::new()
    }

    pub fn bounds_of(key: &MapTileKey, band: &MapTrackBand) -> MapTrackBounds {
        Self::bounds(key.x, key.y, band.index_zoom, 0.0)
    }

    fn index_tiles(x: i64, y: i64, zoom: i64, band: &MapTrackBand) -> Vec<MapTileKey> {
        if zoom >= band.index_zoom {
            let shift = zoom - band.index_zoom;
            return vec![MapTileKey { band: band.band, x: x >> shift, y: y >> shift }];
        }
        let span = 1_i64 << (band.index_zoom - zoom);
        if span > Self::MAXIMUM_SPAN {
            return Vec::new();
        }
        let mut keys = Vec::with_capacity((span * span) as usize);
        for dx in 0..span {
            for dy in 0..span {
                keys.push(MapTileKey { band: band.band, x: x * span + dx, y: y * span + dy });
            }
        }
        keys
    }

    fn bounds(x: i64, y: i64, zoom: i64, padding: f64) -> MapTrackBounds {
        let world = (1_i64 << zoom) as f64;
        let top_left = MapTrackGeometry::coordinate(x as f64 - padding, y as f64 - padding, world);
        let bottom_right = MapTrackGeometry::coordinate((x + 1) as f64 + padding, (y + 1) as f64 + padding, world);
        MapTrackBounds {
            sw: LatLng::new(bottom_right.0, top_left.1),
            ne: LatLng::new(top_left.0, bottom_right.1),
        }
    }

    /// a pass collapsed into one cell is stored as a point and drawn as a dot
    fn features(
        stretch: &MapHeatStretch,
        home: Option<&MapTrackHomeExclusion>,
        places: &[MapTrackHomeExclusion],
        erasures: &MapErasureMask,
    ) -> Vec<Value> {
        let coordinates: Vec<LatLng> =
            stretch.coordinates.chunks_exact(2).map(|pair| LatLng::new(pair[0], pair[1])).collect();
        erasures
            .clip(&coordinates, stretch.day)
            .into_iter()
            .flat_map(|line| match home {
                Some(home) => home.clip(&line),
                None => vec![line],
            })
            .flat_map(|line| {
                places
                    .iter()
                    .fold(vec![line], |lines, zone| lines.iter().flat_map(|line| zone.clip(line)).collect())
            })
            .filter(|line| !line.is_empty())
            .map(|line| {
                let properties = json!({
                    (MapTrackLayer::DAY_KEY): stretch.day,
                    (MapTrackLayer::MODE_KEY): stretch.mode,
                    (MapTrackLayer::DOT_KEY): if line.len() == 1 { 1 } else { 0 },
                    (MapTrackLayer::TEMPERATURE_KEY): stretch.temperature,
                });
                let geometry = if line.len() == 1 {
                    json!({ "type": "Point", "coordinates": [line[0].longitude, line[0].latitude] })
                } else {
                    let points: Vec<[f64; 2]> = line.iter().map(|point| [point.longitude, point.latitude]).collect();
                    json!({ "type": "LineString", "coordinates": points })
                };
                json!({ "type": "Feature", "geometry": geometry, "properties": properties })
            })
            .collect()
    }
}

/// what the map shows of the tracks
/// visibility and the period's first day and the tail the server hasnt ingested yet
/// plus zones cut out of every line
#[derive(Clone, Debug)]
pub struct MapTrackLayerInput {
    pub is_visible: bool,
    pub since_day: i32,
    pub tail: Vec<Vec<LatLng>>,
    pub home_exclusion: Option<MapTrackHomeExclusion>,
    pub place_exclusions: Vec<MapTrackHomeExclusion>,
}

impl MapTrackLayerInput {
    pub fn new(is_visible: bool, since_day: i32, tail: Vec<Vec<LatLng>>) -> Self {
        Self { is_visible, since_day, tail, home_exclusion: None, place_exclusions: Vec::new() }
    }
}

impl PartialEq for MapTrackLayerInput {
    fn eq(&self, other: &Self) -> bool {
        self.is_visible == other.is_visible
            && self.since_day == other.since_day
            && self.home_exclusion == other.home_exclusion
            && self.place_exclusions == other.place_exclusions
            && MapTrackLayer::tail_signature(&self.tail) == MapTrackLayer::tail_signature(&other.tail)
    }
}

/// track layers in the map style
/// wide soft line under a narrow bright one colored by heat
/// plus dots for passes too short for a line and the live tail
pub struct MapTrackLayer {
    pub provider: Arc<MapTrackTileProvider>,
    map: Option<WeakRiseMapHost>,
    has_source: bool,
    applied: Option<MapTrackLayerInput>,
    applied_tick: u64,
    bands: Vec<MapTrackBand>,
    erasures: Vec<MapEraseStroke>,
    tail_task: Option<Task<()>>,
    tail_generation: Rc<Cell<u64>>,
    tiles_in_view: Vec<TileId>,
    tiles_task: Option<Task<()>>,
    tiles_generation: Rc<Cell<u64>>,
}

impl Default for MapTrackLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl MapTrackLayer {
    const SOURCE_ID: &'static str = "rise.tracks";
    const GLOW_ID: &'static str = "rise.tracks.glow";
    const LINE_ID: &'static str = "rise.tracks.line";
    const DOT_ID: &'static str = "rise.tracks.dot";
    const TAIL_SOURCE_ID: &'static str = "rise.tracks.tail";
    const TAIL_GLOW_ID: &'static str = "rise.tracks.tail.glow";
    const TAIL_LINE_ID: &'static str = "rise.tracks.tail.line";

    pub const TAPPABLE_LAYER_IDS: &'static [&'static str] =
        &[Self::LINE_ID, Self::GLOW_ID, Self::DOT_ID, Self::TAIL_LINE_ID, Self::TAIL_GLOW_ID];

    pub const TEMPERATURE_KEY: &'static str = "t";
    const DAY_KEY: &'static str = "day";
    const MODE_KEY: &'static str = "mode";
    const DOT_KEY: &'static str = "dot";

    const COMPUTED_TILES: ComputedTiles = ComputedTiles { min_zoom: 0, max_zoom: 18 };

    const TEMPERATURE_STOPS: [(f64, [f64; 3]); 6] = [
        (0.0, [0.42, 0.78, 1.0]),
        (0.2, [0.13, 0.36, 0.96]),
        (0.4, [0.13, 0.77, 0.37]),
        (0.6, [0.98, 0.80, 0.08]),
        (0.8, [0.98, 0.51, 0.13]),
        (1.0, [0.94, 0.20, 0.20]),
    ];

    pub fn new() -> Self {
        Self {
            provider: Arc::new(MapTrackTileProvider::default()),
            map: None,
            has_source: false,
            applied: None,
            applied_tick: 0,
            bands: Vec::new(),
            erasures: Vec::new(),
            tail_task: None,
            tail_generation: Rc::new(Cell::new(0)),
            tiles_in_view: Vec::new(),
            tiles_task: None,
            tiles_generation: Rc::new(Cell::new(0)),
        }
    }

    /// cheap tail identity
    /// a line only grows at its end and a segment only leaves whole
    pub fn tail_signature(tail: &[Vec<LatLng>]) -> Vec<f64> {
        let mut signature: Vec<f64> = Vec::with_capacity(1 + tail.len() * 3);
        signature.push(tail.len() as f64);
        for line in tail {
            signature.push(line.len() as f64);
            signature.push(line.last().map_or(0.0, |point| point.latitude));
            signature.push(line.last().map_or(0.0, |point| point.longitude));
        }
        signature
    }

    /// true if the layers were installed into the style the map shows now
    /// a new style drops every runtime source so the source still being there proves it
    fn is_installed(&self, map: &RiseMapHost) -> bool {
        self.has_source
            && map.is_style_loaded()
            && map.has_source(Self::SOURCE_ID)
            && self.map.as_ref().and_then(WeakRiseMapHost::upgrade).is_some_and(|held| held.same_map(map))
    }

    pub fn set_erasures(&mut self, strokes: Vec<MapEraseStroke>, map: &RiseMapHost, cx: &mut App) {
        if strokes == self.erasures {
            return;
        }
        self.erasures = strokes.clone();
        self.provider.set_erasures(strokes);
        self.invalidate(Some(MapTrackBounds::world()), map, cx);
        if let Some(applied) = self.applied.clone()
            && self.is_installed(map)
        {
            self.apply(applied, map, true, cx);
        }
    }

    pub fn style_did_load(&mut self, map: &RiseMapHost, cx: &mut App) {
        self.has_source = false;
        let pending = self.applied.take();
        let Some(pending) = pending else { return };
        self.install(map, pending, cx);
    }

    pub fn update(
        &mut self,
        input: &MapTrackLayerInput,
        tick: &RiseMapTrackTilesTick,
        bands: &[MapTrackBand],
        map: &RiseMapHost,
        cx: &mut App,
    ) {
        self.provider.set_home_exclusion(input.home_exclusion);
        self.provider.set_place_exclusions(input.place_exclusions.clone());
        self.bands = bands.to_vec();
        if map.is_style_loaded() {
            if !self.is_installed(map) {
                self.install(map, input.clone(), cx);
            } else if self.applied.as_ref() != Some(input) {
                self.apply(input.clone(), map, false, cx);
            }
        } else {
            self.applied = Some(input.clone());
        }
        if tick.generation == self.applied_tick || !self.has_source {
            return;
        }
        let skipped_invalidations = tick.generation != self.applied_tick.wrapping_add(1);
        self.applied_tick = tick.generation;
        if tick.invalidates_all || skipped_invalidations {
            self.invalidate(Some(MapTrackBounds::world()), map, cx);
            return;
        }
        for key in &tick.invalidated {
            let Some(band) = self.bands.iter().find(|band| band.band == key.band) else { continue };
            let bounds = MapTrackTileProvider::bounds_of(key, band);
            self.invalidate(Some(bounds), map, cx);
        }
    }

    pub fn region_did_change(&mut self, map: &RiseMapHost, cx: &mut App) {
        if !self.is_installed(map) {
            return;
        }
        let Some(projection) = map.projection() else { return };
        if self.tiles_in_view == Self::COMPUTED_TILES.tiles_in_view(&projection) {
            return;
        }
        self.invalidate(None, map, cx);
    }

    /// recomputes the tiles in view when the region touches them
    /// the union is sent whole so one pass serves every invalidated tile
    fn invalidate(&mut self, bounds: Option<MapTrackBounds>, map: &RiseMapHost, cx: &mut App) {
        if !self.has_source {
            return;
        }
        let Some(projection) = map.projection() else { return };
        let tiles = Self::COMPUTED_TILES.tiles_in_view(&projection);
        if let Some(bounds) = bounds {
            let touched = tiles.iter().any(|tile| {
                let (north_west, south_east) = tile.bounds();
                let tile_bounds = MapTrackBounds {
                    sw: LatLng::new(south_east.latitude, north_west.longitude),
                    ne: LatLng::new(north_west.latitude, south_east.longitude),
                };
                tile_bounds.intersects(&bounds)
            });
            if !touched && self.tiles_in_view == tiles {
                return;
            }
        }
        self.tiles_in_view = tiles.clone();
        let generation = self.tiles_generation.get().wrapping_add(1);
        self.tiles_generation.set(generation);
        let tiles_count = tiles.len();
        let tiles_zoom = tiles.first().map_or(0, |tile| tile.zoom);
        let provider = self.provider.clone();
        let computing = cx.background_spawn(async move { provider.features_in_view(&tiles) });
        let weak_map = map.downgrade();
        let current = self.tiles_generation.clone();
        self.tiles_task = Some(cx.spawn(async move |cx: &mut AsyncApp| {
            let computed = computing.await;
            tracing::warn!(
                target: "riseonly::X2068",
                tiles = tiles_count,
                zoom = tiles_zoom,
                features = computed.features.len(),
                misses = computed.misses.len(),
                "track layer computed"
            );
            cx.update(|cx| {
                if !computed.misses.is_empty() {
                    let missed = computed.misses;
                    MapInteractionsStore::shared(cx).update(cx, |store, cx| store.request_track_tiles(missed, cx));
                }
                if current.get() != generation {
                    return;
                }
                let Some(map) = weak_map.upgrade() else { return };
                if !map.has_source(Self::SOURCE_ID) {
                    return;
                }
                map.set_shape(Self::SOURCE_ID, Some(Self::collection(computed.features)));
                map.set_needs_display(cx);
            });
        }));
    }

    fn collection(features: Vec<Value>) -> Value {
        json!({ "type": "FeatureCollection", "features": features })
    }

    fn install(&mut self, map: &RiseMapHost, input: MapTrackLayerInput, cx: &mut App) {
        for id in [Self::TAIL_LINE_ID, Self::TAIL_GLOW_ID, Self::DOT_ID, Self::LINE_ID, Self::GLOW_ID] {
            map.remove_layer(id);
        }
        for id in [Self::TAIL_SOURCE_ID, Self::SOURCE_ID] {
            map.remove_source(id);
        }
        map.set_shape(Self::SOURCE_ID, None);
        map.set_shape(Self::TAIL_SOURCE_ID, None);
        let layers = [
            Self::line(Self::TAIL_GLOW_ID, Self::TAIL_SOURCE_ID, true, true, &input),
            Self::line(Self::TAIL_LINE_ID, Self::TAIL_SOURCE_ID, false, true, &input),
            Self::line(Self::GLOW_ID, Self::SOURCE_ID, true, false, &input),
            Self::line(Self::LINE_ID, Self::SOURCE_ID, false, false, &input),
            Self::dots(Self::SOURCE_ID, &input),
        ];
        let first_symbol = map.first_symbol_layer();
        for layer in layers {
            map.add_layer(layer, first_symbol.as_deref());
        }
        self.map = Some(map.downgrade());
        self.has_source = true;
        self.applied = None;
        self.tiles_in_view = Vec::new();
        self.apply(input, map, false, cx);
    }

    fn apply(&mut self, input: MapTrackLayerInput, map: &RiseMapHost, force_tail: bool, cx: &mut App) {
        let previous = self.applied.replace(input.clone());
        let home_changed = previous.as_ref().and_then(|previous| previous.home_exclusion) != input.home_exclusion
            || previous.as_ref().map(|previous| &previous.place_exclusions) != Some(&input.place_exclusions);
        if home_changed {
            self.invalidate(Some(MapTrackBounds::world()), map, cx);
        }

        let first_symbol = map.first_symbol_layer();
        let layers = [
            Self::line(Self::TAIL_GLOW_ID, Self::TAIL_SOURCE_ID, true, true, &input),
            Self::line(Self::TAIL_LINE_ID, Self::TAIL_SOURCE_ID, false, true, &input),
            Self::line(Self::GLOW_ID, Self::SOURCE_ID, true, false, &input),
            Self::line(Self::LINE_ID, Self::SOURCE_ID, false, false, &input),
            Self::dots(Self::SOURCE_ID, &input),
        ];
        if previous.is_some() {
            for layer in layers {
                map.add_layer(layer, first_symbol.as_deref());
            }
        }

        let tail_changed = force_tail
            || home_changed
            || previous.as_ref().is_none_or(|previous| {
                Self::tail_signature(&previous.tail) != Self::tail_signature(&input.tail)
            });
        if !tail_changed {
            return;
        }
        self.tail_task = None;
        let generation = self.tail_generation.get().wrapping_add(1);
        self.tail_generation.set(generation);
        if input.tail.is_empty() {
            map.set_shape(Self::TAIL_SOURCE_ID, None);
            return;
        }
        let now = Date::now();
        let today = MapTrackGeometry::local_day(
            (now.time_interval_since_1970() * 1_000.0) as i64,
            i64::from(TimeZone::current().seconds_from_gmt(&now) / 60),
        );
        let provider = self.provider.clone();
        let geometry = cx.background_spawn(async move {
            let mask = provider.erasure_mask();
            input
                .tail
                .iter()
                .flat_map(|line| {
                    mask.clip(line, today)
                        .into_iter()
                        .flat_map(|line| match &input.home_exclusion {
                            Some(home) => home.clip(&line),
                            None => vec![line],
                        })
                        .flat_map(|line| {
                            input.place_exclusions.iter().fold(vec![line], |lines, zone| {
                                lines.iter().flat_map(|line| zone.clip(line)).collect()
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<Vec<LatLng>>>()
        });
        let weak_map = map.downgrade();
        let current = self.tail_generation.clone();
        self.tail_task = Some(cx.spawn(async move |cx: &mut AsyncApp| {
            let visible_tail = geometry.await;
            if current.get() != generation {
                return;
            }
            let lines: Vec<Value> = visible_tail
                .into_iter()
                .filter(|line| line.len() >= 2)
                .map(|line| {
                    let points: Vec<[f64; 2]> = line.iter().map(|point| [point.longitude, point.latitude]).collect();
                    json!({
                        "type": "Feature",
                        "geometry": { "type": "LineString", "coordinates": points },
                        "properties": {},
                    })
                })
                .collect();
            cx.update(|cx| {
                let Some(map) = weak_map.upgrade() else { return };
                if !map.has_source(Self::TAIL_SOURCE_ID) {
                    return;
                }
                let shape = (!lines.is_empty()).then(|| Self::collection(lines));
                map.set_shape(Self::TAIL_SOURCE_ID, shape);
                map.set_needs_display(cx);
            });
        }));
    }

    fn color(rgb: [f64; 3]) -> String {
        format!("rgb({:.1}%, {:.1}%, {:.1}%)", rgb[0] * 100.0, rgb[1] * 100.0, rgb[2] * 100.0)
    }

    fn coldest() -> String {
        Self::color(Self::TEMPERATURE_STOPS[0].1)
    }

    /// a stretch takes its color from its own temperature
    /// the tail has none yet and is drawn as a road walked for the first time
    pub fn temperature_color() -> Value {
        let mut expression = vec![json!("interpolate"), json!(["linear"]), json!(["get", Self::TEMPERATURE_KEY])];
        for (stop, rgb) in Self::TEMPERATURE_STOPS {
            expression.push(json!(stop));
            expression.push(json!(Self::color(rgb)));
        }
        Value::Array(expression)
    }

    fn zoom_stops(stops: &[(f64, f64)]) -> Value {
        let mut expression = vec![json!("interpolate"), json!(["linear"]), json!(["zoom"])];
        for (zoom, value) in stops {
            expression.push(json!(zoom));
            expression.push(json!(value));
        }
        Value::Array(expression)
    }

    fn visibility(input: &MapTrackLayerInput) -> &'static str {
        if input.is_visible { "visible" } else { "none" }
    }

    fn since_filter(input: &MapTrackLayerInput) -> Value {
        json!([">=", ["get", Self::DAY_KEY], input.since_day])
    }

    fn line(id: &str, source: &str, is_glow: bool, is_tail: bool, input: &MapTrackLayerInput) -> Value {
        let mut layout = json!({
            "line-cap": "round",
            "line-join": "round",
            "visibility": Self::visibility(input),
        });
        let mut paint = serde_json::Map::new();
        if is_tail {
            paint.insert("line-color".to_owned(), json!(Self::coldest()));
        } else {
            paint.insert("line-color".to_owned(), Self::temperature_color());
            layout["line-sort-key"] = json!(["get", Self::TEMPERATURE_KEY]);
        }
        let widths: &[(f64, f64)] = if is_glow {
            &[(0.0, 5.0), (9.0, 8.0), (14.0, 12.0), (18.0, 18.0)]
        } else {
            &[(0.0, 1.6), (9.0, 2.4), (14.0, 3.4), (18.0, 5.0)]
        };
        paint.insert("line-width".to_owned(), Self::zoom_stops(widths));
        paint.insert("line-opacity".to_owned(), json!(if is_glow { 0.24 } else { 0.95 }));
        if is_glow {
            paint.insert("line-blur".to_owned(), json!(5));
        }
        let mut layer = json!({
            "id": id,
            "type": "line",
            "source": source,
            "layout": layout,
            "paint": Value::Object(paint),
        });
        if !is_tail {
            layer["filter"] = Self::since_filter(input);
        }
        layer
    }

    fn dots(source: &str, input: &MapTrackLayerInput) -> Value {
        json!({
            "id": Self::DOT_ID,
            "type": "circle",
            "source": source,
            "layout": {
                "visibility": Self::visibility(input),
                "circle-sort-key": ["get", Self::TEMPERATURE_KEY],
            },
            "paint": {
                "circle-radius": Self::zoom_stops(&[(0.0, 1.4), (9.0, 2.0), (14.0, 2.8)]),
                "circle-color": Self::temperature_color(),
                "circle-opacity": 0.95,
                "circle-blur": 0.3,
            },
            "filter": ["all", ["==", ["get", Self::DOT_KEY], 1], Self::since_filter(input)],
        })
    }
}
