use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rise_platform::secure_store::{
    ENGINE_SERVICE, InMemorySecureStore, SecretAccessibility, SecretKey, SecureStore,
    SecureStoreError,
};

use crate::rise_core::rise_secure_key_store::{
    RiseEngineKeyStore, RiseEngineKeyStoreError, RiseKeychainKeyStore,
};

fn engine_key(account_id: &str) -> SecretKey {
    SecretKey::new(ENGINE_SERVICE, account_id)
}

fn store_with_memory() -> (RiseKeychainKeyStore, Arc<InMemorySecureStore>) {
    let memory = Arc::new(InMemorySecureStore::new());
    let store = RiseKeychainKeyStore::new(Arc::clone(&memory) as Arc<dyn SecureStore>);
    (store, memory)
}

#[tokio::test]
async fn a_missing_key_is_none() {
    let (store, _) = store_with_memory();
    assert_eq!(store.key("u1").await, Ok(None));
}

#[tokio::test]
async fn create_key_if_absent_stores_32_bytes_bound_to_this_device() {
    let (store, memory) = store_with_memory();
    let created = store.create_key_if_absent("u1").await.unwrap();
    assert_eq!(created.len(), 32);
    assert_eq!(memory.get(&engine_key("u1")).unwrap(), Some(created.clone()));
    assert_eq!(
        memory.stored_accessibility(&engine_key("u1")),
        Some(SecretAccessibility::AfterFirstUnlockThisDeviceOnly)
    );
    assert_eq!(store.create_key_if_absent("u1").await.unwrap(), created);
    assert_eq!(store.key("u1").await, Ok(Some(created)));
}

#[tokio::test]
async fn accounts_get_different_keys() {
    let (store, _) = store_with_memory();
    let first = store.create_key_if_absent("u1").await.unwrap();
    let second = store.create_key_if_absent("u2").await.unwrap();
    assert_ne!(first, second);
}

#[tokio::test]
async fn a_stored_key_of_the_wrong_length_is_refused() {
    let (store, memory) = store_with_memory();
    memory
        .set(&engine_key("u1"), &[7u8; 16], SecretAccessibility::AfterFirstUnlockThisDeviceOnly)
        .unwrap();
    assert_eq!(store.key("u1").await, Err(RiseEngineKeyStoreError::InvalidStoredKey));
    assert_eq!(
        store.create_key_if_absent("u1").await,
        Err(RiseEngineKeyStoreError::InvalidStoredKey)
    );
}

#[tokio::test]
async fn delete_key_removes_the_key_and_ignores_a_missing_one() {
    let (store, _) = store_with_memory();
    assert_eq!(store.delete_key("u1").await, Ok(()));
    store.create_key_if_absent("u1").await.unwrap();
    assert_eq!(store.delete_key("u1").await, Ok(()));
    assert_eq!(store.key("u1").await, Ok(None));
}

#[tokio::test]
async fn the_service_name_scopes_the_entries() {
    let memory = Arc::new(InMemorySecureStore::new());
    let shared = Arc::clone(&memory) as Arc<dyn SecureStore>;
    let other = RiseKeychainKeyStore::with_service(shared, "net.riseonly.other");
    other.create_key_if_absent("u1").await.unwrap();
    assert_eq!(memory.get(&engine_key("u1")).unwrap(), None);
    assert!(memory.get(&SecretKey::new("net.riseonly.other", "u1")).unwrap().is_some());
}

struct LateWriter {
    inner: InMemorySecureStore,
    hidden_once: AtomicBool,
}

impl SecureStore for LateWriter {
    fn set(
        &self,
        key: &SecretKey,
        value: &[u8],
        accessibility: SecretAccessibility,
    ) -> Result<(), SecureStoreError> {
        self.inner.set(key, value, accessibility)
    }

    fn get(&self, key: &SecretKey) -> Result<Option<Vec<u8>>, SecureStoreError> {
        if !self.hidden_once.swap(true, Ordering::SeqCst) {
            return Ok(None);
        }
        self.inner.get(key)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecureStoreError> {
        self.inner.delete(key)
    }

    fn is_available(&self) -> bool {
        true
    }

    fn add(
        &self,
        key: &SecretKey,
        value: &[u8],
        accessibility: SecretAccessibility,
    ) -> Result<(), SecureStoreError> {
        self.inner.add(key, value, accessibility)
    }
}

#[tokio::test]
async fn a_duplicate_add_returns_the_winning_key() {
    let winner = vec![9u8; 32];
    let inner = InMemorySecureStore::new();
    inner
        .set(&engine_key("u1"), &winner, SecretAccessibility::AfterFirstUnlockThisDeviceOnly)
        .unwrap();
    let racing = LateWriter {
        inner,
        hidden_once: AtomicBool::new(false),
    };
    let store = RiseKeychainKeyStore::new(Arc::new(racing));
    assert_eq!(store.create_key_if_absent("u1").await, Ok(winner));
}

#[tokio::test]
async fn the_trait_object_reaches_the_same_store() {
    let (store, _) = store_with_memory();
    let any_store: &dyn RiseEngineKeyStore = &store;
    let created = any_store.create_key_if_absent("u1").await.unwrap();
    assert_eq!(any_store.key("u1").await, Ok(Some(created)));
    assert_eq!(any_store.delete_key("u1").await, Ok(()));
    assert_eq!(any_store.key("u1").await, Ok(None));
}
