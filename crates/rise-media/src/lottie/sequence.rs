//! One animation, rasterised once and replayed forever after.
//!
//! THE PROBLEM THIS SOLVES
//!
//! The obvious implementation re-rasterises every frame on every loop
//! iteration, for the life of the view. A looping sticker only ever shows
//! `frame_count` distinct images, so that is the same vector composite done
//! thousands of times for the same result. With a picker grid of them it is the
//! whole CPU.
//!
//! So each frame is rasterised once, LZ4'd, and replayed from the compressed
//! set. Sticker art is flat fill over large transparent regions and compresses
//! by roughly an order of magnitude, which is what makes keeping every visible
//! sticker animating affordable rather than something to ration.
//!
//! Ported from `RiseLottieFrameSequence`. The reference compresses with LZFSE,
//! which is Apple-only; LZ4 is the portable equivalent at this ratio and is
//! faster to decode, which matters because decode happens once per sticker per
//! presentation and rasterisation does not.

use std::sync::Arc;

use parking_lot::Mutex;

use super::raster_policy::{self, SequenceKey};

/// A rasterised frame, ready to be written into a texture.
///
/// Premultiplied ARGB packed one pixel per `u32` — what rlottie writes, and on
/// every target this ships to (all little-endian) that is BGRA in memory order,
/// which is what the atlas samples. There is no second CPU-side copy kept after
/// the GPU upload; that retention is the documented difference between 300 MB
/// and 12 MB.
///
/// Stored as `u32` rather than bytes so the buffer handed to a C rasteriser is
/// four-byte aligned by construction. A `Vec<u8>` is allocated with alignment
/// one, and the allocator returning something better in practice is not the
/// same as the pointer being valid to write `u32`s through.
#[derive(PartialEq, Eq, Debug)]
pub struct DecodedFrame {
    pub dimension: u32,
    pixels: Box<[u32]>,
}

impl DecodedFrame {
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// The same frame as bytes, for upload. Widening alignment is always sound
    /// in this direction; the reverse is what the `u32` storage exists to avoid.
    pub fn bytes(&self) -> &[u8] {
        as_bytes(&self.pixels)
    }

    pub fn byte_len(&self) -> u64 {
        self.pixels.len() as u64 * 4
    }

    /// Whether anything was actually painted.
    ///
    /// rlottie renders an empty frame rather than failing when an animation
    /// uses a feature it does not implement — text layers, expressions,
    /// embedded assets it cannot load. A caller needs to tell that apart from
    /// a legitimately blank first frame, so it samples the alpha channel.
    ///
    /// Every sixteenth pixel is enough to tell a painted frame from an empty
    /// one without walking the whole surface.
    pub fn has_visible_pixels(&self) -> bool {
        self.pixels
            .iter()
            .step_by(16)
            .any(|pixel| (pixel >> 24) > 1)
    }
}

fn as_bytes(pixels: &[u32]) -> &[u8] {
    // SAFETY: u8 has alignment 1 and every bit pattern is valid, so any
    // initialised [u32] is a valid [u8] of four times the length.
    unsafe { std::slice::from_raw_parts(pixels.as_ptr().cast::<u8>(), pixels.len() * 4) }
}

fn as_bytes_mut(pixels: &mut [u32]) -> &mut [u8] {
    let len = pixels.len() * 4;
    // SAFETY: as above, and the borrow is exclusive for the lifetime of the
    // result, so no aliasing u32 view exists while it is held.
    unsafe { std::slice::from_raw_parts_mut(pixels.as_mut_ptr().cast::<u8>(), len) }
}

/// The vector rasteriser behind a sequence.
///
/// A trait rather than a direct rlottie call for two reasons: the whole sticker
/// system is testable with a synthetic implementation and no C++, and ThorVG
/// can replace rlottie without touching anything above this line.
///
/// `render` takes `&mut self` because rlottie's animation handle carries
/// per-render state; the exclusion is real, and the sequence holds it behind a
/// lock rather than a channel because what is needed here is exclusion, not
/// ordering — frames are independent and any order produces the same cache.
pub trait LottieRasterizer: Send {
    fn frame_count(&self) -> usize;
    fn frame_rate(&self) -> f64;
    /// Write `dimension` × `dimension` premultiplied ARGB into `out`, which is
    /// exactly `dimension * dimension` pixels and is already zeroed.
    fn render(&mut self, frame: usize, dimension: u32, out: &mut [u32]);
}

/// Bounds a sequence refuses to accept from a rasteriser.
///
/// Both are the reference's. A 2400-frame animation at 30 presentations a
/// second is eighty seconds of sticker, and a source claiming 500 fps is
/// telling us its header is wrong.
pub const MAXIMUM_FRAMES: usize = 2400;
pub const MAXIMUM_SOURCE_FRAME_RATE: f64 = 120.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SequenceError {
    /// The rasteriser opened but describes an animation outside the bounds
    /// above. Treated as a bad file, not as a bug.
    ImplausibleTimeline,
}

/// Decoded frames shared across every player, with its own byte budget.
///
/// Kept separate from the compressed store so the two budgets are independent:
/// this one can be evicted freely because refilling it is an LZ4 decode, not a
/// rasterisation. That asymmetry is the reason the reference has two caches and
/// not one, and it is what makes memory pressure cost a decode instead of
/// freezing every sticker on screen.
#[derive(Debug)]
pub struct DecodedFrameCache {
    ceiling_bytes: u64,
    state: Mutex<DecodedFrameCacheState>,
}

#[derive(Debug, Default)]
struct DecodedFrameCacheState {
    entries: Vec<(DecodedKey, Arc<DecodedFrame>, u64)>,
    bytes: u64,
    clock: u64,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct DecodedKey {
    sequence: SequenceKey,
    frame: usize,
}

impl DecodedFrameCache {
    pub fn new(ceiling_bytes: u64) -> Self {
        Self {
            ceiling_bytes,
            state: Mutex::new(DecodedFrameCacheState::default()),
        }
    }

    pub fn bytes(&self) -> u64 {
        self.state.lock().bytes
    }

    pub fn ceiling(&self) -> u64 {
        self.ceiling_bytes
    }

    pub fn get(&self, sequence: &SequenceKey, frame: usize) -> Option<Arc<DecodedFrame>> {
        let mut state = self.state.lock();
        state.clock += 1;
        let used = state.clock;

        let position = state
            .entries
            .iter()
            .position(|(key, _, _)| key.frame == frame && &key.sequence == sequence)?;

        state.entries[position].2 = used;
        Some(state.entries[position].1.clone())
    }

    pub fn insert(&self, sequence: &SequenceKey, frame: usize, decoded: Arc<DecodedFrame>) {
        let cost = decoded.byte_len();
        let mut state = self.state.lock();

        if state
            .entries
            .iter()
            .any(|(key, _, _)| key.frame == frame && &key.sequence == sequence)
        {
            return;
        }

        state.clock += 1;
        let used = state.clock;
        state.entries.push((
            DecodedKey {
                sequence: sequence.clone(),
                frame,
            },
            decoded,
            used,
        ));
        state.bytes += cost;

        while state.bytes > self.ceiling_bytes && state.entries.len() > 1 {
            let Some(victim) = state
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, _, used))| *used)
                .map(|(index, _)| index)
            else {
                break;
            };
            let (_, evicted, _) = state.entries.remove(victim);
            state.bytes -= evicted.byte_len();
        }
    }

    /// The first response to memory pressure.
    ///
    /// Nothing stops animating and no rlottie work is repeated — unlike
    /// dropping the compressed set, which would put every visible sticker back
    /// through a full rasterisation.
    pub fn clear(&self) {
        let mut state = self.state.lock();
        state.entries.clear();
        state.bytes = 0;
    }

    fn forget(&self, sequence: &SequenceKey) {
        let mut state = self.state.lock();
        let mut freed = 0;
        state.entries.retain(|(key, frame, _)| {
            let keep = &key.sequence != sequence;
            if !keep {
                freed += frame.byte_len();
            }
            keep
        });
        state.bytes -= freed.min(state.bytes);
    }
}

#[derive(Debug, Default)]
struct SequenceState {
    compressed: Vec<Option<Box<[u8]>>>,
    compressed_bytes: u64,
    torn_down: bool,
}

/// One parsed animation plus its compressed frame set.
pub struct FrameSequence {
    key: SequenceKey,
    frame_count: usize,
    frame_rate: f64,
    dimension: u32,
    rasterizer: Mutex<Option<Box<dyn LottieRasterizer>>>,
    state: Mutex<SequenceState>,
    decoded: Arc<DecodedFrameCache>,
}

impl std::fmt::Debug for FrameSequence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrameSequence")
            .field("key", &self.key)
            .field("frame_count", &self.frame_count)
            .field("frame_rate", &self.frame_rate)
            .finish_non_exhaustive()
    }
}

impl FrameSequence {
    pub fn open(
        key: SequenceKey,
        rasterizer: Box<dyn LottieRasterizer>,
        decoded: Arc<DecodedFrameCache>,
    ) -> Result<Self, SequenceError> {
        let frame_count = rasterizer.frame_count();
        let frame_rate = rasterizer.frame_rate();

        if frame_count == 0
            || frame_count > MAXIMUM_FRAMES
            || !frame_rate.is_finite()
            || frame_rate <= 0.0
            || frame_rate > MAXIMUM_SOURCE_FRAME_RATE
        {
            return Err(SequenceError::ImplausibleTimeline);
        }

        let dimension = key.dimension;

        Ok(Self {
            key,
            frame_count,
            frame_rate,
            dimension,
            rasterizer: Mutex::new(Some(rasterizer)),
            state: Mutex::new(SequenceState {
                compressed: vec![None; frame_count],
                compressed_bytes: 0,
                torn_down: false,
            }),
            decoded,
        })
    }

    pub fn key(&self) -> &SequenceKey {
        &self.key
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub fn frame_rate(&self) -> f64 {
        self.frame_rate
    }

    pub fn dimension(&self) -> u32 {
        self.dimension
    }

    pub fn frame_bytes(&self) -> u64 {
        raster_policy::frame_bytes(self.dimension)
    }

    pub fn compressed_bytes(&self) -> u64 {
        self.state.lock().compressed_bytes
    }

    pub fn presentation_stride(&self) -> usize {
        raster_policy::presentation_stride(self.frame_rate)
    }

    fn wrap(&self, index: usize) -> usize {
        index % self.frame_count
    }

    /// Non-blocking read of an already decoded frame.
    ///
    /// Safe to call from the display tick: it touches neither the rasteriser
    /// nor the compressed store, so a tick can never be blocked behind a
    /// rasterisation happening on a worker.
    pub fn cached_frame(&self, index: usize) -> Option<Arc<DecodedFrame>> {
        self.decoded.get(&self.key, self.wrap(index))
    }

    /// Produce a frame: decoded-cache hit, else LZ4 decode, else rasterise.
    ///
    /// Blocking, and meant to be called from a raster worker rather than from
    /// the GPUI thread.
    pub fn frame(&self, index: usize) -> Option<Arc<DecodedFrame>> {
        let wrapped = self.wrap(index);

        if let Some(hit) = self.decoded.get(&self.key, wrapped) {
            return Some(hit);
        }

        let compressed = {
            let state = self.state.lock();
            if state.torn_down {
                return None;
            }
            state.compressed[wrapped].clone()
        };

        let decoded = match compressed {
            // Replay: no rasteriser involvement at all, so a warm sticker costs
            // an LZ4 decode rather than a full vector composite.
            Some(bytes) => Arc::new(self.decompress(&bytes)?),
            None => {
                let (frame, bytes) = self.rasterize_compressing(wrapped)?;
                self.retain_compressed(wrapped, bytes);
                Arc::new(frame)
            }
        };

        self.decoded.insert(&self.key, wrapped, decoded.clone());
        Some(decoded)
    }

    /// Index of the first sampled frame that actually paints something.
    ///
    /// `None` means the animation uses features the rasteriser cannot render
    /// and comes out empty, which the caller uses to fall back to a static
    /// poster rather than showing a hole. Rendered frames stay cached, so the
    /// probe is not wasted work.
    pub fn first_visible_frame(&self) -> Option<usize> {
        let last = self.frame_count.saturating_sub(1);
        let mut probed = Vec::new();

        for index in [0, last / 4, last / 2] {
            if probed.contains(&index) {
                continue;
            }
            probed.push(index);

            if let Some(frame) = self.frame(index)
                && frame.has_visible_pixels()
            {
                return Some(index);
            }
        }

        None
    }

    /// Release everything and refuse further work.
    ///
    /// Called by the cache when a sequence is evicted. The rasteriser is
    /// dropped here rather than in `Drop` so eviction frees the C++ animation
    /// immediately instead of whenever the last `Arc` happens to go.
    pub fn teardown(&self) {
        {
            let mut state = self.state.lock();
            state.torn_down = true;
            state.compressed = vec![None; self.frame_count];
            state.compressed_bytes = 0;
        }
        self.rasterizer.lock().take();
        self.decoded.forget(&self.key);
    }

    fn retain_compressed(&self, index: usize, bytes: Box<[u8]>) {
        let mut state = self.state.lock();

        if state.torn_down || state.compressed[index].is_some() {
            return;
        }
        if state.compressed_bytes + bytes.len() as u64 > raster_policy::MAXIMUM_BYTES_PER_SEQUENCE {
            return;
        }

        state.compressed_bytes += bytes.len() as u64;
        state.compressed[index] = Some(bytes);
    }

    fn rasterize_compressing(&self, index: usize) -> Option<(DecodedFrame, Box<[u8]>)> {
        let mut guard = self.rasterizer.lock();
        let rasterizer = guard.as_mut()?;

        let mut pixels = vec![0u32; (self.dimension * self.dimension) as usize];
        rasterizer.render(index, self.dimension, &mut pixels);

        let compressed = lz4_flex::compress_prepend_size(as_bytes(&pixels)).into_boxed_slice();

        Some((
            DecodedFrame {
                dimension: self.dimension,
                pixels: pixels.into_boxed_slice(),
            },
            compressed,
        ))
    }

    fn decompress(&self, compressed: &[u8]) -> Option<DecodedFrame> {
        let mut pixels = vec![0u32; (self.dimension * self.dimension) as usize];

        let written =
            lz4_flex::decompress_into(compressed.get(4..)?, as_bytes_mut(&mut pixels)).ok()?;
        if written as u64 != raster_policy::frame_bytes(self.dimension) {
            return None;
        }

        Some(DecodedFrame {
            dimension: self.dimension,
            pixels: pixels.into_boxed_slice(),
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A rasteriser that paints a solid colour derived from the frame index and
    /// counts how often it was asked to. The count is the assertion the whole
    /// design exists for.
    pub(crate) struct CountingRasterizer {
        pub frames: usize,
        pub rate: f64,
        pub renders: Arc<AtomicUsize>,
        pub blank: bool,
    }

    impl CountingRasterizer {
        pub fn new(frames: usize, rate: f64) -> (Self, Arc<AtomicUsize>) {
            let renders = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    frames,
                    rate,
                    renders: renders.clone(),
                    blank: false,
                },
                renders,
            )
        }
    }

    impl LottieRasterizer for CountingRasterizer {
        fn frame_count(&self) -> usize {
            self.frames
        }

        fn frame_rate(&self) -> f64 {
            self.rate
        }

        fn render(&mut self, frame: usize, dimension: u32, out: &mut [u32]) {
            self.renders.fetch_add(1, Ordering::Relaxed);
            assert_eq!(out.len(), (dimension * dimension) as usize);

            if self.blank {
                return;
            }
            let colour = 0xFF00_0000 | ((frame as u32 % 251) * 0x0001_0101);
            out.fill(colour);
        }
    }

    pub(crate) fn sequence(frames: usize, rate: f64) -> (FrameSequence, Arc<AtomicUsize>) {
        let (rasterizer, renders) = CountingRasterizer::new(frames, rate);
        let cache = Arc::new(DecodedFrameCache::new(raster_policy::MAXIMUM_DECODED_BYTES));
        (
            FrameSequence::open(SequenceKey::new("sticker", 64), Box::new(rasterizer), cache)
                .unwrap(),
            renders,
        )
    }

    #[test]
    fn a_looping_sticker_rasterises_each_frame_exactly_once() {
        let (sequence, renders) = sequence(12, 60.0);

        for _loop_iteration in 0..40 {
            for index in 0..12 {
                assert!(sequence.frame(index).is_some());
            }
        }

        assert_eq!(
            renders.load(Ordering::Relaxed),
            12,
            "forty loops of a twelve-frame sticker must cost twelve rasterisations"
        );
    }

    #[test]
    fn an_evicted_decoded_frame_costs_a_decode_and_not_a_rasterisation() {
        let (rasterizer, renders) = CountingRasterizer::new(8, 30.0);
        // Room for one frame only, so every read after the first evicts.
        let cache = Arc::new(DecodedFrameCache::new(raster_policy::frame_bytes(64)));
        let sequence =
            FrameSequence::open(SequenceKey::new("sticker", 64), Box::new(rasterizer), cache)
                .unwrap();

        for _ in 0..5 {
            for index in 0..8 {
                assert!(sequence.frame(index).is_some());
            }
        }

        assert_eq!(renders.load(Ordering::Relaxed), 8);
    }

    #[test]
    fn frames_survive_the_compression_round_trip_byte_for_byte() {
        let (sequence, _) = sequence(4, 30.0);

        let first = sequence.frame(2).unwrap();
        sequence.decoded.clear();
        let replayed = sequence.frame(2).unwrap();

        assert_eq!(first, replayed);
    }

    #[test]
    fn an_index_past_the_end_wraps_instead_of_panicking() {
        let (sequence, renders) = sequence(5, 30.0);

        let wrapped = sequence.frame(7).unwrap();
        let direct = sequence.frame(2).unwrap();

        assert_eq!(wrapped, direct);
        assert_eq!(renders.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn the_compressed_set_stays_under_the_per_sequence_ceiling() {
        let (rasterizer, _) = CountingRasterizer::new(2000, 60.0);
        let cache = Arc::new(DecodedFrameCache::new(raster_policy::MAXIMUM_DECODED_BYTES));
        let sequence = FrameSequence::open(
            SequenceKey::new("huge", raster_policy::MAXIMUM_DIMENSION),
            Box::new(rasterizer),
            cache,
        )
        .unwrap();

        for index in 0..2000 {
            sequence.frame(index);
        }

        assert!(
            sequence.compressed_bytes() <= raster_policy::MAXIMUM_BYTES_PER_SEQUENCE,
            "one animation grew to {} bytes",
            sequence.compressed_bytes()
        );
    }

    #[test]
    fn an_implausible_timeline_is_refused_rather_than_played() {
        let cache = Arc::new(DecodedFrameCache::new(raster_policy::MAXIMUM_DECODED_BYTES));

        for (frames, rate) in [
            (0, 30.0),
            (MAXIMUM_FRAMES + 1, 30.0),
            (10, 0.0),
            (10, -30.0),
            (10, 500.0),
            (10, f64::NAN),
        ] {
            let (rasterizer, _) = CountingRasterizer::new(frames, rate);
            assert_eq!(
                FrameSequence::open(
                    SequenceKey::new("bad", 64),
                    Box::new(rasterizer),
                    cache.clone()
                )
                .err(),
                Some(SequenceError::ImplausibleTimeline),
                "frames={frames} rate={rate}"
            );
        }
    }

    #[test]
    fn an_animation_the_rasteriser_cannot_paint_reports_no_visible_frame() {
        let (mut rasterizer, _) = CountingRasterizer::new(20, 30.0);
        rasterizer.blank = true;
        let cache = Arc::new(DecodedFrameCache::new(raster_policy::MAXIMUM_DECODED_BYTES));
        let sequence =
            FrameSequence::open(SequenceKey::new("blank", 64), Box::new(rasterizer), cache)
                .unwrap();

        assert_eq!(sequence.first_visible_frame(), None);
    }

    #[test]
    fn a_painted_animation_reports_its_first_visible_frame() {
        let (sequence, _) = sequence(20, 30.0);

        assert_eq!(sequence.first_visible_frame(), Some(0));
    }

    #[test]
    fn teardown_releases_the_rasteriser_and_refuses_further_frames() {
        let (sequence, renders) = sequence(6, 30.0);
        sequence.frame(0);

        sequence.teardown();

        assert_eq!(sequence.frame(0), None);
        assert_eq!(sequence.compressed_bytes(), 0);
        assert_eq!(renders.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn the_decoded_cache_holds_its_ceiling_across_many_sequences() {
        let ceiling = raster_policy::frame_bytes(64) * 4;
        let cache = Arc::new(DecodedFrameCache::new(ceiling));

        for sticker in 0..10 {
            let (rasterizer, _) = CountingRasterizer::new(6, 30.0);
            let sequence = FrameSequence::open(
                SequenceKey::new(format!("sticker-{sticker}"), 64),
                Box::new(rasterizer),
                cache.clone(),
            )
            .unwrap();

            for index in 0..6 {
                sequence.frame(index);
                assert!(
                    cache.bytes() <= ceiling,
                    "sticker {sticker} frame {index} pushed the cache to {}",
                    cache.bytes()
                );
            }
        }
    }
}
