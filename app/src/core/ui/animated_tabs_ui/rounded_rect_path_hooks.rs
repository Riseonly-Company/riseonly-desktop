use std::f32::consts::PI;

use gpui::{Path, PathBuilder, Pixels, Point, point, px};

const CORNER_STEPS: usize = 8;

/// filled rect with fully rounded corners as one outline so a path swap can move it
pub fn rounded_rect_path(x: f32, y: f32, width: f32, height: f32, origin: Point<Pixels>) -> Option<Path<Pixels>> {
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let radius = width.min(height) / 2.0;
    let steps = CORNER_STEPS;
    let corners = [
        (x + width - radius, y + radius, -PI / 2.0),
        (x + width - radius, y + height - radius, 0.0),
        (x + radius, y + height - radius, PI / 2.0),
        (x + radius, y + radius, PI),
    ];
    let mut points = Vec::with_capacity(corners.len() * (steps + 1));
    for (center_x, center_y, start) in corners {
        for step in 0..=steps {
            let angle = start + (PI / 2.0) * step as f32 / steps as f32;
            points.push(point(
                origin.x + px(center_x + radius * angle.cos()),
                origin.y + px(center_y + radius * angle.sin()),
            ));
        }
    }
    let mut builder = PathBuilder::fill();
    builder.add_polygon(&points, true);
    builder.build().ok()
}
