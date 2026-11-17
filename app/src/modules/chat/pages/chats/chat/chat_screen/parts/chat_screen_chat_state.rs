use gpui::{App, Context, Window};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::modules::chat::components::chat::composer::chat_composer_types::resign_chat_keyboard;
use crate::modules::chat::shared::chat_permission_resolution::ChatPermissionResolution;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatPerm, ChatPermExt, ChatPermKey, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;

impl ChatScreen {
    pub(in super::super) fn is_context_preview(&self, cx: &App) -> bool {
        self.context_preview.as_ref().is_some_and(|session| {
            let session = session.read(cx);
            session.is_preview() || session.is_transferring()
        })
    }

    pub(in super::super) fn presence_observation_owner(&self, cx: &mut App) -> String {
        if self.context_preview.is_none() {
            format!("chat-{}", self.resolved_chat_id(cx))
        } else {
            format!("chat-preview-{}", self.rise_message_observation_owner_id)
        }
    }

    pub(in super::super) fn resolved_chat_id(&self, cx: &mut App) -> String {
        let raw = self.chat_id.clone().unwrap_or_default();
        ChatServicesStore::shared(cx).read(cx).real_chat_id(&raw, cx).unwrap_or(raw)
    }

    pub(in super::super) fn join_requests_gate_chat_id(&self, cx: &mut App) -> String {
        let chat = self.resolved_chat(cx);
        if chat.r#type != ChatType::Group && chat.r#type != ChatType::Channel {
            return String::new();
        }
        let can_review = chat.can_accept_join_requests
            || chat.can_manage_chat
            || ChatPerm::is_owner_or_legacy_admin_role(chat.role.as_deref());
        if can_review { chat.id } else { String::new() }
    }

    pub(in super::super) fn accepts_polls(&self, cx: &mut App) -> bool {
        match self.resolved_chat(cx).r#type {
            ChatType::Favourites => false,
            ChatType::Direct => true,
            _ => {
                let chat_id = self.resolved_chat_id(cx);
                let post_topic_id = self.post_topic_id(cx);
                !chat_services(cx, |store, cx| {
                    store.member_send_switched_off_forum_topic_id(
                        &chat_id,
                        ChatPermKey::POLLS,
                        post_topic_id.as_deref(),
                        cx,
                    )
                })
            }
        }
    }

    pub(in super::super) fn link_previews_switched_off(&self, cx: &mut App) -> bool {
        let chat_id = self.resolved_chat_id(cx);
        let post_topic_id = self.post_topic_id(cx);
        chat_services(cx, |store, cx| store.member_link_previews_switched_off(&chat_id, post_topic_id.as_deref(), cx))
    }

    pub(in super::super) fn resolved_thread_root_id(&self) -> Option<&str> {
        self.thread_root_id.as_deref().filter(|root| !root.is_empty())
    }

    pub(in super::super) fn message_scope(&self, cx: &mut App) -> String {
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.resolved_topic_id(cx);
        MessageServicesStore::message_scope(&chat_id, topic_id.as_deref(), self.resolved_thread_root_id())
    }

    /// keeps existing per tab reading positions when history moves to per topic store views
    pub(in super::super) fn scroll_position_key(&self, cx: &mut App) -> String {
        if !self.is_topics_mode(cx) {
            return self.message_scope(cx);
        }
        let chat_id = self.resolved_chat_id(cx);
        match &self.state.selected_topic_tab {
            ChatTopicTab::All => chat_id,
            ChatTopicTab::General => format!("{chat_id}::tab::general"),
            ChatTopicTab::Topic(id) => format!("{chat_id}::tab::{id}"),
        }
    }

    /// a channel subscriber only reads and the server refuses their posts so no composer
    /// an edit from a message menu still gets one since editing isnt posting
    pub(in super::super) fn channel_posting_locked(&self, cx: &mut App) -> bool {
        if self.state.edit_target.is_some() {
            return false;
        }
        let chat = self.resolved_chat(cx);
        chat.r#type == ChatType::Channel && !ChatPermissionResolution::can_post_to_channel(chat.role.as_deref())
    }

    pub(in super::super) fn is_pending_chat(&self, cx: &mut App) -> bool {
        self.resolved_chat_id(cx).starts_with("temp_")
    }

    pub(in super::super) fn is_message_request(&self, cx: &mut App) -> bool {
        let chat = self.resolved_chat(cx);
        chat.is_message_request && chat.message_request_state.as_deref().unwrap_or("PENDING").to_uppercase() == "PENDING"
    }

    /// someone removed from a public community sees neither its history nor a join button
    pub(in super::super) fn viewer_removed_from_community(&self, cx: &mut App) -> bool {
        let chat = self.resolved_chat(cx);
        self.viewer_is_outside(&chat, cx) && (self.public_join_banned || chat.is_removed_from_community())
    }

    /// outside the community, a public preview or a channel reader in the comments of its discussion group
    /// they join before writing
    pub(in super::super) fn viewer_is_outside(&self, chat: &ChatInfo, cx: &mut App) -> bool {
        if chat.is_community_preview() {
            return true;
        }
        self.is_thread_screen()
            && self.has_resolved_chat_metadata(cx)
            && ChatServicesStore::is_discussion_guest_card(chat)
    }

    /// comments of a discussion group the viewer isnt in
    /// the join plate says why it replaces the composer and a refused history load says the same
    pub(in super::super) fn shows_thread_members_only_hint(&self, cx: &mut App) -> bool {
        self.is_thread_screen() && self.is_community_outsider(cx) && !self.viewer_removed_from_community(cx)
    }

    pub(in super::super) fn has_resolved_chat_metadata(&self, cx: &mut App) -> bool {
        let chat_id = self.resolved_chat_id(cx);
        ChatServicesStore::shared(cx).read(cx).chat_info(&chat_id, cx).is_some()
    }

    pub(in super::super) fn has_no_card_or_hint(&self, cx: &mut App) -> bool {
        let chat_id = self.resolved_chat_id(cx);
        !self.is_pending_chat(cx)
            && !chat_id.is_empty()
            && !self.has_resolved_chat_metadata(cx)
            && ChatServicesStore::shared(cx).read(cx).opening_hint(&chat_id).is_none()
    }

    /// a link to an id nobody knows, no metadata and no history and a refused first load
    /// nothing to write to so the screen keeps only the error and its retry
    pub(in super::super) fn chat_unavailable(&self, cx: &mut App) -> bool {
        if !self.has_no_card_or_hint(cx) {
            return false;
        }
        let scope = self.message_scope(cx);
        message_services(cx, |store, cx| {
            store.in_memory_message_count(&scope, cx) == 0
                && store.initial_messages_status(&scope, cx) == SaiWsSliceStatus::Rejected
        })
    }

    /// a link to a chat this device has no card for yet
    /// until it lands the screen cant tell direct from community
    /// so it shows a loading header and no composer
    pub(in super::super) fn chat_metadata_loading(&self, cx: &mut App) -> bool {
        self.has_no_card_or_hint(cx) && !self.chat_unavailable(cx)
    }

    /// only a plain group has a voice chat
    /// so no busy state subscription for a forum or direct chat or topic
    pub(in super::super) fn group_voice_observation_chat_id(&self, cx: &mut App) -> String {
        if !self.has_resolved_chat_metadata(cx) {
            return String::new();
        }
        let chat = self.resolved_chat(cx);
        if chat.r#type != ChatType::Group
            || chat.topics_enabled
            || self.is_community_outsider(cx)
            || !self.topic_id.as_deref().unwrap_or("").is_empty()
        {
            return String::new();
        }
        self.resolved_chat_id(cx)
    }

    pub(in super::super) fn permission_mask_load_chat_id(&self, cx: &mut App) -> String {
        if !self.has_resolved_chat_metadata(cx) || self.is_community_outsider(cx) {
            return String::new();
        }
        let chat = self.resolved_chat(cx);
        match chat.r#type {
            ChatType::Group => self.resolved_chat_id(cx),
            ChatType::Channel if chat.can_manage_chat && !ChatPerm::is_owner_role(chat.role.as_deref()) => {
                self.resolved_chat_id(cx)
            }
            _ => String::new(),
        }
    }

    pub(in super::super) fn resolved_chat(&self, cx: &mut App) -> ChatInfo {
        let chat_id = self.resolved_chat_id(cx);
        let chats = ChatServicesStore::shared(cx);
        let chats = chats.read(cx);
        chats
            .chat_info(&chat_id, cx)
            .or_else(|| chats.opening_hint(&chat_id))
            .unwrap_or_else(|| ChatInfo::new(chat_id))
    }

    pub(in super::super) fn block_state_peer_user_id(&self, cx: &mut App) -> String {
        let chat = self.resolved_chat(cx);
        if chat.r#type != ChatType::Direct {
            return String::new();
        }
        trimming_whitespaces_and_newlines(&chat.participant.id).to_owned()
    }

    /// must stay snapshot only since render runs it on every keystroke
    /// the profile walking viewer_has_blocked decodes the whole cached dto
    /// refresh_block_state on screen entry puts the peer in
    pub(in super::super) fn viewer_has_blocked_peer(&self, chat: &ChatInfo, cx: &mut App) -> bool {
        if chat.r#type != ChatType::Direct {
            return false;
        }
        let peer_id = trimming_whitespaces_and_newlines(&chat.participant.id);
        if peer_id.is_empty() {
            return false;
        }
        UserServicesStore::shared(cx).read(cx).viewer_has_blocked_from_snapshot(peer_id, cx)
    }

    /// the peer blocked me totally so the server refuses my messages
    /// set lookup only like viewer_has_blocked_peer since render runs it on every keystroke
    pub(in super::super) fn peer_restricts_viewer(&self, chat: &ChatInfo, cx: &mut App) -> bool {
        if chat.r#type != ChatType::Direct {
            return false;
        }
        let users = UserServicesStore::shared(cx);
        let users = users.read(cx);
        chat.participant.is_deleted
            || users.peer_restricts_viewer(&chat.participant.id)
            || users.peer_refuses_viewer_messages(&chat.participant.id)
    }

    pub(in super::super) fn restricted_by_peer_text(&self, chat: &ChatInfo, cx: &mut App) -> String {
        if chat.participant.is_deleted {
            return tr("chat_deleted_account");
        }
        let restricts = UserServicesStore::shared(cx).read(cx).peer_restricts_viewer(&chat.participant.id);
        tr(if restricts { "contact_error_blocked_by_user" } else { "block_type_chat_desc" })
    }

    pub(in super::super) fn close_authoritatively_removed_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_context_preview(cx) {
            let presenter = HoldContextMenuPresenter::shared(cx);
            let controller = self
                .context_preview
                .as_ref()
                .and_then(|session| session.read(cx).controller())
                .map(|controller| controller.entity_id());
            let active = presenter
                .read(cx)
                .active_request()
                .and_then(|request| request.controller_preview.as_ref())
                .map(|preview| preview.borrow().controller.entity_id());
            if active == controller {
                HoldContextMenuPresenter::dismiss(cx);
            }
            return;
        }
        resign_chat_keyboard(window, cx);
        let Some(nav_store) = self.nav_store() else {
            return;
        };
        let (path, tab) = {
            let nav = nav_store.read(cx);
            (nav.path.clone(), nav.current_tab)
        };
        let Some(index) = path.iter().position(|route| self.opens_removed_chat(route, cx)) else {
            return;
        };
        let kept = path[..index].to_vec();
        nav_store.update(cx, |nav, cx| nav.restore(kept, tab, cx));
    }

    pub(in super::super) fn opens_removed_chat(&self, route: &RootRoute, cx: &mut App) -> bool {
        Self::opens_chat(route, &self.route_chat_ids(cx))
    }

    pub(in super::super) fn opens_chat(route: &RootRoute, ids: &[String]) -> bool {
        match route {
            RootRoute::Chat { chat_id } | RootRoute::ChatTopic { chat_id, .. } | RootRoute::ChatThread { chat_id, .. } => {
                chat_id.as_ref().is_some_and(|chat_id| ids.contains(chat_id))
            }
            _ => false,
        }
    }

    /// ids a route to this chat may carry, the one it was opened with and the resolved one
    pub(in super::super) fn route_chat_ids(&self, cx: &mut App) -> Vec<String> {
        let mut ids: Vec<String> = self.chat_id.iter().cloned().collect();
        ids.push(self.resolved_chat_id(cx));
        ids
    }

    pub(in super::super) fn is_covered_in_stack(&self, cx: &mut App) -> bool {
        let ids = self.route_chat_ids(cx);
        let Some(nav_store) = self.nav_store() else {
            return false;
        };
        let nav = nav_store.read(cx);
        let path = &nav.path;
        let Some((top, rest)) = path.split_last() else {
            return false;
        };
        !Self::opens_chat(top, &ids) && rest.iter().any(|route| Self::opens_chat(route, &ids))
    }

    pub(in super::super) fn is_hidden_under_sibling_topic(&self, cx: &mut App) -> bool {
        let ids = self.route_chat_ids(cx);
        let own = Self::topic_route_key(&self.first_frame_navigation_route());
        let Some(nav_store) = self.nav_store() else {
            return false;
        };
        let nav = nav_store.read(cx);
        let Some(top) = nav.path.last() else {
            return false;
        };
        Self::opens_chat(top, &ids) && Self::topic_route_key(top) != own
    }

    fn topic_route_key(route: &RootRoute) -> (Option<String>, Option<String>, bool) {
        match route {
            RootRoute::ChatTopic { topic_id, .. } => (topic_id.clone(), None, false),
            RootRoute::ChatThread { root_id, .. } => (None, root_id.clone(), false),
            RootRoute::ForumAll { .. } => (None, None, true),
            _ => (None, None, false),
        }
    }
}
