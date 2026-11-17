use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::notifications::*;

const ALL_HOSTS: [HostOs; 3] = [HostOs::MacOs, HostOs::Linux, HostOs::Windows];

const ALL_STATUSES: [NotificationAuthorizationStatus; 5] = [
    NotificationAuthorizationStatus::NotDetermined,
    NotificationAuthorizationStatus::Denied,
    NotificationAuthorizationStatus::Authorized,
    NotificationAuthorizationStatus::Provisional,
    NotificationAuthorizationStatus::Ephemeral,
];

#[test]
fn a_reported_status_wins_over_the_precondition() {
    for host in ALL_HOSTS {
        for met in [true, false] {
            for status in ALL_STATUSES {
                assert_eq!(
                    resolved_authorization_status(Some(status), availability_for(host, met)),
                    status
                );
            }
        }
    }
}

#[test]
fn without_a_report_a_deliverable_host_is_authorized() {
    for host in ALL_HOSTS {
        assert_eq!(
            resolved_authorization_status(None, availability_for(host, true)),
            NotificationAuthorizationStatus::Authorized
        );
    }
}

#[test]
fn without_a_report_a_blocked_host_is_denied() {
    for host in ALL_HOSTS {
        assert_eq!(
            resolved_authorization_status(None, availability_for(host, false)),
            NotificationAuthorizationStatus::Denied
        );
    }
}

#[test]
fn only_linux_has_no_notification_settings_address() {
    for host in ALL_HOSTS {
        assert_eq!(notification_settings_url(host).is_none(), host == HostOs::Linux, "{host:?}");
    }
}

struct NoNativeRoute;

impl NativeNotificationRoute for NoNativeRoute {}

#[test]
fn a_route_that_overrides_nothing_refuses_the_reply_field_and_the_request() {
    assert_eq!(NoNativeRoute.reply_input_support(), PlatformSupport::Unsupported);
    assert_eq!(NoNativeRoute.request_authorization(), PlatformSupport::Unsupported);
}

#[test]
fn a_gpui_response_carries_no_typed_text() {
    let response: NotificationActionResponse = gpui::SystemNotificationResponse {
        tag: "conv:c1".into(),
        action_id: Some(NotificationAction::REPLY_ID.into()),
    }
    .into();
    assert_eq!(response.tag, "conv:c1");
    assert_eq!(response.action_id.as_deref(), Some(NotificationAction::REPLY_ID));
    assert_eq!(response.user_text, None);
}

#[test]
fn a_text_input_keeps_its_labels_and_the_gpui_route_drops_them() {
    let action = NotificationAction::reply("Reply").with_text_input("Send", "Message");
    assert_eq!(
        action.text_input,
        Some(NotificationTextInput {
            button_title: "Send".to_owned(),
            placeholder: "Message".to_owned(),
        })
    );
    assert_eq!(NotificationAction::reply("Reply").text_input, None);

    let posted: gpui::SystemNotificationAction = action.into();
    assert_eq!(posted.id, NotificationAction::REPLY_ID);
    assert_eq!(posted.label, "Reply");
}
