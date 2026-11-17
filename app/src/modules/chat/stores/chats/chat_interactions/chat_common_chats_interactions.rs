use gpui::Context;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use super::chat_interactions::ChatInteractionsStore;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

impl ChatInteractionsStore {
    pub fn load_common_chats(&self, peer_user_id: &str, force_refresh: bool, cx: &mut Context<Self>) {
        let peer = trimming_whitespaces_and_newlines(peer_user_id);
        if peer.is_empty() {
            return;
        }
        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.get_common_chats_action(peer, None, force_refresh, cx));
    }

    pub fn load_more_common_chats(&self, peer_user_id: &str, cx: &mut Context<Self>) {
        let peer = trimming_whitespaces_and_newlines(peer_user_id);
        let actions = ChatActionsStore::shared(cx);
        if peer.is_empty() || actions.read(cx).get_common_chats_sai.is_pending() {
            return;
        }
        let (has_more, relative_id) = ChatServicesStore::shared(cx).update(cx, |services, cx| {
            (services.common_chats_has_more(peer, cx), services.common_chats_relative_id(peer, cx))
        });
        let Some(relative_id) = relative_id.filter(|relative_id| has_more && !relative_id.is_empty()) else {
            return;
        };
        actions.update(cx, |actions, cx| actions.get_common_chats_action(peer, Some(&relative_id), false, cx));
    }
}
