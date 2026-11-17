use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::desktop_runtime::rise_account_runtime::{GLOBAL_SLOT, RiseAccountRuntime, RiseAccountRuntimeAccount};

const WAIT: Duration = Duration::from_secs(5);

struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

const FIRST_EPOCH: &str = "8F0C2B1E-4D3A-4B6F-9E21-0A7C5D3B9F10";
const SECOND_EPOCH: &str = "2A9D4E6C-1B7F-4C8A-B3E5-6F0D2C9A7B41";

fn account_id(raw: &str) -> String {
    raw.to_owned()
}

fn epoch(raw: &str) -> String {
    raw.to_owned()
}

fn account(raw: &str) -> RiseAccountRuntimeAccount {
    RiseAccountRuntimeAccount {
        account_id: account_id(raw),
        epoch: epoch(FIRST_EPOCH),
    }
}

async fn spawn_parked(runtime: &RiseAccountRuntime) -> (JoinHandle<()>, Arc<AtomicBool>) {
    let dropped = Arc::new(AtomicBool::new(false));
    let flag = DropFlag(Arc::clone(&dropped));
    let (started_tx, started_rx) = oneshot::channel();
    let handle = runtime.spawn(async move {
        let _flag = flag;
        let _ = started_tx.send(());
        std::future::pending::<()>().await;
    });
    started_rx.await.expect("the parked task starts");
    (handle, dropped)
}

async fn assert_cancelled(handle: JoinHandle<()>, dropped: &AtomicBool) {
    let joined = timeout(WAIT, handle)
        .await
        .expect("a cancelled task resolves its handle");
    assert!(joined.is_err_and(|error| error.is_cancelled()));
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_spawn_runs_the_future_and_returns_its_output() {
    let runtime = RiseAccountRuntime::new(Some(account("u1")));

    let value = timeout(WAIT, runtime.spawn(async { 40 + 2 }))
        .await
        .expect("the task finishes")
        .expect("the task is not cancelled");

    assert_eq!(value, 42);
}

#[tokio::test]
async fn test_spawn_blocking_runs_on_the_account_runtime() {
    let runtime = RiseAccountRuntime::new(Some(account("u1")));

    let value = timeout(WAIT, runtime.spawn_blocking(|| "done"))
        .await
        .expect("the blocking work finishes")
        .expect("the blocking work is not cancelled");

    assert_eq!(value, "done");
}

#[tokio::test]
async fn test_shutdown_cancels_pending_tasks_and_drops_their_futures() {
    let runtime = RiseAccountRuntime::new(Some(account("u1")));
    let (handle, dropped) = spawn_parked(&runtime).await;

    runtime.shutdown();

    assert!(runtime.is_shut_down());
    assert_cancelled(handle, &dropped).await;
}

#[tokio::test]
async fn test_spawn_after_shutdown_resolves_as_cancelled() {
    let runtime = RiseAccountRuntime::new(Some(account("u1")));
    runtime.shutdown();

    let joined = timeout(WAIT, runtime.spawn(async { 1 }))
        .await
        .expect("a task spawned on a stopped runtime resolves its handle");

    assert!(joined.is_err_and(|error| error.is_cancelled()));
}

#[tokio::test]
async fn test_each_runtime_has_its_own_workers() {
    let first = RiseAccountRuntime::new(Some(account("u1")));
    let second = RiseAccountRuntime::new(Some(account("u2")));
    let (handle, dropped) = spawn_parked(&first).await;

    assert_ne!(first.serial(), second.serial());
    second.shutdown();
    assert!(!first.is_shut_down());
    assert!(!dropped.load(Ordering::SeqCst));

    first.shutdown();
    assert_cancelled(handle, &dropped).await;
}

#[tokio::test]
async fn test_account_reset_cancels_account_tasks() {
    let _slot = GLOBAL_SLOT.lock().await;
    RiseAccountRuntime::reset();
    let active = RiseAccountRuntime::activate(account_id("u1"), epoch(FIRST_EPOCH));
    let (handle, dropped) = spawn_parked(&active).await;

    RiseAccountRuntime::reset();

    assert!(active.is_shut_down());
    assert!(!active.is_current());
    assert_cancelled(handle, &dropped).await;
    let next = RiseAccountRuntime::current();
    assert!(next.account().is_none());
    assert!(!next.is_shut_down());
    assert_ne!(next.serial(), active.serial());
    RiseAccountRuntime::reset();
}

#[tokio::test]
async fn test_current_without_an_account_is_signed_out_and_stable() {
    let _slot = GLOBAL_SLOT.lock().await;
    RiseAccountRuntime::reset();

    let first = RiseAccountRuntime::current();
    let second = RiseAccountRuntime::current();

    assert!(first.account().is_none());
    assert!(Arc::ptr_eq(&first, &second));
    assert!(first.is_current());
    RiseAccountRuntime::reset();
}

#[tokio::test]
async fn test_activation_adopts_the_signed_out_runtime_without_cancelling_it() {
    let _slot = GLOBAL_SLOT.lock().await;
    RiseAccountRuntime::reset();
    let signed_out = RiseAccountRuntime::current();
    let (handle, dropped) = spawn_parked(&signed_out).await;

    let active = RiseAccountRuntime::activate(account_id("u1"), epoch(FIRST_EPOCH));

    assert_eq!(active.serial(), signed_out.serial());
    assert_eq!(active.account(), Some(&account("u1")));
    assert!(active.is_current());
    assert!(!signed_out.is_current());
    assert!(!dropped.load(Ordering::SeqCst));

    RiseAccountRuntime::reset();
    assert_cancelled(handle, &dropped).await;
}

#[tokio::test]
async fn test_activating_the_same_account_keeps_its_runtime() {
    let _slot = GLOBAL_SLOT.lock().await;
    RiseAccountRuntime::reset();

    let first = RiseAccountRuntime::activate(account_id("u1"), epoch(FIRST_EPOCH));
    let second = RiseAccountRuntime::activate(account_id("u1"), epoch(FIRST_EPOCH));

    assert!(Arc::ptr_eq(&first, &second));
    assert!(Arc::ptr_eq(&first, &RiseAccountRuntime::current()));
    RiseAccountRuntime::reset();
}

#[tokio::test]
async fn test_switching_account_cancels_the_previous_account_tasks() {
    let _slot = GLOBAL_SLOT.lock().await;
    RiseAccountRuntime::reset();
    let previous = RiseAccountRuntime::activate(account_id("u1"), epoch(FIRST_EPOCH));
    let (handle, dropped) = spawn_parked(&previous).await;

    let next = RiseAccountRuntime::activate(account_id("u2"), epoch(FIRST_EPOCH));

    assert!(previous.is_shut_down());
    assert!(!next.is_shut_down());
    assert_ne!(next.serial(), previous.serial());
    assert_cancelled(handle, &dropped).await;
    RiseAccountRuntime::reset();
}

#[tokio::test]
async fn test_a_new_epoch_of_the_same_account_cancels_the_previous_epoch_tasks() {
    let _slot = GLOBAL_SLOT.lock().await;
    RiseAccountRuntime::reset();
    let previous = RiseAccountRuntime::activate(account_id("u1"), epoch(FIRST_EPOCH));
    let (handle, dropped) = spawn_parked(&previous).await;

    let next = RiseAccountRuntime::activate(account_id("u1"), epoch(SECOND_EPOCH));

    assert!(previous.is_shut_down());
    assert_eq!(next.account().map(|a| a.epoch.as_str()), Some(SECOND_EPOCH));
    assert_cancelled(handle, &dropped).await;
    RiseAccountRuntime::reset();
}
