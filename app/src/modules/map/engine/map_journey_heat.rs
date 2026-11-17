use std::sync::{Arc, LazyLock};

use gpui::{Point, point};
use rise_canon::RiseLimits;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::map::engine::core::map_track_heat::{MapHeatPolicy, MapHeatStretch, MapTrackHeat};
use crate::modules::map::engine::core::map_track_tile_store::{MapDecodedTile, MapTrackLine};
use crate::modules::map::engine::rise_map_journeys_repository::MapJourneySnapshot;
use crate::modules::map::recorder::map_track_codec::MapTrackGeometry;
use crate::modules::map::stores::map_actions::map_tracks_types::MapTileKey;

static POINT_BUDGET: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_journey_max_output_points"));
static DEFAULT_WALKS: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_track_heat_walks_default"));

pub enum MapJourneyHeat {}

impl MapJourneyHeat {
    const WORLD_METERS: f64 = 40_075_016.686;

    fn point_budget() -> i64 {
        *POINT_BUDGET
    }

    fn default_walks() -> i64 {
        *DEFAULT_WALKS
    }

    /// frozen frequency map of the published routes
    /// never reads the owner's live tiles
    pub fn prepare(snapshot: &MapJourneySnapshot) -> Vec<MapHeatStretch> {
        let point_budget = Self::point_budget();
        let Some(anchor) = snapshot.lines.first().and_then(|line| line.first()).map(|point| point.lng) else {
            return Vec::new();
        };
        let total: i64 = snapshot.lines.iter().map(|line| line.len() as i64).sum();
        if snapshot.show_heat == Some(false) || total > point_budget {
            return Vec::new();
        }
        let mut projected: Vec<Vec<(f64, f64)>> = Vec::new();
        let mut coordinates: Vec<Vec<f64>> = Vec::new();
        let mut origin = (f64::INFINITY, f64::INFINITY);
        let mut high = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut length = 0.0_f64;
        for line in &snapshot.lines {
            if line.len() < 2 {
                continue;
            }
            let mut points: Vec<(f64, f64)> = Vec::new();
            let mut geographic: Vec<f64> = Vec::new();
            for p in line {
                if !p.lat.is_finite() || !p.lng.is_finite() || p.lat.abs() > 85.0 {
                    return Vec::new();
                }
                let lng = anchor + (p.lng - anchor + 540.0) % 360.0 - 180.0;
                let mercator = MapTrackGeometry::mercator(p.lat, 0.0);
                let point = ((lng + 180.0) / 360.0 * Self::WORLD_METERS, mercator.1 * Self::WORLD_METERS);
                if let Some(previous) = points.last() {
                    length += (point.0 - previous.0).hypot(point.1 - previous.1);
                }
                origin = (origin.0.min(point.0), origin.1.min(point.1));
                high = (high.0.max(point.0), high.1.max(point.1));
                points.push(point);
                geographic.extend([p.lat, lng]);
            }
            projected.push(points);
            coordinates.push(geographic);
        }
        if projected.is_empty() || !length.is_finite() {
            return Vec::new();
        }
        let cell_meters = (MapHeatPolicy::PLACE_METERS / 3.0).max(length / point_budget.max(1) as f64);
        let extent = ((high.0 - origin.0).max(high.1 - origin.1) / cell_meters).ceil() + 4.0;
        let mut lines: Vec<MapTrackLine> = Vec::new();
        for (index, points) in projected.iter().enumerate() {
            let cells: Vec<Point<i32>> = points
                .iter()
                .map(|p| {
                    point(
                        ((p.0 - origin.0) / cell_meters).round() as i32 + 2,
                        ((p.1 - origin.1) / cell_meters).round() as i32 + 2,
                    )
                })
                .collect();
            let geographic = coordinates[index].clone();
            let latitudes: Vec<f64> = geographic.iter().step_by(2).copied().collect();
            let longitudes: Vec<f64> = geographic.iter().skip(1).step_by(2).copied().collect();
            let min = |values: &[f64]| values.iter().copied().reduce(f64::min).unwrap_or(0.0);
            let max = |values: &[f64]| values.iter().copied().reduce(f64::max).unwrap_or(0.0);
            lines.push(MapTrackLine {
                day: 0,
                mode: 0,
                line_id: None,
                passes: Vec::new(),
                cells,
                min_latitude: min(&latitudes),
                max_latitude: max(&latitudes),
                min_longitude: min(&longitudes),
                max_longitude: max(&longitudes),
                coordinates: geographic,
            });
        }
        let tile = MapDecodedTile {
            key: MapTileKey { band: 0, x: 0, y: 0 },
            revision: 0,
            since_day: 0,
            extent,
            lines: Arc::from(lines),
        };
        MapTrackHeat::stretches(
            &tile,
            MapHeatPolicy::from_journey_cell_meters(cell_meters),
            1,
            snapshot.heat_walks.unwrap_or_else(Self::default_walks),
            0.0,
        )
    }
}
