use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{App, Context, Entity, Subscription, Task};
use rise_platform::process_activity::begin_process_activity;
use rise_canon::ChatMessageEntityField;
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::tr;
use rise_platform::AppPaths;
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;
use rise_platform::url_resource_values::{URLResourceValues, set_resource_values};
use serde_json::{Map, Value};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::{SaiWsDiskCache, SaiWsDiskCacheError};
use crate::modules::chat::components::chat::composer::chat_composer_types::{
    ChatComposerCustomEmojiAttachmentPayload, ChatComposerUserMention,
};
use crate::modules::chat::components::chat::composer::chat_link_preview_draft::ChatLinkPreviewPreference;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::shared::utils::chat_list_utils::ChatListUtils;
use crate::modules::chat::shared::utils::voice::voice_message_metadata::VoiceMessageMetadata;
use crate::modules::chat::stores::chat_stores_hooks::chat_stores_components_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

/// composer contents when the chat was left
/// text alone isnt enough, a custom emoji is one placeholder and only the run list says which sticker
/// a mention keeps the picked user id
#[derive(Clone, Debug, PartialEq)]
pub struct ChatDraft {
    pub text: String,
    pub custom_emojis: Vec<StickerItem>,
    pub mentions: Vec<ChatComposerUserMention>,
    pub updated_at: TimeInterval,
    pub link_preview: Option<ChatLinkPreviewPreference>,
}

impl ChatDraft {
    pub fn is_empty(&self) -> bool {
        trimming_whitespaces_and_newlines(&self.text).is_empty()
    }
}

/// voice note recorded on the lock and left unsent
/// the file is in the account data folder and not a cache so clearing cache keeps it
#[derive(Clone, Debug, PartialEq)]
pub struct ChatVoiceDraft {
    pub file_name: String,
    pub metadata: VoiceMessageMetadata,
    pub updated_at: TimeInterval,
}

/// what the chat list draws for a chat with a draft in any scope
#[derive(Clone, Debug, PartialEq)]
pub struct ChatDraftPreview {
    pub text: String,
    pub updated_at: TimeInterval,
}

#[derive(Clone)]
struct PendingDiskSnapshot {
    token: u64,
    drafts: HashMap<String, ChatDraft>,
}

const LOGICAL_DISK_CACHE_KEY: &str = "chatComposerDrafts";
const VOICE_LOGICAL_DISK_CACHE_KEY: &str = "chatComposerVoiceDrafts";
const MAXIMUM_RETAINED_DRAFTS: usize = 256;
const CHECKPOINT_DELAY: Duration = Duration::from_nanos(3_000_000_000);
const FAILED_WRITE_RETRY_DELAY: Duration = Duration::from_nanos(2_000_000_000);

const VOICE_DRAFTS_FOLDER: &str = "ChatVoiceDrafts";
const VOICE_FILE_NAME_PREFIX: &str = "voice-";
const VOICE_FILE_EXTENSION: &str = ".m4a";
const MAXIMUM_DIRECTORY_COMPONENT_CHARACTERS: usize = 128;

const KEY_TEXT: &str = "text";
const KEY_UPDATED_AT: &str = "updatedAt";
const KEY_CUSTOM_EMOJIS: &str = "customEmojis";
const KEY_LINK_PREVIEW: &str = "linkPreview";
const KEY_URL: &str = "url";
const KEY_SUPPRESSED: &str = "suppressed";
const KEY_ABOVE: &str = "above";
const KEY_LARGE: &str = "large";
const KEY_MENTIONS: &str = "mentions";
const KEY_USER_ID: &str = "userId";
const KEY_TAG: &str = "tag";
const KEY_LOCATION: &str = "location";
const KEY_FILE: &str = "file";
const KEY_DURATION_MS: &str = "durationMs";
const KEY_WAVEFORM: &str = "waveform";

static DATA_ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    let paths = AppPaths::resolve(AppEnvironment::current().canon()).ok()?;
    paths.create_all().ok()?;
    Some(paths.data_dir().to_path_buf())
});

/// account scoped disk backed composer drafts
/// local only with no wire field or endpoint
/// nothing on the server knows a draft exists
pub struct ChatDraftStore {
    previews: HashMap<String, ChatDraftPreview>,
    previews_by_scope: HashMap<String, ChatDraftPreview>,
    voice_drafts: HashMap<String, ChatVoiceDraft>,
    voice_drafts_user_id: Option<String>,
    drafts_by_scope: HashMap<String, ChatDraft>,
    loaded_disk_cache_key: Option<String>,
    persistence_generation: u64,
    persisted_generation: u64,
    persistence_in_flight_generation: Option<u64>,
    persistence_task: Option<Task<()>>,
    disk_snapshot_token: u64,
    pending_disk_snapshots: HashMap<String, PendingDiskSnapshot>,
    _app_quit_subscription: Subscription,
    _window_closed_subscription: Subscription,
}

impl ChatDraftStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let app_quit_subscription = cx.on_app_quit(|this, cx| {
            this.flush_pending_to_disk(cx);
            cx.background_executor().spawn(async {
                SaiWsDiskCache::wait_for_asynchronous_writes();
            })
        });
        let weak = cx.weak_entity();
        let window_closed_subscription = cx.on_window_closed(move |cx, _window_id| {
            let _ = weak.update(cx, |this, cx| this.flush_pending_to_disk(cx));
        });
        Self {
            previews: HashMap::new(),
            previews_by_scope: HashMap::new(),
            voice_drafts: HashMap::new(),
            voice_drafts_user_id: None,
            drafts_by_scope: HashMap::new(),
            loaded_disk_cache_key: None,
            persistence_generation: 0,
            persisted_generation: 0,
            persistence_in_flight_generation: None,
            persistence_task: None,
            disk_snapshot_token: 0,
            pending_disk_snapshots: HashMap::new(),
            _app_quit_subscription: app_quit_subscription,
            _window_closed_subscription: window_closed_subscription,
        }
    }

    pub fn previews(&self) -> &HashMap<String, ChatDraftPreview> {
        &self.previews
    }

    pub fn previews_by_scope(&self) -> &HashMap<String, ChatDraftPreview> {
        &self.previews_by_scope
    }

    pub fn voice_drafts(&self) -> &HashMap<String, ChatVoiceDraft> {
        &self.voice_drafts
    }

    pub fn scope_key(chat_id: &str, topic_id: Option<&str>, thread_root_id: Option<&str>) -> String {
        MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id)
    }

    pub fn chat_id(from_scope_key: &str) -> String {
        RiseMessageScope::parse(from_scope_key)
            .map(|scope| scope.chat_id)
            .unwrap_or_else(|| from_scope_key.to_owned())
    }

    pub fn draft(&mut self, for_scope: &str, cx: &mut Context<Self>) -> Option<ChatDraft> {
        self.activate_account_state(cx);
        if for_scope.is_empty() {
            return None;
        }
        self.drafts_by_scope.get(for_scope).cloned()
    }

    pub fn set_draft(
        &mut self,
        text: &str,
        custom_emojis: Vec<StickerItem>,
        mentions: Vec<ChatComposerUserMention>,
        link_preview: Option<ChatLinkPreviewPreference>,
        for_scope: &str,
        cx: &mut Context<Self>,
    ) {
        self.activate_account_state(cx);
        if for_scope.is_empty() {
            return;
        }
        if trimming_whitespaces_and_newlines(text).is_empty() {
            self.clear(for_scope, cx);
            return;
        }
        let unchanged = self.drafts_by_scope.get(for_scope).is_some_and(|previous| {
            previous.text == text
                && previous.custom_emojis == custom_emojis
                && previous.mentions == mentions
                && previous.link_preview == link_preview
        });
        if unchanged {
            return;
        }
        self.drafts_by_scope.insert(
            for_scope.to_owned(),
            ChatDraft {
                text: text.to_owned(),
                custom_emojis,
                mentions,
                updated_at: Date::now().time_interval_since_1970(),
                link_preview,
            },
        );
        self.evict_if_needed();
        self.mark_pending_persistence(cx);
    }

    pub fn clear(&mut self, scope_key: &str, cx: &mut Context<Self>) {
        self.activate_account_state(cx);
        if scope_key.is_empty() || self.drafts_by_scope.remove(scope_key).is_none() {
            return;
        }
        self.mark_pending_persistence(cx);
        self.publish_previews(cx);
    }

    /// removes the chat draft and every topic or thread draft under it
    /// only on a confirmed removal that cant be undone
    pub fn clear_chat(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.activate_account_state(cx);
        if chat_id.is_empty() {
            return;
        }
        let removed_voice: Vec<String> = self
            .voice_drafts
            .keys()
            .filter(|key| Self::chat_id(key) == chat_id)
            .cloned()
            .collect();
        for key in removed_voice {
            self.discard_voice_draft(&key, cx);
        }
        let removed: Vec<String> = self
            .drafts_by_scope
            .keys()
            .filter(|key| Self::chat_id(key) == chat_id)
            .cloned()
            .collect();
        if removed.is_empty() {
            return;
        }
        for key in removed {
            self.drafts_by_scope.remove(&key);
        }
        self.mark_pending_persistence(cx);
        self.publish_previews(cx);
    }

    pub fn voice_draft(&self, for_scope: &str) -> Option<ChatVoiceDraft> {
        if for_scope.is_empty() {
            return None;
        }
        self.voice_drafts.get(for_scope).cloned()
    }

    pub fn voice_draft_file_url(&self, draft: &ChatVoiceDraft) -> Option<PathBuf> {
        let user_id = self.voice_drafts_user_id.as_deref()?;
        Some(Self::voice_draft_directory(user_id)?.join(&draft.file_name))
    }

    pub fn set_voice_draft(
        &mut self,
        recording_url: PathBuf,
        metadata: VoiceMessageMetadata,
        for_scope: &str,
        cx: &mut Context<Self>,
    ) {
        self.activate_account_state(cx);
        let parked = if for_scope.is_empty() {
            None
        } else {
            self.voice_drafts_user_id.clone().and_then(|user_id| {
                Self::voice_draft_directory(&user_id).map(|directory| (user_id, directory))
            })
        };
        let Some((user_id, directory)) = parked else {
            Self::remove_file_detached(recording_url, cx);
            return;
        };
        let file_name = format!("{VOICE_FILE_NAME_PREFIX}{}{VOICE_FILE_EXTENSION}", Uuid::new_v4().uuid_string());
        let target = directory.join(&file_name);
        let moved_task = {
            let source = recording_url.clone();
            let target = target.clone();
            let directory = directory.clone();
            cx.background_executor()
                .spawn(async move { Self::move_file(&source, &target, Some(&directory)) })
        };
        let scope_key = for_scope.to_owned();
        cx.spawn(async move |this, cx| {
            let moved = moved_task.await;
            let stray = if moved { target } else { recording_url };
            let applied = this.update(cx, |this, cx| {
                if !moved || this.voice_drafts_user_id.as_deref() != Some(user_id.as_str()) {
                    Self::remove_file_detached(stray.clone(), cx);
                    return;
                }
                let previous = this.voice_drafts.insert(
                    scope_key,
                    ChatVoiceDraft {
                        file_name: file_name.clone(),
                        metadata,
                        updated_at: Date::now().time_interval_since_1970(),
                    },
                );
                this.persist_voice_drafts(cx);
                this.publish_previews(cx);
                cx.notify();
                if let Some(previous) = previous
                    && previous.file_name != file_name
                {
                    Self::remove_file_detached(directory.join(&previous.file_name), cx);
                }
            });
            if applied.is_err() {
                cx.update(|cx| Self::remove_file_detached(stray, cx));
            }
        })
        .detach();
    }

    pub fn take_voice_draft(
        &mut self,
        for_scope: &str,
        cx: &mut Context<Self>,
    ) -> Task<Option<(PathBuf, VoiceMessageMetadata)>> {
        let Some(user_id) = self.voice_drafts_user_id.clone() else {
            return Task::ready(None);
        };
        let Some(draft) = self.voice_drafts.remove(for_scope) else {
            return Task::ready(None);
        };
        let Some(directory) = Self::voice_draft_directory(&user_id) else {
            return Task::ready(None);
        };
        let source = directory.join(&draft.file_name);
        self.persist_voice_drafts(cx);
        self.publish_previews(cx);
        cx.notify();
        let target = temporary_directory().join(format!(
            "{VOICE_FILE_NAME_PREFIX}{}{VOICE_FILE_EXTENSION}",
            Uuid::new_v4().uuid_string()
        ));
        let moved_task = {
            let target = target.clone();
            cx.background_executor()
                .spawn(async move { Self::move_file(&source, &target, None) })
        };
        cx.spawn(async move |_, cx| {
            let moved = moved_task.await;
            if !moved || !SaiAccountCacheScope::is_current_account(Some(user_id.as_str())) {
                if moved {
                    cx.update(|cx| Self::remove_file_detached(target, cx));
                }
                return None;
            }
            Some((target, draft.metadata))
        })
    }

    pub fn discard_voice_draft(&mut self, for_scope: &str, cx: &mut Context<Self>) {
        let Some(user_id) = self.voice_drafts_user_id.clone() else {
            return;
        };
        let Some(draft) = self.voice_drafts.remove(for_scope) else {
            return;
        };
        self.persist_voice_drafts(cx);
        self.publish_previews(cx);
        cx.notify();
        if let Some(directory) = Self::voice_draft_directory(&user_id) {
            Self::remove_file_detached(directory.join(&draft.file_name), cx);
        }
    }

    /// called when the chat screen is going away and the list is about to be seen
    /// typing itself never publishes
    pub fn publish_pending_previews(&mut self, cx: &mut Context<Self>) {
        self.publish_previews(cx);
    }

    pub fn persisted_disk_cache_keys() -> Vec<String> {
        SaiWsDiskCache::all_indexed_cache_keys()
            .into_iter()
            .filter(|key| {
                let logical = SaiAccountCacheScope::logical_key(key);
                logical == LOGICAL_DISK_CACHE_KEY || logical == VOICE_LOGICAL_DISK_CACHE_KEY
            })
            .collect()
    }

    pub fn clear_all(&mut self, cx: &mut Context<Self>) {
        self.persistence_task = None;
        self.persistence_generation = self.persistence_generation.wrapping_add(1);
        self.persisted_generation = self.persistence_generation;
        self.persistence_in_flight_generation = None;
        self.pending_disk_snapshots.clear();
        self.drafts_by_scope.clear();
        self.previews.clear();
        self.previews_by_scope.clear();
        self.voice_drafts.clear();
        self.voice_drafts_user_id = None;
        self.loaded_disk_cache_key = None;
        SaiWsDiskCache::wait_for_asynchronous_writes();
        for key in Self::persisted_disk_cache_keys() {
            SaiWsDiskCache::remove(&key);
        }
        if let Some(root) = Self::voice_draft_root_directory() {
            Self::remove_file_detached(root, cx);
        }
        cx.notify();
    }

    pub fn reset_account_state(&mut self, persist_pending: bool, cx: &mut Context<Self>) {
        if persist_pending {
            self.persist_current_snapshot_before_reset(cx);
        } else {
            self.pending_disk_snapshots.clear();
        }
        self.persistence_task = None;
        self.persistence_generation = self.persistence_generation.wrapping_add(1);
        self.persisted_generation = self.persistence_generation;
        self.persistence_in_flight_generation = None;
        self.drafts_by_scope.clear();
        self.previews.clear();
        self.previews_by_scope.clear();
        self.voice_drafts.clear();
        self.voice_drafts_user_id = None;
        self.loaded_disk_cache_key = None;
        cx.notify();
    }

    pub fn activate_account_state(&mut self, cx: &mut Context<Self>) {
        let cache_key = SaiAccountCacheScope::scoped_key(LOGICAL_DISK_CACHE_KEY);
        if self.loaded_disk_cache_key.as_deref() == Some(cache_key.as_str()) {
            return;
        }
        self.persist_current_snapshot_before_reset(cx);
        self.persistence_task = None;
        self.persistence_in_flight_generation = None;
        let pending_snapshot = self.pending_disk_snapshots.get(&cache_key).map(|snapshot| snapshot.drafts.clone());
        let has_pending_snapshot = pending_snapshot.is_some();
        self.drafts_by_scope = pending_snapshot.unwrap_or_else(|| Self::load_from_disk(&cache_key));
        self.voice_drafts_user_id = SaiAccountCacheScope::active_user_id();
        self.voice_drafts = self
            .voice_drafts_user_id
            .as_deref()
            .map(Self::load_voice_drafts)
            .unwrap_or_default();
        self.loaded_disk_cache_key = Some(cache_key);
        self.persistence_generation = self.persistence_generation.wrapping_add(1);
        self.persisted_generation = self.persistence_generation;
        if has_pending_snapshot {
            self.persistence_generation = self.persistence_generation.wrapping_add(1);
            self.schedule_safety_checkpoint(CHECKPOINT_DELAY, cx);
        }
        self.publish_previews(cx);
        cx.notify();
    }

    pub fn remove_persisted_account(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let prefix = SaiAccountCacheScope::prefix(user_id);
        self.pending_disk_snapshots.retain(|key, _| !key.starts_with(&prefix));
        if let Some(directory) = Self::voice_draft_directory(user_id) {
            Self::remove_file_detached(directory, cx);
        }
    }

    pub fn flush_pending_to_disk(&mut self, cx: &mut Context<Self>) {
        self.activate_account_state(cx);
        self.persistence_task = None;
        if self.persisted_generation == self.persistence_generation
            || self.persistence_in_flight_generation.is_some()
        {
            return;
        }
        let Some(loaded_disk_cache_key) = self.loaded_disk_cache_key.clone() else {
            return;
        };
        let generation = self.persistence_generation;
        let snapshot = self.drafts_by_scope.clone();
        self.persistence_in_flight_generation = Some(generation);
        let completion_key = loaded_disk_cache_key.clone();
        self.save_asynchronously(
            loaded_disk_cache_key,
            snapshot,
            move |this, result, cx| {
                if this.loaded_disk_cache_key.as_deref() != Some(completion_key.as_str())
                    || this.persistence_in_flight_generation != Some(generation)
                {
                    return;
                }
                this.persistence_in_flight_generation = None;
                match result {
                    Ok(()) => {
                        if this.persistence_generation == generation {
                            this.persisted_generation = generation;
                        } else {
                            this.flush_pending_to_disk(cx);
                        }
                    }
                    Err(_) => this.schedule_safety_checkpoint(FAILED_WRITE_RETRY_DELAY, cx),
                }
            },
            cx,
        );
    }

    fn publish_previews(&mut self, cx: &mut Context<Self>) {
        let mut by_scope: HashMap<String, ChatDraftPreview> = HashMap::with_capacity(self.drafts_by_scope.len());
        let mut by_chat: HashMap<String, ChatDraftPreview> = HashMap::with_capacity(self.drafts_by_scope.len());
        for (scope_key, draft) in &self.drafts_by_scope {
            let preview = ChatDraftPreview {
                text: Self::preview_text(draft),
                updated_at: draft.updated_at,
            };
            by_scope.insert(scope_key.clone(), preview.clone());
            let chat_id = Self::chat_id(scope_key);
            if chat_id.is_empty() {
                continue;
            }
            if by_chat.get(&chat_id).is_some_and(|existing| existing.updated_at >= draft.updated_at) {
                continue;
            }
            by_chat.insert(chat_id, preview);
        }
        if !self.voice_drafts.is_empty() {
            let voice_text = tr("msg_preview_voice");
            for (scope_key, voice) in &self.voice_drafts {
                let preview = ChatDraftPreview {
                    text: voice_text.clone(),
                    updated_at: voice.updated_at,
                };
                let same_scope_text = by_scope.insert(scope_key.clone(), preview.clone());
                let chat_id = Self::chat_id(scope_key);
                if chat_id.is_empty() {
                    continue;
                }
                let keeps_existing = by_chat.get(&chat_id).is_some_and(|existing| {
                    Some(existing) != same_scope_text.as_ref() && existing.updated_at > voice.updated_at
                });
                if keeps_existing {
                    continue;
                }
                by_chat.insert(chat_id, preview);
            }
        }
        let mut did_change = false;
        if by_scope != self.previews_by_scope {
            self.previews_by_scope = by_scope;
            did_change = true;
        }
        if by_chat != self.previews {
            self.previews = by_chat;
            did_change = true;
        }
        if did_change {
            cx.notify();
        }
    }

    /// composer keeps a custom emoji as one placeholder char
    /// left in place it shows as a replacement box in the list row
    fn preview_text(draft: &ChatDraft) -> String {
        let plain = match chat_stores_components_hooks() {
            Some(hooks) => (hooks.plain_custom_emoji)(&draft.text),
            None => draft.text.clone(),
        };
        let flattened = plain.replace('\n', " ");
        trimming_whitespaces_and_newlines(&flattened).to_owned()
    }

    fn evict_if_needed(&mut self) {
        if self.drafts_by_scope.len() <= MAXIMUM_RETAINED_DRAFTS {
            return;
        }
        let excess = self.drafts_by_scope.len() - MAXIMUM_RETAINED_DRAFTS;
        let mut ordered: Vec<(String, TimeInterval)> = self
            .drafts_by_scope
            .iter()
            .map(|(key, draft)| (key.clone(), draft.updated_at))
            .collect();
        ordered.sort_by(|left, right| left.1.total_cmp(&right.1));
        for (key, _) in ordered.into_iter().take(excess) {
            self.drafts_by_scope.remove(&key);
        }
    }

    fn mark_pending_persistence(&mut self, cx: &mut Context<Self>) {
        self.persistence_generation = self.persistence_generation.wrapping_add(1);
        self.schedule_safety_checkpoint(CHECKPOINT_DELAY, cx);
    }

    fn schedule_safety_checkpoint(&mut self, delay: Duration, cx: &mut Context<Self>) {
        self.persistence_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| this.flush_pending_to_disk(cx));
        }));
    }

    fn persist_current_snapshot_before_reset(&mut self, cx: &mut Context<Self>) {
        if self.persisted_generation == self.persistence_generation {
            return;
        }
        let Some(loaded_disk_cache_key) = self.loaded_disk_cache_key.clone() else {
            return;
        };
        let drafts = self.drafts_by_scope.clone();
        let retry_key = loaded_disk_cache_key.clone();
        self.save_asynchronously(
            loaded_disk_cache_key,
            drafts,
            move |this, result, cx| {
                if result.is_ok() {
                    return;
                }
                let Some(pending) = this.pending_disk_snapshots.get(&retry_key).map(|pending| pending.drafts.clone())
                else {
                    return;
                };
                this.save_asynchronously(retry_key, pending, |_, _, _| {}, cx);
            },
            cx,
        );
    }

    fn save_asynchronously(
        &mut self,
        cache_key: String,
        drafts: HashMap<String, ChatDraft>,
        completion: impl FnOnce(&mut Self, Result<(), SaiWsDiskCacheError>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.disk_snapshot_token = self.disk_snapshot_token.wrapping_add(1);
        let token = self.disk_snapshot_token;
        let payload = Self::persistence_payload(&drafts);
        self.pending_disk_snapshots.insert(cache_key.clone(), PendingDiskSnapshot { token, drafts });
        let disk_lease = begin_process_activity("ChatDraftCheckpoint");
        let weak = cx.weak_entity();
        let snapshot_key = cache_key.clone();
        SaiWsDiskCache::save_asynchronously(
            cache_key,
            payload,
            move |result, cx| {
                let _disk_lease = disk_lease;
                let _ = weak.update(cx, |this, cx| {
                    if result.is_ok()
                        && this.pending_disk_snapshots.get(&snapshot_key).map(|pending| pending.token) == Some(token)
                    {
                        this.pending_disk_snapshots.remove(&snapshot_key);
                    }
                    completion(this, result, cx);
                });
            },
            cx,
        );
    }

    fn persist_voice_drafts(&self, cx: &mut Context<Self>) {
        let Some(user_id) = self.voice_drafts_user_id.as_deref() else {
            return;
        };
        let mut payload = Map::with_capacity(self.voice_drafts.len());
        for (scope_key, draft) in &self.voice_drafts {
            let mut value = Map::with_capacity(4);
            value.insert(KEY_FILE.to_owned(), Value::from(draft.file_name.clone()));
            value.insert(KEY_DURATION_MS.to_owned(), Value::from(draft.metadata.duration_milliseconds));
            value.insert(
                KEY_WAVEFORM.to_owned(),
                Value::Array(draft.metadata.waveform.iter().map(|sample| Value::from(f64::from(*sample))).collect()),
            );
            value.insert(KEY_UPDATED_AT.to_owned(), Value::from(draft.updated_at));
            payload.insert(scope_key.clone(), Value::Object(value));
        }
        SaiWsDiskCache::save_asynchronously(
            format!("{}{VOICE_LOGICAL_DISK_CACHE_KEY}", SaiAccountCacheScope::prefix(user_id)),
            Value::Object(payload),
            |_, _| {},
            cx,
        );
    }

    fn load_voice_drafts(user_id: &str) -> HashMap<String, ChatVoiceDraft> {
        let cache_key = format!("{}{VOICE_LOGICAL_DISK_CACHE_KEY}", SaiAccountCacheScope::prefix(user_id));
        let Some(Value::Object(raw)) = SaiWsDiskCache::load(&cache_key) else {
            return HashMap::new();
        };
        let Some(directory) = Self::voice_draft_directory(user_id) else {
            return HashMap::new();
        };
        let mut out = HashMap::with_capacity(raw.len());
        for (scope_key, value) in raw {
            let Some(dictionary) = value.as_object() else {
                continue;
            };
            let Some(file_name) = dictionary.get(KEY_FILE).and_then(Value::as_str).filter(|name| !name.is_empty())
            else {
                continue;
            };
            let Some(duration_ms) = dictionary.get(KEY_DURATION_MS).and_then(Self::integer).filter(|ms| *ms > 0) else {
                continue;
            };
            if !FileManager.file_exists(&directory.join(file_name)) {
                continue;
            }
            let waveform = dictionary
                .get(KEY_WAVEFORM)
                .and_then(Value::as_array)
                .filter(|samples| samples.iter().all(Value::is_number))
                .map(|samples| samples.iter().filter_map(Value::as_f64).map(|sample| sample as f32).collect())
                .unwrap_or_default();
            out.insert(
                scope_key,
                ChatVoiceDraft {
                    file_name: file_name.to_owned(),
                    metadata: VoiceMessageMetadata {
                        waveform,
                        duration_seconds: (duration_ms / 1000).max(1),
                        duration_milliseconds: duration_ms,
                    },
                    updated_at: dictionary.get(KEY_UPDATED_AT).and_then(Value::as_f64).unwrap_or(0.0),
                },
            );
        }
        out
    }

    fn voice_draft_root_directory() -> Option<PathBuf> {
        Some(DATA_ROOT.as_ref()?.join(VOICE_DRAFTS_FOLDER))
    }

    fn voice_draft_directory(user_id: &str) -> Option<PathBuf> {
        let trimmed = trimming_whitespaces_and_newlines(user_id);
        if trimmed.is_empty() {
            return None;
        }
        let component: String = trimmed
            .chars()
            .map(|scalar| if scalar.is_alphanumeric() || matches!(scalar, '-' | '_') { scalar } else { '_' })
            .take(MAXIMUM_DIRECTORY_COMPONENT_CHARACTERS)
            .collect();
        Some(Self::voice_draft_root_directory()?.join(component))
    }

    fn move_file(source: &Path, target: &Path, creating: Option<&Path>) -> bool {
        if let Some(directory) = creating {
            if FileManager.create_directory(directory, true).is_err() {
                return false;
            }
            let values = URLResourceValues {
                is_excluded_from_backup: Some(true),
            };
            let _ = set_resource_values(directory, &values);
        }
        FileManager.move_item(source, target).is_ok()
    }

    fn remove_file_detached(url: PathBuf, cx: &App) {
        cx.background_executor()
            .spawn(async move {
                let _ = FileManager.remove_item(&url);
            })
            .detach();
    }

    fn persistence_payload(drafts: &HashMap<String, ChatDraft>) -> Value {
        let mut payload = Map::with_capacity(drafts.len());
        for (scope_key, draft) in drafts {
            let mut value = Map::new();
            value.insert(KEY_TEXT.to_owned(), Value::from(draft.text.clone()));
            value.insert(KEY_UPDATED_AT.to_owned(), Value::from(draft.updated_at));
            let emojis: Vec<Value> = draft
                .custom_emojis
                .iter()
                .filter_map(|item| ChatComposerCustomEmojiAttachmentPayload::new(item).encoded_string())
                .map(Value::from)
                .collect();
            if !emojis.is_empty() {
                value.insert(KEY_CUSTOM_EMOJIS.to_owned(), Value::Array(emojis));
            }
            if let Some(preview) = &draft.link_preview {
                let mut link_preview = Map::with_capacity(4);
                link_preview.insert(KEY_URL.to_owned(), Value::from(preview.url.clone().unwrap_or_default()));
                link_preview.insert(KEY_SUPPRESSED.to_owned(), Value::from(preview.is_suppressed));
                link_preview.insert(KEY_ABOVE.to_owned(), Value::from(preview.is_above_text));
                link_preview.insert(KEY_LARGE.to_owned(), Value::from(preview.is_large));
                value.insert(KEY_LINK_PREVIEW.to_owned(), Value::Object(link_preview));
            }
            if !draft.mentions.is_empty() {
                let mentions = draft
                    .mentions
                    .iter()
                    .map(|mention| {
                        let mut entry = Map::with_capacity(4);
                        entry.insert(KEY_USER_ID.to_owned(), Value::from(mention.user_id.clone()));
                        entry.insert(KEY_TAG.to_owned(), Value::from(mention.tag.clone()));
                        entry.insert(KEY_LOCATION.to_owned(), Value::from(mention.range.location));
                        entry.insert(
                            ChatMessageEntityField::Length.raw_value().to_owned(),
                            Value::from(mention.range.length),
                        );
                        Value::Object(entry)
                    })
                    .collect();
                value.insert(KEY_MENTIONS.to_owned(), Value::Array(mentions));
            }
            payload.insert(scope_key.clone(), Value::Object(value));
        }
        Value::Object(payload)
    }

    fn load_from_disk(cache_key: &str) -> HashMap<String, ChatDraft> {
        let Some(Value::Object(raw)) = SaiWsDiskCache::load(cache_key) else {
            return HashMap::new();
        };
        let mut out = HashMap::with_capacity(raw.len());
        for (scope_key, value) in raw {
            let Some(dictionary) = value.as_object() else {
                continue;
            };
            let Some(text) = dictionary
                .get(KEY_TEXT)
                .and_then(Value::as_str)
                .filter(|text| !trimming_whitespaces_and_newlines(text).is_empty())
            else {
                continue;
            };
            let updated_at = dictionary.get(KEY_UPDATED_AT).and_then(Value::as_f64).unwrap_or(0.0);
            let custom_emojis = dictionary
                .get(KEY_CUSTOM_EMOJIS)
                .and_then(Value::as_array)
                .map(|emojis| {
                    emojis
                        .iter()
                        .filter_map(|emoji| ChatComposerCustomEmojiAttachmentPayload::decode(emoji.as_str()))
                        .map(|payload| payload.item())
                        .collect()
                })
                .unwrap_or_default();
            let mentions = dictionary
                .get(KEY_MENTIONS)
                .and_then(Value::as_array)
                .filter(|entries| entries.iter().all(Value::is_object))
                .map(|entries| entries.iter().filter_map(Self::mention).collect())
                .unwrap_or_default();
            let link_preview = dictionary.get(KEY_LINK_PREVIEW).and_then(Value::as_object).map(|preview| {
                ChatLinkPreviewPreference {
                    url: preview.get(KEY_URL).and_then(Value::as_str).map(str::to_owned),
                    is_suppressed: preview.get(KEY_SUPPRESSED).and_then(Value::as_bool).unwrap_or(false),
                    is_above_text: preview.get(KEY_ABOVE).and_then(Value::as_bool).unwrap_or(false),
                    is_large: preview.get(KEY_LARGE).and_then(Value::as_bool).unwrap_or(true),
                }
            });
            out.insert(
                scope_key,
                ChatDraft {
                    text: text.to_owned(),
                    custom_emojis,
                    mentions,
                    updated_at,
                    link_preview,
                },
            );
        }
        out
    }

    fn mention(entry: &Value) -> Option<ChatComposerUserMention> {
        let location = usize::try_from(entry.get(KEY_LOCATION).and_then(Self::integer)?).ok()?;
        let length =
            usize::try_from(entry.get(ChatMessageEntityField::Length.raw_value()).and_then(Self::integer)?).ok()?;
        Some(ChatComposerUserMention {
            user_id: entry.get(KEY_USER_ID)?.as_str()?.to_owned(),
            tag: entry.get(KEY_TAG)?.as_str()?.to_owned(),
            range: NSRange { location, length },
        })
    }

    fn integer(value: &Value) -> Option<i64> {
        value.as_i64().or_else(|| value.as_f64().map(|number| number as i64))
    }
}

/// a draft moves a chat like a message does, list is ordered by last activity and typing counts
/// presentation only, nothing writes updated_at
pub enum ChatDraftOrdering {}

impl ChatDraftOrdering {
    pub fn applied(to: Vec<ChatInfo>, previews: &HashMap<String, ChatDraftPreview>) -> Vec<ChatInfo> {
        if previews.is_empty() || to.len() <= 1 {
            return to;
        }
        let moves_any_row = to.iter().any(|chat| !chat.is_pinned && previews.contains_key(&chat.id));
        if !moves_any_row {
            return to;
        }
        let mut pinned = Vec::new();
        let mut unpinned = Vec::new();
        for (offset, chat) in to.into_iter().enumerate() {
            if chat.is_pinned {
                pinned.push(chat);
            } else {
                unpinned.push((Self::activity(&chat, previews), offset, chat));
            }
        }
        unpinned.sort_by(|left, right| right.0.total_cmp(&left.0).then(left.1.cmp(&right.1)));
        pinned.extend(unpinned.into_iter().map(|(_, _, chat)| chat));
        pinned
    }

    pub fn activity(chat: &ChatInfo, previews: &HashMap<String, ChatDraftPreview>) -> TimeInterval {
        ChatListUtils::last_activity(chat).max(previews.get(&chat.id).map_or(0.0, |preview| preview.updated_at))
    }
}
