use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::{RiseRequestJSON, RiseWireRequestMetadata};
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::{CancellationError, RiseEngine, RiseWireMethodDescriptor, RiseWireReplayPolicy, RiseWireTransport};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::core::rise_engine_bridge::rise_canon_endpoints::{RiseCanonEndpoints, RiseEndpointClientTimeout};
use crate::modules::map::engine::map_track_erasure_geometry::MapEraseStroke;
use crate::modules::map::engine::rise_map_engine_models::RiseMapError;
use crate::modules::map::engine::rise_map_journeys_repository::MapJourneySelection;
use crate::modules::map::engine::rise_map_profiles_repository::{MapProfile, ProtoProfileResponseExt};
use crate::modules::map::recorder::map_recorder_journal::{MapRecordedStay, MapSealedSegment};
use crate::modules::map::stores::map_actions::map_photos_types::{
    MapPhotoDraft, MapPhotoGrid, MapPhotoPlacement, MapPhotoRemoval,
};
use crate::modules::map::stores::map_actions::map_photos_wire_types::{
    CreatePhotoExt, ProtoMapPhotoExt, ProtoMapPhotoGridExt,
};
use crate::modules::map::stores::map_actions::map_privacy_types::{
    MapPrivacy, MapPrivacyAction, MapPrivacyKind, MapPrivacyRule,
};
use crate::modules::map::stores::map_actions::map_privacy_wire_types::ProtoPrivacyResponseExt;
use crate::modules::map::stores::map_actions::map_tracks_types::{MapPlace, MapPlaceKind, MapTileKey, MapTrackSummary};
use crate::modules::map::stores::map_actions::map_tracks_wire_types::{
    PlaceExt, PreviewTrackMergeExt, StrokesElementExt, TrackMergeExt, TrackSummaryExt,
};
use crate::modules::map::stores::map_actions::map_types::{
    MapFix, MapFriendRequestOutcome, MapInvite, MapInvitePreview, MapLocationAvailability, MapMergePreview,
    MapMergePreviewRequest, MapRequestDirection, MapSettingsPatch, MapState,
};
use crate::modules::map::stores::map_actions::map_wire_types::{
    FriendAnsweredExt, ProtoFriendRequestResponseExt, ProtoInvitePreviewResponseExt, ProtoInviteResponseExt,
    ProtoStateResponseExt, UpdateLocationFixExt, UpdateSettingsExt,
};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseMapEndpoint {
    pub method: String,
    pub timeout: Duration,
    pub is_read: bool,
    pub survives_suspension: bool,
    pub transport: RiseWireTransport,
}

impl RiseMapEndpoint {
    /// deadline and replay policy and transport come from the canon catalogue
    /// expecting only asserts the declared intent
    /// the wire refuses a request while the app is inactive unless it survives suspension
    #[track_caller]
    pub fn new(canonical: &str, expecting: RiseEndpointKind, survives_suspension: bool) -> Self {
        let entry = RiseCanonEndpoints::required(rise_canon::rise_wire_methods::map::SERVICE, canonical);
        debug_assert!(
            entry.kind == expecting,
            "map.{canonical}: canon declares {}, not {}",
            entry.kind.name(),
            expecting.name()
        );
        Self {
            method: entry.method.to_owned(),
            timeout: entry.client_timeout(),
            is_read: entry.kind == RiseEndpointKind::Read,
            survives_suspension,
            transport: if entry.http_route { RiseWireTransport::Http } else { RiseWireTransport::Socket },
        }
    }

    pub fn descriptor(&self) -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(rise_canon::rise_wire_methods::map::SERVICE, self.method.clone())
            .with_timeout(self.timeout)
            .with_replay_policy(if self.is_read { RiseWireReplayPolicy::ReadOnly } else { RiseWireReplayPolicy::Never })
            .with_survives_suspension(self.survives_suspension)
            .with_transport(self.transport)
    }
}

static PREPARE_JOURNEY: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::PREPARE_JOURNEY, RiseEndpointKind::Mutation, false));
static GET_JOURNEY: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_JOURNEY, RiseEndpointKind::Read, false));
static REVOKE_JOURNEY: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::REVOKE_JOURNEY, RiseEndpointKind::Mutation, false));
static GET_STATE: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_STATE, RiseEndpointKind::Read, false));
static JOIN: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::JOIN, RiseEndpointKind::Mutation, false));
static LEAVE: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::LEAVE, RiseEndpointKind::Mutation, false));
static UPDATE_SETTINGS: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::UPDATE_SETTINGS, RiseEndpointKind::Mutation, false));
static GET_PROFILE: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_PROFILE, RiseEndpointKind::Read, false));
static UPDATE_PHOTO_GRID: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::UPDATE_PHOTO_GRID, RiseEndpointKind::Mutation, false));
static GET_FRIENDS: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_FRIENDS, RiseEndpointKind::Read, false));
static GET_FRIEND_REQUESTS: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_FRIEND_REQUESTS, RiseEndpointKind::Read, false));
static SEND_FRIEND_REQUEST: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::SEND_FRIEND_REQUEST, RiseEndpointKind::Mutation, false));
static RESPOND_FRIEND_REQUEST: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::RESPOND_FRIEND_REQUEST, RiseEndpointKind::Mutation, false));
static CANCEL_FRIEND_REQUEST: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::CANCEL_FRIEND_REQUEST, RiseEndpointKind::Mutation, false));
static REMOVE_FRIEND: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::REMOVE_FRIEND, RiseEndpointKind::Mutation, false));
static GET_INVITE: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_INVITE, RiseEndpointKind::Read, false));
static RESET_INVITE: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::RESET_INVITE, RiseEndpointKind::Mutation, false));
static PREVIEW_INVITE: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::PREVIEW_INVITE, RiseEndpointKind::Read, false));
static USE_INVITE: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::USE_INVITE, RiseEndpointKind::Mutation, false));
static OPEN_SESSION: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::OPEN_SESSION, RiseEndpointKind::Mutation, false));
static HEARTBEAT: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::HEARTBEAT, RiseEndpointKind::Mutation, false));
static CLOSE_SESSION: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::CLOSE_SESSION, RiseEndpointKind::Mutation, false));
static UPDATE_LOCATION: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::UPDATE_LOCATION, RiseEndpointKind::Mutation, true));
static GET_REACTION_INBOX: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_REACTION_INBOX, RiseEndpointKind::Read, false));
static CLAIM_REACTIONS: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::CLAIM_REACTIONS, RiseEndpointKind::Mutation, false));
static GET_REACTION_PAGE: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_REACTION_PAGE, RiseEndpointKind::Read, false));
static SEND_REACTION: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::SEND_REACTION, RiseEndpointKind::Mutation, false));
static UPLOAD_TRACK: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::UPLOAD_TRACK, RiseEndpointKind::IdempotentMutation, true));
static GET_TRACK_TILES: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_TRACK_TILES, RiseEndpointKind::Read, false));
static GET_TRACK_SUMMARY: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_TRACK_SUMMARY, RiseEndpointKind::Read, false));
static ERASE_TRACKS: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::ERASE_TRACKS, RiseEndpointKind::Mutation, false));
static DELETE_ALL_TRACKS: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::DELETE_ALL_TRACKS, RiseEndpointKind::Mutation, false));
static PREVIEW_TRACK_MERGE: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::PREVIEW_TRACK_MERGE, RiseEndpointKind::Read, false));
static GET_PLACES: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_PLACES, RiseEndpointKind::Read, false));
static SET_PLACE: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::SET_PLACE, RiseEndpointKind::Mutation, false));
static DELETE_PLACE: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::DELETE_PLACE, RiseEndpointKind::Mutation, false));
static CREATE_PHOTO: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::CREATE_PHOTO, RiseEndpointKind::IdempotentMutation, false));
static GET_PHOTOS: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_PHOTOS, RiseEndpointKind::Read, false));
static DELETE_PHOTO: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::DELETE_PHOTO, RiseEndpointKind::Mutation, false));
static GET_PHOTO_GRIDS: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_PHOTO_GRIDS, RiseEndpointKind::Read, false));
static GET_GRID_PHOTOS: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_GRID_PHOTOS, RiseEndpointKind::Read, false));
static DELETE_PHOTO_GRID: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::DELETE_PHOTO_GRID, RiseEndpointKind::Mutation, false));
static GET_PRIVACY: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::GET_PRIVACY, RiseEndpointKind::Read, false));
static UPDATE_PRIVACY: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::UPDATE_PRIVACY, RiseEndpointKind::Mutation, false));
static SET_PRIVACY_EXCEPTIONS: LazyLock<RiseMapEndpoint> =
    LazyLock::new(|| RiseMapEndpoint::new(rise_canon::rise_wire_methods::map::SET_PRIVACY_EXCEPTIONS, RiseEndpointKind::Mutation, false));

pub enum RiseMapEndpoints {}

impl RiseMapEndpoints {
    pub fn prepare_journey() -> &'static RiseMapEndpoint {
        &PREPARE_JOURNEY
    }

    pub fn get_journey() -> &'static RiseMapEndpoint {
        &GET_JOURNEY
    }

    pub fn revoke_journey() -> &'static RiseMapEndpoint {
        &REVOKE_JOURNEY
    }

    pub fn get_state() -> &'static RiseMapEndpoint {
        &GET_STATE
    }

    pub fn join() -> &'static RiseMapEndpoint {
        &JOIN
    }

    pub fn leave() -> &'static RiseMapEndpoint {
        &LEAVE
    }

    pub fn update_settings() -> &'static RiseMapEndpoint {
        &UPDATE_SETTINGS
    }

    pub fn get_profile() -> &'static RiseMapEndpoint {
        &GET_PROFILE
    }

    pub fn update_photo_grid() -> &'static RiseMapEndpoint {
        &UPDATE_PHOTO_GRID
    }

    pub fn get_friends() -> &'static RiseMapEndpoint {
        &GET_FRIENDS
    }

    pub fn get_friend_requests() -> &'static RiseMapEndpoint {
        &GET_FRIEND_REQUESTS
    }

    pub fn send_friend_request() -> &'static RiseMapEndpoint {
        &SEND_FRIEND_REQUEST
    }

    pub fn respond_friend_request() -> &'static RiseMapEndpoint {
        &RESPOND_FRIEND_REQUEST
    }

    pub fn cancel_friend_request() -> &'static RiseMapEndpoint {
        &CANCEL_FRIEND_REQUEST
    }

    pub fn remove_friend() -> &'static RiseMapEndpoint {
        &REMOVE_FRIEND
    }

    pub fn get_invite() -> &'static RiseMapEndpoint {
        &GET_INVITE
    }

    pub fn reset_invite() -> &'static RiseMapEndpoint {
        &RESET_INVITE
    }

    pub fn preview_invite() -> &'static RiseMapEndpoint {
        &PREVIEW_INVITE
    }

    pub fn use_invite() -> &'static RiseMapEndpoint {
        &USE_INVITE
    }

    pub fn open_session() -> &'static RiseMapEndpoint {
        &OPEN_SESSION
    }

    pub fn heartbeat() -> &'static RiseMapEndpoint {
        &HEARTBEAT
    }

    pub fn close_session() -> &'static RiseMapEndpoint {
        &CLOSE_SESSION
    }

    pub fn update_location() -> &'static RiseMapEndpoint {
        &UPDATE_LOCATION
    }

    pub fn get_reaction_inbox() -> &'static RiseMapEndpoint {
        &GET_REACTION_INBOX
    }

    pub fn claim_reactions() -> &'static RiseMapEndpoint {
        &CLAIM_REACTIONS
    }

    pub fn get_reaction_page() -> &'static RiseMapEndpoint {
        &GET_REACTION_PAGE
    }

    pub fn send_reaction() -> &'static RiseMapEndpoint {
        &SEND_REACTION
    }

    pub fn upload_track() -> &'static RiseMapEndpoint {
        &UPLOAD_TRACK
    }

    pub fn get_track_tiles() -> &'static RiseMapEndpoint {
        &GET_TRACK_TILES
    }

    pub fn get_track_summary() -> &'static RiseMapEndpoint {
        &GET_TRACK_SUMMARY
    }

    pub fn erase_tracks() -> &'static RiseMapEndpoint {
        &ERASE_TRACKS
    }

    pub fn delete_all_tracks() -> &'static RiseMapEndpoint {
        &DELETE_ALL_TRACKS
    }

    pub fn preview_track_merge() -> &'static RiseMapEndpoint {
        &PREVIEW_TRACK_MERGE
    }

    pub fn get_places() -> &'static RiseMapEndpoint {
        &GET_PLACES
    }

    pub fn set_place() -> &'static RiseMapEndpoint {
        &SET_PLACE
    }

    pub fn delete_place() -> &'static RiseMapEndpoint {
        &DELETE_PLACE
    }

    pub fn create_photo() -> &'static RiseMapEndpoint {
        &CREATE_PHOTO
    }

    pub fn get_photos() -> &'static RiseMapEndpoint {
        &GET_PHOTOS
    }

    pub fn delete_photo() -> &'static RiseMapEndpoint {
        &DELETE_PHOTO
    }

    pub fn get_photo_grids() -> &'static RiseMapEndpoint {
        &GET_PHOTO_GRIDS
    }

    pub fn get_grid_photos() -> &'static RiseMapEndpoint {
        &GET_GRID_PHOTOS
    }

    pub fn delete_photo_grid() -> &'static RiseMapEndpoint {
        &DELETE_PHOTO_GRID
    }

    pub fn get_privacy() -> &'static RiseMapEndpoint {
        &GET_PRIVACY
    }

    pub fn update_privacy() -> &'static RiseMapEndpoint {
        &UPDATE_PRIVACY
    }

    pub fn set_privacy_exceptions() -> &'static RiseMapEndpoint {
        &SET_PRIVACY_EXCEPTIONS
    }
}

#[derive(Clone)]
pub struct RiseMapRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseMapRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    pub async fn get_state(&self) -> anyhow::Result<MapState> {
        let response: rise_canon::rise_response::map::GetState =
            self.call(RiseMapEndpoints::get_state(), &rise_canon::rise_request::map::GetState {}).await?;
        Ok(response.model()?)
    }

    pub async fn join(&self) -> anyhow::Result<MapState> {
        let response: rise_canon::rise_response::map::Join =
            self.call(RiseMapEndpoints::join(), &rise_canon::rise_request::map::Join {}).await?;
        Ok(response.model()?)
    }

    pub async fn leave(&self) -> anyhow::Result<MapState> {
        let response: rise_canon::rise_response::map::Leave =
            self.call(RiseMapEndpoints::leave(), &rise_canon::rise_request::map::Leave {}).await?;
        Ok(response.model()?)
    }

    pub async fn update_settings(&self, patch: &MapSettingsPatch) -> anyhow::Result<MapState> {
        let response: rise_canon::rise_response::map::UpdateSettings = self
            .call(
                RiseMapEndpoints::update_settings(),
                &<rise_canon::rise_request::map::UpdateSettings as UpdateSettingsExt>::from_patch(patch),
            )
            .await?;
        Ok(response.model()?)
    }

    pub async fn get_profile(&self, owner_id: &str) -> anyhow::Result<MapProfile> {
        let response: rise_canon::rise_response::map::GetProfile = self
            .call(
                RiseMapEndpoints::get_profile(),
                &rise_canon::rise_request::map::GetProfile { owner_id: owner_id.to_owned() },
            )
            .await?;
        Ok(response.model()?)
    }

    pub async fn get_profile_friends(
        &self,
        owner_id: &str,
        limit: i64,
        cursor: Option<&str>,
    ) -> anyhow::Result<rise_canon::rise_response::map::GetFriends> {
        self.call(
            RiseMapEndpoints::get_friends(),
            &rise_canon::rise_request::map::GetFriends {
                cursor: cursor.map(str::to_owned),
                limit: Some(limit),
                owner_id: Some(owner_id.to_owned()),
            },
        )
        .await
    }

    pub async fn update_photo_grid(
        &self,
        body: rise_canon::rise_request::map::UpdatePhotoGrid,
    ) -> anyhow::Result<MapPhotoGrid> {
        let result: rise_canon::rise_response::map::UpdatePhotoGrid =
            self.call(RiseMapEndpoints::update_photo_grid(), &body).await?;
        let Some(grid) = result.grid.as_ref().and_then(|grid| grid.model()) else {
            return Err(RiseMapError::MissingActiveSession.into());
        };
        Ok(grid)
    }

    pub async fn get_friends(
        &self,
        limit: i64,
        cursor: Option<&str>,
    ) -> anyhow::Result<rise_canon::rise_response::map::GetFriends> {
        self.call(
            RiseMapEndpoints::get_friends(),
            &rise_canon::rise_request::map::GetFriends {
                cursor: cursor.map(str::to_owned),
                limit: Some(limit),
                owner_id: None,
            },
        )
        .await
    }

    pub async fn get_friend_requests(
        &self,
        direction: MapRequestDirection,
        limit: i64,
        cursor: Option<&str>,
    ) -> anyhow::Result<rise_canon::rise_response::map::GetFriendRequests> {
        self.call(
            RiseMapEndpoints::get_friend_requests(),
            &rise_canon::rise_request::map::GetFriendRequests {
                cursor: cursor.map(str::to_owned),
                direction: Some(direction.raw_value().to_owned()),
                limit: Some(limit),
            },
        )
        .await
    }

    pub async fn send_friend_request(&self, target_user_id: &str) -> anyhow::Result<MapFriendRequestOutcome> {
        let response: rise_canon::rise_response::map::SendFriendRequest = self
            .call(
                RiseMapEndpoints::send_friend_request(),
                &rise_canon::rise_request::map::SendFriendRequest { target_user_id: target_user_id.to_owned() },
            )
            .await?;
        Ok(response.model()?)
    }

    pub async fn respond_friend_request(
        &self,
        request_id: &str,
        accept: bool,
    ) -> anyhow::Result<MapFriendRequestOutcome> {
        let response: rise_canon::rise_response::map::RespondFriendRequest = self
            .call(
                RiseMapEndpoints::respond_friend_request(),
                &rise_canon::rise_request::map::RespondFriendRequest { accept, request_id: request_id.to_owned() },
            )
            .await?;
        Ok(response.model()?)
    }

    pub async fn cancel_friend_request(&self, request_id: &str) -> anyhow::Result<()> {
        let _: rise_canon::rise_response::map::CancelFriendRequest = self
            .call(
                RiseMapEndpoints::cancel_friend_request(),
                &rise_canon::rise_request::map::CancelFriendRequest { request_id: request_id.to_owned() },
            )
            .await?;
        Ok(())
    }

    pub async fn remove_friend(&self, user_id: &str) -> anyhow::Result<()> {
        let _: rise_canon::rise_response::map::RemoveFriend = self
            .call(
                RiseMapEndpoints::remove_friend(),
                &rise_canon::rise_request::map::RemoveFriend { friend_user_id: user_id.to_owned() },
            )
            .await?;
        Ok(())
    }

    pub async fn get_invite(&self) -> anyhow::Result<MapInvite> {
        let response: rise_canon::rise_response::map::GetInvite =
            self.call(RiseMapEndpoints::get_invite(), &rise_canon::rise_request::map::GetInvite {}).await?;
        Ok(response.model()?)
    }

    pub async fn reset_invite(&self) -> anyhow::Result<MapInvite> {
        let response: rise_canon::rise_response::map::ResetInvite =
            self.call(RiseMapEndpoints::reset_invite(), &rise_canon::rise_request::map::ResetInvite {}).await?;
        Ok(response.model()?)
    }

    pub async fn preview_invite(&self, token: &str) -> anyhow::Result<MapInvitePreview> {
        let response: rise_canon::rise_response::map::PreviewInvite = self
            .call(
                RiseMapEndpoints::preview_invite(),
                &rise_canon::rise_request::map::PreviewInvite { token: token.to_owned() },
            )
            .await?;
        Ok(response.model()?)
    }

    pub async fn use_invite(&self, token: &str) -> anyhow::Result<MapFriendRequestOutcome> {
        let response: rise_canon::rise_response::map::UseInvite = self
            .call(RiseMapEndpoints::use_invite(), &rise_canon::rise_request::map::UseInvite { token: token.to_owned() })
            .await?;
        Ok(response.model()?)
    }

    pub async fn open_session(&self) -> anyhow::Result<rise_canon::rise_response::map::OpenSession> {
        self.call(RiseMapEndpoints::open_session(), &rise_canon::rise_request::map::OpenSession {}).await
    }

    pub async fn heartbeat(&self) -> anyhow::Result<rise_canon::rise_response::map::Heartbeat> {
        self.call(RiseMapEndpoints::heartbeat(), &rise_canon::rise_request::map::Heartbeat {}).await
    }

    pub async fn close_session(&self) -> anyhow::Result<()> {
        let _: rise_canon::rise_response::map::CloseSession =
            self.call(RiseMapEndpoints::close_session(), &rise_canon::rise_request::map::CloseSession {}).await?;
        Ok(())
    }

    pub async fn update_location(&self, fix: &MapFix) -> anyhow::Result<rise_canon::rise_response::map::UpdateLocation> {
        self.call(
            RiseMapEndpoints::update_location(),
            &rise_canon::rise_request::map::UpdateLocation {
                availability_recorded_at: None,
                fix: Some(<rise_canon::rise_request::map::update_location::Fix as UpdateLocationFixExt>::from_fix(fix)),
                location_available: None,
            },
        )
        .await
    }

    pub async fn update_location_availability(
        &self,
        availability: MapLocationAvailability,
    ) -> anyhow::Result<rise_canon::rise_response::map::UpdateLocation> {
        self.call(
            RiseMapEndpoints::update_location(),
            &rise_canon::rise_request::map::UpdateLocation {
                availability_recorded_at: Some(availability.recorded_at),
                fix: None,
                location_available: Some(availability.available),
            },
        )
        .await
    }

    pub async fn get_reaction_inbox(&self) -> anyhow::Result<rise_canon::rise_response::map::GetReactionInbox> {
        self.call(
            RiseMapEndpoints::get_reaction_inbox(),
            &rise_canon::rise_request::map::GetReactionInbox { after_id: None },
        )
        .await
    }

    pub async fn claim_reactions(&self, sender_id: &str) -> anyhow::Result<rise_canon::rise_response::map::ClaimReactions> {
        self.call(
            RiseMapEndpoints::claim_reactions(),
            &rise_canon::rise_request::map::ClaimReactions { after_id: None, sender_id: sender_id.to_owned() },
        )
        .await
    }

    pub async fn get_reaction_page(
        &self,
        sender_id: &str,
        claim_id: &str,
        after: Option<i64>,
    ) -> anyhow::Result<rise_canon::rise_response::map::GetReactionPage> {
        self.call(
            RiseMapEndpoints::get_reaction_page(),
            &rise_canon::rise_request::map::GetReactionPage {
                after_id: after,
                claim_id: claim_id.to_owned(),
                sender_id: sender_id.to_owned(),
            },
        )
        .await
    }

    pub async fn send_reaction(&self, target_user_id: &str, emoji: &str, count: i64) -> anyhow::Result<i64> {
        let response: rise_canon::rise_response::map::SendReaction = self
            .call(
                RiseMapEndpoints::send_reaction(),
                &rise_canon::rise_request::map::SendReaction {
                    count: Some(count),
                    emoji: emoji.to_owned(),
                    target_user_id: target_user_id.to_owned(),
                },
            )
            .await?;
        Ok(response.sent_at)
    }

    pub async fn upload_track(
        &self,
        segments: &[MapSealedSegment],
        stays: &[MapRecordedStay],
    ) -> anyhow::Result<rise_canon::rise_response::map::UploadTrack> {
        self.call(
            RiseMapEndpoints::upload_track(),
            &rise_canon::rise_request::map::UploadTrack {
                segments: Some(segments.iter().map(MapSealedSegment::canon_json).collect()),
                stays: Some(stays.iter().map(MapRecordedStay::canon_json).collect()),
            },
        )
        .await
    }

    pub async fn get_track_tiles(
        &self,
        owner_id: Option<&str>,
        tiles: &[MapTileKey],
        since_day: i32,
        known_revision: i64,
    ) -> anyhow::Result<rise_canon::rise_response::map::GetTrackTiles> {
        self.call(
            RiseMapEndpoints::get_track_tiles(),
            &rise_canon::rise_request::map::GetTrackTiles {
                known_revision: Some(known_revision),
                owner_id: owner_id.map(str::to_owned),
                since_day: Some(i64::from(since_day)),
                tiles: Some(tiles.iter().map(MapTileKey::canon_json).collect()),
            },
        )
        .await
    }

    pub async fn get_track_summary(&self, owner_id: Option<&str>, today: i32) -> anyhow::Result<MapTrackSummary> {
        let response: rise_canon::rise_response::map::GetTrackSummary = self
            .call(
                RiseMapEndpoints::get_track_summary(),
                &rise_canon::rise_request::map::GetTrackSummary {
                    owner_id: owner_id.map(str::to_owned),
                    today: Some(i64::from(today)),
                },
            )
            .await?;
        Ok(response.model())
    }

    pub async fn erase_tracks(
        &self,
        strokes: &[MapEraseStroke],
    ) -> anyhow::Result<rise_canon::rise_response::map::EraseTracks> {
        self.call(
            RiseMapEndpoints::erase_tracks(),
            &rise_canon::rise_request::map::EraseTracks {
                strokes: strokes
                    .iter()
                    .map(<rise_canon::rise_request::map::erase_tracks::StrokesElement as StrokesElementExt>::from_stroke)
                    .collect(),
            },
        )
        .await
    }

    pub async fn preview_track_merge(&self, request: &MapMergePreviewRequest) -> anyhow::Result<MapMergePreview> {
        let response: rise_canon::rise_response::map::PreviewTrackMerge = self
            .call(
                RiseMapEndpoints::preview_track_merge(),
                &<rise_canon::rise_request::map::PreviewTrackMerge as PreviewTrackMergeExt>::from_request(request),
            )
            .await?;
        let Some(preview) = response.model() else {
            return Err(RiseMapError::InvalidResponse.into());
        };
        Ok(preview)
    }

    pub async fn delete_all_tracks(&self) -> anyhow::Result<rise_canon::rise_response::map::DeleteAllTracks> {
        self.call(RiseMapEndpoints::delete_all_tracks(), &rise_canon::rise_request::map::DeleteAllTracks {}).await
    }

    pub async fn get_places(&self) -> anyhow::Result<Vec<MapPlace>> {
        let response: rise_canon::rise_response::map::GetPlaces =
            self.call(RiseMapEndpoints::get_places(), &rise_canon::rise_request::map::GetPlaces {}).await?;
        Ok(response.places.iter().filter_map(PlaceExt::model).collect())
    }

    pub async fn set_place(
        &self,
        id: Option<&str>,
        kind: MapPlaceKind,
        name: &str,
        latitude: f64,
        longitude: f64,
        radius_meters: i64,
    ) -> anyhow::Result<Option<MapPlace>> {
        let response: rise_canon::rise_response::map::SetPlace = self
            .call(
                RiseMapEndpoints::set_place(),
                &rise_canon::rise_request::map::SetPlace {
                    description: None,
                    emoji: None,
                    id: id.map(str::to_owned),
                    kind: kind.raw_value().to_owned(),
                    lat: latitude,
                    lng: longitude,
                    name: Some(name.to_owned()),
                    radius_m: Some(radius_meters),
                    record_tracks: None,
                    track_exclusion_radius_m: None,
                },
            )
            .await?;
        Ok(response.place.as_ref().and_then(PlaceExt::model))
    }

    pub async fn save_place(&self, place: &MapPlace, creating: bool) -> anyhow::Result<MapPlace> {
        let response: rise_canon::rise_response::map::SetPlace = self
            .call(
                RiseMapEndpoints::set_place(),
                &rise_canon::rise_request::map::SetPlace {
                    description: Some(place.description.clone()),
                    emoji: Some(place.emoji.clone()),
                    id: if creating { None } else { Some(place.id.clone()) },
                    kind: place.kind.raw_value().to_owned(),
                    lat: place.latitude,
                    lng: place.longitude,
                    name: Some(place.name.clone()),
                    radius_m: Some(place.radius_meters),
                    record_tracks: Some(place.record_tracks),
                    track_exclusion_radius_m: Some(place.track_exclusion_radius_meters),
                },
            )
            .await?;
        let Some(confirmed) = response.place.as_ref().and_then(PlaceExt::model) else {
            return Err(CancellationError.into());
        };
        Ok(confirmed)
    }

    pub async fn delete_place(&self, id: &str) -> anyhow::Result<()> {
        let _: rise_canon::rise_response::map::DeletePlace = self
            .call(
                RiseMapEndpoints::delete_place(),
                &rise_canon::rise_request::map::DeletePlace { place_id: id.to_owned() },
            )
            .await?;
        Ok(())
    }

    pub async fn create_photo(&self, draft: &MapPhotoDraft) -> anyhow::Result<Option<MapPhotoPlacement>> {
        let response: rise_canon::rise_response::map::CreatePhoto = self
            .call(
                RiseMapEndpoints::create_photo(),
                &<rise_canon::rise_request::map::CreatePhoto as CreatePhotoExt>::from_draft(draft),
            )
            .await?;
        let (Some(photo), Some(grid)) = (
            response.photo.as_ref().and_then(|photo| photo.model()),
            response.grid.as_ref().and_then(|grid| grid.model()),
        ) else {
            return Ok(None);
        };
        Ok(Some(MapPhotoPlacement { photo, grid }))
    }

    pub async fn get_photo_grids(
        &self,
        owner_id: Option<&str>,
        limit: i64,
        cursor: Option<&str>,
    ) -> anyhow::Result<rise_canon::rise_response::map::GetPhotoGrids> {
        self.call(
            RiseMapEndpoints::get_photo_grids(),
            &rise_canon::rise_request::map::GetPhotoGrids {
                cursor: cursor.map(str::to_owned),
                east: None,
                limit: Some(limit),
                north: None,
                owner_id: owner_id.map(str::to_owned),
                south: None,
                west: None,
            },
        )
        .await
    }

    pub async fn get_grid_photos(
        &self,
        grid_id: &str,
        limit: i64,
        cursor: Option<&str>,
    ) -> anyhow::Result<rise_canon::rise_response::map::GetGridPhotos> {
        self.call(
            RiseMapEndpoints::get_grid_photos(),
            &rise_canon::rise_request::map::GetGridPhotos {
                cursor: cursor.map(str::to_owned),
                grid_id: grid_id.to_owned(),
                limit: Some(limit),
            },
        )
        .await
    }

    pub async fn delete_photo_grid(&self, id: &str) -> anyhow::Result<bool> {
        let response: rise_canon::rise_response::map::DeletePhotoGrid = self
            .call(
                RiseMapEndpoints::delete_photo_grid(),
                &rise_canon::rise_request::map::DeletePhotoGrid { grid_id: id.to_owned() },
            )
            .await?;
        Ok(response.has_more)
    }

    pub async fn get_photos(
        &self,
        owner_id: Option<&str>,
        limit: i64,
        cursor: Option<&str>,
    ) -> anyhow::Result<rise_canon::rise_response::map::GetPhotos> {
        self.call(
            RiseMapEndpoints::get_photos(),
            &rise_canon::rise_request::map::GetPhotos {
                cursor: cursor.map(str::to_owned),
                east: None,
                limit: Some(limit),
                north: None,
                owner_id: owner_id.map(str::to_owned),
                south: None,
                west: None,
            },
        )
        .await
    }

    pub async fn delete_photo(&self, id: &str) -> anyhow::Result<MapPhotoRemoval> {
        let response: rise_canon::rise_response::map::DeletePhoto = self
            .call(
                RiseMapEndpoints::delete_photo(),
                &rise_canon::rise_request::map::DeletePhoto { photo_id: id.to_owned() },
            )
            .await?;
        Ok(MapPhotoRemoval {
            grid: response.grid.as_ref().and_then(|grid| grid.model()),
            removed_grid_id: if response.removed_grid_id.is_empty() { None } else { Some(response.removed_grid_id) },
        })
    }

    pub async fn get_privacy(&self) -> anyhow::Result<MapPrivacy> {
        let response: rise_canon::rise_response::map::GetPrivacy =
            self.call(RiseMapEndpoints::get_privacy(), &rise_canon::rise_request::map::GetPrivacy {}).await?;
        Ok(response.model()?)
    }

    pub async fn update_privacy(&self, kind: MapPrivacyKind, rule: MapPrivacyRule) -> anyhow::Result<MapPrivacy> {
        let rule_value = |target: MapPrivacyKind| (kind == target).then(|| rule.wire().to_owned());
        let response: rise_canon::rise_response::map::UpdatePrivacy = self
            .call(
                RiseMapEndpoints::update_privacy(),
                &rise_canon::rise_request::map::UpdatePrivacy {
                    friends_rule: rule_value(MapPrivacyKind::Friends),
                    media_rule: rule_value(MapPrivacyKind::Media),
                    presence_rule: rule_value(MapPrivacyKind::Presence),
                    tracks_rule: rule_value(MapPrivacyKind::Tracks),
                },
            )
            .await?;
        Ok(response.model()?)
    }

    pub async fn set_privacy_exceptions(
        &self,
        kind: MapPrivacyKind,
        action: MapPrivacyAction,
        user_ids: Vec<String>,
    ) -> anyhow::Result<MapPrivacy> {
        let response: rise_canon::rise_response::map::SetPrivacyExceptions = self
            .call(
                RiseMapEndpoints::set_privacy_exceptions(),
                &rise_canon::rise_request::map::SetPrivacyExceptions {
                    action: action.raw_value().to_owned(),
                    kind: kind.raw_value().to_owned(),
                    target_user_ids: Some(user_ids.into_iter().map(RiseRequestJSON::String).collect()),
                },
            )
            .await?;
        Ok(response.model()?)
    }

    pub async fn prepare_journey(
        &self,
        selection: &MapJourneySelection,
    ) -> anyhow::Result<rise_canon::rise_response::map::PrepareJourney> {
        self.call(
            RiseMapEndpoints::prepare_journey(),
            &rise_canon::rise_request::map::PrepareJourney {
                from_ms: selection.from_ms,
                grid_ids: selection.grid_ids.clone(),
                id: selection.id.clone(),
                polygon: selection
                    .polygon
                    .iter()
                    .map(|point| rise_canon::rise_request::map::prepare_journey::PolygonElement {
                        lat: Some(point.lat),
                        lng: Some(point.lng),
                    })
                    .collect(),
                until_ms: selection.until_ms,
            },
        )
        .await
    }

    pub async fn get_journey(&self, post_id: i64) -> anyhow::Result<rise_canon::rise_response::map::GetJourney> {
        self.call(RiseMapEndpoints::get_journey(), &rise_canon::rise_request::map::GetJourney { post_id }).await
    }

    pub async fn revoke_journey(&self, id: &str) -> anyhow::Result<rise_canon::rise_response::map::RevokeJourney> {
        self.call(
            RiseMapEndpoints::revoke_journey(),
            &rise_canon::rise_request::map::RevokeJourney { id: id.to_owned() },
        )
        .await
    }

    async fn call<Request, Response>(&self, endpoint: &RiseMapEndpoint, request: &Request) -> anyhow::Result<Response>
    where
        Request: Serialize + Sync,
        Response: DeserializeOwned,
    {
        self.engine
            .call_descriptor_response(&endpoint.descriptor(), request, None, RiseWireRequestMetadata::default())
            .await
    }
}
