use rise_platform::{SecretAccessibility, SecretKey, SecureStoreError};

use super::*;

struct UnavailableStore;

impl SecureStore for UnavailableStore {
    fn set(
        &self,
        _key: &SecretKey,
        _value: &[u8],
        _accessibility: SecretAccessibility,
    ) -> Result<(), SecureStoreError> {
        Err(SecureStoreError::Backend("platform".to_string()))
    }

    fn get(&self, _key: &SecretKey) -> Result<Option<Vec<u8>>, SecureStoreError> {
        Err(SecureStoreError::Backend("platform".to_string()))
    }

    fn delete(&self, _key: &SecretKey) -> Result<(), SecureStoreError> {
        Err(SecureStoreError::Backend("platform".to_string()))
    }

    fn is_available(&self) -> bool {
        false
    }
}

fn key() -> SecretKey {
    SecretKey::new("riseonly.auth", "account")
}

#[test]
fn saving_tokens_works_when_the_os_store_is_unavailable() {
    let store = usable_secure_store(Arc::new(UnavailableStore), None);

    store
        .set(&key(), b"token", SecretAccessibility::AfterFirstUnlock)
        .expect("the session store accepts the write");

    assert_eq!(store.get(&key()).ok().flatten(), Some(b"token".to_vec()));
}

#[test]
fn an_available_os_store_is_the_one_used() {
    let memory = Arc::new(InMemorySecureStore::new());
    let os_store: Arc<dyn SecureStore> = memory.clone();
    let store = usable_secure_store(os_store, None);

    store
        .set(&key(), b"token", SecretAccessibility::AfterFirstUnlock)
        .expect("the session store accepts the write");

    assert_eq!(memory.get(&key()).ok().flatten(), Some(b"token".to_vec()));
}
