use gpui::App;
use rise_canon::rise_key::chat as key;
use rise_canon::{ChatMessageEntityField, ChatPerm};
use rise_core::foundation::foundation_text::trimming_whitespaces;
use rise_core::foundation::ns_range::NSRange;
use serde_json::{Map, Value};

use crate::core::ui::formatted_text_ui::{FormattedTextStyle, RoleMentionSpan};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatRole;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::MessageRoleMentionEntity;

const ROLE_MENTION_ENTITY_TYPE: &str = "role_mention";
const MATCHES_LIMIT: usize = 8;

pub enum RoleMentionComposer {}

impl RoleMentionComposer {
    pub fn candidates(chat_id: &str, cx: &mut App) -> Vec<ChatRole> {
        ChatServicesStore::shared(cx).update(cx, |store, cx| {
            let can_all = store.can(chat_id, ChatPerm::MENTION_EVERYONE, cx);
            store.custom_roles(chat_id, cx).into_iter().filter(|role| role.mentionable || can_all).collect()
        })
    }

    pub fn matches(query: &str, chat_id: &str, cx: &mut App) -> Vec<ChatRole> {
        let all = Self::candidates(chat_id, cx);
        let q = trimming_whitespaces(query).to_lowercase();
        if q.is_empty() {
            return all.into_iter().take(MATCHES_LIMIT).collect();
        }
        let (pref, rest): (Vec<ChatRole>, Vec<ChatRole>) = all.into_iter().partition(|role| role.name.to_lowercase().starts_with(&q));
        let rest = rest.into_iter().filter(|role| role.name.to_lowercase().contains(&q));
        pref.into_iter().chain(rest).take(MATCHES_LIMIT).collect()
    }

    pub fn entities_json(text: &str, chat_id: &str, cx: &mut App) -> Option<String> {
        let mut roles: Vec<ChatRole> = ChatServicesStore::shared(cx)
            .update(cx, |store, cx| store.roles(chat_id, cx))
            .into_iter()
            .filter(|role| !role.is_default && !role.name.is_empty())
            .collect();
        if roles.is_empty() {
            return None;
        }
        roles.sort_by_key(|role| std::cmp::Reverse(role.name.chars().count()));

        let units: Vec<(char, usize)> = Self::utf16_chars(text);
        let total = text.encode_utf16().count();
        let mut taken: Vec<NSRange> = Vec::new();
        let mut entities: Vec<Value> = Vec::new();
        for role in &roles {
            let needle: Vec<char> = std::iter::once('@').chain(role.name.chars()).collect();
            let mut from = 0;
            while from < total {
                let Some(found) = Self::find_case_insensitive(&units, &needle, from) else {
                    break;
                };
                let after = found.location + found.length;
                let boundary_ok = after >= total || Self::is_boundary(&units, after);
                let overlaps = taken.iter().any(|other| Self::intersects(*other, found));
                if boundary_ok && !overlaps {
                    let mut entity = Map::new();
                    entity.insert(ChatMessageEntityField::EntityType.raw_value().to_owned(), Value::from(ROLE_MENTION_ENTITY_TYPE));
                    entity.insert(key::ROLE_ID.to_owned(), Value::from(role.id.clone()));
                    entity.insert(ChatMessageEntityField::Offset.raw_value().to_owned(), Value::from(found.location));
                    entity.insert(ChatMessageEntityField::Length.raw_value().to_owned(), Value::from(found.length));
                    entities.push(Value::Object(entity));
                    taken.push(found);
                }
                from = after;
            }
        }
        if entities.is_empty() {
            return None;
        }
        serde_json::to_string(&entities).ok()
    }

    pub fn render_spans(entities: &[MessageRoleMentionEntity], chat_id: &str, is_own: bool, cx: &mut App) -> Vec<RoleMentionSpan> {
        let mut spans = Vec::with_capacity(entities.len());
        for entity in entities.iter().filter(|entity| entity.length > 0) {
            let role = ChatServicesStore::shared(cx).update(cx, |store, cx| store.role(chat_id, &entity.role_id, cx));
            let Some(role) = role else {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.resolve_mentioned_role(chat_id, &entity.role_id, cx));
                continue;
            };
            let Some(role_color) = role.swift_color() else {
                continue;
            };
            let (Ok(location), Ok(length)) = (usize::try_from(entity.offset), usize::try_from(entity.length)) else {
                continue;
            };
            spans.push(RoleMentionSpan {
                range: NSRange { location, length },
                color: if is_own { FormattedTextStyle::link_color_on_dark_fill() } else { role_color },
                link: None,
            });
        }
        spans
    }

    fn utf16_chars(text: &str) -> Vec<(char, usize)> {
        let mut offset = 0;
        text.chars()
            .map(|c| {
                let entry = (c, offset);
                offset += c.len_utf16();
                entry
            })
            .collect()
    }

    fn find_case_insensitive(units: &[(char, usize)], needle: &[char], from: usize) -> Option<NSRange> {
        let start = units.partition_point(|(_, offset)| *offset < from);
        for window in units.get(start..)?.windows(needle.len()) {
            let same = window.iter().zip(needle).all(|((c, _), n)| c.to_lowercase().eq(n.to_lowercase()));
            if same {
                let (last, last_offset) = window[window.len() - 1];
                let location = window[0].1;
                return Some(NSRange { location, length: last_offset + last.len_utf16() - location });
            }
        }
        None
    }

    fn intersects(a: NSRange, b: NSRange) -> bool {
        a.location.max(b.location) < (a.location + a.length).min(b.location + b.length)
    }

    fn is_boundary(units: &[(char, usize)], offset: usize) -> bool {
        let index = units.partition_point(|(_, unit)| *unit < offset);
        let Some((c, unit)) = units.get(index) else {
            return true;
        };
        if *unit != offset || c.len_utf16() > 1 {
            return true;
        }
        !(c.is_alphanumeric() || *c == '_')
    }
}
