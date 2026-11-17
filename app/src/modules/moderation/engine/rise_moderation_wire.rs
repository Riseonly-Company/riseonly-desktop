use std::collections::BTreeMap;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_moderation::{BlockReason, PlatformRole};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageContentType, MessageMediaItem, MessageMediaKind, RemoteMediaURL,
};
use crate::modules::moderation::engine::core::report_evidence_item::ReportEvidenceItem;
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AppealStatus, ModerationAuditAction, ReportReason, ReportStatus, ReportTargetType, UserBlockType,
};

/// lenient reads every decoder below shares, over one keyed json object
struct KeyedDecodingContainer {
    values: Map<String, Value>,
}

impl KeyedDecodingContainer {
    fn read<'de, D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        Ok(Self { values: Map::deserialize(decoder)? })
    }

    fn lenient<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.values.get(key).and_then(|value| T::deserialize(value).ok())
    }

    fn string(&self, key: &str) -> Option<String> {
        match self.values.get(key)? {
            Value::String(value) => Some(value.clone()),
            value => json_int64(value).map(|number| number.to_string()),
        }
    }

    fn int(&self, key: &str) -> Option<i64> {
        self.int64(key)
    }

    fn int64(&self, key: &str) -> Option<i64> {
        match self.values.get(key)? {
            Value::String(value) => value.parse::<i64>().ok(),
            value => json_int64(value),
        }
    }

    fn bool(&self, key: &str) -> bool {
        self.values.get(key).and_then(Value::as_bool).unwrap_or(false)
    }

    fn strings(&self, key: &str) -> Vec<String> {
        if let Some(list) = self.lenient::<Vec<String>>(key) {
            return list;
        }
        match self.values.get(key) {
            Some(Value::String(text)) => text_list(text),
            _ => Vec::new(),
        }
    }

    /// one undecodable row must not empty the list a moderator works from
    fn lenient_rows<T: DeserializeOwned>(&self, key: &str) -> Vec<T> {
        let Some(Value::Array(list)) = self.values.get(key) else {
            return Vec::new();
        };
        list.iter()
            .filter_map(|value| match T::deserialize(value) {
                Ok(row) => Some(row),
                Err(_) => {
                    let _ = ModerationDiscardedValue::deserialize(value);
                    None
                }
            })
            .collect()
    }

    fn required<T: DeserializeOwned, E: serde::de::Error>(&self, key: &'static str) -> Result<T, E> {
        let value = self.values.get(key).ok_or_else(|| E::missing_field(key))?;
        T::deserialize(value).map_err(E::custom)
    }
}

/// json number as int64, integral values only and in range
fn json_int64(value: &Value) -> Option<i64> {
    let Value::Number(number) = value else {
        return None;
    };
    if let Some(integer) = number.as_i64() {
        return Some(integer);
    }
    let float = number.as_f64()?;
    if float.fract() == 0.0 && float >= i64::MIN as f64 && float < i64::MAX as f64 {
        return Some(float as i64);
    }
    None
}

fn text_list(text: &str) -> Vec<String> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(parsed)) => parsed.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModerationUserDTO {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub is_premium: bool,
    pub is_official: bool,
}

impl<'de> Deserialize<'de> for ModerationUserDTO {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            user_id: c.string("userId"),
            name: c.string("name"),
            tag: c.string("tag"),
            logo: c.string("logo"),
            role: c.string("role"),
            is_premium: c.bool("isPremium"),
            is_official: c.bool("isOfficial"),
        })
    }
}

impl ModerationUserDTO {
    pub fn display_name(&self) -> String {
        if let Some(name) = self.name.as_deref().filter(|name| !name.is_empty()) {
            return name.to_owned();
        }
        if let Some(tag) = self.tag.as_deref().filter(|tag| !tag.is_empty()) {
            return format!("@{tag}");
        }
        tr("moderation_unknown_reporter")
    }

    pub fn is_staff(&self) -> bool {
        let Some(role) = self.role.as_deref() else {
            return false;
        };
        let upper = role.to_uppercase();
        [PlatformRole::Admin, PlatformRole::Moderator, PlatformRole::SuperModer]
            .iter()
            .any(|staff| staff.wire() == upper)
    }
}

/// what the report captured about its object when filed
/// keys differ per kind so scalars are text and string lists are lists
/// parsed once at decode, never in a render
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReportSnapshotFacts {
    pub scalars: BTreeMap<String, String>,
    pub lists: BTreeMap<String, Vec<String>>,
}

impl ReportSnapshotFacts {
    pub const EMPTY: ReportSnapshotFacts = ReportSnapshotFacts { scalars: BTreeMap::new(), lists: BTreeMap::new() };

    pub fn new(scalars: BTreeMap<String, String>, lists: BTreeMap<String, Vec<String>>) -> Self {
        Self { scalars, lists }
    }

    pub fn parse(json: Option<&str>) -> ReportSnapshotFacts {
        let Some(json) = json else {
            return Self::EMPTY;
        };
        let Ok(Value::Object(object)) = serde_json::from_str::<Value>(json) else {
            return Self::EMPTY;
        };
        let mut scalars = BTreeMap::new();
        let mut lists = BTreeMap::new();
        for (key, value) in object {
            match value {
                Value::String(text) => {
                    scalars.insert(key, text);
                }
                Value::Bool(flag) => {
                    scalars.insert(key, if flag { "true" } else { "false" }.to_owned());
                }
                Value::Number(number) => {
                    let text = match number.as_i64() {
                        Some(integer) => integer.to_string(),
                        None => match number.as_f64() {
                            Some(float) if float == float.round() => (float as i64).to_string(),
                            _ => number.to_string(),
                        },
                    };
                    scalars.insert(key, text);
                }
                Value::Array(list) => {
                    lists.insert(key, list.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect());
                }
                Value::Null | Value::Object(_) => {}
            }
        }
        ReportSnapshotFacts { scalars, lists }
    }

    pub fn string(&self, key: &str) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(self.scalars.get(key)?);
        if value.is_empty() {
            return None;
        }
        Some(value.to_owned())
    }

    pub fn int(&self, key: &str) -> Option<i64> {
        self.scalars.get(key).and_then(|value| value.parse::<i64>().ok())
    }

    pub fn int64(&self, key: &str) -> Option<i64> {
        self.scalars.get(key).and_then(|value| value.parse::<i64>().ok())
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.scalars.get(key).map(String::as_str) {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => None,
        }
    }

    pub fn list(&self, key: &str) -> Vec<String> {
        self.lists.get(key).cloned().unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.scalars.is_empty() && self.lists.is_empty()
    }
}

/// one report as the queue, the case and my reports all carry it
/// the three wire messages differ only in optional fields so one lenient shape reads them all
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportRowDTO {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at_ms: Option<i64>,
    pub report_type: String,
    pub status: String,
    pub reason: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_by_id: Option<String>,
    pub report_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<ModerationUserDTO>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_user: Option<ModerationUserDTO>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolver: Option<ModerationUserDTO>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_by_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moderation_action: Option<String>,
    pub content_deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_duration_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_end_at: Option<String>,
    pub warning_decrement: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vacancy_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub story_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_text: Option<String>,
    pub snapshot_media: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_json: Option<String>,
    pub snapshot_facts: ReportSnapshotFacts,
    pub evidence_file_urls: Vec<String>,
    pub evidence_file_ids: Vec<String>,
    pub evidence_count: i64,
    pub has_context: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reopened_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reopened_by_id: Option<String>,
}

impl<'de> Deserialize<'de> for ReportRowDTO {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        let Some(id) = c.int("id") else {
            return Err(<D::Error as serde::de::Error>::missing_field("id"));
        };
        let snapshot_json = c.string("snapshotJson");
        let snapshot_facts = ReportSnapshotFacts::parse(snapshot_json.as_deref());
        let evidence_file_urls = c.strings("evidenceFileUrls");
        let evidence_file_ids = c.strings("evidenceFileIds");
        let evidence_count = c.int("evidenceCount").unwrap_or(evidence_file_urls.len() as i64);
        Ok(Self {
            id,
            created_at: c.string("createdAt"),
            created_at_ms: c.int64("createdAtMs"),
            report_type: c.string("reportType").unwrap_or_default().to_uppercase(),
            status: c.string("status").unwrap_or_default().to_uppercase(),
            reason: c.string("reason").unwrap_or_default().to_uppercase(),
            description: c.string("description").unwrap_or_default(),
            decision: c.string("decision"),
            report_by_id: c.string("reportById"),
            report_count: c.int("reportCount").unwrap_or(1).max(1),
            author: c.lenient("author"),
            reported_user: c.lenient("reportedUser"),
            resolver: c.lenient("resolver"),
            resolved_by_id: c.string("resolvedById"),
            resolved_at_ms: c.int64("resolvedAtMs"),
            moderation_action: c.string("moderationAction"),
            content_deleted: c.bool("contentDeleted"),
            block_type: c.string("blockType"),
            block_duration_days: c.int("blockDurationDays"),
            block_end_at: c.string("blockEndAt"),
            warning_decrement: c.bool("warningDecrement"),
            post_id: c.int("postId"),
            comment_id: c.int("commentId"),
            chat_id: c.string("chatId"),
            user_id: c.string("userId"),
            vacancy_id: c.int("vacancyId"),
            message_id: c.string("messageId"),
            track_id: c.string("trackId"),
            artist_id: c.string("artistId"),
            story_id: c.int("storyId"),
            bot_id: c.string("botId"),
            target_user_id: c.string("targetUserId"),
            snapshot_title: c.string("snapshotTitle"),
            snapshot_text: c.string("snapshotText"),
            snapshot_media: c
                .strings("snapshotMedia")
                .iter()
                .filter_map(|media| RemoteMediaURL::optional_normalized(Some(media.as_str())))
                .collect(),
            snapshot_json,
            snapshot_facts,
            evidence_file_urls,
            evidence_file_ids,
            evidence_count,
            has_context: c.bool("hasContext"),
            reopened_at_ms: c.int64("reopenedAtMs"),
            reopened_by_id: c.string("reopenedById"),
        })
    }
}

impl ReportRowDTO {
    /// same normalisation the lenient decoder applies, for a row built from a canon reply
    #[allow(clippy::too_many_arguments)]
    pub fn with_canon_id(
        id: i64,
        created_at: String,
        created_at_ms: Option<i64>,
        report_type: String,
        status: String,
        reason: String,
        description: String,
        decision: Option<String>,
        report_by_id: String,
        report_count: i64,
        author: Option<ModerationUserDTO>,
        reported_user: Option<ModerationUserDTO>,
        resolver: Option<ModerationUserDTO>,
        resolved_by_id: Option<String>,
        resolved_at_ms: Option<i64>,
        moderation_action: Option<String>,
        content_deleted: bool,
        block_type: Option<String>,
        block_duration_days: Option<i64>,
        block_end_at: Option<String>,
        warning_decrement: bool,
        post_id: Option<i64>,
        comment_id: Option<i64>,
        chat_id: Option<String>,
        user_id: Option<String>,
        vacancy_id: Option<i64>,
        message_id: Option<String>,
        track_id: Option<String>,
        artist_id: Option<String>,
        story_id: Option<i64>,
        bot_id: Option<String>,
        target_user_id: Option<String>,
        snapshot_title: Option<String>,
        snapshot_text: Option<String>,
        snapshot_media: Option<String>,
        snapshot_json: Option<String>,
        evidence_file_urls: Vec<String>,
        evidence_file_ids: Vec<String>,
        evidence_count: i64,
        has_context: bool,
        reopened_at_ms: Option<i64>,
        reopened_by_id: Option<String>,
    ) -> Self {
        let snapshot_facts = ReportSnapshotFacts::parse(snapshot_json.as_deref());
        Self {
            id,
            created_at: if created_at.is_empty() { None } else { Some(created_at) },
            created_at_ms,
            report_type: report_type.to_uppercase(),
            status: status.to_uppercase(),
            reason: reason.to_uppercase(),
            description,
            decision,
            report_by_id: if report_by_id.is_empty() { None } else { Some(report_by_id) },
            report_count: report_count.max(1),
            author,
            reported_user,
            resolver,
            resolved_by_id,
            resolved_at_ms,
            moderation_action,
            content_deleted,
            block_type,
            block_duration_days,
            block_end_at,
            warning_decrement,
            post_id,
            comment_id,
            chat_id,
            user_id,
            vacancy_id,
            message_id,
            track_id,
            artist_id,
            story_id,
            bot_id,
            target_user_id,
            snapshot_title,
            snapshot_text,
            snapshot_media: Self::text_list(snapshot_media.as_deref())
                .iter()
                .filter_map(|media| RemoteMediaURL::optional_normalized(Some(media.as_str())))
                .collect(),
            snapshot_json,
            snapshot_facts,
            evidence_file_urls,
            evidence_file_ids,
            evidence_count,
            has_context,
            reopened_at_ms,
            reopened_by_id,
        }
    }

    fn text_list(value: Option<&str>) -> Vec<String> {
        value.map(text_list).unwrap_or_default()
    }

    pub fn with_resolution(&self, resolution: &ReportResolutionEvent) -> ReportRowDTO {
        let mut copy = self.clone();
        copy.status = resolution.status.clone();
        copy.decision = resolution.decision.clone().or_else(|| self.decision.clone());
        copy.moderation_action = resolution.action.clone().or_else(|| self.moderation_action.clone());
        copy.content_deleted = resolution.content_deleted;
        copy.block_type = resolution.block_type.clone();
        copy.block_duration_days = resolution.block_duration_days;
        copy
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportResolutionEvent {
    pub report_id: i64,
    pub status: String,
    pub decision: Option<String>,
    pub action: Option<String>,
    pub content_deleted: bool,
    pub block_type: Option<String>,
    pub block_duration_days: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportsPageDTO {
    pub reports: Vec<ReportRowDTO>,
    pub total: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_id: Option<i64>,
    pub is_have_more_bot_or_top: bool,
}

impl ReportsPageDTO {
    pub fn new(reports: Vec<ReportRowDTO>, total: i64, relative_id: Option<i64>, is_have_more_bot_or_top: bool) -> Self {
        Self { reports, total, relative_id, is_have_more_bot_or_top }
    }
}

impl<'de> Deserialize<'de> for ReportsPageDTO {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        let reports: Vec<ReportRowDTO> = c.lenient_rows("reports");
        let total = c.int("total").unwrap_or(reports.len() as i64);
        Ok(Self {
            reports,
            total,
            relative_id: c.int("relativeId"),
            is_have_more_bot_or_top: c.bool("isHaveMoreBotOrTop"),
        })
    }
}

pub struct ModerationDiscardedValue;

impl<'de> Deserialize<'de> for ModerationDiscardedValue {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let _ = IgnoredAny::deserialize(decoder);
        Ok(Self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppealDTO {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_type: Option<String>,
    pub appellant_id: String,
    pub text: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_by_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at_ms: Option<i64>,
    pub evidence_file_urls: Vec<String>,
    pub evidence_file_ids: Vec<String>,
}

impl<'de> Deserialize<'de> for AppealDTO {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        let Some(id) = c.int("id") else {
            return Err(<D::Error as serde::de::Error>::missing_field("id"));
        };
        Ok(Self {
            id,
            report_id: c.int("reportId").filter(|report_id| *report_id > 0),
            block_type: c
                .string("blockType")
                .filter(|block_type| !block_type.is_empty())
                .map(|block_type| block_type.to_uppercase()),
            appellant_id: c.string("appellantId").unwrap_or_default(),
            text: c.string("text").unwrap_or_default(),
            status: c.string("status").unwrap_or_else(|| AppealStatus::Pending.wire().to_owned()).to_uppercase(),
            reviewed_by_id: c.string("reviewedById"),
            reviewed_at: c.string("reviewedAt"),
            reviewed_at_ms: c.int64("reviewedAtMs"),
            decision_text: c.string("decisionText"),
            created_at: c.string("createdAt"),
            created_at_ms: c.int64("createdAtMs"),
            evidence_file_urls: c.strings("evidenceFileUrls"),
            evidence_file_ids: c.strings("evidenceFileIds"),
        })
    }
}

impl AppealDTO {
    /// true if this appeal argues against the block
    /// one appeal covers every block of a report
    /// a block outside a report takes an appeal of its type filed after it started
    pub fn concerns(&self, block: &RiseStoredUserBlock) -> bool {
        if let Some(report_id) = block.report_id
            && report_id > 0
        {
            return self.report_id == Some(report_id);
        }
        let Some(created_at_ms) = self.created_at_ms else {
            return false;
        };
        if self.report_id.is_some() || created_at_ms < block.block_start_timestamp {
            return false;
        }
        self.block_type.as_deref() == Some(block.block_type.as_str())
    }
}

/// an appeal with the sanction it argues against, both appeal lists carry this
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppealItemDTO {
    pub appeal: AppealDTO,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_by_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appellant: Option<ModerationUserDTO>,
}

impl AppealItemDTO {
    pub fn id(&self) -> i64 {
        self.appeal.id
    }
}

impl<'de> Deserialize<'de> for AppealItemDTO {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            appeal: c.required::<AppealDTO, D::Error>("appeal")?,
            report_type: c.string("reportType"),
            report_reason: c.string("reportReason"),
            snapshot_title: c.string("snapshotTitle"),
            snapshot_text: c.string("snapshotText"),
            original_decision: c.string("originalDecision"),
            block_type: c.string("blockType"),
            resolved_by_id: c.string("resolvedById"),
            appellant: c.lenient("appellant"),
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppealsPageDTO {
    pub appeals: Vec<AppealItemDTO>,
    pub total: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_id: Option<i64>,
    pub is_have_more_bot_or_top: bool,
}

impl AppealsPageDTO {
    pub fn new(appeals: Vec<AppealItemDTO>, total: i64, relative_id: Option<i64>, is_have_more_bot_or_top: bool) -> Self {
        Self { appeals, total, relative_id, is_have_more_bot_or_top }
    }
}

impl<'de> Deserialize<'de> for AppealsPageDTO {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        let appeals: Vec<AppealItemDTO> = c.lenient_rows("appeals");
        let total = c.int("total").unwrap_or(appeals.len() as i64);
        Ok(Self {
            appeals,
            total,
            relative_id: c.int("relativeId"),
            is_have_more_bot_or_top: c.bool("isHaveMoreBotOrTop"),
        })
    }
}

/// a sanction against the caller as my blocks lists it
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredUserBlock {
    pub block_type: String,
    pub block_start_timestamp: i64,
    pub block_end_timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_title: Option<String>,
    pub can_appeal: bool,
}

impl<'de> Deserialize<'de> for RiseStoredUserBlock {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            block_type: c.string("blockType").unwrap_or_default().to_uppercase(),
            block_start_timestamp: c.int64("blockStartTimestamp").unwrap_or(0),
            block_end_timestamp: c.int64("blockEndTimestamp").unwrap_or(0),
            reason: c.string("reason"),
            reason_code: c.string("reasonCode"),
            report_id: c.int("reportId"),
            report_type: c.string("reportType"),
            report_reason: c.string("reportReason"),
            snapshot_title: c.string("snapshotTitle"),
            can_appeal: c.bool("canAppeal"),
        })
    }
}

impl RiseStoredUserBlock {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        block_type: String,
        block_start_timestamp: i64,
        block_end_timestamp: i64,
        reason: Option<String>,
        reason_code: Option<String>,
        report_id: Option<i64>,
        report_type: Option<String>,
        report_reason: Option<String>,
        snapshot_title: Option<String>,
        can_appeal: bool,
    ) -> Self {
        Self {
            block_type,
            block_start_timestamp,
            block_end_timestamp,
            reason,
            reason_code,
            report_id,
            report_type,
            report_reason,
            snapshot_title,
            can_appeal,
        }
    }

    pub fn id(&self) -> String {
        format!("{}_{}_{}", self.block_type, self.report_id.unwrap_or(0), self.block_start_timestamp)
    }

    pub fn canon_reason_key(&self) -> Option<String> {
        let code = BlockReason::from_wire(self.reason_code.as_deref()?)?;
        Some(code.i18n_key().to_string())
    }

    pub fn with_can_appeal(&self, value: bool) -> RiseStoredUserBlock {
        RiseStoredUserBlock { can_appeal: value, ..self.clone() }
    }
}

/// where the caller stands on the warning ladder, as the server counts it
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredWarningState {
    pub count: i64,
    pub free_count: i64,
    pub next_is_block: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_block_days: Option<i64>,
}

impl<'de> Deserialize<'de> for RiseStoredWarningState {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            count: c.int("count").unwrap_or(0),
            free_count: c.int("freeCount").unwrap_or(0),
            next_is_block: c.bool("nextIsBlock"),
            next_block_days: c.int("nextBlockDays"),
        })
    }
}

impl RiseStoredWarningState {
    pub const EMPTY: RiseStoredWarningState =
        RiseStoredWarningState { count: 0, free_count: 0, next_is_block: false, next_block_days: None };

    pub fn new(count: i64, free_count: i64, next_is_block: bool, next_block_days: Option<i64>) -> Self {
        Self { count, free_count, next_is_block, next_block_days }
    }

    pub fn left_before_block(&self) -> i64 {
        (self.free_count - self.count).max(0)
    }
}

/// one decision taken against the caller, never names the moderator or reporter
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredSanctionHistoryItem {
    pub id: i64,
    pub action: String,
    pub created_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appeal_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_duration_days: Option<i64>,
    pub content_deleted: bool,
}

impl<'de> Deserialize<'de> for RiseStoredSanctionHistoryItem {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        let Some(id) = c.int64("id") else {
            return Err(<D::Error as serde::de::Error>::missing_field("id"));
        };
        Ok(Self {
            id,
            action: c.string("action").unwrap_or_default().to_uppercase(),
            created_at_ms: c.int64("createdAtMs").unwrap_or(0),
            report_id: c.int("reportId"),
            appeal_id: c.int("appealId"),
            report_type: c.string("reportType"),
            report_reason: c.string("reportReason"),
            report_status: c.string("reportStatus"),
            snapshot_title: c.string("snapshotTitle"),
            decision: c.string("decision"),
            block_type: c.string("blockType"),
            block_duration_days: c.int("blockDurationDays"),
            content_deleted: c.bool("contentDeleted"),
        })
    }
}

impl RiseStoredSanctionHistoryItem {
    pub fn resolved_action(&self) -> Option<ModerationAuditAction> {
        ModerationAuditAction::from_wire(&self.action)
    }

    pub fn resolved_report_type(&self) -> Option<ReportTargetType> {
        self.report_type.as_deref().and_then(|report_type| ReportTargetType::from_wire(&report_type.to_uppercase()))
    }

    pub fn resolved_report_reason(&self) -> Option<ReportReason> {
        self.report_reason.as_deref().and_then(|reason| ReportReason::from_wire(&reason.to_uppercase()))
    }

    pub fn resolved_block_type(&self) -> Option<UserBlockType> {
        self.block_type.as_deref().and_then(|block_type| UserBlockType::from_wire(&block_type.to_uppercase()))
    }

    pub fn is_appealed(&self) -> bool {
        self.report_status.as_deref().unwrap_or_default().to_uppercase() == ReportStatus::Appealed.wire()
    }

    /// removed content is appealed through its report, blocks keep their own card above
    pub fn allows_appeal(&self) -> bool {
        self.resolved_action() == Some(ModerationAuditAction::DeleteContent)
            && self.report_id.is_some()
            && self.appeal_id.is_none()
            && !self.is_appealed()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredAutoStanding {
    pub scope: String,
    pub channel: String,
    pub strikes: i64,
    pub strikes_to_mute: i64,
    pub muted_until_ms: i64,
    pub next_mute_ms: i64,
}

impl<'de> Deserialize<'de> for RiseStoredAutoStanding {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            scope: c.string("scope").unwrap_or_default(),
            channel: c.string("channel").unwrap_or_default(),
            strikes: c.int("strikes").unwrap_or(0),
            strikes_to_mute: c.int("strikesToMute").unwrap_or(0),
            muted_until_ms: c.int64("mutedUntilMs").unwrap_or(0),
            next_mute_ms: c.int64("nextMuteMs").unwrap_or(0),
        })
    }
}

impl RiseStoredAutoStanding {
    pub fn new(
        scope: String,
        channel: String,
        strikes: i64,
        strikes_to_mute: i64,
        muted_until_ms: i64,
        next_mute_ms: i64,
    ) -> Self {
        Self { scope, channel, strikes, strikes_to_mute, muted_until_ms, next_mute_ms }
    }

    pub fn id(&self) -> String {
        format!("{}|{}", self.scope, self.channel)
    }

    pub fn is_muted(&self, now_ms: i64) -> bool {
        self.muted_until_ms > now_ms
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredMyBlocks {
    pub blocks: Vec<RiseStoredUserBlock>,
    pub warnings: RiseStoredWarningState,
    pub history: Vec<RiseStoredSanctionHistoryItem>,
    pub auto_standings: Vec<RiseStoredAutoStanding>,
}

impl RiseStoredMyBlocks {
    pub fn new(
        blocks: Vec<RiseStoredUserBlock>,
        warnings: RiseStoredWarningState,
        history: Vec<RiseStoredSanctionHistoryItem>,
        auto_standings: Vec<RiseStoredAutoStanding>,
    ) -> Self {
        Self { blocks, warnings, history, auto_standings }
    }
}

impl<'de> Deserialize<'de> for RiseStoredMyBlocks {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            blocks: c.lenient_rows("blocks"),
            warnings: c.lenient("warnings").unwrap_or(RiseStoredWarningState::EMPTY),
            history: c.lenient_rows("history"),
            auto_standings: c.lenient("autoStandings").unwrap_or_default(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredModerationNote {
    pub id: i64,
    pub report_id: i64,
    pub author_id: String,
    pub text: String,
    pub created_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<ModerationUserDTO>,
    pub evidence_file_urls: Vec<String>,
    pub evidence_file_ids: Vec<String>,
}

impl<'de> Deserialize<'de> for RiseStoredModerationNote {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            id: c.int64("id").unwrap_or(0),
            report_id: c.int("reportId").unwrap_or(0),
            author_id: c.string("authorId").unwrap_or_default(),
            text: c.string("text").unwrap_or_default(),
            created_at_ms: c.int64("createdAtMs").unwrap_or(0),
            author: c.lenient("author"),
            evidence_file_urls: c.lenient("evidenceFileUrls").unwrap_or_default(),
            evidence_file_ids: c.lenient("evidenceFileIds").unwrap_or_default(),
        })
    }
}

impl RiseStoredModerationNote {
    pub fn evidence(&self) -> Vec<ReportEvidenceItem> {
        ReportEvidenceItem::items(&self.evidence_file_urls, &self.evidence_file_ids)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredCaseTimelineEntry {
    pub id: i64,
    pub actor_id: String,
    pub actor_role: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appeal_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_user_id: Option<String>,
    pub details_json: String,
    pub created_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<ModerationUserDTO>,
}

impl<'de> Deserialize<'de> for RiseStoredCaseTimelineEntry {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            id: c.int64("id").unwrap_or(0),
            actor_id: c.string("actorId").unwrap_or_default(),
            actor_role: c.string("actorRole").unwrap_or_default().to_uppercase(),
            action: c.string("action").unwrap_or_default().to_uppercase(),
            report_id: c.int("reportId"),
            appeal_id: c.int("appealId"),
            target_user_id: c.string("targetUserId"),
            details_json: c.string("detailsJson").unwrap_or_default(),
            created_at_ms: c.int64("createdAtMs").unwrap_or(0),
            actor: c.lenient("actor"),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredOffenseCounters {
    pub account_offenses: i64,
    pub comment_offenses: i64,
    pub post_offenses: i64,
    pub chat_offenses: i64,
    pub music_offenses: i64,
    pub warnings: i64,
}

impl<'de> Deserialize<'de> for RiseStoredOffenseCounters {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            account_offenses: c.int("accountOffenses").unwrap_or(0),
            comment_offenses: c.int("commentOffenses").unwrap_or(0),
            post_offenses: c.int("postOffenses").unwrap_or(0),
            chat_offenses: c.int("chatOffenses").unwrap_or(0),
            music_offenses: c.int("musicOffenses").unwrap_or(0),
            warnings: c.int("warnings").unwrap_or(0),
        })
    }
}

impl RiseStoredOffenseCounters {
    pub fn total(&self) -> i64 {
        self.account_offenses + self.comment_offenses + self.post_offenses + self.chat_offenses + self.music_offenses
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredTargetActiveBlock {
    pub block_type: String,
    pub block_start_timestamp: i64,
    pub block_end_timestamp: i64,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moderator_id: Option<String>,
}

impl<'de> Deserialize<'de> for RiseStoredTargetActiveBlock {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            block_type: c.string("blockType").unwrap_or_default().to_uppercase(),
            block_start_timestamp: c.int64("blockStartTimestamp").unwrap_or(0),
            block_end_timestamp: c.int64("blockEndTimestamp").unwrap_or(0),
            reason: c.string("reason").unwrap_or_default(),
            report_id: c.int("reportId"),
            moderator_id: c.string("moderatorId"),
        })
    }
}

impl RiseStoredTargetActiveBlock {
    pub fn id(&self) -> String {
        format!("{}_{}", self.block_type, self.block_start_timestamp)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredTargetIpDossier {
    pub ip: String,
    pub first_seen_at_ms: i64,
    pub last_seen_at_ms: i64,
    pub observation_count: i64,
    pub last_source: String,
    pub is_blocked: bool,
    pub linked_account_count: i64,
}

impl<'de> Deserialize<'de> for RiseStoredTargetIpDossier {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            ip: c.string("ip").unwrap_or_default(),
            first_seen_at_ms: c.int64("firstSeenAtMs").unwrap_or(0),
            last_seen_at_ms: c.int64("lastSeenAtMs").unwrap_or(0),
            observation_count: c.int64("observationCount").unwrap_or(0),
            last_source: c.string("lastSource").unwrap_or_default(),
            is_blocked: c.bool("isBlocked"),
            linked_account_count: c.int("linkedAccountCount").unwrap_or(0),
        })
    }
}

impl RiseStoredTargetIpDossier {
    pub fn id(&self) -> String {
        self.ip.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredTargetIntel {
    pub user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<ModerationUserDTO>,
    pub active_blocks: Vec<RiseStoredTargetActiveBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offense_counters: Option<RiseStoredOffenseCounters>,
    pub reports_received_total: i64,
    pub reports_received_open: i64,
    pub sanctions_total: i64,
    pub appeals_accepted: i64,
    pub reports_filed_total: i64,
    pub ip_dossiers: Vec<RiseStoredTargetIpDossier>,
    pub ip_dossiers_available: bool,
}

impl<'de> Deserialize<'de> for RiseStoredTargetIntel {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            user_id: c.string("userId").unwrap_or_default(),
            user: c.lenient("user"),
            active_blocks: c.lenient("activeBlocks").unwrap_or_default(),
            offense_counters: c.lenient("offenseCounters"),
            reports_received_total: c.int("reportsReceivedTotal").unwrap_or(0),
            reports_received_open: c.int("reportsReceivedOpen").unwrap_or(0),
            sanctions_total: c.int("sanctionsTotal").unwrap_or(0),
            appeals_accepted: c.int("appealsAccepted").unwrap_or(0),
            reports_filed_total: c.int("reportsFiledTotal").unwrap_or(0),
            ip_dossiers: c.lenient("ipDossiers").unwrap_or_default(),
            ip_dossiers_available: c.bool("ipDossiersAvailable"),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredCaseContextMessage {
    pub message_id: String,
    pub sender_id: String,
    pub content: String,
    pub content_type: String,
    pub created_at_ms: i64,
    pub is_reported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<ModerationUserDTO>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_duration_s: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_width: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_height: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sticker_emoji: Option<String>,
}

impl<'de> Deserialize<'de> for RiseStoredCaseContextMessage {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            message_id: c.string("messageId").unwrap_or_else(|| Uuid::new_v4().uuid_string()),
            sender_id: c.string("senderId").unwrap_or_default(),
            content: c.string("content").unwrap_or_default(),
            content_type: c
                .string("contentType")
                .unwrap_or_else(|| MessageContentType::Text.raw_value().to_owned())
                .to_lowercase(),
            created_at_ms: c.int64("createdAtMs").unwrap_or(0),
            is_reported: c.bool("isReported"),
            sender: c.lenient("sender"),
            media_type: c.string("mediaType").map(|media_type| media_type.to_lowercase()),
            media_url: c.string("mediaUrl"),
            thumbnail_url: c.string("thumbnailUrl"),
            media_duration_s: c.int("mediaDurationS"),
            media_width: c.int("mediaWidth"),
            media_height: c.int("mediaHeight"),
            mime_type: c.string("mimeType"),
            file_size: c.int64("fileSize"),
            file_name: c.string("fileName"),
            sticker_emoji: c.string("stickerEmoji"),
        })
    }
}

impl RiseStoredCaseContextMessage {
    pub fn id(&self) -> String {
        self.message_id.clone()
    }

    /// none when the window captured a bare text line
    /// addresses arrive site relative from report-service, the chat media resolver knows the host
    pub fn media_item(&self) -> Option<MessageMediaItem> {
        let address = self.media_url.as_deref().or(self.thumbnail_url.as_deref())?;
        if address.is_empty() {
            return None;
        }
        let kind = MessageMediaKind::from_raw(self.media_type.as_deref().unwrap_or_default())
            .unwrap_or_else(|| self.kind_from_content_type());
        let mut item = MessageMediaItem::new(
            format!("{}:context:0", self.message_id),
            kind,
            RemoteMediaURL::optional_normalized(Some(address)).unwrap_or_else(|| address.to_owned()),
        );
        item.thumbnail_url = RemoteMediaURL::optional_normalized(self.thumbnail_url.as_deref());
        item.width = self.media_width.unwrap_or(0) as f32;
        item.height = self.media_height.unwrap_or(0) as f32;
        if let Some(seconds) = self.media_duration_s
            && seconds > 0
        {
            item.duration = Some(seconds as f64);
        }
        item.file_name = self.file_name.clone();
        item.file_size = self.file_size;
        item.mime_type = self.mime_type.clone();
        if let Some(sticker_emoji) = self.sticker_emoji.as_deref().filter(|emoji| !emoji.is_empty()) {
            item.sticker_emojis = vec![sticker_emoji.to_owned()];
        }
        Some(item)
    }

    fn kind_from_content_type(&self) -> MessageMediaKind {
        match MessageContentType::from_raw_value(&self.content_type) {
            MessageContentType::Video => MessageMediaKind::Video,
            MessageContentType::Animation => MessageMediaKind::Animation,
            MessageContentType::Voice => MessageMediaKind::Voice,
            MessageContentType::Audio => MessageMediaKind::Audio,
            MessageContentType::Document => MessageMediaKind::Document,
            MessageContentType::Sticker => MessageMediaKind::Sticker,
            MessageContentType::VideoNote => MessageMediaKind::VideoNote,
            _ => MessageMediaKind::Image,
        }
    }
}

/// what this caller may do with this case, decided by the server
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredCaseCapabilities {
    pub can_dismiss: bool,
    pub can_warn: bool,
    pub can_delete_content: bool,
    pub can_block_author: bool,
    pub can_mute_author: bool,
    pub can_note: bool,
    pub can_reopen: bool,
    pub can_admin_sanction: bool,
    pub can_resolve_appeals: bool,
    pub is_admin: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_block_days: Option<i64>,
    pub delete_threshold_met: bool,
    pub delete_threshold: i64,
}

impl RiseStoredCaseCapabilities {
    pub const NONE: RiseStoredCaseCapabilities = RiseStoredCaseCapabilities::new();

    pub const fn new() -> Self {
        Self {
            can_dismiss: false,
            can_warn: false,
            can_delete_content: false,
            can_block_author: false,
            can_mute_author: false,
            can_note: false,
            can_reopen: false,
            can_admin_sanction: false,
            can_resolve_appeals: false,
            is_admin: false,
            max_block_days: None,
            delete_threshold_met: false,
            delete_threshold: 0,
        }
    }

    pub fn can_resolve(&self) -> bool {
        self.can_dismiss || self.can_warn || self.can_delete_content || self.can_block_author || self.can_mute_author
    }
}

impl<'de> Deserialize<'de> for RiseStoredCaseCapabilities {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            can_dismiss: c.bool("canDismiss"),
            can_warn: c.bool("canWarn"),
            can_delete_content: c.bool("canDeleteContent"),
            can_block_author: c.bool("canBlockAuthor"),
            can_mute_author: c.bool("canMuteAuthor"),
            can_note: c.bool("canNote"),
            can_reopen: c.bool("canReopen"),
            can_admin_sanction: c.bool("canAdminSanction"),
            can_resolve_appeals: c.bool("canResolveAppeals"),
            is_admin: c.bool("isAdmin"),
            max_block_days: c.int("maxBlockDays"),
            delete_threshold_met: c.bool("deleteThresholdMet"),
            delete_threshold: c.int("deleteThreshold").unwrap_or(0),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredReportCase {
    pub report: ReportRowDTO,
    pub siblings: Vec<ReportRowDTO>,
    pub sibling_total: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<RiseStoredTargetIntel>,
    pub timeline: Vec<RiseStoredCaseTimelineEntry>,
    pub notes: Vec<RiseStoredModerationNote>,
    pub appeals: Vec<AppealDTO>,
    pub context: Vec<RiseStoredCaseContextMessage>,
    pub capabilities: RiseStoredCaseCapabilities,
}

impl<'de> Deserialize<'de> for RiseStoredReportCase {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        let report = c.required::<ReportRowDTO, D::Error>("report")?;
        let siblings: Vec<ReportRowDTO> = c.lenient("siblings").unwrap_or_default();
        let sibling_total = c.int("siblingTotal").unwrap_or(siblings.len() as i64);
        Ok(Self {
            report,
            siblings,
            sibling_total,
            target: c.lenient("target"),
            timeline: c.lenient("timeline").unwrap_or_default(),
            notes: c.lenient("notes").unwrap_or_default(),
            appeals: c.lenient("appeals").unwrap_or_default(),
            context: c.lenient("context").unwrap_or_default(),
            capabilities: c.lenient("capabilities").unwrap_or(RiseStoredCaseCapabilities::NONE),
        })
    }
}

impl RiseStoredReportCase {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        report: ReportRowDTO,
        siblings: Vec<ReportRowDTO>,
        sibling_total: i64,
        target: Option<RiseStoredTargetIntel>,
        timeline: Vec<RiseStoredCaseTimelineEntry>,
        notes: Vec<RiseStoredModerationNote>,
        appeals: Vec<AppealDTO>,
        context: Vec<RiseStoredCaseContextMessage>,
        capabilities: RiseStoredCaseCapabilities,
    ) -> Self {
        Self { report, siblings, sibling_total, target, timeline, notes, appeals, context, capabilities }
    }

    /// every report on this object, the opened one first
    pub fn all_reports(&self) -> Vec<ReportRowDTO> {
        std::iter::once(self.report.clone())
            .chain(self.siblings.iter().filter(|sibling| sibling.id != self.report.id).cloned())
            .collect()
    }

    pub fn with_notes(&self, notes: Vec<RiseStoredModerationNote>) -> RiseStoredReportCase {
        RiseStoredReportCase { notes, ..self.clone() }
    }

    pub fn with_report(&self, report: ReportRowDTO) -> RiseStoredReportCase {
        RiseStoredReportCase { report, ..self.clone() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredAppealCase {
    pub appeal: AppealDTO,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appellant: Option<ModerationUserDTO>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<ModerationUserDTO>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_case: Option<RiseStoredReportCase>,
    pub capabilities: RiseStoredCaseCapabilities,
}

impl<'de> Deserialize<'de> for RiseStoredAppealCase {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        let appeal = c.required::<AppealDTO, D::Error>("appeal")?;
        let report_case: Option<RiseStoredReportCase> = c.lenient("reportCase");
        let capabilities = c
            .lenient("capabilities")
            .or_else(|| report_case.as_ref().map(|report_case| report_case.capabilities.clone()))
            .unwrap_or(RiseStoredCaseCapabilities::NONE);
        Ok(Self {
            appeal,
            appellant: c.lenient("appellant"),
            reviewer: c.lenient("reviewer"),
            report_case,
            capabilities,
        })
    }
}

impl RiseStoredAppealCase {
    pub fn new(
        appeal: AppealDTO,
        appellant: Option<ModerationUserDTO>,
        reviewer: Option<ModerationUserDTO>,
        report_case: Option<RiseStoredReportCase>,
        capabilities: RiseStoredCaseCapabilities,
    ) -> Self {
        Self { appeal, appellant, reviewer, report_case, capabilities }
    }

    pub fn with_appeal(&self, appeal: AppealDTO) -> RiseStoredAppealCase {
        RiseStoredAppealCase { appeal, ..self.clone() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredAdminSanction {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<i64>,
    pub changed: bool,
}

impl<'de> Deserialize<'de> for RiseStoredAdminSanction {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let c = KeyedDecodingContainer::read(decoder)?;
        Ok(Self {
            kind: c.string("kind").unwrap_or_default().to_uppercase(),
            action_id: c.string("actionId"),
            expires_at_ms: c.int64("expiresAtMs"),
            changed: c.bool("changed"),
        })
    }
}

pub type UserBlockDTO = RiseStoredUserBlock;
pub type WarningStateDTO = RiseStoredWarningState;
pub type SanctionHistoryItemDTO = RiseStoredSanctionHistoryItem;
