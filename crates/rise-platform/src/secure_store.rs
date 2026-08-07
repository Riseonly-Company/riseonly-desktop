use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecureStoreError {
    #[error("no secret store is available on this system")]
    Unavailable,
    #[error("the entry was not found")]
    NotFound,
    #[error("the secret store refused access")]
    AccessDenied,
    #[error("secret store: {0}")]
    Backend(String),
}

/// Where the database key and the session token live.
///
/// Two separate concerns behind one trait: credentials that should survive a
/// migration to a new machine, and the installation identity that must not. The
/// reference draws the same line with two Keychain accessibility classes, and
/// the distinction matters — copying an installation identity to a second
/// machine makes two clients claim to be the same device.
pub trait SecureStore: Send + Sync {
    fn set(&self, key: &SecretKey, value: &[u8]) -> Result<(), SecureStoreError>;
    fn get(&self, key: &SecretKey) -> Result<Option<Vec<u8>>, SecureStoreError>;
    fn delete(&self, key: &SecretKey) -> Result<(), SecureStoreError>;
    fn is_available(&self) -> bool;
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SecretKey {
    pub account: String,
    pub purpose: SecretPurpose,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SecretPurpose {
    /// The AES key for this account's store. Losing it loses the local cache,
    /// never the account itself, so it is safe to regenerate.
    DatabaseKey,
    /// Refresh credentials. Migrating these to a new machine is expected.
    AuthCredentials,
    /// Identifies this installation. Must never be copied anywhere.
    InstallationIdentity,
}

impl SecretKey {
    pub fn database_key(account: impl Into<String>) -> Self {
        Self {
            account: account.into(),
            purpose: SecretPurpose::DatabaseKey,
        }
    }

    pub fn auth_credentials(account: impl Into<String>) -> Self {
        Self {
            account: account.into(),
            purpose: SecretPurpose::AuthCredentials,
        }
    }

    pub fn installation_identity() -> Self {
        Self {
            account: "installation".into(),
            purpose: SecretPurpose::InstallationIdentity,
        }
    }

    /// The name the OS store sees. Namespaced by purpose so a database key and
    /// a token for the same account cannot collide.
    pub fn entry_name(&self) -> String {
        let purpose = match self.purpose {
            SecretPurpose::DatabaseKey => "db",
            SecretPurpose::AuthCredentials => "auth",
            SecretPurpose::InstallationIdentity => "install",
        };
        format!("{purpose}.{}", self.account)
    }

    pub fn is_machine_bound(&self) -> bool {
        matches!(self.purpose, SecretPurpose::InstallationIdentity)
    }
}

pub const SERVICE_NAME: &str = "net.riseonly.desktop";

/// Used by tests and by a Linux session with no secret service running.
///
/// It is deliberately not a silent fallback: a caller that ends up here must
/// decide whether to degrade or to refuse, because an in-memory store loses the
/// database key on exit and the next launch cannot read its own cache.
#[derive(Default)]
pub struct InMemorySecureStore {
    entries: parking_lot::Mutex<std::collections::HashMap<String, Vec<u8>>>,
}

impl InMemorySecureStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecureStore for InMemorySecureStore {
    fn set(&self, key: &SecretKey, value: &[u8]) -> Result<(), SecureStoreError> {
        self.entries.lock().insert(key.entry_name(), value.to_vec());
        Ok(())
    }

    fn get(&self, key: &SecretKey) -> Result<Option<Vec<u8>>, SecureStoreError> {
        Ok(self.entries.lock().get(&key.entry_name()).cloned())
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecureStoreError> {
        self.entries.lock().remove(&key.entry_name());
        Ok(())
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_purpose_gets_its_own_entry_for_the_same_account() {
        let database = SecretKey::database_key("u1");
        let auth = SecretKey::auth_credentials("u1");

        assert_ne!(
            database.entry_name(),
            auth.entry_name(),
            "a token and a database key for one account must not collide"
        );
    }

    #[test]
    fn different_accounts_get_different_entries() {
        assert_ne!(
            SecretKey::database_key("u1").entry_name(),
            SecretKey::database_key("u2").entry_name()
        );
    }

    #[test]
    fn only_the_installation_identity_is_machine_bound() {
        assert!(SecretKey::installation_identity().is_machine_bound());
        assert!(!SecretKey::database_key("u1").is_machine_bound());
        assert!(!SecretKey::auth_credentials("u1").is_machine_bound());
    }

    #[test]
    fn a_stored_secret_reads_back() {
        let store = InMemorySecureStore::new();
        let key = SecretKey::database_key("u1");

        store.set(&key, b"secret").unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(b"secret".to_vec()));
    }

    #[test]
    fn a_missing_secret_reads_as_none_rather_than_an_error() {
        let store = InMemorySecureStore::new();
        assert_eq!(store.get(&SecretKey::database_key("absent")).unwrap(), None);
    }

    #[test]
    fn deleting_is_idempotent() {
        let store = InMemorySecureStore::new();
        let key = SecretKey::auth_credentials("u1");

        store.set(&key, b"token").unwrap();
        store.delete(&key).unwrap();
        store.delete(&key).unwrap();

        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn one_accounts_secret_is_not_readable_under_another() {
        let store = InMemorySecureStore::new();
        store.set(&SecretKey::database_key("u1"), b"mine").unwrap();

        assert_eq!(store.get(&SecretKey::database_key("u2")).unwrap(), None);
    }

    #[test]
    fn overwriting_replaces_rather_than_appends() {
        let store = InMemorySecureStore::new();
        let key = SecretKey::database_key("u1");

        store.set(&key, b"first").unwrap();
        store.set(&key, b"second").unwrap();

        assert_eq!(store.get(&key).unwrap(), Some(b"second".to_vec()));
    }
}
