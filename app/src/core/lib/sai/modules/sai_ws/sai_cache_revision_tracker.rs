use std::collections::HashMap;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

/// revision counter of one key
/// each key has its own entity so a view reading one key wakes only on that key
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaiCacheRevisionTrackerKeyRevision {
    pub value: i64,
}

pub struct SaiCacheRevisionTracker {
    by_key: HashMap<String, SaiCacheRevisionTrackerKeyRevision>,
}

/// per key holders kept outside the tracker entity so reaching one doesnt track the tracker
#[derive(Default)]
struct SaiCacheKeyRevisionSlots {
    by_key: HashMap<String, Entity<SaiCacheRevisionTrackerKeyRevision>>,
}

impl Global for SaiCacheKeyRevisionSlots {}

/// fires when a key gets its first holder or on reset
/// lets a reader of a never bumped key wake on the first bump
struct SaiCacheKeyFirstBumps(Entity<SaiCacheRevisionTrackerKeyRevision>);

impl Global for SaiCacheKeyFirstBumps {}

/// emitted with the key on bump and with none on reset
/// so a store with few keys can ignore other cache writes
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiCacheRevisionBumped {
    pub key: Option<String>,
}

impl EventEmitter<SaiCacheRevisionBumped> for SaiCacheRevisionTracker {}

impl SaiCacheRevisionTracker {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let first_bumps = cx.new(|_| SaiCacheRevisionTrackerKeyRevision::default());
        cx.set_global(SaiCacheKeyFirstBumps(first_bumps));
        Self {
            by_key: HashMap::new(),
        }
    }

    /// drops all holders and wakes observers, keys start again from zero
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.by_key.clear();
        let slots: Vec<Entity<SaiCacheRevisionTrackerKeyRevision>> = cx
            .try_global::<SaiCacheKeyRevisionSlots>()
            .map(|slots| slots.by_key.values().cloned().collect())
            .unwrap_or_default();
        for slot in slots {
            slot.update(cx, |revision, cx| {
                revision.value = 0;
                cx.notify();
            });
        }
        Self::notify_first_bumps(cx);
        cx.emit(SaiCacheRevisionBumped { key: None });
        cx.notify();
    }

    fn notify_first_bumps(cx: &mut Context<Self>) {
        if let Some(first_bumps) = cx.try_global::<SaiCacheKeyFirstBumps>().map(|first| first.0.clone()) {
            first_bumps.update(cx, |revision, cx| {
                revision.value = revision.value.wrapping_add(1);
                cx.notify();
            });
        }
    }

    fn holder(&mut self, key: &str) -> &mut SaiCacheRevisionTrackerKeyRevision {
        self.by_key.entry(key.to_owned()).or_default()
    }

    pub fn revision(&self, for_key: &str) -> i64 {
        let trimmed = trimming_whitespaces_and_newlines(for_key);
        if trimmed.is_empty() {
            return 0;
        }
        self.by_key.get(trimmed).map_or(0, |revision| revision.value)
    }

    pub fn bump(&mut self, for_key: &str, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(for_key);
        if trimmed.is_empty() {
            return;
        }
        let revision = self.holder(trimmed);
        revision.value = revision.value.wrapping_add(1);
        let value = revision.value;
        let slot = cx.try_global::<SaiCacheKeyRevisionSlots>().and_then(|slots| slots.by_key.get(trimmed).cloned());
        match slot {
            Some(slot) => slot.update(cx, |revision, cx| {
                revision.value = value;
                cx.notify();
            }),
            None => {
                let slot = cx.new(|_| SaiCacheRevisionTrackerKeyRevision { value });
                cx.default_global::<SaiCacheKeyRevisionSlots>().by_key.insert(trimmed.to_owned(), slot);
                Self::notify_first_bumps(cx);
            }
        }
        cx.emit(SaiCacheRevisionBumped { key: Some(trimmed.to_owned()) });
        cx.notify();
    }

    /// revision of one key, the reading view wakes only on bumps of this key
    pub fn observed_revision(for_key: &str, cx: &mut App) -> i64 {
        let trimmed = trimming_whitespaces_and_newlines(for_key);
        if trimmed.is_empty() {
            return 0;
        }
        let existing = cx.try_global::<SaiCacheKeyRevisionSlots>().and_then(|slots| slots.by_key.get(trimmed).cloned());
        let slot = match existing {
            Some(slot) => slot,
            None => {
                let slot = cx.new(|_| SaiCacheRevisionTrackerKeyRevision::default());
                cx.default_global::<SaiCacheKeyRevisionSlots>().by_key.insert(trimmed.to_owned(), slot.clone());
                slot
            }
        };
        slot.read(cx).value
    }

    /// same as observed_revision but for a read only context
    /// key with no holder yet reads the first bump signal
    pub fn observed_revision_in(for_key: &str, cx: &App) -> i64 {
        let trimmed = trimming_whitespaces_and_newlines(for_key);
        if trimmed.is_empty() {
            return 0;
        }
        if let Some(slot) = cx.try_global::<SaiCacheKeyRevisionSlots>().and_then(|slots| slots.by_key.get(trimmed)) {
            return slot.read(cx).value;
        }
        if let Some(first_bumps) = cx.try_global::<SaiCacheKeyFirstBumps>() {
            let _ = first_bumps.0.read(cx);
            return 0;
        }
        rise_ui::shared::try_read::<Self>(cx).map_or(0, |tracker| tracker.revision(trimmed))
    }
}
