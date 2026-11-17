use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};

use crate::core::ui::animated_gif_view::AnimatedImagePlaybackPolicy;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder};

fn png_data(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    let bitmap = RgbaImage::from_pixel(width, height, Rgba(color));
    let mut data = Vec::new();
    DynamicImage::ImageRgba8(bitmap)
        .write_to(&mut Cursor::new(&mut data), ImageFormat::Png)
        .expect("png encode of a solid bitmap");
    data
}

fn jpeg_data(width: u32, height: u32, color: [u8; 3], quality: u8) -> Vec<u8> {
    let bitmap = RgbImage::from_pixel(width, height, Rgb(color));
    let mut data = Vec::new();
    JpegEncoder::new_with_quality(&mut data, quality)
        .encode_image(&bitmap)
        .expect("jpeg encode of a solid bitmap");
    data
}

fn pixel_size(image: &CleverImage) -> (i32, i32) {
    let size = image.size(0);
    (size.width.0, size.height.0)
}

fn bytes_per_row(image: &CleverImage) -> usize {
    let (_, height) = pixel_size(image);
    let rows = usize::try_from(height.max(1)).unwrap_or(1);
    image.as_bytes(0).map_or(0, |bytes| bytes.len() / rows)
}

#[test]
fn eagerly_creates_displayable_bitmap() {
    let data = png_data(320, 180, [175, 82, 222, 255]);

    let image = CleverImageBitmapDecoder::decode(&data, None).expect("decoded bitmap");
    let (width, height) = pixel_size(&image);

    assert_eq!(width, 320);
    assert_eq!(height, 180);
    assert!(bytes_per_row(&image) > 0);
}

#[test]
fn downsample_limit_preserves_aspect_ratio() {
    let data = jpeg_data(400, 200, [0, 122, 255], 90);

    let image = CleverImageBitmapDecoder::decode(&data, Some(100)).expect("decoded bitmap");
    let (width, height) = pixel_size(&image);

    assert_eq!(width, 100);
    assert_eq!(height, 50);
}

#[test]
fn individual_frame_decode_is_eager_and_bounded() {
    let data = png_data(600, 300, [255, 149, 0, 255]);

    let image = CleverImageBitmapDecoder::decode_frame(&data, 0, 120).expect("decoded frame");
    let (width, height) = pixel_size(&image);
    assert_eq!(width, 120);
    assert_eq!(height, 60);
    assert!(bytes_per_row(&image) > 0);
    assert!(CleverImageBitmapDecoder::decode_frame(&data, 1, 120).is_none());
}

#[test]
fn animated_frame_timing_is_capped_for_inline_playback() {
    assert_eq!(
        AnimatedImagePlaybackPolicy::normalized_frame_duration(None),
        AnimatedImagePlaybackPolicy::DEFAULT_FRAME_DURATION
    );
    assert_eq!(
        AnimatedImagePlaybackPolicy::normalized_frame_duration(Some(0.001)),
        AnimatedImagePlaybackPolicy::MINIMUM_FRAME_DURATION
    );
    assert_eq!(AnimatedImagePlaybackPolicy::normalized_frame_duration(Some(20.0)), 1.0);
}
