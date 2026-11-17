use std::collections::HashSet;

use gpui::{App, Context, Entity};
use rise_canon::RisePrivacyRule;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::user::engine::core::rise_user_achievement_wire_models::RiseAchievement;
use crate::modules::user::engine::core::rise_user_achievements_resource_repository::RiseUserAchievementsCacheSource;

/// load state lives in the snapshot not in a transport object in the view
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseUserAchievementsLoadState {
    Idle,
    Loading,
    Loaded,
    Failed,
}

/// another users showcase,
/// memory only and bounded. never merges into or persists over the accounts own list
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserAchievementsShowcase {
    pub user_id: String,
    pub achievements: Vec<RiseAchievement>,
    pub total: i64,
    pub is_visible: bool,
    pub load_state: RiseUserAchievementsLoadState,
    pub error: Option<String>,
}

impl RiseUserAchievementsShowcase {
    pub fn id(&self) -> &str {
        &self.user_id
    }

    pub fn empty(user_id: &str) -> RiseUserAchievementsShowcase {
        RiseUserAchievementsShowcase {
            user_id: user_id.to_owned(),
            achievements: Vec::new(),
            total: 0,
            is_visible: false,
            load_state: RiseUserAchievementsLoadState::Idle,
            error: None,
        }
    }
}

/// immutable bounded account gated ui state
/// visible and hidden lists are derived once per publish
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserAchievementsSnapshot {
    pub achievements: Vec<RiseAchievement>,
    pub visible_achievements: Vec<RiseAchievement>,
    pub hidden_achievements: Vec<RiseAchievement>,
    pub total: i64,
    pub hidden_count: i64,
    pub catalog_total: i64,
    pub achievements_rule: String,
    pub load_state: RiseUserAchievementsLoadState,
    pub cache_source: RiseUserAchievementsCacheSource,
    pub is_refreshing: bool,
    pub mutating_ids: HashSet<i64>,
    pub showcases: Vec<RiseUserAchievementsShowcase>,
    pub error: Option<String>,
    pub mutation_error: Option<String>,
}

impl RiseUserAchievementsSnapshot {
    pub fn empty() -> RiseUserAchievementsSnapshot {
        RiseUserAchievementsSnapshot {
            achievements: Vec::new(),
            visible_achievements: Vec::new(),
            hidden_achievements: Vec::new(),
            total: 0,
            hidden_count: 0,
            catalog_total: 0,
            achievements_rule: RisePrivacyRule::Achievements.default_audience().raw_value().to_owned(),
            load_state: RiseUserAchievementsLoadState::Idle,
            cache_source: RiseUserAchievementsCacheSource::Miss,
            is_refreshing: false,
            mutating_ids: HashSet::new(),
            showcases: Vec::new(),
            error: None,
            mutation_error: None,
        }
    }

    pub fn has_content(&self) -> bool {
        !self.achievements.is_empty()
    }

    pub fn earned_count(&self) -> i64 {
        let loaded = i64::try_from(self.achievements.len()).unwrap_or(i64::MAX);
        0.max(self.total.max(loaded))
    }

    pub fn progress_total(&self) -> i64 {
        self.catalog_total.max(self.earned_count())
    }

    pub fn unseen_ids(&self) -> Vec<i64> {
        self.achievements
            .iter()
            .filter(|achievement| !achievement.is_seen())
            .map(|achievement| achievement.id)
            .collect()
    }

    pub fn showcase(&self, user_id: &str) -> Option<&RiseUserAchievementsShowcase> {
        self.showcases.iter().find(|showcase| showcase.user_id == user_id)
    }

    pub fn is_mutating(&self, achievement_id: i64) -> bool {
        self.mutating_ids.contains(&achievement_id)
    }
}

/// immutable ui snapshot of the accounts achievements,
/// gated on account and session so a signed-out callback cant paint over
pub struct RiseUserAchievementsPresentation {
    snapshot: RiseUserAchievementsSnapshot,
    generation: u64,
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
}

impl RiseUserAchievementsPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: RiseUserAchievementsSnapshot::empty(),
            generation: 0,
            account_id: None,
            account_epoch: None,
            session_id: None,
        }
    }

    pub fn snapshot(&self) -> &RiseUserAchievementsSnapshot {
        &self.snapshot
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn activate(
        &mut self,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        if self.account_id.as_deref() == Some(account_id)
            && self.account_epoch.as_deref() == Some(account_epoch)
            && self.session_id == Some(session_id)
        {
            return;
        }
        self.account_id = Some(account_id.to_owned());
        self.account_epoch = Some(account_epoch.to_owned());
        self.session_id = Some(session_id);
        self.generation = self.generation.wrapping_add(1);
        self.snapshot = RiseUserAchievementsSnapshot::empty();
        cx.notify();
    }

    pub fn publish(
        &mut self,
        next: RiseUserAchievementsSnapshot,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.account_id.as_deref() != Some(account_id)
            || self.account_epoch.as_deref() != Some(account_epoch)
            || self.session_id != Some(session_id)
        {
            return false;
        }
        if next == self.snapshot {
            return true;
        }
        self.snapshot = next;
        cx.notify();
        true
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(account_id) || self.session_id != Some(session_id) {
            return;
        }
        self.account_id = None;
        self.account_epoch = None;
        self.session_id = None;
        self.generation = self.generation.wrapping_add(1);
        self.snapshot = RiseUserAchievementsSnapshot::empty();
        cx.notify();
    }
}
