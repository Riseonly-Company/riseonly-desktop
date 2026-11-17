use std::future::Future;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::{Mutex, RwLock};
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::task::JoinHandle;

static CURRENT: LazyLock<RwLock<Option<Arc<RiseAccountRuntime>>>> =
    LazyLock::new(|| RwLock::new(None));
static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);
#[cfg(test)]
pub(crate) static GLOBAL_SLOT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// account id and epoch a runtime was activated for, both strings
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseAccountRuntimeAccount {
    pub account_id: String,
    pub epoch: String,
}

struct RiseAccountRuntimeCell {
    #[cfg(test)]
    serial: u64,
    handle: Handle,
    runtime: Mutex<Option<Runtime>>,
}

impl RiseAccountRuntimeCell {
    fn build() -> Self {
        let serial = NEXT_SERIAL.fetch_add(1, Ordering::Relaxed);
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .thread_name(format!("rise-account-{serial}"))
            .build()
            .expect("the OS refused to start the account runtime worker threads");
        Self {
            #[cfg(test)]
            serial,
            handle: runtime.handle().clone(),
            runtime: Mutex::new(Some(runtime)),
        }
    }

    fn shutdown(&self) {
        if let Some(runtime) = self.runtime.lock().take() {
            runtime.shutdown_background();
        }
    }

    fn is_shut_down(&self) -> bool {
        self.runtime.lock().is_none()
    }
}

impl Drop for RiseAccountRuntimeCell {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// runtime of the active account, current() is the only entry for product code
pub struct RiseAccountRuntime {
    account: Option<RiseAccountRuntimeAccount>,
    cell: Arc<RiseAccountRuntimeCell>,
}

impl RiseAccountRuntime {
    pub(crate) fn new(account: Option<RiseAccountRuntimeAccount>) -> Self {
        Self {
            account,
            cell: Arc::new(RiseAccountRuntimeCell::build()),
        }
    }

    /// runtime of the active account, or the signed out one if none is active
    pub fn current() -> Arc<Self> {
        if let Some(current) = CURRENT.read().as_ref() {
            return Arc::clone(current);
        }
        let mut slot = CURRENT.write();
        Arc::clone(slot.get_or_insert_with(|| Arc::new(Self::new(None))))
    }

    /// binds the runtime to an account
    /// the signed out runtime is adopted as is so the login flow keeps running
    /// a different account or epoch gets a fresh runtime
    pub fn activate(account_id: String, epoch: String) -> Arc<Self> {
        let account = RiseAccountRuntimeAccount { account_id, epoch };
        let mut slot = CURRENT.write();
        let next = match slot.as_ref() {
            Some(current) if current.account.as_ref() == Some(&account) => {
                return Arc::clone(current);
            }
            Some(current) if current.account.is_none() && !current.cell.is_shut_down() => {
                Self {
                    account: Some(account),
                    cell: Arc::clone(&current.cell),
                }
            }
            Some(current) => {
                current.shutdown();
                Self::new(Some(account))
            }
            None => Self::new(Some(account)),
        };
        let next = Arc::new(next);
        *slot = Some(Arc::clone(&next));
        next
    }

    /// cancels every task of the active account, next current() starts signed out
    /// call it from the main thread
    pub fn reset() {
        let previous = CURRENT.write().take();
        if let Some(previous) = previous {
            previous.shutdown();
        }
    }

    pub fn account(&self) -> Option<&RiseAccountRuntimeAccount> {
        self.account.as_ref()
    }

    /// true if this is the runtime current() would return now
    pub fn is_current(&self) -> bool {
        CURRENT.read().as_ref().is_some_and(|current| {
            Arc::ptr_eq(&current.cell, &self.cell) && current.account == self.account
        })
    }

    pub fn is_shut_down(&self) -> bool {
        self.cell.is_shut_down()
    }

    /// runs the future on the account runtime, after a reset it resolves as cancelled
    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.cell.handle.spawn(future)
    }

    /// blocking file or system io off the main thread
    /// a call already running is not interrupted by a reset
    pub fn spawn_blocking<F, R>(&self, work: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        self.cell.handle.spawn_blocking(work)
    }

    /// runtime handle for engine internals that must enter it (timers, io drivers)
    pub fn handle(&self) -> &Handle {
        &self.cell.handle
    }

    #[cfg(test)]
    pub(crate) fn serial(&self) -> u64 {
        self.cell.serial
    }

    pub(crate) fn shutdown(&self) {
        self.cell.shutdown();
    }
}
