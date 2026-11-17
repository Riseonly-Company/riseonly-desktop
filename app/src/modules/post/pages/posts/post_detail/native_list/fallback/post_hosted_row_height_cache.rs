use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use gpui::App;
use parking_lot::Mutex;

use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListCachePurge, PostRowLayoutSpec, PreparedPostRow, PreparedPostRowKey,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_preparer::PostRowHostedHeightLookup;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

struct Entry {
    row_height: f32,
    hosted_version: i64,
    tick: u64,
}

/// last verified height of a post at one spec and account, any content version
#[derive(Clone, PartialEq, Eq, Hash)]
struct StickyKey {
    presentation_id: String,
    spec: PostRowLayoutSpec,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
}

impl StickyKey {
    fn new(key: &PreparedPostRowKey) -> Self {
        Self {
            presentation_id: key.presentation_id.clone(),
            spec: key.spec.clone(),
            account_user_id: key.account_user_id.clone(),
            account_epoch: key.account_epoch.clone(),
        }
    }
}

struct HostedRecord {
    post: GetPostFeedResponse,
    version: i64,
}

struct ItemRecord {
    content_version: i64,
    hosted_version: i64,
    theme_id: String,
    item_version: i64,
}

struct State {
    entry_limit: usize,
    record_limit: usize,
    entries: HashMap<PreparedPostRowKey, Entry>,
    sticky: HashMap<StickyKey, f32>,
    hosted_records: HashMap<String, HostedRecord>,
    item_records: HashMap<String, ItemRecord>,
    next_hosted_version: i64,
    next_item_version: i64,
    tick: u64,
}

impl State {
    fn exact_height_locked(&mut self, key: &PreparedPostRowKey) -> Option<f32> {
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        let version = self.hosted_records.get(&key.presentation_id)?.version;
        let entry = self.entries.get_mut(key)?;
        if version != entry.hosted_version {
            return None;
        }
        entry.tick = tick;
        Some(entry.row_height)
    }

    fn register_locked(&mut self, post: &GetPostFeedResponse) -> i64 {
        let presentation_id = post.presentation_id();
        if let Some(record) = self.hosted_records.get(&presentation_id)
            && record.post == *post
        {
            return record.version;
        }
        if self.hosted_records.len() >= self.record_limit {
            self.hosted_records.clear();
        }
        let version = self.next_hosted_version;
        self.next_hosted_version = self.next_hosted_version.wrapping_add(1);
        self.hosted_records.insert(presentation_id, HostedRecord { post: post.clone(), version });
        version
    }

    fn evict_entries_if_needed_locked(&mut self) {
        if self.entries.len() > self.entry_limit {
            let remove_count = self.entries.len() - self.entry_limit * 3 / 4;
            let mut ordered: Vec<(u64, PreparedPostRowKey)> =
                self.entries.iter().map(|(key, entry)| (entry.tick, key.clone())).collect();
            ordered.sort_by_key(|(tick, _)| *tick);
            for (_, key) in ordered.into_iter().take(remove_count) {
                self.entries.remove(&key);
            }
        }
        if self.sticky.len() > self.entry_limit * 2 {
            self.sticky.clear();
            for (key, entry) in self.entries.iter() {
                self.sticky.insert(StickyKey::new(key), entry.row_height);
            }
        }
    }
}

pub struct PostHostedRowHeightCache {
    state: Mutex<State>,
}

static SHARED: OnceLock<PostHostedRowHeightCache> = OnceLock::new();
static IS_PURGE_REGISTERED: AtomicBool = AtomicBool::new(false);

impl PostHostedRowHeightCache {
    pub const DEFAULT_ENTRY_LIMIT: usize = 1_024;
    pub const DEFAULT_RECORD_LIMIT: usize = 2_048;

    pub fn shared() -> &'static PostHostedRowHeightCache {
        SHARED.get_or_init(|| Self::new(Self::DEFAULT_ENTRY_LIMIT, Self::DEFAULT_RECORD_LIMIT))
    }

    pub fn new(entry_limit: usize, record_limit: usize) -> Self {
        Self {
            state: Mutex::new(State {
                entry_limit: entry_limit.max(8),
                record_limit: record_limit.max(8),
                entries: HashMap::new(),
                sticky: HashMap::new(),
                hosted_records: HashMap::new(),
                item_records: HashMap::new(),
                next_hosted_version: 1,
                next_item_version: 1,
                tick: 0,
            }),
        }
    }

    /// height lookup handed to the row preparer
    pub fn lookup(&'static self) -> PostRowHostedHeightLookup {
        Arc::new(move |key: &PreparedPostRowKey| self.height(key))
    }

    /// exact measured height or else the last verified one for the same post, spec and account
    /// so a like on a hosted row doesnt jump to an estimate and back
    pub fn height(&self, key: &PreparedPostRowKey) -> Option<f32> {
        let mut state = self.state.lock();
        if let Some(exact) = state.exact_height_locked(key) {
            return Some(exact);
        }
        state.sticky.get(&StickyKey::new(key)).copied()
    }

    /// height measured for exactly this key and the current hosted content
    pub fn exact_height(&self, key: &PreparedPostRowKey) -> Option<f32> {
        self.state.lock().exact_height_locked(key)
    }

    /// stores a measured row height, dropped if the post is
    /// no longer the latest snapshot or the height is bad
    pub fn insert(&self, row_height: f32, key: &PreparedPostRowKey, post: &GetPostFeedResponse) {
        if !row_height.is_finite() || !(1.0..100_000.0).contains(&row_height) {
            return;
        }
        let mut state = self.state.lock();
        let recorded = state.hosted_records.get(&key.presentation_id).map(|record| (record.post == *post, record.version));
        let version = match recorded {
            Some((true, version)) => version,
            Some((false, _)) => return,
            None => state.register_locked(post),
        };
        state.tick = state.tick.wrapping_add(1);
        let tick = state.tick;
        state.entries.insert(key.clone(), Entry { row_height, hosted_version: version, tick });
        state.sticky.insert(StickyKey::new(key), row_height);
        state.evict_entries_if_needed_locked();
    }

    /// stores the latest snapshot of a post and bumps its hosted content version if it changed
    pub fn register_hosted_content(&self, post: &GetPostFeedResponse) -> i64 {
        self.state.lock().register_locked(post)
    }

    /// engine content_version of a hosted item
    /// goes up when seed, hosted content or palette change
    /// so mounted rows reconfigure on hosted only changes too
    pub fn item_version(&self, prepared: &PreparedPostRow, theme_id: &str) -> i64 {
        let mut state = self.state.lock();
        let hosted_version = state.register_locked(&prepared.post);
        if let Some(record) = state.item_records.get(&prepared.presentation_id)
            && record.content_version == prepared.content_version
            && record.hosted_version == hosted_version
            && record.theme_id == theme_id
        {
            return record.item_version;
        }
        if state.item_records.len() >= state.record_limit {
            state.item_records.clear();
        }
        let version = state.next_item_version;
        state.next_item_version = state.next_item_version.wrapping_add(1);
        state.item_records.insert(
            prepared.presentation_id.clone(),
            ItemRecord {
                content_version: prepared.content_version,
                hosted_version,
                theme_id: theme_id.to_owned(),
                item_version: version,
            },
        );
        version
    }

    pub fn remove_all(&self) {
        let mut state = self.state.lock();
        state.entries.clear();
        state.sticky.clear();
        state.hosted_records.clear();
        state.item_records.clear();
    }

    pub fn register_purge_if_needed(cx: &mut App) {
        if IS_PURGE_REGISTERED.swap(true, Ordering::Relaxed) {
            return;
        }
        let _ = PostListCachePurge::register(|_| PostHostedRowHeightCache::shared().remove_all(), cx);
    }
}
