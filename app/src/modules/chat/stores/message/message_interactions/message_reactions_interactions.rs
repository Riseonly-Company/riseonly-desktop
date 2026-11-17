use gpui::{App, Context};
use rise_canon::RiseGuideSignal;
use rise_i18n::tr;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::stores::chat_guide_sandbox_hooks::chat_guide_sandbox_hooks;
use crate::modules::chat::stores::chat_stores_hooks::chat_stores_components_hooks;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_actions::MessageActionHandler;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageReaction};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

fn guide_sandbox_owns_chat(chat_id: &str, cx: &App) -> bool {
    (chat_guide_sandbox_hooks().owns_chat_id)(chat_id, cx)
}

fn reaction_park_key(scope: &str, message_id: &str, reaction: &str) -> String {
    format!("{scope}|{message_id}|{reaction}")
}

fn durable_reaction_key(scope: &str, message_id: &str) -> String {
    format!("{scope}|{message_id}")
}

pub struct MessageReactionSendLane {
    pub sent: bool,
    pub wanted: bool,
}

#[derive(Clone, Copy)]
enum ReactionSendOutcome {
    Delivered,
    Refused,
    ConnectionLost,
}

#[derive(Clone)]
struct ReactionTarget {
    lane_key: String,
    chat_id: String,
    message_id: String,
    reaction: String,
    topic_id: Option<String>,
    thread_root_id: Option<String>,
    retry_while_connected: bool,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
}

impl ReactionTarget {
    fn settle_handler(&self, outcome: ReactionSendOutcome) -> MessageActionHandler {
        let target = self.clone();
        Box::new(move |cx: &mut App| {
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.settle_reaction_send(&target, outcome, cx);
            });
        })
    }
}

impl MessageInteractionsStore {
    pub fn toggle_custom_emoji_reaction(
        &mut self,
        chat_id: &str,
        message_id: &str,
        custom_emoji_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if custom_emoji_id.is_empty() {
            return;
        }
        let reaction = format!("{}{custom_emoji_id}", MessageReaction::CUSTOM_EMOJI_PREFIX);
        self.toggle_reaction(chat_id, message_id, &reaction, topic_id, thread_root_id, None, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn toggle_reaction(
        &mut self,
        chat_id: &str,
        message_id: &str,
        reaction: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        fallback_message: Option<&GetMessageMessage>,
        cx: &mut Context<Self>,
    ) {
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let message = Self::service(cx)
            .update(cx, |service, cx| service.messages(&scope, cx))
            .into_iter()
            .find(|message| message.matches_message_id(message_id))
            .or_else(|| {
                fallback_message
                    .filter(|message| message.chat_id == chat_id && message.matches_message_id(message_id))
                    .cloned()
            });
        let already_reacted = message
            .as_ref()
            .and_then(|message| message.reactions.iter().find(|candidate| candidate.emoji == reaction))
            .is_some_and(|candidate| candidate.reacted_by_me);

        if !already_reacted {
            let chat = ChatServicesStore::shared(cx).read(cx).chat_info(chat_id, cx);
            let allowed = match (&message, &chat) {
                (Some(message), Some(chat)) => {
                    chat_stores_components_hooks().is_some_and(|hooks| (hooks.can_add)(reaction, message, chat, cx))
                }
                _ => false,
            };
            if !allowed {
                let options = NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                };
                Notifier::send(tr("chat_error_reaction_restricted"), options, cx);
                return;
            }
        }

        let adding = !already_reacted;
        if guide_sandbox_owns_chat(chat_id, cx) {
            self.send_reaction(adding, chat_id, message_id, reaction, topic_id, thread_root_id, None, None, None, cx);
            if adding {
                GuideSignalBus::emit(RiseGuideSignal::ReactionAdded);
            }
            return;
        }

        self.apply_reaction_locally(adding, &scope, message_id, reaction, cx);
        if adding {
            GuideSignalBus::emit(RiseGuideSignal::ReactionAdded);
        }

        let key = reaction_park_key(&scope, message_id, reaction);
        if let Some(parked) = self.parked_reaction_sends.remove(&key) {
            parked.stop();
            cx.notify();
        }
        self.deliver_reaction(adding, chat_id, message_id, reaction, topic_id, thread_root_id, true, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn deliver_reaction(
        &mut self,
        adding: bool,
        chat_id: &str,
        message_id: &str,
        reaction: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        retry_while_connected: bool,
        cx: &mut Context<Self>,
    ) {
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let lane_key = reaction_park_key(&scope, message_id, reaction);
        if let Some(lane) = self.reaction_send_lanes.get_mut(&lane_key) {
            lane.wanted = adding;
            return;
        }
        if !RiseTransportStatus::is_connected(cx)
            || self.durable_reaction_keys.contains(&durable_reaction_key(&scope, message_id))
        {
            self.park_reaction(adding, chat_id, message_id, reaction, topic_id, thread_root_id, cx);
            return;
        }
        self.reaction_send_lanes.insert(
            lane_key.clone(),
            MessageReactionSendLane {
                sent: adding,
                wanted: adding,
            },
        );
        let target = ReactionTarget {
            lane_key,
            chat_id: chat_id.to_owned(),
            message_id: message_id.to_owned(),
            reaction: reaction.to_owned(),
            topic_id: topic_id.map(str::to_owned),
            thread_root_id: thread_root_id.map(str::to_owned),
            retry_while_connected,
            account_user_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
        };
        let handlers = (
            target.settle_handler(ReactionSendOutcome::Refused),
            target.settle_handler(ReactionSendOutcome::ConnectionLost),
            target.settle_handler(ReactionSendOutcome::Delivered),
        );
        self.send_reaction(
            adding,
            chat_id,
            message_id,
            reaction,
            topic_id,
            thread_root_id,
            Some(handlers.0),
            Some(handlers.1),
            Some(handlers.2),
            cx,
        );
    }

    fn settle_reaction_send(&mut self, target: &ReactionTarget, outcome: ReactionSendOutcome, cx: &mut Context<Self>) {
        let Some(lane) = self.reaction_send_lanes.remove(&target.lane_key) else {
            return;
        };
        let topic_id = target.topic_id.as_deref();
        let thread_root_id = target.thread_root_id.as_deref();
        if !SaiAccountCacheScope::is_current_account_epoch(
            target.account_user_id.as_deref(),
            target.account_epoch.as_deref(),
        ) {
            return;
        }
        if lane.wanted != lane.sent {
            self.deliver_reaction(
                lane.wanted,
                &target.chat_id,
                &target.message_id,
                &target.reaction,
                topic_id,
                thread_root_id,
                true,
                cx,
            );
            return;
        }
        match outcome {
            ReactionSendOutcome::Delivered => {
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.preview_reaction_action(
                        &target.chat_id,
                        &target.message_id,
                        &target.reaction,
                        lane.sent,
                        topic_id,
                        thread_root_id,
                        cx,
                    );
                });
            }
            ReactionSendOutcome::Refused => {
                let scope = MessageServicesStore::message_scope(&target.chat_id, topic_id, thread_root_id);
                self.apply_reaction_locally(!lane.sent, &scope, &target.message_id, &target.reaction, cx);
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.preview_reaction_action(
                        &target.chat_id,
                        &target.message_id,
                        &target.reaction,
                        !lane.sent,
                        topic_id,
                        thread_root_id,
                        cx,
                    );
                });
            }
            ReactionSendOutcome::ConnectionLost => {
                if target.retry_while_connected && RiseTransportStatus::is_connected(cx) {
                    self.deliver_reaction(
                        lane.sent,
                        &target.chat_id,
                        &target.message_id,
                        &target.reaction,
                        topic_id,
                        thread_root_id,
                        false,
                        cx,
                    );
                } else {
                    self.park_reaction(
                        lane.sent,
                        &target.chat_id,
                        &target.message_id,
                        &target.reaction,
                        topic_id,
                        thread_root_id,
                        cx,
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn park_reaction(
        &mut self,
        adding: bool,
        chat_id: &str,
        message_id: &str,
        reaction: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let key = reaction_park_key(&scope, message_id, reaction);
        if let Some(parked) = self.parked_reaction_sends.remove(&key) {
            parked.stop();
        }
        let durable_key = durable_reaction_key(&scope, message_id);
        self.durable_reaction_keys.insert(durable_key.clone());
        cx.notify();
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let on_unqueued: MessageActionHandler = {
            let chat_id = chat_id.to_owned();
            let message_id = message_id.to_owned();
            let reaction = reaction.to_owned();
            let topic_id = topic_id.map(str::to_owned);
            let thread_root_id = thread_root_id.map(str::to_owned);
            Box::new(move |cx: &mut App| {
                if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
                {
                    return;
                }
                Self::shared(cx).update(cx, |store, cx| {
                    store.durable_reaction_keys.remove(&durable_key);
                    cx.notify();
                    store.park_reaction_in_memory(
                        adding,
                        &chat_id,
                        &message_id,
                        &reaction,
                        topic_id.as_deref(),
                        thread_root_id.as_deref(),
                        cx,
                    );
                });
            })
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.queue_reaction_action(
                chat_id,
                message_id,
                reaction,
                adding,
                topic_id,
                thread_root_id,
                on_unqueued,
                cx,
            );
        });
        RiseTransportStatus::wake_if_offline(cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn park_reaction_in_memory(
        &mut self,
        adding: bool,
        chat_id: &str,
        message_id: &str,
        reaction: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let key = reaction_park_key(&scope, message_id, reaction);
        if let Some(parked) = self.parked_reaction_sends.remove(&key) {
            parked.stop();
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.preview_reaction_action(chat_id, message_id, reaction, adding, topic_id, thread_root_id, cx);
        });
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let watch = {
            let key = key.clone();
            let chat_id = chat_id.to_owned();
            let message_id = message_id.to_owned();
            let reaction = reaction.to_owned();
            let topic_id = topic_id.map(str::to_owned);
            let thread_root_id = thread_root_id.map(str::to_owned);
            RiseTransportStatus::on_next_connect(
                move |cx: &mut App| {
                    Self::shared(cx).update(cx, |store, cx| {
                        if store.parked_reaction_sends.remove(&key).is_none() {
                            return;
                        }
                        cx.notify();
                        if !SaiAccountCacheScope::is_current_account_epoch(
                            account_user_id.as_deref(),
                            account_epoch.as_deref(),
                        ) {
                            return;
                        }
                        store.deliver_reaction(
                            adding,
                            &chat_id,
                            &message_id,
                            &reaction,
                            topic_id.as_deref(),
                            thread_root_id.as_deref(),
                            false,
                            cx,
                        );
                    });
                },
                cx,
            )
        };
        self.parked_reaction_sends.insert(key, watch);
        cx.notify();
        RiseTransportStatus::wake_if_offline(cx);
    }

    fn apply_reaction_locally(&self, adding: bool, scope: &str, message_id: &str, reaction: &str, cx: &mut App) {
        Self::service(cx).update(cx, |service, cx| {
            if adding {
                service.add_reaction_optimistic(scope, message_id, reaction, cx);
            } else {
                service.remove_reaction_optimistic(scope, message_id, reaction, cx);
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn send_reaction(
        &self,
        adding: bool,
        chat_id: &str,
        message_id: &str,
        reaction: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        on_failed: Option<MessageActionHandler>,
        on_connection_lost: Option<MessageActionHandler>,
        on_succeeded: Option<MessageActionHandler>,
        cx: &mut App,
    ) {
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let rollback: MessageActionHandler = match on_failed {
            Some(on_failed) => on_failed,
            None => {
            let chat_id = chat_id.to_owned();
            let message_id = message_id.to_owned();
            let reaction = reaction.to_owned();
            let topic_id = topic_id.map(str::to_owned);
            let thread_root_id = thread_root_id.map(str::to_owned);
            Box::new(move |cx: &mut App| {
                if guide_sandbox_owns_chat(&chat_id, cx) {
                    return;
                }
                Self::shared(cx).update(cx, |store, cx| {
                    store.apply_reaction_locally(!adding, &scope, &message_id, &reaction, cx);
                });
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.preview_reaction_action(
                        &chat_id,
                        &message_id,
                        &reaction,
                        !adding,
                        topic_id.as_deref(),
                        thread_root_id.as_deref(),
                        cx,
                    );
                });
            })
            }
        };
        Self::actions(cx).update(cx, |actions, cx| {
            if adding {
                actions.add_reaction_action(
                    chat_id,
                    message_id,
                    reaction,
                    topic_id,
                    thread_root_id,
                    Some(rollback),
                    on_connection_lost,
                    on_succeeded,
                    cx,
                );
            } else {
                actions.remove_reaction_action(
                    chat_id,
                    message_id,
                    reaction,
                    topic_id,
                    thread_root_id,
                    Some(rollback),
                    on_connection_lost,
                    on_succeeded,
                    cx,
                );
            }
        });
    }
}
