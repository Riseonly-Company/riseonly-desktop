use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::file_manager::{DirectoryEnumerationOptions, FileAttributeType, FileManager};
use rise_platform::url_resource_values::total_file_allocated_size;

use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::media::remote_media_disk_cache::RemoteMediaDiskCache;
use crate::core::media::rise_media_kind_classifier::{RiseMediaKind, RiseMediaKindClassifier};
use crate::core::storage::rise_chat_media_ledger::{RiseChatMediaFiles, RiseChatMediaLedger};
use crate::core::storage::rise_storage_maintenance::RiseStorageMaintenance;
use crate::core::storage::rise_storage_site_catalog::{RiseStorageSiteCatalog, RiseStorageSiteCatalogResolved};
use crate::core::storage::rise_storage_usage_models::{
    RiseStorageBucket, RiseStorageChatUsage, RiseStorageMediaEntry, RiseStorageSite, RiseStorageUsageSnapshot,
};
use crate::core::ui::animated_gif_view::AnimatedImageSourcePolicy;
use crate::core::ui::clever_image_disk_cache::CleverImageDiskCache;
use crate::modules::chat::shared::utils::voice::voice_message_disk_cache::VoiceMessageDiskCache;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

const MAXIMUM_VISITED_FILES: usize = 200_000;
const CANCELLATION_CHECK_STRIDE: usize = 512;
const MAXIMUM_MEDIA_ENTRIES: usize = 300;

static SHARED: LazyLock<RiseStorageScanner> = LazyLock::new(|| RiseStorageScanner {
    isolation: Mutex::new(()),
});

#[derive(Default)]
struct Measured {
    total: i64,
    by_bucket: HashMap<RiseStorageBucket, i64>,
}

/// measures what the app uses on disk and gives it back
/// measuring reads the filesystem, never a cache listing api since some of those prune while answering
/// purge only via the owning cache clear_all, raw remove_item lets downloads sneak back in
pub struct RiseStorageScanner {
    isolation: Mutex<()>,
}

impl RiseStorageScanner {
    pub fn shared() -> &'static RiseStorageScanner {
        &SHARED
    }

    pub fn scan(&self, account_user_id: Option<&str>, cancelled: &AtomicBool) -> RiseStorageUsageSnapshot {
        let _isolation = self.isolation.lock();
        let mut bucket_bytes: HashMap<RiseStorageBucket, i64> = HashMap::new();
        let mut site_bytes: HashMap<RiseStorageSite, i64> = HashMap::new();

        for site in RiseStorageSite::ALL_CASES {
            if cancelled.load(Ordering::Relaxed) {
                break;
            }
            let resolved = RiseStorageSiteCatalog::resolve(site, account_user_id);
            let measured = self.measure(&resolved, account_user_id, cancelled);
            if measured.total <= 0 {
                continue;
            }
            *site_bytes.entry(site).or_insert(0) += measured.total;
            for (bucket, bytes) in measured.by_bucket {
                *bucket_bytes.entry(bucket).or_insert(0) += bytes;
            }
        }

        RiseStorageUsageSnapshot {
            account_key: account_user_id.unwrap_or_default().to_owned(),
            bucket_bytes,
            site_bytes,
            scanned_at: RiseClock::now(),
        }
    }

    pub fn chat_usage(
        &self,
        titles: &HashMap<String, String>,
        history_bytes: &HashMap<String, i64>,
        account_user_id: Option<&str>,
        cancelled: &AtomicBool,
    ) -> Vec<RiseStorageChatUsage> {
        let _isolation = self.isolation.lock();
        let voice = VoiceMessageDiskCache::chat_usage(account_user_id);
        let mut names: HashMap<String, String> = HashMap::new();
        let mut voice_bytes: HashMap<String, i64> = HashMap::new();
        for entry in voice {
            names.insert(entry.chat_id.clone(), entry.chat_name);
            voice_bytes.insert(entry.chat_id, entry.byte_count);
        }

        let noted_names = RiseChatMediaLedger::chat_names(account_user_id);
        let noted_media = RiseChatMediaLedger::snapshot(account_user_id);

        let mut payload_bytes: HashMap<String, i64> = HashMap::new();
        let mut page_owners: HashSet<String> = titles.keys().cloned().collect();
        page_owners.extend(voice_bytes.keys().cloned());
        page_owners.extend(noted_names.keys().cloned());
        page_owners.extend(noted_media.keys().cloned());
        page_owners.extend(history_bytes.keys().cloned());
        page_owners.extend(Self::chat_ids_with_cached_pages());
        for chat_id in page_owners {
            let bytes: i64 = MessageServicesStore::chat_disk_cache_keys(&chat_id)
                .iter()
                .map(|key| SaiWsDiskCache::debug_payload_file_bytes(key).unwrap_or(0))
                .sum();
            if bytes > 0 {
                payload_bytes.insert(chat_id, bytes);
            }
        }

        let media_bytes = Self::chat_media_bytes(&noted_media, account_user_id, cancelled);

        let mut chat_ids: HashSet<&String> = voice_bytes.keys().collect();
        chat_ids.extend(payload_bytes.keys());
        chat_ids.extend(media_bytes.keys());
        chat_ids.extend(history_bytes.keys());
        let mut rows: Vec<RiseStorageChatUsage> = chat_ids
            .into_iter()
            .filter_map(|chat_id| {
                let voice = voice_bytes.get(chat_id).copied().unwrap_or(0);
                let payload = payload_bytes.get(chat_id).copied().unwrap_or(0);
                let media = media_bytes.get(chat_id).copied().unwrap_or(0);
                let history = history_bytes.get(chat_id).copied().unwrap_or(0).max(0);
                if voice + payload + media + history <= 0 {
                    return None;
                }
                let known = titles.get(chat_id).or_else(|| noted_names.get(chat_id)).or_else(|| names.get(chat_id));
                if known.is_none() && ChatServicesStore::is_draft_direct_chat_id(chat_id) {
                    return None;
                }
                let title = known.unwrap_or(chat_id).clone();
                Some(RiseStorageChatUsage::new(chat_id.clone(), title, voice, payload, media, history))
            })
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.total_bytes()));
        rows
    }

    fn chat_media_bytes(
        noted: &HashMap<String, Vec<String>>,
        account_user_id: Option<&str>,
        cancelled: &AtomicBool,
    ) -> HashMap<String, i64> {
        if noted.is_empty() {
            return HashMap::new();
        }
        let files = RiseChatMediaFiles::new(account_user_id);
        let mut result: HashMap<String, i64> = HashMap::new();
        for (chat_id, urls) in noted {
            if cancelled.load(Ordering::Relaxed) {
                break;
            }
            let bytes = files.bytes(urls);
            if bytes > 0 {
                result.insert(chat_id.clone(), bytes);
            }
        }
        result
    }

    fn chat_ids_with_cached_pages() -> HashSet<String> {
        let prefixes = [
            MessageServicesStore::messages_cache_key(""),
            MessageServicesStore::latest_messages_cache_key(""),
        ];
        let mut result: HashSet<String> = HashSet::new();
        for key in SaiWsDiskCache::all_indexed_cache_keys() {
            for prefix in &prefixes {
                let Some(rest) = key.strip_prefix(prefix.as_str()) else {
                    continue;
                };
                let mut chat_id = rest;
                for marker in [MessageServicesStore::TOPIC_SCOPE_MARKER, MessageServicesStore::THREAD_SCOPE_MARKER] {
                    if let Some(index) = chat_id.find(marker) {
                        chat_id = &chat_id[..index];
                    }
                }
                if !chat_id.is_empty() {
                    result.insert(chat_id.to_owned());
                }
            }
        }
        result
    }

    /// newest cached media, capped so the grid stays a bounded list
    /// count is every entry found before the cap
    pub fn media_entries(&self, account_user_id: Option<&str>) -> (Vec<RiseStorageMediaEntry>, usize) {
        let _isolation = self.isolation.lock();
        let mut all: Vec<RiseStorageMediaEntry> = Vec::new();
        let mut image_index_by_key: HashMap<String, usize> = HashMap::new();
        let mut animation_payloads: Vec<(String, RiseStorageMediaEntry)> = Vec::new();

        for entry in CleverImageDiskCache::list_entries_read_only(account_user_id) {
            if entry.file_bytes <= 0 {
                continue;
            }
            let media_entry = RiseStorageMediaEntry {
                id: format!("image:{}", entry.digest),
                site: RiseStorageSite::Images,
                kind: RiseMediaKind::Image,
                preview_source: Some(entry.cache_key.clone()),
                byte_count: entry.file_bytes,
                modified_at: entry.modified_at,
            };
            if let Some(source) = AnimatedImageSourcePolicy::source_key(&entry.cache_key) {
                animation_payloads.push((source, media_entry));
                continue;
            }
            image_index_by_key.insert(CleverImageDiskCache::normalized_cache_key(&entry.cache_key), all.len());
            all.push(media_entry);
        }

        for (source, payload) in animation_payloads {
            if let Some(index) = image_index_by_key.get(&source).copied() {
                let poster = &mut all[index];
                poster.byte_count += payload.byte_count;
                poster.modified_at = poster.modified_at.max(payload.modified_at);
            } else {
                image_index_by_key.insert(source.clone(), all.len());
                all.push(RiseStorageMediaEntry {
                    preview_source: Some(source),
                    ..payload
                });
            }
        }

        for entry in RemoteMediaDiskCache::list_entries(account_user_id) {
            if entry.byte_count <= 0 {
                continue;
            }
            all.push(RiseStorageMediaEntry {
                id: format!("{}{}", RiseStorageMediaEntry::REMOTE_MEDIA_ID_PREFIX, entry.id),
                site: RiseStorageSite::RemoteMedia,
                kind: RiseMediaKindClassifier::kind_file_extension(
                    entry.content_type.as_deref(),
                    &Self::stored_extension(&entry.file_name),
                ),
                preview_source: None,
                byte_count: entry.byte_count,
                modified_at: entry.last_accessed_at,
            });
        }

        all.sort_by_key(|entry| std::cmp::Reverse(entry.modified_at));
        let total = all.len();
        all.truncate(MAXIMUM_MEDIA_ENTRIES);
        (all, total)
    }

    fn stored_extension(file_name: &str) -> String {
        file_name.split_once('.').map(|(_, ext)| ext.to_lowercase()).unwrap_or_default()
    }

    fn measure(
        &self,
        resolved: &RiseStorageSiteCatalogResolved,
        account_user_id: Option<&str>,
        cancelled: &AtomicBool,
    ) -> Measured {
        let mut measured = Measured::default();
        let classifier = Self::remote_media_classifier(resolved.site, account_user_id);
        let contributes_to_buckets = resolved.site.is_clearable();
        let kept_digests: HashSet<String> = if resolved.site == RiseStorageSite::WsPayloads {
            SaiWsDiskCache::payload_digests(&RiseStorageMaintenance::preserved_payload_cache_keys())
        } else {
            HashSet::new()
        };

        for directory in &resolved.directories {
            Self::walk(directory, cancelled, &mut |url, bytes| {
                if contributes_to_buckets {
                    let name = Self::last_path_component(url);
                    if name.starts_with('_') {
                        return;
                    }
                    if resolved.site == RiseStorageSite::WsPayloads {
                        let Some(digest) = SaiWsDiskCache::payload_digest(&name) else {
                            return;
                        };
                        if kept_digests.contains(&digest) {
                            return;
                        }
                    }
                }
                measured.total += bytes;
                if !contributes_to_buckets {
                    return;
                }
                let bucket = resolved
                    .fixed_bucket
                    .or_else(|| classifier.as_ref().map(|classify| classify(url)))
                    .unwrap_or(RiseStorageBucket::Other);
                *measured.by_bucket.entry(bucket).or_insert(0) += bytes;
            });
        }
        measured
    }

    fn remote_media_classifier(
        site: RiseStorageSite,
        account_user_id: Option<&str>,
    ) -> Option<Box<dyn Fn(&Path) -> RiseStorageBucket>> {
        if site != RiseStorageSite::RemoteMedia {
            return None;
        }
        let mut content_type_by_digest: HashMap<String, String> = HashMap::new();
        for entry in RemoteMediaDiskCache::list_entries(account_user_id) {
            if let Some(content_type) = entry.content_type {
                content_type_by_digest.insert(entry.id, content_type);
            }
        }
        Some(Box::new(move |url: &Path| {
            let name = Self::last_path_component(url);
            let Some((digest, ext)) = name.split_once('.') else {
                return RiseStorageBucket::Files;
            };
            let kind = RiseMediaKindClassifier::kind_file_extension(
                content_type_by_digest.get(digest).map(String::as_str),
                &ext.to_lowercase(),
            );
            RiseStorageBucket::from(kind)
        }))
    }

    fn last_path_component(url: &Path) -> String {
        url.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// depth first and pre order, a directory counts as visited and then its children follow
    /// symlinks are listed but never followed
    fn walk(directory: &Path, cancelled: &AtomicBool, visit: &mut dyn FnMut(&Path, i64)) {
        let Ok(children) = FileManager.contents_of_directory_at(directory, DirectoryEnumerationOptions::default())
        else {
            return;
        };
        let mut pending: Vec<PathBuf> = children.into_iter().rev().collect();

        let mut visited: usize = 0;
        while let Some(url) = pending.pop() {
            visited += 1;
            if visited > MAXIMUM_VISITED_FILES {
                return;
            }
            if visited % CANCELLATION_CHECK_STRIDE == 0 && cancelled.load(Ordering::Relaxed) {
                return;
            }
            let Ok(attributes) = FileManager.attributes_of_item(&url) else {
                continue;
            };
            match attributes.r#type {
                FileAttributeType::TypeDirectory => {
                    if let Ok(children) = FileManager.contents_of_directory_at(&url, DirectoryEnumerationOptions::default())
                    {
                        pending.extend(children.into_iter().rev());
                    }
                    continue;
                }
                FileAttributeType::TypeRegular => {}
                FileAttributeType::TypeSymbolicLink | FileAttributeType::TypeUnknown => continue,
            }
            let bytes = total_file_allocated_size(&url).unwrap_or(attributes.size);
            if bytes <= 0 {
                continue;
            }
            visit(&url, bytes);
        }
    }
}
