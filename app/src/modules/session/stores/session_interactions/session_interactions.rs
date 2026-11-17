use gpui::{App, Context, Entity};
use rise_limits::limit::ACTIVE_SESSIONS;

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::SaiWsPaginationEdge;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::session::stores::session_actions::session_actions::SessionActionsStore;
use crate::modules::session::stores::session_actions::session_types::SessionEntry;
use crate::modules::session::stores::session_services::session_services::SessionServicesStore;

pub struct SessionInteractionsStore;

impl SessionInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self
    }

    fn actions(&self, cx: &mut App) -> Entity<SessionActionsStore> {
        SessionActionsStore::shared(cx)
    }

    fn service(&self, cx: &mut App) -> Entity<SessionServicesStore> {
        SessionServicesStore::shared(cx)
    }

    pub fn ensure_sessions_loaded(&self, cx: &mut App) {
        let actions = self.actions(cx);
        if actions.read(cx).get_sessions_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| {
            actions.get_sessions_action(None, ACTIVE_SESSIONS as i64, true, cx);
        });
    }

    pub fn load_more_sessions(&self, cx: &mut App) {
        let Some(user_id) = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| profile.id.clone())
            .filter(|user_id| !user_id.is_empty())
        else {
            return;
        };
        let cache_key = SessionServicesStore::sessions_cache_key(&user_id);
        let can_request_automatically = SaiWs::pagination_state(&cache_key, cx)
            .read(cx)
            .state(SaiWsPaginationEdge::Trailing)
            .can_request_automatically();
        if !can_request_automatically || !SessionServicesStore::sessions_have_more(cx) {
            return;
        }
        let Some(cursor) = SessionServicesStore::sessions_relative_id(cx) else {
            return;
        };
        let actions = self.actions(cx);
        actions.update(cx, |actions, cx| {
            actions.get_sessions_action(Some(cursor), ACTIVE_SESSIONS as i64, true, cx);
        });
    }

    pub fn refresh_sessions(&self, cx: &mut App) {
        let actions = self.actions(cx);
        actions.update(cx, |actions, cx| {
            actions.get_sessions_action(None, ACTIVE_SESSIONS as i64, true, cx);
        });
    }

    pub fn terminate_session(&self, id: &str, cx: &mut App) {
        let sandbox = GuideSandbox::shared(cx);
        if sandbox.read(cx).owns_sessions_scene() {
            sandbox.update(cx, |sandbox, cx| sandbox.end_practice_sessions(vec![id.to_owned()], cx));
            return;
        }
        let snap = SessionServicesStore::sessions(cx);
        let rows: Vec<SessionEntry> = snap.iter().filter(|session| session.id != id).cloned().collect();
        self.service(cx).update(cx, |service, cx| {
            service.begin_mutation(snap, cx);
            service.apply_optimistic_sessions(rows, cx);
        });
        let actions = self.actions(cx);
        actions.update(cx, |actions, cx| actions.delete_sessions_action(Some(id.to_owned()), false, cx));
    }

    pub fn terminate_all_other(&self, cx: &mut App) {
        let sandbox = GuideSandbox::shared(cx);
        if sandbox.read(cx).owns_sessions_scene() {
            let ids: Vec<String> = SessionServicesStore::other_sessions(cx)
                .into_iter()
                .map(|session| session.id)
                .collect();
            sandbox.update(cx, |sandbox, cx| sandbox.end_practice_sessions(ids, cx));
            return;
        }
        let snap = SessionServicesStore::sessions(cx);
        let rows: Vec<SessionEntry> = snap.iter().filter(|session| session.is_current).cloned().collect();
        self.service(cx).update(cx, |service, cx| {
            service.begin_mutation(snap, cx);
            service.apply_optimistic_sessions(rows, cx);
        });
        let actions = self.actions(cx);
        actions.update(cx, |actions, cx| actions.delete_sessions_action(None, true, cx));
    }

    pub fn terminate_current_session(&self, cx: &mut App) {
        if GuideSandbox::shared(cx).read(cx).owns_sessions_scene() {
            return;
        }
        let Some(current_id) = SessionServicesStore::current_session(cx)
            .map(|session| session.id)
            .filter(|current_id| !current_id.is_empty())
        else {
            return;
        };
        let actions = self.actions(cx);
        actions.update(cx, |actions, cx| actions.delete_sessions_action(Some(current_id), false, cx));
    }
}
