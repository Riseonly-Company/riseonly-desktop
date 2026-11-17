use crate::video::av_player::media_engine_frames::*;

fn rgb_of(y: u8, cb: u8, cr: u8) -> (f32, f32, f32) {
    let (y, cb, cr) = (f32::from(y), f32::from(cb) - 128.0, f32::from(cr) - 128.0);
    (y + 1.402 * cr, y - 0.344_136 * cb - 0.714_136 * cr, y + 1.772 * cb)
}

#[test]
fn grays_keep_their_level_and_neutral_chroma() {
    for level in [0_u8, 1, 17, 128, 200, 254, 255] {
        assert_eq!(ycbcr_of(level, level, level), (level, 128, 128));
    }
}

#[test]
fn primaries_come_back_through_the_shader_matrix() {
    for (blue, green, red) in [(0_u8, 0_u8, 255_u8), (0, 255, 0), (255, 0, 0), (40, 90, 210), (250, 10, 120)] {
        let (y, cb, cr) = ycbcr_of(blue, green, red);
        let (r, g, b) = rgb_of(y, cb, cr);
        assert!((r - f32::from(red)).abs() <= 2.0, "red {red} came back as {r}");
        assert!((g - f32::from(green)).abs() <= 2.0, "green {green} came back as {g}");
        assert!((b - f32::from(blue)).abs() <= 2.0, "blue {blue} came back as {b}");
    }
}

#[test]
fn padded_rows_are_read_by_pitch_and_planes_are_packed() {
    let pitch = 3 * 4 + 4;
    let mut bgra = vec![0_u8; pitch * 3];
    for row in 0..3 {
        for column in 0..3 {
            let at = row * pitch + column * 4;
            bgra[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    let picture = bgra_to_nv12(&bgra, pitch, 3, 3, 7).expect("geometry fits");
    assert_eq!((picture.width, picture.height, picture.generation), (3, 3, 7));
    assert_eq!(picture.luma_stride, 3);
    assert_eq!(picture.chroma_stride, 4);
    assert_eq!(&picture.luma[..], &[255_u8; 9][..]);
    assert_eq!(&picture.chroma[..], &[128_u8; 8][..]);
}

#[test]
fn chroma_is_the_mean_of_its_block() {
    let bgra = [0_u8, 0, 255, 255, 0, 0, 0, 255];
    let picture = bgra_to_nv12(&bgra, 8, 2, 1, 0).expect("geometry fits");
    let (_, red_cb, red_cr) = ycbcr_of(0, 0, 255);
    let expected_cb = (u32::from(red_cb) + 128 + 1) / 2;
    let expected_cr = (u32::from(red_cr) + 128 + 1) / 2;
    assert_eq!(u32::from(picture.chroma[0]), expected_cb);
    assert_eq!(u32::from(picture.chroma[1]), expected_cr);
}

#[test]
fn short_or_empty_input_is_refused() {
    assert!(bgra_to_nv12(&[0; 12], 8, 2, 2, 0).is_none(), "4 bytes short of the last row");
    assert!(bgra_to_nv12(&[0; 16], 4, 2, 2, 0).is_none(), "pitch narrower than a row");
    assert!(bgra_to_nv12(&[], 0, 0, 0, 0).is_none(), "empty picture");
}

#[test]
fn the_clock_tells_new_same_and_wrapped_pictures() {
    let mut clock = PictureClock::default();
    assert_eq!(clock.tick(0), StreamTick::New);
    assert_eq!(clock.tick(0), StreamTick::Same);
    assert_eq!(clock.tick(333_667), StreamTick::New);
    assert_eq!(clock.tick(667_333), StreamTick::New);
    assert_eq!(clock.tick(0), StreamTick::Wrapped);
    assert_eq!(clock.tick(333_667), StreamTick::New);
    clock.reset();
    assert_eq!(clock.tick(0), StreamTick::New, "a new source starts a pass, it does not wrap");
}
