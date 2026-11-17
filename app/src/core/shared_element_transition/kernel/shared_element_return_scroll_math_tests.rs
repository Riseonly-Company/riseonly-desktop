use gpui::{Bounds, Pixels, Point, point, px};

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementReturnScrollMath, SharedElementReturnScrollMathBand, SharedElementReturnScrollMathClamp,
    SharedElementReturnScrollMathPosition,
};
use crate::modules::user::components::profile_content::profile_scroll_coordinator::{
    ProfileVerticalMasterScrollPolicy, ProfileVerticalMasterScrollPolicyPosition,
};

const COLUMNS: usize = 3;
const SPACING: f32 = 2.0;
const CELL_HEIGHT: f32 = 150.0;
const GRID_ORIGIN_Y: f32 = 176.0;
const VIEWPORT_HEIGHT: f32 = 818.0;
const BAND: SharedElementReturnScrollMathBand = SharedElementReturnScrollMathBand { top: 176.0, bottom_inset: 0.0 };
const EPSILON: f32 = ProfileVerticalMasterScrollPolicy::EPSILON;
const TOLERANCE: f32 = 1e-2;

fn cell_width() -> f32 {
    SharedElementReturnScrollMath::cell_width(390.0, COLUMNS, SPACING)
}

fn item_rect(index: usize) -> Bounds<Pixels> {
    SharedElementReturnScrollMath::grid_item_rect(index, COLUMNS, cell_width(), CELL_HEIGHT, SPACING, GRID_ORIGIN_Y)
}

fn band_center() -> f32 {
    SharedElementReturnScrollMath::band_center_y(BAND, VIEWPORT_HEIGHT)
}

fn position(index: usize, collapse: f32, inner: f32, pin_offset: f32) -> SharedElementReturnScrollMathPosition {
    SharedElementReturnScrollMath::position_item_mid_y(
        f32::from(item_rect(index).center().y),
        band_center(),
        pin_offset,
        collapse,
        inner,
        EPSILON,
    )
}

fn predicted_frame(
    index: usize,
    position: SharedElementReturnScrollMathPosition,
    collapse: f32,
    origin: Point<Pixels>,
    pin_offset: f32,
) -> Bounds<Pixels> {
    SharedElementReturnScrollMath::predicted_screen_frame(
        item_rect(index),
        origin,
        pin_offset,
        collapse,
        position.master_offset,
    )
}

fn zero() -> Point<Pixels> {
    point(px(0.0), px(0.0))
}

fn mid_y(bounds: Bounds<Pixels>) -> f32 {
    f32::from(bounds.center().y)
}

#[test]
fn grid_metrics_match_the_profile_grid() {
    assert!((cell_width() - (390.0 - 4.0) / 3.0).abs() < TOLERANCE);
    assert_eq!(f32::from(item_rect(0).left()), 0.0);
    assert_eq!(f32::from(item_rect(0).top()), 176.0);
    assert_eq!(f32::from(item_rect(0).size.height), 150.0);
    let fifth = item_rect(4);
    assert!((f32::from(fifth.left()) - (cell_width() + 2.0)).abs() < TOLERANCE);
    assert_eq!(f32::from(fifth.top()), 176.0 + 152.0);
}

#[test]
fn origin_x_shifts_every_column_and_defaults_to_zero() {
    let inset = 16.0;
    for index in 0..7 {
        let shifted = SharedElementReturnScrollMath::grid_item_rect_with_origin_x(
            index,
            COLUMNS,
            cell_width(),
            CELL_HEIGHT,
            SPACING,
            GRID_ORIGIN_Y,
            inset,
        );
        assert!((f32::from(shifted.left()) - (f32::from(item_rect(index).left()) + inset)).abs() < TOLERANCE);
        assert_eq!(shifted.top(), item_rect(index).top());
    }
    let explicit_zero = SharedElementReturnScrollMath::grid_item_rect_with_origin_x(
        4,
        COLUMNS,
        cell_width(),
        CELL_HEIGHT,
        SPACING,
        GRID_ORIGIN_Y,
        0.0,
    );
    assert_eq!(explicit_zero, item_rect(4));
}

#[test]
fn band_center_sits_between_pinned_tabs_and_bottom_chrome() {
    assert_eq!(band_center(), 497.0);
    let with_chrome = SharedElementReturnScrollMathBand { top: 176.0, bottom_inset: 60.0 };
    assert_eq!(SharedElementReturnScrollMath::band_center_y(with_chrome, 818.0), 467.0);
}

#[test]
fn first_row_re_expands_the_header() {
    let collapse = 200.0;
    let position = position(0, collapse, 900.0, 0.0);
    assert_eq!(position.clamp, SharedElementReturnScrollMathClamp::Top);
    assert!(position.collapse_progress < 1.0);
    assert_eq!(position.master_offset, 0.0);
    assert_eq!(position.inner_offset, 0.0);
    assert!(mid_y(predicted_frame(0, position, collapse, zero(), 0.0)) < band_center());
}

#[test]
fn last_row_clamps_at_the_end() {
    let collapse = 200.0;
    let inner = 900.0;
    let position = position(29, collapse, inner, 0.0);
    assert_eq!(position.clamp, SharedElementReturnScrollMathClamp::Bottom);
    assert_eq!(position.master_offset, collapse + inner);
    assert_eq!(position.inner_offset, inner);
    assert_eq!(position.collapse_progress, 1.0);
    assert!(mid_y(predicted_frame(29, position, collapse, zero(), 0.0)) > band_center());
}

#[test]
fn middle_row_is_centred_in_the_band() {
    let collapse = 200.0;
    let position = position(13, collapse, 900.0, 0.0);
    assert_eq!(position.clamp, SharedElementReturnScrollMathClamp::None);
    assert_eq!(position.collapse_progress, 1.0);
    assert!(position.inner_offset > 0.0);
    assert!((mid_y(predicted_frame(13, position, collapse, zero(), 0.0)) - band_center()).abs() < TOLERANCE);
}

#[test]
fn row_inside_the_collapse_range_is_centred_with_a_partial_header() {
    let collapse = 400.0;
    let position = position(0, collapse, 900.0, 0.0);
    assert_eq!(position.clamp, SharedElementReturnScrollMathClamp::None);
    assert!(position.collapse_progress > 0.0 && position.collapse_progress < 1.0);
    assert_eq!(position.inner_offset, 0.0);
    assert!((mid_y(predicted_frame(0, position, collapse, zero(), 0.0)) - band_center()).abs() < TOLERANCE);
}

#[test]
fn centring_is_independent_of_viewport_origin_and_pin_offset() {
    let collapse = 260.0;
    let origin = point(px(12.0), px(47.0));
    let pin_offset = -20.0;
    let position = position(10, collapse, 1_400.0, pin_offset);
    let frame = predicted_frame(10, position, collapse, origin, pin_offset);
    assert_eq!(position.clamp, SharedElementReturnScrollMathClamp::None);
    assert!((mid_y(frame) - (47.0 + band_center())).abs() < TOLERANCE);
    assert!((f32::from(frame.left()) - (12.0 + f32::from(item_rect(10).left()))).abs() < TOLERANCE);
    assert_eq!(frame.size, item_rect(10).size);
}

#[test]
fn zero_collapse_range_reports_a_collapsed_header() {
    let position = position(13, 0.0, 900.0, 0.0);
    assert_eq!(position.collapse_progress, 1.0);
    assert_eq!(position.inner_offset, position.master_offset);
}

#[test]
fn positions_round_trip_through_the_master_scroll_policy() {
    for collapse in [200.0, 400.0] {
        let inner = 900.0;
        for index in 0..30 {
            let position = position(index, collapse, inner, 0.0);

            let master = ProfileVerticalMasterScrollPolicy::master_offset(
                ProfileVerticalMasterScrollPolicyPosition {
                    collapse_progress: position.collapse_progress,
                    inner_offset: position.inner_offset,
                },
                collapse,
                inner,
            );
            assert!((master - position.master_offset).abs() < TOLERANCE);

            let mapping = ProfileVerticalMasterScrollPolicy::mapping(position.master_offset, collapse, inner);
            assert!((mapping.collapse_progress - position.collapse_progress).abs() < TOLERANCE);
            assert!((mapping.inner_offset - position.inner_offset).abs() < TOLERANCE);
        }
    }
}

#[test]
fn repeated_returns_on_se_keep_the_grid_where_it_was() {
    let viewport_height = 667.0;
    let band = SharedElementReturnScrollMathBand { top: 176.0, bottom_inset: 49.0 };
    let width = SharedElementReturnScrollMath::cell_width(375.0, 3, 2.0);
    let item = SharedElementReturnScrollMath::grid_item_rect_with_origin_x(1, 3, width, width * 16.0 / 9.0, 2.0, 176.0, 0.0);
    let live = 376.0_f32;
    let inner = 1_200.0;

    let return_cycle = |master: f32, parked: f32| -> f32 {
        let mapped = ProfileVerticalMasterScrollPolicy::mapping(master, live, inner);
        let previous = ProfileVerticalMasterScrollPolicyPosition {
            collapse_progress: mapped.collapse_progress,
            inner_offset: mapped.inner_offset,
        };
        let _ = SharedElementReturnScrollMath::position_item_rect(
            item,
            band,
            viewport_height,
            Some(ProfileVerticalMasterScrollPolicy::master_offset(previous, parked, inner)),
            0.0,
            parked,
            inner,
            EPSILON,
        );
        SharedElementReturnScrollMath::position_item_rect(
            item,
            band,
            viewport_height,
            Some(ProfileVerticalMasterScrollPolicy::master_offset(previous, live, inner)),
            0.0,
            live,
            inner,
            EPSILON,
        )
        .master_offset
    };

    for parked in [live, live / 2.0, 0.0] {
        let first = return_cycle(0.0, parked);
        let top = live + f32::from(item.top()) - first;
        assert!(top >= band.top - 0.5);
        assert!(top + f32::from(item.size.height) <= viewport_height - band.bottom_inset + 0.5);
        let mut master = first;
        for _ in 0..3 {
            master = return_cycle(master, parked);
            assert!((master - first).abs() < TOLERANCE);
        }
    }
}
