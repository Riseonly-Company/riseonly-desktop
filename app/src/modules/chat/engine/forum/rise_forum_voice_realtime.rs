use rise_engine::desktop_runtime::rise_runtime_timer::RiseRuntimeTimer;
use std::sync::Arc;
use std::time::Duration;

use rise_canon::{RiseEvent, RiseWireRequestMetadata, rise_event_payload, rise_key, rise_request, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{RiseEngine, RiseJSONValue, RiseWireMethodDescriptor, RiseWirePushEnvelope, RiseWireReplayPolicy};
use tokio::task::AbortHandle;

use crate::modules::chat::engine::forum::rise_forum_voice_presence::{
    RiseForumVoiceUpdate, RiseForumVoiceUpdateKind, RiseForumVoiceUpdateScope,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    VoiceChannelParticipantWireDTO, VoiceChannelParticipantWireDTOExt,
};

const SUBSCRIPTION_TIMEOUT: Duration = Duration::from_secs(10);

const SUBSCRIBE_RETRY_DELAY: Duration = Duration::from_secs(2);

/// name the update kind is held under in the conversation domain
pub type RiseForumVoiceRealtimeKind = RiseForumVoiceUpdateKind;

fn subscribed_in_reply(value: &RiseJSONValue) -> Option<bool> {
    match value.object_value()?.get(rise_key::gateway_reply::SUBSCRIBED)? {
        RiseJSONValue::Bool(subscribed) => Some(*subscribed),
        _ => None,
    }
}

pub struct RiseForumVoiceEndpoints;

impl RiseForumVoiceEndpoints {
    /// gateway control frame, not a microservice rpc
    /// it only registers the socket interest so replay after a reconnect is safe and needed
    pub fn subscribe() -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(
            rise_wire_methods::gateway::SERVICE,
            rise_wire_methods::gateway::VOICE_CHANNELS_SUBSCRIBE,
        )
        .with_timeout(SUBSCRIPTION_TIMEOUT)
        .with_replay_policy(RiseWireReplayPolicy::ReadOnly)
    }

    pub fn unsubscribe() -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(
            rise_wire_methods::gateway::SERVICE,
            rise_wire_methods::gateway::VOICE_CHANNELS_UNSUBSCRIBE,
        )
        .with_timeout(SUBSCRIPTION_TIMEOUT)
        .with_replay_policy(RiseWireReplayPolicy::ReadOnly)
    }
}

struct RiseForumVoiceSubscriptionAbortOnDrop(AbortHandle);

impl Drop for RiseForumVoiceSubscriptionAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct RiseForumVoiceRealtime;

impl RiseForumVoiceRealtime {
    pub const EVENT_TYPE: &'static str = rise_events::name::VOICE_CHANNEL_UPDATE;

    /// server re authorizes every channel id
    /// so this list can only narrow what the socket gets, never widen it
    pub async fn set_subscribed(
        subscribed: bool,
        chat_id: &str,
        topic_ids: &[String],
        include_group: bool,
        engine: &Arc<RiseEngine>,
    ) -> bool {
        let chat_id = trimming_whitespaces_and_newlines(chat_id).to_owned();
        if chat_id.is_empty() {
            return false;
        }
        if subscribed && topic_ids.is_empty() && !include_group {
            return false;
        }
        let topic_ids = topic_ids.to_vec();
        let engine = engine.clone();
        let exchange = RiseAccountRuntime::current()
            .spawn(async move { Self::exchange(subscribed, chat_id, topic_ids, include_group, engine).await });
        let _abort_on_drop = RiseForumVoiceSubscriptionAbortOnDrop(exchange.abort_handle());
        exchange.await.unwrap_or(false)
    }

    async fn exchange(
        subscribed: bool,
        chat_id: String,
        topic_ids: Vec<String>,
        include_group: bool,
        engine: Arc<RiseEngine>,
    ) -> bool {
        let descriptor =
            if subscribed { RiseForumVoiceEndpoints::subscribe() } else { RiseForumVoiceEndpoints::unsubscribe() };
        let body = if subscribed {
            RiseJSONValue::encode(&rise_request::gateway::VoiceChannelsSubscribe {
                chat_id: Some(chat_id),
                include_group: Some(include_group),
                topic_ids: Some(topic_ids),
            })
        } else {
            RiseJSONValue::encode(&rise_request::gateway::VoiceChannelsUnsubscribe {
                chat_id: Some(chat_id),
                include_group: Some(include_group),
                topic_ids: Some(topic_ids),
            })
        };
        let Ok(body) = body else {
            return false;
        };

        for attempt in 0..2 {
            if attempt > 0 {
                RiseRuntimeTimer::sleep(SUBSCRIBE_RETRY_DELAY).await;
            }
            let Ok(value) =
                engine.call_descriptor(&descriptor, &body, None, RiseWireRequestMetadata::default()).await
            else {
                continue;
            };
            if !subscribed {
                return true;
            }
            if subscribed_in_reply(&value).unwrap_or(true) {
                return true;
            }
        }
        false
    }

    pub fn decode(envelope: &RiseWirePushEnvelope) -> Option<RiseForumVoiceUpdate> {
        let data = envelope.data.data(false).ok()?;
        let Ok(RiseEvent::VoiceChannelUpdate(update)) = RiseEvent::decode(&envelope.r#type, &data) else {
            return None;
        };
        if update.chat_id.is_empty() {
            return None;
        }
        let kind = RiseForumVoiceUpdateKind::from_raw_value(&update.kind)?;
        let scope = if update.scope == rise_chat::VoiceUpdateScope::Group.wire() || update.topic_id.is_empty() {
            RiseForumVoiceUpdateScope::Group
        } else {
            RiseForumVoiceUpdateScope::Channel
        };
        let participant = update
            .participant
            .as_ref()
            .map(|participant| VoiceChannelParticipantWireDTO::from_canon(participant).to_participant());
        let user_id = if update.user_id.is_empty() {
            participant.as_ref().map(|participant| participant.user_id.clone()).unwrap_or_default()
        } else {
            update.user_id
        };
        Some(RiseForumVoiceUpdate {
            chat_id: update.chat_id,
            topic_id: update.topic_id,
            scope,
            room_id: update.room_id,
            kind,
            user_id,
            participant_count: update.participant_count.max(0),
            participant,
            occurred_at_ms: update.occurred_at_ms,
        })
    }
}

pub trait VoiceChannelParticipantWireDTOCanonExt {
    fn from_canon(canon: &rise_event_payload::VoiceChannelParticipant) -> Self;
}

impl VoiceChannelParticipantWireDTOCanonExt for VoiceChannelParticipantWireDTO {
    fn from_canon(canon: &rise_event_payload::VoiceChannelParticipant) -> Self {
        Self {
            avatar_url: canon.avatar_url.clone(),
            camera_enabled: canon.camera_enabled,
            connected: canon.connected,
            deafened: canon.deafened,
            hand_expires_at_ms: canon.hand_expires_at_ms,
            hand_raised: canon.hand_raised,
            is_official: canon.is_official,
            is_premium: canon.is_premium,
            joined_at_ms: canon.joined_at_ms,
            microphone_enabled: canon.microphone_enabled,
            name: canon.name.clone(),
            order_sequence: canon.order_sequence,
            role: canon.role.clone(),
            role_color: canon.role_color,
            role_name: canon.role_name.clone(),
            screen_share_enabled: canon.screen_share_enabled,
            server_deafened: canon.server_deafened,
            server_muted: canon.server_muted,
            tag: canon.tag.clone(),
            user_id: canon.user_id.clone(),
        }
    }
}
