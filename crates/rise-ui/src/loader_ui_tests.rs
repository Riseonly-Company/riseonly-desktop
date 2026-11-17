use std::time::Duration;

use crate::loader_ui::{LoaderUi, LoaderUiSize, ring_diameter, ring_spin_degrees, ring_trim};

#[test]
fn sizes_are_the_ios_points() {
    assert_eq!(LoaderUiSize::Small.point_size(), 18.);
    assert_eq!(LoaderUiSize::Large.point_size(), 28.);
    assert_eq!(LoaderUiSize::Custom(40.).point_size(), 40.);
}

#[test]
fn spin_period_maps_speed_one_to_ten_onto_one_and_a_half_to_three_tenths() {
    assert_eq!(LoaderUi::new().speed(1.).spin_period(), Duration::from_secs_f64(1.5));
    assert!((LoaderUi::new().speed(10.).spin_period().as_secs_f64() - 0.3).abs() < 1e-9);
    assert_eq!(LoaderUi::new().speed(0.).spin_period(), LoaderUi::new().speed(1.).spin_period());
    assert!((LoaderUi::new().spin_period().as_secs_f64() - (1.5 - 2. * 1.2 / 9.)).abs() < 1e-9);
}

#[test]
fn ring_geometry_follows_ios() {
    assert_eq!(ring_trim(0.), 0.08);
    assert_eq!(ring_trim(2.), 1.);
    assert_eq!(ring_diameter(28., 3., 2.5), 19.5);
    assert_eq!(ring_diameter(10., 3., 2.5), 8.);
    assert_eq!(ring_spin_degrees(Duration::from_millis(500), Duration::from_secs(1)), 180.);
}
