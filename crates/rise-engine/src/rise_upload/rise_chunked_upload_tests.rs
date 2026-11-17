use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::future::BoxFuture;
use rise_core::foundation::foundation_url::Url;
use tokio::runtime::Handle;

use super::rise_chunked_upload::{
    RiseChunkedUploadClient, RiseChunkedUploadConfiguration, RiseChunkedUploadRequest, RiseChunkedUploadStep,
};
use crate::desktop_runtime::rise_account_runtime::GLOBAL_SLOT;
use crate::rise_wire::rise_wire::CancellationError;

fn client_recording_reactor(has_reactor: Arc<AtomicBool>) -> RiseChunkedUploadClient {
    let validate_context: RiseChunkedUploadStep = Arc::new(move || -> BoxFuture<'static, anyhow::Result<()>> {
        let has_reactor = has_reactor.clone();
        Box::pin(async move {
            has_reactor.store(Handle::try_current().is_ok(), Ordering::SeqCst);
            Err(CancellationError.into())
        })
    });
    let base_url = Url::from_string("http://127.0.0.1:1").expect("a valid literal url");
    RiseChunkedUploadClient::new(RiseChunkedUploadConfiguration::new(
        base_url,
        HashMap::new(),
        None,
        None,
        None,
        Some(validate_context),
        false,
    ))
}

fn request() -> RiseChunkedUploadRequest {
    RiseChunkedUploadRequest::new(
        PathBuf::from("/nonexistent/upload.bin"),
        "upload.bin".to_owned(),
        "application/octet-stream".to_owned(),
        "test".to_owned(),
    )
}

#[test]
fn upload_called_from_a_thread_without_a_reactor_runs_on_the_account_runtime() {
    let _slot = GLOBAL_SLOT.blocking_lock();
    assert!(Handle::try_current().is_err(), "the test thread must have no reactor");
    let has_reactor = Arc::new(AtomicBool::new(false));
    let client = client_recording_reactor(has_reactor.clone());

    let outcome = futures::executor::block_on(client.upload(request(), None, None, None));

    assert!(outcome.is_err());
    assert!(has_reactor.load(Ordering::SeqCst));
}
