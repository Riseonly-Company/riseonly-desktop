use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::rtc::engine::core::rise_rtc_store_layout::RiseRTCStoredCall;
use crate::modules::rtc::engine::rise_rtc_domain::RiseRTCDomain;
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCAnswerOutcome, RiseRTCCallMediaMode, RiseRTCCallState, RiseRTCUserProfile,
};
use crate::modules::rtc::system::rise_rtc_system_bridges::{
    RiseRTCSystemAnswerResult, RiseRTCSystemDomainClient, RiseRTCSystemOutgoingRequest,
    RiseRTCSystemStartedCall, RiseRTCSystemVoIPTokenRegistration, RiseRTCSystemVoIPTokenRevocation,
};
use crate::modules::rtc::system::rise_rtc_system_models::{
    RiseRTCSystemCaller, RiseRTCSystemIncomingEnvelope, RiseRTCSystemMediaMode,
    RiseRTCSystemVoIPEvent,
};

impl RiseRTCSystemIncomingEnvelope {
    pub fn from_ringing(stored: &RiseRTCStoredCall, account_id: &str) -> Option<Self> {
        let call = &stored.call;
        if call.state != RiseRTCCallState::Ringing || call.callee_user_id != account_id {
            return None;
        }
        let call_uuid = Uuid::from_uuid_string(&call.call_id)?;
        Some(Self {
            call_uuid,
            server_call_id: Some(call.call_id.clone()),
            room_id: Some(call.room_id.clone()),
            chat_id: Some(call.chat_id.clone()),
            recipient_user_id: Some(call.callee_user_id.clone()),
            media_mode: if call.media_mode == RiseRTCCallMediaMode::Video {
                RiseRTCSystemMediaMode::Video
            } else {
                RiseRTCSystemMediaMode::Audio
            },
            event: RiseRTCSystemVoIPEvent::Incoming,
            caller: stored.caller.as_ref().map(|caller| RiseRTCSystemCaller {
                id: Some(caller.id.clone()),
                tag: Some(caller.tag.clone()),
                name: Some(caller.name.clone()),
                avatar_url: Some(caller.avatar_url.clone()),
            }),
            created_at_milliseconds: Some(call.created_at_ms),
            expires_at_milliseconds: Some(call.expires_at_ms),
            occurred_at_milliseconds: Some(stored.occurred_at_ms),
            reason: None,
        })
    }
}

/// retained by the account domain and held weakly by the system bridge
/// the system call layer never reaches into repository or transport state
pub struct RiseRTCDomainSystemAdapter {
    pub account_id: String,
    domain: Arc<RiseRTCDomain>,
}

impl RiseRTCDomainSystemAdapter {
    pub fn new(domain: Arc<RiseRTCDomain>) -> Self {
        let account_id = domain.account_id.clone();
        Self { account_id, domain }
    }
}

impl RiseRTCSystemDomainClient for RiseRTCDomainSystemAdapter {
    fn account_id(&self) -> &str {
        &self.account_id
    }

    fn accept_system_incoming_call(
        &self,
        envelope: RiseRTCSystemIncomingEnvelope,
    ) -> BoxFuture<'_, ()> {
        async move {
            let addressed_here = envelope
                .recipient_user_id
                .as_deref()
                .is_none_or(|recipient| recipient == self.account_id);
            if !addressed_here {
                return;
            }
            let Some(call_id) = envelope.server_call_id else {
                return;
            };
            let caller = envelope.caller.and_then(|caller| {
                let id = caller.id?;
                let tag = caller.tag.unwrap_or_default();
                Some(RiseRTCUserProfile {
                    id,
                    name: caller.name.unwrap_or_else(|| tag.clone()),
                    tag,
                    avatar_url: caller.avatar_url.unwrap_or_default(),
                    is_premium: false,
                    is_official: false,
                    is_online: false,
                    last_seen_ms: 0,
                })
            });
            self.domain.accept_system_incoming_call(&call_id, caller).await;
        }
        .boxed()
    }

    fn start_system_outgoing_call(
        &self,
        request: RiseRTCSystemOutgoingRequest,
    ) -> BoxFuture<'_, anyhow::Result<RiseRTCSystemStartedCall>> {
        async move {
            let media_mode = if request.media_mode == RiseRTCSystemMediaMode::Video {
                RiseRTCCallMediaMode::Video
            } else {
                RiseRTCCallMediaMode::Audio
            };
            let call = self
                .domain
                .start_direct_call(
                    &request.callee_id,
                    &request.chat_id,
                    media_mode,
                    &request.idempotency_key,
                )
                .await?;
            Ok(RiseRTCSystemStartedCall { server_call_id: call.call_id })
        }
        .boxed()
    }

    fn answer_system_call(
        &self,
        call_id: String,
        installation_id: String,
    ) -> BoxFuture<'_, anyhow::Result<RiseRTCSystemAnswerResult>> {
        async move {
            match self.domain.answer_call(&call_id, None, &installation_id).await? {
                RiseRTCAnswerOutcome::Answered(_) => Ok(RiseRTCSystemAnswerResult::Answered),
                RiseRTCAnswerOutcome::AnsweredElsewhere(_) => {
                    Ok(RiseRTCSystemAnswerResult::AnsweredElsewhere)
                }
            }
        }
        .boxed()
    }

    fn decline_system_call(
        &self,
        call_id: String,
        idempotency_key: String,
        reason: Option<String>,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        async move {
            self.domain.decline_call(&call_id, reason.as_deref(), &idempotency_key).await?;
            Ok(())
        }
        .boxed()
    }

    fn cancel_system_call(
        &self,
        call_id: String,
        idempotency_key: String,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        async move {
            self.domain.cancel_call(&call_id, None, &idempotency_key).await?;
            Ok(())
        }
        .boxed()
    }

    fn end_system_call(
        &self,
        call_id: String,
        idempotency_key: String,
        reason: Option<String>,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        async move {
            self.domain.end_call(&call_id, reason.as_deref(), &idempotency_key).await?;
            Ok(())
        }
        .boxed()
    }

    fn set_system_call_muted(
        &self,
        _call_id: String,
        is_muted: bool,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        async move { self.domain.set_microphone(!is_muted).await }.boxed()
    }

    fn register_system_vo_ip_token(
        &self,
        registration: RiseRTCSystemVoIPTokenRegistration,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        async move {
            self.domain
                .register_system_vo_ip_token(
                    &registration.token,
                    registration.environment.raw_value(),
                    registration.ownership_generation,
                    &registration.installation_id,
                )
                .await
        }
        .boxed()
    }

    fn revoke_system_vo_ip_token(
        &self,
        revocation: RiseRTCSystemVoIPTokenRevocation,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        async move {
            self.domain
                .revoke_system_vo_ip_token(&revocation.token, revocation.environment.raw_value())
                .await
        }
        .boxed()
    }
}
