use std::path::Path;
use std::sync::Arc;

use rise_platform::{HostOs, SecureStore};
use tokio::sync::watch;

use crate::core::engine_bridge::{EngineBridge, SocketCredential};

use super::core::rise_auth_credential_store::AuthCredentialStore;
use super::core::rise_auth_transport::LiveAuthTransport;
use super::rise_auth_presentation::AuthSnapshot;
use super::rise_auth_repository::{AuthCommand, AuthRepository, AuthRepositoryConfig};

/// The composition root for auth-service.
///
/// Process-scoped, unlike every other domain that will follow it, and for one
/// reason: auth is what decides which account there is. It owns the account
/// registry, the credential store and the token lifecycle, and it is what tells
/// the socket who it is connected as.
///
/// It is not an `AccountScope`. A domain that lived inside the account scope
/// could not survive the sign-out that drops it, and there would be nothing left
/// to draw the sign-in screen.
pub struct RiseAuthDomain {
    repository: AuthRepository,
}

impl RiseAuthDomain {
    pub fn open(
        bridge: &EngineBridge,
        data_directory: &Path,
        secrets: Arc<dyn SecureStore>,
    ) -> Self {
        // The authenticator rather than the bridge: the repository must be able
        // to re-handshake without holding the whole transport alive.
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

    /// A domain over a supplied transport, for tests that drive the screens.
    ///
    /// The screens' own rules — which step follows which, what a keystroke
    /// clears — are reachable only through a real `App`, and building one of
    /// those must not also open a socket.
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
                    now_ms: Arc::new(|| 0),
                },
            ),
        }
    }
}
