use gpui::{Bounds, Pixels, Point, point, px, size};

use super::media_picker_date_scrubber::MediaPickerDateScrubberLayout;

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds { origin: point(px(x), px(y)), size: size(px(width), px(height)) }
}

fn at(x: f32, y: f32) -> Point<Pixels> {
    point(px(x), px(y))
}

/// geometry of a scrubber a third of the way down a 210x600 layer
fn capsule_frame() -> Bounds<Pixels> {
    rect(96.0, 203.0, 96.0, 22.0)
}

fn indicator_frame() -> Bounds<Pixels> {
    rect(204.0, 192.0, 4.0, 44.0)
}

#[test]
fn only_the_handle_answers_touches() {
    assert!(MediaPickerDateScrubberLayout::is_inside_handle(at(140.0, 214.0), capsule_frame(), indicator_frame()));
    assert!(MediaPickerDateScrubberLayout::is_inside_handle(at(206.0, 214.0), capsule_frame(), indicator_frame()));
    assert!(!MediaPickerDateScrubberLayout::is_inside_handle(at(206.0, 40.0), capsule_frame(), indicator_frame()));
    assert!(!MediaPickerDateScrubberLayout::is_inside_handle(at(206.0, 520.0), capsule_frame(), indicator_frame()));
}

#[test]
fn handle_keeps_an_accessible_touch_target_around_the_knob() {
    let indicator = indicator_frame();
    let outset = (MediaPickerDateScrubberLayout::MINIMUM_TOUCH_DIMENSION - f32::from(indicator.size.width)) / 2.0;
    let min_x = f32::from(indicator.origin.x);

    assert!(MediaPickerDateScrubberLayout::is_inside_handle(at(min_x - outset + 1.0, 214.0), Bounds::default(), indicator));
    assert!(!MediaPickerDateScrubberLayout::is_inside_handle(at(min_x - outset - 1.0, 214.0), Bounds::default(), indicator));
}

#[test]
fn month_capsule_is_half_the_legacy_visual_size() {
    assert_eq!(MediaPickerDateScrubberLayout::CAPSULE_HEIGHT, MediaPickerDateScrubberLayout::LEGACY_CAPSULE_HEIGHT / 2.0);
    assert_eq!(
        MediaPickerDateScrubberLayout::MINIMUM_CAPSULE_WIDTH,
        MediaPickerDateScrubberLayout::LEGACY_MINIMUM_CAPSULE_WIDTH / 2.0
    );
}

#[test]
fn compact_visual_retains_accessible_touch_height() {
    let touch_height =
        MediaPickerDateScrubberLayout::CAPSULE_HEIGHT + MediaPickerDateScrubberLayout::CAPSULE_HIT_OUTSET * 2.0;

    assert!(touch_height >= MediaPickerDateScrubberLayout::MINIMUM_TOUCH_DIMENSION);
    assert_eq!(MediaPickerDateScrubberLayout::MINIMUM_TOUCH_DIMENSION, 44.0);
}

#[test]
fn localized_month_width_is_bounded() {
    assert_eq!(MediaPickerDateScrubberLayout::capsule_width(20.0, 168.0), MediaPickerDateScrubberLayout::MINIMUM_CAPSULE_WIDTH);
    assert_eq!(MediaPickerDateScrubberLayout::capsule_width(400.0, 168.0), MediaPickerDateScrubberLayout::MAXIMUM_CAPSULE_WIDTH);
    assert_eq!(MediaPickerDateScrubberLayout::capsule_width(400.0, 70.0), 70.0);
}
