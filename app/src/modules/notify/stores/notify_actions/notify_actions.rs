use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AsyncApp, Context, Entity, ForegroundExecutor, Task};
use rise_canon::RiseWireRequestMetadata;
use rise_canon::rise_request::notify::register_device_token;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::CancellationError;

use super::notify_profile_settings_wire_types::ProfileNotificationSettingsGatewayDTO;
use super::notify_wire_types::{NotificationSettingsGatewayDTO, RiseAPNSRegistrationTarget};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiTypedSlice;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::notify::engine::core::rise_notify_rpc::RiseNotifyEndpoint;
use crate::modules::notify::engine::core::rise_release_notes_seen_store::RiseReleaseNotesDeviceContext;
use crate::modules::notify::engine::rise_notify_domain::{RiseNotifyActionExecutor, RiseNotifyDomain};

pub struct NotifyActionsStore {
    pub get_notification_settings_sai: Rc<SaiTypedSlice<NotificationSettingsGatewayDTO>>,
    pub update_notification_settings_sai: Rc<SaiTypedSlice<NotificationSettingsGatewayDTO>>,
    pub get_profile_notification_settings_sai: Rc<SaiTypedSlice<ProfileNotificationSettingsGatewayDTO>>,
    pub update_profile_notification_settings_sai: Rc<SaiTypedSlice<ProfileNotificationSettingsGatewayDTO>>,
}

struct NotificationsObservation {
    domain: Arc<RiseNotifyDomain>,
    observer_id: Uuid,
    executor: ForegroundExecutor,
}

impl Drop for NotificationsObservation {
    fn drop(&mut self) {
        let domain = Arc::clone(&self.domain);
        let observer_id = self.observer_id;
        self.executor
            .spawn(async move {
                domain.end_observing_notifications(observer_id).await;
            })
            .detach();
    }
}

impl NotifyActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            get_notification_settings_sai: SaiTypedSlice::new(cx),
            update_notification_settings_sai: SaiTypedSlice::new(cx),
            get_profile_notification_settings_sai: SaiTypedSlice::new(cx),
            update_profile_notification_settings_sai: SaiTypedSlice::new(cx),
        }
    }

    pub(super) fn effective_user_id_for_wire(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .filter(|user_id| !user_id.is_empty())
    }

    pub(super) fn slices_did_change(cx: &mut App) {
        cx.defer(|cx| {
            Self::shared(cx).update(cx, |_, cx| cx.notify());
        });
    }

    pub(super) fn with_domain<Work, Fut>(cx: &mut App, work: Work)
    where
        Work: FnOnce(Arc<RiseNotifyDomain>) -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let domain = RiseNotifyActionExecutor::require_domain(cx);
        cx.spawn(async move |_: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            work(domain).await;
        })
        .detach();
    }

    pub fn observe_notifications_action(observer_id: Uuid, cx: &mut App) -> Task<()> {
        let domain = RiseNotifyActionExecutor::require_domain(cx);
        let executor = cx.foreground_executor().clone();
        cx.spawn(async move |_: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            let _observation = NotificationsObservation {
                domain: Arc::clone(&domain),
                observer_id,
                executor,
            };
            domain.begin_observing_notifications(observer_id).await;
            domain.load_notifications(false).await;
            futures::future::pending::<()>().await;
        })
    }

    pub fn load_notifications_action(force: bool, cx: &mut App) {
        Self::with_domain(cx, move |domain| async move {
            domain.load_notifications(force).await;
        });
    }

    pub fn load_more_notifications_action(cx: &mut App) {
        Self::with_domain(cx, |domain| async move {
            domain.load_more_notifications().await;
        });
    }

    pub fn load_newer_notifications_action(cx: &mut App) {
        Self::with_domain(cx, |domain| async move {
            domain.load_newer_notifications().await;
        });
    }

    pub fn mark_notification_read_action(notification_id: String, cx: &mut App) {
        Self::with_domain(cx, move |domain| async move {
            domain.mark_notification_read(&notification_id).await;
        });
    }

    pub fn mark_all_notifications_read_action(cx: &mut App) {
        Self::with_domain(cx, |domain| async move {
            domain.mark_all_notifications_read().await;
        });
    }

    pub fn delete_notification_action(notification_id: String, cx: &mut App) {
        Self::with_domain(cx, move |domain| async move {
            domain.delete_notification(&notification_id).await;
        });
    }

    pub fn clear_unread_badge_action(cx: &mut App) {
        Self::with_domain(cx, |domain| async move {
            domain.clear_unread_badge().await;
        });
    }

    pub fn sync_unread_badge_action(count: i64, cx: &mut App) {
        Self::with_domain(cx, move |domain| async move {
            domain.sync_unread_badge(count).await;
        });
    }

    pub fn adjust_unread_badge_action(by: i64, cx: &mut App) {
        Self::with_domain(cx, move |domain| async move {
            domain.adjust_unread_badge(by).await;
        });
    }

    fn device_token_platform() -> Option<register_device_token::Platform> {
        let platform = RiseReleaseNotesDeviceContext::platform();
        register_device_token::Platform::ALL_CASES
            .iter()
            .copied()
            .find(|candidate| candidate.raw_value() == platform)
    }

    pub fn register_device_token_action(
        user_id: String,
        device_token: String,
        ownership_generation: i64,
        cx: &mut App,
    ) -> Task<anyhow::Result<()>> {
        let domain = RiseNotifyActionExecutor::require_domain(cx);
        cx.spawn(async move |cx: &mut AsyncApp| -> anyhow::Result<()> {
            let domain = domain.await?;
            if domain.account_id != user_id
                || !SaiAccountCacheScope::is_current_account_epoch(Some(&user_id), Some(&domain.account_epoch))
            {
                return Err(CancellationError.into());
            }
            let Some(platform) = Self::device_token_platform() else {
                anyhow::bail!("canon has no device token platform for this client");
            };
            let environment = RiseAPNSRegistrationTarget::environment();
            let body = rise_canon::rise_request::notify::RegisterDeviceToken {
                apns_environment: register_device_token::ApnsEnvironment::ALL_CASES
                    .iter()
                    .copied()
                    .find(|candidate| candidate.raw_value() == environment),
                bundle_id: Some(RiseAPNSRegistrationTarget::bundle_id()),
                device_token,
                ownership_generation: Some(ownership_generation),
                platform,
            };
            let endpoint = RiseNotifyEndpoint::canonical(
                rise_canon::rise_wire_methods::notify::REGISTER_DEVICE_TOKEN,
                RiseEndpointKind::Mutation,
            );
            let metadata = RiseWireRequestMetadata {
                auth_required: true,
                ..RiseWireRequestMetadata::default()
            };
            let _: rise_canon::rise_response::notify::RegisterDeviceToken = domain
                .engine
                .call_descriptor_response(&endpoint.descriptor(), &body, None, metadata)
                .await?;
            cx.update(|cx| {
                Sai::log("Notify", || "device token registered", SaiLogKind::Network, cx);
            });
            Ok(())
        })
    }
}
