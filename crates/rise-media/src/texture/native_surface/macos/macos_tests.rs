use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_video::pixel_buffer::{CVPixelBuffer, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange};
use rise_platform::HostOs;

use std::sync::Arc;

use super::{
    BGRA_PIXEL_FORMAT, DRAWABLE_PIXEL_FORMAT, HostFrameGeometry, NativeSurfaceBinding, Reencode,
    ReencodeTables, VtRotation, drawable_buffer, host_bgra_buffer, host_frame_buffer,
    least_recent, rotated_buffer,
};
use crate::texture::color::{ColorRange, ColorSpace, ConversionParams};
use crate::texture::frame::FrameFormat;
use crate::texture::native_surface::*;

fn buffer(format: u32) -> CVPixelBuffer {
    CVPixelBuffer::new(format, 64, 32, None).expect("CVPixelBufferCreate")
}

fn hold(buffer: &CVPixelBuffer) -> Result<RetainedSurface, SurfaceError> {
    unsafe {
        NativeSurfaceBinding.retain(
            HostOs::MacOs,
            buffer.as_concrete_TypeRef().cast(),
        )
    }
}

#[test]
fn holding_a_decoder_buffer_takes_one_reference_and_dropping_releases_it() {
    let decoded = buffer(DRAWABLE_PIXEL_FORMAT);
    let before = decoded.retain_count();

    let held = hold(&decoded).unwrap();
    assert_eq!(decoded.retain_count(), before + 1, "FFmpeg's unref must not free a held frame");
    assert_eq!(held.address(), decoded.as_concrete_TypeRef() as usize);

    drop(held);
    assert_eq!(decoded.retain_count(), before, "a dropped surface must not leak the buffer");
}

#[test]
fn a_full_range_nv12_buffer_is_drawable_by_metal() {
    let decoded = buffer(DRAWABLE_PIXEL_FORMAT);
    let held = hold(&decoded).unwrap();

    let drawable = drawable_buffer(&held).unwrap();
    assert_eq!(drawable.get_width(), 64);
    assert_eq!(drawable.get_height(), 32);
}

#[test]
fn a_video_range_buffer_is_refused_instead_of_tripping_the_renderer_assert() {
    let decoded = buffer(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange);
    let held = hold(&decoded).unwrap();

    assert!(matches!(
        drawable_buffer(&held),
        Err(SurfaceError::WrongKind { .. })
    ));
}

#[test]
fn a_core_foundation_object_that_is_not_a_pixel_buffer_is_refused() {
    let text = CFString::new("not a frame");

    let held = unsafe {
        NativeSurfaceBinding.retain(HostOs::MacOs, text.as_concrete_TypeRef() as *mut _)
    };

    assert_eq!(
        held,
        Err(SurfaceError::WrongKind {
            expected: "a CVPixelBuffer"
        })
    );
}

#[test]
fn a_surface_built_elsewhere_is_not_drawn() {
    let foreign = RetainedSurface::new(0x10, 1_u8);

    assert!(drawable_buffer(&foreign).is_err());
}

#[test]
fn null_is_refused_before_any_core_foundation_call() {
    let held = unsafe { NativeSurfaceBinding.retain(HostOs::MacOs, std::ptr::null_mut()) };

    assert_eq!(held, Err(SurfaceError::Null));
}

fn conversion(space: ColorSpace, range: ColorRange) -> ConversionParams {
    ConversionParams::new(FrameFormat::Nv12, space, range)
}

fn tables(space: ColorSpace, range: ColorRange) -> ReencodeTables {
    let reencode = Reencode::for_metal(&ycbcr_to_rgb_columns(&conversion(space, range))).unwrap();
    ReencodeTables::new(&reencode)
}

fn host_frame(stream_id: u64, generation: u64, width: u32, height: u32) -> HostNv12Frame {
    let luma_stride = width + 3;
    let chroma_stride = 2 * width.div_ceil(2) + 5;
    let luma: Vec<u8> = (0..luma_stride * height).map(|index| (index % 251) as u8).collect();
    let chroma: Vec<u8> = (0..chroma_stride * height.div_ceil(2))
        .map(|index| (index % 241) as u8)
        .collect();
    HostNv12Frame {
        stream_id,
        generation,
        width,
        height,
        luma: Arc::from(luma),
        luma_stride,
        chroma: Arc::from(chroma),
        chroma_stride,
        conversion: conversion(ColorSpace::Bt601, ColorRange::Full),
    }
}

fn plane_rows(buffer: &CVPixelBuffer, plane: usize, rows: usize, row_bytes: usize) -> Vec<Vec<u8>> {
    assert_eq!(buffer.lock_base_address(1), 0);
    let copied = unsafe {
        let base = buffer.get_base_address_of_plane(plane).cast::<u8>();
        let stride = buffer.get_bytes_per_row_of_plane(plane);
        (0..rows)
            .map(|row| std::slice::from_raw_parts(base.add(row * stride), row_bytes).to_vec())
            .collect()
    };
    assert_eq!(buffer.unlock_base_address(1), 0);
    copied
}

#[test]
fn full_range_bt601_is_what_the_metal_surface_matrix_decodes() {
    let reencode =
        Reencode::for_metal(&ycbcr_to_rgb_columns(&conversion(ColorSpace::Bt601, ColorRange::Full)))
            .unwrap();

    assert!(reencode.is_identity(), "{reencode:?}");
}

#[test]
fn other_conversions_are_reencoded() {
    for (space, range) in [
        (ColorSpace::Bt709, ColorRange::Limited),
        (ColorSpace::Bt709, ColorRange::Full),
        (ColorSpace::Bt601, ColorRange::Limited),
        (ColorSpace::Bt2020, ColorRange::Limited),
    ] {
        let reencode =
            Reencode::for_metal(&ycbcr_to_rgb_columns(&conversion(space, range))).unwrap();
        assert!(!reencode.is_identity(), "{space:?} {range:?}");
    }
}

#[test]
fn limited_range_black_and_white_reach_the_ends_of_the_metal_scale() {
    let limited = tables(ColorSpace::Bt709, ColorRange::Limited);

    assert_eq!(limited.code(0, 16, 128, 128), 0);
    assert_eq!(limited.code(0, 235, 128, 128), 255);
    assert_eq!(limited.code(1, 16, 128, 128), 128);
    assert_eq!(limited.code(2, 235, 128, 128), 128);
}

#[test]
fn reencoded_chroma_never_depends_on_luma() {
    let limited = tables(ColorSpace::Bt709, ColorRange::Limited);

    for (cb, cr) in [(60, 200), (128, 128), (240, 16)] {
        assert_eq!(limited.code(1, 16, cb, cr), limited.code(1, 235, cb, cr));
        assert_eq!(limited.code(2, 16, cb, cr), limited.code(2, 235, cb, cr));
    }
}

#[test]
fn reencoded_codes_clamp_instead_of_wrapping() {
    let limited = tables(ColorSpace::Bt709, ColorRange::Limited);

    assert_eq!(limited.code(0, 0, 128, 128), 0);
    assert_eq!(limited.code(0, 255, 128, 128), 255);
}

#[test]
fn planes_shorter_than_their_rows_are_refused() {
    let mut frame = host_frame(1, 1, 5, 3);
    frame.luma = Arc::from(vec![0_u8; 4]);

    assert!(matches!(
        HostFrameGeometry::of(&frame),
        Err(SurfaceError::WrongKind { .. })
    ));
}

#[test]
fn an_empty_frame_is_refused() {
    let frame = host_frame(1, 1, 0, 0);

    assert!(HostFrameGeometry::of(&frame).is_err());
}

#[test]
fn odd_sizes_round_the_chroma_plane_up() {
    let geometry = HostFrameGeometry::of(&host_frame(1, 1, 5, 3)).unwrap();

    assert_eq!((geometry.chroma_width, geometry.chroma_height), (3, 2));
}

#[test]
fn a_full_range_bt601_frame_is_copied_row_by_row_into_a_420f_buffer() {
    let frame = host_frame(7, 1, 5, 3);

    let buffer = host_frame_buffer(&frame).unwrap();

    assert_eq!(buffer.get_pixel_format(), DRAWABLE_PIXEL_FORMAT);
    assert_eq!((buffer.get_width(), buffer.get_height()), (5, 3));
    let luma = plane_rows(&buffer, 0, 3, 5);
    for (row, written) in luma.iter().enumerate() {
        let start = row * frame.luma_stride as usize;
        assert_eq!(written.as_slice(), &frame.luma[start..start + 5]);
    }
    let chroma = plane_rows(&buffer, 1, 2, 6);
    for (row, written) in chroma.iter().enumerate() {
        let start = row * frame.chroma_stride as usize;
        assert_eq!(written.as_slice(), &frame.chroma[start..start + 6]);
    }
}

#[test]
fn an_unchanged_generation_draws_the_same_buffer_without_an_upload() {
    let first = host_frame_buffer(&host_frame(8, 4, 4, 4)).unwrap();
    let again = host_frame_buffer(&host_frame(8, 4, 4, 4)).unwrap();
    let next = host_frame_buffer(&host_frame(8, 5, 4, 4)).unwrap();

    assert_eq!(first.as_concrete_TypeRef(), again.as_concrete_TypeRef());
    assert_ne!(first.as_concrete_TypeRef(), next.as_concrete_TypeRef());
}

#[test]
fn a_limited_range_frame_is_reencoded_for_the_metal_matrix() {
    let mut frame = host_frame(9, 1, 2, 2);
    frame.luma = Arc::from(vec![16_u8, 235, 16, 235]);
    frame.luma_stride = 2;
    frame.chroma = Arc::from(vec![128_u8, 128]);
    frame.chroma_stride = 2;
    frame.conversion = conversion(ColorSpace::Bt709, ColorRange::Limited);

    let buffer = host_frame_buffer(&frame).unwrap();

    assert_eq!(plane_rows(&buffer, 0, 2, 2), vec![vec![0, 255], vec![0, 255]]);
    assert_eq!(plane_rows(&buffer, 1, 1, 2), vec![vec![128, 128]]);
}

#[test]
fn a_cpu_picture_is_prepared_off_the_main_thread_into_a_drawable_surface() {
    let prepared = std::thread::spawn(|| {
        NativeSurfaceBinding.prepare_host_frame(HostOs::MacOs, host_frame(10, 3, 6, 4))
    })
    .join()
    .unwrap()
    .unwrap();

    let PreparedHostFrame::Surface {
        stream_id,
        generation,
        surface,
        ..
    } = &prepared
    else {
        panic!("a macOS CPU picture must come back as a pool surface: {prepared:?}");
    };
    assert_eq!((*stream_id, *generation), (10, 3));
    assert_eq!(prepared.size(), (6, 4));
    let buffer = drawable_buffer(surface).unwrap();
    assert_eq!((buffer.get_width(), buffer.get_height()), (6, 4));
    assert_eq!(surface.address(), buffer.as_concrete_TypeRef() as usize);
}

#[test]
fn an_older_generation_finishing_late_never_replaces_the_newer_shown_buffer() {
    let newer = host_frame_buffer(&host_frame(11, 6, 4, 4)).unwrap();
    let older = host_frame_buffer(&host_frame(11, 5, 4, 4)).unwrap();
    let newer_again = host_frame_buffer(&host_frame(11, 6, 4, 4)).unwrap();

    assert_ne!(newer.as_concrete_TypeRef(), older.as_concrete_TypeRef());
    assert_eq!(newer.as_concrete_TypeRef(), newer_again.as_concrete_TypeRef());
}

fn bgra_frame(stream_id: u64, generation: u64, width: u32, height: u32) -> HostBgraFrame {
    let stride = width * 4 + 8;
    let len = (stride * height) as usize;
    HostBgraFrame {
        stream_id,
        generation,
        width,
        height,
        stride,
        pixels: Arc::from((0..len).map(|byte| (byte % 251) as u8).collect::<Vec<u8>>()),
    }
}

fn bgra_rows(buffer: &CVPixelBuffer, rows: usize, row_bytes: usize) -> Vec<Vec<u8>> {
    assert_eq!(buffer.lock_base_address(1), 0);
    let base = unsafe { buffer.get_base_address() }.cast::<u8>();
    let stride = buffer.get_bytes_per_row();
    let copied = (0..rows)
        .map(|row| {
            unsafe { std::slice::from_raw_parts(base.add(row * stride), row_bytes) }.to_vec()
        })
        .collect();
    assert_eq!(buffer.unlock_base_address(1), 0);
    copied
}

#[test]
fn a_bgra_frame_is_copied_row_by_row_into_a_32bgra_buffer_without_its_stride_padding() {
    let frame = bgra_frame(101, 1, 3, 2);

    let buffer = host_bgra_buffer(&frame).unwrap();

    assert_eq!(buffer.get_pixel_format(), BGRA_PIXEL_FORMAT);
    assert_eq!((buffer.get_width(), buffer.get_height()), (3, 2));
    for (row, written) in bgra_rows(&buffer, 2, 12).iter().enumerate() {
        let start = row * frame.stride as usize;
        assert_eq!(written.as_slice(), &frame.pixels[start..start + 12]);
    }
}

#[test]
fn an_unchanged_bgra_generation_draws_the_same_buffer_without_a_copy() {
    let first = host_bgra_buffer(&bgra_frame(102, 4, 4, 4)).unwrap();
    let again = host_bgra_buffer(&bgra_frame(102, 4, 4, 4)).unwrap();
    let next = host_bgra_buffer(&bgra_frame(102, 5, 4, 4)).unwrap();
    let resized = host_bgra_buffer(&bgra_frame(102, 5, 8, 4)).unwrap();

    assert_eq!(first.as_concrete_TypeRef(), again.as_concrete_TypeRef());
    assert_ne!(first.as_concrete_TypeRef(), next.as_concrete_TypeRef());
    assert_eq!(resized.get_width(), 8, "a new size is a new picture");
}

#[test]
fn bgra_rows_shorter_than_they_claim_are_refused_before_any_pool_call() {
    let mut frame = bgra_frame(103, 1, 4, 4);
    frame.pixels = Arc::from(vec![0_u8; 10]);

    assert!(matches!(
        host_bgra_buffer(&frame),
        Err(SurfaceError::WrongKind { .. })
    ));
}

#[test]
fn a_prepared_bgra_frame_is_a_surface_metal_can_draw() {
    let prepared = NativeSurfaceBinding
        .prepare_host_bgra_frame(HostOs::MacOs, bgra_frame(104, 2, 2, 2))
        .unwrap();

    let PreparedHostFrame::Surface { stream_id, generation, surface, .. } = prepared else {
        panic!("macOS prepares a BGRA picture into a pool surface");
    };
    assert_eq!((stream_id, generation), (104, 2));
    assert_eq!(drawable_buffer(&surface).unwrap().get_pixel_format(), BGRA_PIXEL_FORMAT);
}

#[test]
fn the_least_recently_used_slot_goes_first() {
    assert_eq!(least_recent([5_u64, 2, 9].into_iter()), Some(1));
    assert_eq!(least_recent(std::iter::empty()), None);
}

#[test]
fn quarter_turns_pick_the_clockwise_videotoolbox_rotation() {
    assert_eq!(VtRotation::of_clockwise_quarter_turns(0), None);
    assert_eq!(VtRotation::of_clockwise_quarter_turns(1), Some(VtRotation::Clockwise90));
    assert_eq!(VtRotation::of_clockwise_quarter_turns(2), Some(VtRotation::Half));
    assert_eq!(VtRotation::of_clockwise_quarter_turns(3), Some(VtRotation::CounterClockwise90));
    assert_eq!(VtRotation::of_clockwise_quarter_turns(5), Some(VtRotation::Clockwise90));
    assert_eq!(VtRotation::of_clockwise_quarter_turns(4), None);
}

#[test]
fn no_turn_draws_the_surface_itself() {
    let decoded = buffer(DRAWABLE_PIXEL_FORMAT);
    let held = hold(&decoded).unwrap();

    let drawn = rotated_buffer(&held, 0).unwrap();

    assert_eq!(drawn.as_concrete_TypeRef(), decoded.as_concrete_TypeRef());
}

#[test]
fn a_quarter_turn_swaps_the_sides_and_keeps_the_format() {
    let decoded = buffer(DRAWABLE_PIXEL_FORMAT);
    let held = hold(&decoded).unwrap();

    let quarter = rotated_buffer(&held, 1).unwrap();
    let half = rotated_buffer(&held, 2).unwrap();

    assert_eq!((quarter.get_width(), quarter.get_height()), (32, 64));
    assert_eq!(quarter.get_pixel_format(), DRAWABLE_PIXEL_FORMAT);
    assert_eq!((half.get_width(), half.get_height()), (64, 32));
}

#[test]
fn a_redraw_of_the_same_picture_and_turn_reuses_the_turned_buffer() {
    let decoded = buffer(DRAWABLE_PIXEL_FORMAT);
    let held = hold(&decoded).unwrap();

    let first = rotated_buffer(&held, 3).unwrap();
    let again = rotated_buffer(&held, 3).unwrap();
    let other_turn = rotated_buffer(&held, 1).unwrap();

    assert_eq!(first.as_concrete_TypeRef(), again.as_concrete_TypeRef());
    assert_ne!(first.as_concrete_TypeRef(), other_turn.as_concrete_TypeRef());
}

#[test]
fn a_turned_bgra_surface_matches_the_hub_turn_of_the_same_picture() {
    let frame = bgra_frame(105, 1, 8, 4);
    let prepared = NativeSurfaceBinding
        .prepare_host_bgra_frame(HostOs::MacOs, frame.clone())
        .unwrap();
    let PreparedHostFrame::Surface { surface, .. } = prepared else {
        panic!("macOS prepares a BGRA picture into a pool surface");
    };

    for turns in 1..4_u8 {
        let turned = rotated_buffer(&surface, turns).unwrap();
        let expected = rotate_host_bgra_frame(&frame, turns).unwrap();

        assert_eq!(turned.get_pixel_format(), BGRA_PIXEL_FORMAT);
        assert_eq!(
            (turned.get_width(), turned.get_height()),
            (expected.width as usize, expected.height as usize)
        );
        let row_bytes = expected.width as usize * 4;
        for (row, written) in bgra_rows(&turned, expected.height as usize, row_bytes).iter().enumerate() {
            let start = row * expected.stride as usize;
            assert_eq!(
                written.as_slice(),
                &expected.pixels[start..start + row_bytes],
                "row {row} after {turns} clockwise quarter turns"
            );
        }
    }
}

#[test]
fn present_rotated_draws_a_held_surface_and_refuses_a_foreign_one() {
    let decoded = buffer(DRAWABLE_PIXEL_FORMAT);
    let held = hold(&decoded).unwrap();
    let foreign = RetainedSurface::new(1, ());

    assert!(NativeSurfaceBinding.present_rotated(HostOs::MacOs, &held, 1).is_ok());
    assert!(matches!(
        NativeSurfaceBinding.present_rotated(HostOs::MacOs, &foreign, 1),
        Err(SurfaceError::WrongKind { .. })
    ));
}
