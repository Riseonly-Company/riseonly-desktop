use std::time::Duration;

use futures::future::{Either, select};
use gpui::{AppContext, AsyncApp};
use rise_core::foundation::foundation_url::Url;

use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheMaterializeOptions,
};

pub struct APIClient;

impl APIClient {
    const TIMEOUT_INTERVAL_FOR_RESOURCE: Duration = Duration::from_secs(20);

    pub async fn download_image(from_url_string: &str, cx: &AsyncApp) -> Option<Vec<u8>> {
        let url = Url::from_string(from_url_string)?;
        let options = RemoteMediaDiskCacheMaterializeOptions::default();
        let cache = RemoteMediaDiskCache::shared();
        let transfer = cache.ensure_cached(&url, options, cx);
        let timeout = cx.background_executor().timer(Self::TIMEOUT_INTERVAL_FOR_RESOURCE);
        let path = match select(Box::pin(transfer), timeout).await {
            Either::Left((path, _)) => path?,
            Either::Right(_) => return None,
        };
        cx.background_spawn(async move { std::fs::read(path).ok() }).await
    }
}
