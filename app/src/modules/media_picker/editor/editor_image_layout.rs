use gpui::{App, Canvas, Edges, ElementId, Size, Window, size};
use rise_ui::expandable_text_ui::EdgeInsets;
use rise_ui::swiftui::measure::measure;

use crate::modules::media_picker::editor::editor_mode::EditorMode;

pub enum EditorMainChromeHeightKey {}

impl EditorMainChromeHeightKey {
    pub fn default_value() -> f32 {
        0.0
    }

    pub fn reduce(value: &mut f32, next_value: f32) {
        *value = value.max(next_value);
    }
}

pub enum EditorCropChromeHeightKey {}

impl EditorCropChromeHeightKey {
    pub fn default_value() -> f32 {
        0.0
    }

    pub fn reduce(value: &mut f32, next_value: f32) {
        *value = value.max(next_value);
    }
}

/// child of the chrome that reports the chrome height after layout and on every change
/// the editor folds it in with the height key reducer
pub fn editor_main_chrome_height_reader(
    id: impl Into<ElementId>,
    on_change: impl FnOnce(f32, &mut Window, &mut App) + 'static,
) -> Canvas<()> {
    measure(id, move |bounds, window, cx| on_change(f32::from(bounds.size.height), window, cx))
}

/// crop chrome twin of editor_main_chrome_height_reader
pub fn editor_crop_chrome_height_reader(
    id: impl Into<ElementId>,
    on_change: impl FnOnce(f32, &mut Window, &mut App) + 'static,
) -> Canvas<()> {
    measure(id, move |bounds, window, cx| on_change(f32::from(bounds.size.height), window, cx))
}

pub enum EditorImageLayout {}

impl EditorImageLayout {
    pub const GEOMETRY_EPSILON_DEGREES: f64 = 0.0001;
    pub const TOOLBAR_CLEARANCE: f32 = 8.0;
    pub const CROP_VIEWPORT_INSET: f32 = 9.0;

    /// callers without a free rotation pass 0.0, the default of free_rotation_degrees
    pub fn oriented_display_size(source: Size<f32>, rotation_steps: i64, free_rotation_degrees: f64) -> Size<f32> {
        let passes = source.width > 0.0 && source.height > 0.0;
        if !passes {
            return source;
        }

        let mut w = source.width;
        let mut h = source.height;
        if rotation_steps % 2 != 0 {
            std::mem::swap(&mut w, &mut h);
        }

        let passes = free_rotation_degrees.abs() > Self::GEOMETRY_EPSILON_DEGREES;
        if !passes {
            return size(w, h);
        }
        let angle = free_rotation_degrees.abs() * std::f64::consts::PI / 180.0;

        let c = angle.cos().abs() as f32;
        let s = angle.sin().abs() as f32;
        size(w * c + h * s, w * s + h * c)
    }

    pub fn fallback_toolbar_height(mode: EditorMode) -> f32 {
        match mode {
            EditorMode::Initial => 146.0,
            EditorMode::Crop => 202.0,
            EditorMode::Draw => 178.0,
            EditorMode::Adjust => 280.0,
            EditorMode::Text { .. } => 0.0,
            EditorMode::TagPeople => 100.0,
        }
    }

    /// callers without side safe areas pass 0.0 for safe_left and safe_right
    #[allow(clippy::too_many_arguments)]
    pub fn compute_insets(
        stage_size: Size<f32>,
        image_size: Size<f32>,
        safe_top: f32,
        safe_bottom: f32,
        safe_left: f32,
        safe_right: f32,
        mode: EditorMode,
        toolbar_height: f32,
    ) -> Edges<f32> {
        let is_crop = mode == EditorMode::Crop;
        let crop_inset = if is_crop { Self::CROP_VIEWPORT_INSET } else { 0.0 };
        let left_pad = if is_crop { safe_left + crop_inset } else { 0.0 };
        let right_pad = if is_crop { safe_right + crop_inset } else { 0.0 };
        let top_pad = (safe_top + crop_inset).max(12.0);

        if let EditorMode::Text { .. } = mode {
            return Edges { top: top_pad, left: 0.0, bottom: 0.0, right: 0.0 };
        }

        let toolbar_h = toolbar_height.max(Self::fallback_toolbar_height(mode));
        let toolbar_top = stage_size.height - toolbar_h - safe_bottom;
        let mut bottom = toolbar_h + safe_bottom + crop_inset;
        let avail_w = (stage_size.width - left_pad - right_pad).max(0.0);

        for _ in 0..16 {
            let avail_h = stage_size.height - top_pad - bottom;
            let passes = avail_h > 1.0 && avail_w > 1.0 && image_size.width > 0.0 && image_size.height > 0.0;
            if !passes {
                break;
            }

            let fit = Self::aspect_fit(image_size, size(avail_w, avail_h));
            let image_bottom = top_pad + (avail_h - fit.height) / 2.0 + fit.height;

            if image_bottom <= toolbar_top - Self::TOOLBAR_CLEARANCE {
                break;
            }
            bottom += (image_bottom - toolbar_top + Self::TOOLBAR_CLEARANCE).ceil();
        }

        Edges { top: top_pad, left: left_pad, bottom, right: right_pad }
    }

    /// raises the reported insets to the safe area
    /// a desktop window has no safe area beyond what layout reports
    /// so each edge is the reported inset floored at zero
    pub fn resolved_safe_area_insets(reported: EdgeInsets) -> Edges<f32> {
        Edges {
            top: reported.top.max(0.0),
            left: reported.leading.max(0.0),
            bottom: reported.bottom.max(0.0),
            right: reported.trailing.max(0.0),
        }
    }

    pub fn aspect_fit(image_size: Size<f32>, box_size: Size<f32>) -> Size<f32> {
        let passes = image_size.width > 0.0 && image_size.height > 0.0 && box_size.width > 0.0 && box_size.height > 0.0;
        if !passes {
            return Size::default();
        }
        let scale = (box_size.width / image_size.width).min(box_size.height / image_size.height);
        size(image_size.width * scale, image_size.height * scale)
    }
}
