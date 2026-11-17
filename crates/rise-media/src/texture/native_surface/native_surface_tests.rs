use crate::texture::native_surface::*;
use rise_platform::HostOs;

#[test]
fn a_null_surface_is_never_held_on_any_host() {
    let held = unsafe { retain_decoder_surface(std::ptr::null_mut()) };

    assert!(held.is_err(), "a null surface must not produce a handle");
}

#[test]
fn every_host_names_what_is_missing_for_a_zero_copy_surface() {
    for host in [HostOs::MacOs, HostOs::Linux, HostOs::Windows] {
        assert!(!unwired_reason(host).is_empty(), "{host:?}");
    }
    assert!(unwired_reason(HostOs::Linux).contains("P27 lin"));
    for host in [HostOs::MacOs, HostOs::Linux, HostOs::Windows] {
        assert!(!host_frame_unwired_reason(host).is_empty(), "{host:?}");
    }
}

fn apply(columns: &[[f32; 4]; 4], yuv: [f32; 3]) -> [f32; 3] {
    let input = [yuv[0], yuv[1], yuv[2], 1.0];
    let mut rgb = [0.0_f32; 3];
    for (channel, value) in rgb.iter_mut().enumerate() {
        *value = (0..4).map(|column| columns[column][channel] * input[column]).sum();
    }
    rgb
}

fn close(left: [f32; 3], right: [f32; 3]) -> bool {
    left.iter().zip(right).all(|(a, b)| (a - b).abs() < 1e-3)
}

#[test]
fn the_shader_matrix_maps_limited_bt709_black_white_and_red_like_the_conversion_params() {
    use crate::texture::color::{ColorRange, ColorSpace, ConversionParams};
    use crate::texture::frame::FrameFormat;

    let params = ConversionParams::new(FrameFormat::Nv12, ColorSpace::Bt709, ColorRange::Limited);
    let columns = ycbcr_to_rgb_columns(&params);

    assert!(close(apply(&columns, [16.0 / 255.0, 0.5, 0.5]), [0.0, 0.0, 0.0]));
    assert!(close(apply(&columns, [235.0 / 255.0, 0.5, 0.5]), [1.0, 1.0, 1.0]));
    let red = apply(&columns, [63.0 / 255.0, 102.0 / 255.0, 240.0 / 255.0]);
    assert!(red[0] > 0.98 && red[1] < 0.02 && red[2] < 0.02, "{red:?}");
    assert_eq!(columns[3][3], 1.0);
    assert_eq!([columns[0][3], columns[1][3], columns[2][3]], [0.0, 0.0, 0.0]);
}

#[test]
fn surfaces_are_equal_by_address_and_keep_their_holder() {
    let first = RetainedSurface::new(0x1000, 7_u32);
    let second = RetainedSurface::new(0x1000, 8_u32);

    assert_eq!(first, second);
    assert_eq!(first.clone().keeper::<u32>(), Some(&7));
    assert_eq!(first.keeper::<u64>(), None);
    assert_eq!(format!("{first:?}"), "RetainedSurface(0x1000)");
}

fn small_frame(stream_id: u64, generation: u64) -> HostNv12Frame {
    use crate::texture::color::{ColorRange, ColorSpace, ConversionParams};
    use crate::texture::frame::FrameFormat;

    HostNv12Frame {
        stream_id,
        generation,
        width: 2,
        height: 2,
        luma: std::sync::Arc::from(vec![16_u8, 64, 128, 235]),
        luma_stride: 2,
        chroma: std::sync::Arc::from(vec![128_u8, 128]),
        chroma_stride: 2,
        conversion: ConversionParams::new(FrameFormat::Nv12, ColorSpace::Bt601, ColorRange::Full),
    }
}

#[test]
fn a_prepared_picture_keeps_its_stream_and_generation() {
    let prepared = prepare_host_frame(small_frame(21, 4)).unwrap();

    assert_eq!((prepared.stream_id(), prepared.generation()), (21, 4));
}

#[test]
fn planes_handed_on_unchanged_report_their_own_stream_and_generation() {
    let prepared = PreparedHostFrame::Planes(small_frame(22, 9));

    assert_eq!((prepared.stream_id(), prepared.generation()), (22, 9));
}

struct DefaultBinding;

impl SurfaceBinding for DefaultBinding {}

fn bgra_frame(width: u32, height: u32, stride: u32, len: usize) -> HostBgraFrame {
    HostBgraFrame {
        stream_id: 31,
        generation: 2,
        width,
        height,
        stride,
        pixels: std::sync::Arc::from(vec![0_u8; len]),
    }
}

#[test]
fn bgra_rows_fit_only_when_the_buffer_covers_every_row_and_the_stride_covers_a_row() {
    assert!(bgra_frame(2, 2, 8, 16).rows_fit());
    assert!(bgra_frame(2, 2, 12, 20).rows_fit(), "the last row needs no padding");
    assert!(!bgra_frame(2, 2, 12, 19).rows_fit(), "last row cut short");
    assert!(!bgra_frame(2, 2, 7, 16).rows_fit(), "stride shorter than a row");
    assert!(!bgra_frame(0, 2, 8, 16).rows_fit(), "empty width");
    assert!(!bgra_frame(2, 0, 8, 16).rows_fit(), "empty height");
}

#[test]
fn the_default_bgra_preparation_hands_a_fitting_picture_on_and_refuses_a_short_one() {
    for host in [HostOs::MacOs, HostOs::Linux, HostOs::Windows] {
        let prepared = DefaultBinding
            .prepare_host_bgra_frame(host, bgra_frame(2, 2, 8, 16))
            .unwrap();
        assert!(matches!(prepared, PreparedHostFrame::Bgra(_)), "{host:?}");
        assert_eq!((prepared.stream_id(), prepared.generation()), (31, 2));

        let refused = DefaultBinding.prepare_host_bgra_frame(host, bgra_frame(2, 2, 8, 15));
        assert!(
            matches!(refused, Err(SurfaceError::WrongKind { .. })),
            "{host:?}"
        );
    }
}

#[test]
fn a_host_without_a_bgra_surface_names_the_card_that_brings_it() {
    for (host, card) in [
        (HostOs::MacOs, "P148"),
        (HostOs::Linux, "P147"),
        (HostOs::Windows, "P146"),
    ] {
        assert!(bgra_frame_unwired_reason(host).contains(card), "{host:?}");
        let presented = DefaultBinding.present_host_bgra_frame(host, &bgra_frame(2, 2, 8, 16));
        match presented {
            Err(SurfaceError::NotOnThisHost { host: named, reason }) => {
                assert_eq!(named, host);
                assert_eq!(reason, bgra_frame_unwired_reason(host));
            }
            _ => panic!("{host:?}: the default must name what is missing"),
        }
    }
}

fn three_by_two_frame() -> HostNv12Frame {
    use crate::texture::color::{ColorRange, ColorSpace, ConversionParams};
    use crate::texture::frame::FrameFormat;

    HostNv12Frame {
        stream_id: 41,
        generation: 6,
        width: 3,
        height: 2,
        luma: std::sync::Arc::from(vec![1_u8, 2, 3, 0, 4, 5, 6, 0]),
        luma_stride: 4,
        chroma: std::sync::Arc::from(vec![10_u8, 11, 20, 21]),
        chroma_stride: 4,
        conversion: ConversionParams::new(FrameFormat::Nv12, ColorSpace::Bt601, ColorRange::Limited),
    }
}

#[test]
fn only_the_four_call_rotations_turn_a_frame() {
    assert_eq!(clockwise_quarter_turns_of_degrees(0), Some(0));
    assert_eq!(clockwise_quarter_turns_of_degrees(90), Some(1));
    assert_eq!(clockwise_quarter_turns_of_degrees(180), Some(2));
    assert_eq!(clockwise_quarter_turns_of_degrees(270), Some(3));
    assert_eq!(clockwise_quarter_turns_of_degrees(45), None);
    assert_eq!(clockwise_quarter_turns_of_degrees(360), None);
}

#[test]
fn a_quarter_turn_swaps_the_sides_and_a_half_turn_keeps_them() {
    assert_eq!(rotated_size(1280, 720, 0), (1280, 720));
    assert_eq!(rotated_size(1280, 720, 1), (720, 1280));
    assert_eq!(rotated_size(1280, 720, 2), (1280, 720));
    assert_eq!(rotated_size(1280, 720, 3), (720, 1280));
    assert_eq!(rotated_size(1280, 720, 5), (720, 1280), "turns are taken modulo 4");
}

#[test]
fn no_turn_keeps_the_picture_and_its_stream() {
    let frame = three_by_two_frame();
    let same = rotate_host_nv12_frame(&frame, 0).unwrap();

    assert_eq!((same.width, same.height), (3, 2));
    assert_eq!(same.stream_id, frame.stream_id);
    assert_eq!(same.generation, frame.generation);
    assert_eq!(&*same.luma, &[1, 2, 3, 4, 5, 6]);
    assert_eq!(same.luma_stride, 3);
    assert_eq!(&*same.chroma, &[10, 11, 20, 21]);
    assert_eq!(same.chroma_stride, 4);
}

#[test]
fn a_clockwise_quarter_turn_puts_the_bottom_left_pixel_top_left() {
    let turned = rotate_host_nv12_frame(&three_by_two_frame(), 1).unwrap();

    assert_eq!((turned.width, turned.height), (2, 3));
    assert_eq!(&*turned.luma, &[4, 1, 5, 2, 6, 3]);
    assert_eq!(turned.luma_stride, 2);
    assert_eq!(&*turned.chroma, &[10, 11, 20, 21], "chroma pairs turn as units");
    assert_eq!(turned.chroma_stride, 2);
    assert_ne!(turned.stream_id, 41, "a turned copy draws under its own stream");
    assert_eq!(turned.generation, 6);
}

#[test]
fn half_and_three_quarter_turns_match_the_clockwise_layout() {
    let half = rotate_host_nv12_frame(&three_by_two_frame(), 2).unwrap();
    assert_eq!((half.width, half.height), (3, 2));
    assert_eq!(&*half.luma, &[6, 5, 4, 3, 2, 1]);
    assert_eq!(&*half.chroma, &[20, 21, 10, 11]);

    let three = rotate_host_nv12_frame(&three_by_two_frame(), 3).unwrap();
    assert_eq!((three.width, three.height), (2, 3));
    assert_eq!(&*three.luma, &[3, 6, 2, 5, 1, 4]);
}

#[test]
fn four_quarter_turns_come_back_to_the_original_picture() {
    let mut frame = three_by_two_frame();
    for _ in 0..4 {
        frame = rotate_host_nv12_frame(&frame, 1).unwrap();
    }

    assert_eq!((frame.width, frame.height), (3, 2));
    assert_eq!(&*frame.luma, &[1, 2, 3, 4, 5, 6]);
    assert_eq!(&*frame.chroma, &[10, 11, 20, 21]);
}

#[test]
fn planes_shorter_than_their_rows_are_not_turned() {
    let mut frame = three_by_two_frame();
    frame.luma = std::sync::Arc::from(vec![1_u8, 2, 3, 0, 4, 5]);

    assert!(rotate_host_nv12_frame(&frame, 1).is_none());
}

#[test]
fn a_mirror_flips_each_row_and_keeps_chroma_pairs_whole() {
    let frame = three_by_two_frame();
    let mirrored = mirror_host_nv12_frame(&frame).unwrap();

    assert_eq!((mirrored.width, mirrored.height), (3, 2));
    assert_eq!(&*mirrored.luma, &[3, 2, 1, 6, 5, 4], "padding of the source rows is dropped");
    assert_eq!(mirrored.luma_stride, 3);
    assert_eq!(&*mirrored.chroma, &[20, 21, 10, 11]);
    assert_eq!(mirrored.chroma_stride, 4);
}

#[test]
fn a_mirror_replaces_the_picture_of_its_stream() {
    let mirrored = mirror_host_nv12_frame(&three_by_two_frame()).unwrap();

    assert_eq!(mirrored.stream_id, 41);
    assert_eq!(mirrored.generation, 6);
}

#[test]
fn two_mirrors_come_back_to_the_original_picture() {
    let twice = mirror_host_nv12_frame(&mirror_host_nv12_frame(&three_by_two_frame()).unwrap()).unwrap();

    assert_eq!(&*twice.luma, &[1, 2, 3, 4, 5, 6]);
    assert_eq!(&*twice.chroma, &[10, 11, 20, 21]);
}

#[test]
fn planes_shorter_than_their_rows_are_not_mirrored() {
    let mut frame = three_by_two_frame();
    frame.luma = std::sync::Arc::from(vec![1_u8, 2, 3, 0, 4, 5]);

    assert!(mirror_host_nv12_frame(&frame).is_none());
}

#[test]
fn a_bgra_picture_turns_whole_pixels_and_packs_its_rows() {
    let frame = HostBgraFrame {
        stream_id: 51,
        generation: 3,
        width: 2,
        height: 1,
        stride: 12,
        pixels: std::sync::Arc::from(vec![1_u8, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0]),
    };
    let turned = rotate_host_bgra_frame(&frame, 1).unwrap();

    assert_eq!((turned.width, turned.height, turned.stride), (1, 2, 4));
    assert_eq!(&*turned.pixels, &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(turned.rows_fit());
    assert!(rotate_host_bgra_frame(&bgra_frame(2, 2, 8, 15), 1).is_none());
}

#[test]
fn a_branch_surface_is_not_turned_by_the_hub() {
    let surface = PreparedHostFrame::Surface {
        stream_id: 1,
        generation: 1,
        width: 6,
        height: 4,
        surface: RetainedSurface::new(0x2000, 0_u8),
    };

    assert_eq!(surface.size(), (6, 4));
    assert_eq!(PreparedHostFrame::Planes(three_by_two_frame()).size(), (3, 2));
    assert!(rotate_prepared_host_frame(&surface, 1).is_none());
    let planes = rotate_prepared_host_frame(&PreparedHostFrame::Planes(three_by_two_frame()), 1).unwrap();
    assert_eq!(planes.generation(), 6);
}

#[test]
fn an_angle_that_is_not_a_call_rotation_is_refused() {
    let frame = PreparedHostFrame::Planes(three_by_two_frame());

    assert!(matches!(
        present_host_frame_rotated(&frame, 45),
        Err(SurfaceError::WrongKind { .. })
    ));
}

#[test]
fn a_branch_without_turned_surfaces_names_what_is_missing() {
    let surface = RetainedSurface::new(0x3000, 0_u8);
    for host in [HostOs::MacOs, HostOs::Linux, HostOs::Windows] {
        match DefaultBinding.present_rotated(host, &surface, 1) {
            Err(SurfaceError::NotOnThisHost { host: named, reason }) => {
                assert_eq!(named, host);
                assert_eq!(reason, rotated_surface_unwired_reason(host));
            }
            _ => panic!("{host:?}: the default must name what is missing"),
        }
    }
}

fn planes_of(stream_id: u64, generation: u64) -> PreparedHostFrame {
    PreparedHostFrame::Planes(HostNv12Frame { stream_id, generation, ..three_by_two_frame() })
}

#[test]
fn presenting_an_unstaged_turn_turns_nothing_on_the_main_thread() {
    let frame = planes_of(9_001, 1);

    assert!(matches!(
        present_host_frame_rotated(&frame, 90),
        Err(SurfaceError::WrongKind { .. })
    ));
    assert!(staged_rotation(&frame, 1).is_none());
}

#[test]
fn a_staged_turn_is_found_for_its_generation_and_older_ones() {
    let frame = planes_of(9_002, 5);

    prepare_host_frame_rotation(&frame, 90).unwrap();

    let staged = staged_rotation(&frame, 1).unwrap();
    assert_eq!(staged.stream_id(), rotated_stream_id(9_002, 1));
    assert_eq!(staged.generation(), 5);
    match staged {
        PreparedHostFrame::Planes(planes) => assert_eq!((planes.width, planes.height), (2, 3)),
        _ => panic!("a CPU picture stays planes when turned"),
    }
    assert!(staged_rotation(&planes_of(9_002, 4), 1).is_some());
    assert!(staged_rotation(&planes_of(9_002, 6), 1).is_none());
    assert!(staged_rotation(&frame, 3).is_none());
}

#[test]
fn a_newer_generation_replaces_the_staged_one_and_an_older_one_does_not() {
    prepare_host_frame_rotation(&planes_of(9_003, 2), 270).unwrap();
    prepare_host_frame_rotation(&planes_of(9_003, 3), 270).unwrap();
    prepare_host_frame_rotation(&planes_of(9_003, 1), 270).unwrap();

    assert_eq!(staged_rotation(&planes_of(9_003, 1), 3).unwrap().generation(), 3);
}

#[test]
fn no_turn_and_a_branch_surface_stage_nothing_in_the_hub() {
    let upright = planes_of(9_004, 1);
    prepare_host_frame_rotation(&upright, 0).unwrap();
    assert!(staged_rotation(&upright, 0).is_none());

    let surface = RetainedSurface::new(0x4000, 0_u8);
    for host in [HostOs::MacOs, HostOs::Linux, HostOs::Windows] {
        assert!(DefaultBinding.prepare_rotated(host, &surface, 1).is_ok());
    }
    assert!(matches!(
        prepare_host_frame_rotation(&upright, 45),
        Err(SurfaceError::WrongKind { .. })
    ));
}
