use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde::{Deserialize, Serialize};

use crate::modules::auth::services::keychain_store::{KeychainStore, KeychainStoreKeychainError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushOwnershipRegistrationContext {
    pub user_id: String,
    pub generation: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushOwnershipGenerationDecision {
    pub context: PushOwnershipRegistrationContext,
    pub high_water_to_persist: Option<i64>,
}

pub enum PushOwnershipGenerationPolicy {}

impl PushOwnershipGenerationPolicy {
    pub fn registration(
        user_id: &str,
        current: Option<&PushOwnershipRegistrationContext>,
        persisted_high_water: i64,
        now_milliseconds: i64,
    ) -> Option<PushOwnershipGenerationDecision> {
        let normalized_user_id = trimming_whitespaces_and_newlines(user_id);
        if normalized_user_id.is_empty() {
            return None;
        }

        if let Some(current) = current
            && current.user_id == normalized_user_id
            && current.generation > 0
            && current.generation >= persisted_high_water.max(0)
        {
            return Some(PushOwnershipGenerationDecision {
                context: current.clone(),
                high_water_to_persist: None,
            });
        }

        let current_generation = current.map_or(0, |current| current.generation);
        let baseline = persisted_high_water.max(current_generation).max(0);
        let incremented = baseline.checked_add(1)?;

        let generation = now_milliseconds.max(incremented).max(1);
        Some(PushOwnershipGenerationDecision {
            context: PushOwnershipRegistrationContext {
                user_id: normalized_user_id.to_owned(),
                generation,
            },
            high_water_to_persist: Some(generation),
        })
    }
}

#[derive(Serialize, Deserialize)]
struct PushOwnershipGenerationStorePayload {
    #[serde(rename = "highWater")]
    high_water: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum PushOwnershipGenerationStoreError {
    #[error(transparent)]
    Encoding(#[from] serde_json::Error),
    #[error(transparent)]
    Keychain(#[from] KeychainStoreKeychainError),
}

const KEY: &str = "riseonly.push.ownership_generation.v1";

pub enum PushOwnershipGenerationStore {}

impl PushOwnershipGenerationStore {
    pub fn load_high_water() -> i64 {
        KeychainStore::data(KEY)
            .and_then(|data| {
                serde_json::from_slice::<PushOwnershipGenerationStorePayload>(&data).ok()
            })
            .map_or(0, |payload| payload.high_water.max(0))
    }

    pub fn persist_high_water(high_water: i64) -> Result<(), PushOwnershipGenerationStoreError> {
        if high_water <= 0 {
            return Ok(());
        }
        let data = serde_json::to_vec(&PushOwnershipGenerationStorePayload { high_water })?;
        KeychainStore::set_data(&data, KEY)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "push_ownership_generation_tests.rs"]
mod tests;
