use image::{Rgba, RgbaImage};

use super::camera_capture::{CameraCaptureError, CameraFrame};
use crate::texture::frame::{FrameFormat, PlaneLayout};

/// the frame as an rgba picture, mirrored when the frame preview is mirrored
/// yuv planes read as bt.601 limited range like webcams state
/// same as yuyv_to_bgra for the preview
pub fn still_rgba_of(frame: &CameraFrame) -> Result<RgbaImage, CameraCaptureError> {
    let geometry = &frame.geometry;
    let (width, height) = (geometry.width(), geometry.height());
    if width == 0 || height == 0 {
        return Err(CameraCaptureError::BadFrame("still frame geometry"));
    }
    if (frame.pixels.len() as u64) < geometry.byte_len() {
        return Err(CameraCaptureError::BadFrame("still frame is shorter than its geometry"));
    }
    let pixels = &frame.pixels[..];
    let planes = geometry.planes();
    let mut image = RgbaImage::new(width, height);
    match geometry.format() {
        FrameFormat::Bgra8 => {
            let plane = plane_of(planes, 0)?;
            for y in 0..height {
                for x in 0..width {
                    let at = sample_at(plane, x, y);
                    let [b, g, r, a] = [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]];
                    image.put_pixel(x, y, Rgba([r, g, b, a]));
                }
            }
        }
        FrameFormat::Nv12 => {
            let (luma, chroma) = (plane_of(planes, 0)?, plane_of(planes, 1)?);
            for y in 0..height {
                for x in 0..width {
                    let uv = sample_at(chroma, x / 2, y / 2);
                    let rgb = bt601(pixels[sample_at(luma, x, y)], pixels[uv], pixels[uv + 1]);
                    image.put_pixel(x, y, opaque(rgb));
                }
            }
        }
        FrameFormat::P010 => {
            let (luma, chroma) = (plane_of(planes, 0)?, plane_of(planes, 1)?);
            for y in 0..height {
                for x in 0..width {
                    let uv = sample_at(chroma, x / 2, y / 2);
                    let rgb = bt601(high_byte(pixels, sample_at(luma, x, y)), high_byte(pixels, uv), high_byte(pixels, uv + 2));
                    image.put_pixel(x, y, opaque(rgb));
                }
            }
        }
        FrameFormat::I420 => {
            let (luma, u, v) = (plane_of(planes, 0)?, plane_of(planes, 1)?, plane_of(planes, 2)?);
            for y in 0..height {
                for x in 0..width {
                    let rgb = bt601(
                        pixels[sample_at(luma, x, y)],
                        pixels[sample_at(u, x / 2, y / 2)],
                        pixels[sample_at(v, x / 2, y / 2)],
                    );
                    image.put_pixel(x, y, opaque(rgb));
                }
            }
        }
    }
    if frame.mirrored {
        image::imageops::flip_horizontal_in_place(&mut image);
    }
    Ok(image)
}

fn plane_of(planes: &[PlaneLayout], index: usize) -> Result<&PlaneLayout, CameraCaptureError> {
    planes.get(index).ok_or(CameraCaptureError::BadFrame("still frame lacks a plane"))
}

/// byte where sample (x, y) of a plane starts
/// the geometry check above keeps it in bounds
fn sample_at(plane: &PlaneLayout, x: u32, y: u32) -> usize {
    plane.offset as usize + y as usize * plane.stride as usize + x as usize * plane.bytes_per_sample as usize
}

/// p010 keeps ten bits in the top of a little endian u16, so the high byte is the 8 bit value
fn high_byte(pixels: &[u8], at: usize) -> u8 {
    pixels[at + 1]
}

fn bt601(y: u8, u: u8, v: u8) -> [u8; 3] {
    let c = (i32::from(y) - 16).max(0) * 298;
    let (u, v) = (i32::from(u) - 128, i32::from(v) - 128);
    let channel = |value: i32| (value >> 8).clamp(0, 255) as u8;
    [channel(c + 409 * v + 128), channel(c - 100 * u - 208 * v + 128), channel(c + 516 * u + 128)]
}

fn opaque([r, g, b]: [u8; 3]) -> Rgba<u8> {
    Rgba([r, g, b, 255])
}
