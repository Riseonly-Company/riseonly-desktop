use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

const CHANNEL_CAPACITY: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct UploadId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UploadState {
    Pending,
    Uploading,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, PartialEq, Debug)]
pub struct UploadProgress {
    pub id: UploadId,
    pub sent_bytes: u64,
    pub total_bytes: u64,
    pub state: UploadState,
}

impl UploadProgress {
    pub fn fraction(&self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.sent_bytes as f64 / self.total_bytes as f64).clamp(0.0, 1.0) as f32
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            UploadState::Completed | UploadState::Failed | UploadState::Cancelled
        )
    }
}

/// Coalesces byte-level progress into updates a UI can afford to observe.
///
/// A 40 MB upload in 64 KB slabs produces roughly 640 callbacks. Publishing all
/// of them would invalidate a view tree hundreds of times for a bar that moves a
/// fraction of a pixel, which is exactly the "hundreds of microscopic state
/// changes" the performance contract forbids. An update is published only when
/// the fraction moves past a threshold, or when the state itself changes —
/// state transitions are never coalesced away.
pub struct UploadProgressHub {
    epsilon: f32,
    published: Mutex<HashMap<u64, UploadProgress>>,
    updates: broadcast::Sender<UploadProgress>,
}

impl UploadProgressHub {
    pub const DEFAULT_EPSILON: f32 = 0.01;

    pub fn new(epsilon: f32) -> Arc<Self> {
        let (updates, _) = broadcast::channel(CHANNEL_CAPACITY);
        Arc::new(Self {
            epsilon: epsilon.clamp(0.0001, 0.5),
            published: Mutex::new(HashMap::new()),
            updates,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<UploadProgress> {
        self.updates.subscribe()
    }

    pub fn snapshot(&self, id: UploadId) -> Option<UploadProgress> {
        self.published
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id.0)
            .cloned()
    }

    pub fn active_count(&self) -> usize {
        self.published
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|progress| !progress.is_terminal())
            .count()
    }

    /// Returns whether the update was published rather than coalesced away.
    pub fn report(&self, progress: UploadProgress) -> bool {
        let mut published = self.published.lock().unwrap_or_else(|e| e.into_inner());

        let should_publish = match published.get(&progress.id.0) {
            None => true,
            Some(previous) => {
                previous.state != progress.state
                    || progress.is_terminal()
                    || (progress.fraction() - previous.fraction()).abs() >= self.epsilon
            }
        };

        if !should_publish {
            return false;
        }

        published.insert(progress.id.0, progress.clone());
        drop(published);

        let _ = self.updates.send(progress);
        true
    }

    /// Drops finished uploads so the map cannot grow for the life of the app.
    pub fn forget_terminal(&self) -> usize {
        let mut published = self.published.lock().unwrap_or_else(|e| e.into_inner());
        let before = published.len();
        published.retain(|_, progress| !progress.is_terminal());
        before - published.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> Arc<UploadProgressHub> {
        UploadProgressHub::new(UploadProgressHub::DEFAULT_EPSILON)
    }

    fn progress(sent: u64, state: UploadState) -> UploadProgress {
        UploadProgress {
            id: UploadId(1),
            sent_bytes: sent,
            total_bytes: 1_000,
            state,
        }
    }

    #[test]
    fn the_first_report_always_publishes() {
        assert!(hub().report(progress(0, UploadState::Pending)));
    }

    #[test]
    fn a_tiny_advance_is_coalesced_away() {
        let hub = hub();
        hub.report(progress(0, UploadState::Uploading));

        assert!(
            !hub.report(progress(2, UploadState::Uploading)),
            "0.2% of movement must not invalidate a view tree"
        );
    }

    #[test]
    fn an_advance_past_the_threshold_publishes() {
        let hub = hub();
        hub.report(progress(0, UploadState::Uploading));
        assert!(hub.report(progress(20, UploadState::Uploading)));
    }

    #[test]
    fn a_state_change_is_never_coalesced_even_without_movement() {
        let hub = hub();
        hub.report(progress(500, UploadState::Uploading));

        assert!(
            hub.report(progress(500, UploadState::Failed)),
            "a failure with no byte movement must still reach the UI"
        );
    }

    #[test]
    fn every_terminal_state_publishes() {
        for state in [
            UploadState::Completed,
            UploadState::Failed,
            UploadState::Cancelled,
        ] {
            let hub = hub();
            hub.report(progress(0, UploadState::Uploading));
            assert!(hub.report(progress(1, state)), "{state:?} must publish");
        }
    }

    #[test]
    fn a_realistic_upload_publishes_about_one_update_per_percent() {
        let hub = hub();
        let total = 40 * 1024 * 1024u64;
        let slab = 64 * 1024u64;

        let mut published = 0;
        let mut sent = 0;
        while sent < total {
            sent = (sent + slab).min(total);
            if hub.report(UploadProgress {
                id: UploadId(7),
                sent_bytes: sent,
                total_bytes: total,
                state: UploadState::Uploading,
            }) {
                published += 1;
            }
        }

        // One slab is 1/640 of the file, so the 1% threshold is crossed every
        // seventh slab: about 92 updates rather than 640.
        let slabs = total.div_ceil(slab);
        assert!(slabs > 600, "the test should exercise many slabs");
        assert!(
            published < slabs as usize / 5,
            "expected roughly one update per percent, got {published} from {slabs} slabs"
        );
        assert!(
            published >= 80,
            "progress must still be visibly smooth, got {published}"
        );
    }

    #[test]
    fn a_subscriber_sees_the_published_updates() {
        let hub = hub();
        let mut updates = hub.subscribe();

        hub.report(progress(0, UploadState::Uploading));
        hub.report(progress(1, UploadState::Uploading));
        hub.report(progress(1_000, UploadState::Completed));

        let first = updates.try_recv().unwrap();
        assert_eq!(first.state, UploadState::Uploading);
        let second = updates.try_recv().unwrap();
        assert_eq!(second.state, UploadState::Completed);
        assert!(
            updates.try_recv().is_err(),
            "the coalesced update must not appear"
        );
    }

    #[test]
    fn fraction_is_bounded_and_safe_on_a_zero_length_upload() {
        let zero = UploadProgress {
            id: UploadId(1),
            sent_bytes: 0,
            total_bytes: 0,
            state: UploadState::Pending,
        };
        assert_eq!(zero.fraction(), 0.0);

        let over = UploadProgress {
            id: UploadId(1),
            sent_bytes: 5_000,
            total_bytes: 1_000,
            state: UploadState::Uploading,
        };
        assert_eq!(over.fraction(), 1.0);
    }

    #[test]
    fn finished_uploads_are_forgotten_so_the_map_stays_bounded() {
        let hub = hub();
        for id in 0..10u64 {
            hub.report(UploadProgress {
                id: UploadId(id),
                sent_bytes: 100,
                total_bytes: 100,
                state: UploadState::Completed,
            });
        }
        hub.report(UploadProgress {
            id: UploadId(99),
            sent_bytes: 1,
            total_bytes: 100,
            state: UploadState::Uploading,
        });

        assert_eq!(hub.active_count(), 1);
        assert_eq!(hub.forget_terminal(), 10);
        assert!(hub.snapshot(UploadId(0)).is_none());
        assert!(hub.snapshot(UploadId(99)).is_some());
    }
}
