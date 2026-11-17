use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use futures::channel::oneshot;
use gpui::{AsyncApp, BackgroundExecutor, RenderImage};
use image::{Delay, Frame, RgbaImage};
use parking_lot::Mutex;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::ns_cache::NSCache;
use rise_media::lottie::rlottie_backend::RlottieRasterizer;
use rise_platform::process_info::active_processor_count;
use smallvec::SmallVec;
use tokio::sync::Semaphore;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::clever_image_disk_cache::CleverImageSharedLoadRegistry;
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::RiseLottiePreparationCancellation;

/// identity of one rasterized animation
/// same sticker at two cell sizes is two sequences, same sticker in two cells is one
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RiseLottieSequenceKey {
    pub cache_key: String,
    pub dimension: i64,
    pub mirrored: bool,
}

pub enum RiseLottieRasterPolicy {}

static MAXIMUM_FRAME_RATE: LazyLock<f64> =
    LazyLock::new(|| RiseLimits::cache_budget("lottie_frame_rate") as f64);
static MAXIMUM_BYTES_PER_SEQUENCE: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::cache_budget("lottie_sequence_bytes"));
static MAXIMUM_CACHE_BYTES: LazyLock<i64> = LazyLock::new(|| RiseLimits::cache_budget("lottie_cache_bytes"));
static MAXIMUM_DECODED_BYTES: LazyLock<i64> = LazyLock::new(|| RiseLimits::cache_budget("lottie_decoded_bytes"));

impl RiseLottieRasterPolicy {
    pub const MAXIMUM_DIMENSION: i64 = 256;

    /// playback cadence for art rasterized at full MAXIMUM_DIMENSION
    /// sources are authored at 60 fps and play at that so stickers move smooth
    /// rasterized once and replayed compressed, so first loop costs and more bytes
    pub fn maximum_frame_rate() -> f64 {
        *MAXIMUM_FRAME_RATE
    }

    /// budgets below count compressed bytes
    /// sticker art is flat vector fill over big transparent regions and compresses about 10x
    /// so a whole animation frame set fits in what a few raw frames would take
    pub fn maximum_bytes_per_sequence() -> i64 {
        *MAXIMUM_BYTES_PER_SEQUENCE
    }

    pub fn maximum_cache_bytes() -> i64 {
        *MAXIMUM_CACHE_BYTES
    }

    pub fn maximum_decoded_bytes() -> i64 {
        *MAXIMUM_DECODED_BYTES
    }

    /// 2x the point size
    /// keeps the 256px ceiling for big chat stickers without forcing a 20pt inline emoji to 72x72
    /// independent of window scale on purpose since the raster threads have no window to ask
    pub fn dimension(for_point_side: f32) -> i64 {
        Self::MAXIMUM_DIMENSION.min(32_i64.max((for_point_side * 2.0).round() as i64))
    }

    fn maximum_pixels_per_second() -> f64 {
        (Self::MAXIMUM_DIMENSION * Self::MAXIMUM_DIMENSION) as f64 * Self::maximum_frame_rate()
    }

    /// cadence a sequence may play at
    /// full size sticker gets exactly maximum_frame_rate, smaller art is cheaper so it plays faster
    /// never above the authored rate
    pub fn presentation_frame_rate(source_frame_rate: f64, pixels_per_frame: i64) -> f64 {
        let source = source_frame_rate.max(1.0);
        if pixels_per_frame <= 0 {
            return source.min(Self::maximum_frame_rate());
        }
        let affordable = Self::maximum_pixels_per_second() / pixels_per_frame as f64;
        source.min(Self::maximum_frame_rate().max(affordable))
    }

    /// how many authored frames one presentation advances
    /// frame indices advance at the animation own rate so the loop keeps its real duration
    /// the stride turns a 120 fps source into 60 presentations a second
    pub fn presentation_stride(source_frame_rate: f64, pixels_per_frame: i64) -> i64 {
        let source = source_frame_rate.max(1.0);
        let cadence = Self::presentation_frame_rate(source, pixels_per_frame);
        1_i64.max((source / cadence).round() as i64)
    }

    /// number of animations allowed to rasterize at once
    /// rlottie is pure cpu and the headroom keeps the render pool from starving the compositor
    /// and main thread
    pub fn render_concurrency() -> i64 {
        1_i64.max(3_i64.min(active_processor_count() as i64 - 2))
    }

    pub fn decode_concurrency() -> i64 {
        2_i64.max(6_i64.min(active_processor_count() as i64 - 1))
    }
}

/// decoded frames ready to draw, shared across every player
/// separate from the compressed store so the two budgets are independent
/// this one evicts freely since refilling is a decode and not a rasterization
struct RiseLottieDecodedFrameCache {
    cache: NSCache<String, Arc<RenderImage>>,
}

static DECODED_FRAME_CACHE: LazyLock<RiseLottieDecodedFrameCache> = LazyLock::new(RiseLottieDecodedFrameCache::new);

impl RiseLottieDecodedFrameCache {
    fn shared() -> &'static Self {
        &DECODED_FRAME_CACHE
    }

    fn count_limit() -> i64 {
        RiseLimits::client_value("lottie_decoded_frame_cache_entries")
    }

    fn new() -> Self {
        let cache = NSCache::new();
        cache.set_total_cost_limit(usize::try_from(RiseLottieRasterPolicy::maximum_decoded_bytes()).unwrap_or(0));
        cache.set_count_limit(usize::try_from(Self::count_limit()).unwrap_or(0));
        Self { cache }
    }

    fn image(&self, key: &str) -> Option<Arc<RenderImage>> {
        self.cache.object(&key.to_owned())
    }

    fn insert(&self, image: Arc<RenderImage>, key: String, cost: i64) {
        self.cache.set_object_cost(image, key, usize::try_from(cost).unwrap_or(0));
    }

    fn remove_all(&self) {
        self.cache.remove_all_objects();
    }
}

enum RiseLottieFrameCodec {}

impl RiseLottieFrameCodec {
    fn compress(source: &[u8]) -> Option<Vec<u8>> {
        let written = lz4_flex::block::compress(source);
        if written.is_empty() {
            return None;
        }
        Some(written)
    }

    fn decompress(data: &[u8], byte_count: i64) -> Option<Vec<u8>> {
        let byte_count = usize::try_from(byte_count).ok()?;
        let mut destination = vec![0u8; byte_count];
        let produced = lz4_flex::block::decompress_into(data, &mut destination).ok()?;
        if produced != byte_count {
            return None;
        }
        Some(destination)
    }
}

struct RiseLottieFrameSequenceFrameRequest {
    is_needed: Arc<dyn Fn() -> bool + Send + Sync>,
    completion: Option<Box<dyn FnOnce(Option<Arc<RenderImage>>) + Send>>,
}

struct RiseLottieFrameSequenceState {
    compressed_frames: Vec<Option<Arc<[u8]>>>,
    inflight: HashMap<i64, Vec<RiseLottieFrameSequenceFrameRequest>>,
    compressed_bytes: i64,
    is_torn_down: bool,
    visible_frame_index: Option<i64>,
    resting_frame_index: Option<i64>,
}

static RENDER_GATE: LazyLock<Semaphore> = LazyLock::new(|| {
    Semaphore::new(usize::try_from(RiseLottieRasterPolicy::render_concurrency()).unwrap_or(1))
});
static DECODE_GATE: LazyLock<Semaphore> = LazyLock::new(|| {
    Semaphore::new(usize::try_from(RiseLottieRasterPolicy::decode_concurrency()).unwrap_or(2))
});

struct RiseLottiePreparationCancellationOnDrop(Arc<RiseLottiePreparationCancellation>);

impl Drop for RiseLottiePreparationCancellationOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// one parsed animation plus its compressed frame set
/// a looping sticker shows only frame_count distinct images so each is rasterized once
/// compressed and replayed
pub struct RiseLottieFrameSequence {
    pub key: RiseLottieSequenceKey,
    pub frame_count: i64,
    pub frame_rate: f64,

    dimension: i64,
    executor: BackgroundExecutor,
    lane: futures::lock::Mutex<()>,
    lock: Mutex<RiseLottieFrameSequenceState>,
    animation: Mutex<Option<RlottieRasterizer>>,
}

impl RiseLottieFrameSequence {
    const RESTING_FRAME_SAMPLES: i64 = 8;

    pub fn cached_visible_frame_index(&self) -> Option<i64> {
        self.lock.lock().visible_frame_index
    }

    fn remember_visible_frame(&self, index: i64) {
        self.lock.lock().visible_frame_index = Some(index);
    }

    fn cached_resting_frame_index(&self) -> Option<i64> {
        self.lock.lock().resting_frame_index
    }

    fn remember_resting_frame(&self, index: i64) {
        self.lock.lock().resting_frame_index = Some(index);
    }

    pub fn bytes_per_frame(&self) -> i64 {
        self.dimension * self.dimension * 4
    }

    pub fn cached_bytes(&self) -> i64 {
        self.lock.lock().compressed_bytes
    }

    fn decoded_key(&self, index: i64) -> String {
        format!("{}|{}|{}|{}", self.key.cache_key, self.key.dimension, self.key.mirrored, index)
    }

    fn new(
        key: RiseLottieSequenceKey,
        animation: RlottieRasterizer,
        dimension: i64,
        frame_count: i64,
        frame_rate: f64,
        executor: BackgroundExecutor,
    ) -> Self {
        let slots = usize::try_from(frame_count).unwrap_or(0);
        Self {
            key,
            frame_count,
            frame_rate,
            dimension,
            executor,
            lane: futures::lock::Mutex::new(()),
            lock: Mutex::new(RiseLottieFrameSequenceState {
                compressed_frames: vec![None; slots],
                inflight: HashMap::new(),
                compressed_bytes: 0,
                is_torn_down: false,
                visible_frame_index: None,
                resting_frame_index: None,
            }),
            animation: Mutex::new(Some(animation)),
        }
    }

    pub async fn make(key: RiseLottieSequenceKey, json_data: Arc<[u8]>, cx: &AsyncApp) -> Option<Arc<Self>> {
        let cancellation = Arc::new(RiseLottiePreparationCancellation::new());
        let _cancel_on_drop = RiseLottiePreparationCancellationOnDrop(Arc::clone(&cancellation));
        let executor = cx.background_executor().clone();
        let sequence_executor = executor.clone();
        executor
            .spawn(async move {
                let _permit = RENDER_GATE.acquire().await.ok();
                if cancellation.is_cancelled() {
                    return None;
                }
                let json = std::str::from_utf8(&json_data).ok()?;
                let animation = RlottieRasterizer::open(json.as_bytes(), &key.cache_key).ok()?;
                let total_frames = i64::try_from(animation.frame_count()).unwrap_or(i64::MAX);
                let source_frame_rate = animation.frame_rate();
                if cancellation.is_cancelled()
                    || total_frames <= 0
                    || total_frames > 2_400
                    || !source_frame_rate.is_finite()
                    || source_frame_rate <= 0.0
                    || source_frame_rate > 120.0
                {
                    return None;
                }
                let dimension = key.dimension;
                Some(Arc::new(Self::new(
                    key,
                    animation,
                    dimension,
                    total_frames,
                    source_frame_rate,
                    sequence_executor,
                )))
            })
            .await
    }

    /// non blocking read of an already decoded frame
    /// safe from the display tick, never touches the compressed store or the lock
    pub fn cached_frame(&self, at: i64) -> Option<Arc<RenderImage>> {
        if self.frame_count <= 0 {
            return None;
        }
        let wrapped = at.rem_euclid(self.frame_count);
        RiseLottieDecodedFrameCache::shared().image(&self.decoded_key(wrapped))
    }

    /// produces frame at, decoded cache hit or else decompress or else rasterize and compress
    /// completion runs on an arbitrary thread
    pub fn request_frame(
        self: &Arc<Self>,
        at: i64,
        is_needed: Arc<dyn Fn() -> bool + Send + Sync>,
        completion: Option<Box<dyn FnOnce(Option<Arc<RenderImage>>) + Send>>,
    ) {
        if self.frame_count <= 0 || !is_needed() {
            if let Some(completion) = completion {
                completion(None);
            }
            return;
        }
        let wrapped = at.rem_euclid(self.frame_count);

        if let Some(decoded) = RiseLottieDecodedFrameCache::shared().image(&self.decoded_key(wrapped)) {
            if let Some(completion) = completion {
                completion(Some(decoded));
            }
            return;
        }

        let compressed = {
            let mut state = self.lock.lock();
            if state.is_torn_down {
                drop(state);
                if let Some(completion) = completion {
                    completion(None);
                }
                return;
            }
            if let Some(waiters) = state.inflight.get_mut(&wrapped) {
                waiters.push(RiseLottieFrameSequenceFrameRequest { is_needed, completion });
                return;
            }
            let compressed = usize::try_from(wrapped)
                .ok()
                .and_then(|slot| state.compressed_frames.get(slot).cloned().flatten());
            state.inflight.insert(wrapped, vec![RiseLottieFrameSequenceFrameRequest { is_needed, completion }]);
            compressed
        };

        let this = Arc::clone(self);
        self.executor
            .spawn(async move {
                let _lane = this.lane.lock().await;
                let mut image: Option<Arc<RenderImage>> = None;
                let mut produced_bytes: Option<Vec<u8>> = None;
                let mut skipped_for_no_demand = false;

                if let Some(compressed) = compressed {
                    let permit = DECODE_GATE.acquire().await.ok();
                    if this.has_frame_demand(wrapped) {
                        image = this.decode_frame(&compressed);
                    } else {
                        skipped_for_no_demand = true;
                    }
                    drop(permit);
                } else {
                    let permit = RENDER_GATE.acquire().await.ok();
                    if this.has_frame_demand(wrapped) {
                        let (rendered, compressed) = this.rasterize_compressing(wrapped);
                        image = rendered;
                        produced_bytes = compressed;
                    } else {
                        skipped_for_no_demand = true;
                    }
                    drop(permit);
                }

                let mut grew_to: Option<i64> = None;
                let waiters = {
                    let mut state = this.lock.lock();
                    let waiters = state.inflight.remove(&wrapped).unwrap_or_default();
                    if let Some(produced_bytes) = produced_bytes {
                        let produced_count = i64::try_from(produced_bytes.len()).unwrap_or(i64::MAX);
                        let slot = usize::try_from(wrapped).ok();
                        let slot_is_empty =
                            slot.is_some_and(|slot| state.compressed_frames.get(slot).is_some_and(Option::is_none));
                        if !state.is_torn_down
                            && slot_is_empty
                            && state.compressed_bytes.saturating_add(produced_count)
                                <= RiseLottieRasterPolicy::maximum_bytes_per_sequence()
                            && let Some(slot) = slot
                        {
                            state.compressed_frames[slot] = Some(Arc::from(produced_bytes));
                            state.compressed_bytes += produced_count;
                            grew_to = Some(state.compressed_bytes);
                        }
                    }
                    waiters
                };

                if let Some(image) = &image {
                    RiseLottieDecodedFrameCache::shared().insert(
                        Arc::clone(image),
                        this.decoded_key(wrapped),
                        this.bytes_per_frame(),
                    );
                }
                if let Some(grew_to) = grew_to {
                    RiseLottieFrameCache::shared().did_grow(&this, grew_to);
                }
                for waiter in waiters {
                    if skipped_for_no_demand && (waiter.is_needed)() {
                        this.request_frame(wrapped, waiter.is_needed, waiter.completion);
                    } else if let Some(completion) = waiter.completion {
                        completion(if (waiter.is_needed)() { image.clone() } else { None });
                    }
                }
            })
            .detach();
    }

    fn has_frame_demand(&self, at: i64) -> bool {
        let needs: Vec<Arc<dyn Fn() -> bool + Send + Sync>> = {
            let state = self.lock.lock();
            state
                .inflight
                .get(&at)
                .map(|requests| requests.iter().map(|request| Arc::clone(&request.is_needed)).collect())
                .unwrap_or_default()
        };
        needs.iter().any(|is_needed| is_needed())
    }

    fn decode_frame(&self, compressed: &[u8]) -> Option<Arc<RenderImage>> {
        let buffer = RiseLottieFrameCodec::decompress(compressed, self.bytes_per_frame())?;
        Self::make_image(buffer, self.dimension)
    }

    /// index of the first sampled frame that paints something
    /// none when rlottie cant render the animation, eg text layers or expressions or embedded assets
    /// none tells callers to fall back to the other engine
    pub async fn first_visible_frame_index(self: &Arc<Self>) -> Option<i64> {
        let cancellation = Arc::new(RiseLottiePreparationCancellation::new());
        let _cancel_on_drop = RiseLottiePreparationCancellationOnDrop(Arc::clone(&cancellation));
        if let Some(index) = self.cached_visible_frame_index() {
            return Some(index);
        }
        let last = 0_i64.max(self.frame_count - 1);
        let mut seen = HashSet::new();
        for index in [0, last / 4, last / 2] {
            if !seen.insert(index) {
                continue;
            }
            let (sender, receiver) = oneshot::channel();
            let needed = Arc::clone(&cancellation);
            self.request_frame(
                index,
                Arc::new(move || !needed.is_cancelled()),
                Some(Box::new(move |image: Option<Arc<RenderImage>>| {
                    let _ = sender.send(image);
                })),
            );
            let Some(image) = receiver.await.ok().flatten() else {
                continue;
            };
            if Self::has_visible_pixels(&image) {
                self.remember_visible_frame(index);
                return Some(index);
            }
        }
        None
    }

    pub async fn resting_frame_index_after_one_shot(self: &Arc<Self>) -> Option<i64> {
        let cancellation = Arc::new(RiseLottiePreparationCancellation::new());
        let _cancel_on_drop = RiseLottiePreparationCancellationOnDrop(Arc::clone(&cancellation));
        if let Some(index) = self.cached_resting_frame_index() {
            return Some(index);
        }
        let last = 0_i64.max(self.frame_count - 1);
        let stride = RiseLottieRasterPolicy::presentation_stride(self.frame_rate, self.bytes_per_frame() / 4);
        let mut candidates = vec![last];
        for sample in 0..Self::RESTING_FRAME_SAMPLES {
            candidates.push((last * sample / Self::RESTING_FRAME_SAMPLES / stride) * stride);
        }
        let mut best: Option<(i64, i64)> = None;
        let mut seen = HashSet::new();
        for index in candidates {
            if !seen.insert(index) {
                continue;
            }
            let (sender, receiver) = oneshot::channel();
            let needed = Arc::clone(&cancellation);
            self.request_frame(
                index,
                Arc::new(move || !needed.is_cancelled()),
                Some(Box::new(move |image: Option<Arc<RenderImage>>| {
                    let _ = sender.send(image);
                })),
            );
            let Some(image) = receiver.await.ok().flatten() else {
                if index == last {
                    return None;
                }
                continue;
            };
            let solid = Self::solid_pixel_count(&image);
            if index == last {
                if solid != 0 {
                    self.remember_resting_frame(last);
                    return Some(last);
                }
            } else if solid > best.map_or(0, |(_, best_solid)| best_solid) {
                best = Some((index, solid));
            }
        }
        let index = best.map_or(last, |(index, _)| index);
        self.remember_resting_frame(index);
        Some(index)
    }

    fn solid_pixel_count(image: &RenderImage) -> i64 {
        let Some(bytes) = image.as_bytes(0) else {
            return 0;
        };
        bytes.iter().skip(3).step_by(64).filter(|alpha| **alpha >= 128).count() as i64
    }

    fn has_visible_pixels(image: &RenderImage) -> bool {
        let Some(bytes) = image.as_bytes(0) else {
            return false;
        };
        bytes.iter().skip(3).step_by(64).any(|alpha| *alpha > 1)
    }

    pub fn teardown(&self) {
        let waiters: Vec<RiseLottieFrameSequenceFrameRequest> = {
            let mut state = self.lock.lock();
            state.is_torn_down = true;
            let slots = state.compressed_frames.len();
            state.compressed_frames = vec![None; slots];
            state.compressed_bytes = 0;
            std::mem::take(&mut state.inflight).into_values().flatten().collect()
        };
        for waiter in waiters {
            if let Some(completion) = waiter.completion {
                completion(None);
            }
        }
    }

    fn rasterize_compressing(&self, frame: i64) -> (Option<Arc<RenderImage>>, Option<Vec<u8>>) {
        if self.lock.lock().is_torn_down {
            return (None, None);
        }
        let Ok(dimension) = u32::try_from(self.dimension) else {
            return (None, None);
        };
        let Ok(frame) = usize::try_from(frame) else {
            return (None, None);
        };
        let side = dimension as usize;
        let mut pixels = vec![0u32; side * side];
        {
            let mut animation = self.animation.lock();
            let Some(animation) = animation.as_mut() else {
                return (None, None);
            };
            animation.render(frame, dimension, &mut pixels);
        }
        if self.key.mirrored {
            for row in pixels.chunks_exact_mut(side) {
                row.reverse();
            }
        }
        let buffer: Vec<u8> = pixels.iter().flat_map(|pixel| pixel.to_ne_bytes()).collect();
        let compressed = RiseLottieFrameCodec::compress(&buffer);
        let image = Self::make_image(buffer, self.dimension);
        (image, compressed)
    }

    fn make_image(buffer: Vec<u8>, dimension: i64) -> Option<Arc<RenderImage>> {
        let side = u32::try_from(dimension).ok()?;
        let bitmap = RgbaImage::from_raw(side, side, buffer)?;
        let frame = Frame::from_parts(bitmap, 0, 0, Delay::from_numer_denom_ms(0, 1));
        let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([frame]);
        Some(rise_ui::image_ui::render_image_atlas::RenderImageAtlas::tracked(Arc::new(RenderImage::new(frames))))
    }
}

impl Drop for RiseLottieFrameSequence {
    fn drop(&mut self) {
        let Some(animation) = self.animation.get_mut().take() else {
            return;
        };
        self.executor.spawn(async move { drop(animation) }).detach();
    }
}

struct RiseLottieFrameCacheEntry {
    sequence: Arc<RiseLottieFrameSequence>,
    bytes: i64,
    last_used: f64,
}

impl RiseLottieFrameCacheEntry {
    fn new(sequence: Arc<RiseLottieFrameSequence>, last_used: f64) -> Self {
        Self {
            sequence,
            bytes: 0,
            last_used,
        }
    }
}

struct RiseLottieFrameCacheState {
    entries: HashMap<RiseLottieSequenceKey, RiseLottieFrameCacheEntry>,
    retain_counts: HashMap<RiseLottieSequenceKey, i64>,
    total_bytes: i64,
}

/// process wide cost bounded registry of rasterized sequences
/// sharing by key means the same emoji in recents and in its pack rasterizes once
pub struct RiseLottieFrameCache {
    lock: Mutex<RiseLottieFrameCacheState>,
    loading: CleverImageSharedLoadRegistry<RiseLottieSequenceKey, Option<Arc<RiseLottieFrameSequence>>>,
}

static FRAME_CACHE: LazyLock<RiseLottieFrameCache> = LazyLock::new(RiseLottieFrameCache::new);

fn current_media_time() -> f64 {
    RiseClock::monotonic().as_secs_f64()
}

impl RiseLottieFrameCache {
    pub fn shared() -> &'static Self {
        &FRAME_CACHE
    }

    fn new() -> Self {
        Self {
            lock: Mutex::new(RiseLottieFrameCacheState {
                entries: HashMap::new(),
                retain_counts: HashMap::new(),
                total_bytes: 0,
            }),
            loading: CleverImageSharedLoadRegistry::new(),
        }
    }

    /// players declare interest so eviction never tears down a sequence that is on screen
    pub fn retain(&self, key: &RiseLottieSequenceKey) {
        let mut state = self.lock.lock();
        *state.retain_counts.entry(key.clone()).or_insert(0) += 1;
        if let Some(entry) = state.entries.get_mut(key) {
            entry.last_used = current_media_time();
        }
    }

    pub fn release(&self, key: &RiseLottieSequenceKey) {
        let evicted = {
            let mut state = self.lock.lock();
            if let Some(count) = state.retain_counts.get(key).copied() {
                if count <= 1 {
                    state.retain_counts.remove(key);
                } else {
                    state.retain_counts.insert(key.clone(), count - 1);
                }
            }
            Self::trim_locked(&mut state, None)
        };
        if let Some(executor) = evicted.first().map(|sequence| sequence.executor.clone()) {
            executor
                .spawn(async move {
                    for sequence in evicted {
                        sequence.teardown();
                    }
                })
                .detach();
        }
    }

    /// sync hit for an already rasterized animation
    /// a resident sequence installs in the same turn
    /// and skips the whole async install chain which made reentry feel like a fresh load
    pub fn cached_sequence(&self, for_key: &RiseLottieSequenceKey) -> Option<Arc<RiseLottieFrameSequence>> {
        let mut state = self.lock.lock();
        let entry = state.entries.get_mut(for_key)?;
        entry.last_used = current_media_time();
        Some(Arc::clone(&entry.sequence))
    }

    pub async fn sequence(
        &self,
        for_key: &RiseLottieSequenceKey,
        json_data: Arc<[u8]>,
        cx: &AsyncApp,
    ) -> Option<Arc<RiseLottieFrameSequence>> {
        if let Some(cached) = self.cached_sequence(for_key) {
            return Some(cached);
        }
        let key = for_key.clone();
        self.loading
            .value(
                for_key.clone(),
                async move |cx: &mut AsyncApp| {
                    let cache = RiseLottieFrameCache::shared();
                    if let Some(cached) = cache.cached_sequence(&key) {
                        return Some(cached);
                    }
                    let made = RiseLottieFrameSequence::make(key.clone(), json_data, cx).await?;
                    let mut state = cache.lock.lock();
                    if let Some(existing) = state.entries.get_mut(&key) {
                        existing.last_used = current_media_time();
                        return Some(Arc::clone(&existing.sequence));
                    }
                    state
                        .entries
                        .insert(key, RiseLottieFrameCacheEntry::new(Arc::clone(&made), current_media_time()));
                    Some(made)
                },
                cx,
            )
            .await
    }

    fn did_grow(&self, sequence: &Arc<RiseLottieFrameSequence>, bytes: i64) {
        let evictable = {
            let mut state = self.lock.lock();
            let mut delta = 0;
            if let Some(entry) = state.entries.get_mut(&sequence.key)
                && Arc::ptr_eq(&entry.sequence, sequence)
            {
                delta = bytes - entry.bytes;
                entry.bytes = bytes;
                entry.last_used = current_media_time();
            }
            state.total_bytes += delta;
            Self::trim_locked(&mut state, Some(&sequence.key))
        };
        for sequence in evictable {
            sequence.teardown();
        }
    }

    fn trim_locked(
        state: &mut RiseLottieFrameCacheState,
        excluding: Option<&RiseLottieSequenceKey>,
    ) -> Vec<Arc<RiseLottieFrameSequence>> {
        let mut evicted = Vec::new();
        while state.total_bytes > RiseLottieRasterPolicy::maximum_cache_bytes() {
            let victim = state
                .entries
                .iter()
                .filter(|(key, _)| Some(*key) != excluding && !state.retain_counts.contains_key(*key))
                .min_by(|left, right| left.1.last_used.total_cmp(&right.1.last_used))
                .map(|(key, _)| key.clone());
            let Some(victim) = victim else {
                break;
            };
            if let Some(entry) = state.entries.remove(&victim) {
                state.total_bytes -= entry.bytes;
                evicted.push(entry.sequence);
            }
        }
        evicted
    }

    pub fn touch(&self, key: &RiseLottieSequenceKey) {
        let mut state = self.lock.lock();
        if let Some(entry) = state.entries.get_mut(key) {
            entry.last_used = current_media_time();
        }
    }

    pub fn release_decoded_frames(&self) {
        RiseLottieDecodedFrameCache::shared().remove_all();
    }

    pub fn release_all(&self) {
        RiseLottieDecodedFrameCache::shared().remove_all();
        let sequences: Vec<RiseLottieFrameCacheEntry> = {
            let mut state = self.lock.lock();
            let keys: Vec<RiseLottieSequenceKey> = state
                .entries
                .keys()
                .filter(|key| !state.retain_counts.contains_key(*key))
                .cloned()
                .collect();
            let mut removed = Vec::with_capacity(keys.len());
            for key in keys {
                if let Some(entry) = state.entries.remove(&key) {
                    state.total_bytes -= entry.bytes;
                    removed.push(entry);
                }
            }
            removed
        };
        for entry in sequences {
            entry.sequence.teardown();
        }
    }
}
