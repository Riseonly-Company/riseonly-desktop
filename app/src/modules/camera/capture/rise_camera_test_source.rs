#![cfg(debug_assertions)]

use std::f32::consts::PI;
use std::io::Cursor;
use std::path::Path;

use gpui::{
    App, Bounds, Corners, IntoElement, RenderOnce, Rgba, Window, canvas, fill, hsla, linear_color_stop,
    linear_gradient, point, prelude::*, px, size,
};
use image::{DynamicImage, ImageFormat, RgbaImage};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_url::Url;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::video_export::FrameVideoWriter;

use crate::core::ui::clever_image_disk_cache::CleverImageBitmapDecoder;
use crate::modules::camera::capture::rise_camera_capture::{RiseCameraCapture, RiseCameraShot};

/// debug only stand in for a machine with no camera
/// moving test picture for the preview, a still for a photo and a short clip for a video never
/// in release
pub enum RiseCameraTestSource {}

impl RiseCameraTestSource {
    pub const PHOTO_SIZE: (u32, u32) = (1080, 1920);
    pub const VIDEO_SIZE: (u32, u32) = (720, 1280);
    const FRAMES_PER_SECOND: u32 = 15;
    const VIDEO_AVERAGE_BIT_RATE: u32 = 2_000_000;

    pub fn colors(front: bool) -> (Rgba, Rgba) {
        if front {
            (Rgba { r: 0.95, g: 0.45, b: 0.25, a: 1.0 }, Rgba { r: 0.55, g: 0.15, b: 0.6, a: 1.0 })
        } else {
            (Rgba { r: 0.15, g: 0.55, b: 0.95, a: 1.0 }, Rgba { r: 0.1, g: 0.8, b: 0.55, a: 1.0 })
        }
    }

    pub fn frame(frame_size: (u32, u32), phase: f32, front: bool) -> RgbaImage {
        let (top, bottom) = Self::colors(front);
        let (width, height) = (frame_size.0 as f32, frame_size.1 as f32);
        let side = width * 0.32;
        let left = (width - side) / 2.0 + (phase * PI * 2.0).sin() * width * 0.25;
        let upper = (height - side) / 2.0 + (phase * PI * 2.0).cos() * height * 0.15;
        let (center_x, center_y, radius) = (left + side / 2.0, upper + side / 2.0, side / 2.0);
        let diagonal = width * width + height * height;
        let disc_alpha = 0.85_f32;
        RgbaImage::from_fn(frame_size.0, frame_size.1, |x, y| {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let t = ((fx * width + fy * height) / diagonal).clamp(0.0, 1.0);
            let mut channels = [
                top.r + (bottom.r - top.r) * t,
                top.g + (bottom.g - top.g) * t,
                top.b + (bottom.b - top.b) * t,
            ];
            let (dx, dy) = (fx - center_x, fy - center_y);
            if dx * dx + dy * dy <= radius * radius {
                for channel in &mut channels {
                    *channel = *channel * (1.0 - disc_alpha) + disc_alpha;
                }
            }
            let [r, g, b] = channels.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
            image::Rgba([r, g, b, 255])
        })
    }

    pub fn photo(front: bool) -> Option<RiseCameraShot> {
        let image = Self::frame(Self::PHOTO_SIZE, 0.1, front);
        let mut data = Vec::new();
        DynamicImage::ImageRgba8(image.clone())
            .to_rgb8()
            .write_to(&mut Cursor::new(&mut data), ImageFormat::Jpeg)
            .ok()?;
        let path = RiseCameraCapture::temporary_url("jpg");
        RiseCameraCapture::write_atomically(&path, &data).ok()?;
        let url = Url::from_file_path(&path)?;
        let pixel_size = size(Self::PHOTO_SIZE.0 as f32, Self::PHOTO_SIZE.1 as f32);
        Some(RiseCameraShot::new(url, Some(CleverImageBitmapDecoder::render_image(image)), pixel_size, 0.0))
    }

    pub async fn video(seconds: TimeInterval, front: bool) -> Option<RiseCameraShot> {
        let path = RiseCameraCapture::temporary_url("mov");
        let url = Url::from_file_path(&path)?;
        let written = RiseAccountRuntime::current()
            .spawn_blocking(move || Self::write_clip(&path, seconds, front))
            .await
            .unwrap_or(false);
        if !written {
            return None;
        }
        RiseCameraCapture::video_shot(url).await
    }

    fn write_clip(path: &Path, seconds: TimeInterval, front: bool) -> bool {
        let Ok(mut writer) = FrameVideoWriter::create(
            path,
            Self::VIDEO_SIZE.0,
            Self::VIDEO_SIZE.1,
            Self::FRAMES_PER_SECOND,
            Self::VIDEO_AVERAGE_BIT_RATE,
        ) else {
            return false;
        };
        let fps = Self::FRAMES_PER_SECOND as f64;
        let frames = ((seconds * fps) as i64).max(1);
        for index in 0..frames {
            let phase = index as f32 / (Self::FRAMES_PER_SECOND * 3) as f32;
            let image = Self::frame(Self::VIDEO_SIZE, phase, front);
            let _ = writer.append(&image, index);
        }
        writer.finish(frames as f64 / fps).is_ok()
    }
}

/// moving test picture in place of the live preview
#[derive(IntoElement)]
pub struct RiseCameraTestPreview {
    pub front: bool,
}

impl RiseCameraTestPreview {
    const LOOP_SECONDS: f64 = 3.0;
}

impl RenderOnce for RiseCameraTestPreview {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        window.request_animation_frame();
        let seconds = RiseClock::monotonic().as_secs_f64();
        let phase = ((seconds % Self::LOOP_SECONDS) / Self::LOOP_SECONDS) as f32;
        let (top, bottom) = RiseCameraTestSource::colors(self.front);
        canvas(
            |_, _, _| (),
            move |bounds: Bounds<gpui::Pixels>, (), window: &mut Window, _: &mut App| {
                window.paint_quad(fill(
                    bounds,
                    linear_gradient(135.0, linear_color_stop(top, 0.0), linear_color_stop(bottom, 1.0)),
                ));
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                let side = width * 0.32;
                let center_x = width / 2.0 + (phase * PI * 2.0).sin() * width * 0.25;
                let center_y = height / 2.0 + (phase * PI * 2.0).cos() * height * 0.15;
                let disc = Bounds::new(
                    point(
                        bounds.origin.x + px(center_x - side / 2.0),
                        bounds.origin.y + px(center_y - side / 2.0),
                    ),
                    size(px(side), px(side)),
                );
                window.paint_quad(fill(disc, hsla(0.0, 0.0, 1.0, 0.85)).corner_radii(Corners::all(px(side / 2.0))));
            },
        )
        .size_full()
    }
}
