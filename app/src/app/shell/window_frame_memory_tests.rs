use gpui::{Bounds, Pixels, WindowBounds, point, px, size};
use rise_platform::user_defaults::DefaultsValue;

use super::window_frame_memory::{WindowFrame, WindowFrameMode};

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

fn frame(mode: WindowFrameMode, bounds: Bounds<Pixels>, display: Option<&str>) -> WindowFrame {
    WindowFrame {
        mode,
        bounds,
        display: display.map(str::to_owned),
    }
}

#[test]
fn a_frame_survives_the_preferences_round_trip() {
    let saved = frame(
        WindowFrameMode::Maximized,
        rect(40.0, 60.0, 1196.0, 760.0),
        Some("display-a"),
    );

    assert_eq!(WindowFrame::from_defaults(&saved.to_defaults()), Some(saved));
}

#[test]
fn a_frame_without_a_display_round_trips_without_one() {
    let saved = frame(WindowFrameMode::Windowed, rect(0.0, 0.0, 800.0, 600.0), None);

    let restored = WindowFrame::from_defaults(&saved.to_defaults());

    assert_eq!(restored.and_then(|frame| frame.display), None);
}

#[test]
fn every_mode_maps_back_to_its_name_and_to_its_gpui_bounds() {
    let bounds = rect(1.0, 2.0, 3.0, 4.0);
    for mode in [
        WindowFrameMode::Windowed,
        WindowFrameMode::Maximized,
        WindowFrameMode::Fullscreen,
    ] {
        assert_eq!(WindowFrameMode::from_name(mode.name()), Some(mode));
        assert_eq!(mode.window_bounds(bounds).get_bounds(), bounds);
    }
    assert!(matches!(
        WindowFrameMode::Fullscreen.window_bounds(bounds),
        WindowBounds::Fullscreen(_)
    ));
    assert_eq!(WindowFrameMode::from_name("minimized"), None);
}

#[test]
fn damaged_preferences_are_not_a_frame() {
    assert_eq!(WindowFrame::from_defaults(&DefaultsValue::from("windowed")), None);

    let mut zero_width = frame(WindowFrameMode::Windowed, rect(0.0, 0.0, 800.0, 600.0), None);
    zero_width.bounds.size.width = px(0.0);
    assert_eq!(WindowFrame::from_defaults(&zero_width.to_defaults()), None);

    let mut endless = frame(WindowFrameMode::Windowed, rect(0.0, 0.0, 800.0, 600.0), None);
    endless.bounds.origin.x = px(f32::INFINITY);
    assert_eq!(WindowFrame::from_defaults(&endless.to_defaults()), None);
}

#[test]
fn a_frame_inside_the_visible_area_is_kept_as_is() {
    let visible = rect(0.0, 25.0, 1440.0, 875.0);
    let saved = frame(WindowFrameMode::Windowed, rect(100.0, 80.0, 1196.0, 760.0), None);

    assert_eq!(saved.fitted(visible), saved.bounds);
}

#[test]
fn a_frame_hanging_off_the_display_is_pulled_back_inside() {
    let visible = rect(0.0, 25.0, 1440.0, 875.0);
    let saved = frame(WindowFrameMode::Windowed, rect(900.0, -40.0, 1196.0, 760.0), None);

    assert_eq!(saved.fitted(visible), rect(244.0, 25.0, 1196.0, 760.0));
}

#[test]
fn a_frame_larger_than_a_smaller_display_shrinks_to_it() {
    let visible = rect(0.0, 0.0, 1280.0, 720.0);
    let saved = frame(WindowFrameMode::Windowed, rect(300.0, 200.0, 2560.0, 1400.0), None);

    assert_eq!(saved.fitted(visible), visible);
}

#[test]
fn a_display_with_an_offset_origin_keeps_the_window_on_it() {
    let visible = rect(1920.0, 0.0, 1920.0, 1040.0);
    let saved = frame(WindowFrameMode::Windowed, rect(0.0, 0.0, 1000.0, 700.0), None);

    assert_eq!(saved.fitted(visible), rect(1920.0, 0.0, 1000.0, 700.0));
}

#[test]
fn a_windowed_frame_keeps_its_own_bounds() {
    let previous = frame(WindowFrameMode::Windowed, rect(10.0, 10.0, 640.0, 480.0), None);
    let current = frame(WindowFrameMode::Windowed, rect(100.0, 80.0, 1196.0, 760.0), None);

    assert_eq!(current.clone().keeping_restore_bounds(Some(&previous)), current);
}

#[test]
fn a_zoomed_frame_takes_the_restore_bounds_of_the_saved_one() {
    let previous = frame(WindowFrameMode::Windowed, rect(100.0, 80.0, 1196.0, 760.0), Some("a"));
    let current = frame(WindowFrameMode::Fullscreen, rect(0.0, 0.0, 2560.0, 1440.0), Some("b"));

    assert_eq!(
        current.keeping_restore_bounds(Some(&previous)),
        frame(WindowFrameMode::Fullscreen, rect(100.0, 80.0, 1196.0, 760.0), Some("b"))
    );
}

#[test]
fn a_first_zoomed_frame_keeps_the_bounds_it_has() {
    let current = frame(WindowFrameMode::Maximized, rect(0.0, 0.0, 2560.0, 1440.0), None);

    assert_eq!(current.clone().keeping_restore_bounds(None), current);
}
