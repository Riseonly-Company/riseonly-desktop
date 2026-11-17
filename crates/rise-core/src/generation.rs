use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Generation(u64);

impl Generation {
    pub const ZERO: Self = Self(0);

    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Default)]
pub struct GenerationCounter {
    current: Arc<AtomicU64>,
}

impl GenerationCounter {
    pub fn new() -> Self {
        Self {
            current: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn current(&self) -> Generation {
        Generation(self.current.load(Ordering::Acquire))
    }

    pub fn bump(&self) -> Generation {
        Generation(self.current.fetch_add(1, Ordering::AcqRel) + 1)
    }

    pub fn is_current(&self, generation: Generation) -> bool {
        self.current() == generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_captured_generation_stops_being_current_after_a_bump() {
        let counter = GenerationCounter::new();
        let captured = counter.current();
        assert!(counter.is_current(captured));

        counter.bump();
        assert!(!counter.is_current(captured));
    }

    #[test]
    fn clones_share_one_counter() {
        let counter = GenerationCounter::new();
        let clone = counter.clone();
        let captured = counter.current();

        clone.bump();

        assert!(!counter.is_current(captured));
        assert_eq!(counter.current(), clone.current());
    }

    #[test]
    fn generations_increase_monotonically() {
        let counter = GenerationCounter::new();
        let mut last = counter.current();
        for _ in 0..1_000 {
            let next = counter.bump();
            assert!(next > last);
            last = next;
        }
    }
}
