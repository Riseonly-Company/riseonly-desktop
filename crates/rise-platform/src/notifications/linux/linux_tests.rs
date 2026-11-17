use super::NativeNotifications;
use crate::app_info::IDENTITIES;
use crate::notifications::*;

#[test]
fn the_bus_service_is_handed_out_once() {
    let identity = &IDENTITIES[2];
    assert!(NativeNotifications.take_service(identity).is_some());
    assert!(NativeNotifications.take_service(identity).is_none());
}

#[test]
fn the_host_precondition_never_blocks_the_caller() {
    assert!(super::host_precondition_met());
}

#[test]
fn no_reply_field_is_promised_before_a_server_answers() {
    assert_eq!(
        NativeNotifications.reply_input_support(),
        crate::gpui_shim::PlatformSupport::Unsupported
    );
}
