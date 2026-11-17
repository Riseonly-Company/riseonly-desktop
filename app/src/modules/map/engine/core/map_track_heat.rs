use std::collections::HashMap;

use gpui::Point;

use crate::modules::map::engine::core::map_track_tile_store::{MapDecodedTile, MapTrackLine};

#[derive(Clone, PartialEq, Debug)]
pub struct MapHeatStretch {
    pub day: i32,
    pub mode: i32,
    pub temperature: f64,
    pub coordinates: Vec<f64>,
    pub min_latitude: f64,
    pub max_latitude: f64,
    pub min_longitude: f64,
    pub max_longitude: f64,
}

impl MapHeatStretch {
    pub fn intersects(&self, min_lat: f64, max_lat: f64, min_lng: f64, max_lng: f64) -> bool {
        self.max_latitude >= min_lat
            && self.min_latitude <= max_lat
            && self.max_longitude >= min_lng
            && self.min_longitude <= max_lng
    }
}

/// heat cell is a ground size, same at every zoom
/// tied to the tile it was 19 m on the street band and 330 m on the city band
/// neighbouring streets warmed each other and far views came out hotter
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapHeatPolicy {
    pub cells_per_heat_cell: f64,
    pub brush: i64,
    pub counts_day_once: bool,
}

impl MapHeatPolicy {
    pub const PLACE_METERS: f64 = 30.0;
    const EQUATOR_METERS: f64 = 40_075_016.686;

    pub fn from_journey_cell_meters(journey_cell_meters: f64) -> Self {
        Self {
            cells_per_heat_cell: (Self::PLACE_METERS / journey_cell_meters).max(1.0),
            brush: if journey_cell_meters <= Self::PLACE_METERS { 1 } else { 0 },
            counts_day_once: false,
        }
    }

    /// makes the heat policy from index zoom and extent bits
    pub fn new(index_zoom: i64, extent_bits: i64) -> Self {
        let geometry_meters = Self::EQUATOR_METERS / (1_i64 << (index_zoom + extent_bits)) as f64;
        let is_coarse = geometry_meters > Self::PLACE_METERS * 2.0;
        Self {
            cells_per_heat_cell: (Self::PLACE_METERS / geometry_meters).round().max(1.0),
            brush: if is_coarse { 0 } else { 1 },
            counts_day_once: is_coarse,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct MapHeatFieldCell {
    value: f32,
    stamp: u32,
    along: f32,
}

/// a street tile is a few hundred cells across and stored dense
/// a city tile is thousands across and almost all empty so its sparse
struct MapHeatField {
    side: i64,
    dense: Vec<f32>,
    dense_stamps: Vec<u32>,
    dense_along: Vec<f32>,
    sparse: HashMap<i64, MapHeatFieldCell>,
    is_dense: bool,
}

impl MapHeatField {
    const DENSE_SIDE_LIMIT: i64 = 512;

    fn new(side: i64) -> Self {
        let is_dense = side <= Self::DENSE_SIDE_LIMIT;
        let count = if is_dense { (side * side) as usize } else { 0 };
        Self {
            side,
            dense: vec![0.0; count],
            dense_stamps: vec![0; count],
            dense_along: vec![0.0; count],
            sparse: HashMap::new(),
            is_dense,
        }
    }

    /// adds weight once per visit of one stamp
    /// a touch more than reentry after the previous one counts as the line coming back
    /// so the way there and the way home are two walks
    fn add(&mut self, column: i64, row: i64, weight: f32, stamp: u32, along: f32, reentry: f32) {
        if column < 0 || column >= self.side || row < 0 || row >= self.side {
            return;
        }
        let index = row * self.side + column;
        if self.is_dense {
            let slot = index as usize;
            let is_same_visit = self.dense_stamps[slot] == stamp && along - self.dense_along[slot] < reentry;
            self.dense_stamps[slot] = stamp;
            self.dense_along[slot] = along;
            if !is_same_visit {
                self.dense[slot] += weight;
            }
        } else {
            let cell = self.sparse.entry(index).or_default();
            let is_same_visit = cell.stamp == stamp && along - cell.along < reentry;
            cell.stamp = stamp;
            cell.along = along;
            if !is_same_visit {
                cell.value += weight;
            }
        }
    }

    fn value(&self, column: i64, row: i64) -> f64 {
        let c = column.max(0).min(self.side - 1);
        let r = row.max(0).min(self.side - 1);
        let index = r * self.side + c;
        if self.is_dense {
            f64::from(self.dense[index as usize])
        } else {
            f64::from(self.sparse.get(&index).map_or(0.0, |cell| cell.value))
        }
    }

    fn raise(&mut self, column: i64, row: i64, minimum: f32) {
        if column < 0 || column >= self.side || row < 0 || row >= self.side {
            return;
        }
        let index = row * self.side + column;
        if self.is_dense {
            let slot = index as usize;
            self.dense[slot] = self.dense[slot].max(minimum);
        } else {
            let cell = self.sparse.entry(index).or_default();
            cell.value = cell.value.max(minimum);
        }
    }
}

/// links of a tile's lines into runs
/// next follows a line and has_previous marks a line some other line leads into
struct MapTrackHeatLinks {
    next: Vec<Option<usize>>,
    has_previous: Vec<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct MapTrackHeatEndpoint {
    day: i32,
    cell: Point<i32>,
}

/// warmth belongs to the place not the line
/// two walks of one street never make the same polyline so per line counting would never see it twice
/// tile is laid over a grid and each walk adds weight to its places
pub enum MapTrackHeat {}

impl MapTrackHeat {
    const REENTRY_CELLS: f32 = 5.0;
    pub const LEVELS: i64 = 40;
    const SMOOTHING_REACH: usize = 3;

    pub fn stretches(
        tile: &MapDecodedTile,
        policy: MapHeatPolicy,
        cooling_days: i64,
        walks_to_hottest: i64,
        now_day: f64,
    ) -> Vec<MapHeatStretch> {
        if tile.lines.is_empty() {
            return Vec::new();
        }
        let cells_per_heat_cell = policy.cells_per_heat_cell;
        let side = ((tile.extent / cells_per_heat_cell).ceil() as i64 + 1).max(1);
        let cooling = cooling_days.max(1) as f64;
        let mut field = MapHeatField::new(side);

        let reentry: f32 = if policy.counts_day_once { f32::INFINITY } else { Self::REENTRY_CELLS };

        if policy.counts_day_once {
            let mut visits: Vec<(i32, usize)> = Vec::new();
            for (index, line) in tile.lines.iter().enumerate() {
                if line.passes.is_empty() {
                    visits.push((line.day, index));
                } else {
                    for pass in &line.passes {
                        visits.push((pass.day, index));
                    }
                }
            }
            visits.sort_by_key(|visit| visit.0);
            let mut stamp: u32 = 0;
            let mut stamped_day: Option<i32> = None;
            for (day, line) in visits {
                let weight = Self::cooled(day, 1, cooling, now_day) as f32;
                if weight <= 0.0 {
                    continue;
                }
                if stamped_day != Some(day) {
                    stamped_day = Some(day);
                    stamp += 1;
                }
                Self::paint(&mut field, &tile.lines[line], weight, stamp, 0.0, policy, reentry);
            }
        } else {
            let links = Self::continuations(&tile.lines);
            let mut painted = vec![false; tile.lines.len()];
            let mut stamp: u32 = 0;
            for index in 0..tile.lines.len() {
                if !links.has_previous[index] {
                    Self::paint_run(index, tile, &links, &mut painted, &mut stamp, &mut field, policy, reentry, cooling, now_day);
                }
            }
            for index in 0..tile.lines.len() {
                Self::paint_run(index, tile, &links, &mut painted, &mut stamp, &mut field, policy, reentry, cooling, now_day);
            }

            let mut dots: Vec<(i32, Point<i32>)> = Vec::new();
            for line in tile.lines.iter().filter(|line| line.cells.len() == 1) {
                let cell = line.cells[0];
                if line.passes.is_empty() {
                    dots.push((line.day, cell));
                } else {
                    for pass in &line.passes {
                        dots.push((pass.day, cell));
                    }
                }
            }
            dots.sort_by_key(|dot| dot.0);
            let mut presence: HashMap<Point<i32>, (i32, f32)> = HashMap::new();
            for (day, dot_cell) in dots {
                let weight = Self::cooled(day, 1, cooling, now_day) as f32;
                if weight <= 0.0 {
                    continue;
                }
                let column = (f64::from(dot_cell.x) / cells_per_heat_cell) as i32;
                let row = (f64::from(dot_cell.y) / cells_per_heat_cell) as i32;
                for dy in -policy.brush..=policy.brush {
                    for dx in -policy.brush..=policy.brush {
                        let cell = Point { x: column.wrapping_add(dx as i32), y: row.wrapping_add(dy as i32) };
                        let held = presence.get(&cell).copied();
                        if i64::from(cell.x) < 0
                            || i64::from(cell.x) >= side
                            || i64::from(cell.y) < 0
                            || i64::from(cell.y) >= side
                            || held.map(|visit| visit.0) == Some(day)
                        {
                            continue;
                        }
                        presence.insert(cell, (day, held.map_or(0.0, |visit| visit.1) + weight));
                    }
                }
            }
            for (cell, (_, weight)) in presence {
                field.raise(i64::from(cell.x), i64::from(cell.y), weight);
            }
        }

        let mut result = Vec::with_capacity(tile.lines.len());
        for line in tile.lines.iter() {
            Self::append(line, &field, policy, walks_to_hottest.max(2) as f64, &mut result);
        }
        result
    }

    fn paint(
        field: &mut MapHeatField,
        line: &MapTrackLine,
        weight: f32,
        stamp: u32,
        offset: f64,
        policy: MapHeatPolicy,
        reentry: f32,
    ) -> f64 {
        let cells_per_heat_cell = policy.cells_per_heat_cell;
        let mut last = (i64::MIN, i64::MIN);
        let mut length = 0.0;
        Self::walk(&line.cells, cells_per_heat_cell / 2.0, |x, y, travelled| {
            length = travelled;
            let centre = ((x / cells_per_heat_cell) as i64, (y / cells_per_heat_cell) as i64);
            if centre == last {
                return;
            }
            last = centre;
            let along = ((offset + travelled) / cells_per_heat_cell) as f32;
            for dy in -policy.brush..=policy.brush {
                for dx in -policy.brush..=policy.brush {
                    field.add(centre.0 + dx, centre.1 + dy, weight, stamp, along, reentry);
                }
            }
        });
        offset + length
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_run(
        start: usize,
        tile: &MapDecodedTile,
        links: &MapTrackHeatLinks,
        painted: &mut [bool],
        stamp: &mut u32,
        field: &mut MapHeatField,
        policy: MapHeatPolicy,
        reentry: f32,
        cooling: f64,
        now_day: f64,
    ) {
        if painted[start] || tile.lines[start].cells.len() <= 1 {
            return;
        }
        *stamp += 1;
        let mut index = start;
        let mut offset = 0.0;
        while !painted[index] {
            painted[index] = true;
            let line = &tile.lines[index];
            let weight = Self::weight(line, cooling, now_day) as f32;
            if weight > 0.0 {
                offset = Self::paint(field, line, weight, *stamp, offset, policy, reentry);
            }
            let Some(next) = links.next[index] else {
                break;
            };
            index = next;
        }
    }

    /// upload checkpoints share an endpoint and day and pass history
    /// keep heat stamp and distance continuous since an upload boundary isnt another visit
    /// ambiguous junctions stay separate rather than guessing
    fn continuations(lines: &[MapTrackLine]) -> MapTrackHeatLinks {
        if lines.iter().any(|line| line.line_id.is_some()) {
            return Self::merged_runs(lines);
        }
        let mut starts: HashMap<MapTrackHeatEndpoint, Vec<usize>> = HashMap::new();
        let mut ends: HashMap<MapTrackHeatEndpoint, Vec<usize>> = HashMap::new();
        for (index, line) in lines.iter().enumerate() {
            if line.cells.len() <= 1 {
                continue;
            }
            let (Some(first), Some(last)) = (line.cells.first(), line.cells.last()) else {
                continue;
            };
            starts.entry(MapTrackHeatEndpoint { day: line.day, cell: *first }).or_default().push(index);
            ends.entry(MapTrackHeatEndpoint { day: line.day, cell: *last }).or_default().push(index);
        }
        let mut links = MapTrackHeatLinks { next: vec![None; lines.len()], has_previous: vec![false; lines.len()] };
        let are_equivalent = |indices: &[usize]| -> bool {
            let Some(first) = indices.first() else {
                return false;
            };
            indices[1..].iter().all(|other| {
                lines[*other].cells == lines[*first].cells && lines[*other].passes == lines[*first].passes
            })
        };
        for (endpoint, incoming) in &ends {
            let Some(outgoing) = starts.get(endpoint) else {
                continue;
            };
            if incoming.len() != outgoing.len()
                || !are_equivalent(incoming)
                || !are_equivalent(outgoing)
                || lines[incoming[0]].passes != lines[outgoing[0]].passes
            {
                continue;
            }
            for (previous, following) in incoming.iter().zip(outgoing.iter()) {
                if previous != following {
                    links.next[*previous] = Some(*following);
                    links.has_previous[*following] = true;
                }
            }
        }
        links
    }

    fn merged_runs(lines: &[MapTrackLine]) -> MapTrackHeatLinks {
        let mut links = MapTrackHeatLinks { next: vec![None; lines.len()], has_previous: vec![false; lines.len()] };
        for index in 0..lines.len().saturating_sub(1) {
            let line = &lines[index];
            let following = &lines[index + 1];
            let Some(id) = line.line_id else {
                continue;
            };
            if following.line_id != Some(id)
                || line.cells.len() <= 1
                || following.cells.len() <= 1
                || line.cells.last() != following.cells.first()
            {
                continue;
            }
            links.next[index] = Some(index + 1);
            links.has_previous[index + 1] = true;
        }
        links
    }

    /// a walk counts from the middle of its day
    /// today's is still warm at midnight and a one day cooling fades through the next morning
    fn cooled(day: i32, times: i32, cooling: f64, now_day: f64) -> f64 {
        let age = (now_day - (f64::from(day) + 0.5)).max(0.0);
        f64::from(times) * (1.0 - age / cooling).max(0.0)
    }

    fn weight(line: &MapTrackLine, cooling: f64, now_day: f64) -> f64 {
        if line.passes.is_empty() {
            return Self::cooled(line.day, 1, cooling, now_day);
        }
        line.passes.iter().fold(0.0, |sum, pass| sum + Self::cooled(pass.day, pass.times, cooling, now_day))
    }

    /// visits points along the line at most step apart
    /// each with the distance travelled so far in the same cells
    fn walk(cells: &[Point<i32>], step: f64, mut visit: impl FnMut(f64, f64, f64)) {
        let Some(first) = cells.first() else {
            return;
        };
        visit(f64::from(first.x), f64::from(first.y), 0.0);
        let mut travelled = 0.0;
        for index in 1..cells.len() {
            let from = cells[index - 1];
            let to = cells[index];
            let dx = f64::from(to.x) - f64::from(from.x);
            let dy = f64::from(to.y) - f64::from(from.y);
            let length = dx.hypot(dy);
            let pieces = ((length / step).ceil() as i64).max(1);
            for piece in 1..=pieces {
                let t = piece as f64 / pieces as f64;
                visit(f64::from(from.x) + dx * t, f64::from(from.y) + dy * t, travelled + length * t);
            }
            travelled += length;
        }
    }

    /// with a brush the field is smooth across the line so its interpolated
    /// without one the cells beside the line are empty and interpolating would halve a road at every corner
    fn sample(field: &MapHeatField, x: f64, y: f64, interpolates: bool) -> f64 {
        if !interpolates {
            return field.value(x as i64, y as i64);
        }
        let fx = (x - 0.5).max(0.0);
        let fy = (y - 0.5).max(0.0);
        let x0 = fx as i64;
        let y0 = fy as i64;
        let tx = fx - x0 as f64;
        let ty = fy - y0 as f64;
        let top = field.value(x0, y0) * (1.0 - tx) + field.value(x0 + 1, y0) * tx;
        let bottom = field.value(x0, y0 + 1) * (1.0 - tx) + field.value(x0 + 1, y0 + 1) * tx;
        top * (1.0 - ty) + bottom * ty
    }

    fn append(
        line: &MapTrackLine,
        field: &MapHeatField,
        policy: MapHeatPolicy,
        walks_to_hottest: f64,
        result: &mut Vec<MapHeatStretch>,
    ) {
        let cells_per_heat_cell = policy.cells_per_heat_cell;
        let mut edges: Vec<usize> = Vec::new();
        let mut fractions: Vec<f64> = Vec::new();
        let mut heat: Vec<f64> = Vec::new();
        let cells = &line.cells;
        let Some(first) = cells.first() else {
            return;
        };

        let mut take = |edge: usize, fraction: f64, x: f64, y: f64| {
            edges.push(edge);
            fractions.push(fraction);
            heat.push(Self::sample(field, x / cells_per_heat_cell, y / cells_per_heat_cell, policy.brush > 0));
        };
        take(0, 0.0, f64::from(first.x), f64::from(first.y));
        for index in 1..cells.len() {
            let from = cells[index - 1];
            let to = cells[index];
            let dx = f64::from(to.x) - f64::from(from.x);
            let dy = f64::from(to.y) - f64::from(from.y);
            let pieces = ((dx.hypot(dy) / cells_per_heat_cell).ceil() as i64).max(1);
            for piece in 1..=pieces {
                let t = piece as f64 / pieces as f64;
                take(index - 1, t, f64::from(from.x) + dx * t, f64::from(from.y) + dy * t);
            }
        }

        let reach = Self::SMOOTHING_REACH;
        let mut level_of = vec![0_i64; heat.len()];
        for index in 0..heat.len() {
            let low = index.saturating_sub(reach);
            let high = (heat.len() - 1).min(index + reach);
            let sum: f64 = heat[low..=high].iter().sum();
            let smoothed = sum / (high - low + 1) as f64;
            let temperature = ((smoothed - 1.0) / (walks_to_hottest - 1.0)).max(0.0).min(1.0);
            level_of[index] = (temperature * (Self::LEVELS - 1) as f64).round() as i64;
        }

        let coordinate = |index: usize| -> (f64, f64) {
            if cells.len() <= 1 {
                return (line.coordinates[0], line.coordinates[1]);
            }
            let edge = edges[index];
            let t = fractions[index];
            let lat = line.coordinates[edge * 2] + (line.coordinates[edge * 2 + 2] - line.coordinates[edge * 2]) * t;
            let lng = line.coordinates[edge * 2 + 1]
                + (line.coordinates[edge * 2 + 3] - line.coordinates[edge * 2 + 1]) * t;
            (lat, lng)
        };

        let mut emit = |start: usize, end: usize, level: i64| {
            let mut coordinates = Vec::with_capacity((end - start + 1) * 2);
            let (mut min_lat, mut max_lat, mut min_lng, mut max_lng) = (90.0_f64, -90.0_f64, 180.0_f64, -180.0_f64);
            for index in start..=end {
                if index > start && index < end && fractions[index] < 1.0 {
                    continue;
                }
                let point = coordinate(index);
                coordinates.push(point.0);
                coordinates.push(point.1);
                min_lat = min_lat.min(point.0);
                max_lat = max_lat.max(point.0);
                min_lng = min_lng.min(point.1);
                max_lng = max_lng.max(point.1);
            }
            result.push(MapHeatStretch {
                day: line.day,
                mode: line.mode,
                temperature: level as f64 / (Self::LEVELS - 1) as f64,
                coordinates,
                min_latitude: min_lat,
                max_latitude: max_lat,
                min_longitude: min_lng,
                max_longitude: max_lng,
            });
        };

        if cells.len() <= 1 {
            emit(0, 0, level_of[0]);
            return;
        }
        let mut start = 0;
        for index in 1..level_of.len() {
            if level_of[index] != level_of[start] {
                emit(start, index, level_of[start]);
                start = index;
            }
        }
        if start < level_of.len() - 1 || level_of.len() == 1 {
            emit(start, level_of.len() - 1, level_of[start]);
        }
    }
}
