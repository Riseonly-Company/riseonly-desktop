use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::FutureExt;

use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::rise_engine::{RiseEngineError, RiseEngineLifecycle};

#[derive(Default)]
struct RiseLifecycleCloseProbe {
    prepared: AtomicBool,
    finished: AtomicBool,
}

impl RiseLifecycleCloseProbe {
    fn mark_prepared(&self) {
        self.prepared.store(true, Ordering::SeqCst);
    }

    fn mark_finished(&self) {
        self.finished.store(true, Ordering::SeqCst);
    }

    fn did_prepare(&self) -> bool {
        self.prepared.load(Ordering::SeqCst)
    }

    fn did_finish(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }
}

#[tokio::test]
async fn close_rejects_new_work_and_waits_for_every_admitted_operation() {
    let lifecycle = Arc::new(RiseEngineLifecycle::new());
    let probe = Arc::new(RiseLifecycleCloseProbe::default());
    lifecycle.begin_operation().await.expect("the open lifecycle admits work");

    let close = {
        let lifecycle = lifecycle.clone();
        let prepared = probe.clone();
        let finished = probe.clone();
        RiseAccountRuntime::current().spawn(async move {
            lifecycle
                .close(
                    async move { prepared.mark_prepared() }.boxed(),
                    async move {
                        finished.mark_finished();
                        Ok::<(), anyhow::Error>(())
                    }
                    .boxed(),
                )
                .await
        })
    };
    for _ in 0..1_000 {
        if !lifecycle.accepts_operations().await && probe.did_prepare() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(!lifecycle.accepts_operations().await);

    assert_eq!(lifecycle.begin_operation().await, Err(RiseEngineError::AlreadyClosed));
    assert!(probe.did_prepare());
    assert!(!probe.did_finish());

    lifecycle.end_operation();
    close.await.expect("close task finished").expect("close succeeded");
    assert!(probe.did_finish());
}
