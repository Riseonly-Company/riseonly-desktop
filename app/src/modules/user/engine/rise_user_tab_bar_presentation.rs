use std::collections::HashMap;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::TAB_BAR_SETTINGS_PREFIX;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::widgets::navigations::main_tab_bar_settings::{
    RiseTabBarPlatform, RiseTabBarSettings, RiseTabBarSettingsChange, RiseTabBarSettingsSet,
};
use crate::modules::user::engine::rise_user_tab_bar_settings_repository::RiseUserTabBarSnapshot;

#[derive(Clone, Copy)]
struct RiseUserTabBarPresentationTap {
    sequence: u64,
    settings: RiseTabBarSettings,
}

/// accounts bar as screens see it, side bar uses this device set
/// last shown set seeds it until the repo answers
pub struct RiseUserTabBarPresentation {
    settings: RiseTabBarSettingsSet,
    is_synced: bool,
    is_resolved: bool,
    published: RiseTabBarSettingsSet,
    taps: HashMap<RiseTabBarPlatform, RiseUserTabBarPresentationTap>,
    last_sequence: u64,
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
    seeded_account_id: Option<String>,
    stored_seed: Option<Vec<u8>>,
}

impl RiseUserTabBarPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        let account_id = SaiAccountCacheScope::active_user_id();
        let stored = account_id.as_deref().and_then(Self::seed);
        let seed = stored.unwrap_or_else(RiseTabBarSettingsSet::defaults);
        Self {
            settings: seed,
            is_synced: true,
            is_resolved: stored.is_some(),
            published: seed,
            taps: HashMap::new(),
            last_sequence: 0,
            account_id: None,
            account_epoch: None,
            session_id: None,
            seeded_account_id: account_id,
            stored_seed: None,
        }
    }

    pub fn settings(&self) -> RiseTabBarSettingsSet {
        self.settings
    }

    pub fn is_synced(&self) -> bool {
        self.is_synced
    }

    pub fn is_resolved(&self) -> bool {
        self.is_resolved
    }

    pub fn current(&self) -> RiseTabBarSettings {
        self.settings[RiseTabBarPlatform::CURRENT]
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
        self.taps.clear();
        self.set_synced(true, cx);
        if self.seeded_account_id.as_deref() != Some(account_id) {
            self.seeded_account_id = Some(account_id.to_owned());
            self.stored_seed = None;
            let stored = Self::seed(account_id);
            self.published = stored.unwrap_or_else(RiseTabBarSettingsSet::defaults);
            self.set_resolved(stored.is_some(), cx);
        }
        self.recompute(cx);
    }

    pub fn publish(
        &mut self,
        snapshot: &RiseUserTabBarSnapshot,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_current(account_id, account_epoch, session_id) {
            return false;
        }
        self.published = snapshot.settings;
        self.set_resolved(true, cx);
        self.set_synced(snapshot.is_synced, cx);
        self.taps.retain(|_, tap| tap.sequence > snapshot.applied_sequence);
        self.recompute(cx);
        true
    }

    pub fn give_up(
        &mut self,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current(account_id, account_epoch, session_id) {
            return;
        }
        self.set_resolved(true, cx);
    }

    /// a tap on the settings page, the number goes along so snapshots tell which taps they hold
    pub fn take(
        &mut self,
        change: RiseTabBarSettingsChange,
        platform: RiseTabBarPlatform,
        cx: &mut Context<Self>,
    ) -> u64 {
        self.last_sequence = self.last_sequence.wrapping_add(1);
        self.taps.insert(
            platform,
            RiseUserTabBarPresentationTap { sequence: self.last_sequence, settings: change.settings() },
        );
        self.recompute(cx);
        self.last_sequence
    }

    pub fn drop(&mut self, sequence: u64, cx: &mut Context<Self>) {
        let before = self.taps.len();
        self.taps.retain(|_, tap| tap.sequence != sequence);
        if self.taps.len() == before {
            return;
        }
        self.recompute(cx);
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(account_id) || self.session_id != Some(session_id) {
            return;
        }
        self.account_id = None;
        self.account_epoch = None;
        self.session_id = None;
        self.taps.clear();
        self.set_synced(true, cx);
    }

    /// account switch, drops the old domain and draws the new accounts seed before its domain opens
    pub fn activate_account_state(&mut self, cx: &mut Context<Self>) {
        let Some(account_id) = SaiAccountCacheScope::active_user_id() else {
            return;
        };
        if self.seeded_account_id.as_deref() == Some(account_id.as_str()) {
            return;
        }
        self.account_id = None;
        self.account_epoch = None;
        self.session_id = None;
        self.taps.clear();
        self.set_synced(true, cx);
        let stored = Self::seed(&account_id);
        self.seeded_account_id = Some(account_id);
        self.stored_seed = None;
        self.published = stored.unwrap_or_else(RiseTabBarSettingsSet::defaults);
        self.set_resolved(stored.is_some(), cx);
        self.recompute(cx);
    }

    fn is_current(&self, account_id: &str, account_epoch: &str, session_id: Uuid) -> bool {
        self.account_id.as_deref() == Some(account_id)
            && self.account_epoch.as_deref() == Some(account_epoch)
            && self.session_id == Some(session_id)
    }

    fn set_synced(&mut self, synced: bool, cx: &mut Context<Self>) {
        if self.is_synced != synced {
            self.is_synced = synced;
            cx.notify();
        }
    }

    fn set_resolved(&mut self, resolved: bool, cx: &mut Context<Self>) {
        if self.is_resolved != resolved {
            self.is_resolved = resolved;
            cx.notify();
        }
    }

    fn recompute(&mut self, cx: &mut Context<Self>) {
        let mut next = self.published;
        for (platform, tap) in &self.taps {
            next[*platform] = tap.settings;
        }
        if next != self.settings {
            self.settings = next;
            cx.notify();
        }
        self.store_seed(&next);
    }

    fn store_seed(&mut self, set: &RiseTabBarSettingsSet) {
        let Some(account_id) = self.account_id.as_deref() else {
            return;
        };
        if self.seeded_account_id.as_deref() != Some(account_id) {
            return;
        }
        let Ok(data) = serde_json::to_vec(set) else {
            return;
        };
        if self.stored_seed.as_ref() == Some(&data) {
            return;
        }
        UserDefaults::standard().set(&Self::seed_key(account_id), data.clone());
        self.stored_seed = Some(data);
    }

    fn seed(account_id: &str) -> Option<RiseTabBarSettingsSet> {
        UserDefaults::standard()
            .data(&Self::seed_key(account_id))
            .and_then(|data| serde_json::from_slice::<RiseTabBarSettingsSet>(&data).ok())
    }

    fn seed_key(account_id: &str) -> String {
        format!("{TAB_BAR_SETTINGS_PREFIX}{account_id}")
    }
}
