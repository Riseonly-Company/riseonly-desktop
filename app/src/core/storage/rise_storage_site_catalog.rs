use std::path::PathBuf;

use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::media::remote_media_disk_cache::RemoteMediaDiskCache;
use crate::core::media::url_cache::URLCache;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::storage::rise_storage_usage_models::{RiseStorageBucket, RiseStorageSite};
use crate::core::ui::clever_image_disk_cache::CleverImageDiskCache;
use crate::core::ui::lottie_sticker::lottie_sticker_loader::LottieStickerLoader;
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::RiseLottieFirstFrameCache;
use crate::modules::analytics::stores::engagement::engagement_services::engagement_services::EngagementServicesStore;
use crate::modules::chat::shared::utils::voice::voice_message_disk_cache::VoiceMessageDiskCache;
use crate::modules::media_picker::editor::media_reedit_store::MediaReeditStore;
use crate::modules::media_picker::shared::media_picker_recent_files::MediaPickerRecentFiles;
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;
use crate::modules::notify::engine::core::rise_release_media_warmer::RiseReleaseMediaWarmer;

/// maps every reportable site to the directories it uses
/// each owner hands over its own root, a site without a root reports zero
/// skips offline queue and temp media and drafts and keyring on purpose
pub enum RiseStorageSiteCatalog {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseStorageSiteCatalogResolved {
    pub site: RiseStorageSite,
    pub directories: Vec<PathBuf>,
    pub fixed_bucket: Option<RiseStorageBucket>,
}

impl RiseStorageSiteCatalog {
    /// blocking path resolution, call it off the main thread with the walk
    pub fn resolve(site: RiseStorageSite, account_user_id: Option<&str>) -> RiseStorageSiteCatalogResolved {
        match site {
            RiseStorageSite::Images => RiseStorageSiteCatalogResolved {
                site,
                directories: CleverImageDiskCache::storage_directory_url(account_user_id).into_iter().collect(),
                fixed_bucket: Some(RiseStorageBucket::Images),
            },

            RiseStorageSite::RemoteMedia => RiseStorageSiteCatalogResolved {
                site,
                directories: RemoteMediaDiskCache::storage_directory_url(account_user_id).into_iter().collect(),
                fixed_bucket: None,
            },

            RiseStorageSite::Voice => RiseStorageSiteCatalogResolved {
                site,
                directories: VoiceMessageDiskCache::storage_directory_url(account_user_id).into_iter().collect(),
                fixed_bucket: Some(RiseStorageBucket::Audio),
            },

            RiseStorageSite::Music => RiseStorageSiteCatalogResolved {
                site,
                directories: MusicPlaybackDiskCache::storage_directory_url(account_user_id).into_iter().collect(),
                fixed_bucket: Some(RiseStorageBucket::Audio),
            },

            RiseStorageSite::Stickers => RiseStorageSiteCatalogResolved {
                site,
                directories: LottieStickerLoader::storage_directory_url().into_iter().collect(),
                fixed_bucket: Some(RiseStorageBucket::Other),
            },

            RiseStorageSite::StickerPosters => RiseStorageSiteCatalogResolved {
                site,
                directories: RiseLottieFirstFrameCache::storage_root_url().into_iter().collect(),
                fixed_bucket: Some(RiseStorageBucket::Other),
            },

            RiseStorageSite::ReleaseVideo => RiseStorageSiteCatalogResolved {
                site,
                directories: RiseReleaseMediaWarmer::storage_directory_url().into_iter().collect(),
                fixed_bucket: Some(RiseStorageBucket::Videos),
            },

            RiseStorageSite::WsPayloads => RiseStorageSiteCatalogResolved {
                site,
                directories: SaiWsDiskCache::storage_directory_url().into_iter().collect(),
                fixed_bucket: Some(RiseStorageBucket::Other),
            },

            RiseStorageSite::HttpResponses => RiseStorageSiteCatalogResolved {
                site,
                directories: Self::http_response_bodies_directory().into_iter().collect(),
                fixed_bucket: Some(RiseStorageBucket::Other),
            },

            RiseStorageSite::EngineDatabase => {
                let directory = account_user_id.and_then(RiseEngineAppRuntime::persisted_account_directory_url);
                RiseStorageSiteCatalogResolved {
                    site,
                    directories: directory.into_iter().collect(),
                    fixed_bucket: None,
                }
            }

            RiseStorageSite::LocalState => RiseStorageSiteCatalogResolved {
                site,
                directories: [
                    MediaPickerRecentFiles::storage_directory_url(account_user_id),
                    MediaReeditStore::storage_directory_url(account_user_id),
                    EngagementServicesStore::storage_directory_url(account_user_id),
                ]
                .into_iter()
                .flatten()
                .collect(),
                fixed_bucket: None,
            },
        }
    }

    fn http_response_bodies_directory() -> Option<PathBuf> {
        URLCache::shared().response_bodies_directory()
    }
}
