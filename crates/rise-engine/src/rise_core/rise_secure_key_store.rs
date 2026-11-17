use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use rand::TryRngCore;
use rand::rngs::OsRng;
use rise_platform::secure_store::{
    ENGINE_SERVICE, InMemorySecureStore, SecretAccessibility, SecretKey, SecureStore,
    SecureStoreError,
};

use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

const ENGINE_KEY_BYTE_COUNT: usize = 32;

/// boxed future of the key store so the trait stays object safe
pub type RiseEngineKeyStoreFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, RiseEngineKeyStoreError>> + Send + 'a>>;

pub trait RiseEngineKeyStore: Send + Sync {
    fn key<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, Option<Vec<u8>>>;
    fn create_key_if_absent<'a>(
        &'a self,
        account_id: &'a str,
    ) -> RiseEngineKeyStoreFuture<'a, Vec<u8>>;
    fn delete_key<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, ()>;
}

/// status code of a failed secret store call
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseKeychainStatus {
    Unavailable,
    NotFound,
    AccessDenied,
    DuplicateItem,
    InvalidAccount(String),
    TooLarge { size: usize, limit: usize },
    Backend(String),
    Interrupted,
}

impl From<SecureStoreError> for RiseKeychainStatus {
    fn from(error: SecureStoreError) -> Self {
        match error {
            SecureStoreError::Unavailable => Self::Unavailable,
            SecureStoreError::NotFound => Self::NotFound,
            SecureStoreError::AccessDenied => Self::AccessDenied,
            SecureStoreError::DuplicateItem => Self::DuplicateItem,
            SecureStoreError::InvalidAccount(account) => Self::InvalidAccount(account),
            SecureStoreError::TooLarge { size, limit } => Self::TooLarge { size, limit },
            SecureStoreError::Backend(message) => Self::Backend(message),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseEngineKeyStoreError {
    #[error("the secret store failed: {0:?}")]
    Keychain(RiseKeychainStatus),
    #[error("the stored engine key is not 32 bytes")]
    InvalidStoredKey,
    #[error("the system random source failed: {0}")]
    RandomGeneration(String),
}

impl From<SecureStoreError> for RiseEngineKeyStoreError {
    fn from(error: SecureStoreError) -> Self {
        Self::Keychain(error.into())
    }
}

static SHARED: OnceLock<RiseKeychainKeyStore> = OnceLock::new();

/// engine key of every account in the os secret store, service net.riseonly.engine
/// never synced and bound to this install
pub struct RiseKeychainKeyStore {
    store: Arc<dyn SecureStore>,
    service: String,
}

impl RiseKeychainKeyStore {
    pub fn install(store: Arc<dyn SecureStore>) -> bool {
        SHARED.set(Self::new(store)).is_ok()
    }

    /// the shared key store, before install it falls back to an in memory one
    pub fn shared() -> &'static RiseKeychainKeyStore {
        SHARED.get_or_init(|| {
            tracing::warn!(
                target: "riseonly.engine",
                "the engine key store was used before install; keys will not survive a restart"
            );
            Self::new(Arc::new(InMemorySecureStore::new()))
        })
    }

    pub fn new(store: Arc<dyn SecureStore>) -> Self {
        Self::with_service(store, ENGINE_SERVICE)
    }

    pub fn with_service(store: Arc<dyn SecureStore>, service: impl Into<String>) -> Self {
        Self {
            store,
            service: service.into(),
        }
    }

    pub async fn key(
        &self,
        account_id: &str,
    ) -> Result<Option<Vec<u8>>, RiseEngineKeyStoreError> {
        let base_query = self.base_query(account_id);
        let stored = self.blocking(move |store| store.get(&base_query)).await??;
        let Some(data) = stored else {
            return Ok(None);
        };
        if data.len() != ENGINE_KEY_BYTE_COUNT {
            return Err(RiseEngineKeyStoreError::InvalidStoredKey);
        }
        Ok(Some(data))
    }

    pub async fn create_key_if_absent(
        &self,
        account_id: &str,
    ) -> Result<Vec<u8>, RiseEngineKeyStoreError> {
        if let Some(existing) = self.key(account_id).await? {
            return Ok(existing);
        }
        let mut bytes = vec![0u8; ENGINE_KEY_BYTE_COUNT];
        OsRng
            .try_fill_bytes(&mut bytes)
            .map_err(|error| RiseEngineKeyStoreError::RandomGeneration(error.to_string()))?;

        let add = self.base_query(account_id);
        let value = bytes.clone();
        let add_status = self
            .blocking(move |store| {
                store.add(&add, &value, SecretAccessibility::AfterFirstUnlockThisDeviceOnly)
            })
            .await?;
        match add_status {
            Ok(()) => Ok(bytes),
            Err(SecureStoreError::DuplicateItem) => match self.key(account_id).await? {
                Some(winner) => Ok(winner),
                None => Err(RiseEngineKeyStoreError::Keychain(
                    RiseKeychainStatus::DuplicateItem,
                )),
            },
            Err(error) => Err(error.into()),
        }
    }

    pub async fn delete_key(&self, account_id: &str) -> Result<(), RiseEngineKeyStoreError> {
        let base_query = self.base_query(account_id);
        self.blocking(move |store| store.delete(&base_query)).await??;
        Ok(())
    }

    fn base_query(&self, account_id: &str) -> SecretKey {
        SecretKey::new(self.service.as_str(), account_id)
    }

    async fn blocking<R, F>(&self, work: F) -> Result<R, RiseEngineKeyStoreError>
    where
        F: FnOnce(&dyn SecureStore) -> R + Send + 'static,
        R: Send + 'static,
    {
        let store = Arc::clone(&self.store);
        RiseAccountRuntime::current()
            .spawn_blocking(move || work(store.as_ref()))
            .await
            .map_err(|_| RiseEngineKeyStoreError::Keychain(RiseKeychainStatus::Interrupted))
    }
}

impl RiseEngineKeyStore for RiseKeychainKeyStore {
    fn key<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, Option<Vec<u8>>> {
        Box::pin(RiseKeychainKeyStore::key(self, account_id))
    }

    fn create_key_if_absent<'a>(
        &'a self,
        account_id: &'a str,
    ) -> RiseEngineKeyStoreFuture<'a, Vec<u8>> {
        Box::pin(RiseKeychainKeyStore::create_key_if_absent(self, account_id))
    }

    fn delete_key<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, ()> {
        Box::pin(RiseKeychainKeyStore::delete_key(self, account_id))
    }
}
