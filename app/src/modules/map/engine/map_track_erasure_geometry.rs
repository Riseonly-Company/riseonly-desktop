use rise_canon::rise_key::map as key;
use rise_canon::{RiseLimits, rise_decode};
use rise_media::map_view::LatLng;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::map::recorder::map_track_codec::MapTrackGeometry;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapErasePoint {
    pub x: f64,
    pub y: f64,
}

impl MapErasePoint {
    pub fn new(coordinate: LatLng) -> Self {
        let longitude = (coordinate.longitude + 180.0) / 360.0;
        let x = longitude - longitude.floor();
        let latitude = MapTrackGeometry::MAXIMUM_MERCATOR_LATITUDE
            .min((-MapTrackGeometry::MAXIMUM_MERCATOR_LATITUDE).max(coordinate.latitude))
            * std::f64::consts::PI
            / 180.0;
        let y = 1.0_f64.min(0.0_f64.max((1.0 - (latitude.tan() + 1.0 / latitude.cos()).ln() / std::f64::consts::PI) / 2.0));
        Self { x, y }
    }
}

impl<'de> Deserialize<'de> for MapErasePoint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self { x: container.rise_required::<f64>(key::X)?, y: container.rise_required::<f64>(key::Y)? })
        })
    }
}

impl Serialize for MapErasePoint {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(key::X, &self.x)?;
        container.serialize_entry(key::Y, &self.y)?;
        container.end()
    }
}

/// point of the erasure plane, normalized mercator with x unwrapped across the antimeridian
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapErasureMaskPoint {
    pub x: f64,
    pub y: f64,
}

/// rect of the erasure plane kept as edges
/// always built from a minimum and a span so never negative
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapErasureMaskRect {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl MapErasureMaskRect {
    fn union(self, other: Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    fn width(&self) -> f64 {
        self.max_x - self.min_x
    }

    fn height(&self) -> f64 {
        self.max_y - self.min_y
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct MapEraseStroke {
    pub id: String,
    pub since_day: i32,
    pub until_ms: i64,
    pub radius: f64,
    pub points: Vec<MapErasePoint>,
}

impl MapEraseStroke {
    pub fn maximum_strokes() -> i64 {
        RiseLimits::service_value("map_eraser_max_strokes")
    }

    pub fn maximum_points() -> i64 {
        RiseLimits::service_value("map_eraser_max_points")
    }

    pub fn maximum_radius() -> f64 {
        RiseLimits::service_value("map_eraser_max_radius_ppm") as f64 / 1_000_000.0
    }

    fn distance_squared(p: MapErasureMaskPoint, a: MapErasureMaskPoint, b: MapErasureMaskPoint) -> f64 {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let length = dx * dx + dy * dy;
        let t = if length > 0.0 { 1.0_f64.min(0.0_f64.max(((p.x - a.x) * dx + (p.y - a.y) * dy) / length)) } else { 0.0 };
        (p.x - a.x - t * dx).powi(2) + (p.y - a.y - t * dy).powi(2)
    }

    fn intersects(
        a: MapErasureMaskPoint,
        b: MapErasureMaskPoint,
        c: MapErasureMaskPoint,
        d: MapErasureMaskPoint,
        radius: f64,
    ) -> bool {
        if !(a.x.min(b.x) <= c.x.max(d.x) + radius
            && a.x.max(b.x) >= c.x.min(d.x) - radius
            && a.y.min(b.y) <= c.y.max(d.y) + radius
            && a.y.max(b.y) >= c.y.min(d.y) - radius)
        {
            return false;
        }
        let cross = |p: MapErasureMaskPoint, q: MapErasureMaskPoint, r: MapErasureMaskPoint| {
            (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x)
        };
        let ab_c = cross(a, b, c);
        let ab_d = cross(a, b, d);
        let cd_a = cross(c, d, a);
        let cd_b = cross(c, d, b);
        if ab_c * ab_d < 0.0 && cd_a * cd_b < 0.0 {
            return true;
        }
        Self::distance_squared(a, c, d)
            .min(Self::distance_squared(b, c, d))
            .min(Self::distance_squared(c, a, b))
            .min(Self::distance_squared(d, a, b))
            <= radius * radius
    }
}

impl<'de> Deserialize<'de> for MapEraseStroke {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                id: container.rise_required::<String>(key::ID)?,
                radius: container.rise_required::<f64>(key::RADIUS)?,
                points: container.rise_required::<Vec<MapErasePoint>>(key::POINTS)?,
                since_day: container.rise_required::<i32>(key::SINCE_DAY)?,
                until_ms: container.rise_required::<i64>(key::UNTIL_MS)?,
            })
        })
    }
}

impl Serialize for MapEraseStroke {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(key::ID, &self.id)?;
        container.serialize_entry(key::RADIUS, &self.radius)?;
        container.serialize_entry(key::POINTS, &self.points)?;
        container.serialize_entry(key::SINCE_DAY, &self.since_day)?;
        container.serialize_entry(key::UNTIL_MS, &self.until_ms)?;
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MapErasureMaskPreparedStroke {
    pub stroke: MapEraseStroke,
    pub root: Option<MapErasureMaskNode>,
}

impl MapErasureMaskPreparedStroke {
    pub fn new(stroke: MapEraseStroke) -> Self {
        let Some(first) = stroke.points.first() else {
            return Self { stroke, root: None };
        };
        let mut previous = MapErasureMaskPoint { x: first.x, y: first.y };
        let points = if stroke.points.len() == 1 { &stroke.points[..] } else { &stroke.points[1..] };
        let mut capsules = Vec::with_capacity(points.len());
        for point in points {
            let next = MapErasureMaskPoint { x: point.x - (point.x - previous.x).round(), y: point.y };
            capsules.push(MapErasureMaskCapsule { a: previous, b: next, radius: stroke.radius });
            previous = next;
        }
        let root = MapErasureMaskNode::build(capsules);
        Self { stroke, root }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapErasureMaskCapsule {
    pub a: MapErasureMaskPoint,
    pub b: MapErasureMaskPoint,
    pub radius: f64,
}

impl MapErasureMaskCapsule {
    pub fn bounds(&self) -> MapErasureMaskRect {
        MapErasureMaskRect {
            min_x: self.a.x.min(self.b.x) - self.radius,
            min_y: self.a.y.min(self.b.y) - self.radius,
            max_x: self.a.x.max(self.b.x) + self.radius,
            max_y: self.a.y.max(self.b.y) + self.radius,
        }
    }
}

#[derive(Clone, Debug)]
pub enum MapErasureMaskNode {
    Leaf(MapErasureMaskRect, Vec<MapErasureMaskCapsule>),
    Branch(MapErasureMaskRect, Box<MapErasureMaskNode>, Box<MapErasureMaskNode>),
}

impl MapErasureMaskNode {
    pub fn bounds(&self) -> MapErasureMaskRect {
        match self {
            MapErasureMaskNode::Leaf(bounds, _) | MapErasureMaskNode::Branch(bounds, _, _) => *bounds,
        }
    }

    pub fn build(capsules: Vec<MapErasureMaskCapsule>) -> Option<MapErasureMaskNode> {
        let first = capsules.first()?;
        let bounds = capsules[1..].iter().fold(first.bounds(), |union, capsule| union.union(capsule.bounds()));
        if capsules.len() <= 8 {
            return Some(MapErasureMaskNode::Leaf(bounds, capsules));
        }
        let horizontal = bounds.width() > bounds.height();
        let mut sorted = capsules;
        sorted.sort_by(|left, right| {
            if horizontal {
                (left.a.x + left.b.x).total_cmp(&(right.a.x + right.b.x))
            } else {
                (left.a.y + left.b.y).total_cmp(&(right.a.y + right.b.y))
            }
        });
        let middle = sorted.len() / 2;
        let right_half = sorted.split_off(middle);
        let left = Self::build(sorted)?;
        let right = Self::build(right_half)?;
        Some(MapErasureMaskNode::Branch(bounds, Box::new(left), Box::new(right)))
    }

    pub fn touches(&self, a: MapErasureMaskPoint, b: MapErasureMaskPoint, edge: MapErasureMaskRect) -> bool {
        let bounds = self.bounds();
        if !(bounds.min_x <= edge.max_x
            && bounds.max_x >= edge.min_x
            && bounds.min_y <= edge.max_y
            && bounds.max_y >= edge.min_y)
        {
            return false;
        }
        match self {
            MapErasureMaskNode::Branch(_, left, right) => left.touches(a, b, edge) || right.touches(a, b, edge),
            MapErasureMaskNode::Leaf(_, capsules) => capsules
                .iter()
                .any(|capsule| MapEraseStroke::intersects(a, b, capsule.a, capsule.b, capsule.radius)),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MapErasureMask {
    pub strokes: Vec<MapErasureMaskPreparedStroke>,
}

impl MapErasureMask {
    pub fn new(strokes: Vec<MapErasureMaskPreparedStroke>) -> Self {
        Self { strokes }
    }

    pub fn touches(&self, a: MapErasePoint, b: MapErasePoint, day: i32) -> bool {
        let x = b.x - (b.x - a.x).round();
        for stroke in self.strokes.iter().filter(|stroke| day >= stroke.stroke.since_day) {
            let Some(root) = &stroke.root else {
                continue;
            };
            for offset in [-1.0, 0.0, 1.0] {
                let start = MapErasureMaskPoint { x: a.x + offset, y: a.y };
                let end = MapErasureMaskPoint { x: x + offset, y: b.y };
                let edge = MapErasureMaskRect {
                    min_x: start.x.min(end.x),
                    min_y: start.y.min(end.y),
                    max_x: start.x.max(end.x),
                    max_y: start.y.max(end.y),
                };
                if root.touches(start, end, edge) {
                    return true;
                }
            }
        }
        false
    }

    pub fn clip(&self, coordinates: &[LatLng], day: i32) -> Vec<Vec<LatLng>> {
        let Some(first) = coordinates.first() else {
            return vec![coordinates.to_vec()];
        };
        if self.strokes.is_empty() {
            return vec![coordinates.to_vec()];
        }
        let mut previous = MapErasePoint::new(*first);
        if coordinates.len() == 1 {
            return if self.touches(previous, previous, day) { Vec::new() } else { vec![coordinates.to_vec()] };
        }
        let mut output = Vec::new();
        let mut line: Vec<LatLng> = Vec::new();
        for index in 1..coordinates.len() {
            let next = MapErasePoint::new(coordinates[index]);
            if self.touches(previous, next, day) {
                if line.len() >= 2 {
                    output.push(std::mem::take(&mut line));
                } else {
                    line.clear();
                }
            } else {
                if line.is_empty() {
                    line.push(coordinates[index - 1]);
                }
                line.push(coordinates[index]);
            }
            previous = next;
        }
        if line.len() >= 2 {
            output.push(line);
        }
        output
    }
}
