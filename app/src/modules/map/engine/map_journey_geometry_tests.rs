use rise_core::foundation::foundation_url::{UrlComponents, UrlQueryItem};
use rise_media::map_view::ScreenPoint;

use crate::modules::map::engine::core::rise_map_photo_upload::MapPhotoURL;
use crate::modules::map::engine::map_journey_geometry::MapJourneyGeometry;
use crate::modules::map::engine::rise_map_journeys_repository::MapJourneyPoint;

type MapJourneyGeometryTests = MapJourneyGeometry;
/// subject of the map journey photo url tests
type MapJourneyPhotoURLTests = MapPhotoURL;

fn p(x: f64, y: f64) -> MapJourneyPoint {
    MapJourneyPoint { lat: y, lng: x }
}

fn stroke(points: &[(f64, f64)]) -> Vec<ScreenPoint> {
    points.iter().map(|&(x, y)| ScreenPoint::new(x, y)).collect()
}

#[test]
fn closes_finger_gaps_and_overshoots_without_keeping_the_seam_loop() {
    let cases: Vec<Vec<(f64, f64)>> = vec![
        vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, 0.0)],
        vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (4.0, 15.0)],
        vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (8.0, -70.0)],
        vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (-5.0, -70.0)],
        vec![
            (0.0, 0.0),
            (25.0, -1.0),
            (100.0, 0.0),
            (100.0, 100.0),
            (0.0, 100.0),
            (-1.0, 20.0),
            (2.0, -3.0),
            (7.0, -1.0),
            (2.0, 3.0),
            (-2.0, 1.0),
        ],
    ];
    for case in cases {
        let ring = MapJourneyGeometryTests::closed_stroke(&stroke(&case), 32.0, 64).expect("ring");
        let smoothed = MapJourneyGeometryTests::smooth(
            &ring
                .iter()
                .map(|point| MapJourneyPoint { lat: 51.0 + point.y * 1e-6, lng: 71.0 + point.x * 1e-6 })
                .collect::<Vec<_>>(),
        );
        assert!(smoothed.is_some());
        assert!(ring.len() <= 64);
    }
}

#[test]
fn open_stroke_and_large_crossing_still_fail() {
    let strokes = [
        vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)],
        vec![(0.0, 0.0), (100.0, 100.0), (0.0, 100.0), (100.0, 0.0), (0.0, 0.0)],
    ];
    for case in strokes {
        assert!(MapJourneyGeometryTests::closed_stroke(&stroke(&case), 32.0, 64).is_none());
    }
}

#[test]
fn smoothing_preserves_a_closed_area_within_the_vertex_budget() {
    let ring = MapJourneyGeometryTests::smooth(&[p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0)]).expect("ring");
    assert_eq!(ring.len(), 8);
    assert!(MapJourneyGeometryTests::contains(&p(1.0, 1.0), &ring));
    assert!(!MapJourneyGeometryTests::contains(&p(3.0, 1.0), &ring));
}

#[test]
fn rejects_crossed_and_degenerate_outlines() {
    assert!(MapJourneyGeometryTests::smooth(&[p(0.0, 0.0), p(2.0, 2.0), p(0.0, 2.0), p(2.0, 0.0)]).is_none());
    assert!(MapJourneyGeometryTests::smooth(&[p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)]).is_none());
    assert!(MapJourneyGeometryTests::smooth(&[p(0.0, 0.0), p(f64::NAN, 2.0), p(2.0, 0.0)]).is_none());
}

#[test]
fn handles_both_sides_of_the_antimeridian() {
    let ring =
        MapJourneyGeometryTests::smooth(&[p(179.0, 0.0), p(-179.0, 0.0), p(-179.0, 2.0), p(179.0, 2.0)]).expect("ring");
    assert!(MapJourneyGeometryTests::contains(&p(179.9, 1.0), &ring));
    assert!(MapJourneyGeometryTests::contains(&p(-179.9, 1.0), &ring));
    assert!(!MapJourneyGeometryTests::contains(&p(0.0, 1.0), &ring));
}

#[test]
fn accepts_only_the_explicit_post_scope_on_map_photos() {
    let path = "/api/file/secure/map_photo/36009762-9a62-4498-a6c4-cb849a95a42f/photo.jpg";
    let raw = MapJourneyPhotoURLTests::absolute(&format!("{path}?map_post_id=42")).expect("raw");
    assert_eq!(
        UrlComponents::from_string(&raw).and_then(|components| components.query_items()),
        Some(vec![UrlQueryItem::new("map_post_id", Some("42".to_owned()))])
    );
    assert_eq!(MapJourneyPhotoURLTests::absolute(&format!("{path}?map_post_id=-1")), None);
    assert_eq!(MapJourneyPhotoURLTests::absolute(&format!("{path}?map_post_id=42&owner=other")), None);
    assert_eq!(MapJourneyPhotoURLTests::absolute(&format!("{path}?owner=other")), None);
    assert_eq!(MapJourneyPhotoURLTests::absolute("/api/file/secure/chat/photo.jpg?map_post_id=42"), None);
}
