use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui::{App, ElementId, SharedString, Window};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::Date;
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::UnitPoint;

use crate::core::lib::canon_format::RiseFormat;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::message_row::MessageRow;
use crate::modules::chat::components::chat::message_system_pill::{
    MessageSystemFormatContext, MessageSystemFormatter, MessageSystemPill, MessageTopicDivider,
};
use crate::modules::chat::components::chat::scroll::chat_outgoing_message_enter::ChatOutgoingMessageEnterModifiers;
use crate::modules::chat::shared::chat_saved_message_presentation::{
    ChatMessageGrouping, ChatSavedMessagePresentation, RiseMessageSenderIdentity,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::message_permissions::MessagePermissions;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType, User};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageSystemKind,
};

pub type MessageListMessageAction = Rc<dyn Fn(&str, &str, &mut Window, &mut App)>;
pub type MessageListSelectTopic = Rc<dyn Fn(Option<&str>, &mut Window, &mut App)>;
pub type MessageListOpenSenderProfile = Rc<dyn Fn(&User, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct MessageList {
    id: ElementId,
    messages: Vec<GetMessageMessage>,
    chat: Rc<ChatInfo>,
    on_reaction: Option<MessageListMessageAction>,
    on_action: Option<MessageListMessageAction>,
    interactions_enabled: bool,
    allows_swipe_reply: bool,
    draws_inline_avatars: bool,
    group_by_topic: bool,
    topic_titles: HashMap<String, String>,
    on_select_topic: Option<MessageListSelectTopic>,
    on_open_sender_profile: Option<MessageListOpenSenderProfile>,
}

struct MessageListEntriesCache {
    cached_entries: Rc<Vec<Entry>>,
    cached_signature: u64,
}

#[derive(Clone, Copy)]
struct GroupFlags {
    is_first: bool,
    is_last: bool,
}

enum EntryKind {
    DateSeparator(String),
    TopicSeparator { topic_id: Option<String>, title: String },
    System(Rc<GetMessageMessage>),
    Message(Rc<GetMessageMessage>, GroupFlags),
}

struct Entry {
    id: SharedString,
    kind: EntryKind,
}

impl MessageList {
    pub fn new(id: impl Into<ElementId>, messages: Vec<GetMessageMessage>, chat: ChatInfo) -> Self {
        Self {
            id: id.into(),
            messages,
            chat: Rc::new(chat),
            on_reaction: None,
            on_action: None,
            interactions_enabled: true,
            allows_swipe_reply: true,
            draws_inline_avatars: false,
            group_by_topic: false,
            topic_titles: HashMap::new(),
            on_select_topic: None,
            on_open_sender_profile: None,
        }
    }

    pub fn on_reaction(mut self, on_reaction: Option<MessageListMessageAction>) -> Self {
        self.on_reaction = on_reaction;
        self
    }

    pub fn on_action(mut self, on_action: Option<MessageListMessageAction>) -> Self {
        self.on_action = on_action;
        self
    }

    pub fn interactions_enabled(mut self, interactions_enabled: bool) -> Self {
        self.interactions_enabled = interactions_enabled;
        self
    }

    pub fn allows_swipe_reply(mut self, allows_swipe_reply: bool) -> Self {
        self.allows_swipe_reply = allows_swipe_reply;
        self
    }

    pub fn draws_inline_avatars(mut self, draws_inline_avatars: bool) -> Self {
        self.draws_inline_avatars = draws_inline_avatars;
        self
    }

    pub fn group_by_topic(mut self, group_by_topic: bool) -> Self {
        self.group_by_topic = group_by_topic;
        self
    }

    pub fn topic_titles(mut self, topic_titles: HashMap<String, String>) -> Self {
        self.topic_titles = topic_titles;
        self
    }

    pub fn on_select_topic(mut self, on_select_topic: Option<MessageListSelectTopic>) -> Self {
        self.on_select_topic = on_select_topic;
        self
    }

    pub fn on_open_sender_profile(mut self, on_open_sender_profile: Option<MessageListOpenSenderProfile>) -> Self {
        self.on_open_sender_profile = on_open_sender_profile;
        self
    }

    pub fn content_signature(messages: &[GetMessageMessage]) -> u64 {
        let mut hasher = DefaultHasher::new();
        messages.len().hash(&mut hasher);
        for m in messages {
            m.id.hash(&mut hasher);
            m.created_at.to_bits().hash(&mut hasher);
            m.edited_at.map(f64::to_bits).hash(&mut hasher);
            m.text.hash(&mut hasher);
            m.sender_id.hash(&mut hasher);
            m.forward_info.as_ref().map(|info| &info.sender_id).hash(&mut hasher);
            m.forward_info.as_ref().map(|info| &info.sender_name).hash(&mut hasher);
            m.content_type.hash(&mut hasher);
            m.media_items.len().hash(&mut hasher);
            for mi in &m.media_items {
                mi.width.to_bits().hash(&mut hasher);
                mi.height.to_bits().hash(&mut hasher);
                mi.shows_upload_overlay().hash(&mut hasher);
                mi.is_upload_queued().hash(&mut hasher);
                mi.is_uploading.hash(&mut hasher);
                mi.is_compressing.hash(&mut hasher);
                mi.upload_progress.to_bits().hash(&mut hasher);
                mi.compression_progress.to_bits().hash(&mut hasher);
                mi.uploaded_bytes.hash(&mut hasher);
                mi.total_bytes.hash(&mut hasher);
                mi.is_cancelled.hash(&mut hasher);
                mi.local_slot_index.hash(&mut hasher);
                mi.display_source().hash(&mut hasher);
            }
            if let Some(lp) = &m.link_preview {
                lp.url.hash(&mut hasher);
                lp.title.hash(&mut hasher);
                lp.description.hash(&mut hasher);
                lp.site_name.hash(&mut hasher);
                lp.image_url.hash(&mut hasher);
                lp.image_aspect.to_bits().hash(&mut hasher);
                lp.size.hash(&mut hasher);
                lp.is_above_text.hash(&mut hasher);
            }
            for r in &m.reactions {
                r.emoji.hash(&mut hasher);
                r.count.hash(&mut hasher);
                r.reacted_by_me.hash(&mut hasher);
                r.reactors.len().hash(&mut hasher);
                for reactor in &r.reactors {
                    reactor.user_id.hash(&mut hasher);
                    reactor.avatar_url.hash(&mut hasher);
                }
            }
            m.comments_count.hash(&mut hasher);
            m.sparks_total.hash(&mut hasher);
            m.my_sparks.hash(&mut hasher);
            m.discussion_message_id.hash(&mut hasher);
            m.discussion_chat_id.hash(&mut hasher);
            m.recent_commenters.len().hash(&mut hasher);
            for c in &m.recent_commenters {
                c.name.hash(&mut hasher);
                c.logo.hash(&mut hasher);
            }
            m.is_pending_outgoing().hash(&mut hasher);
            m.is_read.hash(&mut hasher);
            m.delivery_status.hash(&mut hasher);
            m.played_by.hash(&mut hasher);
            m.reply_markup.hash(&mut hasher);
            m.is_bot_message.hash(&mut hasher);
            m.via_bot.hash(&mut hasher);
            m.via_bot_tag.hash(&mut hasher);
            m.web_app_data.hash(&mut hasher);
            m.topic_id.hash(&mut hasher);
        }
        hasher.finish()
    }

    fn topic_entry_signature(&self) -> u64 {
        if !self.group_by_topic {
            return 0;
        }
        let mut hasher = DefaultHasher::new();
        true.hash(&mut hasher);
        let mut keys: Vec<&String> = self.topic_titles.keys().collect();
        keys.sort();
        for key in keys {
            key.hash(&mut hasher);
            self.topic_titles.get(key).hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn is_own_bubble(m: &GetMessageMessage, chat: &ChatInfo, cx: &mut App) -> bool {
        if chat.r#type == ChatType::Channel {
            return false;
        }
        if ChatSavedMessagePresentation::author(m, chat, cx).is_some() {
            return false;
        }
        AuthServicesStore::shared(cx).read(cx).is_current_user(&m.sender_id)
    }

    /// dot on outgoing media, peer hasnt played or viewed it yet
    /// clears once played_by holds anyone but the author, pure so the off main geometry hash uses it too
    /// never for favourites or channels
    pub fn awaits_peer_playback(m: &GetMessageMessage, chat: &ChatInfo, is_own: bool, viewer_id: &str) -> bool {
        if !is_own || viewer_id.is_empty() {
            return false;
        }
        if chat.r#type != ChatType::Direct && chat.r#type != ChatType::Group {
            return false;
        }
        !m.played_by.iter().any(|reader| reader != viewer_id)
    }

    fn make_entries(
        messages: &[GetMessageMessage],
        chat: &ChatInfo,
        group_by_topic: bool,
        topic_titles: &HashMap<String, String>,
        cx: &mut App,
    ) -> Vec<Entry> {
        if messages.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut last_day: Option<Date> = None;
        let mut last_shown_topic_id: Option<String> = None;
        let mut shown_any_topic_divider = false;
        let calendar = Calendar::current();
        let flags = ChatMessageGrouping::flags(messages, chat, cx);

        for (index, m) in messages.iter().enumerate() {
            let day = calendar.start_of_day(&Date::from_time_interval_since_1970(m.created_at));
            if last_day.as_ref() != Some(&day) {
                let label = RiseFormat::day_label(&day, &Date::now(), &calendar, &RiseFormat::app_locale(), false);
                out.push(Entry {
                    id: SharedString::from(format!("date-{}", day.time_interval_since_1970())),
                    kind: EntryKind::DateSeparator(label),
                });
                last_day = Some(day);
            }
            if group_by_topic {
                let tid = m.topic_id.clone().filter(|id| !id.is_empty());
                if !shown_any_topic_divider || tid != last_shown_topic_id {
                    let title = tid.as_ref().and_then(|id| topic_titles.get(id)).cloned().unwrap_or_else(|| {
                        if tid.is_none() { tr("chat_topics_general") } else { tr("chat_topics_topic") }
                    });
                    out.push(Entry {
                        id: SharedString::from(format!("topic-{}", m.id)),
                        kind: EntryKind::TopicSeparator { topic_id: tid.clone(), title },
                    });
                    last_shown_topic_id = tid;
                    shown_any_topic_divider = true;
                }
            }
            if m.content_type == MessageContentType::System {
                out.push(Entry {
                    id: SharedString::from(format!("sys-{}", m.id)),
                    kind: EntryKind::System(Rc::new(m.clone())),
                });
            } else {
                let group = GroupFlags { is_first: flags[index].is_first, is_last: flags[index].is_last };
                out.push(Entry {
                    id: SharedString::from(format!("msg-{}", m.id)),
                    kind: EntryKind::Message(Rc::new(m.clone()), group),
                });
            }
        }
        out
    }

    fn row(&self, entry: &Entry, cx: &mut App) -> AnyElement {
        match &entry.kind {
            EntryKind::DateSeparator(label) => div()
                .id(entry.id.clone())
                .pt(px(6.0))
                .pb(px(2.0))
                .child(MessageSystemPill::new(label.clone()))
                .into_any_element(),
            EntryKind::TopicSeparator { topic_id, title } => {
                let on_select_topic = self.on_select_topic.clone();
                let topic_id = topic_id.clone();
                div()
                    .id(entry.id.clone())
                    .pt(px(6.0))
                    .pb(px(2.0))
                    .child(MessageTopicDivider::new(
                        title.clone(),
                        Rc::new(move |window: &mut Window, cx: &mut App| {
                            if let Some(on_select_topic) = &on_select_topic {
                                on_select_topic(topic_id.as_deref(), window, cx);
                            }
                        }),
                    ))
                    .into_any_element()
            }
            EntryKind::System(m) => {
                let context = MessageSystemFormatContext::current(cx).acting_channel(&m.chat_id, cx);
                let system = m.system.as_ref();
                div()
                    .id(entry.id.clone())
                    .py(px(2.0))
                    .child(
                        MessageSystemPill::new(MessageSystemFormatter::render(m, &context))
                            .wallpaper_preview(system.and_then(|system| system.wallpaper_preview.clone()))
                            .shows_gift(system.is_some_and(|system| system.kind == MessageSystemKind::PremiumGifted))
                            .gift_item(
                                system
                                    .filter(|system| system.kind == MessageSystemKind::GiftSent)
                                    .and_then(|system| system.gift_item.clone()),
                            ),
                    )
                    .into_any_element()
            }
            EntryKind::Message(m, group) => self.message_row(entry, m, *group, cx),
        }
    }

    fn message_row(&self, entry: &Entry, m: &Rc<GetMessageMessage>, group: GroupFlags, cx: &mut App) -> AnyElement {
        let chat = &self.chat;
        let is_own = Self::is_own_bubble(m, chat, cx);
        let can_reply = self.allows_swipe_reply
            && self.interactions_enabled
            && MessagePermissions::new(m, chat, is_own).can_reply(cx);
        let incoming_age = Date::now().time_interval_since_1970() - m.created_at;
        let is_fresh_incoming = !is_own
            && self.messages.last().is_some_and(|last| last.id == m.id)
            && incoming_age > -1.0
            && incoming_age < 4.0;

        let on_reaction = self.on_reaction.clone();
        let reaction_id = m.id.clone();
        let on_action = self.on_action.clone();
        let action_id = m.id.clone();
        let on_open_sender_profile = self.on_open_sender_profile.clone();
        let profile_message = m.clone();
        let profile_chat = chat.clone();
        let top_padding = if group.is_first {
            MessageSizes::list_first_in_group_top_padding()
        } else {
            MessageSizes::list_continuation_top_padding()
        };
        let bottom_padding = if group.is_last { MessageSizes::list_last_in_group_bottom_padding() } else { 0.0 };

        let row = MessageRow::new(
            (**m).clone(),
            (**chat).clone(),
            is_own,
            group.is_first,
            group.is_last,
            chat.r#type == ChatType::Group || chat.r#type == ChatType::Favourites,
        )
        .draws_inline_avatar(self.draws_inline_avatars)
        .allows_reply(can_reply)
        .interactions_enabled(self.interactions_enabled)
        .on_reaction(Some(Rc::new(move |emoji: &str, window: &mut Window, cx: &mut App| {
            if let Some(on_reaction) = &on_reaction {
                on_reaction(&reaction_id, emoji, window, cx);
            }
        })))
        .on_action(Some(Rc::new(move |action: &str, window: &mut Window, cx: &mut App| {
            if let Some(on_action) = &on_action {
                on_action(&action_id, action, window, cx);
            }
        })))
        .on_open_sender_profile(Some(Rc::new(move |window: &mut Window, cx: &mut App| {
            if let Some(on_open) = &on_open_sender_profile {
                let participant = RiseMessageSenderIdentity::participant(&profile_message, &profile_chat, cx);
                on_open(&participant, window, cx);
            }
        })));

        div()
            .id(entry.id.clone())
            .child(
                div().pt(px(top_padding)).pb(px(bottom_padding)).child(row).chat_outgoing_message_enter(
                    ElementId::Name(SharedString::from(format!("{}-enter", entry.id))),
                    (is_own && m.is_pending_outgoing()) || is_fresh_incoming,
                    if is_own { UnitPoint::BOTTOM_TRAILING } else { UnitPoint::BOTTOM_LEADING },
                ),
            )
            .into_any_element()
    }
}

impl RenderOnce for MessageList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let signature = Self::content_signature(&self.messages).wrapping_add(self.topic_entry_signature());
        let cache = window.use_keyed_state(self.id.clone(), cx, |_, _| MessageListEntriesCache {
            cached_entries: Rc::new(Vec::new()),
            cached_signature: 0,
        });
        let is_stale = {
            let cache = cache.read(cx);
            cache.cached_signature != signature || (cache.cached_entries.is_empty() && !self.messages.is_empty())
        };
        if is_stale {
            let entries =
                Self::make_entries(&self.messages, &self.chat, self.group_by_topic, &self.topic_titles, cx);
            cache.update(cx, |cache, _| {
                cache.cached_signature = signature;
                cache.cached_entries = Rc::new(entries);
            });
        }
        let entries = cache.read(cx).cached_entries.clone();
        let mut column = div().id(self.id.clone()).w_full().flex().flex_col().gap(px(MessageSizes::list_row_spacing()));
        for entry in entries.iter() {
            column = column.child(self.row(entry, cx));
        }
        column
    }
}
