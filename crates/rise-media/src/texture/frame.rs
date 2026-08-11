//! Frame geometry: the pixel formats a hardware decoder actually hands back,
//! and the plane arithmetic needed to describe them to a GPU. Pure — no OS and
//! no graphics API.

use rise_platform::HostOs;

/// The pixel layout of a decoded frame, in the *decoder's* vocabulary.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FrameFormat {
    /// 8-bit biplanar YUV 4:2:0. What VideoToolbox, D3D11VA and VAAPI all
    /// return for H.264 and 8-bit HEVC, and therefore the common case.
    Nv12,
    /// 10-bit biplanar YUV 4:2:0, for HEVC Main10 and 10-bit AV1.
    P010,
    /// 8-bit planar YUV 4:2:0. Software decoders and every WebRTC stack.
    I420,
    /// Already interleaved RGB. Produced by image decode, not by video.
    Bgra8,
}

impl FrameFormat {
    pub const ALL: [Self; 4] = [Self::Nv12, Self::P010, Self::I420, Self::Bgra8];

    pub const fn plane_count(self) -> usize {
        match self {
            Self::Nv12 | Self::P010 => 2,
            Self::I420 => 3,
            Self::Bgra8 => 1,
        }
    }

    /// Bytes per pixel *in the luma plane*. Chroma planes are derived from
    /// this and the subsampling.
    pub const fn bytes_per_luma_sample(self) -> u32 {
        match self {
            Self::Nv12 | Self::I420 => 1,
            Self::P010 => 2,
            Self::Bgra8 => 4,
        }
    }

    pub const fn is_yuv(self) -> bool {
        !matches!(self, Self::Bgra8)
    }

    /// Whether a shader is required to make this displayable.
    pub const fn needs_color_conversion(self) -> bool {
        self.is_yuv()
    }

    /// A frame whose dimensions are not a multiple of this cannot be described
    /// in 4:2:0 without a half-populated chroma sample.
    pub const fn dimension_alignment(self) -> u32 {
        match self {
            Self::Nv12 | Self::P010 | Self::I420 => 2,
            Self::Bgra8 => 1,
        }
    }
}

/// One plane of a frame as the GPU must be told about it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlaneLayout {
    pub width: u32,
    pub height: u32,
    /// Bytes per row, including any padding the decoder added. Never assume
    /// this equals `width * bytes_per_sample`.
    pub stride: u32,
    pub bytes_per_sample: u32,
    pub offset: u64,
}

impl PlaneLayout {
    pub const fn tight_stride(&self) -> u32 {
        self.width * self.bytes_per_sample
    }

    pub const fn is_padded(&self) -> bool {
        self.stride > self.tight_stride()
    }

    pub const fn byte_len(&self) -> u64 {
        self.stride as u64 * self.height as u64
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GeometryError {
    ZeroDimension,
    OddDimensionForSubsampledFormat {
        width: u32,
        height: u32,
    },
    StrideBelowWidth {
        plane: usize,
        stride: u32,
        needed: u32,
    },
    TooLarge {
        width: u32,
        height: u32,
    },
}

/// The largest frame accepted from a decoder, so a corrupt or hostile container
/// cannot ask for a multi-gigabyte allocation.
pub const MAX_DIMENSION: u32 = 8192;

/// A validated description of one decoded frame.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FrameGeometry {
    format: FrameFormat,
    width: u32,
    height: u32,
    planes: Vec<PlaneLayout>,
}

impl FrameGeometry {
    /// Build a geometry with the tightest legal strides. Used for buffers this
    /// crate allocates itself; a decoder's own frames use `with_strides`.
    pub fn packed(format: FrameFormat, width: u32, height: u32) -> Result<Self, GeometryError> {
        let strides = Self::plane_dimensions(format, width, height)?
            .into_iter()
            .map(|(w, _, bps)| w * bps)
            .collect::<Vec<_>>();

        Self::with_strides(format, width, height, &strides)
    }

    /// Build a geometry from the strides a decoder reported.
    pub fn with_strides(
        format: FrameFormat,
        width: u32,
        height: u32,
        strides: &[u32],
    ) -> Result<Self, GeometryError> {
        let dimensions = Self::plane_dimensions(format, width, height)?;

        if strides.len() != dimensions.len() {
            return Err(GeometryError::StrideBelowWidth {
                plane: strides.len().min(dimensions.len()),
                stride: 0,
                needed: 0,
            });
        }

        let mut planes = Vec::with_capacity(dimensions.len());
        let mut offset = 0u64;

        for (index, ((plane_width, plane_height, bytes_per_sample), stride)) in dimensions
            .into_iter()
            .zip(strides.iter().copied())
            .enumerate()
        {
            let needed = plane_width * bytes_per_sample;
            if stride < needed {
                return Err(GeometryError::StrideBelowWidth {
                    plane: index,
                    stride,
                    needed,
                });
            }

            let plane = PlaneLayout {
                width: plane_width,
                height: plane_height,
                stride,
                bytes_per_sample,
                offset,
            };
            offset += plane.byte_len();
            planes.push(plane);
        }

        Ok(Self {
            format,
            width,
            height,
            planes,
        })
    }

    fn plane_dimensions(
        format: FrameFormat,
        width: u32,
        height: u32,
    ) -> Result<Vec<(u32, u32, u32)>, GeometryError> {
        if width == 0 || height == 0 {
            return Err(GeometryError::ZeroDimension);
        }
        if width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(GeometryError::TooLarge { width, height });
        }

        let alignment = format.dimension_alignment();
        if !width.is_multiple_of(alignment) || !height.is_multiple_of(alignment) {
            return Err(GeometryError::OddDimensionForSubsampledFormat { width, height });
        }

        let luma = format.bytes_per_luma_sample();

        Ok(match format {
            // Interleaved UV is half the height but the SAME byte width as luma.
            FrameFormat::Nv12 | FrameFormat::P010 => {
                vec![(width, height, luma), (width / 2, height / 2, luma * 2)]
            }
            FrameFormat::I420 => vec![
                (width, height, 1),
                (width / 2, height / 2, 1),
                (width / 2, height / 2, 1),
            ],
            FrameFormat::Bgra8 => vec![(width, height, 4)],
        })
    }

    pub fn format(&self) -> FrameFormat {
        self.format
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn planes(&self) -> &[PlaneLayout] {
        &self.planes
    }

    /// Total bytes one frame of this geometry occupies, stride padding included.
    pub fn byte_len(&self) -> u64 {
        self.planes.iter().map(PlaneLayout::byte_len).sum()
    }

    /// Whether this geometry can keep the stream's existing texture. A
    /// resolution switch invalidates it; a stride change alone does not.
    pub fn can_reuse_texture_of(&self, other: &Self) -> bool {
        self.format == other.format && self.width == other.width && self.height == other.height
    }
}

/// The row alignment a host's graphics API demands when a CPU-side buffer is
/// copied into a texture. Only relevant on the fallback path.
pub const fn copy_row_alignment(host: HostOs) -> u32 {
    match host {
        // wgpu's COPY_BYTES_PER_ROW_ALIGNMENT.
        HostOs::Linux | HostOs::Windows => 256,
        // Metal's buffer-to-texture copies want 64 on Apple silicon.
        HostOs::MacOs => 64,
    }
}

pub const fn align_up(value: u32, alignment: u32) -> u32 {
    value.div_ceil(alignment) * alignment
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nv12_chroma_is_half_height_but_full_byte_width() {
        let geometry = FrameGeometry::packed(FrameFormat::Nv12, 1920, 1080).unwrap();
        let planes = geometry.planes();

        assert_eq!(planes.len(), 2);
        assert_eq!((planes[0].width, planes[0].height), (1920, 1080));
        assert_eq!((planes[1].width, planes[1].height), (960, 540));
        assert_eq!(
            planes[0].tight_stride(),
            planes[1].tight_stride(),
            "interleaved UV is two samples wide, so its byte width matches luma"
        );
    }

    #[test]
    fn a_packed_nv12_frame_is_exactly_one_and_a_half_bytes_per_pixel() {
        let geometry = FrameGeometry::packed(FrameFormat::Nv12, 1920, 1080).unwrap();
        assert_eq!(geometry.byte_len(), 1920 * 1080 * 3 / 2);
    }

    #[test]
    fn p010_costs_twice_nv12() {
        let nv12 = FrameGeometry::packed(FrameFormat::Nv12, 1280, 720).unwrap();
        let p010 = FrameGeometry::packed(FrameFormat::P010, 1280, 720).unwrap();
        assert_eq!(p010.byte_len(), nv12.byte_len() * 2);
    }

    #[test]
    fn i420_has_three_planes_and_matches_nv12_in_size() {
        let i420 = FrameGeometry::packed(FrameFormat::I420, 640, 480).unwrap();
        let nv12 = FrameGeometry::packed(FrameFormat::Nv12, 640, 480).unwrap();

        assert_eq!(i420.planes().len(), 3);
        assert_eq!(i420.byte_len(), nv12.byte_len());
    }

    #[test]
    fn padded_strides_are_counted_in_the_frame_cost() {
        let packed = FrameGeometry::packed(FrameFormat::Nv12, 1920, 1080).unwrap();
        let padded =
            FrameGeometry::with_strides(FrameFormat::Nv12, 1920, 1080, &[2048, 2048]).unwrap();

        assert!(padded.planes()[0].is_padded());
        assert!(
            padded.byte_len() > packed.byte_len(),
            "a ceiling computed from width*height would under-count a real decoder's frames"
        );
    }

    #[test]
    fn a_stride_narrower_than_the_row_is_rejected() {
        let error =
            FrameGeometry::with_strides(FrameFormat::Nv12, 1920, 1080, &[1024, 2048]).unwrap_err();

        assert_eq!(
            error,
            GeometryError::StrideBelowWidth {
                plane: 0,
                stride: 1024,
                needed: 1920
            }
        );
    }

    #[test]
    fn odd_dimensions_are_rejected_for_subsampled_formats_only() {
        assert_eq!(
            FrameGeometry::packed(FrameFormat::Nv12, 1921, 1080).unwrap_err(),
            GeometryError::OddDimensionForSubsampledFormat {
                width: 1921,
                height: 1080
            }
        );
        assert!(FrameGeometry::packed(FrameFormat::Bgra8, 1921, 1081).is_ok());
    }

    #[test]
    fn zero_and_oversized_dimensions_are_refused_before_any_allocation() {
        assert_eq!(
            FrameGeometry::packed(FrameFormat::Nv12, 0, 1080).unwrap_err(),
            GeometryError::ZeroDimension
        );
        assert_eq!(
            FrameGeometry::packed(FrameFormat::Nv12, 16384, 1080).unwrap_err(),
            GeometryError::TooLarge {
                width: 16384,
                height: 1080
            }
        );
    }

    #[test]
    fn every_video_format_needs_a_conversion_shader() {
        for format in FrameFormat::ALL {
            assert_eq!(format.needs_color_conversion(), format.is_yuv());
        }
        assert!(!FrameFormat::Bgra8.needs_color_conversion());
    }

    #[test]
    fn a_stride_change_alone_does_not_invalidate_the_stream_texture() {
        let first = FrameGeometry::packed(FrameFormat::Nv12, 1280, 720).unwrap();
        let restrided =
            FrameGeometry::with_strides(FrameFormat::Nv12, 1280, 720, &[1536, 1536]).unwrap();
        let resized = FrameGeometry::packed(FrameFormat::Nv12, 640, 360).unwrap();

        assert!(restrided.can_reuse_texture_of(&first));
        assert!(
            !resized.can_reuse_texture_of(&first),
            "an adaptive-bitrate resolution switch must reallocate"
        );
    }

    #[test]
    fn plane_offsets_tile_the_buffer_without_overlap() {
        let geometry = FrameGeometry::packed(FrameFormat::I420, 64, 64).unwrap();
        let mut cursor = 0u64;

        for plane in geometry.planes() {
            assert_eq!(plane.offset, cursor);
            cursor += plane.byte_len();
        }
        assert_eq!(cursor, geometry.byte_len());
    }

    #[test]
    fn row_alignment_is_known_for_every_host() {
        for host in HostOs::ALL {
            let alignment = copy_row_alignment(host);
            assert!(alignment.is_power_of_two());
            assert_eq!(align_up(1920, alignment) % alignment, 0);
            assert!(align_up(1920, alignment) >= 1920);
        }
    }
}
