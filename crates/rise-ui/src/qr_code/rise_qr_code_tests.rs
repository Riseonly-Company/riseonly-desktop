use super::rise_qr_code::{
    RiseQRCodeBitmap, RiseQRCodeGenerator, RiseQRCodeMatrix, RiseQRCodeRenderer, RiseQRCodeStyle,
};

#[test]
fn matrix_rejects_mismatched_module_count() {
    assert!(RiseQRCodeMatrix::new(0, Vec::new()).is_none());
    assert!(RiseQRCodeMatrix::new(2, vec![true; 3]).is_none());
    assert!(RiseQRCodeMatrix::new(2, vec![true; 4]).is_some());
}

#[test]
fn is_dark_outside_the_matrix_is_light() {
    let Some(matrix) = RiseQRCodeMatrix::new(1, vec![true]) else {
        panic!("one-module matrix must build");
    };
    assert!(matrix.is_dark(0, 0));
    assert!(!matrix.is_dark(-1, 0));
    assert!(!matrix.is_dark(1, 0));
    assert!(!matrix.is_dark(0, 1));
}

#[test]
fn generated_code_has_no_quiet_zone() {
    let Some(matrix) = RiseQRCodeGenerator::matrix("https://riseonly.net/u/example") else {
        panic!("generator must encode a short link");
    };
    assert!(matrix.size >= 21);
    assert!(matrix.is_dark(0, 0));
    assert!(matrix.is_dark(matrix.size as i64 - 1, 0));
    assert!(matrix.is_dark(0, matrix.size as i64 - 1));
}

#[test]
fn renderer_sizes_the_raster_by_scale() {
    let Some(matrix) = RiseQRCodeGenerator::matrix("rise") else {
        panic!("generator must encode a word");
    };
    let style = RiseQRCodeStyle::fallback();
    let Some(bitmap) = RiseQRCodeRenderer::image(&matrix, 100.0, 2.0, &style, None) else {
        panic!("renderer must draw a valid matrix");
    };
    assert_eq!((bitmap.width, bitmap.height, bitmap.scale), (200, 200, 2));
    assert_eq!(bitmap.rgba.len(), 200 * 200 * 4);
    assert!(bitmap.rgba[(4 * 200 + 33) * 4 + 3] > 0);
    let centre = (100 * 200 + 100) * 4;
    assert_eq!(bitmap.rgba[centre + 3], 0);
}

#[test]
fn renderer_refuses_matrix_below_version_one() {
    let Some(matrix) = RiseQRCodeMatrix::new(20, vec![true; 400]) else {
        panic!("matrix must build");
    };
    assert!(RiseQRCodeRenderer::image(&matrix, 100.0, 1.0, &RiseQRCodeStyle::fallback(), None).is_none());
    assert!(RiseQRCodeRenderer::image(&matrix, 0.0, 1.0, &RiseQRCodeStyle::fallback(), None).is_none());
}

#[test]
fn logo_lands_in_the_centre() {
    let Some(matrix) = RiseQRCodeGenerator::matrix("rise") else {
        panic!("generator must encode a word");
    };
    let Some(logo) = RiseQRCodeBitmap::new(1, 1, 1, vec![255, 0, 0, 255]) else {
        panic!("one-pixel logo must build");
    };
    let Some(bitmap) =
        RiseQRCodeRenderer::image(&matrix, 100.0, 1.0, &RiseQRCodeStyle::fallback(), Some(&logo))
    else {
        panic!("renderer must draw with a logo");
    };
    let centre = (50 * 100 + 50) * 4;
    let pixel = &bitmap.rgba[centre..centre + 4];
    assert!(pixel[0] > 200 && pixel[1] < 50 && pixel[2] < 50 && pixel[3] > 250);
}
