use rise_canon::rise_proto::report::{
    ProtoAdminSanctionResponse, ProtoAppeal, ProtoAppealCase, ProtoAppealReviewItem, ProtoAppealWithContext,
    ProtoAutoStanding, ProtoCaseCapabilities, ProtoCaseContextMessage, ProtoCaseTimelineEntry, ProtoGetMyBlocksResponse,
    ProtoModerationNote, ProtoOffenseCounters, ProtoReport, ProtoReportCase, ProtoReportWithAuthor,
    ProtoSanctionHistoryItem, ProtoTargetActiveBlock, ProtoTargetIntel, ProtoTargetIpDossier, ProtoUserAuthorInfo,
    ProtoUserBlock, ProtoWarningState,
};
use rise_canon::rise_response::report::{GetAppeals, GetMyAppeals, GetMyReports, GetReports};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::moderation::engine::rise_moderation_wire::{
    AppealDTO, AppealItemDTO, AppealsPageDTO, ModerationUserDTO, ReportRowDTO, ReportsPageDTO, RiseStoredAdminSanction,
    RiseStoredAppealCase, RiseStoredAutoStanding, RiseStoredCaseCapabilities, RiseStoredCaseContextMessage,
    RiseStoredCaseTimelineEntry, RiseStoredModerationNote, RiseStoredMyBlocks, RiseStoredOffenseCounters,
    RiseStoredReportCase, RiseStoredSanctionHistoryItem, RiseStoredTargetActiveBlock, RiseStoredTargetIntel,
    RiseStoredTargetIpDossier, RiseStoredUserBlock, RiseStoredWarningState,
};

const PENDING_APPEAL_STATUS: &str = "PENDING";
const TEXT_CONTENT_TYPE: &str = "text";

pub trait AppealItemDTOCanon<Canon>: Sized {
    fn from_canon(canon: Canon) -> Option<Self>;
}

fn moderation_text(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

impl From<ProtoUserAuthorInfo> for ModerationUserDTO {
    fn from(user: ProtoUserAuthorInfo) -> Self {
        Self {
            user_id: None,
            name: Some(user.name),
            tag: Some(user.tag),
            logo: user.logo,
            role: Some(user.role),
            is_premium: user.is_premium,
            is_official: user.is_official,
        }
    }
}

fn moderation_user(user: Option<ProtoUserAuthorInfo>) -> Option<ModerationUserDTO> {
    user.map(ModerationUserDTO::from)
}

impl From<ProtoReportWithAuthor> for ReportRowDTO {
    fn from(row: ProtoReportWithAuthor) -> Self {
        ReportRowDTO::with_canon_id(
            row.id,
            row.created_at,
            row.created_at_ms,
            row.report_type,
            row.status,
            row.reason,
            row.description,
            row.decision,
            row.report_by_id,
            row.report_count,
            moderation_user(row.author),
            moderation_user(row.reported_user),
            moderation_user(row.resolver),
            row.resolved_by_id,
            row.resolved_at_ms,
            row.moderation_action,
            row.content_deleted.unwrap_or(false),
            row.block_type,
            row.block_duration_days,
            row.block_end_at,
            row.warning_decrement.unwrap_or(false),
            row.post_id,
            row.comment_id,
            row.chat_id,
            row.user_id,
            row.vacancy_id,
            row.message_id,
            row.track_id,
            row.artist_id,
            row.story_id,
            row.bot_id,
            row.target_user_id,
            row.snapshot_title,
            row.snapshot_text,
            row.snapshot_media,
            row.snapshot_json,
            row.evidence_file_urls,
            row.evidence_file_ids,
            row.evidence_count,
            row.has_context,
            row.reopened_at_ms,
            row.reopened_by_id,
        )
    }
}

impl From<ProtoReport> for ReportRowDTO {
    fn from(row: ProtoReport) -> Self {
        let evidence_count = i64::try_from(row.evidence_file_urls.len()).unwrap_or(i64::MAX);
        ReportRowDTO::with_canon_id(
            row.id,
            row.created_at,
            row.created_at_ms,
            row.report_type,
            row.status,
            row.reason,
            row.description,
            row.decision,
            row.report_by_id,
            1,
            None,
            None,
            None,
            row.resolved_by_id,
            row.resolved_at_ms,
            row.moderation_action,
            row.content_deleted.unwrap_or(false),
            row.block_type,
            row.block_duration_days,
            row.block_end_at,
            row.warning_decrement.unwrap_or(false),
            row.post_id,
            row.comment_id,
            row.chat_id,
            row.user_id,
            row.vacancy_id,
            row.message_id,
            row.track_id,
            row.artist_id,
            row.story_id,
            row.bot_id,
            row.target_user_id,
            row.snapshot_title,
            row.snapshot_text,
            row.snapshot_media,
            row.snapshot_json,
            row.evidence_file_urls,
            row.evidence_file_ids,
            evidence_count,
            false,
            row.reopened_at_ms,
            row.reopened_by_id,
        )
    }
}

impl From<ProtoAppeal> for AppealDTO {
    fn from(appeal: ProtoAppeal) -> Self {
        let status = if appeal.status.is_empty() { PENDING_APPEAL_STATUS } else { appeal.status.as_str() };
        Self {
            id: appeal.id,
            report_id: appeal.report_id.filter(|report_id| *report_id > 0),
            block_type: appeal
                .block_type
                .filter(|block_type| !block_type.is_empty())
                .map(|block_type| block_type.to_uppercase()),
            appellant_id: appeal.appellant_id,
            text: appeal.text,
            status: status.to_uppercase(),
            reviewed_by_id: appeal.reviewed_by_id,
            reviewed_at: appeal.reviewed_at,
            reviewed_at_ms: appeal.reviewed_at_ms,
            decision_text: appeal.decision_text,
            created_at: moderation_text(Some(appeal.created_at)),
            created_at_ms: Some(appeal.created_at_ms),
            evidence_file_urls: appeal.evidence_file_urls,
            evidence_file_ids: appeal.evidence_file_ids,
        }
    }
}

impl From<GetMyReports> for ReportsPageDTO {
    fn from(page: GetMyReports) -> Self {
        ReportsPageDTO::new(
            page.reports.into_iter().map(ReportRowDTO::from).collect(),
            page.total,
            page.relative_id,
            page.is_have_more_bot_or_top,
        )
    }
}

impl From<GetReports> for ReportsPageDTO {
    fn from(page: GetReports) -> Self {
        ReportsPageDTO::new(
            page.reports.into_iter().map(ReportRowDTO::from).collect(),
            page.total,
            page.relative_id,
            page.is_have_more_bot_or_top,
        )
    }
}

impl AppealItemDTOCanon<ProtoAppealWithContext> for AppealItemDTO {
    fn from_canon(item: ProtoAppealWithContext) -> Option<Self> {
        let appeal = item.appeal?;
        Some(Self {
            appeal: AppealDTO::from(appeal),
            report_type: moderation_text(Some(item.report_type)),
            report_reason: moderation_text(Some(item.report_reason)),
            snapshot_title: moderation_text(item.snapshot_title),
            snapshot_text: moderation_text(item.snapshot_text),
            original_decision: moderation_text(item.original_decision),
            block_type: moderation_text(item.block_type),
            resolved_by_id: None,
            appellant: None,
        })
    }
}

impl AppealItemDTOCanon<ProtoAppealReviewItem> for AppealItemDTO {
    fn from_canon(item: ProtoAppealReviewItem) -> Option<Self> {
        let appeal = item.appeal?;
        Some(Self {
            appeal: AppealDTO::from(appeal),
            report_type: moderation_text(Some(item.report_type)),
            report_reason: moderation_text(Some(item.report_reason)),
            snapshot_title: moderation_text(item.snapshot_title),
            snapshot_text: moderation_text(item.snapshot_text),
            original_decision: moderation_text(item.original_decision),
            block_type: moderation_text(item.block_type),
            resolved_by_id: moderation_text(item.resolved_by_id),
            appellant: moderation_user(item.appellant),
        })
    }
}

impl From<GetMyAppeals> for AppealsPageDTO {
    fn from(page: GetMyAppeals) -> Self {
        AppealsPageDTO::new(
            page.appeals.into_iter().filter_map(AppealItemDTO::from_canon).collect(),
            page.total,
            page.relative_id,
            page.is_have_more_bot_or_top,
        )
    }
}

impl From<GetAppeals> for AppealsPageDTO {
    fn from(page: GetAppeals) -> Self {
        AppealsPageDTO::new(
            page.appeals.into_iter().filter_map(AppealItemDTO::from_canon).collect(),
            page.total,
            page.relative_id,
            page.is_have_more_bot_or_top,
        )
    }
}

impl From<ProtoUserBlock> for RiseStoredUserBlock {
    fn from(block: ProtoUserBlock) -> Self {
        RiseStoredUserBlock::new(
            block.block_type.to_uppercase(),
            block.block_start_timestamp,
            block.block_end_timestamp,
            Some(block.reason),
            block.reason_code,
            block.report_id,
            block.report_type,
            block.report_reason,
            block.snapshot_title,
            block.can_appeal,
        )
    }
}

impl From<ProtoWarningState> for RiseStoredWarningState {
    fn from(state: ProtoWarningState) -> Self {
        RiseStoredWarningState::new(state.count, state.free_count, state.next_is_block, state.next_block_days)
    }
}

impl From<ProtoSanctionHistoryItem> for RiseStoredSanctionHistoryItem {
    fn from(item: ProtoSanctionHistoryItem) -> Self {
        Self {
            id: item.id,
            action: item.action.to_uppercase(),
            created_at_ms: item.created_at_ms,
            report_id: item.report_id,
            appeal_id: item.appeal_id,
            report_type: item.report_type,
            report_reason: item.report_reason,
            report_status: item.report_status,
            snapshot_title: item.snapshot_title,
            decision: item.decision,
            block_type: item.block_type,
            block_duration_days: item.block_duration_days,
            content_deleted: item.content_deleted,
        }
    }
}

impl From<ProtoGetMyBlocksResponse> for RiseStoredMyBlocks {
    fn from(reply: ProtoGetMyBlocksResponse) -> Self {
        RiseStoredMyBlocks::new(
            reply.blocks.into_iter().map(RiseStoredUserBlock::from).collect(),
            reply.warnings.map(RiseStoredWarningState::from).unwrap_or(RiseStoredWarningState::EMPTY),
            reply.history.into_iter().map(RiseStoredSanctionHistoryItem::from).collect(),
            reply.auto_standings.into_iter().map(RiseStoredAutoStanding::from).collect(),
        )
    }
}

impl From<ProtoAutoStanding> for RiseStoredAutoStanding {
    fn from(standing: ProtoAutoStanding) -> Self {
        RiseStoredAutoStanding::new(
            standing.scope,
            standing.channel,
            standing.strikes,
            standing.strikes_to_mute,
            standing.muted_until_ms,
            standing.next_mute_ms,
        )
    }
}

impl From<ProtoModerationNote> for RiseStoredModerationNote {
    fn from(note: ProtoModerationNote) -> Self {
        Self {
            id: note.id,
            report_id: note.report_id,
            author_id: note.author_id,
            text: note.text,
            created_at_ms: note.created_at_ms,
            author: moderation_user(note.author),
            evidence_file_urls: note.evidence_file_urls,
            evidence_file_ids: note.evidence_file_ids,
        }
    }
}

impl From<ProtoCaseTimelineEntry> for RiseStoredCaseTimelineEntry {
    fn from(entry: ProtoCaseTimelineEntry) -> Self {
        Self {
            id: entry.id,
            actor_id: entry.actor_id,
            actor_role: entry.actor_role.to_uppercase(),
            action: entry.action.to_uppercase(),
            report_id: entry.report_id,
            appeal_id: entry.appeal_id,
            target_user_id: entry.target_user_id,
            details_json: entry.details_json,
            created_at_ms: entry.created_at_ms,
            actor: moderation_user(entry.actor),
        }
    }
}

impl From<ProtoOffenseCounters> for RiseStoredOffenseCounters {
    fn from(counters: ProtoOffenseCounters) -> Self {
        Self {
            account_offenses: counters.account_offenses,
            comment_offenses: counters.comment_offenses,
            post_offenses: counters.post_offenses,
            chat_offenses: counters.chat_offenses,
            music_offenses: counters.music_offenses,
            warnings: counters.warnings,
        }
    }
}

impl From<ProtoTargetActiveBlock> for RiseStoredTargetActiveBlock {
    fn from(block: ProtoTargetActiveBlock) -> Self {
        Self {
            block_type: block.block_type.to_uppercase(),
            block_start_timestamp: block.block_start_timestamp,
            block_end_timestamp: block.block_end_timestamp,
            reason: block.reason,
            report_id: block.report_id,
            moderator_id: block.moderator_id,
        }
    }
}

impl From<ProtoTargetIpDossier> for RiseStoredTargetIpDossier {
    fn from(dossier: ProtoTargetIpDossier) -> Self {
        Self {
            ip: dossier.ip,
            first_seen_at_ms: dossier.first_seen_at_ms,
            last_seen_at_ms: dossier.last_seen_at_ms,
            observation_count: i64::try_from(dossier.observation_count).unwrap_or(i64::MAX),
            last_source: dossier.last_source,
            is_blocked: dossier.is_blocked,
            linked_account_count: dossier.linked_account_count,
        }
    }
}

impl From<ProtoTargetIntel> for RiseStoredTargetIntel {
    fn from(intel: ProtoTargetIntel) -> Self {
        Self {
            user_id: intel.user_id,
            user: moderation_user(intel.user),
            active_blocks: intel.active_blocks.into_iter().map(RiseStoredTargetActiveBlock::from).collect(),
            offense_counters: intel.offense_counters.map(RiseStoredOffenseCounters::from),
            reports_received_total: intel.reports_received_total,
            reports_received_open: intel.reports_received_open,
            sanctions_total: intel.sanctions_total,
            appeals_accepted: intel.appeals_accepted,
            reports_filed_total: intel.reports_filed_total,
            ip_dossiers: intel.ip_dossiers.into_iter().map(RiseStoredTargetIpDossier::from).collect(),
            ip_dossiers_available: intel.ip_dossiers_available,
        }
    }
}

impl From<ProtoCaseContextMessage> for RiseStoredCaseContextMessage {
    fn from(message: ProtoCaseContextMessage) -> Self {
        let message_id =
            if message.message_id.is_empty() { Uuid::new_v4().uuid_string() } else { message.message_id };
        let content_type =
            if message.content_type.is_empty() { TEXT_CONTENT_TYPE } else { message.content_type.as_str() };
        Self {
            message_id,
            sender_id: message.sender_id,
            content: message.content,
            content_type: content_type.to_lowercase(),
            created_at_ms: message.created_at_ms,
            is_reported: message.is_reported,
            sender: moderation_user(message.sender),
            media_type: message.media_type.map(|media_type| media_type.to_lowercase()),
            media_url: message.media_url,
            thumbnail_url: message.thumbnail_url,
            media_duration_s: message.media_duration_s,
            media_width: message.media_width,
            media_height: message.media_height,
            mime_type: message.mime_type,
            file_size: message.file_size,
            file_name: message.file_name,
            sticker_emoji: message.sticker_emoji,
        }
    }
}

impl From<ProtoCaseCapabilities> for RiseStoredCaseCapabilities {
    fn from(capabilities: ProtoCaseCapabilities) -> Self {
        Self {
            can_dismiss: capabilities.can_dismiss,
            can_warn: capabilities.can_warn,
            can_delete_content: capabilities.can_delete_content,
            can_block_author: capabilities.can_block_author,
            can_mute_author: capabilities.can_mute_author,
            can_note: capabilities.can_note,
            can_reopen: capabilities.can_reopen,
            can_admin_sanction: capabilities.can_admin_sanction,
            can_resolve_appeals: capabilities.can_resolve_appeals,
            is_admin: capabilities.is_admin,
            max_block_days: capabilities.max_block_days,
            delete_threshold_met: capabilities.delete_threshold_met,
            delete_threshold: capabilities.delete_threshold,
        }
    }
}

impl RiseStoredReportCase {
    pub fn from_canon(report_case: ProtoReportCase) -> Option<Self> {
        let report = report_case.report?;
        let siblings = report_case.siblings.into_iter().map(ReportRowDTO::from).collect();
        Some(RiseStoredReportCase::new(
            ReportRowDTO::from(report),
            siblings,
            report_case.sibling_total,
            report_case.target.map(RiseStoredTargetIntel::from),
            report_case.timeline.into_iter().map(RiseStoredCaseTimelineEntry::from).collect(),
            report_case.notes.into_iter().map(RiseStoredModerationNote::from).collect(),
            report_case.appeals.into_iter().map(AppealDTO::from).collect(),
            report_case.context.into_iter().map(RiseStoredCaseContextMessage::from).collect(),
            report_case
                .capabilities
                .map(RiseStoredCaseCapabilities::from)
                .unwrap_or(RiseStoredCaseCapabilities::NONE),
        ))
    }
}

impl RiseStoredAppealCase {
    pub fn from_canon(appeal_case: ProtoAppealCase) -> Option<Self> {
        let appeal = appeal_case.appeal?;
        let report_case = appeal_case.report_case.and_then(RiseStoredReportCase::from_canon);
        let capabilities = appeal_case
            .capabilities
            .map(RiseStoredCaseCapabilities::from)
            .or_else(|| report_case.as_ref().map(|report_case| report_case.capabilities.clone()))
            .unwrap_or(RiseStoredCaseCapabilities::NONE);
        Some(RiseStoredAppealCase::new(
            AppealDTO::from(appeal),
            moderation_user(appeal_case.appellant),
            moderation_user(appeal_case.reviewer),
            report_case,
            capabilities,
        ))
    }
}

impl From<ProtoAdminSanctionResponse> for RiseStoredAdminSanction {
    fn from(reply: ProtoAdminSanctionResponse) -> Self {
        Self {
            kind: reply.kind.to_uppercase(),
            action_id: reply.action_id,
            expires_at_ms: reply.expires_at_ms,
            changed: reply.changed,
        }
    }
}
