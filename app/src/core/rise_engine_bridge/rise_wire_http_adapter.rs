use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use gpui::{App, AsyncApp};
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::rise_upload::rise_url_session::{URLRequest, URLSession};
use rise_engine::{
    RiseEngineObservability, RiseWireAccountContext, RiseWireError, RiseWireHttpSender,
};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai_auth_hooks::{sai_auth_access_token, sai_auth_hooks};
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::rise_engine_bridge::rise_engine_trace_bridge::RiseEngineTraceBridge;
use crate::core::stores::http::http_service::HttpService;

/// posts a request frame to the gateway http twin of the socket, returns the response
/// for calls a suspended app still makes, eg map recorder position reports
/// no held socket needed, a dead connection costs one failed request
#[derive(Clone)]
pub struct RiseWireHttpAdapter {
    main: RiseEngineMainHop,
}

impl RiseWireHttpAdapter {
    pub fn new(cx: &mut App) -> Self {
        Self { main: RiseEngineMainHop::shared(cx) }
    }

    /// the token belongs to the account the engine was opened for or the call doesnt go
    async fn access_token(
        account: RiseWireAccountContext,
        refreshing: bool,
        cx: AsyncApp,
    ) -> anyhow::Result<String> {
        if !Self::is_engine_account(&account) {
            return Err(anyhow::Error::new(RiseWireError::StaleAccount));
        }
        let Some(hooks) = sai_auth_hooks() else {
            return Err(anyhow::Error::new(RiseWireError::NotConnected));
        };
        let refresh = cx.update(|cx| {
            if refreshing {
                (hooks.refresh_access_token)(cx)
            } else {
                (hooks.ensure_valid_access_token)(cx)
            }
        });
        refresh.await?;
        if !Self::is_engine_account(&account) {
            return Err(anyhow::Error::new(RiseWireError::StaleAccount));
        }
        cx.update(|cx| sai_auth_access_token(cx))
            .ok_or_else(|| anyhow::Error::new(RiseWireError::NotConnected))
    }

    fn is_engine_account(account: &RiseWireAccountContext) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(
            Some(&account.account_id),
            Some(&account.epoch),
        )
    }
}

impl RiseWireHttpSender for RiseWireHttpAdapter {
    fn post(
        &self,
        frame: Vec<u8>,
        account: RiseWireAccountContext,
        timeout: Duration,
    ) -> BoxFuture<'static, anyhow::Result<Vec<u8>>> {
        let main = self.main.clone();
        async move {
            let deadline = RiseClock::monotonic().saturating_add(timeout);
            let summary = if RiseEngineTraceBridge::DEBUG_TOOLS {
                RiseEngineTraceBridge::request_summary(&frame)
            } else {
                String::new()
            };
            let mut refreshed = false;
            loop {
                let token_account = account.clone();
                let refreshing = refreshed;
                let token = main
                    .run(move |cx: AsyncApp| {
                        Self::access_token(token_account, refreshing, cx).boxed_local()
                    })
                    .await?;
                let remaining = deadline.saturating_sub(RiseClock::monotonic());
                if remaining.is_zero() {
                    return Err(anyhow::Error::new(RiseWireError::Disconnected(
                        "HTTP deadline passed".to_owned(),
                    )));
                }
                let mut request = URLRequest::new(
                    HttpService::resolved_api_base_url().appending_path_component("rpc"),
                );
                request.timeout_interval = remaining;
                request.http_method = "POST".to_owned();
                request.http_body = Some(frame.clone());
                request.set_value("application/json", "Content-Type");
                request.set_value("application/json", "Accept");
                request.set_value(&format!("Bearer {token}"), "Authorization");
                RiseEngineObservability::transport_will_send(&summary);
                let (data, response) =
                    URLSession::shared().data_for(request).await.map_err(|error| {
                        anyhow::Error::new(RiseWireError::Disconnected(format!(
                            "HTTP transport: {error}"
                        )))
                    })?;
                match response.status_code {
                    200 | 404 => return Ok(data),
                    401 if !refreshed => refreshed = true,
                    status => {
                        return Err(anyhow::Error::new(RiseWireError::Disconnected(format!(
                            "HTTP {status}"
                        ))));
                    }
                }
            }
        }
        .boxed()
    }
}
