use std::collections::{HashMap, HashSet};

use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_theme::{RiseThemeAppearance, RiseThemePlatform, RiseThemePreferences};

#[derive(Clone, Debug, PartialEq, Eq, Hash, thiserror::Error)]
pub enum RiseUserSettingsError {
    #[error("missingActiveSession")]
    MissingActiveSession,
    #[error("staleAccount")]
    StaleAccount,
    #[error("invalidResponse")]
    InvalidResponse,
    #[error("rejected({0})")]
    Rejected(String),
}

pub enum RiseUserThemeMutationFailurePolicy {}

impl RiseUserThemeMutationFailurePolicy {
    pub fn requires_authoritative_refresh(after: &anyhow::Error) -> bool {
        after.downcast_ref::<CancellationError>().is_none()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseUserThemeSettingsCacheSource {
    Memory,
    PersistentStore,
    Miss,
}

impl RiseUserThemeSettingsCacheSource {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::PersistentStore => "persistent-store",
            Self::Miss => "miss",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        [Self::Memory, Self::PersistentStore, Self::Miss].into_iter().find(|source| source.raw_value() == raw)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserThemeSettingsRepositorySnapshot {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub revision: i64,
    pub preferences: RiseThemePreferences,
    pub has_value: bool,
    pub cache_source: RiseUserThemeSettingsCacheSource,
    pub is_refreshing: bool,
    pub mutating_platforms: HashSet<RiseThemePlatform>,
    pub error_code: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RiseUserThemeMutationIntent {
    pub platform: RiseThemePlatform,
    pub appearance: Option<RiseThemeAppearance>,
    pub generation: u64,
    pub order: u64,
    pub is_ready: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseUserThemeMutationBuffer {
    pending: HashMap<RiseThemePlatform, RiseUserThemeMutationIntent>,
    in_flight: Option<RiseUserThemeMutationIntent>,
    generations: HashMap<RiseThemePlatform, u64>,
    order: u64,
}

impl RiseUserThemeMutationBuffer {
    pub fn pending(&self) -> &HashMap<RiseThemePlatform, RiseUserThemeMutationIntent> {
        &self.pending
    }

    pub fn in_flight(&self) -> Option<RiseUserThemeMutationIntent> {
        self.in_flight
    }

    pub fn is_mutating(&self) -> bool {
        self.in_flight.is_some() || !self.pending.is_empty()
    }

    pub fn mutating_platforms(&self) -> HashSet<RiseThemePlatform> {
        let mut result: HashSet<RiseThemePlatform> = self.pending.keys().copied().collect();
        if let Some(in_flight) = self.in_flight {
            result.insert(in_flight.platform);
        }
        result
    }

    pub fn enqueue(
        &mut self,
        platform: RiseThemePlatform,
        appearance: Option<RiseThemeAppearance>,
    ) -> RiseUserThemeMutationIntent {
        let generation = self.generations.entry(platform).or_insert(0);
        *generation = generation.wrapping_add(1);
        self.order = self.order.wrapping_add(1);
        let intent = RiseUserThemeMutationIntent {
            platform,
            appearance,
            generation: self.generations.get(&platform).copied().unwrap_or(0),
            order: self.order,
            is_ready: false,
        };
        self.pending.insert(platform, intent);
        intent
    }

    pub fn mark_ready(&mut self, intent: &RiseUserThemeMutationIntent) -> bool {
        let Some(current) = self.pending.get_mut(&intent.platform) else {
            return false;
        };
        if current.generation != intent.generation {
            return false;
        }
        current.is_ready = true;
        true
    }

    pub fn cancel_pending(&mut self, intent: &RiseUserThemeMutationIntent) -> bool {
        if self.pending.get(&intent.platform).map(|pending| pending.generation) != Some(intent.generation) {
            return false;
        }
        self.pending.remove(&intent.platform);
        true
    }

    pub fn begin_next(&mut self) -> Option<RiseUserThemeMutationIntent> {
        if self.in_flight.is_some() {
            return None;
        }
        let next = self.pending.values().filter(|intent| intent.is_ready).min_by_key(|intent| intent.order).copied()?;
        self.pending.remove(&next.platform);
        self.in_flight = Some(next);
        Some(next)
    }

    pub fn finish(&mut self, intent: &RiseUserThemeMutationIntent) -> bool {
        let Some(in_flight) = self.in_flight else {
            return false;
        };
        if in_flight.platform != intent.platform || in_flight.generation != intent.generation {
            return false;
        }
        self.in_flight = None;
        true
    }

    pub fn discard_intents_satisfied(&mut self, by: &RiseThemePreferences) -> HashSet<RiseThemePlatform> {
        let platforms: HashSet<RiseThemePlatform> = self
            .pending
            .iter()
            .filter(|(platform, intent)| by.appearance(**platform) == intent.appearance)
            .map(|(platform, _)| *platform)
            .collect();
        for platform in &platforms {
            self.pending.remove(platform);
        }
        platforms
    }

    pub fn applying_optimistic(&self, to: &RiseThemePreferences) -> RiseThemePreferences {
        let mut result = *to;
        if let Some(in_flight) = self.in_flight {
            result = result.setting(in_flight.appearance, in_flight.platform);
        }
        let mut ordered: Vec<&RiseUserThemeMutationIntent> = self.pending.values().collect();
        ordered.sort_by_key(|intent| intent.order);
        for intent in ordered {
            result = result.setting(intent.appearance, intent.platform);
        }
        result
    }
}
