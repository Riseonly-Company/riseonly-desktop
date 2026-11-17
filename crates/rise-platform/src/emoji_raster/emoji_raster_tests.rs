use super::emoji_raster::*;
use crate::host_os::HostOs;

struct Unwritten;

impl EmojiRasterSource for Unwritten {}

struct Fixed(EmojiRaster);

impl EmojiRasterSource for Fixed {
    fn render(&self, _emoji: &str, _point_size: f32) -> Result<EmojiRaster, EmojiRasterError> {
        Ok(self.0.clone())
    }
}

struct SemiboldOnly {
    regular: EmojiRaster,
    semibold: EmojiRaster,
}

impl EmojiRasterSource for SemiboldOnly {
    fn render(&self, _emoji: &str, _point_size: f32) -> Result<EmojiRaster, EmojiRasterError> {
        Ok(self.regular.clone())
    }

    fn render_weighted(
        &self,
        _text: &str,
        _point_size: f32,
        weight: TextWeight,
    ) -> Result<EmojiRaster, EmojiRasterError> {
        match weight {
            TextWeight::Regular => Ok(self.regular.clone()),
            TextWeight::Semibold => Ok(self.semibold.clone()),
        }
    }
}

fn raster(width: u32, height: u32, bytes: usize) -> EmojiRaster {
    EmojiRaster {
        width,
        height,
        rgba: vec![0; bytes],
    }
}

#[test]
fn unwritten_branch_names_the_host() {
    assert_eq!(
        render_with(&Unwritten, "\u{1F43C}", 300.0),
        Err(EmojiRasterError::BranchNotWritten {
            host: HostOs::current()
        })
    );
}

#[test]
fn empty_string_and_bad_sizes_fail_before_the_source() {
    let source = Fixed(raster(1, 1, 4));
    assert!(matches!(render_with(&source, "", 300.0), Err(EmojiRasterError::Failed { .. })));
    for size in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(matches!(
            render_with(&source, "\u{1F389}", size),
            Err(EmojiRasterError::Failed { .. })
        ));
    }
}

#[test]
fn raster_shape_is_checked() {
    assert_eq!(
        render_with(&Fixed(raster(2, 3, 24)), "\u{1F44D}", 300.0),
        Ok(raster(2, 3, 24))
    );
    assert!(matches!(
        render_with(&Fixed(raster(2, 3, 23)), "\u{1F44D}", 300.0),
        Err(EmojiRasterError::Failed { .. })
    ));
    assert!(matches!(
        render_with(&Fixed(raster(0, 3, 0)), "\u{1F44D}", 300.0),
        Err(EmojiRasterError::Failed { .. })
    ));
}

#[test]
fn a_layer_with_the_weight_draws_it() {
    let source = SemiboldOnly {
        regular: raster(1, 1, 4),
        semibold: raster(2, 2, 16),
    };
    assert_eq!(render_text_with(&source, "A", 118.0, TextWeight::Semibold), Ok(raster(2, 2, 16)));
    assert_eq!(render_text_with(&source, "A", 118.0, TextWeight::Regular), Ok(raster(1, 1, 4)));
    assert_eq!(render_with(&source, "A", 118.0), Ok(raster(1, 1, 4)));
}

#[test]
fn a_layer_without_the_weight_draws_the_regular_face() {
    assert_eq!(
        render_text_with(&Fixed(raster(1, 1, 4)), "A", 118.0, TextWeight::Semibold),
        Ok(raster(1, 1, 4))
    );
}

#[test]
fn an_unwritten_layer_stays_unwritten_for_every_weight() {
    for weight in [TextWeight::Regular, TextWeight::Semibold] {
        assert_eq!(
            render_text_with(&Unwritten, "A", 118.0, weight),
            Err(EmojiRasterError::BranchNotWritten {
                host: HostOs::current()
            })
        );
    }
}

#[test]
fn weighted_input_and_raster_shape_are_checked() {
    let source = Fixed(raster(2, 3, 23));
    assert!(matches!(
        render_text_with(&source, "A", 118.0, TextWeight::Semibold),
        Err(EmojiRasterError::Failed { .. })
    ));
    assert!(matches!(
        render_text_with(&source, "", 118.0, TextWeight::Semibold),
        Err(EmojiRasterError::Failed { .. })
    ));
}

#[test]
fn pixel_box_rounds_up_and_bounds() {
    assert_eq!(pixel_box(358.2, 357.0), Some((359, 357)));
    assert_eq!(pixel_box(0.5, 0.5), Some((1, 1)));
    assert_eq!(pixel_box(0.0, 10.0), None);
    assert_eq!(pixel_box(10.0, f64::NAN), None);
    assert_eq!(pixel_box(f64::from(MAX_RASTER_SIDE), 1.0), Some((MAX_RASTER_SIDE, 1)));
    assert_eq!(pixel_box(f64::from(MAX_RASTER_SIDE) + 0.1, 1.0), None);
}

#[test]
fn unpremultiply_restores_straight_colour() {
    let mut rgba = vec![
        128, 64, 0, 128,
        10, 20, 30, 0,
        200, 100, 50, 255,
        255, 255, 255, 1,
    ];
    unpremultiply_rgba(&mut rgba);
    assert_eq!(&rgba[0..4], &[255, 128, 0, 128]);
    assert_eq!(&rgba[4..8], &[0, 0, 0, 0]);
    assert_eq!(&rgba[8..12], &[200, 100, 50, 255]);
    assert_eq!(&rgba[12..16], &[255, 255, 255, 1]);
}
