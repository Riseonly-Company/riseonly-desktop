use crate::host_os::HostOs;

use super::process_activity::*;

pub(super) struct NativeProcessActivity;

impl ProcessActivitySource for NativeProcessActivity {
    fn begin(&self, _reason: &str) -> Result<ProcessActivityToken, ProcessActivityUnavailable> {
        Err(ProcessActivityUnavailable::NotThrottled {
            host: HostOs::Linux,
        })
    }
}
