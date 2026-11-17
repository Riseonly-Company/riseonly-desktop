use std::collections::HashMap;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::canon_errors::CanonErrors;
use crate::modules::stickers::engine::rise_pack_collaboration_repository::{
    RisePackCollaborationFailure, RisePackInviteLink, RisePackInvitePreview, RisePackJoinRequest, RisePackJoinResult,
    RisePackRights,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_actions::StickersActionsStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_coauthors_types::PackInviteLinkDraft;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

pub type PackCollaborationFailureCallback = Box<dyn FnOnce(String, &mut App)>;

impl StickersInteractionsStore {
    fn collaboration_actions(cx: &mut App) -> Entity<StickersActionsStore> {
        StickersActionsStore::shared(cx)
    }

    fn collaboration_service(cx: &mut App) -> Entity<StickersServicesStore> {
        StickersServicesStore::shared(cx)
    }

    pub fn load_pack_invite_links(
        &mut self,
        pack_id: &str,
        force: bool,
        on_failure: Option<PackCollaborationFailureCallback>,
        cx: &mut Context<Self>,
    ) {
        if pack_id.is_empty() {
            return;
        }
        let service = Self::collaboration_service(cx);
        if !force && service.read(cx).pack_invite_links(pack_id).is_some() {
            return;
        }
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.load_pack_invite_links_action(pack_id, false, Self::collaboration_failure(on_failure), cx);
        });
    }

    pub fn load_more_pack_invite_links(&mut self, pack_id: &str, cx: &mut Context<Self>) {
        let service = Self::collaboration_service(cx);
        if !service.read(cx).pack_invite_links_has_more(pack_id) {
            return;
        }
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.load_pack_invite_links_action(pack_id, true, Self::collaboration_failure(None), cx);
        });
    }

    pub fn create_pack_invite_link(
        &mut self,
        pack_id: &str,
        draft: PackInviteLinkDraft,
        on_success: impl FnOnce(RisePackInviteLink, &mut App) + 'static,
        on_failure: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.create_pack_invite_link_action(
                pack_id,
                Self::normalized_draft(draft),
                move |result, cx| Self::route(result, on_success, on_failure, cx),
                cx,
            );
        });
    }

    pub fn edit_pack_invite_link(
        &mut self,
        link: &RisePackInviteLink,
        draft: PackInviteLinkDraft,
        on_success: impl FnOnce(RisePackInviteLink, &mut App) + 'static,
        on_failure: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.edit_pack_invite_link_action(
                link.clone(),
                Self::normalized_draft(draft),
                move |result, cx| Self::route(result, on_success, on_failure, cx),
                cx,
            );
        });
    }

    pub fn revoke_pack_invite_link(
        &mut self,
        link: &RisePackInviteLink,
        on_success: impl FnOnce(&mut App) + 'static,
        on_failure: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.revoke_pack_invite_link_action(
                link.clone(),
                move |result, cx| Self::route(result, |_, cx| on_success(cx), on_failure, cx),
                cx,
            );
        });
    }

    pub fn load_pack_link_joiners(
        &mut self,
        link_id: &str,
        more: bool,
        on_failure: Option<PackCollaborationFailureCallback>,
        cx: &mut Context<Self>,
    ) {
        if link_id.is_empty() {
            return;
        }
        let service = Self::collaboration_service(cx);
        if more && !service.read(cx).pack_link_joiners_has_more(link_id) {
            return;
        }
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.load_pack_link_joiners_action(link_id, more, Self::collaboration_failure(on_failure), cx);
        });
    }

    pub fn load_pack_invite_preview(
        &mut self,
        code: &str,
        on_success: impl FnOnce(RisePackInvitePreview, &mut App) + 'static,
        on_failure: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let code = trimming_whitespaces_and_newlines(code).to_owned();
        if code.is_empty() {
            return;
        }
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.load_pack_invite_preview_action(
                &code,
                move |result, cx| Self::route(result, on_success, on_failure, cx),
                cx,
            );
        });
    }

    pub fn join_pack_by_link(
        &mut self,
        code: &str,
        on_success: impl FnOnce(RisePackJoinResult, &mut App) + 'static,
        on_failure: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.join_pack_by_link_action(code, move |result, cx| Self::route(result, on_success, on_failure, cx), cx);
        });
    }

    pub fn load_pack_join_requests(
        &mut self,
        pack_id: &str,
        force: bool,
        more: bool,
        on_failure: Option<PackCollaborationFailureCallback>,
        cx: &mut Context<Self>,
    ) {
        if pack_id.is_empty() {
            return;
        }
        let service = Self::collaboration_service(cx);
        if more {
            if !service.read(cx).pack_join_requests_has_more(pack_id) {
                return;
            }
        } else if !force && service.read(cx).pack_join_requests(pack_id).is_some() {
            return;
        }
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.load_pack_join_requests_action(pack_id, more, Self::collaboration_failure(on_failure), cx);
        });
    }

    pub fn review_pack_join_request(
        &mut self,
        request: &RisePackJoinRequest,
        approve: bool,
        on_success: impl FnOnce(&mut App) + 'static,
        on_failure: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.review_pack_join_request_action(
                request.clone(),
                approve,
                move |result, cx| Self::route(result, |_, cx| on_success(cx), on_failure, cx),
                cx,
            );
        });
    }

    pub fn load_pack_coauthors(
        &mut self,
        pack_id: &str,
        force: bool,
        more: bool,
        on_failure: Option<PackCollaborationFailureCallback>,
        cx: &mut Context<Self>,
    ) {
        if pack_id.is_empty() {
            return;
        }
        let service = Self::collaboration_service(cx);
        if more {
            if !service.read(cx).pack_coauthors_has_more(pack_id) {
                return;
            }
        } else if !force && service.read(cx).pack_coauthors(pack_id).is_some() {
            return;
        }
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.load_pack_coauthors_action(pack_id, more, Self::collaboration_failure(on_failure), cx);
        });
    }

    pub fn update_pack_coauthor_rights(
        &mut self,
        pack_id: &str,
        coauthor_id: &str,
        rights: RisePackRights,
        on_success: impl FnOnce(&mut App) + 'static,
        on_failure: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.update_pack_coauthor_rights_action(
                pack_id,
                coauthor_id,
                rights,
                move |result, cx| Self::route(result, |_, cx| on_success(cx), on_failure, cx),
                cx,
            );
        });
    }

    pub fn remove_pack_coauthor(
        &mut self,
        pack_id: &str,
        coauthor_id: &str,
        on_success: impl FnOnce(&mut App) + 'static,
        on_failure: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.remove_pack_coauthor_action(
                pack_id,
                coauthor_id,
                move |result, cx| Self::route(result, |_, cx| on_success(cx), on_failure, cx),
                cx,
            );
        });
    }

    pub fn leave_pack(
        &mut self,
        pack_id: &str,
        on_success: impl FnOnce(&mut App) + 'static,
        on_failure: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.leave_pack_action(
                pack_id,
                move |result, cx| Self::route(result, |_, cx| on_success(cx), on_failure, cx),
                cx,
            );
        });
    }

    pub fn transfer_pack_ownership(
        &mut self,
        pack_id: &str,
        new_owner_id: &str,
        on_success: impl FnOnce(&mut App) + 'static,
        on_failure: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        Self::collaboration_actions(cx).update(cx, |actions, cx| {
            actions.transfer_pack_ownership_action(
                pack_id,
                new_owner_id,
                move |result, cx| Self::route(result, |_, cx| on_success(cx), on_failure, cx),
                cx,
            );
        });
    }

    fn normalized_draft(mut draft: PackInviteLinkDraft) -> PackInviteLinkDraft {
        let name = draft.name.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or_default().to_owned();
        draft.name = if name.is_empty() { None } else { Some(name) };
        if draft.usage_limit.is_some_and(|limit| limit <= 0) {
            draft.usage_limit = None;
        }
        draft
    }

    fn route<Value>(
        result: anyhow::Result<Value>,
        on_success: impl FnOnce(Value, &mut App),
        on_failure: impl FnOnce(String, &mut App),
        cx: &mut App,
    ) {
        match result {
            Ok(value) => on_success(value, cx),
            Err(error) => on_failure(Self::failure_text(&error), cx),
        }
    }

    fn collaboration_failure(
        on_failure: Option<PackCollaborationFailureCallback>,
    ) -> impl FnOnce(anyhow::Result<()>, &mut App) + 'static {
        move |result, cx| {
            if let Err(error) = result
                && let Some(on_failure) = on_failure
            {
                on_failure(Self::failure_text(&error), cx);
            }
        }
    }

    pub fn failure_text(error: &anyhow::Error) -> String {
        if let Some(refusal) = error.downcast_ref::<RisePackCollaborationFailure>()
            && let Some(text) = CanonErrors::text(Some(refusal.code.as_str()), &HashMap::new())
        {
            return text;
        }
        CanonErrors::refusal_text(error)
    }
}
