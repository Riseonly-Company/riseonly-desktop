use gpui::{Bounds, Pixels, Point, point, px, size};

use crate::swiftui::zoom_navigation_transition::{
    zoom_content_opacity, zoom_flight_frame, zoom_flight_matrix,
};

fn source() -> Bounds<Pixels> {
    Bounds {
        origin: point(px(100.), px(200.)),
        size: size(px(100.), px(100.)),
    }
}

fn container() -> Bounds<Pixels> {
    Bounds {
        origin: point(px(0.), px(0.)),
        size: size(px(400.), px(800.)),
    }
}

fn close(a: Point<Pixels>, b: Point<Pixels>) -> bool {
    (f32::from(a.x) - f32::from(b.x)).abs() < 1e-2 && (f32::from(a.y) - f32::from(b.y)).abs() < 1e-2
}

#[test]
fn the_flight_starts_on_the_source_and_covers_it() {
    let frame = zoom_flight_frame(source(), container(), 0.0);
    assert_eq!(frame.clip, source());
    assert_eq!(frame.scale, 0.25);
}

#[test]
fn the_flight_lands_on_the_container_unscaled() {
    let frame = zoom_flight_frame(source(), container(), 1.0);
    assert_eq!(frame.clip, container());
    assert_eq!(frame.scale, 1.0);
}

#[test]
fn an_overshooting_spring_stays_inside_the_flight() {
    assert_eq!(
        zoom_flight_frame(source(), container(), 1.2),
        zoom_flight_frame(source(), container(), 1.0)
    );
}

#[test]
fn an_empty_container_keeps_the_unit_scale() {
    let empty = Bounds {
        origin: point(px(0.), px(0.)),
        size: size(px(0.), px(0.)),
    };
    assert_eq!(zoom_flight_frame(source(), empty, 0.0).scale, 1.0);
}

#[test]
fn the_matrix_puts_the_container_centre_on_the_clip_centre() {
    let frame = zoom_flight_frame(source(), container(), 0.0);
    let center = container().center();
    let matrix = zoom_flight_matrix(frame, center, 1.0);
    assert!(close(matrix.apply(center), source().center()));
    assert!(close(
        matrix.apply(point(px(200.), px(0.))),
        point(px(150.), px(150.))
    ));
}

#[test]
fn the_destination_fades_in_at_the_start_only() {
    assert_eq!(zoom_content_opacity(0.0), 0.0);
    assert_eq!(zoom_content_opacity(0.5), 1.0);
}
