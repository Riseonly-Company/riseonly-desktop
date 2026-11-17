use std::any::Any;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Deref;
use std::rc::Rc;

use gpui::{App, AsyncApp, Context, Entity, Global, Task};
use rise_canon::rise_key::music as key;
use rise_canon::rise_service_text::music_service;
use rise_canon::{RiseRequestBody, rise_request, rise_response, rise_wire_methods};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::{trimming_whitespaces, trimming_whitespaces_and_newlines};
use rise_engine::CancellationError;
use rise_errors::ErrorCode;
use rise_i18n::tr;
use rise_music::PlaylistVisibility;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::restriction::restriction_store::RestrictionStore;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiTypedSlice, SaiWireEmptyDTO};
use crate::core::lib::sai::core::websocket::sai_ws_response_debt_coordinator::SaiWsResponseDebtCoordinator;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::{SaiWsOptimisticLocalFile, SaiWsOptimisticUpdate};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::music::engine::rise_music_domain::RiseMusicActionExecutor;
use crate::modules::music::engine::rise_music_repository::RiseMusicLocalCover;
use crate::modules::music::shared::utils::music::music_effect_file_player::MusicEffectPlaybackLimits;
use crate::modules::music::shared::utils::music::music_models::MusicLocalTrackTransferPhase;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_types::{
    MusicLocalPlaylistDestination, MusicLocalTracksDeleteSnapshot, MusicPlaylistInvitePreview, MusicPlaylistJoinRequest, MusicPlaylistMember,
    MusicListeningAudioEffect, MusicListeningParticipantPermissions, MusicListeningSessionCommand,
    MusicPlaylistListeningSession, MusicPlaylistMemberPermissions,
};
use crate::modules::music::stores::music_actions::music_wire_types::{
    ProtoTrackRecordExt, RiseStoredMusicLocalTrackUpload, WirePlaylistInvitePreviewDTOExt, RiseStoredMusicPlaylist, RiseStoredMusicPlaylistDetail,
    RiseStoredMusicPlaylistLibrary, RiseStoredMusicPlaylistMutation, RiseStoredMusicTrack, RiseStoredMusicUpload,
    WireAlbumRequestsListDTO, WireAlbumsListDTO, WireArtistAnalyticsDTO, WireArtistRequestsListDTO,
    WireArtistStatusDTO, WireArtistsListDTO, WireDeleteLocalTracksDTO, WireLocalPlaylistDestinationsDTO,
    WireLocalTracksListDTO, WireModerationQueueMetricsDTO, WirePlaylistInviteLinkResponseDTO,
    WirePlaylistInvitePreviewResponseDTO, WirePlaylistJoinRequestsDTO, WirePlaylistListeningSessionResponseDTO,
    WirePlaylistListeningSnapshotDTO, WirePlaylistMemberResponseDTO, WirePlaylistMembersDTO,
    WirePlaylistPresenceWriteDTO, WirePlaylistSettingsResponseDTO, WirePublishAlbumDTO, WirePublishTrackDTO,
    WireRequestPlaylistJoinResponseDTO, WireReviewPlaylistJoinRequestResponseDTO, WireSearchDTO,
    WireSingleAlbumDTO, WireSingleAlbumRequestDTO, WireSingleArtistDTO, WireSingleArtistRequestDTO,
    WireSingleTrackDTO, WireToggleArtistFollowDTO, WireToggleTrackLikeDTO, WireTrackRequestsListDTO,
    WireTracksListDTO,
};
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

pub struct MusicActionSlices {
    pub list_tracks_sai: Rc<SaiTypedSlice<WireTracksListDTO>>,
    pub list_artist_tracks_sai: Rc<SaiTypedSlice<WireTracksListDTO>>,
    pub get_track_sai: Rc<SaiTypedSlice<WireSingleTrackDTO>>,
    pub get_liked_sai: Rc<SaiTypedSlice<rise_response::music::GetLiked>>,
    pub get_recent_sai: Rc<SaiTypedSlice<rise_response::music::GetRecent>>,
    pub list_artists_sai: Rc<SaiTypedSlice<WireArtistsListDTO>>,
    pub get_artist_sai: Rc<SaiTypedSlice<WireSingleArtistDTO>>,
    pub now_playing_artist_sai: Rc<SaiTypedSlice<WireSingleArtistDTO>>,
    pub toggle_track_like_sai: Rc<SaiTypedSlice<WireToggleTrackLikeDTO>>,
    pub toggle_artist_follow_sai: Rc<SaiTypedSlice<WireToggleArtistFollowDTO>>,
    pub record_play_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_playlists_mine_sai: Rc<SaiTypedSlice<RiseStoredMusicPlaylistLibrary>>,
    pub list_local_playlist_destinations_sai: Rc<SaiTypedSlice<WireLocalPlaylistDestinationsDTO>>,
    pub get_playlist_sai: Rc<SaiTypedSlice<RiseStoredMusicPlaylistDetail>>,
    pub create_playlist_sai: Rc<SaiTypedSlice<RiseStoredMusicPlaylistMutation>>,
    pub update_playlist_sai: Rc<SaiTypedSlice<RiseStoredMusicPlaylistMutation>>,
    pub delete_playlist_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub add_track_to_playlist_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub remove_track_from_playlist_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub move_playlist_track_sai: Rc<SaiTypedSlice<rise_response::music::MovePlaylistTrack>>,
    pub create_local_track_in_playlist_sai: Rc<SaiTypedSlice<RiseStoredMusicLocalTrackUpload>>,
    pub message_mp3_create_local_track_sai: Rc<SaiTypedSlice<RiseStoredMusicLocalTrackUpload>>,
    pub search_sai: Rc<SaiTypedSlice<WireSearchDTO>>,
    pub search_artists_sai: Rc<SaiTypedSlice<WireSearchDTO>>,
    pub get_artist_status_sai: Rc<SaiTypedSlice<WireArtistStatusDTO>>,
    pub request_artist_sai: Rc<SaiTypedSlice<rise_response::music::RequestArtist>>,
    pub get_artist_analytics_sai: Rc<SaiTypedSlice<WireArtistAnalyticsDTO>>,
    pub get_moderation_queue_metrics_sai: Rc<SaiTypedSlice<WireModerationQueueMetricsDTO>>,
    pub list_my_tracks_sai: Rc<SaiTypedSlice<rise_response::music::ListMyTracks>>,
    pub list_my_local_tracks_sai: Rc<SaiTypedSlice<WireLocalTracksListDTO>>,
    pub delete_local_tracks_sai: Rc<SaiTypedSlice<WireDeleteLocalTracksDTO>>,
    pub create_track_sai: Rc<SaiTypedSlice<RiseStoredMusicUpload>>,
    pub update_track_sai: Rc<SaiTypedSlice<rise_response::music::UpdateTrack>>,
    pub delete_track_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub list_my_albums_sai: Rc<SaiTypedSlice<WireAlbumsListDTO>>,
    pub get_album_sai: Rc<SaiTypedSlice<WireSingleAlbumDTO>>,
    pub request_album_sai: Rc<SaiTypedSlice<RiseStoredMusicUpload>>,
    pub get_my_album_requests_sai: Rc<SaiTypedSlice<rise_response::music::GetMyAlbumRequests>>,
    pub publish_track_sai: Rc<SaiTypedSlice<WirePublishTrackDTO>>,
    pub publish_album_sai: Rc<SaiTypedSlice<WirePublishAlbumDTO>>,
    pub list_artist_requests_sai: Rc<SaiTypedSlice<WireArtistRequestsListDTO>>,
    pub get_artist_request_sai: Rc<SaiTypedSlice<WireSingleArtistRequestDTO>>,
    pub review_artist_request_sai: Rc<SaiTypedSlice<rise_response::music::ReviewArtistRequest>>,
    pub list_track_requests_sai: Rc<SaiTypedSlice<WireTrackRequestsListDTO>>,
    pub get_track_request_sai: Rc<SaiTypedSlice<WireSingleTrackDTO>>,
    pub review_track_request_sai: Rc<SaiTypedSlice<rise_response::music::ReviewTrackRequest>>,
    pub get_my_artist_request_sai: Rc<SaiTypedSlice<rise_response::music::GetMyArtistRequest>>,
    pub list_album_requests_sai: Rc<SaiTypedSlice<WireAlbumRequestsListDTO>>,
    pub get_album_request_sai: Rc<SaiTypedSlice<WireSingleAlbumRequestDTO>>,
    pub review_album_request_sai: Rc<SaiTypedSlice<rise_response::music::ReviewAlbumRequest>>,
    pub get_playlist_settings_sai: Rc<SaiTypedSlice<WirePlaylistSettingsResponseDTO>>,
    pub get_playlist_invite_link_sai: Rc<SaiTypedSlice<WirePlaylistInviteLinkResponseDTO>>,
    pub get_playlist_invite_preview_sai: Rc<SaiTypedSlice<WirePlaylistInvitePreviewResponseDTO>>,
    pub request_playlist_join_sai: Rc<SaiTypedSlice<WireRequestPlaylistJoinResponseDTO>>,
    pub get_playlist_join_requests_sai: Rc<SaiTypedSlice<WirePlaylistJoinRequestsDTO>>,
    pub review_playlist_join_request_sai: Rc<SaiTypedSlice<WireReviewPlaylistJoinRequestResponseDTO>>,
    pub get_playlist_members_sai: Rc<SaiTypedSlice<WirePlaylistMembersDTO>>,
    pub update_playlist_member_permissions_sai: Rc<SaiTypedSlice<WirePlaylistMemberResponseDTO>>,
    pub remove_playlist_member_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub leave_playlist_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_playlist_listening_snapshot_sai: Rc<SaiTypedSlice<WirePlaylistListeningSnapshotDTO>>,
    pub upsert_playlist_listening_presence_sai: Rc<SaiTypedSlice<WirePlaylistPresenceWriteDTO>>,
    pub stop_playlist_listening_presence_sai: Rc<SaiTypedSlice<rise_response::music::StopPlaylistListeningPresence>>,
    pub join_playlist_listening_session_sai: Rc<SaiTypedSlice<WirePlaylistListeningSessionResponseDTO>>,
    pub get_playlist_listening_session_sai: Rc<SaiTypedSlice<WirePlaylistListeningSessionResponseDTO>>,
    pub update_playlist_listening_session_state_sai: Rc<SaiTypedSlice<WirePlaylistListeningSessionResponseDTO>>,
    pub request_playlist_listening_permission_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub update_playlist_listening_participant_permissions_sai:
        Rc<SaiTypedSlice<WirePlaylistListeningSessionResponseDTO>>,
    pub leave_playlist_listening_session_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub end_playlist_listening_session_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
}

pub struct MusicActionsStore {
    slices: Rc<MusicActionSlices>,
    track_requests_list_status: String,
}

impl Deref for MusicActionsStore {
    type Target = MusicActionSlices;

    fn deref(&self) -> &MusicActionSlices {
        &self.slices
    }
}

struct MusicActionSlicesGlobal(Rc<MusicActionSlices>);

impl Global for MusicActionSlicesGlobal {}

impl MusicActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    pub fn slices(cx: &mut App) -> Rc<MusicActionSlices> {
        if !cx.has_global::<MusicActionSlicesGlobal>() {
            let _ = Self::shared(cx);
        }
        Rc::clone(&cx.global::<MusicActionSlicesGlobal>().0)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let slices = Rc::new(MusicActionSlices {
            list_tracks_sai: SaiTypedSlice::new(cx),
            list_artist_tracks_sai: SaiTypedSlice::new(cx),
            get_track_sai: SaiTypedSlice::new(cx),
            get_liked_sai: SaiTypedSlice::new(cx),
            get_recent_sai: SaiTypedSlice::new(cx),
            list_artists_sai: SaiTypedSlice::new(cx),
            get_artist_sai: SaiTypedSlice::new(cx),
            now_playing_artist_sai: SaiTypedSlice::new(cx),
            toggle_track_like_sai: SaiTypedSlice::new(cx),
            toggle_artist_follow_sai: SaiTypedSlice::new(cx),
            record_play_sai: SaiTypedSlice::new(cx),
            get_playlists_mine_sai: SaiTypedSlice::new(cx),
            list_local_playlist_destinations_sai: SaiTypedSlice::new(cx),
            get_playlist_sai: SaiTypedSlice::new(cx),
            create_playlist_sai: SaiTypedSlice::new(cx),
            update_playlist_sai: SaiTypedSlice::new(cx),
            delete_playlist_sai: SaiTypedSlice::new(cx),
            add_track_to_playlist_sai: SaiTypedSlice::new(cx),
            remove_track_from_playlist_sai: SaiTypedSlice::new(cx),
            move_playlist_track_sai: SaiTypedSlice::new(cx),
            create_local_track_in_playlist_sai: SaiTypedSlice::new(cx),
            message_mp3_create_local_track_sai: SaiTypedSlice::new(cx),
            search_sai: SaiTypedSlice::new(cx),
            search_artists_sai: SaiTypedSlice::new(cx),
            get_artist_status_sai: SaiTypedSlice::new(cx),
            request_artist_sai: SaiTypedSlice::new(cx),
            get_artist_analytics_sai: SaiTypedSlice::new(cx),
            get_moderation_queue_metrics_sai: SaiTypedSlice::new(cx),
            list_my_tracks_sai: SaiTypedSlice::new(cx),
            list_my_local_tracks_sai: SaiTypedSlice::new(cx),
            delete_local_tracks_sai: SaiTypedSlice::new(cx),
            create_track_sai: SaiTypedSlice::new(cx),
            update_track_sai: SaiTypedSlice::new(cx),
            delete_track_sai: SaiTypedSlice::new(cx),
            list_my_albums_sai: SaiTypedSlice::new(cx),
            get_album_sai: SaiTypedSlice::new(cx),
            request_album_sai: SaiTypedSlice::new(cx),
            get_my_album_requests_sai: SaiTypedSlice::new(cx),
            publish_track_sai: SaiTypedSlice::new(cx),
            publish_album_sai: SaiTypedSlice::new(cx),
            list_artist_requests_sai: SaiTypedSlice::new(cx),
            get_artist_request_sai: SaiTypedSlice::new(cx),
            review_artist_request_sai: SaiTypedSlice::new(cx),
            list_track_requests_sai: SaiTypedSlice::new(cx),
            get_track_request_sai: SaiTypedSlice::new(cx),
            review_track_request_sai: SaiTypedSlice::new(cx),
            get_my_artist_request_sai: SaiTypedSlice::new(cx),
            list_album_requests_sai: SaiTypedSlice::new(cx),
            get_album_request_sai: SaiTypedSlice::new(cx),
            review_album_request_sai: SaiTypedSlice::new(cx),
            get_playlist_settings_sai: SaiTypedSlice::new(cx),
            get_playlist_invite_link_sai: SaiTypedSlice::new(cx),
            get_playlist_invite_preview_sai: SaiTypedSlice::new(cx),
            request_playlist_join_sai: SaiTypedSlice::new(cx),
            get_playlist_join_requests_sai: SaiTypedSlice::new(cx),
            review_playlist_join_request_sai: SaiTypedSlice::new(cx),
            get_playlist_members_sai: SaiTypedSlice::new(cx),
            update_playlist_member_permissions_sai: SaiTypedSlice::new(cx),
            remove_playlist_member_sai: SaiTypedSlice::new(cx),
            leave_playlist_sai: SaiTypedSlice::new(cx),
            get_playlist_listening_snapshot_sai: SaiTypedSlice::new(cx),
            upsert_playlist_listening_presence_sai: SaiTypedSlice::new(cx),
            stop_playlist_listening_presence_sai: SaiTypedSlice::new(cx),
            join_playlist_listening_session_sai: SaiTypedSlice::new(cx),
            get_playlist_listening_session_sai: SaiTypedSlice::new(cx),
            update_playlist_listening_session_state_sai: SaiTypedSlice::new(cx),
            request_playlist_listening_permission_sai: SaiTypedSlice::new(cx),
            update_playlist_listening_participant_permissions_sai: SaiTypedSlice::new(cx),
            leave_playlist_listening_session_sai: SaiTypedSlice::new(cx),
            end_playlist_listening_session_sai: SaiTypedSlice::new(cx),
        });
        cx.set_global(MusicActionSlicesGlobal(Rc::clone(&slices)));
        Self {
            slices,
            track_requests_list_status: music_service::TRACK_REQUEST_STATUS_PENDING.to_owned(),
        }
    }

    fn has_wire_user(cx: &mut App) -> bool {
        AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .is_some_and(|user_id| !user_id.is_empty())
    }

    fn log_network(text: String, cx: &mut App) {
        Sai::log("Music", move || text, SaiLogKind::Network, cx);
    }

    fn decoded_payload<T: DeserializeOwned + 'static>(payload: Option<Box<dyn Any>>) -> Option<T> {
        match payload?.downcast::<T>() {
            Ok(decoded) => Some(*decoded),
            Err(payload) => SaiJSONCodec::decode::<T>(payload.downcast_ref::<Value>()),
        }
    }

    fn log_only_on_error(label: &'static str) -> SaiWsOnError {
        Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("{label} error: {error}"), cx);
        })
    }

    fn mutation_options(method: &str, allow_concurrent_while_pending: bool) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    pub fn load_sound_track_action(&mut self, track_id: String, force: bool, cx: &mut Context<Self>) -> Task<()> {
        let domain = RiseMusicActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| match domain.await {
            Ok(domain) => domain.load_sound_track(track_id, force).await,
            Err(_) => {
                cx.update(|cx| {
                    Sai::log("Music", || "Sound domain unavailable", SaiLogKind::Network, cx);
                });
            }
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn list_tracks_action(
        &mut self,
        into: Option<Rc<SaiTypedSlice<WireTracksListDTO>>>,
        genre: Option<&str>,
        artist_id: Option<&str>,
        album_id: Option<&str>,
        cursor: Option<&str>,
        reflect_pending_in_ui: bool,
        reload_if_cached: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::ListTracks {
            genre: genre.map(str::to_owned),
            artist_id: artist_id.map(str::to_owned),
            album_id: album_id.map(str::to_owned),
            cursor: cursor.map(str::to_owned),
            ..rise_request::music::ListTracks::default()
        };

        let cache_key = MusicServicesStore::tracks_cache_key(genre, artist_id, album_id);
        let options = MusicServicesStore::shared(cx).read(cx).paginated_read_options(
            rise_wire_methods::music::LIST_TRACKS,
            &cache_key,
            cursor,
            reload_if_cached,
            reflect_pending_in_ui,
            key::TRACKS,
            key::ID,
        );

        let slice = into.unwrap_or_else(|| Rc::clone(&self.list_tracks_sai));
        SaiWs::request_slice(
            &slice,
            body,
            options,
            None,
            Some(Self::log_only_on_error("listTracks")),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_track_action(
        &mut self,
        track_id: &str,
        reflect_pending_in_ui: bool,
        require_fresh_server: bool,
        on_success: Option<Rc<dyn Fn(Option<RiseStoredMusicTrack>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        on_failure: Option<Rc<dyn Fn(&anyhow::Error, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetTrack { track_id: Some(track_id.to_owned()) };
        let cache_key = SaiCache::new(["getTrack", track_id]).key;

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: <rise_request::music::GetTrack as RiseRequestBody>::ACTION.to_owned(),
            cache_key: Some(cache_key.clone()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: !cache_key.is_empty(),
            persist_cache_to_disk: !cache_key.is_empty(),
            reload_when_only_disk_cached: require_fresh_server,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            invoke_success_for_cached_response: !require_fresh_server,
            force_bypass_cache_read: require_fresh_server,
            ..SaiWsRequestOptionsInit::default()
        });

        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response = Self::decoded_payload::<WireSingleTrackDTO>(payload);
            if let Some(on_success) = &on_success {
                on_success(
                    response.and_then(|response| response.track.as_ref().map(RiseStoredMusicTrack::from_canon)),
                    cx,
                );
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("getTrack error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
            if let Some(on_failure) = &on_failure {
                on_failure(error, cx);
            }
        });
        SaiWs::request_slice(&self.get_track_sai, body, options, Some(success), Some(failure), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_local_track_action(
        &mut self,
        source: RiseStoredMusicTrack,
        request_id: &str,
        title: &str,
        artist_name: &str,
        local_cover_file: Option<SaiWsOptimisticLocalFile>,
        on_success: Option<Box<dyn FnOnce(RiseStoredMusicTrack, &mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let cover = local_cover_file.map(|file| RiseMusicLocalCover {
            url: file.url,
            filename: file.filename,
            content_type: file.content_type,
        });
        let request_id = request_id.to_owned();
        let title = title.to_owned();
        let artist_name = artist_name.to_owned();
        let domain = RiseMusicActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => domain.update_local_track(source, request_id, title, artist_name, cover).await,
                Err(error) => Err(error),
            };
            cx.update(|cx| match result {
                Ok(track) => {
                    if let Some(on_success) = on_success {
                        on_success(track, cx);
                    }
                }
                Err(error) if error.downcast_ref::<CancellationError>().is_some() => {
                    if let Some(on_error) = on_error {
                        on_error(tr("music_message_import_account_changed"), cx);
                    }
                }
                Err(error) => {
                    Self::log_network(format!("updateLocalTrack error: {error}"), cx);
                    if let Some(on_error) = on_error {
                        on_error(CanonErrors::refusal_text(&error), cx);
                    }
                }
            });
        })
        .detach();
    }

    pub fn get_liked_action(
        &mut self,
        cursor: Option<&str>,
        reflect_pending_in_ui: bool,
        reload_if_cached: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetLiked {
            cursor: cursor.map(str::to_owned),
            ..rise_request::music::GetLiked::default()
        };

        let cache_key = MusicServicesStore::liked_cache_key();
        let options = MusicServicesStore::shared(cx).read(cx).paginated_read_options(
            rise_wire_methods::music::GET_LIKED,
            &cache_key,
            cursor,
            reload_if_cached,
            reflect_pending_in_ui,
            key::TRACKS,
            key::ID,
        );

        SaiWs::request_slice(
            &self.get_liked_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getLiked")),
            cx,
        );
    }

    pub fn get_recent_action(&mut self, reflect_pending_in_ui: bool, reload_if_cached: bool, cx: &mut Context<Self>) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetRecent::default();
        let cache_key = SaiCache::new(["getRecent"]).key;

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: rise_wire_methods::music::GET_RECENT.to_owned(),
            cache_key: Some(cache_key.clone()),
            reload_if_cached,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: !cache_key.is_empty(),
            persist_cache_to_disk: !cache_key.is_empty(),
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(key::TRACKS.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            ..SaiWsRequestOptionsInit::default()
        });

        SaiWs::request_slice(
            &self.get_recent_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getRecent")),
            cx,
        );
    }

    pub fn list_artists_action(
        &mut self,
        cursor: Option<&str>,
        reflect_pending_in_ui: bool,
        reload_if_cached: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::ListArtists {
            cursor: cursor.map(str::to_owned),
            ..rise_request::music::ListArtists::default()
        };

        let cache_key = MusicServicesStore::artists_cache_key();
        let options = MusicServicesStore::shared(cx).read(cx).paginated_read_options(
            rise_wire_methods::music::LIST_ARTISTS,
            &cache_key,
            cursor,
            reload_if_cached,
            reflect_pending_in_ui,
            key::ARTISTS,
            key::ID,
        );

        SaiWs::request_slice(
            &self.list_artists_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("listArtists")),
            cx,
        );
    }

    pub fn get_artist_action(&mut self, artist_id: &str, reflect_pending_in_ui: bool, cx: &mut Context<Self>) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetArtist { artist_id: Some(artist_id.to_owned()) };
        let cache_key = SaiCache::new(["getArtist", artist_id]).key;

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: rise_wire_methods::music::GET_ARTIST.to_owned(),
            cache_key: Some(cache_key.clone()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: !cache_key.is_empty(),
            persist_cache_to_disk: !cache_key.is_empty(),
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            ..SaiWsRequestOptionsInit::default()
        });

        SaiWs::request_slice(
            &self.get_artist_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getArtist")),
            cx,
        );
    }

    pub fn get_now_playing_artist_action(&mut self, artist_id: &str, cx: &mut Context<Self>) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetArtist { artist_id: Some(artist_id.to_owned()) };
        let cache_key = SaiCache::new(["getArtist", artist_id]).key;

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: rise_wire_methods::music::GET_ARTIST.to_owned(),
            cache_key: Some(cache_key.clone()),
            reload_if_cached: true,
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: !cache_key.is_empty(),
            persist_cache_to_disk: !cache_key.is_empty(),
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            ..SaiWsRequestOptionsInit::default()
        });

        SaiWs::request_slice(
            &self.now_playing_artist_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getArtist")),
            cx,
        );
    }

    pub fn toggle_track_like_action(&mut self, track_id: &str, liked: bool, cx: &mut Context<Self>) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::ToggleTrackLike {
            liked: Some(liked),
            track_id: Some(track_id.to_owned()),
        };

        let mut options = Self::mutation_options(rise_wire_methods::music::TOGGLE_TRACK_LIKE, true);
        options.track_until_acknowledged = true;

        let slice = Rc::clone(&self.toggle_track_like_sai);
        let success_track_id = track_id.to_owned();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let dto = slice.data();
            let track_id = success_track_id.clone();
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.apply_toggle_like_server_result(
                    &track_id,
                    dto.as_ref().map(|dto| dto.is_liked),
                    dto.as_ref().map(|dto| dto.like_count),
                    cx,
                );
            });
        });
        let error_track_id = track_id.to_owned();
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("toggleTrackLike error: {error}"), cx);
            let replay_queued =
                SaiWsResponseDebtCoordinator::is_feature_enabled() && CanonErrors::is_connection_loss(error);
            let track_id = error_track_id.clone();
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                if !replay_queued && services.is_track_liked(&track_id, cx) == liked {
                    services.apply_optimistic_toggle_like(&track_id, cx);
                }
            });
        });
        SaiWs::request_slice(&self.toggle_track_like_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn toggle_artist_follow_action(&mut self, artist_id: &str, cx: &mut Context<Self>) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::ToggleArtistFollow { artist_id: Some(artist_id.to_owned()) };
        let options = Self::mutation_options(rise_wire_methods::music::TOGGLE_ARTIST_FOLLOW, true);

        let slice = Rc::clone(&self.toggle_artist_follow_sai);
        let success_artist_id = artist_id.to_owned();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let is_following = slice.data().map(|dto| dto.is_following);
            let artist_id = success_artist_id.clone();
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.apply_toggle_follow_server_result(&artist_id, is_following, cx);
            });
        });
        let error_artist_id = artist_id.to_owned();
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let artist_id = error_artist_id.clone();
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.apply_optimistic_toggle_artist_follow(&artist_id, cx);
            });
            Self::log_network(format!("toggleArtistFollow error: {error}"), cx);
        });
        SaiWs::request_slice(&self.toggle_artist_follow_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn record_play_action(&mut self, track_id: &str, duration_played_ms: Option<i64>, cx: &mut Context<Self>) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::RecordPlay {
            duration_played_ms,
            track_id: Some(track_id.to_owned()),
        };
        let options = Self::mutation_options(rise_wire_methods::music::RECORD_PLAY, true);
        let ignore: SaiWsOnError = Rc::new(|_: &anyhow::Error, _: &mut App| {});
        SaiWs::request_slice(&self.record_play_sai, body, options, None, Some(ignore), cx);
    }

    pub fn get_playlists_mine_action(
        &mut self,
        reflect_pending_in_ui: bool,
        reload_if_cached: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetPlaylistsMine::default();
        let cache_key = SaiCache::new(["getPlaylistsMine"]).key;

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: rise_wire_methods::music::GET_PLAYLISTS_MINE.to_owned(),
            cache_key: Some(cache_key.clone()),
            reload_if_cached,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: !cache_key.is_empty(),
            persist_cache_to_disk: !cache_key.is_empty(),
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(key::PLAYLISTS.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            ..SaiWsRequestOptionsInit::default()
        });

        let success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, cx: &mut App| {
            cx.defer(|cx| {
                MusicPlayerState::shared(cx).update(cx, |player, cx| {
                    player.refresh_playlist_listening_presence_if_audience_changed(cx);
                });
            });
        });
        SaiWs::request_slice(
            &self.get_playlists_mine_sai,
            body,
            options,
            Some(success),
            Some(Self::log_only_on_error("getPlaylistsMine")),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_playlist_action(
        &mut self,
        playlist_id: &str,
        reflect_pending_in_ui: bool,
        require_fresh_server: bool,
        on_success: Option<Rc<dyn Fn(Option<RiseStoredMusicPlaylist>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        on_failure: Option<Rc<dyn Fn(&anyhow::Error, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetPlaylist { playlist_id: Some(playlist_id.to_owned()) };
        let cache_key = SaiCache::new(["getPlaylist", playlist_id]).key;

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: rise_wire_methods::music::GET_PLAYLIST.to_owned(),
            cache_key: Some(cache_key.clone()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: !cache_key.is_empty(),
            persist_cache_to_disk: !cache_key.is_empty(),
            reload_when_only_disk_cached: require_fresh_server,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            invoke_success_for_cached_response: !require_fresh_server,
            force_bypass_cache_read: require_fresh_server,
            ..SaiWsRequestOptionsInit::default()
        });

        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response = Self::decoded_payload::<RiseStoredMusicPlaylistDetail>(payload);
            if let Some(on_success) = &on_success {
                on_success(response.and_then(|response| response.playlist), cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("getPlaylist error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
            if let Some(on_failure) = &on_failure {
                on_failure(error, cx);
            }
        });
        SaiWs::request_slice(&self.get_playlist_sai, body, options, Some(success), Some(failure), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_playlist_action(
        &mut self,
        title: &str,
        description: Option<&str>,
        visibility: &str,
        local_cover_file: Option<SaiWsOptimisticLocalFile>,
        on_success: Option<Rc<dyn Fn(Option<RiseStoredMusicPlaylist>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let trimmed_title = trimming_whitespaces(title).to_owned();
        if trimmed_title.is_empty() {
            return;
        }

        let body = rise_request::music::CreatePlaylist {
            title: Some(trimmed_title.clone()),
            visibility: Some(visibility.to_owned()),
            description: description.map(str::to_owned),
            ..rise_request::music::CreatePlaylist::default()
        };

        let services = MusicServicesStore::shared(cx);
        let draft = services
            .read(cx)
            .draft_playlist_for_mutation(&trimmed_title, description, visibility, None);

        let has_local_cover = local_cover_file.is_some();
        let mut optimistic = SaiWsOptimisticUpdate::default();
        if let Some(local_cover_file) = local_cover_file {
            optimistic.enabled = true;
            let files_on_error = on_error.clone();
            let files_draft = draft.clone();
            let files_slice = Rc::clone(&self.create_playlist_sai);
            optimistic.files = Some(services.update(cx, |services, cx| {
                services.make_playlist_cover_optimistic_files(local_cover_file, files_draft, files_slice, files_on_error, cx)
            }));
            let pending_slice = Rc::clone(&self.create_playlist_sai);
            services.update(cx, |services, _| {
                services.patch_create_playlist_pending_cover(&pending_slice, 0.0, true, draft.clone());
            });
        }

        let mut options = Self::mutation_options(rise_wire_methods::music::CREATE_PLAYLIST, false);
        options.reflect_pending_in_ui = !has_local_cover;
        options.optimistic_update = has_local_cover.then_some(optimistic);

        let success_slice = Rc::clone(&self.create_playlist_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let playlist = success_slice.data().and_then(|data| data.playlist);
            if let Some(playlist) = playlist.clone() {
                MusicServicesStore::shared(cx).update(cx, |services, cx| services.upsert_playlist_mine(&playlist, cx));
            }
            if let Some(on_success) = &on_success {
                on_success(playlist, cx);
            }
        });
        let error_slice = Rc::clone(&self.create_playlist_sai);
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("createPlaylist error: {error}"), cx);
            error_slice.reset_to_idle();
            if CanonErrors::failure(error).is_some_and(|failure| failure.subscription_lifts_limit())
                && PremiumUpsellStore::shared(cx).read(cx).prompt().is_some()
            {
                return;
            }
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.create_playlist_sai, body, options, Some(success), Some(failure), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_playlist_action(
        &mut self,
        playlist_id: &str,
        title: Option<&str>,
        description: Option<&str>,
        visibility: Option<&str>,
        new_tracks_on_top: Option<bool>,
        local_cover_file: Option<SaiWsOptimisticLocalFile>,
        existing_playlist: Option<RiseStoredMusicPlaylist>,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::UpdatePlaylist {
            playlist_id: Some(playlist_id.to_owned()),
            title: title.map(str::to_owned),
            description: description.map(str::to_owned),
            visibility: visibility.map(str::to_owned),
            new_tracks_on_top,
            ..rise_request::music::UpdatePlaylist::default()
        };

        let draft_title = title
            .map(str::to_owned)
            .or_else(|| existing_playlist.as_ref().and_then(|playlist| playlist.title.clone()))
            .unwrap_or_default();
        let draft_description = description
            .map(str::to_owned)
            .or_else(|| existing_playlist.as_ref().and_then(|playlist| playlist.description.clone()));
        let draft_visibility = visibility
            .map(str::to_owned)
            .or_else(|| existing_playlist.as_ref().and_then(|playlist| playlist.visibility.clone()))
            .unwrap_or_else(|| PlaylistVisibility::PrivatePlaylist.wire().to_owned());

        let services = MusicServicesStore::shared(cx);
        let draft = services.read(cx).draft_playlist_for_mutation(
            &draft_title,
            draft_description.as_deref(),
            &draft_visibility,
            existing_playlist.as_ref(),
        );

        let has_local_cover = local_cover_file.is_some();
        let mut optimistic = SaiWsOptimisticUpdate::default();
        if let Some(local_cover_file) = local_cover_file {
            optimistic.enabled = true;
            let files_on_error = on_error.clone();
            let files_draft = draft.clone();
            let files_slice = Rc::clone(&self.update_playlist_sai);
            optimistic.files = Some(services.update(cx, |services, cx| {
                services.make_playlist_cover_optimistic_files(local_cover_file, files_draft, files_slice, files_on_error, cx)
            }));
            let pending_slice = Rc::clone(&self.update_playlist_sai);
            services.update(cx, |services, _| {
                services.patch_update_playlist_pending_cover(&pending_slice, 0.0, true, draft.clone());
            });
        }

        let mut options = Self::mutation_options(rise_wire_methods::music::UPDATE_PLAYLIST, false);
        options.reflect_pending_in_ui = !has_local_cover;
        options.optimistic_update = has_local_cover.then_some(optimistic);

        let success_slice = Rc::clone(&self.update_playlist_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(playlist) = success_slice.data().and_then(|data| data.playlist) {
                MusicServicesStore::shared(cx).update(cx, |services, cx| services.patch_playlist_detail(&playlist, cx));
            }
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let error_slice = Rc::clone(&self.update_playlist_sai);
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("updatePlaylist error: {error}"), cx);
            error_slice.reset_to_idle();
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.update_playlist_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn delete_playlist_action(
        &mut self,
        playlist_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let current_detail = self.get_playlist_sai.data().and_then(|data| data.playlist);
        let playlist_to_restore = self
            .get_playlists_mine_sai
            .data()
            .and_then(|data| data.playlists)
            .and_then(|playlists| {
                playlists
                    .into_iter()
                    .find(|playlist| playlist.id.as_deref().unwrap_or_default() == playlist_id)
            })
            .or_else(|| current_detail.clone());
        let mine_key = SaiCache::new(["getPlaylistsMine"]).key;

        let optimistic = SaiWsOptimisticUpdate {
            enabled: true,
            delete_mode: true,
            delete_id: Some(playlist_id.to_owned()),
            target_cache_key: Some(mine_key.clone()),
            ..SaiWsOptimisticUpdate::default()
        };

        if current_detail
            .as_ref()
            .and_then(|playlist| playlist.id.as_deref())
            .unwrap_or_default()
            == playlist_id
        {
            let mut fields = Map::new();
            fields.insert(key::PLAYLIST.to_owned(), Value::Null);
            let _ = SaiUpdater::patch_root(
                &SaiCache::new(["getPlaylist", playlist_id]).key,
                &fields,
                SaiUpdaterWriteScope::All,
                cx,
            );
        }

        MusicServicesStore::shared(cx).update(cx, |services, cx| {
            services.remove_list_row(&mine_key, key::PLAYLISTS, playlist_id, cx);
        });

        let body = rise_request::music::DeletePlaylist { playlist_id: Some(playlist_id.to_owned()) };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: rise_wire_methods::music::DELETE_PLAYLIST.to_owned(),
            cache_key: Some(mine_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: false,
            path_to_array: Some(key::PLAYLISTS.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            optimistic_update: Some(optimistic),
            ..SaiWsRequestOptionsInit::default()
        });

        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(playlist) = playlist_to_restore.clone() {
                MusicServicesStore::shared(cx)
                    .update(cx, |services, cx| services.revert_delete_playlist_optimistic(&playlist, cx));
            }
            Self::log_network(format!("deletePlaylist error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.delete_playlist_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn add_track_to_playlist_action(
        &mut self,
        playlist_id: &str,
        track_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let previous = self.get_playlist_sai.data().and_then(|data| data.playlist);
        MusicServicesStore::shared(cx).update(cx, |services, cx| {
            let _ = services.apply_add_track_to_playlist_optimistic(playlist_id, track_id, cx);
        });

        let body = rise_request::music::AddTrackToPlaylist {
            playlist_id: Some(playlist_id.to_owned()),
            track_id: Some(track_id.to_owned()),
        };
        let options = Self::mutation_options(rise_wire_methods::music::ADD_TRACK_TO_PLAYLIST, true);

        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(previous) = previous.clone() {
                MusicServicesStore::shared(cx)
                    .update(cx, |services, cx| services.revert_add_track_to_playlist_optimistic(&previous, cx));
            }
            Self::log_network(format!("addTrackToPlaylist error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.add_track_to_playlist_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn remove_track_from_playlist_action(
        &mut self,
        playlist_id: &str,
        track_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let previous = self.get_playlist_sai.data().and_then(|data| data.playlist);
        MusicServicesStore::shared(cx).update(cx, |services, cx| {
            let _ = services.apply_remove_track_from_playlist_optimistic(playlist_id, track_id, cx);
        });

        let body = rise_request::music::RemoveTrackFromPlaylist {
            playlist_id: Some(playlist_id.to_owned()),
            track_id: Some(track_id.to_owned()),
        };
        let options = Self::mutation_options(rise_wire_methods::music::REMOVE_TRACK_FROM_PLAYLIST, true);

        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(previous) = previous.clone() {
                MusicServicesStore::shared(cx)
                    .update(cx, |services, cx| services.revert_remove_track_from_playlist_optimistic(&previous, cx));
            }
            Self::log_network(format!("removeTrackFromPlaylist error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.remove_track_from_playlist_sai, body, options, Some(success), Some(failure), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn move_playlist_track_action(
        &mut self,
        playlist_id: &str,
        track_id: &str,
        after_track_id: Option<&str>,
        expected_revision: Option<i64>,
        on_success: Option<Rc<dyn Fn(Option<i64>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::MovePlaylistTrack {
            after_track_id: after_track_id.map(str::to_owned),
            expected_revision,
            playlist_id: Some(playlist_id.to_owned()),
            track_id: Some(track_id.to_owned()),
        };
        let options = Self::mutation_options(rise_wire_methods::music::MOVE_PLAYLIST_TRACK, true);

        let success_slice = Rc::clone(&self.move_playlist_track_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(on_success) = &on_success {
                on_success(success_slice.data().map(|data| data.revision), cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("movePlaylistTrack error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.move_playlist_track_sai, body, options, Some(success), Some(failure), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_local_track_in_playlist_action(
        &mut self,
        request_id: &str,
        temp_id: &str,
        playlist_id: &str,
        title: &str,
        description: Option<&str>,
        artist_name: &str,
        duration_ms: i64,
        local_audio_file: SaiWsOptimisticLocalFile,
        local_cover_file: Option<SaiWsOptimisticLocalFile>,
        optimistic_track: Option<RiseStoredMusicTrack>,
        destination: Option<MusicLocalPlaylistDestination>,
        remove_optimistic_on_error: bool,
        request_slice: Option<Rc<SaiTypedSlice<RiseStoredMusicLocalTrackUpload>>>,
        on_success: Option<Box<dyn FnOnce(&mut App)>>,
        on_created: Option<Box<dyn FnOnce(RiseStoredMusicTrack, &mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let request_id = request_id.to_owned();
        let temp_id = temp_id.to_owned();
        let playlist_id = playlist_id.to_owned();
        let title = title.to_owned();
        let description = description.map(str::to_owned);
        let artist_name = artist_name.to_owned();
        let default_slice = Rc::clone(&self.create_local_track_in_playlist_sai);
        cx.defer(move |cx| {
            Self::create_local_track_in_playlist_now(
                default_slice,
                &request_id,
                &temp_id,
                &playlist_id,
                &title,
                description.as_deref(),
                &artist_name,
                duration_ms,
                local_audio_file,
                local_cover_file,
                optimistic_track,
                destination,
                remove_optimistic_on_error,
                request_slice,
                on_success,
                on_created,
                on_error,
                cx,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn create_local_track_in_playlist_now(
        default_slice: Rc<SaiTypedSlice<RiseStoredMusicLocalTrackUpload>>,
        request_id: &str,
        temp_id: &str,
        playlist_id: &str,
        title: &str,
        description: Option<&str>,
        artist_name: &str,
        duration_ms: i64,
        local_audio_file: SaiWsOptimisticLocalFile,
        local_cover_file: Option<SaiWsOptimisticLocalFile>,
        optimistic_track: Option<RiseStoredMusicTrack>,
        destination: Option<MusicLocalPlaylistDestination>,
        remove_optimistic_on_error: bool,
        request_slice: Option<Rc<SaiTypedSlice<RiseStoredMusicLocalTrackUpload>>>,
        on_success: Option<Box<dyn FnOnce(&mut App)>>,
        on_created: Option<Box<dyn FnOnce(RiseStoredMusicTrack, &mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut App,
    ) {
        let user_id = match AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire() {
            Some(user_id) if !user_id.is_empty() => user_id,
            _ => {
                if let Some(on_error) = on_error {
                    on_error(tr("music_message_import_account_changed"), cx);
                }
                return;
            }
        };
        let canonical_request_id = request_id.to_lowercase();
        let target_slice = request_slice.unwrap_or(default_slice);

        let services = MusicServicesStore::shared(cx);
        let draft = match optimistic_track {
            Some(track) => track,
            None => services.read(cx).make_local_track_draft(
                temp_id,
                title,
                description,
                artist_name,
                duration_ms,
                &user_id,
                &local_audio_file.url,
                &local_cover_file
                    .as_ref()
                    .map(|file| file.url.absolute_string().to_owned())
                    .unwrap_or_default(),
                None,
                None,
                None,
                Some(MusicLocalTrackTransferPhase::Uploading),
                0,
                local_audio_file.byte_count.unwrap_or(0).max(0)
                    + local_cover_file
                        .as_ref()
                        .and_then(|file| file.byte_count)
                        .unwrap_or(0)
                        .max(0),
                None,
            ),
        };
        services.update(cx, |services, cx| {
            services.insert_local_track_optimistic(playlist_id, draft.clone(), destination.as_ref(), cx);
            services.patch_local_track_upload_pending(
                &draft,
                temp_id,
                0.0,
                true,
                Some(Rc::clone(&target_slice)),
                cx,
            );
        });

        let mut body = rise_request::music::CreateLocalTrackInPlaylist {
            artist_name: Some(artist_name.to_owned()),
            playlist_id: Some(playlist_id.to_owned()),
            request_id: Some(canonical_request_id.clone()),
            title: Some(title.to_owned()),
            ..rise_request::music::CreateLocalTrackInPlaylist::default()
        };
        if let Some(description) = description.map(trimming_whitespaces_and_newlines)
            && !description.is_empty()
        {
            body.description = Some(description.to_owned());
        }

        let resource_id = format!("{user_id}:{canonical_request_id}");
        let files_slice = Rc::clone(&target_slice);
        let files_draft = draft.clone();
        let files = services.update(cx, |services, cx| {
            services.make_local_track_optimistic_files(
                playlist_id,
                local_audio_file,
                local_cover_file,
                files_draft,
                temp_id,
                &resource_id,
                Some(files_slice),
                cx,
            )
        });
        let optimistic = SaiWsOptimisticUpdate {
            enabled: true,
            files: Some(files),
            ..SaiWsOptimisticUpdate::default()
        };

        let mut options = Self::mutation_options(rise_wire_methods::music::CREATE_LOCAL_TRACK_IN_PLAYLIST, false);
        options.optimistic_update = Some(optimistic);

        let on_error = Rc::new(RefCell::new(on_error));
        let on_success = RefCell::new(on_success);
        let on_created = RefCell::new(on_created);

        let success_draft = draft.clone();
        let success_slice = Rc::clone(&target_slice);
        let success_playlist_id = playlist_id.to_owned();
        let success_temp_id = temp_id.to_owned();
        let success_on_error = Rc::clone(&on_error);
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let services = MusicServicesStore::shared(cx);
            let Some(track) = services.read(cx).local_track_create_response(payload) else {
                services.update(cx, |services, cx| {
                    services.patch_local_track_upload_pending(
                        &success_draft,
                        &success_temp_id,
                        0.0,
                        false,
                        Some(Rc::clone(&success_slice)),
                        cx,
                    );
                    if remove_optimistic_on_error {
                        services.remove_local_track_optimistic(&success_playlist_id, &success_temp_id, cx);
                    }
                });
                let on_error = success_on_error.borrow_mut().take();
                if let Some(on_error) = on_error {
                    on_error(tr("music_local_upload_invalid_response"), cx);
                }
                return;
            };
            services.update(cx, |services, cx| {
                services.replace_local_track_optimistic(&success_playlist_id, &success_temp_id, track.clone(), cx);
            });
            let on_success = on_success.borrow_mut().take();
            if let Some(on_success) = on_success {
                on_success(cx);
            }
            let on_created = on_created.borrow_mut().take();
            if let Some(on_created) = on_created {
                on_created(track, cx);
            }
        });

        let error_slice = Rc::clone(&target_slice);
        let error_playlist_id = playlist_id.to_owned();
        let error_temp_id = temp_id.to_owned();
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.patch_local_track_upload_pending(
                    &draft,
                    &error_temp_id,
                    0.0,
                    false,
                    Some(Rc::clone(&error_slice)),
                    cx,
                );
                if remove_optimistic_on_error {
                    services.remove_local_track_optimistic(&error_playlist_id, &error_temp_id, cx);
                }
            });
            Self::log_network(format!("createLocalTrackInPlaylist error: {error}"), cx);
            let on_error = on_error.borrow_mut().take();
            if let Some(on_error) = on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&target_slice, body, options, Some(success), Some(failure), cx);
    }

    /// search query as sent on the wire
    /// gateway refuses q past the canon ceiling so long queries get clipped silently
    /// field text and cache key keep the full one
    pub fn search_wire_query(q: &str) -> String {
        trimming_whitespaces_and_newlines(q)
            .chars()
            .take(rise_validate::rule_max::SEARCH_QUERY as usize)
            .collect()
    }

    fn relative_id_pagination(relative_id: Option<&str>) -> SaiWsPaginationOptions {
        SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
            relative_id: relative_id.map(str::to_owned),
            response_relative_id_path: key::camel::NEXT_CURSOR.to_owned(),
            has_more_path: Some(key::camel::HAS_MORE.to_owned()),
            duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
            ..SaiWsPaginationRelativeIdArgs::default()
        })
    }

    pub fn search_action(
        &mut self,
        q: &str,
        search_type: Option<&str>,
        relative_id: Option<&str>,
        reflect_pending_in_ui: bool,
        on_result: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }
        if trimming_whitespaces(q).is_empty() {
            return;
        }

        let mut body = rise_request::music::Search {
            limit: Some(20),
            q: Some(Self::search_wire_query(q)),
            r#type: search_type.map(str::to_owned),
            ..rise_request::music::Search::default()
        };
        if let Some(relative_id) = relative_id
            && !relative_id.is_empty()
        {
            body.relative_id = Some(relative_id.to_owned());
        }

        let cache_key = MusicServicesStore::search_cache_key(q, search_type);

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: rise_wire_methods::music::SEARCH.to_owned(),
            cache_key: Some(cache_key.clone()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: !cache_key.is_empty(),
            persist_cache_to_disk: !cache_key.is_empty(),
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(key::TRACKS.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            pagination: Some(Self::relative_id_pagination(relative_id)),
            ..SaiWsRequestOptionsInit::default()
        });

        let query = q.to_owned();
        let success_on_result = on_result.clone();
        let success_query = query.clone();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(on_result) = &success_on_result {
                on_result(success_query.clone(), cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("search error: {error}"), cx);
            if let Some(on_result) = &on_result {
                on_result(query.clone(), cx);
            }
        });
        SaiWs::request_slice(&self.search_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn search_artists_action(
        &mut self,
        q: &str,
        relative_id: Option<&str>,
        limit: i64,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }
        if trimming_whitespaces(q).is_empty() {
            return;
        }

        let mut body = rise_request::music::Search {
            limit: Some(limit),
            q: Some(Self::search_wire_query(q)),
            r#type: Some(key::ARTIST.to_owned()),
            ..rise_request::music::Search::default()
        };
        if let Some(relative_id) = relative_id
            && !relative_id.is_empty()
        {
            body.relative_id = Some(relative_id.to_owned());
        }

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: rise_wire_methods::music::SEARCH.to_owned(),
            cache_key: Some(MusicServicesStore::search_cache_key(
                q,
                Some(key::ARTIST),
            )),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: relative_id.is_none(),
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(key::ARTISTS.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            pagination: Some(Self::relative_id_pagination(relative_id)),
            ..SaiWsRequestOptionsInit::default()
        });

        SaiWs::request_slice(
            &self.search_artists_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("searchArtists")),
            cx,
        );
    }

    fn disk_read_options(
        method: &str,
        cache_key: &str,
        reload_if_cached: bool,
        reflect_pending_in_ui: bool,
        reload_when_only_disk_cached: bool,
    ) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: Some(cache_key.to_owned()),
            reload_if_cached,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: !cache_key.is_empty(),
            persist_cache_to_disk: !cache_key.is_empty(),
            reload_when_only_disk_cached,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    pub fn get_artist_status_action(&mut self, cx: &mut Context<Self>) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetArtistStatus {};
        let cache_key = SaiCache::new(["getArtistStatus"]).key;
        let options =
            Self::disk_read_options(rise_wire_methods::music::GET_ARTIST_STATUS, &cache_key, true, true, false);

        SaiWs::request_slice(
            &self.get_artist_status_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getArtistStatus")),
            cx,
        );
    }

    pub fn request_artist_action(
        &mut self,
        stage_name: &str,
        genre: &str,
        bio: &str,
        instagram: Option<&str>,
        vk: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let mut body = rise_request::music::RequestArtist {
            bio: Some(bio.to_owned()),
            genre: Some(genre.to_owned()),
            stage_name: Some(stage_name.to_owned()),
            ..rise_request::music::RequestArtist::default()
        };
        if let Some(instagram) = instagram
            && !instagram.is_empty()
        {
            body.instagram = Some(instagram.to_owned());
        }
        if let Some(vk) = vk
            && !vk.is_empty()
        {
            body.vk = Some(vk.to_owned());
        }

        let mut options = Self::mutation_options(rise_wire_methods::music::REQUEST_ARTIST, false);
        options.reflect_pending_in_ui = true;

        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            MusicServicesStore::shared(cx)
                .update(cx, |services, cx| services.sync_artist_status_from_request_response(cx));
        });
        SaiWs::request_slice(
            &self.request_artist_sai,
            body,
            options,
            Some(success),
            Some(Self::log_only_on_error("requestArtist")),
            cx,
        );
    }

    pub fn get_artist_analytics_action(
        &mut self,
        reflect_pending_in_ui: bool,
        reload_if_cached: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetArtistAnalytics::default();
        let cache_key = SaiCache::new(["getArtistAnalytics"]).key;
        let options = Self::disk_read_options(
            rise_wire_methods::music::GET_ARTIST_ANALYTICS,
            &cache_key,
            reload_if_cached,
            reflect_pending_in_ui,
            false,
        );

        SaiWs::request_slice(
            &self.get_artist_analytics_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getArtistAnalytics")),
            cx,
        );
    }

    pub fn get_moderation_queue_metrics_action(
        &mut self,
        reflect_pending_in_ui: bool,
        reload_if_cached: bool,
        cx: &mut Context<Self>,
    ) {
        if !MusicServicesStore::shared(cx).read(cx).music_is_staff(cx) {
            return;
        }
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetModerationQueueMetrics::default();
        let cache_key = SaiCache::new(["getModerationQueueMetrics"]).key;
        let options = Self::disk_read_options(
            rise_wire_methods::music::GET_MODERATION_QUEUE_METRICS,
            &cache_key,
            reload_if_cached,
            reflect_pending_in_ui,
            true,
        );

        SaiWs::request_slice(
            &self.get_moderation_queue_metrics_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getModerationQueueMetrics")),
            cx,
        );
    }

    pub fn list_my_tracks_action(
        &mut self,
        cursor: Option<&str>,
        reflect_pending_in_ui: bool,
        reload_if_cached: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::ListMyTracks {
            cursor: cursor.map(str::to_owned),
            ..rise_request::music::ListMyTracks::default()
        };

        let cache_key = MusicServicesStore::my_tracks_cache_key();
        let options = MusicServicesStore::shared(cx).read(cx).paginated_read_options(
            rise_wire_methods::music::LIST_MY_TRACKS,
            &cache_key,
            cursor,
            reload_if_cached,
            reflect_pending_in_ui,
            key::TRACKS,
            key::ID,
        );

        SaiWs::request_slice(
            &self.list_my_tracks_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("listMyTracks")),
            cx,
        );
    }

    pub fn list_my_local_tracks_action(
        &mut self,
        cursor: Option<&str>,
        reflect_pending_in_ui: bool,
        reload_if_cached: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::ListMyLocalTracks {
            limit: Some(30),
            cursor: cursor.map(str::to_owned),
        };

        let cache_key = MusicServicesStore::my_local_tracks_cache_key();
        let options = MusicServicesStore::shared(cx).read(cx).paginated_read_options(
            rise_wire_methods::music::LIST_MY_LOCAL_TRACKS,
            &cache_key,
            cursor,
            reload_if_cached,
            reflect_pending_in_ui,
            key::TRACKS,
            key::ID,
        );

        SaiWs::request_slice(
            &self.list_my_local_tracks_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("listMyLocalTracks")),
            cx,
        );
    }

    pub fn delete_local_tracks_action(
        &mut self,
        track_ids: &[String],
        request_id: &str,
        on_success: Option<Rc<dyn Fn(Vec<String>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let normalized_ids: Vec<String> = track_ids
            .iter()
            .map(|id| trimming_whitespaces_and_newlines(id))
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect();
        let normalized_request_id = trimming_whitespaces_and_newlines(request_id).to_owned();
        if normalized_ids.is_empty() || normalized_request_id.is_empty() {
            return;
        }
        let this = cx.entity();
        cx.defer(move |cx| {
            let snapshot = MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.apply_delete_local_tracks_optimistic(&normalized_ids.iter().cloned().collect(), cx)
            });
            this.update(cx, |this, cx| {
                this.request_delete_local_tracks(normalized_ids, normalized_request_id, snapshot, on_success, on_error, cx);
            });
        });
    }

    fn request_delete_local_tracks(
        &mut self,
        normalized_ids: Vec<String>,
        normalized_request_id: String,
        snapshot: Option<MusicLocalTracksDeleteSnapshot>,
        on_success: Option<Rc<dyn Fn(Vec<String>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let body = rise_request::music::DeleteLocalTracks {
            request_id: Some(normalized_request_id),
            track_ids: Some(normalized_ids.clone()),
        };
        let mut options = Self::mutation_options(rise_wire_methods::music::DELETE_LOCAL_TRACKS, false);
        options.reflect_pending_in_ui = true;

        let success_slice = Rc::clone(&self.delete_local_tracks_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let deleted_ids = success_slice
                .data()
                .map(|data| data.deleted_track_ids)
                .unwrap_or_else(|| normalized_ids.clone());
            let deleted_set: HashSet<String> = deleted_ids.iter().cloned().collect();
            MusicServicesStore::shared(cx)
                .update(cx, |services, cx| services.finalize_delete_local_tracks(&deleted_set, cx));
            if let Some(on_success) = &on_success {
                on_success(deleted_ids, cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(snapshot) = snapshot.clone() {
                MusicServicesStore::shared(cx)
                    .update(cx, |services, cx| services.revert_delete_local_tracks_optimistic(&snapshot, cx));
            }
            Self::log_network(format!("deleteLocalTracks error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.delete_local_tracks_sai, body, options, Some(success), Some(failure), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_track_action(
        &mut self,
        title: &str,
        genre: &str,
        local_cover_file: SaiWsOptimisticLocalFile,
        local_audio_file: SaiWsOptimisticLocalFile,
        album_id: Option<&str>,
        is_exclusive: bool,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let trimmed_title = trimming_whitespaces(title).to_owned();
        if trimmed_title.is_empty() {
            return;
        }

        let mut body = rise_request::music::CreateTrack {
            genre: Some(genre.to_owned()),
            title: Some(trimmed_title.clone()),
            ..rise_request::music::CreateTrack::default()
        };
        if let Some(album_id) = album_id
            && !trimming_whitespaces(album_id).is_empty()
        {
            body.album_id = Some(album_id.to_owned());
        }
        if is_exclusive {
            body.is_exclusive = Some(true);
        }

        let services = MusicServicesStore::shared(cx);
        let files_on_error = on_error.clone();
        let files_slice = Rc::clone(&self.create_track_sai);
        let files = services.update(cx, |services, cx| {
            services.make_create_track_optimistic_files(
                local_cover_file,
                local_audio_file,
                &trimmed_title,
                files_slice,
                files_on_error,
                cx,
            )
        });
        let optimistic = SaiWsOptimisticUpdate {
            enabled: true,
            files: Some(files),
            ..SaiWsOptimisticUpdate::default()
        };
        let pending_slice = Rc::clone(&self.create_track_sai);
        services.update(cx, |services, _| {
            services.patch_create_track_pending_upload(&pending_slice, &trimmed_title, Some(0.0), Some(true), Some(0.0), Some(false));
        });

        let mut options = Self::mutation_options(rise_wire_methods::music::CREATE_TRACK, false);
        options.optimistic_update = Some(optimistic);

        let success_slice = Rc::clone(&self.create_track_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(track) = success_slice.data().and_then(|data| data.track) {
                MusicServicesStore::shared(cx).update(cx, |services, cx| services.insert_my_track(&track, cx));
            }
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let error_slice = Rc::clone(&self.create_track_sai);
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("createTrack error: {error}"), cx);
            error_slice.reset_to_idle();
            if RestrictionStore::shared(cx).update(cx, |restriction, cx| restriction.explains_error(error, cx)) {
                return;
            }
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.create_track_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn update_track_action(
        &mut self,
        track_id: &str,
        is_exclusive: bool,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let trimmed_id = trimming_whitespaces_and_newlines(track_id).to_owned();
        if trimmed_id.is_empty() {
            return;
        }

        let previous = self.list_my_tracks_sai.data().and_then(|data| {
            data.tracks
                .iter()
                .find(|track| track.id == trimmed_id)
                .map(ProtoTrackRecordExt::stored)
        });
        MusicServicesStore::shared(cx).update(cx, |services, cx| {
            services.apply_update_track_exclusive_optimistic(&trimmed_id, is_exclusive, cx);
        });

        let body = rise_request::music::UpdateTrack {
            is_exclusive: Some(is_exclusive),
            track_id: Some(trimmed_id),
            ..rise_request::music::UpdateTrack::default()
        };
        let options = Self::mutation_options(rise_wire_methods::music::UPDATE_TRACK, true);

        let success_slice = Rc::clone(&self.update_track_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(track) = success_slice.data().and_then(|data| data.track) {
                let track = RiseStoredMusicTrack::from_canon(&track);
                MusicServicesStore::shared(cx).update(cx, |services, cx| services.insert_my_track(&track, cx));
            }
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(previous) = previous.clone() {
                MusicServicesStore::shared(cx).update(cx, |services, cx| services.insert_my_track(&previous, cx));
            }
            Self::log_network(format!("updateTrack error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.update_track_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn delete_track_action(
        &mut self,
        track_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let track_to_restore = self.list_my_tracks_sai.data().and_then(|data| {
            data.tracks
                .iter()
                .find(|track| track.id == track_id)
                .map(ProtoTrackRecordExt::stored)
        });
        let list_key = SaiCache::new(["listMyTracks"]).key;

        let optimistic = SaiWsOptimisticUpdate {
            enabled: true,
            delete_mode: true,
            delete_id: Some(track_id.to_owned()),
            target_cache_key: Some(list_key.clone()),
            ..SaiWsOptimisticUpdate::default()
        };

        let body = rise_request::music::DeleteTrack { track_id: Some(track_id.to_owned()) };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: rise_wire_methods::music::DELETE_TRACK.to_owned(),
            cache_key: Some(list_key.clone()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: false,
            path_to_array: Some(key::TRACKS.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            optimistic_update: Some(optimistic),
            ..SaiWsRequestOptionsInit::default()
        });

        MusicServicesStore::shared(cx).update(cx, |services, cx| {
            services.remove_list_row(&list_key, key::TRACKS, track_id, cx);
        });

        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(track) = track_to_restore.clone() {
                MusicServicesStore::shared(cx)
                    .update(cx, |services, cx| services.revert_delete_track_optimistic(&track, cx));
            }
            Self::log_network(format!("deleteTrack error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.delete_track_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn list_my_albums_action(&mut self, reflect_pending_in_ui: bool, reload_if_cached: bool, cx: &mut Context<Self>) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::ListMyAlbums {};
        let cache_key = SaiCache::new(["listMyAlbums"]).key;
        let mut options = Self::disk_read_options(
            rise_wire_methods::music::LIST_MY_ALBUMS,
            &cache_key,
            reload_if_cached,
            reflect_pending_in_ui,
            false,
        );
        options.path_to_array = Some(key::ALBUMS.to_owned());
        options.path_to_id = Some(key::ID.to_owned());

        SaiWs::request_slice(
            &self.list_my_albums_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("listMyAlbums")),
            cx,
        );
    }

    pub fn get_album_action(
        &mut self,
        album_id: &str,
        reflect_pending_in_ui: bool,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetAlbum { album_id: Some(album_id.to_owned()) };
        let cache_key = SaiCache::new(["getAlbum", album_id]).key;
        let options =
            Self::disk_read_options(rise_wire_methods::music::GET_ALBUM, &cache_key, true, reflect_pending_in_ui, false);

        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("getAlbum error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(cx);
            }
        });
        SaiWs::request_slice(&self.get_album_sai, body, options, Some(success), Some(failure), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn request_album_action(
        &mut self,
        title: &str,
        genre: &str,
        local_cover_file: SaiWsOptimisticLocalFile,
        release_date_ms: Option<i64>,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let trimmed_title = trimming_whitespaces(title).to_owned();
        if trimmed_title.is_empty() {
            return;
        }

        let body = rise_request::music::RequestAlbum {
            cover_url: None,
            genre: Some(genre.to_owned()),
            title: Some(trimmed_title.clone()),
            release_date_ms,
        };

        let progress_title = trimmed_title.clone();
        let progress_slice = Rc::clone(&self.request_album_sai);
        let on_progress: Rc<dyn Fn(f64, &mut App)> = Rc::new(move |progress: f64, cx: &mut App| {
            MusicServicesStore::shared(cx).update(cx, |services, _| {
                services.patch_request_album_pending_upload(&progress_slice, &progress_title, progress, true);
            });
        });
        let file_on_error = on_error.clone();
        let on_file_error: Rc<dyn Fn(&anyhow::Error, &mut App)> =
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                if let Some(on_error) = &file_on_error {
                    on_error(CanonErrors::refusal_text(error), cx);
                }
            });
        let reject_slice_handle = Rc::clone(&self.request_album_sai);
        let reject_slice: Rc<dyn Fn(&anyhow::Error, &mut App)> = Rc::new(move |_: &anyhow::Error, _: &mut App| {
            reject_slice_handle.reset_to_idle();
        });

        let services = MusicServicesStore::shared(cx);
        let files = services.update(cx, |services, cx| {
            services.make_scalar_cover_optimistic_files(local_cover_file, on_progress, on_file_error, reject_slice, cx)
        });
        let optimistic = SaiWsOptimisticUpdate {
            enabled: true,
            files: Some(files),
            ..SaiWsOptimisticUpdate::default()
        };
        let pending_slice = Rc::clone(&self.request_album_sai);
        services.update(cx, |services, _| {
            services.patch_request_album_pending_upload(&pending_slice, &trimmed_title, 0.0, true);
        });

        let mut options = Self::mutation_options(rise_wire_methods::music::REQUEST_ALBUM, false);
        options.optimistic_update = Some(optimistic);

        let success_slice = Rc::clone(&self.request_album_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(request) = success_slice.data().and_then(|data| data.request) {
                MusicServicesStore::shared(cx)
                    .update(cx, |services, cx| services.insert_my_album_request(&request, cx));
            }
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let error_slice = Rc::clone(&self.request_album_sai);
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("requestAlbum error: {error}"), cx);
            error_slice.reset_to_idle();
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.request_album_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_my_album_requests_action(
        &mut self,
        cursor: Option<&str>,
        reflect_pending_in_ui: bool,
        reload_if_cached: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetMyAlbumRequests {
            limit: Some(30),
            cursor: cursor.map(str::to_owned),
        };

        let cache_key = MusicServicesStore::my_album_requests_cache_key();
        let options = MusicServicesStore::shared(cx).read(cx).paginated_read_options(
            rise_wire_methods::music::GET_MY_ALBUM_REQUESTS,
            &cache_key,
            cursor,
            reload_if_cached,
            reflect_pending_in_ui,
            key::REQUESTS,
            key::ID,
        );

        SaiWs::request_slice(
            &self.get_my_album_requests_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getMyAlbumRequests")),
            cx,
        );
    }

    pub fn publish_track_action(
        &mut self,
        track_id: &str,
        scheduled_at_ms: Option<i64>,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::PublishTrack {
            track_id: Some(track_id.to_owned()),
            scheduled_at_ms,
        };
        let mut options = Self::mutation_options(rise_wire_methods::music::PUBLISH_TRACK, false);
        options.reflect_pending_in_ui = true;

        let success_slice = Rc::clone(&self.publish_track_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(track) = success_slice.data().and_then(|data| data.track) {
                let track = RiseStoredMusicTrack::from_canon(&track);
                MusicServicesStore::shared(cx).update(cx, |services, cx| services.patch_published_track(&track, cx));
            }
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("publishTrack error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.publish_track_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn publish_album_action(
        &mut self,
        album_id: &str,
        scheduled_at_ms: Option<i64>,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::PublishAlbum {
            album_id: Some(album_id.to_owned()),
            scheduled_at_ms,
        };
        let mut options = Self::mutation_options(rise_wire_methods::music::PUBLISH_ALBUM, false);
        options.reflect_pending_in_ui = true;

        let success_slice = Rc::clone(&self.publish_album_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(album) = success_slice.data().and_then(|data| data.album) {
                MusicServicesStore::shared(cx).update(cx, |services, cx| services.patch_published_album(&album, cx));
            }
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("publishAlbum error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.publish_album_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn list_artist_requests_action(
        &mut self,
        status: &str,
        cursor: Option<&str>,
        fetch_if_have_data: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let is_first = cursor.is_none();
        let cache_key = MusicServicesStore::artist_requests_cache_key(status);
        let services = MusicServicesStore::shared(cx);
        if is_first {
            let slice = Rc::clone(&self.list_artist_requests_sai);
            services.update(cx, |services, cx| services.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        }

        let body = rise_request::music::ListArtistRequests {
            limit: Some(30),
            status: Some(status.to_owned()),
            cursor: cursor.map(str::to_owned),
            ..rise_request::music::ListArtistRequests::default()
        };

        let options = services.read(cx).paginated_read_options(
            rise_wire_methods::music::LIST_ARTIST_REQUESTS,
            &cache_key,
            cursor,
            fetch_if_have_data,
            is_first,
            key::REQUESTS,
            key::ID,
        );

        SaiWs::request_slice(
            &self.list_artist_requests_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("listArtistRequests")),
            cx,
        );
    }

    fn plain_read_options(method: &str) -> SaiWsRequestOptions {
        let mut options = Self::mutation_options(method, false);
        options.reflect_pending_in_ui = true;
        options
    }

    pub fn get_artist_request_action(&mut self, request_id: &str, cx: &mut Context<Self>) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetArtistRequest {
            request_id: Some(request_id.to_owned()),
            ..rise_request::music::GetArtistRequest::default()
        };
        let options = Self::plain_read_options(rise_wire_methods::music::GET_ARTIST_REQUEST);

        SaiWs::request_slice(
            &self.get_artist_request_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getArtistRequest")),
            cx,
        );
    }

    fn review_delete_options(method: &str, list_key: &str, path_to_array: &str, delete_id: &str) -> SaiWsRequestOptions {
        let optimistic = SaiWsOptimisticUpdate {
            enabled: true,
            delete_mode: true,
            delete_id: Some(delete_id.to_owned()),
            target_cache_key: Some(list_key.to_owned()),
            ..SaiWsOptimisticUpdate::default()
        };
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: Some(list_key.to_owned()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: false,
            path_to_array: Some(path_to_array.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            optimistic_update: Some(optimistic),
            ..SaiWsRequestOptionsInit::default()
        })
    }

    pub fn review_artist_request_action(
        &mut self,
        request_id: &str,
        approve: bool,
        rejection_reason: Option<&str>,
        list_status: &str,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let request_to_restore = self
            .list_artist_requests_sai
            .data()
            .and_then(|data| data.requests.into_iter().find(|request| request.id == request_id));
        let list_key = SaiCache::new(["listArtistRequests", list_status]).key;

        let mut body = rise_request::music::ReviewArtistRequest {
            approve: Some(approve),
            request_id: Some(request_id.to_owned()),
            ..rise_request::music::ReviewArtistRequest::default()
        };
        if let Some(rejection_reason) = rejection_reason
            && !rejection_reason.is_empty()
        {
            body.rejection_reason = Some(rejection_reason.to_owned());
        }

        let options = Self::review_delete_options(
            rise_wire_methods::music::REVIEW_ARTIST_REQUEST,
            &list_key,
            key::REQUESTS,
            request_id,
        );

        MusicServicesStore::shared(cx).update(cx, |services, cx| {
            services.remove_list_row(&list_key, key::REQUESTS, request_id, cx);
        });

        let success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {});
        let list_status = list_status.to_owned();
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(request) = request_to_restore.clone() {
                MusicServicesStore::shared(cx)
                    .update(cx, |services, cx| services.revert_artist_request(&request, &list_status, cx));
            }
            Self::log_network(format!("reviewArtistRequest error: {error}"), cx);
        });
        SaiWs::request_slice(&self.review_artist_request_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn track_requests_list_status(&self) -> &str {
        &self.track_requests_list_status
    }

    pub fn list_track_requests_action(
        &mut self,
        status: &str,
        cursor: Option<&str>,
        fetch_if_have_data: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let is_first = cursor.is_none();
        let cache_key = MusicServicesStore::track_requests_cache_key(status);
        let services = MusicServicesStore::shared(cx);
        if is_first {
            self.track_requests_list_status = status.to_owned();
            let slice = Rc::clone(&self.list_track_requests_sai);
            services.update(cx, |services, cx| services.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        }

        let body = rise_request::music::ListTrackRequests {
            limit: Some(30),
            status: Some(status.to_owned()),
            cursor: cursor.map(str::to_owned),
            ..rise_request::music::ListTrackRequests::default()
        };

        let options = services.read(cx).paginated_read_options(
            rise_wire_methods::music::LIST_TRACK_REQUESTS,
            &cache_key,
            cursor,
            fetch_if_have_data,
            is_first,
            key::TRACKS,
            key::ID,
        );

        SaiWs::request_slice(
            &self.list_track_requests_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("listTrackRequests")),
            cx,
        );
    }

    pub fn get_track_request_action(&mut self, track_id: &str, cx: &mut Context<Self>) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetTrack { track_id: Some(track_id.to_owned()) };
        let options = Self::plain_read_options(<rise_request::music::GetTrack as RiseRequestBody>::ACTION);

        SaiWs::request_slice(
            &self.get_track_request_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getTrackRequest")),
            cx,
        );
    }

    pub fn review_track_request_action(
        &mut self,
        track_id: &str,
        approve: bool,
        rejection_reason: Option<&str>,
        list_status: &str,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let track_to_restore = self.list_track_requests_sai.data().and_then(|data| {
            data.tracks
                .iter()
                .find(|track| track.id == track_id)
                .map(ProtoTrackRecordExt::stored)
        });
        let list_key = SaiCache::new(["listTrackRequests", list_status]).key;

        let mut body = rise_request::music::ReviewTrackRequest {
            approve: Some(approve),
            track_id: Some(track_id.to_owned()),
            ..rise_request::music::ReviewTrackRequest::default()
        };
        if let Some(rejection_reason) = rejection_reason
            && !rejection_reason.is_empty()
        {
            body.rejection_reason = Some(rejection_reason.to_owned());
        }

        let options = Self::review_delete_options(
            rise_wire_methods::music::REVIEW_TRACK_REQUEST,
            &list_key,
            key::TRACKS,
            track_id,
        );

        MusicServicesStore::shared(cx).update(cx, |services, cx| {
            services.remove_list_row(&list_key, key::TRACKS, track_id, cx);
        });

        let this = cx.entity().downgrade();
        let success_track_id = track_id.to_owned();
        let success_list_key = list_key.clone();
        let success_list_status = list_status.to_owned();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(store) = this.upgrade() else {
                MusicServicesStore::shared(cx).update(cx, |services, cx| {
                    services.remove_list_row(&success_list_key, key::TRACKS, &success_track_id, cx);
                });
                return;
            };
            let detail_matches = store
                .read(cx)
                .get_track_request_sai
                .data()
                .and_then(|data| data.track)
                .is_some_and(|track| track.id == success_track_id);
            if detail_matches {
                store.read(cx).get_track_request_sai.reset_to_idle();
            }
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.remove_list_row(&success_list_key, key::TRACKS, &success_track_id, cx);
            });
            store.update(cx, |store, cx| {
                store.get_moderation_queue_metrics_action(false, true, cx);
                if store.track_requests_list_status != success_list_status {
                    return;
                }
                store.list_track_requests_action(&success_list_status, None, true, cx);
            });
        });
        let list_status = list_status.to_owned();
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(track) = track_to_restore.clone() {
                MusicServicesStore::shared(cx)
                    .update(cx, |services, cx| services.revert_track_request(&track, &list_status, cx));
            }
            Self::log_network(format!("reviewTrackRequest error: {error}"), cx);
        });
        SaiWs::request_slice(&self.review_track_request_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_my_artist_request_action(&mut self, fetch_if_have_data: bool, cx: &mut Context<Self>) {
        let user_id = match AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire() {
            Some(user_id) if !user_id.is_empty() => user_id,
            _ => return,
        };

        let cache_key = SaiCache::new(["getMyArtistRequest", user_id.as_str()]).key;
        let services = MusicServicesStore::shared(cx);
        let slice = Rc::clone(&self.get_my_artist_request_sai);
        services.update(cx, |services, cx| services.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));

        let body = rise_request::music::GetMyArtistRequest {};
        let options = services.read(cx).moderation_cached_read_options(
            rise_wire_methods::music::GET_MY_ARTIST_REQUEST,
            Some(&cache_key),
            fetch_if_have_data,
            None,
            None,
            true,
        );

        SaiWs::request_slice(
            &self.get_my_artist_request_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getMyArtistRequest")),
            cx,
        );
    }

    pub fn list_album_requests_action(
        &mut self,
        status: &str,
        cursor: Option<&str>,
        fetch_if_have_data: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let is_first = cursor.is_none();
        let cache_key = MusicServicesStore::album_requests_cache_key(status);
        let services = MusicServicesStore::shared(cx);
        if is_first {
            let slice = Rc::clone(&self.list_album_requests_sai);
            services.update(cx, |services, cx| services.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        }

        let body = rise_request::music::ListAlbumRequests {
            limit: Some(30),
            status: Some(status.to_owned()),
            cursor: cursor.map(str::to_owned),
            ..rise_request::music::ListAlbumRequests::default()
        };

        let options = services.read(cx).paginated_read_options(
            rise_wire_methods::music::LIST_ALBUM_REQUESTS,
            &cache_key,
            cursor,
            fetch_if_have_data,
            is_first,
            key::REQUESTS,
            key::ID,
        );

        SaiWs::request_slice(
            &self.list_album_requests_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("listAlbumRequests")),
            cx,
        );
    }

    pub fn get_album_request_action(&mut self, request_id: &str, cx: &mut Context<Self>) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let body = rise_request::music::GetAlbumRequest {
            request_id: Some(request_id.to_owned()),
            ..rise_request::music::GetAlbumRequest::default()
        };
        let options = Self::plain_read_options(rise_wire_methods::music::GET_ALBUM_REQUEST);

        SaiWs::request_slice(
            &self.get_album_request_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getAlbumRequest")),
            cx,
        );
    }

    pub fn review_album_request_action(
        &mut self,
        request_id: &str,
        approve: bool,
        rejection_reason: Option<&str>,
        list_status: &str,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) {
            return;
        }

        let request_to_restore = self
            .list_album_requests_sai
            .data()
            .and_then(|data| data.requests.into_iter().find(|request| request.id == request_id));
        let list_key = SaiCache::new(["listAlbumRequests", list_status]).key;

        let mut body = rise_request::music::ReviewAlbumRequest {
            approve: Some(approve),
            request_id: Some(request_id.to_owned()),
            ..rise_request::music::ReviewAlbumRequest::default()
        };
        if let Some(rejection_reason) = rejection_reason
            && !rejection_reason.is_empty()
        {
            body.rejection_reason = Some(rejection_reason.to_owned());
        }

        let options = Self::review_delete_options(
            rise_wire_methods::music::REVIEW_ALBUM_REQUEST,
            &list_key,
            key::REQUESTS,
            request_id,
        );

        MusicServicesStore::shared(cx).update(cx, |services, cx| {
            services.remove_list_row(&list_key, key::REQUESTS, request_id, cx);
        });

        let this = cx.entity().downgrade();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            this.update(cx, |store, cx| store.get_moderation_queue_metrics_action(false, true, cx)).ok();
        });
        let list_status = list_status.to_owned();
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(request) = request_to_restore.clone() {
                MusicServicesStore::shared(cx)
                    .update(cx, |services, cx| services.revert_album_request(&request, &list_status, cx));
            }
            Self::log_network(format!("reviewAlbumRequest error: {error}"), cx);
        });
        SaiWs::request_slice(&self.review_album_request_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_playlist_settings_action(
        &mut self,
        playlist_id: &str,
        reflect_pending_in_ui: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || playlist_id.is_empty() {
            return;
        }

        let cache_key = MusicServicesStore::playlist_settings_cache_key(playlist_id);
        let body = rise_request::music::GetPlaylistSettings { playlist_id: Some(playlist_id.to_owned()) };
        let options = MusicServicesStore::shared(cx).read(cx).collaboration_read_options(
            rise_wire_methods::music::GET_PLAYLIST_SETTINGS,
            &cache_key,
            reflect_pending_in_ui,
        );

        SaiWs::request_slice(
            &self.get_playlist_settings_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getPlaylistSettings")),
            cx,
        );
    }

    pub fn get_playlist_invite_link_action(
        &mut self,
        playlist_id: &str,
        reflect_pending_in_ui: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || playlist_id.is_empty() {
            return;
        }

        let cache_key = MusicServicesStore::playlist_invite_link_cache_key(playlist_id);
        let body = rise_request::music::GetPlaylistInviteLink { playlist_id: Some(playlist_id.to_owned()) };
        let options = MusicServicesStore::shared(cx).read(cx).collaboration_read_options(
            rise_wire_methods::music::GET_PLAYLIST_INVITE_LINK,
            &cache_key,
            reflect_pending_in_ui,
        );

        SaiWs::request_slice(
            &self.get_playlist_invite_link_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getPlaylistInviteLink")),
            cx,
        );
    }

    /// text an unusable invite reports, the invite sheet maps it to its unavailable state
    fn invalid_invite_link_text() -> String {
        CanonErrors::text(Some(ErrorCode::PlaylistInviteInvalid.wire()), &HashMap::new())
            .unwrap_or_else(|| tr("music_playlist_preview_unavailable"))
    }

    pub fn get_playlist_invite_preview_action(
        &mut self,
        token: &str,
        reflect_pending_in_ui: bool,
        on_success: Option<Rc<dyn Fn(Option<MusicPlaylistInvitePreview>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || token.is_empty() {
            if let Some(on_error) = &on_error {
                on_error(Self::invalid_invite_link_text(), cx);
            }
            return;
        }

        let cache_key = MusicServicesStore::playlist_invite_preview_cache_key(token);
        let body = rise_request::music::GetPlaylistInvitePreview { token: Some(token.to_owned()) };
        let options = MusicServicesStore::shared(cx).read(cx).collaboration_read_options(
            rise_wire_methods::music::GET_PLAYLIST_INVITE_PREVIEW,
            &cache_key,
            reflect_pending_in_ui,
        );

        let preview_handler: Option<SaiWsOnSuccess> = on_success.map(|on_success| {
            let handler: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let response = Self::decoded_payload::<WirePlaylistInvitePreviewResponseDTO>(payload);
                on_success(
                    response.and_then(|response| response.preview.as_ref().and_then(|preview| preview.as_domain())),
                    cx,
                );
            });
            handler
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("getPlaylistInvitePreview error: {error}"), cx);
            let Some(on_error) = &on_error else { return };
            let invite_invalid = CanonErrors::failure(error)
                .and_then(|failure| failure.code)
                .is_some_and(|code| code == ErrorCode::PlaylistInviteInvalid.wire());
            if invite_invalid {
                on_error(Self::invalid_invite_link_text(), cx);
            } else {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.get_playlist_invite_preview_sai, body, options, preview_handler, Some(failure), cx);
    }

    pub fn request_playlist_join_action(
        &mut self,
        token: &str,
        client_request_id: &str,
        on_success: Option<Rc<dyn Fn(Option<MusicPlaylistJoinRequest>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || token.is_empty() || client_request_id.is_empty() {
            return;
        }

        let body = rise_request::music::RequestPlaylistJoin {
            client_request_id: Some(client_request_id.to_owned()),
            token: Some(token.to_owned()),
        };
        let options = MusicServicesStore::shared(cx)
            .read(cx)
            .collaboration_write_options(rise_wire_methods::music::REQUEST_PLAYLIST_JOIN);

        let token = token.to_owned();
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let result = MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.handle_request_playlist_join_success(payload, &token, cx)
            });
            if let Some(on_success) = &on_success {
                on_success(result, cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("requestPlaylistJoin error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.request_playlist_join_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_playlist_join_requests_action(
        &mut self,
        playlist_id: &str,
        cursor: Option<&str>,
        reflect_pending_in_ui: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || playlist_id.is_empty() {
            return;
        }

        let mut body = rise_request::music::GetPlaylistJoinRequests {
            limit: Some(50),
            playlist_id: Some(playlist_id.to_owned()),
            ..rise_request::music::GetPlaylistJoinRequests::default()
        };
        if let Some(cursor) = cursor
            && !cursor.is_empty()
        {
            body.cursor = Some(cursor.to_owned());
        }

        let cache_key = MusicServicesStore::playlist_join_requests_cache_key(playlist_id);
        let options = MusicServicesStore::shared(cx).read(cx).collaboration_paginated_read_options(
            rise_wire_methods::music::GET_PLAYLIST_JOIN_REQUESTS,
            &cache_key,
            cursor,
            reflect_pending_in_ui,
            key::REQUESTS,
            key::ID,
        );

        SaiWs::request_slice(
            &self.get_playlist_join_requests_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getPlaylistJoinRequests")),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn review_playlist_join_request_action(
        &mut self,
        playlist_id: &str,
        request_id: &str,
        approve: bool,
        client_request_id: &str,
        on_success: Option<Rc<dyn Fn(Option<MusicPlaylistMember>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(Option<String>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || playlist_id.is_empty() || request_id.is_empty() || client_request_id.is_empty()
        {
            return;
        }
        let services = MusicServicesStore::shared(cx);
        if !services.update(cx, |services, _| services.begin_playlist_collaboration_mutation(playlist_id)) {
            return;
        }

        let body = rise_request::music::ReviewPlaylistJoinRequest {
            approve: Some(approve),
            client_request_id: Some(client_request_id.to_owned()),
            playlist_id: Some(playlist_id.to_owned()),
            request_id: Some(request_id.to_owned()),
        };
        let options = services
            .read(cx)
            .collaboration_write_options(rise_wire_methods::music::REVIEW_PLAYLIST_JOIN_REQUEST);
        let snapshot = services.update(cx, |services, cx| {
            services.apply_review_playlist_join_request_optimistic(playlist_id, request_id, cx)
        });
        services.update(cx, |services, _| services.mark_playlist_mutation_acknowledged(client_request_id));

        let success_playlist_id = playlist_id.to_owned();
        let success_request_id = request_id.to_owned();
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let member = MusicServicesStore::shared(cx).update(cx, |services, cx| {
                let member = services.handle_review_playlist_join_request_success(
                    payload,
                    &success_playlist_id,
                    &success_request_id,
                    cx,
                );
                services.finish_playlist_collaboration_mutation(&success_playlist_id);
                member
            });
            if let Some(on_success) = &on_success {
                on_success(member, cx);
            }
        });
        let error_playlist_id = playlist_id.to_owned();
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.restore_collaboration_mutation(&snapshot, cx);
                services.finish_playlist_collaboration_mutation(&error_playlist_id);
            });
            Self::log_network(format!("reviewPlaylistJoinRequest error: {error}"), cx);
            let paywall_offered = CanonErrors::failure(error).is_some_and(|failure| failure.subscription_lifts_limit());
            if let Some(on_error) = &on_error {
                on_error((!paywall_offered).then(|| CanonErrors::refusal_text(error)), cx);
            }
        });
        SaiWs::request_slice(&self.review_playlist_join_request_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_playlist_members_action(
        &mut self,
        playlist_id: &str,
        include_suspended: bool,
        cursor: Option<&str>,
        reflect_pending_in_ui: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || playlist_id.is_empty() {
            return;
        }

        let mut body = rise_request::music::GetPlaylistMembers {
            include_suspended: Some(include_suspended),
            limit: Some(50),
            playlist_id: Some(playlist_id.to_owned()),
            ..rise_request::music::GetPlaylistMembers::default()
        };
        if let Some(cursor) = cursor
            && !cursor.is_empty()
        {
            body.cursor = Some(cursor.to_owned());
        }

        let cache_key = MusicServicesStore::playlist_members_cache_key(playlist_id, include_suspended);
        let options = MusicServicesStore::shared(cx).read(cx).collaboration_paginated_read_options(
            rise_wire_methods::music::GET_PLAYLIST_MEMBERS,
            &cache_key,
            cursor,
            reflect_pending_in_ui,
            key::MEMBERS,
            key::USER_ID,
        );

        SaiWs::request_slice(
            &self.get_playlist_members_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("getPlaylistMembers")),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_playlist_member_permissions_action(
        &mut self,
        playlist_id: &str,
        member_user_id: &str,
        permissions: MusicPlaylistMemberPermissions,
        expected_version: Option<i64>,
        client_request_id: &str,
        on_success: Option<Rc<dyn Fn(MusicPlaylistMember, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx)
            || playlist_id.is_empty()
            || member_user_id.is_empty()
            || client_request_id.is_empty()
        {
            return;
        }
        let services = MusicServicesStore::shared(cx);
        if !services.update(cx, |services, _| services.begin_playlist_collaboration_mutation(playlist_id)) {
            return;
        }

        let body = rise_request::music::UpdatePlaylistMemberPermissions {
            client_request_id: Some(client_request_id.to_owned()),
            member_user_id: Some(member_user_id.to_owned()),
            permissions: Some(rise_request::music::update_playlist_member_permissions::Permissions {
                can_add_tracks: Some(permissions.can_add_tracks),
                can_edit_playlist: Some(permissions.can_edit_playlist),
                can_remove_tracks: Some(permissions.can_remove_tracks),
            }),
            playlist_id: Some(playlist_id.to_owned()),
            expected_version,
        };
        let options = services
            .read(cx)
            .collaboration_write_options(rise_wire_methods::music::UPDATE_PLAYLIST_MEMBER_PERMISSIONS);
        let snapshot = services.update(cx, |services, cx| {
            services.apply_playlist_member_permissions_optimistic(playlist_id, member_user_id, permissions, cx)
        });
        services.update(cx, |services, _| services.mark_playlist_mutation_acknowledged(client_request_id));

        let success_playlist_id = playlist_id.to_owned();
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let member = MusicServicesStore::shared(cx).update(cx, |services, cx| {
                let member = services.handle_update_playlist_member_success(payload, &success_playlist_id, cx);
                services.finish_playlist_collaboration_mutation(&success_playlist_id);
                member
            });
            if let Some(member) = member
                && let Some(on_success) = &on_success
            {
                on_success(member, cx);
            }
        });
        let error_playlist_id = playlist_id.to_owned();
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.restore_collaboration_mutation(&snapshot, cx);
                services.finish_playlist_collaboration_mutation(&error_playlist_id);
            });
            Self::log_network(format!("updatePlaylistMemberPermissions error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(
            &self.update_playlist_member_permissions_sai,
            body,
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn remove_playlist_member_action(
        &mut self,
        playlist_id: &str,
        member_user_id: &str,
        client_request_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx)
            || playlist_id.is_empty()
            || member_user_id.is_empty()
            || client_request_id.is_empty()
        {
            return;
        }
        let services = MusicServicesStore::shared(cx);
        if !services.update(cx, |services, _| services.begin_playlist_collaboration_mutation(playlist_id)) {
            return;
        }

        let body = rise_request::music::RemovePlaylistMember {
            client_request_id: Some(client_request_id.to_owned()),
            member_user_id: Some(member_user_id.to_owned()),
            playlist_id: Some(playlist_id.to_owned()),
        };
        let options = services
            .read(cx)
            .collaboration_write_options(rise_wire_methods::music::REMOVE_PLAYLIST_MEMBER);
        let snapshot = services.update(cx, |services, cx| {
            services.apply_playlist_member_removal_optimistic(playlist_id, member_user_id, cx)
        });
        services.update(cx, |services, _| services.mark_playlist_mutation_acknowledged(client_request_id));

        let success_playlist_id = playlist_id.to_owned();
        let success_member_user_id = member_user_id.to_owned();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.handle_playlist_member_removed(&success_playlist_id, &success_member_user_id, cx);
                services.finish_playlist_collaboration_mutation(&success_playlist_id);
            });
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let error_playlist_id = playlist_id.to_owned();
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.restore_collaboration_mutation(&snapshot, cx);
                services.finish_playlist_collaboration_mutation(&error_playlist_id);
            });
            Self::log_network(format!("removePlaylistMember error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.remove_playlist_member_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn leave_playlist_action(
        &mut self,
        playlist_id: &str,
        client_request_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || playlist_id.is_empty() || client_request_id.is_empty() {
            return;
        }
        let services = MusicServicesStore::shared(cx);
        if !services.update(cx, |services, _| services.begin_playlist_collaboration_mutation(playlist_id)) {
            return;
        }

        let body = rise_request::music::LeavePlaylist {
            client_request_id: Some(client_request_id.to_owned()),
            playlist_id: Some(playlist_id.to_owned()),
        };
        let options = services
            .read(cx)
            .collaboration_write_options(rise_wire_methods::music::LEAVE_PLAYLIST);
        let snapshot = services.update(cx, |services, cx| services.apply_leave_playlist_optimistic(playlist_id, cx));
        services.update(cx, |services, _| services.mark_playlist_mutation_acknowledged(client_request_id));

        let success_playlist_id = playlist_id.to_owned();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.handle_playlist_access_revoked(
                    &success_playlist_id,
                    music_service::PLAYLIST_MEMBER_EXIT_LEFT,
                    cx,
                );
                services.finish_playlist_collaboration_mutation(&success_playlist_id);
            });
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let error_playlist_id = playlist_id.to_owned();
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.restore_collaboration_mutation(&snapshot, cx);
                services.finish_playlist_collaboration_mutation(&error_playlist_id);
            });
            Self::log_network(format!("leavePlaylist error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(&self.leave_playlist_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_playlist_listening_snapshot_action(
        &mut self,
        playlist_id: &str,
        track_id: Option<&str>,
        last_presence_sequence: Option<i64>,
        reflect_pending_in_ui: bool,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || playlist_id.is_empty() {
            return;
        }

        let mut body = rise_request::music::GetPlaylistListeningSnapshot {
            playlist_id: Some(playlist_id.to_owned()),
            ..rise_request::music::GetPlaylistListeningSnapshot::default()
        };
        if let Some(track_id) = track_id
            && !track_id.is_empty()
        {
            body.track_id = Some(track_id.to_owned());
        }
        if let Some(last_presence_sequence) = last_presence_sequence
            && last_presence_sequence > 0
        {
            body.last_presence_sequence = Some(last_presence_sequence);
        }

        let cache_key = MusicServicesStore::playlist_listening_snapshot_cache_key(playlist_id, track_id);
        let options = MusicServicesStore::shared(cx).read(cx).realtime_read_options(
            rise_wire_methods::music::GET_PLAYLIST_LISTENING_SNAPSHOT,
            &cache_key,
            reflect_pending_in_ui,
            false,
        );
        let request_started_at_ms = RiseClock::now().ms();

        let success_playlist_id = playlist_id.to_owned();
        let success_track_id = track_id.map(str::to_owned);
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.handle_playlist_listening_snapshot_success(
                    payload,
                    &success_playlist_id,
                    success_track_id.as_deref(),
                    request_started_at_ms,
                    cx,
                );
            });
        });
        SaiWs::request_slice(
            &self.get_playlist_listening_snapshot_sai,
            body,
            options,
            Some(success),
            Some(Self::log_only_on_error("getPlaylistListeningSnapshot")),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_playlist_listening_presence_action(
        &mut self,
        playlist_id: &str,
        track_id: &str,
        position_ms: i64,
        is_playing: bool,
        client_sequence: i64,
        client_instance_id: &str,
        client_command_id: &str,
        audio_effects: &[MusicListeningAudioEffect],
        on_success: Option<Rc<dyn Fn(WirePlaylistPresenceWriteDTO, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx)
            || playlist_id.is_empty()
            || track_id.is_empty()
            || client_instance_id.is_empty()
            || client_command_id.is_empty()
        {
            return;
        }

        let body = rise_request::music::UpsertPlaylistListeningPresence {
            audio_effects: Some(rise_request::music::upsert_playlist_listening_presence::AudioEffects {
                effects: Some(
                    audio_effects
                        .iter()
                        .map(|effect| rise_request::music::upsert_playlist_listening_presence::audio_effects::EffectsElement {
                            kind: Some(effect.kind.clone()),
                            value_milli: Some(effect.value_milli),
                        })
                        .collect(),
                ),
            }),
            client_command_id: Some(client_command_id.to_owned()),
            client_instance_id: Some(client_instance_id.to_owned()),
            client_sequence: Some(client_sequence.max(1)),
            is_playing: Some(is_playing),
            playlist_id: Some(playlist_id.to_owned()),
            position_ms: Some(position_ms.max(0)),
            track_id: Some(track_id.to_owned()),
        };
        let options = MusicServicesStore::shared(cx)
            .read(cx)
            .realtime_write_options(rise_wire_methods::music::UPSERT_PLAYLIST_LISTENING_PRESENCE, true);

        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(response) = Self::decoded_payload::<WirePlaylistPresenceWriteDTO>(payload) else { return };
            if let Some(on_success) = &on_success {
                on_success(response, cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("upsertPlaylistListeningPresence error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(
            &self.upsert_playlist_listening_presence_sai,
            body,
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn stop_playlist_listening_presence_action(
        &mut self,
        playlist_id: &str,
        client_instance_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || playlist_id.is_empty() || client_instance_id.is_empty() {
            return;
        }

        let body = rise_request::music::StopPlaylistListeningPresence {
            client_instance_id: Some(client_instance_id.to_owned()),
            playlist_id: Some(playlist_id.to_owned()),
        };
        let options = MusicServicesStore::shared(cx)
            .read(cx)
            .realtime_write_options(rise_wire_methods::music::STOP_PLAYLIST_LISTENING_PRESENCE, false);

        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("stopPlaylistListeningPresence error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(
            &self.stop_playlist_listening_presence_sai,
            body,
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn join_playlist_listening_session_action(
        &mut self,
        playlist_id: &str,
        target_user_id: &str,
        client_instance_id: &str,
        client_request_id: &str,
        expected_session_id: Option<&str>,
        on_success: Option<Rc<dyn Fn(MusicPlaylistListeningSession, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx)
            || playlist_id.is_empty()
            || target_user_id.is_empty()
            || client_instance_id.is_empty()
            || client_request_id.is_empty()
        {
            return;
        }

        let body = rise_request::music::JoinPlaylistListeningSession {
            client_instance_id: Some(client_instance_id.to_owned()),
            client_request_id: Some(client_request_id.to_owned()),
            expected_session_id: expected_session_id.map(str::to_owned),
            playlist_id: Some(playlist_id.to_owned()),
            target_user_id: Some(target_user_id.to_owned()),
        };
        let options = MusicServicesStore::shared(cx)
            .read(cx)
            .realtime_write_options(rise_wire_methods::music::JOIN_PLAYLIST_LISTENING_SESSION, false);

        let success_on_error = on_error.clone();
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let session = MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.handle_listening_session_response(payload, None, None, true, true, cx)
            });
            match session {
                Some(session) => {
                    if let Some(on_success) = &on_success {
                        on_success(session, cx);
                    }
                }
                None => {
                    if let Some(on_error) = &success_on_error {
                        on_error(tr("music_live_action_failed"), cx);
                    }
                }
            }
        });
        let has_expected_session = expected_session_id.is_some();
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("joinPlaylistListeningSession error: {error}"), cx);
            let session_gone = CanonErrors::failure(error)
                .and_then(|failure| failure.code)
                .is_some_and(|code| code == ErrorCode::MusicListeningSessionNotFound.wire());
            if has_expected_session && session_gone {
                if let Some(on_error) = &on_error {
                    on_error(tr("music_live_session_ended_state"), cx);
                }
                return;
            }
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(
            &self.join_playlist_listening_session_sai,
            body,
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn get_playlist_listening_session_action(
        &mut self,
        session_id: &str,
        reflect_pending_in_ui: bool,
        on_success: Option<Rc<dyn Fn(MusicPlaylistListeningSession, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || session_id.is_empty() {
            return;
        }

        let cache_key = MusicServicesStore::playlist_listening_session_cache_key(session_id);
        let body = rise_request::music::GetPlaylistListeningSession { session_id: Some(session_id.to_owned()) };
        let options = MusicServicesStore::shared(cx).read(cx).realtime_read_options(
            rise_wire_methods::music::GET_PLAYLIST_LISTENING_SESSION,
            &cache_key,
            reflect_pending_in_ui,
            false,
        );
        let request_started_at_ms = RiseClock::now().ms();

        let success_session_id = session_id.to_owned();
        let success_on_error = on_error.clone();
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let services = MusicServicesStore::shared(cx);
            let session = services.update(cx, |services, cx| {
                services.handle_listening_session_response(
                    payload,
                    None,
                    Some(request_started_at_ms),
                    true,
                    false,
                    cx,
                )
            });
            let Some(session) = session else {
                services.update(cx, |services, cx| {
                    services.handle_listening_session_ended(
                        &success_session_id,
                        music_service::LISTENING_END_REASON_ENDED,
                        cx,
                    );
                });
                if let Some(on_error) = &success_on_error {
                    on_error(tr("music_live_session_ended_state"), cx);
                }
                return;
            };
            if let Some(on_success) = &on_success {
                on_success(session, cx);
            }
        });
        let error_session_id = session_id.to_owned();
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let code = error.downcast_ref::<SaiWsResponseError>().and_then(|error| error.code.clone());
            let message = error.to_string();
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.handle_listening_session_refresh_error(&error_session_id, &message, code.as_deref(), cx);
            });
            Self::log_network(format!("getPlaylistListeningSession error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(
            &self.get_playlist_listening_session_sai,
            body,
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_playlist_listening_session_state_action(
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
        audio_effects: &[MusicListeningAudioEffect],
        expected_revision: i64,
        client_sequence: i64,
        client_instance_id: &str,
        client_command_id: &str,
        on_success: Option<Rc<dyn Fn(MusicPlaylistListeningSession, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, Option<String>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx)
            || session_id.is_empty()
            || track_id.is_empty()
            || client_instance_id.is_empty()
            || client_command_id.is_empty()
        {
            if let Some(on_error) = &on_error {
                on_error(tr("music_live_action_failed"), None, cx);
            }
            return;
        }

        let body = rise_request::music::UpdatePlaylistListeningSessionState {
            audio_effect_enabled: Some(audio_effect_enabled),
            audio_effect_rate_milli: Some(MusicEffectPlaybackLimits::rate_milli(MusicEffectPlaybackLimits::rate(
                audio_effect_rate_milli,
            ))),
            audio_effects: Some(rise_request::music::update_playlist_listening_session_state::AudioEffects {
                effects: Some(
                    audio_effects
                        .iter()
                        .map(|effect| {
                            rise_request::music::update_playlist_listening_session_state::audio_effects::EffectsElement {
                                kind: Some(effect.kind.clone()),
                                value_milli: Some(effect.value_milli),
                            }
                        })
                        .collect(),
                ),
            }),
            client_command_id: Some(client_command_id.to_owned()),
            client_instance_id: Some(client_instance_id.to_owned()),
            client_sequence: Some(client_sequence.max(1)),
            command: Some(command.wire().to_owned()),
            expected_revision: Some(expected_revision.max(1)),
            is_playing: Some(is_playing),
            position_ms: Some(position_ms.max(0)),
            repeat_mode: Some(repeat_mode.to_owned()),
            session_id: Some(session_id.to_owned()),
            shuffle_enabled: Some(shuffle_enabled),
            shuffle_seed: Some(shuffle_seed.max(0)),
            track_id: Some(track_id.to_owned()),
        };
        let options = MusicServicesStore::shared(cx)
            .read(cx)
            .realtime_write_options(rise_wire_methods::music::UPDATE_PLAYLIST_LISTENING_SESSION_STATE, false);

        let success_on_error = on_error.clone();
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let session = MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.handle_listening_session_response(payload, None, None, false, false, cx)
            });
            match session {
                Some(session) => {
                    if let Some(on_success) = &on_success {
                        on_success(session, cx);
                    }
                }
                None => {
                    if let Some(on_error) = &success_on_error {
                        on_error(tr("music_live_action_failed"), None, cx);
                    }
                }
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("updatePlaylistListeningSessionState error: {error}"), cx);
            if let Some(on_error) = &on_error {
                let code = error.downcast_ref::<SaiWsResponseError>().and_then(|error| error.code.clone());
                on_error(CanonErrors::refusal_text(error), code, cx);
            }
        });
        SaiWs::request_slice(
            &self.update_playlist_listening_session_state_sai,
            body,
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn request_playlist_listening_permission_action(
        &mut self,
        session_id: &str,
        permissions: MusicListeningParticipantPermissions,
        client_request_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || session_id.is_empty() || client_request_id.is_empty() {
            return;
        }

        let body = rise_request::music::RequestPlaylistListeningPermission {
            client_request_id: Some(client_request_id.to_owned()),
            requested_permissions: Some(rise_request::music::request_playlist_listening_permission::RequestedPermissions {
                can_apply_effects: Some(permissions.can_apply_effects),
                can_change_tracks: Some(permissions.can_change_tracks),
                can_control_playback: Some(permissions.can_control_playback),
                can_seek: Some(permissions.can_seek),
            }),
            session_id: Some(session_id.to_owned()),
        };
        let options = MusicServicesStore::shared(cx)
            .read(cx)
            .realtime_write_options(rise_wire_methods::music::REQUEST_PLAYLIST_LISTENING_PERMISSION, false);

        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("requestPlaylistListeningPermission error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(
            &self.request_playlist_listening_permission_sai,
            body,
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn update_playlist_listening_participant_permissions_action(
        &mut self,
        session_id: &str,
        participant_user_id: &str,
        permissions: MusicListeningParticipantPermissions,
        client_request_id: &str,
        on_success: Option<Rc<dyn Fn(MusicPlaylistListeningSession, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx)
            || session_id.is_empty()
            || participant_user_id.is_empty()
            || client_request_id.is_empty()
        {
            if let Some(on_error) = &on_error {
                on_error(tr("retry_later_error"), cx);
            }
            return;
        }

        let body = rise_request::music::UpdatePlaylistListeningParticipantPermissions {
            client_request_id: Some(client_request_id.to_owned()),
            participant_user_id: Some(participant_user_id.to_owned()),
            permissions: Some(rise_request::music::update_playlist_listening_participant_permissions::Permissions {
                can_apply_effects: Some(permissions.can_apply_effects),
                can_change_tracks: Some(permissions.can_change_tracks),
                can_control_playback: Some(permissions.can_control_playback),
                can_seek: Some(permissions.can_seek),
            }),
            session_id: Some(session_id.to_owned()),
        };
        let options = MusicServicesStore::shared(cx).read(cx).realtime_write_options(
            rise_wire_methods::music::UPDATE_PLAYLIST_LISTENING_PARTICIPANT_PERMISSIONS,
            false,
        );

        let success_on_error = on_error.clone();
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let session = MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.handle_listening_session_response(payload, None, None, true, false, cx)
            });
            let Some(session) = session else {
                Self::log_network(
                    "updatePlaylistListeningParticipantPermissions: response session not installed".to_owned(),
                    cx,
                );
                if let Some(on_error) = &success_on_error {
                    on_error(tr("retry_later_error"), cx);
                }
                return;
            };
            if let Some(on_success) = &on_success {
                on_success(session, cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("updatePlaylistListeningParticipantPermissions error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(
            &self.update_playlist_listening_participant_permissions_sai,
            body,
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn leave_playlist_listening_session_action(
        &mut self,
        session_id: &str,
        client_request_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || session_id.is_empty() || client_request_id.is_empty() {
            return;
        }

        let body = rise_request::music::LeavePlaylistListeningSession {
            client_request_id: Some(client_request_id.to_owned()),
            session_id: Some(session_id.to_owned()),
        };
        let options = MusicServicesStore::shared(cx)
            .read(cx)
            .realtime_write_options(rise_wire_methods::music::LEAVE_PLAYLIST_LISTENING_SESSION, false);

        let success_session_id = session_id.to_owned();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.handle_local_listening_session_exit(&success_session_id, cx);
            });
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("leavePlaylistListeningSession error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(
            &self.leave_playlist_listening_session_sai,
            body,
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn end_playlist_listening_session_action(
        &mut self,
        session_id: &str,
        client_request_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !Self::has_wire_user(cx) || session_id.is_empty() || client_request_id.is_empty() {
            return;
        }

        let body = rise_request::music::EndPlaylistListeningSession {
            client_request_id: Some(client_request_id.to_owned()),
            session_id: Some(session_id.to_owned()),
        };
        let options = MusicServicesStore::shared(cx)
            .read(cx)
            .realtime_write_options(rise_wire_methods::music::END_PLAYLIST_LISTENING_SESSION, false);

        let success_session_id = session_id.to_owned();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.handle_listening_session_ended(
                    &success_session_id,
                    music_service::LISTENING_END_REASON_HOST_ENDED,
                    cx,
                );
            });
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("endPlaylistListeningSession error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });
        SaiWs::request_slice(
            &self.end_playlist_listening_session_sai,
            body,
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }
}
