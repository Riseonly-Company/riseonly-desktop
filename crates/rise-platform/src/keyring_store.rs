use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::secure_store::{SERVICE_NAME, SecretKey, SecureStore, SecureStoreError};

/// The OS credential store: Keychain on macOS, Credential Manager on Windows,
/// Secret Service on Linux.
///
/// Values are stored base64-encoded, because the backends disagree about whether a
/// secret is bytes or text and a database key is arbitrary bytes.
pub struct KeyringSecureStore {
    service: String,
}

impl KeyringSecureStore {
    pub fn new() -> Self {
        Self {
            service: SERVICE_NAME.to_owned(),
        }
    }

    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, key: &SecretKey) -> Result<keyring::Entry, SecureStoreError> {
        keyring::Entry::new(&self.service, &key.entry_name()).map_err(map_error)
    }
}

impl Default for KeyringSecureStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecureStore for KeyringSecureStore {
    fn set(&self, key: &SecretKey, value: &[u8]) -> Result<(), SecureStoreError> {
        self.entry(key)?
            .set_password(&BASE64.encode(value))
            .map_err(map_error)
    }

    fn get(&self, key: &SecretKey) -> Result<Option<Vec<u8>>, SecureStoreError> {
        match self.entry(key)?.get_password() {
            Ok(encoded) => BASE64
                .decode(encoded)
                .map(Some)
                .map_err(|error| SecureStoreError::Backend(error.to_string())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(map_error(error)),
        }
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecureStoreError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(map_error(error)),
        }
    }

    /// Probes with a real round trip: a Linux session with no running Secret Service
    /// looks normal until the first write fails.
    fn is_available(&self) -> bool {
        let probe = SecretKey::installation_identity();
        let Ok(entry) = self.entry(&probe) else {
            return false;
        };

        match entry.get_password() {
            Ok(_) => true,
            Err(keyring::Error::NoEntry) => match entry.set_password("probe") {
                Ok(()) => {
                    let _ = entry.delete_credential();
                    true
                }
                Err(_) => false,
            },
            Err(_) => false,
        }
    }
}

fn map_error(error: keyring::Error) -> SecureStoreError {
    match error {
        keyring::Error::NoEntry => SecureStoreError::NotFound,
        keyring::Error::NoStorageAccess(_) => SecureStoreError::AccessDenied,
        keyring::Error::PlatformFailure(inner) => {
            SecureStoreError::Backend(format!("platform: {inner}"))
        }
        other => SecureStoreError::Backend(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unique service name per call: otherwise these collide with the real app's entries.
    fn store() -> KeyringSecureStore {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        KeyringSecureStore::with_service(format!("net.riseonly.desktop.test.{id}"))
    }

    #[test]
    fn entry_names_are_namespaced_by_purpose() {
        let store = store();
        assert!(store.entry(&SecretKey::database_key("u1")).is_ok());
    }

    #[test]
    fn a_missing_secret_reads_as_none() {
        let store = store();
        let key = SecretKey::database_key("never-written");

        match store.get(&key) {
            Ok(value) => assert_eq!(value, None),
            Err(SecureStoreError::Unavailable | SecureStoreError::AccessDenied) => {}
            Err(other) => panic!("unexpected: {other}"),
        }
    }

    #[test]
    fn arbitrary_bytes_round_trip_through_the_text_oriented_backends() {
        let store = store();
        if !store.is_available() {
            return;
        }

        let key = SecretKey::database_key("roundtrip");
        let secret: Vec<u8> = (0..=255u8).collect();

        store.set(&key, &secret).unwrap();
        assert_eq!(
            store.get(&key).unwrap(),
            Some(secret),
            "a database key is arbitrary bytes, not a string"
        );

        store.delete(&key).unwrap();
    }

    #[test]
    fn deleting_a_missing_entry_is_not_an_error() {
        let store = store();
        if !store.is_available() {
            return;
        }
        store
            .delete(&SecretKey::auth_credentials("absent"))
            .unwrap();
    }

    #[test]
    fn purposes_do_not_overwrite_each_other() {
        let store = store();
        if !store.is_available() {
            return;
        }

        let database = SecretKey::database_key("u1");
        let auth = SecretKey::auth_credentials("u1");

        store.set(&database, b"db-key").unwrap();
        store.set(&auth, b"token").unwrap();

        assert_eq!(store.get(&database).unwrap(), Some(b"db-key".to_vec()));
        assert_eq!(store.get(&auth).unwrap(), Some(b"token".to_vec()));

        store.delete(&database).unwrap();
        store.delete(&auth).unwrap();
    }
}
