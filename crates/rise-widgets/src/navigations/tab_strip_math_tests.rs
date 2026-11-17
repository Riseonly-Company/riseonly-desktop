use crate::navigations::tab_strip_math::{LensDrag, LensScrollDrive, TabStripLayout, rubber_banded};

const TOLERANCE: f32 = 1e-3;

fn close(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < TOLERANCE
}

/// six buttons, three on screen
/// 100 points a slot and 300 points to scroll
fn six_in_three() -> TabStripLayout {
    TabStripLayout::new(300.0, 6, 3, 0.0)
}

#[test]
fn slots_take_the_peek_only_when_buttons_overflow() {
    assert!(close(TabStripLayout::new(300.0, 5, 3, 0.5).visible_slots, 3.5));
    assert!(close(TabStripLayout::new(300.0, 3, 3, 0.5).visible_slots, 3.0));
    assert!(close(TabStripLayout::new(300.0, 2, 5, 0.0).visible_slots, 2.0));
    assert!(close(TabStripLayout::new(300.0, 5, 0, -1.0).visible_slots, 1.0));
}

#[test]
fn geometry_follows_the_slot_width() {
    let layout = six_in_three();
    assert!(close(layout.slot_width(), 100.0));
    assert!(close(layout.content_width(), 600.0));
    assert!(close(layout.max_scroll(), 300.0));
    assert!(layout.is_scrollable());
    assert!(close(layout.center(2), 250.0));
    assert!(!TabStripLayout::new(300.0, 2, 5, 0.0).is_scrollable());
}

#[test]
fn empty_strip_is_inert() {
    let layout = TabStripLayout::new(-10.0, 0, 0, 0.0);
    assert!(close(layout.viewport_width, 0.0));
    assert!(close(layout.slot_width(), 0.0));
    assert_eq!(layout.index(50.0), 0);
    assert_eq!(layout.slot(50.0), 0);
    assert_eq!(layout.wrapped(-3), 0);
    assert_eq!(layout.nearest_slot(2, 9), 2);
    assert!(close(layout.revealing_scroll(1, 40.0), 0.0));
}

#[test]
fn index_is_clamped_to_the_buttons() {
    let layout = six_in_three();
    assert_eq!(layout.index(250.0), 2);
    assert_eq!(layout.index(-10.0), 0);
    assert_eq!(layout.index(10_000.0), 5);
}

#[test]
fn centered_scroll_stops_at_the_ends() {
    let layout = six_in_three();
    assert!(close(layout.centered_scroll(0), 0.0));
    assert!(close(layout.centered_scroll(3), 200.0));
    assert!(close(layout.centered_scroll(5), 300.0));
}

#[test]
fn revealing_scroll_keeps_the_outer_neighbour_on_screen() {
    let layout = six_in_three();
    assert!(close(layout.revealing_scroll(4, 0.0), 300.0));
    assert!(close(layout.revealing_scroll(1, 300.0), 0.0));
    assert!(close(layout.revealing_scroll(1, 0.0), 0.0));
    assert!(close(layout.revealing_scroll(2, 50.0), 100.0));
    assert!(close(layout.revealing_scroll(2, 100.0), 100.0));
}

#[test]
fn lens_range_runs_between_the_visible_slot_centres() {
    let range = six_in_three().lens_range();
    assert!(close(*range.start(), 50.0));
    assert!(close(*range.end(), 250.0));
}

#[test]
fn looping_slots_wrap_both_ways() {
    let layout = six_in_three();
    assert!(close(layout.period(), 600.0));
    assert_eq!(layout.slot(-50.0), -1);
    assert_eq!(layout.wrapped(-1), 5);
    assert_eq!(layout.wrapped(13), 1);
    assert_eq!(layout.nearest_slot(0, 11), 12);
    assert_eq!(layout.nearest_slot(5, -2), -1);
    assert!(close(layout.looping_revealing_scroll(7, 0.0), 600.0));
    assert!(close(layout.looping_revealing_scroll(-2, 0.0), -300.0));
}

#[test]
fn drive_pushes_only_past_its_line_and_eases_in() {
    let layout = six_in_three();
    let drive = LensScrollDrive { threshold: 0.7, gain: 2.0, ramp: 20.0 };
    assert!(close(drive.forward_line(&layout), 210.0));
    assert!(close(drive.backward_line(&layout), 90.0));
    assert!(close(drive.scroll_delta(210.0, 220.0, &layout), 5.0));
    assert!(close(drive.scroll_delta(210.0, 240.0, &layout), 40.0));
    assert!(close(drive.scroll_delta(240.0, 210.0, &layout), 0.0));
    assert!(close(drive.scroll_delta(100.0, 60.0, &layout), -40.0));
    assert!(close(drive.scroll_delta(150.0, 150.0, &layout), 0.0));

    let linear = LensScrollDrive { threshold: 0.7, gain: 2.0, ramp: 0.0 };
    assert!(close(linear.scroll_delta(210.0, 240.0, &layout), 60.0));
}

#[test]
fn auto_gain_is_bounded() {
    let layout = six_in_three();
    assert!(close(LensScrollDrive::auto_gain(&layout, 0.7, 20.0), 4.0));
    assert!(close(LensScrollDrive::auto_gain(&layout, 0.5, 0.0), 2.4));
    let still = TabStripLayout::new(300.0, 2, 5, 0.0);
    assert!(close(LensScrollDrive::auto_gain(&still, 0.5, 0.0), 1.0));
}

#[test]
fn drag_scrolls_clamped_unless_the_strip_loops() {
    let layout = six_in_three();
    let drive = LensScrollDrive { threshold: 0.7, gain: 2.0, ramp: 20.0 };
    let mut drag = LensDrag { lens_x: 200.0, scroll: 0.0 };
    drag.r#move(40.0, &layout, &drive, false);
    assert!(close(drag.scroll, 40.0));
    drag.r#move(100.0, &layout, &drive, false);
    assert!(close(drag.scroll, 240.0));
    let mut looping = drag;
    drag.r#move(200.0, &layout, &drive, false);
    assert!(close(drag.lens_x, 540.0));
    assert!(close(drag.scroll, 300.0));
    looping.r#move(200.0, &layout, &drive, true);
    assert!(close(looping.scroll, 640.0));

    assert!(close(drag.displayed_x(&layout), 281.979_95));
    assert_eq!(drag.hovered_index(&layout), 5);
    assert_eq!(drag.hovered_slot(&layout), 5);
}

#[test]
fn drag_on_a_still_strip_moves_only_the_lens() {
    let layout = TabStripLayout::new(300.0, 2, 5, 0.0);
    let drive = LensScrollDrive { threshold: 0.7, gain: 2.0, ramp: 20.0 };
    let mut drag = LensDrag { lens_x: 100.0, scroll: 0.0 };
    drag.r#move(150.0, &layout, &drive, false);
    assert!(close(drag.lens_x, 250.0));
    assert!(close(drag.scroll, 0.0));
}

#[test]
fn rubber_band_resists_past_both_ends() {
    assert!(close(rubber_banded(120.0, 50.0..=250.0, 40.0), 120.0));
    assert!(close(rubber_banded(300.0, 50.0..=250.0, 0.0), 250.0));
    assert!(close(rubber_banded(-10.0, 50.0..=250.0, 0.0), 50.0));
    let past = rubber_banded(290.0, 50.0..=250.0, 40.0);
    let before = rubber_banded(10.0, 50.0..=250.0, 40.0);
    assert!(past > 250.0 && past < 290.0);
    assert!(close(past - 250.0, 50.0 - before));
}
