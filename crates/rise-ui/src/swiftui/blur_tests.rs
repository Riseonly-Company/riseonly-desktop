use super::blur::{
    blur_pixels, combined_blur_radius, downscale_factor, gaussian_kernel, source_sigma,
};

const TOLERANCE: f32 = 1e-4;

fn uniform(width: usize, height: usize, pixel: [u8; 4]) -> Vec<u8> {
    pixel.iter().copied().cycle().take(width * height * 4).collect()
}

#[test]
fn kernel_is_normalised_and_symmetric() {
    for sigma in [0.5_f32, 1.0, 2.5, 6.0] {
        let kernel = gaussian_kernel(sigma);
        assert_eq!(kernel.len() % 2, 1);
        let sum: f32 = kernel.iter().sum();
        assert!((sum - 1.0).abs() < TOLERANCE, "sigma {sigma} sums to {sum}");
        for (left, right) in kernel.iter().zip(kernel.iter().rev()) {
            assert!((left - right).abs() < TOLERANCE);
        }
        let middle = kernel[kernel.len() / 2];
        assert!(kernel.iter().all(|weight| *weight <= middle));
    }
}

#[test]
fn sigma_is_half_the_radius_in_points_over_the_fill_scale() {
    assert_eq!(source_sigma(18.0, (96.0, 96.0), (192.0, 192.0)), 4.5);
    assert_eq!(source_sigma(8.0, (100.0, 100.0), (400.0, 100.0)), 1.0);
    assert_eq!(source_sigma(0.0, (96.0, 96.0), (192.0, 192.0)), 0.0);
    assert_eq!(source_sigma(18.0, (0.0, 96.0), (192.0, 192.0)), 0.0);
    assert_eq!(source_sigma(18.0, (96.0, 96.0), (0.0, 0.0)), 0.0);
}

#[test]
fn sigma_snaps_to_quarter_pixels() {
    let sigma = source_sigma(10.0, (100.0, 100.0), (130.0, 130.0));
    assert_eq!((sigma * 4.0).fract(), 0.0);
}

#[test]
fn nested_blurs_add_their_variances() {
    assert!((combined_blur_radius(3.0, 4.0) - 5.0).abs() < TOLERANCE);
    assert_eq!(combined_blur_radius(0.0, 7.0), 7.0);
}

#[test]
fn wide_blurs_downscale_first() {
    assert_eq!(downscale_factor(1.0), 1);
    assert_eq!(downscale_factor(3.0), 1);
    assert_eq!(downscale_factor(6.5), 2);
    assert_eq!(downscale_factor(30.0), 10);
}

#[test]
fn opaque_blur_keeps_a_uniform_image_uniform() {
    let pixel = [40, 120, 200, 255];
    let (out, width, height) =
        blur_pixels(&uniform(8, 6, pixel), 8, 6, 2.0, true).expect("blurs");
    assert_eq!((width, height), (8, 6));
    for chunk in out.chunks_exact(4) {
        for (value, expected) in chunk.iter().zip(pixel) {
            assert!(value.abs_diff(expected) <= 1, "{chunk:?}");
        }
    }
}

#[test]
fn non_opaque_blur_fades_the_edges_but_not_the_colour() {
    let pixel = [10, 200, 90, 255];
    let (out, width, _) = blur_pixels(&uniform(12, 12, pixel), 12, 12, 2.0, false).expect("blurs");
    let at = |x: usize, y: usize| &out[(y * width + x) * 4..(y * width + x) * 4 + 4];
    assert!(at(0, 0)[3] < 200, "corner alpha {}", at(0, 0)[3]);
    assert!(at(6, 6)[3] > at(0, 6)[3]);
    for (value, expected) in at(0, 0)[..3].iter().zip(&pixel[..3]) {
        assert!(value.abs_diff(*expected) <= 2);
    }
}

#[test]
fn blur_spreads_a_single_bright_pixel() {
    let mut bytes = uniform(9, 9, [0, 0, 0, 255]);
    let centre = (4 * 9 + 4) * 4;
    bytes[centre..centre + 3].copy_from_slice(&[255, 255, 255]);
    let (out, _, _) = blur_pixels(&bytes, 9, 9, 1.0, true).expect("blurs");
    assert!(out[centre] < 255);
    let neighbour = (4 * 9 + 5) * 4;
    assert!(out[neighbour] > 0);
}

#[test]
fn a_short_buffer_is_refused() {
    assert!(blur_pixels(&[0; 12], 2, 2, 1.0, false).is_none());
    assert!(blur_pixels(&[], 0, 0, 1.0, false).is_none());
}

#[test]
fn wide_blur_returns_the_smaller_bitmap() {
    let (out, width, height) =
        blur_pixels(&uniform(20, 10, [1, 2, 3, 255]), 20, 10, 9.0, true).expect("blurs");
    assert_eq!((width, height), (7, 4));
    assert_eq!(out.len(), 7 * 4 * 4);
}
