use std::ffi::c_void;

use objc2::rc::Retained;
use objc2_foundation::{ns_string, NSString};

use super::display_sleep::*;

type CfStringRef = *const c_void;
type IoPmAssertionId = u32;
type IoPmAssertionLevel = u32;
type IoReturn = i32;

const ASSERTION_LEVEL_ON: IoPmAssertionLevel = 255;
const IO_RETURN_SUCCESS: IoReturn = 0;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    #[link_name = "IOPMAssertionCreateWithName"]
    fn iopm_assertion_create_with_name(
        assertion_type: CfStringRef,
        level: IoPmAssertionLevel,
        name: CfStringRef,
        assertion_id: *mut IoPmAssertionId,
    ) -> IoReturn;
    #[link_name = "IOPMAssertionRelease"]
    fn iopm_assertion_release(assertion_id: IoPmAssertionId) -> IoReturn;
}

pub(super) struct NativeDisplaySleep;

impl DisplaySleepInhibitor for NativeDisplaySleep {
    fn inhibit(&self, reason: &str) -> Result<SystemInhibition, DisplaySleepUnavailable> {
        let assertion_type: &NSString = ns_string!("PreventUserIdleDisplaySleep");
        let name: Retained<NSString> = NSString::from_str(reason);
        let mut assertion_id: IoPmAssertionId = 0;
        let status = unsafe {
            iopm_assertion_create_with_name(
                std::ptr::from_ref(assertion_type).cast(),
                ASSERTION_LEVEL_ON,
                Retained::as_ptr(&name).cast(),
                &mut assertion_id,
            )
        };
        if status != IO_RETURN_SUCCESS {
            return Err(DisplaySleepUnavailable::SystemRefused(format!(
                "IOPMAssertionCreateWithName returned {status:#x}"
            )));
        }
        Ok(SystemInhibition::new(HeldAssertion(assertion_id)))
    }
}

struct HeldAssertion(IoPmAssertionId);

impl Drop for HeldAssertion {
    fn drop(&mut self) {
        let status = unsafe { iopm_assertion_release(self.0) };
        if status != IO_RETURN_SUCCESS {
            tracing::warn!(target: "riseonly", status, "IOPMAssertionRelease refused the display hold");
        }
    }
}
