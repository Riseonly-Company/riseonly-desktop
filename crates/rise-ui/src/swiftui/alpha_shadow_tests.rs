use gpui::{Bounds, point, px, size};

use super::alpha_shadow::{AlphaShadowPlan, alpha_shadow_bounds, alpha_shadow_mask_pixels, alpha_shadow_pixels};
use super::blur::gaussian_kernel;

const BLACK: [u8; 4] = [0, 0, 0, 255];

/// square picture, transparent except one opaque block in the middle
fn picture_with_block(side: usize, block: usize) -> Vec<u8> {
    let start = (side - block) / 2;
    let mut bytes = vec![0_u8; side * side * 4];
    for row in start..start + block {
        for column in start..start + block {
            bytes[(row * side + column) * 4 + 3] = 255;
        }
    }
    bytes
}

fn alpha_at(baked: &[u8], width: usize, row: usize, column: usize) -> u8 {
    baked[(row * width + column) * 4 + 3]
}

#[test]
fn plan_reduces_a_wide_blur_and_pads_by_the_kernel_reach() {
    let plan = AlphaShadowPlan::new(6.0);
    assert_eq!(plan.factor, 4);
    assert_eq!(plan.padding, gaussian_kernel(plan.working_sigma).len() / 2);
    let narrow = AlphaShadowPlan::new(1.0);
    assert_eq!(narrow.factor, 1);
    assert_eq!(narrow.padding, gaussian_kernel(narrow.working_sigma).len() / 2);
}

#[test]
fn shadow_follows_the_silhouette_and_not_the_box() {
    let side = 64;
    let bytes = picture_with_block(side, 16);
    let plan = AlphaShadowPlan::new(6.0);
    let (baked, width, height) = alpha_shadow_pixels(&bytes, side, side, plan, BLACK).expect("valid picture");
    assert_eq!((width, height), plan.baked_size(side, side));
    let middle = width / 2;
    assert!(alpha_at(&baked, width, middle, middle) > 100, "the middle sits under the block");
    assert_eq!(alpha_at(&baked, width, 0, 0), 0, "a corner of the box stays clear");
    let corner_of_picture = plan.padding;
    assert_eq!(alpha_at(&baked, width, corner_of_picture, corner_of_picture), 0, "no rectangular halo");
}

#[test]
fn shadow_is_filled_with_the_color_and_its_alpha() {
    let side = 32;
    let bytes = picture_with_block(side, 32);
    let plan = AlphaShadowPlan::new(1.0);
    let color = [10, 20, 30, 46];
    let (baked, width, _) = alpha_shadow_pixels(&bytes, side, side, plan, color).expect("valid picture");
    let middle = width / 2;
    let offset = (middle * width + middle) * 4;
    assert_eq!(&baked[offset..offset + 3], &[10, 20, 30]);
    assert_eq!(baked[offset + 3], 46);
}

#[test]
fn short_or_empty_input_makes_no_shadow() {
    let plan = AlphaShadowPlan::new(2.0);
    assert!(alpha_shadow_pixels(&[], 0, 4, plan, BLACK).is_none());
    assert!(alpha_shadow_pixels(&[0; 8], 4, 4, plan, BLACK).is_none());
}

#[test]
fn bounds_keep_the_picture_in_place_and_add_the_offset() {
    let fitted = Bounds {
        origin: point(px(100.0), px(200.0)),
        size: size(px(64.0), px(64.0)),
    };
    let plan = AlphaShadowPlan {
        factor: 2,
        working_sigma: 1.5,
        padding: 5,
    };
    let target = alpha_shadow_bounds(fitted, (128, 128), plan, (0.0, 3.0));
    assert_eq!(f32::from(target.origin.x), 95.0);
    assert_eq!(f32::from(target.origin.y), 198.0);
    assert_eq!(f32::from(target.size.width), 74.0);
    assert_eq!(f32::from(target.size.height), 74.0);
}

#[test]
fn mask_reduces_to_the_side_limit_and_keeps_alpha() {
    let side = 512;
    let bytes = picture_with_block(side, 256);
    let (mask, width, height) = alpha_shadow_mask_pixels(&bytes, side, side, side * 4).expect("valid picture");
    assert_eq!((width, height), (128, 128));
    assert_eq!(mask[(64 * width + 64) * 4 + 3], 255);
    assert_eq!(mask[3], 0);
}

#[test]
fn mask_honours_the_stride() {
    let (width, height, stride) = (2, 2, 16);
    let mut bytes = vec![0_u8; stride * height];
    bytes[stride + 4 + 3] = 200;
    let (mask, mask_width, mask_height) = alpha_shadow_mask_pixels(&bytes, width, height, stride).expect("valid picture");
    assert_eq!((mask_width, mask_height), (2, 2));
    assert_eq!(mask[(2 + 1) * 4 + 3], 200);
    assert_eq!(mask[3], 0);
}
