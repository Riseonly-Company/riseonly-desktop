//! The threads that turn vectors into pixels, and what stops them multiplying.
//!
//! Two properties matter and neither is obvious from the outside.
//!
//! DEDUPLICATION. The same sticker is asked for by the display tick, by the
//! prefetch that follows it, and by every other cell showing the same emoji.
//! Without a set of in-flight requests, a picker grid submits the same
//! rasterisation dozens of times and rlottie does all of them.
//!
//! BOUNDED QUEUE. A fast scroll asks for frames faster than they can be
//! rasterised, and every one of those requests is for a sticker that is already
//! off screen by the time a worker reaches it. An unbounded queue turns a
//! two-second flick into a minute of CPU nobody is waiting for. The queue is
//! capped and the OLDEST request is dropped, because the oldest is the one the
//! user has scrolled furthest away from.
//!
//! The pool is the desktop equivalent of the reference's raster pool, its
//! per-sequence lane and its two semaphores.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;

use parking_lot::{Condvar, Mutex};

use super::frame_cache::FrameCache;
use super::raster_policy::{self, SequenceKey};
use super::sequence::FrameSequence;

/// Requests allowed to be waiting at once.
///
/// Sized for a full screen of stickers each with its prefetch window, plus room
/// for one scroll tick's worth of new arrivals. Past that, the queue is
/// describing the past rather than the viewport.
pub const MAXIMUM_PENDING: usize = 256;

struct Job {
    sequence: Arc<FrameSequence>,
    frame: usize,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    inflight: HashSet<(SequenceKey, usize)>,
    running: usize,
    shutdown: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    work: Condvar,
    idle: Condvar,
    cache: Arc<FrameCache>,
    dropped: AtomicU64,
    rasterised: AtomicU64,
}

/// A fixed set of workers producing frames for whoever asked.
pub struct RasterPool {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for RasterPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RasterPool")
            .field("workers", &self.workers.len())
            .field("dropped", &self.dropped())
            .finish_non_exhaustive()
    }
}

impl RasterPool {
    pub fn new(cache: Arc<FrameCache>) -> Self {
        let parallelism = std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(2);

        Self::with_workers(cache, raster_policy::render_concurrency(parallelism))
    }

    pub fn with_workers(cache: Arc<FrameCache>, workers: usize) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            work: Condvar::new(),
            idle: Condvar::new(),
            cache,
            dropped: AtomicU64::new(0),
            rasterised: AtomicU64::new(0),
        });

        let workers = (0..workers.max(1))
            .map(|index| {
                let shared = shared.clone();
                std::thread::Builder::new()
                    .name(format!("rise-lottie-raster-{index}"))
                    .spawn(move || worker(shared))
                    .expect("the raster pool can start a thread")
            })
            .collect();

        Self { shared, workers }
    }

    /// Requests dropped because the queue was full. A non-zero value on a
    /// normal scroll means the cap is too low or the stickers are too large.
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    pub fn rasterised(&self) -> u64 {
        self.shared.rasterised.load(Ordering::Relaxed)
    }

    pub fn pending(&self) -> usize {
        self.shared.queue.lock().jobs.len()
    }

    /// Ask for a frame. Cheap, idempotent, and safe to call from the tick.
    pub fn request(&self, sequence: Arc<FrameSequence>, frame: usize) {
        let frame = frame % sequence.frame_count().max(1);

        if sequence.cached_frame(frame).is_some() {
            return;
        }

        let key = (sequence.key().clone(), frame);
        let mut queue = self.shared.queue.lock();

        if queue.shutdown || !queue.inflight.insert(key) {
            return;
        }

        if queue.jobs.len() >= MAXIMUM_PENDING
            && let Some(evicted) = queue.jobs.pop_front()
        {
            queue
                .inflight
                .remove(&(evicted.sequence.key().clone(), evicted.frame));
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }

        queue.jobs.push_back(Job { sequence, frame });
        drop(queue);
        self.shared.work.notify_one();
    }

    /// Block until nothing is queued or running.
    ///
    /// For tests and for teardown. Never call it from the GPUI thread — the
    /// whole point of the pool is that the tick does not wait for it.
    pub fn wait_idle(&self) {
        let mut queue = self.shared.queue.lock();
        while !queue.jobs.is_empty() || queue.running > 0 {
            self.shared.idle.wait(&mut queue);
        }
    }
}

impl Drop for RasterPool {
    fn drop(&mut self) {
        {
            let mut queue = self.shared.queue.lock();
            queue.shutdown = true;
            queue.jobs.clear();
            queue.inflight.clear();
        }
        self.shared.work.notify_all();

        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn worker(shared: Arc<Shared>) {
    loop {
        let job = {
            let mut queue = shared.queue.lock();

            loop {
                if queue.shutdown {
                    return;
                }
                if let Some(job) = queue.jobs.pop_front() {
                    queue.running += 1;
                    break job;
                }
                shared.work.wait(&mut queue);
            }
        };

        let produced = job.sequence.frame(job.frame).is_some();
        if produced {
            shared.rasterised.fetch_add(1, Ordering::Relaxed);
            // A sequence only grows here, so this is the only place the ledger
            // can fall behind what the sequences actually hold.
            shared.cache.sweep();
        }

        let mut queue = shared.queue.lock();
        queue
            .inflight
            .remove(&(job.sequence.key().clone(), job.frame));
        queue.running -= 1;
        if queue.jobs.is_empty() && queue.running == 0 {
            shared.idle.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::sequence::tests::CountingRasterizer;
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn pool_with_sequence(
        frames: usize,
        workers: usize,
    ) -> (RasterPool, Arc<FrameSequence>, Arc<AtomicUsize>) {
        let cache = Arc::new(FrameCache::new());
        let (rasterizer, renders) = CountingRasterizer::new(frames, 30.0);
        let sequence = cache
            .open(SequenceKey::new("sticker", 32), Box::new(rasterizer))
            .unwrap();

        (RasterPool::with_workers(cache, workers), sequence, renders)
    }

    #[test]
    fn a_frame_asked_for_by_fifty_cells_is_rasterised_once() {
        let (pool, sequence, renders) = pool_with_sequence(10, 3);

        for _ in 0..50 {
            pool.request(sequence.clone(), 4);
        }
        pool.wait_idle();

        assert_eq!(renders.load(Ordering::Relaxed), 1);
        assert!(sequence.cached_frame(4).is_some());
    }

    #[test]
    fn an_already_cached_frame_never_reaches_a_worker() {
        let (pool, sequence, renders) = pool_with_sequence(10, 1);
        sequence.frame(2);

        pool.request(sequence.clone(), 2);
        pool.wait_idle();

        assert_eq!(renders.load(Ordering::Relaxed), 1);
        assert_eq!(pool.rasterised(), 0);
    }

    #[test]
    fn a_whole_animation_is_produced_across_the_workers() {
        let (pool, sequence, renders) = pool_with_sequence(24, 3);

        for frame in 0..24 {
            pool.request(sequence.clone(), frame);
        }
        pool.wait_idle();

        assert_eq!(renders.load(Ordering::Relaxed), 24);
        for frame in 0..24 {
            assert!(sequence.cached_frame(frame).is_some(), "frame {frame}");
        }
    }

    #[test]
    fn a_flick_that_outruns_the_workers_drops_the_oldest_request() {
        let cache = Arc::new(FrameCache::new());
        let pool = RasterPool::with_workers(cache.clone(), 1);

        let mut sequences = Vec::new();
        for index in 0..MAXIMUM_PENDING + 64 {
            let (rasterizer, _) = CountingRasterizer::new(4, 30.0);
            let sequence = cache
                .open(
                    SequenceKey::new(format!("sticker-{index}"), 32),
                    Box::new(rasterizer),
                )
                .unwrap();
            pool.request(sequence.clone(), 0);
            sequences.push(sequence);
        }

        assert!(
            pool.pending() <= MAXIMUM_PENDING,
            "the queue grew to {}",
            pool.pending()
        );
        pool.wait_idle();
    }

    #[test]
    fn an_index_past_the_end_wraps_rather_than_being_refused() {
        let (pool, sequence, _) = pool_with_sequence(5, 2);

        pool.request(sequence.clone(), 12);
        pool.wait_idle();

        assert!(sequence.cached_frame(2).is_some());
    }

    #[test]
    fn dropping_the_pool_joins_its_workers_with_a_full_queue() {
        let cache = Arc::new(FrameCache::new());
        let (rasterizer, _) = CountingRasterizer::new(2000, 60.0);
        let sequence = cache
            .open(SequenceKey::new("huge", 256), Box::new(rasterizer))
            .unwrap();

        let pool = RasterPool::with_workers(cache, 2);
        for frame in 0..2000 {
            pool.request(sequence.clone(), frame);
        }

        drop(pool);
    }
}
