use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::future::BoxFuture;
use gpui::{App, Context, Entity, Task};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use serde::{Deserialize, Serialize};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::modules::auth::services::auth_local_storage::AuthLocalStorage;
use crate::modules::rtc::engine::rise_rtc_call_system_hooks::RiseRTCCallSystemHooks;
use crate::modules::rtc::system::rise_rtc_system_models::{RiseRTCSystemIncomingEnvelope, RiseRTCSystemMediaMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseRTCSystemDomainBridgeError {
    #[error("RTC domain is unavailable")]
    DomainUnavailable,
    #[error("The active account changed while handling the call")]
    AccountChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseRTCSystemAnswerResult {
    Answered,
    AnsweredElsewhere,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCSystemOutgoingRequest {
    pub local_call_uuid: Uuid,
    pub callee_id: String,
    pub chat_id: String,
    pub media_mode: RiseRTCSystemMediaMode,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCSystemStartedCall {
    pub server_call_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiseRTCSystemVoIPAPNSEnvironment {
    Sandbox,
    Production,
}

impl RiseRTCSystemVoIPAPNSEnvironment {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Production => "production",
        }
    }

    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        match raw_value {
            "sandbox" => Some(Self::Sandbox),
            "production" => Some(Self::Production),
            _ => None,
        }
    }

    pub fn current() -> Self {
        if AppEnvironment::compiled() == AppEnvironment::Prod { Self::Production } else { Self::Sandbox }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCSystemVoIPTokenRegistration {
    pub token: String,
    pub environment: RiseRTCSystemVoIPAPNSEnvironment,
    pub ownership_generation: i64,
    pub installation_id: String,
}

impl RiseRTCSystemVoIPTokenRegistration {
    pub fn confirmation_key(&self) -> String {
        [
            self.token.as_str(),
            self.environment.raw_value(),
            &self.ownership_generation.to_string(),
            self.installation_id.as_str(),
        ]
        .join("|")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCSystemVoIPTokenRevocation {
    pub token: String,
    pub environment: RiseRTCSystemVoIPAPNSEnvironment,
}

/// tiny adapter the account domain installs on activation and removes on shutdown
pub trait RiseRTCSystemDomainClient: Send + Sync {
    fn account_id(&self) -> &str;

    fn accept_system_incoming_call(&self, envelope: RiseRTCSystemIncomingEnvelope) -> BoxFuture<'_, ()>;

    fn start_system_outgoing_call(
        &self,
        request: RiseRTCSystemOutgoingRequest,
    ) -> BoxFuture<'_, anyhow::Result<RiseRTCSystemStartedCall>>;

    fn answer_system_call(
        &self,
        call_id: String,
        installation_id: String,
    ) -> BoxFuture<'_, anyhow::Result<RiseRTCSystemAnswerResult>>;

    fn decline_system_call(
        &self,
        call_id: String,
        idempotency_key: String,
        reason: Option<String>,
    ) -> BoxFuture<'_, anyhow::Result<()>>;

    fn cancel_system_call(&self, call_id: String, idempotency_key: String) -> BoxFuture<'_, anyhow::Result<()>>;

    fn end_system_call(
        &self,
        call_id: String,
        idempotency_key: String,
        reason: Option<String>,
    ) -> BoxFuture<'_, anyhow::Result<()>>;

    fn set_system_call_muted(&self, call_id: String, is_muted: bool) -> BoxFuture<'_, anyhow::Result<()>>;

    fn register_system_vo_ip_token(
        &self,
        registration: RiseRTCSystemVoIPTokenRegistration,
    ) -> BoxFuture<'_, anyhow::Result<()>>;

    fn revoke_system_vo_ip_token(&self, revocation: RiseRTCSystemVoIPTokenRevocation) -> BoxFuture<'_, anyhow::Result<()>>;
}

pub struct RiseRTCSystemDomainBridge {
    client: Option<Weak<dyn RiseRTCSystemDomainClient>>,
    owner_id: Option<Uuid>,
}

impl RiseRTCSystemDomainBridge {
    pub const RESTORATION_TIMEOUT: Duration = Duration::from_secs(10);
    const RESTORATION_POLL_INTERVAL: Duration = Duration::from_millis(100);

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { client: None, owner_id: None }
    }

    pub fn install(&mut self, client: &Arc<dyn RiseRTCSystemDomainClient>, owner_id: Uuid, cx: &mut Context<Self>) {
        self.client = Some(Arc::downgrade(client));
        self.owner_id = Some(owner_id);
        cx.notify();
        cx.defer(RiseRTCCallSystemHooks::retry_vo_ip_registration);
    }

    pub fn remove(&mut self, owner_id: Uuid, cx: &mut Context<Self>) {
        if self.owner_id != Some(owner_id) {
            return;
        }
        self.owner_id = None;
        self.client = None;
        cx.notify();
    }

    pub fn current_client(&self, account_id: Option<&str>) -> Option<Arc<dyn RiseRTCSystemDomainClient>> {
        let client = self.client.as_ref()?.upgrade()?;
        if let Some(account_id) = account_id
            && client.account_id() != account_id
        {
            return None;
        }
        Some(client)
    }

    pub fn require_client(
        &self,
        account_id: String,
        restoration_timeout: Duration,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Arc<dyn RiseRTCSystemDomainClient>>> {
        if let Some(client) = self.current_client(Some(&account_id)) {
            return Task::ready(Ok(client));
        }

        let deadline = RiseClock::now().adding_duration(restoration_timeout);
        cx.spawn(async move |this, cx| {
            loop {
                let scoped_account_id = Self::normalized(SaiAccountCacheScope::active_user_id());
                if let Some(scoped_account_id) = scoped_account_id.as_deref()
                    && scoped_account_id != account_id
                {
                    return Err(RiseRTCSystemDomainBridgeError::AccountChanged.into());
                }
                if scoped_account_id.as_deref() == Some(account_id.as_str()) {
                    let preparation = cx.update(|cx| {
                        RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx))
                    });
                    if let Err(error) = preparation.await
                        && error.downcast_ref::<CancellationError>().is_some()
                    {
                        return Err(CancellationError.into());
                    }
                    if let Some(client) = this.read_with(cx, |bridge, _| bridge.current_client(Some(&account_id)))? {
                        return Ok(client);
                    }
                } else if let Some(persisted_account_id) = Self::normalized(AuthLocalStorage::active_account_id())
                    && persisted_account_id != account_id
                {
                    return Err(RiseRTCSystemDomainBridgeError::AccountChanged.into());
                }

                if RiseClock::now() >= deadline {
                    break;
                }
                cx.background_executor().timer(Self::RESTORATION_POLL_INTERVAL).await;
            }
            Err(RiseRTCSystemDomainBridgeError::DomainUnavailable.into())
        })
    }

    fn normalized(value: Option<String>) -> Option<String> {
        let value = value?;
        let value = trimming_whitespaces_and_newlines(&value);
        (!value.is_empty()).then(|| value.to_owned())
    }
}

pub type RiseRTCMediaSystemBridgeAudioOperation = Rc<dyn Fn(&mut App)>;
pub type RiseRTCMediaSystemBridgeResetOperation = Rc<dyn Fn(&mut App)>;

/// neutral bridge for the media layer to install audio ops later
/// desktop has no audio session
pub struct RiseRTCMediaSystemBridge {
    owner_id: Option<Uuid>,
    activate_operation: RiseRTCMediaSystemBridgeAudioOperation,
    deactivate_operation: RiseRTCMediaSystemBridgeAudioOperation,
    reset_operation: RiseRTCMediaSystemBridgeResetOperation,
}

impl RiseRTCMediaSystemBridge {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            owner_id: None,
            activate_operation: Rc::new(|_cx: &mut App| {}),
            deactivate_operation: Rc::new(|_cx: &mut App| {}),
            reset_operation: Rc::new(|_cx: &mut App| {}),
        }
    }

    pub fn install(
        &mut self,
        owner_id: Uuid,
        activate: RiseRTCMediaSystemBridgeAudioOperation,
        deactivate: RiseRTCMediaSystemBridgeAudioOperation,
        reset: RiseRTCMediaSystemBridgeResetOperation,
    ) -> Uuid {
        self.owner_id = Some(owner_id);
        self.activate_operation = activate;
        self.deactivate_operation = deactivate;
        self.reset_operation = reset;
        owner_id
    }

    pub fn remove(&mut self, owner_id: &Uuid) {
        if self.owner_id.as_ref() != Some(owner_id) {
            return;
        }
        self.owner_id = None;
        self.activate_operation = Rc::new(|_cx: &mut App| {});
        self.deactivate_operation = Rc::new(|_cx: &mut App| {});
        self.reset_operation = Rc::new(|_cx: &mut App| {});
    }

    /// the operation runs outside the bridge update because the media layer may read the bridge again
    pub fn activate(cx: &mut App) {
        let operation = Self::shared(cx).read(cx).activate_operation.clone();
        operation(cx);
    }

    pub fn deactivate(cx: &mut App) {
        let operation = Self::shared(cx).read(cx).deactivate_operation.clone();
        operation(cx);
    }

    pub fn reset(cx: &mut App) {
        let operation = Self::shared(cx).read(cx).reset_operation.clone();
        operation(cx);
    }
}
