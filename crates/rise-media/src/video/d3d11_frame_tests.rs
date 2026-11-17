use std::mem::{offset_of, size_of};

use crate::video::d3d11_frame::*;

const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

#[test]
fn the_frame_planes_give_the_texture_and_the_slice_index() {
    let texture = 0x1000 as *mut u8;
    let frame = D3d11FrameRef::from_frame_planes(texture, 5 as *mut u8, 1920, 1080, IDENTITY).unwrap();

    assert_eq!(frame.texture as usize, 0x1000);
    assert_eq!(frame.array_index, 5);
    assert_eq!((frame.width, frame.height), (1920, 1080));
}

#[test]
fn a_frame_without_a_texture_or_with_a_negative_size_is_refused() {
    assert_eq!(
        D3d11FrameRef::from_frame_planes(std::ptr::null_mut(), std::ptr::null_mut(), 16, 16, IDENTITY),
        Err(D3d11FrameError::NoTexture)
    );
    assert_eq!(
        D3d11FrameRef::from_frame_planes(0x10 as *mut u8, std::ptr::null_mut(), -1, 16, IDENTITY),
        Err(D3d11FrameError::Empty)
    );
}

#[test]
fn the_reference_keeps_a_stable_c_layout() {
    let pointer = size_of::<usize>();
    assert_eq!(offset_of!(D3d11FrameRef, texture), 0);
    assert_eq!(offset_of!(D3d11FrameRef, array_index), pointer);
    assert_eq!(offset_of!(D3d11FrameRef, width), pointer + 4);
    assert_eq!(offset_of!(D3d11FrameRef, height), pointer + 8);
    assert_eq!(offset_of!(D3d11FrameRef, ycbcr_to_rgb), pointer + 12);
}

#[test]
fn an_odd_size_rounds_up_inside_an_aligned_pool_and_down_at_its_edge() {
    assert_eq!(shared_copy_size(1919, 1081, 1920, 1088, 0, 20), Ok((1920, 1082)));
    assert_eq!(shared_copy_size(1921, 1081, 1921, 1081, 0, 20), Ok((1920, 1080)));
    assert_eq!(shared_copy_size(1280, 720, 1280, 720, 19, 20), Ok((1280, 720)));
}

#[test]
fn a_slice_outside_the_pool_or_a_picture_larger_than_it_is_refused() {
    assert_eq!(
        shared_copy_size(64, 64, 64, 64, 4, 4),
        Err(D3d11FrameError::SliceOutOfPool { index: 4, pool: 4 })
    );
    assert!(matches!(
        shared_copy_size(65, 64, 64, 64, 0, 4),
        Err(D3d11FrameError::LargerThanPool { .. })
    ));
    assert_eq!(shared_copy_size(1, 1, 1, 1, 0, 1), Err(D3d11FrameError::Empty));
}

#[test]
fn the_pool_reuses_then_grows_then_stops_at_its_ceiling() {
    assert_eq!(choose_slot(3, 1), SlotChoice::Reuse);
    assert_eq!(choose_slot(0, 0), SlotChoice::Grow);
    assert_eq!(choose_slot(SHARED_SLOTS_MAX - 1, 0), SlotChoice::Grow);
    assert_eq!(choose_slot(SHARED_SLOTS_MAX, 0), SlotChoice::Exhausted);
}

#[test]
fn idle_slots_beyond_the_kept_three_are_dropped() {
    assert_eq!(surplus_free_slots(0), 0);
    assert_eq!(surplus_free_slots(SHARED_SLOTS_KEPT), 0);
    assert_eq!(surplus_free_slots(SHARED_SLOTS_KEPT + 2), 2);
}
