use rise_canon::{
    RiseTimingBatch, RiseTimingCanon, RiseTimingDebounce, RiseTimingHeartbeat, RiseTimingPage,
    RiseTimingServerWindow,
};

/// timing canon helpers for this bridge
pub trait RiseTimingCanonExt {
    /// heartbeat by canon name, a miss is a typo in code so it panics instead of guessing a period
    fn heartbeat(name: &str) -> RiseTimingHeartbeat;

    /// batch by canon name, chunk size and queue cap and flush threshold
    fn batch(name: &str) -> RiseTimingBatch;

    fn debounce(name: &str) -> RiseTimingDebounce;

    /// server window by canon name
    /// the client doesnt control it but must know its width
    /// so no deadline from silence and no stretching by clock skew
    fn server_window(name: &str) -> RiseTimingServerWindow;

    fn page(name: &str) -> RiseTimingPage;

    /// freshness window in milliseconds, no window outside the canon ladder exists
    fn freshness_ms(name: &str) -> i64;

    /// same window in seconds, for call sites measuring age as a time interval
    fn freshness_seconds(name: &str) -> f64;
}

impl RiseTimingCanonExt for RiseTimingCanon {
    fn heartbeat(name: &str) -> RiseTimingHeartbeat {
        match RiseTimingCanon::HEARTBEATS.iter().find(|spec| spec.name == name) {
            Some(spec) => spec.clone(),
            None => panic!("Canon timing heartbeat {name} is missing"),
        }
    }

    fn batch(name: &str) -> RiseTimingBatch {
        match RiseTimingCanon::BATCHES.iter().find(|spec| spec.name == name) {
            Some(spec) => spec.clone(),
            None => panic!("Canon timing batch {name} is missing"),
        }
    }

    fn debounce(name: &str) -> RiseTimingDebounce {
        match RiseTimingCanon::DEBOUNCES.iter().find(|spec| spec.name == name) {
            Some(spec) => spec.clone(),
            None => panic!("Canon timing debounce {name} is missing"),
        }
    }

    fn server_window(name: &str) -> RiseTimingServerWindow {
        match RiseTimingCanon::SERVER_WINDOWS.iter().find(|spec| spec.name == name) {
            Some(spec) => spec.clone(),
            None => panic!("Canon timing server window {name} is missing"),
        }
    }

    fn page(name: &str) -> RiseTimingPage {
        match RiseTimingCanon::PAGES.iter().find(|spec| spec.name == name) {
            Some(spec) => spec.clone(),
            None => panic!("Canon timing page {name} is missing"),
        }
    }

    fn freshness_ms(name: &str) -> i64 {
        match RiseTimingCanon::FRESHNESS.iter().find(|spec| spec.name == name) {
            Some(spec) => spec.window_ms,
            None => panic!("Canon timing freshness {name} is missing"),
        }
    }

    fn freshness_seconds(name: &str) -> f64 {
        Self::freshness_ms(name) as f64 / 1_000.0
    }
}

/// heartbeat timing helpers for this bridge
pub trait RiseTimingHeartbeatExt {
    /// for a nanosecond sleep, the sleep unit and not the canon unit
    fn client_interval_nanoseconds(&self) -> u64;

    fn client_interval_seconds(&self) -> f64;
}

impl RiseTimingHeartbeatExt for RiseTimingHeartbeat {
    fn client_interval_nanoseconds(&self) -> u64 {
        non_negative(self.client_interval_ms, &self.name, "clientIntervalMs") * 1_000_000
    }

    fn client_interval_seconds(&self) -> f64 {
        self.client_interval_ms as f64 / 1_000.0
    }
}

/// batch timing helpers for this bridge
pub trait RiseTimingBatchExt {
    fn queue_cap_items(&self) -> i64;

    fn flush_threshold_items(&self) -> i64;

    fn flush_interval_seconds(&self) -> u64;

    /// retries of a failed send before giving up
    fn maximum_retries(&self) -> i64;
}

impl RiseTimingBatchExt for RiseTimingBatch {
    fn queue_cap_items(&self) -> i64 {
        required(self.client_queue_cap_items, &self.name, "clientQueueCapItems")
    }

    fn flush_threshold_items(&self) -> i64 {
        required(self.client_flush_threshold_items, &self.name, "clientFlushThresholdItems")
    }

    fn flush_interval_seconds(&self) -> u64 {
        let interval_ms = required(self.client_flush_interval_ms, &self.name, "clientFlushIntervalMs");
        non_negative(interval_ms, &self.name, "clientFlushIntervalMs") / 1_000
    }

    fn maximum_retries(&self) -> i64 {
        required(self.client_maximum_retries, &self.name, "clientMaximumRetries")
    }
}

fn required(value: Option<i64>, name: &str, field: &str) -> i64 {
    match value {
        Some(value) => value,
        None => panic!("Canon timing batch {name} has no {field}"),
    }
}

fn non_negative(value: i64, name: &str, field: &str) -> u64 {
    match u64::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("Canon timing {name} has a negative {field}"),
    }
}

/// debounce timing helpers for this bridge
pub trait RiseTimingDebounceExt {
    fn client_debounce_nanoseconds(&self) -> u64;
}

impl RiseTimingDebounceExt for RiseTimingDebounce {
    fn client_debounce_nanoseconds(&self) -> u64 {
        non_negative(self.client_debounce_ms, &self.name, "clientDebounceMs") * 1_000_000
    }
}

/// page timing helpers for this bridge
pub trait RiseTimingPageExt {
    /// requested page size, none where the client sends no limit
    fn page_size_items(&self) -> i64;
}

impl RiseTimingPageExt for RiseTimingPage {
    fn page_size_items(&self) -> i64 {
        match self.client_page_size_items {
            Some(size) => size,
            None => panic!("Canon timing page {} has no client page size", self.name),
        }
    }
}
