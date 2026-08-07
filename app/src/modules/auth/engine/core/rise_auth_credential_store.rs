use std::path::{Path, PathBuf};

use rise_platform::{SecretKey, SecureStore, SecureStoreError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::modules::auth::engine::rise_auth_engine_models::{PersistedAccount, StoredTokens};

/// Where the account list and its credentials live.
///
/// Deliberately outside RiseEngine, and the reason is a chicken and egg: the
/// engine's database is keyed by account, and this is what answers "which
/// accounts are there" before any account is open. The reference splits it the
/// same way — metadata in UserDefaults, secrets in the Keychain — and the split
/// is the point: the file is readable without unlocking anything, and the tokens
/// are not in it.
pub struct AuthCredentialStore {
    registry_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("secure store: {0}")]
    SecureStore(#[from] SecureStoreError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("registry: {0}")]
    Registry(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    accounts: Vec<PersistedAccount>,
    #[serde(default)]
    active: Option<String>,
}

impl AuthCredentialStore {
    pub fn new(data_directory: &Path) -> Self {
        Self {
            registry_path: data_directory.join("accounts.json"),
        }
    }

    pub fn accounts(&self) -> Vec<PersistedAccount> {
        self.registry().accounts
    }

    pub fn active_account_id(&self) -> Option<String> {
        self.registry().active
    }

    pub fn tokens(
        &self,
        user_id: &str,
        secrets: &dyn SecureStore,
    ) -> Result<Option<StoredTokens>, CredentialError> {
        let Some(bytes) = secrets.get(&secret_key(user_id))? else {
            return Ok(None);
        };
        Ok(serde_json::from_slice(&bytes).ok())
    }

    pub fn save(
        &self,
        account: PersistedAccount,
        tokens: &StoredTokens,
        secrets: &dyn SecureStore,
    ) -> Result<(), CredentialError> {
        let encoded =
            serde_json::to_vec(tokens).map_err(|e| CredentialError::Registry(e.to_string()))?;
        secrets.set(&secret_key(&account.user.id), &encoded)?;

        let mut registry = self.registry();
        match registry
            .accounts
            .iter_mut()
            .find(|existing| existing.user.id == account.user.id)
        {
            Some(existing) => *existing = account.clone(),
            None => registry.accounts.push(account.clone()),
        }
        registry.active = Some(account.user.id);
        self.write(&registry)
    }

    /// Rotates a pair only if the one on disk is still the one the caller
    /// started from.
    ///
    /// This is the whole defence against a stale refresh: two refreshes can be
    /// in flight for the same account, and the loser must not overwrite the
    /// winner's newer pair with the one it was handed. `false` means "somebody
    /// else already rotated" and is not an error.
    pub fn rotate_if_matching(
        &self,
        user_id: &str,
        expected: &StoredTokens,
        next: &StoredTokens,
        secrets: &dyn SecureStore,
    ) -> Result<bool, CredentialError> {
        let Some(current) = self.tokens(user_id, secrets)? else {
            return Ok(false);
        };
        if current.access != expected.access || current.refresh != expected.refresh {
            return Ok(false);
        }

        let encoded =
            serde_json::to_vec(next).map_err(|e| CredentialError::Registry(e.to_string()))?;
        secrets.set(&secret_key(user_id), &encoded)?;
        Ok(true)
    }

    pub fn set_active(&self, user_id: &str, last_used_ms: i64) -> Result<(), CredentialError> {
        let mut registry = self.registry();
        if let Some(account) = registry
            .accounts
            .iter_mut()
            .find(|existing| existing.user.id == user_id)
        {
            account.last_used_ms = last_used_ms;
        }
        registry.active = Some(user_id.to_owned());
        self.write(&registry)
    }

    pub fn remove(&self, user_id: &str, secrets: &dyn SecureStore) -> Result<(), CredentialError> {
        secrets.delete(&secret_key(user_id))?;

        let mut registry = self.registry();
        registry
            .accounts
            .retain(|account| account.user.id != user_id);
        if registry.active.as_deref() == Some(user_id) {
            registry.active = registry
                .accounts
                .first()
                .map(|account| account.user.id.clone());
        }
        self.write(&registry)
    }

    /// A malformed or missing file reads as "no accounts" rather than failing.
    ///
    /// The alternative is an app that will not start because a JSON file got
    /// truncated by a power cut, and the recovery from "no accounts" is signing
    /// in again — which is the same recovery a hard failure would force anyway.
    fn registry(&self) -> Registry {
        std::fs::read(&self.registry_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Written through a temporary file and renamed.
    ///
    /// A half-written registry is the one failure that loses every account at
    /// once, and truncating in place makes that the outcome of any crash during
    /// the write.
    fn write(&self, registry: &Registry) -> Result<(), CredentialError> {
        if let Some(parent) = self.registry_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let encoded =
            serde_json::to_vec(registry).map_err(|e| CredentialError::Registry(e.to_string()))?;
        let temporary = self.registry_path.with_extension("json.tmp");
        std::fs::write(&temporary, encoded)?;
        std::fs::rename(&temporary, &self.registry_path)?;
        Ok(())
    }
}

/// The account id is hashed before it becomes a credential-store entry name.
///
/// The entry name is visible in Keychain Access, in `secret-tool search` and in
/// Credential Manager, and a user id there is a piece of the account's identity
/// sitting in a list anybody at the machine can read.
fn secret_key(user_id: &str) -> SecretKey {
    let digest = Sha256::digest(user_id.as_bytes());
    let hex: String = digest
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    SecretKey::auth_credentials(hex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::auth::engine::rise_auth_engine_models::AuthUser;
    use rise_platform::InMemorySecureStore;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rise-credentials-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn account(id: &str) -> PersistedAccount {
        PersistedAccount {
            user: AuthUser {
                id: id.to_owned(),
                name: id.to_owned(),
                ..AuthUser::default()
            },
            last_used_ms: 1,
        }
    }

    fn tokens(access: &str) -> StoredTokens {
        StoredTokens {
            access: access.to_owned(),
            refresh: format!("{access}-refresh"),
            session: "session".to_owned(),
        }
    }

    #[test]
    fn an_account_round_trips_and_becomes_the_active_one() {
        let dir = temp_dir("roundtrip");
        let store = AuthCredentialStore::new(&dir);
        let secrets = InMemorySecureStore::new();

        store.save(account("u1"), &tokens("a1"), &secrets).unwrap();

        assert_eq!(store.accounts().len(), 1);
        assert_eq!(store.active_account_id().as_deref(), Some("u1"));
        assert_eq!(store.tokens("u1", &secrets).unwrap(), Some(tokens("a1")));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saving_the_same_account_twice_updates_it_rather_than_duplicating_it() {
        let dir = temp_dir("update");
        let store = AuthCredentialStore::new(&dir);
        let secrets = InMemorySecureStore::new();

        store.save(account("u1"), &tokens("a1"), &secrets).unwrap();
        let mut renamed = account("u1");
        renamed.user.name = "Renamed".into();
        store.save(renamed, &tokens("a2"), &secrets).unwrap();

        let accounts = store.accounts();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].user.name, "Renamed");
        assert_eq!(store.tokens("u1", &secrets).unwrap(), Some(tokens("a2")));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_rotation_refuses_to_overwrite_a_pair_somebody_else_already_replaced() {
        let dir = temp_dir("rotate");
        let store = AuthCredentialStore::new(&dir);
        let secrets = InMemorySecureStore::new();
        store.save(account("u1"), &tokens("a1"), &secrets).unwrap();

        assert!(
            store
                .rotate_if_matching("u1", &tokens("a1"), &tokens("a2"), &secrets)
                .unwrap()
        );
        assert_eq!(store.tokens("u1", &secrets).unwrap(), Some(tokens("a2")));

        assert!(
            !store
                .rotate_if_matching("u1", &tokens("a1"), &tokens("a3"), &secrets)
                .unwrap(),
            "the loser of two concurrent refreshes must not reinstate an older pair"
        );
        assert_eq!(store.tokens("u1", &secrets).unwrap(), Some(tokens("a2")));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rotating_an_account_that_is_gone_reports_false_rather_than_recreating_it() {
        let dir = temp_dir("rotate-missing");
        let store = AuthCredentialStore::new(&dir);
        let secrets = InMemorySecureStore::new();

        assert!(
            !store
                .rotate_if_matching("ghost", &tokens("a1"), &tokens("a2"), &secrets)
                .unwrap()
        );
        assert_eq!(store.tokens("ghost", &secrets).unwrap(), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn removing_the_active_account_promotes_another_one() {
        let dir = temp_dir("remove");
        let store = AuthCredentialStore::new(&dir);
        let secrets = InMemorySecureStore::new();

        store.save(account("u1"), &tokens("a1"), &secrets).unwrap();
        store.save(account("u2"), &tokens("a2"), &secrets).unwrap();
        assert_eq!(store.active_account_id().as_deref(), Some("u2"));

        store.remove("u2", &secrets).unwrap();
        assert_eq!(store.active_account_id().as_deref(), Some("u1"));
        assert_eq!(store.tokens("u2", &secrets).unwrap(), None);

        store.remove("u1", &secrets).unwrap();
        assert_eq!(store.active_account_id(), None);
        assert!(store.accounts().is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_truncated_registry_reads_as_no_accounts_instead_of_refusing_to_start() {
        let dir = temp_dir("corrupt");
        let store = AuthCredentialStore::new(&dir);
        std::fs::write(dir.join("accounts.json"), "{\"accounts\": [").unwrap();

        assert!(store.accounts().is_empty());
        assert_eq!(store.active_account_id(), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_credential_entry_name_never_contains_the_account_id() {
        let key = secret_key("user@example.com");
        assert!(
            !key.entry_name().contains("user@example.com"),
            "the entry name is visible in Keychain Access and in secret-tool"
        );
        assert_eq!(secret_key("u1").entry_name(), secret_key("u1").entry_name());
        assert_ne!(secret_key("u1").entry_name(), secret_key("u2").entry_name());
    }
}
