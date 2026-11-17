use std::collections::HashSet;

use gpui::{App, Context, Entity, Task};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::custom_emoji_stickers_hooks::custom_emoji_stickers_hooks;

/// badges and reactions may hydrate cached emoji but not list the recent or saved catalogues
/// those reads belong to an opened picker
/// bare ce id has no pack identity so one in no pack is resolved by id, same pass ones go as a batch
#[derive(Default)]
pub struct CustomEmojiResourceWarmup {
    warmed_account_epoch: Option<String>,
    hydration_task: Option<Task<()>>,
    pending_custom_emoji_ids: HashSet<String>,
    in_flight_custom_emoji_ids: HashSet<String>,
    custom_emoji_task: Option<Task<()>>,
    unresolved_custom_emoji_ids: HashSet<String>,
}

impl CustomEmojiResourceWarmup {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::default())
    }

    pub fn unresolved_custom_emoji_ids(&self) -> &HashSet<String> {
        &self.unresolved_custom_emoji_ids
    }

    pub fn resource_revision(&self, cx: &App) -> u64 {
        custom_emoji_stickers_hooks().map_or(0, |hooks| (hooks.resource_revision)(cx))
    }

    pub fn ensure_emoji_resources_loaded(&mut self, cx: &mut Context<Self>) {
        let Some(epoch) = SaiAccountCacheScope::active_epoch() else {
            return;
        };
        if self.warmed_account_epoch.as_deref() == Some(epoch.as_str()) {
            return;
        }
        self.warmed_account_epoch = Some(epoch.clone());
        self.hydration_task = Some(cx.spawn(async move |_this, cx| {
            let Some(hooks) = custom_emoji_stickers_hooks() else {
                return;
            };
            let hydration = cx.update(|cx| (hooks.hydrate_cached_emoji_resources)(epoch, cx));
            hydration.await;
        }));
    }

    pub fn ensure_custom_emoji(&mut self, id: &str, cx: &mut Context<Self>) {
        self.ensure_emoji_resources_loaded(cx);
        if id.is_empty() {
            return;
        }
        let Some(epoch) = SaiAccountCacheScope::active_epoch() else {
            return;
        };
        self.pending_custom_emoji_ids.insert(id.to_string());
        if self.custom_emoji_task.is_some() {
            return;
        }
        self.custom_emoji_task = Some(cx.spawn(async move |this, cx| {
            let Ok(ids) = this.update(cx, |this, _| {
                let ids: Vec<String> = this
                    .pending_custom_emoji_ids
                    .difference(&this.in_flight_custom_emoji_ids)
                    .cloned()
                    .collect();
                this.pending_custom_emoji_ids.clear();
                if let Some(task) = this.custom_emoji_task.take() {
                    task.detach();
                }
                ids
            }) else {
                return;
            };
            if ids.is_empty() {
                return;
            }
            if this
                .update(cx, |this, _| this.in_flight_custom_emoji_ids.extend(ids.iter().cloned()))
                .is_err()
            {
                return;
            }
            let did_look_up = match custom_emoji_stickers_hooks() {
                Some(hooks) => {
                    let lookup = cx.update(|cx| (hooks.resolve_custom_emoji)(ids.clone(), epoch.clone(), cx));
                    lookup.await
                }
                None => false,
            };
            if SaiAccountCacheScope::active_epoch().as_deref() != Some(epoch.as_str()) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                for id in &ids {
                    this.in_flight_custom_emoji_ids.remove(id);
                }
                let has_item = |id: &String, cx: &App| {
                    custom_emoji_stickers_hooks()
                        .and_then(|hooks| (hooks.custom_emoji_item)(id, cx))
                        .is_some()
                };
                let resolved: Vec<&String> = ids.iter().filter(|id| has_item(*id, cx)).collect();
                let mut changed = false;
                if resolved.iter().any(|id| this.unresolved_custom_emoji_ids.contains(*id)) {
                    for id in resolved {
                        this.unresolved_custom_emoji_ids.remove(id);
                    }
                    changed = true;
                }
                if did_look_up {
                    let unresolved: Vec<String> = ids
                        .iter()
                        .filter(|id| !has_item(*id, cx) && !this.unresolved_custom_emoji_ids.contains(*id))
                        .cloned()
                        .collect();
                    if !unresolved.is_empty() {
                        this.unresolved_custom_emoji_ids.extend(unresolved);
                        changed = true;
                    }
                }
                if changed {
                    cx.notify();
                }
            });
        }));
    }

    pub fn is_unresolved_custom_emoji(&self, id: &str) -> bool {
        self.unresolved_custom_emoji_ids.contains(id)
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.hydration_task = None;
        self.warmed_account_epoch = None;
        self.custom_emoji_task = None;
        self.pending_custom_emoji_ids.clear();
        self.in_flight_custom_emoji_ids.clear();
        if !self.unresolved_custom_emoji_ids.is_empty() {
            self.unresolved_custom_emoji_ids.clear();
            cx.notify();
        }
    }
}
