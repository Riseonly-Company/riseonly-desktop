//! Every number the sticker pipeline is allowed to invent, in one place.
//!
//! Ported from `RiseLottieRasterPolicy` in the reference, which is where these
//! values were measured. Two of them change on a desktop and both changes are
//! marked; the rest are the reference's and are not to be "tuned" without a
//! trace showing why.
//!
//! The whole file is pure, which is what lets the sticker system be tested
//! without a window, a GPU or an animation.

/// Identity of one rasterised animation.
///
/// The same sticker at two sizes is two sequences; the same sticker in two
/// cells is one. That is the entire reason a sticker-heavy chat stays smooth:
/// an emoji shown in recents and again in its pack rasterises once.
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

/// Playback cadence, in presentations per second.
///
/// Lottie sources are authored at 60 fps but read identically at 30, and
/// halving the cadence halves both the rasterisation work and the number of
/// texture writes per second. On a 165 Hz monitor this is still 30: the
/// refresh rate is what the compositor does, not what a sticker needs.
pub const MAXIMUM_FRAME_RATE: f64 = 30.0;

/// Compressed bytes one animation's frame set may retain.
///
/// Sticker art is flat vector fill over large fully transparent regions, which
/// LZ4 takes down by roughly an order of magnitude, so a whole animation fits
/// in the space a handful of raw frames used to take. That is what makes it
/// affordable to keep every visible sticker animating instead of rationing
/// playback slots.
pub const MAXIMUM_BYTES_PER_SEQUENCE: u64 = 6 * 1024 * 1024;

/// Compressed bytes across every resident sequence.
pub const MAXIMUM_CACHE_BYTES: u64 = 64 * 1024 * 1024;

/// Decoded BGRA frames waiting to be shown.
///
/// Bounded separately from the compressed set on purpose: evicting here costs
/// an LZ4 decode, evicting there costs a full vector rasterisation.
pub const MAXIMUM_DECODED_BYTES: u64 = 24 * 1024 * 1024;

/// Animations kept open at once, independently of what their frames cost.
///
/// A byte ceiling alone is not enough. Every resident sequence also holds a
/// rasteriser — for rlottie, a parsed animation model living on the C++ heap
/// that this process's ledger cannot see. A picker scrolled past hundreds of
/// small stickers therefore stays under the byte ceiling forever while
/// accumulating hundreds of C++ animations, which is invisible right up until
/// the process is enormous.
///
/// 256 is several screens' worth on the largest window anyone runs, so nothing
/// visible is ever evicted for the sake of this cap. It is the same reasoning
/// as the reference's `NSCache.countLimit`, which exists beside its cost limit
/// for exactly this class of hidden cost.
pub const MAXIMUM_RESIDENT_SEQUENCES: usize = 256;

/// The smallest and largest square a sticker is ever rasterised at.
pub const MINIMUM_DIMENSION: u32 = 32;
pub const MAXIMUM_DIMENSION: u32 = 256;

/// Bytes one decoded frame of a given side costs. BGRA, premultiplied, which is
/// what rlottie writes and what the GPU samples.
pub const fn frame_bytes(dimension: u32) -> u64 {
    dimension as u64 * dimension as u64 * 4
}

/// DESKTOP CHANGE. The reference hardcodes 2x, because every device it runs on
/// is 2x or 3x and it deliberately avoids reading `UIScreen` from a raster
/// thread. A desktop has 1x monitors, 2x monitors, and Linux compositors
/// offering 1.25 and 1.5, and the window can be dragged between them.
///
/// Snapping to 1 or 2 rather than using the factor directly is what keeps that
/// drag from being a cache flush: the scale is part of the sequence key, so a
/// continuous scale would rasterise the same sticker at every fractional size
/// the user ever passes through.
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

/// How many authored frames one presentation advances.
///
/// Frame indices always advance at the animation's own rate so the loop keeps
/// its real duration; the stride is what turns a 60 fps source into 30
/// presentations a second instead of playing it at half speed.
pub fn presentation_stride(source_frame_rate: f64) -> usize {
    if !source_frame_rate.is_finite() || source_frame_rate <= 1.0 {
        return 1;
    }

    ((source_frame_rate / MAXIMUM_FRAME_RATE).round() as usize).max(1)
}

/// Animations allowed to rasterise at once.
///
/// rlottie is pure CPU and a large sticker is milliseconds of work. Leaving
/// headroom is what keeps the raster pool from starving the GPUI thread, which
/// is the one thread whose stall the user sees.
pub fn render_concurrency(available_parallelism: usize) -> usize {
    available_parallelism.saturating_sub(2).clamp(1, 3)
}

/// Replay decodes are far cheaper than rasterisation but there is one per
/// visible sticker per presentation, so they get their own wider gate. The
/// point is to bound thread growth, not to serialise them behind rlottie.
pub fn decode_concurrency(available_parallelism: usize) -> usize {
    available_parallelism.saturating_sub(1).clamp(2, 6)
}

/// Presentations rasterised ahead of the one being shown.
///
/// Three is enough to cover the first loop of a 30 fps sticker without ever
/// blocking a tick, and small enough that a sticker scrolled away mid-loop has
/// not queued work nobody will look at.
pub const PREFETCH_PRESENTATIONS: usize = 3;

// The relationship between the budgets, checked at compile time rather than in
// a test: one sticker must not be able to consume the whole shared allowance,
// and it must be able to hold at least one frame at the largest size. A test
// would only fail after someone had already built and shipped the edit.
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
