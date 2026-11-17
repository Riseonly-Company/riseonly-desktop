use rise_canon::{rise_proto, rise_request, rise_view};

use super::map_types::{
    MapFix, MapFriend, MapFriendPosition, MapFriendRequest, MapFriendRequestOutcome, MapInvite,
    MapInvitePreview, MapInviteRelation, MapLiveFriend, MapMotion, MapRequestStatus, MapSettings,
    MapSettingsPatch, MapState, MapUser, MapVenueKind,
};
use crate::modules::map::engine::rise_map_engine_models::RiseMapError;

pub trait UpdateSettingsExt {
    fn from_patch(patch: &MapSettingsPatch) -> Self;
}

impl UpdateSettingsExt for rise_request::map::UpdateSettings {
    fn from_patch(patch: &MapSettingsPatch) -> Self {
        Self {
            battery_visible: patch.battery_visible,
            frozen_until: patch.frozen_until,
            home_track_exclusion_radius_m: patch.home_track_exclusion_radius_m.map(MapSettings::clamped_home_track_radius),
            sharing_enabled: patch.sharing_enabled,
            track_cooling_days: patch.track_cooling_days,
            track_heat_walks: patch.track_heat_walks,
            track_merge_enabled: patch.track_merge_enabled,
            track_merge_lanes: patch.track_merge_lanes.map(MapSettings::clamped_merge_lanes),
            track_merge_radius_m: patch.track_merge_radius_m.map(MapSettings::clamped_merge_radius),
            track_precise_recording: patch.track_precise_recording,
            tracks_enabled: patch.tracks_enabled,
        }
    }
}

pub trait UpdateLocationFixExt {
    fn from_fix(fix: &MapFix) -> Self;
}

impl UpdateLocationFixExt for rise_request::map::update_location::Fix {
    fn from_fix(fix: &MapFix) -> Self {
        Self {
            accuracy_m: Some(fix.accuracy_meters as f32),
            battery_level: Some(fix.battery_level),
            heading_deg: Some(fix.heading_degrees as f32),
            is_charging: Some(fix.is_charging),
            lat: Some(fix.latitude),
            lng: Some(fix.longitude),
            motion: Some(fix.motion.raw_value().to_owned()),
            place_kind: Some(fix.place_kind.map(MapVenueKind::raw_value).unwrap_or_default().to_owned()),
            recorded_at: Some(fix.recorded_at),
            speed_mps: Some(fix.speed_meters_per_second as f32),
            stay_since: Some(fix.stay_since),
        }
    }
}

pub trait MapNonEmpty {
    fn map_non_empty(&self) -> Option<String>;
}

impl MapNonEmpty for Option<String> {
    fn map_non_empty(&self) -> Option<String> {
        self.as_ref().filter(|value| !value.is_empty()).cloned()
    }
}

pub trait ProtoMapUserExt {
    fn model(&self) -> Option<MapUser>;
}

impl ProtoMapUserExt for rise_proto::map::ProtoMapUser {
    fn model(&self) -> Option<MapUser> {
        if self.id.is_empty() {
            return None;
        }
        Some(MapUser {
            id: self.id.clone(),
            tag: self.tag.clone(),
            name: self.name.clone(),
            logo: self.logo.clone(),
            is_premium: self.is_premium,
            is_official: self.is_official,
            name_color: self.name_color.map_non_empty(),
            premium_badge_emoji: self.premium_badge_emoji.map_non_empty(),
            is_deleted: self.is_deleted,
        })
    }
}

pub trait ProtoMapSettingsExt {
    fn model(&self) -> MapSettings;
}

impl ProtoMapSettingsExt for rise_proto::map::ProtoMapSettings {
    fn model(&self) -> MapSettings {
        MapSettings::new(
            self.sharing_enabled,
            self.tracks_enabled,
            self.battery_visible,
            self.frozen_until,
            if self.track_cooling_days > 0 { self.track_cooling_days } else { MapSettings::default_cooling_days() },
            if self.track_heat_walks > 0 { self.track_heat_walks } else { MapSettings::default_heat_walks() },
            if self.home_track_exclusion_radius_m > 0 {
                MapSettings::clamped_home_track_radius(self.home_track_exclusion_radius_m)
            } else {
                MapSettings::default_home_track_radius()
            },
            self.track_merge_enabled,
            if self.track_merge_radius_m > 0 {
                MapSettings::clamped_merge_radius(self.track_merge_radius_m)
            } else {
                MapSettings::default_merge_radius()
            },
            if self.track_merge_lanes > 0 {
                MapSettings::clamped_merge_lanes(self.track_merge_lanes)
            } else {
                MapSettings::default_merge_lanes()
            },
            self.track_precise_recording,
        )
    }
}

pub trait ProtoMapStateExt {
    fn model(&self) -> MapState;
}

impl ProtoMapStateExt for rise_proto::map::ProtoMapState {
    fn model(&self) -> MapState {
        MapState {
            is_member: self.is_member,
            joined_at: self.joined_at,
            settings: self.settings.as_ref().map(ProtoMapSettingsExt::model).unwrap_or_else(MapSettings::initial),
            friends_count: self.friends_count.max(0),
            incoming_requests_count: self.incoming_requests_count.max(0),
        }
    }
}

pub trait ProtoStateResponseExt {
    fn model(&self) -> Result<MapState, RiseMapError>;
}

impl ProtoStateResponseExt for rise_proto::map::ProtoStateResponse {
    fn model(&self) -> Result<MapState, RiseMapError> {
        let Some(state) = self.state.as_ref() else {
            return Err(RiseMapError::InvalidResponse);
        };
        Ok(state.model())
    }
}

pub trait ProtoMapFriendExt {
    fn model(&self) -> Option<MapFriend>;
}

impl ProtoMapFriendExt for rise_proto::map::ProtoMapFriend {
    fn model(&self) -> Option<MapFriend> {
        let user = self.user.as_ref().and_then(ProtoMapUserExt::model)?;
        Some(MapFriend { user, since: self.since })
    }
}

pub trait ProtoMapFriendRequestExt {
    fn model(&self) -> Option<MapFriendRequest>;
}

impl ProtoMapFriendRequestExt for rise_proto::map::ProtoMapFriendRequest {
    fn model(&self) -> Option<MapFriendRequest> {
        if self.id.is_empty() {
            return None;
        }
        let from_user = self.from_user.as_ref().and_then(ProtoMapUserExt::model)?;
        let to_user = self.to_user.as_ref().and_then(ProtoMapUserExt::model)?;
        Some(MapFriendRequest {
            id: self.id.clone(),
            from_user,
            to_user,
            status: MapRequestStatus::from_wire(&self.status),
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

pub trait ProtoFriendRequestResponseExt {
    fn model(&self) -> Result<MapFriendRequestOutcome, RiseMapError>;
}

impl ProtoFriendRequestResponseExt for rise_proto::map::ProtoFriendRequestResponse {
    fn model(&self) -> Result<MapFriendRequestOutcome, RiseMapError> {
        let Some(request) = self.request.as_ref().and_then(ProtoMapFriendRequestExt::model) else {
            return Err(RiseMapError::InvalidResponse);
        };
        Ok(MapFriendRequestOutcome {
            request,
            friend: self.friend.as_ref().and_then(ProtoMapFriendExt::model),
            target_is_member: self.target_is_member,
        })
    }
}

pub trait FriendAnsweredExt {
    fn model(&self) -> Result<MapFriendRequestOutcome, RiseMapError>;
}

impl FriendAnsweredExt for rise_view::map::FriendAnswered {
    fn model(&self) -> Result<MapFriendRequestOutcome, RiseMapError> {
        let Some(request) = self.request.as_ref().and_then(ProtoMapFriendRequestExt::model) else {
            return Err(RiseMapError::InvalidResponse);
        };
        Ok(MapFriendRequestOutcome {
            request,
            friend: self.friend.as_ref().and_then(ProtoMapFriendExt::model),
            target_is_member: true,
        })
    }
}

pub trait ProtoMapInviteExt {
    fn model(&self) -> Option<MapInvite>;
}

impl ProtoMapInviteExt for rise_proto::map::ProtoMapInvite {
    fn model(&self) -> Option<MapInvite> {
        if self.token.is_empty() {
            return None;
        }
        Some(MapInvite { token: self.token.clone(), created_at: self.created_at, uses: self.uses.max(0) })
    }
}

pub trait ProtoInviteResponseExt {
    fn model(&self) -> Result<MapInvite, RiseMapError>;
}

impl ProtoInviteResponseExt for rise_proto::map::ProtoInviteResponse {
    fn model(&self) -> Result<MapInvite, RiseMapError> {
        self.invite.as_ref().and_then(ProtoMapInviteExt::model).ok_or(RiseMapError::InvalidResponse)
    }
}

pub trait ProtoInvitePreviewResponseExt {
    fn model(&self) -> Result<MapInvitePreview, RiseMapError>;
}

impl ProtoInvitePreviewResponseExt for rise_proto::map::ProtoInvitePreviewResponse {
    fn model(&self) -> Result<MapInvitePreview, RiseMapError> {
        let Some(owner) = self.owner.as_ref().and_then(ProtoMapUserExt::model) else {
            return Err(RiseMapError::InvalidResponse);
        };
        Ok(MapInvitePreview {
            owner,
            relation: MapInviteRelation::from_wire(&self.relation),
            viewer_is_member: self.viewer_is_member,
            request_id: self.request_id.map_non_empty(),
        })
    }
}

pub trait ProtoMapFixExt {
    fn model(&self) -> Option<MapFix>;
}

impl ProtoMapFixExt for rise_proto::map::ProtoMapFix {
    fn model(&self) -> Option<MapFix> {
        if !self.lat.is_finite() || !self.lng.is_finite() || self.lat.abs() > 90.0 || self.lng.abs() > 180.0 {
            return None;
        }
        Some(MapFix {
            latitude: self.lat,
            longitude: self.lng,
            accuracy_meters: f64::from(self.accuracy_m),
            speed_meters_per_second: f64::from(self.speed_mps),
            heading_degrees: f64::from(self.heading_deg),
            battery_level: self.battery_level,
            is_charging: self.is_charging,
            motion: MapMotion::from_wire(&self.motion),
            recorded_at: self.recorded_at,
            stay_since: self.stay_since,
            place_kind: MapVenueKind::from_raw_value(&self.place_kind),
        })
    }
}

pub trait ProtoFriendPositionExt {
    fn model(&self) -> Option<MapFriendPosition>;
}

impl ProtoFriendPositionExt for rise_proto::map::ProtoFriendPosition {
    fn model(&self) -> Option<MapFriendPosition> {
        if self.user_id.is_empty() {
            return None;
        }
        let fix = self.fix.as_ref().and_then(ProtoMapFixExt::model)?;
        Some(MapFriendPosition {
            user_id: self.user_id.clone(),
            fix,
            at_home: self.at_home,
            location_unavailable_since: Some(self.location_unavailable_since),
            availability_recorded_at: Some(self.availability_recorded_at),
        })
    }
}

pub trait ProtoLiveFriendExt {
    fn model(&self) -> Option<MapLiveFriend>;
}

impl ProtoLiveFriendExt for rise_proto::map::ProtoLiveFriend {
    fn model(&self) -> Option<MapLiveFriend> {
        let user = self.user.as_ref().and_then(ProtoMapUserExt::model)?;
        Some(MapLiveFriend { user, position: self.position.as_ref().and_then(ProtoFriendPositionExt::model) })
    }
}
