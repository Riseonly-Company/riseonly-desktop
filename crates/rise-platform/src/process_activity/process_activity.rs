use thiserror::Error;

use crate::host_os::HostOs;

/// why the lease protects nothing
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum ProcessActivityUnavailable {
    #[error("process activity is not held on {host:?}: the branch is not written")]
    BranchNotWritten { host: HostOs },
    #[error("{host:?} does not throttle a process without a visible window")]
    NotThrottled { host: HostOs },
}

/// token one os returns for a running activity, dropping it ends the activity
pub type ProcessActivityToken = Box<dyn Send + Sync>;

/// the system call of one os, it only asks the os to note the activity
/// so any thread is fine, main included
pub trait ProcessActivitySource {
    fn begin(&self, reason: &str) -> Result<ProcessActivityToken, ProcessActivityUnavailable> {
        let _ = reason;
        Err(ProcessActivityUnavailable::BranchNotWritten {
            host: HostOs::current(),
        })
    }
}

/// a running activity, ended when dropped
/// no expiration handler since the desktop os never takes the lease back
#[must_use = "the activity ends when the lease is dropped"]
pub struct ProcessActivityLease {
    token: Option<ProcessActivityToken>,
}

impl ProcessActivityLease {
    /// true when the os holds the activity, false from an os with nothing to hold off
    pub fn is_held(&self) -> bool {
        self.token.is_some()
    }
}

pub fn begin_process_activity_with(
    source: &dyn ProcessActivitySource,
    reason: &str,
) -> ProcessActivityLease {
    match source.begin(reason) {
        Ok(token) => ProcessActivityLease { token: Some(token) },
        Err(unavailable) => {
            tracing::debug!(target: "riseonly", %unavailable, reason, "process activity not held");
            ProcessActivityLease { token: None }
        }
    }
}

/// starts a process activity with this reason, ends when the lease drops
pub fn begin_process_activity(reason: &str) -> ProcessActivityLease {
    begin_process_activity_with(&super::os::NativeProcessActivity, reason)
}
