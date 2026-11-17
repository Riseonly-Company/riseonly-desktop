use std::time::Duration;

use rise_canon::{rise_blob, rise_http_path};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::{URLRequest, URLSession};

use crate::core::stores::http::http_service::HttpService;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegistrationPolicyProbeMode {
    Open,
    InviteOnly,
    CaptchaRequired,
    Closed,
}

impl RegistrationPolicyProbeMode {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InviteOnly => "invite_only",
            Self::CaptchaRequired => "captcha_required",
            Self::Closed => "closed",
        }
    }

    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        [Self::Open, Self::InviteOnly, Self::CaptchaRequired, Self::Closed]
            .into_iter()
            .find(|mode| mode.raw_value() == raw_value)
    }
}

/// registration switch read before sign up steps
/// invite_only adds the invite step, closed stops at phone with server refusal
/// unknown values from a newer server read as open
pub enum RegistrationPolicyProbe {}

impl RegistrationPolicyProbe {
    pub const PATH: &'static str = rise_http_path::bootstrap::REGISTRATION;

    const TIMEOUT_INTERVAL: Duration = Duration::from_secs(6);

    /// none when the answer couldnt be obtained, the flow then acts as open
    pub async fn read() -> Option<RegistrationPolicyProbeMode> {
        let mut request = URLRequest::new(HttpService::absolute_url(Self::PATH));
        request.http_method = "GET".to_owned();
        request.timeout_interval = Self::TIMEOUT_INTERVAL;
        let (data, response) = RiseAccountRuntime::current()
            .spawn(async move { URLSession::shared().data_for(request).await })
            .await
            .ok()?
            .ok()?;
        if response.status_code != 200 {
            return None;
        }
        let payload: rise_blob::bootstrap::Registration = serde_json::from_slice(&data).ok()?;
        let mode = payload.mode.as_deref().map(str::trim).unwrap_or_default();
        Some(RegistrationPolicyProbeMode::from_raw_value(mode).unwrap_or(RegistrationPolicyProbeMode::Open))
    }
}
