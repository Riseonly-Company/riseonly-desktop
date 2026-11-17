use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use gpui::{App, AsyncApp, Task};
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_engine::CancellationError;
use rise_media::video::av_asset::{
    AVAssetResourceLoaderDelegate, AVAssetResourceLoadingRequest, AVURLAsset,
};
use rise_platform::uniform_type::UTType;

use crate::core::media::rise_media_stream_session::{RiseMediaStreamRegistry, RiseMediaStreamSession};

/// streaming asset fed by the stream session and not the player own http
/// one request per object, ranges served from the growing download file
/// custom scheme because the backend wont hand an https asset to a loader
pub struct RiseMediaStreamingAsset {
    pub asset: AVURLAsset,
    delegate: Rc<RiseMediaResourceLoaderDelegate>,
}

impl RiseMediaStreamingAsset {
    pub const SCHEME: &'static str = "riseonlystream";

    /// none for anything that cant be served this way, caller falls back to a plain remote asset
    pub fn make(
        remote: &Url,
        cache_key: &str,
        maximum_byte_count: i64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
    ) -> Option<RiseMediaStreamingAsset> {
        let scheme = remote.scheme()?.to_lowercase();
        if scheme != "https" && scheme != "http" {
            return None;
        }
        let mut components = UrlComponents::from_url(remote, false)?;
        components.set_scheme(Some(Self::SCHEME));
        let stream_url = components.url()?;

        let delegate = RiseMediaResourceLoaderDelegate::new(
            remote.clone(),
            cache_key,
            maximum_byte_count,
            account_user_id,
            account_epoch,
        );
        let asset = AVURLAsset::new(stream_url);
        let weak_delegate = Rc::downgrade(&delegate);
        let weak_delegate: Weak<dyn AVAssetResourceLoaderDelegate> = weak_delegate;
        asset.resource_loader().set_delegate(weak_delegate);
        Some(RiseMediaStreamingAsset { asset, delegate })
    }

    /// true while the backend has a range request open, a slow source is loading not stalled
    pub fn has_open_requests(&self) -> bool {
        self.delegate.has_open_requests()
    }

    pub fn teardown(&self) {
        self.delegate.teardown();
    }
}

#[derive(Default)]
struct RiseMediaResourceLoaderDelegateState {
    tasks: HashMap<u64, Task<()>>,
    is_torn_down: bool,
    resolved_session: Option<Arc<RiseMediaStreamSession>>,
}

struct RiseMediaResourceLoaderDelegate {
    remote: Url,
    cache_key: String,
    maximum_byte_count: i64,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    this: Weak<RiseMediaResourceLoaderDelegate>,
    state: RefCell<RiseMediaResourceLoaderDelegateState>,
}

impl RiseMediaResourceLoaderDelegate {
    const READ_CHUNK: i64 = 256 * 1_024;

    fn new(
        remote: Url,
        cache_key: &str,
        maximum_byte_count: i64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
    ) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            remote,
            cache_key: cache_key.to_owned(),
            maximum_byte_count,
            account_user_id,
            account_epoch,
            this: this.clone(),
            state: RefCell::new(RiseMediaResourceLoaderDelegateState::default()),
        })
    }

    fn has_open_requests(&self) -> bool {
        let state = self.state.borrow();
        !state.is_torn_down && !state.tasks.is_empty()
    }

    fn teardown(&self) {
        let open = {
            let mut state = self.state.borrow_mut();
            state.is_torn_down = true;
            std::mem::take(&mut state.tasks)
        };
        drop(open);
    }

    fn forget(&self, identifier: u64) {
        let finished = self.state.borrow_mut().tasks.remove(&identifier);
        drop(finished);
    }

    async fn session(&self) -> Arc<RiseMediaStreamSession> {
        if let Some(resolved) = self.state.borrow().resolved_session.clone() {
            return resolved;
        }
        let session = RiseMediaStreamRegistry::shared()
            .session(
                self.cache_key.clone(),
                self.remote.clone(),
                self.account_user_id.clone(),
                self.account_epoch.clone(),
                self.maximum_byte_count,
                Some(self.remote.last_path_component()),
            )
            .await;
        self.state.borrow_mut().resolved_session = Some(Arc::clone(&session));
        session
    }

    async fn serve(&self, request: AVAssetResourceLoadingRequest, cx: &AsyncApp) {
        let session = self.session().await;
        match self.serve_bytes(&session, &request, cx).await {
            Ok(()) => request.finish_loading(),
            Err(error) if error.downcast_ref::<CancellationError>().is_some() => {}
            Err(error) => request.finish_loading_with_error(error),
        }
    }

    async fn serve_bytes(
        &self,
        session: &RiseMediaStreamSession,
        request: &AVAssetResourceLoadingRequest,
        cx: &AsyncApp,
    ) -> anyhow::Result<()> {
        let information = session.content_information(cx).await?;
        if let Some(content_information_request) = request.content_information_request() {
            content_information_request.set_is_byte_range_access_supported(true);
            content_information_request.set_content_length(information.content_length);
            content_information_request
                .set_content_type(Self::content_type(information.content_type.as_deref(), &self.remote));
        }

        let Some(data_request) = request.data_request() else {
            return Ok(());
        };

        let mut offset = data_request.current_offset();
        let end = if data_request.requests_all_data_to_end_of_resource() {
            information.content_length
        } else {
            match data_request
                .requested_offset()
                .checked_add(data_request.requested_length())
            {
                Some(sum) => information.content_length.min(sum),
                None => information.content_length,
            }
        };

        while offset < end {
            let wanted = Self::READ_CHUNK.min(end - offset);
            let chunk = session.read(offset, wanted, cx).await?;
            if chunk.is_empty() {
                break;
            }
            data_request.respond(&chunk);
            offset += i64::try_from(chunk.len()).unwrap_or(i64::MAX);
        }
        Ok(())
    }

    /// backend wants a uti and refuses to start without one it knows, not a mime type
    fn content_type(mime_type: Option<&str>, remote: &Url) -> Option<String> {
        let audiovisual = UTType::audiovisual_content();
        if let Some(mime_type) = mime_type
            && let Some(r#type) = UTType::from_mime_type(mime_type)
            && r#type.conforms_to(&audiovisual)
        {
            return Some(r#type.identifier());
        }
        let file_extension = remote.path_extension().to_lowercase();
        if !file_extension.is_empty()
            && let Some(r#type) = UTType::from_filename_extension(&file_extension)
            && r#type.conforms_to(&audiovisual)
        {
            return Some(r#type.identifier());
        }
        Some(UTType::mpeg4_movie().identifier())
    }
}

impl Drop for RiseMediaResourceLoaderDelegate {
    fn drop(&mut self) {
        self.teardown();
    }
}

impl AVAssetResourceLoaderDelegate for RiseMediaResourceLoaderDelegate {
    fn should_wait_for_loading_of_requested_resource(
        &self,
        loading_request: AVAssetResourceLoadingRequest,
        cx: &mut App,
    ) -> bool {
        if self.state.borrow().is_torn_down {
            return false;
        }
        let identifier = loading_request.identifier();
        let this = self.this.clone();
        let task = cx.spawn(async move |cx| {
            let Some(delegate) = this.upgrade() else {
                return;
            };
            delegate.serve(loading_request, cx).await;
            delegate.forget(identifier);
        });
        let replaced = self.state.borrow_mut().tasks.insert(identifier, task);
        drop(replaced);
        true
    }

    fn did_cancel(&self, loading_request: AVAssetResourceLoadingRequest, _cx: &mut App) {
        let cancelled = self.state.borrow_mut().tasks.remove(&loading_request.identifier());
        drop(cancelled);
    }
}
