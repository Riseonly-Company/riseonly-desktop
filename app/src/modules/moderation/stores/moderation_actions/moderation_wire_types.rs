use rise_canon::rise_proto::report::{
    ProtoGetModeratorStatsResponse, ProtoModeratorProfile, ProtoModeratorProfileWithUser, ProtoModeratorStats,
    ProtoReportWithAuthor, ProtoUserAuthorInfo,
};
use rise_canon::rise_response::report::{GetAllModerators, GetModeratorHistory};
use rise_core::foundation::foundation_uuid::Uuid;

use super::moderation_types::{
    ModeratorKind, ModeratorPermissions, ModeratorScopeType, ModeratorStatFormatting, ReportReason, ReportStatus,
    ReportTargetType, ReportTargetTypeExt,
};

pub type ReportAuthorWireDTO = ProtoUserAuthorInfo;

pub type ReportQueueWireDTO = ProtoReportWithAuthor;

pub trait ProtoReportWithAuthorExt {
    fn resolved_status(&self) -> ReportStatus;
    fn resolved_reason(&self) -> Option<ReportReason>;
    fn resolved_type(&self) -> ReportTargetType;
    fn resolved_scope(&self) -> Option<ModeratorScopeType>;
    fn is_closed(&self) -> bool;
}

impl ProtoReportWithAuthorExt for ProtoReportWithAuthor {
    fn resolved_status(&self) -> ReportStatus {
        ReportStatus::from_wire(&self.status.to_uppercase()).unwrap_or(ReportStatus::Active)
    }

    fn resolved_reason(&self) -> Option<ReportReason> {
        ReportReason::from_wire(&self.reason.to_uppercase())
    }

    fn resolved_type(&self) -> ReportTargetType {
        ReportTargetType::from_wire(&self.report_type.to_uppercase()).unwrap_or(ReportTargetType::Other)
    }

    fn resolved_scope(&self) -> Option<ModeratorScopeType> {
        self.resolved_type().moderator_scope()
    }

    fn is_closed(&self) -> bool {
        match self.resolved_status() {
            ReportStatus::Active => false,
            ReportStatus::Resolved | ReportStatus::Rejected | ReportStatus::Appealed => true,
        }
    }
}

pub type ReportsQueueGatewayDTO = GetModeratorHistory;
pub type ModeratorProfileInnerDTO = ProtoModeratorProfile;

pub type ModeratorProfileGatewayDTO = ProtoModeratorProfileWithUser;

pub type ModeratorWithUserWireDTO = ProtoModeratorProfileWithUser;

pub trait ProtoModeratorProfileWithUserExt {
    fn id(&self) -> String;
    fn target_user_id(&self) -> Option<String>;
    fn resolved_kind(&self) -> Option<ModeratorKind>;
    fn is_active(&self) -> bool;
    fn permissions(&self) -> ModeratorPermissions;
    fn overturn_rate_percent(&self) -> String;
}

impl ProtoModeratorProfileWithUserExt for ProtoModeratorProfileWithUser {
    fn id(&self) -> String {
        self.profile
            .as_ref()
            .map(|profile| profile.user_id.clone())
            .or_else(|| self.user.as_ref().map(|user| user.tag.clone()))
            .unwrap_or_else(|| Uuid::new_v4().uuid_string())
    }

    fn target_user_id(&self) -> Option<String> {
        self.profile.as_ref().map(|profile| profile.user_id.clone())
    }

    fn resolved_kind(&self) -> Option<ModeratorKind> {
        let kind = self.profile.as_ref().map(|profile| profile.kind.as_str()).unwrap_or("");
        ModeratorKind::from_wire(&kind.to_uppercase())
    }

    fn is_active(&self) -> bool {
        self.profile.as_ref().map(|profile| profile.is_active).unwrap_or(false)
    }

    fn permissions(&self) -> ModeratorPermissions {
        ModeratorPermissions::from_json_string(self.profile.as_ref().map(|profile| profile.permissions_json.as_str()))
    }

    fn overturn_rate_percent(&self) -> String {
        ModeratorStatFormatting::percent(Some(self.overturn_rate))
    }
}

pub type AllModeratorsGatewayDTO = GetAllModerators;

pub type ModeratorStatsWireDTO = ProtoModeratorStats;

pub type ModeratorStatsGatewayDTO = ProtoGetModeratorStatsResponse;
