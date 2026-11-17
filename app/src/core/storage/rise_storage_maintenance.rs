use std::collections::{HashMap, HashSet};

use gpui::{App, Task};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::media::remote_media_disk_cache::RemoteMediaDiskCache;
use crate::core::media::rise_media_transfer_hub::RiseMediaTransferHub;
use crate::core::media::url_cache::URLCache;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::core::storage::rise_chat_media_ledger::{RiseChatMediaFiles, RiseChatMediaLedger};
use crate::core::storage::rise_storage_usage_models::{RiseStorageMediaEntry, RiseStorageSite};
use crate::core::ui::animated_gif_view::AnimatedImageSourcePolicy;
use crate::core::ui::clever_image_disk_cache::{CleverImageDiskCache, CleverImageRenderMemoryCache};
use crate::core::ui::lottie_sticker::lottie_sticker_loader::LottieStickerLoader;
use crate::core::ui::lottie_sticker::rise_lottie_frame_cache::RiseLottieFrameCache;
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::RiseLottieFirstFrameCache;
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatDraftStore;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::ChatScrollPositionStore;
use crate::modules::chat::shared::utils::voice::voice_message_disk_cache::VoiceMessageDiskCache;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;
use crate::modules::notify::engine::core::rise_release_media_warmer::RiseReleaseMediaWarmer;

/// frees disk by asking each cache to empty itself and keep its manifest in step
/// nothing here deletes a directory it doesnt own, images and remote media fence writers with a token
/// other caches dont fence so a file can be left behind until the next purge
pub enum RiseStorageMaintenance {}

impl RiseStorageMaintenance {
    pub fn clear(sites: &HashSet<RiseStorageSite>, account_user_id: Option<&str>, cx: &mut App) -> Task<()> {
        let clearable: Vec<RiseStorageSite> = RiseStorageSite::ALL_CASES
            .into_iter()
            .filter(|site| sites.contains(site) && site.is_clearable())
            .collect();
        if clearable.is_empty() {
            return Task::ready(());
        }

        if clearable.contains(&RiseStorageSite::WsPayloads) {
            SaiWsResponseCache::prepare_disk_clear(cx);
        }
        let clears_stickers = clearable.contains(&RiseStorageSite::Stickers);
        let account_user_id = account_user_id.map(str::to_owned);
        let runtime = RiseAccountRuntime::current();

        cx.spawn(async move |_cx| {
            if clears_stickers {
                LottieStickerLoader::shared().clear_disk_cache().await;
            }

            let worker_runtime = runtime.clone();
            let work = runtime.spawn_blocking(move || {
                let account_user_id = account_user_id.as_deref();
                for site in clearable {
                    match site {
                        RiseStorageSite::WsPayloads => {
                            SaiWsDiskCache::remove_all_payload_files(&Self::preserved_payload_cache_keys());
                        }
                        RiseStorageSite::Images => {
                            CleverImageDiskCache::clear_all(account_user_id);
                            CleverImageRenderMemoryCache::clear_all();
                            MediaViewerImageMemoryCache::clear_all();
                        }
                        RiseStorageSite::RemoteMedia => {
                            RemoteMediaDiskCache::clear_all(account_user_id);
                            worker_runtime.spawn(async move {
                                RiseMediaTransferHub::shared().reload_presence().await;
                            });
                        }
                        RiseStorageSite::Voice => VoiceMessageDiskCache::clear_all(account_user_id),
                        RiseStorageSite::Music => MusicPlaybackDiskCache::clear_all(account_user_id),
                        RiseStorageSite::Stickers => RiseLottieFrameCache::shared().release_all(),
                        RiseStorageSite::StickerPosters => RiseLottieFirstFrameCache::clear_disk_cache(),
                        RiseStorageSite::ReleaseVideo => RiseReleaseMediaWarmer::clear_disk_cache(),
                        RiseStorageSite::HttpResponses => URLCache::shared().remove_all_cached_responses(),
                        RiseStorageSite::EngineDatabase | RiseStorageSite::LocalState => continue,
                    }
                }
            });
            let _ = work.await;
        })
    }

    /// payload keys the purge keeps, so the scan leaves them out of what a purge frees
    /// blocking, both stores read the payload index from disk
    pub fn preserved_payload_cache_keys() -> Vec<String> {
        let mut keys = ChatScrollPositionStore::persisted_disk_cache_keys();
        keys.extend(ChatDraftStore::persisted_disk_cache_keys());
        keys
    }

    pub fn clear_everything(account_user_id: Option<&str>, cx: &mut App) -> Task<()> {
        let sites: HashSet<RiseStorageSite> =
            RiseStorageSite::ALL_CASES.into_iter().filter(RiseStorageSite::is_clearable).collect();
        Self::clear(&sites, account_user_id, cx)
    }

    /// clears cached bytes of one chat, its voice notes and payload keys
    /// plus the media its bubbles noted in the ledger
    /// same list the storage screen measured so the row number is what this frees
    pub fn clear_chat(chat_id: &str, account_user_id: Option<&str>, cx: &mut App) -> Task<()> {
        let trimmed = trimming_whitespaces_and_newlines(chat_id).to_owned();
        if trimmed.is_empty() {
            return Task::ready(());
        }

        let cache_keys = MessageServicesStore::chat_disk_cache_keys(&trimmed);
        SaiWsResponseCache::prepare_disk_clear_cache_keys(&cache_keys, cx);

        let account_user_id = account_user_id.map(str::to_owned);
        let work = RiseAccountRuntime::current().spawn_blocking(move || {
            let account_user_id = account_user_id.as_deref();
            for key in &cache_keys {
                SaiWsDiskCache::remove(key);
            }
            VoiceMessageDiskCache::clear_chat(&trimmed);
            let mut noted: HashMap<String, Vec<String>> = RiseChatMediaLedger::snapshot(account_user_id);
            let urls = noted.remove(&trimmed).unwrap_or_default();
            let media = RiseChatMediaFiles::new(account_user_id).resolve(&urls);
            for key in &media.image_keys {
                CleverImageDiskCache::remove_cache_key(key, account_user_id, None);
            }
            RemoteMediaDiskCache::remove(&media.remote_digests, account_user_id);
            RiseChatMediaLedger::forget(&trimmed, account_user_id);
        });
        cx.spawn(async move |_cx| {
            let _ = work.await;
        })
    }

    pub fn clear_media_entry(entry: &RiseStorageMediaEntry, account_user_id: Option<&str>, cx: &mut App) -> Task<()> {
        let account_user_id = account_user_id.map(str::to_owned);
        if let Some(digest) = entry.remote_media_digest() {
            let work = RiseAccountRuntime::current().spawn_blocking(move || {
                RemoteMediaDiskCache::remove(&HashSet::from([digest]), account_user_id.as_deref());
            });
            return cx.spawn(async move |_cx| {
                let _ = work.await;
            });
        }
        let (RiseStorageSite::Images, Some(source)) = (entry.site, entry.preview_source.clone()) else {
            return Task::ready(());
        };
        let work = RiseAccountRuntime::current().spawn_blocking(move || {
            let account_user_id = account_user_id.as_deref();
            CleverImageDiskCache::remove_cache_key(&source, account_user_id, None);
            CleverImageDiskCache::remove_cache_key(
                &AnimatedImageSourcePolicy::payload_cache_key(&source),
                account_user_id,
                None,
            );
        });
        cx.spawn(async move |_cx| {
            let _ = work.await;
        })
    }
}
