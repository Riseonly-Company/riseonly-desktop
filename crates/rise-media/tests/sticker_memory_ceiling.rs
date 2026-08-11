//! Scroll a sticker picker and hold three ceilings: the compressed frame sets,
//! the decoded frames, and the GPU atlas.

use std::sync::Arc;

use rise_media::lottie::atlas::{TextureAtlas, Tile};
use rise_media::lottie::frame_cache::FrameCache;
use rise_media::lottie::raster_policy::{self, SequenceKey};
use rise_media::lottie::sequence::LottieRasterizer;
use rise_media::texture::external_texture::BudgetTracker;

const PICKER_LENGTH: usize = 500;
const VISIBLE_STICKERS: usize = 40;
const FRAMES_PER_STICKER: usize = 30;
const STICKER_DIMENSION: u32 = 64;

/// Stands in for rlottie, painting a real frame so the caches do real work.
struct SyntheticRasterizer;

impl LottieRasterizer for SyntheticRasterizer {
    fn frame_count(&self) -> usize {
        FRAMES_PER_STICKER
    }

    fn frame_rate(&self) -> f64 {
        60.0
    }

    fn render(&mut self, frame: usize, _dimension: u32, out: &mut [u32]) {
        // A gradient, not a flat fill: a solid colour compresses to nothing.
        for (index, pixel) in out.iter_mut().enumerate() {
            let shade = ((index + frame * 7) % 256) as u32;
            *pixel = 0xFF00_0000 | (shade << 16) | (shade << 8) | shade;
        }
    }
}

struct Picker {
    cache: Arc<FrameCache>,
    atlas: TextureAtlas,
    live: Vec<(SequenceKey, Tile)>,
    peak_compressed: u64,
    peak_atlas_pages: usize,
}

impl Picker {
    fn new() -> Self {
        Self {
            cache: Arc::new(FrameCache::new()),
            atlas: TextureAtlas::new(BudgetTracker::default()),
            live: Vec::new(),
            peak_compressed: 0,
            peak_atlas_pages: 0,
        }
    }

    /// Bring one more sticker on screen and let the oldest one leave.
    fn scroll_to(&mut self, index: usize) {
        let key = SequenceKey::new(format!("sticker-{index}"), STICKER_DIMENSION);

        let sequence = self
            .cache
            .open(key.clone(), Box::new(SyntheticRasterizer))
            .expect("a synthetic animation always opens");
        self.cache.retain(&key);

        let tile = self
            .atlas
            .allocate(STICKER_DIMENSION)
            .expect("the atlas must have room for what is on screen");
        self.live.push((key, tile));

        // One loop at the presentation stride: all the display driver asks for.
        let stride = sequence.presentation_stride();
        for frame in (0..FRAMES_PER_STICKER).step_by(stride) {
            sequence.frame(frame);
        }
        self.cache.sweep();

        while self.live.len() > VISIBLE_STICKERS {
            let (gone, tile) = self.live.remove(0);
            self.cache.release(&gone);
            self.atlas.release(tile);
        }

        self.peak_compressed = self.peak_compressed.max(self.cache.compressed_bytes());
        self.peak_atlas_pages = self.peak_atlas_pages.max(self.atlas.page_count());
    }
}

#[test]
fn scrolling_a_picker_never_crosses_the_compressed_frame_ceiling() {
    let mut picker = Picker::new();

    for index in 0..PICKER_LENGTH {
        picker.scroll_to(index);

        assert!(
            picker.cache.compressed_bytes() <= raster_policy::MAXIMUM_CACHE_BYTES,
            "after {index} stickers the cache held {} bytes",
            picker.cache.compressed_bytes()
        );
        assert!(
            picker.cache.decoded_frames().bytes() <= raster_policy::MAXIMUM_DECODED_BYTES,
            "after {index} stickers the decoded pool held {} bytes",
            picker.cache.decoded_frames().bytes()
        );
    }

    assert!(
        picker.peak_compressed > 0,
        "nothing was ever rasterised, so this proved nothing"
    );
}

#[test]
fn a_five_hundred_sticker_picker_costs_what_a_forty_sticker_one_does() {
    let mut short = Picker::new();
    for index in 0..VISIBLE_STICKERS {
        short.scroll_to(index);
    }

    let mut long = Picker::new();
    for index in 0..PICKER_LENGTH {
        long.scroll_to(index);
    }

    assert_eq!(
        long.peak_atlas_pages, short.peak_atlas_pages,
        "the atlas grew with how far the user scrolled"
    );
    assert!(
        long.cache.resident_count() <= raster_policy::MAXIMUM_RESIDENT_SEQUENCES,
        "{} animations stayed open after scrolling past {PICKER_LENGTH}",
        long.cache.resident_count()
    );
    assert!(
        long.peak_compressed <= raster_policy::MAXIMUM_CACHE_BYTES,
        "the compressed set reached {} bytes",
        long.peak_compressed
    );
}

#[test]
fn a_full_loop_of_every_visible_sticker_rasterises_nothing_twice() {
    let cache = FrameCache::new();
    let mut sequences = Vec::new();

    for index in 0..VISIBLE_STICKERS {
        let key = SequenceKey::new(format!("sticker-{index}"), STICKER_DIMENSION);
        sequences.push(cache.open(key, Box::new(SyntheticRasterizer)).unwrap());
    }

    let before = cache.compressed_bytes();
    for _ in 0..20 {
        for sequence in &sequences {
            for frame in (0..FRAMES_PER_STICKER).step_by(sequence.presentation_stride()) {
                sequence.frame(frame);
            }
        }
    }
    cache.sweep();

    let after = cache.compressed_bytes();
    assert!(
        after >= before,
        "the cache shrank while frames were being produced"
    );

    // Twenty loops must cost exactly one loop's worth of retained frames.
    let one_loop: u64 = sequences
        .iter()
        .map(|sequence| sequence.compressed_bytes())
        .sum();
    assert_eq!(after, one_loop);
}

#[test]
fn leaving_the_picker_returns_every_byte_of_gpu_memory() {
    let tracker = BudgetTracker::default();
    let mut atlas = TextureAtlas::new(tracker.clone());

    let tiles: Vec<Tile> = (0..PICKER_LENGTH)
        .map(|_| atlas.allocate(STICKER_DIMENSION).unwrap())
        .collect();
    assert!(tracker.in_use() > 0);

    for tile in tiles {
        atlas.release(tile);
    }
    atlas.release_all();

    assert_eq!(
        tracker.in_use(),
        0,
        "closing the picker must release the atlas, not merely stop drawing it"
    );
}

/// Resident set size in bytes, or `None` where the OS will not report it.
fn resident_bytes() -> Option<u64> {
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return None;
    }

    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;

    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|kib| kib * 1024)
}

#[test]
fn the_process_itself_does_not_grow_across_a_long_picker_scroll() {
    let Some(before) = resident_bytes() else {
        eprintln!("skipped: resident set size is not readable on this host");
        return;
    };

    {
        let mut picker = Picker::new();
        for index in 0..PICKER_LENGTH {
            picker.scroll_to(index);
        }
        picker.cache.release_unretained();
        picker.atlas.release_all();
    }

    let after = resident_bytes().expect("rss was readable a moment ago");
    let growth = after.saturating_sub(before);

    // Generous: this catches a leak proportional to scroll depth, not noise.
    const ALLOWED_GROWTH: u64 = 128 * 1024 * 1024;

    assert!(
        growth < ALLOWED_GROWTH,
        "scrolling {PICKER_LENGTH} stickers grew RSS by {} MiB",
        growth / 1024 / 1024
    );
}
