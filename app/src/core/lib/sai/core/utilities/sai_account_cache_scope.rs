use std::sync::LazyLock;

use parking_lot::{Mutex, ReentrantMutex};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{SAI_ACTIVE_ACCOUNT, SAI_ACTIVE_ACCOUNT_EPOCH};

const ACCOUNT_PREFIX: &str = "account:";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Snapshot {
    user_id: Option<String>,
    epoch: Option<String>,
}

static SNAPSHOT: LazyLock<Mutex<Option<Snapshot>>> = LazyLock::new(|| Mutex::new(None));
static WRITE_LOCK: LazyLock<ReentrantMutex<()>> = LazyLock::new(|| ReentrantMutex::new(()));

pub enum SaiAccountCacheScope {}

impl SaiAccountCacheScope {
    pub fn activate(user_id: &str) {
        let user_id = trimming_whitespaces_and_newlines(user_id);
        if user_id.is_empty() {
            Self::clear_active_account();
            return;
        }
        let _write = WRITE_LOCK.lock();
        let active = Self::current();
        let rotates_epoch = active.user_id.as_deref() != Some(user_id) || active.epoch.is_none();
        let epoch = if rotates_epoch {
            Some(Uuid::new_v4().uuid_string())
        } else {
            active.epoch
        };
        let next = Snapshot {
            user_id: Some(user_id.to_owned()),
            epoch: epoch.clone(),
        };
        *SNAPSHOT.lock() = Some(next);
        let defaults = UserDefaults::standard();
        if rotates_epoch && let Some(epoch) = epoch {
            defaults.set(SAI_ACTIVE_ACCOUNT_EPOCH, epoch);
        }
        defaults.set(SAI_ACTIVE_ACCOUNT, user_id);
    }

    pub fn clear_active_account() {
        let _write = WRITE_LOCK.lock();
        *SNAPSHOT.lock() = Some(Snapshot::default());
        let defaults = UserDefaults::standard();
        defaults.remove_object(SAI_ACTIVE_ACCOUNT);
        defaults.remove_object(SAI_ACTIVE_ACCOUNT_EPOCH);
    }

    pub fn reload_from_defaults() {
        let _write = WRITE_LOCK.lock();
        *SNAPSHOT.lock() = None;
    }

    pub fn active_user_id() -> Option<String> {
        Self::current().user_id
    }

    pub fn active_epoch() -> Option<String> {
        Self::current().epoch
    }

    pub fn is_current_account(user_id: Option<&str>) -> bool {
        Self::normalized(user_id) == Self::current().user_id
    }

    pub fn is_current_account_epoch(user_id: Option<&str>, epoch: Option<&str>) -> bool {
        let active = Self::current();
        Self::normalized(user_id) == active.user_id && Self::normalized(epoch) == active.epoch
    }

    pub fn scoped_key(logical_key: &str) -> String {
        let key = trimming_whitespaces_and_newlines(logical_key);
        if key.is_empty() {
            return key.to_owned();
        }
        if key.starts_with(ACCOUNT_PREFIX) {
            return key.to_owned();
        }
        let Some(user_id) = Self::active_user_id() else {
            return key.to_owned();
        };
        Self::prefix(&user_id) + key
    }

    pub fn prefix(user_id: &str) -> String {
        format!("{ACCOUNT_PREFIX}{user_id}:")
    }

    pub fn logical_key(from_cache_key: &str) -> String {
        let Some(remainder) = from_cache_key.strip_prefix(ACCOUNT_PREFIX) else {
            return from_cache_key.to_owned();
        };
        let Some(separator) = remainder.find(':') else {
            return from_cache_key.to_owned();
        };
        remainder[separator + 1..].to_owned()
    }

    fn current() -> Snapshot {
        if let Some(cached) = SNAPSHOT.lock().clone() {
            return cached;
        }
        let defaults = UserDefaults::standard();
        let loaded = Snapshot {
            user_id: Self::normalized(defaults.string(SAI_ACTIVE_ACCOUNT).as_deref()),
            epoch: Self::normalized(defaults.string(SAI_ACTIVE_ACCOUNT_EPOCH).as_deref()),
        };
        let mut state = SNAPSHOT.lock();
        if let Some(state) = state.as_ref() {
            return state.clone();
        }
        *state = Some(loaded.clone());
        loaded
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        if value.is_empty() {
            None
        } else {
            Some(value.to_owned())
        }
    }
}
