use std::any::Any;

use gpui::{App, Context, Entity};
use serde_json::Value;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::services::account_state_reset_coordinator::AccountStateResetReason;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::session::stores::session_actions::session_actions::SessionActionsStore;
use crate::modules::session::stores::session_actions::session_types::SessionEntry;
use crate::modules::session::stores::session_actions::session_wire_types::{
    DeleteUserSessionsGatewayDTO, GetUserSessionsGatewayDTO,
};

#[derive(Default)]
pub struct SessionServicesStore {
    optimistic_sessions: Option<Vec<SessionEntry>>,
    mutation_snapshot: Option<Vec<SessionEntry>>,
    is_mutating: bool,
    last_error_text: Option<String>,
}

impl SessionServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self::default()
    }

    pub fn optimistic_sessions(&self) -> Option<&[SessionEntry]> {
        self.optimistic_sessions.as_deref()
    }

    pub fn is_mutating(&self) -> bool {
        self.is_mutating
    }

    pub fn last_error_text(&self) -> Option<&str> {
        self.last_error_text.as_deref()
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.optimistic_sessions = None;
        self.mutation_snapshot = None;
        self.is_mutating = false;
        self.last_error_text = None;
        cx.notify();
    }

    pub fn sessions_cache_key(user_id: &str) -> String {
        SaiCache::new(["getUserSessions", user_id]).key
    }

    fn cached_sessions_dto(cx: &mut App) -> Option<GetUserSessionsGatewayDTO> {
        let auth = AuthServicesStore::shared(cx);
        let user_id = auth.read(cx).profile()?.id.clone();
        if user_id.is_empty() {
            return None;
        }

        let payload: Value = SaiWsResponseCache::observed_payload(
            &Self::sessions_cache_key(&user_id),
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )?;

        SaiJSONCodec::decode::<GetUserSessionsGatewayDTO>(Some(&payload))
    }

    pub fn sessions(cx: &mut App) -> Vec<SessionEntry> {
        let loaded = Self::loaded_sessions(cx);
        GuideSandbox::shared(cx).read(cx).practice_sessions(loaded)
    }

    fn loaded_sessions(cx: &mut App) -> Vec<SessionEntry> {
        if let Some(optimistic_sessions) = Self::shared(cx).read(cx).optimistic_sessions.clone() {
            return optimistic_sessions;
        }
        let current_id = AuthServicesStore::shared(cx)
            .read(cx)
            .tokens_and_other_data()
            .session_id
            .clone();
        let rows = Self::cached_sessions_dto(cx)
            .map(|dto| dto.sessions)
            .unwrap_or_default();
        Self::normalize(
            rows.iter()
                .map(|row| SessionEntry::from(row, &current_id))
                .collect(),
        )
    }

    pub fn current_session(cx: &mut App) -> Option<SessionEntry> {
        let sessions = Self::sessions(cx);
        sessions
            .iter()
            .find(|session| session.is_current)
            .or_else(|| sessions.first())
            .cloned()
    }

    pub fn other_sessions(cx: &mut App) -> Vec<SessionEntry> {
        let current_id = Self::current_session(cx).map(|session| session.id);
        Self::sessions(cx)
            .into_iter()
            .filter(|session| Some(&session.id) != current_id.as_ref())
            .collect()
    }

    pub fn did_load(cx: &mut App) -> bool {
        SessionActionsStore::shared(cx).read(cx).get_sessions_sai.status() == SaiWsSliceStatus::Fulfilled
    }

    pub fn sessions_relative_id(cx: &mut App) -> Option<String> {
        Self::cached_sessions_dto(cx)?.relative_id
    }

    pub fn sessions_have_more(cx: &mut App) -> bool {
        Self::cached_sessions_dto(cx)
            .map(|dto| dto.is_have_more)
            .unwrap_or(false)
    }

    pub fn error_text(cx: &mut App) -> Option<String> {
        if let Some(last_error_text) = Self::shared(cx).read(cx).last_error_text.clone() {
            return Some(last_error_text);
        }
        SessionActionsStore::shared(cx).read(cx).get_sessions_sai.error()
    }

    pub fn begin_mutation(&mut self, snapshot: Vec<SessionEntry>, cx: &mut Context<Self>) {
        self.mutation_snapshot = Some(snapshot.clone());
        self.optimistic_sessions = Some(snapshot);
        self.is_mutating = true;
        cx.notify();
    }

    pub fn apply_optimistic_sessions(&mut self, rows: Vec<SessionEntry>, cx: &mut Context<Self>) {
        self.optimistic_sessions = Some(rows);
        cx.notify();
    }

    pub fn commit_mutation(&mut self, cx: &mut Context<Self>) {
        self.mutation_snapshot = None;
        self.optimistic_sessions = None;
        self.is_mutating = false;
        cx.notify();
    }

    pub fn rollback_mutation(&mut self, cx: &mut Context<Self>) {
        self.optimistic_sessions = self.mutation_snapshot.take();
        self.is_mutating = false;
        cx.notify();
    }

    pub fn get_sessions_success_handler(&mut self, cx: &mut Context<Self>) {
        self.optimistic_sessions = None;
        self.last_error_text = None;
        cx.notify();
    }

    pub fn get_sessions_error_handler(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        let text = CanonErrors::refusal_text(error);
        self.last_error_text = Some(text.clone());
        cx.notify();
        Sai::log(
            "Session",
            || format!("get_user_sessions error: {text}"),
            SaiLogKind::Network,
            cx,
        );
    }

    pub fn delete_sessions_success_handler(
        payload: Option<Box<dyn Any>>,
        user_id: &str,
        session_id_to_terminate: Option<&str>,
        is_all: bool,
        cx: &mut App,
    ) {
        Self::shared(cx).update(cx, |service, cx| {
            service.commit_mutation(cx);
            service.last_error_text = None;
            cx.notify();
        });

        let dto = Self::decoded_delete_payload(payload);

        if dto.as_ref().is_some_and(|dto| dto.should_logout) {
            AuthServicesStore::shared(cx).update(cx, |auth, cx| {
                auth.full_clear(AccountStateResetReason::ForcedLogout, cx);
            });
            return;
        }

        let cache_key = Self::sessions_cache_key(user_id);
        let rows = if let Some(dto) = dto {
            dto.remaining_sessions
        } else if is_all {
            Self::sessions(cx)
                .iter()
                .filter(|session| session.is_current)
                .map(SessionEntry::as_wire_dto)
                .collect()
        } else if let Some(sid) = session_id_to_terminate.filter(|sid| !sid.is_empty()) {
            Self::sessions(cx)
                .iter()
                .filter(|session| session.id != sid)
                .map(SessionEntry::as_wire_dto)
                .collect()
        } else {
            return;
        };
        let sessions_dto = GetUserSessionsGatewayDTO {
            error: String::new(),
            is_have_more: false,
            limit: 0,
            relative_id: None,
            sessions: rows,
            success: true,
        };

        if let Some(root) = SaiJSONCodec::encode(&sessions_dto) {
            let _ = SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::All, cx);
        }
    }

    fn decoded_delete_payload(payload: Option<Box<dyn Any>>) -> Option<DeleteUserSessionsGatewayDTO> {
        match payload?.downcast::<DeleteUserSessionsGatewayDTO>() {
            Ok(dto) => Some(*dto),
            Err(payload) => SaiJSONCodec::decode::<DeleteUserSessionsGatewayDTO>(payload.downcast_ref::<Value>()),
        }
    }

    pub fn delete_sessions_error_handler(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.rollback_mutation(cx);
        let text = CanonErrors::refusal_text(error);
        self.last_error_text = Some(text.clone());
        cx.notify();
        Sai::log(
            "Session",
            || format!("delete_user_sessions error: {text}"),
            SaiLogKind::Network,
            cx,
        );
    }

    pub fn normalize(rows: Vec<SessionEntry>) -> Vec<SessionEntry> {
        let (current, mut others): (Vec<SessionEntry>, Vec<SessionEntry>) =
            rows.into_iter().partition(|row| row.is_current);
        others.sort_by(|left, right| right.last_active.cmp(&left.last_active));
        current.into_iter().chain(others).collect()
    }
}
