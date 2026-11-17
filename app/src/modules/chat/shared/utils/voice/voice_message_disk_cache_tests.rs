use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode};
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDownloadRetryPolicy, RemoteMediaHttpResponse, RemoteMediaTransferError,
};
use crate::core::ui::clever_image_disk_cache::CleverImageDiskCache;
use crate::core::ui::lottie_sticker::lottie_sticker_loader::LottieStickerLoader;
use crate::modules::chat::shared::utils::voice::voice_message_disk_cache::VoiceMessageDiskCache;

static SERIAL: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// test guard that drops the account folder and the active account for voice tests
struct DiskCacheSafetyTestsVoiceAccount {
    user_id: String,
}

impl Drop for DiskCacheSafetyTestsVoiceAccount {
    fn drop(&mut self) {
        VoiceMessageDiskCache::remove_persisted_account(&self.user_id);
        SaiAccountCacheScope::clear_active_account();
    }
}

struct DiskCacheSafetyTestsImageAccount {
    user_id: String,
}

impl Drop for DiskCacheSafetyTestsImageAccount {
    fn drop(&mut self) {
        CleverImageDiskCache::remove_persisted_account(&self.user_id);
    }
}

struct DiskCacheSafetyTestsDirectory {
    path: PathBuf,
}

impl Drop for DiskCacheSafetyTestsDirectory {
    fn drop(&mut self) {
        FileManager.remove_item(&self.path).ok();
    }
}

fn response(status_code: i64, header_fields: Vec<(String, String)>) -> RemoteMediaHttpResponse {
    let url = Url::from_string("https://example.invalid/voice.m4a").expect("a literal URL parses");
    RemoteMediaHttpResponse::new(url, status_code, header_fields)
}

fn content_range(value: &str) -> Vec<(String, String)> {
    vec![("Content-Range".to_owned(), value.to_owned())]
}

#[test]
fn voice_paths_hash_untrusted_identifiers() {
    let _serial = SERIAL.lock();
    let user_id = format!("voice-path-{}", Uuid::new_v4().uuid_string());
    SaiAccountCacheScope::activate(&user_id);
    let _account = DiskCacheSafetyTestsVoiceAccount {
        user_id: user_id.clone(),
    };

    let base = PathBuf::from(VoiceMessageDiskCache::folder_path_for_inspector());
    let file = VoiceMessageDiskCache::file_url("..", "../../outside").expect("a cache path for any identifier");

    assert!(file.starts_with(&base) && file != base);
    let chat_directory = file
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    assert!(chat_directory.starts_with("chat-"));
    let file_name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    assert!(file_name.starts_with("voice-"));
    assert!(!file.to_string_lossy().contains("../"));
}

#[test]
fn voice_legacy_files_migrate_to_hashed_paths() {
    let _serial = SERIAL.lock();
    let user_id = format!("voice-migration-{}", Uuid::new_v4().uuid_string());
    SaiAccountCacheScope::activate(&user_id);
    let _account = DiskCacheSafetyTestsVoiceAccount {
        user_id: user_id.clone(),
    };

    let base = PathBuf::from(VoiceMessageDiskCache::folder_path_for_inspector());
    let legacy_directory = base.join("legacy-chat");
    FileManager
        .create_directory(&legacy_directory, true)
        .expect("the legacy chat folder is created");
    let legacy = legacy_directory.join("legacy-message.m4a");
    let mut payload = vec![0_u8; 2_048];
    payload[0..3].copy_from_slice(&[0x49, 0x44, 0x33]);
    std::fs::write(&legacy, &payload).expect("the legacy note is written");

    let migrated = VoiceMessageDiskCache::cached_url("legacy-chat", "legacy-message")
        .expect("the legacy note is found");
    assert_ne!(migrated, legacy);
    let migrated_name = migrated
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    assert!(migrated_name.starts_with("voice-"));
    assert!(FileManager.file_exists(&migrated));
    assert!(!FileManager.file_exists(&legacy));
}

#[test]
fn voice_download_requires_full_http_response() {
    let full = response(200, Vec::new());
    let partial = response(206, content_range("bytes 0-2047/4096"));
    let complete_partial_status = response(206, content_range("bytes 0-4095/4096"));

    assert!(VoiceMessageDiskCache::is_complete_download_response(&full));
    assert!(!VoiceMessageDiskCache::is_complete_download_response(&partial));
    assert!(!VoiceMessageDiskCache::is_complete_download_response_with_byte_count(
        &partial, 2_048
    ));
    assert!(VoiceMessageDiskCache::is_complete_download_response_with_byte_count(
        &complete_partial_status,
        4_096
    ));
    assert!(VoiceMessageDiskCache::expected_download_byte_count_matches(
        82_061,
        Some(82_061)
    ));
    assert!(!VoiceMessageDiskCache::expected_download_byte_count_matches(
        40_000,
        Some(82_061)
    ));
}

#[test]
fn voice_download_retries_transient_cdn_responses_only() {
    for status_code in [-1, 206, 404, 408, 425, 429, 500, 503, 599] {
        assert!(VoiceMessageDiskCache::should_retry_download(status_code), "{status_code}");
    }
    for status_code in [200, 400, 401, 403, 410, 422] {
        assert!(!VoiceMessageDiskCache::should_retry_download(status_code), "{status_code}");
    }
}

#[test]
fn stale_image_cache_write_cannot_survive_account_removal() {
    let user_id = format!("image-invalidation-{}", Uuid::new_v4().uuid_string());
    let cache_key = "https://example.invalid/avatar.jpg";
    let token = CleverImageDiskCache::invalidation_token(Some(user_id.as_str()));
    CleverImageDiskCache::remove_persisted_account(&user_id);
    let _account = DiskCacheSafetyTestsImageAccount {
        user_id: user_id.clone(),
    };

    CleverImageDiskCache::store_data(&[1, 2, 3], cache_key, Some(user_id.as_str()), Some(token));

    assert!(CleverImageDiskCache::image_data(cache_key, Some(user_id.as_str()), None).is_none());
}

#[test]
fn broken_image_eviction_preserves_a_concurrent_repair() {
    let user_id = format!("image-repair-{}", Uuid::new_v4().uuid_string());
    let key = "https://example.invalid/attachment.gif";
    let _account = DiskCacheSafetyTestsImageAccount {
        user_id: user_id.clone(),
    };
    let broken: [u8; 3] = [1, 2, 3];
    let repaired: [u8; 3] = [4, 5, 6];
    CleverImageDiskCache::store_data(&broken, key, Some(user_id.as_str()), None);
    CleverImageDiskCache::store_data(&repaired, key, Some(user_id.as_str()), None);

    CleverImageDiskCache::remove_cache_key(key, Some(user_id.as_str()), Some(&broken[..]));

    assert_eq!(
        CleverImageDiskCache::image_data(key, Some(user_id.as_str()), None),
        Some(repaired.to_vec())
    );
    CleverImageDiskCache::remove_cache_key(key, Some(user_id.as_str()), Some(&repaired[..]));
    assert!(CleverImageDiskCache::image_data(key, Some(user_id.as_str()), None).is_none());
}

#[test]
fn media_read_recovers_from_one_transient_failure() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let value = RiseAccountRuntime::current()
        .handle()
        .block_on(RemoteMediaDownloadRetryPolicy::retrying(move || {
            let counter = Arc::clone(&counter);
            async move {
                let attempt = counter.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt == 1 {
                    return Err(RemoteMediaTransferError::from(URLError::new(
                        URLErrorCode::NetworkConnectionLost,
                    )));
                }
                Ok(42)
            }
        }))
        .expect("the second attempt succeeds");
    assert_eq!(value, 42);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[test]
fn media_read_stops_after_its_retry_budget() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let result: Result<i64, RemoteMediaTransferError> = RiseAccountRuntime::current()
        .handle()
        .block_on(RemoteMediaDownloadRetryPolicy::retrying(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err(RemoteMediaTransferError::from(URLError::new(URLErrorCode::TimedOut)))
            }
        }));
    let Err(error) = result else {
        panic!("Failed media read unexpectedly succeeded");
    };
    assert!(matches!(
        error,
        RemoteMediaTransferError::Url(URLError {
            code: URLErrorCode::TimedOut
        })
    ));
    assert_eq!(
        i64::try_from(attempts.load(Ordering::SeqCst)).unwrap_or(i64::MAX),
        RemoteMediaDownloadRetryPolicy::maximum_retries() + 1
    );
}

#[test]
fn media_read_does_not_retry_cancellation() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let result: Result<i64, RemoteMediaTransferError> = RiseAccountRuntime::current()
        .handle()
        .block_on(RemoteMediaDownloadRetryPolicy::retrying(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err(RemoteMediaTransferError::from(CancellationError))
            }
        }));
    let Err(error) = result else {
        panic!("Cancelled media read unexpectedly succeeded");
    };
    assert!(matches!(error, RemoteMediaTransferError::Cancelled(_)));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[test]
fn lottie_cleanup_removes_hidden_incoming_files() {
    let directory = temporary_directory().join(Uuid::new_v4().uuid_string());
    FileManager
        .create_directory(&directory, true)
        .expect("the scratch folder is created");
    let _directory = DiskCacheSafetyTestsDirectory {
        path: directory.clone(),
    };
    let incoming = directory.join(".incoming-abandoned");
    let valid = directory.join("kept.json");
    std::fs::write(&incoming, [1_u8]).expect("the incoming file is written");
    std::fs::write(&valid, [2_u8]).expect("the kept file is written");

    LottieStickerLoader::remove_stale_incoming_files(&directory);

    assert!(!FileManager.file_exists(&incoming));
    assert!(FileManager.file_exists(&valid));
}
