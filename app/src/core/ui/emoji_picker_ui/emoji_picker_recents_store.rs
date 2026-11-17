use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use gpui::{App, Context, Entity};
use rise_canon::RiseLimits;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{EMOJI_PICKER_RECENTS, EMOJI_RECENTS_ACCOUNT_SCOPE};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalogCell;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

static MAX_COUNT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("emoji_picker_recents_store_max_count"));

const CUSTOM_ENTRY_PREFIX: &str = "custom:";

pub struct EmojiPickerRecentsStore {
    pick_order: Vec<String>,
    loaded_storage_key: Option<String>,
}

impl EmojiPickerRecentsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        let mut store = Self {
            pick_order: Vec::new(),
            loaded_storage_key: None,
        };
        store.load_account_state();
        store
    }

    pub fn pick_order(&self) -> &[String] {
        &self.pick_order
    }

    /// plain emoji only, newest first
    /// the list reactions and the shared picker read
    pub fn recents(&self) -> Vec<String> {
        self.pick_order
            .iter()
            .filter(|entry| !Self::is_custom_entry(entry))
            .cloned()
            .collect()
    }

    pub fn register(&mut self, emoji: &str, cx: &mut Context<Self>) {
        if emoji.is_empty() {
            return;
        }
        self.push(emoji.to_owned(), cx);
    }

    /// puts a pack emoji pick in the shared order, the server keeps its own recents list
    pub fn register_custom(&mut self, sticker_id: &str, cx: &mut Context<Self>) {
        if sticker_id.is_empty() {
            return;
        }
        self.push(format!("{CUSTOM_ENTRY_PREFIX}{sticker_id}"), cx);
    }

    fn push(&mut self, entry: String, cx: &mut Context<Self>) {
        self.load_account_state();
        let mut next = self.pick_order.clone();
        next.retain(|kept| *kept != entry);
        next.insert(0, entry);
        let max_count = *MAX_COUNT;
        let mut plain_count: i64 = 0;
        let mut custom_count: i64 = 0;
        next.retain(|kept| {
            if Self::is_custom_entry(kept) {
                custom_count += 1;
                return custom_count <= max_count;
            }
            plain_count += 1;
            plain_count <= max_count
        });
        self.pick_order = next.clone();
        cx.notify();
        UserDefaults::standard().set(&Self::current_storage_key(), next);
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.load_account_state();
        self.pick_order = Vec::new();
        cx.notify();
        UserDefaults::standard().remove_object(&Self::current_storage_key());
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.pick_order = Vec::new();
        self.loaded_storage_key = None;
        cx.notify();
    }

    pub fn activate_account_state(&mut self, cx: &mut Context<Self>) {
        if self.load_account_state() {
            cx.notify();
        }
    }

    fn load_account_state(&mut self) -> bool {
        Self::migrate_legacy_state_if_needed();
        let key = Self::current_storage_key();
        if self.loaded_storage_key.as_deref() == Some(key.as_str()) {
            return false;
        }
        self.pick_order = UserDefaults::standard().string_array(&key).unwrap_or_default();
        self.loaded_storage_key = Some(key);
        true
    }

    /// one recents row for both kinds in the local pick order
    /// unicode matches the plain entries of pick_order one to one and pack emoji are found by custom_id
    /// pack recents the order doesnt know about come after
    pub fn ordered<C, U, R>(
        pick_order: &[String],
        custom: &[C],
        custom_id: impl Fn(&C) -> String,
        unicode: &[U],
        custom_slot: impl Fn(&C) -> R,
        unicode_slot: impl Fn(&U) -> R,
    ) -> Vec<R> {
        let mut custom_by_id: HashMap<String, &C> = HashMap::new();
        for item in custom {
            custom_by_id.entry(custom_id(item)).or_insert(item);
        }
        let mut placed: HashSet<String> = HashSet::new();
        let mut out: Vec<R> = Vec::with_capacity(custom.len() + unicode.len());
        let mut unicode_index = 0;
        for entry in pick_order {
            let Some(sticker_id) = entry.strip_prefix(CUSTOM_ENTRY_PREFIX) else {
                if let Some(cell) = unicode.get(unicode_index) {
                    out.push(unicode_slot(cell));
                }
                unicode_index += 1;
                continue;
            };
            let Some(&item) = custom_by_id.get(sticker_id) else {
                continue;
            };
            if !placed.insert(sticker_id.to_owned()) {
                continue;
            }
            out.push(custom_slot(item));
        }
        for item in custom {
            if placed.insert(custom_id(item)) {
                out.push(custom_slot(item));
            }
        }
        while let Some(cell) = unicode.get(unicode_index) {
            out.push(unicode_slot(cell));
            unicode_index += 1;
        }
        out
    }

    fn is_custom_entry(entry: &str) -> bool {
        entry.starts_with(CUSTOM_ENTRY_PREFIX)
    }

    pub fn remove_persisted_account(&self, user_id: &str) {
        UserDefaults::standard()
            .remove_object(&(SaiAccountCacheScope::prefix(user_id) + EMOJI_PICKER_RECENTS));
    }

    fn current_storage_key() -> String {
        SaiAccountCacheScope::scoped_key(EMOJI_PICKER_RECENTS)
    }

    fn migrate_legacy_state_if_needed() {
        let defaults = UserDefaults::standard();
        if SaiAccountCacheScope::active_user_id().is_none() || defaults.bool(EMOJI_RECENTS_ACCOUNT_SCOPE) {
            return;
        }
        let current_key = Self::current_storage_key();
        if defaults.object(&current_key).is_none()
            && let Some(legacy) = defaults.string_array(EMOJI_PICKER_RECENTS)
        {
            defaults.set(&current_key, legacy);
        }
        defaults.remove_object(EMOJI_PICKER_RECENTS);
        defaults.set(EMOJI_RECENTS_ACCOUNT_SCOPE, true);
    }
}

/// one cell of the shared recents row, a pack emoji or a plain emoji
#[derive(Clone, PartialEq)]
pub enum EmojiRecentSlot {
    Custom(StickerItem),
    Unicode(EmojiPickerCatalogCell),
}

impl EmojiRecentSlot {
    pub fn id(&self) -> String {
        match self {
            EmojiRecentSlot::Custom(item) => format!("custom:{}", item.id),
            EmojiRecentSlot::Unicode(cell) => cell.id.clone(),
        }
    }
}
