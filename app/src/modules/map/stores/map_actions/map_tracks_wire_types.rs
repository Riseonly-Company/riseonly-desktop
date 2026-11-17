use std::collections::BTreeMap;
use std::sync::LazyLock;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use gpui::{Point, point};
use rise_canon::{RiseLimits, RiseRequestJSON, rise_key, rise_proto, rise_request, rise_view};
use rise_catalog::{MapPlaceSource, MapTrackRejection};

use super::map_tracks_types::{
    MapPlace, MapPlaceEmoji, MapPlaceKind, MapPlacePolicy, MapTileKey, MapTrackBand, MapTrackPeriod,
    MapTrackPeriodStats, MapTrackSummary,
};
use super::map_types::{MapMergePreview, MapMergePreviewRequest, MapMergePreviewStroke, MapSettings};
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::map::engine::map_track_erasure_geometry::MapEraseStroke;
use crate::modules::map::recorder::map_recorder_journal::{MapRecordedStay, MapSealedSegment};
use crate::modules::map::recorder::map_track_codec::{MapTrackCodec, MapTrackCodecTilePart};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_wire_types::{
    StickerWireDTO, StickerWireDTOExt,
};

impl MapSealedSegment {
    pub fn canon_json(&self) -> RiseRequestJSON {
        RiseRequestJSON::Object(BTreeMap::from([
            (rise_key::map::ID.to_owned(), RiseRequestJSON::String(self.id.clone())),
            (rise_key::map::STARTED_AT.to_owned(), RiseRequestJSON::Integer(self.started_at)),
            (rise_key::map::UTC_OFFSET_MIN.to_owned(), RiseRequestJSON::Integer(self.utc_offset_minutes)),
            (rise_key::map::MODE.to_owned(), RiseRequestJSON::String(self.mode.clone())),
            (rise_key::map::POINTS.to_owned(), RiseRequestJSON::String(STANDARD.encode(&self.points))),
        ]))
    }
}

impl MapRecordedStay {
    pub fn canon_json(&self) -> RiseRequestJSON {
        RiseRequestJSON::Object(BTreeMap::from([
            (rise_key::map::ARRIVED_AT.to_owned(), RiseRequestJSON::Integer(self.arrived_at)),
            (rise_key::map::DEPARTED_AT.to_owned(), RiseRequestJSON::Integer(self.departed_at)),
            (rise_key::map::LAT.to_owned(), RiseRequestJSON::Number(self.latitude)),
            (rise_key::map::LNG.to_owned(), RiseRequestJSON::Number(self.longitude)),
            (rise_key::map::UTC_OFFSET_MIN.to_owned(), RiseRequestJSON::Integer(self.utc_offset_minutes)),
        ]))
    }
}

impl MapTileKey {
    pub fn canon_json(&self) -> RiseRequestJSON {
        RiseRequestJSON::Object(BTreeMap::from([
            (rise_key::map::BAND.to_owned(), RiseRequestJSON::Integer(self.band)),
            (rise_key::map::X.to_owned(), RiseRequestJSON::Integer(self.x)),
            (rise_key::map::Y.to_owned(), RiseRequestJSON::Integer(self.y)),
        ]))
    }
}

pub trait StrokesElementExt {
    fn from_stroke(stroke: &MapEraseStroke) -> Self;
}

impl StrokesElementExt for rise_request::map::erase_tracks::StrokesElement {
    fn from_stroke(stroke: &MapEraseStroke) -> Self {
        Self {
            id: stroke.id.clone(),
            points: stroke
                .points
                .iter()
                .map(|point| rise_request::map::erase_tracks::strokes_element::PointsElement {
                    x: Some(point.x),
                    y: Some(point.y),
                })
                .collect(),
            radius: Some(stroke.radius),
            since_day: i64::from(stroke.since_day),
            until_ms: stroke.until_ms,
        }
    }
}

pub trait PreviewTrackMergeExt {
    fn from_request(request: &MapMergePreviewRequest) -> Self;
}

impl PreviewTrackMergeExt for rise_request::map::PreviewTrackMerge {
    fn from_request(request: &MapMergePreviewRequest) -> Self {
        Self {
            lanes: Some(MapSettings::clamped_merge_lanes(request.lanes)),
            lat_e6: request.centre.map(|centre| i64::from((centre.latitude * 1_000_000.0).round() as i32)),
            lng_e6: request.centre.map(|centre| i64::from((centre.longitude * 1_000_000.0).round() as i32)),
            radius_m: Some(MapSettings::clamped_merge_radius(request.radius_m)),
            source: Some(request.source.raw_value().to_owned()),
        }
    }
}

pub trait ProtoRejectedSegmentExt {
    fn is_final(&self) -> bool;
}

impl ProtoRejectedSegmentExt for rise_proto::map::ProtoRejectedSegment {
    fn is_final(&self) -> bool {
        self.reason != MapTrackRejection::Deferred.wire() && self.reason != MapTrackRejection::RecordingOff.wire()
    }
}

pub trait TrackTileExt {
    fn key(&self) -> MapTileKey;
    fn day(&self) -> i32;
    fn lines(&self) -> Vec<u32>;
}

impl TrackTileExt for rise_view::map::TrackTile {
    fn key(&self) -> MapTileKey {
        MapTileKey { band: self.band, x: self.x, y: self.y }
    }

    fn day(&self) -> i32 {
        self.since_day as i32
    }

    fn lines(&self) -> Vec<u32> {
        self.part_lines.iter().map(|line| *line as u32).collect()
    }
}

pub trait TrackPeriodExt {
    fn model(&self) -> Option<MapTrackPeriodStats>;
}

impl TrackPeriodExt for rise_view::map::TrackPeriod {
    fn model(&self) -> Option<MapTrackPeriodStats> {
        let period = MapTrackPeriod::from_wire(&self.period)?;
        Some(MapTrackPeriodStats {
            period,
            since_day: self.since_day as i32,
            distance_meters: self.distance_m.max(0),
            duration_seconds: self.duration_s.max(0),
            active_days: self.active_days.max(0),
            walk_run_distance_meters: Some(self.walk_run_distance_m.max(0)),
        })
    }
}

pub trait ProtoTrackBandExt {
    fn model(&self) -> Option<MapTrackBand>;
}

impl ProtoTrackBandExt for rise_proto::map::ProtoTrackBand {
    fn model(&self) -> Option<MapTrackBand> {
        if self.band < 0
            || !(0..=24).contains(&self.index_zoom)
            || self.min_zoom > self.max_zoom
            || !(1..=24).contains(&self.extent_bits)
            || self.index_zoom + self.extent_bits > 48
        {
            return None;
        }
        Some(MapTrackBand {
            band: self.band,
            index_zoom: self.index_zoom,
            min_zoom: self.min_zoom,
            max_zoom: self.max_zoom,
            extent_bits: self.extent_bits,
        })
    }
}

pub trait TrackSummaryExt {
    fn model(&self) -> MapTrackSummary;
}

impl TrackSummaryExt for rise_view::map::TrackSummary {
    fn model(&self) -> MapTrackSummary {
        let mut bands: Vec<MapTrackBand> = self.bands.iter().filter_map(ProtoTrackBandExt::model).collect();
        bands.sort_by(|a, b| a.band.cmp(&b.band));
        MapTrackSummary {
            revision: self.revision,
            periods: self.periods.iter().filter_map(TrackPeriodExt::model).collect(),
            bands,
            first_day: self.first_day as i32,
            last_day: self.last_day as i32,
        }
    }
}

pub trait PlaceExt {
    fn model(&self) -> Option<MapPlace>;
}

impl PlaceExt for rise_view::map::Place {
    fn model(&self) -> Option<MapPlace> {
        let kind = MapPlaceKind::from_raw_value(&self.kind)?;
        if self.id.is_empty()
            || !self.lat.is_finite()
            || !self.lng.is_finite()
            || self.lat.abs() > 90.0
            || self.lng.abs() > 180.0
        {
            return None;
        }
        Some(MapPlace {
            id: self.id.clone(),
            kind,
            is_inferred: self.source == MapPlaceSource::Inferred.wire(),
            name: self.name.clone(),
            latitude: self.lat,
            longitude: self.lng,
            radius_meters: self.radius_m.max(0),
            description: self.description.clone(),
            emoji: if self.emoji.is_empty() { "\u{1F4CD}".to_owned() } else { self.emoji.clone() },
            custom_emoji: place_sticker(self).map(|sticker| MapPlaceEmoji::new(&sticker.to_sticker())),
            record_tracks: self.record_tracks,
            track_exclusion_radius_meters: if self.track_exclusion_radius_m > 0 {
                MapPlacePolicy::radius(self.track_exclusion_radius_m)
            } else {
                MapPlacePolicy::default_radius()
            },
        })
    }
}

fn place_sticker(place: &rise_view::map::Place) -> Option<StickerWireDTO> {
    let custom_emoji = place.custom_emoji.as_ref()?;
    if custom_emoji.is_null() {
        return None;
    }
    serde_json::from_value::<StickerWireDTO>(custom_emoji.clone()).ok()
}

static MAXIMUM_PARTS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("map_max_detail_parts_per_tile"));
static MAXIMUM_VERTICES: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_max_segment_points"));
static MAXIMUM_PASSES: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("map_max_heat_passes_per_line"));

pub trait TrackMergeExt {
    fn model(&self) -> Option<MapMergePreview>;
}

impl TrackMergeExt for rise_view::map::TrackMerge {
    fn model(&self) -> Option<MapMergePreview> {
        if self.extent <= 0 || self.cell_m <= 0.0 {
            return None;
        }
        let walk_parts = track_merge_parts(self.walks.as_deref().unwrap_or_default())?;
        let line_parts = track_merge_parts(self.lines.as_deref().unwrap_or_default())?;
        let scale = 1.0 / self.extent as f64;
        let stroke = |part: &MapTrackCodecTilePart, line_id: Option<u32>, passes: i64| MapMergePreviewStroke {
            points: part
                .vertices
                .iter()
                .map(|vertex| -> Point<f64> { point(f64::from(vertex.x) * scale, f64::from(vertex.y) * scale) })
                .collect(),
            line_id,
            passes,
        };
        let part_ids: Vec<u32> = self.part_lines.iter().map(|line| *line as u32).collect();
        let line_ids: Vec<u32> = if part_ids.len() == line_parts.len() { part_ids } else { Vec::new() };
        Some(MapMergePreview {
            walks: walk_parts.iter().map(|part| stroke(part, None, 1)).collect(),
            lines: line_parts
                .iter()
                .enumerate()
                .map(|(index, part)| {
                    stroke(
                        part,
                        if line_ids.is_empty() { None } else { Some(line_ids[index]) },
                        part.passes.iter().fold(0_i64, |sum, pass| sum + i64::from(pass.times)),
                    )
                })
                .collect(),
            radius_m: self.radius_m,
            lanes: self.lanes,
            window_meters: self.extent as f64 * self.cell_m,
            truncated: self.truncated,
            walk_count: self.walk_count,
            line_count: self.line_count,
        })
    }
}

fn track_merge_parts(encoded: &str) -> Option<Vec<MapTrackCodecTilePart>> {
    let payload = STANDARD.decode(encoded).ok()?;
    MapTrackCodec::decode_tile(&payload, *MAXIMUM_PARTS, *MAXIMUM_VERTICES, *MAXIMUM_PASSES)
}
