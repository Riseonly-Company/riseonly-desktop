use std::collections::BTreeMap;
use std::sync::LazyLock;

use parking_lot::Mutex;
use rise_canon::rise_blob::moderator_permissions;
use rise_canon::rise_key::report;
use rise_canon::rise_request::report::CreateReport;
use rise_core::foundation::foundation_date::Date;
use rise_i18n::date_format::date_formatter::{DateFormatter, Style};
use rise_i18n::date_format::iso8601_date_formatter::{ISO8601DateFormatter, ISO8601DateFormatterOptions};
use rise_i18n::date_format::relative_date_time_formatter::{RelativeDateTimeFormatter, UnitsStyle};
use rise_i18n::language_store::Locale;
use rise_i18n::{AppDateFormatter, tr};
use rise_moderation::{GlobalCapability, Ladder, ModeratorPreset, ReportMenu};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::moderation_wire_types::{ReportAuthorWireDTO, ReportQueueWireDTO, ModeratorStatsWireDTO};
use crate::core::lib::canon_format::RiseFormat;
use crate::modules::moderation::engine::core::report_evidence_item::ReportEvidenceItem;
use crate::modules::moderation::engine::rise_moderation_wire::{
    AppealDTO, AppealItemDTO, ReportRowDTO, ReportSnapshotFacts, RiseStoredCaseCapabilities,
    RiseStoredCaseTimelineEntry, RiseStoredTargetActiveBlock, RiseStoredUserBlock,
};

pub type ReportQueueSort = rise_moderation::ReportQueueSort;
pub type ReportReason = rise_moderation::ReportReason;
pub type ReportStatus = rise_moderation::ReportStatus;
pub type ReportTargetType = rise_moderation::ReportTarget;
pub type UserBlockType = rise_moderation::BlockType;
pub type AppealStatus = rise_moderation::AppealStatus;
pub type AppealDecision = rise_moderation::AppealDecision;
pub type ModeratorCapability = rise_moderation::ModeratorCapability;
pub type ModeratorKind = rise_moderation::ModeratorKind;
pub type ResolveAction = rise_moderation::ResolveAction;
pub type ModerationAuditAction = rise_moderation::AuditAction;

pub type ModeratorScopeType = rise_moderation::ReportTarget;

pub type ModerationMuteScope = rise_moderation::BlockType;

pub trait ReportQueueSortExt {
    fn id(self) -> &'static str;
    fn title_key(self) -> &'static str;
}

impl ReportQueueSortExt for ReportQueueSort {
    fn id(self) -> &'static str {
        self.wire()
    }

    fn title_key(self) -> &'static str {
        self.i18n_key()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportTarget {
    Post(i64),
    Comment(i64),
    User(String),
    Message { chat_id: String, message_id: String },
    MusicTrack(String),
    Artist(String),
    Chat(String),
    Vacancy(i64),
    Story(i64),
    Bot(String),
}

impl ReportTarget {
    pub fn canon_target(&self) -> ReportTargetType {
        match self {
            ReportTarget::Post(_) => ReportTargetType::Post,
            ReportTarget::Comment(_) => ReportTargetType::Comment,
            ReportTarget::User(_) => ReportTargetType::User,
            ReportTarget::Message { .. } => ReportTargetType::Message,
            ReportTarget::MusicTrack(_) => ReportTargetType::MusicTrack,
            ReportTarget::Artist(_) => ReportTargetType::Artist,
            ReportTarget::Chat(_) => ReportTargetType::Chat,
            ReportTarget::Vacancy(_) => ReportTargetType::Vacancy,
            ReportTarget::Story(_) => ReportTargetType::Story,
            ReportTarget::Bot(_) => ReportTargetType::Bot,
        }
    }

    pub fn report_type(&self) -> &'static str {
        self.canon_target().wire()
    }

    pub fn apply(&self, request: &mut CreateReport) {
        match self {
            ReportTarget::Post(id) => request.post_id = Some(*id),
            ReportTarget::Comment(id) => request.comment_id = Some(*id),
            ReportTarget::User(id) => request.reported_user_id = Some(id.clone()),
            ReportTarget::Message { chat_id, message_id } => {
                request.chat_id = Some(chat_id.clone());
                request.message_id = Some(message_id.clone());
            }
            ReportTarget::MusicTrack(id) => request.track_id = Some(id.clone()),
            ReportTarget::Artist(id) => request.artist_id = Some(id.clone()),
            ReportTarget::Chat(id) => request.chat_id = Some(id.clone()),
            ReportTarget::Vacancy(id) => request.vacancy_id = Some(*id),
            ReportTarget::Story(id) => request.story_id = Some(*id),
            ReportTarget::Bot(id) => request.bot_id = Some(id.clone()),
        }
    }

    pub fn supports_context(&self) -> bool {
        matches!(self, ReportTarget::Message { .. })
    }

    pub fn is_user_scoped(&self) -> bool {
        self.canon_target().menu() == ReportMenu::User
    }
}

pub trait ReportReasonExt {
    fn id(self) -> &'static str;
    fn title_key(self) -> &'static str;
    fn icon_name(self) -> &'static str;
    fn user_reasons() -> &'static [ReportReason];
    fn content_reasons() -> &'static [ReportReason];
    fn reasons(target: &ReportTarget) -> &'static [ReportReason];
}

impl ReportReasonExt for ReportReason {
    fn id(self) -> &'static str {
        self.wire()
    }

    fn title_key(self) -> &'static str {
        self.i18n_key()
    }

    fn icon_name(self) -> &'static str {
        match self {
            ReportReason::Spam => "envelope.badge",
            ReportReason::ExplicitContent => "eye.slash",
            ReportReason::Discrimination => "person.2.slash",
            ReportReason::Terrorism => "exclamationmark.octagon",
            ReportReason::Violence => "hand.raised.slash",
            ReportReason::Suicide => "heart.slash",
            ReportReason::Political => "megaphone",
            ReportReason::Inadequacy => "face.dashed",
            ReportReason::UntruthInformation => "newspaper",
            ReportReason::Harassment => "person.crop.circle.badge.exclamationmark",
            ReportReason::Impersonation => "person.crop.circle.badge.questionmark",
            ReportReason::Scam => "creditcard.trianglebadge.exclamationmark",
            ReportReason::Copyright => "c.circle",
            ReportReason::ChildSafety => "figure.and.child.holdinghands",
            ReportReason::PersonalData => "person.text.rectangle",
            ReportReason::AiGenerated => "sparkles",
            ReportReason::Other => "ellipsis.circle",
        }
    }

    fn user_reasons() -> &'static [ReportReason] {
        ReportMenu::User.reasons()
    }

    fn content_reasons() -> &'static [ReportReason] {
        ReportMenu::Content.reasons()
    }

    fn reasons(target: &ReportTarget) -> &'static [ReportReason] {
        match target.canon_target().menu() {
            ReportMenu::User => Self::user_reasons(),
            ReportMenu::Content => Self::content_reasons(),
        }
    }
}

pub trait ReportStatusExt {
    fn id(self) -> &'static str;
    fn title_key(self) -> &'static str;
    fn parse_or_default(raw: Option<&str>) -> ReportStatus;
    fn is_closed(self) -> bool;
}

impl ReportStatusExt for ReportStatus {
    fn id(self) -> &'static str {
        self.wire()
    }

    fn title_key(self) -> &'static str {
        self.i18n_key()
    }

    fn parse_or_default(raw: Option<&str>) -> ReportStatus {
        ReportStatus::from_wire(&raw.unwrap_or("").to_uppercase()).unwrap_or(ReportStatus::Active)
    }

    fn is_closed(self) -> bool {
        self != ReportStatus::Active
    }
}

pub trait ReportTargetTypeExt {
    fn id(self) -> &'static str;
    fn title_key(self) -> &'static str;
    fn parse_or_default(raw: Option<&str>) -> ReportTargetType;
    fn moderator_scopes() -> &'static [ModeratorScopeType];
    fn moderator_scope(self) -> Option<ModeratorScopeType>;
}

impl ReportTargetTypeExt for ReportTargetType {
    fn id(self) -> &'static str {
        self.wire()
    }

    fn title_key(self) -> &'static str {
        self.i18n_key()
    }

    fn parse_or_default(raw: Option<&str>) -> ReportTargetType {
        ReportTargetType::from_wire(&raw.unwrap_or("").to_uppercase()).unwrap_or(ReportTargetType::Other)
    }

    fn moderator_scopes() -> &'static [ModeratorScopeType] {
        &rise_moderation::MODERATOR_SCOPE_ORDER
    }

    fn moderator_scope(self) -> Option<ModeratorScopeType> {
        if Self::moderator_scopes().contains(&self) { Some(self) } else { None }
    }
}

pub trait UserBlockTypeExt {
    fn id(self) -> &'static str;
    fn parse_or_default(raw: Option<&str>) -> UserBlockType;
    fn title_key(self) -> &'static str;
    fn description_key(self) -> &'static str;
    fn icon_name(self) -> &'static str;
    fn default_for(scope: ModeratorScopeType) -> ModerationMuteScope;
    fn mute_scopes() -> Vec<ModerationMuteScope>;
}

impl UserBlockTypeExt for UserBlockType {
    fn id(self) -> &'static str {
        self.wire()
    }

    fn parse_or_default(raw: Option<&str>) -> UserBlockType {
        UserBlockType::from_wire(&raw.unwrap_or("").to_uppercase()).unwrap_or(UserBlockType::Full)
    }

    fn title_key(self) -> &'static str {
        self.i18n_key()
    }

    fn description_key(self) -> &'static str {
        self.description_i18n_key()
    }

    fn icon_name(self) -> &'static str {
        match self {
            UserBlockType::Full => "nosign",
            UserBlockType::Comment => "text.bubble.fill",
            UserBlockType::Chat => "bubble.left.and.bubble.right.fill",
            UserBlockType::Post => "doc.text.fill",
            UserBlockType::Music => "music.note",
        }
    }

    fn default_for(scope: ModeratorScopeType) -> ModerationMuteScope {
        rise_moderation::MUTE_SCOPE_DEFAULTS
            .iter()
            .find(|entry| entry.target == scope)
            .map(|entry| entry.block_type)
            .unwrap_or(UserBlockType::Post)
    }

    fn mute_scopes() -> Vec<ModerationMuteScope> {
        UserBlockType::ALL.into_iter().filter(|block_type| *block_type != UserBlockType::Full).collect()
    }
}

pub trait AppealStatusExt {
    fn id(self) -> &'static str;
    fn parse_or_default(raw: Option<&str>) -> AppealStatus;
    fn title_key(self) -> &'static str;
    fn icon_name(self) -> &'static str;
    fn wire_value(self) -> &'static str;
}

impl AppealStatusExt for AppealStatus {
    fn id(self) -> &'static str {
        self.wire()
    }

    fn parse_or_default(raw: Option<&str>) -> AppealStatus {
        AppealStatus::from_wire(&raw.unwrap_or("").to_uppercase()).unwrap_or(AppealStatus::Pending)
    }

    fn title_key(self) -> &'static str {
        self.i18n_key()
    }

    fn icon_name(self) -> &'static str {
        match self {
            AppealStatus::Pending => "clock.fill",
            AppealStatus::Accepted => "checkmark.seal.fill",
            AppealStatus::Rejected => "xmark.seal.fill",
        }
    }

    fn wire_value(self) -> &'static str {
        self.wire()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppealStatusFilter {
    All,
    Pending,
    Accepted,
    Rejected,
}

impl AppealStatusFilter {
    pub const ALL: [AppealStatusFilter; 4] = [
        AppealStatusFilter::All,
        AppealStatusFilter::Pending,
        AppealStatusFilter::Accepted,
        AppealStatusFilter::Rejected,
    ];

    pub fn raw_value(self) -> &'static str {
        match self {
            AppealStatusFilter::All => rise_moderation::ALL_FILTER_NAME,
            AppealStatusFilter::Pending => AppealStatus::Pending.wire(),
            AppealStatusFilter::Accepted => AppealStatus::Accepted.wire(),
            AppealStatusFilter::Rejected => AppealStatus::Rejected.wire(),
        }
    }

    pub fn from_raw(raw: &str) -> Option<AppealStatusFilter> {
        Self::ALL.into_iter().find(|filter| filter.raw_value() == raw)
    }

    pub fn id(self) -> &'static str {
        self.raw_value()
    }

    pub fn title_key(self) -> &'static str {
        self.status().map(AppealStatusExt::title_key).unwrap_or(rise_moderation::ALL_FILTER_I18N_KEY)
    }

    pub fn wire_value(self) -> Option<&'static str> {
        if self == AppealStatusFilter::All { None } else { Some(self.raw_value()) }
    }

    pub fn status(self) -> Option<AppealStatus> {
        if self == AppealStatusFilter::All { None } else { AppealStatus::from_wire(self.raw_value()) }
    }
}

pub trait AppealDecisionExt {
    fn id(self) -> &'static str;
    fn title_key(self) -> &'static str;
    fn icon_name(self) -> &'static str;
}

impl AppealDecisionExt for AppealDecision {
    fn id(self) -> &'static str {
        self.wire()
    }

    fn title_key(self) -> &'static str {
        self.i18n_key()
    }

    fn icon_name(self) -> &'static str {
        match self {
            AppealDecision::Accept => "checkmark.seal.fill",
            AppealDecision::Reject => "xmark.seal.fill",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppealReviewFilter {
    Pending,
    Accepted,
    Rejected,
}

impl AppealReviewFilter {
    pub const ALL: [AppealReviewFilter; 3] =
        [AppealReviewFilter::Pending, AppealReviewFilter::Accepted, AppealReviewFilter::Rejected];

    pub fn raw_value(self) -> &'static str {
        match self {
            AppealReviewFilter::Pending => AppealStatus::Pending.wire(),
            AppealReviewFilter::Accepted => AppealStatus::Accepted.wire(),
            AppealReviewFilter::Rejected => AppealStatus::Rejected.wire(),
        }
    }

    pub fn from_raw(raw: &str) -> Option<AppealReviewFilter> {
        Self::ALL.into_iter().find(|filter| filter.raw_value() == raw)
    }

    pub fn id(self) -> &'static str {
        self.raw_value()
    }

    pub fn title_key(self) -> &'static str {
        self.status().title_key()
    }

    pub fn wire_value(self) -> &'static str {
        self.raw_value()
    }

    pub fn status(self) -> AppealStatus {
        AppealStatus::parse_or_default(Some(self.raw_value()))
    }
}

pub trait ModeratorCapabilityExt {
    fn id(self) -> &'static str;
    fn title_key(self) -> &'static str;
}

impl ModeratorCapabilityExt for ModeratorCapability {
    fn id(self) -> &'static str {
        self.wire()
    }

    fn title_key(self) -> &'static str {
        self.i18n_key()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ModeratorScopePermissions {
    pub can_review: bool,
    pub can_dismiss: bool,
    pub can_warn: bool,
    pub can_delete_content: bool,
    pub can_block_author: bool,
    pub can_mute_author: bool,
}

impl ModeratorScopePermissions {
    pub fn new(
        can_review: bool,
        can_dismiss: bool,
        can_warn: bool,
        can_delete_content: bool,
        can_block_author: bool,
        can_mute_author: bool,
    ) -> Self {
        Self { can_review, can_dismiss, can_warn, can_delete_content, can_block_author, can_mute_author }
    }

    pub fn from_capabilities(capabilities: &[ModeratorCapability]) -> Self {
        let mut permissions = Self::default();
        for cap in capabilities {
            permissions.set(*cap, true);
        }
        permissions
    }

    pub fn has(&self, cap: ModeratorCapability) -> bool {
        match cap {
            ModeratorCapability::Review => self.can_review,
            ModeratorCapability::Dismiss => self.can_dismiss,
            ModeratorCapability::Warn => self.can_warn,
            ModeratorCapability::DeleteContent => self.can_delete_content,
            ModeratorCapability::BlockAuthor => self.can_block_author,
            ModeratorCapability::MuteAuthor => self.can_mute_author,
        }
    }

    pub fn set(&mut self, cap: ModeratorCapability, value: bool) {
        match cap {
            ModeratorCapability::Review => self.can_review = value,
            ModeratorCapability::Dismiss => self.can_dismiss = value,
            ModeratorCapability::Warn => self.can_warn = value,
            ModeratorCapability::DeleteContent => self.can_delete_content = value,
            ModeratorCapability::BlockAuthor => self.can_block_author = value,
            ModeratorCapability::MuteAuthor => self.can_mute_author = value,
        }
    }

    pub fn all() -> Self {
        Self::from_capabilities(&ModeratorCapability::ALL)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ModeratorPermissions {
    pub scopes: BTreeMap<String, ModeratorScopePermissions>,
    pub can_resolve_appeals: bool,
    pub max_block_days: Option<i64>,
}

impl ModeratorPermissions {
    pub fn new(
        scopes: BTreeMap<String, ModeratorScopePermissions>,
        can_resolve_appeals: bool,
        max_block_days: Option<i64>,
    ) -> Self {
        Self { scopes, can_resolve_appeals, max_block_days }
    }

    pub fn scope(&self, scope_type: ModeratorScopeType) -> ModeratorScopePermissions {
        self.scopes.get(scope_type.wire()).copied().unwrap_or_default()
    }

    pub fn can(&self, scope_type: ModeratorScopeType, cap: ModeratorCapability) -> bool {
        self.scope(scope_type).has(cap)
    }

    pub fn can_review(&self, scope_type: ModeratorScopeType) -> bool {
        self.can(scope_type, ModeratorCapability::Review)
    }

    pub fn can_dismiss(&self, scope_type: ModeratorScopeType) -> bool {
        self.can(scope_type, ModeratorCapability::Dismiss)
    }

    pub fn can_warn(&self, scope_type: ModeratorScopeType) -> bool {
        self.can(scope_type, ModeratorCapability::Warn)
    }

    pub fn can_delete_content(&self, scope_type: ModeratorScopeType) -> bool {
        self.can(scope_type, ModeratorCapability::DeleteContent)
    }

    pub fn can_block(&self, scope_type: ModeratorScopeType) -> bool {
        self.can(scope_type, ModeratorCapability::BlockAuthor)
    }

    pub fn can_mute(&self, scope_type: ModeratorScopeType) -> bool {
        self.can(scope_type, ModeratorCapability::MuteAuthor)
    }

    pub fn reviewable_types(&self) -> Vec<ModeratorScopeType> {
        ModeratorScopeType::moderator_scopes().iter().copied().filter(|scope| self.can_review(*scope)).collect()
    }

    pub fn to_json_string(&self) -> String {
        let Ok(mut value) = serde_json::to_value(self) else {
            return "{}".to_owned();
        };
        value.sort_all_objects();
        serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_owned())
    }

    pub fn from_json_string(json_string: Option<&str>) -> ModeratorPermissions {
        let Some(json_string) = json_string.filter(|text| !text.is_empty()) else {
            return ModeratorPermissions::default();
        };
        serde_json::from_str::<ModeratorPermissions>(json_string).unwrap_or_default()
    }

    pub fn from_preset(preset: ModeratorPreset) -> ModeratorPermissions {
        let scope_permissions = ModeratorScopePermissions::from_capabilities(preset.scope_capabilities());
        let mut scopes = BTreeMap::new();
        for scope in preset.scopes() {
            scopes.insert(scope.wire().to_owned(), scope_permissions);
        }

        ModeratorPermissions::new(
            scopes,
            preset.global_capabilities().contains(&GlobalCapability::ResolveAppeals),
            preset.max_block_days().map(i64::from),
        )
    }

    pub fn comment_preset() -> ModeratorPermissions {
        Self::from_preset(ModeratorPreset::Comment)
    }

    pub fn content_preset() -> ModeratorPermissions {
        Self::from_preset(ModeratorPreset::Content)
    }

    pub fn super_preset() -> ModeratorPermissions {
        Self::from_preset(ModeratorPreset::SuperModerator)
    }
}

pub trait ResolveActionExt {
    fn id(self) -> &'static str;
    fn title_key(self) -> &'static str;
    fn icon_name(self) -> &'static str;
    fn hint_key(self) -> &'static str;
    fn takes_duration(self) -> bool;
    fn available(capabilities: &RiseStoredCaseCapabilities, has_content: bool) -> Vec<ResolveAction>;
}

impl ResolveActionExt for ResolveAction {
    fn id(self) -> &'static str {
        self.wire()
    }

    fn title_key(self) -> &'static str {
        self.i18n_key()
    }

    fn icon_name(self) -> &'static str {
        match self {
            ResolveAction::Dismiss => "checkmark.circle",
            ResolveAction::Warn => "exclamationmark.triangle",
            ResolveAction::DeleteContent => "trash",
            ResolveAction::BlockUser => "nosign",
            ResolveAction::Mute => "speaker.slash",
            ResolveAction::Both => "shield.lefthalf.filled",
        }
    }

    fn hint_key(self) -> &'static str {
        match self {
            ResolveAction::Dismiss => "moderation_action_dismiss_hint",
            ResolveAction::Warn => "moderation_action_warn_hint",
            ResolveAction::DeleteContent => "moderation_action_delete_content_hint",
            ResolveAction::BlockUser => "moderation_action_block_user_hint",
            ResolveAction::Mute => "moderation_action_mute_hint",
            ResolveAction::Both => "moderation_action_both_hint",
        }
    }

    fn takes_duration(self) -> bool {
        match self {
            ResolveAction::BlockUser | ResolveAction::Mute | ResolveAction::Both => true,
            ResolveAction::Dismiss | ResolveAction::Warn | ResolveAction::DeleteContent => false,
        }
    }

    fn available(capabilities: &RiseStoredCaseCapabilities, has_content: bool) -> Vec<ResolveAction> {
        let mut actions = Vec::new();
        if capabilities.can_dismiss {
            actions.push(ResolveAction::Dismiss);
        }
        if capabilities.can_warn {
            actions.push(ResolveAction::Warn);
        }
        if capabilities.can_delete_content && has_content {
            actions.push(ResolveAction::DeleteContent);
        }
        if capabilities.can_mute_author {
            actions.push(ResolveAction::Mute);
        }
        if capabilities.can_block_author {
            actions.push(ResolveAction::BlockUser);
        }
        if capabilities.can_block_author && capabilities.can_delete_content && has_content {
            actions.push(ResolveAction::Both);
        }
        actions
    }
}

pub trait ModerationAuditActionExt {
    fn title_key(self) -> &'static str;
    fn icon_name(self) -> &'static str;
}

impl ModerationAuditActionExt for ModerationAuditAction {
    fn title_key(self) -> &'static str {
        match self {
            ModerationAuditAction::Dismiss => "moderation_action_dismiss",
            ModerationAuditAction::Warn => "moderation_action_warn",
            ModerationAuditAction::DeleteContent => "moderation_action_delete_content",
            ModerationAuditAction::BlockUser => "moderation_action_block_user",
            ModerationAuditAction::Mute => "moderation_action_mute",
            ModerationAuditAction::Both => "moderation_action_both",
            ModerationAuditAction::CreateAppeal => "timeline_action_create_appeal",
            ModerationAuditAction::AppealAccept => "timeline_action_appeal_accept",
            ModerationAuditAction::AppealReject => "timeline_action_appeal_reject",
            ModerationAuditAction::ApproveModerator => "timeline_action_approve_moderator",
            ModerationAuditAction::ApproveSuperModer => "timeline_action_approve_super_moder",
            ModerationAuditAction::RejectModerator => "timeline_action_reject_moderator",
            ModerationAuditAction::UpdatePermissions => "timeline_action_update_permissions",
            ModerationAuditAction::RemoveModerator => "timeline_action_remove_moderator",
            ModerationAuditAction::AddNote => "timeline_action_add_note",
            ModerationAuditAction::ReopenReport => "timeline_action_reopen_report",
            ModerationAuditAction::AdminBlock => "timeline_action_admin_block",
            ModerationAuditAction::AdminUnblock => "timeline_action_admin_unblock",
            ModerationAuditAction::AdminBlockIp => "timeline_action_admin_block_ip",
            ModerationAuditAction::AdminUnblockIp => "timeline_action_admin_unblock_ip",
            ModerationAuditAction::RestoreContent => "timeline_action_restore_content",
            ModerationAuditAction::ResolveReport => "timeline_action_resolve_report",
            ModerationAuditAction::Block => "timeline_action_block",
        }
    }

    fn icon_name(self) -> &'static str {
        match self {
            ModerationAuditAction::Dismiss => "checkmark.circle",
            ModerationAuditAction::Warn => "exclamationmark.triangle",
            ModerationAuditAction::DeleteContent => "trash",
            ModerationAuditAction::BlockUser | ModerationAuditAction::Block | ModerationAuditAction::AdminBlock => {
                "nosign"
            }
            ModerationAuditAction::Mute => "speaker.slash",
            ModerationAuditAction::Both => "shield.lefthalf.filled",
            ModerationAuditAction::CreateAppeal => "arrow.uturn.backward.circle",
            ModerationAuditAction::AppealAccept => "checkmark.seal",
            ModerationAuditAction::AppealReject => "xmark.seal",
            ModerationAuditAction::ApproveModerator | ModerationAuditAction::ApproveSuperModer => {
                "person.badge.shield.checkmark"
            }
            ModerationAuditAction::RejectModerator | ModerationAuditAction::RemoveModerator => "person.badge.minus",
            ModerationAuditAction::UpdatePermissions => "slider.horizontal.3",
            ModerationAuditAction::AddNote => "note.text",
            ModerationAuditAction::ReopenReport => "arrow.counterclockwise.circle",
            ModerationAuditAction::AdminUnblock => "lock.open",
            ModerationAuditAction::AdminBlockIp => "network.badge.shield.half.filled",
            ModerationAuditAction::AdminUnblockIp => "network",
            ModerationAuditAction::RestoreContent => "arrow.uturn.backward",
            ModerationAuditAction::ResolveReport => "checkmark.circle",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AdminSanctionKind {
    Block,
    Unblock,
    BlockIp,
    UnblockIp,
    RestoreContent,
}

impl AdminSanctionKind {
    pub const ALL: [AdminSanctionKind; 5] = [
        AdminSanctionKind::Block,
        AdminSanctionKind::Unblock,
        AdminSanctionKind::BlockIp,
        AdminSanctionKind::UnblockIp,
        AdminSanctionKind::RestoreContent,
    ];

    pub fn raw_value(self) -> &'static str {
        self.audit_action().wire()
    }

    pub fn id(self) -> &'static str {
        self.raw_value()
    }

    pub fn audit_action(self) -> ModerationAuditAction {
        match self {
            AdminSanctionKind::Block => ModerationAuditAction::AdminBlock,
            AdminSanctionKind::Unblock => ModerationAuditAction::AdminUnblock,
            AdminSanctionKind::BlockIp => ModerationAuditAction::AdminBlockIp,
            AdminSanctionKind::UnblockIp => ModerationAuditAction::AdminUnblockIp,
            AdminSanctionKind::RestoreContent => ModerationAuditAction::RestoreContent,
        }
    }

    pub fn title_key(self) -> &'static str {
        match self {
            AdminSanctionKind::Block => "admin_sanction_block",
            AdminSanctionKind::Unblock => "admin_sanction_unblock",
            AdminSanctionKind::BlockIp => "admin_sanction_block_ip",
            AdminSanctionKind::UnblockIp => "admin_sanction_unblock_ip",
            AdminSanctionKind::RestoreContent => "admin_sanction_restore_content",
        }
    }

    pub fn hint_key(self) -> &'static str {
        match self {
            AdminSanctionKind::Block => "admin_sanction_block_hint",
            AdminSanctionKind::Unblock => "admin_sanction_unblock_hint",
            AdminSanctionKind::BlockIp => "admin_sanction_block_ip_hint",
            AdminSanctionKind::UnblockIp => "admin_sanction_unblock_ip_hint",
            AdminSanctionKind::RestoreContent => "admin_sanction_restore_content_hint",
        }
    }

    pub fn icon_name(self) -> &'static str {
        self.audit_action().icon_name()
    }

    pub fn takes_duration(self) -> bool {
        self == AdminSanctionKind::Block || self == AdminSanctionKind::BlockIp
    }

    pub fn takes_block_type(self) -> bool {
        self == AdminSanctionKind::Block || self == AdminSanctionKind::Unblock
    }

    pub fn takes_ip(self) -> bool {
        self == AdminSanctionKind::BlockIp || self == AdminSanctionKind::UnblockIp
    }

    pub fn is_destructive(self) -> bool {
        self == AdminSanctionKind::Block || self == AdminSanctionKind::BlockIp
    }
}

pub trait ModeratorKindExt {
    fn title_key(self) -> &'static str;
}

impl ModeratorKindExt for ModeratorKind {
    fn title_key(self) -> &'static str {
        self.i18n_key()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModeratorKindFilter {
    All,
    Moderator,
    SuperModer,
}

impl ModeratorKindFilter {
    pub const ALL: [ModeratorKindFilter; 3] =
        [ModeratorKindFilter::All, ModeratorKindFilter::Moderator, ModeratorKindFilter::SuperModer];

    pub fn raw_value(self) -> &'static str {
        match self {
            ModeratorKindFilter::All => rise_moderation::ALL_FILTER_NAME,
            ModeratorKindFilter::Moderator => ModeratorKind::Moderator.wire(),
            ModeratorKindFilter::SuperModer => ModeratorKind::SuperModer.wire(),
        }
    }

    pub fn id(self) -> &'static str {
        self.raw_value()
    }

    pub fn title_key(self) -> &'static str {
        match self {
            ModeratorKindFilter::All => rise_moderation::ALL_FILTER_I18N_KEY,
            ModeratorKindFilter::Moderator => ModeratorKind::Moderator.title_key(),
            ModeratorKindFilter::SuperModer => ModeratorKind::SuperModer.title_key(),
        }
    }

    pub fn wire_value(self) -> Option<&'static str> {
        if self == ModeratorKindFilter::All { None } else { Some(self.raw_value()) }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ModeratorDetailState {
    pub stats: Option<ModeratorStatsWireDTO>,
    pub stats_user: Option<ReportAuthorWireDTO>,
    pub did_load_stats: bool,
    pub stats_error: Option<String>,

    pub history: Vec<ReportQueueWireDTO>,
    pub history_has_more: bool,
    pub history_relative_id: Option<i64>,
    pub history_total: i64,
    pub did_load_history: bool,
    pub history_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModerationDurationChoice {
    Days(i64),
    Permanent,
}

impl ModerationDurationChoice {
    pub fn id(&self) -> String {
        match self {
            ModerationDurationChoice::Days(days) => format!("d{days}"),
            ModerationDurationChoice::Permanent => "permanent".to_owned(),
        }
    }

    pub fn wire_days(&self) -> i64 {
        match self {
            ModerationDurationChoice::Days(days) => *days,
            ModerationDurationChoice::Permanent => 0,
        }
    }

    pub fn choices(ladder: Ladder, max_block_days: Option<i64>, allows_permanent: bool) -> Vec<ModerationDurationChoice> {
        let mut steps: Vec<i64> = ladder.steps().iter().map(|step| i64::from(*step)).collect();
        if let Some(max_block_days) = max_block_days.filter(|days| *days > 0) {
            steps.retain(|step| *step <= max_block_days);
            if !steps.contains(&max_block_days) {
                steps.push(max_block_days);
            }
        }
        steps.sort();
        let mut result: Vec<ModerationDurationChoice> = steps.into_iter().map(ModerationDurationChoice::Days).collect();
        if allows_permanent {
            result.push(ModerationDurationChoice::Permanent);
        }
        result
    }
}

pub enum BlockTimeFormatting {}

static BLOCK_TIME_ABSOLUTE_FORMATTER: LazyLock<Mutex<DateFormatter>> = LazyLock::new(|| {
    let mut formatter = AppDateFormatter::make(None, None, None);
    formatter.set_date_style(Style::Medium);
    formatter.set_time_style(Style::Short);
    Mutex::new(formatter)
});

impl BlockTimeFormatting {
    pub fn date(from_ms: i64) -> Date {
        Date::from_time_interval_since_1970(from_ms as f64 / 1000.0)
    }

    fn relative_formatter() -> RelativeDateTimeFormatter {
        let mut formatter = RelativeDateTimeFormatter::new();
        formatter.set_locale(Locale::app_locale().identifier());
        formatter.set_units_style(UnitsStyle::Full);
        formatter
    }

    pub fn absolute(from_ms: Option<i64>) -> Option<String> {
        let ms = from_ms.filter(|ms| *ms > 0)?;
        let mut formatter = BLOCK_TIME_ABSOLUTE_FORMATTER.lock();
        formatter.set_locale(Locale::app_locale().identifier());
        Some(formatter.string_from(&Self::date(ms)))
    }

    pub fn remaining(until_ms: Option<i64>) -> Option<String> {
        let ms = until_ms.filter(|ms| *ms > 0)?;
        let target = Self::date(ms);
        if target <= Date::now() {
            return None;
        }
        Some(Self::relative_formatter().localized_string_for(&target, &Date::now()))
    }

    pub fn since_relative(from_ms: Option<i64>) -> Option<String> {
        let ms = from_ms.filter(|ms| *ms > 0)?;
        Some(Self::relative_formatter().localized_string_for(&Self::date(ms), &Date::now()))
    }
}

pub enum ModerationDateFormatting {}

impl ModerationDateFormatting {
    pub fn short(ms: Option<i64>, iso: Option<&str>) -> Option<String> {
        if let Some(text) = BlockTimeFormatting::absolute(ms) {
            return Some(text);
        }
        ReportDateFormatting::short(iso)
    }

    pub fn relative(ms: Option<i64>, iso: Option<&str>) -> Option<String> {
        if let Some(ms) = ms.filter(|ms| *ms > 0) {
            return Some(Self::age(ms, Date::now()));
        }
        ReportDateFormatting::short(iso)
    }

    pub fn age(ms: i64, now: Date) -> String {
        let now_ms = (now.time_interval_since_1970() * 1000.0) as i64;
        RiseFormat::elapsed(ms.min(now_ms), now_ms)
    }
}

pub enum AppealDateFormatting {}

impl AppealDateFormatting {
    pub fn short(rfc3339: Option<&str>) -> Option<String> {
        ReportDateFormatting::short(rfc3339)
    }
}

pub enum ModeratorStatFormatting {}

impl ModeratorStatFormatting {
    pub fn percent(value: Option<f64>) -> String {
        RiseFormat::percent(value.unwrap_or(0.0))
    }
}

impl ReportRowDTO {
    pub fn resolved_status(&self) -> ReportStatus {
        ReportStatus::parse_or_default(Some(&self.status))
    }

    pub fn resolved_reason(&self) -> Option<ReportReason> {
        ReportReason::from_wire(&self.reason)
    }

    pub fn resolved_type(&self) -> ReportTargetType {
        ReportTargetType::parse_or_default(Some(&self.report_type))
    }

    pub fn resolved_scope(&self) -> Option<ModeratorScopeType> {
        self.resolved_type().moderator_scope()
    }

    pub fn resolved_block_type(&self) -> Option<UserBlockType> {
        let block_type = self.block_type.as_deref().filter(|block_type| !block_type.is_empty())?;
        Some(UserBlockType::parse_or_default(Some(block_type)))
    }

    pub fn resolved_action(&self) -> Option<ResolveAction> {
        let action = ResolveAction::from_wire(&self.moderation_action.as_deref().unwrap_or("").to_uppercase());
        if action == Some(ResolveAction::BlockUser)
            && let Some(block_type) = self.resolved_block_type()
            && block_type != UserBlockType::Full
        {
            return Some(ResolveAction::Mute);
        }
        action
    }

    pub fn is_closed(&self) -> bool {
        self.resolved_status().is_closed()
    }

    pub fn evidence(&self) -> Vec<ReportEvidenceItem> {
        ReportEvidenceItem::items(&self.evidence_file_urls, &self.evidence_file_ids)
    }

    pub fn reported_user_id(&self) -> Option<String> {
        self.target_user_id.clone().or_else(|| {
            if self.resolved_type() == ReportTargetType::User { self.user_id.clone() } else { None }
        })
    }

    pub fn subject_line(&self) -> String {
        if let Some(title) = self.snapshot_title.as_deref().filter(|title| !title.is_empty()) {
            return title.to_owned();
        }
        if let Some(user) =
            self.reported_user.as_ref().and_then(|user| user.name.as_deref()).filter(|name| !name.is_empty())
        {
            return user.to_owned();
        }
        if let Some(text) = self.snapshot_text.as_deref().filter(|text| !text.is_empty()) {
            return text.to_owned();
        }
        tr(self.resolved_type().title_key())
    }

    pub fn created_date_text(&self) -> Option<String> {
        ModerationDateFormatting::short(self.created_at_ms, self.created_at.as_deref())
    }

    pub fn created_relative_text(&self) -> Option<String> {
        ModerationDateFormatting::relative(self.created_at_ms, self.created_at.as_deref())
    }

    pub fn resolved_post_id(&self) -> Option<i64> {
        self.post_id.or_else(|| self.snapshot_facts.int(report::POST_ID))
    }
}

impl AppealDTO {
    pub fn resolved_status(&self) -> AppealStatus {
        AppealStatus::parse_or_default(Some(&self.status))
    }

    pub fn is_reviewed(&self) -> bool {
        self.resolved_status() != AppealStatus::Pending
    }

    pub fn evidence(&self) -> Vec<ReportEvidenceItem> {
        ReportEvidenceItem::items(&self.evidence_file_urls, &self.evidence_file_ids)
    }

    pub fn created_date_text(&self) -> Option<String> {
        ModerationDateFormatting::short(self.created_at_ms, self.created_at.as_deref())
    }

    pub fn reviewed_date_text(&self) -> Option<String> {
        ModerationDateFormatting::short(self.reviewed_at_ms, self.reviewed_at.as_deref())
    }
}

impl AppealItemDTO {
    pub fn resolved_report_type(&self) -> Option<ReportTargetType> {
        let report_type = self.report_type.as_deref().filter(|report_type| !report_type.is_empty())?;
        ReportTargetType::from_wire(&report_type.to_uppercase())
    }

    pub fn resolved_report_reason(&self) -> Option<ReportReason> {
        ReportReason::from_wire(&self.report_reason.as_deref().unwrap_or("").to_uppercase())
    }

    pub fn resolved_block_type(&self) -> Option<UserBlockType> {
        let block_type = self.block_type.as_deref().filter(|block_type| !block_type.is_empty())?;
        Some(UserBlockType::parse_or_default(Some(block_type)))
    }

    pub fn resolved_status(&self) -> AppealStatus {
        self.appeal.resolved_status()
    }

    pub fn report_id(&self) -> Option<i64> {
        self.appeal.report_id
    }
}

impl RiseStoredUserBlock {
    pub fn resolved_type(&self) -> UserBlockType {
        UserBlockType::parse_or_default(Some(&self.block_type))
    }

    pub fn resolved_report_type(&self) -> Option<ReportTargetType> {
        let report_type = self.report_type.as_deref().filter(|report_type| !report_type.is_empty())?;
        ReportTargetType::from_wire(&report_type.to_uppercase())
    }

    pub fn resolved_report_reason(&self) -> Option<ReportReason> {
        ReportReason::from_wire(&self.report_reason.as_deref().unwrap_or("").to_uppercase())
    }

    pub fn is_permanent(&self) -> bool {
        self.block_end_timestamp == 0
    }

    pub fn is_expired(&self) -> bool {
        if self.block_end_timestamp <= 0 {
            return false;
        }
        BlockTimeFormatting::date(self.block_end_timestamp) < Date::now()
    }

    pub fn allows_appeal(&self) -> bool {
        self.can_appeal && !self.is_expired()
    }
}

impl RiseStoredTargetActiveBlock {
    pub fn resolved_type(&self) -> UserBlockType {
        UserBlockType::parse_or_default(Some(&self.block_type))
    }

    pub fn is_permanent(&self) -> bool {
        self.block_end_timestamp == 0
    }
}

impl RiseStoredCaseTimelineEntry {
    pub fn resolved_action(&self) -> Option<ModerationAuditAction> {
        ModerationAuditAction::from_wire(&self.action)
    }

    pub fn details_facts(&self) -> ReportSnapshotFacts {
        ReportSnapshotFacts::parse(Some(&self.details_json))
    }
}

pub enum ReportDateFormatting {}

static REPORT_DATE_INPUT_FORMATTER: LazyLock<ISO8601DateFormatter> = LazyLock::new(|| {
    let mut formatter = ISO8601DateFormatter::new();
    formatter.set_format_options(
        ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME | ISO8601DateFormatterOptions::WITH_FRACTIONAL_SECONDS,
    );
    formatter
});

static REPORT_DATE_FALLBACK_FORMATTER: LazyLock<ISO8601DateFormatter> = LazyLock::new(|| {
    let mut formatter = ISO8601DateFormatter::new();
    formatter.set_format_options(ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME);
    formatter
});

static REPORT_DATE_DISPLAY_FORMATTER: LazyLock<Mutex<DateFormatter>> = LazyLock::new(|| {
    let mut formatter = AppDateFormatter::make(None, None, None);
    formatter.set_date_style(Style::Medium);
    formatter.set_time_style(Style::Short);
    Mutex::new(formatter)
});

impl ReportDateFormatting {
    pub fn short(rfc3339: Option<&str>) -> Option<String> {
        let rfc3339 = rfc3339.filter(|text| !text.is_empty())?;
        let date =
            REPORT_DATE_INPUT_FORMATTER.date_from(rfc3339).or_else(|| REPORT_DATE_FALLBACK_FORMATTER.date_from(rfc3339));
        let Some(date) = date else {
            return Some(rfc3339.to_owned());
        };
        let mut formatter = REPORT_DATE_DISPLAY_FORMATTER.lock();
        formatter.set_locale(Locale::app_locale().identifier());
        Some(formatter.string_from(&date))
    }
}

impl ModeratorScopePermissions {
    pub fn from_canon(scope: &moderator_permissions::Scope) -> Self {
        Self::new(
            scope.can_review.unwrap_or(false),
            scope.can_dismiss.unwrap_or(false),
            scope.can_warn.unwrap_or(false),
            scope.can_delete_content.unwrap_or(false),
            scope.can_block_author.unwrap_or(false),
            scope.can_mute_author.unwrap_or(false),
        )
    }

    pub fn canon(&self) -> moderator_permissions::Scope {
        moderator_permissions::Scope {
            can_review: Some(self.can_review),
            can_dismiss: Some(self.can_dismiss),
            can_warn: Some(self.can_warn),
            can_delete_content: Some(self.can_delete_content),
            can_block_author: Some(self.can_block_author),
            can_mute_author: Some(self.can_mute_author),
        }
    }
}

impl<'de> Deserialize<'de> for ModeratorScopePermissions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        moderator_permissions::Scope::deserialize(deserializer).map(|scope| Self::from_canon(&scope))
    }
}

impl Serialize for ModeratorScopePermissions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.canon().serialize(serializer)
    }
}

impl ModeratorPermissions {
    pub fn from_canon(permissions: &moderator_permissions::Permissions) -> Self {
        Self::new(
            permissions
                .scopes
                .as_ref()
                .map(|scopes| {
                    scopes.iter().map(|(key, scope)| (key.clone(), ModeratorScopePermissions::from_canon(scope))).collect()
                })
                .unwrap_or_default(),
            permissions.can_resolve_appeals.unwrap_or(false),
            permissions.max_block_days,
        )
    }

    pub fn canon(&self) -> moderator_permissions::Permissions {
        moderator_permissions::Permissions {
            scopes: Some(self.scopes.iter().map(|(key, scope)| (key.clone(), scope.canon())).collect()),
            can_resolve_appeals: Some(self.can_resolve_appeals),
            max_block_days: self.max_block_days,
            ..Default::default()
        }
    }
}

impl<'de> Deserialize<'de> for ModeratorPermissions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        moderator_permissions::Permissions::deserialize(deserializer).map(|permissions| Self::from_canon(&permissions))
    }
}

impl Serialize for ModeratorPermissions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.canon().serialize(serializer)
    }
}
