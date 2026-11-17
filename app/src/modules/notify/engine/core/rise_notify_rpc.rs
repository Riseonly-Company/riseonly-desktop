use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::{RiseWireRequestMetadata, rise_request, rise_response, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::{RiseEngine, RiseWireMethodDescriptor, RiseWireReplayPolicy};

use crate::core::rise_engine_bridge::rise_canon_endpoints::{
    RiseCanonEndpoints, RiseEndpointClientTimeout,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseNotifyEndpointReplay {
    ReadOnly,
    Never,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseNotifyEndpoint {
    pub method: String,
    pub timeout: Duration,
    pub replay: RiseNotifyEndpointReplay,
}

impl RiseNotifyEndpoint {
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

    /// new endpoint descriptor, timeout has a default
    pub fn new(method: impl Into<String>, timeout: Duration, replay: RiseNotifyEndpointReplay) -> Self {
        Self {
            method: method.into(),
            timeout,
            replay,
        }
    }

    /// descriptor from the canon catalogue
    /// deadline and replay policy come from there, expecting only checks the declared intent
    #[track_caller]
    pub fn canonical(method: &str, expecting: RiseEndpointKind) -> Self {
        let entry = RiseCanonEndpoints::required(rise_wire_methods::notify::SERVICE, method);
        debug_assert!(
            entry.kind == expecting,
            "notify.{method}: canon declares {}, not {}",
            entry.kind.name(),
            expecting.name()
        );
        Self {
            method: entry.method.to_owned(),
            timeout: entry.client_timeout(),
            replay: if entry.kind == RiseEndpointKind::Read {
                RiseNotifyEndpointReplay::ReadOnly
            } else {
                RiseNotifyEndpointReplay::Never
            },
        }
    }

    pub fn descriptor(&self) -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(rise_wire_methods::notify::SERVICE, self.method.clone())
            .with_timeout(self.timeout)
            .with_replay_policy(match self.replay {
                RiseNotifyEndpointReplay::ReadOnly => RiseWireReplayPolicy::ReadOnly,
                RiseNotifyEndpointReplay::Never => RiseWireReplayPolicy::Never,
            })
    }
}

static GET_APP_RELEASE_NOTES: LazyLock<RiseNotifyEndpoint> = LazyLock::new(|| {
    RiseNotifyEndpoint::canonical(rise_wire_methods::notify::GET_APP_RELEASE_NOTES, RiseEndpointKind::Read)
});

static ACK_APP_RELEASE: LazyLock<RiseNotifyEndpoint> = LazyLock::new(|| {
    RiseNotifyEndpoint::canonical(rise_wire_methods::notify::ACK_APP_RELEASE, RiseEndpointKind::Mutation)
});

static GET_PENDING_ANNOUNCEMENTS: LazyLock<RiseNotifyEndpoint> = LazyLock::new(|| {
    RiseNotifyEndpoint::canonical(rise_wire_methods::notify::GET_PENDING_ANNOUNCEMENTS, RiseEndpointKind::Read)
});

static ACK_ANNOUNCEMENT: LazyLock<RiseNotifyEndpoint> = LazyLock::new(|| {
    RiseNotifyEndpoint::canonical(rise_wire_methods::notify::ACK_ANNOUNCEMENT, RiseEndpointKind::Mutation)
});

static GET_UNREAD_NOTIFICATIONS_COUNT: LazyLock<RiseNotifyEndpoint> = LazyLock::new(|| {
    RiseNotifyEndpoint::canonical(
        rise_wire_methods::notify::GET_UNREAD_NOTIFICATIONS_COUNT,
        RiseEndpointKind::Read,
    )
});

static GET_NOTIFICATIONS: LazyLock<RiseNotifyEndpoint> = LazyLock::new(|| {
    RiseNotifyEndpoint::canonical(rise_wire_methods::notify::GET_NOTIFICATIONS, RiseEndpointKind::Read)
});

static MARK_NOTIFICATION_READ: LazyLock<RiseNotifyEndpoint> = LazyLock::new(|| {
    RiseNotifyEndpoint::canonical(rise_wire_methods::notify::MARK_NOTIFICATION_READ, RiseEndpointKind::Mutation)
});

static MARK_ALL_NOTIFICATIONS_READ: LazyLock<RiseNotifyEndpoint> = LazyLock::new(|| {
    RiseNotifyEndpoint::canonical(
        rise_wire_methods::notify::MARK_ALL_NOTIFICATIONS_READ,
        RiseEndpointKind::Mutation,
    )
});

static DELETE_NOTIFICATION: LazyLock<RiseNotifyEndpoint> = LazyLock::new(|| {
    RiseNotifyEndpoint::canonical(rise_wire_methods::notify::DELETE_NOTIFICATION, RiseEndpointKind::Mutation)
});

pub enum RiseNotifyEndpoints {}

impl RiseNotifyEndpoints {
    pub fn get_app_release_notes() -> &'static RiseNotifyEndpoint {
        &GET_APP_RELEASE_NOTES
    }

    pub fn ack_app_release() -> &'static RiseNotifyEndpoint {
        &ACK_APP_RELEASE
    }

    /// account scoped unlike the release read
    /// gateway takes user_id from the authenticated socket and the answer depends on audience and receipts
    pub fn get_pending_announcements() -> &'static RiseNotifyEndpoint {
        &GET_PENDING_ANNOUNCEMENTS
    }

    pub fn ack_announcement() -> &'static RiseNotifyEndpoint {
        &ACK_ANNOUNCEMENT
    }

    pub fn get_unread_notifications_count() -> &'static RiseNotifyEndpoint {
        &GET_UNREAD_NOTIFICATIONS_COUNT
    }

    /// one page of the feed, newest first
    /// account scoped like the badge
    pub fn get_notifications() -> &'static RiseNotifyEndpoint {
        &GET_NOTIFICATIONS
    }

    pub fn mark_notification_read() -> &'static RiseNotifyEndpoint {
        &MARK_NOTIFICATION_READ
    }

    pub fn mark_all_notifications_read() -> &'static RiseNotifyEndpoint {
        &MARK_ALL_NOTIFICATIONS_READ
    }

    pub fn delete_notification() -> &'static RiseNotifyEndpoint {
        &DELETE_NOTIFICATION
    }

    pub fn all() -> [&'static RiseNotifyEndpoint; 9] {
        [
            Self::get_app_release_notes(),
            Self::ack_app_release(),
            Self::get_pending_announcements(),
            Self::ack_announcement(),
            Self::get_unread_notifications_count(),
            Self::get_notifications(),
            Self::mark_notification_read(),
            Self::mark_all_notifications_read(),
            Self::delete_notification(),
        ]
    }

    pub fn endpoint(method: &str) -> Option<&'static RiseNotifyEndpoint> {
        let method = trimming_whitespaces_and_newlines(method);
        Self::all().into_iter().find(|endpoint| endpoint.method == method)
    }
}

#[derive(Clone)]
pub struct RiseNotifyRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseNotifyRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    pub async fn get_app_release_notes(
        &self,
        request: rise_request::notify::GetAppReleaseNotes,
    ) -> anyhow::Result<rise_response::notify::GetAppReleaseNotes> {
        self.call(RiseNotifyEndpoints::get_app_release_notes(), &request).await
    }

    pub async fn ack_app_release(
        &self,
        request: rise_request::notify::AckAppRelease,
    ) -> anyhow::Result<rise_response::notify::AckAppRelease> {
        self.call(RiseNotifyEndpoints::ack_app_release(), &request).await
    }

    pub async fn get_pending_announcements(
        &self,
        request: rise_request::notify::GetPendingAnnouncements,
    ) -> anyhow::Result<rise_response::notify::GetPendingAnnouncements> {
        self.call(RiseNotifyEndpoints::get_pending_announcements(), &request).await
    }

    pub async fn get_unread_notifications_count(
        &self,
        request: rise_request::notify::GetUnreadNotificationsCount,
    ) -> anyhow::Result<rise_response::notify::GetUnreadNotificationsCount> {
        self.call(RiseNotifyEndpoints::get_unread_notifications_count(), &request).await
    }

    pub async fn get_notifications(
        &self,
        request: rise_request::notify::GetNotifications,
    ) -> anyhow::Result<rise_response::notify::GetNotifications> {
        self.call(RiseNotifyEndpoints::get_notifications(), &request).await
    }

    pub async fn get_notifications_surviving_suspension(
        &self,
        request: rise_request::notify::GetNotifications,
    ) -> anyhow::Result<rise_response::notify::GetNotifications> {
        self.engine
            .call_descriptor_response(
                &RiseNotifyEndpoints::get_notifications().descriptor().with_survives_suspension(true),
                &request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await
    }

    pub async fn mark_notification_read(
        &self,
        request: rise_request::notify::MarkNotificationRead,
    ) -> anyhow::Result<rise_response::notify::MarkNotificationRead> {
        self.call(RiseNotifyEndpoints::mark_notification_read(), &request).await
    }

    pub async fn mark_all_notifications_read(
        &self,
        request: rise_request::notify::MarkAllNotificationsRead,
    ) -> anyhow::Result<rise_response::notify::MarkAllNotificationsRead> {
        self.call(RiseNotifyEndpoints::mark_all_notifications_read(), &request).await
    }

    pub async fn delete_notification(
        &self,
        request: rise_request::notify::DeleteNotification,
    ) -> anyhow::Result<rise_response::notify::DeleteNotification> {
        self.call(RiseNotifyEndpoints::delete_notification(), &request).await
    }

    pub async fn ack_announcement(
        &self,
        request: rise_request::notify::AckAnnouncement,
    ) -> anyhow::Result<rise_response::notify::AckAnnouncement> {
        self.call(RiseNotifyEndpoints::ack_announcement(), &request).await
    }

    async fn call<Request, Response>(
        &self,
        endpoint: &RiseNotifyEndpoint,
        request: &Request,
    ) -> anyhow::Result<Response>
    where
        Request: serde::Serialize + Sync,
        Response: serde::de::DeserializeOwned,
    {
        self.engine
            .call_descriptor_response(
                &endpoint.descriptor(),
                request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await
    }
}
