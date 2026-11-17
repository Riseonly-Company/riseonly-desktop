use std::time::Duration;

use crate::texture::color::{ColorDescription, ColorRange, ColorSpace};
use crate::video::av_player::clip_pacing::*;
use crate::video::decoder::HostI420Planes;

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

#[test]
fn the_first_picture_is_due_at_once_and_later_ones_follow_their_timestamps() {
    let mut clock = ClipClock::default();
    assert_eq!(clock.due(5_000_000, ms(100)), PictureDue::Now);
    assert_eq!(clock.due(5_040_000, ms(100)), PictureDue::After(ms(40)));
    assert_eq!(clock.due(5_040_000, ms(140)), PictureDue::Now);
}

#[test]
fn a_paused_clock_shifts_the_pacing_by_the_time_it_stood() {
    let mut clock = ClipClock::default();
    clock.due(0, ms(0));
    clock.pause(ms(10));
    assert!(clock.is_paused());
    clock.resume(ms(510));
    assert!(!clock.is_paused());
    assert_eq!(clock.due(40_000, ms(510)), PictureDue::After(ms(30)));
}

#[test]
fn a_long_stall_restarts_the_pacing_at_the_late_picture() {
    let mut clock = ClipClock::default();
    clock.due(0, ms(0));
    assert_eq!(clock.due(40_000, ms(1_000)), PictureDue::Now);
    assert_eq!(clock.due(80_000, ms(1_000)), PictureDue::After(ms(40)));
}

#[test]
fn a_short_lag_keeps_the_pacing_so_the_clip_catches_up() {
    let mut clock = ClipClock::default();
    clock.due(0, ms(0));
    assert_eq!(clock.due(40_000, ms(100)), PictureDue::Now);
    assert_eq!(clock.due(80_000, ms(100)), PictureDue::Now);
}

#[test]
fn timestamp_jumps_either_way_never_wait() {
    let mut clock = ClipClock::default();
    clock.due(10_000_000, ms(0));
    assert_eq!(clock.due(30_000_000, ms(0)), PictureDue::Now);
    assert_eq!(clock.due(1_000_000, ms(0)), PictureDue::Now);
    assert_eq!(clock.due(1_040_000, ms(0)), PictureDue::After(ms(40)));
}

#[test]
fn a_restart_anchors_the_next_pass_at_its_first_picture() {
    let mut clock = ClipClock::default();
    clock.due(0, ms(0));
    clock.due(2_000_000, ms(2_000));
    clock.restart();
    assert_eq!(clock.due(0, ms(2_040)), PictureDue::Now);
    assert_eq!(clock.due(40_000, ms(2_040)), PictureDue::After(ms(40)));
}

#[test]
fn every_pass_posts_its_end_and_only_a_held_looper_restarts() {
    assert_eq!(pass_end(true), PassEnd { posts_end: true, restarts: true });
    assert_eq!(pass_end(false), PassEnd { posts_end: true, restarts: false });
}

#[test]
fn tagged_pictures_keep_their_colour_and_untagged_ones_resolve_by_height() {
    assert_eq!(
        picture_color(1, 1, false, 1_080),
        ColorDescription { space: ColorSpace::Bt709, range: ColorRange::Limited }
    );
    assert_eq!(
        picture_color(6, 2, false, 1_080),
        ColorDescription { space: ColorSpace::Bt601, range: ColorRange::Full }
    );
    assert_eq!(
        picture_color(9, 0, false, 2_160),
        ColorDescription { space: ColorSpace::Bt2020, range: ColorRange::Limited }
    );
    assert_eq!(picture_color(2, 0, false, 480), ColorDescription::resolve(None, None, 480));
    assert_eq!(picture_color(2, 0, false, 720), ColorDescription::resolve(None, None, 720));
}

#[test]
fn a_yuvj_picture_is_full_range_whatever_its_tag() {
    assert_eq!(picture_color(1, 1, true, 720).range, ColorRange::Full);
}

#[test]
fn i420_planes_with_padded_rows_become_packed_nv12() {
    let y = [1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 9];
    let u = [10, 11, 0, 0, 12, 13];
    let v = [20, 21, 0, 0, 22, 23];
    let planes = HostI420Planes { width: 3, height: 3, y: &y, u: &u, v: &v, stride_y: 4, stride_u: 4, stride_v: 4 };
    let picture = nv12_from_i420(&planes, 7).expect("planes fit");
    assert_eq!(&*picture.luma, &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
    assert_eq!(&*picture.chroma, &[10, 20, 11, 21, 12, 22, 13, 23]);
    assert_eq!((picture.luma_stride, picture.chroma_stride), (3, 4));
    assert_eq!((picture.width, picture.height, picture.generation), (3, 3, 7));
}

#[test]
fn short_i420_planes_are_refused() {
    let y = [0_u8; 8];
    let u = [0_u8; 2];
    let v = [0_u8; 1];
    let planes = HostI420Planes { width: 3, height: 3, y: &y, u: &u, v: &v, stride_y: 3, stride_u: 2, stride_v: 2 };
    assert!(nv12_from_i420(&planes, 0).is_none());
    let empty = HostI420Planes { width: 0, height: 2, y: &[], u: &[], v: &[], stride_y: 0, stride_u: 0, stride_v: 0 };
    assert!(nv12_from_i420(&empty, 0).is_none());
}

#[test]
fn semi_planar_rows_lose_their_padding() {
    let y = [1, 2, 0, 3, 4];
    let uv = [5, 6, 0, 0];
    let picture = nv12_from_semi_planar(2, 2, &y, 3, &uv, 4, 1).expect("planes fit");
    assert_eq!(&*picture.luma, &[1, 2, 3, 4]);
    assert_eq!(&*picture.chroma, &[5, 6]);
    assert_eq!((picture.luma_stride, picture.chroma_stride), (2, 2));
    assert!(nv12_from_semi_planar(2, 2, &y[..4], 3, &uv, 4, 1).is_none());
    assert!(nv12_from_semi_planar(2, 2, &y, 1, &uv, 4, 1).is_none());
}
