use gpui::App;
use rise_canon::{ChatMessageEntityField, ChatMessageEntityType, ChatPerm};
use rise_core::foundation::foundation_text::trimming_whitespaces;
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::tr;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use serde_json::{Map, Value};

use crate::app::router::navigation_types::RiseRTCRoomTarget;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::formatted_text_ui::{FormattedTextStyle, RoleMentionSpan};
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::pages::chats::chat::chat_roles::topic_mention_composer_hooks::topic_mention_composer_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatPermExt, ChatTopic, ChatVoiceConnectGateDecision};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::MessageTopicMentionEntity;

/// in a forum # names a channel the way @ names a person
/// the list and topic_mention entity and bubble span and tap live here
/// so composer and bubble and link opener agree
pub enum TopicMentionComposer {}

impl TopicMentionComposer {
    const LIST_LIMIT: usize = 8;

    fn entity_type() -> &'static str {
        ChatMessageEntityType::TopicMention.raw_value()
    }

    /// the mention channels right where the person writes
    /// a channel overwrite of the open topic wins over the chat level, same order as the server
    pub fn can_mention(chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> bool {
        let permissions = ChatServicesStore::shared(cx).update(cx, |store, cx| {
            store
                .effective_permissions(chat_id, topic_id, cx)
                .or_else(|| store.effective_permissions(chat_id, None, cx))
        });
        ChatPerm::has(permissions.map_or(0, |permissions| permissions.permissions), ChatPerm::MENTION_TOPICS)
    }

    /// text and voice channels this person sees in forum order
    /// general has no topic row so it cant be named
    pub fn candidates(chat_id: &str, cx: &mut App) -> Vec<ChatTopic> {
        ChatServicesStore::shared(cx)
            .update(cx, |store, cx| store.forum_topics(chat_id, cx))
            .into_iter()
            .filter(|topic| !topic.is_general && !trimming_whitespaces(&topic.title).is_empty())
            .collect()
    }

    pub fn matches(query: &str, chat_id: &str, cx: &mut App) -> Vec<ChatTopic> {
        let all = Self::candidates(chat_id, cx);
        let q = trimming_whitespaces(query).to_lowercase();
        if q.is_empty() {
            return all.into_iter().take(Self::LIST_LIMIT).collect();
        }
        let (prefix, rest): (Vec<ChatTopic>, Vec<ChatTopic>) =
            all.into_iter().partition(|topic| topic.title.to_lowercase().starts_with(&q));
        let rest = rest.into_iter().filter(|topic| topic.title.to_lowercase().contains(&q));
        prefix.into_iter().chain(rest).take(Self::LIST_LIMIT).collect()
    }

    /// finds every #channel of this forum in the text and adds a topic_mention for it
    /// the server keeps only channels the sender may name
    pub fn entities_json(
        text: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        merging_into: Option<String>,
        cx: &mut App,
    ) -> Option<String> {
        if !text.contains('#') || !Self::can_mention(chat_id, topic_id, cx) {
            return merging_into;
        }
        let mut topics = Self::candidates(chat_id, cx);
        if topics.is_empty() {
            return merging_into;
        }
        let mut entities = Self::existing_entities(merging_into.as_deref());
        let mut taken: Vec<NSRange> = entities.iter().filter_map(Self::entity_range).collect();
        let units = Self::utf16_chars(text);
        let total = text.encode_utf16().count();
        let mut added = false;
        topics.sort_by_key(|topic| std::cmp::Reverse(topic.title.encode_utf16().count()));
        for topic in &topics {
            let needle: Vec<char> = std::iter::once('#').chain(topic.title.chars()).collect();
            let mut from = 0;
            while from < total {
                let Some(found) = Self::find_case_insensitive(&units, &needle, from) else {
                    break;
                };
                let after = found.location + found.length;
                let start_is_free = found.location == 0 || Self::is_whitespace_before(&units, found.location);
                let end_is_free = after >= total || Self::is_boundary(&units, after);
                let overlaps = taken.iter().any(|other| Self::intersects(*other, found));
                if start_is_free && end_is_free && !overlaps {
                    let mut entity = Map::new();
                    entity.insert(ChatMessageEntityField::EntityType.raw_value().to_owned(), Value::from(Self::entity_type()));
                    entity.insert(ChatMessageEntityField::TopicId.raw_value().to_owned(), Value::from(topic.id.clone()));
                    entity.insert(ChatMessageEntityField::ChatId.raw_value().to_owned(), Value::from(chat_id.to_owned()));
                    entity.insert(ChatMessageEntityField::Offset.raw_value().to_owned(), Value::from(found.location));
                    entity.insert(ChatMessageEntityField::Length.raw_value().to_owned(), Value::from(found.length));
                    entities.push(entity);
                    taken.push(found);
                    added = true;
                }
                from = after;
            }
        }
        if !added {
            return merging_into;
        }
        let offset_key = ChatMessageEntityField::Offset.raw_value();
        entities.sort_by_key(|entity| Self::integer(entity.get(offset_key)).unwrap_or(i64::MAX));
        let entities: Vec<Value> = entities.into_iter().map(Value::Object).collect();
        match serde_json::to_string(&entities) {
            Ok(json) => Some(json),
            Err(_) => merging_into,
        }
    }

    /// a channel name in a bubble is a link to that channel
    /// the entity keeps its group so a forwarded message still leads to the original group
    pub fn render_spans(entities: &[MessageTopicMentionEntity], chat_id: &str) -> Vec<RoleMentionSpan> {
        entities
            .iter()
            .filter_map(|entity| {
                if entity.length <= 0 || entity.topic_id.is_empty() {
                    return None;
                }
                let group = if entity.chat_id.is_empty() { chat_id.to_owned() } else { entity.chat_id.clone() };
                let url = RiseonlyPublicLink::ChatTopic { chat_id: group, topic_id: entity.topic_id.clone() }
                    .web_url(AppEnvironment::current())?;
                let (Ok(location), Ok(length)) = (usize::try_from(entity.offset), usize::try_from(entity.length)) else {
                    return None;
                };
                Some(RoleMentionSpan {
                    range: NSRange { location, length },
                    color: FormattedTextStyle::standard_link_color(),
                    link: Some(url.to_string()),
                })
            })
            .collect()
    }

    /// a tap on a named channel, returns false when the caller should route it as a link
    /// a text channel goes through the same route as a deep link, so does an unknown group
    /// a voice channel asks first
    pub fn open(chat_id: &str, topic_id: &str, cx: &mut App) -> bool {
        let topics = ChatServicesStore::shared(cx).update(cx, |store, cx| store.forum_topics(chat_id, cx));
        let Some(topic) = topics.iter().find(|topic| topic.id == topic_id) else {
            if topics.is_empty() {
                return false;
            }
            NativeConfirmAlert::present_notice(&tr("chat_topic_mention_missing"), "", None, None, |_| {}, cx);
            return true;
        };
        if !topic.is_rtc_channel() {
            ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.topic_opened(chat_id, Some(topic_id), cx));
            return false;
        }
        let title = topic.title.clone();
        let (room_chat_id, room_topic_id) = (chat_id.to_owned(), topic_id.to_owned());
        let on_result = Box::new(move |decision: ChatVoiceConnectGateDecision, cx: &mut App| match decision {
            ChatVoiceConnectGateDecision::Allowed => {
                NativeConfirmAlert::present_confirm(
                    &tr("chat_topic_mention_join_title"),
                    &title,
                    &tr("chat_topic_mention_join"),
                    move |cx| {
                        let target = RiseRTCRoomTarget::Topic { chat_id: room_chat_id, topic_id: room_topic_id };
                        if let Some(hooks) = topic_mention_composer_hooks() {
                            (hooks.present_room)(target, cx);
                        }
                    },
                    cx,
                );
            }
            ChatVoiceConnectGateDecision::Denied => {
                NativeConfirmAlert::present_notice(&tr("chat_topic_mention_no_access"), &title, None, None, |_| {}, cx);
            }
            ChatVoiceConnectGateDecision::RefreshRequired => {
                Notifier::send(
                    tr("rtc_permission_check_failed"),
                    NotifierSendOptions {
                        r#type: NotifierType::Error,
                        position: NotifierPosition::Bottom,
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            }
        });
        ChatInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.resolve_voice_connect_permission(chat_id, Some(topic_id), on_result, cx));
        true
    }

    fn existing_entities(value: Option<&str>) -> Vec<Map<String, Value>> {
        value.and_then(|value| serde_json::from_str::<Vec<Map<String, Value>>>(value).ok()).unwrap_or_default()
    }

    fn entity_range(entity: &Map<String, Value>) -> Option<NSRange> {
        let offset = Self::integer(entity.get(ChatMessageEntityField::Offset.raw_value()))?;
        let length = Self::integer(entity.get(ChatMessageEntityField::Length.raw_value()))?;
        if offset < 0 || length <= 0 {
            return None;
        }
        Some(NSRange { location: usize::try_from(offset).ok()?, length: usize::try_from(length).ok()? })
    }

    fn integer(value: Option<&Value>) -> Option<i64> {
        match value? {
            Value::Number(number) => number.as_i64().or_else(|| number.as_f64().map(|value| value as i64)),
            Value::String(text) => text.parse().ok(),
            _ => None,
        }
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

    fn is_whitespace_before(units: &[(char, usize)], offset: usize) -> bool {
        let index = units.partition_point(|(_, unit)| *unit < offset);
        let Some(index) = index.checked_sub(1) else {
            return false;
        };
        let (c, unit) = units[index];
        if unit + c.len_utf16() != offset || c.len_utf16() > 1 {
            return false;
        }
        c.is_whitespace()
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
