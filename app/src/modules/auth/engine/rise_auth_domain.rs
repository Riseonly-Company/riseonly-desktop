use std::path::Path;
use std::sync::Arc;

use rise_platform::{HostOs, SecureStore};
use tokio::sync::watch;

use crate::core::engine_bridge::{EngineBridge, SocketCredential};

use super::core::rise_auth_credential_store::AuthCredentialStore;
use super::core::rise_auth_transport::LiveAuthTransport;
use super::rise_auth_presentation::AuthSnapshot;
use super::rise_auth_repository::{
    AuthCommand, AuthRepository, AuthRepositoryConfig, TAG_CHECK_DEBOUNCE,
};

pub struct RiseAuthDomain {
    repository: AuthRepository,
}

impl RiseAuthDomain {
    pub fn open(
        bridge: &EngineBridge,
        data_directory: &Path,
        secrets: Arc<dyn SecureStore>,
    ) -> Self {
        let reauthenticate: Arc<dyn Fn(SocketCredential) + Send + Sync> =
            Arc::new(bridge.authenticator());

        let transport = Arc::new(LiveAuthTransport::new(
            Arc::clone(bridge.wire()),
            Arc::clone(bridge.http()),
            reauthenticate,
        ));

        let repository = AuthRepository::spawn(
            &bridge.handle(),
            AuthRepositoryConfig {
                transport,
                credentials: Arc::new(AuthCredentialStore::new(data_directory)),
                secrets,
                host: HostOs::current(),
                locale: rise_platform::device_locale::current(),
                tag_check_debounce: TAG_CHECK_DEBOUNCE,
                now_ms: Arc::new(|| {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|since| since.as_millis() as i64)
                        .unwrap_or_default()
                }),
            },
        );

        Self { repository }
    }

    pub fn snapshot(&self) -> Arc<AuthSnapshot> {
        self.repository.snapshot()
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<AuthSnapshot>> {
        self.repository.subscribe()
    }

    pub fn dispatch(&self, command: AuthCommand) {
        self.repository.dispatch(command);
    }

    #[cfg(test)]
    pub fn for_test(
        runtime: &tokio::runtime::Handle,
        transport: Arc<dyn super::core::rise_auth_transport::AuthTransport>,
        credentials: Arc<AuthCredentialStore>,
        secrets: Arc<dyn SecureStore>,
    ) -> Self {
        Self {
            repository: AuthRepository::spawn(
                runtime,
                AuthRepositoryConfig {
                    transport,
                    credentials,
                    secrets,
                    host: HostOs::MacOs,
                    locale: rise_platform::DeviceLocale::default(),
                    tag_check_debounce: std::time::Duration::ZERO,
                    now_ms: Arc::new(|| 0),
                },
            ),
        }
    }
}
