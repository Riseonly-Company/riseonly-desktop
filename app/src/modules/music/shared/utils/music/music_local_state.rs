use std::collections::HashSet;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{MUSIC_LOCAL_STATE_ACCOUNT_SCOPE, MUSIC_RECENT, MUSIC_SEARCHES};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::music::shared::utils::music::music_models::TrackModel;

const RECENTLY_PLAYED_LIMIT: usize = 20;
const RECENT_SEARCHES_LIMIT: usize = 12;

pub struct MusicLocalState {
    pub recently_played_ids: Vec<String>,
    pub recent_searches: Vec<String>,
    loaded_recent_key: Option<String>,
}

impl MusicLocalState {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| {
            let mut state = Self {
                recently_played_ids: Vec::new(),
                recent_searches: Vec::new(),
                loaded_recent_key: None,
            };
            state.load_account_state();
            state
        })
    }

    pub fn mark_played(&mut self, track: &TrackModel, cx: &mut Context<Self>) {
        self.load_account_state();
        self.recently_played_ids.retain(|id| *id != track.id);
        self.recently_played_ids.insert(0, track.id.clone());
        self.recently_played_ids.truncate(RECENTLY_PLAYED_LIMIT);
        UserDefaults::standard().set(&Self::recent_key(), self.recently_played_ids.clone());
        cx.notify();
    }

    pub fn remove_tracks(&mut self, track_ids: &HashSet<String>, cx: &mut Context<Self>) {
        self.load_account_state();
        if track_ids.is_empty() {
            cx.notify();
            return;
        }
        self.recently_played_ids.retain(|id| !track_ids.contains(id));
        UserDefaults::standard().set(&Self::recent_key(), self.recently_played_ids.clone());
        cx.notify();
    }

    pub fn add_search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.load_account_state();
        let trimmed = trimming_whitespaces(query);
        if trimmed.is_empty() {
            cx.notify();
            return;
        }
        let trimmed = trimmed.to_owned();
        self.recent_searches.retain(|search| *search != trimmed);
        self.recent_searches.insert(0, trimmed);
        self.recent_searches.truncate(RECENT_SEARCHES_LIMIT);
        UserDefaults::standard().set(&Self::searches_key(), self.recent_searches.clone());
        cx.notify();
    }

    pub fn clear_searches(&mut self, cx: &mut Context<Self>) {
        self.load_account_state();
        self.recent_searches.clear();
        UserDefaults::standard().remove_object(&Self::searches_key());
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.recently_played_ids.clear();
        self.recent_searches.clear();
        self.loaded_recent_key = None;
        cx.notify();
    }

    pub fn activate_account_state(&mut self, cx: &mut Context<Self>) {
        self.load_account_state();
        cx.notify();
    }

    pub fn remove_persisted_account(&self, user_id: &str) {
        let defaults = UserDefaults::standard();
        defaults.remove_object(&Self::scoped_key(MUSIC_RECENT, user_id));
        defaults.remove_object(&Self::scoped_key(MUSIC_SEARCHES, user_id));
    }

    fn load_account_state(&mut self) {
        Self::migrate_legacy_state_if_needed();
        let key = Self::recent_key();
        if self.loaded_recent_key.as_deref() == Some(key.as_str()) {
            return;
        }
        let defaults = UserDefaults::standard();
        self.recently_played_ids = defaults.string_array(&key).unwrap_or_default();
        self.recent_searches = defaults.string_array(&Self::searches_key()).unwrap_or_default();
        self.loaded_recent_key = Some(key);
    }

    fn recent_key() -> String {
        SaiAccountCacheScope::scoped_key(MUSIC_RECENT)
    }

    fn searches_key() -> String {
        SaiAccountCacheScope::scoped_key(MUSIC_SEARCHES)
    }

    fn scoped_key(logical_key: &str, user_id: &str) -> String {
        SaiAccountCacheScope::prefix(user_id) + logical_key
    }

    fn migrate_legacy_state_if_needed() {
        let defaults = UserDefaults::standard();
        if SaiAccountCacheScope::active_user_id().is_none() || defaults.bool(MUSIC_LOCAL_STATE_ACCOUNT_SCOPE) {
            return;
        }
        let recent_key = Self::recent_key();
        if defaults.object(&recent_key).is_none()
            && let Some(legacy) = defaults.string_array(MUSIC_RECENT)
        {
            defaults.set(&recent_key, legacy);
        }
        let searches_key = Self::searches_key();
        if defaults.object(&searches_key).is_none()
            && let Some(legacy) = defaults.string_array(MUSIC_SEARCHES)
        {
            defaults.set(&searches_key, legacy);
        }
        defaults.remove_object(MUSIC_RECENT);
        defaults.remove_object(MUSIC_SEARCHES);
        defaults.set(MUSIC_LOCAL_STATE_ACCOUNT_SCOPE, true);
    }
}
