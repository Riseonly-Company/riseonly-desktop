use std::collections::HashMap;

use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_canon::rise_key::rtc as rtc_key;
use rise_canon::rise_request::chat::{GetGroupVoiceChannel, UpdateGroupVoiceChannel, update_group_voice_channel};
use rise_canon::rise_response::chat::GetGroupVoiceChannel as GetGroupVoiceChannelResponse;
use rise_canon::rise_wire_methods;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::CancellationError;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatChannelKind, ChatPerm, ChatPermExt, VoiceChannelConfig,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    VoiceChannelConfigWireDTOExt, VoiceChannelStateWireDTOExt,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::rtc::engine::rise_rtc_engine_models::RiseRTCStateError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCGroupVoiceSettingsSnapshot {
    pub account_id: Option<String>,
    pub chat_id: String,
    pub room_id: Option<String>,
    pub config: VoiceChannelConfig,
    pub is_loading: bool,
    pub is_saving: bool,
    pub error_code: Option<String>,
    pub is_loaded: bool,
}

impl RiseRTCGroupVoiceSettingsSnapshot {
    const FALLBACK_AUDIO_BITRATE_KBPS: i64 = 32;

    pub fn empty(chat_id: String) -> Self {
        Self {
            account_id: None,
            chat_id,
            room_id: None,
            config: VoiceChannelConfig::defaults(ChatChannelKind::Voice).unwrap_or_else(|| VoiceChannelConfig {
                user_limit: 0,
                speaker_limit: 0,
                audio_bitrate_kbps: Self::FALLBACK_AUDIO_BITRATE_KBPS,
                video_enabled: true,
                screen_share_enabled: true,
                default_muted: false,
                region: String::new(),
            }),
            is_loading: false,
            is_saving: false,
            error_code: None,
            is_loaded: false,
        }
    }
}

pub struct RTCGroupVoiceSettingsServicesStore {
    revision: u64,
    account_generation: u64,
    snapshots: HashMap<String, RiseRTCGroupVoiceSettingsSnapshot>,
}

impl RTCGroupVoiceSettingsServicesStore {
    const SNAPSHOT_LIMIT: usize = 32;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { revision: 0, account_generation: 0, snapshots: HashMap::new() }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn account_generation(&self) -> u64 {
        self.account_generation
    }

    pub fn snapshot(&self, chat_id: &str, cx: &mut App) -> RiseRTCGroupVoiceSettingsSnapshot {
        let account_id = AuthServicesStore::shared(cx).read(cx).active_account_id().map(str::to_owned);
        self.snapshots
            .get(&Self::key(account_id.as_deref(), chat_id))
            .cloned()
            .unwrap_or_else(|| RiseRTCGroupVoiceSettingsSnapshot::empty(chat_id.to_owned()))
    }

    pub fn publish(&mut self, snapshot: RiseRTCGroupVoiceSettingsSnapshot, cx: &mut Context<Self>) {
        let active_account_id = AuthServicesStore::shared(cx).read(cx).active_account_id().map(str::to_owned);
        if snapshot.account_id != active_account_id {
            return;
        }
        let key = Self::key(snapshot.account_id.as_deref(), &snapshot.chat_id);
        self.snapshots.insert(key, snapshot);
        while self.snapshots.len() > Self::SNAPSHOT_LIMIT {
            let Some(key) = self.snapshots.keys().next().cloned() else {
                break;
            };
            self.snapshots.remove(&key);
        }
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
    }

    pub fn reset(&mut self, account_id: Option<&str>, cx: &mut Context<Self>) {
        self.account_generation = self.account_generation.wrapping_add(1);
        match account_id {
            Some(account_id) => {
                let prefix = format!("{account_id}|");
                self.snapshots.retain(|key, _| !key.starts_with(&prefix));
            }
            None => {
                self.snapshots = HashMap::new();
            }
        }
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
    }

    fn key(account_id: Option<&str>, chat_id: &str) -> String {
        format!("{}|{chat_id}", account_id.unwrap_or("anonymous"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RTCGroupVoiceSettingsContext {
    account_id: String,
    chat_id: String,
    account_generation: u64,
}

pub struct RTCGroupVoiceSettingsInteractionsStore {
    service: Entity<RTCGroupVoiceSettingsServicesStore>,
}

impl RTCGroupVoiceSettingsInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self { service: RTCGroupVoiceSettingsServicesStore::shared(cx) }
    }

    fn publish(service: &Entity<RTCGroupVoiceSettingsServicesStore>, snapshot: RiseRTCGroupVoiceSettingsSnapshot, cx: &mut App) {
        service.update(cx, |service, cx| service.publish(snapshot, cx));
    }

    fn current_snapshot(service: &Entity<RTCGroupVoiceSettingsServicesStore>, chat_id: &str, cx: &mut App) -> RiseRTCGroupVoiceSettingsSnapshot {
        service.update(cx, |service, cx| service.snapshot(chat_id, cx))
    }

    pub fn load(&mut self, raw_chat_id: &str, cx: &mut Context<Self>) -> Task<bool> {
        let Some(context) = self.context(raw_chat_id, cx) else {
            return Task::ready(true);
        };
        let service = self.service.clone();
        let current = Self::current_snapshot(&service, &context.chat_id, cx);
        Self::publish(
            &service,
            RiseRTCGroupVoiceSettingsSnapshot {
                account_id: Some(context.account_id.clone()),
                chat_id: context.chat_id.clone(),
                room_id: current.room_id.clone(),
                config: current.config.clone(),
                is_loading: true,
                is_saving: current.is_saving,
                error_code: None,
                is_loaded: current.is_loaded,
            },
            cx,
        );
        let request = GetGroupVoiceChannel::new(context.chat_id.clone());
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<GetGroupVoiceChannelResponse> = RiseConversationActionExecutor::call(
                rise_wire_methods::chat::SERVICE,
                rise_wire_methods::chat::GET_GROUP_VOICE_CHANNEL,
                &request,
                cx,
            )
            .await;
            cx.update(|cx| match result {
                Ok(response) => {
                    if !Self::is_current(&service, &context, cx) {
                        return true;
                    }
                    let channel = response.channel.as_ref();
                    tracing::warn!(
                        target: "riseonly::X1865",
                        chat_id = %context.chat_id,
                        participant_count = ?channel.and_then(|channel| channel.voice_state.as_ref()).map(|state| state.participant_count),
                        "group voice load answered"
                    );
                    let config = channel
                        .and_then(|channel| channel.voice_config.as_ref())
                        .and_then(|config| config.to_voice_channel_config(ChatChannelKind::Voice))
                        .or_else(|| VoiceChannelConfig::defaults(ChatChannelKind::Voice))
                        .unwrap_or_else(|| current.config.clone());
                    Self::publish(
                        &service,
                        RiseRTCGroupVoiceSettingsSnapshot {
                            account_id: Some(context.account_id.clone()),
                            chat_id: context.chat_id.clone(),
                            room_id: channel.map(|channel| channel.room_id.clone()),
                            config,
                            is_loading: false,
                            is_saving: false,
                            error_code: None,
                            is_loaded: true,
                        },
                        cx,
                    );
                    let state = channel
                        .and_then(|channel| channel.voice_state.as_ref())
                        .map(|state| state.to_voice_channel_state());
                    ChatActionsStore::shared(cx).update(cx, |actions, cx| {
                        actions.adopt_group_voice_state_action(&context.chat_id, state, cx);
                    });
                    true
                }
                Err(error) if error.is::<CancellationError>() => true,
                Err(error) => {
                    tracing::warn!(target: "riseonly::X1865", chat_id = %context.chat_id, %error, "group voice load failed");
                    if !Self::is_current(&service, &context, cx) {
                        return true;
                    }
                    Self::publish(
                        &service,
                        RiseRTCGroupVoiceSettingsSnapshot {
                            account_id: Some(context.account_id.clone()),
                            chat_id: context.chat_id.clone(),
                            room_id: current.room_id.clone(),
                            config: current.config.clone(),
                            is_loading: false,
                            is_saving: false,
                            error_code: Some("group_voice_load_failed".to_owned()),
                            is_loaded: current.is_loaded,
                        },
                        cx,
                    );
                    false
                }
            })
        })
    }

    pub fn save(&mut self, raw_chat_id: &str, config: VoiceChannelConfig, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        let Some(context) = self.context(raw_chat_id, cx) else {
            return Task::ready(Err(RiseRTCStateError::Validation(rtc_key::CHAT_ID.to_owned()).into()));
        };
        let can_manage = ChatServicesStore::shared(cx)
            .update(cx, |services, cx| services.can(&context.chat_id, ChatPerm::MANAGE_VOICE_CHANNEL, cx));
        if !can_manage {
            return Task::ready(Err(RiseRTCStateError::Validation("manage_voice_channel".to_owned()).into()));
        }
        let is_premium = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .is_some_and(|profile| profile.more.is_premium);
        let normalized = match config.normalized(ChatChannelKind::Voice) {
            Ok(normalized) => normalized.capped_for_author(is_premium),
            Err(error) => return Task::ready(Err(error.into())),
        };
        let service = self.service.clone();
        let current = Self::current_snapshot(&service, &context.chat_id, cx);
        Self::publish(
            &service,
            RiseRTCGroupVoiceSettingsSnapshot {
                account_id: Some(context.account_id.clone()),
                chat_id: context.chat_id.clone(),
                room_id: current.room_id.clone(),
                config: normalized.clone(),
                is_loading: false,
                is_saving: true,
                error_code: None,
                is_loaded: current.is_loaded,
            },
            cx,
        );
        let mut request = UpdateGroupVoiceChannel::new(context.chat_id.clone());
        request.voice_config = Some(update_group_voice_channel::VoiceConfig {
            audio_bitrate_kbps: Some(normalized.audio_bitrate_kbps),
            default_muted: Some(normalized.default_muted),
            region: Some(normalized.region.clone()),
            screen_share_enabled: Some(normalized.screen_share_enabled),
            speaker_limit: Some(normalized.speaker_limit),
            user_limit: Some(normalized.user_limit),
            video_enabled: Some(normalized.video_enabled),
        });
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: anyhow::Result<GetGroupVoiceChannelResponse> = RiseConversationActionExecutor::call(
                rise_wire_methods::chat::SERVICE,
                rise_wire_methods::chat::UPDATE_GROUP_VOICE_CHANNEL,
                &request,
                cx,
            )
            .await;
            cx.update(|cx| {
                let result = result.and_then(|response| {
                    if Self::is_current(&service, &context, cx) {
                        Ok(response)
                    } else {
                        Err(RiseRTCStateError::StaleAccount.into())
                    }
                });
                match result {
                    Ok(response) => {
                        let channel = response.channel.as_ref();
                        let confirmed = channel
                            .and_then(|channel| channel.voice_config.as_ref())
                            .and_then(|config| config.to_voice_channel_config(ChatChannelKind::Voice))
                            .unwrap_or(normalized);
                        Self::publish(
                            &service,
                            RiseRTCGroupVoiceSettingsSnapshot {
                                account_id: Some(context.account_id.clone()),
                                chat_id: context.chat_id.clone(),
                                room_id: channel.map(|channel| channel.room_id.clone()).or(current.room_id),
                                config: confirmed,
                                is_loading: false,
                                is_saving: false,
                                error_code: None,
                                is_loaded: true,
                            },
                            cx,
                        );
                        Ok(())
                    }
                    Err(error) => {
                        if !Self::is_current(&service, &context, cx) {
                            return Err(error);
                        }
                        Self::publish(
                            &service,
                            RiseRTCGroupVoiceSettingsSnapshot {
                                account_id: Some(context.account_id.clone()),
                                chat_id: context.chat_id.clone(),
                                room_id: current.room_id.clone(),
                                config: current.config.clone(),
                                is_loading: false,
                                is_saving: false,
                                error_code: Some("group_voice_save_failed".to_owned()),
                                is_loaded: current.is_loaded,
                            },
                            cx,
                        );
                        Err(error)
                    }
                }
            })
        })
    }

    fn context(&self, raw_chat_id: &str, cx: &mut App) -> Option<RTCGroupVoiceSettingsContext> {
        let chat_id = trimming_whitespaces_and_newlines(raw_chat_id);
        let account_id = AuthServicesStore::shared(cx).read(cx).active_account_id().map(str::to_owned)?;
        if account_id.is_empty() || chat_id.is_empty() {
            return None;
        }
        Some(RTCGroupVoiceSettingsContext {
            account_id,
            chat_id: chat_id.to_owned(),
            account_generation: self.service.read(cx).account_generation(),
        })
    }

    fn is_current(
        service: &Entity<RTCGroupVoiceSettingsServicesStore>,
        context: &RTCGroupVoiceSettingsContext,
        cx: &mut App,
    ) -> bool {
        AuthServicesStore::shared(cx).read(cx).active_account_id() == Some(context.account_id.as_str())
            && service.read(cx).account_generation() == context.account_generation
    }
}
