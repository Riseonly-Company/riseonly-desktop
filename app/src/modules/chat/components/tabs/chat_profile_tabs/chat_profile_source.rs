use gpui::App;
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, DateComponents};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_i18n::tr;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;

use crate::core::lib::canon_format::RiseFormat;
use crate::modules::chat::engine::chat_engine_hooks::{chat_engine_components_hooks, chat_engine_hooks};
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::shared::message_permissions::{ChatMessageAction, MessagePermissions};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageLinkPreview, MessageMediaItem, MessageMediaKind,
};

#[derive(Clone, Debug, PartialEq)]
pub struct ChatProfileMediaEntry {
    pub id: String,
    pub message_id: String,
    pub created_at: TimeInterval,
    pub item: MessageMediaItem,
    pub sender_name: String,
    pub sender_id: String,
    pub has_protected_content: bool,
    pub sender_logo: Option<String>,
    pub topic_id: Option<String>,
}

impl ChatProfileMediaEntry {
    /// the grid cell id and the viewer frame key, message id then media index
    pub fn entry_id(message_id: &str, media_index: i64) -> String {
        format!("{message_id}-{media_index}")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatProfileLinkEntry {
    pub id: String,
    pub message_id: String,
    pub created_at: TimeInterval,
    pub preview: MessageLinkPreview,
    pub source_text: String,
    pub sender_name: String,
    pub sender_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatProfileFavEntry {
    pub id: String,
    pub message: GetMessageMessage,
}

impl ChatProfileFavEntry {
    pub fn created_at(&self) -> TimeInterval {
        self.message.created_at
    }
}

pub enum ChatProfileSource {}

impl ChatProfileSource {
    pub fn messages(chat_id: &str, cx: &mut App) -> Vec<GetMessageMessage> {
        chat_engine_hooks().map(|hooks| (hooks.messages)(chat_id, cx)).unwrap_or_default()
    }

    pub fn media_items(chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> Vec<ChatProfileMediaEntry> {
        let live = chat_engine_hooks()
            .map(|hooks| (hooks.media_tab_entries)(&(hooks.message_scope)(chat_id, topic_id, None), cx))
            .unwrap_or_default();
        if Self::is_presentation_active(cx) || !live.is_empty() {
            return live;
        }
        if topic_id.is_some() {
            return Vec::new();
        }
        Self::collect(chat_id, &[MessageMediaKind::Image, MessageMediaKind::Video], cx)
    }

    pub fn video_items(chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry> {
        let mut media = Self::media_items(chat_id, None, cx);
        media.retain(|entry| entry.item.kind == MessageMediaKind::Video);
        media
    }

    pub fn gif_items(chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry> {
        let live = chat_engine_hooks().map(|hooks| (hooks.gif_tab_entries)(chat_id, cx)).unwrap_or_default();
        if Self::is_presentation_active(cx) || !live.is_empty() {
            return live;
        }
        Self::collect(chat_id, &[MessageMediaKind::Animation, MessageMediaKind::Gif], cx)
    }

    pub fn document_items(chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> Vec<ChatProfileMediaEntry> {
        let live = chat_engine_hooks()
            .map(|hooks| (hooks.file_tab_entries)(&(hooks.message_scope)(chat_id, topic_id, None), cx))
            .unwrap_or_default();
        if Self::is_presentation_active(cx) || !live.is_empty() {
            return live;
        }
        if topic_id.is_some() {
            return Vec::new();
        }
        Self::collect(chat_id, &[MessageMediaKind::Document], cx)
    }

    pub fn audio_items(chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry> {
        let live = chat_engine_hooks().map(|hooks| (hooks.music_tab_entries)(chat_id, cx)).unwrap_or_default();
        if Self::is_presentation_active(cx) || !live.is_empty() {
            return live;
        }
        Self::collect(chat_id, &[MessageMediaKind::Audio], cx)
    }

    pub fn voice_items(chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry> {
        let live = chat_engine_hooks().map(|hooks| (hooks.audio_tab_entries)(chat_id, cx)).unwrap_or_default();
        if Self::is_presentation_active(cx) || !live.is_empty() {
            return live;
        }
        Self::collect(chat_id, &[MessageMediaKind::Voice], cx)
    }

    pub fn favourite_items(chat_id: &str, cx: &mut App) -> Vec<ChatProfileFavEntry> {
        chat_engine_hooks().map(|hooks| (hooks.fav_tab_entries)(chat_id, cx)).unwrap_or_default()
    }

    /// bare protected check offered forward for sources the bubble hides and the server refused the send
    pub fn forwardable_message(
        entry: &ChatProfileMediaEntry,
        chat_id: &str,
        cx: &mut App,
    ) -> Option<GetMessageMessage> {
        let chat = chat_engine_hooks().and_then(|hooks| (hooks.chat_info)(chat_id, cx))?;
        let message = Self::forward_message(entry, chat_id, cx);
        let is_own = Self::is_own_bubble(&message, &chat, cx);
        MessagePermissions::new(&message, &chat, is_own).can_forward(cx).then_some(message)
    }

    pub fn can_forward(entry: &ChatProfileMediaEntry, chat_id: &str, cx: &mut App) -> bool {
        Self::forwardable_message(entry, chat_id, cx).is_some()
    }

    pub fn delete_action(
        message_id: &str,
        sender_id: &str,
        chat_id: &str,
        cx: &mut App,
    ) -> Option<HoldContextMenuAction> {
        let chat = chat_engine_hooks().and_then(|hooks| (hooks.chat_info)(chat_id, cx))?;
        let message = chat_engine_hooks()
            .and_then(|hooks| (hooks.message)(message_id, chat_id, cx))
            .unwrap_or_else(|| {
                let mut message = GetMessageMessage::new(message_id.to_owned());
                message.chat_id = chat_id.to_owned();
                message.sender_id = sender_id.to_owned();
                message
            });
        let is_own = Self::is_own_bubble(&message, &chat, cx);
        MessagePermissions::new(&message, &chat, is_own).delete_row(cx)
    }

    /// true if a delete row deletes for everyone, none for any other action
    pub fn deletes_for_everyone(action: &str) -> Option<bool> {
        match action {
            ChatMessageAction::DELETE_EVERYONE
            | ChatMessageAction::DELETE_MODERATE
            | ChatMessageAction::DELETE_FROM_FAVOURITES => Some(true),
            ChatMessageAction::DELETE_ME => Some(false),
            _ => None,
        }
    }

    pub fn forward_message(entry: &ChatProfileMediaEntry, chat_id: &str, cx: &mut App) -> GetMessageMessage {
        if let Some(cached) = chat_engine_hooks().and_then(|hooks| (hooks.message)(&entry.message_id, chat_id, cx)) {
            return cached;
        }
        let mut message = GetMessageMessage::new(entry.message_id.clone());
        message.chat_id = chat_id.to_owned();
        message.sender_id = entry.sender_id.clone();
        message.sender_name = entry.sender_name.clone();
        message.created_at = entry.created_at;
        message.media_items = vec![entry.item.clone()];
        message.has_protected_content = entry.has_protected_content;
        message.content_type = match entry.item.kind {
            MessageMediaKind::Image => MessageContentType::Image,
            MessageMediaKind::Video => MessageContentType::Video,
            MessageMediaKind::Animation | MessageMediaKind::Gif => MessageContentType::Animation,
            MessageMediaKind::Audio => MessageContentType::Audio,
            MessageMediaKind::Voice => MessageContentType::Voice,
            MessageMediaKind::Document => MessageContentType::Document,
            MessageMediaKind::Sticker => MessageContentType::Sticker,
            MessageMediaKind::VideoNote => MessageContentType::VideoNote,
        };
        message
    }

    pub fn link_items(chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> Vec<ChatProfileLinkEntry> {
        let live = chat_engine_hooks()
            .map(|hooks| (hooks.link_tab_entries)(&(hooks.message_scope)(chat_id, topic_id, None), cx))
            .unwrap_or_default();
        if Self::is_presentation_active(cx) || !live.is_empty() {
            return live;
        }
        if topic_id.is_some() {
            return Vec::new();
        }
        let mut out: Vec<ChatProfileLinkEntry> = Self::messages(chat_id, cx)
            .into_iter()
            .filter_map(|message| {
                let preview = message.link_preview?;
                Some(ChatProfileLinkEntry {
                    id: format!("link-{}", message.id),
                    message_id: message.id,
                    created_at: message.created_at,
                    preview,
                    source_text: message.text,
                    sender_name: message.sender_name,
                    sender_id: message.sender_id,
                })
            })
            .collect();
        out.sort_by(|left, right| right.created_at.total_cmp(&left.created_at));
        out
    }

    fn collect(chat_id: &str, kinds: &[MessageMediaKind], cx: &mut App) -> Vec<ChatProfileMediaEntry> {
        let mut out = Vec::new();
        for message in Self::messages(chat_id, cx) {
            if message.content_type == MessageContentType::System {
                continue;
            }
            for (index, item) in message.media_items.iter().enumerate() {
                if !kinds.contains(&item.kind) || item.view_limit.is_some() {
                    continue;
                }
                out.push(ChatProfileMediaEntry {
                    id: ChatProfileMediaEntry::entry_id(&message.id, i64::try_from(index).unwrap_or(i64::MAX)),
                    message_id: message.id.clone(),
                    created_at: message.created_at,
                    item: item.clone(),
                    sender_name: message.sender_name.clone(),
                    sender_id: message.sender_id.clone(),
                    has_protected_content: message.has_protected_content,
                    sender_logo: message.sender_logo.clone(),
                    topic_id: message.topic_id.clone(),
                });
            }
        }
        out.sort_by(|left, right| right.created_at.total_cmp(&left.created_at));
        out
    }

    fn is_presentation_active(cx: &mut App) -> bool {
        RiseConversationPresentation::shared(cx).read(cx).is_active()
    }

    fn is_own_bubble(message: &GetMessageMessage, chat: &ChatInfo, cx: &mut App) -> bool {
        chat_engine_components_hooks().is_some_and(|hooks| (hooks.is_own_bubble)(message, chat, cx))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ChatProfileSectionLabel {
    Today,
    Yesterday,
    ThisWeek,
    ThisMonth,
    Month(Date),
}

impl ChatProfileSectionLabel {
    pub fn title(&self) -> String {
        match self {
            Self::Today => tr("chat_profile_section_today"),
            Self::Yesterday => tr("chat_profile_section_yesterday"),
            Self::ThisWeek => tr("chat_profile_section_this_week"),
            Self::ThisMonth => tr("chat_profile_section_this_month"),
            Self::Month(date) => {
                let raw = RiseFormat::date_formatter(
                    &tr("date_month_year_format"),
                    &RiseFormat::app_locale(),
                    &Calendar::current(),
                    None,
                )
                .string_from(date);
                let mut chars = raw.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                    None => raw,
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatProfileSection<T> {
    pub id: String,
    pub label: ChatProfileSectionLabel,
    pub items: Vec<T>,
}

impl<T> ChatProfileSection<T> {
    pub fn title(&self) -> String {
        self.label.title()
    }
}

pub enum ChatProfileSectioner {}

impl ChatProfileSectioner {
    pub fn section<T: Clone>(items: &[T], date: impl Fn(&T) -> TimeInterval) -> Vec<ChatProfileSection<T>> {
        if items.is_empty() {
            return Vec::new();
        }
        let mut buckets: Vec<(String, ChatProfileSectionLabel, Vec<T>)> = Vec::new();
        let cal = Calendar::current();
        let now = RiseClock::now();
        let week_start = cal.date_interval(CalendarComponent::WeekOfYear, &now).map(|interval| interval.start);
        let month_start = cal.date_interval(CalendarComponent::Month, &now).map(|interval| interval.start);

        for item in items {
            let d = Date::from_time_interval_since_1970(date(item));
            let (key, label) = if cal.is_date_in_today(&d) {
                ("today".to_owned(), ChatProfileSectionLabel::Today)
            } else if cal.is_date_in_yesterday(&d) {
                ("yesterday".to_owned(), ChatProfileSectionLabel::Yesterday)
            } else if week_start.is_some_and(|start| d >= start) {
                ("week".to_owned(), ChatProfileSectionLabel::ThisWeek)
            } else if month_start.is_some_and(|start| d >= start) {
                ("month".to_owned(), ChatProfileSectionLabel::ThisMonth)
            } else {
                let components = cal.date_components(&[CalendarComponent::Year, CalendarComponent::Month], &d);
                let month_date = cal
                    .date_from_components(&DateComponents {
                        year: components.year,
                        month: components.month,
                        day: Some(1),
                        ..DateComponents::default()
                    })
                    .unwrap_or(d);
                (
                    format!("{}-{}", components.year.unwrap_or(0), components.month.unwrap_or(0)),
                    ChatProfileSectionLabel::Month(month_date),
                )
            };
            if let Some(bucket) = buckets.iter_mut().find(|bucket| bucket.0 == key) {
                bucket.2.push(item.clone());
            } else {
                buckets.push((key, label, vec![item.clone()]));
            }
        }
        buckets
            .into_iter()
            .enumerate()
            .map(|(index, (key, label, items))| ChatProfileSection { id: format!("sec-{index}-{key}"), label, items })
            .collect()
    }
}
