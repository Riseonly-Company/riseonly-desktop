use std::collections::BTreeMap;

use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::NOTIFY_SETTINGS_OUTBOX_PREFIX;
use serde_json::{Map, Value};

/// notification settings changes the server hasnt acked yet
/// kept per account so a change right before closing is resent next launch
/// values are absolute so sending twice is harmless
pub enum RiseNotificationSettingsOutbox {}

impl RiseNotificationSettingsOutbox {
    fn defaults() -> UserDefaults {
        UserDefaults::standard()
    }

    fn key(user_id: &str) -> String {
        format!("{NOTIFY_SETTINGS_OUTBOX_PREFIX}{user_id}")
    }

    pub fn load(user_id: &str) -> BTreeMap<String, bool> {
        if user_id.is_empty() {
            return BTreeMap::new();
        }
        let Some(stored) = Self::defaults().dictionary(&Self::key(user_id)) else {
            return BTreeMap::new();
        };
        stored
            .into_iter()
            .map(|(key, value)| value.as_bool().map(|flag| (key, flag)))
            .collect::<Option<BTreeMap<String, bool>>>()
            .unwrap_or_default()
    }

    pub fn save(fields: &Map<String, Value>, user_id: &str) {
        let values: BTreeMap<String, bool> = fields
            .iter()
            .filter_map(|(key, value)| value.as_bool().map(|flag| (key.clone(), flag)))
            .collect();
        Self::save_values(values, user_id);
    }

    /// drops the fields an answered request carried unless a later change replaced them
    pub fn forget(sent: &Map<String, Value>, user_id: &str) {
        let mut values = Self::load(user_id);
        if values.is_empty() {
            return;
        }
        for (key, value) in sent {
            if values.get(key).copied() == value.as_bool() {
                values.remove(key);
            }
        }
        Self::save_values(values, user_id);
    }

    fn save_values(values: BTreeMap<String, bool>, user_id: &str) {
        if user_id.is_empty() {
            return;
        }
        let key = Self::key(user_id);
        if values.is_empty() {
            Self::defaults().remove_object(&key);
        } else {
            Self::defaults().set(&key, values);
        }
    }
}
