use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::time::Duration;

use gpui::{App, Global, Task};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;

/// a debounced callback, runs on the main thread with the app
pub type SaiDebouncedCallback = Box<dyn FnOnce(&mut App) + 'static>;

pub const DEFAULT_GROUP_KEY: &str = "default";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiDebouncerOptions {
    pub delay_milliseconds: i64,
    pub group_key: String,
    pub max_cache_data: i64,
}

impl Default for SaiDebouncerOptions {
    fn default() -> Self {
        Self {
            delay_milliseconds: 500,
            group_key: DEFAULT_GROUP_KEY.to_owned(),
            max_cache_data: 10,
        }
    }
}

/// result handle of a debounced action
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiDebouncedHandle {
    pub composite_key: String,
}

impl SaiDebouncedHandle {
    pub fn is_debouncing(&self, cx: &App) -> bool {
        SaiDebouncer::is_debouncing_composite_key(&self.composite_key, cx)
    }

    pub fn cancel(&self, cx: &mut App) {
        SaiDebouncer::cancel_composite_key(&self.composite_key, cx);
    }

    pub fn flush(&self, cx: &mut App) {
        SaiDebouncer::flush_composite_key(&self.composite_key, cx);
    }
}

struct Slot {
    work: Option<Task<()>>,
    generation: Uuid,
    timestamp: Duration,
    action: SaiDebouncedCallback,
    on_invalidate: Option<SaiDebouncedCallback>,
}

/// one debouncer per app, kept as a gpui global since every entry point runs on the main thread
/// callbacks are invoked after the global is released
#[derive(Default)]
pub struct SaiDebouncer {
    slots: HashMap<String, Slot>,
    debouncing: HashSet<String>,
}

impl Global for SaiDebouncer {}

impl SaiDebouncer {
    pub fn shared(cx: &mut App) -> &mut Self {
        cx.default_global::<Self>()
    }

    pub fn debounced_action_key(
        key: &str,
        action: SaiDebouncedCallback,
        options: SaiDebouncerOptions,
        cx: &mut App,
    ) -> SaiDebouncedHandle {
        Self::debounced_action_id(key, action, options, cx)
    }

    pub fn debounced_action_id(
        id: impl Display,
        action: SaiDebouncedCallback,
        options: SaiDebouncerOptions,
        cx: &mut App,
    ) -> SaiDebouncedHandle {
        Self::debounced_action_on_invalidate(id, action, options, None, cx)
    }

    /// debounced action by id with options and an on invalidate hook
    pub fn debounced_action_on_invalidate(
        id: impl Display,
        action: SaiDebouncedCallback,
        options: SaiDebouncerOptions,
        on_invalidate: Option<SaiDebouncedCallback>,
        cx: &mut App,
    ) -> SaiDebouncedHandle {
        let composite = Self::composite_key(&options.group_key, id);
        let generation = Uuid::new_v4();

        let delay = Duration::from_millis(u64::try_from(options.delay_milliseconds.max(0)).unwrap_or(0));
        let timer = cx.background_executor().timer(delay);
        let fire_key = composite.clone();
        let work = cx.spawn(async move |cx| {
            timer.await;
            cx.update(|cx| Self::fire_if_current(&fire_key, generation, cx));
        });

        let replaced = {
            let this = cx.default_global::<Self>();
            this.debouncing.insert(composite.clone());
            this.slots.insert(
                composite.clone(),
                Slot {
                    work: Some(work),
                    generation,
                    timestamp: RiseClock::monotonic(),
                    action,
                    on_invalidate,
                },
            )
        };
        drop(replaced);

        Self::prune(&options.group_key, options.max_cache_data, cx);
        SaiDebouncedHandle { composite_key: composite }
    }

    /// debounced action with delay in ms and group key and max cache data and invalidate hook
    /// defaults are the default group key and 10 and none
    pub fn debounced_action_delay_milliseconds(
        id: impl Display,
        action: SaiDebouncedCallback,
        delay_milliseconds: i64,
        group_key: &str,
        max_cache_data: i64,
        on_invalidate: Option<SaiDebouncedCallback>,
        cx: &mut App,
    ) -> SaiDebouncedHandle {
        let options = SaiDebouncerOptions {
            delay_milliseconds,
            group_key: group_key.to_owned(),
            max_cache_data,
        };
        Self::debounced_action_on_invalidate(id, action, options, on_invalidate, cx)
    }

    pub fn is_debouncing(id: impl Display, group_key: &str, cx: &App) -> bool {
        Self::is_debouncing_composite_key(&Self::composite_key(group_key, id), cx)
    }

    pub fn flush(id: impl Display, group_key: &str, cx: &mut App) {
        Self::flush_composite_key(&Self::composite_key(group_key, id), cx);
    }

    pub fn cancel(id: impl Display, group_key: &str, cx: &mut App) {
        Self::cancel_composite_key(&Self::composite_key(group_key, id), cx);
    }

    pub fn cancel_all_in_group(group_key: &str, cx: &mut App) {
        let prefix = Self::group_prefix(group_key);
        let keys: Vec<String> = cx
            .try_global::<Self>()
            .map(|this| this.slots.keys().filter(|key| key.starts_with(&prefix)).cloned().collect())
            .unwrap_or_default();
        for key in keys {
            Self::cancel_composite_key(&key, cx);
        }
    }

    pub fn cancel_all(cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        let slots = {
            let this = cx.global_mut::<Self>();
            this.debouncing.clear();
            std::mem::take(&mut this.slots)
        };
        for (_, slot) in slots {
            let Slot { work, on_invalidate, .. } = slot;
            drop(work);
            if let Some(on_invalidate) = on_invalidate {
                on_invalidate(cx);
            }
        }
    }

    pub fn active_ids(in_group: &str, cx: &App) -> Vec<String> {
        let prefix = Self::group_prefix(in_group);
        cx.try_global::<Self>()
            .map(|this| {
                this.slots
                    .keys()
                    .filter_map(|key| key.strip_prefix(&prefix).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn active_count(in_group: &str, cx: &App) -> usize {
        Self::active_ids(in_group, cx).len()
    }

    fn composite_key(group_key: &str, id: impl Display) -> String {
        format!("{group_key}_{id}")
    }

    fn group_prefix(group_key: &str) -> String {
        format!("{group_key}_")
    }

    fn is_debouncing_composite_key(composite_key: &str, cx: &App) -> bool {
        cx.try_global::<Self>()
            .is_some_and(|this| this.debouncing.contains(composite_key))
    }

    fn take_slot(composite_key: &str, cx: &mut App) -> Option<Slot> {
        if !cx.has_global::<Self>() {
            return None;
        }
        let this = cx.global_mut::<Self>();
        this.debouncing.remove(composite_key);
        this.slots.remove(composite_key)
    }

    fn cancel_composite_key(composite_key: &str, cx: &mut App) {
        let Some(Slot { work, on_invalidate, .. }) = Self::take_slot(composite_key, cx) else {
            return;
        };
        drop(work);
        if let Some(on_invalidate) = on_invalidate {
            on_invalidate(cx);
        }
    }

    fn flush_composite_key(composite_key: &str, cx: &mut App) {
        let Some(Slot { work, action, .. }) = Self::take_slot(composite_key, cx) else {
            return;
        };
        drop(work);
        action(cx);
    }

    fn fire_if_current(composite_key: &str, generation: Uuid, cx: &mut App) {
        let is_current = cx
            .try_global::<Self>()
            .and_then(|this| this.slots.get(composite_key))
            .is_some_and(|slot| slot.generation == generation);
        if !is_current {
            return;
        }
        let Some(Slot { work, action, .. }) = Self::take_slot(composite_key, cx) else {
            return;
        };
        if let Some(work) = work {
            work.detach();
        }
        action(cx);
    }

    fn prune(group_key: &str, max_entries: i64, cx: &mut App) {
        if max_entries <= 0 {
            return;
        }
        let prefix = Self::group_prefix(group_key);
        let mut entries: Vec<(String, Duration)> = cx
            .try_global::<Self>()
            .map(|this| {
                this.slots
                    .iter()
                    .filter(|(key, _)| key.starts_with(&prefix))
                    .map(|(key, slot)| (key.clone(), slot.timestamp))
                    .collect()
            })
            .unwrap_or_default();
        let max_entries = usize::try_from(max_entries).unwrap_or(usize::MAX);
        if entries.len() <= max_entries {
            return;
        }
        entries.sort_by_key(|(_, timestamp)| *timestamp);
        let overflow = entries.len() - max_entries;
        for (key, _) in entries.into_iter().take(overflow) {
            Self::cancel_composite_key(&key, cx);
        }
    }
}

pub fn sai_debounced_action(
    key: &str,
    action: SaiDebouncedCallback,
    options: SaiDebouncerOptions,
    cx: &mut App,
) -> SaiDebouncedHandle {
    SaiDebouncer::debounced_action_key(key, action, options, cx)
}

pub fn sai_is_debouncing(id: impl Display, group_key: &str, cx: &App) -> bool {
    SaiDebouncer::is_debouncing(id, group_key, cx)
}

pub fn sai_flush_debounced_action(id: impl Display, group_key: &str, cx: &mut App) {
    SaiDebouncer::flush(id, group_key, cx);
}

pub fn sai_cancel_debounced_action(id: impl Display, group_key: &str, cx: &mut App) {
    SaiDebouncer::cancel(id, group_key, cx);
}

pub fn sai_cancel_debounced_actions_by_group(group_key: &str, cx: &mut App) {
    SaiDebouncer::cancel_all_in_group(group_key, cx);
}

pub fn sai_cancel_all_debounced_actions(cx: &mut App) {
    SaiDebouncer::cancel_all(cx);
}

#[cfg(test)]
#[path = "sai_debouncer_tests.rs"]
mod tests;
