use std::path::PathBuf;

use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_upload::rise_url_session::URLRequest;

use super::{CachedURLResponse, URLCache};
use crate::core::media::remote_media_disk_cache::RemoteMediaHttpResponse;

fn scratch_directory() -> PathBuf {
    std::env::temp_dir().join(format!("riseonly-url-cache-{}", Uuid::new_v4().uuid_string()))
}

fn request(address: &str) -> URLRequest {
    URLRequest::new(Url::from_string(address).expect("test url"))
}

fn cached(address: &str, body: Vec<u8>) -> CachedURLResponse {
    let url = Url::from_string(address).expect("test url");
    CachedURLResponse::new(
        RemoteMediaHttpResponse::new(url, 200, vec![("Content-Type".to_owned(), "image/png".to_owned())]),
        body,
    )
}

#[test]
fn stored_response_comes_back_from_memory() {
    let cache = URLCache::new(2_000, 0, None);
    let address = "https://cdn.example.test/a.png";
    cache.store_cached_response(cached(address, vec![1; 50]), &request(address));
    let hit = cache.cached_response(&request(address)).expect("memory hit");
    assert_eq!(hit.data, vec![1; 50]);
    assert_eq!(hit.response.value("content-type"), Some("image/png"));
    assert_eq!(cache.current_memory_usage(), 50);
}

#[test]
fn response_over_five_percent_of_memory_is_not_kept() {
    let cache = URLCache::new(1_000, 0, None);
    let address = "https://cdn.example.test/big.png";
    cache.store_cached_response(cached(address, vec![1; 51]), &request(address));
    assert!(cache.cached_response(&request(address)).is_none());
    assert_eq!(cache.current_memory_usage(), 0);
}

#[test]
fn memory_evicts_the_least_recently_used() {
    let cache = URLCache::new(2_000, 0, None);
    for index in 0..21 {
        let address = format!("https://cdn.example.test/{index}.png");
        cache.store_cached_response(cached(&address, vec![0; 100]), &request(&address));
    }
    assert!(cache.cached_response(&request("https://cdn.example.test/0.png")).is_none());
    assert!(cache.cached_response(&request("https://cdn.example.test/20.png")).is_some());
    assert!(cache.current_memory_usage() <= 2_000);
}

#[test]
fn disk_survives_a_new_instance_and_remove_all_clears_it() {
    let directory = scratch_directory();
    let address = "https://cdn.example.test/disk.png";
    {
        let cache = URLCache::new(0, 10_000, Some(directory.clone()));
        cache.store_cached_response(cached(address, vec![7; 300]), &request(address));
        assert_eq!(cache.current_disk_usage(), 300);
    }
    let cache = URLCache::new(0, 10_000, Some(directory.clone()));
    let hit = cache.cached_response(&request(address)).expect("disk hit");
    assert_eq!(hit.data, vec![7; 300]);
    assert_eq!(hit.response.status_code, 200);
    assert_eq!(hit.response.url.absolute_string(), address);
    cache.remove_all_cached_responses();
    assert!(cache.cached_response(&request(address)).is_none());
    assert_eq!(cache.current_disk_usage(), 0);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn disk_trims_to_capacity() {
    let directory = scratch_directory();
    let cache = URLCache::new(0, 4_000, Some(directory.clone()));
    for index in 0..30 {
        let address = format!("https://cdn.example.test/{index}.bin");
        cache.store_cached_response(cached(&address, vec![0; 200]), &request(&address));
    }
    assert!(cache.current_disk_usage() <= 4_000);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn remove_cached_response_drops_one_entry() {
    let directory = scratch_directory();
    let cache = URLCache::new(2_000, 10_000, Some(directory.clone()));
    let kept = "https://cdn.example.test/kept.png";
    let dropped = "https://cdn.example.test/dropped.png";
    cache.store_cached_response(cached(kept, vec![1; 10]), &request(kept));
    cache.store_cached_response(cached(dropped, vec![2; 10]), &request(dropped));
    cache.remove_cached_response(&request(dropped));
    assert!(cache.cached_response(&request(dropped)).is_none());
    assert!(cache.cached_response(&request(kept)).is_some());
    let _ = std::fs::remove_dir_all(&directory);
}
