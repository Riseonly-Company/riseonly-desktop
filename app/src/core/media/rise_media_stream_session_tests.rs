use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::task::{Context, Poll};

use futures::FutureExt;
use gpui::{AsyncApp, TestAppContext};
use parking_lot::Mutex;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode, URLRequest};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{RemoteMediaDiskCache, RemoteMediaHttpResponse, RemoteMediaTransferError};
use crate::core::media::rise_media_stream_session::{RiseMediaStreamSession, RiseMediaStreamSessionRangeFetcher};
use crate::core::media::rise_progressive_data_fetcher::{
    RiseProgressiveDataFetcherOnChunk, RiseProgressiveDataFetcherOnResponse,
};

static SERIAL: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

const OBJECT_URL: &str = "https://media.test/object.mp4";
const OBJECT_KEY: &str = "media:https://media.test/object.mp4";

struct RiseMediaRangeServerYield(bool);

impl Future for RiseMediaRangeServerYield {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            return Poll::Ready(());
        }
        self.0 = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RiseMediaRangeServerServed {
    start: i64,
    length: i64,
}

#[derive(Default)]
struct RiseMediaRangeServerState {
    served: Vec<RiseMediaRangeServerServed>,
    stall_after: Option<i64>,
}

/// stub media host with range support
/// records every range so tests can prove the player and download share one stream
struct RiseMediaRangeServer {
    payload: Vec<u8>,
    content_type: String,
    supports_range: bool,
    chunk_size: usize,
    state: Mutex<RiseMediaRangeServerState>,
}

struct RiseMediaRangeServerRecord {
    server: Arc<RiseMediaRangeServer>,
    start: i64,
    delivered: i64,
}

impl Drop for RiseMediaRangeServerRecord {
    fn drop(&mut self) {
        self.server.state.lock().served.push(RiseMediaRangeServerServed {
            start: self.start,
            length: self.delivered,
        });
    }
}

impl RiseMediaRangeServer {
    fn new(payload: Vec<u8>, supports_range: bool, chunk_size: usize) -> Arc<Self> {
        Arc::new(Self {
            payload,
            content_type: "video/mp4".to_owned(),
            supports_range,
            chunk_size,
            state: Mutex::new(RiseMediaRangeServerState::default()),
        })
    }

    /// kills the next response after this many bytes to test retry and resume
    fn stall(&self, after_byte_count: i64) {
        self.state.lock().stall_after = Some(after_byte_count);
    }

    fn served_ranges(&self) -> Vec<RiseMediaRangeServerServed> {
        self.state.lock().served.clone()
    }

    fn request_count(&self) -> usize {
        self.state.lock().served.len()
    }

    fn fetcher(self: &Arc<Self>) -> RiseMediaStreamSessionRangeFetcher {
        let server = Arc::clone(self);
        Arc::new(
            move |request: URLRequest,
                  on_response: RiseProgressiveDataFetcherOnResponse,
                  on_chunk: RiseProgressiveDataFetcherOnChunk,
                  _cx: AsyncApp| {
                let server = Arc::clone(&server);
                let start = Self::range_start(&request);
                async move { server.serve(start, on_response, on_chunk).await }.boxed_local()
            },
        )
    }

    fn range_start(request: &URLRequest) -> i64 {
        let Some(header) = request.value(reqwest::header::RANGE.as_str()) else {
            return 0;
        };
        let Some(value) = header.strip_prefix("bytes=") else {
            return 0;
        };
        let Some((start, _)) = value.split_once('-') else {
            return 0;
        };
        start.parse::<i64>().unwrap_or(0)
    }

    async fn serve(
        self: Arc<Self>,
        requested_start: i64,
        on_response: RiseProgressiveDataFetcherOnResponse,
        on_chunk: RiseProgressiveDataFetcherOnChunk,
    ) -> Result<(), RemoteMediaTransferError> {
        let total = i64::try_from(self.payload.len()).unwrap_or(i64::MAX);
        let start = if self.supports_range { requested_start.min(total) } else { 0 };
        let status_code = if self.supports_range && requested_start > 0 { 206 } else { 200 };
        let mut headers = vec![(reqwest::header::CONTENT_TYPE.as_str().to_owned(), self.content_type.clone())];
        if status_code == 206 {
            headers.push((
                reqwest::header::CONTENT_RANGE.as_str().to_owned(),
                format!("bytes {start}-{}/{total}", total - 1),
            ));
            headers.push((reqwest::header::CONTENT_LENGTH.as_str().to_owned(), (total - start).to_string()));
        } else {
            headers.push((reqwest::header::CONTENT_LENGTH.as_str().to_owned(), total.to_string()));
        }
        let response = RemoteMediaHttpResponse::new(
            Url::from_string(OBJECT_URL).expect("fixture URL parses"),
            status_code,
            headers,
        );
        if !on_response(&response) {
            return Ok(());
        }

        let limit = self.state.lock().stall_after.take();
        let mut record = RiseMediaRangeServerRecord {
            server: Arc::clone(&self),
            start,
            delivered: 0,
        };
        let mut cursor = start;
        while cursor < total {
            if let Some(limit) = limit
                && record.delivered >= limit
            {
                return Err(URLError::new(URLErrorCode::NetworkConnectionLost).into());
            }
            let end = total.min(cursor + i64::try_from(self.chunk_size).unwrap_or(i64::MAX));
            let slice = &self.payload[usize::try_from(cursor).unwrap_or(0)..usize::try_from(end).unwrap_or(0)];
            on_chunk(slice);
            record.delivered += i64::try_from(slice.len()).unwrap_or(0);
            cursor = end;
            RiseMediaRangeServerYield(false).await;
        }
        Ok(())
    }
}

struct RiseMediaStreamSessionTestsTeardown(String);

impl Drop for RiseMediaStreamSessionTestsTeardown {
    fn drop(&mut self) {
        RemoteMediaDiskCache::clear_all(Some(&self.0));
        RemoteMediaDiskCache::remove_partials(Some(&self.0));
        SaiAccountCacheScope::clear_active_account();
    }
}

fn payload(byte_count: usize) -> Vec<u8> {
    (0..byte_count).map(|index| u8::try_from(index % 251).unwrap_or(0)).collect()
}

fn activate_account() -> RiseMediaStreamSessionTestsTeardown {
    let user_id = format!("stream-test-{}", Uuid::new_v4().uuid_string().to_lowercase());
    SaiAccountCacheScope::activate(&user_id);
    RiseMediaStreamSessionTestsTeardown(user_id)
}

fn object_url() -> Url {
    Url::from_string(OBJECT_URL).expect("fixture URL parses")
}

fn make_session(server: &Arc<RiseMediaRangeServer>, cache_key: &str) -> Arc<RiseMediaStreamSession> {
    RiseMediaStreamSession::new(
        cache_key,
        object_url(),
        SaiAccountCacheScope::active_user_id(),
        SaiAccountCacheScope::active_epoch(),
        64 * 1_024 * 1_024,
        Some("object.mp4".to_owned()),
        Some(server.fetcher()),
    )
}

fn stored(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).expect("committed file reads")
}

#[gpui::test]
async fn completed_stream_is_cached(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let _teardown = activate_account();

    let bytes = payload(40_000);
    let server = RiseMediaRangeServer::new(bytes.clone(), true, 1_024);
    let session = make_session(&server, OBJECT_KEY);

    let committed = session.complete(&async_cx).await.expect("stream completes");
    assert_eq!(stored(&committed), bytes);

    let user_id = SaiAccountCacheScope::active_user_id();
    let epoch = SaiAccountCacheScope::active_epoch();
    let looked = RemoteMediaDiskCache::shared().existing_local_url(
        &object_url(),
        Some(OBJECT_KEY),
        user_id.as_deref(),
        epoch.as_deref(),
    );
    assert!(looked.is_some());
    assert_eq!(server.request_count(), 1);
}

#[gpui::test]
async fn reads_serve_while_the_download_is_still_running(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let _teardown = activate_account();

    let bytes = payload(200_000);
    let server = RiseMediaRangeServer::new(bytes.clone(), true, 2_048);
    let session = make_session(&server, OBJECT_KEY);

    let information = session.content_information(&async_cx).await.expect("content information");
    assert_eq!(information.content_length, 200_000);
    assert_eq!(information.content_type.as_deref(), Some("video/mp4"));

    let head = session.read(0, 1_024, &async_cx).await.expect("head reads");
    assert!(!head.is_empty());
    assert_eq!(head, bytes[..head.len()]);

    let downloaded = session.downloaded_byte_count().await;
    assert!(downloaded < 200_000);

    let committed = session.complete(&async_cx).await.expect("stream completes");
    assert_eq!(stored(&committed), bytes);
}

#[gpui::test]
async fn seek_reaims_the_single_stream(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let _teardown = activate_account();

    let total: usize = 6 * 1_024 * 1_024;
    let bytes = payload(total);
    let chunk: usize = 64 * 1_024;
    let server = RiseMediaRangeServer::new(bytes.clone(), true, chunk);
    let session = make_session(&server, OBJECT_KEY);

    session.content_information(&async_cx).await.expect("content information");
    let seek_offset: usize = 5 * 1_024 * 1_024;
    let tail = session
        .read(i64::try_from(seek_offset).unwrap_or(0), 4_096, &async_cx)
        .await
        .expect("tail reads");
    assert!(!tail.is_empty());
    assert_eq!(tail, bytes[seek_offset..seek_offset + tail.len()]);

    let ranges = server.served_ranges();
    assert!(
        ranges
            .iter()
            .any(|range| range.start == i64::try_from(seek_offset).unwrap_or(0))
    );

    let committed = session.complete(&async_cx).await.expect("stream completes");
    assert_eq!(stored(&committed), bytes);

    let fetched: i64 = server.served_ranges().iter().map(|range| range.length).sum();
    let total = i64::try_from(total).unwrap_or(0);
    let chunk = i64::try_from(chunk).unwrap_or(0);
    assert!(fetched >= total);
    assert!(fetched <= total + 3 * chunk);
}

#[gpui::test]
async fn resumes_after_a_transport_failure(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let _teardown = activate_account();

    let bytes = payload(50_000);
    let server = RiseMediaRangeServer::new(bytes.clone(), true, 1_024);
    server.stall(10_000);
    let session = make_session(&server, OBJECT_KEY);

    let committed = session.complete(&async_cx).await.expect("stream completes");
    assert_eq!(stored(&committed), bytes);

    let ranges = server.served_ranges();
    assert!(ranges.len() >= 2);
    assert!(ranges.iter().skip(1).all(|range| range.start > 0));
}

#[gpui::test]
async fn handles_a_server_without_range_support(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let _teardown = activate_account();

    let bytes = payload(30_000);
    let server = RiseMediaRangeServer::new(bytes.clone(), false, 4_096);
    let session = make_session(&server, OBJECT_KEY);

    session.content_information(&async_cx).await.expect("content information");
    let middle = session.read(20_000, 512, &async_cx).await.expect("middle reads");
    assert_eq!(middle, bytes[20_000..20_000 + middle.len()]);

    let committed = session.complete(&async_cx).await.expect("stream completes");
    assert_eq!(stored(&committed), bytes);
}

#[gpui::test]
async fn completed_object_is_not_fetched_again(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    let async_cx = cx.to_async();
    let _teardown = activate_account();

    let bytes = payload(20_000);
    let server = RiseMediaRangeServer::new(bytes.clone(), true, 1_024);

    let first = make_session(&server, OBJECT_KEY);
    first.complete(&async_cx).await.expect("first stream completes");
    let after_first = server.request_count();

    let second = make_session(&server, OBJECT_KEY);
    let committed = second.complete(&async_cx).await.expect("second session completes");
    assert_eq!(stored(&committed), bytes);
    assert_eq!(server.request_count(), after_first);
}
