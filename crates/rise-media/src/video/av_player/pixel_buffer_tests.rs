use std::sync::Arc;

use crate::video::av_player::media_engine_frames::Nv12Picture;
use crate::video::av_player::pixel_buffer::*;

const LUMA_ONLY: [[f32; 4]; 4] = [[1.0, 1.0, 1.0, 0.0], [0.0; 4], [0.0; 4], [0.0, 0.0, 0.0, 1.0]];

const BT601_FULL: [[f32; 4]; 4] = [
    [1.0, 1.0, 1.0, 0.0],
    [0.0, -0.3441, 1.7720, 0.0],
    [1.4020, -0.7141, 0.0, 0.0],
    [-0.7010, 0.5291, -0.8860, 1.0],
];

/// test picture 3x2, luma 10 20 30 on the first row and 40 50 60 on the second
fn picture() -> Nv12Picture {
    Nv12Picture {
        width: 3,
        height: 2,
        luma: Arc::from(vec![10_u8, 20, 30, 40, 50, 60]),
        luma_stride: 3,
        chroma: Arc::from(vec![128_u8, 128, 128, 128]),
        chroma_stride: 4,
        generation: 1,
    }
}

fn blues(bgra: &[u8]) -> Vec<u8> {
    bgra.chunks_exact(4).map(|pixel| pixel[0]).collect()
}

#[test]
fn an_upright_picture_keeps_its_rows_and_is_opaque() {
    let (width, height, bgra) = oriented_bgra_of_nv12(&picture(), &LUMA_ONLY, 0).expect("converts");
    assert_eq!((width, height), (3, 2));
    assert_eq!(blues(&bgra), vec![10, 20, 30, 40, 50, 60]);
    assert!(bgra.chunks_exact(4).all(|pixel| pixel[3] == u8::MAX));
}

#[test]
fn quarter_turns_rotate_clockwise() {
    let (width, height, bgra) = oriented_bgra_of_nv12(&picture(), &LUMA_ONLY, 1).expect("converts");
    assert_eq!((width, height), (2, 3));
    assert_eq!(blues(&bgra), vec![40, 10, 50, 20, 60, 30]);

    let (width, height, bgra) = oriented_bgra_of_nv12(&picture(), &LUMA_ONLY, 2).expect("converts");
    assert_eq!((width, height), (3, 2));
    assert_eq!(blues(&bgra), vec![60, 50, 40, 30, 20, 10]);

    let (width, height, bgra) = oriented_bgra_of_nv12(&picture(), &LUMA_ONLY, 3).expect("converts");
    assert_eq!((width, height), (2, 3));
    assert_eq!(blues(&bgra), vec![30, 60, 20, 50, 10, 40]);
}

#[test]
fn neutral_chroma_through_bt601_is_grey_and_red_chroma_is_red() {
    let mut grey = picture();
    grey.luma = Arc::from(vec![255_u8; 6]);
    let (_, _, bgra) = oriented_bgra_of_nv12(&grey, &BT601_FULL, 0).expect("converts");
    assert!(bgra[..3].iter().all(|channel| *channel >= 253), "white {:?}", &bgra[..4]);
    assert_eq!(bgra[3], u8::MAX);

    let mut red = picture();
    red.luma = Arc::from(vec![76_u8; 6]);
    red.chroma = Arc::from(vec![85_u8, 255, 85, 255]);
    let (_, _, bgra) = oriented_bgra_of_nv12(&red, &BT601_FULL, 0).expect("converts");
    assert!(bgra[2] > 250, "red {}", bgra[2]);
    assert!(bgra[1] < 5, "green {}", bgra[1]);
    assert!(bgra[0] < 5, "blue {}", bgra[0]);
}

#[test]
fn short_planes_and_empty_pictures_are_refused() {
    let mut short = picture();
    short.luma = Arc::from(vec![0_u8; 5]);
    assert!(oriented_bgra_of_nv12(&short, &LUMA_ONLY, 0).is_none());

    let mut narrow = picture();
    narrow.chroma_stride = 2;
    assert!(oriented_bgra_of_nv12(&narrow, &LUMA_ONLY, 0).is_none());

    let mut empty = picture();
    empty.width = 0;
    assert!(oriented_bgra_of_nv12(&empty, &LUMA_ONLY, 0).is_none());
}

#[test]
fn ffmpeg_display_rotation_turns_the_other_way() {
    assert_eq!(clockwise_quarter_turns_of_display_rotation(0.0), 0);
    assert_eq!(clockwise_quarter_turns_of_display_rotation(-90.0), 1);
    assert_eq!(clockwise_quarter_turns_of_display_rotation(90.0), 3);
    assert_eq!(clockwise_quarter_turns_of_display_rotation(180.0), 2);
    assert_eq!(clockwise_quarter_turns_of_display_rotation(-180.0), 2);
    assert_eq!(clockwise_quarter_turns_of_display_rotation(-89.6), 1);
    assert_eq!(clockwise_quarter_turns_of_display_rotation(f64::NAN), 0);
}

#[test]
fn a_buffer_turns_into_an_image_of_the_turned_size() {
    let buffer = AVPlayerPixelBuffer::new(Box::new(Nv12PixelBuffer::new(picture(), LUMA_ONLY, 5)));
    let image = buffer.oriented_image().expect("image");
    let size = image.size(0);
    assert_eq!((size.width.0, size.height.0), (2, 3));
}

#[test]
fn a_quarter_turn_moves_the_left_column_to_the_top_row() {
    let turned = turned_nv12(&picture(), 1).expect("turns");
    assert_eq!((turned.width, turned.height, turned.luma_stride, turned.chroma_stride), (2, 3, 2, 2));
    assert_eq!(&*turned.luma, &[40, 10, 50, 20, 60, 30]);
    assert_eq!(turned.chroma.len(), 4);
    assert_eq!(turned.generation, 1);
}

#[test]
fn the_turned_planes_match_the_turned_copy_of_the_picture() {
    let source = picture();
    for turns in 0..4 {
        let turned = turned_nv12(&source, turns).expect("turns");
        let (_, _, upright) = oriented_bgra_of_nv12(&turned, &LUMA_ONLY, 0).expect("converts");
        let (_, _, copied) = oriented_bgra_of_nv12(&source, &LUMA_ONLY, turns).expect("converts");
        assert_eq!(blues(&upright), blues(&copied), "turns {turns}");
    }
}

#[test]
fn chroma_follows_its_pixels_through_a_half_turn() {
    let source = Nv12Picture {
        width: 4,
        height: 2,
        luma: Arc::from(vec![0_u8; 8]),
        luma_stride: 4,
        chroma: Arc::from(vec![1_u8, 2, 3, 4]),
        chroma_stride: 4,
        generation: 7,
    };
    let turned = turned_nv12(&source, 2).expect("turns");
    assert_eq!(&*turned.chroma, &[3, 4, 1, 2]);
}

#[test]
fn four_quarter_turns_give_the_picture_back() {
    let mut turned = picture();
    for _ in 0..4 {
        turned = turned_nv12(&turned, 1).expect("turns");
    }
    assert_eq!(turned, picture());
}

#[test]
fn planes_shorter_than_their_strides_are_refused() {
    let mut short = picture();
    short.luma = Arc::from(vec![0_u8; 4]);
    assert!(turned_nv12(&short, 1).is_none());
}
