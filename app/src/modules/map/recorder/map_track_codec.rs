use gpui::Point;

/// byte formats of the track layer, mirrored from the backend map-service tracks codec
/// points blob is version 1 then count then per point zigzag deltas
/// tile payload is version 2 with passes and varints, bad buffer reads as none
pub enum MapTrackCodec {}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MapTrackCodecPoint {
    pub lat_e6: i32,
    pub lng_e6: i32,
    pub time_ms: i64,
}

/// the road was walked times on day
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MapTrackCodecPass {
    pub day: i32,
    pub times: i32,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MapTrackCodecTilePart {
    pub day: i32,
    pub mode: i32,
    pub passes: Vec<MapTrackCodecPass>,
    pub vertices: Vec<Point<i32>>,
}

impl MapTrackCodec {
    pub const POINTS_VERSION: u8 = 1;
    pub const TILE_VERSION: u8 = 2;
    const TILE_VERSION_WITHOUT_PASSES: u8 = 1;

    pub fn encode_points(points: &[MapTrackCodecPoint], started_at: i64) -> Vec<u8> {
        let mut out = Vec::with_capacity(2 + points.len() * 5);
        out.push(Self::POINTS_VERSION);
        Self::put_varint(points.len() as u64, &mut out);
        let mut lat: i64 = 0;
        let mut lng: i64 = 0;
        let mut time_ms = started_at;
        for point in points {
            Self::put_varint(Self::zigzag(i64::from(point.lat_e6) - lat), &mut out);
            Self::put_varint(Self::zigzag(i64::from(point.lng_e6) - lng), &mut out);
            let seconds = point.time_ms.saturating_sub(time_ms).max(0) / 1_000;
            Self::put_varint(seconds as u64, &mut out);
            lat = i64::from(point.lat_e6);
            lng = i64::from(point.lng_e6);
            time_ms += seconds * 1_000;
        }
        out
    }

    /// reader of encode_points for segments sealed on disk and still waiting for the network
    /// the map draws them from here
    pub fn decode_points(blob: &[u8], started_at: i64, maximum_points: i64) -> Option<Vec<MapTrackCodecPoint>> {
        let mut reader = MapTrackCodecReader::new(blob);
        if reader.byte()? != Self::POINTS_VERSION {
            return None;
        }
        let count = reader.varint()?;
        if count > maximum_points.max(0) as u64 {
            return None;
        }

        let mut points = Vec::with_capacity(count as usize);
        let mut lat: i64 = 0;
        let mut lng: i64 = 0;
        let mut time_ms = started_at;
        for _ in 0..count {
            let d_lat = reader.varint()?;
            let d_lng = reader.varint()?;
            let seconds = reader.varint()?;
            if seconds > i32::MAX as u64 {
                return None;
            }
            lat = lat.wrapping_add(Self::unzigzag(d_lat));
            lng = lng.wrapping_add(Self::unzigzag(d_lng));
            time_ms += seconds as i64 * 1_000;
            let lat_e6 = i32::try_from(lat).ok()?;
            let lng_e6 = i32::try_from(lng).ok()?;
            points.push(MapTrackCodecPoint { lat_e6, lng_e6, time_ms });
        }
        reader.is_at_end().then_some(points)
    }

    pub fn decode_tile(
        payload: &[u8],
        maximum_parts: i64,
        maximum_vertices: i64,
        maximum_passes: i64,
    ) -> Option<Vec<MapTrackCodecTilePart>> {
        let mut reader = MapTrackCodecReader::new(payload);
        let version = reader.byte()?;
        if version != Self::TILE_VERSION && version != Self::TILE_VERSION_WITHOUT_PASSES {
            return None;
        }
        let part_count = reader.varint()?;
        if part_count > maximum_parts.max(0) as u64 {
            return None;
        }
        let carries_passes = version == Self::TILE_VERSION;

        let mut parts = Vec::with_capacity(part_count as usize);
        for _ in 0..part_count {
            let day = reader.varint()?;
            if day > i32::MAX as u64 {
                return None;
            }
            let mode = reader.varint()?;
            if mode > i32::MAX as u64 {
                return None;
            }
            let mut passes = Vec::new();
            if carries_passes {
                let pass_count = reader.varint()?;
                if pass_count > maximum_passes.max(0) as u64 {
                    return None;
                }
                passes.reserve(pass_count as usize);
                let mut pass_day = day as i64;
                for _ in 0..pass_count {
                    let back = reader.varint()?;
                    if back > i32::MAX as u64 {
                        return None;
                    }
                    let times = reader.varint()?;
                    if times > i32::MAX as u64 {
                        return None;
                    }
                    pass_day -= back as i64;
                    if pass_day < 0 {
                        return None;
                    }
                    passes.push(MapTrackCodecPass { day: pass_day as i32, times: (times as i32).max(1) });
                }
            }
            let count = reader.varint()?;
            if count > maximum_vertices.max(0) as u64 {
                return None;
            }
            let mut vertices = Vec::with_capacity(count as usize);
            let mut x: i64 = 0;
            let mut y: i64 = 0;
            for _ in 0..count {
                let dx = reader.varint()?;
                let dy = reader.varint()?;
                x = x.wrapping_add(Self::unzigzag(dx));
                y = y.wrapping_add(Self::unzigzag(dy));
                let cell_x = i32::try_from(x).ok()?;
                let cell_y = i32::try_from(y).ok()?;
                vertices.push(Point { x: cell_x, y: cell_y });
            }
            parts.push(MapTrackCodecTilePart { day: day as i32, mode: mode as i32, passes, vertices });
        }
        reader.is_at_end().then_some(parts)
    }

    pub fn zigzag(value: i64) -> u64 {
        ((value << 1) ^ (value >> 63)) as u64
    }

    pub fn unzigzag(value: u64) -> i64 {
        ((value >> 1) as i64) ^ -((value & 1) as i64)
    }

    pub fn put_varint(value: u64, out: &mut Vec<u8>) {
        let mut rest = value;
        while rest >= 0x80 {
            out.push((rest as u8) | 0x80);
            rest >>= 7;
        }
        out.push(rest as u8);
    }
}

struct MapTrackCodecReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> MapTrackCodecReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn is_at_end(&self) -> bool {
        self.position == self.bytes.len()
    }

    fn byte(&mut self) -> Option<u8> {
        let next = *self.bytes.get(self.position)?;
        self.position += 1;
        Some(next)
    }

    /// a varint of a u64 never needs more than ten bytes
    fn varint(&mut self) -> Option<u64> {
        let mut value: u64 = 0;
        for index in 0..10u32 {
            let next = self.byte()?;
            value |= u64::from(next & 0x7f).wrapping_shl(7 * index);
            if next & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }
}

/// web mercator and the band pyramid, the client half of tracks geo
/// bands are never compiled in, they arrive with the track summary
pub enum MapTrackGeometry {}

impl MapTrackGeometry {
    pub const MAXIMUM_MERCATOR_LATITUDE: f64 = 85.051_128_779_806_59;

    /// latitude and longitude of a global cell of a band, world_cells across
    pub fn coordinate(cell_x: f64, cell_y: f64, world_cells: f64) -> (f64, f64) {
        let x = cell_x / world_cells;
        let y = cell_y / world_cells;
        let longitude = x * 360.0 - 180.0;
        let latitude = (std::f64::consts::PI * (1.0 - 2.0 * y)).sinh().atan() * 180.0 / std::f64::consts::PI;
        (latitude, longitude)
    }

    /// normalized mercator of a coordinate, both axes in 0 to 1
    pub fn mercator(latitude: f64, longitude: f64) -> (f64, f64) {
        let lat = latitude.clamp(-Self::MAXIMUM_MERCATOR_LATITUDE, Self::MAXIMUM_MERCATOR_LATITUDE)
            * std::f64::consts::PI
            / 180.0;
        let x = (longitude + 180.0) / 360.0;
        let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0;
        (x.max(0.0).min(1.0 - f64::EPSILON), y.max(0.0).min(1.0 - f64::EPSILON))
    }

    /// owner's local day number of an instant, the unit of every period
    pub fn local_day(time_ms: i64, utc_offset_minutes: i64) -> i32 {
        let shifted = time_ms.wrapping_add(utc_offset_minutes.wrapping_mul(60_000));
        let day: i64 = 86_400_000;
        let floored = if shifted >= 0 { shifted / day } else { (shifted - (day - 1)) / day };
        floored as i32
    }

    pub fn haversine_meters(a_latitude: f64, a_longitude: f64, b_latitude: f64, b_longitude: f64) -> f64 {
        let lat1 = a_latitude * std::f64::consts::PI / 180.0;
        let lat2 = b_latitude * std::f64::consts::PI / 180.0;
        let d_lat = lat2 - lat1;
        let d_lng = (b_longitude - a_longitude) * std::f64::consts::PI / 180.0;
        let h = (d_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (d_lng / 2.0).sin().powi(2);
        2.0 * 6_371_008.8 * h.sqrt().min(1.0).asin()
    }
}
