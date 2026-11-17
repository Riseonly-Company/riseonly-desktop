use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};

use super::process_activity::*;

pub(super) struct NativeProcessActivity;

struct Activity(Retained<ProtocolObject<dyn NSObjectProtocol>>);

unsafe impl Send for Activity {}
unsafe impl Sync for Activity {}

impl Drop for Activity {
    fn drop(&mut self) {
        unsafe { NSProcessInfo::processInfo().endActivity(&self.0) };
    }
}

impl ProcessActivitySource for NativeProcessActivity {
    fn begin(&self, reason: &str) -> Result<ProcessActivityToken, ProcessActivityUnavailable> {
        let activity = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
            NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
            &NSString::from_str(reason),
        );
        Ok(Box::new(Activity(activity)))
    }
}
