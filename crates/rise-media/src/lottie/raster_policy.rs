//! Every number the sticker pipeline is allowed to invent, in one place. These
//! are measured values, not to be tuned without a trace showing why.

/// Identity of one rasterised animation. The same sticker at two sizes is two
/// sequences; the same sticker in two cells is one.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SequenceKey {
    pub cache_key: String,
    pub dimension: u32,
}

impl SequenceKey {
    pub fn new(cache_key: impl Into<String>, dimension: u32) -> Self {
        Self {
            cache_key: cache_key.into(),
            dimension,
        }
    }
}

/// Playback cadence, in presentations per second. Independent of the monitor's
/// refresh rate: 30 on a 165 Hz display too.
pub const MAXIMUM_FRAME_RATE: f64 = 30.0;

/// Compressed bytes one animation's frame set may retain.
pub const MAXIMUM_BYTES_PER_SEQUENCE: u64 = 6 * 1024 * 1024;

/// Compressed bytes across every resident sequence.
pub const MAXIMUM_CACHE_BYTES: u64 = 64 * 1024 * 1024;

/// Decoded BGRA frames waiting to be shown. Bounded separately from the
/// compressed set: evicting here costs a decode, there a rasterisation.
pub const MAXIMUM_DECODED_BYTES: u64 = 24 * 1024 * 1024;

/// Animations kept open at once, independently of what their frames cost. Each
/// resident sequence also holds a rasteriser whose parsed animation lives on
/// the C++ heap, which the byte ledger cannot see.
pub const MAXIMUM_RESIDENT_SEQUENCES: usize = 256;

/// The smallest and largest square a sticker is ever rasterised at.
pub const MINIMUM_DIMENSION: u32 = 32;
pub const MAXIMUM_DIMENSION: u32 = 256;

/// Bytes one decoded frame of a given side costs. BGRA, premultiplied, which is
/// what rlottie writes and what the GPU samples.
pub const fn frame_bytes(dimension: u32) -> u64 {
    dimension as u64 * dimension as u64 * 4
}

/// The integer scale a sticker is rasterised at. Snapped to 1 or 2 rather than
/// used directly: the scale is part of the sequence key, so a fractional
/// compositor scale would rasterise the same sticker at every size a window
/// passes through while being dragged between monitors.
pub fn raster_scale(scale_factor: f32) -> u32 {
    if scale_factor.is_finite() && scale_factor >= 1.5 {
        2
    } else {
        1
    }
}

/// The square a sticker of this point size is rasterised at.
pub fn dimension(point_side: f32, scale_factor: f32) -> u32 {
    let scale = raster_scale(scale_factor);
    let scaled = if point_side.is_finite() && point_side > 0.0 {
        (point_side * scale as f32).round() as i64
    } else {
        0
    };

    scaled.clamp(MINIMUM_DIMENSION as i64, MAXIMUM_DIMENSION as i64) as u32
}

/// How many authored frames one presentation advances. Indices advance at the
/// animation's own rate, so a 60 fps source becomes 30 presentations a second
/// rather than playing at half speed.
pub fn presentation_stride(source_frame_rate: f64) -> usize {
    if !source_frame_rate.is_finite() || source_frame_rate <= 1.0 {
        return 1;
    }

    ((source_frame_rate / MAXIMUM_FRAME_RATE).round() as usize).max(1)
}

/// Animations allowed to rasterise at once. Leaves headroom so the raster pool
/// cannot starve the GPUI thread.
pub fn render_concurrency(available_parallelism: usize) -> usize {
    available_parallelism.saturating_sub(2).clamp(1, 3)
}

/// Replay decodes allowed at once. Wider than the raster gate: there is one
/// decode per visible sticker per presentation.
pub fn decode_concurrency(available_parallelism: usize) -> usize {
    available_parallelism.saturating_sub(1).clamp(2, 6)
}

/// Presentations rasterised ahead of the one being shown.
pub const PREFETCH_PRESENTATIONS: usize = 3;

const _: () = assert!(MAXIMUM_BYTES_PER_SEQUENCE < MAXIMUM_CACHE_BYTES);
const _: () = assert!(MAXIMUM_BYTES_PER_SEQUENCE >= frame_bytes(MAXIMUM_DIMENSION));
const _: () = assert!(MAXIMUM_DECODED_BYTES >= frame_bytes(MAXIMUM_DIMENSION));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stride_keeps_authored_duration_for_fast_sources() {
        assert_eq!(presentation_stride(60.0), 2);
        assert_eq!(presentation_stride(30.0), 1);
        assert_eq!(presentation_stride(120.0), 4);
    }

    #[test]
    fn stride_never_stalls_slow_or_degenerate_sources() {
        assert_eq!(presentation_stride(24.0), 1);
        assert_eq!(presentation_stride(0.0), 1);
        assert_eq!(presentation_stride(-5.0), 1);
        assert_eq!(presentation_stride(f64::NAN), 1);
    }

    #[test]
    fn dimension_is_bounded_and_proportional_at_2x() {
        assert_eq!(dimension(20.0, 2.0), 40);
        assert_eq!(dimension(74.0, 2.0), 148);
        assert_eq!(dimension(180.0, 2.0), 256);
        assert_eq!(dimension(4.0, 2.0), 32);
        assert_eq!(dimension(0.0, 2.0), 32);
    }

    #[test]
    fn a_one_x_monitor_rasterises_at_the_point_size() {
        assert_eq!(dimension(74.0, 1.0), 74);
        assert_eq!(dimension(300.0, 1.0), 256);
    }

    #[test]
    fn fractional_scales_snap_so_dragging_between_monitors_is_not_a_cache_flush() {
        assert_eq!(raster_scale(1.0), 1);
        assert_eq!(raster_scale(1.25), 1);
        assert_eq!(raster_scale(1.5), 2);
        assert_eq!(raster_scale(2.0), 2);
        assert_eq!(raster_scale(3.0), 2);
        assert_eq!(raster_scale(f32::NAN), 1);
    }

    #[test]
    fn render_concurrency_leaves_headroom_on_every_machine_size() {
        for cores in 1..=64 {
            let concurrency = render_concurrency(cores);
            assert!((1..=3).contains(&concurrency), "{cores} cores");
        }
        assert_eq!(render_concurrency(1), 1);
        assert_eq!(render_concurrency(8), 3);
    }

    #[test]
    fn decode_concurrency_is_wider_than_raster_concurrency() {
        for cores in 1..=64 {
            assert!(
                decode_concurrency(cores) >= render_concurrency(cores),
                "{cores} cores"
            );
        }
    }
}
