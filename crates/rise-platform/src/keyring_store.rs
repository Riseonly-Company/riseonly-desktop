use crate::app_info::AppIdentity;
use crate::credential_persistence::{LocalWrite, write_this_device_only};
use crate::secure_store::{
    EntryBackend, SecretAccessibility, SecretKey, SecretStoreUnavailable, SecureStore,
    SecureStoreError, availability_from_probe, delete_split, namespaced_service, read_split,
    write_split,
};

/// os credential store: keychain, credential manager or secret service
/// secrets are raw bytes split into entries of max 2560 bytes
/// this-device-only entries on windows use local machine persistence, not roaming
pub struct KeyringSecureStore {
    namespace: String,
}

struct ServiceEntries {
    service: String,
    this_device_only: bool,
}

impl KeyringSecureStore {
    pub fn for_identity(identity: &AppIdentity) -> Self {
        Self::with_namespace(identity.bundle_identifier)
    }

    pub fn with_namespace(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
        }
    }

    fn entries(&self, service: &str) -> ServiceEntries {
        ServiceEntries {
            service: namespaced_service(&self.namespace, service),
            this_device_only: false,
        }
    }

    fn entries_for_write(
        &self,
        service: &str,
        accessibility: SecretAccessibility,
    ) -> ServiceEntries {
        ServiceEntries {
            this_device_only: accessibility.is_this_device_only(),
            ..self.entries(service)
        }
    }
}

impl ServiceEntries {
    fn named(&self, name: &str) -> Result<keyring::Entry, SecureStoreError> {
        keyring::Entry::new(&self.service, name).map_err(map_error)
    }
}

impl EntryBackend for ServiceEntries {
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, SecureStoreError> {
        match self.named(name)?.get_secret() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(map_error(error)),
        }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), SecureStoreError> {
        let entry = self.named(name)?;
        if self.this_device_only {
            match write_this_device_only(&self.service, name, bytes)? {
                LocalWrite::Written => return Ok(()),
                LocalWrite::StoreIsLocal => {}
            }
        }
        entry.set_secret(bytes).map_err(map_error)
    }

    fn remove(&self, name: &str) -> Result<(), SecureStoreError> {
        match self.named(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(map_error(error)),
        }
    }
}

impl SecureStore for KeyringSecureStore {
    fn set(
        &self,
        key: &SecretKey,
        value: &[u8],
        accessibility: SecretAccessibility,
    ) -> Result<(), SecureStoreError> {
        write_split(&self.entries_for_write(&key.service, accessibility), key, value)
    }

    fn get(&self, key: &SecretKey) -> Result<Option<Vec<u8>>, SecureStoreError> {
        read_split(&self.entries(&key.service), key)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecureStoreError> {
        delete_split(&self.entries(&key.service), key)
    }

    fn is_available(&self) -> bool {
        self.availability().is_ok()
    }

    fn availability(&self) -> Result<(), SecretStoreUnavailable> {
        availability_from_probe(self.probe())
    }
}

impl KeyringSecureStore {
    fn probe(&self) -> Result<(), SecureStoreError> {
        let entry = self.entries(PROBE_SERVICE).named(PROBE_ACCOUNT)?;
        match entry.get_secret() {
            Ok(_) => Ok(()),
            Err(keyring::Error::NoEntry) => {
                entry.set_secret(b"probe").map_err(map_error)?;
                let _ = entry.delete_credential();
                Ok(())
            }
            Err(error) => Err(map_error(error)),
        }
    }
}

const PROBE_SERVICE: &str = "availability";
const PROBE_ACCOUNT: &str = "probe";

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

/// file based secret store for tester stealth mode, one file per entry
/// rebuilt binaries change signature so the keychain would nag every build
/// never in a product build, files hold secrets in the clear
#[cfg(feature = "dev-drive")]
pub struct FileSecureStore {
    root: std::path::PathBuf,
}

#[cfg(feature = "dev-drive")]
const DEV_DRIVE_SECRETS_DIR: &str = ".riseonly-dev-drive-secrets";

#[cfg(feature = "dev-drive")]
struct FileEntries {
    directory: std::path::PathBuf,
}

#[cfg(feature = "dev-drive")]
impl FileSecureStore {
    pub fn in_home(identity: &AppIdentity) -> Option<Self> {
        let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
        Some(Self::at(
            home.join(DEV_DRIVE_SECRETS_DIR)
                .join(file_component(identity.bundle_identifier)),
        ))
    }

    pub fn at(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn entries(&self, service: &str) -> FileEntries {
        FileEntries {
            directory: self.root.join(file_component(service)),
        }
    }
}

#[cfg(feature = "dev-drive")]
fn file_component(raw: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw.as_bytes())
}

#[cfg(feature = "dev-drive")]
fn file_error(error: std::io::Error) -> SecureStoreError {
    SecureStoreError::Backend(format!("dev-drive secret file: {error}"))
}

#[cfg(feature = "dev-drive")]
impl EntryBackend for FileEntries {
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, SecureStoreError> {
        match std::fs::read(self.directory.join(file_component(name))) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(file_error(error)),
        }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), SecureStoreError> {
        std::fs::create_dir_all(&self.directory).map_err(file_error)?;
        let component = file_component(name);
        let partial = self.directory.join(format!("{component}.partial"));
        std::fs::write(&partial, bytes).map_err(file_error)?;
        std::fs::rename(&partial, self.directory.join(component)).map_err(file_error)
    }

    fn remove(&self, name: &str) -> Result<(), SecureStoreError> {
        match std::fs::remove_file(self.directory.join(file_component(name))) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(file_error(error)),
        }
    }
}

#[cfg(feature = "dev-drive")]
impl SecureStore for FileSecureStore {
    fn set(
        &self,
        key: &SecretKey,
        value: &[u8],
        _accessibility: SecretAccessibility,
    ) -> Result<(), SecureStoreError> {
        write_split(&self.entries(&key.service), key, value)
    }

    fn get(&self, key: &SecretKey) -> Result<Option<Vec<u8>>, SecureStoreError> {
        read_split(&self.entries(&key.service), key)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecureStoreError> {
        delete_split(&self.entries(&key.service), key)
    }

    fn is_available(&self) -> bool {
        self.availability().is_ok()
    }

    fn availability(&self) -> Result<(), SecretStoreUnavailable> {
        std::fs::create_dir_all(&self.root).map_err(|_| SecretStoreUnavailable::NoService)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secure_store::{AUTH_SERVICE, ENGINE_SERVICE};

    const AFTER_FIRST_UNLOCK: SecretAccessibility = SecretAccessibility::AfterFirstUnlock;
    const THIS_DEVICE_ONLY: SecretAccessibility =
        SecretAccessibility::AfterFirstUnlockThisDeviceOnly;

    fn store() -> KeyringSecureStore {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        KeyringSecureStore::with_namespace(format!("net.riseonly.desktop.test.{id}"))
    }

    #[test]
    fn a_missing_secret_reads_as_none() {
        let store = store();
        let key = SecretKey::new(ENGINE_SERVICE, "never-written");

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

        let key = SecretKey::new(ENGINE_SERVICE, "roundtrip");
        let secret: Vec<u8> = (0..=255u8).collect();

        store.set(&key, &secret, THIS_DEVICE_ONLY).unwrap();
        assert_eq!(
            store.get(&key).unwrap(),
            Some(secret),
            "a database key is arbitrary bytes, not a string"
        );

        store.delete(&key).unwrap();
    }

    #[test]
    fn a_secret_larger_than_one_credential_manager_blob_round_trips() {
        let store = store();
        if !store.is_available() {
            return;
        }

        let key = SecretKey::new(AUTH_SERVICE, "large");
        let secret: Vec<u8> = (0..6000u32).map(|i| (i % 251) as u8).collect();

        store.set(&key, &secret, AFTER_FIRST_UNLOCK).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(secret));

        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn deleting_a_missing_entry_is_not_an_error() {
        let store = store();
        if !store.is_available() {
            return;
        }
        store.delete(&SecretKey::new(AUTH_SERVICE, "absent")).unwrap();
    }

    #[test]
    fn services_do_not_overwrite_each_other() {
        let store = store();
        if !store.is_available() {
            return;
        }

        let engine = SecretKey::new(ENGINE_SERVICE, "u1");
        let auth = SecretKey::new(AUTH_SERVICE, "u1");

        store.set(&engine, b"db-key", THIS_DEVICE_ONLY).unwrap();
        store.set(&auth, b"token", AFTER_FIRST_UNLOCK).unwrap();

        assert_eq!(store.get(&engine).unwrap(), Some(b"db-key".to_vec()));
        assert_eq!(store.get(&auth).unwrap(), Some(b"token".to_vec()));

        store.delete(&engine).unwrap();
        store.delete(&auth).unwrap();
    }

    #[test]
    fn add_keeps_the_first_writer() {
        let store = store();
        if !store.is_available() {
            return;
        }

        let key = SecretKey::new(ENGINE_SERVICE, "race");
        store.add(&key, b"winner", THIS_DEVICE_ONLY).unwrap();

        assert!(matches!(
            store.add(&key, b"loser", THIS_DEVICE_ONLY),
            Err(SecureStoreError::DuplicateItem)
        ));
        assert_eq!(store.get(&key).unwrap(), Some(b"winner".to_vec()));

        store.delete(&key).unwrap();
    }

    #[cfg(feature = "dev-drive")]
    fn file_store() -> (FileSecureStore, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rise-dev-drive-secrets-{}-{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        (FileSecureStore::at(&root), root)
    }

    #[cfg(feature = "dev-drive")]
    #[test]
    fn the_dev_drive_file_store_round_trips_a_large_secret_and_forgets_it() {
        let (store, root) = file_store();
        assert!(store.is_available());

        let key = SecretKey::new(AUTH_SERVICE, "tokens/with.separators");
        let secret: Vec<u8> = (0..6000u32).map(|i| (i % 251) as u8).collect();

        store.set(&key, &secret, AFTER_FIRST_UNLOCK).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(secret));
        assert_eq!(
            store.get(&SecretKey::new(ENGINE_SERVICE, "tokens/with.separators")).unwrap(),
            None
        );

        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
        store.delete(&key).unwrap();

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn namespaces_do_not_see_each_other() {
        let dev = store();
        let production = store();
        if !dev.is_available() {
            return;
        }

        let key = SecretKey::new(AUTH_SERVICE, "tokens");
        dev.set(&key, b"dev-token", AFTER_FIRST_UNLOCK).unwrap();

        assert_eq!(production.get(&key).unwrap(), None);
        dev.delete(&key).unwrap();
    }
}
