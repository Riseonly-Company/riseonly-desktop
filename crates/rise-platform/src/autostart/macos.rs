use std::sync::OnceLock;

use objc2::{msg_send, MainThreadMarker};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSError};

use super::autostart::*;
use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

const STATUS_NOT_REGISTERED: isize = 0;
const STATUS_ENABLED: isize = 1;
const STATUS_REQUIRES_APPROVAL: isize = 2;
const STATUS_NOT_FOUND: isize = 3;

fn unavailable() -> AutostartError {
    AutostartError::Unavailable(AutostartMechanism::for_host(HostOs::current()))
}

fn main_app_service(config: &AutostartConfig) -> Option<Retained<AnyObject>> {
    if !is_bundled_macos_executable(&config.executable) {
        return None;
    }

    let class = AnyClass::get(c"SMAppService")?;

    unsafe { msg_send![class, mainAppService] }
}

fn status_of(service: &AnyObject) -> Result<AutostartState, AutostartError> {
    let status: isize = unsafe { msg_send![service, status] };

    match status {
        STATUS_NOT_REGISTERED => Ok(AutostartState::Disabled),
        STATUS_ENABLED => Ok(AutostartState::Enabled),
        STATUS_REQUIRES_APPROVAL => Ok(AutostartState::RequiresApproval),
        STATUS_NOT_FOUND => Err(unavailable()),
        other => Err(AutostartError::Refused(format!(
            "SMAppService reported an unknown status {other}"
        ))),
    }
}

pub(super) fn launched_at_login() -> bool {
    static ANSWER: OnceLock<bool> = OnceLock::new();
    if ANSWER.get().is_none() && MainThreadMarker::new().is_none() {
        return false;
    }
    *ANSWER.get_or_init(launch_event_says_login)
}

fn launch_event_says_login() -> bool {
    let Some(event) = NSAppleEventManager::sharedAppleEventManager().currentAppleEvent() else {
        return false;
    };
    let event_id: u32 = unsafe { msg_send![&*event, eventID] };
    let kind: Option<Retained<NSAppleEventDescriptor>> =
        unsafe { msg_send![&*event, paramDescriptorForKeyword: APPLE_EVENT_PROPERTY_DATA] };
    let kind = kind.map(|kind| -> u32 { unsafe { msg_send![&*kind, enumCodeValue] } });
    apple_event_is_login_launch(event_id, kind)
}

pub(super) fn availability(config: &AutostartConfig) -> PlatformSupport {
    match main_app_service(config) {
        Some(_) => PlatformSupport::Performed,
        None => PlatformSupport::Unsupported,
    }
}

pub(super) fn state(config: &AutostartConfig) -> Result<AutostartState, AutostartError> {
    let service = main_app_service(config).ok_or_else(unavailable)?;
    status_of(&service)
}

pub(super) fn set_enabled(
    config: &AutostartConfig,
    enabled: bool,
) -> Result<PlatformSupport, AutostartError> {
    let Some(service) = main_app_service(config) else {
        return Ok(PlatformSupport::Unsupported);
    };

    let action = transition(status_of(&service)?, enabled);
    if action == AutostartTransition::AlreadySettled {
        return Ok(PlatformSupport::Performed);
    }

    let outcome: Result<(), Retained<NSError>> = unsafe {
        if action == AutostartTransition::Register {
            msg_send![&*service, registerAndReturnError: _]
        } else {
            msg_send![&*service, unregisterAndReturnError: _]
        }
    };

    match outcome {
        Ok(()) => Ok(PlatformSupport::Performed),
        Err(error) => {
            if matches!(status_of(&service), Ok(state) if state.is_registered() == enabled) {
                Ok(PlatformSupport::Performed)
            } else {
                Err(AutostartError::Refused(error.to_string()))
            }
        }
    }
}
