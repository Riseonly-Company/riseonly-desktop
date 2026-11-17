use std::collections::HashSet;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::RiseLimits;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_engine::rise_upload::rise_url_session::{URLRequest, URLSession};
use serde::Deserialize;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::map::stores::map_actions::map_types::MapVenueKind;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct MapPlaceProbeTileKey {
    x: i64,
    y: i64,
}

#[derive(Default)]
struct MapPlaceProbeState {
    template: Option<String>,
    template_loaded_at_ms: Option<i64>,
    tiles: Vec<(MapPlaceProbeTileKey, Arc<Vec<MapVectorTilePlace>>)>,
}

#[derive(Deserialize)]
struct MapPlaceProbeTileJson {
    #[serde(default)]
    tiles: Option<Vec<String>>,
}

static MAP_PLACE_PROBE_SHARED: LazyLock<MapPlaceProbe> = LazyLock::new(MapPlaceProbe::new);

/// what kind of place the reader is standing at, read from the same vector tiles the map draws
/// only the kind leaves the device and never a name
/// asked once per stop so one small tile per place
pub struct MapPlaceProbe {
    state: RiseIsolated<MapPlaceProbeState>,
}

impl MapPlaceProbe {
    const ZOOM: u32 = 14;
    const TILE_JSON: &'static str = "https://tiles.openfreemap.org/planet";
    const TEMPLATE_LIFETIME_MS: i64 = 86_400_000;
    const KEPT_TILES: usize = 4;
    const MAXIMUM_TILE_BYTES: usize = 4_000_000;
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

    pub fn shared() -> &'static MapPlaceProbe {
        &MAP_PLACE_PROBE_SHARED
    }

    fn new() -> Self {
        Self { state: RiseIsolated::new(MapPlaceProbeState::default()) }
    }

    fn radius_meters() -> f64 {
        RiseLimits::client_value("map_place_probe_radius_m") as f64
    }

    pub async fn kind(&self, latitude: f64, longitude: f64) -> Option<MapVenueKind> {
        if !(latitude.abs() < 85.0 && longitude.abs() <= 180.0) {
            return None;
        }
        let scale = f64::from(1u32 << Self::ZOOM);
        let world_x = (longitude + 180.0) / 360.0 * scale;
        let sine = (latitude * std::f64::consts::PI / 180.0).sin();
        let world_y = (0.5 - ((1.0 + sine) / (1.0 - sine)).ln() / (4.0 * std::f64::consts::PI)) * scale;
        let meters_per_tile = 40_075_016.686 * (latitude * std::f64::consts::PI / 180.0).cos() / scale;
        let radius_meters = Self::radius_meters();
        let reach = radius_meters / meters_per_tile;
        let tiles_across = scale as i64;

        let mut keys: HashSet<MapPlaceProbeTileKey> = HashSet::new();
        for dx in [-reach, 0.0, reach] {
            for dy in [-reach, 0.0, reach] {
                let x = (world_x + dx).floor() as i64;
                let y = (world_y + dy).floor() as i64;
                if y < 0 || y >= tiles_across {
                    continue;
                }
                keys.insert(MapPlaceProbeTileKey { x: (x % tiles_across + tiles_across) % tiles_across, y });
            }
        }

        let mut best: Option<(MapVenueKind, f64)> = None;
        for key in keys {
            let Some(places) = self.places(key).await else {
                continue;
            };
            for place in places.iter() {
                let dx = (key.x as f64 + place.x - world_x) * meters_per_tile;
                let dy = (key.y as f64 + place.y - world_y) * meters_per_tile;
                let meters = (dx * dx + dy * dy).sqrt();
                if meters > radius_meters || meters >= best.map(|(_, best)| best).unwrap_or(f64::INFINITY) {
                    continue;
                }
                let Some(kind) = MapVenueKind::from_poi(&place.poi_class, &place.subclass) else {
                    continue;
                };
                best = Some((kind, meters));
            }
        }
        best.map(|(kind, _)| kind)
    }

    async fn places(&self, key: MapPlaceProbeTileKey) -> Option<Arc<Vec<MapVectorTilePlace>>> {
        let hit = self
            .state
            .with(|state| state.tiles.iter().find(|(cached, _)| *cached == key).map(|(_, places)| places.clone()));
        if hit.is_some() {
            return hit;
        }
        let template = self.resolved_template().await?;
        let address = template
            .replace("{z}", &Self::ZOOM.to_string())
            .replace("{x}", &key.x.to_string())
            .replace("{y}", &key.y.to_string());
        let url = Url::from_string(&address)?;
        let mut request = URLRequest::new(url);
        request.timeout_interval = Self::REQUEST_TIMEOUT;
        let (data, response) = URLSession::shared().data_for(request).await.ok()?;
        if response.status_code != 200 || data.len() > Self::MAXIMUM_TILE_BYTES {
            return None;
        }

        let places = Arc::new(MapVectorTile::places(&data));
        self.state.with(|state| {
            state.tiles.push((key, places.clone()));
            if state.tiles.len() > Self::KEPT_TILES {
                let excess = state.tiles.len() - Self::KEPT_TILES;
                state.tiles.drain(..excess);
            }
        });
        Some(places)
    }

    async fn resolved_template(&self) -> Option<String> {
        let now_ms = RiseClock::now().ms();
        let cached = self.state.with(|state| {
            let fresh = state
                .template_loaded_at_ms
                .is_some_and(|loaded_at| now_ms - loaded_at < Self::TEMPLATE_LIFETIME_MS);
            (state.template.clone(), fresh)
        });
        if let (Some(template), true) = &cached {
            return Some(template.clone());
        }
        let fallback = cached.0;
        let Some(url) = Url::from_string(Self::TILE_JSON) else {
            return fallback;
        };
        let mut request = URLRequest::new(url);
        request.timeout_interval = Self::REQUEST_TIMEOUT;
        let Ok((data, response)) = URLSession::shared().data_for(request).await else {
            return fallback;
        };
        if response.status_code != 200 {
            return fallback;
        }
        let Some(first) = serde_json::from_slice::<MapPlaceProbeTileJson>(&data)
            .ok()
            .and_then(|document| document.tiles)
            .and_then(|tiles| tiles.into_iter().next())
            .filter(|first| first.starts_with("https://"))
        else {
            return fallback;
        };
        let loaded_at_ms = RiseClock::now().ms();
        self.state.with(|state| {
            state.template = Some(first.clone());
            state.template_loaded_at_ms = Some(loaded_at_ms);
        });
        Some(first)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct MapVectorTilePlace {
    pub x: f64,
    pub y: f64,
    pub poi_class: String,
    pub subclass: String,
}

pub enum MapVectorTile {}

impl MapVectorTile {
    const MAXIMUM_PLACES: usize = 4_096;

    pub fn places(data: &[u8]) -> Vec<MapVectorTilePlace> {
        let mut reader = MapVectorTileReader::new(data);
        let mut found: Vec<MapVectorTilePlace> = Vec::new();
        while let Some((field, wire)) = reader.tag() {
            if field == 3 && wire == 2 {
                let Some(layer) = reader.slice() else {
                    break;
                };
                found.extend(Self::places_in_layer(layer));
                if found.len() >= Self::MAXIMUM_PLACES {
                    break;
                }
            } else if !reader.skip(wire) {
                break;
            }
        }
        found
    }

    fn places_in_layer(bytes: &[u8]) -> Vec<MapVectorTilePlace> {
        let mut reader = MapVectorTileReader::new(bytes);
        let mut name = String::new();
        let mut keys: Vec<String> = Vec::new();
        let mut values: Vec<Option<String>> = Vec::new();
        let mut features: Vec<&[u8]> = Vec::new();
        let mut extent = 4_096.0;

        while let Some((field, wire)) = reader.tag() {
            match (field, wire) {
                (1, 2) => {
                    let Some(text) = reader.string() else {
                        return Vec::new();
                    };
                    name = text;
                    if name != "poi" {
                        return Vec::new();
                    }
                }
                (2, 2) => {
                    let Some(feature) = reader.slice() else {
                        return Vec::new();
                    };
                    features.push(feature);
                }
                (3, 2) => {
                    let Some(key) = reader.string() else {
                        return Vec::new();
                    };
                    keys.push(key);
                }
                (4, 2) => {
                    let Some(value) = reader.slice() else {
                        return Vec::new();
                    };
                    values.push(Self::string_value(value));
                }
                (5, 0) => {
                    let Some(raw) = reader.varint() else {
                        return Vec::new();
                    };
                    extent = (raw as f64).max(1.0);
                }
                _ => {
                    if !reader.skip(wire) {
                        return Vec::new();
                    }
                }
            }
        }
        if name != "poi" {
            return Vec::new();
        }
        let Some(class_key) = keys.iter().position(|key| key == "class") else {
            return Vec::new();
        };
        let subclass_key = keys.iter().position(|key| key == "subclass");

        features
            .into_iter()
            .take(Self::MAXIMUM_PLACES)
            .filter_map(|feature| Self::place(feature, class_key, subclass_key, &values, extent))
            .collect()
    }

    fn place(
        bytes: &[u8],
        class_key: usize,
        subclass_key: Option<usize>,
        values: &[Option<String>],
        extent: f64,
    ) -> Option<MapVectorTilePlace> {
        let mut reader = MapVectorTileReader::new(bytes);
        let mut tags: Vec<u64> = Vec::new();
        let mut geometry: Vec<u64> = Vec::new();
        let mut geometry_type: u64 = 0;

        while let Some((field, wire)) = reader.tag() {
            match (field, wire) {
                (2, 2) => tags = reader.packed_varints()?,
                (3, 0) => geometry_type = reader.varint()?,
                (4, 2) => geometry = reader.packed_varints()?,
                _ => {
                    if !reader.skip(wire) {
                        return None;
                    }
                }
            }
        }
        if !(geometry_type == 1 && geometry.len() >= 3 && geometry[0] & 7 == 1 && geometry[0] >> 3 >= 1) {
            return None;
        }
        let x = Self::zigzag(geometry[1]) as f64 / extent;
        let y = Self::zigzag(geometry[2]) as f64 / extent;
        if !((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)) {
            return None;
        }

        let mut poi_class = String::new();
        let mut subclass = String::new();
        for pair in tags.chunks_exact(2) {
            let key = pair[0] as i64;
            let value = pair[1] as i64;
            let Some(Some(text)) = usize::try_from(value).ok().and_then(|value| values.get(value)) else {
                continue;
            };
            if usize::try_from(key).ok() == Some(class_key) {
                poi_class = text.clone();
            }
            if subclass_key.is_some() && usize::try_from(key).ok() == subclass_key {
                subclass = text.clone();
            }
        }
        if poi_class.is_empty() && subclass.is_empty() {
            return None;
        }
        Some(MapVectorTilePlace { x, y, poi_class, subclass })
    }

    fn string_value(bytes: &[u8]) -> Option<String> {
        let mut reader = MapVectorTileReader::new(bytes);
        while let Some((field, wire)) = reader.tag() {
            if field == 1 && wire == 2 {
                return reader.string();
            }
            if !reader.skip(wire) {
                return None;
            }
        }
        None
    }

    fn zigzag(value: u64) -> i64 {
        ((value >> 1) as i64) ^ -((value & 1) as i64)
    }
}

struct MapVectorTileReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> MapVectorTileReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn varint(&mut self) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        while self.cursor < self.bytes.len() && shift < 64 {
            let byte = self.bytes[self.cursor];
            self.cursor += 1;
            result |= u64::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
        }
        None
    }

    fn tag(&mut self) -> Option<(u64, u64)> {
        if self.cursor >= self.bytes.len() {
            return None;
        }
        let raw = self.varint()?;
        Some((raw >> 3, raw & 7))
    }

    fn slice(&mut self) -> Option<&'a [u8]> {
        let length = self.varint()?;
        let remaining = self.bytes.len() - self.cursor;
        if length > remaining as u64 {
            return None;
        }
        let start = self.cursor;
        self.cursor += length as usize;
        Some(&self.bytes[start..self.cursor])
    }

    fn string(&mut self) -> Option<String> {
        self.slice().and_then(|bytes| String::from_utf8(bytes.to_vec()).ok())
    }

    fn packed_varints(&mut self) -> Option<Vec<u64>> {
        let body = self.slice()?;
        let mut inner = MapVectorTileReader::new(body);
        let mut values: Vec<u64> = Vec::new();
        while inner.cursor < inner.bytes.len() {
            values.push(inner.varint()?);
        }
        Some(values)
    }

    fn skip(&mut self, wire: u64) -> bool {
        match wire {
            0 => self.varint().is_some(),
            1 => self.advance(8),
            2 => self.slice().is_some(),
            5 => self.advance(4),
            _ => false,
        }
    }

    fn advance(&mut self, count: usize) -> bool {
        if self.bytes.len() - self.cursor < count {
            return false;
        }
        self.cursor += count;
        true
    }
}
