use std::collections::HashMap;
use std::sync::Arc;

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::Shared;
use gpui::{AsyncApp, TestAppContext};
use parking_lot::Mutex;

use crate::core::media::rise_media_prefetch_limits::RiseImagePrefetchLimits;
use crate::core::ui::clever_image_disk_cache::{CleverImagePrefetchQueue, CleverImagePrefetchQueueRequest};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;

#[derive(Clone, Default)]
struct CleverImagePrefetchOperationProbeSnapshot {
    active: i64,
    maximum_active: i64,
    started: HashMap<String, i64>,
    completed: HashMap<String, i64>,
    cancelled: HashMap<String, i64>,
}

struct CleverImagePrefetchOperationProbe {
    state: Mutex<CleverImagePrefetchOperationProbeSnapshot>,
    release: Mutex<Option<oneshot::Sender<()>>>,
    gate: Shared<oneshot::Receiver<()>>,
}

impl CleverImagePrefetchOperationProbe {
    fn new() -> Arc<Self> {
        let (release, gate) = oneshot::channel();
        Arc::new(Self {
            state: Mutex::new(CleverImagePrefetchOperationProbeSnapshot::default()),
            release: Mutex::new(Some(release)),
            gate: gate.shared(),
        })
    }

    fn begin(&self, key: &str) {
        let mut state = self.state.lock();
        state.active += 1;
        state.maximum_active = state.maximum_active.max(state.active);
        *state.started.entry(key.to_owned()).or_insert(0) += 1;
    }

    fn finish(&self, key: &str) {
        let mut state = self.state.lock();
        state.active -= 1;
        *state.completed.entry(key.to_owned()).or_insert(0) += 1;
    }

    fn cancel(&self, key: &str) {
        let mut state = self.state.lock();
        state.active -= 1;
        *state.cancelled.entry(key.to_owned()).or_insert(0) += 1;
    }

    fn release(&self) {
        if let Some(release) = self.release.lock().take() {
            let _ = release.send(());
        }
    }

    fn snapshot(&self) -> CleverImagePrefetchOperationProbeSnapshot {
        self.state.lock().clone()
    }
}

struct CleverImagePrefetchOperationProbeRun {
    probe: Arc<CleverImagePrefetchOperationProbe>,
    key: String,
    finished: bool,
}

impl Drop for CleverImagePrefetchOperationProbeRun {
    fn drop(&mut self) {
        if !self.finished {
            self.probe.cancel(&self.key);
        }
    }
}

fn make_queue(maximum_concurrent: i64, probe: &Arc<CleverImagePrefetchOperationProbe>) -> Arc<CleverImagePrefetchQueue> {
    let probe = Arc::clone(probe);
    CleverImagePrefetchQueue::new(
        maximum_concurrent,
        RiseImagePrefetchLimits::queue_pending(),
        Arc::new(move |request: CleverImagePrefetchQueueRequest, _cx: AsyncApp| {
            let probe = Arc::clone(&probe);
            async move {
                probe.begin(&request.cache_key);
                let mut run = CleverImagePrefetchOperationProbeRun {
                    probe: Arc::clone(&probe),
                    key: request.cache_key.clone(),
                    finished: false,
                };
                let _ = probe.gate.clone().await;
                run.finished = true;
                probe.finish(&request.cache_key);
            }
            .boxed_local()
        }),
    )
}

fn account() -> (Option<String>, Option<String>) {
    (Some("account-a".to_owned()), Some("epoch-a".to_owned()))
}

fn total(counts: &HashMap<String, i64>) -> i64 {
    counts.values().sum()
}

fn count(values: &[String]) -> i64 {
    i64::try_from(values.len()).unwrap_or(i64::MAX)
}

#[gpui::test]
async fn bounds_concurrency_and_deduplicates_across_owners(cx: &mut TestAppContext) {
    let async_cx = cx.to_async();
    let probe = CleverImagePrefetchOperationProbe::new();
    let queue = make_queue(3, &probe);
    let urls: Vec<String> = (0..5).map(|index| format!("https://example.test/image-{index}.jpg")).collect();
    let mut input = urls.clone();
    input.push(urls[0].clone());
    input.push(format!(" {} ", urls[1]));

    let (user, epoch) = account();
    queue
        .enqueue(&input, user, epoch, None, true, CleverImageCachePolicy::Standard, None, &async_cx)
        .await;
    let (user, epoch) = account();
    queue
        .replace_directional_urls(
            &urls[..2],
            "chat-a",
            1,
            user,
            epoch,
            None,
            true,
            CleverImageCachePolicy::Standard,
            &async_cx,
        )
        .await;

    cx.run_until_parked();
    let saturated = probe.snapshot().active == 3;
    assert!(saturated);

    let before_cancel = probe.snapshot();
    assert_eq!(before_cancel.maximum_active, 3);
    assert_eq!(total(&before_cancel.started), 3);
    assert!(before_cancel.started.values().all(|value| *value == 1));

    queue.cancel_directional("chat-a", 2, &async_cx).await;
    probe.release();

    cx.run_until_parked();
    let completed = total(&probe.snapshot().completed) == count(&urls);
    assert!(completed);

    let last = probe.snapshot();
    assert_eq!(last.maximum_active, 3);
    assert_eq!(total(&last.started), count(&urls));
    assert!(last.started.values().all(|value| *value == 1));
    assert!(last.cancelled.is_empty());
}

#[gpui::test]
async fn replaces_direction_and_rejects_stale_generations(cx: &mut TestAppContext) {
    let async_cx = cx.to_async();
    let probe = CleverImagePrefetchOperationProbe::new();
    let queue = make_queue(1, &probe);
    let old: Vec<String> = (0..3).map(|index| format!("https://example.test/old-{index}.jpg")).collect();
    let new: Vec<String> = (0..2).map(|index| format!("https://example.test/new-{index}.jpg")).collect();
    let stale = vec!["https://example.test/stale.jpg".to_owned()];
    let stale_after_cancel = vec!["https://example.test/stale-after-cancel.jpg".to_owned()];

    let (user, epoch) = account();
    queue
        .replace_directional_urls(&old, "chat-a", 1, user, epoch, None, true, CleverImageCachePolicy::Standard, &async_cx)
        .await;
    cx.run_until_parked();
    let first_started = probe.snapshot().started.get(&old[0]) == Some(&1);
    assert!(first_started);

    let (user, epoch) = account();
    queue
        .replace_directional_urls(&new, "chat-a", 2, user, epoch, None, true, CleverImageCachePolicy::Standard, &async_cx)
        .await;
    probe.release();

    cx.run_until_parked();
    let replacement_completed = total(&probe.snapshot().completed) == count(&new);
    assert!(replacement_completed);

    let (user, epoch) = account();
    queue
        .replace_directional_urls(&stale, "chat-a", 1, user, epoch, None, true, CleverImageCachePolicy::Standard, &async_cx)
        .await;
    queue.cancel_directional("chat-a", 3, &async_cx).await;
    let (user, epoch) = account();
    queue
        .replace_directional_urls(
            &stale_after_cancel,
            "chat-a",
            2,
            user,
            epoch,
            None,
            true,
            CleverImageCachePolicy::Standard,
            &async_cx,
        )
        .await;
    cx.run_until_parked();

    let last = probe.snapshot();
    assert_eq!(last.maximum_active, 1);
    assert_eq!(last.started.get(&old[0]), Some(&1));
    assert_eq!(last.cancelled.get(&old[0]), Some(&1));
    assert_eq!(last.started.get(&old[1]), None);
    assert_eq!(last.started.get(&old[2]), None);
    assert_eq!(last.started.get(&new[0]), Some(&1));
    assert_eq!(last.started.get(&new[1]), Some(&1));
    assert_eq!(last.started.get(&stale[0]), None);
    assert_eq!(last.started.get(&stale_after_cancel[0]), None);
}

#[gpui::test]
async fn successor_survives_replacement_and_reset_drops_pending_work(cx: &mut TestAppContext) {
    let async_cx = cx.to_async();
    let probe = CleverImagePrefetchOperationProbe::new();
    let queue = make_queue(1, &probe);
    let repeated = vec!["https://example.test/repeated.jpg".to_owned()];
    let dropped = vec!["https://example.test/dropped.jpg".to_owned()];
    let after_reset = vec!["https://example.test/after-reset.jpg".to_owned()];

    let (user, epoch) = account();
    queue
        .replace_directional_urls(&repeated, "chat-a", 1, user, epoch, None, true, CleverImageCachePolicy::Standard, &async_cx)
        .await;
    cx.run_until_parked();
    let first_started = probe.snapshot().started.get(&repeated[0]) == Some(&1);
    assert!(first_started);

    let (user, epoch) = account();
    queue
        .replace_directional_urls(&repeated, "chat-a", 2, user, epoch, None, true, CleverImageCachePolicy::Standard, &async_cx)
        .await;
    cx.run_until_parked();
    let successor_started = probe.snapshot().started.get(&repeated[0]) == Some(&2);
    assert!(successor_started);

    let (user, epoch) = account();
    queue
        .enqueue(&dropped, user, epoch, None, true, CleverImageCachePolicy::Standard, None, &async_cx)
        .await;
    queue.reset(false).await;

    cx.run_until_parked();
    let reset_snapshot = probe.snapshot();
    let reset_cancelled = reset_snapshot.active == 0 && reset_snapshot.cancelled.get(&repeated[0]) == Some(&2);
    assert!(reset_cancelled);
    probe.release();
    cx.run_until_parked();

    let after_drop = probe.snapshot();
    assert_eq!(after_drop.started.get(&dropped[0]), None);

    let (user, epoch) = account();
    queue
        .enqueue(&after_reset, user, epoch, None, true, CleverImageCachePolicy::Standard, None, &async_cx)
        .await;
    cx.run_until_parked();
    let recovered = probe.snapshot().completed.get(&after_reset[0]) == Some(&1);
    assert!(recovered);
}
