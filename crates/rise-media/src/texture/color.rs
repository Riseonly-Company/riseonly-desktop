//! The matrix and offset the conversion shader is fed.
//!
//! Getting these wrong does not crash and does not drop a frame; it produces
//! video that is subtly wrong — grey blacks, or oversaturated skin tones — which
//! survives review because it still looks like video. That is why the values
//! are derived here, in one place, with the arithmetic tested.

use super::frame::FrameFormat;

/// The colour primaries and transfer a stream declares.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ColorSpace {
    /// SD, and what most user-uploaded phone video from older devices carries.
    Bt601,
    /// HD. The default when a stream declares nothing, because almost all
    /// modern 720p+ content is this.
    #[default]
    Bt709,
    /// HDR and wide gamut. Reached through 10-bit formats.
    Bt2020,
}

/// Whether luma spans the full 0-255 or the broadcast-legal 16-235.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ColorRange {
    /// 16-235 luma, 16-240 chroma. What a camera and almost every encoder emit.
    #[default]
    Limited,
    /// 0-255. JPEG-derived content and some screen recordings.
    Full,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PlaneLayoutKind {
    Biplanar = 0,
    Triplanar = 1,
}

impl PlaneLayoutKind {
    pub const fn for_format(format: FrameFormat) -> Self {
        match format {
            FrameFormat::Nv12 | FrameFormat::P010 | FrameFormat::Bgra8 => Self::Biplanar,
            FrameFormat::I420 => Self::Triplanar,
        }
    }
}

/// Exactly what the shader's uniform buffer holds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ConversionParams {
    /// Row-major 3x3. The shader multiplies this by (Y, U, V) minus `offset`.
    pub matrix: [[f32; 3]; 3],
    pub offset: [f32; 3],
    pub plane_layout: u32,
}

impl ConversionParams {
    pub fn new(format: FrameFormat, space: ColorSpace, range: ColorRange) -> Self {
        // Kr and Kb, the luma weights each standard assigns to red and blue.
        // Kg follows from them summing to one.
        let (kr, kb) = match space {
            ColorSpace::Bt601 => (0.299_f32, 0.114_f32),
            ColorSpace::Bt709 => (0.2126_f32, 0.0722_f32),
            ColorSpace::Bt2020 => (0.2627_f32, 0.0593_f32),
        };
        let kg = 1.0 - kr - kb;

        // Limited range packs 0..1 of signal into 219/255 of luma and 224/255
        // of chroma, so the matrix is scaled up to undo that compression.
        let (luma_scale, chroma_scale, offset) = match range {
            ColorRange::Limited => (255.0 / 219.0, 255.0 / 224.0, [16.0 / 255.0, 0.5, 0.5]),
            ColorRange::Full => (1.0, 1.0, [0.0, 0.5, 0.5]),
        };

        let cr_to_r = 2.0 * (1.0 - kr) * chroma_scale;
        let cb_to_b = 2.0 * (1.0 - kb) * chroma_scale;
        let cb_to_g = -2.0 * kb * (1.0 - kb) / kg * chroma_scale;
        let cr_to_g = -2.0 * kr * (1.0 - kr) / kg * chroma_scale;

        Self {
            matrix: [
                [luma_scale, 0.0, cr_to_r],
                [luma_scale, cb_to_g, cr_to_g],
                [luma_scale, cb_to_b, 0.0],
            ],
            offset,
            plane_layout: PlaneLayoutKind::for_format(format) as u32,
        }
    }

    /// Apply the conversion on the CPU. The shader is the production path;
    /// this exists so the matrix arithmetic can be asserted against known
    /// colours instead of eyeballed on screen.
    #[cfg(test)]
    fn apply(&self, yuv: [f32; 3]) -> [f32; 3] {
        let centred = [
            yuv[0] - self.offset[0],
            yuv[1] - self.offset[1],
            yuv[2] - self.offset[2],
        ];

        let mut rgb = [0.0f32; 3];
        for (channel, row) in self.matrix.iter().enumerate() {
            rgb[channel] = row[0] * centred[0] + row[1] * centred[1] + row[2] * centred[2];
        }
        rgb
    }
}

/// What a stream declared, resolved against the defaults when it declared
/// nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct ColorDescription {
    pub space: ColorSpace,
    pub range: ColorRange,
}

impl ColorDescription {
    /// A stream that carries no colour metadata at all.
    ///
    /// Guessing BT.709 limited is the standard behaviour and matches what
    /// VideoToolbox, libavcodec and every browser assume; guessing anything
    /// else makes untagged content look wrong in exactly the way users blame
    /// on the app rather than the file.
    pub fn resolve(space: Option<ColorSpace>, range: Option<ColorRange>, height: u32) -> Self {
        Self {
            space: space.unwrap_or(if height <= 576 {
                // SD-height untagged content predates BT.709 in practice.
                ColorSpace::Bt601
            } else {
                ColorSpace::Bt709
            }),
            range: range.unwrap_or_default(),
        }
    }

    pub fn params(&self, format: FrameFormat) -> ConversionParams {
        ConversionParams::new(format, self.space, self.range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 0.01;

    fn assert_close(actual: [f32; 3], expected: [f32; 3], what: &str) {
        for channel in 0..3 {
            assert!(
                (actual[channel] - expected[channel]).abs() < EPSILON,
                "{what}: channel {channel} was {} not {}",
                actual[channel],
                expected[channel]
            );
        }
    }

    #[test]
    fn limited_range_black_maps_to_black_and_white_to_white() {
        let params =
            ConversionParams::new(FrameFormat::Nv12, ColorSpace::Bt709, ColorRange::Limited);

        assert_close(
            params.apply([16.0 / 255.0, 0.5, 0.5]),
            [0.0, 0.0, 0.0],
            "black",
        );
        assert_close(
            params.apply([235.0 / 255.0, 0.5, 0.5]),
            [1.0, 1.0, 1.0],
            "white",
        );
    }

    #[test]
    fn full_range_black_and_white_use_the_whole_scale() {
        let params = ConversionParams::new(FrameFormat::Nv12, ColorSpace::Bt709, ColorRange::Full);

        assert_close(params.apply([0.0, 0.5, 0.5]), [0.0, 0.0, 0.0], "black");
        assert_close(params.apply([1.0, 0.5, 0.5]), [1.0, 1.0, 1.0], "white");
    }

    #[test]
    fn treating_limited_content_as_full_is_what_lifts_the_blacks() {
        let full = ConversionParams::new(FrameFormat::Nv12, ColorSpace::Bt709, ColorRange::Full);
        let black_as_full = full.apply([16.0 / 255.0, 0.5, 0.5]);

        assert!(
            black_as_full[0] > EPSILON,
            "the mismatch this test guards against produces grey blacks, not an error"
        );
    }

    #[test]
    fn grey_stays_grey_in_every_space_and_range() {
        for space in [ColorSpace::Bt601, ColorSpace::Bt709, ColorSpace::Bt2020] {
            for range in [ColorRange::Limited, ColorRange::Full] {
                let params = ConversionParams::new(FrameFormat::Nv12, space, range);
                let luma = match range {
                    ColorRange::Limited => (16.0 + 219.0 * 0.5) / 255.0,
                    ColorRange::Full => 0.5,
                };

                let rgb = params.apply([luma, 0.5, 0.5]);
                assert_close(rgb, [0.5, 0.5, 0.5], &format!("{space:?}/{range:?}"));
            }
        }
    }

    #[test]
    fn the_luma_column_is_identical_across_channels() {
        let params =
            ConversionParams::new(FrameFormat::Nv12, ColorSpace::Bt709, ColorRange::Limited);

        assert_eq!(params.matrix[0][0], params.matrix[1][0]);
        assert_eq!(params.matrix[1][0], params.matrix[2][0]);
    }

    #[test]
    fn cb_does_not_reach_red_and_cr_does_not_reach_blue() {
        for space in [ColorSpace::Bt601, ColorSpace::Bt709, ColorSpace::Bt2020] {
            let params = ConversionParams::new(FrameFormat::Nv12, space, ColorRange::Limited);
            assert_eq!(params.matrix[0][1], 0.0);
            assert_eq!(params.matrix[2][2], 0.0);
        }
    }

    #[test]
    fn bt709_and_bt601_differ_enough_to_be_visible() {
        let bt601 =
            ConversionParams::new(FrameFormat::Nv12, ColorSpace::Bt601, ColorRange::Limited);
        let bt709 =
            ConversionParams::new(FrameFormat::Nv12, ColorSpace::Bt709, ColorRange::Limited);

        let sample = [0.5, 0.7, 0.3];
        let a = bt601.apply(sample);
        let b = bt709.apply(sample);

        assert!(
            (0..3).any(|channel| (a[channel] - b[channel]).abs() > 0.02),
            "if these matched, picking the wrong space would be harmless and this could be one constant"
        );
    }

    #[test]
    fn the_plane_layout_flag_follows_the_frame_format() {
        assert_eq!(
            ConversionParams::new(FrameFormat::Nv12, ColorSpace::Bt709, ColorRange::Limited)
                .plane_layout,
            PlaneLayoutKind::Biplanar as u32
        );
        assert_eq!(
            ConversionParams::new(FrameFormat::I420, ColorSpace::Bt709, ColorRange::Limited)
                .plane_layout,
            PlaneLayoutKind::Triplanar as u32
        );
    }

    #[test]
    fn untagged_streams_resolve_by_height() {
        assert_eq!(
            ColorDescription::resolve(None, None, 480).space,
            ColorSpace::Bt601
        );
        assert_eq!(
            ColorDescription::resolve(None, None, 1080).space,
            ColorSpace::Bt709
        );
        assert_eq!(
            ColorDescription::resolve(None, None, 1080).range,
            ColorRange::Limited
        );
    }

    #[test]
    fn a_declared_space_is_never_overridden_by_the_height_guess() {
        let resolved =
            ColorDescription::resolve(Some(ColorSpace::Bt2020), Some(ColorRange::Full), 480);

        assert_eq!(resolved.space, ColorSpace::Bt2020);
        assert_eq!(resolved.range, ColorRange::Full);
    }
}
