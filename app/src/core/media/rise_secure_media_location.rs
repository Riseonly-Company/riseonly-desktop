use std::time::Duration;

use gpui::AsyncApp;
use rise_core::foundation::foundation_url::Url;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::core::engine_bridge::network_transport_config::NetworkClientSlot;
use rise_engine::rise_upload::rise_url_session::URLRequest;

use crate::core::lib::canon_failure_bridge::RiseMediaRefusal;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{RemoteMediaDiskCache, RemoteMediaHttpResponse};

/// resolves a protected media route to its presigned storage address
/// route answers 307 and a bearer on top gets 403, player cant strip headers
pub enum RiseSecureMediaLocation {}

static REDIRECT_CAPTURE: NetworkClientSlot = NetworkClientSlot::without_redirects();

impl RiseSecureMediaLocation {
    const TIMEOUT: Duration = Duration::from_secs(15);

    /// signed address behind the source, none if its not a protected route or the hop failed
    pub async fn resolve(source: &Url, cx: &mut AsyncApp) -> Option<Url> {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let bearer = RemoteMediaDiskCache::bearer_token(source, None, cx).await?;

        let mut request = URLRequest::new(source.clone());
        request.timeout_interval = Self::TIMEOUT;
        request.set_value(&format!("Bearer {bearer}"), reqwest::header::AUTHORIZATION.as_str());

        let outbound = request.clone();
        let hop = RiseAccountRuntime::current()
            .spawn(async move {
                let session = REDIRECT_CAPTURE.current()?;
                let mut builder = session
                    .get(outbound.url.absolute_string())
                    .timeout(outbound.timeout_interval);
                for (name, value) in outbound.all_http_header_fields() {
                    builder = builder.header(name.as_str(), value.as_str());
                }
                let response = builder.send().await.ok()?;
                let head = RemoteMediaHttpResponse::from_reqwest(&response, outbound.url.clone());
                let body = response.bytes().await.ok()?.to_vec();
                Some((body, head))
            })
            .await
            .ok()??;
        let (body, response) = hop;
        RiseMediaRefusal::publish(
            body,
            Some(response.status_code),
            Some(response.url.clone()),
            request,
            account_user_id.clone(),
            account_epoch.clone(),
            cx,
        )
        .await;
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
            || !(300..400).contains(&response.status_code)
        {
            return None;
        }
        let location = response.value(reqwest::header::LOCATION.as_str())?;
        let resolved = Url::from_string_relative_to(location, Some(source))?;
        Url::from_string(resolved.absolute_string())
    }
}
