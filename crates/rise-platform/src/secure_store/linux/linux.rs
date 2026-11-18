use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};

use std::sync::Arc;

use crate::app_info::AppIdentity;
use crate::secure_store::{
    SecretAccessibility, SecretKey, SecretStoreUnavailable, SecureStore, SecureStoreError,
};

const DIRECTORY: &str = "secrets";
const MAGIC: [u8; 4] = *b"RSF1";
const KEY_SALT: &[u8] = b"riseonly secret file store v1";
const MACHINE_ID_PATHS: [&str; 2] = ["/etc/machine-id", "/var/lib/dbus/machine-id"];
const PARTIAL_SUFFIX: &str = ".partial";
const DIRECTORY_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

/// sealed secret files in the app data directory, keyed by machine id and user id
pub(crate) struct SecretFileStore {
    root: PathBuf,
    key: LessSafeKey,
    random: SystemRandom,
}

impl SecretFileStore {
    pub(crate) fn in_data_dir(data_dir: &Path, identity: &AppIdentity) -> Option<Self> {
        let machine = MACHINE_ID_PATHS.iter().find_map(|path| {
            let id = std::fs::read_to_string(path).ok()?;
            let id = id.trim();
            (!id.is_empty()).then(|| id.to_owned())
        })?;
        let user = std::fs::metadata("/proc/self").ok()?.uid();
        Self::with_material(data_dir.join(DIRECTORY), machine.as_bytes(), user, identity.bundle_identifier)
    }

    /// store at root keyed by machine id, user id and purpose
    pub(crate) fn with_material(root: PathBuf, machine: &[u8], user: u32, purpose: &str) -> Option<Self> {
        let mut material = machine.to_vec();
        material.extend_from_slice(&user.to_le_bytes());
        let pseudo_random = hkdf::Salt::new(hkdf::HKDF_SHA256, KEY_SALT).extract(&material);
        let info = [purpose.as_bytes()];
        let okm = pseudo_random.expand(&info, &AES_256_GCM).ok()?;
        Some(Self {
            root,
            key: LessSafeKey::new(UnboundKey::from(okm)),
            random: SystemRandom::new(),
        })
    }

    /// moves every secret into target, a refused one stays, partial and foreign files go
    pub(crate) fn move_into(&self, target: &dyn SecureStore) -> usize {
        let mut moved = 0;
        for (key, path) in self.entries() {
            let Some(key) = key else {
                let _ = std::fs::remove_file(&path);
                continue;
            };
            let Ok(file) = std::fs::read(&path) else {
                continue;
            };
            let Ok(value) = self.open(&key, &file) else {
                let _ = std::fs::remove_file(&path);
                continue;
            };
            if target.set(&key, &value, SecretAccessibility::AfterFirstUnlockThisDeviceOnly).is_err() {
                continue;
            }
            if self.delete(&key).is_ok() {
                moved += 1;
            }
        }
        self.remove_empty_directories();
        moved
    }

    fn entries(&self) -> Vec<(Option<SecretKey>, PathBuf)> {
        let Ok(services) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for service_entry in services.flatten() {
            let Some(service) = decoded_component(&service_entry.file_name()) else {
                continue;
            };
            let Ok(entries) = std::fs::read_dir(service_entry.path()) else {
                continue;
            };
            for entry in entries.flatten() {
                let key = decoded_component(&entry.file_name()).map(|account| SecretKey::new(service.clone(), account));
                found.push((key, entry.path()));
            }
        }
        found
    }

    fn remove_empty_directories(&self) {
        if let Ok(services) = std::fs::read_dir(&self.root) {
            for service in services.flatten() {
                let _ = std::fs::remove_dir(service.path());
            }
        }
        let _ = std::fs::remove_dir(&self.root);
    }

    fn entry_path(&self, key: &SecretKey) -> PathBuf {
        self.root.join(component(&key.service)).join(component(&key.account))
    }

    fn seal(&self, key: &SecretKey, value: &[u8]) -> Result<Vec<u8>, SecureStoreError> {
        let mut nonce = [0u8; NONCE_LEN];
        self.random
            .fill(&mut nonce)
            .map_err(|_| SecureStoreError::Backend("no randomness for a secret file nonce".to_owned()))?;
        let mut sealed = value.to_vec();
        self.key
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(associated(key)), &mut sealed)
            .map_err(|_| SecureStoreError::Backend("a secret file could not be sealed".to_owned()))?;
        let mut file = Vec::with_capacity(MAGIC.len() + NONCE_LEN + sealed.len());
        file.extend_from_slice(&MAGIC);
        file.extend_from_slice(&nonce);
        file.extend_from_slice(&sealed);
        Ok(file)
    }

    fn open(&self, key: &SecretKey, file: &[u8]) -> Result<Vec<u8>, SecureStoreError> {
        let unreadable = || SecureStoreError::Backend("a secret file is not readable on this machine".to_owned());
        let body = file.strip_prefix(&MAGIC).ok_or_else(unreadable)?;
        if body.len() < NONCE_LEN {
            return Err(unreadable());
        }
        let (nonce, sealed) = body.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| unreadable())?;
        let mut sealed = sealed.to_vec();
        let opened = self
            .key
            .open_in_place(nonce, Aad::from(associated(key)), &mut sealed)
            .map_err(|_| unreadable())?;
        Ok(opened.to_vec())
    }

    fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<(), SecureStoreError> {
        let directory = path.parent().ok_or_else(|| file_error(std::io::Error::other("no directory")))?;
        DirBuilder::new().recursive(true).mode(DIRECTORY_MODE).create(directory).map_err(file_error)?;
        for owned in [self.root.as_path(), directory] {
            std::fs::set_permissions(owned, std::fs::Permissions::from_mode(DIRECTORY_MODE)).map_err(file_error)?;
        }
        let mut partial = path.as_os_str().to_owned();
        partial.push(PARTIAL_SUFFIX);
        let partial = PathBuf::from(partial);
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(FILE_MODE)
            .open(&partial)
            .map_err(file_error)?;
        file.set_permissions(std::fs::Permissions::from_mode(FILE_MODE)).map_err(file_error)?;
        file.write_all(bytes).and_then(|()| file.sync_all()).map_err(file_error)?;
        std::fs::rename(&partial, path).map_err(file_error)
    }
}

impl SecureStore for SecretFileStore {
    fn set(&self, key: &SecretKey, value: &[u8], _accessibility: SecretAccessibility) -> Result<(), SecureStoreError> {
        let sealed = self.seal(key, value)?;
        self.write_file(&self.entry_path(key), &sealed)
    }

    fn get(&self, key: &SecretKey) -> Result<Option<Vec<u8>>, SecureStoreError> {
        match std::fs::read(self.entry_path(key)) {
            Ok(file) => self.open(key, &file).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(file_error(error)),
        }
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecureStoreError> {
        match std::fs::remove_file(self.entry_path(key)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(file_error(error)),
        }
    }

    fn is_available(&self) -> bool {
        self.availability().is_ok()
    }

    fn availability(&self) -> Result<(), SecretStoreUnavailable> {
        DirBuilder::new()
            .recursive(true)
            .mode(DIRECTORY_MODE)
            .create(&self.root)
            .map_err(|_| SecretStoreUnavailable::NoService)
    }
}

fn associated(key: &SecretKey) -> Vec<u8> {
    let mut associated = key.service.as_bytes().to_vec();
    associated.push(0);
    associated.extend_from_slice(key.account.as_bytes());
    associated
}

fn component(raw: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw.as_bytes())
}

fn decoded_component(name: &std::ffi::OsStr) -> Option<String> {
    let name = name.to_str()?;
    if name.ends_with(PARTIAL_SUFFIX) {
        return None;
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(name).ok()?;
    String::from_utf8(bytes).ok()
}

fn file_error(error: std::io::Error) -> SecureStoreError {
    SecureStoreError::Backend(format!("secret file: {error}"))
}

pub(crate) fn fallback_secure_store(data_dir: &Path, identity: &AppIdentity) -> Option<Arc<dyn SecureStore>> {
    let store = SecretFileStore::in_data_dir(data_dir, identity)?;
    store.availability().ok()?;
    Some(Arc::new(store))
}

pub(crate) fn move_fallback_secrets(data_dir: &Path, identity: &AppIdentity, target: &dyn SecureStore) -> usize {
    SecretFileStore::in_data_dir(data_dir, identity).map_or(0, |store| store.move_into(target))
}
