use std::collections::VecDeque;

/// what a cache may hold, two ceilings since bytes alone and count alone each fail
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImageLimits {
    pub max_entries: usize,
    pub max_bytes: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Slot {
    key: u64,
    cost: u64,
    is_loading: bool,
}

/// the eviction policy, no toolkit in it
#[derive(Debug)]
pub struct ImageBudget {
    limits: ImageLimits,
    order: VecDeque<Slot>,
    bytes: u64,
}

impl ImageBudget {
    pub fn new(limits: ImageLimits) -> Self {
        Self {
            limits,
            order: VecDeque::new(),
            bytes: 0,
        }
    }

    pub fn limits(&self) -> ImageLimits {
        self.limits
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn contains(&self, key: u64) -> bool {
        self.order.iter().any(|slot| slot.key == key)
    }

    /// moves an entry to the most recently used end, false means a miss
    pub fn touch(&mut self, key: u64) -> bool {
        let Some(index) = self.order.iter().position(|slot| slot.key == key) else {
            return false;
        };

        if let Some(slot) = self.order.remove(index) {
            self.order.push_back(slot);
        }
        true
    }

    /// records that a load started, returns the keys the caller must drop
    pub fn insert_loading(&mut self, key: u64) -> Vec<u64> {
        if self.touch(key) {
            return Vec::new();
        }

        self.order.push_back(Slot {
            key,
            cost: 0,
            is_loading: true,
        });
        self.enforce(key)
    }

    /// records a finished decode and its cost, returns the keys to drop
    /// a failed load resolves with cost zero and stays so a broken url isnt refetched
    pub fn resolve(&mut self, key: u64, cost: u64) -> Vec<u64> {
        let Some(index) = self.order.iter().position(|slot| slot.key == key) else {
            return Vec::new();
        };

        let previous = self.order[index].cost;
        self.bytes = self.bytes.saturating_sub(previous).saturating_add(cost);
        self.order[index].cost = cost;
        self.order[index].is_loading = false;

        if let Some(slot) = self.order.remove(index) {
            self.order.push_back(slot);
        }

        self.enforce(key)
    }

    pub fn remove(&mut self, key: u64) -> bool {
        let Some(index) = self.order.iter().position(|slot| slot.key == key) else {
            return false;
        };

        let slot = self.order.remove(index).expect("the index was just found");
        self.bytes = self.bytes.saturating_sub(slot.cost);
        true
    }

    pub fn drain(&mut self) -> Vec<u64> {
        self.bytes = 0;
        self.order.drain(..).map(|slot| slot.key).collect()
    }

    fn enforce(&mut self, protect: u64) -> Vec<u64> {
        let mut evicted = Vec::new();

        while self.len() > self.limits.max_entries || self.bytes > self.limits.max_bytes {
            let Some(index) = self
                .order
                .iter()
                .position(|slot| !slot.is_loading && slot.key != protect)
            else {
                break;
            };

            let slot = self.order.remove(index).expect("the index was just found");
            self.bytes = self.bytes.saturating_sub(slot.cost);
            evicted.push(slot.key);
        }

        evicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: ImageLimits = ImageLimits {
        max_entries: 3,
        max_bytes: 1_000,
    };

    const TEST_FEED: ImageLimits = ImageLimits {
        max_entries: 192,
        max_bytes: 96 * 1024 * 1024,
    };

    fn budget() -> ImageBudget {
        ImageBudget::new(SMALL)
    }

    fn load(budget: &mut ImageBudget, key: u64, cost: u64) -> Vec<u64> {
        let mut evicted = budget.insert_loading(key);
        evicted.extend(budget.resolve(key, cost));
        evicted
    }

    #[test]
    fn a_hit_moves_the_entry_away_from_the_eviction_end() {
        let mut budget = budget();
        load(&mut budget, 1, 10);
        load(&mut budget, 2, 10);
        load(&mut budget, 3, 10);

        assert!(budget.touch(1), "1 is still resident");
        let evicted = load(&mut budget, 4, 10);

        assert_eq!(evicted, vec![2], "the touched entry is not the oldest now");
        assert!(budget.contains(1));
        assert!(!budget.contains(2));
    }

    #[test]
    fn room_is_made_when_a_load_starts_rather_than_when_it_finishes() {
        let mut budget = budget();
        load(&mut budget, 1, 10);
        load(&mut budget, 2, 10);
        load(&mut budget, 3, 10);

        assert_eq!(
            budget.insert_loading(4),
            vec![1],
            "waiting for the decode would let the cache sit a whole request over its ceiling"
        );
        assert!(budget.resolve(4, 10).is_empty());
    }

    #[test]
    fn the_count_ceiling_holds_however_small_the_images_are() {
        let mut budget = budget();
        for key in 0..50 {
            load(&mut budget, key, 1);
        }

        assert_eq!(budget.len(), SMALL.max_entries);
        assert!(budget.bytes() <= SMALL.max_bytes);
    }

    #[test]
    fn the_byte_ceiling_holds_however_few_the_images_are() {
        let mut budget = ImageBudget::new(ImageLimits {
            max_entries: 1_000,
            max_bytes: 1_000,
        });

        for key in 0..50 {
            load(&mut budget, key, 400);
        }

        assert!(
            budget.bytes() <= 1_000,
            "two 400-byte images fit and three do not; the cache holds {}",
            budget.bytes()
        );
        assert!(budget.len() <= 3);
    }

    #[test]
    fn a_long_scroll_converges_instead_of_growing() {
        let mut budget = ImageBudget::new(TEST_FEED);
        for key in 0..5_000 {
            load(&mut budget, key, 256 * 1024);
        }

        assert!(budget.len() <= TEST_FEED.max_entries);
        assert!(budget.bytes() <= TEST_FEED.max_bytes);
    }

    #[test]
    fn an_image_in_flight_is_never_evicted_out_from_under_whoever_asked_for_it() {
        let mut budget = budget();
        for key in 1..=4 {
            assert!(
                budget.insert_loading(key).is_empty(),
                "dropping an in-flight entry leaves an element waiting on a discarded task"
            );
        }

        assert_eq!(
            budget.len(),
            4,
            "over the count ceiling, and deliberately so"
        );

        assert!(budget.resolve(1, 10).is_empty());
        assert_eq!(budget.len(), 4);

        assert_eq!(budget.resolve(2, 10), vec![1]);
        assert_eq!(budget.len(), 3);
    }

    #[test]
    fn a_cache_smaller_than_the_viewport_thrashes_and_that_is_the_sizing_answer() {
        let mut budget = budget();
        for key in 1..=4 {
            load(&mut budget, key, 10);
        }

        assert_eq!(budget.len(), SMALL.max_entries);
        assert!(!budget.contains(1), "the oldest of the four is gone");
    }

    #[test]
    fn an_image_larger_than_the_whole_budget_is_kept_rather_than_thrashed() {
        let mut budget = budget();
        load(&mut budget, 1, 10);
        let evicted = load(&mut budget, 2, 10_000);

        assert_eq!(evicted, vec![1]);
        assert!(
            budget.contains(2),
            "evicting the entry that was just admitted makes the next frame ask for it again"
        );
        assert!(budget.bytes() > SMALL.max_bytes, "over budget, knowingly");
    }

    #[test]
    fn resolving_the_same_key_twice_does_not_double_count_its_bytes() {
        let mut budget = ImageBudget::new(ImageLimits {
            max_entries: 10,
            max_bytes: 10_000,
        });

        budget.insert_loading(1);
        budget.resolve(1, 100);
        budget.resolve(1, 300);

        assert_eq!(budget.bytes(), 300);
    }

    #[test]
    fn a_failed_load_is_remembered_so_a_broken_url_is_not_refetched_every_frame() {
        let mut budget = budget();
        budget.insert_loading(1);
        budget.resolve(1, 0);

        assert!(budget.contains(1));
        assert_eq!(budget.bytes(), 0);
    }

    #[test]
    fn removing_and_draining_give_the_bytes_back() {
        let mut budget = budget();
        load(&mut budget, 1, 100);
        load(&mut budget, 2, 100);

        assert!(budget.remove(1));
        assert_eq!(budget.bytes(), 100);
        assert!(!budget.remove(1), "removing twice is not an error");

        assert_eq!(budget.drain(), vec![2]);
        assert_eq!(budget.bytes(), 0);
        assert!(budget.is_empty());
    }

    #[test]
    fn asking_for_a_resident_image_again_never_starts_a_second_load() {
        let mut budget = budget();
        budget.insert_loading(1);
        let evicted = budget.insert_loading(1);

        assert!(evicted.is_empty());
        assert_eq!(budget.len(), 1);
    }
}
