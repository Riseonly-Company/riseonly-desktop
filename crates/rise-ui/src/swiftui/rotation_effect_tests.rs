use gpui::{Pixels, Point, TransformationMatrix, point, px};

use crate::swiftui::rotation_effect::rotation_effect_matrix;

fn close(a: Point<Pixels>, b: Point<Pixels>) -> bool {
    (f32::from(a.x) - f32::from(b.x)).abs() < 1e-3 && (f32::from(a.y) - f32::from(b.y)).abs() < 1e-3
}

#[test]
fn the_centre_stays_in_place() {
    let matrix = rotation_effect_matrix(point(px(50.), px(20.)), 45., 1.);
    assert!(close(matrix.apply(point(px(50.), px(20.))), point(px(50.), px(20.))));
}

#[test]
fn a_positive_angle_turns_clockwise_on_screen() {
    let matrix = rotation_effect_matrix(point(px(10.), px(10.)), 90., 1.);
    assert!(close(matrix.apply(point(px(20.), px(10.))), point(px(10.), px(20.))));
}

#[test]
fn the_turn_works_in_scaled_pixels() {
    let matrix = rotation_effect_matrix(point(px(10.), px(10.)), 90., 2.);
    assert!(close(matrix.apply(point(px(40.), px(20.))), point(px(20.), px(40.))));
}

#[test]
fn a_zero_angle_is_the_unit_matrix() {
    let matrix = rotation_effect_matrix(point(px(30.), px(40.)), 0., 2.);
    let unit = TransformationMatrix::unit();
    assert!(close(matrix.apply(point(px(7.), px(9.))), unit.apply(point(px(7.), px(9.)))));
}
