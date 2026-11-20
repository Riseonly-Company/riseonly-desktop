use std::any::Any;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use rise_canon::rise_key::events;
use rise_canon::rise_key::music as key;
use rise_canon::rise_service_text::music_service;
use rise_canon::{RiseLimits, RiseTimingCanon, rise_proto, rise_wire_methods};
use rise_catalog::MusicTrackStatus;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_events::name;
use rise_i18n::tr;
use rise_moderation::PlatformRole;
use rise_music::PlaylistVisibility;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_identity::{RiseIdentity, RiseRealtimeStamp};
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::{RiseTimingCanonExt, RiseTimingHeartbeatExt};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::{SaiCacheCleanLayer, SaiCacheCleaner};
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiTypedSlice};
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::{
    SaiWsOptimisticError, SaiWsOptimisticFiles, SaiWsOptimisticLocalFile,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::music::engine::rise_music_presentation::RiseMusicPresentation;
use crate::modules::music::shared::utils::music::music_cover_upload::MusicCoverUpload;
use crate::modules::music::shared::utils::music::music_models::{MusicLocalTrackTransferPhase, TrackModel};
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::shared::utils::music::music_track_file_upload::MusicTrackFileUpload;
use crate::modules::music::shared::utils::music::music_track_upload_notifier::MusicTrackUploadNotifier;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_message_import_types::{
    MusicMessageMP3ImportJob, MusicMessageMP3PendingCompletion, MusicMessageMP3ProgressPersistence,
};
use crate::modules::music::stores::music_actions::music_types::{
    MusicIndexedTrackSnapshot, MusicListeningCommandPipelinePolicy, MusicListeningSessionCommand, MusicListeningSessionStateVector,
    MusicListeningSessionUpdate, MusicPlaylistInviteLink, MusicPlaylistInvitePreview, MusicPlaylistInviteSheetState,
    MusicPlaylistJoinRequest, MusicPlaylistListener, MusicPlaylistListeningSession, MusicPlaylistListeningSnapshot,
    MusicLocalPlaylistDestination, MusicLocalTracksDeleteSnapshot, MusicPlaylistMember, MusicPlaylistMemberPermissions, MusicPlaylistSettings, MusicPlaylistTrackLinkError,
    MusicPlaylistTracksDeleteSnapshot, MusicTrackLinkError, MusicListeningParticipantPermissions, MusicListeningServerErrorCode,
    MusicListeningAudioEffect, MusicListeningAudioEffectWire,
};
use crate::modules::music::stores::music_actions::music_wire_types::{
    ProtoTrackRecordExt, RiseStoredMusicLocalTrackUpload, RiseStoredMusicLocalTracksPage, RiseStoredMusicPlaylist, RiseStoredMusicPlaylistDetail,
    RiseStoredMusicPlaylistLibrary, RiseStoredMusicPlaylistMutation, RiseStoredMusicTrack, RiseStoredMusicUpload,
    WireAlbumDTO, WireAlbumRequestDTO, WireArtistDTO, WireArtistDTOExt, WireArtistRequestDTO, WireArtistsListDTOExt,
    WireLocalTracksListDTO, WireLocalTracksListDTOExt, WirePlaylistInviteLinkDTOExt, WirePlaylistInviteLinkResponseDTO,
    WirePlaylistInvitePreviewDTOExt, WirePlaylistInvitePreviewResponseDTO, WirePlaylistJoinRequestDTOExt,
    WirePlaylistJoinRequestsDTO, WirePlaylistListeningSessionDTOExt, WirePlaylistListeningSessionResponseDTO,
    WirePlaylistListeningSnapshotDTO, WirePlaylistListeningSnapshotDTOExt, WirePlaylistMemberDTO, WirePlaylistMemberDTOExt,
    WirePlaylistMemberPermissionsDTO, WirePlaylistMemberResponseDTO, WireRequestPlaylistJoinResponseDTO,
    WireReviewPlaylistJoinRequestResponseDTO, WirePlaylistListeningSessionDTO, WireListeningParticipantPermissionsDTO,
    WireListeningParticipantDTO, WireListeningParticipantPermissionsDTOExt, WireListeningAudioEffectDTO, WireUserRefDTO,
    WireUserRefDTOExt, WirePlaylistPresenceWriteDTO,
    WirePlaylistMembersDTO, WirePlaylistSettingsDTOExt, WirePlaylistSettingsResponseDTO, WireSearchDTO,
    WireSearchDTOExt, WireSingleAlbumDTOExt, WireSingleArtistDTOExt, WireSingleTrackDTO, WireTracksListDTOExt,
};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_guide_sandbox_hooks::MusicGuideSandboxHooks;

const MUSIC_FILE_CONTEXT: &str = "music";
const MUSIC_AUDIO_FILE_CONTEXT: &str = "music_audio";
const MUSIC_COVER_FILE_CONTEXT: &str = "music_cover";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicPaginationState {
    pub cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicPlaylistRevokedNotice {
    pub id: Uuid,
    pub playlist_id: String,
}

impl MusicPlaylistRevokedNotice {
    pub fn new(playlist_id: String) -> Self {
        Self { id: Uuid::new_v4(), playlist_id }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MusicCollaborationMutationSnapshot {
    roots_by_cache_key: HashMap<String, Map<String, Value>>,
}

#[derive(Clone, Debug)]
struct MusicLocalListeningPresenceState {
    playlist_id: String,
    track_id: String,
    anchor_position_ms: i64,
    is_playing: bool,
    local_anchor_time_ms: i64,
}

#[derive(Clone, Debug)]
struct MusicListeningObservation {
    playlist_id: String,
    track_id: Option<String>,
}

type MusicListeningSessionSuccessHandler = Box<dyn FnOnce(MusicPlaylistListeningSession, &mut App)>;
type MusicListeningSessionErrorHandler = Box<dyn FnOnce(String, &mut App)>;

struct MusicListeningStateCommandIntent {
    id: Uuid,
    session_id: String,
    command: MusicListeningSessionCommand,
    state: MusicListeningSessionStateVector,
    captured_at_ms: i64,
    fallback_revision: i64,
    retry_attempt: i64,
    on_success: Vec<MusicListeningSessionSuccessHandler>,
    on_error: Vec<MusicListeningSessionErrorHandler>,
}

impl MusicListeningStateCommandIntent {
    fn merging_newer(self, newer: MusicListeningStateCommandIntent) -> Self {
        let MusicListeningStateCommandIntent {
            id,
            session_id,
            command: newer_command,
            state,
            captured_at_ms,
            fallback_revision: newer_fallback_revision,
            retry_attempt: newer_retry_attempt,
            on_success: newer_on_success,
            on_error: newer_on_error,
        } = newer;
        let mut on_success = self.on_success;
        on_success.extend(newer_on_success);
        let mut on_error = self.on_error;
        on_error.extend(newer_on_error);
        Self {
            id,
            session_id,
            command: MusicListeningCommandPipelinePolicy::dominant(self.command, newer_command),
            state,
            captured_at_ms,
            fallback_revision: self.fallback_revision.max(newer_fallback_revision),
            retry_attempt: self.retry_attempt.max(newer_retry_attempt),
            on_success,
            on_error,
        }
    }
}

pub type MusicListeningSessionObserver = Box<dyn Fn(&MusicListeningSessionUpdate, &mut App)>;

pub struct MusicServicesStore {
    pub playlist_invite_sheet: Option<MusicPlaylistInviteSheetState>,
    pub revoked_playlist_notice: Option<MusicPlaylistRevokedNotice>,

    presence_heartbeat_tasks: HashMap<String, Task<()>>,
    local_presence_states: HashMap<String, MusicLocalListeningPresenceState>,
    listening_observation_tasks: HashMap<String, Task<()>>,
    listening_observations: HashMap<String, MusicListeningObservation>,
    listening_session_sync_tasks: HashMap<String, Task<()>>,
    listening_session_recovery_tasks: HashMap<String, Task<()>>,
    playlist_snapshot_recovery_tasks: HashMap<String, Task<()>>,
    listening_session_observers: HashMap<Uuid, Rc<dyn Fn(&MusicListeningSessionUpdate, &mut App)>>,
    server_clock_offset_ms_by_playlist: HashMap<String, f64>,
    last_client_sequence_by_scope: HashMap<String, i64>,
    listening_state_command_in_flight: HashMap<String, MusicListeningStateCommandIntent>,
    pending_listening_state_commands: HashMap<String, MusicListeningStateCommandIntent>,
    listening_state_command_recovery: HashSet<String>,
    ended_listening_session_ids: HashMap<String, Date>,
    recent_playlist_mutation_request_ids: HashMap<String, i64>,
    collaboration_mutation_playlists_in_flight: HashSet<String>,
    pub message_mp3_import_queue: Vec<MusicMessageMP3ImportJob>,
    pub message_mp3_import_task: Option<Task<()>>,
    pub message_mp3_import_generation: u64,
    pub message_mp3_pending_completions: HashMap<String, MusicMessageMP3PendingCompletion>,
    pub message_mp3_completed_request_ids: HashSet<String>,
    pub message_mp3_active_import_temp_ids: HashSet<String>,
    pub message_mp3_cancelled_temp_ids: HashSet<String>,
    pub message_mp3_uploaded_temp_ids: HashSet<String>,
    pub message_mp3_failed_jobs: HashMap<String, MusicMessageMP3ImportJob>,
    pub message_mp3_progress_persistence: HashMap<String, MusicMessageMP3ProgressPersistence>,

    playlist_listening_client_instance_id: String,
    track_like_overrides: HashMap<String, bool>,
}

impl MusicServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    pub fn try_read(cx: &App) -> Option<&Self> {
        rise_ui::shared::try_read::<Self>(cx)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            playlist_invite_sheet: None,
            revoked_playlist_notice: None,
            presence_heartbeat_tasks: HashMap::new(),
            local_presence_states: HashMap::new(),
            listening_observation_tasks: HashMap::new(),
            listening_observations: HashMap::new(),
            listening_session_sync_tasks: HashMap::new(),
            listening_session_recovery_tasks: HashMap::new(),
            playlist_snapshot_recovery_tasks: HashMap::new(),
            listening_session_observers: HashMap::new(),
            server_clock_offset_ms_by_playlist: HashMap::new(),
            last_client_sequence_by_scope: HashMap::new(),
            listening_state_command_in_flight: HashMap::new(),
            pending_listening_state_commands: HashMap::new(),
            listening_state_command_recovery: HashSet::new(),
            ended_listening_session_ids: HashMap::new(),
            recent_playlist_mutation_request_ids: HashMap::new(),
            collaboration_mutation_playlists_in_flight: HashSet::new(),
            message_mp3_import_queue: Vec::new(),
            message_mp3_import_task: None,
            message_mp3_import_generation: 0,
            message_mp3_pending_completions: HashMap::new(),
            message_mp3_completed_request_ids: HashSet::new(),
            message_mp3_active_import_temp_ids: HashSet::new(),
            message_mp3_cancelled_temp_ids: HashSet::new(),
            message_mp3_uploaded_temp_ids: HashSet::new(),
            message_mp3_failed_jobs: HashMap::new(),
            message_mp3_progress_persistence: HashMap::new(),
            playlist_listening_client_instance_id: Self::make_playlist_listening_client_instance_id(),
            track_like_overrides: HashMap::new(),
        }
    }

    fn make_playlist_listening_client_instance_id() -> String {
        let defaults_key = user_defaults_keys::MUSIC_PLAYLIST_LISTENING_CLIENT_INSTANCE_ID;
        let defaults = UserDefaults::standard();
        if let Some(value) = defaults.string(defaults_key)
            && Uuid::from_uuid_string(&value).is_some()
        {
            return value.to_lowercase();
        }
        let value = Uuid::new_v4().uuid_string().to_lowercase();
        defaults.set(defaults_key, value.as_str());
        value
    }

    fn actions(cx: &mut App) -> Entity<MusicActionsStore> {
        MusicActionsStore::shared(cx)
    }

    pub fn sound_track(&self, track_id: &str, cx: &mut App) -> Option<RiseStoredMusicTrack> {
        if let Some(invented) = MusicGuideSandboxHooks::sound_track(track_id, cx) {
            return Some(invented);
        }
        let sound = &RiseMusicPresentation::shared(cx).read(cx).snapshot().sound;
        if sound.track_id == track_id { sound.track.clone() } else { None }
    }

    pub fn sound_track_failed(&self, track_id: &str, cx: &mut App) -> bool {
        let sound = &RiseMusicPresentation::shared(cx).read(cx).snapshot().sound;
        sound.track_id == track_id && sound.failed
    }

    pub fn sound_track_unavailable(&self, track_id: &str, cx: &mut App) -> bool {
        let sound = &RiseMusicPresentation::shared(cx).read(cx).snapshot().sound;
        sound.track_id == track_id && sound.unavailable
    }

    fn ended_listening_sessions_limit() -> i64 {
        RiseLimits::client_value("music_services_store_ended_listening_sessions_limit")
    }

    fn ended_listening_session_ttl() -> f64 {
        RiseTimingCanon::heartbeat("music_ended_session_memory").client_interval_seconds()
    }

    pub fn tracks_cache_key(genre: Option<&str>, artist_id: Option<&str>, album_id: Option<&str>) -> String {
        SaiCache::new(["listTracks", genre.unwrap_or_default(), artist_id.unwrap_or_default(), album_id.unwrap_or_default()])
            .key
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.presence_heartbeat_tasks.clear();
        self.local_presence_states.clear();
        self.listening_observation_tasks.clear();
        self.listening_observations.clear();
        self.listening_session_sync_tasks.clear();
        self.listening_session_recovery_tasks.clear();
        self.playlist_snapshot_recovery_tasks.clear();
        self.revoked_playlist_notice = None;
        self.server_clock_offset_ms_by_playlist.clear();
        self.last_client_sequence_by_scope.clear();
        self.listening_state_command_in_flight.clear();
        self.pending_listening_state_commands.clear();
        self.listening_state_command_recovery.clear();
        self.ended_listening_session_ids.clear();
        self.recent_playlist_mutation_request_ids.clear();
        self.collaboration_mutation_playlists_in_flight.clear();
        self.playlist_invite_sheet = None;
        self.track_like_overrides.clear();
        self.reset_message_mp3_import_state(cx);
        MusicPlayerState::shared(cx).update(cx, |player, cx| player.clear_ad_hoc_effect_memory(cx));
        cx.notify();
    }

    pub fn liked_cache_key() -> String {
        SaiCache::new(["getLiked"]).key
    }

    pub fn artists_cache_key() -> String {
        SaiCache::new(["listArtists"]).key
    }

    pub fn my_tracks_cache_key() -> String {
        SaiCache::new(["listMyTracks"]).key
    }

    pub fn my_local_tracks_cache_key() -> String {
        SaiCache::new(["listMyLocalTracks"]).key
    }

    pub fn my_album_requests_cache_key() -> String {
        SaiCache::new(["getMyAlbumRequests"]).key
    }

    pub fn search_cache_key(query: &str, search_type: Option<&str>) -> String {
        SaiCache::new(["search", query, search_type.unwrap_or_default()]).key
    }

    pub fn playlist_track_link_error(&self, error: &anyhow::Error) -> MusicPlaylistTrackLinkError {
        if CanonErrors::is_connection_loss(error) {
            return MusicPlaylistTrackLinkError::LoadFailed;
        }
        let Some(failure) = CanonErrors::failure(error) else {
            return MusicPlaylistTrackLinkError::LoadFailed;
        };
        let value = failure.message.to_lowercase();
        if value.contains("suspended") {
            return MusicPlaylistTrackLinkError::AccessSuspended;
        }
        if value.contains("not found") || value.contains("no longer exists") {
            return MusicPlaylistTrackLinkError::PlaylistNotFound;
        }
        if value.contains("invalid playlist") || value.contains("invalid resource") || value.contains("bad request") {
            return MusicPlaylistTrackLinkError::InvalidLink;
        }
        if value.contains("forbidden")
            || value.contains("private")
            || value.contains("permission")
            || value.contains("access")
            || value.contains("unauthorized")
        {
            return MusicPlaylistTrackLinkError::AccessDenied;
        }
        MusicPlaylistTrackLinkError::LoadFailed
    }

    pub fn track_link_error(&self, error: &anyhow::Error) -> MusicTrackLinkError {
        if CanonErrors::is_connection_loss(error) {
            return MusicTrackLinkError::LoadFailed;
        }
        let Some(failure) = CanonErrors::failure(error) else {
            return MusicTrackLinkError::LoadFailed;
        };
        let value = failure.message.to_lowercase();
        if value.contains("not found") || value.contains("no longer exists") {
            return MusicTrackLinkError::NotFound;
        }
        if value.contains("invalid track") || value.contains("invalid resource") || value.contains("bad request") {
            return MusicTrackLinkError::InvalidLink;
        }
        if value.contains("forbidden")
            || value.contains("private")
            || value.contains("permission")
            || value.contains("access")
            || value.contains("unauthorized")
        {
            return MusicTrackLinkError::AccessDenied;
        }
        MusicTrackLinkError::LoadFailed
    }

    pub fn is_public_link_track(&self, track: &RiseStoredMusicTrack) -> bool {
        let status = track.status.as_deref().map(str::to_lowercase);
        matches!(
            status.as_deref().and_then(MusicTrackStatus::from_wire),
            Some(MusicTrackStatus::Published | MusicTrackStatus::OriginalAudio)
        )
    }

    pub fn artist_requests_cache_key(status: &str) -> String {
        SaiCache::new(["listArtistRequests", status]).key
    }

    pub fn track_requests_cache_key(status: &str) -> String {
        SaiCache::new(["listTrackRequests", status]).key
    }

    pub fn album_requests_cache_key(status: &str) -> String {
        SaiCache::new(["listAlbumRequests", status]).key
    }

    pub fn playlist_settings_cache_key(playlist_id: &str) -> String {
        SaiCache::new(["getPlaylistSettings", playlist_id]).key
    }

    pub fn playlist_invite_link_cache_key(playlist_id: &str) -> String {
        SaiCache::new(["getPlaylistInviteLink", playlist_id]).key
    }

    pub fn playlist_invite_preview_cache_key(token: &str) -> String {
        SaiCache::new(["getPlaylistInvitePreview", token]).key
    }

    pub fn playlist_join_requests_cache_key(playlist_id: &str) -> String {
        SaiCache::new(["getPlaylistJoinRequests", playlist_id]).key
    }

    pub fn playlist_members_cache_key(playlist_id: &str, include_suspended: bool) -> String {
        SaiCache::new(["getPlaylistMembers", playlist_id, if include_suspended { "all" } else { "active" }]).key
    }

    pub fn playlist_listening_snapshot_cache_key(playlist_id: &str, track_id: Option<&str>) -> String {
        SaiCache::new(["getPlaylistListeningSnapshot", playlist_id, track_id.unwrap_or("all")]).key
    }

    pub fn playlist_listening_session_cache_key(session_id: &str) -> String {
        SaiCache::new(["getPlaylistListeningSession", session_id]).key
    }

    pub fn collaboration_read_options(
        &self,
        method: &str,
        cache_key: &str,
        reflect_pending_in_ui: bool,
    ) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: Some(cache_key.to_owned()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    pub fn collaboration_paginated_read_options(
        &self,
        method: &str,
        cache_key: &str,
        cursor: Option<&str>,
        reflect_pending_in_ui: bool,
        path_to_array: &str,
        path_to_id: &str,
    ) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: Some(cache_key.to_owned()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: cursor.is_none() && reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(path_to_array.to_owned()),
            path_to_id: Some(path_to_id.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id: cursor.map(str::to_owned),
                response_relative_id_path: key::camel::NEXT_CURSOR.to_owned(),
                has_more_path: Some(key::camel::HAS_MORE.to_owned()),
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        })
    }

    pub fn collaboration_write_options(&self, method: &str) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    pub fn realtime_read_options(
        &self,
        method: &str,
        cache_key: &str,
        reflect_pending_in_ui: bool,
        store_response_in_cache: bool,
    ) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: Some(cache_key.to_owned()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache,
            persist_cache_to_disk: false,
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    pub fn realtime_write_options(&self, method: &str, allow_concurrent: bool) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: allow_concurrent,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    fn cached_dto<T: DeserializeOwned>(&self, cache_key: &str, cx: &mut App) -> Option<T> {
        let payload = SaiWsResponseCache::observed_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        SaiJSONCodec::decode::<T>(payload.as_ref())
    }

    fn current_cached_dto<T: DeserializeOwned>(&self, cache_key: &str, cx: &mut App) -> Option<T> {
        let payload = SaiWsResponseCache::current_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        SaiJSONCodec::decode::<T>(payload.as_ref())
    }

    fn install_memory_dto<T: Serialize>(&self, dto: &T, cache_key: &str, cx: &mut App) {
        let Some(root) = SaiJSONCodec::encode(dto) else { return };
        let _ = SaiWsResponseCache::install_optimistic_root(
            Value::Object(root),
            cache_key,
            true,
            false,
            SaiWsDiskCachePolicy::PreserveExisting,
            false,
            cx,
        );
    }

    fn collaboration_snapshot(&self, cache_keys: &[String], cx: &mut App) -> MusicCollaborationMutationSnapshot {
        let mut roots = HashMap::new();
        for cache_key in cache_keys {
            let payload = SaiWsResponseCache::current_payload(
                cache_key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            );
            let Some(root) = AnyDictionaryBridge::dictionary(payload.as_ref()) else { continue };
            roots.insert(cache_key.clone(), root);
        }
        MusicCollaborationMutationSnapshot { roots_by_cache_key: roots }
    }

    pub fn restore_collaboration_mutation(&self, snapshot: &MusicCollaborationMutationSnapshot, cx: &mut App) {
        let mine_key = SaiCache::new(["getPlaylistsMine"]).key;
        for (cache_key, root) in &snapshot.roots_by_cache_key {
            let _ = SaiUpdater::replace_root(cache_key, root.clone(), SaiUpdaterWriteScope::Cache, cx);
            if *cache_key == mine_key {
                let _ = SaiWsResponseCache::apply_sai_updater_payload(
                    Value::Object(root.clone()),
                    cache_key,
                    SaiUpdaterWriteScope::State,
                    false,
                    cx,
                );
            }
        }
    }

    pub fn playlist_settings(&self, playlist_id: &str, cx: &mut App) -> Option<MusicPlaylistSettings> {
        if let Some(staged) = MusicGuideSandboxHooks::playlist_settings(playlist_id, cx) {
            return Some(staged);
        }
        self.cached_dto::<WirePlaylistSettingsResponseDTO>(&Self::playlist_settings_cache_key(playlist_id), cx)
            .and_then(|response| response.settings)
            .and_then(|settings| settings.as_domain())
    }

    pub fn add_track_destination_playlists(&self, cx: &mut App) -> Vec<RiseStoredMusicPlaylist> {
        let viewer_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        let playlists = MusicActionsStore::slices(cx)
            .get_playlists_mine_sai
            .data()
            .and_then(|library| library.playlists)
            .unwrap_or_default();
        let mut destinations = Vec::new();
        for playlist in playlists {
            if playlist.visibility.as_deref() == Some(PlaylistVisibility::LocalPlaylist.wire()) {
                continue;
            }
            let Some(id) = playlist.id.clone().filter(|id| !id.is_empty()) else { continue };
            let accepted = if let Some(can_add_tracks) = playlist.can_add_tracks {
                can_add_tracks
            } else {
                let owner_id = playlist.owner_user_id.clone().unwrap_or_default();
                if owner_id.is_empty() || owner_id == viewer_id {
                    true
                } else {
                    self.playlist_settings(&id, cx)
                        .is_some_and(|settings| settings.viewer_permissions.can_add_tracks)
                }
            };
            if accepted {
                destinations.push(playlist);
            }
        }
        destinations
    }

    pub fn playlist_invite_link(&self, playlist_id: &str, cx: &mut App) -> Option<MusicPlaylistInviteLink> {
        if let Some(staged) = MusicGuideSandboxHooks::invite_link(playlist_id, cx) {
            return Some(staged);
        }
        self.cached_dto::<WirePlaylistInviteLinkResponseDTO>(&Self::playlist_invite_link_cache_key(playlist_id), cx)
            .and_then(|response| response.invite)
            .and_then(|invite| invite.as_domain())
    }

    pub fn playlist_invite_preview(&self, token: &str, cx: &mut App) -> Option<MusicPlaylistInvitePreview> {
        self.cached_dto::<WirePlaylistInvitePreviewResponseDTO>(&Self::playlist_invite_preview_cache_key(token), cx)
            .and_then(|response| response.preview)
            .and_then(|preview| preview.as_domain())
    }

    pub fn playlist_join_requests(&self, playlist_id: &str, cx: &mut App) -> Vec<MusicPlaylistJoinRequest> {
        if let Some(staged) = MusicGuideSandboxHooks::join_requests(playlist_id, cx) {
            return staged;
        }
        self.cached_dto::<WirePlaylistJoinRequestsDTO>(&Self::playlist_join_requests_cache_key(playlist_id), cx)
            .map(|response| response.requests.iter().filter_map(|request| request.as_domain()).collect())
            .unwrap_or_default()
    }

    pub fn playlist_pending_join_request_count(&self, playlist_id: &str, cx: &mut App) -> Option<i64> {
        if MusicGuideSandboxHooks::join_requests(playlist_id, cx).is_some() {
            return None;
        }
        let count = self
            .cached_dto::<WirePlaylistJoinRequestsDTO>(&Self::playlist_join_requests_cache_key(playlist_id), cx)?
            .pending_request_count;
        Some(count.max(0))
    }

    pub fn playlist_members(
        &self,
        playlist_id: &str,
        include_suspended: bool,
        cx: &mut App,
    ) -> Vec<MusicPlaylistMember> {
        if let Some(staged) = MusicGuideSandboxHooks::members(playlist_id, cx) {
            return staged;
        }
        self.cached_dto::<WirePlaylistMembersDTO>(
            &Self::playlist_members_cache_key(playlist_id, include_suspended),
            cx,
        )
        .map(|response| response.members.iter().filter_map(|member| member.as_domain()).collect())
        .unwrap_or_default()
    }

    pub fn playlist_member_counts(
        &self,
        playlist_id: &str,
        include_suspended: bool,
        cx: &mut App,
    ) -> Option<(i64, i64)> {
        if MusicGuideSandboxHooks::members(playlist_id, cx).is_some() {
            return None;
        }
        let cached = self.cached_dto::<WirePlaylistMembersDTO>(
            &Self::playlist_members_cache_key(playlist_id, include_suspended),
            cx,
        )?;
        let active = cached.active_participant_count;
        Some((active.max(0), active.max(cached.total_participant_count)))
    }

    pub fn playlist_listening_snapshot(
        &self,
        playlist_id: &str,
        track_id: Option<&str>,
        cx: &mut App,
    ) -> Option<MusicPlaylistListeningSnapshot> {
        if let Some(staged) = MusicGuideSandboxHooks::listening_snapshot(playlist_id, cx) {
            return Some(staged);
        }
        if let Some(exact) = self
            .cached_dto::<WirePlaylistListeningSnapshotDTO>(
                &Self::playlist_listening_snapshot_cache_key(playlist_id, track_id),
                cx,
            )
            .map(|dto| dto.as_domain(playlist_id))
        {
            return Some(exact);
        }

        let track_id = track_id.filter(|id| !id.is_empty())?;
        let playlist_wide = self
            .cached_dto::<WirePlaylistListeningSnapshotDTO>(
                &Self::playlist_listening_snapshot_cache_key(playlist_id, None),
                cx,
            )?
            .as_domain(playlist_id);

        let listeners: Vec<MusicPlaylistListener> =
            playlist_wide.listeners.into_iter().filter(|listener| listener.track_id == track_id).collect();
        Some(MusicPlaylistListeningSnapshot {
            total_listener_count: listeners.len() as i64,
            listeners,
            presence_sequence: playlist_wide.presence_sequence,
            server_time_ms: playlist_wide.server_time_ms,
            active_session: playlist_wide.active_session,
            watcher_lease_until_ms: playlist_wide.watcher_lease_until_ms,
            unchanged: playlist_wide.unchanged,
        })
    }

    pub fn playlist_listeners(
        &self,
        playlist_id: &str,
        track_id: Option<&str>,
        cx: &mut App,
    ) -> Vec<MusicPlaylistListener> {
        self.playlist_listening_snapshot(playlist_id, track_id, cx)
            .map(|snapshot| snapshot.listeners)
            .unwrap_or_default()
    }

    pub fn playlist_listening_session(&self, session_id: &str, cx: &mut App) -> Option<MusicPlaylistListeningSession> {
        self.cached_dto::<WirePlaylistListeningSessionResponseDTO>(
            &Self::playlist_listening_session_cache_key(session_id),
            cx,
        )
        .and_then(|response| response.session)
        .and_then(|session| session.as_domain())
    }

    pub fn active_playlist_listening_session(
        &self,
        playlist_id: &str,
        cx: &mut App,
    ) -> Option<MusicPlaylistListeningSession> {
        if let Some(session) =
            self.playlist_listening_snapshot(playlist_id, None, cx).and_then(|snapshot| snapshot.active_session)
        {
            return Some(session);
        }

        let observed_track_ids: Vec<Option<String>> = self
            .listening_observations
            .values()
            .filter(|observation| observation.playlist_id == playlist_id)
            .map(|observation| observation.track_id.clone())
            .collect();
        for track_id in observed_track_ids {
            if let Some(session) = self
                .playlist_listening_snapshot(playlist_id, track_id.as_deref(), cx)
                .and_then(|snapshot| snapshot.active_session)
            {
                return Some(session);
            }
        }
        None
    }

    pub fn estimated_playlist_server_time_ms(&self, playlist_id: &str, local_now_ms: i64) -> i64 {
        local_now_ms + self.server_clock_offset_ms_by_playlist.get(playlist_id).copied().unwrap_or(0.0).round() as i64
    }

    pub fn projected_position_ms_for_listener(
        &self,
        listener: &MusicPlaylistListener,
        local_now_ms: i64,
        rate_multiplier: f64,
    ) -> i64 {
        if !listener.is_playing {
            return listener.anchor_position_ms.max(0);
        }
        let server_now = self.estimated_playlist_server_time_ms(&listener.playlist_id, local_now_ms);
        let elapsed_ms = (server_now - listener.anchor_server_time_ms).max(0) as f64;
        let advanced_ms = (elapsed_ms * rate_multiplier.max(0.1)).round() as i64;
        (listener.anchor_position_ms + advanced_ms).max(0)
    }

    pub fn projected_position_ms_for_session(&self, session: &MusicPlaylistListeningSession, local_now_ms: i64) -> i64 {
        if !session.is_playing {
            return session.anchor_position_ms.max(0);
        }
        let server_now = self.estimated_playlist_server_time_ms(&session.playlist_id, local_now_ms);
        let elapsed_ms = (server_now - session.anchor_server_time_ms).max(0) as f64;
        let advanced_ms = (elapsed_ms * session.playback_rate_multiplier()).round() as i64;
        (session.anchor_position_ms + advanced_ms).max(0)
    }

    pub fn pagination_state(&self, cache_key: &str, cx: &mut App) -> MusicPaginationState {
        let payload = SaiWsResponseCache::observed_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        let Some(root) = AnyDictionaryBridge::dictionary(payload.as_ref()) else {
            return MusicPaginationState { cursor: None, has_more: false };
        };
        let raw = root.get(key::camel::NEXT_CURSOR).or_else(|| root.get(key::NEXT_CURSOR));
        let cursor = match raw {
            Some(Value::String(value)) => {
                let normalized = trimming_whitespaces_and_newlines(value);
                if normalized.is_empty() { None } else { Some(normalized.to_owned()) }
            }
            Some(Value::Number(value)) => Some(value.to_string()),
            Some(Value::Bool(value)) => Some(if *value { 1 } else { 0 }.to_string()),
            _ => None,
        };
        let has_more = cursor.is_some();
        MusicPaginationState { cursor, has_more }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn paginated_read_options(
        &self,
        method: &str,
        cache_key: &str,
        cursor: Option<&str>,
        reload_if_cached: bool,
        reflect_pending_in_ui: bool,
        path_to_array: &str,
        path_to_id: &str,
    ) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: Some(cache_key.to_owned()),
            reload_if_cached,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            reload_when_only_disk_cached: cursor.is_none() && reload_if_cached,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(path_to_array.to_owned()),
            path_to_id: Some(path_to_id.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id: cursor.map(str::to_owned),
                response_relative_id_path: key::camel::NEXT_CURSOR.to_owned(),
                has_more_path: None,
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        })
    }

    fn mark_slice<T: Serialize + DeserializeOwned + 'static>(slice: &SaiTypedSlice<T>, dto: &T) {
        let Some(dict) = SaiJSONCodec::encode(dto) else { return };
        slice.mark_fulfilled(Some(&Value::Object(dict)), None);
    }

    pub fn clear_search(&self, cx: &mut App) {
        let slice = MusicActionsStore::slices(cx).search_sai.clone();
        Self::mark_slice(&slice, &WireSearchDTO::EMPTY);
    }

    pub fn clear_search_artists(&self, cx: &mut App) {
        let slice = MusicActionsStore::slices(cx).search_artists_sai.clone();
        Self::mark_slice(&slice, &WireSearchDTO::EMPTY);
    }

    fn track_like_patch(&self, is_liked: bool, like_count: Option<i64>) -> Map<String, Value> {
        let mut patch = Map::new();
        patch.insert(key::IS_LIKED.to_owned(), Value::Bool(is_liked));
        if let Some(like_count) = like_count {
            patch.insert(key::LIKE_COUNT.to_owned(), Value::from(like_count));
        }
        patch
    }

    fn artist_follow_patch(&self, is_following: bool) -> Map<String, Value> {
        let mut patch = Map::new();
        patch.insert(key::IS_FOLLOWING.to_owned(), Value::Bool(is_following));
        patch
    }

    fn patch_track_row_in_caches(&self, track_id: &str, is_liked: bool, like_count: Option<i64>, cx: &mut App) {
        let patch = self.track_like_patch(is_liked, like_count);
        let keys = [
            (SaiCache::new(["getLiked"]).key, key::TRACKS),
            (SaiCache::new(["getRecent"]).key, key::TRACKS),
            (SaiCache::new(["listMyTracks"]).key, key::TRACKS),
            (Self::my_local_tracks_cache_key(), key::TRACKS),
        ];
        for (cache_key, path_to_array) in keys {
            let _ = SaiUpdater::patch_row(
                &cache_key,
                path_to_array,
                key::ID,
                track_id,
                &patch,
                SaiUpdaterWriteScope::All,
                cx,
            );
        }
    }

    pub fn music_user_role(&self, cx: &App) -> String {
        let role = AuthServicesStore::try_read(cx)
            .and_then(|auth| auth.profile())
            .map(|profile| profile.more.role.clone())
            .unwrap_or_default();
        trimming_whitespaces_and_newlines(&role).to_uppercase()
    }
}

impl MusicServicesStore {
    pub fn music_is_staff(&self, cx: &App) -> bool {
        let role = self.music_user_role(cx);
        role == PlatformRole::Admin.wire() || role == PlatformRole::Moderator.wire()
    }

    pub fn music_is_admin(&self, cx: &App) -> bool {
        self.music_user_role(cx) == PlatformRole::Admin.wire()
    }

    pub fn music_is_approved_artist(&self, cx: &mut App) -> bool {
        MusicActionsStore::slices(cx)
            .get_artist_status_sai
            .data()
            .is_some_and(|status| status.status == music_service::ARTIST_STATUS_APPROVED)
    }

    pub fn is_track_liked_track(&self, track: &TrackModel, cx: &mut App) -> bool {
        if let Some(override_value) = self.track_like_overrides.get(&track.id) {
            return *override_value;
        }
        let open_track = MusicActionsStore::slices(cx).get_track_sai.data().and_then(|response| response.track);
        if let Some(open_track) = open_track
            && open_track.id == track.id
        {
            return open_track.is_liked;
        }
        track.is_liked
    }

    pub fn is_track_liked(&self, track_id: &str, cx: &mut App) -> bool {
        let id = trimming_whitespaces_and_newlines(track_id);
        if id.is_empty() {
            return false;
        }
        if let Some(override_value) = self.track_like_overrides.get(id) {
            return *override_value;
        }
        let actions = MusicActionsStore::slices(cx);
        let liked = actions.get_liked_sai.data().map(|page| page.tracks).unwrap_or_default();
        if liked.iter().any(|row| row.id == id) {
            return true;
        }
        let open_track = actions.get_track_sai.data().and_then(|response| response.track);
        if let Some(open_track) = open_track
            && open_track.id == id
        {
            return open_track.is_liked;
        }
        self.find_track_wire(id, cx).is_some_and(|row| row.is_liked == Some(true))
    }

    pub fn apply_optimistic_toggle_like(&mut self, track_id: &str, cx: &mut Context<Self>) {
        let id = trimming_whitespaces_and_newlines(track_id).to_owned();
        if id.is_empty() {
            return;
        }
        let next_liked = !self.is_track_liked(&id, cx);
        self.patch_liked_list(&id, next_liked, cx);
        self.patch_track_is_liked(&id, next_liked, None, cx);
    }

    pub fn apply_optimistic_toggle_artist_follow(&mut self, artist_id: &str, cx: &mut Context<Self>) {
        let id = trimming_whitespaces_and_newlines(artist_id).to_owned();
        if id.is_empty() {
            return;
        }
        let slices = MusicActionsStore::slices(cx);
        let current = [&slices.get_artist_sai, &slices.now_playing_artist_sai]
            .into_iter()
            .find_map(|slice| slice.data().and_then(|response| response.artist).filter(|artist| artist.id == id))
            .is_some_and(|artist| artist.is_following);
        self.patch_artist_is_following(&id, !current, cx);
    }

    pub fn apply_toggle_like_server_result(
        &mut self,
        track_id: &str,
        is_liked: Option<bool>,
        like_count: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let id = trimming_whitespaces_and_newlines(track_id).to_owned();
        if id.is_empty() {
            return;
        }
        if let Some(is_liked) = is_liked {
            self.patch_liked_list(&id, is_liked, cx);
            self.patch_track_is_liked(&id, is_liked, like_count, cx);
        }
    }

    pub fn draft_playlist_for_mutation(
        &self,
        title: &str,
        description: Option<&str>,
        visibility: &str,
        existing: Option<&RiseStoredMusicPlaylist>,
    ) -> RiseStoredMusicPlaylist {
        let same_title = existing.is_some_and(|existing| existing.title.as_deref() == Some(title));
        RiseStoredMusicPlaylist {
            id: existing.and_then(|existing| existing.id.clone()),
            owner_user_id: existing.and_then(|existing| existing.owner_user_id.clone()),
            title: Some(title.to_owned()),
            description: Some(description.unwrap_or_default().to_owned()),
            cover_url: existing.and_then(|existing| existing.cover_url.clone()),
            visibility: Some(visibility.to_owned()),
            tracks: existing.and_then(|existing| existing.tracks.clone()),
            created_at_ms: existing.and_then(|existing| existing.created_at_ms),
            updated_at_ms: existing.and_then(|existing| existing.updated_at_ms),
            revision: existing.and_then(|existing| existing.revision),
            is_default_local: if same_title { existing.and_then(|existing| existing.is_default_local) } else { Some(false) },
            viewer_role: existing.and_then(|existing| existing.viewer_role.clone()),
            can_add_tracks: existing.and_then(|existing| existing.can_add_tracks),
            active_member_count: existing.and_then(|existing| existing.active_member_count),
            new_tracks_on_top: existing.and_then(|existing| existing.new_tracks_on_top),
        }
    }

    pub fn patch_create_playlist_pending_cover(
        &self,
        slice: &SaiTypedSlice<RiseStoredMusicPlaylistMutation>,
        progress: f64,
        uploading: bool,
        draft: RiseStoredMusicPlaylist,
    ) {
        Self::mark_slice(
            slice,
            &RiseStoredMusicPlaylistMutation {
                playlist: Some(draft),
                cover_upload_progress: Some(progress),
                cover_uploading: Some(uploading),
            },
        );
    }

    pub fn patch_update_playlist_pending_cover(
        &self,
        slice: &SaiTypedSlice<RiseStoredMusicPlaylistMutation>,
        progress: f64,
        uploading: bool,
        draft: RiseStoredMusicPlaylist,
    ) {
        Self::mark_slice(
            slice,
            &RiseStoredMusicPlaylistMutation {
                playlist: Some(draft),
                cover_upload_progress: Some(progress),
                cover_uploading: Some(uploading),
            },
        );
    }

    fn cover_url_slot(done: &Map<String, Value>, slot_key: &str) -> anyhow::Result<Value> {
        let Some(url) = MusicCoverUpload::extract_remote_url(done) else {
            return Err(anyhow::Error::new(SaiWsOptimisticError::FileUploadInvalidSlotPayload));
        };
        let mut slot = Map::new();
        slot.insert(slot_key.to_owned(), Value::String(url));
        Ok(Value::Object(slot))
    }

    fn patch_playlist_pending_cover(
        slice: &SaiTypedSlice<RiseStoredMusicPlaylistMutation>,
        progress: f64,
        draft: &RiseStoredMusicPlaylist,
    ) {
        Self::mark_slice(
            slice,
            &RiseStoredMusicPlaylistMutation {
                playlist: Some(draft.clone()),
                cover_upload_progress: Some(progress),
                cover_uploading: Some(true),
            },
        );
    }

    pub fn make_playlist_cover_optimistic_files(
        &self,
        local_file: SaiWsOptimisticLocalFile,
        draft: RiseStoredMusicPlaylist,
        slice: Rc<SaiTypedSlice<RiseStoredMusicPlaylistMutation>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        _cx: &mut App,
    ) -> SaiWsOptimisticFiles {
        let mut files = SaiWsOptimisticFiles::new(MUSIC_FILE_CONTEXT.to_owned());
        files.compression = MediaCompressionConfiguration::cover();
        files.local_files = vec![local_file];
        files.merge_uploaded_slots_into_body_root = true;
        files.extract_uploaded_files = Some(Arc::new(|done: &Map<String, Value>, _: usize, _: &SaiWsOptimisticLocalFile, _: &mut App| {
            Self::cover_url_slot(done, key::camel::COVER_URL)
        }));
        let progress_draft = draft.clone();
        let progress_slice = Rc::clone(&slice);
        files.on_progress = Some(Arc::new(move |_, fraction: f64, _: &mut App| {
            let progress = (fraction * 100.0).clamp(0.0, 100.0);
            Self::patch_playlist_pending_cover(&progress_slice, progress, &progress_draft);
        }));
        let success_slice = Rc::clone(&slice);
        files.on_file_success = Some(Arc::new(move |_, _: &Map<String, Value>, _: &mut App| {
            Self::patch_playlist_pending_cover(&success_slice, 100.0, &draft);
        }));
        files.on_file_error = Some(Arc::new(move |_, error: &anyhow::Error, cx: &mut App| {
            slice.reset_to_idle();
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        }));
        files
    }

    pub fn patch_create_track_pending_upload(
        &self,
        slice: &SaiTypedSlice<RiseStoredMusicUpload>,
        title: &str,
        cover_progress: Option<f64>,
        cover_uploading: Option<bool>,
        audio_progress: Option<f64>,
        audio_uploading: Option<bool>,
    ) {
        let prev = slice.data();
        Self::mark_slice(
            slice,
            &RiseStoredMusicUpload {
                title: Some(title.to_owned()),
                cover_upload_progress: cover_progress.or_else(|| prev.as_ref().and_then(|prev| prev.cover_upload_progress)),
                cover_uploading: cover_uploading.or_else(|| prev.as_ref().and_then(|prev| prev.cover_uploading)),
                audio_upload_progress: audio_progress.or_else(|| prev.as_ref().and_then(|prev| prev.audio_upload_progress)),
                audio_uploading: audio_uploading.or_else(|| prev.as_ref().and_then(|prev| prev.audio_uploading)),
                track: None,
                request: None,
            },
        );
    }

    pub fn patch_request_album_pending_upload(
        &self,
        slice: &SaiTypedSlice<RiseStoredMusicUpload>,
        title: &str,
        progress: f64,
        uploading: bool,
    ) {
        Self::mark_slice(
            slice,
            &RiseStoredMusicUpload {
                title: Some(title.to_owned()),
                cover_upload_progress: Some(progress),
                cover_uploading: Some(uploading),
                ..RiseStoredMusicUpload::default()
            },
        );
    }

    pub fn make_scalar_cover_optimistic_files(
        &self,
        local_file: SaiWsOptimisticLocalFile,
        on_progress: Rc<dyn Fn(f64, &mut App)>,
        on_file_error: Rc<dyn Fn(&anyhow::Error, &mut App)>,
        reject_slice: Rc<dyn Fn(&anyhow::Error, &mut App)>,
        _cx: &mut App,
    ) -> SaiWsOptimisticFiles {
        let mut files = SaiWsOptimisticFiles::new(MUSIC_FILE_CONTEXT.to_owned());
        files.compression = MediaCompressionConfiguration::cover();
        files.local_files = vec![local_file];
        files.merge_uploaded_slots_into_body_root = true;
        files.extract_uploaded_files = Some(Arc::new(|done: &Map<String, Value>, _: usize, _: &SaiWsOptimisticLocalFile, _: &mut App| {
            Self::cover_url_slot(done, key::camel::COVER_URL)
        }));
        let progress_handler = Rc::clone(&on_progress);
        files.on_progress = Some(Arc::new(move |_, fraction: f64, cx: &mut App| {
            progress_handler((fraction * 100.0).clamp(0.0, 100.0), cx);
        }));
        files.on_file_success = Some(Arc::new(move |_, _: &Map<String, Value>, cx: &mut App| {
            on_progress(100.0, cx);
        }));
        files.on_file_error = Some(Arc::new(move |_, error: &anyhow::Error, cx: &mut App| {
            reject_slice(error, cx);
            on_file_error(error, cx);
        }));
        files
    }

    fn patch_create_track_pending_upload_shared(
        slice: &SaiTypedSlice<RiseStoredMusicUpload>,
        title: &str,
        cover_progress: Option<f64>,
        cover_uploading: Option<bool>,
        audio_progress: Option<f64>,
        audio_uploading: Option<bool>,
        cx: &mut App,
    ) {
        MusicServicesStore::shared(cx).update(cx, |services, _| {
            services.patch_create_track_pending_upload(
                slice,
                title,
                cover_progress,
                cover_uploading,
                audio_progress,
                audio_uploading,
            );
        });
    }

    pub fn make_create_track_optimistic_files(
        &self,
        cover_file: SaiWsOptimisticLocalFile,
        audio_file: SaiWsOptimisticLocalFile,
        title: &str,
        slice: Rc<SaiTypedSlice<RiseStoredMusicUpload>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        _cx: &mut App,
    ) -> SaiWsOptimisticFiles {
        let mut files = SaiWsOptimisticFiles::new(MUSIC_FILE_CONTEXT.to_owned());
        files.compression = MediaCompressionConfiguration::cover();
        files.local_files = vec![cover_file, audio_file];
        files.merge_uploaded_slots_into_body_root = true;
        files.extract_uploaded_files = Some(Arc::new(|done: &Map<String, Value>, index: usize, _: &SaiWsOptimisticLocalFile, _: &mut App| {
            if index == 0 {
                Self::cover_url_slot(done, key::camel::COVER_URL)
            } else {
                Self::cover_url_slot(done, key::camel::AUDIO_URL)
            }
        }));
        let progress_title = title.to_owned();
        let progress_slice = Rc::clone(&slice);
        files.on_progress = Some(Arc::new(move |index: usize, fraction: f64, cx: &mut App| {
            let progress = (fraction * 100.0).clamp(0.0, 100.0);
            if index == 0 {
                Self::patch_create_track_pending_upload_shared(
                    &progress_slice,
                    &progress_title,
                    Some(progress),
                    Some(true),
                    None,
                    Some(false),
                    cx,
                );
            } else {
                Self::patch_create_track_pending_upload_shared(
                    &progress_slice,
                    &progress_title,
                    None,
                    Some(false),
                    Some(progress),
                    Some(true),
                    cx,
                );
            }
        }));
        let success_title = title.to_owned();
        let success_slice = Rc::clone(&slice);
        files.on_file_success = Some(Arc::new(move |index: usize, _: &Map<String, Value>, cx: &mut App| {
            if index == 0 {
                Self::patch_create_track_pending_upload_shared(&success_slice, &success_title, Some(100.0), Some(true), None, None, cx);
            } else {
                Self::patch_create_track_pending_upload_shared(&success_slice, &success_title, None, None, Some(100.0), Some(true), cx);
            }
        }));
        files.on_file_error = Some(Arc::new(move |_, error: &anyhow::Error, cx: &mut App| {
            slice.reset_to_idle();
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        }));
        files
    }

    pub fn preload_slice_from_disk_if_needed<T: DeserializeOwned + Clone + 'static>(
        &self,
        slice: &Rc<SaiTypedSlice<T>>,
        cache_key: &str,
        cx: &mut App,
    ) {
        SaiUpdater::hydrate_slice_from_disk(slice, cache_key, None, cx);
    }
}

impl MusicServicesStore {
    pub fn liked_tracks_total_count(&self, cx: &mut App) -> i64 {
        let page = MusicActionsStore::slices(cx).get_liked_sai.data();
        let loaded = page.as_ref().map_or(0, |page| page.tracks.len()) as i64;
        let Some(page) = page else { return loaded };
        let (Some(_), Some(total)) = (page.next_cursor.as_ref(), page.total_count) else { return loaded };
        total.max(loaded)
    }

    fn shift_liked_total_count(&self, delta: i64, cx: &mut App) {
        let Some(total) = MusicActionsStore::slices(cx).get_liked_sai.data().and_then(|page| page.total_count) else {
            return;
        };
        let shifted = (total + delta).max(0);
        let mut fields = Map::new();
        fields.insert(key::camel::TOTAL_COUNT.to_owned(), Value::from(shifted));
        fields.insert(key::TOTAL_COUNT.to_owned(), Value::from(shifted));
        let _ = SaiUpdater::patch_root(&Self::liked_cache_key(), &fields, SaiUpdaterWriteScope::All, cx);
    }

    fn patch_liked_list(&self, track_id: &str, is_liked: bool, cx: &mut App) {
        let cache_key = SaiCache::new(["getLiked"]).key;
        let was_listed = MusicActionsStore::slices(cx)
            .get_liked_sai
            .data()
            .is_some_and(|page| page.tracks.iter().any(|row| row.id == track_id));
        if is_liked {
            let Some(wire) = self.find_track_wire(track_id, cx) else { return };
            let Some(row) = SaiJSONCodec::encode(&wire.with(Some(true), None, None)) else { return };
            if !was_listed {
                self.shift_liked_total_count(1, cx);
            }
            let _ = SaiUpdater::insert_row_if_missing(
                &cache_key,
                key::TRACKS,
                key::ID,
                row,
                true,
                SaiUpdaterWriteScope::All,
                cx,
            );
            let _ = SaiUpdater::patch_row(
                &cache_key,
                key::TRACKS,
                key::ID,
                track_id,
                &self.track_like_patch(true, None),
                SaiUpdaterWriteScope::All,
                cx,
            );
        } else {
            if was_listed {
                self.shift_liked_total_count(-1, cx);
            }
            let _ = SaiUpdater::remove_row(&cache_key, key::TRACKS, key::ID, track_id, SaiUpdaterWriteScope::All, cx);
        }
    }

    fn patch_track_is_liked(&mut self, track_id: &str, is_liked: bool, like_count: Option<i64>, cx: &mut App) {
        self.track_like_overrides.insert(track_id.to_owned(), is_liked);
        self.patch_track_row_in_caches(track_id, is_liked, like_count, cx);

        let patch = |t: &RiseStoredMusicTrack| -> RiseStoredMusicTrack {
            if t.id.as_deref().unwrap_or("") != track_id {
                return t.clone();
            }
            t.with(Some(is_liked), like_count, None)
        };
        let patch_canon = |t: rise_proto::music::ProtoTrackRecord| -> rise_proto::music::ProtoTrackRecord {
            if t.id == track_id { t.with(is_liked, like_count) } else { t }
        };

        let (list_tracks, list_artist_tracks, search, get_track, get_artist, playlists_mine, get_playlist, get_album) = {
            let actions = MusicActionsStore::slices(cx);
            (
                actions.list_tracks_sai.clone(),
                actions.list_artist_tracks_sai.clone(),
                actions.search_sai.clone(),
                actions.get_track_sai.clone(),
                actions.get_artist_sai.clone(),
                actions.get_playlists_mine_sai.clone(),
                actions.get_playlist_sai.clone(),
                actions.get_album_sai.clone(),
            )
        };

        if let Some(data) = list_tracks.data() {
            Self::mark_slice(&list_tracks, &data.map_tracks(&patch_canon));
        }
        if let Some(data) = list_artist_tracks.data() {
            Self::mark_slice(&list_artist_tracks, &data.map_tracks(&patch_canon));
        }
        if let Some(data) = search.data() {
            Self::mark_slice(&search, &data.map_tracks(&patch_canon));
        }
        if let Some(mut data) = get_track.data()
            && let Some(track) = data.track.clone()
            && track.id == track_id
        {
            let patched = patch_canon(track.clone());
            data.track = Some(patched.clone());
            Self::mark_slice(&get_track, &data);
            if !track.id.is_empty()
                && let Some(row) = SaiJSONCodec::encode(&patched)
            {
                let mut fields = Map::new();
                fields.insert(key::TRACK.to_owned(), Value::Object(row));
                let _ = SaiUpdater::patch_root(
                    &SaiCache::new(["getTrack", track.id.as_str()]).key,
                    &fields,
                    SaiUpdaterWriteScope::All,
                    cx,
                );
            }
        }
        if let Some(data) = get_artist.data() {
            let mut updated = data.clone();
            updated.top_tracks = data.top_tracks.iter().cloned().map(&patch_canon).collect();
            updated.albums = data
                .albums
                .iter()
                .map(|al| {
                    let mut album = al.clone();
                    album.tracks = al.tracks.iter().cloned().map(&patch_canon).collect();
                    album
                })
                .collect();
            Self::mark_slice(&get_artist, &updated);
        }
        if let Some(data) = playlists_mine.data() {
            Self::mark_slice(
                &playlists_mine,
                &data.map_playlists(|pl| {
                    let Some(ts) = &pl.tracks else { return pl };
                    let tracks = ts.iter().map(&patch).collect();
                    RiseStoredMusicPlaylist { tracks: Some(tracks), ..pl }
                }),
            );
            let row_patch = self.track_like_patch(is_liked, like_count);
            let _ = SaiUpdater::update(
                &SaiCache::new(["getPlaylistsMine"]).key,
                |ctx| {
                    let mut root = AnyDictionaryBridge::dictionary(ctx.payload.as_ref()).unwrap_or_default();
                    let Some(playlists) = Self::json_dictionaries(root.get(key::PLAYLISTS)) else {
                        return Ok(serde_json::to_vec(&Value::Object(root))?);
                    };
                    let playlists: Vec<Value> = playlists
                        .into_iter()
                        .map(|pl| {
                            let Some(tracks) = Self::json_dictionaries(pl.get(key::TRACKS)) else {
                                return Value::Object(pl);
                            };
                            let tracks: Vec<Value> = tracks
                                .into_iter()
                                .map(|row| {
                                    if row.get(key::ID).and_then(Value::as_str) != Some(track_id) {
                                        return Value::Object(row);
                                    }
                                    let mut merged = row;
                                    for (k, v) in &row_patch {
                                        merged.insert(k.clone(), v.clone());
                                    }
                                    Value::Object(merged)
                                })
                                .collect();
                            let mut out = pl;
                            out.insert(key::TRACKS.to_owned(), Value::Array(tracks));
                            Value::Object(out)
                        })
                        .collect();
                    root.insert(key::PLAYLISTS.to_owned(), Value::Array(playlists));
                    Ok(serde_json::to_vec(&Value::Object(root))?)
                },
                SaiUpdaterWriteScope::All,
                cx,
            );
        }
        if let Some(data) = get_playlist.data()
            && let Some(pl) = data.playlist
            && let Some(ts) = pl.tracks.clone()
        {
            let pid = pl.id.clone();
            let updated_playlist = RiseStoredMusicPlaylist { tracks: Some(ts.iter().map(&patch).collect()), ..pl };
            let playlist_row = SaiJSONCodec::encode(&updated_playlist);
            Self::mark_slice(&get_playlist, &RiseStoredMusicPlaylistDetail { playlist: Some(updated_playlist) });
            if let Some(pid) = pid
                && !pid.is_empty()
                && let Some(playlist_row) = playlist_row
            {
                let mut fields = Map::new();
                fields.insert(key::PLAYLIST.to_owned(), Value::Object(playlist_row));
                let _ = SaiUpdater::patch_root(
                    &SaiCache::new(["getPlaylist", pid.as_str()]).key,
                    &fields,
                    SaiUpdaterWriteScope::All,
                    cx,
                );
            }
        }
        if let Some(data) = get_album.data() {
            Self::mark_slice(&get_album, &data.map_tracks(&patch_canon));
        }
    }

    fn json_dictionaries(value: Option<&Value>) -> Option<Vec<Map<String, Value>>> {
        value?.as_array()?.iter().map(|row| row.as_object().cloned()).collect()
    }

    fn patch_artist_is_following(&self, artist_id: &str, is_following: bool, cx: &mut App) {
        let patch = self.artist_follow_patch(is_following);
        let _ = SaiUpdater::patch_row(
            &SaiCache::new(["listArtists"]).key,
            key::ARTISTS,
            key::ID,
            artist_id,
            &patch,
            SaiUpdaterWriteScope::All,
            cx,
        );

        let (get_artist, now_playing_artist, list_artists, search, search_artists) = {
            let actions = MusicActionsStore::slices(cx);
            (
                actions.get_artist_sai.clone(),
                actions.now_playing_artist_sai.clone(),
                actions.list_artists_sai.clone(),
                actions.search_sai.clone(),
                actions.search_artists_sai.clone(),
            )
        };

        if let Some(data) = get_artist.data()
            && let Some(artist) = data.artist.clone()
            && artist.id == artist_id
        {
            let updated = artist.with(Some(is_following));
            Self::mark_slice(&get_artist, &data.map_artist(|_| updated.clone()));
            if let Some(row) = SaiJSONCodec::encode(&updated) {
                let mut fields = Map::new();
                fields.insert(key::ARTIST.to_owned(), Value::Object(row));
                let _ = SaiUpdater::patch_root(
                    &SaiCache::new(["getArtist", artist_id]).key,
                    &fields,
                    SaiUpdaterWriteScope::All,
                    cx,
                );
            }
        }
        if let Some(data) = now_playing_artist.data()
            && let Some(artist) = data.artist.clone()
            && artist.id == artist_id
        {
            let updated = artist.with(Some(is_following));
            Self::mark_slice(&now_playing_artist, &data.map_artist(|_| updated.clone()));
        }
        let patch_row = |artist: WireArtistDTO| -> WireArtistDTO {
            if artist.id == artist_id { artist.with(Some(is_following)) } else { artist }
        };
        if let Some(data) = list_artists.data() {
            Self::mark_slice(&list_artists, &data.map_artists(&patch_row));
        }
        if let Some(data) = search.data() {
            Self::mark_slice(&search, &data.map_artists(&patch_row));
        }
        if let Some(data) = search_artists.data() {
            Self::mark_slice(&search_artists, &data.map_artists(&patch_row));
        }
    }

    pub fn apply_toggle_follow_server_result(&self, artist_id: &str, is_following: Option<bool>, cx: &mut App) {
        let id = trimming_whitespaces_and_newlines(artist_id);
        let Some(is_following) = is_following else { return };
        if id.is_empty() {
            return;
        }
        self.patch_artist_is_following(id, is_following, cx);
    }

    pub fn upsert_playlist_mine(&self, playlist: &RiseStoredMusicPlaylist, cx: &mut App) {
        let Some(id) = playlist.id.clone().filter(|id| !id.is_empty()) else { return };
        let Some(row) = SaiJSONCodec::encode(playlist) else { return };
        let cache_key = SaiCache::new(["getPlaylistsMine"]).key;
        let listed = MusicActionsStore::slices(cx)
            .get_playlists_mine_sai
            .data()
            .and_then(|library| library.playlists)
            .is_some_and(|playlists| playlists.iter().any(|row| row.id.as_deref().unwrap_or("") == id));
        if listed {
            let _ = SaiUpdater::patch_row(
                &cache_key,
                key::PLAYLISTS,
                key::ID,
                &id,
                &row,
                SaiUpdaterWriteScope::All,
                cx,
            );
        } else {
            let _ = SaiUpdater::insert_row_if_missing(
                &cache_key,
                key::PLAYLISTS,
                key::ID,
                row,
                true,
                SaiUpdaterWriteScope::All,
                cx,
            );
        }
    }

    pub fn patch_playlist_detail(&self, playlist: &RiseStoredMusicPlaylist, cx: &mut App) {
        let Some(id) = playlist.id.clone().filter(|id| !id.is_empty()) else { return };
        self.upsert_playlist_mine(playlist, cx);

        if let Some(row) = SaiJSONCodec::encode(playlist) {
            let mut fields = Map::new();
            fields.insert(key::PLAYLIST.to_owned(), Value::Object(row));
            let _ = SaiUpdater::patch_root(
                &SaiCache::new(["getPlaylist", id.as_str()]).key,
                &fields,
                SaiUpdaterWriteScope::All,
                cx,
            );
        }

        let get_playlist = MusicActionsStore::slices(cx).get_playlist_sai.clone();
        let open_id = get_playlist.data().and_then(|detail| detail.playlist).and_then(|open| open.id);
        if open_id.as_deref().unwrap_or("") == id {
            Self::mark_slice(&get_playlist, &RiseStoredMusicPlaylistDetail { playlist: Some(playlist.clone()) });
        }
    }

    pub fn revert_delete_playlist_optimistic(&self, playlist: &RiseStoredMusicPlaylist, cx: &mut App) {
        self.upsert_playlist_mine(playlist, cx);
    }

    pub fn revert_delete_track_optimistic(&self, track: &RiseStoredMusicTrack, cx: &mut App) {
        self.insert_my_track(track, cx);
    }

    pub fn apply_update_track_exclusive_optimistic(&self, track_id: &str, is_exclusive: bool, cx: &mut App) {
        let current = MusicActionsStore::slices(cx)
            .list_my_tracks_sai
            .data()
            .and_then(|page| page.tracks.into_iter().find(|row| row.id == track_id));
        let Some(current) = current else { return };
        self.insert_my_track(&current.stored().with(None, None, Some(is_exclusive)), cx);
    }

    pub fn insert_my_track(&self, track: &RiseStoredMusicTrack, cx: &mut App) {
        let Some(id) = track.id.clone().filter(|id| !id.is_empty()) else { return };
        let Some(row) = SaiJSONCodec::encode(track) else { return };
        let cache_key = SaiCache::new(["listMyTracks"]).key;
        let listed = MusicActionsStore::slices(cx)
            .list_my_tracks_sai
            .data()
            .is_some_and(|page| page.tracks.iter().any(|row| row.id == id));
        if listed {
            let _ = SaiUpdater::patch_row(&cache_key, key::TRACKS, key::ID, &id, &row, SaiUpdaterWriteScope::All, cx);
        } else {
            let _ = SaiUpdater::insert_row_if_missing(
                &cache_key,
                key::TRACKS,
                key::ID,
                row,
                true,
                SaiUpdaterWriteScope::All,
                cx,
            );
        }
    }

    pub fn insert_my_album_request(&self, request: &WireAlbumRequestDTO, cx: &mut App) {
        let id = request.id.clone();
        if id.is_empty() {
            return;
        }
        let Some(row) = SaiJSONCodec::encode(request) else { return };
        let cache_key = SaiCache::new(["getMyAlbumRequests"]).key;
        let listed = MusicActionsStore::slices(cx)
            .get_my_album_requests_sai
            .data()
            .is_some_and(|page| page.requests.iter().any(|row| row.id == id));
        if listed {
            let _ = SaiUpdater::patch_row(&cache_key, key::REQUESTS, key::ID, &id, &row, SaiUpdaterWriteScope::All, cx);
        } else {
            let _ = SaiUpdater::insert_row_if_missing(
                &cache_key,
                key::REQUESTS,
                key::ID,
                row,
                true,
                SaiUpdaterWriteScope::All,
                cx,
            );
        }
    }

    pub fn patch_published_track(&self, track: &RiseStoredMusicTrack, cx: &mut App) {
        self.insert_my_track(track, cx);
    }

    pub fn patch_published_album(&self, album: &WireAlbumDTO, cx: &mut App) {
        let id = album.id.clone();
        if id.is_empty() {
            return;
        }
        let Some(row) = SaiJSONCodec::encode(album) else { return };
        let cache_key = SaiCache::new(["listMyAlbums"]).key;
        let listed = MusicActionsStore::slices(cx)
            .list_my_albums_sai
            .data()
            .is_some_and(|page| page.albums.iter().any(|row| row.id == id));
        if listed {
            let _ = SaiUpdater::patch_row(&cache_key, key::ALBUMS, key::ID, &id, &row, SaiUpdaterWriteScope::All, cx);
        } else {
            let _ = SaiUpdater::insert_row_if_missing(
                &cache_key,
                key::ALBUMS,
                key::ID,
                row,
                true,
                SaiUpdaterWriteScope::All,
                cx,
            );
        }
    }

    pub fn apply_add_track_to_playlist_optimistic(
        &self,
        playlist_id: &str,
        track_id: &str,
        cx: &mut App,
    ) -> Option<RiseStoredMusicPlaylist> {
        let pid = trimming_whitespaces_and_newlines(playlist_id);
        let tid = trimming_whitespaces_and_newlines(track_id);
        if pid.is_empty() || tid.is_empty() {
            return None;
        }
        let playlist = MusicActionsStore::slices(cx).get_playlist_sai.data().and_then(|detail| detail.playlist)?;
        if playlist.id.as_deref().unwrap_or("") != pid {
            return None;
        }
        let track = self.find_track_wire(tid, cx)?;
        let mut tracks = playlist.tracks.clone().unwrap_or_default();
        if tracks.iter().any(|row| row.id.as_deref().unwrap_or("") == tid) {
            return Some(playlist);
        }
        if playlist.new_tracks_on_top == Some(true) {
            tracks.insert(0, track);
        } else {
            tracks.push(track);
        }
        let updated = playlist.with(None, None, None, None, Some(tracks));
        self.patch_playlist_detail(&updated, cx);
        Some(updated)
    }

    pub fn revert_add_track_to_playlist_optimistic(&self, previous: &RiseStoredMusicPlaylist, cx: &mut App) {
        self.patch_playlist_detail(previous, cx);
    }

    pub fn apply_remove_track_from_playlist_optimistic(
        &self,
        playlist_id: &str,
        track_id: &str,
        cx: &mut App,
    ) -> Option<RiseStoredMusicPlaylist> {
        let pid = trimming_whitespaces_and_newlines(playlist_id);
        let tid = trimming_whitespaces_and_newlines(track_id);
        if pid.is_empty() || tid.is_empty() {
            return None;
        }
        let playlist = MusicActionsStore::slices(cx).get_playlist_sai.data().and_then(|detail| detail.playlist)?;
        if playlist.id.as_deref().unwrap_or("") != pid {
            return None;
        }
        let tracks: Vec<RiseStoredMusicTrack> = playlist
            .tracks
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|row| row.id.as_deref().unwrap_or("") != tid)
            .collect();
        let updated = playlist.with(None, None, None, None, Some(tracks));
        self.patch_playlist_detail(&updated, cx);
        Some(updated)
    }

    pub fn revert_remove_track_from_playlist_optimistic(&self, previous: &RiseStoredMusicPlaylist, cx: &mut App) {
        self.patch_playlist_detail(previous, cx);
    }

    pub fn apply_move_playlist_track_optimistic(
        &self,
        playlist_id: &str,
        track_id: &str,
        after_track_id: Option<&str>,
        cx: &mut App,
    ) -> Option<RiseStoredMusicPlaylist> {
        let playlist = MusicActionsStore::slices(cx).get_playlist_sai.data().and_then(|detail| detail.playlist)?;
        if playlist.id.as_deref().unwrap_or("") != playlist_id {
            return None;
        }
        let mut tracks = playlist.tracks.clone().unwrap_or_default();
        let from = tracks.iter().position(|row| row.id.as_deref().unwrap_or("") == track_id)?;
        let moved = tracks.remove(from);
        let to = match after_track_id {
            Some(after_track_id) => {
                tracks.iter().position(|row| row.id.as_deref().unwrap_or("") == after_track_id)? + 1
            }
            None => 0,
        };
        tracks.insert(to, moved);
        self.patch_playlist_detail(&playlist.with(None, None, None, None, Some(tracks)), cx);
        Some(playlist)
    }

    pub fn sync_artist_status_from_request_response(&self, cx: &mut App) {
        let Some(status) = MusicActionsStore::slices(cx).request_artist_sai.data() else { return };
        let Some(root) = SaiJSONCodec::encode(&status) else { return };
        let _ = SaiUpdater::replace_root(
            &SaiCache::new(["getArtistStatus"]).key,
            root,
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    pub fn moderation_cached_read_options(
        &self,
        method: &str,
        cache_key: Option<&str>,
        fetch_if_have_data: bool,
        path_to_array: Option<&str>,
        path_to_id: Option<&str>,
        store_in_cache: bool,
    ) -> SaiWsRequestOptions {
        let has_cache = !cache_key.unwrap_or("").is_empty();
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: cache_key.map(str::to_owned),
            reload_if_cached: fetch_if_have_data,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: store_in_cache && has_cache,
            persist_cache_to_disk: store_in_cache && has_cache,
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: path_to_array.map(str::to_owned),
            path_to_id: path_to_id.map(str::to_owned),
            ..SaiWsRequestOptionsInit::default()
        })
    }

    pub fn revert_artist_request(&self, request: &WireArtistRequestDTO, list_status: &str, cx: &mut App) {
        let Some(row) = SaiJSONCodec::encode(request) else { return };
        let _ = SaiUpdater::insert_row_if_missing(
            &SaiCache::new(["listArtistRequests", list_status]).key,
            key::REQUESTS,
            key::ID,
            row,
            true,
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    pub fn revert_track_request(&self, track: &RiseStoredMusicTrack, list_status: &str, cx: &mut App) {
        if track.id.as_deref().unwrap_or("").is_empty() {
            return;
        }
        let Some(row) = SaiJSONCodec::encode(track) else { return };
        let _ = SaiUpdater::insert_row_if_missing(
            &SaiCache::new(["listTrackRequests", list_status]).key,
            key::TRACKS,
            key::ID,
            row,
            true,
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    pub fn revert_album_request(&self, request: &WireAlbumRequestDTO, list_status: &str, cx: &mut App) {
        let Some(row) = SaiJSONCodec::encode(request) else { return };
        let _ = SaiUpdater::insert_row_if_missing(
            &SaiCache::new(["listAlbumRequests", list_status]).key,
            key::REQUESTS,
            key::ID,
            row,
            true,
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    pub fn remove_list_row(&self, list_cache_key: &str, path_to_array: &str, id: &str, cx: &mut App) {
        let _ = SaiUpdater::remove_row(list_cache_key, path_to_array, key::ID, id, SaiUpdaterWriteScope::All, cx);
    }

    fn local_tracks_dto(&self, cx: &mut App) -> WireLocalTracksListDTO {
        let payload = SaiWsResponseCache::observed_payload(
            &Self::my_local_tracks_cache_key(),
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        if let Some(cached) = SaiJSONCodec::decode::<WireLocalTracksListDTO>(payload.as_ref()) {
            return cached;
        }
        MusicActionsStore::slices(cx).list_my_local_tracks_sai.data().unwrap_or_default()
    }

    fn local_track_row(&self, track_id: &str, cx: &mut App) -> Option<RiseStoredMusicTrack> {
        let hit = self.local_tracks_dto(cx).tracks.into_iter().find(|row| row.id == track_id)?;
        let overridden =
            RiseMusicPresentation::shared(cx).read(cx).snapshot().track_overrides.get(track_id).cloned();
        Some(overridden.unwrap_or_else(|| hit.stored()))
    }

    pub fn local_tracks_page(&self, cx: &mut App) -> RiseStoredMusicLocalTracksPage {
        let page = self.local_tracks_dto(cx);
        RiseStoredMusicLocalTracksPage {
            tracks: self.applying_rise_music_overrides(Some(page.stored_tracks()), cx).unwrap_or_default(),
            next_cursor: page.next_cursor,
            has_more: page.has_more,
            total_count: page.total_count,
        }
    }

    pub fn local_tracks(&self, cx: &mut App) -> Vec<TrackModel> {
        self.local_tracks_page(cx).tracks.iter().filter_map(RiseStoredMusicTrack::as_track_model).collect()
    }

    pub fn local_tracks_total_count(&self, cx: &mut App) -> i64 {
        let page = self.local_tracks_page(cx);
        let loaded = page.tracks.len() as i64;
        let Some(total) = page.total_count.filter(|_| page.has_more) else { return loaded };
        total.max(loaded)
    }

    pub fn local_track_wire(&self, track_id: &str, cx: &mut App) -> Option<RiseStoredMusicTrack> {
        if let Some(track) =
            self.local_tracks_page(cx).tracks.into_iter().find(|row| row.id.as_deref() == Some(track_id))
        {
            return Some(track);
        }
        let actions = MusicActionsStore::slices(cx);
        if let Some(track) = actions
            .get_playlist_sai
            .data()
            .and_then(|detail| detail.playlist)
            .and_then(|playlist| playlist.tracks)
            .and_then(|tracks| tracks.into_iter().find(|row| row.id.as_deref() == Some(track_id)))
        {
            return Some(track);
        }
        for playlist in actions.get_playlists_mine_sai.data().and_then(|library| library.playlists).unwrap_or_default() {
            if let Some(track) =
                playlist.tracks.and_then(|tracks| tracks.into_iter().find(|row| row.id.as_deref() == Some(track_id)))
            {
                return Some(track);
            }
        }
        actions
            .get_track_sai
            .data()
            .and_then(|response| response.track)
            .and_then(|track| if track.id == track_id { Some(RiseStoredMusicTrack::from_canon(&track)) } else { None })
    }

    pub fn apply_local_track_update_projection(
        &self,
        track: &RiseStoredMusicTrack,
        persist_to_disk: bool,
        cx: &mut App,
    ) {
        let Some(track_id) = track.id.clone().filter(|id| !id.is_empty()) else { return };
        let replace = |tracks: Option<Vec<RiseStoredMusicTrack>>| -> Option<Vec<RiseStoredMusicTrack>> {
            let tracks = tracks?;
            if !tracks.iter().any(|row| row.id.as_deref() == Some(track_id.as_str())) {
                return None;
            }
            Some(
                tracks
                    .into_iter()
                    .map(|row| if row.id.as_deref() == Some(track_id.as_str()) { track.clone() } else { row })
                    .collect(),
            )
        };

        let (local_tracks, get_playlist, playlists_mine, get_track) = {
            let actions = MusicActionsStore::slices(cx);
            (
                actions.list_my_local_tracks_sai.clone(),
                actions.get_playlist_sai.clone(),
                actions.get_playlists_mine_sai.clone(),
                actions.get_track_sai.clone(),
            )
        };

        if let Some(page) = local_tracks.data()
            && let Some(tracks) = replace(Some(page.stored_tracks()))
        {
            Self::mark_slice(&local_tracks, &page.replacing_tracks(&tracks));
        }
        if let Some(detail) = get_playlist.data().and_then(|detail| detail.playlist)
            && let Some(tracks) = replace(detail.tracks.clone())
        {
            self.patch_playlist_detail(&detail.with(None, None, None, None, Some(tracks)), cx);
        }
        if let Some(mine) = playlists_mine.data() {
            let playlists = mine
                .playlists
                .unwrap_or_default()
                .into_iter()
                .map(|playlist| match replace(playlist.tracks.clone()) {
                    Some(tracks) => playlist.with(None, None, None, None, Some(tracks)),
                    None => playlist,
                })
                .collect();
            Self::mark_slice(&playlists_mine, &RiseStoredMusicPlaylistLibrary { playlists: Some(playlists) });
        }
        if get_track.data().and_then(|response| response.track).is_some_and(|open| open.id == track_id) {
            Self::mark_slice(
                &get_track,
                &WireSingleTrackDTO { error: String::new(), success: true, track: Some(track.canon()) },
            );
        }

        if persist_to_disk && let Some(row) = SaiJSONCodec::encode(track) {
            let _ = SaiUpdater::patch_row(
                &Self::my_local_tracks_cache_key(),
                key::TRACKS,
                key::ID,
                &track_id,
                &row,
                SaiUpdaterWriteScope::All,
                cx,
            );
            if let Some(playlist_id) = get_playlist.data().and_then(|detail| detail.playlist).and_then(|open| open.id) {
                let _ = SaiUpdater::patch_row(
                    &SaiCache::new(["getPlaylist", playlist_id.as_str()]).key,
                    &format!("{}.{}", key::PLAYLIST, key::TRACKS),
                    key::ID,
                    &track_id,
                    &row,
                    SaiUpdaterWriteScope::All,
                    cx,
                );
            }
        }
    }

    fn applying_rise_music_overrides(
        &self,
        tracks: Option<Vec<RiseStoredMusicTrack>>,
        cx: &mut App,
    ) -> Option<Vec<RiseStoredMusicTrack>> {
        let tracks = tracks?;
        let presentation = RiseMusicPresentation::shared(cx);
        let overrides = &presentation.read(cx).snapshot().track_overrides;
        if overrides.is_empty() {
            return Some(tracks);
        }
        Some(
            tracks
                .into_iter()
                .map(|track| match track.id.as_ref().and_then(|id| overrides.get(id)) {
                    Some(overridden) => overridden.clone(),
                    None => track,
                })
                .collect(),
        )
    }

    fn applying_rise_music_overrides_to_playlist(
        &self,
        playlist: Option<&RiseStoredMusicPlaylist>,
        cx: &mut App,
    ) -> Option<RiseStoredMusicPlaylist> {
        let playlist = playlist?;
        Some(playlist.with(None, None, None, None, self.applying_rise_music_overrides(playlist.tracks.clone(), cx)))
    }

    fn removing_local_track_optimistic_rows(playlist: Option<&RiseStoredMusicPlaylist>) -> Option<RiseStoredMusicPlaylist> {
        let playlist = playlist?;
        let tracks =
            playlist.tracks.clone().unwrap_or_default().into_iter().filter(|row| row.transfer_phase.is_none()).collect();
        Some(playlist.with(None, None, None, None, Some(tracks)))
    }

    fn find_track_wire(&self, track_id: &str, cx: &mut App) -> Option<RiseStoredMusicTrack> {
        let local = {
            let actions = MusicActionsStore::slices(cx);
            if let Some(t) = actions.get_track_sai.data().and_then(|response| response.track)
                && t.id == track_id
            {
                return Some(RiseStoredMusicTrack::from_canon(&t));
            }
            let lists = [
                actions.get_liked_sai.data().map(|page| page.tracks),
                actions.list_tracks_sai.data().map(|page| page.tracks),
                actions.get_recent_sai.data().map(|page| page.tracks),
                actions.search_sai.data().map(|page| page.tracks),
                actions.get_artist_sai.data().map(|page| page.top_tracks),
                actions.list_artist_tracks_sai.data().map(|page| page.tracks),
                actions.list_my_tracks_sai.data().map(|page| page.tracks),
            ];
            for list in lists.into_iter().flatten() {
                if let Some(hit) = list.iter().find(|row| row.id == track_id) {
                    return Some(hit.stored());
                }
            }
            for pl in actions.get_playlists_mine_sai.data().and_then(|library| library.playlists).unwrap_or_default() {
                if let Some(hit) =
                    pl.tracks.and_then(|tracks| tracks.into_iter().find(|row| row.id.as_deref().unwrap_or("") == track_id))
                {
                    return Some(hit);
                }
            }
            if let Some(hit) = actions
                .get_playlist_sai
                .data()
                .and_then(|detail| detail.playlist)
                .and_then(|pl| pl.tracks)
                .and_then(|tracks| tracks.into_iter().find(|row| row.id.as_deref().unwrap_or("") == track_id))
            {
                return Some(hit);
            }
            actions
                .get_album_sai
                .data()
                .and_then(|response| response.album)
                .and_then(|al| al.tracks.into_iter().find(|row| row.id == track_id))
        };
        if let Some(hit) = local {
            return Some(RiseStoredMusicTrack::from_canon(&hit));
        }
        self.local_track_row(track_id, cx)
    }
}

impl MusicServicesStore {
    pub fn apply_delete_local_tracks_optimistic(
        &self,
        track_ids: &HashSet<String>,
        cx: &mut App,
    ) -> Option<MusicLocalTracksDeleteSnapshot> {
        let normalized_ids: HashSet<String> = track_ids
            .iter()
            .filter_map(|raw| {
                let id = trimming_whitespaces_and_newlines(raw);
                if id.is_empty() { None } else { Some(id.to_owned()) }
            })
            .collect();
        if normalized_ids.is_empty() {
            return None;
        }

        let page = self.local_tracks_page(cx);
        let local_tracks: Vec<MusicIndexedTrackSnapshot> = page
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(index, track)| {
                let id = track.id.as_ref()?;
                if !normalized_ids.contains(id) {
                    return None;
                }
                Some(MusicIndexedTrackSnapshot { index: index as i64, track: track.clone() })
            })
            .collect();
        if local_tracks.is_empty() {
            return None;
        }

        let mut playlist_snapshots_by_id: HashMap<String, MusicPlaylistTracksDeleteSnapshot> = HashMap::new();
        let mut capture_playlist = |playlist: &RiseStoredMusicPlaylist| {
            let Some(playlist_id) = playlist.id.clone().filter(|id| !id.is_empty()) else { return };
            let removed: Vec<MusicIndexedTrackSnapshot> = playlist
                .tracks
                .iter()
                .flatten()
                .enumerate()
                .filter_map(|(index, track)| {
                    let id = track.id.as_ref()?;
                    if !normalized_ids.contains(id) {
                        return None;
                    }
                    Some(MusicIndexedTrackSnapshot { index: index as i64, track: track.clone() })
                })
                .collect();
            if removed.is_empty() {
                return;
            }

            let tracks = match playlist_snapshots_by_id.get(&playlist_id) {
                Some(existing) => {
                    let existing_ids: HashSet<String> =
                        existing.tracks.iter().filter_map(|item| item.track.id.clone()).collect();
                    let mut merged = existing.tracks.clone();
                    merged.extend(
                        removed.into_iter().filter(|item| item.track.id.as_ref().is_some_and(|id| !existing_ids.contains(id))),
                    );
                    merged
                }
                None => removed,
            };
            playlist_snapshots_by_id
                .insert(playlist_id.clone(), MusicPlaylistTracksDeleteSnapshot { playlist_id, tracks });
        };

        let mine_key = SaiCache::new(["getPlaylistsMine"]).key;
        let cached_mine = SaiWsResponseCache::observed_payload(
            &mine_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
        .and_then(|payload| SaiJSONCodec::decode::<RiseStoredMusicPlaylistLibrary>(Some(&payload)));
        let (slice_mine, detail) = {
            let actions = MusicActionsStore::slices(cx);
            (
                actions.get_playlists_mine_sai.data().and_then(|library| library.playlists).unwrap_or_default(),
                actions.get_playlist_sai.data().and_then(|detail| detail.playlist),
            )
        };
        if let Some(mine) = cached_mine {
            for playlist in mine.playlists.unwrap_or_default() {
                capture_playlist(&playlist);
            }
        }
        for playlist in &slice_mine {
            capture_playlist(playlist);
        }
        if let Some(detail) = &detail {
            capture_playlist(detail);
        }

        let known_playlist_ids: HashSet<String> = playlist_snapshots_by_id.keys().cloned().collect();
        self.remove_local_track_rows(&normalized_ids, known_playlist_ids, cx);

        Some(MusicLocalTracksDeleteSnapshot {
            local_tracks,
            playlists: playlist_snapshots_by_id.into_values().collect(),
        })
    }

    pub fn revert_delete_local_tracks_optimistic(&self, snapshot: &MusicLocalTracksDeleteSnapshot, cx: &mut App) {
        let local_key = Self::my_local_tracks_cache_key();
        let observed = SaiWsResponseCache::observed_payload(
            &local_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        if observed.is_some() {
            let _ = SaiUpdater::update(
                &local_key,
                |context| {
                    let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                    let mut rows = self.json_array(root.get(key::TRACKS));
                    let present_ids: HashSet<String> = rows.iter().filter_map(|row| self.json_row_id(row)).collect();
                    for item in &snapshot.local_tracks {
                        let Some(id) = &item.track.id else { continue };
                        if present_ids.contains(id) {
                            continue;
                        }
                        let Some(row) = SaiJSONCodec::encode(&item.track) else { continue };
                        rows.push(Value::Object(row));
                    }
                    rows.sort_by(|lhs, rhs| {
                        Self::precedes_ordering(
                            self.local_track_json_precedes(lhs, rhs),
                            self.local_track_json_precedes(rhs, lhs),
                        )
                    });
                    root.insert(key::TRACKS.to_owned(), Value::Array(rows));
                    Ok(serde_json::to_vec(&Value::Object(root))?)
                },
                SaiUpdaterWriteScope::All,
                cx,
            );
        } else {
            let slice = MusicActionsStore::slices(cx).list_my_local_tracks_sai.clone();
            if let Some(page) = slice.data() {
                let merged = self.merge_local_track_snapshots(page.stored_tracks(), &snapshot.local_tracks);
                Self::mark_slice(&slice, &page.replacing_tracks(&merged));
            }
        }

        self.restore_playlist_tracks(&snapshot.playlists, cx);
    }

    pub fn finalize_delete_local_tracks(&self, track_ids: &HashSet<String>, cx: &mut App) {
        self.remove_local_track_rows(track_ids, HashSet::new(), cx);
    }

    fn remove_local_track_rows(
        &self,
        track_ids: &HashSet<String>,
        known_playlist_ids: HashSet<String>,
        cx: &mut App,
    ) {
        if track_ids.is_empty() {
            return;
        }

        let mut affected_playlist_ids = known_playlist_ids;
        let collect_affected_playlists = |playlists: &[RiseStoredMusicPlaylist], affected: &mut HashSet<String>| {
            for playlist in playlists {
                let Some(playlist_id) = &playlist.id else { continue };
                let touched = playlist
                    .tracks
                    .as_ref()
                    .is_some_and(|tracks| tracks.iter().any(|row| row.id.as_ref().is_some_and(|id| track_ids.contains(id))));
                if touched {
                    affected.insert(playlist_id.clone());
                }
            }
        };
        let keeps_json_row = |row: &Value| self.json_row_id(row).is_none_or(|id| !track_ids.contains(&id));
        let keeps_track = |row: &RiseStoredMusicTrack| row.id.as_ref().is_none_or(|id| !track_ids.contains(id));

        let (local_slice, mine_slice, detail_slice) = {
            let actions = MusicActionsStore::slices(cx);
            (
                actions.list_my_local_tracks_sai.clone(),
                actions.get_playlists_mine_sai.clone(),
                actions.get_playlist_sai.clone(),
            )
        };

        let local_key = Self::my_local_tracks_cache_key();
        let local_observed = SaiWsResponseCache::observed_payload(
            &local_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        if local_observed.is_some() {
            let _ = SaiUpdater::update(
                &local_key,
                |context| {
                    let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                    let rows: Vec<Value> =
                        self.json_array(root.get(key::TRACKS)).into_iter().filter(|row| keeps_json_row(row)).collect();
                    root.insert(key::TRACKS.to_owned(), Value::Array(rows));
                    Ok(serde_json::to_vec(&Value::Object(root))?)
                },
                SaiUpdaterWriteScope::All,
                cx,
            );
        } else if let Some(page) = local_slice.data() {
            let kept: Vec<RiseStoredMusicTrack> = page.stored_tracks().into_iter().filter(|row| keeps_track(row)).collect();
            Self::mark_slice(&local_slice, &page.replacing_tracks(&kept));
        }

        let mine_key = SaiCache::new(["getPlaylistsMine"]).key;
        let mine_payload = SaiWsResponseCache::observed_payload(
            &mine_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        if let Some(mine_payload) = mine_payload {
            if let Some(mine) = SaiJSONCodec::decode::<RiseStoredMusicPlaylistLibrary>(Some(&mine_payload)) {
                collect_affected_playlists(&mine.playlists.unwrap_or_default(), &mut affected_playlist_ids);
            }
            let _ = SaiUpdater::update(
                &mine_key,
                |context| {
                    let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                    let playlists: Vec<Value> = self
                        .json_array(root.get(key::PLAYLISTS))
                        .into_iter()
                        .map(|raw_playlist| {
                            let Some(mut playlist) = AnyDictionaryBridge::dictionary(Some(&raw_playlist)) else {
                                return raw_playlist;
                            };
                            let tracks: Vec<Value> = self
                                .json_array(playlist.get(key::TRACKS))
                                .into_iter()
                                .filter(|row| keeps_json_row(row))
                                .collect();
                            playlist.insert(key::TRACKS.to_owned(), Value::Array(tracks));
                            Value::Object(playlist)
                        })
                        .collect();
                    root.insert(key::PLAYLISTS.to_owned(), Value::Array(playlists));
                    Ok(serde_json::to_vec(&Value::Object(root))?)
                },
                SaiUpdaterWriteScope::All,
                cx,
            );
        } else if let Some(mine) = mine_slice.data() {
            let playlists = mine.playlists.unwrap_or_default();
            collect_affected_playlists(&playlists, &mut affected_playlist_ids);
            let playlists = playlists
                .iter()
                .map(|playlist| {
                    let kept = playlist.tracks.iter().flatten().filter(|row| keeps_track(row)).cloned().collect();
                    playlist.with(None, None, None, None, Some(kept))
                })
                .collect();
            Self::mark_slice(&mine_slice, &RiseStoredMusicPlaylistLibrary { playlists: Some(playlists) });
        }

        if let Some(current_detail) = detail_slice.data().and_then(|detail| detail.playlist)
            && let Some(playlist_id) = current_detail.id.clone().filter(|id| !id.is_empty())
        {
            let touched = current_detail
                .tracks
                .as_ref()
                .is_some_and(|tracks| tracks.iter().any(|row| row.id.as_ref().is_some_and(|id| track_ids.contains(id))));
            if touched {
                affected_playlist_ids.insert(playlist_id);
            }
        }

        for playlist_id in &affected_playlist_ids {
            let detail_key = SaiCache::new(["getPlaylist", playlist_id.as_str()]).key;
            let observed = SaiWsResponseCache::observed_payload(
                &detail_key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            );
            if observed.is_none() {
                continue;
            }
            let _ = SaiUpdater::update(
                &detail_key,
                |context| {
                    let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                    if let Some(mut playlist) = AnyDictionaryBridge::dictionary(root.get(key::PLAYLIST)) {
                        let tracks: Vec<Value> = self
                            .json_array(playlist.get(key::TRACKS))
                            .into_iter()
                            .filter(|row| keeps_json_row(row))
                            .collect();
                        playlist.insert(key::TRACKS.to_owned(), Value::Array(tracks));
                        root.insert(key::PLAYLIST.to_owned(), Value::Object(playlist));
                    }
                    Ok(serde_json::to_vec(&Value::Object(root))?)
                },
                SaiUpdaterWriteScope::All,
                cx,
            );
        }

        if let Some(current_detail) = detail_slice.data().and_then(|detail| detail.playlist)
            && let Some(playlist_id) = current_detail.id.clone().filter(|id| !id.is_empty())
        {
            let detail_key = SaiCache::new(["getPlaylist", playlist_id.as_str()]).key;
            let observed = SaiWsResponseCache::observed_payload(
                &detail_key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            );
            if observed.is_none() {
                let kept = current_detail.tracks.iter().flatten().filter(|row| keeps_track(row)).cloned().collect();
                Self::mark_slice(
                    &detail_slice,
                    &RiseStoredMusicPlaylistDetail {
                        playlist: Some(current_detail.with(None, None, None, None, Some(kept))),
                    },
                );
            }
        }
    }

    fn restore_playlist_tracks(&self, snapshots: &[MusicPlaylistTracksDeleteSnapshot], cx: &mut App) {
        if snapshots.is_empty() {
            return;
        }
        let snapshots_by_id: HashMap<&str, &MusicPlaylistTracksDeleteSnapshot> =
            snapshots.iter().map(|snapshot| (snapshot.playlist_id.as_str(), snapshot)).collect();
        let mine_key = SaiCache::new(["getPlaylistsMine"]).key;
        let (mine_slice, detail_slice) = {
            let actions = MusicActionsStore::slices(cx);
            (actions.get_playlists_mine_sai.clone(), actions.get_playlist_sai.clone())
        };

        let mine_observed = SaiWsResponseCache::observed_payload(
            &mine_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        if mine_observed.is_some() {
            let _ = SaiUpdater::update(
                &mine_key,
                |context| {
                    let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                    let playlists: Vec<Value> = self
                        .json_array(root.get(key::PLAYLISTS))
                        .into_iter()
                        .map(|raw_playlist| {
                            let Some(mut playlist) = AnyDictionaryBridge::dictionary(Some(&raw_playlist)) else {
                                return raw_playlist;
                            };
                            let Some(playlist_id) = self.json_string(playlist.get(key::ID)) else {
                                return raw_playlist;
                            };
                            let Some(snapshot) = snapshots_by_id.get(playlist_id.as_str()) else {
                                return raw_playlist;
                            };
                            let tracks =
                                self.restore_json_tracks(self.json_array(playlist.get(key::TRACKS)), &snapshot.tracks);
                            playlist.insert(key::TRACKS.to_owned(), Value::Array(tracks));
                            Value::Object(playlist)
                        })
                        .collect();
                    root.insert(key::PLAYLISTS.to_owned(), Value::Array(playlists));
                    Ok(serde_json::to_vec(&Value::Object(root))?)
                },
                SaiUpdaterWriteScope::All,
                cx,
            );
        } else if let Some(mine) = mine_slice.data() {
            let playlists = mine
                .playlists
                .unwrap_or_default()
                .into_iter()
                .map(|playlist| {
                    let Some(snapshot) = playlist.id.as_deref().and_then(|id| snapshots_by_id.get(id)) else {
                        return playlist;
                    };
                    let restored = self.restore_wire_tracks(playlist.tracks.clone().unwrap_or_default(), &snapshot.tracks);
                    playlist.with(None, None, None, None, Some(restored))
                })
                .collect();
            Self::mark_slice(&mine_slice, &RiseStoredMusicPlaylistLibrary { playlists: Some(playlists) });
        }

        for snapshot in snapshots {
            let detail_key = SaiCache::new(["getPlaylist", snapshot.playlist_id.as_str()]).key;
            let observed = SaiWsResponseCache::observed_payload(
                &detail_key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            );
            if observed.is_none() {
                continue;
            }
            let _ = SaiUpdater::update(
                &detail_key,
                |context| {
                    let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                    if let Some(mut playlist) = AnyDictionaryBridge::dictionary(root.get(key::PLAYLIST)) {
                        let tracks =
                            self.restore_json_tracks(self.json_array(playlist.get(key::TRACKS)), &snapshot.tracks);
                        playlist.insert(key::TRACKS.to_owned(), Value::Array(tracks));
                        root.insert(key::PLAYLIST.to_owned(), Value::Object(playlist));
                    }
                    Ok(serde_json::to_vec(&Value::Object(root))?)
                },
                SaiUpdaterWriteScope::All,
                cx,
            );
        }

        if let Some(current_detail) = detail_slice.data().and_then(|detail| detail.playlist)
            && let Some(playlist_id) = current_detail.id.clone()
        {
            let observed = SaiWsResponseCache::observed_payload(
                &SaiCache::new(["getPlaylist", playlist_id.as_str()]).key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            );
            if observed.is_none()
                && let Some(snapshot) = snapshots_by_id.get(playlist_id.as_str())
            {
                let restored =
                    self.restore_wire_tracks(current_detail.tracks.clone().unwrap_or_default(), &snapshot.tracks);
                Self::mark_slice(
                    &detail_slice,
                    &RiseStoredMusicPlaylistDetail {
                        playlist: Some(current_detail.with(None, None, None, None, Some(restored))),
                    },
                );
            }
        }
    }

    fn merge_local_track_snapshots(
        &self,
        current: Vec<RiseStoredMusicTrack>,
        snapshots: &[MusicIndexedTrackSnapshot],
    ) -> Vec<RiseStoredMusicTrack> {
        let mut present_ids: HashSet<String> = current.iter().filter_map(|row| row.id.clone()).collect();
        let mut merged = current;
        for snapshot in snapshots {
            let Some(id) = &snapshot.track.id else { continue };
            if !present_ids.insert(id.clone()) {
                continue;
            }
            merged.push(snapshot.track.clone());
        }
        merged.sort_by(|lhs, rhs| {
            Self::precedes_ordering(self.local_track_wire_precedes(lhs, rhs), self.local_track_wire_precedes(rhs, lhs))
        });
        merged
    }

    fn restore_wire_tracks(
        &self,
        current: Vec<RiseStoredMusicTrack>,
        snapshots: &[MusicIndexedTrackSnapshot],
    ) -> Vec<RiseStoredMusicTrack> {
        let mut present_ids: HashSet<String> = current.iter().filter_map(|row| row.id.clone()).collect();
        let mut restored = current;
        let mut ordered: Vec<&MusicIndexedTrackSnapshot> = snapshots.iter().collect();
        ordered.sort_by_key(|snapshot| snapshot.index);
        for snapshot in ordered {
            let Some(id) = &snapshot.track.id else { continue };
            if !present_ids.insert(id.clone()) {
                continue;
            }
            let at = (snapshot.index.max(0) as usize).min(restored.len());
            restored.insert(at, snapshot.track.clone());
        }
        restored
    }

    fn restore_json_tracks(&self, current: Vec<Value>, snapshots: &[MusicIndexedTrackSnapshot]) -> Vec<Value> {
        let mut present_ids: HashSet<String> = current.iter().filter_map(|row| self.json_row_id(row)).collect();
        let mut restored = current;
        let mut ordered: Vec<&MusicIndexedTrackSnapshot> = snapshots.iter().collect();
        ordered.sort_by_key(|snapshot| snapshot.index);
        for snapshot in ordered {
            let Some(id) = &snapshot.track.id else { continue };
            if !present_ids.insert(id.clone()) {
                continue;
            }
            let Some(row) = SaiJSONCodec::encode(&snapshot.track) else { continue };
            let at = (snapshot.index.max(0) as usize).min(restored.len());
            restored.insert(at, Value::Object(row));
        }
        restored
    }

    fn json_array(&self, value: Option<&Value>) -> Vec<Value> {
        match value {
            Some(Value::Array(rows)) => rows.clone(),
            _ => Vec::new(),
        }
    }

    fn json_string(&self, value: Option<&Value>) -> Option<String> {
        match value? {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            Value::Bool(value) => Some(if *value { 1 } else { 0 }.to_string()),
            _ => None,
        }
    }

    fn json_row_id(&self, row: &Value) -> Option<String> {
        self.json_string(AnyDictionaryBridge::dictionary(Some(row))?.get(key::ID))
    }

    fn local_track_json_precedes(&self, lhs: &Value, rhs: &Value) -> bool {
        let left = AnyDictionaryBridge::dictionary(Some(lhs)).unwrap_or_default();
        let right = AnyDictionaryBridge::dictionary(Some(rhs)).unwrap_or_default();
        let left_created = self.json_int64(left.get(key::CREATED_AT_MS));
        let right_created = self.json_int64(right.get(key::CREATED_AT_MS));
        if left_created != right_created {
            return left_created > right_created;
        }
        self.json_string(left.get(key::ID)).unwrap_or_default() > self.json_string(right.get(key::ID)).unwrap_or_default()
    }

    fn local_track_wire_precedes(&self, lhs: &RiseStoredMusicTrack, rhs: &RiseStoredMusicTrack) -> bool {
        let left_created = lhs.created_at_ms.unwrap_or(0);
        let right_created = rhs.created_at_ms.unwrap_or(0);
        if left_created != right_created {
            return left_created > right_created;
        }
        lhs.id.as_deref().unwrap_or("") > rhs.id.as_deref().unwrap_or("")
    }

    fn json_int64(&self, value: Option<&Value>) -> i64 {
        match value {
            Some(Value::Number(number)) => {
                number.as_i64().or_else(|| number.as_u64().map(|v| v as i64)).or_else(|| number.as_f64().map(|v| v as i64)).unwrap_or(0)
            }
            Some(Value::Bool(value)) => i64::from(*value),
            Some(Value::String(value)) => value.parse::<i64>().unwrap_or(0),
            _ => 0,
        }
    }

    fn precedes_ordering(lhs_first: bool, rhs_first: bool) -> Ordering {
        if lhs_first {
            Ordering::Less
        } else if rhs_first {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

impl MusicServicesStore {
    pub fn message_mp3_pending_playlist_cache_key(playlist_id: &str) -> String {
        SaiCache::new(["musicMessageMP3PendingPlaylist", playlist_id]).key
    }

    pub fn merging_local_track_optimistic_playlist(
        base: Option<RiseStoredMusicPlaylist>,
        pending: Option<RiseStoredMusicPlaylist>,
    ) -> Option<RiseStoredMusicPlaylist> {
        let Some(pending) = pending else { return base };
        let Some(base) = base else { return Some(pending) };

        let mut tracks = base.tracks.clone().unwrap_or_default();
        let mut added: Vec<RiseStoredMusicTrack> = Vec::new();
        for pending_track in pending.tracks.unwrap_or_default() {
            let Some(id) = pending_track.id.clone().filter(|id| !id.is_empty()) else { continue };
            if let Some(index) = tracks.iter().position(|row| row.id.as_deref() == Some(id.as_str())) {
                tracks[index] = pending_track;
            } else {
                added.push(pending_track);
            }
        }
        if base.new_tracks_on_top == Some(true) {
            added.reverse();
            added.extend(tracks);
            return Some(base.with(None, None, None, None, Some(added)));
        }
        tracks.extend(added);
        Some(base.with(None, None, None, None, Some(tracks)))
    }

    pub fn bounded_message_mp3_pending_tracks(
        tracks: &[RiseStoredMusicTrack],
        preferred_limit: i64,
    ) -> Vec<RiseStoredMusicTrack> {
        let limit = preferred_limit.max(0);
        let active_indices: HashSet<usize> = (0..tracks.len())
            .filter(|&index| {
                let Some(phase) = tracks[index].transfer_phase.as_deref().and_then(MusicLocalTrackTransferPhase::from_raw)
                else {
                    return false;
                };
                match phase {
                    MusicLocalTrackTransferPhase::Queued
                    | MusicLocalTrackTransferPhase::Downloading
                    | MusicLocalTrackTransferPhase::Preparing
                    | MusicLocalTrackTransferPhase::Uploading
                    | MusicLocalTrackTransferPhase::WaitingForConnection => true,
                    MusicLocalTrackTransferPhase::Failed => false,
                }
            })
            .collect();
        let terminal_slots = (limit - active_indices.len() as i64).max(0) as usize;
        let terminal_indices: Vec<usize> = (0..tracks.len()).filter(|index| !active_indices.contains(index)).collect();
        let retained_terminal_indices: HashSet<usize> =
            terminal_indices[terminal_indices.len().saturating_sub(terminal_slots)..].iter().copied().collect();
        (0..tracks.len())
            .filter(|index| active_indices.contains(index) || retained_terminal_indices.contains(index))
            .map(|index| tracks[index].clone())
            .collect()
    }

    pub fn message_mp3_pending_playlist(&self, playlist_id: &str, cx: &mut App) -> Option<RiseStoredMusicPlaylist> {
        let cache_key = Self::message_mp3_pending_playlist_cache_key(playlist_id);
        let payload = SaiWsResponseCache::observed_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )?;
        let response = SaiJSONCodec::decode::<RiseStoredMusicPlaylistDetail>(Some(&payload));
        response.and_then(|response| response.playlist).filter(|playlist| playlist.id.as_deref() == Some(playlist_id))
    }

    pub fn reconcile_interrupted_message_mp3_imports(&self, playlist_id: &str, cx: &mut App) {
        if self.message_mp3_import_task.is_some() || !self.message_mp3_import_queue.is_empty() {
            return;
        }
        if MusicActionsStore::slices(cx).create_local_track_in_playlist_sai.is_pending() {
            return;
        }
        let Some(pending) = self.message_mp3_pending_playlist(playlist_id, cx) else { return };
        let Some(mut tracks) = pending.tracks.clone() else { return };

        let mut changed = false;
        for track in tracks.iter_mut() {
            if track.source_chat_id.is_none() {
                continue;
            }
            let Some(phase) = track.transfer_phase.as_deref().and_then(MusicLocalTrackTransferPhase::from_raw) else {
                continue;
            };
            if phase == MusicLocalTrackTransferPhase::Failed || phase == MusicLocalTrackTransferPhase::WaitingForConnection
            {
                continue;
            }
            track.transfer_phase = Some(MusicLocalTrackTransferPhase::Failed.raw_value().to_owned());
            track.transfer_error = Some(tr("music_message_import_download_failed"));
            changed = true;
        }
        if !changed {
            return;
        }
        self.persist_message_mp3_pending_playlist(
            &pending.with(None, None, None, None, Some(tracks)),
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    pub fn playlist(&self, playlist_id: &str, cx: &mut App) -> Option<RiseStoredMusicPlaylist> {
        if let Some(staged) = MusicGuideSandboxHooks::playlist(playlist_id, cx) {
            return Some(staged);
        }
        let pending = self.message_mp3_pending_playlist(playlist_id, cx);
        let merged = Self::merging_local_track_optimistic_playlist(
            self.playlist_without_local_track_optimism(playlist_id, cx),
            pending,
        );
        self.applying_rise_music_overrides_to_playlist(merged.as_ref(), cx)
    }

    pub fn playlists_mine(&self, cx: &mut App) -> Vec<RiseStoredMusicPlaylist> {
        let real = MusicActionsStore::slices(cx)
            .get_playlists_mine_sai
            .data()
            .and_then(|library| library.playlists)
            .unwrap_or_default();
        let Some(staged) =
            MusicGuideSandboxHooks::playlist_id().and_then(|guide_id| MusicGuideSandboxHooks::playlist(guide_id, cx))
        else {
            return real;
        };
        let staged_id = staged.id.clone();
        let mut out = vec![staged];
        out.extend(real.into_iter().filter(|row| row.id != staged_id));
        out
    }

    fn playlist_without_local_track_optimism(&self, playlist_id: &str, cx: &mut App) -> Option<RiseStoredMusicPlaylist> {
        let detail_key = SaiCache::new(["getPlaylist", playlist_id]).key;
        let detail = SaiWsResponseCache::observed_payload(
            &detail_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
        .and_then(|payload| SaiJSONCodec::decode::<RiseStoredMusicPlaylistDetail>(Some(&payload)));
        if let Some(detail) = detail
            && detail.playlist.as_ref().and_then(|playlist| playlist.id.as_deref()) == Some(playlist_id)
        {
            return Self::removing_local_track_optimistic_rows(detail.playlist.as_ref());
        }

        let mine_key = SaiCache::new(["getPlaylistsMine"]).key;
        let mine = SaiWsResponseCache::observed_payload(
            &mine_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
        .and_then(|payload| SaiJSONCodec::decode::<RiseStoredMusicPlaylistLibrary>(Some(&payload)));
        if let Some(mine) = mine {
            let hit = mine
                .playlists
                .unwrap_or_default()
                .into_iter()
                .find(|row| row.id.as_deref() == Some(playlist_id));
            return Self::removing_local_track_optimistic_rows(hit.as_ref());
        }

        let actions = MusicActionsStore::slices(cx);
        let open = actions.get_playlist_sai.data().and_then(|detail| detail.playlist);
        let slice_playlist = if open.as_ref().and_then(|playlist| playlist.id.as_deref()) == Some(playlist_id) {
            open
        } else {
            actions
                .get_playlists_mine_sai
                .data()
                .and_then(|library| library.playlists)
                .and_then(|playlists| playlists.into_iter().find(|row| row.id.as_deref() == Some(playlist_id)))
        };
        Self::removing_local_track_optimistic_rows(slice_playlist.as_ref())
    }
}

impl MusicServicesStore {
    #[allow(clippy::too_many_arguments)]
    pub fn make_local_track_draft(
        &self,
        temp_id: &str,
        title: &str,
        description: Option<&str>,
        artist_name: &str,
        duration_ms: i64,
        owner_user_id: &str,
        local_audio_url: &Url,
        cover_url: &str,
        source_chat_id: Option<&str>,
        source_message_id: Option<&str>,
        remote_audio_url: Option<&str>,
        transfer_phase: Option<MusicLocalTrackTransferPhase>,
        transferred_bytes: i64,
        transfer_total_bytes: i64,
        transfer_error: Option<&str>,
    ) -> RiseStoredMusicTrack {
        let artist = WireArtistDTO { stage_name: artist_name.to_owned(), ..WireArtistDTO::default() };
        let mut track = RiseStoredMusicTrack {
            id: Some(temp_id.to_owned()),
            title: Some(title.to_owned()),
            description: description.map(str::to_owned),
            duration_ms: Some(duration_ms),
            cover_url: Some(cover_url.to_owned()),
            audio_url: Some(local_audio_url.absolute_string().to_owned()),
            play_count: Some(0),
            like_count: Some(0),
            is_liked: Some(false),
            album: None,
            artist: Some(artist),
            created_at_ms: Some((Date::now().time_interval_since_1970() * 1000.0) as i64),
            status: Some(music_service::TRACK_STATUS_UPLOADING.to_owned()),
            genre: Some(music_service::TRACK_GENRE_LOCAL_UPLOAD.to_owned()),
            rejection_reason: None,
            scheduled_at_ms: None,
            is_exclusive: Some(false),
            library_owner_user_id: Some(owner_user_id.to_owned()),
            ..RiseStoredMusicTrack::default()
        };
        track.source_chat_id = source_chat_id.map(str::to_owned);
        track.source_message_id = source_message_id.map(str::to_owned);
        track.remote_audio_url = remote_audio_url.map(str::to_owned);
        track.transfer_phase = transfer_phase.map(|phase| phase.raw_value().to_owned());
        track.transferred_bytes = Some(transferred_bytes.max(0));
        track.transfer_total_bytes = Some(transfer_total_bytes.max(0));
        track.transfer_error = transfer_error.map(str::to_owned);
        track
    }

    pub fn insert_local_track_optimistic(
        &self,
        playlist_id: &str,
        track: RiseStoredMusicTrack,
        destination: Option<&MusicLocalPlaylistDestination>,
        cx: &mut App,
    ) {
        let current_pending = self.message_mp3_pending_playlist(playlist_id, cx);
        let server_playlist = self.playlist_without_local_track_optimism(playlist_id, cx);
        let seed = current_pending.clone().or_else(|| server_playlist.clone()).unwrap_or_else(|| RiseStoredMusicPlaylist {
            id: Some(playlist_id.to_owned()),
            owner_user_id: destination.map(|destination| destination.owner_user_id.clone()),
            title: destination.map(|destination| destination.title.clone()),
            description: destination.map(|destination| destination.description.clone()),
            cover_url: destination.map(|destination| destination.cover_url.clone()),
            visibility: Some(PlaylistVisibility::LocalPlaylist.wire().to_owned()),
            tracks: Some(Vec::new()),
            created_at_ms: None,
            updated_at_ms: destination.map(|destination| destination.updated_at_ms),
            revision: destination.map(|destination| destination.revision),
            ..RiseStoredMusicPlaylist::default()
        });
        let mut tracks = current_pending.and_then(|pending| pending.tracks).unwrap_or_default();
        if let Some(index) = tracks.iter().position(|row| row.id == track.id) {
            tracks[index] = track;
        } else {
            tracks.push(track);
        }
        let tracks = Self::bounded_message_mp3_pending_tracks(&tracks, 20);
        let pending = seed.with(None, None, None, None, Some(tracks));
        self.persist_message_mp3_pending_playlist(&pending, SaiUpdaterWriteScope::All, cx);

        let visible_base = server_playlist.unwrap_or_else(|| seed.with(None, None, None, None, Some(Vec::new())));
        if let Some(merged) = Self::merging_local_track_optimistic_playlist(Some(visible_base), Some(pending)) {
            self.patch_playlist_detail(&merged, cx);
        }
    }

    pub fn local_track_create_response(&self, payload: Option<Box<dyn Any>>) -> Option<RiseStoredMusicTrack> {
        let response = match payload?.downcast::<RiseStoredMusicLocalTrackUpload>() {
            Ok(decoded) => Some(*decoded),
            Err(payload) => SaiJSONCodec::decode::<RiseStoredMusicLocalTrackUpload>(payload.downcast_ref::<Value>()),
        };
        response?.track
    }

    pub fn replace_local_track_optimistic(
        &mut self,
        playlist_id: &str,
        temp_id: &str,
        track: RiseStoredMusicTrack,
        cx: &mut App,
    ) {
        self.remove_message_mp3_pending_track(playlist_id, temp_id, cx);
        let pending = self.message_mp3_pending_playlist(playlist_id, cx);
        let base = self
            .playlist_without_local_track_optimism(playlist_id, cx)
            .or_else(|| pending.as_ref().map(|pending| pending.with(None, None, None, None, Some(Vec::new()))));
        if let Some(base) = base {
            let mut tracks = base.tracks.clone().unwrap_or_default();
            if !tracks.iter().any(|row| row.id == track.id) {
                if base.new_tracks_on_top == Some(true) {
                    tracks.insert(0, track.clone());
                } else {
                    tracks.push(track.clone());
                }
            }
            let playlist = base.with(None, None, None, None, Some(tracks));
            let merged = Self::merging_local_track_optimistic_playlist(Some(playlist.clone()), pending)
                .unwrap_or(playlist);
            self.patch_playlist_detail(&merged, cx);
        }
        self.insert_local_track_into_loaded_library(&track, cx);
        if let Some(model) = track.as_track_model() {
            let temp_id = temp_id.to_owned();
            cx.defer(move |cx| {
                MusicPlayerState::shared(cx).update(cx, |state, cx| state.adopt_uploaded_track(&temp_id, &model, cx));
            });
        }
    }

    fn insert_local_track_into_loaded_library(&self, track: &RiseStoredMusicTrack, cx: &mut App) {
        if track.id.as_deref().unwrap_or("").is_empty() {
            return;
        }
        let Some(row) = SaiJSONCodec::encode(track) else { return };

        let local_key = Self::my_local_tracks_cache_key();
        let observed = SaiWsResponseCache::observed_payload(
            &local_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        if observed.is_some() {
            let _ = SaiUpdater::insert_row_if_missing(
                &local_key,
                key::TRACKS,
                key::ID,
                row,
                true,
                SaiUpdaterWriteScope::All,
                cx,
            );
        } else {
            let slice = MusicActionsStore::slices(cx).list_my_local_tracks_sai.clone();
            if let Some(page) = slice.data() {
                let tracks = self.merge_local_track_snapshots(
                    page.stored_tracks(),
                    &[MusicIndexedTrackSnapshot { index: 0, track: track.clone() }],
                );
                Self::mark_slice(&slice, &page.replacing_tracks(&tracks));
            }
        }
    }

    pub fn remove_local_track_optimistic(&mut self, playlist_id: &str, temp_id: &str, cx: &mut App) {
        self.remove_message_mp3_pending_track(playlist_id, temp_id, cx);
        let base = self.playlist_without_local_track_optimism(playlist_id, cx);
        let pending = self.message_mp3_pending_playlist(playlist_id, cx);
        let Some(playlist) = Self::merging_local_track_optimistic_playlist(base, pending) else { return };
        self.patch_playlist_detail(&playlist, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn patch_message_mp3_pending_track(
        &mut self,
        playlist_id: &str,
        temp_id: &str,
        phase: MusicLocalTrackTransferPhase,
        completed_bytes: i64,
        total_bytes: i64,
        error: Option<&str>,
        cx: &mut App,
    ) {
        let Some(pending) = self.message_mp3_pending_playlist(playlist_id, cx) else { return };
        let Some(mut tracks) = pending.tracks.clone() else { return };
        let Some(index) = tracks.iter().position(|row| row.id.as_deref() == Some(temp_id)) else { return };

        let mut track = tracks[index].clone();
        let current_phase = track.transfer_phase.as_deref().and_then(MusicLocalTrackTransferPhase::from_raw);
        let Some((completed, total)) = MusicLocalTrackTransferPhase::reconciled_progress_update(
            current_phase,
            phase,
            track.transferred_bytes.unwrap_or(0),
            track.transfer_total_bytes.unwrap_or(0),
            completed_bytes,
            total_bytes,
        ) else {
            return;
        };
        let error = error.map(str::to_owned);
        if track.transfer_phase.as_deref() == Some(phase.raw_value())
            && track.transferred_bytes == Some(completed)
            && track.transfer_total_bytes == Some(total)
            && track.transfer_error == error
        {
            return;
        }
        track.transfer_phase = Some(phase.raw_value().to_owned());
        track.transferred_bytes = Some(completed);
        track.transfer_total_bytes = Some(total);
        track.transfer_error = error;
        tracks[index] = track;

        let now = Date::now().time_interval_since_reference_date();
        let checkpoint = self.message_mp3_progress_persistence.get(temp_id);
        let should_persist = match checkpoint {
            None => true,
            Some(checkpoint) => {
                checkpoint.phase != phase.raw_value()
                    || completed - checkpoint.completed_bytes >= 512 * 1_024
                    || now - checkpoint.persisted_at >= 1.0
                    || phase == MusicLocalTrackTransferPhase::Failed
                    || (total > 0 && completed >= total)
            }
        };
        if should_persist {
            self.message_mp3_progress_persistence.insert(
                temp_id.to_owned(),
                MusicMessageMP3ProgressPersistence {
                    phase: phase.raw_value().to_owned(),
                    completed_bytes: completed,
                    persisted_at: now,
                },
            );
        }
        let updated_pending = pending.with(None, None, None, None, Some(tracks));
        self.persist_message_mp3_pending_playlist(
            &updated_pending,
            if should_persist { SaiUpdaterWriteScope::All } else { SaiUpdaterWriteScope::Cache },
            cx,
        );
        self.refresh_loaded_playlist_slice(playlist_id, Some(updated_pending), cx);
    }

    fn refresh_loaded_playlist_slice(
        &self,
        playlist_id: &str,
        pending: Option<RiseStoredMusicPlaylist>,
        cx: &mut App,
    ) {
        let slice = MusicActionsStore::slices(cx).get_playlist_sai.clone();
        let open_id = slice.data().and_then(|detail| detail.playlist).and_then(|playlist| playlist.id);
        if open_id.as_deref() != Some(playlist_id) {
            return;
        }
        let Some(playlist) = Self::merging_local_track_optimistic_playlist(
            self.playlist_without_local_track_optimism(playlist_id, cx),
            pending,
        ) else {
            return;
        };
        Self::mark_slice(&slice, &RiseStoredMusicPlaylistDetail { playlist: Some(playlist) });
    }

    fn remove_message_mp3_pending_track(&mut self, playlist_id: &str, temp_id: &str, cx: &mut App) {
        self.message_mp3_progress_persistence.remove(temp_id);
        let Some(pending) = self.message_mp3_pending_playlist(playlist_id, cx) else { return };
        let tracks: Vec<RiseStoredMusicTrack> =
            pending.tracks.clone().unwrap_or_default().into_iter().filter(|row| row.id.as_deref() != Some(temp_id)).collect();
        self.persist_message_mp3_pending_playlist(
            &pending.with(None, None, None, None, Some(tracks)),
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    fn persist_message_mp3_pending_playlist(
        &self,
        playlist: &RiseStoredMusicPlaylist,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) {
        let Some(playlist_id) = playlist.id.clone() else { return };
        let Some(root) = SaiJSONCodec::encode(&RiseStoredMusicPlaylistDetail { playlist: Some(playlist.clone()) }) else {
            return;
        };
        let cache_key = Self::message_mp3_pending_playlist_cache_key(&playlist_id);
        if write_to != SaiUpdaterWriteScope::All {
            let _ = SaiUpdater::replace_root(&cache_key, root, write_to, cx);
            return;
        }

        let _ = SaiUpdater::replace_root(&cache_key, root.clone(), SaiUpdaterWriteScope::Cache, cx);
        SaiWsDiskCache::save_asynchronously(
            cache_key,
            Value::Object(root),
            |result, cx| {
                if let Err(error) = result {
                    Sai::log(
                        "Music",
                        move || format!("pending MP3 playlist checkpoint failed: {error}"),
                        SaiLogKind::Cache,
                        cx,
                    );
                }
            },
            cx,
        );
    }

    pub fn patch_local_track_upload_pending(
        &self,
        track: &RiseStoredMusicTrack,
        temp_id: &str,
        progress: f64,
        uploading: bool,
        slice: Option<Rc<SaiTypedSlice<RiseStoredMusicLocalTrackUpload>>>,
        cx: &mut App,
    ) {
        let target_slice = slice.unwrap_or_else(|| MusicActionsStore::slices(cx).create_local_track_in_playlist_sai.clone());
        Self::mark_slice(
            &target_slice,
            &RiseStoredMusicLocalTrackUpload {
                track: Some(track.clone()),
                upload_progress: Some(progress.clamp(0.0, 100.0)),
                is_uploading: Some(uploading),
                temp_id: Some(temp_id.to_owned()),
            },
        );
    }
}

impl MusicServicesStore {
    #[allow(clippy::too_many_arguments)]
    pub fn make_local_track_optimistic_files(
        &self,
        playlist_id: &str,
        audio_file: SaiWsOptimisticLocalFile,
        cover_file: Option<SaiWsOptimisticLocalFile>,
        track: RiseStoredMusicTrack,
        temp_id: &str,
        resource_id: &str,
        slice: Option<Rc<SaiTypedSlice<RiseStoredMusicLocalTrackUpload>>>,
        cx: &mut App,
    ) -> SaiWsOptimisticFiles {
        let target_slice = slice.unwrap_or_else(|| MusicActionsStore::slices(cx).create_local_track_in_playlist_sai.clone());
        let mut audio = audio_file;
        audio.file_context_override = Some(MUSIC_AUDIO_FILE_CONTEXT.to_owned());
        let mut files_list = vec![audio];
        if let Some(mut cover_file) = cover_file {
            cover_file.file_context_override = Some(MUSIC_COVER_FILE_CONTEXT.to_owned());
            files_list.push(cover_file);
        }
        let file_count = files_list.len();
        let completed_bytes = Rc::new(RefCell::new(vec![0_i64; file_count]));
        let total_bytes =
            Rc::new(RefCell::new(files_list.iter().map(|file| file.byte_count.unwrap_or(0).max(0)).collect::<Vec<i64>>()));
        let uploaded_indices = Rc::new(RefCell::new(HashSet::<usize>::new()));

        let mut files = SaiWsOptimisticFiles::new(MUSIC_AUDIO_FILE_CONTEXT.to_owned());
        files.compression = MediaCompressionConfiguration::cover();
        files.resource_id = Some(resource_id.to_owned());
        let success_temp_id = temp_id.to_owned();
        files.on_file_success = Some(Arc::new(move |index: usize, _: &Map<String, Value>, cx: &mut App| {
            let uploaded_count = {
                let mut uploaded = uploaded_indices.borrow_mut();
                uploaded.insert(index);
                uploaded.len()
            };
            if uploaded_count != file_count {
                return;
            }
            let temp_id = success_temp_id.clone();
            MusicServicesStore::shared(cx).update(cx, |services, _| {
                if services.message_mp3_active_import_temp_ids.contains(&temp_id) {
                    services.message_mp3_uploaded_temp_ids.insert(temp_id);
                }
            });
        }));
        files.local_files = files_list;
        files.merge_uploaded_slots_into_body_root = true;
        files.extract_uploaded_files =
            Some(Arc::new(|done: &Map<String, Value>, index: usize, _: &SaiWsOptimisticLocalFile, cx: &mut App| -> anyhow::Result<Value> {
                let invalid = || anyhow::Error::new(SaiWsOptimisticError::FileUploadInvalidSlotPayload);
                let url = MusicTrackFileUpload::remote_url(done, cx).ok_or_else(invalid)?;
                let asset_id = MusicTrackFileUpload::remote_asset_id(done).ok_or_else(invalid)?;
                let mut slot = Map::new();
                if index == 0 {
                    let access_url = MusicTrackFileUpload::remote_access_url(done).ok_or_else(invalid)?;
                    slot.insert(key::camel::AUDIO_URL.to_owned(), Value::String(url));
                    slot.insert(key::camel::AUDIO_ACCESS_URL.to_owned(), Value::String(access_url));
                    slot.insert(key::camel::AUDIO_ASSET_ID.to_owned(), Value::String(asset_id));
                    return Ok(Value::Object(slot));
                }
                slot.insert(key::camel::COVER_URL.to_owned(), Value::String(url));
                slot.insert(key::camel::COVER_ASSET_ID.to_owned(), Value::String(asset_id));
                Ok(Value::Object(slot))
            }));
        let progress_playlist_id = playlist_id.to_owned();
        let progress_temp_id = temp_id.to_owned();
        let progress_slice = Rc::clone(&target_slice);
        files.on_byte_progress = Some(Arc::new(move |index: usize, completed: i64, total: i64, cx: &mut App| {
            let (aggregate_completed, aggregate_total) = {
                let mut completed_bytes = completed_bytes.borrow_mut();
                let mut total_bytes = total_bytes.borrow_mut();
                if index >= completed_bytes.len() {
                    return;
                }
                completed_bytes[index] = completed.max(0).min(total.max(0));
                total_bytes[index] = total.max(0);
                (completed_bytes.iter().sum::<i64>(), total_bytes.iter().sum::<i64>())
            };
            let progress =
                if aggregate_total > 0 { aggregate_completed as f64 / aggregate_total as f64 * 100.0 } else { 0.0 };
            let slice = Rc::clone(&progress_slice);
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.patch_local_track_upload_pending(&track, &progress_temp_id, progress, true, Some(slice), cx);
                services.patch_message_mp3_pending_track(
                    &progress_playlist_id,
                    &progress_temp_id,
                    MusicLocalTrackTransferPhase::Uploading,
                    aggregate_completed,
                    aggregate_total,
                    None,
                    cx,
                );
            });
            MusicTrackUploadNotifier::upload(&progress_temp_id, aggregate_completed, aggregate_total, cx);
        }));
        files.on_file_error = Some(Arc::new(move |_, error: &anyhow::Error, _: &mut App| {
            target_slice.mark_rejected(error);
        }));
        files
    }
}

impl MusicServicesStore {
    pub fn handle_request_playlist_join_success(
        &self,
        payload: Option<Box<dyn Any>>,
        token: &str,
        cx: &mut App,
    ) -> Option<MusicPlaylistJoinRequest> {
        let response = match payload?.downcast::<WireRequestPlaylistJoinResponseDTO>() {
            Ok(decoded) => *decoded,
            Err(payload) => SaiJSONCodec::decode::<WireRequestPlaylistJoinResponseDTO>(payload.downcast_ref::<Value>())?,
        };

        let preview_key = Self::playlist_invite_preview_cache_key(token);
        if let Some(cached) = self.current_cached_dto::<WirePlaylistInvitePreviewResponseDTO>(&preview_key, cx)
            && let Some(mut preview) = cached.preview.clone()
        {
            preview.viewer_state = if response.viewer_state.is_empty() {
                music_service::PLAYLIST_VIEWER_STATE_PENDING.to_owned()
            } else {
                response.viewer_state.clone()
            };
            let mut reply = cached;
            reply.preview = Some(preview);
            self.install_memory_dto(&reply, &preview_key, cx);
        }
        response.request.as_ref().and_then(|request| request.as_domain())
    }

    pub fn apply_review_playlist_join_request_optimistic(
        &self,
        playlist_id: &str,
        request_id: &str,
        cx: &mut App,
    ) -> MusicCollaborationMutationSnapshot {
        let requests_key = Self::playlist_join_requests_cache_key(playlist_id);
        let settings_key = Self::playlist_settings_cache_key(playlist_id);
        let snapshot = self.collaboration_snapshot(&[requests_key.clone(), settings_key], cx);
        let Some(cached) = self.current_cached_dto::<WirePlaylistJoinRequestsDTO>(&requests_key, cx) else {
            return snapshot;
        };

        let next: Vec<_> = cached.requests.iter().filter(|request| request.id != request_id).cloned().collect();
        if next.len() == cached.requests.len() {
            return snapshot;
        }
        self.install_memory_dto(
            &WirePlaylistJoinRequestsDTO {
                next_cursor: cached.next_cursor.clone(),
                pending_request_count: (cached.pending_request_count - 1).max(0),
                requests: next,
            },
            &requests_key,
            cx,
        );
        let pending_request_count =
            (self.playlist_settings(playlist_id, cx).map_or(1, |settings| settings.pending_request_count) - 1).max(0);
        self.patch_playlist_settings_summary(playlist_id, Some(pending_request_count), 0, None, cx);
        snapshot
    }

    pub fn apply_playlist_member_permissions_optimistic(
        &self,
        playlist_id: &str,
        member_user_id: &str,
        permissions: MusicPlaylistMemberPermissions,
        cx: &mut App,
    ) -> MusicCollaborationMutationSnapshot {
        let member_keys: Vec<String> = [false, true]
            .into_iter()
            .map(|include_suspended| Self::playlist_members_cache_key(playlist_id, include_suspended))
            .collect();
        let settings_key = Self::playlist_settings_cache_key(playlist_id);
        let mut snapshot_keys = member_keys.clone();
        snapshot_keys.push(settings_key);
        let snapshot = self.collaboration_snapshot(&snapshot_keys, cx);
        let wire_permissions = WirePlaylistMemberPermissionsDTO {
            can_add_tracks: permissions.can_add_tracks,
            can_edit_playlist: permissions.can_edit_playlist,
            can_remove_tracks: permissions.can_remove_tracks,
        };
        let mut viewer_member: Option<WirePlaylistMemberDTO> = None;
        for key in &member_keys {
            let Some(cached) = self.current_cached_dto::<WirePlaylistMembersDTO>(key, cx) else { continue };
            let mut members = cached.members.clone();
            let Some(index) = members.iter().position(|member| member.user_id == member_user_id) else { continue };
            let updated = WirePlaylistMemberDTO { permissions: Some(wire_permissions.clone()), ..members[index].clone() };
            members[index] = updated.clone();
            viewer_member = Some(updated);
            self.install_memory_dto(
                &WirePlaylistMembersDTO {
                    active_participant_count: cached.active_participant_count,
                    members,
                    next_cursor: cached.next_cursor.clone(),
                    total_participant_count: cached.total_participant_count,
                },
                key,
                cx,
            );
        }
        let viewer_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        if viewer_member.as_ref().map(|member| member.user_id.clone()) == viewer_id {
            self.patch_playlist_settings_summary(playlist_id, None, 0, viewer_member.as_ref(), cx);
        }
        snapshot
    }

    pub fn apply_playlist_member_removal_optimistic(
        &self,
        playlist_id: &str,
        member_user_id: &str,
        cx: &mut App,
    ) -> MusicCollaborationMutationSnapshot {
        let keys = [
            Self::playlist_members_cache_key(playlist_id, false),
            Self::playlist_members_cache_key(playlist_id, true),
            Self::playlist_settings_cache_key(playlist_id),
        ];
        let snapshot = self.collaboration_snapshot(&keys, cx);
        self.handle_playlist_member_removed(playlist_id, member_user_id, cx);
        snapshot
    }

    pub fn apply_leave_playlist_optimistic(&self, playlist_id: &str, cx: &mut App) -> MusicCollaborationMutationSnapshot {
        let key = SaiCache::new(["getPlaylistsMine"]).key;
        let mut roots = self.collaboration_snapshot(std::slice::from_ref(&key), cx).roots_by_cache_key;
        if !roots.contains_key(&key)
            && let Some(mine) = MusicActionsStore::slices(cx).get_playlists_mine_sai.data()
            && let Some(root) = SaiJSONCodec::encode(&mine)
        {
            roots.insert(key.clone(), root);
        }
        let snapshot = MusicCollaborationMutationSnapshot { roots_by_cache_key: roots };
        self.remove_playlist_from_library(playlist_id, cx);
        snapshot
    }

    fn remove_playlist_from_library(&self, playlist_id: &str, cx: &mut App) {
        let key = SaiCache::new(["getPlaylistsMine"]).key;
        let mine = match self.current_cached_dto::<RiseStoredMusicPlaylistLibrary>(&key, cx) {
            Some(mine) => Some(mine),
            None => MusicActionsStore::slices(cx).get_playlists_mine_sai.data(),
        };
        let Some(mine) = mine else { return };

        let updated = RiseStoredMusicPlaylistLibrary {
            playlists: Some(
                mine.playlists
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|playlist| playlist.id.as_deref() != Some(playlist_id))
                    .collect(),
            ),
        };
        self.install_memory_dto(&updated, &key, cx);
        let slice = MusicActionsStore::slices(cx).get_playlists_mine_sai.clone();
        Self::mark_slice(&slice, &updated);
    }

    pub fn handle_review_playlist_join_request_success(
        &self,
        payload: Option<Box<dyn Any>>,
        playlist_id: &str,
        request_id: &str,
        cx: &mut App,
    ) -> Option<MusicPlaylistMember> {
        let response = match payload?.downcast::<WireReviewPlaylistJoinRequestResponseDTO>() {
            Ok(decoded) => *decoded,
            Err(payload) => {
                SaiJSONCodec::decode::<WireReviewPlaylistJoinRequestResponseDTO>(payload.downcast_ref::<Value>())?
            }
        };

        let requests_key = Self::playlist_join_requests_cache_key(playlist_id);
        if let Some(cached) = self.current_cached_dto::<WirePlaylistJoinRequestsDTO>(&requests_key, cx) {
            self.install_memory_dto(
                &WirePlaylistJoinRequestsDTO {
                    next_cursor: cached.next_cursor.clone(),
                    pending_request_count: response.pending_request_count,
                    requests: cached.requests.iter().filter(|request| request.id != request_id).cloned().collect(),
                },
                &requests_key,
                cx,
            );
        }
        let member_already_present = match &response.member {
            Some(incoming) => [false, true].into_iter().any(|include_suspended| {
                let key = Self::playlist_members_cache_key(playlist_id, include_suspended);
                self.current_cached_dto::<WirePlaylistMembersDTO>(&key, cx)
                    .is_some_and(|cached| cached.members.iter().any(|member| member.user_id == incoming.user_id))
            }),
            None => false,
        };
        if let Some(member) = &response.member {
            self.upsert_playlist_member_in_caches(member, playlist_id, cx);
        }
        self.patch_playlist_settings_summary(
            playlist_id,
            Some(response.pending_request_count),
            if response.member.is_some() && !member_already_present { 1 } else { 0 },
            response.member.as_ref(),
            cx,
        );
        response.member.as_ref().and_then(|member| member.as_domain())
    }

    pub fn handle_update_playlist_member_success(
        &self,
        payload: Option<Box<dyn Any>>,
        playlist_id: &str,
        cx: &mut App,
    ) -> Option<MusicPlaylistMember> {
        let response = match payload?.downcast::<WirePlaylistMemberResponseDTO>() {
            Ok(decoded) => *decoded,
            Err(payload) => SaiJSONCodec::decode::<WirePlaylistMemberResponseDTO>(payload.downcast_ref::<Value>())?,
        };
        let member = response.member?;

        self.upsert_playlist_member_in_caches(&member, playlist_id, cx);
        self.patch_playlist_settings_summary(playlist_id, None, 0, Some(&member), cx);
        member.as_domain()
    }

    fn upsert_playlist_member_in_caches(&self, member: &WirePlaylistMemberDTO, playlist_id: &str, cx: &mut App) {
        for include_suspended in [false, true] {
            let key = Self::playlist_members_cache_key(playlist_id, include_suspended);
            let Some(cached) = self.current_cached_dto::<WirePlaylistMembersDTO>(&key, cx) else { continue };

            let mut members = cached.members.clone();
            if let Some(index) = members.iter().position(|row| row.user_id == member.user_id) {
                members[index] = member.clone();
            } else if include_suspended || member.status == music_service::PLAYLIST_MEMBER_STATUS_ACTIVE {
                members.push(member.clone());
            }
            if !include_suspended {
                members.retain(|row| row.status != music_service::PLAYLIST_MEMBER_STATUS_SUSPENDED_PLAN);
            }
            self.install_memory_dto(
                &WirePlaylistMembersDTO {
                    active_participant_count: cached.active_participant_count,
                    members,
                    next_cursor: cached.next_cursor.clone(),
                    total_participant_count: cached.total_participant_count,
                },
                &key,
                cx,
            );
        }
    }

    pub fn handle_playlist_member_removed(&self, playlist_id: &str, member_user_id: &str, cx: &mut App) {
        let mut removed_active_member = false;
        for include_suspended in [false, true] {
            let key = Self::playlist_members_cache_key(playlist_id, include_suspended);
            let Some(cached) = self.current_cached_dto::<WirePlaylistMembersDTO>(&key, cx) else { continue };

            let previous_members = &cached.members;
            let did_remove = previous_members.iter().any(|member| member.user_id == member_user_id);
            let removed_was_active = previous_members
                .iter()
                .find(|member| member.user_id == member_user_id)
                .is_some_and(|member| member.status != music_service::PLAYLIST_MEMBER_STATUS_SUSPENDED_PLAN);
            if !include_suspended && did_remove {
                removed_active_member = true;
            }
            let members: Vec<_> = previous_members.iter().filter(|member| member.user_id != member_user_id).cloned().collect();
            self.install_memory_dto(
                &WirePlaylistMembersDTO {
                    active_participant_count: (cached.active_participant_count
                        - i64::from(did_remove && removed_was_active))
                    .max(0),
                    members,
                    next_cursor: cached.next_cursor.clone(),
                    total_participant_count: (cached.total_participant_count - i64::from(did_remove)).max(0),
                },
                &key,
                cx,
            );
        }
        if removed_active_member {
            self.patch_playlist_settings_summary(playlist_id, None, -1, None, cx);
        }
    }

    pub fn handle_playlist_access_revoked(&mut self, playlist_id: &str, reason: &str, cx: &mut Context<Self>) {
        let active_session = self.active_playlist_listening_session(playlist_id, cx);
        let playback_scope_id = format!("playlist:{playlist_id}");
        let playback_will_notify =
            MusicPlayerState::shared(cx).read(cx).playback_scope_id.as_deref() == Some(playback_scope_id.as_str());

        for key in [
            Self::playlist_settings_cache_key(playlist_id),
            Self::playlist_invite_link_cache_key(playlist_id),
            Self::playlist_members_cache_key(playlist_id, false),
            Self::playlist_members_cache_key(playlist_id, true),
            Self::playlist_join_requests_cache_key(playlist_id),
            Self::playlist_listening_snapshot_cache_key(playlist_id, None),
            SaiCache::new(["getPlaylist", playlist_id]).key,
        ] {
            SaiCacheCleaner::clean(&[key], SaiCacheCleanLayer::Everywhere, cx);
        }
        let playlist_slice = MusicActionsStore::slices(cx).get_playlist_sai.clone();
        if playlist_slice.data().and_then(|detail| detail.playlist).and_then(|playlist| playlist.id).as_deref()
            == Some(playlist_id)
        {
            Self::mark_slice(&playlist_slice, &RiseStoredMusicPlaylistDetail { playlist: None });
        }
        self.remove_playlist_from_library(playlist_id, cx);

        let observation_keys: Vec<String> = self
            .listening_observations
            .iter()
            .filter(|(_, observation)| observation.playlist_id == playlist_id)
            .map(|(key, _)| key.clone())
            .collect();
        for key in observation_keys {
            self.listening_observation_tasks.remove(&key);
            self.listening_observations.remove(&key);
        }
        self.stop_playlist_listening_presence(playlist_id, false, cx);
        if let Some(active_session) = active_session {
            self.handle_listening_session_ended(&active_session.session_id, reason, cx);
        }
        self.notify_listening_session_observers(
            MusicListeningSessionUpdate::AccessRevoked { playlist_id: playlist_id.to_owned(), reason: reason.to_owned() },
            cx,
        );
        if reason != music_service::PLAYLIST_MEMBER_EXIT_LEFT {
            self.revoked_playlist_notice = Some(MusicPlaylistRevokedNotice::new(playlist_id.to_owned()));
        }
        if !playback_will_notify && reason != music_service::PLAYLIST_MEMBER_EXIT_LEFT {
            let is_subscription_suspension = reason == music_service::PLAYLIST_ACCESS_PAUSED_REASON;
            let is_deleted = reason == music_service::PLAYLIST_REVOKED_REASON_DELETED;
            let title_key = if is_subscription_suspension {
                "music_playlist_suspended_title"
            } else if is_deleted {
                "music_playlist_deleted"
            } else {
                "music_playlist_access_removed_title"
            };
            Notifier::send(
                tr(title_key),
                NotifierSendOptions {
                    content: if is_subscription_suspension {
                        Some(tr("music_playlist_suspended_hint"))
                    } else if is_deleted {
                        None
                    } else {
                        Some(tr("music_playlist_access_removed_hint"))
                    },
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Top,
                    icon: NotifierIcon::System("lock.fill".to_owned()),
                    is_notification: true,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        }
        cx.notify();
    }

    pub fn handle_playlist_access_restored(&self, playlist_id: &str, reason: &str, cx: &mut App) {
        let members_cached: Vec<(bool, bool)> = [false, true]
            .into_iter()
            .map(|include_suspended| {
                let key = Self::playlist_members_cache_key(playlist_id, include_suspended);
                (include_suspended, self.current_cached_dto::<WirePlaylistMembersDTO>(&key, cx).is_some())
            })
            .collect();
        let restored_playlist_id = playlist_id.to_owned();
        cx.defer(move |cx| {
            let actions = Self::actions(cx);
            if !actions.read(cx).get_playlist_sai.is_pending() {
                actions.update(cx, |actions, cx| {
                    actions.get_playlist_action(&restored_playlist_id, false, false, None, None, None, cx);
                });
            }
            if !actions.read(cx).get_playlists_mine_sai.is_pending() {
                actions.update(cx, |actions, cx| actions.get_playlists_mine_action(false, true, cx));
            }
        });
        self.schedule_playlist_collaboration_refresh(playlist_id, cx);
        let members_playlist_id = playlist_id.to_owned();
        cx.defer(move |cx| {
            let actions = Self::actions(cx);
            for (include_suspended, is_cached) in members_cached {
                if is_cached && !actions.read(cx).get_playlist_members_sai.is_pending() {
                    actions.update(cx, |actions, cx| {
                        actions.get_playlist_members_action(&members_playlist_id, include_suspended, None, false, cx);
                    });
                }
            }
        });
        if reason == music_service::PLAYLIST_ACCESS_RESTORED_REASON_MEMBER {
            self.notify_listening_session_observers(MusicListeningSessionUpdate::AccessRestored, cx);
        }
    }

    fn refresh_open_playlist_invite_preview(playlist_id: String, cx: &mut App) {
        cx.defer(move |cx| {
            MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.refresh_open_playlist_invite_preview(&playlist_id, cx);
            });
        });
    }

    fn schedule_playlist_collaboration_refresh(&self, playlist_id: &str, cx: &mut App) {
        if playlist_id.is_empty() {
            return;
        }

        let playlist_id = playlist_id.to_owned();
        cx.defer(move |cx| {
            let actions = Self::actions(cx);
            if !actions.read(cx).get_playlist_settings_sai.is_pending() {
                actions.update(cx, |actions, cx| actions.get_playlist_settings_action(&playlist_id, false, cx));
            }
        });
    }

    pub fn mark_playlist_mutation_acknowledged(&mut self, client_request_id: &str) {
        let id = trimming_whitespaces_and_newlines(client_request_id);
        if id.is_empty() {
            return;
        }
        let now = (Date::now().time_interval_since_1970() * 1_000.0) as i64;
        self.recent_playlist_mutation_request_ids.retain(|_, acknowledged_at| now - *acknowledged_at < 30_000);
        self.recent_playlist_mutation_request_ids.insert(id.to_owned(), now);
    }

    pub fn begin_playlist_collaboration_mutation(&mut self, playlist_id: &str) -> bool {
        if playlist_id.is_empty() || self.collaboration_mutation_playlists_in_flight.contains(playlist_id) {
            return false;
        }
        self.collaboration_mutation_playlists_in_flight.insert(playlist_id.to_owned());
        true
    }

    pub fn finish_playlist_collaboration_mutation(&mut self, playlist_id: &str) {
        self.collaboration_mutation_playlists_in_flight.remove(playlist_id);
    }

    fn patch_playlist_settings_summary(
        &self,
        playlist_id: &str,
        pending_request_count: Option<i64>,
        participant_delta: i64,
        viewer_member: Option<&WirePlaylistMemberDTO>,
        cx: &mut App,
    ) {
        let key = Self::playlist_settings_cache_key(playlist_id);
        let Some(cached) = self.current_cached_dto::<WirePlaylistSettingsResponseDTO>(&key, cx) else { return };
        let Some(settings) = cached.settings.clone() else { return };
        let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let updates_viewer = viewer_member.map(|member| member.user_id.clone()) == current_user_id;
        let mut updated = settings.clone();
        updated.active_participant_count = (settings.active_participant_count + participant_delta).max(0);
        updated.total_participant_count = (settings.total_participant_count + participant_delta).max(0);
        updated.pending_request_count = pending_request_count.unwrap_or(settings.pending_request_count);
        if updates_viewer && let Some(viewer_member) = viewer_member {
            updated.viewer_permissions = viewer_member.permissions.clone();
            updated.viewer_role = viewer_member.role.clone();
        }
        let mut reply = cached;
        reply.settings = Some(updated);
        self.install_memory_dto(&reply, &key, cx);
    }

    pub fn playlist_collaboration_event_handler(
        &mut self,
        payload: Option<&Value>,
        event_type: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = AnyDictionaryBridge::dictionary(payload) else { return };
        let Some(playlist_id) = self.string_value(root.get(key::PLAYLIST_ID)) else { return };
        if playlist_id.is_empty() {
            return;
        }
        if let Some(request_id) = self.string_value(root.get(key::CLIENT_REQUEST_ID))
            && self.recent_playlist_mutation_request_ids.contains_key(&request_id)
        {
            return;
        }

        match event_type {
            name::PLAYLIST_JOIN_REQUEST_CREATED | name::PLAYLIST_JOIN_REQUEST_REVIEWED => {
                if let Some(pending) = self.int64_value(root.get(key::PENDING_REQUEST_COUNT)) {
                    self.patch_playlist_settings_summary(&playlist_id, Some(pending.max(0)), 0, None, cx);
                }
                self.schedule_playlist_collaboration_refresh(&playlist_id, cx);
                let requests_cached = self
                    .current_cached_dto::<WirePlaylistJoinRequestsDTO>(&Self::playlist_join_requests_cache_key(&playlist_id), cx)
                    .is_some();
                cx.defer(move |cx| {
                    let actions = Self::actions(cx);
                    if !actions.read(cx).get_playlist_join_requests_sai.is_pending() && requests_cached {
                        actions.update(cx, |actions, cx| {
                            actions.get_playlist_join_requests_action(&playlist_id, None, false, cx);
                        });
                    }
                });
            }
            name::PLAYLIST_JOIN_APPROVED | name::PLAYLIST_JOIN_REJECTED => {
                Self::refresh_playlists_mine(cx);
                self.schedule_playlist_collaboration_refresh(&playlist_id, cx);
            }
            name::PLAYLIST_MEMBER_PERMISSIONS_UPDATED => {
                self.patch_viewer_permissions_from_event(&playlist_id, &root, cx);
                Self::refresh_playlists_mine(cx);
                self.schedule_playlist_collaboration_refresh(&playlist_id, cx);
                let members_cached: Vec<(bool, bool)> = [false, true]
                    .into_iter()
                    .map(|include_suspended| {
                        let key = Self::playlist_members_cache_key(&playlist_id, include_suspended);
                        (include_suspended, self.current_cached_dto::<WirePlaylistMembersDTO>(&key, cx).is_some())
                    })
                    .collect();
                cx.defer(move |cx| {
                    let actions = Self::actions(cx);
                    for (include_suspended, is_cached) in members_cached {
                        if is_cached && !actions.read(cx).get_playlist_members_sai.is_pending() {
                            actions.update(cx, |actions, cx| {
                                actions.get_playlist_members_action(&playlist_id, include_suspended, None, false, cx);
                            });
                        }
                    }
                });
            }
            name::PLAYLIST_ACCESS_REVOKED | name::PLAYLIST_ACCESS_SUSPENDED => {
                let reason = self.string_value(root.get(key::REASON)).unwrap_or_else(|| event_type.to_owned());
                self.handle_playlist_access_revoked(&playlist_id, &reason, cx);
                Self::refresh_open_playlist_invite_preview(playlist_id, cx);
            }
            name::PLAYLIST_ACCESS_RESTORED => {
                let reason = self.string_value(root.get(key::REASON)).unwrap_or_else(|| event_type.to_owned());
                self.handle_playlist_access_restored(&playlist_id, &reason, cx);
                Self::refresh_open_playlist_invite_preview(playlist_id, cx);
            }
            name::PLAYLIST_CONTENT_CHANGED => {
                self.handle_playlist_content_changed_event(&root, &playlist_id, cx);
            }
            name::PLAYLIST_LISTENING_SESSION_ENDED => {
                if let Some(session_id) = self.string_value(root.get(key::SESSION_ID)) {
                    let reason = self
                        .string_value(root.get(key::REASON))
                        .unwrap_or_else(|| music_service::LISTENING_END_REASON_HOST_ENDED.to_owned());
                    self.handle_listening_session_ended(&session_id, &reason, cx);
                }
            }
            name::PLAYLIST_LISTENING_PARTICIPANT_JOINED
            | name::PLAYLIST_LISTENING_PERMISSION_REQUESTED
            | name::PLAYLIST_LISTENING_PARTICIPANT_LEFT
            | name::PLAYLIST_LISTENING_PERMISSIONS_UPDATED => {
                if let Some(session_id) = self.string_value(root.get(key::SESSION_ID)) {
                    self.schedule_listening_session_recovery(&session_id, cx);
                }
            }
            _ => {}
        }
    }

    fn patch_viewer_permissions_from_event(&self, playlist_id: &str, root: &Map<String, Value>, cx: &mut App) {
        let Some(member_user_id) = self.string_value(root.get(key::MEMBER_USER_ID)) else { return };
        let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        if current_user_id.as_deref() != Some(member_user_id.as_str()) {
            return;
        }
        let cache_key = Self::playlist_settings_cache_key(playlist_id);
        let Some(mut reply) = self.current_cached_dto::<WirePlaylistSettingsResponseDTO>(&cache_key, cx) else { return };
        let Some(settings) = reply.settings.as_mut() else { return };
        let permissions = root.get(key::PERMISSIONS).and_then(Value::as_object).unwrap_or(root);
        let previous = settings.viewer_permissions.clone();
        let flag = |name: &str, fallback: bool| self.bool_value(permissions.get(name)).unwrap_or(fallback);
        let updated = WirePlaylistMemberPermissionsDTO {
            can_add_tracks: flag(key::CAN_ADD_TRACKS, previous.as_ref().is_some_and(|p| p.can_add_tracks)),
            can_edit_playlist: flag(key::CAN_EDIT_PLAYLIST, previous.as_ref().is_some_and(|p| p.can_edit_playlist)),
            can_remove_tracks: flag(key::CAN_REMOVE_TRACKS, previous.as_ref().is_some_and(|p| p.can_remove_tracks)),
        };
        if previous.as_ref() == Some(&updated) {
            return;
        }
        settings.viewer_permissions = Some(updated);
        self.install_memory_dto(&reply, &cache_key, cx);
    }

    fn refresh_playlists_mine(cx: &mut App) {
        cx.defer(|cx| {
            let actions = Self::actions(cx);
            if !actions.read(cx).get_playlists_mine_sai.is_pending() {
                actions.update(cx, |actions, cx| actions.get_playlists_mine_action(false, true, cx));
            }
        });
    }

    fn handle_playlist_content_changed_event(&self, root: &Map<String, Value>, playlist_id: &str, cx: &mut App) {
        let revision = self.int64_value(root.get(key::REVISION)).unwrap_or(0);
        let settings_revision = self.playlist_settings(playlist_id, cx).map_or(0, |settings| settings.playlist.revision);
        let playlist_revision = self.playlist(playlist_id, cx).and_then(|playlist| playlist.revision).unwrap_or(0);
        if revision <= settings_revision.max(playlist_revision) {
            return;
        }

        let changed_playlist_id = playlist_id.to_owned();
        cx.defer(move |cx| {
            let actions = Self::actions(cx);
            if !actions.read(cx).get_playlist_sai.is_pending() {
                actions.update(cx, |actions, cx| {
                    actions.get_playlist_action(&changed_playlist_id, false, false, None, None, None, cx);
                });
            }
        });
        self.schedule_playlist_collaboration_refresh(playlist_id, cx);
        Self::refresh_playlists_mine(cx);
    }

    fn string_value(&self, value: Option<&Value>) -> Option<String> {
        match value? {
            Value::String(value) => {
                let normalized = trimming_whitespaces_and_newlines(value);
                if normalized.is_empty() { None } else { Some(normalized.to_owned()) }
            }
            Value::Number(value) => Some(value.to_string()),
            Value::Bool(value) => Some(i64::from(*value).to_string()),
            _ => None,
        }
    }

    fn int64_value(&self, value: Option<&Value>) -> Option<i64> {
        match value? {
            Value::Number(number) => number
                .as_i64()
                .or_else(|| number.as_u64().map(|value| value as i64))
                .or_else(|| number.as_f64().map(|value| value as i64)),
            Value::Bool(value) => Some(i64::from(*value)),
            Value::String(value) => value.parse::<i64>().ok(),
            _ => None,
        }
    }

    fn bool_value(&self, value: Option<&Value>) -> Option<bool> {
        match value? {
            Value::Bool(value) => Some(*value),
            Value::Number(number) => Some(number.as_f64().is_some_and(|value| value != 0.0)),
            Value::String(value) => {
                if value == "1" || value.to_lowercase() == "true" {
                    Some(true)
                } else if value == "0" || value.to_lowercase() == "false" {
                    Some(false)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn add_listening_session_update_observer(&mut self, observer: MusicListeningSessionObserver) -> Uuid {
        let id = Uuid::new_v4();
        self.listening_session_observers.insert(id, Rc::from(observer));
        id
    }

    pub fn remove_listening_session_update_observer(&mut self, id: Uuid) {
        self.listening_session_observers.remove(&id);
    }

    fn notify_listening_session_observers(&self, update: MusicListeningSessionUpdate, cx: &mut App) {
        let observers: Vec<Rc<dyn Fn(&MusicListeningSessionUpdate, &mut App)>> =
            self.listening_session_observers.values().cloned().collect();
        cx.defer(move |cx| {
            for observer in observers {
                observer(&update, cx);
            }
        });
    }
}

impl MusicServicesStore {
    pub fn handle_playlist_listening_snapshot_success(
        &mut self,
        payload: Option<Box<dyn Any>>,
        playlist_id: &str,
        track_id: Option<&str>,
        request_started_at_ms: i64,
        cx: &mut App,
    ) {
        let Some(payload) = payload else { return };
        let incoming = match payload.downcast::<WirePlaylistListeningSnapshotDTO>() {
            Ok(decoded) => *decoded,
            Err(payload) => {
                let Some(decoded) = SaiJSONCodec::decode::<WirePlaylistListeningSnapshotDTO>(payload.downcast_ref::<Value>())
                else {
                    return;
                };
                decoded
            }
        };

        let cache_key = Self::playlist_listening_snapshot_cache_key(playlist_id, track_id);
        let previous = self.current_cached_dto::<WirePlaylistListeningSnapshotDTO>(&cache_key, cx);
        let merged = match previous {
            Some(mut kept) if incoming.unchanged => {
                if incoming.presence_sequence > 0 {
                    kept.presence_sequence = incoming.presence_sequence;
                }
                if incoming.server_time_ms > 0 {
                    kept.server_time_ms = incoming.server_time_ms;
                }
                if incoming.watcher_lease_until_ms > 0 {
                    kept.watcher_lease_until_ms = incoming.watcher_lease_until_ms;
                }
                kept.active_session = incoming.active_session.clone();
                kept.unchanged = true;
                kept
            }
            _ => self.without_ended_session_listeners(incoming),
        };
        self.install_memory_dto(&merged, &cache_key, cx);

        let received_at_ms = (Date::now().time_interval_since_1970() * 1_000.0) as i64;
        if merged.server_time_ms > 0 {
            self.update_server_clock_offset(playlist_id, merged.server_time_ms, request_started_at_ms, received_at_ms);
        }
        if let Some(session) = merged.active_session.as_ref() {
            let server_time_ms = if merged.server_time_ms > 0 {
                merged.server_time_ms
            } else if session.anchor_server_time_ms > 0 {
                session.anchor_server_time_ms
            } else {
                self.estimated_playlist_server_time_ms(playlist_id, received_at_ms)
            };
            self.install_listening_session(session, server_time_ms, true, None, false, cx);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn handle_listening_session_response(
        &mut self,
        payload: Option<Box<dyn Any>>,
        fallback_server_time_ms: Option<i64>,
        request_started_at_ms: Option<i64>,
        notify_observers: bool,
        is_explicit_join: bool,
        cx: &mut App,
    ) -> Option<MusicPlaylistListeningSession> {
        let response = match payload?.downcast::<WirePlaylistListeningSessionResponseDTO>() {
            Ok(decoded) => *decoded,
            Err(payload) => {
                SaiJSONCodec::decode::<WirePlaylistListeningSessionResponseDTO>(payload.downcast_ref::<Value>())?
            }
        };
        let session = response.session.as_ref()?;

        let received_at_ms = (Date::now().time_interval_since_1970() * 1_000.0) as i64;
        let reported_server_time_ms = (response.server_time_ms > 0).then_some(response.server_time_ms);
        let server_time_ms = reported_server_time_ms
            .or(fallback_server_time_ms)
            .unwrap_or_else(|| self.estimated_playlist_server_time_ms(&session.playlist_id, received_at_ms));
        let playlist_id = session.playlist_id.as_str();
        if !playlist_id.is_empty() && reported_server_time_ms.is_some() {
            self.update_server_clock_offset(
                playlist_id,
                server_time_ms,
                request_started_at_ms.unwrap_or(received_at_ms),
                received_at_ms,
            );
        }
        self.install_listening_session(session, server_time_ms, notify_observers, None, is_explicit_join, cx)
    }

    fn install_listening_session(
        &mut self,
        session: &WirePlaylistListeningSessionDTO,
        server_time_ms: i64,
        notify: bool,
        actor_user_id: Option<&str>,
        is_explicit_join: bool,
        cx: &mut App,
    ) -> Option<MusicPlaylistListeningSession> {
        let domain = session.as_domain()?;

        if is_explicit_join {
            self.ended_listening_session_ids.remove(&domain.session_id);
        } else if self.is_listening_session_tombstoned(&domain.session_id) {
            return None;
        }

        let cache_key = Self::playlist_listening_session_cache_key(&domain.session_id);
        if let Some(current_wire) = self
            .current_cached_dto::<WirePlaylistListeningSessionResponseDTO>(&cache_key, cx)
            .and_then(|response| response.session)
            && let Some(current) = current_wire.as_domain()
            && (current.generation > domain.generation
                || (current.generation == domain.generation && current.revision > domain.revision))
        {
            self.patch_active_session_in_snapshots(Some(&current_wire), &current.playlist_id, cx);
            return Some(current);
        }

        self.install_memory_dto(
            &WirePlaylistListeningSessionResponseDTO { server_time_ms, session: Some(session.clone()) },
            &cache_key,
            cx,
        );
        self.patch_active_session_in_snapshots(Some(session), &domain.playlist_id, cx);
        if notify {
            self.notify_listening_session_observers(
                MusicListeningSessionUpdate::Session {
                    session: domain.clone(),
                    server_time_ms,
                    actor_user_id: actor_user_id.map(str::to_owned),
                },
                cx,
            );
        }
        Some(domain)
    }

    fn remove_cached_listening_participant(
        &mut self,
        session_id: &str,
        participant_user_id: &str,
        generation: Option<i64>,
        revision: Option<i64>,
        cx: &mut App,
    ) -> bool {
        if self.is_listening_session_tombstoned(session_id) {
            return false;
        }
        let cache_key = Self::playlist_listening_session_cache_key(session_id);
        let Some(cached) = self.current_cached_dto::<WirePlaylistListeningSessionResponseDTO>(&cache_key, cx) else {
            return true;
        };
        let Some(session) = cached.session.as_ref() else { return true };
        let playlist_id = session.playlist_id.clone();

        let current_generation = session.generation;
        let incoming_generation = generation.unwrap_or(current_generation);
        let incoming_revision = revision.unwrap_or(session.revision);
        if !(incoming_generation > current_generation
            || (incoming_generation == current_generation && incoming_revision >= session.revision))
        {
            return false;
        }

        let patched = WirePlaylistListeningSessionDTO {
            generation: incoming_generation,
            participants: session
                .participants
                .iter()
                .filter(|participant| participant.user_id != participant_user_id)
                .cloned()
                .collect(),
            revision: incoming_revision,
            ..session.clone()
        };
        self.install_memory_dto(
            &WirePlaylistListeningSessionResponseDTO { server_time_ms: cached.server_time_ms, session: Some(patched.clone()) },
            &cache_key,
            cx,
        );
        self.patch_active_session_in_snapshots(Some(&patched), &playlist_id, cx);
        true
    }

    fn patch_cached_listening_permissions(
        &self,
        session_id: &str,
        participant_user_id: &str,
        permissions: &MusicListeningParticipantPermissions,
        cx: &mut App,
    ) {
        let cache_key = Self::playlist_listening_session_cache_key(session_id);
        let Some(cached) = self.current_cached_dto::<WirePlaylistListeningSessionResponseDTO>(&cache_key, cx) else {
            return;
        };
        let Some(session) = cached.session.as_ref() else { return };
        let playlist_id = session.playlist_id.clone();

        let wire_permissions = WireListeningParticipantPermissionsDTO {
            can_apply_effects: permissions.can_apply_effects,
            can_change_tracks: permissions.can_change_tracks,
            can_control_playback: permissions.can_control_playback,
            can_seek: permissions.can_seek,
        };
        let participants = session
            .participants
            .iter()
            .map(|participant| {
                if participant.user_id == participant_user_id {
                    WireListeningParticipantDTO {
                        joined_at_ms: participant.joined_at_ms,
                        permissions: Some(wire_permissions.clone()),
                        user: participant.user.clone(),
                        user_id: participant.user_id.clone(),
                    }
                } else {
                    participant.clone()
                }
            })
            .collect();
        let is_viewer =
            AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().as_deref() == Some(participant_user_id);
        let patched = WirePlaylistListeningSessionDTO {
            participants,
            viewer_permissions: if is_viewer { Some(wire_permissions) } else { session.viewer_permissions.clone() },
            ..session.clone()
        };

        self.install_memory_dto(
            &WirePlaylistListeningSessionResponseDTO { server_time_ms: cached.server_time_ms, session: Some(patched.clone()) },
            &cache_key,
            cx,
        );
        self.patch_active_session_in_snapshots(Some(&patched), &playlist_id, cx);
    }

    fn patch_active_session_in_snapshots(
        &self,
        session: Option<&WirePlaylistListeningSessionDTO>,
        playlist_id: &str,
        cx: &mut App,
    ) {
        for key in self.known_snapshot_cache_keys(playlist_id) {
            let Some(cached) = self.current_cached_dto::<WirePlaylistListeningSnapshotDTO>(&key, cx) else { continue };

            let mut updated = cached;
            updated.active_session = session.cloned();
            self.install_memory_dto(&updated, &key, cx);
        }
    }

    fn release_ended_session_listeners(&mut self, playlist_id: Option<&str>, cx: &mut App) {
        let mut playlist_ids: HashSet<String> =
            self.listening_observations.values().map(|observation| observation.playlist_id.clone()).collect();
        if let Some(playlist_id) = playlist_id {
            playlist_ids.insert(playlist_id.to_owned());
        }
        for pid in playlist_ids {
            for key in self.known_snapshot_cache_keys(&pid) {
                let Some(cached) = self.current_cached_dto::<WirePlaylistListeningSnapshotDTO>(&key, cx) else { continue };
                let released = self.without_ended_session_listeners(cached.clone());
                if released != cached {
                    self.install_memory_dto(&released, &key, cx);
                }
            }
        }
    }

    fn without_ended_session_listeners(
        &mut self,
        snapshot: WirePlaylistListeningSnapshotDTO,
    ) -> WirePlaylistListeningSnapshotDTO {
        let mut updated = snapshot;
        let listeners = std::mem::take(&mut updated.listeners);
        updated.listeners = listeners
            .into_iter()
            .map(|listener| {
                let Some(sid) = listener.session_id.as_deref() else { return listener };
                if !self.is_listening_session_tombstoned(sid) {
                    return listener;
                }
                let mut released = listener;
                released.session_id = None;
                released
            })
            .collect();
        updated
    }

    fn known_snapshot_cache_keys(&self, playlist_id: &str) -> HashSet<String> {
        let mut keys = HashSet::from([Self::playlist_listening_snapshot_cache_key(playlist_id, None)]);
        for observation in self.listening_observations.values().filter(|observation| observation.playlist_id == playlist_id) {
            keys.insert(Self::playlist_listening_snapshot_cache_key(playlist_id, observation.track_id.as_deref()));
        }
        keys
    }

    fn update_server_clock_offset(
        &mut self,
        playlist_id: &str,
        server_time_ms: i64,
        request_started_at_ms: i64,
        received_at_ms: i64,
    ) {
        if playlist_id.is_empty() {
            return;
        }

        let midpoint = (request_started_at_ms + (received_at_ms - request_started_at_ms).max(0) / 2) as f64;
        let sample = server_time_ms as f64 - midpoint;
        let next = match self.server_clock_offset_ms_by_playlist.get(playlist_id) {
            Some(current) if (current - sample).abs() < 5_000.0 => current * 0.8 + sample * 0.2,
            _ => sample,
        };
        self.server_clock_offset_ms_by_playlist.insert(playlist_id.to_owned(), next);
    }

    pub fn handle_listening_session_refresh_error(
        &mut self,
        session_id: &str,
        message: &str,
        code: Option<&str>,
        cx: &mut App,
    ) {
        if let Some(code) = code
            && !code.is_empty()
        {
            let ends = code == MusicListeningServerErrorCode::NOT_FOUND
                || code == MusicListeningServerErrorCode::MEMBERSHIP_LOST
                || code == MusicListeningServerErrorCode::FORBIDDEN;
            if ends {
                self.handle_listening_session_ended(session_id, music_service::LISTENING_END_REASON_HOST_TIMEOUT, cx);
            }
            return;
        }

        let normalized = message.to_lowercase();
        if normalized.contains("not found") || normalized.contains("ended") {
            self.handle_listening_session_ended(session_id, music_service::LISTENING_END_REASON_HOST_TIMEOUT, cx);
        }
    }

    pub fn handle_local_listening_session_exit(&mut self, session_id: &str, cx: &mut App) {
        self.handle_listening_session_ended(session_id, music_service::PLAYLIST_MEMBER_EXIT_LEFT, cx);
    }

    pub fn handle_listening_session_ended(&mut self, session_id: &str, reason: &str, cx: &mut App) {
        let session = self.playlist_listening_session(session_id, cx);
        self.tombstone_listening_session(session_id);
        self.stop_playlist_listening_session_sync(session_id, cx);
        self.listening_state_command_in_flight.remove(session_id);
        self.pending_listening_state_commands.remove(session_id);
        self.listening_state_command_recovery.remove(session_id);
        self.listening_session_recovery_tasks.remove(session_id);

        if let Some(session) = &session {
            self.patch_active_session_in_snapshots(None, &session.playlist_id, cx);
        }
        self.release_ended_session_listeners(session.as_ref().map(|session| session.playlist_id.as_str()), cx);
        SaiCacheCleaner::clean(
            &[Self::playlist_listening_session_cache_key(session_id)],
            SaiCacheCleanLayer::Everywhere,
            cx,
        );
        self.notify_listening_session_observers(
            MusicListeningSessionUpdate::Ended { session_id: session_id.to_owned(), reason: reason.to_owned() },
            cx,
        );
    }

    fn tombstone_listening_session(&mut self, session_id: &str) {
        self.ended_listening_session_ids.insert(session_id.to_owned(), Date::now());
        let limit = usize::try_from(Self::ended_listening_sessions_limit()).unwrap_or(0);
        if self.ended_listening_session_ids.len() <= limit {
            return;
        }

        let mut keep_after: Vec<(String, Date)> = self.ended_listening_session_ids.drain().collect();
        keep_after.sort_by(|lhs, rhs| rhs.1.cmp(&lhs.1));
        keep_after.truncate(limit);
        self.ended_listening_session_ids = keep_after.into_iter().collect();
    }

    fn is_listening_session_tombstoned(&mut self, session_id: &str) -> bool {
        let Some(ended_at) = self.ended_listening_session_ids.get(session_id) else { return false };
        if Date::now().time_interval_since(ended_at) > Self::ended_listening_session_ttl() {
            self.ended_listening_session_ids.remove(session_id);
            return false;
        }
        true
    }

    pub fn schedule_listening_session_recovery(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if session_id.is_empty() {
            return;
        }

        let recovered_session_id = session_id.to_owned();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(150)).await;
            if this.upgrade().is_none() {
                return;
            }
            cx.update(|cx| {
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.get_playlist_listening_session_action(&recovered_session_id, false, None, None, cx);
                });
            });
            let _ = this.update(cx, |this, _| {
                this.listening_session_recovery_tasks.remove(&recovered_session_id);
            });
        });
        self.listening_session_recovery_tasks.insert(session_id.to_owned(), task);
    }

    pub fn schedule_playlist_listening_snapshot_recovery(&mut self, playlist_id: &str, cx: &mut Context<Self>) {
        if playlist_id.is_empty() {
            return;
        }

        let recovered_playlist_id = playlist_id.to_owned();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(120)).await;
            let _ = this.update(cx, |this, cx| {
                let observed_track_ids: Vec<Option<String>> = this
                    .listening_observations
                    .values()
                    .filter(|observation| observation.playlist_id == recovered_playlist_id)
                    .map(|observation| observation.track_id.clone())
                    .collect();
                let snapshot_key = Self::playlist_listening_snapshot_cache_key(&recovered_playlist_id, None);
                if observed_track_ids.is_empty()
                    && this.current_cached_dto::<WirePlaylistListeningSnapshotDTO>(&snapshot_key, cx).is_some()
                {
                    this.request_playlist_listening_snapshot(&recovered_playlist_id, None, false, cx);
                } else {
                    for track_id in &observed_track_ids {
                        this.request_playlist_listening_snapshot(&recovered_playlist_id, track_id.as_deref(), false, cx);
                    }
                }
                this.playlist_snapshot_recovery_tasks.remove(&recovered_playlist_id);
            });
        });
        self.playlist_snapshot_recovery_tasks.insert(playlist_id.to_owned(), task);
    }

    fn request_playlist_listening_snapshot(
        &self,
        playlist_id: &str,
        track_id: Option<&str>,
        reflect_pending_in_ui: bool,
        cx: &mut App,
    ) {
        let sequence = self
            .current_cached_dto::<WirePlaylistListeningSnapshotDTO>(
                &Self::playlist_listening_snapshot_cache_key(playlist_id, track_id),
                cx,
            )
            .map(|snapshot| snapshot.presence_sequence);
        let playlist_id = playlist_id.to_owned();
        let track_id = track_id.map(str::to_owned);
        cx.defer(move |cx| {
            Self::actions(cx).update(cx, |actions, cx| {
                actions.get_playlist_listening_snapshot_action(
                    &playlist_id,
                    track_id.as_deref(),
                    sequence,
                    reflect_pending_in_ui,
                    cx,
                );
            });
        });
    }
}

impl MusicServicesStore {
    pub fn playlist_realtime_event_handler(&mut self, payload: Option<&Value>, event_type: &str, cx: &mut Context<Self>) {
        let Some(root) = AnyDictionaryBridge::dictionary(payload) else { return };
        let Some(playlist_id) = self.string_value(root.get(key::PLAYLIST_ID)) else { return };
        if playlist_id.is_empty() {
            return;
        }

        let received_at_ms = (Date::now().time_interval_since_1970() * 1_000.0) as i64;
        if let Some(server_time_ms) = self.int64_value(root.get(key::SERVER_TIME_MS)) {
            self.update_server_clock_offset(&playlist_id, server_time_ms, received_at_ms, received_at_ms);
        }

        match event_type {
            name::PLAYLIST_PRESENCE_DELTA => {
                self.schedule_playlist_listening_snapshot_recovery(&playlist_id, cx);
            }
            name::LISTENING_SESSION_STATE => {
                self.apply_realtime_listening_session_state(&root, events::STATE, cx);
                if let Some(session_id) = self.string_value(root.get(key::SESSION_ID)) {
                    self.schedule_listening_session_recovery(&session_id, cx);
                }
            }
            name::LISTENING_PERMISSION_REQUESTED => {
                if let Some(session_id) = self.string_value(root.get(key::SESSION_ID))
                    && let Some(participant_user_id) = self.string_value(root.get(key::PARTICIPANT_USER_ID))
                    && let Some(raw_permissions) = root.get(key::REQUESTED_PERMISSIONS)
                    && let Some(permissions) =
                        SaiJSONCodec::decode::<WireListeningParticipantPermissionsDTO>(Some(raw_permissions))
                            .map(|permissions| permissions.as_domain())
                {
                    self.notify_listening_session_observers(
                        MusicListeningSessionUpdate::PermissionRequested { session_id, participant_user_id, permissions },
                        cx,
                    );
                }
            }
            name::LISTENING_SESSION_ENDED => {
                let Some(session_id) = self.string_value(root.get(key::SESSION_ID)) else { return };
                self.apply_realtime_listening_session_state(&root, events::LAST_STATE, cx);
                let reason = self
                    .string_value(root.get(key::REASON))
                    .unwrap_or_else(|| music_service::LISTENING_END_REASON_HOST_ENDED.to_owned());
                self.handle_listening_session_ended(&session_id, &reason, cx);
            }
            name::LISTENING_PARTICIPANT_JOINED => {
                if let Some(session_id) = self.string_value(root.get(key::SESSION_ID)) {
                    self.schedule_listening_session_recovery(&session_id, cx);
                }
                self.schedule_playlist_listening_snapshot_recovery(&playlist_id, cx);
            }
            name::LISTENING_PARTICIPANT_LEFT => {
                if let Some(session_id) = self.string_value(root.get(key::SESSION_ID)) {
                    if let Some(participant_user_id) = self.string_value(root.get(key::PARTICIPANT_USER_ID)) {
                        let generation = self.int64_value(root.get(key::GENERATION));
                        let revision = self.int64_value(root.get(key::REVISION));
                        if !self.remove_cached_listening_participant(
                            &session_id,
                            &participant_user_id,
                            generation,
                            revision,
                            cx,
                        ) {
                            return;
                        }
                        let participant_user_dto = SaiJSONCodec::decode::<WireUserRefDTO>(
                            root.get(events::PARTICIPANT_USER).or_else(|| root.get(key::USER)),
                        );
                        let participant_user =
                            participant_user_dto.map(|user| user.as_music_user_reference(&participant_user_id));
                        self.notify_listening_session_observers(
                            MusicListeningSessionUpdate::ParticipantLeft {
                                session_id: session_id.clone(),
                                participant_user_id,
                                participant_user,
                            },
                            cx,
                        );
                    }
                    self.schedule_listening_session_recovery(&session_id, cx);
                }
                self.schedule_playlist_listening_snapshot_recovery(&playlist_id, cx);
            }
            name::LISTENING_PERMISSIONS_UPDATED => {
                if let Some(session_id) = self.string_value(root.get(key::SESSION_ID)) {
                    if let Some(participant_user_id) = self.string_value(root.get(key::PARTICIPANT_USER_ID))
                        && let Some(permissions) =
                            SaiJSONCodec::decode::<WireListeningParticipantPermissionsDTO>(root.get(key::PERMISSIONS))
                                .map(|permissions| permissions.as_domain())
                    {
                        self.patch_cached_listening_permissions(&session_id, &participant_user_id, &permissions, cx);
                        self.notify_listening_session_observers(
                            MusicListeningSessionUpdate::PermissionsUpdated {
                                session_id: session_id.clone(),
                                participant_user_id,
                                permissions,
                            },
                            cx,
                        );
                    }
                    self.schedule_listening_session_recovery(&session_id, cx);
                }
                self.schedule_playlist_listening_snapshot_recovery(&playlist_id, cx);
            }
            _ => {}
        }
    }

    fn apply_realtime_listening_session_state(&mut self, root: &Map<String, Value>, state_key: &str, cx: &mut App) {
        let Some(session_id) = self.string_value(root.get(key::SESSION_ID)) else { return };
        let Some(state) = AnyDictionaryBridge::dictionary(root.get(state_key)) else { return };
        let Some(cached) = self.current_cached_dto::<WirePlaylistListeningSessionResponseDTO>(
            &Self::playlist_listening_session_cache_key(&session_id),
            cx,
        ) else {
            return;
        };
        let Some(session) = cached.session.as_ref() else { return };

        let incoming_generation = self.int64_value(root.get(key::GENERATION)).unwrap_or(session.generation);
        let incoming_revision = self.int64_value(root.get(key::REVISION)).unwrap_or(session.revision);
        if !RiseIdentity::realtime_applies(
            Some(&RiseRealtimeStamp {
                session_id: session_id.clone(),
                generation: session.generation,
                revision: session.revision,
            }),
            &RiseRealtimeStamp { session_id: session_id.clone(), generation: incoming_generation, revision: incoming_revision },
        ) {
            return;
        }

        let updated = WirePlaylistListeningSessionDTO {
            anchor_position_ms: self.int64_value(state.get(key::ANCHOR_POSITION_MS)).unwrap_or(session.anchor_position_ms),
            anchor_server_time_ms: self
                .int64_value(state.get(key::ANCHOR_SERVER_TIME_MS))
                .unwrap_or(session.anchor_server_time_ms),
            audio_effect_enabled: self
                .bool_value(state.get(key::AUDIO_EFFECT_ENABLED))
                .unwrap_or(session.audio_effect_enabled),
            audio_effect_rate_milli: self
                .int64_value(state.get(key::AUDIO_EFFECT_RATE_MILLI))
                .unwrap_or(session.audio_effect_rate_milli),
            audio_effects: self.realtime_audio_effects(&state).unwrap_or_else(|| session.audio_effects.clone()),
            generation: incoming_generation,
            is_playing: self.bool_value(state.get(key::IS_PLAYING)).unwrap_or(session.is_playing),
            repeat_mode: self.string_value(state.get(key::REPEAT_MODE)).unwrap_or_else(|| session.repeat_mode.clone()),
            revision: incoming_revision,
            shuffle_enabled: self.bool_value(state.get(key::SHUFFLE_ENABLED)).unwrap_or(session.shuffle_enabled),
            shuffle_seed: self.int64_value(state.get(key::SHUFFLE_SEED)).unwrap_or(session.shuffle_seed),
            track_id: self.string_value(state.get(key::TRACK_ID)).unwrap_or_else(|| session.track_id.clone()),
            ..session.clone()
        };
        let server_time_ms = self.int64_value(root.get(key::SERVER_TIME_MS)).unwrap_or_else(|| {
            if updated.anchor_server_time_ms > 0 {
                updated.anchor_server_time_ms
            } else {
                (Date::now().time_interval_since_1970() * 1_000.0) as i64
            }
        });
        let actor_user_id = self.string_value(root.get(key::ACTOR_USER_ID));
        let _ = self.install_listening_session(&updated, server_time_ms, true, actor_user_id.as_deref(), false, cx);
    }

    fn realtime_audio_effects(&self, state: &Map<String, Value>) -> Option<Vec<WireListeningAudioEffectDTO>> {
        let Some(Value::Array(raw)) = state.get(key::AUDIO_EFFECTS) else { return None };
        Some(
            raw.iter()
                .filter_map(|entry| {
                    let object = AnyDictionaryBridge::dictionary(Some(entry))?;
                    let kind = self.string_value(object.get(key::KIND))?;
                    let value_milli = self.int64_value(object.get(key::VALUE_MILLI))?;
                    Some(WireListeningAudioEffectDTO { kind, value_milli })
                })
                .collect(),
        )
    }
}

fn once_value_handler<T: 'static>(handler: Option<Box<dyn FnOnce(T, &mut App)>>) -> Option<Rc<dyn Fn(T, &mut App)>> {
    let slot = RefCell::new(Some(handler?));
    Some(Rc::new(move |value: T, cx: &mut App| {
        let handler = slot.borrow_mut().take();
        if let Some(handler) = handler {
            handler(value, cx);
        }
    }))
}

fn once_handler(handler: Option<Box<dyn FnOnce(&mut App)>>) -> Option<Rc<dyn Fn(&mut App)>> {
    let slot = RefCell::new(Some(handler?));
    Some(Rc::new(move |cx: &mut App| {
        let handler = slot.borrow_mut().take();
        if let Some(handler) = handler {
            handler(cx);
        }
    }))
}

fn local_now_ms() -> i64 {
    (Date::now().time_interval_since_1970() * 1_000.0) as i64
}

impl MusicServicesStore {
    pub fn start_playlist_listening_observation(
        &mut self,
        playlist_id: &str,
        track_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if playlist_id.is_empty() {
            return;
        }

        let cache_key = Self::playlist_listening_snapshot_cache_key(playlist_id, track_id);
        if self.listening_observation_tasks.contains_key(&cache_key) {
            return;
        }

        self.listening_observations.insert(
            cache_key.clone(),
            MusicListeningObservation { playlist_id: playlist_id.to_owned(), track_id: track_id.map(str::to_owned) },
        );
        let snapshot_poll_interval = RiseTimingCanon::heartbeat("music_snapshot_poll").client_interval_nanoseconds();

        let observed_key = cache_key.clone();
        let observed_playlist_id = playlist_id.to_owned();
        let observed_track_id = track_id.map(str::to_owned);
        let task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_nanos(snapshot_poll_interval)).await;
                let keeps_polling = this
                    .update(cx, |this, cx| {
                        if !this.listening_observations.contains_key(&observed_key) {
                            return false;
                        }
                        this.request_playlist_listening_snapshot(
                            &observed_playlist_id,
                            observed_track_id.as_deref(),
                            false,
                            cx,
                        );
                        true
                    })
                    .unwrap_or(false);
                if !keeps_polling {
                    return;
                }
            }
        });
        self.listening_observation_tasks.insert(cache_key, task);
        let reflect_pending_in_ui = self.playlist_listening_snapshot(playlist_id, track_id, cx).is_none();
        self.request_playlist_listening_snapshot(playlist_id, track_id, reflect_pending_in_ui, cx);
    }

    pub fn stop_playlist_listening_observation(&mut self, playlist_id: &str, track_id: Option<&str>, _cx: &mut App) {
        let cache_key = Self::playlist_listening_snapshot_cache_key(playlist_id, track_id);
        self.listening_observation_tasks.remove(&cache_key);
        self.listening_observations.remove(&cache_key);
    }

    pub fn refresh_playlist_listening_snapshot(&self, playlist_id: &str, track_id: Option<&str>, cx: &mut App) {
        if playlist_id.is_empty() {
            return;
        }

        self.request_playlist_listening_snapshot(playlist_id, track_id, true, cx);
    }

    pub fn update_playlist_listening_presence(
        &mut self,
        playlist_id: &str,
        track_id: &str,
        position_ms: i64,
        is_playing: bool,
        cx: &mut Context<Self>,
    ) {
        if playlist_id.is_empty() || track_id.is_empty() {
            return;
        }

        self.local_presence_states.insert(
            playlist_id.to_owned(),
            MusicLocalListeningPresenceState {
                playlist_id: playlist_id.to_owned(),
                track_id: track_id.to_owned(),
                anchor_position_ms: position_ms.max(0),
                is_playing,
                local_anchor_time_ms: local_now_ms(),
            },
        );
        self.send_current_playlist_listening_presence(playlist_id, cx);
        if self.presence_heartbeat_tasks.contains_key(playlist_id) {
            return;
        }

        let presence_interval = RiseTimingCanon::heartbeat("music_presence").client_interval_nanoseconds();
        let heartbeat_playlist_id = playlist_id.to_owned();
        let task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_nanos(presence_interval)).await;
                let keeps_beating = this
                    .update(cx, |this, cx| {
                        if !this.local_presence_states.contains_key(&heartbeat_playlist_id) {
                            return false;
                        }
                        this.send_current_playlist_listening_presence(&heartbeat_playlist_id, cx);
                        true
                    })
                    .unwrap_or(false);
                if !keeps_beating {
                    return;
                }
            }
        });
        self.presence_heartbeat_tasks.insert(playlist_id.to_owned(), task);
    }

    pub fn stop_playlist_listening_presence(&mut self, playlist_id: &str, send_stop: bool, cx: &mut App) {
        self.presence_heartbeat_tasks.remove(playlist_id);
        self.local_presence_states.remove(playlist_id);
        if !send_stop {
            return;
        }

        let playlist_id = playlist_id.to_owned();
        let client_instance_id = self.playlist_listening_client_instance_id.clone();
        cx.defer(move |cx| {
            Self::actions(cx).update(cx, |actions, cx| {
                actions.stop_playlist_listening_presence_action(&playlist_id, &client_instance_id, None, None, cx);
            });
        });
    }

    fn presence_position_ms(state: &MusicLocalListeningPresenceState, now_ms: i64, cx: &mut App) -> i64 {
        if !state.is_playing {
            return state.anchor_position_ms;
        }

        let player_entity = MusicPlayerState::shared(cx);
        let player = player_entity.read(cx);
        if player.current_track.as_ref().map(|track| track.id.as_str()) == Some(state.track_id.as_str())
            && player.is_playing
            && !player.is_preparing_playback
        {
            return (player.current_time.max(0.0) * 1_000.0) as i64;
        }

        state.anchor_position_ms + (now_ms - state.local_anchor_time_ms).max(0)
    }

    fn send_current_playlist_listening_presence(&mut self, playlist_id: &str, cx: &mut App) {
        let Some(state) = self.local_presence_states.get(playlist_id).cloned() else { return };

        let now = local_now_ms();
        let client_sequence = self.next_client_sequence(&format!("presence:{playlist_id}"));
        let client_instance_id = self.playlist_listening_client_instance_id.clone();
        let client_command_id = Uuid::new_v4().uuid_string().to_lowercase();
        let playlist_id = playlist_id.to_owned();
        cx.defer(move |cx| {
            let position = Self::presence_position_ms(&state, now, cx);
            let audio_effects = MusicPlayerState::shared(cx).read(cx).audio_effect_settings().wire_effects();
            let success_playlist_id = playlist_id.clone();
            let on_success: Rc<dyn Fn(WirePlaylistPresenceWriteDTO, &mut App)> =
                Rc::new(move |response: WirePlaylistPresenceWriteDTO, cx: &mut App| {
                    if response.server_time_ms <= 0 {
                        return;
                    }
                    let received_at_ms = local_now_ms();
                    MusicServicesStore::shared(cx).update(cx, |services, _| {
                        services.update_server_clock_offset(
                            &success_playlist_id,
                            response.server_time_ms,
                            now,
                            received_at_ms,
                        );
                    });
                });
            let error_playlist_id = playlist_id.clone();
            let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |_: String, cx: &mut App| {
                MusicServicesStore::shared(cx).update(cx, |services, cx| {
                    services.schedule_playlist_listening_snapshot_recovery(&error_playlist_id, cx);
                });
            });
            Self::actions(cx).update(cx, |actions, cx| {
                actions.upsert_playlist_listening_presence_action(
                    &playlist_id,
                    &state.track_id,
                    position,
                    state.is_playing,
                    client_sequence,
                    &client_instance_id,
                    &client_command_id,
                    &audio_effects,
                    Some(on_success),
                    Some(on_error),
                    cx,
                );
            });
        });
    }

    fn next_client_sequence(&mut self, scope: &str) -> i64 {
        let key = format!(
            "{}{}.{}",
            user_defaults_keys::MUSIC_PLAYLIST_LISTENING_SEQUENCE_PREFIX,
            self.playlist_listening_client_instance_id,
            scope
        );
        let defaults = UserDefaults::standard();
        let persisted = defaults.object(&key).and_then(|value| value.as_i64()).unwrap_or(0);
        let wall_clock = local_now_ms() * 1_000;
        let next = (persisted + 1)
            .max(self.last_client_sequence_by_scope.get(scope).copied().unwrap_or(0) + 1)
            .max(wall_clock);
        self.last_client_sequence_by_scope.insert(scope.to_owned(), next);
        defaults.set(&key, next);
        next
    }

    #[allow(clippy::too_many_arguments)]
    pub fn join_playlist_listening_session(
        &self,
        playlist_id: &str,
        target_user_id: &str,
        expected_session_id: Option<&str>,
        on_success: Option<Box<dyn FnOnce(MusicPlaylistListeningSession, &mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut App,
    ) {
        let completion = once_value_handler(on_success);
        let handle_success: Rc<dyn Fn(MusicPlaylistListeningSession, &mut App)> =
            Rc::new(move |session: MusicPlaylistListeningSession, cx: &mut App| {
                MusicServicesStore::shared(cx).update(cx, |services, cx| {
                    let position_ms = services.projected_position_ms_for_session(&session, local_now_ms());
                    services.update_playlist_listening_presence(
                        &session.playlist_id,
                        &session.track_id,
                        position_ms,
                        session.is_playing,
                        cx,
                    );
                    services.start_playlist_listening_session_sync(&session.session_id, cx);
                });
                if let Some(completion) = &completion {
                    completion(session, cx);
                }
            });
        let handle_error = once_value_handler(on_error);
        let playlist_id = playlist_id.to_owned();
        let target_user_id = target_user_id.to_owned();
        let expected_session_id = expected_session_id.map(str::to_owned);
        let client_instance_id = self.playlist_listening_client_instance_id.clone();
        let client_request_id = Uuid::new_v4().uuid_string().to_lowercase();
        cx.defer(move |cx| {
            Self::actions(cx).update(cx, |actions, cx| {
                actions.join_playlist_listening_session_action(
                    &playlist_id,
                    &target_user_id,
                    &client_instance_id,
                    &client_request_id,
                    expected_session_id.as_deref(),
                    Some(handle_success),
                    handle_error,
                    cx,
                );
            });
        });
    }

    fn heartbeat_position_ms(&self, session: &MusicPlaylistListeningSession, cx: &mut App) -> i64 {
        let player_entity = MusicPlayerState::shared(cx);
        let player = player_entity.read(cx);
        let reads_engine = player.listening_session.as_ref().map(|current| current.session_id.as_str())
            == Some(session.session_id.as_str())
            && player.is_listening_session_host()
            && player.current_track.as_ref().map(|track| track.id.as_str()) == Some(session.track_id.as_str())
            && !player.is_suspended_for_external_audio()
            && !player.is_preparing_playback;
        if !reads_engine {
            return self.projected_position_ms_for_session(session, local_now_ms());
        }

        (player.current_time.max(0.0) * 1_000.0) as i64
    }

    pub fn start_playlist_listening_session_sync(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if session_id.is_empty() || self.listening_session_sync_tasks.contains_key(session_id) {
            return;
        }

        let session_interval = RiseTimingCanon::heartbeat("music_listening_session").client_interval_nanoseconds();

        let synced_session_id = session_id.to_owned();
        let task = cx.spawn(async move |this, cx| {
            let mut heartbeat_count: i64 = 0;
            loop {
                cx.background_executor().timer(Duration::from_nanos(session_interval)).await;
                let keeps_syncing = this
                    .update(cx, |this, cx| {
                        let Some(session) = this.playlist_listening_session(&synced_session_id, cx) else {
                            let refresh_session_id = synced_session_id.clone();
                            cx.defer(move |cx| {
                                Self::actions(cx).update(cx, |actions, cx| {
                                    actions.get_playlist_listening_session_action(&refresh_session_id, false, None, None, cx);
                                });
                            });
                            return true;
                        };
                        let position_ms = this.heartbeat_position_ms(&session, cx);
                        this.send_playlist_listening_session_state(
                            &synced_session_id,
                            MusicListeningSessionCommand::Heartbeat,
                            &session.track_id,
                            position_ms,
                            session.is_playing,
                            session.shuffle_enabled,
                            session.shuffle_seed,
                            &session.repeat_mode,
                            session.audio_effect_enabled,
                            session.audio_effect_rate_milli,
                            session.audio_effects.clone(),
                            session.revision,
                            None,
                            None,
                            cx,
                        );
                        heartbeat_count += 1;
                        if heartbeat_count % 2 == 0 {
                            let refresh_session_id = synced_session_id.clone();
                            cx.defer(move |cx| {
                                Self::actions(cx).update(cx, |actions, cx| {
                                    actions.get_playlist_listening_session_action(&refresh_session_id, false, None, None, cx);
                                });
                            });
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keeps_syncing {
                    return;
                }
            }
        });
        self.listening_session_sync_tasks.insert(session_id.to_owned(), task);

        let reflect_pending_in_ui = self.playlist_listening_session(session_id, cx).is_none();
        let refreshed_session_id = session_id.to_owned();
        cx.defer(move |cx| {
            let on_success: Rc<dyn Fn(MusicPlaylistListeningSession, &mut App)> =
                Rc::new(|session: MusicPlaylistListeningSession, cx: &mut App| {
                    MusicServicesStore::shared(cx).update(cx, |services, cx| {
                        let position_ms = services.projected_position_ms_for_session(&session, local_now_ms());
                        services.update_playlist_listening_presence(
                            &session.playlist_id,
                            &session.track_id,
                            position_ms,
                            session.is_playing,
                            cx,
                        );
                    });
                });
            Self::actions(cx).update(cx, |actions, cx| {
                actions.get_playlist_listening_session_action(
                    &refreshed_session_id,
                    reflect_pending_in_ui,
                    Some(on_success),
                    None,
                    cx,
                );
            });
        });
    }

    pub fn stop_playlist_listening_session_sync(&mut self, session_id: &str, _cx: &mut App) {
        self.listening_session_sync_tasks.remove(session_id);
        self.listening_state_command_in_flight.remove(session_id);
        self.pending_listening_state_commands.remove(session_id);
        self.listening_state_command_recovery.remove(session_id);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_playlist_listening_session_state(
        &mut self,
        session_id: &str,
        command: MusicListeningSessionCommand,
        track_id: &str,
        position_ms: i64,
        is_playing: bool,
        shuffle_enabled: bool,
        shuffle_seed: i64,
        repeat_mode: &str,
        audio_effect_enabled: bool,
        audio_effect_rate_milli: i64,
        audio_effects: Vec<MusicListeningAudioEffect>,
        expected_revision: i64,
        on_success: Option<Box<dyn FnOnce(MusicPlaylistListeningSession, &mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut App,
    ) {
        let intent = MusicListeningStateCommandIntent {
            id: Uuid::new_v4(),
            session_id: session_id.to_owned(),
            command,
            state: MusicListeningSessionStateVector {
                track_id: track_id.to_owned(),
                position_ms: position_ms.max(0),
                is_playing,
                shuffle_enabled,
                shuffle_seed,
                repeat_mode: repeat_mode.to_owned(),
                audio_effect_enabled,
                audio_effect_rate_milli,
                audio_effects_canonical: MusicListeningAudioEffectWire::canonical(&audio_effects),
            },
            captured_at_ms: local_now_ms(),
            fallback_revision: expected_revision.max(1),
            retry_attempt: 0,
            on_success: on_success.into_iter().collect(),
            on_error: on_error.into_iter().collect(),
        };
        self.enqueue_listening_state_command(intent, cx);
    }

    fn enqueue_listening_state_command(&mut self, intent: MusicListeningStateCommandIntent, cx: &mut App) {
        let session_id = intent.session_id.clone();
        let has_active_work = self.listening_state_command_in_flight.contains_key(&session_id)
            || self.pending_listening_state_commands.contains_key(&session_id)
            || self.listening_state_command_recovery.contains(&session_id);

        if intent.command == MusicListeningSessionCommand::Heartbeat && has_active_work {
            return;
        }

        if self.listening_state_command_in_flight.contains_key(&session_id)
            || self.listening_state_command_recovery.contains(&session_id)
        {
            let merged = match self.pending_listening_state_commands.remove(&session_id) {
                Some(pending) => pending.merging_newer(intent),
                None => intent,
            };
            self.pending_listening_state_commands.insert(session_id, merged);
            return;
        }

        self.dispatch_listening_state_command(intent, cx);
    }

    fn dispatch_next_listening_state_command(&mut self, session_id: &str, cx: &mut App) {
        if self.listening_state_command_in_flight.contains_key(session_id)
            || self.listening_state_command_recovery.contains(session_id)
        {
            return;
        }
        let Some(next) = self.pending_listening_state_commands.remove(session_id) else { return };
        self.dispatch_listening_state_command(next, cx);
    }

    fn dispatch_listening_state_command(&mut self, intent: MusicListeningStateCommandIntent, cx: &mut App) {
        let session_id = intent.session_id.clone();
        if self.listening_state_command_in_flight.contains_key(&session_id) {
            self.enqueue_listening_state_command(intent, cx);
            return;
        }

        let now_ms = local_now_ms();
        let mut desired = intent.state.clone();
        if desired.is_playing {
            let rate = MusicListeningAudioEffectWire::tempo_rate(&desired.audio_effects_canonical);
            let elapsed_ms = (now_ms - intent.captured_at_ms).max(0) as f64;
            desired.position_ms += (elapsed_ms * rate).round() as i64;
            desired.is_playing = true;
        }

        let current_session = self.playlist_listening_session(&session_id, cx);
        let authoritative = current_session.as_ref().map(|session| MusicListeningSessionStateVector {
            track_id: session.track_id.clone(),
            position_ms: self.projected_position_ms_for_session(session, now_ms),
            is_playing: session.is_playing,
            shuffle_enabled: session.shuffle_enabled,
            shuffle_seed: session.shuffle_seed,
            repeat_mode: session.repeat_mode.clone(),
            audio_effect_enabled: session.audio_effect_enabled,
            audio_effect_rate_milli: session.audio_effect_rate_milli,
            audio_effects_canonical: MusicListeningAudioEffectWire::canonical(&session.audio_effects),
        });
        let command =
            MusicListeningCommandPipelinePolicy::resolved_command(intent.command, authoritative.as_ref(), &desired);
        let expected_revision =
            current_session.as_ref().map_or(intent.fallback_revision, |session| session.revision).max(1);

        let intent_id = intent.id;
        self.listening_state_command_in_flight.insert(session_id.clone(), intent);
        let client_sequence = self.next_client_sequence(&format!("session:{session_id}"));
        let client_instance_id = self.playlist_listening_client_instance_id.clone();
        let client_command_id = Uuid::new_v4().uuid_string().to_lowercase();

        let success_session_id = session_id.clone();
        let on_success: Rc<dyn Fn(MusicPlaylistListeningSession, &mut App)> =
            Rc::new(move |session: MusicPlaylistListeningSession, cx: &mut App| {
                let services = MusicServicesStore::shared(cx);
                let completions = services.update(cx, |services, _| {
                    if !services
                        .listening_state_command_in_flight
                        .get(&success_session_id)
                        .is_some_and(|in_flight| in_flight.id == intent_id)
                    {
                        return None;
                    }
                    services.listening_state_command_in_flight.remove(&success_session_id).map(|intent| intent.on_success)
                });
                let Some(completions) = completions else { return };
                for completion in completions {
                    completion(session.clone(), cx);
                }
                services.update(cx, |services, cx| services.dispatch_next_listening_state_command(&success_session_id, cx));
            });
        let error_session_id = session_id.clone();
        let on_error: Rc<dyn Fn(String, Option<String>, &mut App)> =
            Rc::new(move |error: String, code: Option<String>, cx: &mut App| {
                MusicServicesStore::shared(cx).update(cx, |services, cx| {
                    services.finish_listening_state_command_failure(
                        intent_id,
                        &error_session_id,
                        &error,
                        code.as_deref(),
                        cx,
                    );
                });
            });
        let audio_effects = MusicListeningAudioEffectWire::effects(&desired.audio_effects_canonical);
        cx.defer(move |cx| {
            Self::actions(cx).update(cx, |actions, cx| {
                actions.update_playlist_listening_session_state_action(
                    &session_id,
                    command,
                    &desired.track_id,
                    desired.position_ms,
                    desired.is_playing,
                    desired.shuffle_enabled,
                    desired.shuffle_seed,
                    &desired.repeat_mode,
                    desired.audio_effect_enabled,
                    desired.audio_effect_rate_milli,
                    &audio_effects,
                    expected_revision,
                    client_sequence,
                    &client_instance_id,
                    &client_command_id,
                    Some(on_success),
                    Some(on_error),
                    cx,
                );
            });
        });
    }

    fn finish_listening_state_command_failure(
        &mut self,
        intent_id: Uuid,
        session_id: &str,
        message: &str,
        code: Option<&str>,
        cx: &mut App,
    ) {
        if !self.listening_state_command_in_flight.get(session_id).is_some_and(|in_flight| in_flight.id == intent_id) {
            return;
        }
        let Some(mut intent) = self.listening_state_command_in_flight.remove(session_id) else { return };

        if code == Some(MusicListeningServerErrorCode::MEMBERSHIP_LOST) {
            let completions = std::mem::take(&mut intent.on_error);
            let shown = message.to_owned();
            cx.defer(move |cx| {
                for completion in completions {
                    completion(shown.clone(), cx);
                }
            });
            self.handle_listening_session_ended(session_id, music_service::PLAYLIST_MEMBER_EXIT_REMOVED, cx);
            return;
        }

        if !MusicListeningCommandPipelinePolicy::should_retry_stale_revision(message, code, intent.retry_attempt) {
            let shown = if code == Some(MusicListeningServerErrorCode::FORBIDDEN) {
                tr("music_live_control_locked_title")
            } else {
                message.to_owned()
            };
            let completions = std::mem::take(&mut intent.on_error);
            cx.defer(move |cx| {
                for completion in completions {
                    completion(shown.clone(), cx);
                }
            });
            self.dispatch_next_listening_state_command(session_id, cx);
            return;
        }

        intent.retry_attempt += 1;
        let retry = match self.pending_listening_state_commands.remove(session_id) {
            Some(newer) => intent.merging_newer(newer),
            None => intent,
        };
        self.pending_listening_state_commands.insert(session_id.to_owned(), retry);
        self.recover_listening_state_command_revision(session_id, cx);
    }

    fn recover_listening_state_command_revision(&mut self, session_id: &str, cx: &mut App) {
        if !self.listening_state_command_recovery.insert(session_id.to_owned()) {
            return;
        }

        let success_session_id = session_id.to_owned();
        let on_success: Rc<dyn Fn(MusicPlaylistListeningSession, &mut App)> =
            Rc::new(move |_: MusicPlaylistListeningSession, cx: &mut App| {
                MusicServicesStore::shared(cx).update(cx, |services, cx| {
                    services.listening_state_command_recovery.remove(&success_session_id);
                    services.dispatch_next_listening_state_command(&success_session_id, cx);
                });
            });
        let error_session_id = session_id.to_owned();
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |error: String, cx: &mut App| {
            let completions = MusicServicesStore::shared(cx).update(cx, |services, _| {
                services.listening_state_command_recovery.remove(&error_session_id);
                services.pending_listening_state_commands.remove(&error_session_id).map(|pending| pending.on_error)
            });
            for completion in completions.unwrap_or_default() {
                completion(error.clone(), cx);
            }
        });
        let session_id = session_id.to_owned();
        cx.defer(move |cx| {
            Self::actions(cx).update(cx, |actions, cx| {
                actions.get_playlist_listening_session_action(&session_id, false, Some(on_success), Some(on_error), cx);
            });
        });
    }

    pub fn request_playlist_listening_permissions(
        &self,
        session_id: &str,
        permissions: MusicListeningParticipantPermissions,
        on_success: Option<Box<dyn FnOnce(&mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut App,
    ) {
        let on_success = once_handler(on_success);
        let on_error = once_value_handler(on_error);
        let session_id = session_id.to_owned();
        let client_request_id = Uuid::new_v4().uuid_string().to_lowercase();
        cx.defer(move |cx| {
            Self::actions(cx).update(cx, |actions, cx| {
                actions.request_playlist_listening_permission_action(
                    &session_id,
                    permissions,
                    &client_request_id,
                    on_success,
                    on_error,
                    cx,
                );
            });
        });
    }

    pub fn update_playlist_listening_participant_permissions(
        &self,
        session_id: &str,
        participant_user_id: &str,
        permissions: MusicListeningParticipantPermissions,
        on_success: Option<Box<dyn FnOnce(MusicPlaylistListeningSession, &mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut App,
    ) {
        let on_success = once_value_handler(on_success);
        let on_error = once_value_handler(on_error);
        let session_id = session_id.to_owned();
        let participant_user_id = participant_user_id.to_owned();
        let client_request_id = Uuid::new_v4().uuid_string().to_lowercase();
        cx.defer(move |cx| {
            Self::actions(cx).update(cx, |actions, cx| {
                actions.update_playlist_listening_participant_permissions_action(
                    &session_id,
                    &participant_user_id,
                    permissions,
                    &client_request_id,
                    on_success,
                    on_error,
                    cx,
                );
            });
        });
    }

    pub fn leave_playlist_listening_session(
        &self,
        session_id: &str,
        on_success: Option<Box<dyn FnOnce(&mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut App,
    ) {
        let on_success = once_handler(on_success);
        let on_error = once_value_handler(on_error);
        let session_id = session_id.to_owned();
        let client_request_id = Uuid::new_v4().uuid_string().to_lowercase();
        cx.defer(move |cx| {
            Self::actions(cx).update(cx, |actions, cx| {
                actions.leave_playlist_listening_session_action(
                    &session_id,
                    &client_request_id,
                    on_success,
                    on_error,
                    cx,
                );
            });
        });
    }

    pub fn end_playlist_listening_session(
        &self,
        session_id: &str,
        on_success: Option<Box<dyn FnOnce(&mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut App,
    ) {
        let on_success = once_handler(on_success);
        let on_error = once_value_handler(on_error);
        let session_id = session_id.to_owned();
        let client_request_id = Uuid::new_v4().uuid_string().to_lowercase();
        cx.defer(move |cx| {
            Self::actions(cx).update(cx, |actions, cx| {
                actions.end_playlist_listening_session_action(
                    &session_id,
                    &client_request_id,
                    on_success,
                    on_error,
                    cx,
                );
            });
        });
    }
}
