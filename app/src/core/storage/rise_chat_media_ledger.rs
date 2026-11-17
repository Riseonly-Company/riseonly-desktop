use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use parking_lot::Mutex;
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::AppPaths;
use rise_platform::file_manager::FileManager;
use serde::{Deserialize, Serialize};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::RemoteMediaDiskCache;
use crate::core::media::rise_media_transfer_hub::RiseMediaCacheIdentity;
use crate::core::ui::animated_gif_view::AnimatedImageSourcePolicy;
use crate::core::ui::clever_image_disk_cache::CleverImageDiskCache;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageMediaItem, MessageMediaKind};

#[derive(Default)]
struct LedgerState {
    loaded_account: Option<String>,
    chats: HashMap<String, Vec<String>>,
    names: HashMap<String, String>,
    known: HashSet<String>,
    write_pending: bool,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    chats: HashMap<String, Vec<String>>,
    names: HashMap<String, String>,
}

/// one note hop, what the bubble handed over already filtered on the caller thread
struct NoteRecord {
    account: String,
    chat_id: String,
    name: String,
    urls: Vec<String>,
}

#[derive(Default)]
struct NoteQueueState {
    pending: VecDeque<NoteRecord>,
    is_draining: bool,
}

/// clears a scheduling flag if the guarded work is dropped unrun
/// an account reset cancels queued work and a stale flag would block every later note or write
struct ScheduledFlagGuard {
    clear: fn(),
    armed: bool,
}

impl ScheduledFlagGuard {
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for ScheduledFlagGuard {
    fn drop(&mut self) {
        if self.armed {
            (self.clear)();
        }
    }
}

static LOCK: LazyLock<Mutex<LedgerState>> = LazyLock::new(|| Mutex::new(LedgerState::default()));
static NOTE_QUEUE: LazyLock<Mutex<NoteQueueState>> = LazyLock::new(|| Mutex::new(NoteQueueState::default()));
static CACHE_ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    AppPaths::resolve(AppEnvironment::current().canon())
        .ok()
        .map(|paths| paths.cache_dir().to_path_buf())
});

/// which chat each cached photo, video and file came from
/// caches key by url and know no chats so the storage list reads this
/// only urls kept, an evicted file prices at zero
pub enum RiseChatMediaLedger {}

impl RiseChatMediaLedger {
    const MAXIMUM_URLS_PER_CHAT: usize = 4_000;
    const WRITE_DELAY: Duration = Duration::from_secs(2);
    const FOLDER: &'static str = "rise-chat-media-ledger";
    const ANONYMOUS_ACCOUNT: &'static str = "anonymous";
    const FILE_EXTENSION: &'static str = "json";

    /// saves the chat title and the remote media of one message under its chat
    /// title is kept for every opened chat, so the storage list can name chats missing from the loaded list
    pub fn note(raw_chat_id: &str, raw_name: &str, items: &[MessageMediaItem]) {
        let chat_id = trimming_whitespaces_and_newlines(raw_chat_id);
        if chat_id.is_empty() {
            return;
        }
        let name = trimming_whitespaces_and_newlines(raw_name);
        let urls: Vec<String> = items
            .iter()
            .filter(|item| item.kind != MessageMediaKind::Voice && item.kind != MessageMediaKind::Sticker)
            .flat_map(|item| [item.url.clone(), item.thumbnail_url.clone().unwrap_or_default()])
            .filter(|url| Self::is_remote(url))
            .collect();
        if urls.is_empty() && name.is_empty() {
            return;
        }
        let account = Self::account_key(SaiAccountCacheScope::active_user_id().as_deref());
        let note = NoteRecord {
            account,
            chat_id: chat_id.to_owned(),
            name: name.to_owned(),
            urls,
        };
        let starts_drain = {
            let mut queue = NOTE_QUEUE.lock();
            queue.pending.push_back(note);
            !std::mem::replace(&mut queue.is_draining, true)
        };
        if !starts_drain {
            return;
        }
        let guard = ScheduledFlagGuard {
            clear: Self::clear_draining,
            armed: true,
        };
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            Self::drain_notes();
            guard.disarm();
        }));
    }

    fn drain_notes() {
        loop {
            let next = {
                let mut queue = NOTE_QUEUE.lock();
                let next = queue.pending.pop_front();
                if next.is_none() {
                    queue.is_draining = false;
                }
                next
            };
            let Some(note) = next else {
                return;
            };
            Self::record(&note.account, &note.chat_id, &note.name, &note.urls);
        }
    }

    fn clear_draining() {
        NOTE_QUEUE.lock().is_draining = false;
    }

    fn clear_write_pending() {
        LOCK.lock().write_pending = false;
    }

    fn record(account: &str, chat_id: &str, name: &str, urls: &[String]) {
        let should_schedule = {
            let mut state = LOCK.lock();
            Self::load_if_needed(&mut state, account);
            let mut added = false;
            if !name.is_empty() && state.names.get(chat_id).map(String::as_str) != Some(name) {
                state.names.insert(chat_id.to_owned(), name.to_owned());
                added = true;
            }
            for url in urls {
                if state.known.insert(Self::known_key(chat_id, url)) {
                    state.chats.entry(chat_id.to_owned()).or_default().push(url.clone());
                    added = true;
                }
            }
            if !added {
                false
            } else {
                let overflow = state
                    .chats
                    .get(chat_id)
                    .map_or(0, |list| list.len().saturating_sub(Self::MAXIMUM_URLS_PER_CHAT));
                if overflow > 0 {
                    let dropped: Vec<String> = state
                        .chats
                        .get_mut(chat_id)
                        .map(|list| list.drain(..overflow).collect())
                        .unwrap_or_default();
                    for url in &dropped {
                        state.known.remove(&Self::known_key(chat_id, url));
                    }
                }
                if state.write_pending {
                    false
                } else {
                    state.write_pending = true;
                    true
                }
            }
        };
        if !should_schedule {
            return;
        }
        let account = account.to_owned();
        let guard = ScheduledFlagGuard {
            clear: Self::clear_write_pending,
            armed: true,
        };
        let runtime = RiseAccountRuntime::current();
        let writer = Arc::clone(&runtime);
        drop(runtime.spawn(async move {
            tokio::time::sleep(Self::WRITE_DELAY).await;
            drop(writer.spawn_blocking(move || {
                guard.disarm();
                Self::persist(&account);
            }));
        }));
    }

    /// every chat that has noted media with its urls
    pub fn snapshot(user_id: Option<&str>) -> HashMap<String, Vec<String>> {
        let account = Self::account_key(user_id);
        let mut state = LOCK.lock();
        Self::load_if_needed(&mut state, &account);
        state.chats.clone()
    }

    pub fn chat_names(user_id: Option<&str>) -> HashMap<String, String> {
        let account = Self::account_key(user_id);
        let mut state = LOCK.lock();
        Self::load_if_needed(&mut state, &account);
        state.names.clone()
    }

    /// drops the chat media list
    /// the title stays, its a few bytes and the chat may open again
    pub fn forget(raw_chat_id: &str, user_id: Option<&str>) {
        let chat_id = trimming_whitespaces_and_newlines(raw_chat_id);
        if chat_id.is_empty() {
            return;
        }
        let account = Self::account_key(user_id);
        {
            let mut state = LOCK.lock();
            Self::load_if_needed(&mut state, &account);
            if let Some(urls) = state.chats.remove(chat_id) {
                for url in &urls {
                    state.known.remove(&Self::known_key(chat_id, url));
                }
            }
        }
        Self::persist(&account);
    }

    pub fn remove_persisted_account(user_id: &str) {
        let account = Self::account_key(Some(user_id));
        let mut state = LOCK.lock();
        if state.loaded_account.as_deref() == Some(account.as_str()) {
            state.loaded_account = None;
            state.chats.clear();
            state.names.clear();
            state.known.clear();
        }
        if let Some(url) = Self::file_url(&account) {
            FileManager.remove_item(&url).ok();
        }
    }

    fn is_remote(value: &str) -> bool {
        let lowered = value.to_lowercase();
        lowered.starts_with("https://") || lowered.starts_with("http://")
    }

    fn persist(account: &str) {
        let mut state = LOCK.lock();
        state.write_pending = false;
        if state.loaded_account.as_deref() != Some(account) {
            return;
        }
        let Some(url) = Self::file_url(account) else {
            return;
        };
        if state.chats.is_empty() && state.names.is_empty() {
            FileManager.remove_item(&url).ok();
            return;
        }
        let stored = Stored {
            chats: state.chats.clone(),
            names: state.names.clone(),
        };
        let Ok(data) = serde_json::to_vec(&stored) else {
            return;
        };
        Self::write_atomically(&url, &data).ok();
    }

    fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut temporary = url.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, data)?;
        std::fs::rename(&temporary, url).inspect_err(|_| {
            std::fs::remove_file(&temporary).ok();
        })
    }

    /// call under the lock
    /// switching accounts drops the other account ledger from memory
    fn load_if_needed(state: &mut LedgerState, account: &str) {
        if state.loaded_account.as_deref() == Some(account) {
            return;
        }
        state.loaded_account = Some(account.to_owned());
        state.chats.clear();
        state.names.clear();
        state.known.clear();
        let Some(url) = Self::file_url(account) else {
            return;
        };
        let Ok(data) = std::fs::read(&url) else {
            return;
        };
        if let Ok(stored) = serde_json::from_slice::<Stored>(&data) {
            state.chats = stored.chats;
            state.names = stored.names;
        } else if let Ok(legacy) = serde_json::from_slice::<HashMap<String, Vec<String>>>(&data) {
            state.chats = legacy;
        }
        let known: HashSet<String> = state
            .chats
            .iter()
            .flat_map(|(chat_id, urls)| urls.iter().map(move |url| Self::known_key(chat_id, url)))
            .collect();
        state.known = known;
    }

    fn file_url(account: &str) -> Option<PathBuf> {
        let caches = CACHE_ROOT.as_ref()?;
        let directory = caches.join(Self::FOLDER);
        FileManager.create_directory(&directory, true).ok();
        Some(directory.join(format!("{account}.{}", Self::FILE_EXTENSION)))
    }

    fn account_key(user_id: Option<&str>) -> String {
        let value = user_id.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if value.is_empty() {
            return Self::ANONYMOUS_ACCOUNT.to_owned();
        }
        sha256_hex(value.as_bytes())
    }

    fn known_key(chat_id: &str, url: &str) -> String {
        format!("{chat_id}|{url}")
    }
}

/// result of resolving noted urls
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseChatMediaFilesResolved {
    pub image_keys: HashSet<String>,
    pub remote_digests: HashSet<String>,
    pub bytes: i64,
}

/// noted urls checked against what the two media caches hold right now
/// image cache keys and remote media digests for videos and files
pub struct RiseChatMediaFiles {
    image_bytes: HashMap<String, i64>,
    remote_bytes: HashMap<String, i64>,
}

impl RiseChatMediaFiles {
    /// blocking file io, call it off the main thread
    pub fn new(account_user_id: Option<&str>) -> Self {
        let mut images: HashMap<String, i64> = HashMap::new();
        for entry in CleverImageDiskCache::list_entries_read_only(account_user_id) {
            if entry.file_bytes > 0 {
                images.insert(CleverImageDiskCache::normalized_cache_key(&entry.cache_key), entry.file_bytes);
            }
        }
        let mut remote: HashMap<String, i64> = HashMap::new();
        for entry in RemoteMediaDiskCache::list_entries(account_user_id) {
            if entry.byte_count > 0 {
                remote.insert(entry.id, entry.byte_count);
            }
        }
        Self {
            image_bytes: images,
            remote_bytes: remote,
        }
    }

    pub fn bytes(&self, urls: &[String]) -> i64 {
        self.resolve(urls).bytes
    }

    pub fn resolve(&self, urls: &[String]) -> RiseChatMediaFilesResolved {
        let mut resolved = RiseChatMediaFilesResolved::default();
        for url in urls {
            let image_key = CleverImageDiskCache::normalized_cache_key(url);
            let payload_key =
                CleverImageDiskCache::normalized_cache_key(&AnimatedImageSourcePolicy::payload_cache_key(url));
            for key in [image_key, payload_key] {
                let Some(bytes) = self.image_bytes.get(&key).copied() else {
                    continue;
                };
                if resolved.image_keys.insert(key) {
                    resolved.bytes += bytes;
                }
            }
            let Some(remote) = Url::from_string(url) else {
                continue;
            };
            let digest = RemoteMediaDiskCache::key_digest(&RiseMediaCacheIdentity::key(&remote));
            if let Some(bytes) = self.remote_bytes.get(&digest).copied()
                && resolved.remote_digests.insert(digest)
            {
                resolved.bytes += bytes;
            }
        }
        resolved
    }
}
