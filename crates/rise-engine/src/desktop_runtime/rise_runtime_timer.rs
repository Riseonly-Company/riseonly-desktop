use std::time::Duration;

use tokio::runtime::Handle;

use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

/// timers safe to await from any executor
/// engine async fns run on account runtime and gpui tasks
/// tokio timer panics outside a runtime
pub enum RiseRuntimeTimer {}

impl RiseRuntimeTimer {
    /// waits duration, false if the account runtime shut down before it fired
    /// so a retry loop can stop
    pub async fn sleep(duration: Duration) -> bool {
        if Handle::try_current().is_ok() {
            tokio::time::sleep(duration).await;
            return true;
        }
        RiseAccountRuntime::current()
            .spawn(async move { tokio::time::sleep(duration).await })
            .await
            .is_ok()
    }
}
