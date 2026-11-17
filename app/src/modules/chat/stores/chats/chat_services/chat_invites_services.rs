use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_key::chat as key;
use rise_canon::{RiseEvent, RiseLimits};
use rise_catalog::ChatInviteJoinStatus;
use rise_chat::JoinRequestStatus;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_errors::ErrorCode;
use rise_i18n::tr;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatInfo, ChatJoinPreview, ChatPerm, ChatPermExt, ChatType, InviteLink, JoinLinkSheetPhase, JoinLinkSheetState,
    JoinRequest,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    GetInviteLinksGatewayDTO, GetJoinRequestsGatewayDTO, InviteLinkStatsWireDTO, InviteLinkWireDTO,
    InviteLinkWireDTOExt, JoinRequestWireDTOExt, RiseStoredChat,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

#[derive(Clone, Copy, Debug)]
pub struct JoinRequestsTotalMark {
    pub total: Option<i64>,
    pub shift: i64,
    pub in_flight: i64,
}

impl ChatServicesStore {
    pub(crate) fn observed_gateway_dto<T: DeserializeOwned + 'static>(cache_key: &str, cx: &mut App) -> Option<Rc<T>> {
        SaiWsResponseCache::observed_projection(
            cache_key,
            std::any::type_name::<T>(),
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            |payload| SaiJSONCodec::decode::<T>(payload).map(Rc::new),
            cx,
        )
    }

    pub fn invite_links_cache_key(chat_id: &str) -> String {
        SaiCache::new(["getInviteLinks", chat_id]).key
    }

    fn invite_links_dto(&self, chat_id: &str, cx: &mut App) -> Option<Rc<GetInviteLinksGatewayDTO>> {
        Self::observed_gateway_dto(&Self::invite_links_cache_key(chat_id), cx)
    }

    pub fn invite_links(&self, chat_id: &str, cx: &mut App) -> Vec<InviteLink> {
        let mut links: Vec<InviteLink> = self
            .invite_links_dto(chat_id, cx)
            .map(|dto| dto.invite_links.iter().map(|link| link.to_invite_link()).collect())
            .unwrap_or_default();
        links.sort_by(|lhs, rhs| {
            rhs.is_active()
                .cmp(&lhs.is_active())
                .then_with(|| rhs.is_primary.cmp(&lhs.is_primary))
                .then_with(|| rhs.created_at.total_cmp(&lhs.created_at))
        });
        links
    }

    pub fn primary_invite_link(&self, chat_id: &str, cx: &mut App) -> Option<InviteLink> {
        let links = self.invite_links(chat_id, cx);
        links
            .iter()
            .find(|link| link.is_primary && link.is_active())
            .or_else(|| links.iter().find(|link| link.is_active()))
            .cloned()
    }

    pub fn invite_link_stats(&self, chat_id: &str, cx: &mut App) -> Option<InviteLinkStatsWireDTO> {
        self.invite_links_dto(chat_id, cx).and_then(|dto| dto.stats.clone())
    }

    pub fn begin_join_link_preview(&mut self, token: &str, force: bool, cx: &mut Context<Self>) -> Option<Uuid> {
        let token = trimming_whitespaces_and_newlines(token);
        if token.is_empty() {
            return None;
        }

        if let Some(current) = &self.join_link_sheet {
            if current.is_submitting() {
                return None;
            }
            if current.token == token
                && !force
                && matches!(current.phase, JoinLinkSheetPhase::Loading | JoinLinkSheetPhase::Preview(_))
            {
                return None;
            }
        }

        let state = JoinLinkSheetState::new(token.to_owned());
        let request_id = state.preview_request_id;
        self.join_link_sheet = Some(state);
        cx.notify();
        Some(request_id)
    }

    pub fn retry_join_link_preview(&mut self, cx: &mut Context<Self>) -> Option<(String, Uuid)> {
        let state = self.join_link_sheet.as_mut()?;
        if state.is_loading() || state.is_submitting() {
            return None;
        }

        state.preview_request_id = Uuid::new_v4();
        state.phase = JoinLinkSheetPhase::Loading;
        state.submission_error = None;
        let retried = (state.token.clone(), state.preview_request_id);
        cx.notify();
        Some(retried)
    }

    pub fn resolve_join_link_preview(
        &mut self,
        token: &str,
        request_id: Uuid,
        preview: Option<ChatJoinPreview>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(state) = self.join_link_sheet.as_mut() else {
            return false;
        };
        if state.token != token || state.preview_request_id != request_id {
            return false;
        }

        state.phase = preview.map(JoinLinkSheetPhase::Preview).unwrap_or(JoinLinkSheetPhase::Empty);
        state.submission_error = None;
        cx.notify();
        true
    }

    pub fn is_loading_join_link_preview(&self, token: &str, request_id: Uuid) -> bool {
        self.join_link_sheet
            .as_ref()
            .is_some_and(|state| state.token == token && state.preview_request_id == request_id && state.is_loading())
    }

    pub fn fail_join_link_preview(
        &mut self,
        token: &str,
        request_id: Uuid,
        message: String,
        retryable: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(state) = self.join_link_sheet.as_mut() else {
            return false;
        };
        if state.token != token || state.preview_request_id != request_id {
            return false;
        }

        state.phase = JoinLinkSheetPhase::Failed(message, retryable);
        state.submission_error = None;
        cx.notify();
        true
    }

    pub fn begin_join_link_submission(&mut self, chat_id: &str, token: &str, cx: &mut Context<Self>) -> Option<Uuid> {
        let state = self.join_link_sheet.as_mut()?;
        if state.token != token
            || !state.preview().is_some_and(|preview| preview.chat_id == chat_id && preview.can_join)
            || state.is_submitting()
        {
            return None;
        }

        let submission_id = Uuid::new_v4();
        state.submission_id = Some(submission_id);
        state.submission_error = None;
        cx.notify();
        Some(submission_id)
    }

    pub fn finish_join_link_submission(
        &mut self,
        submission_id: Uuid,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(state) = self.join_link_sheet.as_mut() else {
            return false;
        };
        if state.submission_id != Some(submission_id) {
            return false;
        }

        state.submission_id = None;
        state.submission_error = error;
        cx.notify();
        true
    }

    pub fn dismiss_join_link_sheet(&mut self, cx: &mut Context<Self>) -> bool {
        if self.join_link_sheet.as_ref().is_some_and(|state| state.is_submitting()) {
            return false;
        }
        self.join_link_sheet = None;
        cx.notify();
        true
    }

    pub fn localized_join_preview_failure(&self, raw: Option<&str>) -> String {
        let value = self.normalized_join_failure(raw);
        if value.contains("expired") {
            return tr("join_link_expired");
        }
        if self.is_dead_link_failure(&value) {
            return tr("join_link_invalid");
        }
        tr("join_preview_load_failed")
    }

    pub fn join_preview_failure(&self, join_status: Option<&str>, raw: Option<&str>) -> (String, bool) {
        let status = self.normalized_join_failure(join_status);
        if status == ChatInviteJoinStatus::LinkExpired.wire() {
            return (tr("join_link_expired"), false);
        }
        if status == ChatInviteJoinStatus::LinkUsageLimitReached.wire() {
            return (tr("join_link_limit_reached"), false);
        }
        if status == ChatInviteJoinStatus::LinkInvalid.wire()
            || status == ChatInviteJoinStatus::LinkRevoked.wire()
            || status == ChatInviteJoinStatus::ChatUnavailable.wire()
        {
            return (tr("join_link_invalid"), false);
        }
        if status == ChatInviteJoinStatus::VoiceUnavailable.wire() {
            return (tr("join_voice_unavailable"), false);
        }
        if status == ChatInviteJoinStatus::VoiceForbidden.wire() {
            return (tr("join_voice_forbidden"), false);
        }
        if status == ChatInviteJoinStatus::TopicUnavailable.wire() {
            return (tr("join_topic_unavailable"), false);
        }
        if status == ChatInviteJoinStatus::TopicForbidden.wire() {
            return (tr("join_topic_forbidden"), false);
        }
        if status == ChatInviteJoinStatus::MembersOnly.wire() {
            return (tr("join_link_members_only"), false);
        }
        let value = self.normalized_join_failure(raw);
        let dead = value.contains("expired") || self.is_dead_link_failure(&value);
        (self.localized_join_preview_failure(raw), !dead)
    }

    fn is_dead_link_failure(&self, value: &str) -> bool {
        value.contains("revoked")
            || value.contains("invalid")
            || value.contains("not found")
            || value.contains("no longer valid")
    }

    pub fn localized_join_submission_failure(&self, raw: Option<&str>, requesting: bool) -> String {
        let value = self.normalized_join_failure(raw);
        if value.contains("banned") {
            return tr("join_banned");
        }
        if value.contains("expired") {
            return tr("join_link_expired");
        }
        if self.is_dead_link_failure(&value) {
            return tr("join_link_invalid");
        }
        if requesting { tr("join_request_failed") } else { tr("join_link_failed") }
    }

    pub fn is_banned_join_failure(&self, raw: Option<&str>) -> bool {
        self.normalized_join_failure(raw).contains("banned")
    }

    fn join_error_has_code(error: &anyhow::Error, code: ErrorCode) -> bool {
        CanonErrors::failure(error).is_some_and(|failure| failure.code.as_deref() == Some(code.wire()))
    }

    fn join_error_response_message(error: &anyhow::Error) -> Option<&str> {
        error.downcast_ref::<SaiWsResponseError>().map(|response| response.message.as_str())
    }

    pub fn is_banned_join_error(&self, error: &anyhow::Error) -> bool {
        if Self::join_error_has_code(error, ErrorCode::ChatMemberBanned) {
            return true;
        }
        self.is_banned_join_failure(Self::join_error_response_message(error))
    }

    pub fn localized_join_submission_error(&self, error: &anyhow::Error, requesting: bool) -> String {
        if self.is_banned_join_error(error) {
            return tr("join_banned");
        }
        let description = error.to_string();
        let raw = Self::join_error_response_message(error).unwrap_or(&description);
        self.localized_join_submission_failure(Some(raw), requesting)
    }

    pub fn is_already_member_join_error(&self, error: &anyhow::Error) -> bool {
        if Self::join_error_has_code(error, ErrorCode::ChatAlreadyMember) {
            return true;
        }
        self.is_already_member_join_failure(Self::join_error_response_message(error))
    }

    pub fn is_already_member_join_failure(&self, raw: Option<&str>) -> bool {
        let value = self.normalized_join_failure(raw);
        value.contains("already") && value.contains("member")
    }

    pub fn is_pending_join_request_error(&self, error: &anyhow::Error) -> bool {
        if Self::join_error_has_code(error, ErrorCode::ChatApprovalRequired) {
            return true;
        }
        self.is_pending_join_request_failure(Self::join_error_response_message(error))
    }

    pub fn is_pending_join_request_failure(&self, raw: Option<&str>) -> bool {
        let value = self.normalized_join_failure(raw);
        value.contains("already") && value.contains("pending")
    }

    pub fn reconcile_joined_chat(
        &mut self,
        chat_id: &str,
        chat: Option<&RiseStoredChat>,
        cx: &mut Context<Self>,
    ) -> String {
        let chat_info = chat.map(|chat| chat.to_chat_info());
        let joined_id = chat_info.as_ref().map_or_else(|| chat_id.to_owned(), |info| info.id.clone());
        self.left_chat_ids.remove(&joined_id);
        self.left_chat_ids.remove(chat_id);
        self.confirmed_removed_chat_ids.remove(&joined_id);
        self.confirmed_removed_chat_ids.remove(chat_id);
        if let Some(chat_info) = chat_info {
            self.upsert_chat(chat_info, cx);
        }
        cx.notify();
        joined_id
    }

    fn normalized_join_failure(&self, raw: Option<&str>) -> String {
        raw.map(|raw| trimming_whitespaces_and_newlines(raw).to_lowercase()).unwrap_or_default()
    }

    pub fn insert_invite_link(&mut self, wire: InviteLinkWireDTO, chat_id: &str, cx: &mut Context<Self>) {
        let Some(row) = SaiJSONCodec::encode(&wire) else {
            return;
        };

        let _ = SaiUpdater::insert_row_if_missing(
            &Self::invite_links_cache_key(chat_id),
            key::INVITE_LINKS,
            key::ID,
            row,
            true,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        cx.notify();
    }

    pub fn invite_link_raw_row(&self, chat_id: &str, link_id: &str, cx: &mut App) -> Option<Map<String, Value>> {
        let dto = self.invite_links_dto(chat_id, cx)?;
        let wire = dto.invite_links.iter().find(|link| link.id == link_id)?;
        SaiJSONCodec::encode(wire)
    }

    pub fn remove_invite_link(&mut self, chat_id: &str, link_id: &str, cx: &mut Context<Self>) {
        let _ = SaiUpdater::remove_row(
            &Self::invite_links_cache_key(chat_id),
            key::INVITE_LINKS,
            key::ID,
            link_id,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        cx.notify();
    }

    pub fn restore_invite_link(&mut self, row: Map<String, Value>, chat_id: &str, cx: &mut Context<Self>) {
        let _ = SaiUpdater::insert_row_if_missing(
            &Self::invite_links_cache_key(chat_id),
            key::INVITE_LINKS,
            key::ID,
            row,
            true,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        cx.notify();
    }

    pub fn join_requests_page_limit() -> i64 {
        RiseLimits::client_value("chat_services_store_join_requests_page_limit")
    }

    pub fn join_requests_cache_key(chat_id: &str) -> String {
        SaiCache::new(["getJoinRequests", chat_id]).key
    }

    fn join_requests_dto(&self, chat_id: &str, cx: &mut App) -> Option<Rc<GetJoinRequestsGatewayDTO>> {
        Self::observed_gateway_dto(&Self::join_requests_cache_key(chat_id), cx)
    }

    pub fn join_requests(&self, chat_id: &str, cx: &mut App) -> Vec<JoinRequest> {
        let mut requests: Vec<JoinRequest> = self
            .join_requests_dto(chat_id, cx)
            .map(|dto| {
                dto.requests
                    .iter()
                    .map(|request| request.to_join_request())
                    .filter(|request| request.is_pending())
                    .collect()
            })
            .unwrap_or_default();
        requests.sort_by(|lhs, rhs| rhs.requested_at.total_cmp(&lhs.requested_at));
        requests
    }

    pub fn pending_join_request_count(&self, chat_id: &str, cx: &mut App) -> i64 {
        self.join_requests(chat_id, cx).len() as i64
    }

    pub fn join_requests_have_more(&self, chat_id: &str, cx: &mut App) -> bool {
        self.join_requests_dto(chat_id, cx).is_some_and(|dto| dto.have_more)
    }

    pub fn join_requests_count_label(&self, chat_id: &str, cx: &mut App) -> (i64, bool) {
        let loaded = self.join_requests(chat_id, cx).len() as i64;
        let total = self.join_requests_dto(chat_id, cx).map_or(0, |dto| dto.total_count);
        if total >= loaded && total > 0 {
            return (total, true);
        }
        (loaded, !self.join_requests_have_more(chat_id, cx))
    }

    fn shift_join_requests_total(&mut self, chat_id: &str, delta: i64, cx: &mut Context<Self>) {
        *self.join_requests_local_shift.entry(chat_id.to_owned()).or_insert(0) += delta;
        let Some(total) = self.join_requests_dto(chat_id, cx).map(|dto| dto.total_count) else {
            return;
        };
        if total <= 0 && delta <= 0 {
            return;
        }
        self.patch_join_requests_total(chat_id, (total + delta).max(0), cx);
    }

    fn patch_join_requests_total(&self, chat_id: &str, value: i64, cx: &mut App) {
        let mut fields = Map::new();
        fields.insert(key::TOTAL_COUNT.to_owned(), Value::from(value));
        let _ = SaiUpdater::patch_root(
            &Self::join_requests_cache_key(chat_id),
            &fields,
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    pub fn join_requests_total_mark(&self, chat_id: &str, cx: &mut App) -> JoinRequestsTotalMark {
        JoinRequestsTotalMark {
            total: self.join_requests_dto(chat_id, cx).map(|dto| dto.total_count),
            shift: self.join_requests_local_shift.get(chat_id).copied().unwrap_or(0),
            in_flight: self.join_request_decisions_in_flight.get(chat_id).copied().unwrap_or(0),
        }
    }

    pub fn reconcile_join_requests_total(&mut self, chat_id: &str, mark: &JoinRequestsTotalMark, cx: &mut Context<Self>) {
        let shift = self.join_requests_local_shift.get(chat_id).copied().unwrap_or(0);
        let overlapped = mark.in_flight > 0
            || shift != mark.shift
            || self.join_request_decisions_in_flight.get(chat_id).copied().unwrap_or(0) > 0;
        if !overlapped {
            return;
        }
        let Some(before) = mark.total.filter(|before| *before > 0) else {
            return;
        };
        self.patch_join_requests_total(chat_id, (before + shift - mark.shift).max(0), cx);
        cx.notify();
    }

    pub fn begin_join_request_decision(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        *self.join_request_decisions_in_flight.entry(chat_id.to_owned()).or_insert(0) += 1;
        cx.notify();
    }

    pub fn end_join_request_decision(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let remaining = (self.join_request_decisions_in_flight.get(chat_id).copied().unwrap_or(0) - 1).max(0);
        self.join_request_decisions_in_flight.insert(chat_id.to_owned(), remaining);
        cx.notify();
    }

    pub fn join_requests_next_relative_id(&self, chat_id: &str, cx: &mut App) -> Option<String> {
        if let Some(relative_id) = self
            .join_requests_dto(chat_id, cx)
            .and_then(|dto| dto.relative_id.clone())
            .filter(|relative_id| !relative_id.is_empty())
        {
            return Some(relative_id);
        }
        self.join_requests(chat_id, cx).last().map(|request| request.id.clone())
    }

    pub fn can_show_join_banner(&self, chat: &ChatInfo, cx: &mut App) -> bool {
        if chat.r#type != ChatType::Group && chat.r#type != ChatType::Channel {
            return false;
        }
        self.can_review_join_requests(chat)
            && !self.dismissed_join_banner_chat_ids.contains(&chat.id)
            && self.pending_join_request_count(&chat.id, cx) > 0
    }

    pub fn can_review_join_requests(&self, chat: &ChatInfo) -> bool {
        chat.can_accept_join_requests
            || ChatPerm::is_owner_role(chat.role.as_deref())
            || (ChatPerm::is_owner_or_legacy_admin_role(chat.role.as_deref()) && chat.can_invite_users)
    }

    pub fn dismiss_join_banner(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.dismissed_join_banner_chat_ids.insert(chat_id.to_owned());
        cx.notify();
    }

    pub fn restore_join_banner(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.dismissed_join_banner_chat_ids.remove(chat_id);
        cx.notify();
    }

    pub fn join_request_raw_row(&self, chat_id: &str, request_id: &str, cx: &mut App) -> Option<Map<String, Value>> {
        let dto = self.join_requests_dto(chat_id, cx)?;
        let wire = dto.requests.iter().find(|request| request.id == request_id)?;
        SaiJSONCodec::encode(wire)
    }

    pub fn remove_join_request(&mut self, chat_id: &str, request_id: &str, cx: &mut Context<Self>) {
        let was_loaded = self
            .join_requests_dto(chat_id, cx)
            .is_some_and(|dto| dto.requests.iter().any(|request| request.id == request_id));
        let _ = SaiUpdater::remove_row(
            &Self::join_requests_cache_key(chat_id),
            key::REQUESTS,
            key::ID,
            request_id,
            SaiUpdaterWriteScope::All,
            cx,
        );
        if was_loaded {
            self.shift_join_requests_total(chat_id, -1, cx);
        }
        cx.notify();
    }

    pub fn restore_join_request(&mut self, row: Map<String, Value>, chat_id: &str, cx: &mut Context<Self>) {
        let request_id = row.get(key::ID).and_then(Value::as_str).map(str::to_owned);
        let was_loaded = self.join_requests_dto(chat_id, cx).is_none_or(|dto| {
            dto.requests.iter().any(|request| request_id.as_deref() == Some(request.id.as_str()))
        });
        let _ = SaiUpdater::insert_row_if_missing(
            &Self::join_requests_cache_key(chat_id),
            key::REQUESTS,
            key::ID,
            row,
            true,
            SaiUpdaterWriteScope::All,
            cx,
        );
        if !was_loaded {
            self.shift_join_requests_total(chat_id, 1, cx);
        }
        cx.notify();
    }

    pub fn join_request_created_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::JoinRequestCreated(event)) =
            RiseEvent::decode_payload(rise_events::name::JOIN_REQUEST_CREATED, any)
        else {
            return;
        };

        let chat_id = event.chat_id;
        let request_id = event.request_id;
        if chat_id.is_empty() || request_id.is_empty() {
            return;
        }

        let mut user_row = Map::new();
        let mut more_row = Map::new();
        if let Some(info) = &event.user_info {
            user_row.insert(key::ID.to_owned(), Value::from(info.id.clone()));
            user_row.insert(key::NAME.to_owned(), Value::from(info.name.clone()));
            user_row.insert(key::TAG.to_owned(), Value::from(info.tag.clone()));
            user_row.insert(key::IS_PREMIUM.to_owned(), Value::from(info.is_premium));
            user_row.insert(key::IS_OFFICIAL.to_owned(), Value::from(info.is_official));
            more_row.insert(key::LOGO.to_owned(), Value::from(info.more.logo.clone()));
        }
        user_row.insert(key::MORE.to_owned(), Value::Object(more_row));

        let user_id = if event.user_id.is_empty() {
            event.user_info.as_ref().map(|info| info.id.clone()).unwrap_or_default()
        } else {
            event.user_id
        };
        let status =
            if event.status.is_empty() { JoinRequestStatus::Pending.wire().to_owned() } else { event.status };

        let mut row = Map::new();
        row.insert(key::ID.to_owned(), Value::from(request_id.clone()));
        row.insert(key::CHAT_ID.to_owned(), Value::from(chat_id.clone()));
        row.insert(key::USER_ID.to_owned(), Value::from(user_id));
        row.insert(key::STATUS.to_owned(), Value::from(status));
        row.insert(key::USER.to_owned(), Value::Object(user_row));
        if event.requested_at != 0 {
            row.insert(key::REQUESTED_AT.to_owned(), Value::from(event.requested_at));
        }
        if let Some(message) = event.message.filter(|message| !message.is_empty()) {
            row.insert(key::MESSAGE.to_owned(), Value::from(message));
        }

        let was_loaded = self
            .join_requests_dto(&chat_id, cx)
            .is_none_or(|dto| dto.requests.iter().any(|request| request.id == request_id));
        let _ = SaiUpdater::insert_row_if_missing(
            &Self::join_requests_cache_key(&chat_id),
            key::REQUESTS,
            key::ID,
            row,
            true,
            SaiUpdaterWriteScope::All,
            cx,
        );
        if !was_loaded {
            self.shift_join_requests_total(&chat_id, 1, cx);
        }

        self.dismissed_join_banner_chat_ids.remove(&chat_id);
        cx.notify();
    }

    pub fn join_request_processed_handler(&mut self, any: Option<&Value>, name: &str, cx: &mut Context<Self>) {
        let event = match RiseEvent::decode_payload(name, any) {
            Some(RiseEvent::JoinRequestApproved(approved)) => approved,
            Some(RiseEvent::JoinRequestRejected(rejected)) => rejected,
            _ => return,
        };

        if event.chat_id.is_empty() || event.request_id.is_empty() {
            return;
        }

        let my_id =
            AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default();
        if !my_id.is_empty() && event.user_id == my_id {
            if event.status.to_uppercase() == JoinRequestStatus::Approved.wire() {
                self.refetch_chat_as_member(&event.chat_id, cx);
                Self::send_join_request_toast(tr("join_request_approved_toast"), NotifierType::Success, cx);
            } else {
                Self::send_join_request_toast(tr("join_request_rejected_toast"), NotifierType::Info, cx);
            }
            return;
        }

        self.remove_join_request(&event.chat_id, &event.request_id, cx);
    }

    fn send_join_request_toast(title: String, r#type: NotifierType, cx: &mut App) {
        Notifier::send(
            title,
            NotifierSendOptions { r#type, position: NotifierPosition::Bottom, ..NotifierSendOptions::default() },
            cx,
        );
    }
}
