use std::future::Future;
use std::sync::Arc;

use gpui::{App, AsyncApp, Context};
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::stickers::engine::rise_pack_collaboration_repository::{
    RisePackCoauthor, RisePackCollaborationRepository, RisePackInviteLink, RisePackInvitePreview, RisePackJoinRequest,
    RisePackJoinResult, RisePackRights,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_actions::StickersActionsStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_coauthors_types::PackInviteLinkDraft;

impl StickersActionsStore {
    pub fn load_pack_invite_links_action(
        &mut self,
        pack_id: &str,
        more: bool,
        on_result: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        self.collaborate(
            move |repository| async move { repository.invite_links(&pack_id, more).await.map(|_| ()) },
            on_result,
            cx,
        );
    }

    pub fn create_pack_invite_link_action(
        &mut self,
        pack_id: &str,
        draft: PackInviteLinkDraft,
        on_result: impl FnOnce(anyhow::Result<RisePackInviteLink>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        self.collaborate(
            move |repository| async move {
                repository
                    .create_invite_link(
                        &pack_id,
                        draft.name,
                        draft.expires_at_ms,
                        draft.usage_limit,
                        draft.creates_join_request,
                    )
                    .await
            },
            on_result,
            cx,
        );
    }

    pub fn edit_pack_invite_link_action(
        &mut self,
        link: RisePackInviteLink,
        draft: PackInviteLinkDraft,
        on_result: impl FnOnce(anyhow::Result<RisePackInviteLink>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.collaborate(
            move |repository| async move {
                repository
                    .edit_invite_link(&link, draft.name, draft.expires_at_ms, draft.usage_limit, draft.creates_join_request)
                    .await
            },
            on_result,
            cx,
        );
    }

    pub fn revoke_pack_invite_link_action(
        &mut self,
        link: RisePackInviteLink,
        on_result: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.collaborate(move |repository| async move { repository.revoke_invite_link(&link).await }, on_result, cx);
    }

    pub fn load_pack_link_joiners_action(
        &mut self,
        link_id: &str,
        more: bool,
        on_result: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let link_id = link_id.to_owned();
        self.collaborate(
            move |repository| async move { repository.link_joiners(&link_id, more).await.map(|_| ()) },
            on_result,
            cx,
        );
    }

    pub fn load_pack_invite_preview_action(
        &mut self,
        code: &str,
        on_result: impl FnOnce(anyhow::Result<RisePackInvitePreview>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let code = code.to_owned();
        self.collaborate(move |repository| async move { repository.invite_preview(&code).await }, on_result, cx);
    }

    pub fn join_pack_by_link_action(
        &mut self,
        code: &str,
        on_result: impl FnOnce(anyhow::Result<RisePackJoinResult>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let code = code.to_owned();
        self.collaborate(move |repository| async move { repository.join(&code).await }, on_result, cx);
    }

    pub fn load_pack_join_requests_action(
        &mut self,
        pack_id: &str,
        more: bool,
        on_result: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        self.collaborate(
            move |repository| async move { repository.join_requests(&pack_id, more).await.map(|_| ()) },
            on_result,
            cx,
        );
    }

    pub fn review_pack_join_request_action(
        &mut self,
        request: RisePackJoinRequest,
        approve: bool,
        on_result: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.collaborate(
            move |repository| async move { repository.review_join_request(&request, approve).await },
            on_result,
            cx,
        );
    }

    pub fn load_pack_coauthors_action(
        &mut self,
        pack_id: &str,
        more: bool,
        on_result: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        self.collaborate(
            move |repository| async move { repository.coauthors(&pack_id, more).await.map(|_| ()) },
            on_result,
            cx,
        );
    }

    pub fn update_pack_coauthor_rights_action(
        &mut self,
        pack_id: &str,
        coauthor_id: &str,
        rights: RisePackRights,
        on_result: impl FnOnce(anyhow::Result<RisePackCoauthor>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        let coauthor_id = coauthor_id.to_owned();
        self.collaborate(
            move |repository| async move { repository.update_rights(&pack_id, &coauthor_id, &rights).await },
            on_result,
            cx,
        );
    }

    pub fn remove_pack_coauthor_action(
        &mut self,
        pack_id: &str,
        coauthor_id: &str,
        on_result: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        let coauthor_id = coauthor_id.to_owned();
        self.collaborate(
            move |repository| async move { repository.remove_coauthor(&pack_id, &coauthor_id).await },
            on_result,
            cx,
        );
    }

    pub fn leave_pack_action(
        &mut self,
        pack_id: &str,
        on_result: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        self.collaborate(move |repository| async move { repository.leave(&pack_id).await }, on_result, cx);
    }

    pub fn transfer_pack_ownership_action(
        &mut self,
        pack_id: &str,
        new_owner_id: &str,
        on_result: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        let new_owner_id = new_owner_id.to_owned();
        self.collaborate(
            move |repository| async move { repository.transfer_ownership(&pack_id, &new_owner_id).await },
            on_result,
            cx,
        );
    }

    fn collaborate<Value, Operation, Work>(
        &mut self,
        operation: Operation,
        on_result: impl FnOnce(anyhow::Result<Value>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) where
        Value: Send + 'static,
        Operation: FnOnce(Arc<RisePackCollaborationRepository>) -> Work + 'static,
        Work: Future<Output = anyhow::Result<Value>> + Send + 'static,
    {
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match RiseConversationActionExecutor::require_domain(cx).await {
                Ok(domain) => {
                    match RiseAccountRuntime::current().spawn(operation(Arc::clone(&domain.pack_collaboration))).await {
                        Ok(result) => result,
                        Err(_) => Err(CancellationError.into()),
                    }
                }
                Err(error) => Err(error),
            };
            if let Err(error) = &result {
                let text = format!("pack collaboration failed: {error}");
                cx.update(|cx| Sai::log("Stickers", move || text, SaiLogKind::Network, cx));
            }
            cx.update(|cx| on_result(result, cx));
        })
        .detach();
    }
}
