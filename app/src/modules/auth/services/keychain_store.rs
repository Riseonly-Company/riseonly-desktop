use std::sync::{Arc, OnceLock};

use rise_platform::{
    AUTH_SERVICE, InMemorySecureStore, SecretAccessibility, SecretKey, SecureStore,
    SecureStoreError,
};

/// auth items of the os credential store (keychain, credential manager or secret service)
/// service riseonly.auth, account is the key every call blocks so run off the main thread
pub enum KeychainStore {}

#[derive(Debug, thiserror::Error)]
pub enum KeychainStoreKeychainError {
    #[error("keychain: {0}")]
    UnexpectedStatus(SecureStoreError),
}

static STORE: OnceLock<Arc<dyn SecureStore>> = OnceLock::new();

impl KeychainStore {
    const SERVICE: &'static str = AUTH_SERVICE;

    pub fn install(store: Arc<dyn SecureStore>) -> bool {
        STORE.set(store).is_ok()
    }

    fn store() -> &'static Arc<dyn SecureStore> {
        STORE.get_or_init(|| {
            tracing::warn!(
                target: "riseonly.auth",
                "the auth keychain was used before install; items will not survive a restart"
            );
            Arc::new(InMemorySecureStore::new())
        })
    }

    fn query(key: &str) -> SecretKey {
        SecretKey::new(Self::SERVICE, key)
    }

    pub fn set_data(data: &[u8], for_key: &str) -> Result<(), KeychainStoreKeychainError> {
        Self::store()
            .set(
                &Self::query(for_key),
                data,
                SecretAccessibility::AfterFirstUnlock,
            )
            .map_err(KeychainStoreKeychainError::UnexpectedStatus)
    }

    /// device bound material that mustnt migrate through backups
    /// auth credentials keep set_data, this is for installation scoped secrets
    pub fn set_this_device_only_data(
        data: &[u8],
        for_key: &str,
    ) -> Result<(), KeychainStoreKeychainError> {
        Self::store()
            .set(
                &Self::query(for_key),
                data,
                SecretAccessibility::AfterFirstUnlockThisDeviceOnly,
            )
            .map_err(KeychainStoreKeychainError::UnexpectedStatus)
    }

    /// one item, a missing item or a failed read is none
    pub fn data(for_key: &str) -> Option<Vec<u8>> {
        Self::store().get(&Self::query(for_key)).ok().flatten()
    }

    /// removes the item, a failure is not the caller concern
    pub fn remove(for_key: &str) {
        if let Err(error) = Self::store().delete(&Self::query(for_key)) {
            tracing::debug!(target: "riseonly.auth", "keychain remove failed: {error}");
        }
    }
}
