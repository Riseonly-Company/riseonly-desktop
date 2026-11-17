use rise_media::map_view::ScreenPoint;

use crate::modules::map::engine::rise_map_journeys_repository::MapJourneyPoint;

const LEAST_NONZERO_MAGNITUDE: f64 = f64::from_bits(1);

pub enum MapJourneyGeometry {}

impl MapJourneyGeometry {
    /// closes the stroke in screen space before converting to coordinates
    /// a short overshoot at the seam is clipped to its intersection and real concavities stay
    pub fn closed_stroke(input: &[ScreenPoint], tolerance: f64, maximum_vertices: usize) -> Option<Vec<ScreenPoint>> {
        if input.len() < 3
            || maximum_vertices < 3
            || tolerance <= 0.0
            || !input.iter().all(|point| point.x.is_finite() && point.y.is_finite())
        {
            return None;
        }
        let mut points: Vec<ScreenPoint> = Vec::new();
        for &point in input {
            if points.last().is_none_or(|last| Self::distance(*last, point) > 0.5) {
                points.push(point);
            }
        }
        if points.len() < 3 {
            return None;
        }
        let mut lengths: Vec<f64> = vec![0.0];
        for i in 1..points.len() {
            lengths.push(lengths[i - 1] + Self::distance(points[i - 1], points[i]));
        }
        let length = *lengths.last()?;
        if length < tolerance * 4.0 {
            return None;
        }
        let head_length = (length / 4.0).min(tolerance * 2.0);
        let mut ring: Option<Vec<ScreenPoint>> = None;
        let mut best_area = 0.0_f64;
        let mut consider = |candidate: Vec<ScreenPoint>| {
            let area = Self::signed_area(&candidate).abs();
            if area > best_area {
                best_area = area;
                ring = Some(candidate);
            }
        };
        let count = points.len();
        for i in 0..(count - 2) {
            if lengths[i] > head_length {
                continue;
            }
            for j in (i + 2)..(count - 1) {
                if lengths[j + 1] < length * 0.75 {
                    continue;
                }
                if let Some(seam) = Self::intersection(points[i], points[i + 1], points[j], points[j + 1])
                    && Self::distance(seam, points[0]) <= tolerance * 2.0
                {
                    let mut candidate = vec![seam];
                    candidate.extend_from_slice(&points[(i + 1)..=j]);
                    consider(candidate);
                }
                let nearby = Self::projection(points[i], points[j], points[j + 1]);
                if Self::distance(nearby, points[i]) <= tolerance {
                    let mut candidate = vec![points[i]];
                    candidate.extend_from_slice(&points[(i + 1)..=j]);
                    candidate.push(nearby);
                    consider(candidate);
                }
            }
            let seam = Self::projection(points[count - 1], points[i], points[i + 1]);
            if Self::distance(seam, points[count - 1]) <= tolerance {
                let mut candidate = vec![seam];
                candidate.extend_from_slice(&points[(i + 1)..(count - 1)]);
                consider(candidate);
            }
        }
        let mut ring = ring?;
        if best_area < tolerance * tolerance / 4.0 {
            return None;
        }
        if let Some(split) = Self::farthest_from_start(&ring)
            && split > 0
        {
            let epsilon = tolerance / 16.0;
            let first = Self::simplify(&ring[0..=split], epsilon);
            let mut second_input = ring[split..].to_vec();
            second_input.push(ring[0]);
            let second = Self::simplify(&second_input, epsilon);
            let mut joined = first[..first.len() - 1].to_vec();
            joined.extend_from_slice(&second[..second.len() - 1]);
            ring = joined;
        }
        if ring.len() < 3 {
            return None;
        }
        if ring.len() > maximum_vertices {
            let mut closed = ring.clone();
            closed.push(ring[0]);
            let perimeter = closed.windows(2).fold(0.0, |sum, pair| sum + Self::distance(pair[0], pair[1]));
            let mut sampled: Vec<ScreenPoint> = Vec::new();
            let mut index = 0;
            let mut offset = 0.0;
            for n in 0..maximum_vertices {
                let target = perimeter * n as f64 / maximum_vertices as f64;
                while index + 1 < closed.len() - 1 && offset + Self::distance(closed[index], closed[index + 1]) < target {
                    offset += Self::distance(closed[index], closed[index + 1]);
                    index += 1;
                }
                let a = closed[index];
                let b = closed[index + 1];
                let t = (target - offset) / Self::distance(a, b).max(LEAST_NONZERO_MAGNITUDE);
                sampled.push(ScreenPoint::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t));
            }
            ring = sampled;
        }
        Some(ring)
    }

    fn farthest_from_start(ring: &[ScreenPoint]) -> Option<usize> {
        let start = *ring.first()?;
        let mut best = 0;
        for index in 1..ring.len() {
            if Self::distance(ring[best], start) < Self::distance(ring[index], start) {
                best = index;
            }
        }
        Some(best)
    }

    fn distance(a: ScreenPoint, b: ScreenPoint) -> f64 {
        (a.x - b.x).hypot(a.y - b.y)
    }

    fn signed_area(points: &[ScreenPoint]) -> f64 {
        let Some(origin) = points.first() else {
            return 0.0;
        };
        (0..points.len()).fold(0.0, |result, i| {
            let a = points[i];
            let b = points[(i + 1) % points.len()];
            result + (a.x - origin.x) * (b.y - origin.y) - (a.y - origin.y) * (b.x - origin.x)
        }) / 2.0
    }

    fn projection(p: ScreenPoint, a: ScreenPoint, b: ScreenPoint) -> ScreenPoint {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let t = 0.0_f64.max(
            1.0_f64.min(((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy).max(LEAST_NONZERO_MAGNITUDE)),
        );
        ScreenPoint::new(a.x + dx * t, a.y + dy * t)
    }

    fn intersection(a: ScreenPoint, b: ScreenPoint, c: ScreenPoint, d: ScreenPoint) -> Option<ScreenPoint> {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let ex = d.x - c.x;
        let ey = d.y - c.y;
        let denominator = dx * ey - dy * ex;
        if denominator.abs() <= 1e-8 {
            return None;
        }
        let t = ((c.x - a.x) * ey - (c.y - a.y) * ex) / denominator;
        let u = ((c.x - a.x) * dy - (c.y - a.y) * dx) / denominator;
        if !(0.0..=1.0).contains(&t) || !(0.0..=1.0).contains(&u) {
            return None;
        }
        Some(ScreenPoint::new(a.x + dx * t, a.y + dy * t))
    }

    fn simplify(points: &[ScreenPoint], tolerance: f64) -> Vec<ScreenPoint> {
        if points.len() <= 2 {
            return points.to_vec();
        }
        let a = points[0];
        let b = points[points.len() - 1];
        let mut farthest = 0;
        let mut maximum = 0.0_f64;
        for i in 1..(points.len() - 1) {
            let delta = Self::distance(points[i], Self::projection(points[i], a, b));
            if delta > maximum {
                maximum = delta;
                farthest = i;
            }
        }
        if maximum <= tolerance {
            return vec![a, b];
        }
        let head = Self::simplify(&points[..=farthest], tolerance);
        let mut result = head[..head.len() - 1].to_vec();
        result.extend(Self::simplify(&points[farthest..], tolerance));
        result
    }

    pub fn contains(p: &MapJourneyPoint, polygon: &[MapJourneyPoint]) -> bool {
        let Some(anchor) = polygon.first() else {
            return false;
        };
        let x = |lng: f64| anchor.lng + (lng - anchor.lng + 540.0) % 360.0 - 180.0;
        let mut inside = false;
        for i in 0..polygon.len() {
            let a = &polygon[i];
            let b = &polygon[(i + 1) % polygon.len()];
            if (a.lat > p.lat) != (b.lat > p.lat)
                && x(p.lng) < (x(b.lng) - x(a.lng)) * (p.lat - a.lat) / (b.lat - a.lat) + x(a.lng)
            {
                inside = !inside;
            }
        }
        inside
    }

    pub fn smooth(points: &[MapJourneyPoint]) -> Option<Vec<MapJourneyPoint>> {
        if points.len() < 3 {
            return None;
        }
        let mut result: Vec<MapJourneyPoint> = Vec::new();
        for i in 0..points.len() {
            let a = &points[i];
            let b = &points[(i + 1) % points.len()];
            let delta = (b.lng - a.lng + 540.0) % 360.0 - 180.0;
            for t in [0.25, 0.75] {
                let lng = (a.lng + delta * t + 540.0) % 360.0 - 180.0;
                result.push(MapJourneyPoint { lat: a.lat + (b.lat - a.lat) * t, lng });
            }
        }
        let anchor = result[0].lng;
        let ring: Vec<MapJourneyPoint> = result
            .iter()
            .map(|point| MapJourneyPoint { lat: point.lat, lng: anchor + (point.lng - anchor + 540.0) % 360.0 - 180.0 })
            .collect();
        let epsilon = 1e-10;
        let max_lng = ring.iter().map(|point| point.lng).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
        let min_lng = ring.iter().map(|point| point.lng).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.min(v))));
        if !ring.iter().all(|point| point.lat.is_finite() && point.lng.is_finite() && point.lat.abs() <= 85.0)
            || max_lng.unwrap_or(0.0) - min_lng.unwrap_or(0.0) >= 180.0
        {
            return None;
        }
        let cross = |a: &MapJourneyPoint, b: &MapJourneyPoint, c: &MapJourneyPoint| -> f64 {
            (b.lng - a.lng) * (c.lat - a.lat) - (b.lat - a.lat) * (c.lng - a.lng)
        };
        let on_edge = |p: &MapJourneyPoint, a: &MapJourneyPoint, b: &MapJourneyPoint| -> bool {
            cross(a, b, p).abs() <= epsilon * (b.lng - a.lng).hypot(b.lat - a.lat)
                && p.lng >= a.lng.min(b.lng) - epsilon
                && p.lng <= a.lng.max(b.lng) + epsilon
                && p.lat >= a.lat.min(b.lat) - epsilon
                && p.lat <= a.lat.max(b.lat) + epsilon
        };
        let mut area = 0.0;
        let count = ring.len();
        for i in 0..count {
            let a = &ring[i];
            let b = &ring[(i + 1) % count];
            if (a.lat - b.lat).abs() + (a.lng - b.lng).abs() <= epsilon {
                return None;
            }
            area += cross(&ring[0], a, b);
            for j in (i + 1)..count {
                if j == i + 1 || (i == 0 && j == count - 1) {
                    continue;
                }
                let c = &ring[j];
                let d = &ring[(j + 1) % count];
                if (cross(a, b, c) * cross(a, b, d) < 0.0 && cross(c, d, a) * cross(c, d, b) < 0.0)
                    || on_edge(c, a, b)
                    || on_edge(d, a, b)
                    || on_edge(a, c, d)
                    || on_edge(b, c, d)
                {
                    return None;
                }
            }
        }
        if area.abs() > epsilon * epsilon { Some(result) } else { None }
    }
}
