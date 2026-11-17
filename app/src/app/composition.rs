use std::sync::{Arc, OnceLock};

use gpui::App;
use rise_engine::rise_core::rise_secure_key_store::RiseKeychainKeyStore;
use rise_platform::app_info::{AppIdentity, identity_for};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::notifications::NotificationService;
use rise_platform::{AppPaths, InMemorySecureStore, KeyringSecureStore, SecureStore};
use tokio::runtime::Runtime;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::desktop_platform::app_menu::install_app_menu;
use crate::core::desktop_platform::av_audio_session_route::AVAudioSessionRouteBridge;
use crate::core::desktop_platform::rise_app_lifecycle::RiseAppLifecycle;
use crate::core::desktop_platform::rise_app_quit_drain::install_system_requests;
use crate::core::engine_bridge::network_transport_config::watch_network_changes;
use crate::core::lib::sai::core::websocket::sai_ws_deferred_outbound_coordinator::SaiWsDeferredDiskProtection;
use crate::core::push_notifications::app_delegate::AppDelegate;
use crate::core::push_notifications::desktop_link_routing;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::modules::auth::services::keychain_store::KeychainStore;
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;

pub fn install_storage() {
    let environment = AppEnvironment::compiled();

    match AppPaths::resolve(environment.canon()) {
        Ok(paths) => {
            if let Err(error) = paths.create_all() {
                tracing::error!(target: "riseonly", "the app directories could not be created: {error}");
            }
            UserDefaults::install(&paths);
            SaiWsDeferredDiskProtection::install(&paths);
            RiseEngineAppRuntime::install(&paths);
        }
        Err(error) => {
            tracing::error!(target: "riseonly", "the app directories are unknown: {error}");
        }
    }

    let secrets: Arc<dyn SecureStore> = match identity_for(environment.as_str()) {
        Some(identity) => usable_secure_store(os_secure_store(identity)),
        None => {
            RiseEngineAppRuntime::use_launch_scoped_directory();
            Arc::new(InMemorySecureStore::new())
        }
    };
    RiseKeychainKeyStore::install(Arc::clone(&secrets));
    KeychainStore::install(secrets);
}

fn os_secure_store(identity: &AppIdentity) -> Arc<dyn SecureStore> {
    #[cfg(feature = "dev-drive")]
    {
        if crate::app::dev_drive::DevDrive::stealth_from_launch() {
            if let Some(store) = rise_platform::keyring_store::FileSecureStore::in_home(identity) {
                return Arc::new(store);
            }
        }
    }
    Arc::new(KeyringSecureStore::for_identity(identity))
}

/// a session can run without a keychain eg no secret service on linux
/// tokens then stay in memory until quit, whatever the os store holds is left alone
fn usable_secure_store(os_store: Arc<dyn SecureStore>) -> Arc<dyn SecureStore> {
    match os_store.availability() {
        Ok(()) => os_store,
        Err(reason) => {
            tracing::warn!(
                target: "riseonly",
                "{reason}; sessions will not survive a restart"
            );
            RiseEngineAppRuntime::use_launch_scoped_directory();
            Arc::new(InMemorySecureStore::new())
        }
    }
}

pub fn install(cx: &mut App) {
    RiseAppLifecycle::install(cx);
    install_system_requests(cx);
    install_app_menu(cx);
    AVAudioSessionRouteBridge::install(cx);
    EditorImageBake::install(cx);
    watch_network_changes();
    desktop_link_routing::install(cx);
}

pub fn did_finish_launching(cx: &mut App) {
    let Some(identity) = identity_for(AppEnvironment::compiled().as_str()) else {
        tracing::error!(target: "riseonly", "this build names no app identity; the launch hooks did not run");
        return;
    };
    AppDelegate::did_finish_launching(identity, spawn_notification_service, cx);
}

fn spawn_notification_service(service: NotificationService) {
    static RUNTIME: OnceLock<std::io::Result<Runtime>> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("rise-notifications")
            .enable_all()
            .build()
    });
    match runtime {
        Ok(runtime) => {
            runtime.spawn(service);
        }
        Err(error) => {
            tracing::error!(target: "riseonly", "the notification service could not start: {error}");
        }
    }
}

#[cfg(test)]
#[path = "composition_tests.rs"]
mod tests;
