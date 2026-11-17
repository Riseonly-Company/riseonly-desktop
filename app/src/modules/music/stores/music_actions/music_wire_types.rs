use rise_canon::rise_key::message as message_key;
use rise_canon::rise_key::music as key;
use rise_canon::rise_service_text::music_service;
use rise_canon::{RiseWireKey, rise_decode, rise_proto, rise_reply, rise_response};
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::{trimming_whitespaces, trimming_whitespaces_and_newlines};
use rise_i18n::{tr, tr_plural};
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use super::music_types::{
    ArtistAnalyticsModel, ModerationQueueMetricsModel, MusicListeningAudioEffect, MusicListeningParticipant,
    MusicListeningParticipantPermissions, MusicLocalPlaylistDestination, MusicPlaylistInviteLink,
    MusicPlaylistInvitePreview, MusicPlaylistJoinRequest, MusicPlaylistListener, MusicPlaylistListeningSession,
    MusicPlaylistListeningSnapshot, MusicPlaylistMember, MusicPlaylistMemberPermissions, MusicPlaylistSettings,
    MusicPlaylistSummary, MusicUserReference,
};
use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::music::shared::utils::music::music_audio_effect_settings::{
    MusicAudioEffectKind, MusicAudioEffectKindExt,
};
use crate::modules::music::shared::utils::music::music_effect_file_player::MusicEffectPlaybackLimits;
use crate::modules::music::shared::utils::music::music_media_url::MusicMediaURL;
use crate::modules::music::shared::utils::music::music_models::{
    AlbumModel, ArtistModel, MusicDefaultLocalPlaylist, MusicLocalTrackTransferPhase, PlaylistModel, TrackModel,
};

pub type WireUserRefDTO = rise_proto::music::ProtoUserRef;
pub type WireAlbumRefDTO = rise_proto::music::ProtoAlbumRef;
pub type WireArtistDTO = rise_proto::music::ProtoArtistRecord;
pub type WireAlbumDTO = rise_proto::music::ProtoAlbumRecord;
pub type WireAlbumRequestDTO = rise_proto::music::ProtoAlbumRequestRecord;
pub type WireSingleAlbumRequestDTO = rise_response::music::GetAlbumRequest;
pub type WirePublishTrackDTO = rise_response::music::PublishTrack;
pub type WirePublishAlbumDTO = rise_response::music::PublishAlbum;
pub type WireRequestAlbumDTO = rise_response::music::RequestAlbum;
pub type WireAlbumsListDTO = rise_response::music::ListMyAlbums;
pub type WireSingleTrackDTO = rise_response::music::GetTrack;
pub type WireSingleAlbumDTO = rise_response::music::GetAlbum;
pub type WirePlaylistMemberPermissionsDTO = rise_proto::music::ProtoPlaylistMemberPermissions;
pub type WirePlaylistSettingsDTO = rise_proto::music::ProtoPlaylistSettingsRecord;
pub type WirePlaylistSettingsResponseDTO = rise_response::music::GetPlaylistSettings;
pub type WirePlaylistInviteLinkDTO = rise_proto::music::ProtoPlaylistInviteLink;
pub type WirePlaylistInviteLinkResponseDTO = rise_response::music::GetPlaylistInviteLink;
pub type WirePlaylistInvitePreviewDTO = rise_proto::music::ProtoPlaylistInvitePreview;
pub type WirePlaylistInvitePreviewResponseDTO = rise_response::music::GetPlaylistInvitePreview;
pub type WirePlaylistMemberDTO = rise_proto::music::ProtoPlaylistMemberRecord;
pub type WirePlaylistMemberResponseDTO = rise_response::music::UpdatePlaylistMemberPermissions;
pub type WireListeningParticipantPermissionsDTO = rise_proto::music::ProtoListeningParticipantPermissions;
pub type WireListeningAudioEffectDTO = rise_proto::music::ProtoListeningAudioEffect;
pub type WireListeningParticipantDTO = rise_proto::music::ProtoListeningParticipantRecord;
pub type WirePlaylistListeningSessionDTO = rise_proto::music::ProtoPlaylistListeningSessionRecord;
pub type WireArtistAnalyticsDTO = rise_response::music::GetArtistAnalytics;
pub type WireArtistRequestDTO = rise_proto::music::ProtoArtistRequestRecord;
pub type WireSingleArtistRequestDTO = rise_response::music::GetArtistRequest;

/// a track as the app keeps it
/// canon record plus the transfer state of a track still importing from a chat message
/// server fields are read through the proto record
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RiseStoredMusicTrack {
    pub id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub duration_ms: Option<i64>,
    pub cover_url: Option<String>,
    pub audio_url: Option<String>,
    pub play_count: Option<i64>,
    pub like_count: Option<i64>,
    pub is_liked: Option<bool>,
    pub album: Option<WireAlbumRefDTO>,
    pub artist: Option<WireArtistDTO>,
    pub created_at_ms: Option<i64>,
    pub status: Option<String>,
    pub genre: Option<String>,
    pub rejection_reason: Option<String>,
    pub scheduled_at_ms: Option<i64>,
    pub is_exclusive: Option<bool>,
    pub library_owner_user_id: Option<String>,
    pub source_chat_id: Option<String>,
    pub source_message_id: Option<String>,
    pub remote_audio_url: Option<String>,
    pub transfer_phase: Option<String>,
    pub transferred_bytes: Option<i64>,
    pub transfer_total_bytes: Option<i64>,
    pub transfer_error: Option<String>,
}

impl RiseStoredMusicTrack {
    pub fn from_canon(track: &rise_proto::music::ProtoTrackRecord) -> Self {
        Self {
            id: Some(track.id.clone()),
            title: Some(track.title.clone()),
            description: track.description.clone(),
            duration_ms: Some(track.duration_ms),
            cover_url: Some(track.cover_url.clone()),
            audio_url: Some(track.audio_url.clone()),
            play_count: Some(track.play_count),
            like_count: Some(track.like_count),
            is_liked: Some(track.is_liked),
            album: track.album.clone(),
            artist: track.artist.clone(),
            created_at_ms: Some(track.created_at_ms),
            status: Some(track.status.clone()),
            genre: Some(track.genre.clone()),
            rejection_reason: track.rejection_reason.clone(),
            scheduled_at_ms: track.scheduled_at_ms,
            is_exclusive: Some(track.is_exclusive),
            library_owner_user_id: track.library_owner_user_id.clone(),
            source_chat_id: None,
            source_message_id: None,
            remote_audio_url: None,
            transfer_phase: None,
            transferred_bytes: None,
            transfer_total_bytes: None,
            transfer_error: None,
        }
    }

    /// canon record of this row so a cached row is written in the shape the server sends
    pub fn canon(&self) -> rise_proto::music::ProtoTrackRecord {
        rise_proto::music::ProtoTrackRecord {
            album: self.album.clone(),
            artist: self.artist.clone(),
            audio_url: self.audio_url.clone().unwrap_or_default(),
            cover_url: self.cover_url.clone().unwrap_or_default(),
            created_at_ms: self.created_at_ms.unwrap_or(0),
            description: self.description.clone(),
            duration_ms: self.duration_ms.unwrap_or(0),
            genre: self.genre.clone().unwrap_or_default(),
            id: self.id.clone().unwrap_or_default(),
            is_exclusive: self.is_exclusive.unwrap_or(false),
            is_liked: self.is_liked.unwrap_or(false),
            library_owner_user_id: self.library_owner_user_id.clone(),
            like_count: self.like_count.unwrap_or(0),
            play_count: self.play_count.unwrap_or(0),
            rejection_reason: self.rejection_reason.clone(),
            scheduled_at_ms: self.scheduled_at_ms,
            status: self.status.clone().unwrap_or_default(),
            title: self.title.clone().unwrap_or_default(),
        }
    }
}

impl<'de> Deserialize<'de> for RiseStoredMusicTrack {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |local| {
            let mut track = Self::from_canon(&local.decode::<rise_proto::music::ProtoTrackRecord>()?);
            track.source_chat_id = local.rise::<String>(message_key::SOURCE_CHAT_ID)?;
            track.source_message_id = local.rise::<String>(message_key::SOURCE_MESSAGE_ID)?;
            track.remote_audio_url = local.rise::<String>(RiseLocalKey::REMOTE_AUDIO_URL)?;
            track.transfer_phase = local.rise::<String>(RiseLocalKey::TRANSFER_PHASE)?;
            track.transferred_bytes = local.rise::<i64>(RiseLocalKey::TRANSFERRED_BYTES)?;
            track.transfer_total_bytes = local.rise::<i64>(RiseLocalKey::TRANSFER_TOTAL_BYTES)?;
            track.transfer_error = local.rise::<String>(RiseLocalKey::TRANSFER_ERROR)?;
            Ok(track)
        })
    }
}

impl Serialize for RiseStoredMusicTrack {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut local = canon_object(&self.canon()).map_err(S::Error::custom)?;
        encode_if_present(&mut local, camel(message_key::SOURCE_CHAT_ID), &self.source_chat_id)
            .map_err(S::Error::custom)?;
        encode_if_present(&mut local, camel(message_key::SOURCE_MESSAGE_ID), &self.source_message_id)
            .map_err(S::Error::custom)?;
        encode_if_present(&mut local, camel(RiseLocalKey::REMOTE_AUDIO_URL), &self.remote_audio_url)
            .map_err(S::Error::custom)?;
        encode_if_present(&mut local, camel(RiseLocalKey::TRANSFER_PHASE), &self.transfer_phase)
            .map_err(S::Error::custom)?;
        encode_if_present(&mut local, camel(RiseLocalKey::TRANSFERRED_BYTES), &self.transferred_bytes)
            .map_err(S::Error::custom)?;
        encode_if_present(&mut local, camel(RiseLocalKey::TRANSFER_TOTAL_BYTES), &self.transfer_total_bytes)
            .map_err(S::Error::custom)?;
        encode_if_present(&mut local, camel(RiseLocalKey::TRANSFER_ERROR), &self.transfer_error)
            .map_err(S::Error::custom)?;
        local.serialize(serializer)
    }
}

/// a playlist as the app keeps it
/// rows may include tracks still importing on this device
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RiseStoredMusicPlaylist {
    pub id: Option<String>,
    pub owner_user_id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub visibility: Option<String>,
    pub tracks: Option<Vec<RiseStoredMusicTrack>>,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub revision: Option<i64>,
    pub is_default_local: Option<bool>,
    pub viewer_role: Option<String>,
    pub can_add_tracks: Option<bool>,
    pub active_member_count: Option<i64>,
    pub new_tracks_on_top: Option<bool>,
}

impl RiseStoredMusicPlaylist {
    pub fn from_canon(playlist: &rise_proto::music::ProtoPlaylistRecord) -> Self {
        Self {
            id: Some(playlist.id.clone()),
            owner_user_id: Some(playlist.owner_user_id.clone()),
            title: Some(playlist.title.clone()),
            description: Some(playlist.description.clone()),
            cover_url: Some(playlist.cover_url.clone()),
            visibility: Some(playlist.visibility.clone()),
            tracks: Some(playlist.tracks.iter().map(RiseStoredMusicTrack::from_canon).collect()),
            created_at_ms: Some(playlist.created_at_ms),
            updated_at_ms: Some(playlist.updated_at_ms),
            revision: Some(playlist.revision),
            is_default_local: Some(playlist.is_default_local),
            viewer_role: playlist.viewer_role.clone(),
            can_add_tracks: playlist.can_add_tracks,
            active_member_count: playlist.active_member_count,
            new_tracks_on_top: Some(playlist.new_tracks_on_top),
        }
    }

    pub fn is_solo_playlist(&self) -> bool {
        self.active_member_count == Some(0)
    }

    pub fn canon(&self) -> rise_proto::music::ProtoPlaylistRecord {
        rise_proto::music::ProtoPlaylistRecord {
            active_member_count: self.active_member_count,
            can_add_tracks: self.can_add_tracks,
            cover_url: self.cover_url.clone().unwrap_or_default(),
            created_at_ms: self.created_at_ms.unwrap_or(0),
            description: self.description.clone().unwrap_or_default(),
            id: self.id.clone().unwrap_or_default(),
            is_default_local: self.is_default_local.unwrap_or(false),
            new_tracks_on_top: self.new_tracks_on_top.unwrap_or(false),
            owner_user_id: self.owner_user_id.clone().unwrap_or_default(),
            revision: self.revision.unwrap_or(0),
            title: self.title.clone().unwrap_or_default(),
            tracks: self.tracks.iter().flatten().map(RiseStoredMusicTrack::canon).collect(),
            updated_at_ms: self.updated_at_ms.unwrap_or(0),
            viewer_role: self.viewer_role.clone(),
            visibility: self.visibility.clone().unwrap_or_default(),
        }
    }
}

impl<'de> Deserialize<'de> for RiseStoredMusicPlaylist {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |rows| {
            let mut playlist = Self::from_canon(&rows.decode::<rise_proto::music::ProtoPlaylistRecord>()?);
            playlist.tracks = Some(rows.rise::<Vec<RiseStoredMusicTrack>>(key::TRACKS)?.unwrap_or_default());
            Ok(playlist)
        })
    }
}

impl Serialize for RiseStoredMusicPlaylist {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut rows = canon_object(&self.canon()).map_err(S::Error::custom)?;
        let tracks = self.tracks.clone().unwrap_or_default();
        rows.insert(key::TRACKS.to_owned(), serde_json::to_value(&tracks).map_err(S::Error::custom)?);
        rows.serialize(serializer)
    }
}

/// get_playlist as the playlist screen keeps it, importing rows merged in
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RiseStoredMusicPlaylistDetail {
    pub playlist: Option<RiseStoredMusicPlaylist>,
}

impl<'de> Deserialize<'de> for RiseStoredMusicPlaylistDetail {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self { playlist: container.rise::<RiseStoredMusicPlaylist>(key::PLAYLIST)? })
        })
    }
}

impl Serialize for RiseStoredMusicPlaylistDetail {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = Map::new();
        encode_if_present(&mut container, key::PLAYLIST.to_owned(), &self.playlist).map_err(S::Error::custom)?;
        container.serialize(serializer)
    }
}

impl RiseStoredMusicPlaylistDetail {
    pub fn from_canon(reply: &rise_response::music::GetPlaylist) -> Self {
        Self { playlist: reply.playlist.as_ref().map(RiseStoredMusicPlaylist::from_canon) }
    }
}

/// get_playlists_mine as the library keeps it, importing rows merged in
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RiseStoredMusicPlaylistLibrary {
    pub playlists: Option<Vec<RiseStoredMusicPlaylist>>,
}

impl<'de> Deserialize<'de> for RiseStoredMusicPlaylistLibrary {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self { playlists: container.rise::<Vec<RiseStoredMusicPlaylist>>(key::PLAYLISTS)? })
        })
    }
}

impl Serialize for RiseStoredMusicPlaylistLibrary {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = Map::new();
        encode_if_present(&mut container, key::PLAYLISTS.to_owned(), &self.playlists).map_err(S::Error::custom)?;
        container.serialize(serializer)
    }
}

/// create_track or request_album answer plus the progress of the files they wait for
/// only the answer comes from the server
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RiseStoredMusicUpload {
    pub title: Option<String>,
    pub cover_upload_progress: Option<f64>,
    pub cover_uploading: Option<bool>,
    pub audio_upload_progress: Option<f64>,
    pub audio_uploading: Option<bool>,
    pub track: Option<RiseStoredMusicTrack>,
    pub request: Option<WireAlbumRequestDTO>,
}

impl<'de> Deserialize<'de> for RiseStoredMusicUpload {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                title: container.rise::<String>(key::TITLE)?,
                cover_upload_progress: container.rise::<f64>(RiseLocalKey::COVER_UPLOAD_PROGRESS)?,
                cover_uploading: container.rise::<bool>(RiseLocalKey::COVER_UPLOADING)?,
                audio_upload_progress: container.rise::<f64>(RiseLocalKey::AUDIO_UPLOAD_PROGRESS)?,
                audio_uploading: container.rise::<bool>(RiseLocalKey::AUDIO_UPLOADING)?,
                track: container.rise::<RiseStoredMusicTrack>(key::TRACK)?,
                request: container.rise::<WireAlbumRequestDTO>(key::REQUEST)?,
            })
        })
    }
}

impl Serialize for RiseStoredMusicUpload {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = Map::new();
        encode_if_present(&mut container, key::TITLE.to_owned(), &self.title).map_err(S::Error::custom)?;
        encode_if_present(
            &mut container,
            camel(RiseLocalKey::COVER_UPLOAD_PROGRESS),
            &self.cover_upload_progress,
        )
        .map_err(S::Error::custom)?;
        encode_if_present(&mut container, camel(RiseLocalKey::COVER_UPLOADING), &self.cover_uploading)
            .map_err(S::Error::custom)?;
        encode_if_present(
            &mut container,
            camel(RiseLocalKey::AUDIO_UPLOAD_PROGRESS),
            &self.audio_upload_progress,
        )
        .map_err(S::Error::custom)?;
        encode_if_present(&mut container, camel(RiseLocalKey::AUDIO_UPLOADING), &self.audio_uploading)
            .map_err(S::Error::custom)?;
        encode_if_present(&mut container, key::TRACK.to_owned(), &self.track).map_err(S::Error::custom)?;
        encode_if_present(&mut container, key::REQUEST.to_owned(), &self.request).map_err(S::Error::custom)?;
        container.serialize(serializer)
    }
}

/// create_playlist or update_playlist answer plus cover upload progress
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RiseStoredMusicPlaylistMutation {
    pub playlist: Option<RiseStoredMusicPlaylist>,
    pub cover_upload_progress: Option<f64>,
    pub cover_uploading: Option<bool>,
}

impl<'de> Deserialize<'de> for RiseStoredMusicPlaylistMutation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                playlist: container.rise::<RiseStoredMusicPlaylist>(key::PLAYLIST)?,
                cover_upload_progress: container.rise::<f64>(RiseLocalKey::COVER_UPLOAD_PROGRESS)?,
                cover_uploading: container.rise::<bool>(RiseLocalKey::COVER_UPLOADING)?,
            })
        })
    }
}

impl Serialize for RiseStoredMusicPlaylistMutation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = Map::new();
        encode_if_present(&mut container, key::PLAYLIST.to_owned(), &self.playlist).map_err(S::Error::custom)?;
        encode_if_present(
            &mut container,
            camel(RiseLocalKey::COVER_UPLOAD_PROGRESS),
            &self.cover_upload_progress,
        )
        .map_err(S::Error::custom)?;
        encode_if_present(&mut container, camel(RiseLocalKey::COVER_UPLOADING), &self.cover_uploading)
            .map_err(S::Error::custom)?;
        container.serialize(serializer)
    }
}

/// create_local_track_in_playlist answer plus upload progress and the temp id of the optimistic row
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RiseStoredMusicLocalTrackUpload {
    pub track: Option<RiseStoredMusicTrack>,
    pub upload_progress: Option<f64>,
    pub is_uploading: Option<bool>,
    pub temp_id: Option<String>,
}

impl<'de> Deserialize<'de> for RiseStoredMusicLocalTrackUpload {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                track: container.rise::<RiseStoredMusicTrack>(key::TRACK)?,
                upload_progress: container.rise::<f64>(RiseLocalKey::UPLOAD_PROGRESS)?,
                is_uploading: container.rise::<bool>(RiseLocalKey::IS_UPLOADING)?,
                temp_id: container.rise::<String>(message_key::TEMP_ID)?,
            })
        })
    }
}

impl Serialize for RiseStoredMusicLocalTrackUpload {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = Map::new();
        encode_if_present(&mut container, key::TRACK.to_owned(), &self.track).map_err(S::Error::custom)?;
        encode_if_present(&mut container, camel(RiseLocalKey::UPLOAD_PROGRESS), &self.upload_progress)
            .map_err(S::Error::custom)?;
        encode_if_present(&mut container, camel(RiseLocalKey::IS_UPLOADING), &self.is_uploading)
            .map_err(S::Error::custom)?;
        encode_if_present(&mut container, camel(message_key::TEMP_ID), &self.temp_id).map_err(S::Error::custom)?;
        container.serialize(serializer)
    }
}

pub type WireAlbumRequestsListDTO = rise_response::music::ListAlbumRequests;
pub type WireTracksListDTO = rise_response::music::ListTracks;
pub type WireLocalTracksListDTO = rise_response::music::ListMyLocalTracks;
pub type WireDeleteLocalTracksDTO = rise_response::music::DeleteLocalTracks;
pub type WireArtistsListDTO = rise_response::music::ListArtists;
pub type WireLocalPlaylistDestinationDTO = rise_proto::music::ProtoLocalPlaylistDestination;
pub type WireLocalPlaylistDestinationsDTO = rise_response::music::ListLocalPlaylistDestinations;
pub type WireSingleArtistDTO = rise_response::music::GetArtist;
pub type WirePlaylistJoinRequestDTO = rise_proto::music::ProtoPlaylistJoinRequestRecord;
pub type WireRequestPlaylistJoinResponseDTO = rise_response::music::RequestPlaylistJoin;
pub type WirePlaylistJoinRequestsDTO = rise_response::music::GetPlaylistJoinRequests;
pub type WireReviewPlaylistJoinRequestResponseDTO = rise_response::music::ReviewPlaylistJoinRequest;
pub type WirePlaylistMembersDTO = rise_response::music::GetPlaylistMembers;
pub type WirePlaylistListenerDTO = rise_proto::music::ProtoPlaylistListenerRecord;
pub type WirePlaylistListeningSnapshotDTO = rise_response::music::GetPlaylistListeningSnapshot;
pub type WirePlaylistPresenceWriteDTO = rise_response::music::UpsertPlaylistListeningPresence;
pub type WirePlaylistListeningSessionResponseDTO = rise_response::music::GetPlaylistListeningSession;
pub type WireExternalTrackDTO = rise_proto::music::ProtoExternalTrackRecord;
pub type WireSearchDTO = rise_reply::music::SearchPage;
pub type WireToggleTrackLikeDTO = rise_response::music::ToggleTrackLike;
pub type WireToggleArtistFollowDTO = rise_response::music::ToggleArtistFollow;
pub type WireModerationQueueMetricsDTO = rise_response::music::GetModerationQueueMetrics;
pub type WireArtistStatusDTO = rise_response::music::GetArtistStatus;
pub type WireArtistRequestsListDTO = rise_response::music::ListArtistRequests;
pub type WireTrackRequestsListDTO = rise_response::music::ListTrackRequests;

/// my local tracks as the library shows them, canon page with local edits applied
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RiseStoredMusicLocalTracksPage {
    pub tracks: Vec<RiseStoredMusicTrack>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub total_count: Option<i64>,
}

pub trait WireLocalTracksListDTOExt {
    fn stored_tracks(&self) -> Vec<RiseStoredMusicTrack>;
    fn replacing_tracks(&self, rows: &[RiseStoredMusicTrack]) -> WireLocalTracksListDTO;
}

impl WireLocalTracksListDTOExt for WireLocalTracksListDTO {
    fn stored_tracks(&self) -> Vec<RiseStoredMusicTrack> {
        self.tracks.iter().map(|track| track.stored()).collect()
    }

    fn replacing_tracks(&self, rows: &[RiseStoredMusicTrack]) -> WireLocalTracksListDTO {
        let mut updated = self.clone();
        updated.tracks = rows.iter().map(RiseStoredMusicTrack::canon).collect();
        updated
    }
}

pub trait WireExternalTrackDTOExt {
    fn id(&self) -> String;
}

impl WireExternalTrackDTOExt for WireExternalTrackDTO {
    fn id(&self) -> String {
        format!("{}:{}", self.source, self.external_id)
    }
}

pub trait WireSearchDTOExt: Sized {
    const EMPTY: Self;

    fn map_tracks(
        &self,
        transform: impl Fn(rise_proto::music::ProtoTrackRecord) -> rise_proto::music::ProtoTrackRecord,
    ) -> Self;

    fn map_artists(&self, transform: impl Fn(WireArtistDTO) -> WireArtistDTO) -> Self;
}

impl WireSearchDTOExt for WireSearchDTO {
    const EMPTY: Self = WireSearchDTO {
        tracks: Vec::new(),
        next_cursor: None,
        has_more: false,
        artists: Some(Vec::new()),
        albums: Some(Vec::new()),
        playlists: Some(Vec::new()),
        external_tracks: Some(Vec::new()),
    };

    fn map_tracks(
        &self,
        transform: impl Fn(rise_proto::music::ProtoTrackRecord) -> rise_proto::music::ProtoTrackRecord,
    ) -> Self {
        let mut updated = self.clone();
        updated.tracks = self.tracks.iter().cloned().map(&transform).collect();
        updated.playlists = self.playlists.as_ref().map(|playlists| {
            playlists
                .iter()
                .map(|pl| {
                    let mut playlist = pl.clone();
                    playlist.tracks = pl.tracks.iter().cloned().map(&transform).collect();
                    playlist
                })
                .collect()
        });
        updated
    }

    fn map_artists(&self, transform: impl Fn(WireArtistDTO) -> WireArtistDTO) -> Self {
        let mut updated = self.clone();
        updated.artists = self.artists.as_ref().map(|artists| artists.iter().cloned().map(&transform).collect());
        updated
    }
}

impl RiseStoredMusicTrack {
    /// copy with changed like fields, pass none to keep the current value
    pub fn with(&self, is_liked: Option<bool>, like_count: Option<i64>, is_exclusive: Option<bool>) -> Self {
        let mut updated = self.clone();
        updated.like_count = like_count.or(self.like_count);
        updated.is_liked = is_liked.or(self.is_liked);
        updated.is_exclusive = is_exclusive.or(self.is_exclusive);
        updated
    }

    /// copy with local title, artist and optional cover url
    pub fn with_local_metadata(&self, title: &str, artist_name: &str, cover_url: Option<String>) -> Self {
        let mut updated_artist = self.artist.clone().unwrap_or_default();
        updated_artist.stage_name = artist_name.to_owned();
        let mut updated = self.clone();
        updated.title = Some(title.to_owned());
        updated.cover_url = cover_url.or_else(|| self.cover_url.clone());
        updated.artist = Some(updated_artist);
        updated
    }
}

pub trait ProtoTrackRecordExt {
    /// the row as the app keeps it, through the one converter
    fn stored(&self) -> RiseStoredMusicTrack;
    fn as_track_model(&self) -> Option<TrackModel>;
    fn with(&self, is_liked: bool, like_count: Option<i64>) -> rise_proto::music::ProtoTrackRecord;
}

impl ProtoTrackRecordExt for rise_proto::music::ProtoTrackRecord {
    fn stored(&self) -> RiseStoredMusicTrack {
        RiseStoredMusicTrack::from_canon(self)
    }

    fn as_track_model(&self) -> Option<TrackModel> {
        self.stored().as_track_model()
    }

    fn with(&self, is_liked: bool, like_count: Option<i64>) -> rise_proto::music::ProtoTrackRecord {
        let mut updated = self.clone();
        updated.is_liked = is_liked;
        updated.like_count = like_count.unwrap_or(self.like_count);
        updated
    }
}

pub trait WireArtistDTOExt {
    /// copy with is_following changed, none keeps the current value
    fn with(&self, is_following: Option<bool>) -> WireArtistDTO;
    fn as_artist_model(&self) -> Option<ArtistModel>;
}

impl WireArtistDTOExt for WireArtistDTO {
    fn with(&self, is_following: Option<bool>) -> WireArtistDTO {
        let mut updated = self.clone();
        updated.is_following = is_following.unwrap_or(self.is_following);
        updated
    }

    fn as_artist_model(&self) -> Option<ArtistModel> {
        if self.id.is_empty() {
            return None;
        }
        Some(ArtistModel {
            id: self.id.clone(),
            name: self.stage_name.clone(),
            cover_colors: Vec::new(),
            cover_image_url: self.avatar_url.clone(),
            monthly_listeners: self.monthly_listeners.max(0),
            genres: self.genres.clone(),
            top_track_ids: Vec::new(),
            album_ids: Vec::new(),
        })
    }
}

pub trait WireTracksListDTOExt {
    fn map_tracks(
        &self,
        transform: impl Fn(rise_proto::music::ProtoTrackRecord) -> rise_proto::music::ProtoTrackRecord,
    ) -> WireTracksListDTO;
}

impl WireTracksListDTOExt for WireTracksListDTO {
    fn map_tracks(
        &self,
        transform: impl Fn(rise_proto::music::ProtoTrackRecord) -> rise_proto::music::ProtoTrackRecord,
    ) -> WireTracksListDTO {
        let mut updated = self.clone();
        updated.tracks = self.tracks.iter().cloned().map(transform).collect();
        updated
    }
}

pub trait WireArtistsListDTOExt {
    fn map_artists(&self, transform: impl Fn(WireArtistDTO) -> WireArtistDTO) -> WireArtistsListDTO;
}

impl WireArtistsListDTOExt for WireArtistsListDTO {
    fn map_artists(&self, transform: impl Fn(WireArtistDTO) -> WireArtistDTO) -> WireArtistsListDTO {
        let mut updated = self.clone();
        updated.artists = self.artists.iter().cloned().map(transform).collect();
        updated
    }
}

impl RiseStoredMusicPlaylistLibrary {
    pub fn map_playlists(
        &self,
        transform: impl Fn(RiseStoredMusicPlaylist) -> RiseStoredMusicPlaylist,
    ) -> RiseStoredMusicPlaylistLibrary {
        RiseStoredMusicPlaylistLibrary {
            playlists: self.playlists.as_ref().map(|playlists| playlists.iter().cloned().map(transform).collect()),
        }
    }
}

pub trait WireSingleArtistDTOExt {
    fn map_artist(&self, transform: impl Fn(WireArtistDTO) -> WireArtistDTO) -> WireSingleArtistDTO;
}

impl WireSingleArtistDTOExt for WireSingleArtistDTO {
    fn map_artist(&self, transform: impl Fn(WireArtistDTO) -> WireArtistDTO) -> WireSingleArtistDTO {
        let Some(a) = self.artist.clone() else {
            return self.clone();
        };
        let mut updated = self.clone();
        updated.artist = Some(transform(a));
        updated
    }
}

pub trait WireSingleAlbumDTOExt {
    fn map_tracks(
        &self,
        transform: impl Fn(rise_proto::music::ProtoTrackRecord) -> rise_proto::music::ProtoTrackRecord,
    ) -> WireSingleAlbumDTO;
}

impl WireSingleAlbumDTOExt for WireSingleAlbumDTO {
    fn map_tracks(
        &self,
        transform: impl Fn(rise_proto::music::ProtoTrackRecord) -> rise_proto::music::ProtoTrackRecord,
    ) -> WireSingleAlbumDTO {
        let Some(mut album) = self.album.clone() else {
            return self.clone();
        };
        album.tracks = album.tracks.into_iter().map(transform).collect();
        let mut updated = self.clone();
        updated.album = Some(album);
        updated
    }
}

impl RiseStoredMusicTrack {
    /// duration in seconds truncated like every canon duration
    /// rounding up made 111.5 s read 1:52 in lists and 1:51 in the player
    fn duration_seconds(from_ms: Option<i64>) -> i64 {
        let Some(ms) = from_ms.filter(|ms| *ms > 0) else {
            return 0;
        };
        (ms / 1000).max(1)
    }

    pub fn artist_title_line(&self) -> String {
        let shown_title = if self.status.as_deref() == Some(RiseMusicTrackStatus::OriginalAudio.wire()) {
            tr("original_audio_title")
        } else {
            self.title.clone().unwrap_or_default()
        };
        let stage_name = self.artist.as_ref().map(|artist| artist.stage_name.as_str()).unwrap_or("");
        format!("{stage_name} - {shown_title}")
    }

    pub fn as_track_model(&self) -> Option<TrackModel> {
        let id = self.id.clone().filter(|id| !id.is_empty())?;
        let artist_id = self.artist.as_ref().map(|artist| artist.id.clone()).unwrap_or_default();
        Some(TrackModel {
            id,
            title: self.title.clone().unwrap_or_default(),
            artist: self.artist.as_ref().map(|artist| artist.stage_name.clone()).unwrap_or_default(),
            artist_id: artist_id.clone(),
            album_title: self.album.as_ref().map(|album| album.title.clone()).unwrap_or_default(),
            album_id: self.album.as_ref().map(|album| album.id.clone()).unwrap_or_default(),
            duration_seconds: Self::duration_seconds(self.duration_ms),
            cover_colors: Vec::new(),
            cover_image_url: MusicMediaURL::resolved_url_string(self.cover_url.as_deref()).unwrap_or_default(),
            audio_url: MusicMediaURL::resolved_url_string(self.audio_url.as_deref()).unwrap_or_default(),
            is_exclusive: self.is_exclusive == Some(true),
            status: self.status.clone().unwrap_or_default(),
            description: self.description.clone(),
            source_chat_id: self.source_chat_id.clone(),
            source_message_id: self.source_message_id.clone(),
            artist_is_registered: self.source_chat_id.is_none() || !artist_id.is_empty(),
            remote_audio_url: MusicMediaURL::resolved_url_string(self.remote_audio_url.as_deref()),
            transfer_phase: self.transfer_phase.as_deref().and_then(MusicLocalTrackTransferPhase::from_raw),
            transferred_bytes: self.transferred_bytes.unwrap_or(0).max(0),
            transfer_total_bytes: self.transfer_total_bytes.unwrap_or(0).max(0),
            transfer_error: self.transfer_error.clone(),
            library_owner_user_id: self.library_owner_user_id.clone(),
            playback_cache_id: None,
            is_liked: self.is_liked == Some(true),
        })
    }
}

pub trait WireLocalPlaylistDestinationDTOExt {
    fn as_domain(&self) -> Option<MusicLocalPlaylistDestination>;
}

impl WireLocalPlaylistDestinationDTOExt for WireLocalPlaylistDestinationDTO {
    fn as_domain(&self) -> Option<MusicLocalPlaylistDestination> {
        if self.id.is_empty() {
            return None;
        }
        Some(MusicLocalPlaylistDestination {
            id: self.id.clone(),
            title: MusicDefaultLocalPlaylist::resolved_title(Some(self.title.as_str()), Some(self.is_default_local)),
            description: self.description.clone(),
            cover_url: RemoteMediaURL::optional_normalized(Some(self.cover_url.as_str())).unwrap_or_default(),
            owner_user_id: self.owner_user_id.clone(),
            viewer_role: if self.viewer_role.is_empty() { "member".to_owned() } else { self.viewer_role.clone() },
            can_add_tracks: self.can_add_tracks,
            updated_at_ms: self.updated_at_ms,
            revision: self.revision,
        })
    }
}

pub trait WireAlbumDTOExt {
    fn as_album_model(&self) -> Option<AlbumModel>;
}

impl WireAlbumDTOExt for WireAlbumDTO {
    fn as_album_model(&self) -> Option<AlbumModel> {
        if self.id.is_empty() {
            return None;
        }
        let year = if self.release_date_ms > 0 {
            Calendar::current().component(
                CalendarComponent::Year,
                &Date::from_time_interval_since_1970(self.release_date_ms as f64 / 1000.0),
            )
        } else {
            Calendar::current().component(CalendarComponent::Year, &RiseClock::now())
        };
        Some(AlbumModel {
            id: self.id.clone(),
            title: self.title.clone(),
            artist_id: self.artist.as_ref().map(|artist| artist.id.clone()).unwrap_or_default(),
            artist_name: self.artist.as_ref().map(|artist| artist.stage_name.clone()).unwrap_or_default(),
            year,
            cover_colors: Vec::new(),
            cover_image_url: RemoteMediaURL::optional_normalized(Some(self.cover_url.as_str())).unwrap_or_default(),
            tracks: self
                .tracks
                .iter()
                .filter_map(|track| RiseStoredMusicTrack::from_canon(track).as_track_model())
                .collect(),
            status: if self.status.is_empty() { "published".to_owned() } else { self.status.clone() },
            scheduled_at_ms: self.scheduled_at_ms,
        })
    }
}

pub trait WireAlbumRequestDTOExt {
    fn status_badge(&self) -> String;
    fn cover_image_url(&self) -> String;
}

impl WireAlbumRequestDTOExt for WireAlbumRequestDTO {
    fn status_badge(&self) -> String {
        match self.status.to_lowercase().as_str() {
            music_service::ALBUM_REQUEST_STATUS_PENDING => tr("music_status_pending"),
            music_service::ALBUM_REQUEST_STATUS_APPROVED => tr("music_status_approved"),
            music_service::ALBUM_REQUEST_STATUS_REJECTED => tr("music_status_rejected"),
            _ => self.status.clone(),
        }
    }

    fn cover_image_url(&self) -> String {
        RemoteMediaURL::optional_normalized(Some(self.cover_url.as_str())).unwrap_or_default()
    }
}

impl RiseStoredMusicPlaylist {
    pub fn display_title(&self) -> String {
        MusicDefaultLocalPlaylist::resolved_title(self.title.as_deref(), self.is_default_local)
    }

    pub fn playlist_visibility_label(&self) -> String {
        match self.visibility.as_deref().unwrap_or("private") {
            "public" => tr("music_visibility_public"),
            "local" => tr("music_visibility_local"),
            _ => tr("music_visibility_private"),
        }
    }

    pub fn playlist_list_subtitle(&self) -> String {
        let count = self
            .tracks
            .as_ref()
            .map(|tracks| tracks.iter().filter(|track| track.transfer_phase.is_none()).count())
            .unwrap_or(0);
        format!("{} \u{2022} {}", self.playlist_visibility_label(), tr_plural("music_track_count", count as i64))
    }

    /// copy with changed playlist fields, pass none to keep the current value
    pub fn with(
        &self,
        title: Option<String>,
        description: Option<String>,
        cover_url: Option<String>,
        visibility: Option<String>,
        tracks: Option<Vec<RiseStoredMusicTrack>>,
    ) -> RiseStoredMusicPlaylist {
        let renamed = title.is_some() && title != self.title;
        let mut updated = self.clone();
        updated.title = title.or_else(|| self.title.clone());
        updated.description = description.or_else(|| self.description.clone());
        updated.cover_url = cover_url.or_else(|| self.cover_url.clone());
        updated.visibility = visibility.or_else(|| self.visibility.clone());
        updated.tracks = tracks.or_else(|| self.tracks.clone());
        updated.is_default_local = if renamed { Some(false) } else { self.is_default_local };
        updated
    }

    pub fn as_playlist_model(&self) -> Option<PlaylistModel> {
        let id = self.id.clone().filter(|id| !id.is_empty())?;
        let desc = trimming_whitespaces(self.description.as_deref().unwrap_or(""));
        let subtitle = if desc.is_empty() {
            self.playlist_list_subtitle()
        } else {
            format!("{}\n{desc}", self.playlist_list_subtitle())
        };
        Some(PlaylistModel {
            id,
            title: MusicDefaultLocalPlaylist::resolved_title(self.title.as_deref(), self.is_default_local),
            subtitle,
            cover_colors: Vec::new(),
            cover_image_url: RemoteMediaURL::optional_normalized(self.cover_url.as_deref()).unwrap_or_default(),
            tracks: self.track_models(),
            visibility: self.visibility.clone().unwrap_or_else(|| "private".to_owned()),
            owner_user_id: self.owner_user_id.clone().unwrap_or_default(),
        })
    }

    pub fn as_music_playlist_summary(&self) -> Option<MusicPlaylistSummary> {
        let id = self.id.clone().filter(|id| !id.is_empty())?;

        Some(MusicPlaylistSummary {
            id,
            owner_user_id: self.owner_user_id.clone().unwrap_or_default(),
            title: MusicDefaultLocalPlaylist::resolved_title(self.title.as_deref(), self.is_default_local),
            description: self.description.clone().unwrap_or_default(),
            cover_url: RemoteMediaURL::optional_normalized(self.cover_url.as_deref()).unwrap_or_default(),
            visibility: self.visibility.clone().unwrap_or_else(|| "private".to_owned()),
            tracks: self.track_models(),
            created_at_ms: self.created_at_ms.unwrap_or(0),
            updated_at_ms: self.updated_at_ms.unwrap_or(0),
            revision: self.revision.unwrap_or(0),
            new_tracks_on_top: self.new_tracks_on_top.unwrap_or(false),
        })
    }

    fn track_models(&self) -> Vec<TrackModel> {
        self.tracks
            .as_ref()
            .map(|tracks| tracks.iter().filter_map(RiseStoredMusicTrack::as_track_model).collect())
            .unwrap_or_default()
    }
}

pub trait WireArtistAnalyticsDTOExt {
    fn as_model(&self) -> ArtistAnalyticsModel;
}

impl WireArtistAnalyticsDTOExt for WireArtistAnalyticsDTO {
    fn as_model(&self) -> ArtistAnalyticsModel {
        ArtistAnalyticsModel {
            total_plays: self.total_plays,
            total_likes: self.total_likes,
            total_followers: self.total_followers,
            monthly_listeners: self.monthly_listeners,
            total_tracks: self.total_tracks,
            total_albums: self.total_albums,
            retention7d: self.retention7d.unwrap_or(0.0),
            retention30d: self.retention30d.unwrap_or(0.0),
        }
    }
}

pub trait WireModerationQueueMetricsDTOExt {
    fn as_model(&self) -> ModerationQueueMetricsModel;
}

impl WireModerationQueueMetricsDTOExt for WireModerationQueueMetricsDTO {
    fn as_model(&self) -> ModerationQueueMetricsModel {
        ModerationQueueMetricsModel {
            pending_tracks: self.pending_tracks,
            pending_albums: self.pending_albums,
            approved_today: self.approved_today,
            rejected_today: self.rejected_today,
            avg_review_time_hours: self.avg_review_time_hours.unwrap_or(0.0),
        }
    }
}

pub trait WireUserRefDTOExt {
    fn as_music_user_reference(&self, fallback_id: &str) -> MusicUserReference;
}

impl WireUserRefDTOExt for WireUserRefDTO {
    fn as_music_user_reference(&self, fallback_id: &str) -> MusicUserReference {
        let id = if self.id.is_empty() { fallback_id } else { self.id.as_str() };
        MusicUserReference {
            id: trimming_whitespaces_and_newlines(id).to_owned(),
            name: self.name.clone(),
            tag: self.tag.clone(),
            logo: RemoteMediaURL::optional_normalized(Some(self.logo.as_str())).unwrap_or_default(),
            is_premium: self.is_premium,
            premium_badge_emoji: self.premium_badge_emoji.clone(),
            name_color: self.name_color.clone(),
        }
    }
}

impl MusicUserReference {
    /// stand in for a row whose user object didnt arrive, only the id is known
    pub fn bare(id: &str) -> MusicUserReference {
        MusicUserReference {
            id: id.to_owned(),
            name: String::new(),
            tag: String::new(),
            logo: String::new(),
            is_premium: false,
            premium_badge_emoji: None,
            name_color: None,
        }
    }
}

/// user of a row or the bare stand in when the user object didnt arrive
fn user_reference(user: Option<&WireUserRefDTO>, user_id: &str) -> MusicUserReference {
    user.map(|user| user.as_music_user_reference(user_id)).unwrap_or_else(|| MusicUserReference::bare(user_id))
}

pub trait WirePlaylistMemberPermissionsDTOExt {
    fn as_domain(&self) -> MusicPlaylistMemberPermissions;
}

impl WirePlaylistMemberPermissionsDTOExt for WirePlaylistMemberPermissionsDTO {
    fn as_domain(&self) -> MusicPlaylistMemberPermissions {
        MusicPlaylistMemberPermissions {
            can_edit_playlist: self.can_edit_playlist,
            can_add_tracks: self.can_add_tracks,
            can_remove_tracks: self.can_remove_tracks,
        }
    }
}

pub trait WirePlaylistSettingsDTOExt {
    fn as_domain(&self) -> Option<MusicPlaylistSettings>;
}

impl WirePlaylistSettingsDTOExt for WirePlaylistSettingsDTO {
    fn as_domain(&self) -> Option<MusicPlaylistSettings> {
        let playlist = self
            .playlist
            .as_ref()
            .and_then(|playlist| RiseStoredMusicPlaylist::from_canon(playlist).as_music_playlist_summary())?;

        Some(MusicPlaylistSettings {
            playlist,
            viewer_role: if self.viewer_role.is_empty() { "member".to_owned() } else { self.viewer_role.clone() },
            viewer_permissions: self
                .viewer_permissions
                .as_ref()
                .map(WirePlaylistMemberPermissionsDTOExt::as_domain)
                .unwrap_or(MusicPlaylistMemberPermissions::NONE),
            total_participant_count: self.total_participant_count,
            active_participant_count: self.active_participant_count,
            suspended_member_count: self.suspended_member_count,
            pending_request_count: self.pending_request_count,
            effective_participant_limit: self.effective_participant_limit,
            owner_is_premium: self.owner_is_premium,
            can_invite: self.can_invite,
            can_review_requests: self.can_review_requests,
            access_state: if self.access_state.is_empty() { "active".to_owned() } else { self.access_state.clone() },
            access_epoch: self.access_epoch,
        })
    }
}

pub trait WirePlaylistInviteLinkDTOExt {
    fn as_domain(&self) -> Option<MusicPlaylistInviteLink>;
}

impl WirePlaylistInviteLinkDTOExt for WirePlaylistInviteLinkDTO {
    fn as_domain(&self) -> Option<MusicPlaylistInviteLink> {
        if self.playlist_id.is_empty() || self.token.is_empty() {
            return None;
        }

        Some(MusicPlaylistInviteLink {
            playlist_id: self.playlist_id.clone(),
            url: self.url.clone(),
            token: self.token.clone(),
            version: self.version,
        })
    }
}

pub trait WirePlaylistInvitePreviewDTOExt {
    fn as_domain(&self) -> Option<MusicPlaylistInvitePreview>;
}

impl WirePlaylistInvitePreviewDTOExt for WirePlaylistInvitePreviewDTO {
    fn as_domain(&self) -> Option<MusicPlaylistInvitePreview> {
        if self.playlist_id.is_empty() {
            return None;
        }

        let resolved_owner_id = if self.owner_user_id.is_empty() {
            self.owner.as_ref().map(|owner| owner.id.clone()).unwrap_or_default()
        } else {
            self.owner_user_id.clone()
        };
        Some(MusicPlaylistInvitePreview {
            playlist_id: self.playlist_id.clone(),
            owner_user_id: resolved_owner_id.clone(),
            title: MusicDefaultLocalPlaylist::resolved_title(Some(self.title.as_str()), None),
            description: self.description.clone(),
            cover_url: RemoteMediaURL::optional_normalized(Some(self.cover_url.as_str())).unwrap_or_default(),
            owner: user_reference(self.owner.as_ref(), &resolved_owner_id),
            active_participant_count: self.active_participant_count,
            effective_participant_limit: self.effective_participant_limit,
            owner_is_premium: self.owner_is_premium,
            viewer_state: if self.viewer_state.is_empty() {
                music_service::PLAYLIST_VIEWER_STATE_CAN_REQUEST.to_owned()
            } else {
                self.viewer_state.clone()
            },
        })
    }
}

pub trait WirePlaylistJoinRequestDTOExt {
    fn as_domain(&self) -> Option<MusicPlaylistJoinRequest>;
}

impl WirePlaylistJoinRequestDTOExt for WirePlaylistJoinRequestDTO {
    fn as_domain(&self) -> Option<MusicPlaylistJoinRequest> {
        if self.id.is_empty() || self.playlist_id.is_empty() || self.user_id.is_empty() {
            return None;
        }

        Some(MusicPlaylistJoinRequest {
            id: self.id.clone(),
            playlist_id: self.playlist_id.clone(),
            user_id: self.user_id.clone(),
            user: user_reference(self.user.as_ref(), &self.user_id),
            status: if self.status.is_empty() { "pending".to_owned() } else { self.status.clone() },
            requested_at_ms: self.requested_at_ms,
            processed_at_ms: self.processed_at_ms,
            request_version: self.request_version,
        })
    }
}

pub trait WirePlaylistMemberDTOExt {
    fn as_domain(&self) -> Option<MusicPlaylistMember>;
}

impl WirePlaylistMemberDTOExt for WirePlaylistMemberDTO {
    fn as_domain(&self) -> Option<MusicPlaylistMember> {
        if self.playlist_id.is_empty() || self.user_id.is_empty() {
            return None;
        }

        Some(MusicPlaylistMember {
            playlist_id: self.playlist_id.clone(),
            user_id: self.user_id.clone(),
            user: user_reference(self.user.as_ref(), &self.user_id),
            role: if self.role.is_empty() { "member".to_owned() } else { self.role.clone() },
            status: if self.status.is_empty() { "active".to_owned() } else { self.status.clone() },
            permissions: self
                .permissions
                .as_ref()
                .map(WirePlaylistMemberPermissionsDTOExt::as_domain)
                .unwrap_or(MusicPlaylistMemberPermissions::NONE),
            accepted_at_ms: self.accepted_at_ms,
            version: self.version,
        })
    }
}

pub trait WirePlaylistListenerDTOExt {
    fn as_domain(&self, playlist_id: &str) -> Option<MusicPlaylistListener>;
}

impl WirePlaylistListenerDTOExt for WirePlaylistListenerDTO {
    fn as_domain(&self, playlist_id: &str) -> Option<MusicPlaylistListener> {
        if self.user_id.is_empty() || self.track_id.is_empty() {
            return None;
        }

        Some(MusicPlaylistListener {
            playlist_id: playlist_id.to_owned(),
            user_id: self.user_id.clone(),
            user: user_reference(self.user.as_ref(), &self.user_id),
            track_id: self.track_id.clone(),
            anchor_position_ms: self.anchor_position_ms,
            is_playing: self.is_playing,
            anchor_server_time_ms: self.anchor_server_time_ms,
            client_sequence: self.client_sequence,
            session_id: self.session_id.clone(),
            audio_effects: self.audio_effects.iter().filter_map(WireListeningAudioEffectDTOExt::as_domain).collect(),
        })
    }
}

pub trait WireListeningParticipantPermissionsDTOExt {
    fn as_domain(&self) -> MusicListeningParticipantPermissions;
}

impl WireListeningParticipantPermissionsDTOExt for WireListeningParticipantPermissionsDTO {
    fn as_domain(&self) -> MusicListeningParticipantPermissions {
        MusicListeningParticipantPermissions {
            can_control_playback: self.can_control_playback,
            can_seek: self.can_seek,
            can_change_tracks: self.can_change_tracks,
            can_apply_effects: self.can_apply_effects,
        }
    }
}

pub trait WireListeningAudioEffectDTOExt {
    fn as_domain(&self) -> Option<MusicListeningAudioEffect>;
}

impl WireListeningAudioEffectDTOExt for WireListeningAudioEffectDTO {
    fn as_domain(&self) -> Option<MusicListeningAudioEffect> {
        MusicAudioEffectKind::from_raw_value(&self.kind)?;

        Some(MusicListeningAudioEffect { kind: self.kind.clone(), value_milli: self.value_milli })
    }
}

pub trait WireListeningParticipantDTOExt {
    fn as_domain(&self) -> Option<MusicListeningParticipant>;
}

impl WireListeningParticipantDTOExt for WireListeningParticipantDTO {
    fn as_domain(&self) -> Option<MusicListeningParticipant> {
        if self.user_id.is_empty() {
            return None;
        }

        Some(MusicListeningParticipant {
            user_id: self.user_id.clone(),
            user: user_reference(self.user.as_ref(), &self.user_id),
            permissions: self
                .permissions
                .as_ref()
                .map(WireListeningParticipantPermissionsDTOExt::as_domain)
                .unwrap_or(MusicListeningParticipantPermissions::NONE),
            joined_at_ms: self.joined_at_ms,
        })
    }
}

pub trait WirePlaylistListeningSessionDTOExt {
    fn as_domain(&self) -> Option<MusicPlaylistListeningSession>;
}

impl WirePlaylistListeningSessionDTOExt for WirePlaylistListeningSessionDTO {
    fn as_domain(&self) -> Option<MusicPlaylistListeningSession> {
        if self.session_id.is_empty()
            || self.playlist_id.is_empty()
            || self.host_user_id.is_empty()
            || self.track_id.is_empty()
        {
            return None;
        }

        Some(MusicPlaylistListeningSession {
            session_id: self.session_id.clone(),
            playlist_id: self.playlist_id.clone(),
            host_user_id: self.host_user_id.clone(),
            host_user: user_reference(self.host_user.as_ref(), &self.host_user_id),
            track_id: self.track_id.clone(),
            anchor_position_ms: self.anchor_position_ms,
            is_playing: self.is_playing,
            anchor_server_time_ms: self.anchor_server_time_ms,
            revision: self.revision,
            generation: self.generation,
            shuffle_enabled: self.shuffle_enabled,
            shuffle_seed: self.shuffle_seed.max(0),
            repeat_mode: if self.repeat_mode.is_empty() { "off".to_owned() } else { self.repeat_mode.clone() },
            participants: self.participants.iter().filter_map(WireListeningParticipantDTOExt::as_domain).collect(),
            viewer_permissions: self
                .viewer_permissions
                .as_ref()
                .map(WireListeningParticipantPermissionsDTOExt::as_domain)
                .unwrap_or(MusicListeningParticipantPermissions::NONE),
            lease_until_ms: self.lease_until_ms,
            audio_effect_enabled: self.audio_effect_enabled,
            audio_effect_rate_milli: legacy_rate_milli(self),
            audio_effects: if self.audio_effects.is_empty() {
                legacy_audio_effects(self)
            } else {
                self.audio_effects.iter().filter_map(WireListeningAudioEffectDTOExt::as_domain).collect()
            },
        })
    }
}

fn legacy_rate_milli(session: &WirePlaylistListeningSessionDTO) -> i64 {
    MusicEffectPlaybackLimits::rate_milli(MusicEffectPlaybackLimits::rate(
        if session.audio_effect_rate_milli == 0 {
            MusicEffectPlaybackLimits::rate_milli(1.0)
        } else {
            session.audio_effect_rate_milli
        },
    ))
}

fn legacy_audio_effects(session: &WirePlaylistListeningSessionDTO) -> Vec<MusicListeningAudioEffect> {
    if !session.audio_effect_enabled {
        return Vec::new();
    }
    vec![MusicListeningAudioEffect {
        kind: MusicAudioEffectKind::Slowed.raw_value().to_owned(),
        value_milli: legacy_rate_milli(session),
    }]
}

pub trait WirePlaylistListeningSnapshotDTOExt {
    fn as_domain(&self, playlist_id: &str) -> MusicPlaylistListeningSnapshot;
}

impl WirePlaylistListeningSnapshotDTOExt for WirePlaylistListeningSnapshotDTO {
    fn as_domain(&self, playlist_id: &str) -> MusicPlaylistListeningSnapshot {
        MusicPlaylistListeningSnapshot {
            listeners: self.listeners.iter().filter_map(|listener| listener.as_domain(playlist_id)).collect(),
            total_listener_count: self.total_listener_count,
            presence_sequence: self.presence_sequence,
            server_time_ms: self.server_time_ms,
            active_session: self.active_session.as_ref().and_then(WirePlaylistListeningSessionDTOExt::as_domain),
            watcher_lease_until_ms: self.watcher_lease_until_ms,
            unchanged: self.unchanged,
        }
    }
}

fn camel(key: &str) -> String {
    <RiseWireKey as RiseWireKeyCamel>::camel(key)
}

/// json object a canon record encodes to so device local keys can join it in one container
fn canon_object<T: Serialize>(canon: &T) -> Result<Map<String, Value>, serde_json::Error> {
    match serde_json::to_value(canon)? {
        Value::Object(object) => Ok(object),
        _ => Ok(Map::new()),
    }
}

fn encode_if_present<T: Serialize>(
    container: &mut Map<String, Value>,
    key: String,
    value: &Option<T>,
) -> Result<(), serde_json::Error> {
    if let Some(value) = value {
        container.insert(key, serde_json::to_value(value)?);
    }
    Ok(())
}
