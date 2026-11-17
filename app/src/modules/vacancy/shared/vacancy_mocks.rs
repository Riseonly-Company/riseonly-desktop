use std::collections::HashSet;

use gpui::{App, Context, Entity};
use rise_platform::user_defaults::{DefaultsValue, UserDefaults};
use rise_platform::user_defaults_keys::{VACANCY_DEFAULT_RESUME, VACANCY_VIEWED};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::vacancy::stores::vacancy_guide_sandbox_hooks::is_practice_job_id;

/// local state of the vacancies, viewed ids and the default resume
pub struct VacancyLocalState {
    viewed_ids: HashSet<i64>,
    loaded_viewed_key: Option<String>,
}

impl VacancyLocalState {
    /// the shared vacancy local state
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        let mut state = Self { viewed_ids: HashSet::new(), loaded_viewed_key: None };
        state.load_account_state();
        state
    }

    /// ids of vacancies already viewed
    pub fn viewed_ids(&self) -> &HashSet<i64> {
        &self.viewed_ids
    }

    pub fn mark_viewed(&mut self, id: i64, cx: &mut Context<Self>) {
        if is_practice_job_id(id) {
            return;
        }
        self.activate_account_state(cx);
        if self.viewed_ids.contains(&id) {
            return;
        }
        self.viewed_ids.insert(id);
        let stored: Vec<i64> = self.viewed_ids.iter().copied().collect();
        UserDefaults::standard().set(&Self::viewed_key(), stored);
        cx.notify();
    }

    /// default resume id, zero in the store means none
    pub fn default_resume_id(&mut self, cx: &mut Context<Self>) -> Option<i64> {
        self.activate_account_state(cx);
        let value = UserDefaults::standard().integer(&Self::default_resume_key());
        (value > 0).then_some(value)
    }

    pub fn set_default_resume_id(&mut self, id: i64, cx: &mut Context<Self>) {
        self.activate_account_state(cx);
        UserDefaults::standard().set(&Self::default_resume_key(), id);
    }

    pub fn clear_default_resume_id(&mut self, cx: &mut Context<Self>) {
        self.activate_account_state(cx);
        UserDefaults::standard().remove_object(&Self::default_resume_key());
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.viewed_ids.clear();
        self.loaded_viewed_key = None;
        cx.notify();
    }

    /// switches to the active accounts state
    pub fn activate_account_state(&mut self, cx: &mut Context<Self>) {
        if self.load_account_state() {
            cx.notify();
        }
    }

    pub fn remove_persisted_account(&mut self, user_id: &str, _cx: &mut Context<Self>) {
        let defaults = UserDefaults::standard();
        defaults.remove_object(&Self::scoped_key(VACANCY_VIEWED, user_id));
        defaults.remove_object(&Self::scoped_key(VACANCY_DEFAULT_RESUME, user_id));
    }

    fn load_account_state(&mut self) -> bool {
        let key = Self::viewed_key();
        if self.loaded_viewed_key.as_deref() == Some(key.as_str()) {
            return false;
        }
        self.viewed_ids = UserDefaults::standard()
            .array(&key)
            .map(|values| values.iter().filter_map(DefaultsValue::as_i64).collect())
            .unwrap_or_default();
        self.loaded_viewed_key = Some(key);
        true
    }

    fn viewed_key() -> String {
        SaiAccountCacheScope::scoped_key(VACANCY_VIEWED)
    }

    fn default_resume_key() -> String {
        SaiAccountCacheScope::scoped_key(VACANCY_DEFAULT_RESUME)
    }

    fn scoped_key(logical_key: &str, user_id: &str) -> String {
        SaiAccountCacheScope::prefix(user_id) + logical_key
    }
}
