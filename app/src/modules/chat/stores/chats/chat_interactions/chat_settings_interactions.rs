use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_errors::{ErrorCode, ErrorParam};
use rise_i18n::tr;

use super::chat_interactions::ChatInteractionsStore;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatInfo, ChatPermKey, ChatReactionCatalog, ChatReactionsType, ChatSettingsModel,
};
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::MessageThreadOrder;

fn notify_info(text: String, cx: &mut App) {
    Notifier::send(
        text,
        NotifierSendOptions {
            r#type: NotifierType::Info,
            position: NotifierPosition::Bottom,
            ..NotifierSendOptions::default()
        },
        cx,
    );
}

fn guide_invents(chat_id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).invents(Some(chat_id)))
}

fn guide_owns_discussion(chat_id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_discussion(Some(chat_id)))
}

fn chat_managing_reactions(chat_id: &str, cx: &mut App) -> Option<ChatInfo> {
    let chat = ChatServicesStore::shared(cx).update(cx, |store, cx| store.chat_info(chat_id, cx))?;
    ChatProfilePermissions::manages_reactions(&chat, cx).then_some(chat)
}

impl ChatInteractionsStore {
    pub fn load_chat_settings(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() || guide_invents(chat_id, cx) {
            return;
        }

        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.get_chat_settings_action(chat_id, cx));
    }

    pub fn slow_mode_blocks_send(&mut self, chat_id: &str, cx: &mut Context<Self>) -> bool {
        let remaining = ChatServicesStore::shared(cx).update(cx, |store, cx| store.slow_mode_remaining_seconds(chat_id, cx));
        if remaining <= 0 {
            return false;
        }

        let details = HashMap::from([(ErrorParam::RetryAfterS.wire().to_owned(), remaining.to_string())]);
        let text = CanonErrors::text(Some(ErrorCode::ChatSlowModeActive.wire()), &details)
            .unwrap_or_else(|| tr("chat_perm_slowmode"));
        notify_info(text, cx);
        true
    }

    pub fn slow_mode_refused_send(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        ChatServicesStore::shared(cx).update(cx, |store, cx| store.restart_slow_mode_from_history(chat_id, cx));
        if !self.slow_mode_blocks_send(chat_id, cx) {
            notify_info(tr("chat_perm_slowmode"), cx);
        }
        self.load_chat_settings(chat_id, cx);
    }

    pub fn prime_slow_mode(&mut self, chat_id: &str, own_sent_at: TimeInterval, cx: &mut Context<Self>) {
        let sent_at = MessageThreadOrder::normalize(own_sent_at);
        if sent_at <= 0.0 {
            return;
        }
        let start = Date::from_time_interval_since_1970(sent_at);
        ChatServicesStore::shared(cx).update(cx, |store, cx| store.start_slow_mode(chat_id, start, cx));
    }

    pub fn slow_mode_countdown_ended(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        ChatServicesStore::shared(cx).update(cx, |store, cx| store.end_slow_mode(chat_id, cx));
    }

    pub fn send_switched_off(&mut self, chat_id: &str, key: &str, cx: &mut Context<Self>) -> bool {
        if !ChatServicesStore::shared(cx).update(cx, |store, cx| store.member_send_switched_off(chat_id, key, cx)) {
            return false;
        }

        self.member_send_refused(ErrorCode::ChatSendRestricted.wire(), cx);
        true
    }

    pub fn send_switched_off_forum_topic_id(
        &mut self,
        chat_id: &str,
        key: &str,
        forum_topic_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !ChatServicesStore::shared(cx)
            .update(cx, |store, cx| store.member_send_switched_off_forum_topic_id(chat_id, key, forum_topic_id, cx))
        {
            return false;
        }

        self.member_send_refused(ErrorCode::ChatSendRestricted.wire(), cx);
        true
    }

    pub fn picker_send_switched_off(
        &mut self,
        chat_id: &str,
        key: &str,
        forum_topic_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        if chat_id.is_empty() {
            return false;
        }
        ChatServicesStore::shared(cx)
            .update(cx, |store, cx| store.member_send_switched_off_forum_topic_id(chat_id, key, forum_topic_id, cx))
    }

    pub fn member_send_refused(&mut self, code: &str, cx: &mut Context<Self>) {
        let text = CanonErrors::text(Some(code), &HashMap::new()).unwrap_or_else(|| tr("chat_error_send_restricted"));
        notify_info(text, cx);
    }

    pub fn set_paid_reactions_allowed(&mut self, chat_id: &str, allowed: bool, cx: &mut Context<Self>) {
        if chat_managing_reactions(chat_id, cx).is_none() {
            return;
        }

        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.update_paid_reactions_action(chat_id, allowed, cx));
    }

    pub fn set_reactions_type(&mut self, chat_id: &str, r#type: ChatReactionsType, cx: &mut Context<Self>) {
        if chat_managing_reactions(chat_id, cx).is_none() {
            return;
        }

        let current = ChatServicesStore::shared(cx)
            .update(cx, |store, cx| store.chat_settings(chat_id, cx).map(|settings| settings.allowed_reactions.clone()))
            .unwrap_or_default();
        let allowed = if matches!(r#type, ChatReactionsType::Custom) && current.is_empty() {
            ChatReactionCatalog::quick_bar()
        } else {
            current
        };
        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.update_reactions_action(chat_id, r#type, &allowed, cx));
    }

    pub fn toggle_allowed_reaction(&mut self, chat_id: &str, emoji: &str, cx: &mut Context<Self>) {
        if chat_managing_reactions(chat_id, cx).is_none() {
            return;
        }

        let mut allowed = ChatServicesStore::shared(cx)
            .update(cx, |store, cx| store.chat_settings(chat_id, cx).map(|settings| settings.allowed_reactions.clone()))
            .unwrap_or_default();
        if let Some(index) = allowed.iter().position(|reaction| reaction == emoji) {
            allowed.remove(index);
        } else {
            let catalogue = ChatReactionCatalog::all_emojis();
            let rank_of = |reaction: &str| catalogue.iter().position(|entry| entry == reaction);
            let slot = rank_of(emoji).and_then(|rank| {
                allowed
                    .iter()
                    .position(|reaction| rank_of(reaction.as_str()).unwrap_or(usize::MAX) > rank)
            });
            allowed.insert(slot.unwrap_or(allowed.len()), emoji.to_owned());
        }
        let r#type = if allowed.is_empty() {
            ChatReactionsType::None
        } else {
            ChatReactionsType::Custom
        };
        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.update_reactions_action(chat_id, r#type, &allowed, cx));
    }

    pub fn set_message_perm(&mut self, chat_id: &str, key: &str, value: bool, cx: &mut Context<Self>) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.update_permissions_action(
                chat_id,
                |model: &mut ChatSettingsModel| {
                    model.message_allow.insert(key.to_owned(), value);
                },
                cx,
            );
        });
    }

    pub fn set_media_perm(&mut self, chat_id: &str, key: &str, value: bool, cx: &mut Context<Self>) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.update_permissions_action(
                chat_id,
                |model: &mut ChatSettingsModel| {
                    model.media_allow.insert(key.to_owned(), value);
                },
                cx,
            );
        });
    }

    pub fn set_all_media(&mut self, chat_id: &str, value: bool, cx: &mut Context<Self>) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.update_permissions_action(
                chat_id,
                |model: &mut ChatSettingsModel| {
                    for key in ChatPermKey::MEDIA {
                        model.media_allow.insert(key.to_owned(), value);
                    }
                },
                cx,
            );
        });
    }

    pub fn set_add_members(&mut self, chat_id: &str, value: bool, cx: &mut Context<Self>) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.update_permissions_action(
                chat_id,
                |model: &mut ChatSettingsModel| model.members_can_add_members = value,
                cx,
            );
        });
    }

    pub fn set_change_info(&mut self, chat_id: &str, value: bool, cx: &mut Context<Self>) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.update_permissions_action(
                chat_id,
                |model: &mut ChatSettingsModel| model.members_can_change_pfp = value,
                cx,
            );
        });
    }

    pub fn set_slow_mode(&mut self, chat_id: &str, seconds: i64, cx: &mut Context<Self>) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.update_permissions_action(chat_id, |model: &mut ChatSettingsModel| model.slow_mode = seconds, cx);
        });
    }

    pub fn load_linked_chat(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() || guide_invents(chat_id, cx) {
            return;
        }

        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.get_linked_chat_action(chat_id, cx));
    }

    pub fn link_discussion_group(
        &mut self,
        chat_id: &str,
        group_id: &str,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if guide_owns_discussion(chat_id, cx) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.link_discussion(group_id, cx));
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(true, cx));
            }
            return;
        }
        let reporting: Rc<dyn Fn(bool, &mut App)> = Rc::new(move |ok: bool, cx: &mut App| {
            if ok {
                GuideSignalBus::emit(RiseGuideSignal::ChatDiscussionLinked);
            }
            if let Some(on_result) = &on_result {
                on_result(ok, cx);
            }
        });
        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.link_chat_action(chat_id, group_id, Some(reporting), cx));
    }

    pub fn unlink_discussion(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if guide_owns_discussion(chat_id, cx) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.unlink_discussion(cx));
            return;
        }
        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.unlink_chat_action(chat_id, cx));
    }

    pub fn load_analytics(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }

        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.get_chat_analytics_action(chat_id, cx));
    }
}
