use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// samples the main thread call stack while its busy
/// not supported on desktop, nothing can capture it from inside the process
/// callers take the no sampler branch
pub struct RiseMainThreadSampler {
    turn_started_at: AtomicU64,
    is_stopping: AtomicBool,
}

impl RiseMainThreadSampler {
    pub const IS_SUPPORTED: bool = false;

    pub fn is_supported() -> bool {
        Self::IS_SUPPORTED
    }

    /// on_sample gets symbol names and the turn age in ms
    /// never called on the desktop so its not kept
    pub fn new(on_sample: impl Fn(Vec<String>, f64) + Send + Sync + 'static) -> Self {
        drop(on_sample);
        Self {
            turn_started_at: AtomicU64::new(0),
            is_stopping: AtomicBool::new(true),
        }
    }

    pub fn note_turn_began(&self, at: f64) {
        self.turn_started_at.store(at.to_bits(), Ordering::Relaxed);
    }

    pub fn note_turn_ended(&self) {
        self.turn_started_at.store(0, Ordering::Relaxed);
    }

    pub fn start(&self) {
        if !Self::is_supported() {
            return;
        }
        self.is_stopping.store(false, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.is_stopping.store(true, Ordering::Relaxed);
    }
}
