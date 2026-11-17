use std::sync::Arc;

use gpui::{AppContext, Edges, Entity, RenderImage, Size, TestAppContext, bounds, point, px, size};
use image::{Frame, Rgba, RgbaImage};

use crate::modules::media_picker::editor::editor_image_stage::EditorImageStageUIView;
use crate::modules::media_picker::editor::editor_mode::EditorMode;
use crate::modules::media_picker::editor::editor_state::EditorImageState;

fn make_test_image(width: u32, height: u32) -> Arc<RenderImage> {
    let pixels = RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]));
    Arc::new(RenderImage::new([Frame::new(pixels)]))
}

fn relayout(stage: &Entity<EditorImageStageUIView>, cx: &mut TestAppContext) {
    stage.update(cx, |stage, cx| {
        stage.set_needs_layout(cx);
        stage.layout_if_needed(cx);
    });
}

fn base_size(stage: &Entity<EditorImageStageUIView>, cx: &mut TestAppContext) -> Size<f32> {
    stage.read_with(cx, |stage, _| stage.image_container_size())
}

fn zoom(stage: &Entity<EditorImageStageUIView>, cx: &mut TestAppContext) -> f32 {
    stage.read_with(cx, |stage, _| stage.zoom_scale())
}

fn expect_close(actual: Size<f32>, expected: Size<f32>, tolerance: f32) {
    assert!((actual.width - expected.width).abs() < tolerance);
    assert!((actual.height - expected.height).abs() < tolerance);
}

fn make_crop_stage_fixture(
    cx: &mut TestAppContext,
) -> (Entity<EditorImageStageUIView>, EditorImageState) {
    let image = make_test_image(120, 80);
    let state = EditorImageState::empty(Some("zoom-view-invariant".to_string()), size(120.0, 80.0));
    let stage = cx.new(EditorImageStageUIView::new);
    stage.update(cx, |stage, cx| {
        stage.set_stage_bounds(bounds(point(px(0.0), px(0.0)), size(px(390.0), px(700.0))), cx);
        stage.update_side_insets(Edges { top: 62.0, left: 14.0, bottom: 174.0, right: 14.0 }, false, cx);
        stage.set(image, state.clone(), EditorMode::Crop, cx);
    });
    relayout(&stage, cx);

    cx.run_until_parked();
    relayout(&stage, cx);

    let zoom_view = base_size(&stage, cx);
    assert!(zoom_view.width > 0.0);
    assert!(zoom_view.height > 0.0);

    (stage, state)
}

#[gpui::test]
fn crop_stage_layout_at_zoom_does_not_change_zoom_view_base_bounds(cx: &mut TestAppContext) {
    let (stage, _) = make_crop_stage_fixture(cx);
    let base_bounds = base_size(&stage, cx);

    stage.update(cx, |stage, cx| stage.set_zoom_scale(2.5, false, cx));
    assert!((zoom(&stage, cx) - 2.5).abs() < 0.001);
    expect_close(base_size(&stage, cx), base_bounds, 0.001);

    relayout(&stage, cx);
    expect_close(base_size(&stage, cx), base_bounds, 0.001);

    relayout(&stage, cx);
    expect_close(base_size(&stage, cx), base_bounds, 0.001);
    assert!((zoom(&stage, cx) - 2.5).abs() < 0.001);
}

#[gpui::test]
fn high_quality_bitmap_upgrade_at_zoom_is_geometry_idempotent(cx: &mut TestAppContext) {
    let (stage, state) = make_crop_stage_fixture(cx);
    let base_bounds = base_size(&stage, cx);
    let replacement = make_test_image(480, 320);

    stage.update(cx, |stage, cx| stage.set_zoom_scale(2.25, false, cx));
    assert!((zoom(&stage, cx) - 2.25).abs() < 0.001);

    for _ in 0..2 {
        let image = Arc::clone(&replacement);
        let state = state.clone();
        stage.update(cx, |stage, cx| stage.set(image, state, EditorMode::Crop, cx));
        relayout(&stage, cx);
        expect_close(base_size(&stage, cx), base_bounds, 0.001);
        assert!((zoom(&stage, cx) - 2.25).abs() < 0.001);
    }
}

#[gpui::test]
fn unanimated_zoom_during_a_zoom_animation_clears_is_zooming(cx: &mut TestAppContext) {
    let (stage, _) = make_crop_stage_fixture(cx);

    stage.update(cx, |stage, cx| stage.set_zoom_scale(2.5, true, cx));
    assert!(stage.read_with(cx, |stage, _| stage.is_zooming()));

    stage.update(cx, |stage, cx| stage.set_zoom_scale(1.5, false, cx));
    assert!(!stage.read_with(cx, |stage, _| stage.is_zooming()));
    assert!((zoom(&stage, cx) - 1.5).abs() < 0.001);
}

#[test]
fn rotated_clamp_jitter_is_not_an_offset_move() {
    let offset = point(412.337_f32, 198.004_f32);
    let angle = 7.3_f32.to_radians();
    let (u, v) = (point(angle.cos(), angle.sin()), point(-angle.sin(), angle.cos()));
    let center_u = offset.x * u.x + offset.y * u.y;
    let center_v = offset.x * v.x + offset.y * v.y;
    let round_trip = point(u.x * center_u + v.x * center_v, u.y * center_u + v.y * center_v);
    assert!(!EditorImageStageUIView::content_offset_moved(offset, round_trip));
    assert!(!EditorImageStageUIView::content_offset_moved(offset, offset));
    assert!(EditorImageStageUIView::content_offset_moved(offset, point(offset.x + 0.5, offset.y)));
    assert!(EditorImageStageUIView::content_offset_moved(offset, point(offset.x, offset.y - 1.0)));
}
