use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Once};
use std::time::Duration;

use futures::FutureExt;
use futures::future::{Aborted, BoxFuture, LocalBoxFuture};
use gpui::{App, TestAppContext};
use parking_lot::Mutex;
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::URLRequest;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_configuration::SaiFetchHttpConfiguration;
use crate::core::lib::sai::sai_auth_hooks::{SaiAuthHooks, set_sai_auth_hooks};
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheConfiguration, RemoteMediaDiskCacheDownloadResult,
    RemoteMediaDiskCacheDownloader, RemoteMediaDiskCacheError, RemoteMediaDiskCacheLimits,
    RemoteMediaDiskCacheMaterializeOptions, RemoteMediaDiskCacheProgressHandler,
    RemoteMediaDiskCacheProgressiveDownloader, RemoteMediaDiskCacheStatistics,
    RemoteMediaDownloadProgress, RemoteMediaDownloadProgressCoalescer,
    RemoteMediaDownloadProgressSink, RemoteMediaHttpResponse,
};
use crate::modules::auth::services::account_state_reset_coordinator::{
    AccountStateResetCoordinator, AccountStateResetReason,
};

static SERIAL: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

#[derive(Clone)]
struct RemoteMediaDownloadProbeReply {
    data: Vec<u8>,
    status_code: i64,
    content_type: String,
    delay: Duration,
}

impl RemoteMediaDownloadProbeReply {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            status_code: 200,
            content_type: "application/octet-stream".to_owned(),
            delay: Duration::ZERO,
        }
    }

    fn status_code(mut self, status_code: i64) -> Self {
        self.status_code = status_code;
        self
    }

    fn content_type(mut self, content_type: &str) -> Self {
        self.content_type = content_type.to_owned();
        self
    }

    fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

#[derive(Default)]
struct RemoteMediaDownloadProbeState {
    request_count: usize,
    temporary_urls: Vec<PathBuf>,
}

struct RemoteMediaDownloadProbe {
    replies: HashMap<String, RemoteMediaDownloadProbeReply>,
    fallback: RemoteMediaDownloadProbeReply,
    state: Mutex<RemoteMediaDownloadProbeState>,
}

impl RemoteMediaDownloadProbe {
    fn new(
        replies: HashMap<String, RemoteMediaDownloadProbeReply>,
        fallback: RemoteMediaDownloadProbeReply,
    ) -> Arc<Self> {
        Arc::new(Self {
            replies,
            fallback,
            state: Mutex::new(RemoteMediaDownloadProbeState::default()),
        })
    }

    async fn download(&self, request: URLRequest) -> RemoteMediaDiskCacheDownloadResult {
        self.state.lock().request_count += 1;
        let reply = self
            .replies
            .get(&request.url.path())
            .unwrap_or(&self.fallback)
            .clone();
        if !reply.delay.is_zero() {
            tokio::time::sleep(reply.delay).await;
        }
        let temporary_url = std::env::temp_dir().join(format!(
            "remote-media-test-{}",
            Uuid::new_v4().uuid_string().to_lowercase()
        ));
        std::fs::write(&temporary_url, &reply.data)?;
        self.state.lock().temporary_urls.push(temporary_url.clone());
        let response = RemoteMediaHttpResponse::new(
            request.url.clone(),
            reply.status_code,
            vec![
                (CONTENT_TYPE.as_str().to_owned(), reply.content_type.clone()),
                (CONTENT_LENGTH.as_str().to_owned(), reply.data.len().to_string()),
            ],
        );
        Ok((temporary_url, response))
    }

    fn count(&self) -> usize {
        self.state.lock().request_count
    }

    fn cleanup(&self) {
        let urls = std::mem::take(&mut self.state.lock().temporary_urls);
        for url in urls {
            let _ = std::fs::remove_file(url);
        }
    }

    fn downloader(self: &Arc<Self>) -> RemoteMediaDiskCacheDownloader {
        let probe = Arc::clone(self);
        Arc::new(
            move |request: URLRequest| -> BoxFuture<'static, RemoteMediaDiskCacheDownloadResult> {
                let probe = Arc::clone(&probe);
                async move { probe.download(request).await }.boxed()
            },
        )
    }
}

#[derive(Clone, Default)]
struct RemoteMediaProgressRecorder {
    values: Arc<Mutex<Vec<RemoteMediaDownloadProgress>>>,
}

impl RemoteMediaProgressRecorder {
    fn count(&self) -> usize {
        self.values.lock().len()
    }

    fn last(&self) -> Option<RemoteMediaDownloadProgress> {
        self.values.lock().last().cloned()
    }

    fn handler(&self) -> RemoteMediaDiskCacheProgressHandler {
        let values = Arc::clone(&self.values);
        Arc::new(move |progress: RemoteMediaDownloadProgress| values.lock().push(progress))
    }
}

/// removes the cache root and signs the account scope out, for tests
struct RootCleanup(PathBuf);

impl Drop for RootCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        SaiAccountCacheScope::clear_active_account();
    }
}

fn temporary_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "remote-media-cache-tests-{}",
        Uuid::new_v4().uuid_string().to_lowercase()
    ))
}

async fn sleep(duration: Duration) {
    let _ = RiseAccountRuntime::current()
        .spawn(async move { tokio::time::sleep(duration).await })
        .await;
}

async fn eventually<F>(condition: F) -> bool
where
    F: Fn() -> bool,
{
    let timeout = Duration::from_secs(1);
    let start = RiseClock::monotonic();
    while RiseClock::monotonic().saturating_sub(start) < timeout {
        if condition() {
            return true;
        }
        sleep(Duration::from_millis(5)).await;
    }
    condition()
}

fn url(string: &str) -> Url {
    Url::from_string(string).expect("test url")
}

fn keyed(cache_key: &str) -> RemoteMediaDiskCacheMaterializeOptions {
    RemoteMediaDiskCacheMaterializeOptions {
        cache_key: Some(cache_key.to_owned()),
        ..Default::default()
    }
}

fn cache_with_downloader(
    root: &Path,
    limits: RemoteMediaDiskCacheLimits,
    probe: &Arc<RemoteMediaDownloadProbe>,
) -> Arc<RemoteMediaDiskCache> {
    Arc::new(RemoteMediaDiskCache::new(RemoteMediaDiskCacheConfiguration {
        root_directory: Some(root.to_path_buf()),
        limits,
        downloader: Some(probe.downloader()),
        ..Default::default()
    }))
}

/// existing local path for a key using the active account and epoch
fn existing(cache: &RemoteMediaDiskCache, remote: &Url, cache_key: &str) -> Option<PathBuf> {
    cache.existing_local_url(
        remote,
        Some(cache_key),
        SaiAccountCacheScope::active_user_id().as_deref(),
        SaiAccountCacheScope::active_epoch().as_deref(),
    )
}

fn statistics(cache: &RemoteMediaDiskCache) -> RemoteMediaDiskCacheStatistics {
    cache.statistics(
        SaiAccountCacheScope::active_user_id().as_deref(),
        SaiAccountCacheScope::active_epoch().as_deref(),
    )
}

fn cache_error(error: &anyhow::Error) -> Option<&RemoteMediaDiskCacheError> {
    error.downcast_ref::<RemoteMediaDiskCacheError>()
}

#[test]
fn progress_coalescer_bounds_update_frequency_and_publishes_completion() {
    let mut coalescer = RemoteMediaDownloadProgressCoalescer::default();

    let initial = coalescer.should_publish(RemoteMediaDownloadProgress::new(1, None), 10.0);
    assert!(initial);
    let below_interval =
        coalescer.should_publish(RemoteMediaDownloadProgress::new(8_193, Some(1_000_000)), 10.05);
    assert!(!below_interval);
    let interval_elapsed =
        coalescer.should_publish(RemoteMediaDownloadProgress::new(8_193, Some(1_000_000)), 10.125);
    assert!(interval_elapsed);
    let small_increment =
        coalescer.should_publish(RemoteMediaDownloadProgress::new(8_194, Some(1_000_000)), 10.25);
    assert!(!small_increment);
    let silence_elapsed =
        coalescer.should_publish(RemoteMediaDownloadProgress::new(8_194, Some(1_000_000)), 10.375);
    assert!(silence_elapsed);
    let completion = coalescer.should_publish(
        RemoteMediaDownloadProgress::new(1_000_000, Some(1_000_000)),
        10.625,
    );
    assert!(completion);
}

fn progressive_probe_downloader(
    probe: &Arc<RemoteMediaDownloadProbe>,
    steps: &'static [(i64, u64)],
) -> RemoteMediaDiskCacheProgressiveDownloader {
    let probe = Arc::clone(probe);
    Arc::new(
        move |request: URLRequest,
              _maximum_byte_count: i64,
              on_progress: RemoteMediaDownloadProgressSink|
              -> BoxFuture<'static, RemoteMediaDiskCacheDownloadResult> {
            let probe = Arc::clone(&probe);
            async move {
                for (downloaded, sleep_ms) in steps {
                    on_progress
                        .call_as_function(RemoteMediaDownloadProgress::new(*downloaded, Some(8)));
                    if *sleep_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(*sleep_ms)).await;
                    }
                }
                probe.download(request).await
            }
            .boxed()
        },
    )
}

#[gpui::test]
async fn deduplicated_download_publishes_progress_to_every_caller(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let user_id = "remote-cache-progress-user";
    let root = temporary_root();
    let bytes = vec![0x25, 0x50, 0x44, 0x46, 0x2D, 0x31, 0x2E, 0x37];
    let probe = RemoteMediaDownloadProbe::new(
        HashMap::new(),
        RemoteMediaDownloadProbeReply::new(bytes).content_type("application/pdf"),
    );
    let first_progress = RemoteMediaProgressRecorder::default();
    let second_progress = RemoteMediaProgressRecorder::default();
    let cache = Arc::new(RemoteMediaDiskCache::new(RemoteMediaDiskCacheConfiguration {
        root_directory: Some(root.clone()),
        progressive_downloader: Some(progressive_probe_downloader(
            &probe,
            &[(2, 120), (6, 120), (8, 50)],
        )),
        ..Default::default()
    }));
    let _cleanup = RootCleanup(root);
    SaiAccountCacheScope::activate(user_id);
    let remote = url("https://media.example.test/document.pdf");

    let first = cache.materialize(
        &remote,
        RemoteMediaDiskCacheMaterializeOptions {
            on_progress: Some(first_progress.handler()),
            ..keyed("document-progress")
        },
        &async_cx,
    );
    let second = cache.materialize(
        &remote,
        RemoteMediaDiskCacheMaterializeOptions {
            on_progress: Some(second_progress.handler()),
            ..keyed("document-progress")
        },
        &async_cx,
    );
    let (first_url, second_url) = futures::try_join!(first, second).expect("materialize");
    let delivered = eventually(|| {
        first_progress.last().map(|p| p.downloaded_bytes) == Some(8)
            && second_progress.last().map(|p| p.downloaded_bytes) == Some(8)
    })
    .await;

    assert_eq!(first_url, second_url);
    assert!(delivered);
    assert_eq!(probe.count(), 1);
    probe.cleanup();
}

#[gpui::test]
async fn cancelling_caller_removes_progress_without_cancelling_cache_work(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let user_id = "remote-cache-progress-cancellation-user";
    let root = temporary_root();
    let bytes = vec![0x25, 0x50, 0x44, 0x46, 0x2D, 0x31, 0x2E, 0x37];
    let probe = RemoteMediaDownloadProbe::new(
        HashMap::new(),
        RemoteMediaDownloadProbeReply::new(bytes).content_type("application/pdf"),
    );
    let progress = RemoteMediaProgressRecorder::default();
    let cache = Arc::new(RemoteMediaDiskCache::new(RemoteMediaDiskCacheConfiguration {
        root_directory: Some(root.clone()),
        progressive_downloader: Some(progressive_probe_downloader(
            &probe,
            &[(1, 200), (4, 100), (8, 0)],
        )),
        ..Default::default()
    }));
    let _cleanup = RootCleanup(root);
    SaiAccountCacheScope::activate(user_id);
    let remote = url("https://media.example.test/cancelled-document.pdf");

    let (caller, cancellation) = futures::future::abortable(cache.materialize(
        &remote,
        RemoteMediaDiskCacheMaterializeOptions {
            on_progress: Some(progress.handler()),
            ..keyed("cancelled-document-progress")
        },
        &async_cx,
    ));
    let driver = async {
        sleep(Duration::from_millis(80)).await;
        let delivered_before_cancellation = progress.count() > 0;
        let count_at_cancellation = progress.count();
        let cancellation_started = RiseClock::monotonic();
        cancellation.abort();
        (delivered_before_cancellation, count_at_cancellation, cancellation_started)
    };
    let (caller_result, (delivered_before_cancellation, count_at_cancellation, cancellation_started)) =
        futures::join!(caller, driver);
    let cancellation_observed = matches!(caller_result, Err(Aborted));
    let cancellation_latency = RiseClock::monotonic().saturating_sub(cancellation_started);

    assert!(delivered_before_cancellation);
    assert!(cancellation_observed);
    assert!(cancellation_latency < Duration::from_millis(150));
    sleep(Duration::from_millis(300)).await;
    assert_eq!(progress.count(), count_at_cancellation);
    assert!(existing(&cache, &remote, "cancelled-document-progress").is_some());
    assert_eq!(probe.count(), 1);
    probe.cleanup();
}

#[gpui::test]
async fn deduplicates_downloads_and_keeps_explicit_keys_stable(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let user_id = "remote-cache-dedupe-user";
    let root = temporary_root();
    let bytes = vec![0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70];
    let probe = RemoteMediaDownloadProbe::new(
        HashMap::new(),
        RemoteMediaDownloadProbeReply::new(bytes.clone())
            .content_type("video/mp4")
            .delay(Duration::from_millis(80)),
    );
    let cache = cache_with_downloader(&root, RemoteMediaDiskCacheLimits::default(), &probe);
    let _cleanup = RootCleanup(root);
    SaiAccountCacheScope::activate(user_id);

    let first_remote = url("https://media.example.test/video.mp4?signature=one");
    let first = cache.materialize(&first_remote, keyed("chat:42:message:91:video"), &async_cx);
    let second = cache.materialize(&first_remote, keyed("chat:42:message:91:video"), &async_cx);
    let (first_url, second_url) = futures::try_join!(first, second).expect("materialize");

    assert_eq!(first_url, second_url);
    assert!(first_url.is_absolute());
    assert_eq!(std::fs::read(&first_url).expect("cached file"), bytes);
    assert_eq!(probe.count(), 1);

    let refreshed_signature = url("https://media.example.test/video.mp4?signature=two");
    let reused = cache
        .materialize(&refreshed_signature, keyed("chat:42:message:91:video"), &async_cx)
        .await
        .expect("materialize");
    assert_eq!(reused, first_url);
    assert_eq!(probe.count(), 1);
    assert_eq!(
        existing(&cache, &first_remote, "chat:42:message:91:video"),
        Some(first_url)
    );
    probe.cleanup();
}

#[gpui::test]
async fn refetches_when_an_explicit_key_points_to_a_different_source_path(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let user_id = "remote-cache-source-change-user";
    let root = temporary_root();
    let first_bytes = vec![1, 2, 3, 4];
    let second_bytes = vec![5, 6, 7, 8];
    let probe = RemoteMediaDownloadProbe::new(
        HashMap::from([
            (
                "/first.mp4".to_owned(),
                RemoteMediaDownloadProbeReply::new(first_bytes.clone()).content_type("video/mp4"),
            ),
            (
                "/second.mp4".to_owned(),
                RemoteMediaDownloadProbeReply::new(second_bytes.clone()).content_type("video/mp4"),
            ),
        ]),
        RemoteMediaDownloadProbeReply::new(vec![9]),
    );
    let cache = cache_with_downloader(&root, RemoteMediaDiskCacheLimits::default(), &probe);
    let _cleanup = RootCleanup(root);
    SaiAccountCacheScope::activate(user_id);

    let first_remote = url("https://media.example.test/first.mp4?signature=one");
    let second_remote = url("https://media.example.test/second.mp4?signature=two");
    let first_url = cache
        .materialize(&first_remote, keyed("stable-media-key"), &async_cx)
        .await
        .expect("materialize");
    assert_eq!(std::fs::read(&first_url).expect("cached file"), first_bytes);

    let second_url = cache
        .materialize(&second_remote, keyed("stable-media-key"), &async_cx)
        .await
        .expect("materialize");
    assert_eq!(second_url, first_url);
    assert_eq!(std::fs::read(&second_url).expect("cached file"), second_bytes);
    assert_eq!(probe.count(), 2);
    probe.cleanup();
}

#[gpui::test]
async fn rejects_stale_account_downloads_before_they_reach_disk(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let first_user = "remote-cache-stale-a";
    let second_user = "remote-cache-stale-b";
    let root = temporary_root();
    let probe = RemoteMediaDownloadProbe::new(
        HashMap::new(),
        RemoteMediaDownloadProbeReply::new(vec![0x01, 0x02, 0x03, 0x04])
            .content_type("video/mp4")
            .delay(Duration::from_millis(150)),
    );
    let cache = cache_with_downloader(&root, RemoteMediaDiskCacheLimits::default(), &probe);
    let _cleanup = RootCleanup(root);
    SaiAccountCacheScope::activate(first_user);
    let first_epoch = SaiAccountCacheScope::active_epoch().expect("active epoch");
    let remote = url("https://media.example.test/stale.mp4");

    let work = cache.ensure_cached(
        &remote,
        RemoteMediaDiskCacheMaterializeOptions {
            cache_key: Some("stale-media".to_owned()),
            expected_account_user_id: Some(first_user.to_owned()),
            expected_account_epoch: Some(first_epoch),
            ..Default::default()
        },
        &async_cx,
    );
    let driver = async {
        let started = eventually(|| probe.count() == 1).await;
        SaiAccountCacheScope::activate(second_user);
        started
    };
    let (cached, started) = futures::join!(work, driver);

    assert!(started);
    assert_eq!(cached, None);
    SaiAccountCacheScope::activate(first_user);
    assert_eq!(existing(&cache, &remote, "stale-media"), None);
    probe.cleanup();
}

#[gpui::test]
async fn post_reset_request_does_not_join_stale_in_flight_work(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let user_id = "remote-cache-reset-in-flight-user";
    let root = temporary_root();
    let bytes = vec![0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70];
    let probe = RemoteMediaDownloadProbe::new(
        HashMap::new(),
        RemoteMediaDownloadProbeReply::new(bytes.clone())
            .content_type("video/mp4")
            .delay(Duration::from_millis(150)),
    );
    let cache = cache_with_downloader(&root, RemoteMediaDiskCacheLimits::default(), &probe);
    let _cleanup = RootCleanup(root);
    SaiAccountCacheScope::activate(user_id);
    let remote = url("https://media.example.test/reset.mp4");

    let stale = cache.materialize(&remote, keyed("reset-media"), &async_cx);
    let fresh_flow = async {
        let stale_started = eventually(|| probe.count() == 1).await;
        RemoteMediaDiskCache::reset_account_state();
        let fresh = cache.materialize(&remote, keyed("reset-media"), &async_cx);
        let fresh_watch = eventually(|| probe.count() == 2);
        let (fresh_result, fresh_started) = futures::join!(fresh, fresh_watch);
        (stale_started, fresh_started, fresh_result)
    };
    let (stale_result, (stale_started, fresh_started, fresh_result)) =
        futures::join!(stale, fresh_flow);

    assert!(stale_started);
    assert!(fresh_started);
    let fresh_url = fresh_result.expect("fresh materialize");
    assert_eq!(std::fs::read(&fresh_url).expect("cached file"), bytes);
    let stale_error = match stale_result {
        Ok(_) => None,
        Err(error) => cache_error(&error).cloned(),
    };
    assert_eq!(stale_error, Some(RemoteMediaDiskCacheError::AccountChanged));
    probe.cleanup();
}

#[gpui::test]
async fn enforces_lru_limits_and_refetches_after_corrupted_metadata(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let user_id = "remote-cache-lru-user";
    let root = temporary_root();
    let probe = RemoteMediaDownloadProbe::new(
        HashMap::from([
            ("/one.bin".to_owned(), RemoteMediaDownloadProbeReply::new(vec![1, 1, 1, 1])),
            ("/two.bin".to_owned(), RemoteMediaDownloadProbeReply::new(vec![2, 2, 2, 2])),
            ("/three.bin".to_owned(), RemoteMediaDownloadProbeReply::new(vec![3, 3, 3, 3])),
        ]),
        RemoteMediaDownloadProbeReply::new(vec![9]),
    );
    let cache = cache_with_downloader(&root, RemoteMediaDiskCacheLimits::new(2, 8, 8), &probe);
    let _cleanup = RootCleanup(root);
    SaiAccountCacheScope::activate(user_id);
    let one = url("https://media.example.test/one.bin");
    let two = url("https://media.example.test/two.bin");
    let three = url("https://media.example.test/three.bin");

    let first_url = cache.materialize(&one, keyed("one"), &async_cx).await.expect("one");
    let _ = cache.materialize(&two, keyed("two"), &async_cx).await.expect("two");
    let _ = existing(&cache, &one, "one");
    let _ = cache.materialize(&three, keyed("three"), &async_cx).await.expect("three");

    assert!(existing(&cache, &one, "one").is_some());
    assert_eq!(existing(&cache, &two, "two"), None);
    assert!(existing(&cache, &three, "three").is_some());
    assert_eq!(
        statistics(&cache),
        RemoteMediaDiskCacheStatistics {
            entry_count: 2,
            total_bytes: 8,
        }
    );

    let account = cache
        .folder_url_for_inspector(
            SaiAccountCacheScope::active_user_id().as_deref(),
            SaiAccountCacheScope::active_epoch().as_deref(),
        )
        .expect("account folder");
    std::fs::write(account.join("_media_index.json"), b"{").expect("corrupt index");
    assert_eq!(existing(&cache, &one, "one"), None);
    assert!(!first_url.exists());

    let recovered_url = cache.materialize(&one, keyed("one"), &async_cx).await.expect("recovered");
    assert_eq!(recovered_url, first_url);
    assert_eq!(std::fs::read(&recovered_url).expect("cached file"), vec![1, 1, 1, 1]);
    assert_eq!(probe.count(), 4);

    std::fs::write(&recovered_url, b"").expect("truncate");
    assert_eq!(existing(&cache, &one, "one"), None);
    assert!(!recovered_url.exists());
    assert_eq!(
        statistics(&cache),
        RemoteMediaDiskCacheStatistics {
            entry_count: 1,
            total_bytes: 4,
        }
    );
    probe.cleanup();
}

#[gpui::test]
async fn rejects_error_pages_failures_and_oversized_payloads(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let user_id = "remote-cache-validation-user";
    let root = temporary_root();
    let probe = RemoteMediaDownloadProbe::new(
        HashMap::from([
            (
                "/error.html".to_owned(),
                RemoteMediaDownloadProbeReply::new(b"<!doctype html><html>error</html>".to_vec())
                    .content_type("text/html"),
            ),
            (
                "/error.mp4".to_owned(),
                RemoteMediaDownloadProbeReply::new(b"{\"e\":1}".to_vec())
                    .content_type("application/json"),
            ),
            (
                "/error-octet.mp4".to_owned(),
                RemoteMediaDownloadProbeReply::new(b"{}".to_vec()),
            ),
            (
                "/denied.bin".to_owned(),
                RemoteMediaDownloadProbeReply::new(vec![1]).status_code(401),
            ),
            ("/large.bin".to_owned(), RemoteMediaDownloadProbeReply::new(vec![7; 9])),
        ]),
        RemoteMediaDownloadProbeReply::new(vec![1]),
    );
    let cache = cache_with_downloader(&root, RemoteMediaDiskCacheLimits::new(2, 8, 8), &probe);
    let _cleanup = RootCleanup(root);
    SaiAccountCacheScope::activate(user_id);

    let cases = [
        ("https://media.example.test/error.html", RemoteMediaDiskCacheError::UnsupportedPayload),
        ("https://media.example.test/error.mp4", RemoteMediaDiskCacheError::UnsupportedPayload),
        (
            "https://media.example.test/error-octet.mp4",
            RemoteMediaDiskCacheError::UnsupportedPayload,
        ),
        (
            "https://media.example.test/denied.bin",
            RemoteMediaDiskCacheError::InvalidResponse {
                status_code: Some(401),
            },
        ),
        ("https://media.example.test/large.bin", RemoteMediaDiskCacheError::PayloadTooLarge),
    ];
    for (remote, expected) in cases {
        let error = cache
            .materialize(
                &url(remote),
                RemoteMediaDiskCacheMaterializeOptions::default(),
                &async_cx,
            )
            .await
            .expect_err(remote);
        assert_eq!(cache_error(&error), Some(&expected), "{remote}");
    }
    assert_eq!(
        statistics(&cache),
        RemoteMediaDiskCacheStatistics {
            entry_count: 0,
            total_bytes: 0,
        }
    );
    probe.cleanup();
}

#[gpui::test]
fn account_cleanup_removes_only_the_selected_media_directory(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.blocking_lock();
    let first_user = "remote-cache-cleanup-a";
    let second_user = "remote-cache-cleanup-b";
    let first_directory =
        PathBuf::from(RemoteMediaDiskCache::folder_path_for_inspector(Some(first_user)));
    let second_directory =
        PathBuf::from(RemoteMediaDiskCache::folder_path_for_inspector(Some(second_user)));
    let first_marker = first_directory.join("marker");
    let second_marker = second_directory.join("marker");
    std::fs::write(&first_marker, [1]).expect("first marker");
    std::fs::write(&second_marker, [2]).expect("second marker");

    cx.update(|cx| AccountStateResetCoordinator::remove_persisted_cache(first_user, cx));

    assert!(!first_marker.exists());
    assert!(second_marker.exists());

    RemoteMediaDiskCache::remove_persisted_account(first_user);
    RemoteMediaDiskCache::remove_persisted_account(second_user);
    cx.update(|cx| AccountStateResetCoordinator::hard_reset(AccountStateResetReason::ForcedLogout, true, false, cx));
}

const TEST_BASE_URL: &str = "https://api.example.test";
const TEST_ACTIVE_USER: &str = "remote-cache-bearer-user";
const TEST_FOREIGN_USER: &str = "remote-cache-bearer-foreign";
const TEST_FRESH_TOKEN: &str = "token-active-fresh";
const TEST_EXPIRED_TOKEN: &str = "token-active-expired";
const TEST_REFRESHED_TOKEN: &str = "token-active-refreshed";
const TEST_FOREIGN_EXPIRED_TOKEN: &str = "token-foreign-expired";

static TEST_ACCESS_TOKEN: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));
static TEST_REFRESH_COUNT: AtomicUsize = AtomicUsize::new(0);
static TEST_HOOKS: Once = Once::new();

fn install_test_auth_hooks() {
    TEST_HOOKS.call_once(|| {
        set_sai_auth_hooks(SaiAuthHooks {
            access_token: Box::new(|_: &App| {
                let token = TEST_ACCESS_TOKEN.lock().clone();
                (!token.is_empty()).then_some(token)
            }),
            is_expired: Box::new(|token: &str, _leeway: f64| {
                token == TEST_EXPIRED_TOKEN || token == TEST_FOREIGN_EXPIRED_TOKEN
            }),
            extract_user_id: Box::new(|token: &str| match token {
                TEST_FRESH_TOKEN | TEST_EXPIRED_TOKEN | TEST_REFRESHED_TOKEN => {
                    Some(TEST_ACTIVE_USER.to_owned())
                }
                TEST_FOREIGN_EXPIRED_TOKEN => Some(TEST_FOREIGN_USER.to_owned()),
                _ => None,
            }),
            refresh_access_token: Box::new(|_: &mut App| -> LocalBoxFuture<'static, anyhow::Result<()>> {
                async { Ok(()) }.boxed_local()
            }),
            ensure_valid_access_token: Box::new(|_: &mut App| -> LocalBoxFuture<'static, anyhow::Result<()>> {
                TEST_REFRESH_COUNT.fetch_add(1, Ordering::SeqCst);
                *TEST_ACCESS_TOKEN.lock() = TEST_REFRESHED_TOKEN.to_owned();
                async { Ok(()) }.boxed_local()
            }),
            handle_auth_error: Box::new(|_: &mut App| {}),
            is_auth_error: Box::new(|_: Option<&str>, _: Option<&str>| false),
            note_authorized_reply: Box::new(|_: &mut App| {}),
            handle_account_blocked: Box::new(|_: &mut App| {}),
            is_premium: Box::new(|_: &App| false),
            effective_user_id_for_wire: Box::new(|_: &App| None),
            debug_first_account: Box::new(|_: &str, _: &App| None),
            debug_detach_active_account: Box::new(|_: &mut App| {}),
            active_account_id: Box::new(|_: &App| Some(TEST_ACTIVE_USER.to_owned())),
            switch_account: Box::new(|_: &str, _: &mut App| {}),
            load_account_backend_id: Box::new(|_: &str| None),
        });
    });
}

fn prepare_bearer(cx: &mut TestAppContext, access_token: &str) {
    install_test_auth_hooks();
    *TEST_ACCESS_TOKEN.lock() = access_token.to_owned();
    TEST_REFRESH_COUNT.store(0, Ordering::SeqCst);
    SaiAccountCacheScope::activate(TEST_ACTIVE_USER);
    cx.update(|cx| {
        SaiFetchHttpConfiguration::shared(cx).update(cx, |configuration, _| {
            configuration.base_url = Some(url(TEST_BASE_URL));
        });
    });
}

fn secure_remote(base: &str) -> Url {
    let route = rise_limits::UPLOAD_CONTEXTS
        .iter()
        .find_map(|context| context.secure_route)
        .expect("a secure route in canon");
    url(&format!("{base}{route}photo.jpg"))
}

#[gpui::test]
async fn bearer_token_goes_only_to_own_origin_secure_path(cx: &mut TestAppContext) {
    let _serial = SERIAL.lock().await;
    prepare_bearer(cx, TEST_FRESH_TOKEN);
    let async_cx = cx.to_async();

    let own_secure = secure_remote(TEST_BASE_URL);
    let foreign_secure = secure_remote("https://storage.example.test");
    let own_public = url(&format!("{TEST_BASE_URL}/media/photo.jpg"));

    assert_eq!(
        RemoteMediaDiskCache::bearer_token(&own_secure, None, &async_cx).await,
        Some(TEST_FRESH_TOKEN.to_owned())
    );
    assert_eq!(RemoteMediaDiskCache::bearer_token(&foreign_secure, None, &async_cx).await, None);
    assert_eq!(RemoteMediaDiskCache::bearer_token(&own_public, None, &async_cx).await, None);
    assert_eq!(TEST_REFRESH_COUNT.load(Ordering::SeqCst), 0);
    SaiAccountCacheScope::clear_active_account();
}

#[gpui::test]
async fn bearer_token_refreshes_an_expired_active_account_token(cx: &mut TestAppContext) {
    let _serial = SERIAL.lock().await;
    prepare_bearer(cx, TEST_EXPIRED_TOKEN);
    let async_cx = cx.to_async();
    let remote = secure_remote(TEST_BASE_URL);

    assert_eq!(
        RemoteMediaDiskCache::bearer_token(&remote, None, &async_cx).await,
        Some(TEST_REFRESHED_TOKEN.to_owned())
    );
    assert_eq!(TEST_REFRESH_COUNT.load(Ordering::SeqCst), 1);

    *TEST_ACCESS_TOKEN.lock() = TEST_EXPIRED_TOKEN.to_owned();
    assert_eq!(
        RemoteMediaDiskCache::bearer_token(&remote, Some(TEST_EXPIRED_TOKEN), &async_cx).await,
        Some(TEST_REFRESHED_TOKEN.to_owned())
    );
    assert_eq!(TEST_REFRESH_COUNT.load(Ordering::SeqCst), 2);
    SaiAccountCacheScope::clear_active_account();
}

#[gpui::test]
async fn bearer_token_keeps_a_foreign_explicit_token_unrefreshed(cx: &mut TestAppContext) {
    let _serial = SERIAL.lock().await;
    prepare_bearer(cx, TEST_FRESH_TOKEN);
    let async_cx = cx.to_async();
    let remote = secure_remote(TEST_BASE_URL);

    assert_eq!(
        RemoteMediaDiskCache::bearer_token(&remote, Some(TEST_FOREIGN_EXPIRED_TOKEN), &async_cx)
            .await,
        Some(TEST_FOREIGN_EXPIRED_TOKEN.to_owned())
    );
    assert_eq!(
        RemoteMediaDiskCache::bearer_token(&remote, Some("   "), &async_cx).await,
        None
    );
    assert_eq!(TEST_REFRESH_COUNT.load(Ordering::SeqCst), 0);
    SaiAccountCacheScope::clear_active_account();
}

#[test]
fn redirect_request_drops_authorization_on_a_foreign_origin() {
    let source = secure_remote(TEST_BASE_URL);
    let original = URLRequest::new(source.clone());
    let bearer = format!("Bearer {TEST_FRESH_TOKEN}");

    let mut presigned = URLRequest::new(url("https://storage.example.test/bucket/photo.jpg?sig=1"));
    presigned.set_value(&bearer, AUTHORIZATION.as_str());
    let continued = RemoteMediaDiskCache::redirect_request(&original, &presigned);
    assert_eq!(continued.value(AUTHORIZATION.as_str()), None);
    assert_eq!(continued.url, presigned.url);

    let mut same_origin = URLRequest::new(url(&format!("{TEST_BASE_URL}/media/photo.jpg")));
    same_origin.set_value(&bearer, AUTHORIZATION.as_str());
    let kept = RemoteMediaDiskCache::redirect_request(&original, &same_origin);
    assert_eq!(kept.value(AUTHORIZATION.as_str()), Some(bearer.as_str()));

    let unsigned = URLRequest::new(url("https://storage.example.test/bucket/photo.jpg?sig=2"));
    let untouched = RemoteMediaDiskCache::redirect_request(&original, &unsigned);
    assert_eq!(untouched.value(AUTHORIZATION.as_str()), None);
}
