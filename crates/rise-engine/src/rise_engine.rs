use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use fs4::fs_std::FileExt;
use sha2::{Digest, Sha256};
use thiserror::Error;

use rise_core::{AccountId, RequestIdAllocator};

use crate::rise_store::rise_store_cipher::ValueCipher;
use crate::rise_store::rise_store_concurrency::{StoreConfig, StoreError, StoreHandle};

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("account {0} already has an open engine in this process")]
    AlreadyOpenInProcess(String),
    #[error("another Riseonly process already owns this account's database")]
    OwnedByAnotherProcess,
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub struct EngineConfig {
    pub account: AccountId,
    pub base_directory: Option<PathBuf>,
    pub cipher: Arc<dyn ValueCipher>,
}

/// One account, one engine, one database.
///
/// Opening is refused when the account is already open: an in-process lease
/// guards a second engine here, an advisory file lock guards a second process.
pub struct RiseEngine {
    account: AccountId,
    directory: PathBuf,
    store: StoreHandle,
    requests: Arc<RequestIdAllocator>,
    cipher: Arc<dyn ValueCipher>,
    _lease: AccountLease,
    _file_lock: Option<ProcessLock>,
}

impl RiseEngine {
    pub async fn open(config: EngineConfig) -> Result<Self, EngineError> {
        let lease = AccountLease::claim(config.account.as_str())?;

        let (directory, database_path, file_lock) = match &config.base_directory {
            Some(base) => {
                let directory = base.join(account_directory_name(config.account.as_str()));
                std::fs::create_dir_all(&directory)?;
                let database_path = directory.join("engine.sqlite");
                let lock = ProcessLock::acquire(&directory.join("engine.lock"))?;
                (directory, Some(database_path), Some(lock))
            }
            None => (PathBuf::new(), None, None),
        };

        let store = StoreHandle::open(StoreConfig {
            account_id: config.account.as_str().to_owned(),
            path: database_path,
            cipher: Arc::clone(&config.cipher),
        })?;

        Ok(Self {
            account: config.account,
            directory,
            store,
            requests: Arc::new(RequestIdAllocator::new()),
            cipher: config.cipher,
            _lease: lease,
            _file_lock: file_lock,
        })
    }

    pub fn account(&self) -> &AccountId {
        &self.account
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn store(&self) -> &StoreHandle {
        &self.store
    }

    pub fn requests(&self) -> &Arc<RequestIdAllocator> {
        &self.requests
    }

    pub fn cipher(&self) -> &Arc<dyn ValueCipher> {
        &self.cipher
    }

    /// Drains the store and releases both guards. Await it before opening the
    /// same account again; Drop cannot do this.
    pub async fn close(self) -> Result<(), EngineError> {
        self.store.close().await?;
        Ok(())
    }
}

/// Hashed so an account id never becomes a directory name on disk.
fn account_directory_name(account_id: &str) -> String {
    let digest = Sha256::digest(account_id.as_bytes());
    hex_prefix(&digest, 16)
}

fn hex_prefix(bytes: &[u8], length: usize) -> String {
    bytes
        .iter()
        .take(length)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn registry() -> &'static Mutex<HashSet<String>> {
    static REGISTRY: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashSet::new()))
}

struct AccountLease {
    account_id: String,
}

impl AccountLease {
    fn claim(account_id: &str) -> Result<Self, EngineError> {
        let mut held = registry().lock().unwrap_or_else(|e| e.into_inner());
        if !held.insert(account_id.to_owned()) {
            return Err(EngineError::AlreadyOpenInProcess(account_id.to_owned()));
        }
        Ok(Self {
            account_id: account_id.to_owned(),
        })
    }
}

impl Drop for AccountLease {
    fn drop(&mut self) {
        let mut held = registry().lock().unwrap_or_else(|e| e.into_inner());
        held.remove(&self.account_id);
    }
}

struct ProcessLock {
    file: File,
}

impl ProcessLock {
    fn acquire(path: &Path) -> Result<Self, EngineError> {
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        // Contention is Ok(false), not Err; Err is reserved for a failed syscall.
        let acquired =
            FileExt::try_lock_exclusive(&file).map_err(|_| EngineError::OwnedByAnotherProcess)?;
        if !acquired {
            return Err(EngineError::OwnedByAnotherProcess);
        }

        Ok(Self { file })
    }
}

impl Drop for ProcessLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rise_store::rise_store_cipher::{AesGcmCipher, PlaintextCipher};

    fn account(raw: &str) -> AccountId {
        AccountId::new(raw).unwrap()
    }

    fn memory_config(raw: &str) -> EngineConfig {
        EngineConfig {
            account: account(raw),
            base_directory: None,
            cipher: Arc::new(PlaintextCipher),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rise-engine-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn an_engine_opens_and_closes() {
        let engine = RiseEngine::open(memory_config("u1")).await.unwrap();
        assert_eq!(engine.account().as_str(), "u1");
        engine.close().await.unwrap();
    }

    #[tokio::test]
    async fn one_account_cannot_have_two_engines_in_this_process() {
        let first = RiseEngine::open(memory_config("dup")).await.unwrap();

        let second = RiseEngine::open(memory_config("dup")).await;
        assert!(
            matches!(second, Err(EngineError::AlreadyOpenInProcess(_))),
            "two writers would each hold their own idea of the current revision"
        );

        first.close().await.unwrap();
    }

    #[tokio::test]
    async fn closing_releases_the_lease_for_an_account_switch() {
        let first = RiseEngine::open(memory_config("switch")).await.unwrap();
        first.close().await.unwrap();

        let second = RiseEngine::open(memory_config("switch")).await;
        assert!(second.is_ok(), "a closed engine must free its account");
        second.unwrap().close().await.unwrap();
    }

    #[tokio::test]
    async fn two_accounts_coexist() {
        let a = RiseEngine::open(memory_config("a")).await.unwrap();
        let b = RiseEngine::open(memory_config("b")).await.unwrap();
        a.close().await.unwrap();
        b.close().await.unwrap();
    }

    #[tokio::test]
    async fn each_account_gets_its_own_directory_and_the_id_never_appears_in_the_path() {
        let base = temp_dir("dirs");

        let engine = RiseEngine::open(EngineConfig {
            account: account("user@example.com"),
            base_directory: Some(base.clone()),
            cipher: Arc::new(PlaintextCipher),
        })
        .await
        .unwrap();

        let directory = engine.directory().to_path_buf();
        assert!(directory.starts_with(&base));
        assert!(
            !directory.to_string_lossy().contains("user@example.com"),
            "an account id must not become a directory name"
        );
        assert!(directory.join("engine.sqlite").exists());

        engine.close().await.unwrap();
        std::fs::remove_dir_all(&base).ok();
    }

    #[tokio::test]
    async fn different_accounts_never_share_a_directory() {
        assert_ne!(
            account_directory_name("u1"),
            account_directory_name("u2"),
            "a shared directory would let one account read another's store"
        );
        assert_eq!(account_directory_name("u1"), account_directory_name("u1"));
    }

    #[tokio::test]
    async fn a_second_process_cannot_take_the_same_database() {
        let base = temp_dir("lock");
        let directory = base.join(account_directory_name("locked"));
        std::fs::create_dir_all(&directory).unwrap();
        let lock_path = directory.join("engine.lock");

        let held = ProcessLock::acquire(&lock_path).unwrap();

        let engine = RiseEngine::open(EngineConfig {
            account: account("locked"),
            base_directory: Some(base.clone()),
            cipher: Arc::new(PlaintextCipher),
        })
        .await;

        assert!(
            matches!(engine, Err(EngineError::OwnedByAnotherProcess)),
            "a desktop app can be launched twice; the second must be refused"
        );

        drop(held);
        std::fs::remove_dir_all(&base).ok();
    }

    #[tokio::test]
    async fn the_lock_is_released_when_the_engine_closes() {
        let base = temp_dir("lock-release");

        let engine = RiseEngine::open(EngineConfig {
            account: account("relock"),
            base_directory: Some(base.clone()),
            cipher: Arc::new(PlaintextCipher),
        })
        .await
        .unwrap();
        engine.close().await.unwrap();

        let again = RiseEngine::open(EngineConfig {
            account: account("relock"),
            base_directory: Some(base.clone()),
            cipher: Arc::new(PlaintextCipher),
        })
        .await;

        assert!(again.is_ok(), "closing must free the file lock");
        again.unwrap().close().await.unwrap();
        std::fs::remove_dir_all(&base).ok();
    }

    #[tokio::test]
    async fn a_persisted_store_survives_a_reopen_with_the_same_key() {
        use crate::rise_store::rise_store_models::Namespace;
        use crate::rise_store::rise_store_writes::WriteBatch;

        let base = temp_dir("persist");
        let cipher: Arc<dyn ValueCipher> = Arc::new(AesGcmCipher::new(&[9u8; 32]).unwrap());
        let ns = Namespace::new("test.v1");

        let engine = RiseEngine::open(EngineConfig {
            account: account("persist"),
            base_directory: Some(base.clone()),
            cipher: Arc::clone(&cipher),
        })
        .await
        .unwrap();

        let mut batch = WriteBatch::new();
        batch.put_value(&ns, "survives", b"yes".to_vec());
        engine
            .store()
            .commit(batch, "persist".into(), Arc::clone(&cipher), 0)
            .await
            .unwrap();
        engine.close().await.unwrap();

        let engine = RiseEngine::open(EngineConfig {
            account: account("persist"),
            base_directory: Some(base.clone()),
            cipher: Arc::clone(&cipher),
        })
        .await
        .unwrap();

        let found = engine
            .store()
            .with_connection(move |connection| {
                crate::rise_store::rise_store_reads::read_value(
                    connection,
                    "persist",
                    &AesGcmCipher::new(&[9u8; 32]).unwrap(),
                    &Namespace::new("test.v1"),
                    "survives",
                )
                .unwrap()
                .map(|value| value.plaintext)
            })
            .await
            .unwrap();

        assert_eq!(found, Some(b"yes".to_vec()));
        engine.close().await.unwrap();
        std::fs::remove_dir_all(&base).ok();
    }

    #[tokio::test]
    async fn request_ids_are_unique_within_an_engine() {
        let engine = RiseEngine::open(memory_config("ids")).await.unwrap();
        let allocator = engine.requests();

        let ids: HashSet<u64> = (0..1_000).map(|_| allocator.next().get()).collect();
        assert_eq!(ids.len(), 1_000);

        engine.close().await.unwrap();
    }
}
