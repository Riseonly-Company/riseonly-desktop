use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_date::Date;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{
    AUTH_ACCOUNTS, AUTH_ACTIVE_ACCOUNT, AUTH_MULTI_ACCOUNT_MIGRATED, KEYCHAIN_BOOTSTRAP,
};
use serde::{Deserialize, Serialize};

use crate::core::stores::http::http_service::HttpService;
use crate::modules::auth::services::keychain_store::KeychainStore;
use crate::modules::auth::stores::auth_actions::auth_types::StoredAccessTokens;
use crate::modules::auth::stores::auth_actions::auth_wire_types::AuthUserDTO;
use crate::modules::user::shared::contact_device_book::ContactDeviceStateStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AuthLocalStorageKey {
    Tokens,
    SessionId,
    Profile,
}

impl AuthLocalStorageKey {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Tokens => "tokens",
            Self::SessionId => "sessionId",
            Self::Profile => "profile",
        }
    }
}

/// an account persisted in auth local storage
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthPersistedAccount {
    pub user: AuthUserDTO,
    pub tokens: StoredAccessTokens,
    pub session_id: String,
    pub last_used_at: i64,
    pub backend_id: Option<String>,
    pub is_mirror: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AuthAccountMetadata {
    user: AuthUserDTO,
    last_used_at: i64,
    backend_id: Option<String>,
    is_mirror: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthAccountSecret {
    tokens: StoredAccessTokens,
    session_id: String,
}

/// auth local storage, every call hits the os credential store and blocks
/// callers run it off the main thread
pub enum AuthLocalStorage {}

impl AuthLocalStorage {
    fn suite() -> UserDefaults {
        UserDefaults::standard()
    }

    fn ensure_bootstrap() {
        let suite = Self::suite();
        if suite.bool(KEYCHAIN_BOOTSTRAP) {
            return;
        }
        KeychainStore::remove(AuthLocalStorageKey::Tokens.raw_value());
        KeychainStore::remove(AuthLocalStorageKey::SessionId.raw_value());
        KeychainStore::remove(AUTH_ACTIVE_ACCOUNT);
        suite.set(KEYCHAIN_BOOTSTRAP, true);
    }

    pub fn save_tokens(value: &StoredAccessTokens) -> anyhow::Result<()> {
        let data = serde_json::to_vec(value)?;
        KeychainStore::set_data(&data, AuthLocalStorageKey::Tokens.raw_value())?;
        Ok(())
    }

    pub fn save_session_id(id: &str) {
        let _ = KeychainStore::set_data(id.as_bytes(), AuthLocalStorageKey::SessionId.raw_value());
    }

    pub fn load_tokens() -> Option<StoredAccessTokens> {
        Self::ensure_bootstrap();
        if let Some(data) = KeychainStore::data(AuthLocalStorageKey::Tokens.raw_value()) {
            return serde_json::from_slice(&data).ok();
        }
        let suite = Self::suite();
        let legacy = suite.data(AuthLocalStorageKey::Tokens.raw_value())?;
        let _ = KeychainStore::set_data(&legacy, AuthLocalStorageKey::Tokens.raw_value());
        suite.remove_object(AuthLocalStorageKey::Tokens.raw_value());
        serde_json::from_slice(&legacy).ok()
    }

    pub fn load_session_id() -> Option<String> {
        Self::ensure_bootstrap();
        if let Some(data) = KeychainStore::data(AuthLocalStorageKey::SessionId.raw_value()) {
            return String::from_utf8(data).ok();
        }
        let suite = Self::suite();
        let legacy = suite.string(AuthLocalStorageKey::SessionId.raw_value())?;
        let _ = KeychainStore::set_data(legacy.as_bytes(), AuthLocalStorageKey::SessionId.raw_value());
        suite.remove_object(AuthLocalStorageKey::SessionId.raw_value());
        Some(legacy)
    }

    pub fn save_profile_user(user: &AuthUserDTO) -> anyhow::Result<()> {
        let data = serde_json::to_vec(user)?;
        Self::suite().set(AuthLocalStorageKey::Profile.raw_value(), data);
        Ok(())
    }

    pub fn load_profile_user() -> Option<AuthUserDTO> {
        let data = Self::suite().data(AuthLocalStorageKey::Profile.raw_value())?;
        serde_json::from_slice(&data).ok()
    }

    pub fn clear_auth_payload() {
        KeychainStore::remove(AuthLocalStorageKey::Tokens.raw_value());
        KeychainStore::remove(AuthLocalStorageKey::SessionId.raw_value());
        let suite = Self::suite();
        suite.remove_object(AuthLocalStorageKey::Tokens.raw_value());
        suite.remove_object(AuthLocalStorageKey::SessionId.raw_value());
        suite.remove_object(AuthLocalStorageKey::Profile.raw_value());
    }

    pub fn load_accounts() -> Vec<AuthPersistedAccount> {
        Self::migrate_legacy_session_if_needed();
        let mut accounts: Vec<AuthPersistedAccount> = Self::load_metadata()
            .into_iter()
            .filter_map(|metadata| {
                let secret = Self::load_secret(&metadata.user.id)?;
                Some(AuthPersistedAccount {
                    user: metadata.user,
                    tokens: secret.tokens,
                    session_id: secret.session_id,
                    last_used_at: metadata.last_used_at,
                    backend_id: metadata.backend_id,
                    is_mirror: metadata.is_mirror.unwrap_or(false),
                })
            })
            .collect();
        accounts.sort_by(|lhs, rhs| {
            rhs.last_used_at.cmp(&lhs.last_used_at).then_with(|| lhs.user.id.cmp(&rhs.user.id))
        });
        accounts
    }

    pub fn load_account(user_id: &str) -> Option<AuthPersistedAccount> {
        Self::load_accounts().into_iter().find(|account| account.user.id == user_id)
    }

    pub fn active_account_id() -> Option<String> {
        Self::migrate_legacy_session_if_needed();
        let metadata = Self::load_metadata();
        if let Some(data) = KeychainStore::data(AUTH_ACTIVE_ACCOUNT)
            && let Ok(id) = String::from_utf8(data)
            && metadata.iter().any(|entry| entry.user.id == id)
        {
            return Some(id);
        }
        Self::suite().string(AUTH_ACTIVE_ACCOUNT)
    }

    pub fn save_account(
        user: &AuthUserDTO,
        tokens: &StoredAccessTokens,
        session_id: &str,
        last_used_at: i64,
        backend_id: Option<&str>,
        is_mirror: bool,
    ) -> anyhow::Result<()> {
        let secret = AuthAccountSecret {
            tokens: tokens.clone(),
            session_id: session_id.to_owned(),
        };
        let secret_data = serde_json::to_vec(&secret)?;
        KeychainStore::set_data(&secret_data, &Self::secret_key(&user.id))?;

        let mut metadata = Self::load_metadata();
        let entry = AuthAccountMetadata {
            user: user.clone(),
            last_used_at,
            backend_id: backend_id.map(ToOwned::to_owned),
            is_mirror: Some(is_mirror),
        };
        if let Some(index) = metadata.iter().position(|existing| existing.user.id == user.id) {
            metadata[index] = entry;
        } else {
            metadata.push(entry);
        }
        Self::save_metadata(&metadata);
        Self::set_active_account(&user.id, last_used_at);
        Ok(())
    }

    pub fn update_tokens(user_id: &str, tokens: &StoredAccessTokens, session_id: &str) -> anyhow::Result<()> {
        let Some(mut secret) = Self::load_secret(user_id) else {
            return Ok(());
        };
        secret.tokens = tokens.clone();
        secret.session_id = session_id.to_owned();
        let data = serde_json::to_vec(&secret)?;
        KeychainStore::set_data(&data, &Self::secret_key(user_id))?;
        Ok(())
    }

    pub fn store_rotated_tokens(
        user_id: &str,
        expected_session_id: &str,
        tokens: &StoredAccessTokens,
        session_id: &str,
    ) -> anyhow::Result<bool> {
        if !Self::load_metadata().iter().any(|entry| entry.user.id == user_id) {
            return Ok(false);
        }
        let Some(secret) = Self::load_secret(user_id) else {
            return Ok(false);
        };
        if secret.session_id != expected_session_id {
            return Ok(false);
        }

        let updated = AuthAccountSecret {
            tokens: tokens.clone(),
            session_id: session_id.to_owned(),
        };
        let data = serde_json::to_vec(&updated)?;
        KeychainStore::set_data(&data, &Self::secret_key(user_id))?;
        Ok(true)
    }

    pub fn update_user(user: &AuthUserDTO) {
        let mut metadata = Self::load_metadata();
        let Some(index) = metadata.iter().position(|entry| entry.user.id == user.id) else {
            return;
        };
        metadata[index] = AuthAccountMetadata {
            user: user.clone(),
            last_used_at: metadata[index].last_used_at,
            backend_id: metadata[index].backend_id.clone(),
            is_mirror: metadata[index].is_mirror,
        };
        Self::save_metadata(&metadata);
    }

    pub fn stamp_backend_id_if_missing(user_id: &str, backend_id: &str) {
        let mut metadata = Self::load_metadata();
        let Some(index) = metadata.iter().position(|entry| entry.user.id == user_id) else {
            return;
        };
        if metadata[index].backend_id.is_some() {
            return;
        }
        metadata[index].backend_id = Some(backend_id.to_owned());
        Self::save_metadata(&metadata);
    }

    /// forgets which account is active without removing any, so a later activation picks afresh
    pub fn clear_active_account_pointer() {
        KeychainStore::remove(AUTH_ACTIVE_ACCOUNT);
        Self::suite().remove_object(AUTH_ACTIVE_ACCOUNT);
    }

    pub fn set_active_account(user_id: &str, last_used_at: i64) {
        let mut metadata = Self::load_metadata();
        if let Some(index) = metadata.iter().position(|entry| entry.user.id == user_id) {
            metadata[index].last_used_at = last_used_at;
            Self::save_metadata(&metadata);
        }
        Self::suite().set(AUTH_ACTIVE_ACCOUNT, user_id);
        if KeychainStore::set_data(user_id.as_bytes(), AUTH_ACTIVE_ACCOUNT).is_err() {
            KeychainStore::remove(AUTH_ACTIVE_ACCOUNT);
            tracing::warn!(target: "Auth", "could not persist active account selection");
        }
    }

    pub fn remove_account(user_id: &str) {
        let was_active = Self::active_account_id().as_deref() == Some(user_id);
        KeychainStore::remove(&Self::secret_key(user_id));
        ContactDeviceStateStore::remove_account(user_id);
        let remaining: Vec<AuthAccountMetadata> =
            Self::load_metadata().into_iter().filter(|entry| entry.user.id != user_id).collect();
        Self::save_metadata(&remaining);
        if was_active {
            Self::clear_active_account_pointer();
        }
    }

    pub fn clear_all_accounts() {
        for metadata in Self::load_metadata() {
            KeychainStore::remove(&Self::secret_key(&metadata.user.id));
            ContactDeviceStateStore::remove_account(&metadata.user.id);
        }
        Self::suite().remove_object(AUTH_ACCOUNTS);
        Self::clear_active_account_pointer();
        Self::clear_auth_payload();
    }

    fn load_metadata() -> Vec<AuthAccountMetadata> {
        let Some(data) = Self::suite().data(AUTH_ACCOUNTS) else {
            return Vec::new();
        };
        let Ok(mut metadata) = serde_json::from_slice::<Vec<AuthAccountMetadata>>(&data) else {
            return Vec::new();
        };

        let mut migrated = false;
        for entry in &mut metadata {
            let Some(backend_id) = entry.backend_id.as_deref() else {
                continue;
            };
            let canonical_id = HttpService::canonical_backend_id(backend_id);
            if canonical_id == backend_id {
                continue;
            }
            entry.backend_id = Some(canonical_id);
            migrated = true;
        }
        if migrated {
            Self::save_metadata(&metadata);
        }
        metadata
    }

    fn save_metadata(metadata: &[AuthAccountMetadata]) {
        let Ok(data) = serde_json::to_vec(metadata) else {
            return;
        };
        Self::suite().set(AUTH_ACCOUNTS, data);
    }

    fn load_secret(user_id: &str) -> Option<AuthAccountSecret> {
        let data = KeychainStore::data(&Self::secret_key(user_id))?;
        serde_json::from_slice(&data).ok()
    }

    fn secret_key(user_id: &str) -> String {
        let value = sha256_hex(user_id.as_bytes());
        format!("riseonly.auth.account.{value}.secret.v1")
    }

    fn migrate_legacy_session_if_needed() {
        let suite = Self::suite();
        if suite.bool(AUTH_MULTI_ACCOUNT_MIGRATED) {
            return;
        }
        Self::ensure_bootstrap();
        Self::migrate_legacy_session();
        suite.set(AUTH_MULTI_ACCOUNT_MIGRATED, true);
    }

    fn migrate_legacy_session() {
        if !Self::load_metadata().is_empty() {
            return;
        }
        let Some(tokens) = Self::load_tokens() else {
            return;
        };
        let Some(session_id) = Self::load_session_id() else {
            return;
        };
        let Some(user) = Self::load_profile_user() else {
            return;
        };
        if user.id.is_empty() {
            return;
        }
        let last_used_at = (Date::now().time_interval_since_1970() * 1_000.0) as i64;
        let backend_id = HttpService::resolved_backend_id();
        let _ = Self::save_account(&user, &tokens, &session_id, last_used_at, Some(&backend_id), false);
        Self::clear_auth_payload();
    }
}
