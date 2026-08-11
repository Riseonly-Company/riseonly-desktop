//! Which animations are resident, and what they are allowed to cost.
//!
//! Sequences are shared by key, so the same emoji in recents, in its pack and
//! inline in a message is one rasterised sequence. The cache is owned rather
//! than a singleton: dropping it drops everything it holds.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use super::raster_policy::{self, SequenceKey};
use super::sequence::{DecodedFrameCache, FrameSequence, LottieRasterizer, SequenceError};

#[derive(Debug)]
struct Entry {
    sequence: Arc<FrameSequence>,
    bytes: u64,
    last_used: u64,
}

#[derive(Debug, Default)]
struct CacheState {
    entries: HashMap<SequenceKey, Entry>,
    retained: HashMap<SequenceKey, usize>,
    bytes: u64,
    clock: u64,
}

impl CacheState {
    fn touch(&mut self, key: &SequenceKey) {
        self.clock += 1;
        let used = self.clock;
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_used = used;
        }
    }
}

#[derive(Debug)]
pub struct FrameCache {
    ceiling_bytes: u64,
    decoded: Arc<DecodedFrameCache>,
    state: Mutex<CacheState>,
}

impl FrameCache {
    pub fn new() -> Self {
        Self::with_ceilings(
            raster_policy::MAXIMUM_CACHE_BYTES,
            raster_policy::MAXIMUM_DECODED_BYTES,
        )
    }

    pub fn with_ceilings(compressed_ceiling: u64, decoded_ceiling: u64) -> Self {
        Self {
            ceiling_bytes: compressed_ceiling,
            decoded: Arc::new(DecodedFrameCache::new(decoded_ceiling)),
            state: Mutex::new(CacheState::default()),
        }
    }

    pub fn decoded_frames(&self) -> &Arc<DecodedFrameCache> {
        &self.decoded
    }

    pub fn compressed_bytes(&self) -> u64 {
        self.state.lock().bytes
    }

    pub fn resident_count(&self) -> usize {
        self.state.lock().entries.len()
    }

    /// A hit for an already-open animation. Synchronous, so a view can install
    /// a resident sequence in the same turn.
    pub fn cached(&self, key: &SequenceKey) -> Option<Arc<FrameSequence>> {
        let mut state = self.state.lock();
        state.touch(key);
        state.entries.get(key).map(|entry| entry.sequence.clone())
    }

    /// Register an animation, or return the one already registered. On a hit
    /// the passed rasteriser is dropped without ever being used.
    pub fn open(
        &self,
        key: SequenceKey,
        rasterizer: Box<dyn LottieRasterizer>,
    ) -> Result<Arc<FrameSequence>, SequenceError> {
        if let Some(resident) = self.cached(&key) {
            return Ok(resident);
        }

        let sequence = Arc::new(FrameSequence::open(
            key.clone(),
            rasterizer,
            self.decoded.clone(),
        )?);

        let mut state = self.state.lock();
        if let Some(entry) = state.entries.get(&key) {
            return Ok(entry.sequence.clone());
        }

        state.clock += 1;
        let used = state.clock;
        state.entries.insert(
            key,
            Entry {
                sequence: sequence.clone(),
                bytes: 0,
                last_used: used,
            },
        );

        Ok(sequence)
    }

    /// Players declare interest so eviction never tears down what is on screen.
    pub fn retain(&self, key: &SequenceKey) {
        let mut state = self.state.lock();
        *state.retained.entry(key.clone()).or_insert(0) += 1;
        state.touch(key);
    }

    pub fn release(&self, key: &SequenceKey) {
        let mut state = self.state.lock();
        if let Some(count) = state.retained.get_mut(key) {
            *count -= 1;
            if *count == 0 {
                state.retained.remove(key);
            }
        }
    }

    pub fn is_retained(&self, key: &SequenceKey) -> bool {
        self.state.lock().retained.contains_key(key)
    }

    /// Re-price every resident animation and evict until the ceiling holds.
    ///
    /// Call after a raster job — the only thing that makes a sequence grow.
    pub fn sweep(&self) {
        let mut evicted: Vec<Arc<FrameSequence>> = Vec::new();
        let mut state = self.state.lock();

        state.bytes = 0;
        let sizes: Vec<(SequenceKey, u64)> = state
            .entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.sequence.compressed_bytes()))
            .collect();
        for (key, bytes) in sizes {
            if let Some(entry) = state.entries.get_mut(&key) {
                entry.bytes = bytes;
            }
            state.bytes += bytes;
        }

        while state.bytes > self.ceiling_bytes
            || state.entries.len() > raster_policy::MAXIMUM_RESIDENT_SEQUENCES
        {
            let victim = state
                .entries
                .iter()
                .filter(|(key, _)| !state.retained.contains_key(*key))
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone());

            let Some(victim) = victim else {
                // Everything resident is on screen; refusing to evict beats freezing a visible sticker.
                break;
            };

            if let Some(entry) = state.entries.remove(&victim) {
                state.bytes -= entry.bytes.min(state.bytes);
                evicted.push(entry.sequence);
            }
        }

        drop(state);

        for sequence in evicted {
            sequence.teardown();
        }
    }

    /// First response to memory pressure: drop decoded frames only. They refill
    /// with an LZ4 decode, so nothing stops animating.
    pub fn release_decoded_frames(&self) {
        self.decoded.clear();
    }

    /// Second response, and what an account switch does: drop everything that
    /// is not on screen right now.
    pub fn release_unretained(&self) {
        let mut evicted: Vec<Arc<FrameSequence>> = Vec::new();
        let mut state = self.state.lock();

        let victims: Vec<SequenceKey> = state
            .entries
            .keys()
            .filter(|key| !state.retained.contains_key(*key))
            .cloned()
            .collect();

        for key in victims {
            if let Some(entry) = state.entries.remove(&key) {
                state.bytes -= entry.bytes.min(state.bytes);
                evicted.push(entry.sequence);
            }
        }

        drop(state);

        self.decoded.clear();
        for sequence in evicted {
            sequence.teardown();
        }
    }
}

impl Default for FrameCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::sequence::tests::CountingRasterizer;
    use super::*;
    use std::sync::atomic::Ordering;

    fn rasterizer(frames: usize) -> Box<dyn LottieRasterizer> {
        let (rasterizer, _) = CountingRasterizer::new(frames, 30.0);
        Box::new(rasterizer)
    }

    #[test]
    fn the_same_sticker_at_the_same_size_is_opened_once() {
        let cache = FrameCache::new();
        let key = SequenceKey::new("emoji", 64);

        let (first_rasterizer, renders) = CountingRasterizer::new(4, 30.0);
        let first = cache.open(key.clone(), Box::new(first_rasterizer)).unwrap();
        let second = cache.open(key.clone(), rasterizer(4)).unwrap();

        assert!(Arc::ptr_eq(&first, &second));

        second.frame(0);
        assert_eq!(
            renders.load(Ordering::Relaxed),
            1,
            "the second caller must be using the first animation, not its own"
        );
        assert_eq!(cache.resident_count(), 1);
    }

    #[test]
    fn the_same_sticker_at_two_sizes_is_two_sequences() {
        let cache = FrameCache::new();

        cache
            .open(SequenceKey::new("emoji", 64), rasterizer(4))
            .unwrap();
        cache
            .open(SequenceKey::new("emoji", 128), rasterizer(4))
            .unwrap();

        assert_eq!(cache.resident_count(), 2);
    }

    #[test]
    fn eviction_never_takes_a_sticker_that_is_on_screen() {
        let ceiling = 1;
        let cache = FrameCache::with_ceilings(ceiling, raster_policy::MAXIMUM_DECODED_BYTES);

        let visible = SequenceKey::new("visible", 64);
        let sequence = cache.open(visible.clone(), rasterizer(4)).unwrap();
        cache.retain(&visible);
        sequence.frame(0);

        for index in 0..10 {
            let key = SequenceKey::new(format!("offscreen-{index}"), 64);
            let other = cache.open(key, rasterizer(4)).unwrap();
            other.frame(0);
            cache.sweep();
        }

        assert!(cache.cached(&visible).is_some());
        assert!(
            sequence.frame(0).is_some(),
            "a retained sequence was torn down"
        );
    }

    /// What one two-frame sequence costs, so a ceiling can be set to exactly one.
    fn one_sequence_bytes() -> u64 {
        let cache = FrameCache::new();
        let sequence = cache
            .open(SequenceKey::new("probe", 64), rasterizer(2))
            .unwrap();
        sequence.frame(0);
        sequence.frame(1);
        cache.sweep();
        cache.compressed_bytes()
    }

    #[test]
    fn the_least_recently_used_offscreen_sticker_is_the_one_evicted() {
        let cache =
            FrameCache::with_ceilings(one_sequence_bytes(), raster_policy::MAXIMUM_DECODED_BYTES);

        let old = SequenceKey::new("old", 64);
        let recent = SequenceKey::new("recent", 64);

        for key in [&old, &recent] {
            let sequence = cache.open(key.clone(), rasterizer(2)).unwrap();
            sequence.frame(0);
            sequence.frame(1);
        }
        cache.cached(&recent);

        cache.sweep();

        assert!(cache.cached(&old).is_none());
        assert!(cache.cached(&recent).is_some());
    }

    #[test]
    fn the_compressed_ceiling_holds_across_a_long_scroll() {
        let ceiling = raster_policy::frame_bytes(64) * 2;
        let cache = FrameCache::with_ceilings(ceiling, raster_policy::MAXIMUM_DECODED_BYTES);

        for index in 0..200 {
            let key = SequenceKey::new(format!("sticker-{index}"), 64);
            let sequence = cache.open(key, rasterizer(8)).unwrap();
            for frame in 0..8 {
                sequence.frame(frame);
            }
            cache.sweep();

            assert!(
                cache.compressed_bytes() <= ceiling,
                "after {index} stickers the cache held {} bytes",
                cache.compressed_bytes()
            );
        }
    }

    #[test]
    fn cheap_animations_cannot_accumulate_under_the_byte_ceiling() {
        let cache = FrameCache::new();

        // Well under the byte ceiling, which is where counting only bytes lets handles pile up.
        for index in 0..raster_policy::MAXIMUM_RESIDENT_SEQUENCES * 3 {
            let key = SequenceKey::new(format!("tiny-{index}"), 32);
            cache.open(key, rasterizer(2)).unwrap().frame(0);
            cache.sweep();
        }

        assert!(cache.compressed_bytes() < raster_policy::MAXIMUM_CACHE_BYTES);
        assert!(
            cache.resident_count() <= raster_policy::MAXIMUM_RESIDENT_SEQUENCES,
            "{} animations stayed open",
            cache.resident_count()
        );
    }

    #[test]
    fn the_resident_cap_still_never_evicts_what_is_on_screen() {
        let cache = FrameCache::new();
        let visible = SequenceKey::new("visible", 32);
        cache.open(visible.clone(), rasterizer(2)).unwrap();
        cache.retain(&visible);

        for index in 0..raster_policy::MAXIMUM_RESIDENT_SEQUENCES * 2 {
            cache
                .open(SequenceKey::new(format!("tiny-{index}"), 32), rasterizer(2))
                .unwrap();
            cache.sweep();
        }

        assert!(cache.cached(&visible).is_some());
    }

    #[test]
    fn releasing_decoded_frames_keeps_every_animation_playable() {
        let cache = FrameCache::new();
        let key = SequenceKey::new("emoji", 64);
        let (rasterizer, renders) = CountingRasterizer::new(4, 30.0);
        let sequence = cache.open(key, Box::new(rasterizer)).unwrap();

        for frame in 0..4 {
            sequence.frame(frame);
        }
        cache.release_decoded_frames();
        for frame in 0..4 {
            assert!(sequence.frame(frame).is_some());
        }

        assert_eq!(
            renders.load(Ordering::Relaxed),
            4,
            "dropping decoded frames must cost a decode, never a re-rasterisation"
        );
    }

    #[test]
    fn releasing_unretained_leaves_only_what_is_on_screen() {
        let cache = FrameCache::new();
        let visible = SequenceKey::new("visible", 64);

        cache.open(visible.clone(), rasterizer(2)).unwrap();
        cache.retain(&visible);
        for index in 0..5 {
            cache
                .open(SequenceKey::new(format!("gone-{index}"), 64), rasterizer(2))
                .unwrap();
        }

        cache.release_unretained();

        assert_eq!(cache.resident_count(), 1);
        assert!(cache.cached(&visible).is_some());
    }

    #[test]
    fn a_released_sticker_becomes_evictable_again() {
        let cache = FrameCache::with_ceilings(1, raster_policy::MAXIMUM_DECODED_BYTES);
        let key = SequenceKey::new("sticker", 64);

        cache.open(key.clone(), rasterizer(2)).unwrap().frame(0);
        cache.retain(&key);
        cache.sweep();
        assert!(cache.cached(&key).is_some());

        cache.release(&key);
        cache.sweep();

        assert!(cache.cached(&key).is_none());
    }
}
