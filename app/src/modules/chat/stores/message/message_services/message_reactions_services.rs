use std::collections::HashMap;

use gpui::Context;
use rise_canon::rise_key;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::{Map, Value};

use crate::core::lib::sai::modules::sai_updater::sai_updater::SaiUpdaterWriteScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_payload_casing::ws_string;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageReaction, ReactionReactor, RemoteMediaURL,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessagePayloadCodec;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

const MAX_OWN_REACTIONS_PER_MESSAGE: usize = 3;
const MILLISECOND_TIMESTAMP_THRESHOLD: f64 = 1_000_000_000_000.0;

fn current_user_id(cx: &mut Context<MessageServicesStore>) -> String {
    AuthServicesStore::shared(cx)
        .read(cx)
        .profile()
        .map(|me| me.id.clone())
        .unwrap_or_default()
}

impl MessageServicesStore {
    fn rebuild_reactions_from_event(
        &mut self,
        payload: &Map<String, Value>,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<Vec<MessageReaction>> {
        let raw_entries = payload.get(rise_key::bot::REACTED_BY);
        let entries = MessagePayloadCodec::array_of_dictionaries(raw_entries);

        let my_id = current_user_id(cx);
        let mut known_reactors: HashMap<String, ReactionReactor> = HashMap::new();
        let mut reactors_by_emoji: HashMap<String, Vec<ReactionReactor>> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for entry in &entries {
            let Some(emoji) = entry
                .get(rise_key::bot::REACTION)
                .and_then(Value::as_str)
                .filter(|emoji| !emoji.is_empty())
            else {
                continue;
            };
            let sender = entry.get(rise_key::bot::SENDER).and_then(Value::as_object);
            let more = sender
                .and_then(|sender| sender.get(rise_key::bot::MORE))
                .and_then(Value::as_object);
            let logo = more
                .and_then(|more| more.get(rise_key::bot::LOGO))
                .and_then(Value::as_str)
                .or_else(|| {
                    sender
                        .and_then(|sender| sender.get(rise_key::bot::LOGO))
                        .and_then(Value::as_str)
                });
            let raw_ts = entry
                .get(rise_key::bot::CREATED_AT)
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let created_sec = if raw_ts > MILLISECOND_TIMESTAMP_THRESHOLD {
                raw_ts / 1000.0
            } else {
                raw_ts
            };
            let sender_id = sender
                .and_then(|sender| sender.get(rise_key::bot::ID))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let sender_name = sender
                .and_then(|sender| sender.get(rise_key::bot::NAME))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut reactor = ReactionReactor {
                user_id: trimming_whitespaces_and_newlines(sender_id).to_owned(),
                name: sender_name.to_owned(),
                avatar_url: RemoteMediaURL::optional_normalized(logo),
                created_at: created_sec,
            };
            if reactor.avatar_url.is_none() && !reactor.user_id.is_empty() {
                let known = match known_reactors.get(&reactor.user_id) {
                    Some(known) => known.clone(),
                    None => self.resolve_reactor(&reactor.user_id, chat_id, cx),
                };
                known_reactors.insert(reactor.user_id.clone(), known.clone());
                reactor.avatar_url = known.avatar_url;
                if reactor.name.is_empty() {
                    reactor.name = known.name;
                }
            }
            if !reactors_by_emoji.contains_key(emoji) {
                order.push(emoji.to_owned());
            }
            reactors_by_emoji.entry(emoji.to_owned()).or_default().push(reactor);
        }

        let raw_summaries = payload.get(rise_key::bot::REACTIONS);
        let summaries = MessagePayloadCodec::array_of_dictionaries(raw_summaries);
        if raw_summaries.is_some() {
            return Some(
                summaries
                    .iter()
                    .filter_map(|summary| {
                        let emoji = summary
                            .get(rise_key::bot::REACTION)
                            .and_then(Value::as_str)
                            .filter(|emoji| !emoji.is_empty())?;
                        let count = summary
                            .get(rise_key::bot::COUNT)
                            .and_then(Value::as_f64)
                            .map_or(0, |count| count as i64);
                        if count <= 0 {
                            return None;
                        }
                        let reactors = reactors_by_emoji.get(emoji).cloned().unwrap_or_default();
                        let reacted_by_you = summary
                            .get(rise_key::bot::REACTED_BY_YOU)
                            .and_then(Value::as_bool)
                            == Some(true);
                        Some(MessageReaction {
                            emoji: emoji.to_owned(),
                            count,
                            reacted_by_me: reacted_by_you
                                || (!my_id.is_empty() && reactors.iter().any(|reactor| reactor.user_id == my_id)),
                            reactors,
                        })
                    })
                    .collect(),
            );
        }

        raw_entries?;
        Some(
            order
                .into_iter()
                .map(|emoji| {
                    let mut reactors = reactors_by_emoji.remove(&emoji).unwrap_or_default();
                    reactors.sort_by(|lhs, rhs| rhs.created_at.total_cmp(&lhs.created_at));
                    MessageReaction {
                        count: reactors.len() as i64,
                        reacted_by_me: !my_id.is_empty() && reactors.iter().any(|reactor| reactor.user_id == my_id),
                        emoji,
                        reactors,
                    }
                })
                .collect(),
        )
    }

    pub fn reaction_added_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(payload) = MessagePayloadCodec::dictionary(any) else {
            return;
        };
        let Some(chat_id) = ws_string(&payload, &[rise_key::message::CHAT_ID]) else {
            return;
        };
        let Some(message_id) = ws_string(&payload, &[rise_key::message::MESSAGE_ID]) else {
            return;
        };

        if let Some(rebuilt) = self.rebuild_reactions_from_event(&payload, &chat_id, cx) {
            let apply = move |_: &[MessageReaction]| rebuilt.clone();
            self.mutate_reaction_in_realtime_scopes(&payload, &chat_id, &message_id, &apply, cx);
            return;
        }

        let Some(reaction) = payload.get(rise_key::bot::REACTION).and_then(Value::as_str) else {
            return;
        };

        let my_id = current_user_id(cx);
        let reactor_id =
            ws_string(&payload, &[rise_key::message::USER_ID, rise_key::message::SENDER_ID]).unwrap_or_default();
        let reactor = self.resolve_reactor(&reactor_id, &chat_id, cx);

        let apply_add = |reactions: &[MessageReaction]| {
            let mut arr = reactions.to_vec();
            if let Some(idx) = arr.iter().position(|item| item.emoji == reaction) {
                let echoes_own_optimistic = !my_id.is_empty() && reactor_id == my_id && arr[idx].reacted_by_me;
                if !echoes_own_optimistic {
                    arr[idx].count += 1;
                }
                if reactor_id == my_id {
                    arr[idx].reacted_by_me = true;
                }
                if !reactor_id.is_empty() {
                    arr[idx].reactors.retain(|item| item.user_id != reactor_id);
                    arr[idx].reactors.insert(0, reactor.clone());
                }
            } else {
                arr.push(MessageReaction {
                    emoji: reaction.to_owned(),
                    count: 1,
                    reacted_by_me: reactor_id == my_id,
                    reactors: if reactor_id.is_empty() { Vec::new() } else { vec![reactor.clone()] },
                });
            }
            arr
        };

        self.mutate_reaction_in_realtime_scopes(&payload, &chat_id, &message_id, &apply_add, cx);
    }

    pub fn reaction_removed_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(payload) = MessagePayloadCodec::dictionary(any) else {
            return;
        };
        let Some(chat_id) = ws_string(&payload, &[rise_key::message::CHAT_ID]) else {
            return;
        };
        let Some(message_id) = ws_string(&payload, &[rise_key::message::MESSAGE_ID]) else {
            return;
        };

        if let Some(rebuilt) = self.rebuild_reactions_from_event(&payload, &chat_id, cx) {
            let apply = move |_: &[MessageReaction]| rebuilt.clone();
            self.mutate_reaction_in_realtime_scopes(&payload, &chat_id, &message_id, &apply, cx);
            return;
        }

        let Some(reaction) = payload.get(rise_key::bot::REACTION).and_then(Value::as_str) else {
            return;
        };

        let my_id = current_user_id(cx);
        let reactor_id =
            ws_string(&payload, &[rise_key::message::USER_ID, rise_key::message::SENDER_ID]).unwrap_or_default();

        let apply_remove = |reactions: &[MessageReaction]| {
            let mut arr = reactions.to_vec();
            let Some(idx) = arr.iter().position(|item| item.emoji == reaction) else {
                return arr;
            };
            let echoes_own_optimistic = !my_id.is_empty() && reactor_id == my_id && !arr[idx].reacted_by_me;
            if !echoes_own_optimistic {
                arr[idx].count = (arr[idx].count - 1).max(0);
            }
            if reactor_id == my_id {
                arr[idx].reacted_by_me = false;
            }
            if !reactor_id.is_empty() {
                arr[idx].reactors.retain(|item| item.user_id != reactor_id);
            }
            if arr[idx].count == 0 {
                arr.remove(idx);
            }
            arr
        };

        self.mutate_reaction_in_realtime_scopes(&payload, &chat_id, &message_id, &apply_remove, cx);
    }

    pub fn add_reaction_optimistic(
        &mut self,
        chat_id: &str,
        message_id: &str,
        reaction: &str,
        cx: &mut Context<Self>,
    ) {
        let my_id = current_user_id(cx);
        let me = self.resolve_reactor(&my_id, chat_id, cx);

        let transform = |reactions: &[MessageReaction]| {
            let mut arr = reactions.to_vec();
            let already_mine = arr
                .iter()
                .find(|item| item.emoji == reaction)
                .is_some_and(|item| item.reacted_by_me);
            if !already_mine {
                let mine: Vec<usize> = (0..arr.len()).filter(|index| arr[*index].reacted_by_me).collect();
                let own_timestamp = |index: usize| {
                    arr[index]
                        .reactors
                        .iter()
                        .find(|item| item.user_id == my_id)
                        .map_or(-f64::MAX, |item| item.created_at)
                };
                let evicted_index = (mine.len() >= MAX_OWN_REACTIONS_PER_MESSAGE)
                    .then(|| {
                        mine.iter().copied().min_by(|lhs, rhs| {
                            own_timestamp(*lhs).total_cmp(&own_timestamp(*rhs)).then(lhs.cmp(rhs))
                        })
                    })
                    .flatten();
                if let Some(evicted_index) = evicted_index {
                    arr[evicted_index].count = (arr[evicted_index].count - 1).max(0);
                    arr[evicted_index].reacted_by_me = false;
                    arr[evicted_index].reactors.retain(|item| item.user_id != my_id);
                    if arr[evicted_index].count == 0 {
                        arr.remove(evicted_index);
                    }
                }
            }
            if let Some(idx) = arr.iter().position(|item| item.emoji == reaction) {
                if !arr[idx].reacted_by_me {
                    arr[idx].count += 1;
                }
                arr[idx].reacted_by_me = true;
                if !my_id.is_empty() {
                    arr[idx].reactors.retain(|item| item.user_id != my_id);
                    arr[idx].reactors.insert(0, me.clone());
                }
            } else {
                arr.push(MessageReaction {
                    emoji: reaction.to_owned(),
                    count: 1,
                    reacted_by_me: !my_id.is_empty(),
                    reactors: if my_id.is_empty() { Vec::new() } else { vec![me.clone()] },
                });
            }
            arr
        };
        self.mutate_reaction(chat_id, message_id, &transform, cx);
    }

    pub fn remove_reaction_optimistic(
        &mut self,
        chat_id: &str,
        message_id: &str,
        reaction: &str,
        cx: &mut Context<Self>,
    ) {
        let my_id = current_user_id(cx);

        let transform = |reactions: &[MessageReaction]| {
            let mut arr = reactions.to_vec();
            let Some(idx) = arr.iter().position(|item| item.emoji == reaction) else {
                return arr;
            };
            arr[idx].count = (arr[idx].count - 1).max(0);
            arr[idx].reacted_by_me = false;
            if !my_id.is_empty() {
                arr[idx].reactors.retain(|item| item.user_id != my_id);
            }
            if arr[idx].count == 0 {
                arr.remove(idx);
            }
            arr
        };
        self.mutate_reaction(chat_id, message_id, &transform, cx);
    }

    pub fn add_custom_emoji_reaction_optimistic(
        &mut self,
        chat_id: &str,
        message_id: &str,
        custom_emoji_id: &str,
        cx: &mut Context<Self>,
    ) {
        let reaction = format!("{}{custom_emoji_id}", MessageReaction::CUSTOM_EMOJI_PREFIX);
        self.add_reaction_optimistic(chat_id, message_id, &reaction, cx);
    }

    pub fn remove_custom_emoji_reaction_optimistic(
        &mut self,
        chat_id: &str,
        message_id: &str,
        custom_emoji_id: &str,
        cx: &mut Context<Self>,
    ) {
        let reaction = format!("{}{custom_emoji_id}", MessageReaction::CUSTOM_EMOJI_PREFIX);
        self.remove_reaction_optimistic(chat_id, message_id, &reaction, cx);
    }

    fn resolve_reactor(&mut self, user_id: &str, chat_id: &str, cx: &mut Context<Self>) -> ReactionReactor {
        let now = RiseClock::now().time_interval_since_1970();
        let trimmed = trimming_whitespaces_and_newlines(user_id);
        if trimmed.is_empty() {
            return ReactionReactor { created_at: now, ..ReactionReactor::default() };
        }

        let own_profile = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .filter(|me| me.id == trimmed)
            .map(|me| {
                (
                    me.name.clone(),
                    RemoteMediaURL::optional_normalized(me.avatar_url.as_deref().or(Some(me.more.logo.as_str()))),
                )
            });
        if let Some((name, avatar_url)) = own_profile {
            return ReactionReactor { user_id: trimmed.to_owned(), name, avatar_url, created_at: now };
        }

        let mut named_only: Option<ReactionReactor> = None;
        let chat = ChatServicesStore::shared(cx).update(cx, |chats, cx| chats.chat_info(chat_id, cx));
        if let Some(chat) = chat
            && chat.r#type == ChatType::Direct
            && chat.participant.id == trimmed
        {
            if let Some(logo) = RemoteMediaURL::optional_normalized(Some(chat.participant.more.logo.as_str())) {
                return ReactionReactor {
                    user_id: trimmed.to_owned(),
                    name: chat.participant.name,
                    avatar_url: Some(logo),
                    created_at: now,
                };
            }
            if !chat.participant.name.is_empty() {
                named_only = Some(ReactionReactor {
                    user_id: trimmed.to_owned(),
                    name: chat.participant.name,
                    avatar_url: None,
                    created_at: now,
                });
            }
        }

        let thread = self.messages(chat_id, cx);
        for message in thread.iter() {
            for reaction in &message.reactions {
                let Some(hit) = reaction.reactors.iter().find(|item| item.user_id == trimmed) else {
                    continue;
                };
                if hit.avatar_url.is_some() {
                    return ReactionReactor {
                        user_id: trimmed.to_owned(),
                        name: hit.name.clone(),
                        avatar_url: hit.avatar_url.clone(),
                        created_at: now,
                    };
                }
                if named_only.is_none() && !hit.name.is_empty() {
                    named_only = Some(ReactionReactor {
                        user_id: trimmed.to_owned(),
                        name: hit.name.clone(),
                        avatar_url: None,
                        created_at: now,
                    });
                }
            }
        }
        let sender = thread
            .iter()
            .find(|message| message.sender_id == trimmed && message.sender_logo.is_some())
            .or_else(|| thread.iter().find(|message| message.sender_id == trimmed));
        if let Some(sender) = sender {
            return ReactionReactor {
                user_id: trimmed.to_owned(),
                name: if sender.sender_name.is_empty() {
                    named_only.map(|named| named.name).unwrap_or_default()
                } else {
                    sender.sender_name.clone()
                },
                avatar_url: RemoteMediaURL::optional_normalized(sender.sender_logo.as_deref()),
                created_at: now,
            };
        }
        named_only.unwrap_or_else(|| ReactionReactor {
            user_id: trimmed.to_owned(),
            created_at: now,
            ..ReactionReactor::default()
        })
    }

    fn mutate_reaction_in_realtime_scopes(
        &mut self,
        payload: &Map<String, Value>,
        chat_id: &str,
        message_id: &str,
        transform: &dyn Fn(&[MessageReaction]) -> Vec<MessageReaction>,
        cx: &mut Context<Self>,
    ) {
        let scope = self.realtime_scope(payload, chat_id);
        self.mutate_reaction(&scope, message_id, transform, cx);
        if let Some(thread_scope) = self.on_screen_thread_scope(chat_id, cx) {
            self.mutate_reaction(&thread_scope, message_id, transform, cx);
        }
    }

    fn mutate_reaction(
        &mut self,
        chat_id: &str,
        message_id: &str,
        transform: &dyn Fn(&[MessageReaction]) -> Vec<MessageReaction>,
        cx: &mut Context<Self>,
    ) {
        self.mutate_messages_list(
            chat_id,
            SaiUpdaterWriteScope::Cache,
            |messages| {
                if let Some(idx) = messages.iter().position(|message| message.matches_message_id(message_id)) {
                    messages[idx].reactions = transform(&messages[idx].reactions);
                }
            },
            cx,
        );
    }
}
