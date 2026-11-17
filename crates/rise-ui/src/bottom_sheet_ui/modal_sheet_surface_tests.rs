use rise_theme::AppTheme;

use super::modal_sheet_surface::{ModalSheetFrame, ModalSheetMetrics, modal_sheet_outline};

const ASKED_WIDTH: f32 = 460.0;
const MARGIN: f32 = 12.0;

fn frame(width: f32, height: f32) -> ModalSheetFrame {
    ModalSheetFrame::laid_out(width, height, ASKED_WIDTH, MARGIN)
}

#[test]
fn a_wide_window_gets_the_dialog_width_and_the_height_ceiling() {
    let frame = frame(1600.0, 1000.0);
    assert_eq!(frame.width, ASKED_WIDTH);
    assert_eq!(frame.max_height, 1000.0 * ModalSheetMetrics::MAX_HEIGHT_FRACTION);
}

#[test]
fn a_narrow_window_keeps_the_margins_on_the_sides() {
    assert_eq!(frame(400.0, 800.0).width, 400.0 - MARGIN * 2.0);
}

#[test]
fn a_low_window_keeps_the_margins_above_and_below() {
    assert_eq!(frame(1000.0, 100.0).max_height, 100.0 - MARGIN * 2.0);
}

#[test]
fn an_empty_container_still_gives_a_positive_frame() {
    let frame = frame(0.0, 0.0);
    assert_eq!(frame.width, 1.0);
    assert_eq!(frame.max_height, 1.0);
}

#[test]
fn a_filled_height_stays_between_the_floor_and_the_ceiling() {
    let frame = frame(1600.0, 1000.0);
    assert_eq!(frame.fill_floor(), ModalSheetMetrics::FILL_FLOOR_HEIGHT);
    assert_eq!(frame.filled_height(100.0), ModalSheetMetrics::FILL_FLOOR_HEIGHT);
    assert_eq!(frame.filled_height(600.0), 600.0);
    assert_eq!(frame.filled_height(5000.0), frame.max_height);
}

#[test]
fn the_floor_never_passes_the_ceiling_in_a_low_window() {
    let frame = frame(1600.0, 400.0);
    assert_eq!(frame.fill_floor(), frame.max_height);
    assert_eq!(frame.filled_height(100.0), frame.max_height);
}

#[test]
fn the_content_ceiling_leaves_room_for_the_inset() {
    let frame = frame(1600.0, 1000.0);
    assert_eq!(
        frame.content_max_height(),
        frame.max_height - ModalSheetMetrics::CONTENT_INSET * 2.0
    );
}

#[test]
fn the_outline_is_a_visible_tint_of_the_theme_border() {
    let theme = AppTheme::dark();
    let outline = modal_sheet_outline(&theme);
    assert!(outline.a > 0.0);
    assert!(outline.a <= theme.border._200.a);
    assert_eq!((outline.h, outline.s, outline.l), (theme.border._200.h, theme.border._200.s, theme.border._200.l));
}
