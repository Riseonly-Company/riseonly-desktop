use std::sync::Arc;

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::{Either, Shared, select};
use gpui::{AsyncApp, TestAppContext};
use parking_lot::Mutex;

use crate::core::ui::clever_image_disk_cache::{
    CleverImageAsyncWorkLimiter, CleverImageSharedLoadRegistry, CleverImageWorkLane,
};

fn gate() -> (Mutex<Option<oneshot::Sender<()>>>, Shared<oneshot::Receiver<()>>) {
    let (release, gate) = oneshot::channel();
    (Mutex::new(Some(release)), gate.shared())
}

#[derive(Clone, Copy, Default)]
struct CleverImageSharedLoadProbeSnapshot {
    started: i64,
    completed: i64,
    cancelled: i64,
}

struct CleverImageSharedLoadProbe {
    state: Mutex<CleverImageSharedLoadProbeSnapshot>,
    release: Mutex<Option<oneshot::Sender<()>>>,
    gate: Shared<oneshot::Receiver<()>>,
}

struct CleverImageSharedLoadProbeRun {
    probe: Arc<CleverImageSharedLoadProbe>,
    finished: bool,
}

impl Drop for CleverImageSharedLoadProbeRun {
    fn drop(&mut self) {
        if !self.finished {
            self.probe.state.lock().cancelled += 1;
        }
    }
}

impl CleverImageSharedLoadProbe {
    fn new() -> Arc<Self> {
        let (release, gate) = gate();
        Arc::new(Self {
            state: Mutex::new(CleverImageSharedLoadProbeSnapshot::default()),
            release,
            gate,
        })
    }

    async fn run(self: Arc<Self>, value: i64) -> i64 {
        self.state.lock().started += 1;
        let mut run = CleverImageSharedLoadProbeRun {
            probe: Arc::clone(&self),
            finished: false,
        };
        let _ = self.gate.clone().await;
        run.finished = true;
        self.state.lock().completed += 1;
        value
    }

    fn release(&self) {
        if let Some(release) = self.release.lock().take() {
            let _ = release.send(());
        }
    }

    fn snapshot(&self) -> CleverImageSharedLoadProbeSnapshot {
        *self.state.lock()
    }
}

#[derive(Clone, Copy, Default)]
struct CleverImageLimiterProbeSnapshot {
    active: i64,
    maximum_active: i64,
    completed: i64,
}

struct CleverImageLimiterProbe {
    state: Mutex<CleverImageLimiterProbeSnapshot>,
    release: Mutex<Option<oneshot::Sender<()>>>,
    gate: Shared<oneshot::Receiver<()>>,
}

impl CleverImageLimiterProbe {
    fn new() -> Arc<Self> {
        let (release, gate) = gate();
        Arc::new(Self {
            state: Mutex::new(CleverImageLimiterProbeSnapshot::default()),
            release,
            gate,
        })
    }

    async fn run(&self) {
        {
            let mut state = self.state.lock();
            state.active += 1;
            state.maximum_active = state.maximum_active.max(state.active);
        }
        let _ = self.gate.clone().await;
        let mut state = self.state.lock();
        state.active -= 1;
        state.completed += 1;
    }

    fn release(&self) {
        if let Some(release) = self.release.lock().take() {
            let _ = release.send(());
        }
    }

    fn snapshot(&self) -> CleverImageLimiterProbeSnapshot {
        *self.state.lock()
    }
}

type Registry = Arc<CleverImageSharedLoadRegistry<String, i64>>;

fn shared_value(
    cx: &AsyncApp,
    registry: &Registry,
    probe: &Arc<CleverImageSharedLoadProbe>,
    key: &str,
    value: i64,
) -> gpui::Task<i64> {
    let registry = Arc::clone(registry);
    let probe = Arc::clone(probe);
    let key = key.to_owned();
    cx.spawn(async move |cx| {
        registry
            .value(key, async move |_cx: &mut AsyncApp| probe.run(value).await, cx)
            .await
    })
}

#[gpui::test]
async fn shared_load_survives_one_cancelled_waiter(cx: &mut TestAppContext) {
    let async_cx = cx.to_async();
    let registry: Registry = Arc::new(CleverImageSharedLoadRegistry::new());
    let probe = CleverImageSharedLoadProbe::new();

    let first = shared_value(&async_cx, &registry, &probe, "same", 41);
    cx.run_until_parked();
    let first_started = probe.snapshot().started == 1;
    assert!(first_started);

    let second = shared_value(&async_cx, &registry, &probe, "same", 99);
    cx.run_until_parked();
    let both_leased = registry.waiter_count(&"same".to_owned()).await == 2;
    assert!(both_leased);

    drop(first);
    cx.run_until_parked();
    let first_released = registry.waiter_count(&"same".to_owned()).await == 1;
    assert!(first_released);
    assert_eq!(probe.snapshot().cancelled, 0);

    probe.release();
    assert_eq!(second.await, 41);
    cx.run_until_parked();
    assert_eq!(probe.snapshot().started, 1);
    assert_eq!(probe.snapshot().completed, 1);
    assert_eq!(registry.active_entry_count().await, 0);
}

#[gpui::test]
async fn last_cancelled_waiter_cancels_shared_operation(cx: &mut TestAppContext) {
    let async_cx = cx.to_async();
    let registry: Registry = Arc::new(CleverImageSharedLoadRegistry::new());
    let probe = CleverImageSharedLoadProbe::new();
    let waiter = shared_value(&async_cx, &registry, &probe, "only", 7);

    cx.run_until_parked();
    let waiter_count = registry.waiter_count(&"only".to_owned()).await;
    let started = waiter_count == 1 && probe.snapshot().started == 1;
    assert!(started);
    drop(waiter);

    cx.run_until_parked();
    let active_entries = registry.active_entry_count().await;
    let cancelled = probe.snapshot().cancelled == 1 && active_entries == 0;
    assert!(cancelled);
}

#[gpui::test]
async fn reset_cancels_all_entries_and_allows_fresh_work(cx: &mut TestAppContext) {
    let async_cx = cx.to_async();
    let registry: Registry = Arc::new(CleverImageSharedLoadRegistry::new());
    let probe = CleverImageSharedLoadProbe::new();
    let first = shared_value(&async_cx, &registry, &probe, "a", 1);
    let second = shared_value(&async_cx, &registry, &probe, "b", 2);
    cx.run_until_parked();
    let started = probe.snapshot().started == 2;
    assert!(started);

    registry.reset().await;
    let _ = first.await;
    let _ = second.await;
    cx.run_until_parked();
    let active_entries = registry.active_entry_count().await;
    let reset_finished = probe.snapshot().cancelled == 2 && active_entries == 0;
    assert!(reset_finished);

    probe.release();
    let fresh = shared_value(&async_cx, &registry, &probe, "a", 3).await;
    assert_eq!(fresh, 3);
    assert_eq!(probe.snapshot().started, 3);
}

#[gpui::test]
async fn async_limiter_bounds_concurrent_work_without_blocking_threads(cx: &mut TestAppContext) {
    let async_cx = cx.to_async();
    let limiter = Arc::new(CleverImageAsyncWorkLimiter::new(2));
    let probe = CleverImageLimiterProbe::new();
    let tasks: Vec<gpui::Task<()>> = (0..8)
        .map(|_| {
            let limiter = Arc::clone(&limiter);
            let probe = Arc::clone(&probe);
            async_cx.spawn(async move |_cx| {
                let Some(permit) = limiter.acquire(CleverImageWorkLane::Visible).await else {
                    return;
                };
                probe.run().await;
                drop(permit);
            })
        })
        .collect();

    cx.run_until_parked();
    let (limiter_active, limiter_waiting) = limiter.snapshot().await;
    let saturated = limiter_active == 2 && limiter_waiting == 6 && probe.snapshot().active == 2;
    assert!(saturated);
    assert_eq!(probe.snapshot().maximum_active, 2);

    probe.release();
    for task in tasks {
        task.await;
    }
    assert_eq!(probe.snapshot().completed, 8);
    assert_eq!(probe.snapshot().maximum_active, 2);
    let (final_active, final_waiting) = limiter.snapshot().await;
    assert_eq!(final_active, 0);
    assert_eq!(final_waiting, 0);
}

#[gpui::test]
async fn cancelled_queued_permit_does_not_leak_capacity(cx: &mut TestAppContext) {
    let async_cx = cx.to_async();
    let limiter = Arc::new(CleverImageAsyncWorkLimiter::new(1));
    let held = limiter.acquire(CleverImageWorkLane::Visible).await;
    assert!(held.is_some());

    let (cancel, cancelled) = oneshot::channel::<()>();
    let queued_limiter = Arc::clone(&limiter);
    let queued = async_cx.spawn(async move |_cx| {
        let acquire = queued_limiter.acquire(CleverImageWorkLane::Visible).boxed_local();
        match select(acquire, cancelled).await {
            Either::Left((permit, _)) => permit.is_some(),
            Either::Right(_) => false,
        }
    });
    cx.run_until_parked();
    let did_queue = limiter.snapshot().await.1 == 1;
    assert!(did_queue);

    let _ = cancel.send(());
    assert!(!queued.await);
    drop(held);
    let (empty_active, empty_waiting) = limiter.snapshot().await;
    assert_eq!(empty_active, 0);
    assert_eq!(empty_waiting, 0);

    let again = limiter.acquire(CleverImageWorkLane::Visible).await;
    assert!(again.is_some());
    drop(again);
}
