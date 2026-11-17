use std::collections::HashMap;

use gpui::{App, AppContext, Context, Entity};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::gifts::engine::rise_gifts_engine_models::RiseGiftAppearanceDTO;

pub struct RiseGiftAppearancePresentationSlot {
    value: Option<RiseGiftAppearanceDTO>,
    last_use: u64,
}

impl RiseGiftAppearancePresentationSlot {
    pub fn value(&self) -> Option<&RiseGiftAppearanceDTO> {
        self.value.as_ref()
    }
}

pub struct RiseGiftAppearancePresentation {
    slots: HashMap<String, Entity<RiseGiftAppearancePresentationSlot>>,
    use_clock: u64,
}

impl RiseGiftAppearancePresentation {
    pub const SLOT_LIMIT: usize = 2_048;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self {
            slots: HashMap::new(),
            use_clock: 0,
        })
    }

    pub fn appearance(&mut self, user_id: &str, cx: &mut Context<Self>) -> Option<RiseGiftAppearanceDTO> {
        let slot = self.slot(user_id, cx);
        slot.read(cx).value.clone()
    }

    /// one short commit per answer, a slot is written only when its value really changed
    pub fn apply(&mut self, values: HashMap<String, RiseGiftAppearanceDTO>, cx: &mut Context<Self>) {
        for (id, value) in values {
            let slot = self.slot(&id, cx);
            slot.update(cx, |slot, cx| {
                if slot.value.as_ref() != Some(&value) {
                    slot.value = Some(value);
                    cx.notify();
                }
            });
        }
        self.trim(cx);
    }

    /// the account is going away, every view drops what it drew
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        for slot in self.slots.values() {
            slot.update(cx, |slot, cx| {
                slot.value = None;
                cx.notify();
            });
        }
        self.slots.clear();
    }

    fn slot(&mut self, user_id: &str, cx: &mut Context<Self>) -> Entity<RiseGiftAppearancePresentationSlot> {
        self.use_clock = self.use_clock.wrapping_add(1);
        let use_clock = self.use_clock;
        if let Some(existing) = self.slots.get(user_id) {
            let existing = existing.clone();
            existing.update(cx, |slot, _| slot.last_use = use_clock);
            return existing;
        }
        let created = cx.new(|_| RiseGiftAppearancePresentationSlot {
            value: None,
            last_use: use_clock,
        });
        self.slots.insert(user_id.to_owned(), created.clone());
        created
    }

    fn trim(&mut self, cx: &App) {
        if self.slots.len() <= Self::SLOT_LIMIT {
            return;
        }
        let own = SaiAccountCacheScope::active_user_id();
        let mut candidates: Vec<(String, u64)> = Vec::new();
        for (id, slot) in &self.slots {
            if own.as_deref() == Some(id.as_str()) {
                continue;
            }
            candidates.push((id.clone(), slot.read(cx).last_use));
        }
        candidates.sort_by_key(|(_, last_use)| *last_use);
        let evicted = self.slots.len() - Self::SLOT_LIMIT * 3 / 4;
        for (id, _) in candidates.into_iter().take(evicted) {
            self.slots.remove(&id);
        }
    }
}
