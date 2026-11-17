use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AsyncApp};
use rise_core::rise_app_events::{RiseAppEvents, UIApplication};
use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use rise_platform::app_info::AppIdentity;
use rise_platform::notifications::NotificationService;

use crate::core::diagnostics::rise_diagnostics::RiseDiagnostics;
use crate::core::push_notifications::desktop_notification_delivery::DesktopNotificationDelivery;
use crate::core::push_notifications::push_notification_manager::PushNotificationManager;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::map::recorder::rise_map_recorder::RiseMapRecorder;
use crate::modules::rtc::media::rise_rtc_live_kit_audio_coordinator::RiseRTCLiveKitAudioCoordinator;
use crate::modules::rtc::system::rise_rtc_call_system_coordinator::RiseRTCCallSystemCoordinator;

pub enum AppDelegate {}

impl AppDelegate {
    pub fn did_finish_launching(
        identity: &AppIdentity,
        spawn_service: impl FnOnce(NotificationService),
        cx: &mut App,
    ) {
        DesktopNotificationDelivery::start(identity, spawn_service, cx);
        PushNotificationManager::shared(cx).update(cx, |manager, cx| manager.setup(cx));

        RiseRTCLiveKitAudioCoordinator::shared(cx)
            .update(cx, |coordinator, cx| coordinator.install_system_bridge(cx));
        RiseRTCCallSystemCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.start(cx));

        RiseDiagnostics::shared().install();

        AuthServicesStore::shared(cx);
        RiseMapRecorder::shared(cx).update(cx, |recorder, cx| recorder.resume_after_launch(cx));

        Self::observe_lifecycle(cx);

        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Lifecycle, || "app launched".to_owned());
    }

    fn observe_lifecycle(cx: &mut App) {
        let (sender, mut receiver) = mpsc::unbounded::<()>();
        RiseAppEvents::default_center()
            .observe(UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION, move |_| {
                let _ = sender.unbounded_send(());
            })
            .detach();
        cx.spawn(async move |cx: &mut AsyncApp| {
            while receiver.next().await.is_some() {
                cx.update(Self::application_did_enter_background);
            }
        })
        .detach();
        cx.on_app_quit(|cx| {
            Self::application_will_terminate(cx);
            async {}
        })
        .detach();
    }

    fn application_did_enter_background(cx: &mut App) {
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Lifecycle, || {
            "app entered background".to_owned()
        });
        Self::checkpoint_pending_chat_caches(cx);
    }

    fn application_will_terminate(cx: &mut App) {
        Self::checkpoint_pending_chat_caches(cx);
    }

    fn checkpoint_pending_chat_caches(cx: &mut App) {
        MessageInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.checkpoint_pending_chat_caches(cx));
    }
}
