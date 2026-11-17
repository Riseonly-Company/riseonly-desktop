use gpui::Bounds;

/// where the editor stage drew the image, in stage coordinates
/// declared beside the crop models so the geometry update doesnt tie the model layer to the stage view
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EditorImageRect {
    pub rect: Bounds<f32>,
    pub viewport_rect: Bounds<f32>,
    pub stage_rect: Bounds<f32>,
    pub available_rect: Bounds<f32>,
    pub zoom: f32,
}
