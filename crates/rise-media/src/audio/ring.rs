//! The handoff between the decode thread and the audio callback.
//!
//! Single-producer, single-consumer and lock-free: the callback runs on an OS
//! real-time thread and may not allocate, lock or block.

use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

struct Shared {
    /// Only the producer writes `[write, write + n)`; only the consumer reads `[read, read + n)`.
    buffer: UnsafeCell<Box<[f32]>>,
    mask: usize,
    write: AtomicUsize,
    read: AtomicUsize,
    underruns: AtomicU64,
    overruns: AtomicU64,
}

// SAFETY: the un-cloneable producer and consumer halves touch disjoint index ranges, published through the release/acquire pairs below.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

/// Fills the ring. Lives on the decode thread.
pub struct Producer {
    shared: Arc<Shared>,
}

/// Drains the ring. Lives on the audio callback thread.
pub struct Consumer {
    shared: Arc<Shared>,
}

/// Create a ring holding `capacity_samples`, rounded up to a power of two.
pub fn ring(capacity_samples: usize) -> (Producer, Consumer) {
    let capacity = capacity_samples.max(2).next_power_of_two();

    let shared = Arc::new(Shared {
        buffer: UnsafeCell::new(vec![0.0; capacity].into_boxed_slice()),
        mask: capacity - 1,
        write: AtomicUsize::new(0),
        read: AtomicUsize::new(0),
        underruns: AtomicU64::new(0),
        overruns: AtomicU64::new(0),
    });

    (
        Producer {
            shared: shared.clone(),
        },
        Consumer { shared },
    )
}

impl Shared {
    fn capacity(&self) -> usize {
        self.mask + 1
    }

    fn filled(&self) -> usize {
        self.write
            .load(Ordering::Acquire)
            .wrapping_sub(self.read.load(Ordering::Acquire))
    }
}

impl Producer {
    pub fn capacity(&self) -> usize {
        self.shared.capacity()
    }

    pub fn filled(&self) -> usize {
        self.shared.filled()
    }

    pub fn free(&self) -> usize {
        self.capacity() - self.filled()
    }

    /// Samples the producer tried to write into a full ring.
    pub fn overruns(&self) -> u64 {
        self.shared.overruns.load(Ordering::Relaxed)
    }

    /// Write as much of `samples` as fits. Returns how much was written.
    pub fn push(&mut self, samples: &[f32]) -> usize {
        let write = self.shared.write.load(Ordering::Relaxed);
        let read = self.shared.read.load(Ordering::Acquire);

        let free = self.shared.capacity() - write.wrapping_sub(read);
        let count = samples.len().min(free);

        if count < samples.len() {
            self.shared
                .overruns
                .fetch_add((samples.len() - count) as u64, Ordering::Relaxed);
        }

        // SAFETY: the `count` slots at `write` are outside the consumer's `[read, write)` range until the store below publishes them.
        let buffer = unsafe { &mut *self.shared.buffer.get() };
        for (offset, sample) in samples[..count].iter().enumerate() {
            buffer[write.wrapping_add(offset) & self.shared.mask] = *sample;
        }

        self.shared
            .write
            .store(write.wrapping_add(count), Ordering::Release);
        count
    }
}

impl Consumer {
    pub fn capacity(&self) -> usize {
        self.shared.capacity()
    }

    pub fn filled(&self) -> usize {
        self.shared.filled()
    }

    /// Samples the device asked for and did not get — silence the user heard.
    pub fn underruns(&self) -> u64 {
        self.shared.underruns.load(Ordering::Relaxed)
    }

    /// Fill `out` completely, padding with silence if the ring runs dry.
    pub fn pop_or_silence(&mut self, out: &mut [f32]) -> usize {
        let read = self.shared.read.load(Ordering::Relaxed);
        let write = self.shared.write.load(Ordering::Acquire);

        let available = write.wrapping_sub(read);
        let count = out.len().min(available);

        // SAFETY: the `count` slots at `read` were published by the producer's release store and are not rewritten until the store below frees them.
        let buffer = unsafe { &*self.shared.buffer.get() };
        for (offset, slot) in out[..count].iter_mut().enumerate() {
            *slot = buffer[read.wrapping_add(offset) & self.shared.mask];
        }

        out[count..].fill(0.0);

        if count < out.len() {
            self.shared
                .underruns
                .fetch_add((out.len() - count) as u64, Ordering::Relaxed);
        }

        self.shared
            .read
            .store(read.wrapping_add(count), Ordering::Release);
        count
    }

    /// Throw away everything buffered — what a seek does, since the ring holds
    /// audio from before the jump.
    pub fn clear(&mut self) {
        let write = self.shared.write.load(Ordering::Acquire);
        self.shared.read.store(write, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_is_rounded_up_to_a_power_of_two() {
        let (producer, _consumer) = ring(1000);
        assert_eq!(producer.capacity(), 1024);
    }

    #[test]
    fn what_goes_in_comes_out_in_order() {
        let (mut producer, mut consumer) = ring(16);
        let input: Vec<f32> = (0..8).map(|value| value as f32).collect();

        assert_eq!(producer.push(&input), 8);
        let mut output = vec![0.0; 8];
        assert_eq!(consumer.pop_or_silence(&mut output), 8);

        assert_eq!(output, input);
    }

    #[test]
    fn the_ring_wraps_without_reordering_or_losing_a_sample() {
        let (mut producer, mut consumer) = ring(8);
        let mut expected = 0.0f32;
        let mut received = Vec::new();

        for _ in 0..1000 {
            let block: Vec<f32> = (0..5).map(|offset| expected + offset as f32).collect();
            let written = producer.push(&block);
            expected += written as f32;

            let mut out = vec![0.0; written];
            consumer.pop_or_silence(&mut out);
            received.extend(out);
        }

        let expected_sequence: Vec<f32> = (0..received.len()).map(|v| v as f32).collect();
        assert_eq!(received, expected_sequence);
    }

    #[test]
    fn a_full_ring_refuses_the_overflow_rather_than_overwriting_unplayed_audio() {
        let (mut producer, _consumer) = ring(8);

        assert_eq!(producer.push(&[1.0; 8]), 8);
        assert_eq!(producer.push(&[2.0; 4]), 0);
        assert_eq!(producer.overruns(), 4);
    }

    #[test]
    fn an_empty_ring_hands_the_device_silence_and_counts_it() {
        let (_producer, mut consumer) = ring(8);
        let mut out = vec![7.0; 6];

        assert_eq!(consumer.pop_or_silence(&mut out), 0);

        assert_eq!(
            out,
            vec![0.0; 6],
            "stale samples are a much louder artefact"
        );
        assert_eq!(consumer.underruns(), 6);
    }

    #[test]
    fn a_partial_read_pads_the_tail_instead_of_leaving_it_stale() {
        let (mut producer, mut consumer) = ring(8);
        producer.push(&[1.0, 2.0, 3.0]);

        let mut out = vec![9.0; 6];
        assert_eq!(consumer.pop_or_silence(&mut out), 3);

        assert_eq!(out, vec![1.0, 2.0, 3.0, 0.0, 0.0, 0.0]);
        assert_eq!(consumer.underruns(), 3);
    }

    #[test]
    fn clearing_drops_audio_from_before_a_seek() {
        let (mut producer, mut consumer) = ring(16);
        producer.push(&[1.0; 12]);

        consumer.clear();

        assert_eq!(consumer.filled(), 0);
        let mut out = vec![0.0; 4];
        assert_eq!(consumer.pop_or_silence(&mut out), 0);
    }

    #[test]
    fn a_decode_thread_and_a_callback_thread_agree_on_every_sample() {
        let (mut producer, mut consumer) = ring(1024);
        const TOTAL: usize = 200_000;
        const BLOCK: usize = 64;

        let writer = std::thread::spawn(move || {
            let mut written = 0usize;
            while written < TOTAL {
                let block: Vec<f32> = (written..(written + BLOCK).min(TOTAL))
                    .map(|value| value as f32)
                    .collect();
                let count = producer.push(&block);
                written += count;
                if count == 0 {
                    std::thread::yield_now();
                }
            }
        });

        let mut received = Vec::with_capacity(TOTAL);
        while received.len() < TOTAL {
            // Wait for a full block so the test is about ordering, not timing.
            if consumer.filled() < BLOCK {
                std::thread::yield_now();
                continue;
            }
            let mut out = vec![0.0; BLOCK];
            let count = consumer.pop_or_silence(&mut out);
            received.extend_from_slice(&out[..count]);
        }

        writer.join().unwrap();

        assert_eq!(consumer.underruns(), 0);
        let expected: Vec<f32> = (0..TOTAL).map(|value| value as f32).collect();
        assert_eq!(received, expected);
    }
}
