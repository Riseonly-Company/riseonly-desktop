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
    #[error("the secret is {size} bytes, more than the {limit} bytes the store can hold")]
    TooLarge { size: usize, limit: usize },
    #[error("an entry already exists for this key")]
    DuplicateItem,
    #[error("the account {0:?} is reserved by the store")]
    InvalidAccount(String),
}

/// why the os store cant keep a secret on this machine
/// the sign in screen turns code() into text and wont keep the session past this run
/// no variant falls back to a plain file, tokens are never stored in the clear
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SecretStoreUnavailable {
    NoService,
    Locked,
}

impl SecretStoreUnavailable {
    /// stable key the localized reason is looked up by
    pub const fn code(self) -> &'static str {
        match self {
            Self::NoService => "secret_store_no_service",
            Self::Locked => "secret_store_locked",
        }
    }
}

impl std::fmt::Display for SecretStoreUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NoService => "no OS secret store is running on this system",
            Self::Locked => "the OS secret store is locked and was not unlocked",
        })
    }
}

/// answer of an availability probe from the error of its first failing call
/// a refusal means the store is locked
/// anything else (no dbus name or session bus or backend failure) means no usable store
pub fn availability_from_probe(
    probe: Result<(), SecureStoreError>,
) -> Result<(), SecretStoreUnavailable> {
    match probe {
        Ok(()) => Ok(()),
        Err(SecureStoreError::AccessDenied) => Err(SecretStoreUnavailable::Locked),
        Err(_) => Err(SecretStoreUnavailable::NoService),
    }
}

pub const AUTH_SERVICE: &str = "riseonly.auth";
pub const ENGINE_SERVICE: &str = "net.riseonly.engine";
pub const SHARE_EXTENSION_SERVICE: &str = "riseonly.share-extension";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SecretAccessibility {
    AfterFirstUnlock,
    AfterFirstUnlockThisDeviceOnly,
}

impl SecretAccessibility {
    pub fn is_this_device_only(self) -> bool {
        matches!(self, Self::AfterFirstUnlockThisDeviceOnly)
    }
}

/// service and account of one item
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SecretKey {
    pub service: String,
    pub account: String,
}

impl SecretKey {
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }

    /// name the os store sees within the service, the account itself
    pub fn entry_name(&self) -> &str {
        &self.account
    }

    /// entry holding part index (1 based) of a generation of a secret split across entries
    /// accounts that start with the part prefix are refused so none can collide with a whole entry
    pub(crate) fn part_entry_name(&self, generation: u8, index: usize) -> String {
        format!("{PART_PREFIX}{generation}.{index}.{}", self.account)
    }

    pub(crate) fn validated(&self) -> Result<&Self, SecureStoreError> {
        if self.account.starts_with(PART_PREFIX) {
            return Err(SecureStoreError::InvalidAccount(self.account.clone()));
        }
        Ok(self)
    }
}

const PART_PREFIX: char = '#';

/// service name the os store sees for one namespace
pub fn namespaced_service(namespace: &str, service: &str) -> String {
    format!("{namespace}/{service}")
}

/// the secure item calls the stores make, on the os credential store
pub trait SecureStore: Send + Sync {
    /// updates the item, adds it with the given accessibility when missing
    fn set(
        &self,
        key: &SecretKey,
        value: &[u8],
        accessibility: SecretAccessibility,
    ) -> Result<(), SecureStoreError>;

    /// gets one item, a missing item is none and not an error
    fn get(&self, key: &SecretKey) -> Result<Option<Vec<u8>>, SecureStoreError>;

    /// deletes an item, deleting a missing one is not an error
    fn delete(&self, key: &SecretKey) -> Result<(), SecureStoreError>;

    fn is_available(&self) -> bool;

    /// is_available with the reason a caller shows when false
    /// default knows no more than the boolean, a store that can tell locked from missing overrides it
    fn availability(&self) -> Result<(), SecretStoreUnavailable> {
        if self.is_available() {
            Ok(())
        } else {
            Err(SecretStoreUnavailable::NoService)
        }
    }

    /// add only, an existing item stays and the call fails with DuplicateItem
    /// check and write are two calls, fine since one app instance runs per user
    fn add(
        &self,
        key: &SecretKey,
        value: &[u8],
        accessibility: SecretAccessibility,
    ) -> Result<(), SecureStoreError> {
        if self.get(key)?.is_some() {
            return Err(SecureStoreError::DuplicateItem);
        }
        self.set(key, value, accessibility)
    }
}

pub(crate) const MAX_ENTRY_BYTES: usize = 5 * 512;

const HEAD_MAGIC: [u8; 3] = [0x00, b'R', b'S'];
const HEAD_VERSION: u8 = 1;
const HEAD_LEN: usize = HEAD_MAGIC.len() + 3;
const MAX_ENTRIES: u8 = u8::MAX;
const HEAD_CAPACITY: usize = MAX_ENTRY_BYTES - HEAD_LEN;

pub(crate) const MAX_SECRET_BYTES: usize =
    HEAD_CAPACITY + (MAX_ENTRIES as usize - 1) * MAX_ENTRY_BYTES;

/// a secret laid out as os entries, each at most MAX_ENTRY_BYTES
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SplitSecret {
    pub head: Vec<u8>,
    pub parts: Vec<Vec<u8>>,
}

impl SplitSecret {
    pub fn entry_count(&self) -> usize {
        1 + self.parts.len()
    }
}

/// generation names the parts this layout is written under
pub(crate) fn split_secret(value: &[u8], generation: u8) -> Result<SplitSecret, SecureStoreError> {
    let too_large = || SecureStoreError::TooLarge {
        size: value.len(),
        limit: MAX_SECRET_BYTES,
    };
    if value.len() > MAX_SECRET_BYTES {
        return Err(too_large());
    }

    let first_len = value.len().min(HEAD_CAPACITY);
    let (first, rest) = value.split_at(first_len);
    let parts: Vec<Vec<u8>> = rest.chunks(MAX_ENTRY_BYTES).map(<[u8]>::to_vec).collect();
    let entries = u8::try_from(1 + parts.len()).map_err(|_| too_large())?;

    let mut head = Vec::with_capacity(HEAD_LEN + first.len());
    head.extend_from_slice(&HEAD_MAGIC);
    head.push(HEAD_VERSION);
    head.push(entries);
    head.push(generation);
    head.extend_from_slice(first);

    Ok(SplitSecret { head, parts })
}

/// what the head entry says about the rest of the secret
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StoredHead {
    pub first: Vec<u8>,
    pub entries: usize,
    pub generation: u8,
}

pub(crate) fn parse_head(head: &[u8]) -> Result<StoredHead, SecureStoreError> {
    if !head.starts_with(&HEAD_MAGIC) {
        return Err(SecureStoreError::Backend("secret head has no marker".into()));
    }
    if head.len() < HEAD_LEN {
        return Err(SecureStoreError::Backend("secret head is truncated".into()));
    }
    let version = head[HEAD_MAGIC.len()];
    if version != HEAD_VERSION {
        let message = format!("secret head has unknown version {version}");
        return Err(SecureStoreError::Backend(message));
    }
    let entries = usize::from(head[HEAD_MAGIC.len() + 1]);
    if entries == 0 {
        return Err(SecureStoreError::Backend("secret head counts no entries".into()));
    }
    Ok(StoredHead {
        first: head[HEAD_LEN..].to_vec(),
        entries,
        generation: head[HEAD_MAGIC.len() + 2],
    })
}

pub(crate) trait EntryBackend {
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, SecureStoreError>;
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), SecureStoreError>;
    /// removes an entry, removing a missing one is not an error
    fn remove(&self, name: &str) -> Result<(), SecureStoreError>;
}

/// the parts the stored head points at, as generation and entry count
/// unreadable or foreign head means no parts
fn stored_parts(backend: &impl EntryBackend, key: &SecretKey) -> Option<(u8, usize)> {
    let head = backend.read(key.entry_name()).ok()??;
    let stored = parse_head(&head).ok()?;
    Some((stored.generation, stored.entries))
}

fn remove_parts_from(
    backend: &impl EntryBackend,
    key: &SecretKey,
    generation: u8,
    from: usize,
) -> Result<(), SecureStoreError> {
    for index in from..usize::from(MAX_ENTRIES) {
        let name = key.part_entry_name(generation, index);
        if backend.read(&name)?.is_none() {
            break;
        }
        backend.remove(&name)?;
    }
    Ok(())
}

/// writes the new parts under the other generation names, then switches the head in one write
/// only after that the old parts are removed, so a crash keeps the old secret whole
pub(crate) fn write_split(
    backend: &impl EntryBackend,
    key: &SecretKey,
    value: &[u8],
) -> Result<(), SecureStoreError> {
    let key = key.validated()?;
    let previous = stored_parts(backend, key);
    let generation = previous.map_or(0, |(old, _)| old ^ 1);
    let split = split_secret(value, generation)?;

    for (offset, part) in split.parts.iter().enumerate() {
        backend.write(&key.part_entry_name(generation, offset + 1), part)?;
    }
    backend.write(key.entry_name(), &split.head)?;

    remove_parts_from(backend, key, generation, split.entry_count())?;
    if let Some((old, entries)) = previous {
        for index in 1..entries {
            backend.remove(&key.part_entry_name(old, index))?;
        }
    }
    Ok(())
}

pub(crate) fn read_split(
    backend: &impl EntryBackend,
    key: &SecretKey,
) -> Result<Option<Vec<u8>>, SecureStoreError> {
    let key = key.validated()?;
    let Some(head) = backend.read(key.entry_name())? else {
        return Ok(None);
    };

    let StoredHead {
        first,
        entries,
        generation,
    } = parse_head(&head)?;
    let mut value = first;
    for index in 1..entries {
        let missing =
            || SecureStoreError::Backend(format!("secret part {index}/{entries} is missing"));
        let name = key.part_entry_name(generation, index);
        let part = backend.read(&name)?.ok_or_else(missing)?;
        value.extend_from_slice(&part);
    }
    Ok(Some(value))
}

/// head goes first, so the secret reads as gone even if a part removal fails
pub(crate) fn delete_split(
    backend: &impl EntryBackend,
    key: &SecretKey,
) -> Result<(), SecureStoreError> {
    let key = key.validated()?;
    let parts = stored_parts(backend, key);
    backend.remove(key.entry_name())?;
    if let Some((generation, entries)) = parts {
        for index in 1..entries {
            backend.remove(&key.part_entry_name(generation, index))?;
        }
    }
    Ok(())
}

/// for tests and for linux with no secret service, contents are lost on exit
#[derive(Default)]
pub struct InMemorySecureStore {
    entries: parking_lot::Mutex<std::collections::HashMap<SecretKey, StoredSecret>>,
}

struct StoredSecret {
    value: Vec<u8>,
    accessibility: SecretAccessibility,
}

impl InMemorySecureStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// accessibility flag of a stored item, for tests that must see ThisDeviceOnly
    pub fn stored_accessibility(&self, key: &SecretKey) -> Option<SecretAccessibility> {
        self.entries.lock().get(key).map(|stored| stored.accessibility)
    }
}

impl SecureStore for InMemorySecureStore {
    fn set(
        &self,
        key: &SecretKey,
        value: &[u8],
        accessibility: SecretAccessibility,
    ) -> Result<(), SecureStoreError> {
        let key = key.validated()?;
        let stored = StoredSecret {
            value: value.to_vec(),
            accessibility,
        };
        self.entries.lock().insert(key.clone(), stored);
        Ok(())
    }

    fn get(&self, key: &SecretKey) -> Result<Option<Vec<u8>>, SecureStoreError> {
        let key = key.validated()?;
        Ok(self.entries.lock().get(key).map(|stored| stored.value.clone()))
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecureStoreError> {
        let key = key.validated()?;
        self.entries.lock().remove(key);
        Ok(())
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AFTER_FIRST_UNLOCK: SecretAccessibility = SecretAccessibility::AfterFirstUnlock;
    const THIS_DEVICE_ONLY: SecretAccessibility =
        SecretAccessibility::AfterFirstUnlockThisDeviceOnly;

    #[test]
    fn a_refused_probe_is_a_locked_store_and_any_other_failure_is_no_store() {
        assert_eq!(availability_from_probe(Ok(())), Ok(()));
        assert_eq!(
            availability_from_probe(Err(SecureStoreError::AccessDenied)),
            Err(SecretStoreUnavailable::Locked)
        );
        for failure in [
            SecureStoreError::Unavailable,
            SecureStoreError::Backend("org.freedesktop.DBus.Error.ServiceUnknown".into()),
            SecureStoreError::NotFound,
        ] {
            assert_eq!(
                availability_from_probe(Err(failure)),
                Err(SecretStoreUnavailable::NoService)
            );
        }
    }

    #[test]
    fn every_unavailable_reason_has_its_own_code() {
        let codes = [
            SecretStoreUnavailable::NoService.code(),
            SecretStoreUnavailable::Locked.code(),
        ];
        assert_ne!(codes[0], codes[1]);
        assert!(codes.iter().all(|code| code.starts_with("secret_store_")));
    }

    struct Unreachable;

    impl SecureStore for Unreachable {
        fn set(&self, _: &SecretKey, _: &[u8], _: SecretAccessibility) -> Result<(), SecureStoreError> {
            Err(SecureStoreError::Unavailable)
        }
        fn get(&self, _: &SecretKey) -> Result<Option<Vec<u8>>, SecureStoreError> {
            Err(SecureStoreError::Unavailable)
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecureStoreError> {
            Err(SecureStoreError::Unavailable)
        }
        fn is_available(&self) -> bool {
            false
        }
    }

    #[test]
    fn the_default_availability_follows_is_available() {
        assert_eq!(InMemorySecureStore::new().availability(), Ok(()));
        assert_eq!(Unreachable.availability(), Err(SecretStoreUnavailable::NoService));
    }

    #[test]
    fn the_same_account_under_two_services_is_two_items() {
        let store = InMemorySecureStore::new();
        let auth = SecretKey::new(AUTH_SERVICE, "u1");
        let engine = SecretKey::new(ENGINE_SERVICE, "u1");

        store.set(&auth, b"token", AFTER_FIRST_UNLOCK).unwrap();
        store.set(&engine, b"db-key", THIS_DEVICE_ONLY).unwrap();

        assert_eq!(store.get(&auth).unwrap(), Some(b"token".to_vec()));
        assert_eq!(store.get(&engine).unwrap(), Some(b"db-key".to_vec()));
    }

    #[test]
    fn the_ios_service_names_are_distinct() {
        assert_ne!(AUTH_SERVICE, ENGINE_SERVICE);
        assert_ne!(AUTH_SERVICE, SHARE_EXTENSION_SERVICE);
        assert_ne!(ENGINE_SERVICE, SHARE_EXTENSION_SERVICE);
    }

    #[test]
    fn environments_get_separate_os_services() {
        assert_ne!(
            namespaced_service("net.riseonly.desktop.dev", AUTH_SERVICE),
            namespaced_service("net.riseonly.desktop", AUTH_SERVICE),
            "a dev build must not read production tokens"
        );
    }

    #[test]
    fn only_this_device_only_is_machine_bound() {
        assert!(THIS_DEVICE_ONLY.is_this_device_only());
        assert!(!AFTER_FIRST_UNLOCK.is_this_device_only());
    }

    #[test]
    fn a_stored_secret_reads_back_with_its_accessibility() {
        let store = InMemorySecureStore::new();
        let key = SecretKey::new(ENGINE_SERVICE, "u1");

        store.set(&key, b"secret", THIS_DEVICE_ONLY).unwrap();
        assert_eq!(store.get(&key).unwrap(), Some(b"secret".to_vec()));
        assert_eq!(store.stored_accessibility(&key), Some(THIS_DEVICE_ONLY));
    }

    #[test]
    fn a_missing_secret_reads_as_none_rather_than_an_error() {
        let store = InMemorySecureStore::new();
        let key = SecretKey::new(ENGINE_SERVICE, "absent");
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn deleting_is_idempotent() {
        let store = InMemorySecureStore::new();
        let key = SecretKey::new(AUTH_SERVICE, "tokens");

        store.set(&key, b"token", AFTER_FIRST_UNLOCK).unwrap();
        store.delete(&key).unwrap();
        store.delete(&key).unwrap();

        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn one_accounts_secret_is_not_readable_under_another() {
        let store = InMemorySecureStore::new();
        let mine = SecretKey::new(ENGINE_SERVICE, "u1");
        store.set(&mine, b"mine", THIS_DEVICE_ONLY).unwrap();

        assert_eq!(store.get(&SecretKey::new(ENGINE_SERVICE, "u2")).unwrap(), None);
    }

    #[test]
    fn add_refuses_an_existing_item_and_keeps_it() {
        let store = InMemorySecureStore::new();
        let key = SecretKey::new(ENGINE_SERVICE, "u1");

        store.add(&key, b"winner", THIS_DEVICE_ONLY).unwrap();
        let second = store.add(&key, b"loser", THIS_DEVICE_ONLY);

        assert!(matches!(second, Err(SecureStoreError::DuplicateItem)));
        assert_eq!(store.get(&key).unwrap(), Some(b"winner".to_vec()));
    }

    #[test]
    fn an_account_in_the_part_format_is_refused() {
        let store = InMemorySecureStore::new();
        let key = SecretKey::new(AUTH_SERVICE, "#1.tokens");

        let error = store.set(&key, b"x", AFTER_FIRST_UNLOCK).unwrap_err();
        assert!(matches!(error, SecureStoreError::InvalidAccount(_)));
    }

    /// test backend that refuses a write over MAX_ENTRY_BYTES like credential manager
    #[derive(Default)]
    struct CappedEntries {
        entries: parking_lot::Mutex<std::collections::HashMap<String, Vec<u8>>>,
        refuse_heads: std::sync::atomic::AtomicBool,
    }

    impl EntryBackend for CappedEntries {
        fn read(&self, name: &str) -> Result<Option<Vec<u8>>, SecureStoreError> {
            Ok(self.entries.lock().get(name).cloned())
        }

        fn write(&self, name: &str, bytes: &[u8]) -> Result<(), SecureStoreError> {
            if bytes.len() > MAX_ENTRY_BYTES {
                return Err(SecureStoreError::TooLarge {
                    size: bytes.len(),
                    limit: MAX_ENTRY_BYTES,
                });
            }
            let is_head = !name.starts_with(PART_PREFIX);
            if is_head && self.refuse_heads.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(SecureStoreError::Backend("stopped before the head".into()));
            }
            self.entries.lock().insert(name.to_owned(), bytes.to_vec());
            Ok(())
        }

        fn remove(&self, name: &str) -> Result<(), SecureStoreError> {
            self.entries.lock().remove(name);
            Ok(())
        }
    }

    fn bytes(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn a_secret_that_fits_one_entry_stays_one_entry() {
        assert_eq!(split_secret(&bytes(HEAD_CAPACITY), 0).unwrap().entry_count(), 1);
        assert_eq!(split_secret(&[], 0).unwrap().entry_count(), 1);
    }

    #[test]
    fn one_byte_past_the_head_capacity_takes_a_second_entry() {
        let split = split_secret(&bytes(HEAD_CAPACITY + 1), 0).unwrap();
        assert_eq!(split.entry_count(), 2);
        assert_eq!(split.parts[0].len(), 1);
    }

    #[test]
    fn no_entry_exceeds_the_credential_manager_blob_limit() {
        for len in [0, 960, 961, 2560, 2561, 6000, MAX_SECRET_BYTES] {
            let split = split_secret(&bytes(len), 0).unwrap();
            assert!(split.head.len() <= MAX_ENTRY_BYTES, "head at {len}");
            assert!(split.parts.iter().all(|p| p.len() <= MAX_ENTRY_BYTES), "part at {len}");
        }
    }

    #[test]
    fn a_secret_past_the_split_limit_is_refused_before_anything_is_written() {
        let backend = CappedEntries::default();
        let key = SecretKey::new(AUTH_SERVICE, "tokens");

        let error = write_split(&backend, &key, &bytes(MAX_SECRET_BYTES + 1)).unwrap_err();
        assert!(matches!(error, SecureStoreError::TooLarge { .. }));
        assert!(backend.entries.lock().is_empty());
    }

    #[test]
    fn a_token_pair_larger_than_one_blob_round_trips() {
        let backend = CappedEntries::default();
        let key = SecretKey::new(AUTH_SERVICE, "tokens");
        let secret = bytes(4 * 1024);

        write_split(&backend, &key, &secret).unwrap();
        assert_eq!(read_split(&backend, &key).unwrap(), Some(secret));
    }

    #[test]
    fn every_length_around_the_boundaries_round_trips() {
        let backend = CappedEntries::default();
        let key = SecretKey::new(ENGINE_SERVICE, "u1");
        let edge = HEAD_CAPACITY + MAX_ENTRY_BYTES;

        for len in [0, 1, HEAD_CAPACITY - 1, HEAD_CAPACITY, HEAD_CAPACITY + 1, edge, edge + 1] {
            let secret = bytes(len);
            write_split(&backend, &key, &secret).unwrap();
            assert_eq!(read_split(&backend, &key).unwrap(), Some(secret), "length {len}");
        }
    }

    #[test]
    fn shrinking_a_secret_removes_the_parts_it_no_longer_needs() {
        let backend = CappedEntries::default();
        let key = SecretKey::new(AUTH_SERVICE, "tokens");

        write_split(&backend, &key, &bytes(3 * MAX_ENTRY_BYTES)).unwrap();
        write_split(&backend, &key, b"short").unwrap();

        assert_eq!(backend.entries.lock().len(), 1);
        assert_eq!(read_split(&backend, &key).unwrap(), Some(b"short".to_vec()));
    }

    #[test]
    fn a_write_stopped_before_its_head_keeps_the_old_secret_whole() {
        let backend = CappedEntries::default();
        let key = SecretKey::new(AUTH_SERVICE, "tokens");
        let old = bytes(3 * MAX_ENTRY_BYTES);
        write_split(&backend, &key, &old).unwrap();

        backend
            .refuse_heads
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let new: Vec<u8> = bytes(3 * MAX_ENTRY_BYTES).iter().map(|b| b ^ 0xFF).collect();
        assert!(write_split(&backend, &key, &new).is_err());
        assert_eq!(read_split(&backend, &key).unwrap(), Some(old));

        backend
            .refuse_heads
            .store(false, std::sync::atomic::Ordering::SeqCst);
        write_split(&backend, &key, &new).unwrap();
        assert_eq!(read_split(&backend, &key).unwrap(), Some(new));
        assert_eq!(backend.entries.lock().len(), 4);
    }

    #[test]
    fn rewrites_alternate_generations_and_leave_no_stale_parts() {
        let backend = CappedEntries::default();
        let key = SecretKey::new(ENGINE_SERVICE, "u1");

        for len in [3 * MAX_ENTRY_BYTES, 2 * MAX_ENTRY_BYTES, 4 * MAX_ENTRY_BYTES, 10] {
            let secret = bytes(len);
            write_split(&backend, &key, &secret).unwrap();
            assert_eq!(read_split(&backend, &key).unwrap(), Some(secret), "length {len}");
            let expected = split_secret(&bytes(len), 0).unwrap().entry_count();
            assert_eq!(backend.entries.lock().len(), expected, "length {len}");
        }
    }

    #[test]
    fn deleting_a_split_secret_removes_every_part() {
        let backend = CappedEntries::default();
        let key = SecretKey::new(AUTH_SERVICE, "tokens");

        write_split(&backend, &key, &bytes(3 * MAX_ENTRY_BYTES)).unwrap();
        delete_split(&backend, &key).unwrap();

        assert!(backend.entries.lock().is_empty());
        assert_eq!(read_split(&backend, &key).unwrap(), None);
    }

    #[test]
    fn a_missing_part_is_an_error_rather_than_a_short_secret() {
        let backend = CappedEntries::default();
        let key = SecretKey::new(AUTH_SERVICE, "tokens");

        write_split(&backend, &key, &bytes(2 * MAX_ENTRY_BYTES)).unwrap();
        backend.remove(&key.part_entry_name(0, 1)).unwrap();

        assert!(matches!(
            read_split(&backend, &key),
            Err(SecureStoreError::Backend(_))
        ));
    }

    #[test]
    fn an_entry_without_the_head_marker_is_refused() {
        let backend = CappedEntries::default();
        let key = SecretKey::new(ENGINE_SERVICE, "u1");
        backend
            .entries
            .lock()
            .insert(key.entry_name().to_owned(), b"ZGIta2V5".to_vec());

        assert!(matches!(
            read_split(&backend, &key),
            Err(SecureStoreError::Backend(_))
        ));
    }

    #[test]
    fn a_head_with_an_unknown_version_is_refused() {
        let mut head = split_secret(b"x", 0).unwrap().head;
        head[HEAD_MAGIC.len()] = HEAD_VERSION + 1;
        assert!(parse_head(&head).is_err());
    }

    #[test]
    fn part_entries_never_collide_with_whole_entries() {
        let key = SecretKey::new(AUTH_SERVICE, "tokens");

        assert_ne!(key.part_entry_name(0, 1), key.entry_name());
        assert_ne!(key.part_entry_name(0, 1), key.part_entry_name(1, 1));
        assert!(SecretKey::new(AUTH_SERVICE, key.part_entry_name(0, 1)).validated().is_err());
        assert!(SecretKey::new(AUTH_SERVICE, "1.tokens").validated().is_ok());
    }

    #[test]
    fn overwriting_replaces_rather_than_appends() {
        let store = InMemorySecureStore::new();
        let key = SecretKey::new(ENGINE_SERVICE, "u1");

        store.set(&key, b"first", THIS_DEVICE_ONLY).unwrap();
        store.set(&key, b"second", THIS_DEVICE_ONLY).unwrap();

        assert_eq!(store.get(&key).unwrap(), Some(b"second".to_vec()));
    }
}
