use std::fmt;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const SEQUENCE_BITS: u32 = 20;
const SEQUENCE_MASK: u64 = (1 << SEQUENCE_BITS) - 1;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(u64);

impl RequestId {
    pub fn new(raw: u64) -> Option<Self> {
        (raw != 0).then_some(Self(raw))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RequestId({})", self.0)
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub struct RequestIdAllocator {
    state: Mutex<AllocatorState>,
}

struct AllocatorState {
    logical_millis: u64,
    sequence: u64,
}

impl RequestIdAllocator {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(AllocatorState {
                logical_millis: 0,
                sequence: 0,
            }),
        }
    }

    pub fn next(&self) -> RequestId {
        self.next_at(wall_clock_millis())
    }

    // The logical clock never regresses: a backwards wall clock advances the
    // logical millisecond instead, so ids stay unique and monotonic across a
    // system time change or a leap second.
    pub fn next_at(&self, wall_millis: u64) -> RequestId {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        if wall_millis > state.logical_millis {
            state.logical_millis = wall_millis;
            state.sequence = 0;
        } else if state.sequence >= SEQUENCE_MASK {
            state.logical_millis += 1;
            state.sequence = 0;
        } else {
            state.sequence += 1;
        }

        let raw = (state.logical_millis << SEQUENCE_BITS) | state.sequence;
        RequestId(raw.max(1))
    }
}

impl Default for RequestIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

fn wall_clock_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(String);

impl AccountId {
    pub fn new(raw: impl Into<String>) -> Option<Self> {
        let raw = raw.into();
        (!raw.is_empty()).then_some(Self(raw))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn request_id_is_never_zero() {
        let allocator = RequestIdAllocator::new();
        assert!(allocator.next_at(0).get() >= 1);
        assert!(RequestId::new(0).is_none());
        assert!(RequestId::new(1).is_some());
    }

    #[test]
    fn sequence_increments_within_the_same_millisecond() {
        let allocator = RequestIdAllocator::new();
        let a = allocator.next_at(1_000);
        let b = allocator.next_at(1_000);
        let c = allocator.next_at(1_000);
        assert!(a < b && b < c);
        assert_eq!(a.get() >> SEQUENCE_BITS, c.get() >> SEQUENCE_BITS);
    }

    #[test]
    fn a_backwards_wall_clock_still_produces_increasing_ids() {
        let allocator = RequestIdAllocator::new();
        let first = allocator.next_at(10_000);
        let second = allocator.next_at(5_000);
        let third = allocator.next_at(1);
        assert!(first < second, "id regressed on clock rewind");
        assert!(second < third, "id regressed on further rewind");
    }

    #[test]
    fn sequence_overflow_rolls_into_the_next_logical_millisecond() {
        let allocator = RequestIdAllocator::new();
        let mut last = allocator.next_at(42);
        for _ in 0..(SEQUENCE_MASK + 8) {
            let next = allocator.next_at(42);
            assert!(next > last, "id regressed across sequence overflow");
            last = next;
        }
        assert!(last.get() >> SEQUENCE_BITS > 42);
    }

    #[test]
    fn ids_are_unique_across_a_long_run() {
        let allocator = RequestIdAllocator::new();
        let seen: HashSet<u64> = (0..50_000).map(|_| allocator.next_at(7).get()).collect();
        assert_eq!(seen.len(), 50_000);
    }

    #[test]
    fn account_id_rejects_empty() {
        assert!(AccountId::new("").is_none());
        assert_eq!(AccountId::new("u1").unwrap().as_str(), "u1");
    }
}
