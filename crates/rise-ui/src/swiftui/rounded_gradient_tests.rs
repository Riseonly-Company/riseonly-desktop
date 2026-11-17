use crate::swiftui::rounded_gradient::{keep_side, rounded_outline, stroke_border_pieces};

#[test]
fn the_outline_stays_inside_the_frame_and_touches_every_edge() {
    let outline = rounded_outline((10.0, 20.0), (100.0, 100.0), 16.0);
    let xs: Vec<f32> = outline.iter().map(|p| p.0).collect();
    let ys: Vec<f32> = outline.iter().map(|p| p.1).collect();
    let min_x = xs.iter().copied().fold(f32::INFINITY, f32::min);
    let max_x = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let min_y = ys.iter().copied().fold(f32::INFINITY, f32::min);
    let max_y = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!((min_x - 10.0).abs() < 0.01 && (max_x - 110.0).abs() < 0.01);
    assert!((min_y - 20.0).abs() < 0.01 && (max_y - 120.0).abs() < 0.01);
}

#[test]
fn a_radius_above_half_the_side_becomes_a_circle() {
    let outline = rounded_outline((0.0, 0.0), (10.0, 10.0), 100.0);
    for point in outline {
        let distance = ((point.0 - 5.0).powi(2) + (point.1 - 5.0).powi(2)).sqrt();
        assert!((distance - 5.0).abs() < 0.01);
    }
}

#[test]
fn the_stroke_has_one_quad_per_outline_segment() {
    let outline = rounded_outline((0.0, 0.0), (50.0, 50.0), 9.0);
    let pieces = stroke_border_pieces((0.0, 0.0), (50.0, 50.0), 9.0, 2.0);
    assert_eq!(pieces.len(), outline.len());
    assert!(pieces.iter().all(|piece| piece.len() == 4));
}

#[test]
fn clipping_keeps_only_the_wanted_side() {
    let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
    let left = keep_side(&square, |p| 5.0 - p.0);
    assert_eq!(left.len(), 4);
    assert!(left.iter().all(|p| p.0 <= 5.0));
    assert!(keep_side(&square, |p| -1.0 - p.0).is_empty());
}
