use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use gpui::AsyncApp;
use rise_canon::rise_event_payload::{RtcIncomingCaller, RtcPublicCall};
use rise_canon::rise_key;
use rise_canon::rise_room_event;
use rise_canon::{
    RiseEvent, RiseRoomEvent, RiseWireRequestMetadata, rise_request, rise_response, rise_view,
    rise_wire_methods,
};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::{
    character_count, character_prefix, trimming_whitespaces_and_newlines,
};
use rise_core::foundation::foundation_url::UrlComponents;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_entities::{
    RiseEntityKey, RiseEntityViewMembership, RiseEntityViewRetention, RiseEntityWindow,
    RiseEntityWindowQuery, RiseEntityWritePolicy, RiseStoreEntity,
};
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreObservation, RiseStoreSnapshot, RiseStoreSnapshotRequest,
    RiseStoreValueKey, RiseStoreVersionPrecondition, RiseStoreWriteBatch, RiseStoredValue,
    RiseStoredValueWritePolicy,
};
use rise_engine::{
    CancellationError, RiseEngine, RiseWireError, RiseWireMethodDescriptor, RiseWirePushEnvelope,
    RiseWireReplayPolicy,
};
use rise_rtc::RtcWireEnum;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::rtc::engine::core::rise_rtc_store_layout::{
    RiseRTCCallRepositorySnapshot, RiseRTCRoomMeta, RiseRTCRoomRepositorySnapshot,
    RiseRTCStoreLayout, RiseRTCStoredCall,
};
use crate::modules::rtc::engine::core::rise_rtc_wire_models::{
    RiseRTCJoinCredentialsWire, RiseRTCWireModel,
};
use crate::modules::rtc::engine::core::rise_rtcrpc::{
    RiseRTCEndpoint, RiseRTCEndpoints, RiseRTCErrorExt, RiseRTCRPC, RiseRTCStartCallRecordingBody,
};
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCAnswerOutcome, RiseRTCCallMediaMode, RiseRTCCallState, RiseRTCDirectCall, RiseRTCEvictionReason,
    RiseRTCFlexibleWireEnum, RiseRTCJoinCredentials, RiseRTCMediaEvent, RiseRTCModerationAction,
    RiseRTCParticipant, RiseRTCParticipantRole, RiseRTCRaisedHand, RiseRTCReaction,
    RiseRTCRoomConfig, RiseRTCRoomControlOutcome, RiseRTCRoomEviction, RiseRTCRoomMode,
    RiseRTCRoomSnapshot, RiseRTCStateError, RiseRTCUserProfile,
};
use crate::modules::rtc::shared::rtc_field_rules::RTCFieldRules;
use crate::modules::user::shared::contact_device_book::RiseInstallationIdentity;

#[derive(Clone)]
pub struct RiseRTCJoinResult {
    pub snapshot: RiseRTCRoomSnapshot,
    pub participant: RiseRTCParticipant,
    pub credentials: RiseRTCJoinCredentials,
}

#[derive(Clone)]
pub struct RiseRTCCallJoinResult {
    pub call: RiseRTCDirectCall,
    pub credentials: RiseRTCJoinCredentials,
}

type RiseRTCParticipantRemovalTask = Shared<BoxFuture<'static, Result<(), Arc<anyhow::Error>>>>;

struct RiseRTCRepositoryState {
    profile_by_user_id: HashMap<String, RiseRTCUserProfile>,
    profile_order: Vec<String>,
    room_refresh_generation: HashMap<String, u64>,
    media_mutation_generation: HashMap<String, u64>,
    loaded_participant_pages: HashMap<String, usize>,
    last_active_room_marker_version: i64,
    participant_removal_task: Option<RiseRTCParticipantRemovalTask>,
    participant_removal_generation: u64,
}

/// account scoped rtc source of truth, persisted data is normalized
/// into bounded store entity views and credentials never enter a batch
pub struct RiseRTCRepository {
    pub account_id: String,
    pub engine: Arc<RiseEngine>,
    pub rpc: RiseRTCRPC,
    pub page_limit: usize,
    pub participant_limit: usize,
    pub raised_hand_limit: usize,
    pub call_limit: usize,
    profile_limit: usize,
    state: RiseIsolated<RiseRTCRepositoryState>,
}

impl RiseRTCRepository {
    pub const PUSH_EVENT_TYPE: &'static str = rise_events::name::RTC_CALL;

    const TRUSTED_SESSION_PLACEHOLDER: &'static str = "rise-engine";
    const VOIP_TOKEN_TIMEOUT: Duration = Duration::from_secs(15);

    /// builds the repository with the default limits
    pub fn new(engine: Arc<RiseEngine>, account_id: String) -> Self {
        Self::new_with_limits(engine, account_id, 100, 512, 256, 128)
    }

    pub fn new_with_limits(
        engine: Arc<RiseEngine>,
        account_id: String,
        page_limit: usize,
        participant_limit: usize,
        raised_hand_limit: usize,
        call_limit: usize,
    ) -> Self {
        assert!(!account_id.is_empty());
        assert!((1..=100).contains(&page_limit));
        assert!(participant_limit >= page_limit);
        assert!(raised_hand_limit > 0 && call_limit > 0);
        Self {
            rpc: RiseRTCRPC::new(engine.clone()),
            engine,
            account_id,
            page_limit,
            participant_limit,
            raised_hand_limit,
            call_limit,
            profile_limit: 1_024,
            state: RiseIsolated::new(RiseRTCRepositoryState {
                profile_by_user_id: HashMap::new(),
                profile_order: Vec::new(),
                room_refresh_generation: HashMap::new(),
                media_mutation_generation: HashMap::new(),
                loaded_participant_pages: HashMap::new(),
                last_active_room_marker_version: 0,
                participant_removal_task: None,
                participant_removal_generation: 0,
            }),
        }
    }

    pub async fn observe_calls(
        &self,
    ) -> anyhow::Result<UnboundedReceiver<anyhow::Result<RiseRTCCallRepositorySnapshot>>> {
        let query = RiseEntityWindowQuery::new(
            RiseRTCStoreLayout::calls(),
            RiseRTCStoreLayout::RECENT_CALLS_VIEW,
            RiseEntityWindow::Latest { limit: self.call_limit },
        );
        let request = RiseStoreSnapshotRequest {
            entity_windows: HashSet::from([query.clone()]),
            ..RiseStoreSnapshotRequest::default()
        };
        let observation = self.engine.observe(request.clone()).await?;
        let engine = self.engine.clone();
        let account_id = self.account_id.clone();
        let (continuation, stream) = unbounded_channel();
        RiseAccountRuntime::current().spawn(Self::forward_observation(
            continuation,
            observation,
            engine,
            request,
            move |snapshot| Self::calls_snapshot(snapshot, &query, &account_id),
        ));
        Ok(stream)
    }

    pub async fn observe_room(
        &self,
        room_id: &str,
    ) -> anyhow::Result<UnboundedReceiver<anyhow::Result<RiseRTCRoomRepositorySnapshot>>> {
        let room_id = Self::normalized_id(room_id, rise_key::rtc::ROOM_ID)?;
        let participant_query = RiseEntityWindowQuery::new(
            RiseRTCStoreLayout::participants(),
            RiseRTCStoreLayout::participant_view(&room_id),
            RiseEntityWindow::First { limit: self.participant_limit },
        );
        let hand_query = RiseEntityWindowQuery::new(
            RiseRTCStoreLayout::raised_hands(),
            RiseRTCStoreLayout::raised_hands_view(&room_id),
            RiseEntityWindow::First { limit: self.raised_hand_limit },
        );
        let meta_key = RiseStoreValueKey::new(
            RiseRTCStoreLayout::rooms(),
            RiseRTCStoreLayout::room_meta_key(&room_id),
        );
        let room_key = RiseEntityKey::new(RiseRTCStoreLayout::rooms(), room_id.clone());
        let request = RiseStoreSnapshotRequest {
            values: HashSet::from([meta_key.clone()]),
            entities: HashSet::from([room_key.clone()]),
            entity_windows: HashSet::from([participant_query.clone(), hand_query.clone()]),
            ..RiseStoreSnapshotRequest::default()
        };
        let observation = self.engine.observe(request.clone()).await?;
        let engine = self.engine.clone();
        let account_id = self.account_id.clone();
        let (continuation, stream) = unbounded_channel();
        RiseAccountRuntime::current().spawn(Self::forward_observation(
            continuation,
            observation,
            engine,
            request,
            move |snapshot| {
                Self::room_snapshot(
                    snapshot,
                    &room_key,
                    &meta_key,
                    &participant_query,
                    &hand_query,
                    &account_id,
                )
            },
        ));
        Ok(stream)
    }

    async fn forward_observation<T, Project>(
        continuation: UnboundedSender<anyhow::Result<T>>,
        observation: RiseStoreObservation,
        engine: Arc<RiseEngine>,
        request: RiseStoreSnapshotRequest,
        project: Project,
    ) where
        T: Send + 'static,
        Project: Fn(&RiseStoreSnapshot) -> anyhow::Result<T> + Send + Sync + 'static,
    {
        let mut updates = observation.updates.make_async_iterator();
        let outcome: anyhow::Result<()> = async {
            if continuation.send(Ok(project(&observation.seed)?)).is_err() {
                return Ok(());
            }
            loop {
                let update = tokio::select! {
                    update = updates.next() => update,
                    () = continuation.closed() => return Ok(()),
                };
                if update.is_none() {
                    return Ok(());
                }
                let snapshot = engine.snapshot(request.clone()).await?;
                if continuation.send(Ok(project(&snapshot)?)).is_err() {
                    return Ok(());
                }
            }
        }
        .await;
        if let Err(error) = outcome
            && !error.is::<CancellationError>()
        {
            let _ = continuation.send(Err(error));
        }
    }

    pub async fn cleanup_stale_room_cache(&self) {
        let Some(value) = self
            .engine
            .value(RiseRTCStoreLayout::rooms(), RiseRTCStoreLayout::ACTIVE_ROOM_MARKER_KEY.to_owned())
            .await
            .ok()
            .flatten()
        else {
            return;
        };
        let Ok(room_id) = RiseRTCStoreLayout::decode::<String>(&value.data) else {
            return;
        };
        let _ = self.release_room_cache(&room_id).await;
    }

    pub async fn mark_room_active(&self, room_id: &str) -> anyhow::Result<()> {
        let room_id = Self::normalized_id(room_id, rise_key::rtc::ROOM_ID)?;
        let now = Self::now_milliseconds();
        let wall_version = RiseRTCStoreLayout::timestamp_version(now, 0).max(1);
        let marker_version = self.state.with(|state| {
            let next_version = if state.last_active_room_marker_version == i64::MAX {
                i64::MAX
            } else {
                state.last_active_room_marker_version + 1
            };
            let marker_version = wall_version.max(next_version);
            state.last_active_room_marker_version = marker_version;
            marker_version
        });
        self.engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::SetValue {
                    namespace: RiseRTCStoreLayout::rooms(),
                    key: RiseRTCStoreLayout::ACTIVE_ROOM_MARKER_KEY.to_owned(),
                    value: RiseStoredValue::new(
                        RiseRTCStoreLayout::encode(&room_id)?,
                        marker_version,
                        now,
                    ),
                    precondition: RiseStoreVersionPrecondition::Any,
                    policy: RiseStoredValueWritePolicy::ServerAuthoritative,
                }],
                HashSet::new(),
            ))
            .await?;
        Ok(())
    }

    pub async fn release_room_cache(&self, room_id: &str) -> anyhow::Result<()> {
        let room_id = Self::normalized_id(room_id, rise_key::rtc::ROOM_ID)?;
        self.engine
            .write(RiseStoreWriteBatch::new(
                vec![
                    RiseStoreMutation::RemoveAllEntityViewMemberships {
                        namespace: RiseRTCStoreLayout::participants(),
                        view: RiseRTCStoreLayout::participant_view(&room_id),
                        if_version_at_most: None,
                    },
                    RiseStoreMutation::RemoveAllEntityViewMemberships {
                        namespace: RiseRTCStoreLayout::raised_hands(),
                        view: RiseRTCStoreLayout::raised_hands_view(&room_id),
                        if_version_at_most: None,
                    },
                    RiseStoreMutation::RemoveValue {
                        namespace: RiseRTCStoreLayout::rooms(),
                        key: RiseRTCStoreLayout::room_meta_key(&room_id),
                        precondition: RiseStoreVersionPrecondition::Any,
                    },
                    RiseStoreMutation::PruneUnreferencedEntities {
                        namespace: RiseRTCStoreLayout::participants(),
                        limit: self.participant_limit as i64,
                    },
                    RiseStoreMutation::PruneUnreferencedEntities {
                        namespace: RiseRTCStoreLayout::raised_hands(),
                        limit: self.raised_hand_limit as i64,
                    },
                    RiseStoreMutation::PruneUnreferencedEntities {
                        namespace: RiseRTCStoreLayout::rooms(),
                        limit: 64,
                    },
                ],
                HashSet::new(),
            ))
            .await?;
        self.release_active_room_marker(&room_id).await
    }

    async fn release_active_room_marker(&self, if_owned_by: &str) -> anyhow::Result<()> {
        let Some(marker) = self
            .engine
            .value(RiseRTCStoreLayout::rooms(), RiseRTCStoreLayout::ACTIVE_ROOM_MARKER_KEY.to_owned())
            .await?
        else {
            return Ok(());
        };
        let Ok(owned_room_id) = RiseRTCStoreLayout::decode::<String>(&marker.data) else {
            return Ok(());
        };
        if owned_room_id != if_owned_by {
            return Ok(());
        }
        self.engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::RemoveValue {
                    namespace: RiseRTCStoreLayout::rooms(),
                    key: RiseRTCStoreLayout::ACTIVE_ROOM_MARKER_KEY.to_owned(),
                    precondition: RiseStoreVersionPrecondition::Equals(marker.version),
                }],
                HashSet::new(),
            ))
            .await?;
        Ok(())
    }

    pub async fn join_room(
        &self,
        room_id: &str,
        display_name: Option<&str>,
        preferred_role: Option<RiseRTCParticipantRole>,
    ) -> anyhow::Result<RiseRTCJoinResult> {
        let room_id = Self::normalized_id(room_id, rise_key::rtc::ROOM_ID)?;
        self.state.with(|state| state.loaded_participant_pages.remove(&room_id));
        Self::log(format!("join rpc start room={room_id}")).await;
        let response = match self
            .rpc
            .call::<_, rise_response::rtc::JoinRoom>(
                RiseRTCEndpoints::join_room(),
                rise_request::rtc::JoinRoom {
                    display_name: Self::normalized_optional(display_name),
                    preferred_role: preferred_role.and_then(|role| {
                        rise_request::rtc::join_room::PreferredRole::ALL_CASES
                            .iter()
                            .copied()
                            .find(|candidate| candidate.raw_value() == role.wire())
                    }),
                    room_id: Some(room_id.clone()),
                    session_id: Some(Self::TRUSTED_SESSION_PLACEHOLDER.to_owned()),
                },
            )
            .await
        {
            Ok(response) => response,
            Err(error) => {
                Self::log(format!(
                    "join rpc/decode failed room={room_id} {}",
                    Self::error_summary(&error)
                ))
                .await;
                return Err(error);
            }
        };
        let (Some(snapshot), Some(participant), Some(wire_credentials)) = (
            response.snapshot.as_ref().and_then(RiseRTCWireModel::model),
            response.participant.as_ref().map(RiseRTCWireModel::model),
            response.credentials.as_ref(),
        ) else {
            Self::log(format!("join malformed response room={room_id}")).await;
            return Err(RiseRTCStateError::MalformedResponse("join_room".to_owned()).into());
        };
        let credentials = wire_credentials.credentials(
            &room_id,
            &snapshot.room.livekit_room_name,
            snapshot.room.mode,
            None,
            Some(snapshot.room.audio_bitrate_kbps),
        );
        Self::log(format!(
            "join decoded room={room_id} endpoint={}",
            Self::sanitized_endpoint(&credentials.server_url)
        ))
        .await;
        let stored: anyhow::Result<()> = async {
            self.store_authoritative_snapshot(
                &snapshot,
                snapshot.participants.len() >= self.page_limit,
                None,
            )
            .await?;
            self.store_participant(&participant, &room_id).await
        }
        .await;
        if let Err(error) = stored {
            Self::log(format!(
                "join snapshot store failed room={room_id} {}",
                Self::error_summary(&error)
            ))
            .await;
            let _ = self.leave_room(&credentials).await;
            return Err(error);
        }
        Self::log(format!("join snapshot stored room={room_id}")).await;
        Ok(RiseRTCJoinResult { snapshot, participant, credentials })
    }

    pub async fn leave_room(&self, credentials: &RiseRTCJoinCredentials) -> anyhow::Result<()> {
        if credentials.room_mode == RiseRTCRoomMode::DirectCall {
            Self::log(format!(
                "leave_room suppressed for direct call room={}",
                credentials.room_id
            ))
            .await;
            return Ok(());
        }
        self.rpc
            .call::<_, rise_response::rtc::LeaveRoom>(
                RiseRTCEndpoints::leave_room(),
                rise_request::rtc::LeaveRoom {
                    connection_id: Some(credentials.connection_id.clone()),
                    room_id: Some(credentials.room_id.clone()),
                    session_id: Some(Self::TRUSTED_SESSION_PLACEHOLDER.to_owned()),
                },
            )
            .await?;
        Ok(())
    }

    pub async fn heartbeat(&self, credentials: &RiseRTCJoinCredentials) -> anyhow::Result<i64> {
        let response = self
            .rpc
            .call::<_, rise_response::rtc::Heartbeat>(
                RiseRTCEndpoints::heartbeat(),
                rise_request::rtc::Heartbeat {
                    connection_id: Some(credentials.connection_id.clone()),
                    room_id: Some(credentials.room_id.clone()),
                    session_id: Some(Self::TRUSTED_SESSION_PLACEHOLDER.to_owned()),
                },
            )
            .await?;
        Ok(response.lease_expires_at_ms)
    }

    /// false when roster changes kept invalidating the read and nothing was stored
    /// caller must schedule another pass since only livekit join and leave reschedule on their own
    pub async fn refresh_room(&self, room_id: &str) -> anyhow::Result<bool> {
        let room_id = Self::normalized_id(room_id, rise_key::rtc::ROOM_ID)?;
        let (generation, mut media_generation) = self.state.with(|state| {
            let generation = state
                .room_refresh_generation
                .get(&room_id)
                .copied()
                .unwrap_or(0)
                .wrapping_add(1);
            state.room_refresh_generation.insert(room_id.clone(), generation);
            (generation, Self::media_generation(state, &room_id))
        });
        let mut response = self.request_room_snapshot(&room_id).await?;
        let (is_current, latest_media_generation) = self.state.with(|state| {
            (
                state.room_refresh_generation.get(&room_id).copied() == Some(generation),
                Self::media_generation(state, &room_id),
            )
        });
        if !is_current {
            return Ok(true);
        }
        if latest_media_generation != media_generation {
            media_generation = latest_media_generation;
            response = self.request_room_snapshot(&room_id).await?;
            let (is_current, latest_media_generation) = self.state.with(|state| {
                (
                    state.room_refresh_generation.get(&room_id).copied() == Some(generation),
                    Self::media_generation(state, &room_id),
                )
            });
            if !is_current {
                return Ok(true);
            }
            if latest_media_generation != media_generation {
                return Ok(false);
            }
        }
        let Some(snapshot) = response.snapshot.as_ref().and_then(RiseRTCWireModel::model) else {
            return Err(RiseRTCStateError::MalformedResponse("get_room_snapshot".to_owned()).into());
        };
        self.store_authoritative_snapshot(
            &snapshot,
            response.has_more_participants,
            response.next_participant_cursor.clone(),
        )
        .await?;
        let loaded_pages = self
            .state
            .with(|state| state.loaded_participant_pages.get(&room_id).copied().unwrap_or(0));
        for _ in 0..loaded_pages {
            let is_current = self.state.with(|state| {
                state.room_refresh_generation.get(&room_id).copied() == Some(generation)
            });
            if !is_current || !self.append_next_participant_page(&room_id).await? {
                break;
            }
        }
        Ok(true)
    }

    async fn request_room_snapshot(
        &self,
        room_id: &str,
    ) -> anyhow::Result<rise_response::rtc::GetRoomSnapshot> {
        self.rpc
            .call::<_, rise_response::rtc::GetRoomSnapshot>(
                RiseRTCEndpoints::get_room_snapshot(),
                rise_request::rtc::GetRoomSnapshot {
                    participant_cursor: None,
                    participant_limit: Some(self.page_limit as i64),
                    room_id: Some(room_id.to_owned()),
                },
            )
            .await
    }

    pub async fn load_more_participants(&self, room_id: &str) -> anyhow::Result<()> {
        let room_id = Self::normalized_id(room_id, rise_key::rtc::ROOM_ID)?;
        if self.append_next_participant_page(&room_id).await? {
            self.state.with(|state| {
                *state.loaded_participant_pages.entry(room_id.clone()).or_insert(0) += 1;
            });
        }
        Ok(())
    }

    /// true when a page after the stored cursor was appended
    async fn append_next_participant_page(&self, room_id: &str) -> anyhow::Result<bool> {
        let room_id = room_id.to_owned();
        let Some(meta) = self.room_meta(&room_id).await? else {
            return Ok(false);
        };
        if !meta.has_more_participants {
            return Ok(false);
        }
        let Some(cursor) = Self::normalized_optional(meta.next_participant_cursor.as_deref())
        else {
            return Ok(false);
        };
        let media_generation = self.state.with(|state| Self::media_generation(state, &room_id));
        let response = self
            .rpc
            .call::<_, rise_response::rtc::ListParticipants>(
                RiseRTCEndpoints::list_participants(),
                rise_request::rtc::ListParticipants {
                    cursor: Some(cursor),
                    limit: Some(self.page_limit as i64),
                    room_id: Some(room_id.clone()),
                },
            )
            .await?;
        if self.state.with(|state| Self::media_generation(state, &room_id)) != media_generation {
            return Ok(false);
        }
        self.append_participants(
            response.participants.iter().map(RiseRTCWireModel::model).collect(),
            &room_id,
            &meta,
            response.has_more,
            response.next_cursor.clone(),
            response.profiles_complete,
            response.profile_error.clone(),
        )
        .await?;
        Ok(true)
    }

    pub async fn list_channel_participants(
        &self,
        room_id: &str,
        cursor: Option<String>,
        limit: i64,
    ) -> anyhow::Result<(Vec<RiseRTCParticipant>, bool)> {
        let room_id = Self::normalized_id(room_id, rise_key::rtc::ROOM_ID)?;
        let response = self
            .rpc
            .call::<_, rise_response::rtc::ListParticipants>(
                RiseRTCEndpoints::list_participants(),
                rise_request::rtc::ListParticipants {
                    cursor,
                    limit: Some(limit.max(1).min(self.page_limit as i64)),
                    room_id: Some(room_id),
                },
            )
            .await?;
        Ok((response.participants.iter().map(RiseRTCWireModel::model).collect(), response.has_more))
    }

    pub async fn refresh_raised_hands(&self, room_id: &str) -> anyhow::Result<()> {
        let room_id = Self::normalized_id(room_id, rise_key::rtc::ROOM_ID)?;
        let media_generation = self.state.with(|state| Self::media_generation(state, &room_id));
        let response = self
            .rpc
            .call::<_, rise_response::rtc::ListRaisedHands>(
                RiseRTCEndpoints::list_raised_hands(),
                rise_request::rtc::ListRaisedHands {
                    cursor: None,
                    limit: Some(self.page_limit.min(self.raised_hand_limit) as i64),
                    room_id: Some(room_id.clone()),
                },
            )
            .await?;
        if self.state.with(|state| Self::media_generation(state, &room_id)) != media_generation {
            return Ok(());
        }
        self.replace_raised_hands(
            response.raised_hands.iter().map(RiseRTCWireModel::model).collect(),
            &room_id,
            response.has_more,
            response.next_cursor.clone(),
        )
        .await
    }

    pub async fn update_self_media(
        &self,
        credentials: &RiseRTCJoinCredentials,
        microphone: Option<bool>,
        camera: Option<bool>,
        screen_share: Option<bool>,
        deafened: Option<bool>,
    ) -> anyhow::Result<RiseRTCParticipant> {
        if microphone.is_none() && camera.is_none() && screen_share.is_none() && deafened.is_none()
        {
            return Err(RiseRTCStateError::Validation("empty_media_patch".to_owned()).into());
        }
        let response = self
            .rpc
            .call::<_, rise_response::rtc::UpdateSelfMedia>(
                RiseRTCEndpoints::update_self_media(),
                rise_request::rtc::UpdateSelfMedia {
                    camera_enabled: camera,
                    connection_id: Some(credentials.connection_id.clone()),
                    deafened,
                    microphone_enabled: microphone,
                    room_id: Some(credentials.room_id.clone()),
                    screen_share_enabled: screen_share,
                    session_id: Some(Self::TRUSTED_SESSION_PLACEHOLDER.to_owned()),
                },
            )
            .await?;
        let Some(participant) = response.participant.as_ref().map(RiseRTCWireModel::model) else {
            return Err(RiseRTCStateError::MalformedResponse("update_self_media".to_owned()).into());
        };
        self.store_participant(&participant, &credentials.room_id).await?;
        Ok(participant)
    }

    pub async fn request_to_speak(
        &self,
        credentials: &RiseRTCJoinCredentials,
        note: Option<&str>,
    ) -> anyhow::Result<RiseRTCRaisedHand> {
        let response = self
            .rpc
            .call::<_, rise_response::rtc::RequestToSpeak>(
                RiseRTCEndpoints::request_to_speak(),
                rise_request::rtc::RequestToSpeak {
                    connection_id: Some(credentials.connection_id.clone()),
                    note: Self::normalized_optional(note),
                    room_id: Some(credentials.room_id.clone()),
                    session_id: Some(Self::TRUSTED_SESSION_PLACEHOLDER.to_owned()),
                },
            )
            .await?;
        let Some(hand) = response.raised_hand.as_ref().map(RiseRTCWireModel::model) else {
            return Err(RiseRTCStateError::MalformedResponse("request_to_speak".to_owned()).into());
        };
        self.store_raised_hand(&hand, &credentials.room_id).await?;
        Ok(hand)
    }

    pub async fn cancel_request_to_speak(
        &self,
        credentials: &RiseRTCJoinCredentials,
    ) -> anyhow::Result<()> {
        self.rpc
            .call::<_, rise_response::rtc::CancelRequestToSpeak>(
                RiseRTCEndpoints::cancel_request_to_speak(),
                rise_request::rtc::CancelRequestToSpeak {
                    connection_id: Some(credentials.connection_id.clone()),
                    room_id: Some(credentials.room_id.clone()),
                    session_id: Some(Self::TRUSTED_SESSION_PLACEHOLDER.to_owned()),
                },
            )
            .await?;
        self.remove_raised_hand(&self.account_id, &credentials.room_id).await
    }

    pub async fn change_role(
        &self,
        room_id: &str,
        user_id: &str,
        role: RiseRTCParticipantRole,
    ) -> anyhow::Result<RiseRTCParticipant> {
        let target_room_id = Self::normalized_id(room_id, rise_key::rtc::ROOM_ID)?;
        let target_user_id = Self::normalized_id(user_id, rise_key::rtc::TARGET_USER_ID)?;
        let (endpoint, response): (&RiseRTCEndpoint, rise_response::rtc::PromoteToSpeaker) =
            match role {
                RiseRTCParticipantRole::Speaker | RiseRTCParticipantRole::Moderator => {
                    let endpoint = RiseRTCEndpoints::promote_to_speaker();
                    let response = self
                        .rpc
                        .call::<_, rise_response::rtc::PromoteToSpeaker>(
                            endpoint,
                            rise_request::rtc::PromoteToSpeaker {
                                room_id: Some(target_room_id),
                                target_user_id: Some(target_user_id),
                            },
                        )
                        .await?;
                    (endpoint, response)
                }
                RiseRTCParticipantRole::Listener => {
                    let endpoint = RiseRTCEndpoints::move_to_audience();
                    let response = self
                        .rpc
                        .call::<_, rise_response::rtc::MoveToAudience>(
                            endpoint,
                            rise_request::rtc::MoveToAudience {
                                room_id: Some(target_room_id),
                                target_user_id: Some(target_user_id),
                            },
                        )
                        .await?;
                    (endpoint, response)
                }
                RiseRTCParticipantRole::Unspecified => {
                    return Err(RiseRTCStateError::Validation(
                        "invalid_participant_role".to_owned(),
                    )
                    .into());
                }
            };
        let Some(participant) = response.participant.as_ref().map(RiseRTCWireModel::model) else {
            return Err(RiseRTCStateError::MalformedResponse(endpoint.method.clone()).into());
        };
        self.store_participant(&participant, room_id).await?;
        Ok(participant)
    }

    pub async fn moderate(
        &self,
        room_id: &str,
        user_id: &str,
        action: RiseRTCModerationAction,
        reason: Option<&str>,
    ) -> anyhow::Result<Option<RiseRTCParticipant>> {
        if action == RiseRTCModerationAction::Unspecified {
            return Err(RiseRTCStateError::Validation("invalid_moderation_action".to_owned()).into());
        }
        let room_id = Self::normalized_id(room_id, rise_key::rtc::ROOM_ID)?;
        let user_id = Self::normalized_id(user_id, rise_key::rtc::TARGET_USER_ID)?;
        let normalized_action = RiseRTCModerationAction::normalized(action.wire());
        let response = self
            .rpc
            .call::<_, rise_response::rtc::ModerateParticipant>(
                RiseRTCEndpoints::moderate_participant(),
                rise_request::rtc::ModerateParticipant {
                    action: rise_request::rtc::moderate_participant::Action::ALL_CASES
                        .iter()
                        .copied()
                        .find(|candidate| candidate.raw_value() == normalized_action),
                    reason: Self::bounded_optional(
                        reason,
                        RTCFieldRules::moderation_reason().maximum().unwrap_or(0),
                    ),
                    room_id: Some(room_id.clone()),
                    target_user_id: Some(user_id.clone()),
                },
            )
            .await?;
        if action == RiseRTCModerationAction::Kick {
            self.remove_participant(&user_id, &room_id).await?;
            return Ok(None);
        }
        let participant = response.participant.as_ref().map(RiseRTCWireModel::model);
        if let Some(participant) = &participant {
            self.store_participant(participant, &room_id).await?;
        }
        Ok(participant)
    }

    pub async fn send_reaction(
        &self,
        credentials: &RiseRTCJoinCredentials,
        emoji: &str,
        client_event_id: &str,
    ) -> anyhow::Result<RiseRTCReaction> {
        let emoji = trimming_whitespaces_and_newlines(emoji).to_owned();
        if !(1..=16).contains(&character_count(&emoji)) {
            return Err(RiseRTCStateError::Validation("invalid_reaction".to_owned()).into());
        }
        let response = self
            .rpc
            .call::<_, rise_response::rtc::SendReaction>(
                RiseRTCEndpoints::send_reaction(),
                rise_request::rtc::SendReaction {
                    client_event_id: Some(client_event_id.to_owned()),
                    connection_id: Some(credentials.connection_id.clone()),
                    emoji: Some(emoji.clone()),
                    room_id: Some(credentials.room_id.clone()),
                    session_id: Some(Self::TRUSTED_SESSION_PLACEHOLDER.to_owned()),
                },
            )
            .await?;
        Ok(RiseRTCReaction {
            event_id: response.event_id,
            room_id: credentials.room_id.clone(),
            actor_user_id: self.account_id.clone(),
            emoji,
            created_at_ms: response.created_at_ms,
        })
    }

    pub async fn start_direct_call(
        &self,
        callee_id: &str,
        chat_id: &str,
        media_mode: RiseRTCCallMediaMode,
        idempotency_key: &str,
        e2ee_public_key: Option<String>,
    ) -> anyhow::Result<RiseRTCCallJoinResult> {
        if media_mode != RiseRTCCallMediaMode::Audio && media_mode != RiseRTCCallMediaMode::Video {
            return Err(RiseRTCStateError::Validation("invalid_call_media_mode".to_owned()).into());
        }
        let installation_id = Self::current_installation_id();
        let request = rise_request::rtc::StartDirectCall {
            callee_user_id: Some(Self::normalized_id(callee_id, rise_key::rtc::CALLEE_USER_ID)?),
            chat_id: Some(Self::normalized_id(chat_id, rise_key::rtc::CHAT_ID)?),
            e2ee_public_key,
            idempotency_key: Some(Self::safe_key(idempotency_key, rise_key::rtc::IDEMPOTENCY_KEY)?),
            installation_id,
            media_mode: rise_request::rtc::start_direct_call::MediaMode::ALL_CASES
                .iter()
                .copied()
                .find(|candidate| candidate.raw_value() == media_mode.wire()),
            session_id: Some(Self::TRUSTED_SESSION_PLACEHOLDER.to_owned()),
        };
        let response = self
            .rpc
            .call::<_, rise_response::rtc::StartDirectCall>(
                RiseRTCEndpoints::start_direct_call(),
                request,
            )
            .await?;
        let (Some(call), Some(wire_credentials)) = (
            response.call.as_ref().map(RiseRTCWireModel::model),
            response.credentials.as_ref(),
        ) else {
            return Err(RiseRTCStateError::MalformedResponse("start_direct_call".to_owned()).into());
        };
        self.store_call(&call, None, None, None).await?;
        let credentials = wire_credentials.credentials(
            &call.room_id,
            &call.room_id,
            RiseRTCRoomMode::DirectCall,
            Some(call.media_mode),
            None,
        );
        Ok(RiseRTCCallJoinResult { call, credentials })
    }

    pub async fn answer_call(
        &self,
        call_id: &str,
        display_name: Option<&str>,
        installation_id: &str,
        e2ee_public_key: Option<String>,
    ) -> anyhow::Result<(RiseRTCAnswerOutcome, Option<RiseRTCJoinCredentials>)> {
        let request = rise_request::rtc::AnswerCall {
            call_id: Some(Self::normalized_id(call_id, rise_key::rtc::CALL_ID)?),
            display_name: Self::normalized_optional(display_name),
            e2ee_public_key,
            installation_id: Some(Self::safe_key(installation_id, rise_key::rtc::INSTALLATION_ID)?),
            session_id: Some(Self::TRUSTED_SESSION_PLACEHOLDER.to_owned()),
        };
        let response = self
            .rpc
            .call::<_, rise_response::rtc::AnswerCall>(RiseRTCEndpoints::answer_call(), request)
            .await?;
        let answered = response.call.as_ref().map(RiseRTCWireModel::model);
        if let Some(answered) = &answered {
            self.store_call(answered, None, None, None).await?;
        }
        if response.answered_elsewhere {
            return Ok((RiseRTCAnswerOutcome::AnsweredElsewhere(answered), None));
        }
        let (Some(call), Some(wire_credentials)) = (answered, response.credentials.as_ref()) else {
            return Err(RiseRTCStateError::MalformedResponse("answer_call".to_owned()).into());
        };
        let credentials = wire_credentials.credentials(
            &call.room_id,
            &call.room_id,
            RiseRTCRoomMode::DirectCall,
            Some(call.media_mode),
            None,
        );
        Ok((RiseRTCAnswerOutcome::Answered(call), Some(credentials)))
    }

    pub async fn rejoin_call(
        &self,
        call_id: &str,
        installation_id: Option<&str>,
    ) -> anyhow::Result<RiseRTCCallJoinResult> {
        let request = rise_request::rtc::RejoinCall {
            call_id: Some(Self::normalized_id(call_id, rise_key::rtc::CALL_ID)?),
            installation_id: installation_id
                .map(|value| Self::safe_key(value, rise_key::rtc::INSTALLATION_ID))
                .transpose()?,
            session_id: Some(Self::TRUSTED_SESSION_PLACEHOLDER.to_owned()),
        };
        let response = self
            .rpc
            .call::<_, rise_response::rtc::RejoinCall>(RiseRTCEndpoints::rejoin_call(), request)
            .await?;
        let (Some(call), Some(wire_credentials)) = (
            response.call.as_ref().map(RiseRTCWireModel::model),
            response.credentials.as_ref(),
        ) else {
            return Err(RiseRTCStateError::MalformedResponse("rejoin_call".to_owned()).into());
        };
        self.store_call(&call, None, None, None).await?;
        let credentials = wire_credentials.credentials(
            &call.room_id,
            &call.room_id,
            RiseRTCRoomMode::DirectCall,
            Some(call.media_mode),
            None,
        );
        Ok(RiseRTCCallJoinResult { call, credentials })
    }

    pub async fn call_action(
        &self,
        endpoint: &RiseRTCEndpoint,
        call_id: &str,
        idempotency_key: &str,
        reason: Option<&str>,
    ) -> anyhow::Result<RiseRTCDirectCall> {
        let decline_method = &RiseRTCEndpoints::decline_call().method;
        let cancel_method = &RiseRTCEndpoints::cancel_call().method;
        let end_method = &RiseRTCEndpoints::end_call().method;
        assert!([decline_method, cancel_method, end_method].contains(&&endpoint.method));
        let call_id = Self::normalized_id(call_id, rise_key::rtc::CALL_ID)?;
        let key = Self::safe_key(idempotency_key, rise_key::rtc::IDEMPOTENCY_KEY)?;
        let reason = Self::bounded_optional(
            reason,
            RTCFieldRules::moderation_reason().maximum().unwrap_or(0),
        );
        let installation_id = Self::current_installation_id();
        let response: rise_view::rtc::CallCurrent = if &endpoint.method == decline_method {
            self.rpc
                .call::<_, rise_response::rtc::DeclineCall>(
                    endpoint,
                    rise_request::rtc::DeclineCall {
                        call_id: Some(call_id),
                        idempotency_key: Some(key),
                        installation_id,
                        reason,
                    },
                )
                .await?
        } else if &endpoint.method == cancel_method {
            self.rpc
                .call::<_, rise_response::rtc::CancelCall>(
                    endpoint,
                    rise_request::rtc::CancelCall {
                        call_id: Some(call_id),
                        idempotency_key: Some(key),
                        installation_id,
                        reason,
                    },
                )
                .await?
        } else {
            self.rpc
                .call::<_, rise_response::rtc::EndCall>(
                    endpoint,
                    rise_request::rtc::EndCall {
                        call_id: Some(call_id),
                        idempotency_key: Some(key),
                        installation_id,
                        reason,
                    },
                )
                .await?
        };
        let Some(call) = response.call.as_ref().map(RiseRTCWireModel::model) else {
            return Err(RiseRTCStateError::MalformedResponse(endpoint.method.clone()).into());
        };
        self.store_call(&call, None, None, None).await?;
        Ok(call)
    }

    /// call recording start or stop (premium audit)
    /// tier gate lives in rtc-service, a refusal carries upgrade_tier
    /// and the relay opens the paywall so this only shapes the request
    pub async fn call_recording(
        &self,
        start: bool,
        call_id: &str,
        idempotency_key: &str,
    ) -> anyhow::Result<RiseRTCDirectCall> {
        let call_id = Self::normalized_id(call_id, rise_key::rtc::CALL_ID)?;
        let key = Self::safe_key(idempotency_key, rise_key::rtc::IDEMPOTENCY_KEY)?;
        let response: rise_view::rtc::CallCurrent = if start {
            self.rpc
                .call::<_, rise_view::rtc::CallCurrent>(
                    RiseRTCEndpoints::start_call_recording(),
                    RiseRTCStartCallRecordingBody { call_id, idempotency_key: key },
                )
                .await?
        } else {
            self.rpc
                .call::<_, rise_response::rtc::StopCallRecording>(
                    RiseRTCEndpoints::stop_call_recording(),
                    rise_request::rtc::StopCallRecording {
                        call_id: Some(call_id),
                        idempotency_key: Some(key),
                    },
                )
                .await?
        };
        let Some(call) = response.call.as_ref().map(RiseRTCWireModel::model) else {
            let method = if start { "start_call_recording" } else { "stop_call_recording" };
            return Err(RiseRTCStateError::MalformedResponse(method.to_owned()).into());
        };
        self.store_call(&call, None, None, None).await?;
        Ok(call)
    }

    pub async fn get_call(&self, call_id: &str) -> anyhow::Result<RiseRTCDirectCall> {
        let request = rise_request::rtc::GetCall {
            call_id: Some(Self::normalized_id(call_id, rise_key::rtc::CALL_ID)?),
        };
        let response = self
            .rpc
            .call::<_, rise_response::rtc::GetCall>(RiseRTCEndpoints::get_call(), request)
            .await?;
        let Some(call) = response.call.as_ref().map(RiseRTCWireModel::model) else {
            return Err(RiseRTCStateError::MalformedResponse("get_call".to_owned()).into());
        };
        self.store_call(&call, None, None, None).await?;
        Ok(call)
    }

    pub async fn accept_system_call(&self, call_id: &str, caller: Option<RiseRTCUserProfile>) {
        let Ok(call) = self.get_call(call_id).await else {
            return;
        };
        let _ = self.store_call(&call, caller, None, None).await;
    }

    pub async fn register_voip_token(
        &self,
        token: &str,
        environment: &str,
        ownership_generation: i64,
        installation_id: &str,
    ) -> anyhow::Result<()> {
        let descriptor = RiseWireMethodDescriptor::new(
            rise_wire_methods::notify::SERVICE,
            rise_wire_methods::notify::REGISTER_VOIP_DEVICE_TOKEN,
        )
        .with_timeout(Self::VOIP_TOKEN_TIMEOUT)
        .with_replay_policy(RiseWireReplayPolicy::Never);
        let request = rise_request::notify::RegisterVoipDeviceToken {
            device_token: Some(token.to_owned()),
            environment: rise_request::notify::register_voip_device_token::Environment::ALL_CASES
                .iter()
                .copied()
                .find(|candidate| candidate.raw_value() == environment),
            installation_id: Some(installation_id.to_owned()),
            ownership_generation: Some(ownership_generation),
        };
        self.engine
            .call_descriptor_response::<_, rise_response::notify::RegisterVoipDeviceToken>(
                &descriptor,
                &request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await?;
        Ok(())
    }

    pub async fn revoke_voip_token(&self, token: &str, environment: &str) -> anyhow::Result<()> {
        let descriptor = RiseWireMethodDescriptor::new(
            rise_wire_methods::notify::SERVICE,
            rise_wire_methods::notify::REVOKE_VOIP_DEVICE_TOKEN,
        )
        .with_timeout(Self::VOIP_TOKEN_TIMEOUT)
        .with_replay_policy(RiseWireReplayPolicy::Never);
        let request = rise_request::notify::RevokeVoipDeviceToken {
            device_token: Some(token.to_owned()),
            environment: rise_request::notify::revoke_voip_device_token::Environment::ALL_CASES
                .iter()
                .copied()
                .find(|candidate| candidate.raw_value() == environment),
        };
        self.engine
            .call_descriptor_response::<_, rise_response::notify::RevokeVoipDeviceToken>(
                &descriptor,
                &request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await?;
        Ok(())
    }

    pub async fn accept_media_event(
        &self,
        event: &RiseRTCMediaEvent,
    ) -> anyhow::Result<Option<RiseRTCRoomControlOutcome>> {
        match event {
            RiseRTCMediaEvent::ParticipantJoined { session, .. } => {
                self.bump_media_generation(&session.room_id);
                Ok(None)
            }
            RiseRTCMediaEvent::ParticipantLeft { session, user_id, .. } => {
                self.bump_media_generation(&session.room_id);
                self.remove_participant(user_id, &session.room_id).await?;
                Ok(None)
            }
            RiseRTCMediaEvent::Data { session, payload, .. } => {
                self.bump_media_generation(&session.room_id);
                self.reduce_room_control_payload(payload, &session.room_id).await
            }
            RiseRTCMediaEvent::Connected { .. }
            | RiseRTCMediaEvent::Reconnecting { .. }
            | RiseRTCMediaEvent::Reconnected { .. }
            | RiseRTCMediaEvent::Disconnected { .. }
            | RiseRTCMediaEvent::Failure { .. }
            | RiseRTCMediaEvent::LocalScreenShareEnded { .. }
            | RiseRTCMediaEvent::LocalBroadcastStarted { .. }
            | RiseRTCMediaEvent::LocalCameraPaused { .. }
            | RiseRTCMediaEvent::LocalCameraResumed { .. }
            | RiseRTCMediaEvent::LocalCameraResumeRequested { .. } => Ok(None),
        }
    }

    fn bump_media_generation(&self, room_id: &str) {
        self.state.with(|state| {
            let generation = state.media_mutation_generation.entry(room_id.to_owned()).or_insert(0);
            *generation = generation.wrapping_add(1);
        });
    }

    fn media_generation(state: &RiseRTCRepositoryState, room_id: &str) -> u64 {
        state.media_mutation_generation.get(room_id).copied().unwrap_or(0)
    }

    async fn store_authoritative_snapshot(
        &self,
        snapshot: &RiseRTCRoomSnapshot,
        has_more_participants: bool,
        next_participant_cursor: Option<String>,
    ) -> anyhow::Result<()> {
        let room_id = snapshot.room.room_id.clone();
        let snapshot_version = RiseRTCStoreLayout::room_version(&snapshot.room)
            .max(RiseRTCStoreLayout::timestamp_version(snapshot.generated_at_ms, 0))
            .max(1);
        let participants: Vec<RiseRTCParticipant> = snapshot
            .participants
            .iter()
            .take(self.participant_limit)
            .map(|participant| self.retaining_profile_participant(participant))
            .collect();
        tracing::warn!(
            target: "riseonly::X2163",
            room = %room_id,
            count = snapshot.participant_count,
            rows = ?participants.iter().map(|participant| (participant.user_id.as_str(), &participant.role)).collect::<Vec<_>>(),
            "rtc roster snapshot"
        );
        let hands: Vec<RiseRTCRaisedHand> = snapshot
            .raised_hands
            .iter()
            .take(self.raised_hand_limit)
            .map(|hand| self.retaining_profile_hand(hand))
            .collect();
        let meta = RiseRTCRoomMeta {
            participant_count: snapshot.participant_count,
            speaker_count: snapshot.speaker_count,
            generated_at_ms: snapshot.generated_at_ms,
            profiles_complete: snapshot.profiles_complete,
            profile_error: snapshot.profile_error.clone(),
            has_more_participants,
            next_participant_cursor,
            has_more_raised_hands: snapshot.raised_hands.len() >= self.raised_hand_limit,
            next_raised_hand_cursor: None,
        };
        let mut mutations = vec![
            RiseStoreMutation::UpsertEntity {
                namespace: RiseRTCStoreLayout::rooms(),
                entity: RiseStoreEntity::new(
                    room_id.clone(),
                    RiseRTCStoreLayout::encode(&snapshot.room)?,
                    RiseRTCStoreLayout::room_version(&snapshot.room),
                    snapshot.room.updated_at_ms,
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            },
            RiseStoreMutation::UpsertEntityViewMembership {
                namespace: RiseRTCStoreLayout::rooms(),
                view: RiseRTCStoreLayout::RECENT_ROOMS_VIEW.to_owned(),
                membership: RiseEntityViewMembership::new(
                    room_id.clone(),
                    RiseRTCStoreLayout::room_position(&snapshot.room),
                    snapshot_version,
                ),
            },
            RiseStoreMutation::RemoveAllEntityViewMemberships {
                namespace: RiseRTCStoreLayout::participants(),
                view: RiseRTCStoreLayout::participant_view(&room_id),
                if_version_at_most: Some(snapshot_version),
            },
            RiseStoreMutation::RemoveAllEntityViewMemberships {
                namespace: RiseRTCStoreLayout::raised_hands(),
                view: RiseRTCStoreLayout::raised_hands_view(&room_id),
                if_version_at_most: Some(snapshot_version),
            },
        ];
        for participant in &participants {
            mutations.extend(Self::participant_mutations(
                participant,
                &room_id,
                snapshot_version.max(RiseRTCStoreLayout::participant_version(participant)),
            )?);
        }
        for hand in &hands {
            mutations.extend(Self::raised_hand_mutations(
                hand,
                &room_id,
                snapshot_version.max(RiseRTCStoreLayout::hand_version(hand)),
            )?);
        }
        mutations.extend([
            RiseStoreMutation::TrimEntityView {
                namespace: RiseRTCStoreLayout::rooms(),
                view: RiseRTCStoreLayout::RECENT_ROOMS_VIEW.to_owned(),
                keeping: RiseEntityViewRetention::Latest(32),
                if_version_at_most: None,
            },
            RiseStoreMutation::TrimEntityView {
                namespace: RiseRTCStoreLayout::participants(),
                view: RiseRTCStoreLayout::participant_view(&room_id),
                keeping: RiseEntityViewRetention::First(self.participant_limit),
                if_version_at_most: None,
            },
            RiseStoreMutation::TrimEntityView {
                namespace: RiseRTCStoreLayout::raised_hands(),
                view: RiseRTCStoreLayout::raised_hands_view(&room_id),
                keeping: RiseEntityViewRetention::First(self.raised_hand_limit),
                if_version_at_most: None,
            },
            RiseStoreMutation::SetValue {
                namespace: RiseRTCStoreLayout::rooms(),
                key: RiseRTCStoreLayout::room_meta_key(&room_id),
                value: RiseStoredValue::new(
                    RiseRTCStoreLayout::encode(&meta)?,
                    snapshot_version,
                    snapshot.generated_at_ms,
                ),
                precondition: RiseStoreVersionPrecondition::Any,
                policy: RiseStoredValueWritePolicy::ServerAuthoritative,
            },
        ]);
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn append_participants(
        &self,
        incoming: Vec<RiseRTCParticipant>,
        room_id: &str,
        previous_meta: &RiseRTCRoomMeta,
        has_more: bool,
        next_cursor: Option<String>,
        profiles_complete: bool,
        profile_error: Option<String>,
    ) -> anyhow::Result<()> {
        let now = Self::now_milliseconds();
        let version = RiseRTCStoreLayout::timestamp_version(now, 0).max(1);
        let mut mutations = Vec::new();
        let retained: Vec<RiseRTCParticipant> = incoming
            .iter()
            .take(self.page_limit)
            .map(|participant| self.retaining_profile_participant(participant))
            .collect();
        for participant in &retained {
            mutations.extend(Self::participant_mutations(
                participant,
                room_id,
                version.max(RiseRTCStoreLayout::participant_version(participant)),
            )?);
        }
        let meta = RiseRTCRoomMeta {
            participant_count: previous_meta.participant_count.max(incoming.len() as i64),
            speaker_count: previous_meta.speaker_count,
            generated_at_ms: previous_meta.generated_at_ms.max(now),
            profiles_complete: previous_meta.profiles_complete && profiles_complete,
            profile_error: profile_error.or_else(|| previous_meta.profile_error.clone()),
            has_more_participants: has_more,
            next_participant_cursor: next_cursor,
            has_more_raised_hands: previous_meta.has_more_raised_hands,
            next_raised_hand_cursor: previous_meta.next_raised_hand_cursor.clone(),
        };
        mutations.extend([
            RiseStoreMutation::TrimEntityView {
                namespace: RiseRTCStoreLayout::participants(),
                view: RiseRTCStoreLayout::participant_view(room_id),
                keeping: RiseEntityViewRetention::First(self.participant_limit),
                if_version_at_most: None,
            },
            RiseStoreMutation::SetValue {
                namespace: RiseRTCStoreLayout::rooms(),
                key: RiseRTCStoreLayout::room_meta_key(room_id),
                value: RiseStoredValue::new(RiseRTCStoreLayout::encode(&meta)?, version, now),
                precondition: RiseStoreVersionPrecondition::Any,
                policy: RiseStoredValueWritePolicy::ServerAuthoritative,
            },
        ]);
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        Ok(())
    }

    async fn replace_raised_hands(
        &self,
        incoming: Vec<RiseRTCRaisedHand>,
        room_id: &str,
        has_more: bool,
        next_cursor: Option<String>,
    ) -> anyhow::Result<()> {
        let now = Self::now_milliseconds();
        let version = RiseRTCStoreLayout::timestamp_version(now, 0).max(1);
        let mut mutations = vec![RiseStoreMutation::RemoveAllEntityViewMemberships {
            namespace: RiseRTCStoreLayout::raised_hands(),
            view: RiseRTCStoreLayout::raised_hands_view(room_id),
            if_version_at_most: Some(version),
        }];
        let retained: Vec<RiseRTCRaisedHand> = incoming
            .iter()
            .take(self.raised_hand_limit)
            .map(|hand| self.retaining_profile_hand(hand))
            .collect();
        for hand in &retained {
            mutations.extend(Self::raised_hand_mutations(hand, room_id, version)?);
        }
        if let Some(previous) = self.room_meta(room_id).await? {
            let meta = RiseRTCRoomMeta {
                participant_count: previous.participant_count,
                speaker_count: previous.speaker_count,
                generated_at_ms: previous.generated_at_ms.max(now),
                profiles_complete: previous.profiles_complete,
                profile_error: previous.profile_error.clone(),
                has_more_participants: previous.has_more_participants,
                next_participant_cursor: previous.next_participant_cursor.clone(),
                has_more_raised_hands: has_more,
                next_raised_hand_cursor: next_cursor,
            };
            mutations.push(RiseStoreMutation::SetValue {
                namespace: RiseRTCStoreLayout::rooms(),
                key: RiseRTCStoreLayout::room_meta_key(room_id),
                value: RiseStoredValue::new(RiseRTCStoreLayout::encode(&meta)?, version, now),
                precondition: RiseStoreVersionPrecondition::Any,
                policy: RiseStoredValueWritePolicy::ServerAuthoritative,
            });
        }
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        Ok(())
    }

    async fn store_participant(
        &self,
        incoming: &RiseRTCParticipant,
        room_id: &str,
    ) -> anyhow::Result<()> {
        let participant = self.retaining_profile_participant(incoming);
        let mutations = Self::participant_mutations(
            &participant,
            room_id,
            RiseRTCStoreLayout::participant_version(&participant),
        )?;
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        Ok(())
    }

    async fn store_raised_hand(
        &self,
        incoming: &RiseRTCRaisedHand,
        room_id: &str,
    ) -> anyhow::Result<()> {
        let hand = self.retaining_profile_hand(incoming);
        let mutations = Self::raised_hand_mutations(
            &hand,
            room_id,
            RiseRTCStoreLayout::hand_version(&hand),
        )?;
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        Ok(())
    }

    async fn remove_participant(&self, user_id: &str, room_id: &str) -> anyhow::Result<()> {
        tracing::warn!(target: "riseonly::X2163", %user_id, %room_id, "rtc roster remove");
        let engine = self.engine.clone();
        let user_id = user_id.to_owned();
        let room_id = room_id.to_owned();
        let (task, generation) = self.state.with(|state| {
            let previous = state.participant_removal_task.take();
            state.participant_removal_generation = state.participant_removal_generation.wrapping_add(1);
            let generation = state.participant_removal_generation;
            let handle = RiseAccountRuntime::current().spawn(async move {
                if let Some(previous) = previous {
                    let _ = previous.await;
                }
                Self::remove_stored_participant(&engine, &user_id, &room_id).await
            });
            let task: RiseRTCParticipantRemovalTask = async move {
                match handle.await {
                    Ok(result) => result.map_err(Arc::new),
                    Err(_) => Err(Arc::new(anyhow::Error::new(CancellationError))),
                }
            }
            .boxed()
            .shared();
            state.participant_removal_task = Some(task.clone());
            (task, generation)
        });
        let result = task.await;
        self.state.with(|state| {
            if state.participant_removal_generation == generation {
                state.participant_removal_task = None;
            }
        });
        result.map_err(|error| anyhow::Error::msg(error.to_string()))
    }

    async fn remove_stored_participant(
        engine: &RiseEngine,
        user_id: &str,
        room_id: &str,
    ) -> anyhow::Result<()> {
        let local_id = RiseRTCStoreLayout::participant_local_id(room_id, user_id);
        let was_member = engine
            .entity_view_position(
                RiseRTCStoreLayout::participants(),
                RiseRTCStoreLayout::participant_view(room_id),
                local_id.clone(),
            )
            .await?
            .is_some();
        let departing = engine
            .entity_local_id(RiseRTCStoreLayout::participants(), local_id.clone())
            .await?
            .map(|entity| RiseRTCStoreLayout::decode::<RiseRTCParticipant>(&entity.data))
            .transpose()?;
        let mut mutations = vec![
            RiseStoreMutation::RemoveEntityViewMembership {
                namespace: RiseRTCStoreLayout::participants(),
                view: RiseRTCStoreLayout::participant_view(room_id),
                local_id,
                if_version_at_most: None,
            },
            RiseStoreMutation::RemoveEntityViewMembership {
                namespace: RiseRTCStoreLayout::raised_hands(),
                view: RiseRTCStoreLayout::raised_hands_view(room_id),
                local_id: RiseRTCStoreLayout::raised_hand_local_id(room_id, user_id),
                if_version_at_most: None,
            },
        ];
        if was_member && let Some(previous) = Self::stored_room_meta(engine, room_id).await? {
            let now = Self::now_milliseconds();
            let was_speaker = departing.as_ref().is_some_and(|departing| {
                departing.role == RiseRTCParticipantRole::Speaker
                    || departing.role == RiseRTCParticipantRole::Moderator
            });
            let meta = RiseRTCRoomMeta {
                participant_count: (previous.participant_count - 1).max(0),
                speaker_count: if was_speaker {
                    (previous.speaker_count - 1).max(0)
                } else {
                    previous.speaker_count
                },
                generated_at_ms: previous.generated_at_ms.max(now),
                profiles_complete: previous.profiles_complete,
                profile_error: previous.profile_error.clone(),
                has_more_participants: previous.has_more_participants,
                next_participant_cursor: previous.next_participant_cursor.clone(),
                has_more_raised_hands: previous.has_more_raised_hands,
                next_raised_hand_cursor: previous.next_raised_hand_cursor.clone(),
            };
            mutations.push(RiseStoreMutation::SetValue {
                namespace: RiseRTCStoreLayout::rooms(),
                key: RiseRTCStoreLayout::room_meta_key(room_id),
                value: RiseStoredValue::new(
                    RiseRTCStoreLayout::encode(&meta)?,
                    RiseRTCStoreLayout::timestamp_version(now, 0).max(1),
                    now,
                ),
                precondition: RiseStoreVersionPrecondition::Any,
                policy: RiseStoredValueWritePolicy::ServerAuthoritative,
            });
        }
        engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        Ok(())
    }

    async fn remove_raised_hand(&self, user_id: &str, room_id: &str) -> anyhow::Result<()> {
        self.engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::RemoveEntityViewMembership {
                    namespace: RiseRTCStoreLayout::raised_hands(),
                    view: RiseRTCStoreLayout::raised_hands_view(room_id),
                    local_id: RiseRTCStoreLayout::raised_hand_local_id(room_id, user_id),
                    if_version_at_most: None,
                }],
                HashSet::new(),
            ))
            .await?;
        Ok(())
    }

    fn participant_mutations(
        participant: &RiseRTCParticipant,
        room_id: &str,
        membership_version: i64,
    ) -> anyhow::Result<Vec<RiseStoreMutation>> {
        let local_id = RiseRTCStoreLayout::participant_local_id(room_id, &participant.user_id);
        Ok(vec![
            RiseStoreMutation::UpsertEntity {
                namespace: RiseRTCStoreLayout::participants(),
                entity: RiseStoreEntity::new(
                    local_id.clone(),
                    RiseRTCStoreLayout::encode(participant)?,
                    RiseRTCStoreLayout::participant_version(participant),
                    participant.updated_at_ms,
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            },
            RiseStoreMutation::UpsertEntityViewMembership {
                namespace: RiseRTCStoreLayout::participants(),
                view: RiseRTCStoreLayout::participant_view(room_id),
                membership: RiseEntityViewMembership::new(
                    local_id,
                    RiseRTCStoreLayout::participant_position(participant),
                    membership_version,
                ),
            },
        ])
    }

    fn raised_hand_mutations(
        hand: &RiseRTCRaisedHand,
        room_id: &str,
        membership_version: i64,
    ) -> anyhow::Result<Vec<RiseStoreMutation>> {
        let local_id = RiseRTCStoreLayout::raised_hand_local_id(room_id, &hand.user_id);
        Ok(vec![
            RiseStoreMutation::UpsertEntity {
                namespace: RiseRTCStoreLayout::raised_hands(),
                entity: RiseStoreEntity::new(
                    local_id.clone(),
                    RiseRTCStoreLayout::encode(hand)?,
                    RiseRTCStoreLayout::hand_version(hand),
                    hand.requested_at_ms,
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            },
            RiseStoreMutation::UpsertEntityViewMembership {
                namespace: RiseRTCStoreLayout::raised_hands(),
                view: RiseRTCStoreLayout::raised_hands_view(room_id),
                membership: RiseEntityViewMembership::new(
                    local_id,
                    RiseRTCStoreLayout::hand_position(hand),
                    membership_version,
                ),
            },
        ])
    }

    async fn store_call(
        &self,
        call: &RiseRTCDirectCall,
        caller: Option<RiseRTCUserProfile>,
        event_id: Option<String>,
        occurred_at_ms: Option<i64>,
    ) -> anyhow::Result<()> {
        if let Some(batch) = self
            .make_call_batch(
                call.clone(),
                caller,
                event_id,
                occurred_at_ms.unwrap_or(call.updated_at_ms),
            )
            .await?
        {
            self.engine.write(batch).await?;
        }
        Ok(())
    }

    pub async fn make_call_push_batch(
        &self,
        push: RiseWirePushEnvelope,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let RiseEvent::RtcCall(event) = RiseEvent::decode(&push.r#type, &push.data.data(false)?)?
        else {
            return Ok(None);
        };
        self.make_call_batch(
            RiseRTCDirectCall::from_canon(&event.call),
            event.caller.as_ref().map(RiseRTCUserProfile::from_canon),
            (!event.event_id.is_empty()).then(|| event.event_id.clone()),
            event.occurred_at_ms,
        )
        .await
    }

    async fn make_call_batch(
        &self,
        incoming: RiseRTCDirectCall,
        caller: Option<RiseRTCUserProfile>,
        event_id: Option<String>,
        occurred_at_ms: i64,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let mut incoming = incoming;
        let version = RiseRTCStoreLayout::call_version(&incoming);
        let mut preserved_caller = caller.clone();
        if let Some(existing) = self
            .engine
            .entity_local_id(RiseRTCStoreLayout::calls(), incoming.call_id.clone())
            .await?
        {
            if existing.version > version {
                return Ok(None);
            }
            if let Ok(stored) = RiseRTCStoreLayout::decode::<RiseRTCStoredCall>(&existing.data) {
                incoming = incoming.keeping_key_halves(&stored.call);
                preserved_caller = caller.or(stored.caller.clone());
                if existing.version == version
                    && stored.call == incoming
                    && stored.caller == preserved_caller
                {
                    return Ok(None);
                }
            }
        }
        let record = RiseRTCStoredCall {
            call: incoming.clone(),
            caller: preserved_caller,
            event_id,
            occurred_at_ms,
        };
        Ok(Some(RiseStoreWriteBatch::new(
            vec![
                RiseStoreMutation::UpsertEntity {
                    namespace: RiseRTCStoreLayout::calls(),
                    entity: RiseStoreEntity::new(
                        incoming.call_id.clone(),
                        RiseRTCStoreLayout::encode(&record)?,
                        version,
                        incoming.updated_at_ms,
                    ),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                },
                RiseStoreMutation::UpsertEntityViewMembership {
                    namespace: RiseRTCStoreLayout::calls(),
                    view: RiseRTCStoreLayout::RECENT_CALLS_VIEW.to_owned(),
                    membership: RiseEntityViewMembership::new(
                        incoming.call_id.clone(),
                        RiseRTCStoreLayout::call_position(&incoming),
                        version,
                    ),
                },
                RiseStoreMutation::TrimEntityView {
                    namespace: RiseRTCStoreLayout::calls(),
                    view: RiseRTCStoreLayout::RECENT_CALLS_VIEW.to_owned(),
                    keeping: RiseEntityViewRetention::Latest(self.call_limit),
                    if_version_at_most: None,
                },
                RiseStoreMutation::PruneUnreferencedEntities {
                    namespace: RiseRTCStoreLayout::calls(),
                    limit: 32,
                },
            ],
            HashSet::new(),
        )))
    }

    async fn reduce_room_control_payload(
        &self,
        payload: &[u8],
        expected_room_id: &str,
    ) -> anyhow::Result<Option<RiseRTCRoomControlOutcome>> {
        let packet = RiseRoomEvent::decode(payload)?;
        if packet.room_id != expected_room_id {
            return Ok(None);
        }
        match &packet.event {
            RiseRoomEvent::ParticipantLeft(member) => {
                tracing::warn!(target: "riseonly::X2163", user_id = %member.user_id, actor = %packet.actor_user_id, "rtc room event participant left");
                if !member.user_id.is_empty() {
                    self.remove_participant(&member.user_id, &packet.room_id).await?;
                }
            }
            RiseRoomEvent::ParticipantKicked(kick) => {
                if !kick.target_user_id.is_empty() {
                    self.remove_participant(&kick.target_user_id, &packet.room_id).await?;
                    if kick.target_user_id == self.account_id {
                        return Ok(Some(RiseRTCRoomControlOutcome {
                            reaction: None,
                            eviction: Some(RiseRTCRoomEviction {
                                room_id: packet.room_id.clone(),
                                reason: RiseRTCEvictionReason::Removed,
                                occurred_at_ms: packet.occurred_at_ms,
                            }),
                        }));
                    }
                }
            }
            RiseRoomEvent::HandCancelled(member) => {
                if !member.user_id.is_empty() {
                    self.remove_raised_hand(&member.user_id, &packet.room_id).await?;
                }
            }
            RiseRoomEvent::ParticipantMediaUpdated(participant)
            | RiseRoomEvent::ParticipantPromoted(participant)
            | RiseRoomEvent::ParticipantMovedToAudience(participant) => {
                self.store_participant(&RiseRTCParticipant::from_canon(participant), &packet.room_id)
                    .await?;
            }
            RiseRoomEvent::ParticipantModerated(moderation) => {
                self.store_participant(
                    &RiseRTCParticipant::from_canon(&moderation.participant),
                    &packet.room_id,
                )
                .await?;
            }
            RiseRoomEvent::HandRaised(hand) => {
                self.store_raised_hand(&RiseRTCRaisedHand::from_canon(hand), &packet.room_id)
                    .await?;
            }
            RiseRoomEvent::Reaction(reaction) => {
                if reaction.emoji.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(RiseRTCRoomControlOutcome {
                    reaction: Some(RiseRTCReaction {
                        event_id: packet.event_id.clone(),
                        room_id: packet.room_id.clone(),
                        actor_user_id: packet.actor_user_id.clone(),
                        emoji: reaction.emoji.clone(),
                        created_at_ms: packet.occurred_at_ms,
                    }),
                    eviction: None,
                }));
            }
            RiseRoomEvent::Untyped { .. } => {}
        }
        Ok(None)
    }

    fn calls_snapshot(
        snapshot: &RiseStoreSnapshot,
        query: &RiseEntityWindowQuery,
        account_id: &str,
    ) -> anyhow::Result<RiseRTCCallRepositorySnapshot> {
        let mut calls = snapshot
            .entity_windows
            .get(query)
            .map(|window| window.items.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|item| RiseRTCStoreLayout::decode::<RiseRTCStoredCall>(&item.entity.data))
            .collect::<anyhow::Result<Vec<_>>>()?;
        calls.sort_by(|lhs, rhs| {
            rhs.call
                .updated_at_ms
                .cmp(&lhs.call.updated_at_ms)
                .then_with(|| lhs.call.call_id.cmp(&rhs.call.call_id))
        });
        Ok(RiseRTCCallRepositorySnapshot {
            account_id: account_id.to_owned(),
            revision: snapshot.revision,
            calls,
        })
    }

    fn room_snapshot(
        snapshot: &RiseStoreSnapshot,
        room_key: &RiseEntityKey,
        meta_key: &RiseStoreValueKey,
        participant_query: &RiseEntityWindowQuery,
        hand_query: &RiseEntityWindowQuery,
        account_id: &str,
    ) -> anyhow::Result<RiseRTCRoomRepositorySnapshot> {
        let room = snapshot
            .entities
            .get(room_key)
            .map(|entity| RiseRTCStoreLayout::decode::<RiseRTCRoomConfig>(&entity.data))
            .transpose()?;
        let participants = snapshot
            .entity_windows
            .get(participant_query)
            .map(|window| window.items.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|item| RiseRTCStoreLayout::decode::<RiseRTCParticipant>(&item.entity.data))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let raised_hands = snapshot
            .entity_windows
            .get(hand_query)
            .map(|window| window.items.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|item| RiseRTCStoreLayout::decode::<RiseRTCRaisedHand>(&item.entity.data))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let meta = snapshot
            .values
            .get(meta_key)
            .map(|value| RiseRTCStoreLayout::decode::<RiseRTCRoomMeta>(&value.data))
            .transpose()?;
        Ok(RiseRTCRoomRepositorySnapshot {
            account_id: account_id.to_owned(),
            revision: snapshot.revision,
            room,
            participants,
            raised_hands,
            meta,
        })
    }

    async fn room_meta(&self, room_id: &str) -> anyhow::Result<Option<RiseRTCRoomMeta>> {
        Self::stored_room_meta(&self.engine, room_id).await
    }

    async fn stored_room_meta(
        engine: &RiseEngine,
        room_id: &str,
    ) -> anyhow::Result<Option<RiseRTCRoomMeta>> {
        engine
            .value(RiseRTCStoreLayout::rooms(), RiseRTCStoreLayout::room_meta_key(room_id))
            .await?
            .map(|value| RiseRTCStoreLayout::decode::<RiseRTCRoomMeta>(&value.data))
            .transpose()
    }

    /// copies the participant with its profile retained
    fn retaining_profile_participant(&self, participant: &RiseRTCParticipant) -> RiseRTCParticipant {
        if let Some(profile) = &participant.profile {
            self.remember(profile);
            return participant.clone();
        }
        let Some(profile) =
            self.state.with(|state| state.profile_by_user_id.get(&participant.user_id).cloned())
        else {
            return participant.clone();
        };
        RiseRTCParticipant { profile: Some(profile), ..participant.clone() }
    }

    /// copies the raised hand with its profile retained
    fn retaining_profile_hand(&self, hand: &RiseRTCRaisedHand) -> RiseRTCRaisedHand {
        if let Some(profile) = &hand.profile {
            self.remember(profile);
            return hand.clone();
        }
        let Some(profile) =
            self.state.with(|state| state.profile_by_user_id.get(&hand.user_id).cloned())
        else {
            return hand.clone();
        };
        RiseRTCRaisedHand { profile: Some(profile), ..hand.clone() }
    }

    fn remember(&self, profile: &RiseRTCUserProfile) {
        let profile_limit = self.profile_limit;
        self.state.with(|state| {
            state.profile_by_user_id.insert(profile.id.clone(), profile.clone());
            state.profile_order.retain(|id| id != &profile.id);
            state.profile_order.push(profile.id.clone());
            while state.profile_order.len() > profile_limit {
                let evicted = state.profile_order.remove(0);
                state.profile_by_user_id.remove(&evicted);
            }
        });
    }

    fn normalized_id(value: &str, field: &str) -> anyhow::Result<String> {
        let value = trimming_whitespaces_and_newlines(value);
        if value.is_empty() || value.len() > 1_024 {
            return Err(RiseRTCStateError::Validation(field.to_owned()).into());
        }
        Ok(value.to_owned())
    }

    fn normalized_optional(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        if value.is_empty() {
            return None;
        }
        Some(value.to_owned())
    }

    fn bounded_optional(value: Option<&str>, maximum_characters: usize) -> Option<String> {
        let value = Self::normalized_optional(value)?;
        Some(character_prefix(&value, maximum_characters).to_owned())
    }

    fn safe_key(value: &str, field: &str) -> anyhow::Result<String> {
        let value = trimming_whitespaces_and_newlines(value);
        let is_safe = (8..=128).contains(&value.len())
            && value.bytes().all(|byte| {
                byte < 128
                    && (byte.is_ascii_digit()
                        || byte.is_ascii_uppercase()
                        || byte.is_ascii_lowercase()
                        || [b'-', b'.', b':', b'_'].contains(&byte))
            });
        if !is_safe {
            return Err(RiseRTCStateError::Validation(field.to_owned()).into());
        }
        Ok(value.to_owned())
    }

    /// current installation id as a safe key, none if it cant be read
    fn current_installation_id() -> Option<String> {
        RiseInstallationIdentity::current()
            .ok()
            .and_then(|identity| Self::safe_key(&identity, rise_key::rtc::INSTALLATION_ID).ok())
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }

    fn sanitized_endpoint(value: &str) -> String {
        let Some(components) = UrlComponents::from_string(value) else {
            return "invalid".to_owned();
        };
        let (Some(scheme), Some(host)) = (components.scheme(), components.host()) else {
            return "invalid".to_owned();
        };
        let port = components.port().map(|port| format!(":{port}")).unwrap_or_default();
        format!("{scheme}://{host}{port}")
    }

    fn error_summary(error: &anyhow::Error) -> String {
        let error_type = if error.is::<RiseRTCStateError>() {
            "RiseRTCStateError"
        } else if error.is::<RiseWireError>() {
            "RiseWireError"
        } else if error.is::<CancellationError>() {
            "CancellationError"
        } else if error.is::<serde_json::Error>() {
            "DecodingError"
        } else {
            "Error"
        };
        let mut fields = vec![format!("error_type={error_type}")];
        if let Some(decoding) = error.rise_rtc_safe_decoding_summary() {
            fields.push(decoding);
        }
        fields.join(" ")
    }

    async fn log(message: String) {
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| Sai::log("RTC", move || message, SaiLogKind::Network, cx));
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }
}

trait RiseRTCDirectCallCanon {
    fn keeping_key_halves(&self, stored: &RiseRTCDirectCall) -> RiseRTCDirectCall;
    fn from_canon(call: &RtcPublicCall) -> RiseRTCDirectCall;
}

impl RiseRTCDirectCallCanon for RiseRTCDirectCall {
    fn keeping_key_halves(&self, stored: &RiseRTCDirectCall) -> RiseRTCDirectCall {
        let caller = self.caller_e2ee_public_key.clone().or_else(|| stored.caller_e2ee_public_key.clone());
        let callee = self.callee_e2ee_public_key.clone().or_else(|| stored.callee_e2ee_public_key.clone());
        let same_recording = self.recording_state == stored.recording_state;
        let started_by = self.recording_started_by_user_id.clone().or_else(|| {
            if same_recording { stored.recording_started_by_user_id.clone() } else { None }
        });
        let path = self
            .recording_path
            .clone()
            .or_else(|| if same_recording { stored.recording_path.clone() } else { None });
        if caller == self.caller_e2ee_public_key
            && callee == self.callee_e2ee_public_key
            && started_by == self.recording_started_by_user_id
            && path == self.recording_path
        {
            return self.clone();
        }
        RiseRTCDirectCall {
            caller_e2ee_public_key: caller,
            callee_e2ee_public_key: callee,
            recording_started_by_user_id: started_by,
            recording_path: path,
            ..self.clone()
        }
    }

    fn from_canon(call: &RtcPublicCall) -> RiseRTCDirectCall {
        RiseRTCDirectCall {
            call_id: call.call_id.clone(),
            room_id: call.room_id.clone(),
            chat_id: call.chat_id.clone(),
            caller_user_id: call.caller_user_id.clone(),
            callee_user_id: call.callee_user_id.clone(),
            media_mode: RiseRTCCallMediaMode::parse(&call.media_mode),
            state: RiseRTCCallState::parse(&call.state),
            created_at_ms: call.created_at_ms,
            updated_at_ms: call.updated_at_ms,
            expires_at_ms: call.expires_at_ms,
            answered_at_ms: call.answered_at_ms,
            ended_at_ms: call.ended_at_ms,
            ended_by_user_id: call.ended_by_user_id.clone(),
            end_reason: call.end_reason.clone(),
            caller_e2ee_public_key: None,
            callee_e2ee_public_key: None,
            recording_state: call.recording_state.clone(),
            recording_started_by_user_id: None,
            recording_path: None,
        }
    }
}

trait RiseRTCUserProfileCanon {
    fn from_canon(caller: &RtcIncomingCaller) -> RiseRTCUserProfile;
}

impl RiseRTCUserProfileCanon for RiseRTCUserProfile {
    fn from_canon(caller: &RtcIncomingCaller) -> RiseRTCUserProfile {
        RiseRTCUserProfile {
            id: caller.id.clone(),
            tag: caller.tag.clone(),
            name: caller.name.clone(),
            avatar_url: caller.avatar_url.clone(),
            is_premium: caller.is_premium,
            is_official: caller.is_official,
            is_online: caller.is_online,
            last_seen_ms: caller.last_seen_ms,
        }
    }
}

trait RiseRTCParticipantCanon {
    fn from_canon(participant: &rise_room_event::Participant) -> RiseRTCParticipant;
}

impl RiseRTCParticipantCanon for RiseRTCParticipant {
    fn from_canon(participant: &rise_room_event::Participant) -> RiseRTCParticipant {
        RiseRTCParticipant {
            user_id: participant.user_id.clone(),
            role: participant.role.unwrap_or(RiseRTCParticipantRole::Unspecified),
            connected: participant.connected,
            microphone_enabled: participant.microphone_enabled,
            camera_enabled: participant.camera_enabled,
            screen_share_enabled: participant.screen_share_enabled,
            server_muted: participant.server_muted,
            server_deafened: participant.server_deafened,
            deafened: participant.deafened,
            hand_raised: participant.hand_raised,
            joined_at_ms: participant.joined_at_ms,
            updated_at_ms: participant.updated_at_ms,
            lease_expires_at_ms: participant.lease_expires_at_ms,
            order_sequence: participant.order_sequence,
            profile: None,
        }
    }
}

trait RiseRTCRaisedHandCanon {
    fn from_canon(hand: &rise_room_event::RaisedHand) -> RiseRTCRaisedHand;
}

impl RiseRTCRaisedHandCanon for RiseRTCRaisedHand {
    fn from_canon(hand: &rise_room_event::RaisedHand) -> RiseRTCRaisedHand {
        RiseRTCRaisedHand {
            user_id: hand.user_id.clone(),
            requested_at_ms: hand.requested_at_ms,
            expires_at_ms: (hand.expires_at_ms > 0).then_some(hand.expires_at_ms),
            note: hand.note.clone(),
            profile: None,
        }
    }
}
