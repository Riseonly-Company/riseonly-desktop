use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use rise_canon::rise_service_text::music_service;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_errors::ErrorCode;
use rise_i18n::{FormatArg, tr_args};
use rise_music::{SEEK_TOLERANCE_MS, SessionCommand, WIRE_ENTRY_SEPARATOR, WIRE_PAIR_SEPARATOR};

use super::music_wire_types::{RiseStoredMusicPlaylist, RiseStoredMusicTrack};
use crate::core::lib::canon_format::RiseFormat;
use crate::modules::music::shared::utils::music::music_audio_effect_settings::{
    MusicAudioEffectKind, MusicAudioEffectKindExt, MusicAudioEffectSettings,
};
use crate::modules::music::shared::utils::music::music_effect_file_player::MusicEffectPlaybackLimits;
use crate::modules::music::shared::utils::music::music_models::TrackModel;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MusicPlaylistTrackLinkError {
    InvalidLink,
    PlaylistNotFound,
    AccessDenied,
    AccessSuspended,
    TrackNotFound,
    LoadFailed,
}

impl MusicPlaylistTrackLinkError {
    pub fn localization_key(&self) -> &'static str {
        match self {
            Self::InvalidLink => "music_playlist_link_invalid",
            Self::PlaylistNotFound => "music_playlist_link_not_found",
            Self::AccessDenied => "music_playlist_link_access_denied",
            Self::AccessSuspended => "music_playlist_suspended_hint",
            Self::TrackNotFound => "music_playlist_link_track_not_found",
            Self::LoadFailed => "music_playlist_link_load_failed",
        }
    }

    pub fn is_retryable(&self) -> bool {
        *self == Self::LoadFailed
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MusicTrackLinkError {
    InvalidLink,
    NotFound,
    AccessDenied,
    LoadFailed,
}

impl MusicTrackLinkError {
    pub fn localization_key(&self) -> &'static str {
        match self {
            Self::InvalidLink => "music_playlist_link_invalid",
            Self::NotFound => "music_track_link_not_found",
            Self::AccessDenied => "music_track_link_access_denied",
            Self::LoadFailed => "music_track_link_load_failed",
        }
    }

    pub fn is_retryable(&self) -> bool {
        *self == Self::LoadFailed
    }
}

#[derive(Clone, Debug)]
pub struct MusicResolvedPlaylistTrackLink {
    pub playlist: Option<RiseStoredMusicPlaylist>,
    pub track: RiseStoredMusicTrack,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArtistAnalyticsModel {
    pub total_plays: i64,
    pub total_likes: i64,
    pub total_followers: i64,
    pub monthly_listeners: i64,
    pub total_tracks: i64,
    pub total_albums: i64,
    pub retention7d: f64,
    pub retention30d: f64,
}

impl ArtistAnalyticsModel {
    pub fn retention7d_percent_text(&self) -> String {
        Self::format_retention_percent(self.retention7d)
    }

    pub fn retention30d_percent_text(&self) -> String {
        Self::format_retention_percent(self.retention30d)
    }

    fn format_retention_percent(rate: f64) -> String {
        RiseFormat::percent(rate)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModerationQueueMetricsModel {
    pub pending_tracks: i64,
    pub pending_albums: i64,
    pub approved_today: i64,
    pub rejected_today: i64,
    pub avg_review_time_hours: f64,
}

impl ModerationQueueMetricsModel {
    pub fn avg_review_time_hours_text(&self) -> String {
        if self.avg_review_time_hours <= 0.0 {
            return "\u{2014}".to_owned();
        }
        let total_minutes = 1_i64.max((self.avg_review_time_hours * 60.0).round() as i64);
        if total_minutes < 60 {
            return tr_args("duration_minutes_abbr", &[FormatArg::from(total_minutes)]);
        }
        let hours = total_minutes / 60;
        let rest_minutes = total_minutes % 60;
        if rest_minutes == 0 {
            tr_args("duration_hours_abbr", &[FormatArg::from(hours)])
        } else {
            tr_args("duration_hours_minutes_abbr", &[FormatArg::from(hours), FormatArg::from(rest_minutes)])
        }
    }
}

#[derive(Clone, Debug)]
pub struct MusicIndexedTrackSnapshot {
    pub index: i64,
    pub track: RiseStoredMusicTrack,
}

#[derive(Clone, Debug)]
pub struct MusicPlaylistTracksDeleteSnapshot {
    pub playlist_id: String,
    pub tracks: Vec<MusicIndexedTrackSnapshot>,
}

#[derive(Clone, Debug)]
pub struct MusicLocalTracksDeleteSnapshot {
    pub local_tracks: Vec<MusicIndexedTrackSnapshot>,
    pub playlists: Vec<MusicPlaylistTracksDeleteSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MusicUserReference {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub logo: String,
    pub is_premium: bool,
    pub premium_badge_emoji: Option<String>,
    pub name_color: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MusicPlaylistSummary {
    pub id: String,
    pub owner_user_id: String,
    pub title: String,
    pub description: String,
    pub cover_url: String,
    pub visibility: String,
    pub tracks: Vec<TrackModel>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub revision: i64,
    pub new_tracks_on_top: bool,
}

impl PartialEq for MusicPlaylistSummary {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.revision == other.revision
    }
}

impl Eq for MusicPlaylistSummary {}

impl Hash for MusicPlaylistSummary {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.revision.hash(state);
    }
}

#[derive(Clone, Debug)]
pub struct MusicLocalPlaylistDestination {
    pub id: String,
    pub title: String,
    pub description: String,
    pub cover_url: String,
    pub owner_user_id: String,
    pub viewer_role: String,
    pub can_add_tracks: bool,
    pub updated_at_ms: i64,
    pub revision: i64,
}

impl PartialEq for MusicLocalPlaylistDestination {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.revision == other.revision
    }
}

impl Eq for MusicLocalPlaylistDestination {}

impl Hash for MusicLocalPlaylistDestination {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.revision.hash(state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MusicPlaylistMemberPermissions {
    pub can_edit_playlist: bool,
    pub can_add_tracks: bool,
    pub can_remove_tracks: bool,
}

impl MusicPlaylistMemberPermissions {
    pub const NONE: Self = Self { can_edit_playlist: false, can_add_tracks: false, can_remove_tracks: false };

    pub const OWNER: Self = Self { can_edit_playlist: true, can_add_tracks: true, can_remove_tracks: true };
}

#[derive(Clone, Debug, PartialEq)]
pub struct MusicPlaylistSettings {
    pub playlist: MusicPlaylistSummary,
    pub viewer_role: String,
    pub viewer_permissions: MusicPlaylistMemberPermissions,
    pub total_participant_count: i64,
    pub active_participant_count: i64,
    pub suspended_member_count: i64,
    pub pending_request_count: i64,
    pub effective_participant_limit: i64,
    pub owner_is_premium: bool,
    pub can_invite: bool,
    pub can_review_requests: bool,
    pub access_state: String,
    pub access_epoch: i64,
}

impl MusicPlaylistSettings {
    pub fn is_owner(&self) -> bool {
        self.viewer_role == music_service::PLAYLIST_ROLE_OWNER
    }

    pub fn has_active_access(&self) -> bool {
        self.access_state == music_service::PLAYLIST_ACCESS_STATE_ACTIVE
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MusicPlaylistInviteLink {
    pub playlist_id: String,
    pub url: String,
    pub token: String,
    pub version: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicPlaylistInvitePreview {
    pub playlist_id: String,
    pub owner_user_id: String,
    pub title: String,
    pub description: String,
    pub cover_url: String,
    pub owner: MusicUserReference,
    pub active_participant_count: i64,
    pub effective_participant_limit: i64,
    pub owner_is_premium: bool,
    pub viewer_state: String,
}

impl MusicPlaylistInvitePreview {
    pub fn id(&self) -> &str {
        &self.playlist_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MusicPlaylistInviteSheetPhase {
    Loading,
    Preview(MusicPlaylistInvitePreview),
    Failed { message: String, retryable: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicPlaylistInviteSheetState {
    pub token: String,
    pub preview_request_id: Uuid,
    pub phase: MusicPlaylistInviteSheetPhase,
    pub submission_id: Option<Uuid>,
    pub submission_error: Option<String>,
}

impl MusicPlaylistInviteSheetState {
    pub fn new(token: String) -> Self {
        Self {
            token,
            preview_request_id: Uuid::new_v4(),
            phase: MusicPlaylistInviteSheetPhase::Loading,
            submission_id: None,
            submission_error: None,
        }
    }

    pub fn id(&self) -> &str {
        &self.token
    }

    pub fn is_loading(&self) -> bool {
        self.phase == MusicPlaylistInviteSheetPhase::Loading
    }

    pub fn is_submitting(&self) -> bool {
        self.submission_id.is_some()
    }

    pub fn preview(&self) -> Option<&MusicPlaylistInvitePreview> {
        match &self.phase {
            MusicPlaylistInviteSheetPhase::Preview(preview) => Some(preview),
            MusicPlaylistInviteSheetPhase::Loading | MusicPlaylistInviteSheetPhase::Failed { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicPlaylistJoinRequest {
    pub id: String,
    pub playlist_id: String,
    pub user_id: String,
    pub user: MusicUserReference,
    pub status: String,
    pub requested_at_ms: i64,
    pub processed_at_ms: Option<i64>,
    pub request_version: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicPlaylistMember {
    pub playlist_id: String,
    pub user_id: String,
    pub user: MusicUserReference,
    pub role: String,
    pub status: String,
    pub permissions: MusicPlaylistMemberPermissions,
    pub accepted_at_ms: i64,
    pub version: i64,
}

impl MusicPlaylistMember {
    pub fn id(&self) -> &str {
        &self.user_id
    }

    pub fn is_owner(&self) -> bool {
        self.role == music_service::PLAYLIST_ROLE_OWNER
    }

    pub fn has_active_access(&self) -> bool {
        self.status == music_service::PLAYLIST_MEMBER_STATUS_ACTIVE
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicPlaylistListener {
    pub playlist_id: String,
    pub user_id: String,
    pub user: MusicUserReference,
    pub track_id: String,
    pub anchor_position_ms: i64,
    pub is_playing: bool,
    pub anchor_server_time_ms: i64,
    pub client_sequence: i64,
    pub session_id: Option<String>,
    pub audio_effects: Vec<MusicListeningAudioEffect>,
}

impl MusicPlaylistListener {
    pub fn id(&self) -> &str {
        &self.user_id
    }

    pub fn playback_rate_multiplier(&self) -> f64 {
        MusicListeningAudioEffectWire::tempo_rate(&MusicListeningAudioEffectWire::canonical(&self.audio_effects))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MusicListeningParticipantPermissions {
    pub can_control_playback: bool,
    pub can_seek: bool,
    pub can_change_tracks: bool,
    pub can_apply_effects: bool,
}

impl MusicListeningParticipantPermissions {
    pub const NONE: Self =
        Self { can_control_playback: false, can_seek: false, can_change_tracks: false, can_apply_effects: false };

    pub const HOST: Self =
        Self { can_control_playback: true, can_seek: true, can_change_tracks: true, can_apply_effects: true };

    pub fn new(can_control_playback: bool, can_seek: bool, can_change_tracks: bool) -> Self {
        Self::new_with_apply_effects(can_control_playback, can_seek, can_change_tracks, false)
    }

    pub fn new_with_apply_effects(
        can_control_playback: bool,
        can_seek: bool,
        can_change_tracks: bool,
        can_apply_effects: bool,
    ) -> Self {
        Self { can_control_playback, can_seek, can_change_tracks, can_apply_effects }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MusicListeningAudioEffect {
    pub kind: String,
    pub value_milli: i64,
}

pub enum MusicListeningAudioEffectWire {}

impl MusicListeningAudioEffectWire {
    pub fn canonical(effects: &[MusicListeningAudioEffect]) -> String {
        let mut seen: HashSet<&'static str> = HashSet::new();
        let mut pairs: Vec<(&'static str, i64)> = effects
            .iter()
            .filter_map(|effect| {
                let kind = MusicAudioEffectKind::from_raw_value(&effect.kind)?;
                if !seen.insert(kind.raw_value()) {
                    return None;
                }
                Some((kind.raw_value(), kind.quantize_milli(effect.value_milli)))
            })
            .collect();
        pairs.sort_by_key(|pair| pair.0);
        pairs
            .into_iter()
            .map(|(kind, value)| format!("{}{}{}", kind, WIRE_PAIR_SEPARATOR, value))
            .collect::<Vec<_>>()
            .join(WIRE_ENTRY_SEPARATOR)
    }

    pub fn effects(canonical: &str) -> Vec<MusicListeningAudioEffect> {
        canonical
            .split(WIRE_ENTRY_SEPARATOR)
            .filter(|entry| !entry.is_empty())
            .filter_map(|entry| {
                let parts: Vec<&str> = entry.split(WIRE_PAIR_SEPARATOR).filter(|part| !part.is_empty()).collect();
                if parts.len() != 2 || MusicAudioEffectKind::from_raw_value(parts[0]).is_none() {
                    return None;
                }
                let value_milli = parts[1].parse::<i64>().ok()?;
                Some(MusicListeningAudioEffect { kind: parts[0].to_owned(), value_milli })
            })
            .collect()
    }

    pub fn tempo_rate(canonical: &str) -> f64 {
        let effects = Self::effects(canonical);
        let tempo = effects
            .iter()
            .find(|effect| effect.kind == MusicAudioEffectKind::Nightcore.raw_value())
            .or_else(|| effects.iter().find(|effect| effect.kind == MusicAudioEffectKind::Slowed.raw_value()));
        let Some(tempo) = tempo else {
            return 1.0;
        };
        tempo.value_milli as f64 / 1_000.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicListeningParticipant {
    pub user_id: String,
    pub user: MusicUserReference,
    pub permissions: MusicListeningParticipantPermissions,
    pub joined_at_ms: i64,
}

impl MusicListeningParticipant {
    pub fn id(&self) -> &str {
        &self.user_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicPlaylistListeningSession {
    pub session_id: String,
    pub playlist_id: String,
    pub host_user_id: String,
    pub host_user: MusicUserReference,
    pub track_id: String,
    pub anchor_position_ms: i64,
    pub is_playing: bool,
    pub anchor_server_time_ms: i64,
    pub revision: i64,
    pub generation: i64,
    pub shuffle_enabled: bool,
    pub shuffle_seed: i64,
    pub repeat_mode: String,
    pub participants: Vec<MusicListeningParticipant>,
    pub viewer_permissions: MusicListeningParticipantPermissions,
    pub lease_until_ms: i64,
    pub audio_effect_enabled: bool,
    pub audio_effect_rate_milli: i64,
    pub audio_effects: Vec<MusicListeningAudioEffect>,
}

impl MusicPlaylistListeningSession {
    pub fn id(&self) -> &str {
        &self.session_id
    }

    pub fn audio_effect_rate(&self) -> f64 {
        MusicEffectPlaybackLimits::rate(self.audio_effect_rate_milli)
    }

    pub fn audio_effect_settings(&self) -> MusicAudioEffectSettings {
        MusicAudioEffectSettings::from_wire_effects_only(&self.audio_effects)
    }

    pub fn playback_rate_multiplier(&self) -> f64 {
        self.audio_effect_settings().playback_rate()
    }

    pub fn is_host(&self, user_id: &str) -> bool {
        self.host_user_id == user_id
    }

    pub fn contains(&self, user_id: &str) -> bool {
        self.host_user_id == user_id || self.participants.iter().any(|participant| participant.user_id == user_id)
    }

    pub fn removing_participant(&self, user_id: &str) -> Self {
        Self {
            participants: self
                .participants
                .iter()
                .filter(|participant| participant.user_id != user_id)
                .cloned()
                .collect(),
            ..self.clone()
        }
    }

    pub fn updating_participant_permissions(
        &self,
        user_id: &str,
        permissions: MusicListeningParticipantPermissions,
        viewer_user_id: &str,
    ) -> Self {
        Self {
            participants: self
                .participants
                .iter()
                .map(|participant| {
                    if participant.user_id != user_id {
                        return participant.clone();
                    }
                    MusicListeningParticipant {
                        user_id: participant.user_id.clone(),
                        user: participant.user.clone(),
                        permissions,
                        joined_at_ms: participant.joined_at_ms,
                    }
                })
                .collect(),
            viewer_permissions: if user_id == viewer_user_id { permissions } else { self.viewer_permissions },
            ..self.clone()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicListeningSessionTransition {
    pub playlist_id: String,
    pub target_user_id: String,
    pub target_session_id: Option<String>,
    pub current_session_id: Option<String>,
    pub participant_count: i64,
}

impl MusicListeningSessionTransition {
    pub fn id(&self) -> String {
        [
            self.playlist_id.as_str(),
            self.target_user_id.as_str(),
            self.target_session_id.as_deref().unwrap_or_default(),
            self.current_session_id.as_deref().unwrap_or_default(),
        ]
        .join(":")
    }

    pub fn targets(&self, other: &MusicListeningSessionTransition) -> bool {
        self.playlist_id == other.playlist_id
            && self.target_user_id == other.target_user_id
            && (self.target_session_id.is_none()
                || other.target_session_id.is_none()
                || self.target_session_id == other.target_session_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicListeningSessionTransitionDecision {
    AlreadyJoined,
    Join,
    EndThenJoin,
    ConfirmEndThenJoin,
    LeaveThenJoin,
}

pub enum MusicListeningSessionTransitionPolicy {}

impl MusicListeningSessionTransitionPolicy {
    pub fn decision(
        current_session: Option<&MusicPlaylistListeningSession>,
        current_user_id: &str,
        target_user_id: &str,
        target_session_id: Option<&str>,
    ) -> MusicListeningSessionTransitionDecision {
        let Some(current_session) = current_session else {
            return MusicListeningSessionTransitionDecision::Join;
        };

        if let Some(target_session_id) = target_session_id {
            if target_session_id == current_session.session_id {
                return MusicListeningSessionTransitionDecision::AlreadyJoined;
            }
        } else if current_session.contains(target_user_id) {
            return MusicListeningSessionTransitionDecision::AlreadyJoined;
        }
        if current_session.host_user_id == current_user_id {
            return if current_session.participants.is_empty() {
                MusicListeningSessionTransitionDecision::EndThenJoin
            } else {
                MusicListeningSessionTransitionDecision::ConfirmEndThenJoin
            };
        }
        MusicListeningSessionTransitionDecision::LeaveThenJoin
    }

    pub fn can_continue_after_close_error(message: &str, code: Option<&str>) -> bool {
        if let Some(code) = code.filter(|code| !code.is_empty()) {
            return code == MusicListeningServerErrorCode::NOT_FOUND
                || code == MusicListeningServerErrorCode::MEMBERSHIP_LOST;
        }

        let normalized = message.to_lowercase();
        normalized.contains("not found")
            || normalized.contains("already ended")
            || normalized.contains("session ended")
            || normalized.contains("listening session not found")
    }
}

pub enum MusicListeningServerErrorCode {}

impl MusicListeningServerErrorCode {
    pub const STALE_REVISION: &'static str = ErrorCode::MusicListeningStaleRevision.wire();
    pub const STALE_SEQUENCE: &'static str = ErrorCode::MusicListeningStaleCommand.wire();
    pub const MEMBERSHIP_LOST: &'static str = ErrorCode::MusicListeningMembershipLost.wire();
    pub const NOT_FOUND: &'static str = ErrorCode::MusicListeningSessionNotFound.wire();
    pub const FORBIDDEN: &'static str = ErrorCode::MusicListeningControlNotGranted.wire();
    pub const PRESENCE_REQUIRED: &'static str = ErrorCode::MusicListeningPresenceRequired.wire();
    pub const TOO_MANY_CONTROLLERS: &'static str = ErrorCode::MusicListeningTooManyControllers.wire();
    pub const REALTIME_UNAVAILABLE: &'static str = ErrorCode::MusicListeningRealtimeUnavailable.wire();
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicPlaylistListeningSnapshot {
    pub listeners: Vec<MusicPlaylistListener>,
    pub total_listener_count: i64,
    pub presence_sequence: i64,
    pub server_time_ms: i64,
    pub active_session: Option<MusicPlaylistListeningSession>,
    pub watcher_lease_until_ms: i64,
    pub unchanged: bool,
}

pub type MusicListeningSessionCommand = SessionCommand;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicListeningSessionStateVector {
    pub track_id: String,
    pub position_ms: i64,
    pub is_playing: bool,
    pub shuffle_enabled: bool,
    pub shuffle_seed: i64,
    pub repeat_mode: String,
    pub audio_effect_enabled: bool,
    pub audio_effect_rate_milli: i64,
    pub audio_effects_canonical: String,
}

pub enum MusicListeningCommandPipelinePolicy {}

impl MusicListeningCommandPipelinePolicy {
    pub fn dominant(
        left: MusicListeningSessionCommand,
        right: MusicListeningSessionCommand,
    ) -> MusicListeningSessionCommand {
        if right == SessionCommand::Heartbeat { left } else { right }
    }

    pub fn resolved_command(
        requested: MusicListeningSessionCommand,
        authoritative: Option<&MusicListeningSessionStateVector>,
        desired: &MusicListeningSessionStateVector,
    ) -> MusicListeningSessionCommand {
        let Some(authoritative) = authoritative else {
            return requested;
        };
        if authoritative.track_id != desired.track_id
            || authoritative.shuffle_enabled != desired.shuffle_enabled
            || authoritative.shuffle_seed != desired.shuffle_seed
            || authoritative.repeat_mode != desired.repeat_mode
        {
            return SessionCommand::ChangeTrack;
        }
        if authoritative.is_playing != desired.is_playing {
            return SessionCommand::Playback;
        }
        if authoritative.audio_effects_canonical != desired.audio_effects_canonical {
            return SessionCommand::Effects;
        }
        if (authoritative.position_ms - desired.position_ms).abs() > SEEK_TOLERANCE_MS {
            return SessionCommand::Seek;
        }
        requested
    }

    pub fn should_retry_stale_revision(message: &str, code: Option<&str>, attempt: i64) -> bool {
        if attempt >= 2 {
            return false;
        }
        if let Some(code) = code.filter(|code| !code.is_empty()) {
            return code == MusicListeningServerErrorCode::STALE_REVISION;
        }

        let normalized = message.to_lowercase();
        normalized.contains("stale revision")
            || normalized.contains("session changed")
            || normalized.contains("refresh its snapshot")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MusicListeningSessionUpdate {
    Session {
        session: MusicPlaylistListeningSession,
        server_time_ms: i64,
        actor_user_id: Option<String>,
    },
    Ended {
        session_id: String,
        reason: String,
    },
    AccessRevoked {
        playlist_id: String,
        reason: String,
    },
    AccessRestored,
    PermissionRequested {
        session_id: String,
        participant_user_id: String,
        permissions: MusicListeningParticipantPermissions,
    },
    ParticipantLeft {
        session_id: String,
        participant_user_id: String,
        participant_user: Option<MusicUserReference>,
    },
    PermissionsUpdated {
        session_id: String,
        participant_user_id: String,
        permissions: MusicListeningParticipantPermissions,
    },
}
