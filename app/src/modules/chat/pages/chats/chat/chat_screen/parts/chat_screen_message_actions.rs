use std::collections::HashMap;
use gpui::prelude::*;
use gpui::{App, AppContext, Context, Window};
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::components::chat::composer::chat_composer_input_surface::ChatComposerInputSurface;
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::shared::message_permissions::{ChatMessageAction, MessagePermissions};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::message::message_actions::message_types::MessageDeliveryStatus;
use crate::modules::chat::widgets::bottomsheets::chat_moderation_sheet::chat_moderation_sheet::{
    ChatModerationSheetStore, ChatModerationTarget,
};
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::pasteboard::UIPasteboard;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::media::media_save_coordinator::MediaSaveCoordinator;
use crate::core::widgets::notifier::notifier_store::NotifierIcon;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsDone;
use crate::modules::chat::widgets::bottomsheets::chat_moderation_sheet::chat_moderation_sheet::ChatModerationSelection;
use crate::modules::moderation::stores::moderation_actions::moderation_actions::ModerationActionsStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::music::widgets::bottomsheets::playlists::message_mp3_import_sheet::MessageMusicImportPresenter;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::SparksTarget;
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;
use crate::modules::chat::stores::message::message_actions::message_types::MessageMediaItem;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;

impl ChatScreen {
    pub(in super::super) fn delete_current_chat(&mut self, for_everyone: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.state.show_chat_delete_dialog = false;
        self.cancel_message_selection(window, cx);
        let chat_id = self.resolved_chat_id(cx);
        let nav_store = self.nav_store();
        let on_error: ChatInteractionsDone = Box::new(|cx: &mut App| {
            Notifier::send(
                tr("chat_delete_failed_title"),
                NotifierSendOptions {
                    content: Some(tr("chat_delete_failed_text")),
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Top,
                    icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
        let completion: ChatInteractionsDone = Box::new(move |cx: &mut App| {
            if let Some(nav_store) = nav_store {
                nav_store.update(cx, |store, cx| store.pop(cx));
            }
        });
        chat_interactions(cx, |store, cx| {
            store.delete_chat(&chat_id, for_everyone, false, None, Some(on_error), Some(completion), cx);
        });
        cx.notify();
    }

    pub(in super::super) fn clear_favourites_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.show_chat_delete_dialog = false;
        self.cancel_message_selection(window, cx);
        cx.notify();
        if self.resolved_chat(cx).r#type != ChatType::Favourites {
            return;
        }
        let Some(user_id) = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().filter(|id| !id.is_empty()) else {
            return;
        };
        let chat_id = self.resolved_chat_id(cx);
        message_interactions(cx, |store, cx| store.delete_all_messages_from_user(&chat_id, &user_id, cx));
    }

    pub(in super::super) fn handle_message_reaction(&mut self, message_id: &str, emoji: &str, _window: &mut Window, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if chat_id.is_empty() || self.is_community_outsider(cx) {
            return;
        }
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        message_interactions(cx, |store, cx| {
            store.toggle_reaction(&chat_id, message_id, emoji, topic_id.as_deref(), thread_root_id.as_deref(), None, cx);
        });
    }

    pub(in super::super) fn handle_message_action(&mut self, message_id: &str, action_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        let scope = self.message_scope(cx);
        if let Some(target_id) = ChatMessageAction::jump_to_message_id(action_id) {
            if chat_id.is_empty() {
                return;
            }
            let mut jump_id = target_id;
            if GetSaiServerId::is_temp_id(Some(jump_id.as_str())) {
                let server_id = message_services(cx, |store, cx| store.server_message_id(&jump_id, &scope, cx));
                if !server_id.is_empty() && !GetSaiServerId::is_temp_id(Some(server_id.as_str())) {
                    jump_id = server_id;
                }
            }
            self.jump_to_message_id(&jump_id, window, cx);
            return;
        }
        if let Some(target_id) = ChatMessageAction::reply_to_message_id(action_id) {
            self.handle_message_action(&target_id, ChatMessageAction::REPLY, window, cx);
            return;
        }
        if chat_id.is_empty() {
            return;
        }
        let Some(message) = message_services(cx, |store, cx| store.message(message_id, &scope, cx)) else {
            return;
        };
        if let Some(query) = ChatMessageAction::bot_inline_query(action_id) {
            self.insert_bot_inline_query(&query, &message, cx);
            return;
        }
        if action_id == ChatMessageAction::MENTION_VIA_BOT {
            self.insert_via_bot_mention(&message, cx);
            return;
        }
        if let Some(user_id) = ChatMessageAction::read_receipt_user_id(action_id) {
            let server_id = message.sai_server_id();
            let receipts = message_services(cx, |store, cx| store.message_read_receipts(&chat_id, &server_id, cx));
            let tag = receipts
                .readers
                .iter()
                .find(|reader| reader.id == user_id)
                .map(|reader| reader.tag.clone())
                .filter(|tag| !tag.is_empty());
            self.push_route(RootRoute::UserPage { user_id: Some(user_id), tag }, cx);
            return;
        }
        let chat = self.resolved_chat(cx);
        let is_own = MessageList::is_own_bubble(&message, &chat, cx);
        let perms = MessagePermissions::new(&message, &chat, is_own);
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        let ids = [message_id.to_owned()];
        let pin = |for_everyone: Option<bool>, silent: Option<bool>, cx: &mut App| {
            message_interactions(cx, |store, cx| {
                store.pin_message(message_id, &chat_id, topic_id.as_deref(), thread_root_id.as_deref(), for_everyone, silent, cx);
            });
        };

        match action_id {
            ChatMessageAction::REPLY => {
                self.begin_reply(&message, MessagePermissions::new(&message, &chat, is_own), window, cx);
                if !(self.picker_visible || self.composer_input_surface == ChatComposerInputSurface::Picker) {
                    self.composer_focus_token = self.composer_focus_token.wrapping_add(1);
                }
            }
            ChatMessageAction::SEND_SPARKS => {
                let server_id = message.sai_server_id();
                let target = SparksTarget::Message {
                    chat_id: chat.id.clone(),
                    message_id: if server_id.is_empty() { message.id.clone() } else { server_id },
                };
                SparksInteractionsStore::shared(cx).update(cx, |store, cx| store.open_send_sheet(target, false, None, cx));
            }
            ChatMessageAction::SHOW_IN_CHAT => self.jump_to_pinned_message(&message, window, cx),
            ChatMessageAction::COPY => {
                UIPasteboard::general().set_string(perms.copy_text(), cx);
                let _ = notification(NotificationKind::Success);
            }
            ChatMessageAction::COPY_LINK => {
                let Some(link) = perms.message_link(cx) else {
                    return;
                };
                UIPasteboard::general().set_string(&link.absolute_string(), cx);
                Notifier::copied(cx);
            }
            ChatMessageAction::FORWARD => {
                if perms.can_forward(cx) {
                    let messages = vec![message.clone()];
                    ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_messages(messages, cx));
                }
            }
            ChatMessageAction::ADD_TO_LOCAL_PLAYLIST => {
                if perms.can_add_mp3_to_local_playlist() {
                    MessageMusicImportPresenter::shared(cx).update(cx, |presenter, cx| presenter.present(&message, cx));
                }
            }
            ChatMessageAction::SELECT => self.begin_message_selection(&message, window, cx),
            ChatMessageAction::PIN_EVERYONE | ChatMessageAction::PIN_EVERYONE_NOTIFY => pin(Some(true), Some(false), cx),
            ChatMessageAction::PIN_EVERYONE_SILENT => pin(Some(true), Some(true), cx),
            ChatMessageAction::PIN_ME => pin(Some(false), None, cx),
            ChatMessageAction::PIN => pin(None, None, cx),
            ChatMessageAction::UNPIN => {
                message_interactions(cx, |store, cx| {
                    store.unpin_message(message_id, &chat_id, topic_id.as_deref(), thread_root_id.as_deref(), cx);
                });
            }
            ChatMessageAction::EDIT => self.begin_edit(&message, cx),
            ChatMessageAction::CANCEL_SENDING => {
                if perms.can_cancel_sending() {
                    message_interactions(cx, |store, cx| store.cancel_outgoing_message(&message, &chat, cx));
                }
            }
            ChatMessageAction::RETRY_SENDING => {
                if perms.can_retry_sending(cx) {
                    message_interactions(cx, |store, cx| store.retry_outgoing_message(&message, &chat, cx));
                }
            }
            ChatMessageAction::FAILED_SEND_OPTIONS => {
                if perms.can_cancel_sending() && message.delivery_status == MessageDeliveryStatus::Failed {
                    self.state.failed_send_target = Some(ChatFailedSendTarget {
                        message_id: message_id.to_owned(),
                        can_retry: perms.can_retry_sending(cx),
                        reason: CanonErrors::text(message.send_failure_code.as_deref(), &HashMap::new()),
                    });
                    cx.notify();
                }
            }
            ChatMessageAction::DELETE | ChatMessageAction::DELETE_EVERYONE | ChatMessageAction::DELETE_FROM_FAVOURITES => {
                self.delete_message_ids(&ids, true, cx);
            }
            ChatMessageAction::DELETE_ME => self.delete_message_ids(&ids, false, cx),
            ChatMessageAction::DELETE_MODERATE => {
                let target = ChatModerationTarget {
                    chat_id: chat_id.clone(),
                    message_id: message.sai_server_id(),
                    sender_id: message.sender_id.clone(),
                    sender_name: message.sender_name.clone(),
                    is_channel: chat.r#type == ChatType::Channel,
                };
                HoldContextMenuPresenter::close_without_motion(cx);
                let screen = cx.weak_entity();
                ChatModerationSheetStore::shared(cx).update(cx, |store, cx| {
                    store.present(
                        target,
                        move |target, selection, cx| {
                            let _ = screen.update(cx, |screen, cx| screen.handle_moderation_confirm(target, selection, cx));
                        },
                        cx,
                    );
                });
            }
            ChatMessageAction::REPORT => {
                let target = ReportTarget::Message { chat_id: chat_id.clone(), message_id: message.sai_server_id() };
                ReportSheetStore::shared(cx).update(cx, |store, cx| store.present(target, None, None, cx));
            }
            ChatMessageAction::SAVE_TO_GALLERY => {
                let items: Vec<MessageMediaItem> = perms.gallery_savable_items().into_iter().cloned().collect();
                let message = message.clone();
                MediaSaveCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.save_to_gallery(message, items, cx));
            }
            ChatMessageAction::SAVE => {
                let items: Vec<MessageMediaItem> = perms.file_savable_items().into_iter().cloned().collect();
                let message = message.clone();
                MediaSaveCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.save_to_files(message, items, cx));
            }
            _ => {}
        }
    }

    pub(in super::super) fn handle_moderation_confirm(&mut self, target: ChatModerationTarget, selection: ChatModerationSelection, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if chat_id.is_empty() {
            return;
        }
        if selection.delete_all_from_user {
            message_interactions(cx, |store, cx| store.delete_all_messages_from_user(&chat_id, &target.sender_id, cx));
        } else {
            self.delete_message_ids(std::slice::from_ref(&target.message_id), true, cx);
        }
        if selection.report_spam {
            ModerationActionsStore::shared(cx).update(cx, |store, cx| {
                store.report_chat_spam_action(&chat_id, Some(target.message_id.as_str()), cx);
            });
        }
        if selection.ban {
            ChatActionsStore::shared(cx).update(cx, |store, cx| store.ban_member_action(&chat_id, &target.sender_id, None, cx));
        }
        if selection.restrict {
            chat_interactions(cx, |store, cx| store.restrict_member(&chat_id, &target.sender_id, &target.sender_name, cx));
        }
    }
}
