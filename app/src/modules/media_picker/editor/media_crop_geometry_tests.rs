use gpui::{Bounds, Point, bounds, point, red, size, white};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::media_picker::editor::editor_crop_auto_fit::{
    EditorCropAutoFitPlanner, EditorCropAutoFitTransition,
};
use crate::modules::media_picker::editor::editor_crop_overlay::{
    EditorCropOverlayUIView, EditorCropOverlayUIViewResizeEdge,
};
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::media_picker::editor::editor_image_layout::EditorImageLayout;
use crate::modules::media_picker::editor::editor_mode::EditorMode;
use crate::modules::media_picker::editor::editor_state::{
    EditorDrawStroke, EditorDrawStrokeTool, EditorImageState, EditorOverlayCoordinateSpace, EditorTextEntity,
    EditorTextEntityStyleMode, EditorTextEntityTextAlignment,
};

const FRAME_TOLERANCE: f32 = 0.001;
const PROJECTION_TOLERANCE: f32 = 0.01;

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<f32> {
    bounds(point(x, y), size(width, height))
}

fn min_x(r: &Bounds<f32>) -> f32 {
    r.origin.x
}

fn max_x(r: &Bounds<f32>) -> f32 {
    r.origin.x + r.size.width
}

fn min_y(r: &Bounds<f32>) -> f32 {
    r.origin.y
}

fn max_y(r: &Bounds<f32>) -> f32 {
    r.origin.y + r.size.height
}

fn mid_x(r: &Bounds<f32>) -> f32 {
    r.origin.x + r.size.width / 2.0
}

fn mid_y(r: &Bounds<f32>) -> f32 {
    r.origin.y + r.size.height / 2.0
}

fn expect_close_point(actual: Point<f32>, expected: Point<f32>, tolerance: f32) {
    assert!((actual.x - expected.x).abs() < tolerance);
    assert!((actual.y - expected.y).abs() < tolerance);
}

fn expect_close(actual: Bounds<f32>, expected: Bounds<f32>, tolerance: f32) {
    assert!((min_x(&actual) - min_x(&expected)).abs() < tolerance);
    assert!((min_y(&actual) - min_y(&expected)).abs() < tolerance);
    assert!((actual.size.width - expected.size.width).abs() < tolerance);
    assert!((actual.size.height - expected.size.height).abs() < tolerance);
}

fn resize_edge(at: Point<f32>, crop: Bounds<f32>) -> EditorCropOverlayUIViewResizeEdge {
    EditorCropOverlayUIView::resize_edge(
        at,
        crop,
        EditorCropOverlayUIView::DEFAULT_HANDLE_SIZE,
        EditorCropOverlayUIView::DEFAULT_HIT_OUTSET,
    )
}

#[test]
fn centered_crop_expands_to_horizontal_edges() {
    let available = rect(14.0, 100.0, 362.0, 620.0);
    let crop = rect(241.0, 260.0, 120.0, 90.0);
    let target = EditorCropAutoFitPlanner::target_rect(crop, available, 24.0).unwrap();

    assert!((min_x(&target) - min_x(&available)).abs() < FRAME_TOLERANCE);
    assert!((max_x(&target) - max_x(&available)).abs() < FRAME_TOLERANCE);
    assert!((mid_y(&target) - mid_y(&available)).abs() < FRAME_TOLERANCE);
}

#[test]
fn tall_crop_stops_at_vertical_bounds() {
    let available = rect(0.0, 80.0, 390.0, 600.0);
    let crop = rect(170.0, 220.0, 50.0, 400.0);
    let target = EditorCropAutoFitPlanner::target_rect(crop, available, 24.0).unwrap();

    assert!((min_y(&target) - min_y(&available)).abs() < FRAME_TOLERANCE);
    assert!((max_y(&target) - max_y(&available)).abs() < FRAME_TOLERANCE);
    assert!(target.size.width < available.size.width);
}

#[test]
fn maximum_zoom_produces_reachable_frame() {
    let available = rect(0.0, 0.0, 390.0, 700.0);
    let crop = rect(180.0, 340.0, 1.0, 1.0);
    let target = EditorCropAutoFitPlanner::target_rect(crop, available, 24.0).unwrap();

    assert!((target.size.width - 24.0).abs() < FRAME_TOLERANCE);
    assert!((target.size.height - 24.0).abs() < FRAME_TOLERANCE);
}

#[test]
fn off_center_selection_becomes_the_target_without_changing_its_content() {
    let available = rect(0.0, 90.0, 390.0, 640.0);
    let image_before = rect(74.0, 130.0, 242.0, 484.0);
    let crop_before = rect(92.0, 248.0, 96.0, 180.0);
    let relative = EditorCropAutoFitPlanner::relative_rect(crop_before, image_before).unwrap();
    let target = EditorCropAutoFitPlanner::target_rect(crop_before, available, 512.0).unwrap();
    let scale = target.size.width / crop_before.size.width;
    let image_after_width = image_before.size.width * scale;
    let image_after_height = image_before.size.height * scale;
    let image_after = rect(
        mid_x(&target) - mid_x(&relative) * image_after_width,
        mid_y(&target) - mid_y(&relative) * image_after_height,
        image_after_width,
        image_after_height,
    );
    let projected = EditorCropAutoFitPlanner::project(relative, image_after);

    expect_close(projected, target, PROJECTION_TOLERANCE);
}

#[test]
fn projection_round_trip_does_not_drift() {
    let image = rect(-240.0, -110.0, 1400.0, 2100.0);
    let original = rect(84.0, 312.0, 513.0, 777.0);
    let mut current = original;

    for _ in 0..100 {
        let relative = EditorCropAutoFitPlanner::relative_rect(current, image).unwrap();
        current = EditorCropAutoFitPlanner::project(relative, image);
    }

    expect_close(current, original, PROJECTION_TOLERANCE);
}

#[test]
fn auto_fit_keeps_frame_attached_to_the_selected_image_region() {
    let source_image = rect(42.0, 118.0, 306.0, 544.0);
    let source_frame = rect(86.0, 176.0, 112.0, 196.0);
    let target_frame = rect(14.0, 92.0, 362.0, 633.5);
    let relative = EditorCropAutoFitPlanner::relative_rect(source_frame, source_image).unwrap();
    let source_zoom = 1.25_f32;
    let target_zoom = source_zoom * target_frame.size.width / source_frame.size.width;
    let transition = EditorCropAutoFitTransition { source_frame, target_frame, source_zoom, target_zoom };

    for raw_progress in [0.0_f32, 0.1, 0.37, 0.75, 1.0] {
        let sample = transition.sample(raw_progress);
        let scale = sample.zoom / source_zoom;
        let image_width = source_image.size.width * scale;
        let image_height = source_image.size.height * scale;
        let image_rect = rect(
            mid_x(&sample.crop_frame) - mid_x(&relative) * image_width,
            mid_y(&sample.crop_frame) - mid_y(&relative) * image_height,
            image_width,
            image_height,
        );
        let projected = EditorCropAutoFitPlanner::project(relative, image_rect);

        expect_close(projected, sample.crop_frame, PROJECTION_TOLERANCE);
    }
}

#[test]
fn auto_fit_transition_starts_and_finishes_on_exact_planned_frames() {
    let source = rect(92.0, 248.0, 96.0, 180.0);
    let target = rect(14.0, 102.0, 362.0, 678.75);
    let transition = EditorCropAutoFitTransition {
        source_frame: source,
        target_frame: target,
        source_zoom: 1.4,
        target_zoom: 1.4 * target.size.width / source.size.width,
    };

    expect_close(transition.sample(0.0).crop_frame, source, FRAME_TOLERANCE);
    expect_close(transition.sample(1.0).crop_frame, target, FRAME_TOLERANCE);
}

#[test]
fn oversized_crop_is_strictly_contained() {
    let bounds = rect(20.0, 40.0, 300.0, 500.0);
    let crop = rect(-100.0, -200.0, 900.0, 1200.0);
    let result = EditorCropOverlayUIView::clamp(crop, bounds, EditorCropOverlayUIView::DEFAULT_CLAMP_MIN_SIDE);

    assert_eq!(result, bounds);
}

#[test]
fn single_pixel_crop_is_not_treated_as_full_frame() {
    let one_pixel_removed_from_width = rect(0.0, 0.0, 3999.0 / 4000.0, 1.0);
    let one_pixel_removed_from_height = rect(0.0, 0.0, 1.0, 2999.0 / 3000.0);

    assert!(!EditorCropAutoFitPlanner::is_effectively_full(Some(one_pixel_removed_from_width)));
    assert!(!EditorCropAutoFitPlanner::is_effectively_full(Some(one_pixel_removed_from_height)));
}

#[test]
fn overlay_orientation_mapping_round_trips_for_every_transform_combination() {
    let source_size = size(4032.0_f32, 3024.0);
    let points = [point(0.001_f32, 0.999), point(0.173, 0.287), point(0.5, 0.5), point(0.941, 0.071)];

    for rotation_steps in -1..=4_i64 {
        for mirrored in [false, true] {
            for free_rotation in [-44.9_f64, -18.75, -0.001, 0.0, 0.001, 37.25, 44.9] {
                let space = EditorOverlayCoordinateSpace {
                    crop_rect_relative: rect(0.0, 0.0, 1.0, 1.0),
                    rotation_steps,
                    free_rotation_degrees: free_rotation,
                    is_mirrored: mirrored,
                };

                for test_point in points {
                    let oriented = EditorImageBake::map_to_oriented_unit(test_point, source_size, &space);
                    let restored = EditorImageBake::map_from_oriented_unit(oriented, source_size, &space);

                    expect_close_point(restored, test_point, FRAME_TOLERANCE);
                }
            }
        }
    }
}

#[test]
fn sequential_crop_remap_matches_direct_remap() {
    let mut state = EditorImageState::empty(Some("crop-regression".to_string()), size(4032.0, 3024.0));
    state.strokes = vec![EditorDrawStroke {
        tool: EditorDrawStrokeTool::Pen,
        color: red(),
        width: 0.012,
        points: vec![point(0.48, 0.49), point(0.50, 0.50), point(0.52, 0.51)],
    }];
    state.texts = vec![EditorTextEntity {
        id: Uuid::from_uuid_string("608846EF-DB8D-4D10-9E84-C6BA55A63028").unwrap(),
        text: "crop".to_string(),
        font_size: 0.06,
        color: white(),
        center: point(0.5, 0.5),
        rotation: 0.17,
        scale: 1.1,
        style_mode: EditorTextEntityStyleMode::Background,
        alignment: EditorTextEntityTextAlignment::Center,
    }];

    let intermediate = EditorOverlayCoordinateSpace {
        crop_rect_relative: rect(0.15, 0.2, 0.7, 0.6),
        rotation_steps: 1,
        free_rotation_degrees: 17.5,
        is_mirrored: true,
    };
    let destination = EditorOverlayCoordinateSpace {
        crop_rect_relative: rect(0.25, 0.15, 0.5, 0.7),
        rotation_steps: 3,
        free_rotation_degrees: -31.25,
        is_mirrored: false,
    };

    let sequential = EditorImageBake::remap_overlay_coordinates(
        &EditorImageBake::remap_overlay_coordinates(&state, intermediate),
        destination,
    );
    let direct = EditorImageBake::remap_overlay_coordinates(&state, destination);

    assert_eq!(sequential.overlay_coordinate_space, Some(destination));
    assert_eq!(sequential.strokes.len(), direct.strokes.len());
    assert_eq!(sequential.texts.len(), direct.texts.len());
    assert_eq!(sequential.strokes[0].points.len(), direct.strokes[0].points.len());

    for (sequential_point, direct_point) in sequential.strokes[0].points.iter().zip(&direct.strokes[0].points) {
        expect_close_point(*sequential_point, *direct_point, FRAME_TOLERANCE);
    }
    expect_close_point(sequential.texts[0].center, direct.texts[0].center, FRAME_TOLERANCE);
    assert!((sequential.strokes[0].width - direct.strokes[0].width).abs() < FRAME_TOLERANCE);
    assert!((sequential.texts[0].font_size - direct.texts[0].font_size).abs() < FRAME_TOLERANCE);
}

#[test]
fn crop_insets_include_safe_area_and_viewport_gutter() {
    let safe_top = 62.0_f32;
    let insets = EditorImageLayout::compute_insets(
        size(402.0, 874.0),
        size(1200.0, 1600.0),
        safe_top,
        34.0,
        0.0,
        0.0,
        EditorMode::Crop,
        154.0,
    );

    assert!((insets.top - safe_top - EditorImageLayout::CROP_VIEWPORT_INSET).abs() < FRAME_TOLERANCE);
    assert!((insets.left - EditorImageLayout::CROP_VIEWPORT_INSET).abs() < FRAME_TOLERANCE);
    assert!((insets.right - EditorImageLayout::CROP_VIEWPORT_INSET).abs() < FRAME_TOLERANCE);

    let landscape_insets = EditorImageLayout::compute_insets(
        size(874.0, 402.0),
        size(1600.0, 1200.0),
        0.0,
        21.0,
        59.0,
        21.0,
        EditorMode::Crop,
        154.0,
    );
    assert!((landscape_insets.left - 59.0 - EditorImageLayout::CROP_VIEWPORT_INSET).abs() < FRAME_TOLERANCE);
    assert!((landscape_insets.right - 21.0 - EditorImageLayout::CROP_VIEWPORT_INSET).abs() < FRAME_TOLERANCE);
}

#[test]
fn very_long_portrait_fits_inside_crop_available_rect() {
    let stage_size = size(402.0_f32, 874.0);
    let image_size = size(1080.0_f32, 6000.0);
    let safe_top = 62.0_f32;
    let safe_bottom = 34.0_f32;
    let toolbar_height = 154.0_f32;
    let insets = EditorImageLayout::compute_insets(
        stage_size,
        image_size,
        safe_top,
        safe_bottom,
        0.0,
        0.0,
        EditorMode::Crop,
        toolbar_height,
    );
    let available = rect(
        insets.left,
        insets.top,
        stage_size.width - insets.left - insets.right,
        stage_size.height - insets.top - insets.bottom,
    );
    let fitted_size = EditorImageLayout::aspect_fit(image_size, available.size);
    let fitted = rect(
        mid_x(&available) - fitted_size.width / 2.0,
        mid_y(&available) - fitted_size.height / 2.0,
        fitted_size.width,
        fitted_size.height,
    );
    let toolbar_top = stage_size.height - toolbar_height - safe_bottom;

    assert!(available.size.width > 0.0);
    assert!(available.size.height > 0.0);
    assert!(min_x(&fitted) >= min_x(&available) - FRAME_TOLERANCE);
    assert!(max_x(&fitted) <= max_x(&available) + FRAME_TOLERANCE);
    assert!(min_y(&fitted) >= min_y(&available) - FRAME_TOLERANCE);
    assert!(max_y(&fitted) <= max_y(&available) + FRAME_TOLERANCE);
    assert!((min_y(&fitted) - min_y(&available)).abs() < FRAME_TOLERANCE);
    assert!((max_y(&fitted) - max_y(&available)).abs() < FRAME_TOLERANCE);
    assert!(max_y(&fitted) <= toolbar_top - EditorImageLayout::TOOLBAR_CLEARANCE + FRAME_TOLERANCE);
}

#[test]
fn fine_rotation_does_not_expand_outer_crop_layout() {
    let source = size(1080.0_f32, 6000.0);

    for rotation_steps in 0..4_i64 {
        let base_layout = EditorImageLayout::oriented_display_size(source, rotation_steps, 0.0);

        for fine_rotation in [-44.9_f64, -17.25, 17.25, 44.9] {
            let rotated_content_bounds = EditorImageLayout::oriented_display_size(source, rotation_steps, fine_rotation);
            let outer_layout = EditorImageLayout::oriented_display_size(source, rotation_steps, 0.0);

            assert_eq!(outer_layout, base_layout);
            assert_ne!(rotated_content_bounds, base_layout);
        }
    }
}

#[test]
fn unequal_corner_offsets_still_prefer_every_corner_resize_zone() {
    let crop = rect(100.0, 160.0, 200.0, 300.0);
    let cases = [
        (point(126.0, 151.0), EditorCropOverlayUIViewResizeEdge::TopLeft),
        (point(271.0, 174.0), EditorCropOverlayUIViewResizeEdge::TopRight),
        (point(117.0, 427.0), EditorCropOverlayUIViewResizeEdge::BottomLeft),
        (point(266.0, 451.0), EditorCropOverlayUIViewResizeEdge::BottomRight),
    ];

    for (at, expected) in cases {
        assert_eq!(resize_edge(at, crop), expected);
    }
}

#[test]
fn midpoint_resize_zones_remain_edges() {
    let crop = rect(100.0, 160.0, 200.0, 300.0);
    let cases = [
        (point(mid_x(&crop), min_y(&crop)), EditorCropOverlayUIViewResizeEdge::Top),
        (point(mid_x(&crop), max_y(&crop)), EditorCropOverlayUIViewResizeEdge::Bottom),
        (point(min_x(&crop), mid_y(&crop)), EditorCropOverlayUIViewResizeEdge::Left),
        (point(max_x(&crop), mid_y(&crop)), EditorCropOverlayUIViewResizeEdge::Right),
    ];

    for (at, expected) in cases {
        assert_eq!(resize_edge(at, crop), expected);
    }
}

#[test]
fn near_limit_fine_rotation_zoom_symmetrically_covers_crop() {
    let crop_size = size(320.0_f32, 540.0);
    let image_size = size(280.0_f32, 460.0);
    let angle = 44.9_f32 * std::f32::consts::PI / 180.0;
    let positive_zoom = EditorCropAutoFitPlanner::minimum_zoom_covering(crop_size, image_size, angle, 0.0);
    let negative_zoom = EditorCropAutoFitPlanner::minimum_zoom_covering(crop_size, image_size, -angle, 0.0);
    let cosine = angle.cos().abs();
    let sine = angle.sin().abs();
    let required_width = cosine * crop_size.width + sine * crop_size.height;
    let required_height = sine * crop_size.width + cosine * crop_size.height;
    let expected_zoom = (required_width / image_size.width).max(required_height / image_size.height);

    assert!((positive_zoom - negative_zoom).abs() < FRAME_TOLERANCE);
    assert!((positive_zoom - expected_zoom).abs() < FRAME_TOLERANCE);
    assert!(image_size.width * positive_zoom >= required_width - FRAME_TOLERANCE);
    assert!(image_size.height * positive_zoom >= required_height - FRAME_TOLERANCE);
}

fn bake_source(width: u32, height: u32) -> std::sync::Arc<gpui::RenderImage> {
    let pixels = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
    std::sync::Arc::new(gpui::RenderImage::new([image::Frame::new(pixels)]))
}

fn baked_size(image: &gpui::RenderImage) -> (i32, i32) {
    let size = image.size(0);
    (size.width.0, size.height.0)
}

#[test]
fn crop_commit_applies_quarter_turn_to_canvas_and_export() {
    let source = bake_source(40, 20);
    let mut state = EditorImageState::empty(None, size(40.0, 20.0));
    state.rotation_steps = 3;
    state.was_edited = true;
    let (baked, committed) = EditorImageBake::commit_canvas(&source, &state).expect("a turn commits");
    assert_eq!(baked_size(&baked), (20, 40));
    assert_eq!(committed.rotation_steps, 3);
    assert!(committed.crop_baked_into_canvas);
    assert!(committed.has_baked_edits());
    let exported = EditorImageBake::render_edited_image(&source, &committed);
    assert_eq!(baked_size(&exported), (20, 40));
}

#[test]
fn crop_commit_applies_free_rotation_to_canvas_and_export() {
    let source = bake_source(40, 20);
    let mut state = EditorImageState::empty(None, size(40.0, 20.0));
    state.free_rotation_degrees = 20.0;
    let (baked, committed) = EditorImageBake::commit_canvas(&source, &state).expect("a turn commits");
    let (width, height) = baked_size(&baked);
    assert!(width > 40 && height > 20, "turned canvas grows: {width}x{height}");
    assert!((committed.free_rotation_degrees - 20.0).abs() < 1e-9);
    let exported = EditorImageBake::render_edited_image(&source, &committed);
    assert_eq!(baked_size(&exported), (width, height));
}
