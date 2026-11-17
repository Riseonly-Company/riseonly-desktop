use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::channel::oneshot;
use rise_core::RiseIsolated;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use crate::core::lib::rise_call_audio_ownership::RiseCallAudioOwnership;

/// blocking audio setup is serialized away from the ui
/// a prepared player is owned only by this queue until the result is handed to its caller
pub enum RiseAudioPlaybackWork {}

static QUEUE: LazyLock<RiseIsolated<()>> = LazyLock::new(|| RiseIsolated::new(()));

#[derive(Default)]
struct Cancellation {
    cancelled: AtomicBool,
}

impl Cancellation {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn check(&self) -> Result<(), CancellationError> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(CancellationError)
        } else {
            Ok(())
        }
    }
}

struct CancelOnDrop(Arc<Cancellation>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl RiseAudioPlaybackWork {
    pub fn dispose(operation: impl FnOnce() + Send + 'static) {
        drop(RiseAccountRuntime::current().spawn_blocking(move || QUEUE.with(|_| operation())));
    }

    /// runs the operation on the queue and gives it a cancel check for between blocking steps
    /// a value prepared after the caller left goes to discard so no player leaks
    pub async fn perform<Value, Failure, Discard, Operation>(
        discard: Discard,
        operation: Operation,
    ) -> Result<Value, Failure>
    where
        Value: Send + 'static,
        Failure: From<CancellationError> + Send + 'static,
        Discard: FnOnce(Value) + Send + 'static,
        Operation: FnOnce(&dyn Fn() -> Result<(), CancellationError>) -> Result<Value, Failure>
            + Send
            + 'static,
    {
        let cancellation = Arc::new(Cancellation::default());
        let _cancel_on_drop = CancelOnDrop(Arc::clone(&cancellation));
        let (sender, receiver) = oneshot::channel::<Result<Value, Failure>>();

        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            QUEUE.with(|_| {
                let check = || cancellation.check();
                if let Err(error) = check() {
                    let _ = sender.send(Err(error.into()));
                    return;
                }
                let value = match operation(&check) {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        return;
                    }
                };
                if let Err(error) = check() {
                    discard(value);
                    let _ = sender.send(Err(error.into()));
                    return;
                }
                if let Err(Ok(value)) = sender.send(Ok(value)) {
                    discard(value);
                }
            });
        }));

        match receiver.await {
            Ok(outcome) => outcome,
            Err(oneshot::Canceled) => Err(CancellationError.into()),
        }
    }
}

enum ActivateFailure {
    Cancelled,
    CallOwnsAudioSession,
}

impl From<CancellationError> for ActivateFailure {
    fn from(_: CancellationError) -> Self {
        Self::Cancelled
    }
}

pub enum RisePlaybackAudioSession {}

impl RisePlaybackAudioSession {
    /// true if playback may start now
    /// desktop has no audio session, only queue order matters
    /// cancelled by pause means false
    pub async fn activate() -> bool {
        if RiseCallAudioOwnership::is_call_active() {
            return true;
        }
        let checked = RiseAudioPlaybackWork::perform(|_: ()| {}, |check_cancellation| {
            check_cancellation()?;
            if RiseCallAudioOwnership::is_call_active() {
                return Err(ActivateFailure::CallOwnsAudioSession);
            }
            Ok::<(), ActivateFailure>(())
        })
        .await;
        match checked {
            Ok(()) => true,
            Err(ActivateFailure::CallOwnsAudioSession) => true,
            Err(ActivateFailure::Cancelled) => false,
        }
    }
}
