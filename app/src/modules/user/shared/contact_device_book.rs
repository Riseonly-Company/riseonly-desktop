use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::LazyLock;

use aes_gcm::aead::{Aead, AeadCore, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::{RngCore, TryRngCore};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_character::characters;
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_crypto::{Sha256, hex_string, sha256_hex};
use rise_core::foundation::foundation_text::{character_count, character_suffix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::SecureStoreError;
use rise_platform::contacts::{
    self as device_contacts, ContactAuthorization, ContactChange, ContactKey, ContactLabel,
    ContactSaveRequest, DeviceContact, InstantMessageAddress, LabeledValue,
};
use rise_platform::file_manager::FileManager;
use rise_platform::url_resource_values::{URLResourceValues, set_resource_values};
use rise_ui::phone_country_catalog::PhoneCountryCatalog;
use serde::{Deserialize, Serialize};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::modules::auth::services::keychain_store::{KeychainStore, KeychainStoreKeychainError};
use crate::modules::user::stores::user::user_actions::user_contact_types::{
    UserContactLimits, UserContactMutationError,
};
use crate::modules::user::stores::user::user_actions::user_contact_wire_types::UserDeviceContact;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactDeviceAuthorization {
    Authorized,
    Limited,
    Denied,
    Restricted,
    NotDetermined,
}

impl ContactDeviceAuthorization {
    pub fn new(value: ContactAuthorization) -> Self {
        match value {
            ContactAuthorization::Authorized => ContactDeviceAuthorization::Authorized,
            ContactAuthorization::Limited => ContactDeviceAuthorization::Limited,
            ContactAuthorization::Denied => ContactDeviceAuthorization::Denied,
            ContactAuthorization::Restricted => ContactDeviceAuthorization::Restricted,
            ContactAuthorization::NotDetermined => ContactDeviceAuthorization::NotDetermined,
        }
    }

    pub fn can_read_and_write(self) -> bool {
        self == ContactDeviceAuthorization::Authorized || self == ContactDeviceAuthorization::Limited
    }
}

#[derive(Clone, Debug)]
pub struct ContactDeviceSyncPlan {
    pub account_id: String,
    pub device_id: String,
    pub device_generation: i64,
    pub contacts: Vec<UserDeviceContact>,
    pub is_unchanged: bool,
    current_history_token: Option<Vec<u8>>,
    records: HashMap<String, ContactDeviceRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactDevicePhone {
    pub raw_value: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactDeviceRecord {
    pub identifier: String,
    pub first_name: String,
    pub last_name: String,
    pub phones: Vec<ContactDevicePhone>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactDeviceMapping {
    pub identifier: String,
    pub created_by_riseonly: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactDeviceState {
    #[serde(default, with = "base64_optional_data")]
    pub current_history_token: Option<Vec<u8>>,
    pub records: HashMap<String, ContactDeviceRecord>,
    pub mappings: HashMap<String, ContactDeviceMapping>,
    #[serde(with = "base64_data")]
    pub client_identifier_salt: Vec<u8>,
    pub device_id: String,
    pub last_issued_generation: i64,
}

impl ContactDeviceState {
    pub fn empty() -> ContactDeviceState {
        ContactDeviceState {
            current_history_token: None,
            records: HashMap::new(),
            mappings: HashMap::new(),
            client_identifier_salt: ContactDeviceStateStore::random_data(32),
            device_id: ContactDeviceStateStore::installation_device_id(),
            last_issued_generation: 0,
        }
    }
}

mod base64_data {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        STANDARD.decode(encoded).map_err(serde::de::Error::custom)
    }
}

mod base64_optional_data {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error> {
        match bytes {
            Some(bytes) => serializer.serialize_some(&STANDARD.encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error> {
        let encoded = Option::<String>::deserialize(deserializer)?;
        encoded
            .map(|encoded| STANDARD.decode(encoded).map_err(serde::de::Error::custom))
            .transpose()
    }
}

/// one device bound id shared by contact sync, push ownership and rtc fencing
/// keeps the existing keychain item so an update never makes a second identity
pub enum RiseInstallationIdentity {}

impl RiseInstallationIdentity {
    pub fn current() -> anyhow::Result<String> {
        ContactDeviceStateStore::require_installation_device_id()
    }
}

type ContactDeviceSnapshotNonce = Nonce<<Aes256Gcm as AeadCore>::NonceSize>;

pub enum ContactDeviceStateStore {}

impl ContactDeviceStateStore {
    const INSTALLATION_DEVICE_KEY: &'static str = "riseonly.contacts.device.id.v2.thisdevice";
    const SNAPSHOT_KEY_BYTES: usize = 32;

    pub fn remove_account(account_id: &str) {
        let identity = Self::account_identity(account_id);
        KeychainStore::remove(&Self::keychain_key(&identity));
        KeychainStore::remove(&Self::legacy_keychain_key(&identity));
        KeychainStore::remove(&Self::recovery_key(&identity));
        if let Ok(file) = Self::file_url(&identity, false) {
            let _ = FileManager.remove_item(&file);
        }
    }

    pub fn load(account_id: &str) -> ContactDeviceState {
        let identity = Self::account_identity(account_id);
        let mut state = Self::decoded_snapshot(&identity).unwrap_or_else(ContactDeviceState::empty);
        for (target_user_id, mapping) in Self::recovery_mappings(&identity) {
            state.mappings.insert(target_user_id, mapping);
        }
        state
    }

    fn decoded_snapshot(identity: &str) -> Option<ContactDeviceState> {
        let combined = std::fs::read(Self::file_url(identity, false).ok()?).ok()?;
        let key_data = KeychainStore::data(&Self::keychain_key(identity))?;
        if key_data.len() != Self::SNAPSHOT_KEY_BYTES {
            return None;
        }
        let clear = Self::open_sealed(&combined, &key_data)?;
        serde_json::from_slice(&clear).ok()
    }

    pub fn save(state: &ContactDeviceState, account_id: &str) -> anyhow::Result<()> {
        let identity = Self::account_identity(account_id);
        let key_data = match KeychainStore::data(&Self::keychain_key(&identity)) {
            Some(existing) if existing.len() == Self::SNAPSHOT_KEY_BYTES => existing,
            _ => {
                let key_data = Self::secure_random_data(Self::SNAPSHOT_KEY_BYTES)?;
                KeychainStore::set_this_device_only_data(&key_data, &Self::keychain_key(&identity))?;
                key_data
            }
        };

        let clear = serde_json::to_vec(state)?;
        let combined = Self::seal(&clear, &key_data).ok_or(UserContactMutationError::InvalidResponse)?;

        let file = Self::file_url(&identity, true)?;
        let directory = file.parent().map(PathBuf::from).ok_or(UserContactMutationError::InvalidResponse)?;
        FileManager.create_directory(&directory, true)?;
        let staging = directory.join(format!(".{}.{}", Uuid::new_v4().uuid_string(), Self::SNAPSHOT_EXTENSION));
        std::fs::write(&staging, &combined)?;
        if let Err(error) = std::fs::rename(&staging, &file) {
            let _ = std::fs::remove_file(&staging);
            return Err(error.into());
        }
        KeychainStore::remove(&Self::recovery_key(&identity));
        let values = URLResourceValues { is_excluded_from_backup: Some(true) };
        let _ = set_resource_values(&directory, &values);
        Ok(())
    }

    fn seal(clear: &[u8], key_data: &[u8]) -> Option<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(key_data).ok()?;
        let mut nonce = ContactDeviceSnapshotNonce::default();
        rand::rng().fill_bytes(nonce.as_mut_slice());
        let ciphertext_and_tag = cipher.encrypt(&nonce, clear).ok()?;
        let mut combined = nonce.to_vec();
        combined.extend_from_slice(&ciphertext_and_tag);
        Some(combined)
    }

    fn open_sealed(combined: &[u8], key_data: &[u8]) -> Option<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(key_data).ok()?;
        let nonce_length = ContactDeviceSnapshotNonce::default().len();
        if combined.len() < nonce_length {
            return None;
        }
        let (nonce, ciphertext_and_tag) = combined.split_at(nonce_length);
        cipher.decrypt(ContactDeviceSnapshotNonce::from_slice(nonce), ciphertext_and_tag).ok()
    }

    pub fn random_data(count: usize) -> Vec<u8> {
        if let Ok(secure) = Self::secure_random_data(count) {
            return secure;
        }
        let mut digest = Sha256::hash(Uuid::new_v4().uuid_string().as_bytes()).to_vec();
        digest.truncate(count);
        digest
    }

    pub fn secure_random_data(count: usize) -> Result<Vec<u8>, KeychainStoreKeychainError> {
        let mut bytes = vec![0_u8; count];
        rand::rngs::OsRng.try_fill_bytes(&mut bytes).map_err(|error| {
            KeychainStoreKeychainError::UnexpectedStatus(SecureStoreError::Backend(error.to_string()))
        })?;
        Ok(bytes)
    }

    fn stored_installation_device_id() -> Option<String> {
        let data = KeychainStore::data(Self::INSTALLATION_DEVICE_KEY)?;
        let stored = String::from_utf8(data).ok()?;
        let stored = trimming_whitespaces_and_newlines(&stored);
        if stored.is_empty() {
            return None;
        }
        Some(stored.to_owned())
    }

    pub fn installation_device_id() -> String {
        if let Some(stored) = Self::stored_installation_device_id() {
            return stored;
        }
        Self::create_installation_device_id().unwrap_or_default()
    }

    pub fn require_installation_device_id() -> anyhow::Result<String> {
        if let Some(stored) = Self::stored_installation_device_id() {
            return Ok(stored);
        }
        Self::create_installation_device_id()
    }

    fn create_installation_device_id() -> anyhow::Result<String> {
        let generated = Uuid::new_v4().uuid_string().to_lowercase();
        KeychainStore::set_this_device_only_data(generated.as_bytes(), Self::INSTALLATION_DEVICE_KEY)?;
        Ok(generated)
    }

    pub fn next_generation(account_id: &str, snapshot_generation: i64) -> anyhow::Result<i64> {
        let identity = Self::account_identity(account_id);
        let key = Self::generation_key(&identity);
        let stored = KeychainStore::data(&key)
            .and_then(|data| String::from_utf8(data).ok())
            .and_then(|raw| raw.parse::<i64>().ok())
            .map_or(0, |value| value.max(0));
        let Some(next) = Self::next_generation_value(stored, snapshot_generation) else {
            return Err(UserContactMutationError::InvalidResponse.into());
        };
        KeychainStore::set_this_device_only_data(next.to_string().as_bytes(), &key)?;
        Ok(next)
    }

    pub fn next_generation_value(stored_generation: i64, snapshot_generation: i64) -> Option<i64> {
        let current = stored_generation.max(snapshot_generation).max(0);
        if current >= i64::MAX {
            return None;
        }
        Some(current + 1)
    }

    fn account_identity(account_id: &str) -> String {
        sha256_hex(account_id.as_bytes())
    }

    fn keychain_key(identity: &str) -> String {
        format!("riseonly.contacts.snapshot.{identity}.key.v2.thisdevice")
    }

    fn legacy_keychain_key(identity: &str) -> String {
        format!("riseonly.contacts.snapshot.{identity}.key.v1")
    }

    fn generation_key(identity: &str) -> String {
        format!("riseonly.contacts.snapshot.{identity}.generation.v2.thisdevice")
    }

    fn recovery_key(identity: &str) -> String {
        format!("riseonly.contacts.snapshot.{identity}.recovery.v2.thisdevice")
    }

    fn recovery_mappings(identity: &str) -> HashMap<String, ContactDeviceMapping> {
        KeychainStore::data(&Self::recovery_key(identity))
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    pub fn retain_recovery_mapping(
        account_id: &str,
        target_user_id: &str,
        mapping: ContactDeviceMapping,
    ) -> anyhow::Result<()> {
        let identity = Self::account_identity(account_id);
        let mut mappings = Self::recovery_mappings(&identity);
        mappings.insert(target_user_id.to_owned(), mapping);
        let data = serde_json::to_vec(&mappings)?;
        KeychainStore::set_this_device_only_data(&data, &Self::recovery_key(&identity))?;
        Ok(())
    }

    const DIRECTORY_NAME: &'static str = "ContactSync";
    const SNAPSHOT_EXTENSION: &'static str = "snapshot";

    fn file_url(identity: &str, create: bool) -> anyhow::Result<PathBuf> {
        let base = RiseEngineAppRuntime::application_support_directory(create)?;
        Ok(base.join(Self::DIRECTORY_NAME).join(format!("{identity}.{}", Self::SNAPSHOT_EXTENSION)))
    }
}

pub struct ContactDeviceBook {
    isolation: RiseIsolated<()>,
}

static SHARED: LazyLock<ContactDeviceBook> = LazyLock::new(|| ContactDeviceBook { isolation: RiseIsolated::new(()) });

const SYNC_KEYS_TO_FETCH: [ContactKey; 4] = [
    ContactKey::Identifier,
    ContactKey::GivenName,
    ContactKey::FamilyName,
    ContactKey::PhoneNumbers,
];

const MUTATION_KEYS_TO_FETCH: [ContactKey; 7] = [
    ContactKey::Identifier,
    ContactKey::GivenName,
    ContactKey::FamilyName,
    ContactKey::PhoneNumbers,
    ContactKey::ImageData,
    ContactKey::UrlAddresses,
    ContactKey::InstantMessageAddresses,
];

struct ContactDeviceBookNextState {
    token: Option<Vec<u8>>,
    records: HashMap<String, ContactDeviceRecord>,
    is_unchanged: bool,
}

impl ContactDeviceBook {
    pub const TRANSACTION_AUTHOR: &'static str = "riseonly.contact-sync";
    const OWNERSHIP_MARKER_SERVICE: &'static str = "riseonly.contact";
    const OWNERSHIP_MARKER_LABEL: &'static str = "Riseonly";

    pub fn shared() -> &'static ContactDeviceBook {
        &SHARED
    }

    pub fn authorization() -> ContactDeviceAuthorization {
        ContactDeviceAuthorization::new(device_contacts::authorization_status())
    }

    pub async fn request_authorization(&self) -> anyhow::Result<ContactDeviceAuthorization> {
        device_contacts::request_access().await?;
        Ok(Self::authorization())
    }

    pub async fn prepare_sync(&self, account_id: &str) -> anyhow::Result<ContactDeviceSyncPlan> {
        self.isolation.with(|_| self.prepare_sync_isolated(account_id))
    }

    fn prepare_sync_isolated(&self, account_id: &str) -> anyhow::Result<ContactDeviceSyncPlan> {
        if !Self::authorization().can_read_and_write() {
            return Err(UserContactMutationError::InvalidContact.into());
        }

        let mut previous = ContactDeviceStateStore::load(account_id);
        let installation_device_id = ContactDeviceStateStore::require_installation_device_id()?;
        if previous.device_id != installation_device_id {
            previous.device_id = installation_device_id;
            previous.current_history_token = None;
            previous.records = HashMap::new();
        }
        let next = match previous.current_history_token.clone() {
            Some(token) => self.incremental_state(&token, &previous.records)?,
            None => self.full_state()?,
        };

        previous.last_issued_generation =
            ContactDeviceStateStore::next_generation(account_id, previous.last_issued_generation)?;
        ContactDeviceStateStore::save(&previous, account_id)?;

        Ok(ContactDeviceSyncPlan {
            account_id: account_id.to_owned(),
            device_id: previous.device_id.clone(),
            device_generation: previous.last_issued_generation,
            contacts: self.wire_contacts(&next.records, account_id, &previous.client_identifier_salt),
            is_unchanged: next.is_unchanged,
            current_history_token: next.token,
            records: next.records,
        })
    }

    pub async fn commit(&self, plan: &ContactDeviceSyncPlan) -> anyhow::Result<()> {
        self.isolation.with(|_| {
            let mut state = ContactDeviceStateStore::load(&plan.account_id);
            if plan.device_generation != state.last_issued_generation {
                return Ok(());
            }
            state.current_history_token = plan.current_history_token.clone();
            state.records = plan.records.clone();
            ContactDeviceStateStore::save(&state, &plan.account_id)
        })
    }

    pub async fn clear_address_book_snapshot(&self, account_id: &str) -> anyhow::Result<()> {
        self.isolation.with(|_| {
            let mut state = ContactDeviceStateStore::load(account_id);
            state.current_history_token = None;
            state.records = HashMap::new();
            ContactDeviceStateStore::save(&state, account_id)
        })
    }

    pub async fn can_save_peer_contact(
        &self,
        account_id: &str,
        target_user_id: &str,
        phone: Option<&str>,
        profile_tag: &str,
    ) -> bool {
        self.isolation.with(|_| {
            if !Self::authorization().can_read_and_write() {
                return false;
            }
            let trimmed_phone = phone.map(trimming_whitespaces_and_newlines).unwrap_or_default();
            if !trimmed_phone.is_empty() {
                return true;
            }
            let state = ContactDeviceStateStore::load(account_id);
            if let Some(mapping) = state.mappings.get(target_user_id)
                && device_contacts::unified_contact(&mapping.identifier, &MUTATION_KEYS_TO_FETCH).is_ok()
            {
                return true;
            }
            self.profile_url(target_user_id, profile_tag).is_some()
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn save_peer_contact(
        &self,
        account_id: &str,
        target_user_id: &str,
        first_name: &str,
        last_name: &str,
        phone: Option<&str>,
        profile_tag: &str,
        image_data: Option<Vec<u8>>,
    ) -> anyhow::Result<()> {
        self.isolation.with(|_| {
            self.save_peer_contact_isolated(
                account_id,
                target_user_id,
                first_name,
                last_name,
                phone,
                profile_tag,
                image_data,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn save_peer_contact_isolated(
        &self,
        account_id: &str,
        target_user_id: &str,
        first_name: &str,
        last_name: &str,
        phone: Option<&str>,
        profile_tag: &str,
        image_data: Option<Vec<u8>>,
    ) -> anyhow::Result<()> {
        if !Self::authorization().can_read_and_write() {
            return Err(UserContactMutationError::ContactsPermissionUnavailable.into());
        }

        let mut state = ContactDeviceStateStore::load(account_id);
        let existing_mapping = state.mappings.get(target_user_id).cloned();
        let existing_contact = existing_mapping.as_ref().and_then(|mapping| {
            device_contacts::unified_contact(&mapping.identifier, &MUTATION_KEYS_TO_FETCH).ok()
        });
        let riseonly_profile_url = self.profile_url(target_user_id, profile_tag);
        let stable_riseonly_profile_url = self.profile_url(target_user_id, "");
        let marker_match = if existing_contact.is_none() {
            self.contact_matching_ownership_marker_for(target_user_id)?
        } else {
            None
        };
        let url_match = if existing_contact.is_none() && marker_match.is_none() {
            let candidates: Vec<String> = [riseonly_profile_url.clone(), stable_riseonly_profile_url]
                .into_iter()
                .flatten()
                .collect();
            self.contact_matching_any_profile_url(&candidates)?
        } else {
            None
        };
        let phone_match = if existing_contact.is_none() && marker_match.is_none() && url_match.is_none() {
            self.contact_matching(phone)?
        } else {
            None
        };
        let source_was_created_by_riseonly = if existing_contact.is_some() {
            existing_mapping.as_ref().is_some_and(|mapping| mapping.created_by_riseonly)
        } else if marker_match.is_some() {
            true
        } else {
            url_match.is_none() && phone_match.is_none()
        };
        let source = existing_contact.or(marker_match).or(url_match).or(phone_match);
        let normalized_phone = phone.map(trimming_whitespaces_and_newlines).unwrap_or_default().to_owned();
        if source.is_none() && normalized_phone.is_empty() && riseonly_profile_url.is_none() {
            return Err(UserContactMutationError::PhoneUnavailable.into());
        }
        let mut mutable = source.clone().unwrap_or_default();

        mutable.given_name = first_name.to_owned();
        mutable.family_name = last_name.to_owned();
        if !normalized_phone.is_empty() {
            let already_present = mutable
                .phone_numbers
                .iter()
                .any(|number| Self::phones_equivalent(&number.value, &normalized_phone));
            if !already_present {
                mutable
                    .phone_numbers
                    .push(LabeledValue::new(Some(ContactLabel::PhoneNumberMobile), normalized_phone.clone()));
            }
        }
        if let Some(riseonly_profile_url) = &riseonly_profile_url {
            let already_present = mutable
                .url_addresses
                .iter()
                .any(|address| compare_case_insensitive(&address.value, riseonly_profile_url) == Ordering::Equal);
            if !already_present {
                mutable.url_addresses.push(LabeledValue::new(
                    Some(ContactLabel::Custom(Self::OWNERSHIP_MARKER_LABEL.to_owned())),
                    riseonly_profile_url.clone(),
                ));
            }
        }
        if source_was_created_by_riseonly {
            let already_marked = mutable
                .instant_message_addresses
                .iter()
                .any(|address| Self::is_ownership_marker(&address.value, target_user_id));
            if !already_marked {
                mutable.instant_message_addresses.push(LabeledValue::new(
                    Some(ContactLabel::Custom(Self::OWNERSHIP_MARKER_LABEL.to_owned())),
                    InstantMessageAddress {
                        username: target_user_id.to_owned(),
                        service: Self::OWNERSHIP_MARKER_SERVICE.to_owned(),
                    },
                ));
            }
        }
        if let Some(image_data) = image_data {
            mutable.image_data = Some(image_data);
        }

        let mut request = ContactSaveRequest::new(Some(Self::TRANSACTION_AUTHOR.to_owned()));
        if source.is_none() {
            request.add(mutable.clone());
        } else {
            request.update(mutable.clone());
        }
        let identifiers = device_contacts::execute(&request)?;
        if let Some(identifier) = identifiers.into_iter().next() {
            mutable.identifier = identifier;
        }

        let mapping = ContactDeviceMapping {
            identifier: mutable.identifier.clone(),
            created_by_riseonly: source_was_created_by_riseonly,
        };
        state.records.insert(mutable.identifier.clone(), self.record(&mutable));
        state.mappings.insert(target_user_id.to_owned(), mapping.clone());
        if let Err(state_error) = ContactDeviceStateStore::save(&state, account_id) {
            if self.rollback_contact_mutation(source.as_ref(), &mutable).is_err()
                && ContactDeviceStateStore::save(&state, account_id).is_err()
            {
                let _ = ContactDeviceStateStore::retain_recovery_mapping(account_id, target_user_id, mapping);
            }
            return Err(state_error);
        }
        Ok(())
    }

    pub async fn update_mapped_photo(
        &self,
        account_id: &str,
        target_user_id: &str,
        image_data: Vec<u8>,
    ) -> anyhow::Result<()> {
        self.isolation.with(|_| {
            if !Self::authorization().can_read_and_write() {
                return Err(UserContactMutationError::ContactsPermissionUnavailable.into());
            }

            let state = ContactDeviceStateStore::load(account_id);
            let Some(mapping) = state.mappings.get(target_user_id) else {
                return Ok(());
            };
            let Ok(mut mutable) = device_contacts::unified_contact(&mapping.identifier, &MUTATION_KEYS_TO_FETCH)
            else {
                return Ok(());
            };

            mutable.image_data = Some(image_data);
            let mut request = ContactSaveRequest::new(Some(Self::TRANSACTION_AUTHOR.to_owned()));
            request.update(mutable);
            device_contacts::execute(&request)?;
            Ok(())
        })
    }

    pub async fn add_mapped_phone(&self, account_id: &str, target_user_id: &str, phone: &str) -> anyhow::Result<()> {
        self.isolation.with(|_| {
            if !Self::authorization().can_read_and_write() {
                return Ok(());
            }

            let mut state = ContactDeviceStateStore::load(account_id);
            let Some(mapping) = state.mappings.get(target_user_id) else {
                return Ok(());
            };
            let Ok(mut mutable) = device_contacts::unified_contact(&mapping.identifier, &MUTATION_KEYS_TO_FETCH)
            else {
                return Ok(());
            };
            if mutable.phone_numbers.iter().any(|number| Self::phones_equivalent(&number.value, phone)) {
                return Ok(());
            }

            mutable
                .phone_numbers
                .push(LabeledValue::new(Some(ContactLabel::PhoneNumberMobile), phone.to_owned()));
            let mut request = ContactSaveRequest::new(Some(Self::TRANSACTION_AUTHOR.to_owned()));
            request.update(mutable.clone());
            device_contacts::execute(&request)?;

            state.records.insert(mutable.identifier.clone(), self.record(&mutable));
            ContactDeviceStateStore::save(&state, account_id)
        })
    }

    pub async fn delete_mapped_peer_contact(&self, account_id: &str, target_user_id: &str) -> anyhow::Result<()> {
        self.isolation.with(|_| {
            let mut state = ContactDeviceStateStore::load(account_id);
            let Some(mapping) = state.mappings.get(target_user_id).cloned() else {
                return Ok(());
            };

            if !mapping.created_by_riseonly {
                state.mappings.remove(target_user_id);
                return ContactDeviceStateStore::save(&state, account_id);
            }
            if !Self::authorization().can_read_and_write() {
                return Err(UserContactMutationError::ContactsPermissionUnavailable.into());
            }

            let source = match device_contacts::unified_contact(&mapping.identifier, &MUTATION_KEYS_TO_FETCH) {
                Ok(source) => source,
                Err(error) if error.is_record_does_not_exist() => {
                    state.mappings.remove(target_user_id);
                    state.records.remove(&mapping.identifier);
                    return ContactDeviceStateStore::save(&state, account_id);
                }
                Err(error) => return Err(error.into()),
            };

            let mut request = ContactSaveRequest::new(Some(Self::TRANSACTION_AUTHOR.to_owned()));
            request.delete(source.identifier);
            device_contacts::execute(&request)?;

            state.mappings.remove(target_user_id);
            state.records.remove(&mapping.identifier);
            ContactDeviceStateStore::save(&state, account_id)
        })
    }

    fn full_state(&self) -> anyhow::Result<ContactDeviceBookNextState> {
        let result = device_contacts::fetch_all(&SYNC_KEYS_TO_FETCH)
            .map_err(|_| UserContactMutationError::InvalidResponse)?;
        let records = result
            .contacts
            .iter()
            .map(|contact| (contact.identifier.clone(), self.record(contact)))
            .collect();
        Ok(ContactDeviceBookNextState { token: result.current_history_token, records, is_unchanged: false })
    }

    fn incremental_state(
        &self,
        starting_token: &[u8],
        previous: &HashMap<String, ContactDeviceRecord>,
    ) -> anyhow::Result<ContactDeviceBookNextState> {
        let Ok(result) = device_contacts::fetch_changes(starting_token, &SYNC_KEYS_TO_FETCH, &[]) else {
            return self.full_state();
        };

        let mut records = previous.clone();
        let mut did_change = false;
        for change in &result.changes {
            match change {
                ContactChange::DropEverything => return self.full_state(),
                ContactChange::Add(contact) | ContactChange::Update(contact) => {
                    records.insert(contact.identifier.clone(), self.record(contact));
                    did_change = true;
                }
                ContactChange::Delete { contact_identifier } => {
                    records.remove(contact_identifier);
                    did_change = true;
                }
            }
        }
        Ok(ContactDeviceBookNextState { token: result.current_history_token, records, is_unchanged: !did_change })
    }

    fn record(&self, contact: &DeviceContact) -> ContactDeviceRecord {
        let mut seen: HashSet<String> = HashSet::new();
        let phones = contact
            .phone_numbers
            .iter()
            .filter_map(|value| {
                let raw = trimming_whitespaces_and_newlines(&value.value);
                if raw.is_empty() {
                    return None;
                }
                let fingerprint = Self::digits(raw);
                if fingerprint.is_empty() || !seen.insert(fingerprint) {
                    return None;
                }
                Some(ContactDevicePhone { raw_value: raw.to_owned(), label: self.phone_label(value.label.as_ref()) })
            })
            .collect();
        let name_limit = UserContactLimits::name();
        ContactDeviceRecord {
            identifier: contact.identifier.clone(),
            first_name: UserContactLimits::clip(&contact.given_name, name_limit),
            last_name: UserContactLimits::clip(&contact.family_name, name_limit),
            phones,
        }
    }

    fn wire_contacts(
        &self,
        records: &HashMap<String, ContactDeviceRecord>,
        account_id: &str,
        salt: &[u8],
    ) -> Vec<UserDeviceContact> {
        let region = PhoneCountryCatalog::default_country().region_code;
        let mut by_phone: HashMap<String, UserDeviceContact> = HashMap::new();

        let mut sorted: Vec<&ContactDeviceRecord> = records.values().collect();
        sorted.sort_by(|left, right| left.identifier.cmp(&right.identifier));
        for record in sorted {
            for phone in &record.phones {
                let fingerprint = Self::digits(&phone.raw_value);
                if fingerprint.is_empty() {
                    continue;
                }
                let mut identity_material = salt.to_vec();
                identity_material.extend_from_slice(account_id.as_bytes());
                identity_material.extend_from_slice(record.identifier.as_bytes());
                identity_material.extend_from_slice(fingerprint.as_bytes());
                let identifier = hex_string(&Sha256::hash(&identity_material));
                let item = UserDeviceContact {
                    client_contact_id: identifier,
                    raw_phone: phone.raw_value.clone(),
                    region_iso: region.clone(),
                    phone_e164: None,
                    first_name: record.first_name.clone(),
                    last_name: record.last_name.clone(),
                    phone_label: phone.label.clone(),
                };
                let replaces = by_phone.get(&fingerprint).is_none_or(|existing| {
                    let existing_name_length =
                        character_count(&existing.first_name) + character_count(&existing.last_name);
                    let candidate_name_length = character_count(&item.first_name) + character_count(&item.last_name);
                    candidate_name_length > existing_name_length
                });
                if replaces {
                    by_phone.insert(fingerprint, item);
                }
            }
        }
        let mut contacts: Vec<UserDeviceContact> = by_phone.into_values().collect();
        contacts.sort_by(|left, right| left.client_contact_id.cmp(&right.client_contact_id));
        contacts
    }

    fn phone_label(&self, label: Option<&ContactLabel>) -> String {
        let label = match label {
            Some(ContactLabel::PhoneNumberMobile) => "mobile",
            Some(ContactLabel::PhoneNumberIPhone) => "iphone",
            Some(ContactLabel::Home) => "home",
            Some(ContactLabel::Work) => "work",
            Some(ContactLabel::PhoneNumberMain) => "main",
            Some(ContactLabel::PhoneNumberPager) => "pager",
            Some(ContactLabel::Other) | Some(ContactLabel::Custom(_)) | None => "other",
        };
        label.to_owned()
    }

    fn contact_matching(&self, phone: Option<&str>) -> anyhow::Result<Option<DeviceContact>> {
        let Some(phone) = phone.map(trimming_whitespaces_and_newlines).filter(|phone| !phone.is_empty()) else {
            return Ok(None);
        };
        let contacts = device_contacts::unified_contacts_matching_phone(phone, &MUTATION_KEYS_TO_FETCH)?;
        Ok(contacts.into_iter().next())
    }

    fn contact_matching_ownership_marker_for(&self, target_user_id: &str) -> anyhow::Result<Option<DeviceContact>> {
        self.first_enumerated_contact(&mut |contact: &DeviceContact| {
            contact
                .instant_message_addresses
                .iter()
                .any(|address| Self::is_ownership_marker(&address.value, target_user_id))
        })
    }

    fn contact_matching_any_profile_url(&self, candidates: &[String]) -> anyhow::Result<Option<DeviceContact>> {
        let normalized_candidates: HashSet<String> = candidates
            .iter()
            .filter(|candidate| !candidate.is_empty())
            .map(|candidate| candidate.to_lowercase())
            .collect();
        if normalized_candidates.is_empty() {
            return Ok(None);
        }
        self.first_enumerated_contact(&mut |contact: &DeviceContact| {
            contact
                .url_addresses
                .iter()
                .any(|address| normalized_candidates.contains(&address.value.to_lowercase()))
        })
    }

    fn first_enumerated_contact(
        &self,
        matching: &mut dyn FnMut(&DeviceContact) -> bool,
    ) -> anyhow::Result<Option<DeviceContact>> {
        Ok(device_contacts::first_contact(&MUTATION_KEYS_TO_FETCH, matching)?)
    }

    fn is_ownership_marker(address: &InstantMessageAddress, target_user_id: &str) -> bool {
        compare_case_insensitive(&address.service, Self::OWNERSHIP_MARKER_SERVICE) == Ordering::Equal
            && address.username == target_user_id
    }

    fn phones_equivalent(lhs: &str, rhs: &str) -> bool {
        let left = Self::digits(lhs);
        let right = Self::digits(rhs);
        if left.is_empty() || right.is_empty() {
            return false;
        }
        if left == right {
            return true;
        }
        let comparable_length = 10.min(character_count(&left).min(character_count(&right)));
        if comparable_length < 7 {
            return false;
        }
        character_suffix(&left, comparable_length) == character_suffix(&right, comparable_length)
    }

    fn digits(text: &str) -> String {
        characters(text).filter(|character| character.is_number()).map(|character| character.as_str()).collect()
    }

    fn rollback_contact_mutation(&self, original: Option<&DeviceContact>, mutated: &DeviceContact) -> anyhow::Result<()> {
        let mut request = ContactSaveRequest::new(Some(Self::TRANSACTION_AUTHOR.to_owned()));
        match original {
            Some(original) => request.update(original.clone()),
            None => request.delete(mutated.identifier.clone()),
        }
        device_contacts::execute(&request)?;
        Ok(())
    }

    fn profile_url(&self, target_user_id: &str, profile_tag: &str) -> Option<String> {
        let tag = trimming_whitespaces_and_newlines(profile_tag);
        let fallback = trimming_whitespaces_and_newlines(target_user_id);
        let slug = if tag.is_empty() { fallback } else { tag };
        if slug.is_empty() {
            return None;
        }
        RiseonlyPublicLink::Profile { tag: slug.to_owned() }
            .web_url(AppEnvironment::current())
            .map(|url| url.absolute_string().to_owned())
    }
}
