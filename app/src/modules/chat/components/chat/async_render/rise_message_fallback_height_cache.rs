use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use gpui::{App, Context, Entity};
use rise_canon::RiseLimits;
use rise_chat::ChatType;
use rise_core::foundation::foundation_calendar::{Calendar, TimeZone};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::language_store::Locale;
use rise_theme::ThemeStore;
use rise_ui::swiftui::environment::LayoutDirection;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{
    ChatMessageRenderEntry, ChatMessageRenderEntryGroupFlags, ChatMessageRenderEntryKind,
};
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageBotReplyMarkup, MessageContactInfo, MessageDeliveryStatus, MessageForwardInfo,
    MessageLinkPreview, MessageLocationInfo, MessageMediaItem, MessageMediaKind, MessagePollInfo, MessageReaction,
    MessageReplyTo, MessageSharedPost, MessageStoryReply,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RiseMessageGeometryVersion {
    pub raw_value: u64,
}

pub struct RiseMessageFallbackGeometryContext {
    pub current_user_id: String,

    group_chat_id: Option<String>,
    custom_titles_by_user_id: Option<HashMap<String, String>>,
}

impl RiseMessageFallbackGeometryContext {
    pub fn new(chat: &ChatInfo, cx: &mut App) -> Self {
        let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        Self {
            current_user_id,
            group_chat_id: (chat.r#type == ChatType::Group).then(|| chat.id.clone()),
            custom_titles_by_user_id: None,
        }
    }

    pub fn custom_title(&mut self, user_id: &str, cx: &mut App) -> Option<String> {
        let group_chat_id = self.group_chat_id.as_deref()?;
        if self.custom_titles_by_user_id.is_none() {
            let mut titles = HashMap::new();
            let members =
                ChatServicesStore::shared(cx).update(cx, |store, cx| store.members_in_memory(group_chat_id, cx));
            for member in members {
                if member.custom_title.is_empty() {
                    continue;
                }
                titles.insert(member.user_id, member.custom_title);
            }
            self.custom_titles_by_user_id = Some(titles);
        }
        self.custom_titles_by_user_id.as_ref().and_then(|titles| titles.get(user_id).cloned())
    }
}

pub enum RiseMessageFallbackGeometry {}

impl RiseMessageFallbackGeometry {
    /// default: context none builds a context for this call only
    pub fn version(
        entry: &ChatMessageRenderEntry,
        chat: &ChatInfo,
        calendar: &Calendar,
        time_zone: &TimeZone,
        context: Option<&mut RiseMessageFallbackGeometryContext>,
        cx: &mut App,
    ) -> RiseMessageGeometryVersion {
        let mut owned_context;
        let context = match context {
            Some(context) => context,
            None => {
                owned_context = RiseMessageFallbackGeometryContext::new(chat, cx);
                &mut owned_context
            }
        };
        if let Some(prepared) = entry.prepared_geometry_version {
            return Self::contextualized_prepared_version(prepared, entry, chat, context, cx);
        }
        let mut hasher = RiseMessageStableGeometryHasher::new();
        hasher.string("rise-message-geometry-v9-block-document");
        hasher.string(&chat.id);
        hasher.string(chat.r#type.wire());
        hasher.string(&format!("{:?}", calendar.identifier));
        hasher.string(&time_zone.identifier());
        hasher.integer(Self::kind_code(entry.kind));
        hasher.string(&entry.label);
        hasher.boolean(entry.group.is_some());
        if let Some(group) = entry.group {
            hasher.boolean(group.is_first);
            hasher.boolean(group.is_last);
        }

        let message = match (entry.kind, entry.message.as_deref()) {
            (ChatMessageRenderEntryKind::Message, Some(message)) => message,
            _ => return RiseMessageGeometryVersion { raw_value: hasher.finalize() },
        };

        let is_own = MessageList::is_own_bubble(message, chat, cx);
        hasher.boolean(is_own);
        Self::hash_message(message, chat, is_own, entry.group, context, &mut hasher, cx);
        RiseMessageGeometryVersion { raw_value: hasher.finalize() }
    }

    fn contextualized_prepared_version(
        prepared: RiseMessageGeometryVersion,
        entry: &ChatMessageRenderEntry,
        chat: &ChatInfo,
        context: &mut RiseMessageFallbackGeometryContext,
        cx: &mut App,
    ) -> RiseMessageGeometryVersion {
        let mut hasher = DefaultHasher::new();
        "rise-message-geometry-context-v1".hash(&mut hasher);
        prepared.hash(&mut hasher);
        chat.r#type.hash(&mut hasher);
        context.current_user_id.hash(&mut hasher);

        let message = match (entry.kind, entry.message.as_deref()) {
            (ChatMessageRenderEntryKind::Message, Some(message)) => message,
            _ => return RiseMessageGeometryVersion { raw_value: hasher.finish() },
        };

        let is_own = MessageList::is_own_bubble(message, chat, cx);
        is_own.hash(&mut hasher);
        let is_first_in_group = entry.group.is_some_and(|group| group.is_first);
        if chat.r#type == ChatType::Group && !is_own && is_first_in_group {
            context.custom_title(&message.sender_id, cx).hash(&mut hasher);
        } else if chat.r#type == ChatType::Channel && is_first_in_group {
            chat.title.hash(&mut hasher);
        }
        RiseMessageGeometryVersion { raw_value: hasher.finish() }
    }

    pub fn effective_bubble_width(
        viewport_width: f32,
        entry: &ChatMessageRenderEntry,
        chat: &ChatInfo,
        cx: &mut App,
    ) -> f32 {
        let message = match (entry.kind, entry.message.as_deref()) {
            (ChatMessageRenderEntryKind::Message, Some(message)) => message,
            _ => return viewport_width.max(1.0),
        };
        let is_own = MessageList::is_own_bubble(message, chat, cx);
        MessageSizes::available_bubble_width(viewport_width, chat, is_own)
    }

    pub fn effective_bubble_width_bucket(
        viewport_width: f32,
        entry: &ChatMessageRenderEntry,
        chat: &ChatInfo,
        display_scale: f32,
        cx: &mut App,
    ) -> (i64, i64) {
        let normalized_scale = if display_scale.is_finite() { display_scale.max(1.0) } else { 1.0 };
        let effective_width = Self::effective_bubble_width(viewport_width, entry, chat, cx);
        ((effective_width * normalized_scale).round() as i64, (normalized_scale * 1_000.0).round() as i64)
    }

    fn hash_message(
        message: &GetMessageMessage,
        chat: &ChatInfo,
        is_own: bool,
        group: Option<ChatMessageRenderEntryGroupFlags>,
        context: &mut RiseMessageFallbackGeometryContext,
        hasher: &mut RiseMessageStableGeometryHasher,
        cx: &mut App,
    ) {
        hasher.string(message.content_type.raw_value());
        hasher.string(&message.text);
        hasher.optional_string(message.entities.as_deref());
        hasher.floating(if message.created_at.is_finite() { (message.created_at / 60.0).floor() } else { 0.0 });
        hasher.boolean(message.edited_at.is_some());
        hasher.boolean(message.is_pinned);
        hasher.boolean(message.is_temp);
        hasher.boolean(message.is_pending_outgoing());
        hasher.integer(Self::delivery_status_code(message.delivery_status));
        hasher.boolean(message.is_read);
        hasher.boolean(MessageList::awaits_peer_playback(message, chat, is_own, &context.current_user_id));

        if chat.r#type == ChatType::Channel {
            hasher.optional_integer(message.view_count);
        }

        let is_first_in_group = group.is_some_and(|group| group.is_first);
        if chat.r#type == ChatType::Group && !is_own && is_first_in_group {
            hasher.string(&message.sender_name);
            hasher.boolean(message.sender_is_official);
            hasher.optional_string(context.custom_title(&message.sender_id, cx).as_deref());
        } else if chat.r#type == ChatType::Favourites && !is_own && is_first_in_group {
            let forward = message.forward_info.as_ref();
            hasher.optional_string(forward.and_then(|forward| forward.sender_id.as_deref()));
            hasher.optional_string(forward.map(|forward| forward.sender_name.as_str()));
        } else if chat.r#type == ChatType::Channel && is_first_in_group {
            hasher.string(&chat.title);
        }

        hasher.integer(message.media_items.len() as i64);
        for item in &message.media_items {
            Self::hash_media_item(item, hasher);
        }

        Self::hash_reply(message.reply_to.as_ref(), hasher);
        Self::hash_story_reply(message.story_reply.as_ref(), hasher);
        hasher.optional_string(message.document_raw.as_deref());
        Self::hash_forward(message.forward_info.as_ref(), hasher);
        Self::hash_link_preview(message.link_preview.as_ref(), hasher);
        Self::hash_contact(message.contact.as_ref(), hasher);
        Self::hash_location(message.location.as_ref(), hasher);
        Self::hash_poll(message.poll.as_ref(), hasher);
        Self::hash_shared_post(message.shared_post.as_ref(), hasher);
        Self::hash_reactions(&message.reactions, hasher);
        hasher.integer(message.sparks_total);
        Self::hash_reply_markup(message.reply_markup.as_ref(), hasher);

        let shows_comments_footer = chat.r#type == ChatType::Channel
            && message.discussion_chat_id.as_ref().is_some_and(|discussion| !discussion.is_empty());
        hasher.boolean(shows_comments_footer);
        if shows_comments_footer {
            hasher.optional_integer(message.comments_count);
            hasher.integer(message.recent_commenters.len().min(3) as i64);
        }
    }

    fn hash_media_item(media_item: &MessageMediaItem, hasher: &mut RiseMessageStableGeometryHasher) {
        hasher.string(media_item.kind.raw_value());
        hasher.optional_string(media_item.sticker_kind.map(|kind| kind.raw_value()));
        hasher.floating(f64::from(media_item.width));
        hasher.floating(f64::from(media_item.height));
        hasher.optional_floating(media_item.duration);
        hasher.optional_string(media_item.file_name.as_deref());
        hasher.optional_integer64(media_item.file_size);
        let normalized_thumbnail_url =
            media_item.thumbnail_url.as_deref().map(trimming_whitespaces_and_newlines);
        hasher.boolean(normalized_thumbnail_url.is_some_and(|url| !url.is_empty()));
        hasher.boolean(media_item.is_cancelled);
    }

    fn hash_reply(reply: Option<&MessageReplyTo>, hasher: &mut RiseMessageStableGeometryHasher) {
        hasher.boolean(reply.is_some());
        let Some(reply) = reply else { return };
        hasher.string(&reply.sender_name);
        hasher.string(reply.content_type.raw_value());
        hasher.string(&reply.content);
        let has_thumbnail = reply.media_items.iter().any(|item| match item.kind {
            MessageMediaKind::Image | MessageMediaKind::Video | MessageMediaKind::Animation => {
                !item.display_source().is_empty()
            }
            _ => false,
        });
        hasher.boolean(has_thumbnail);
    }

    fn hash_story_reply(story_reply: Option<&MessageStoryReply>, hasher: &mut RiseMessageStableGeometryHasher) {
        hasher.boolean(story_reply.is_some());
        let Some(story_reply) = story_reply else { return };
        hasher.integer(story_reply.story_id);
        hasher.string(&story_reply.author.name);
    }

    fn hash_forward(forward: Option<&MessageForwardInfo>, hasher: &mut RiseMessageStableGeometryHasher) {
        hasher.boolean(forward.is_some());
        let Some(forward) = forward else { return };
        hasher.string(forward.from_chat_title.as_deref().unwrap_or(&forward.sender_name));
        hasher.boolean(
            forward.from_chat_logo.as_ref().or(forward.sender_logo.as_ref()).is_some_and(|logo| !logo.is_empty()),
        );
        hasher.boolean(forward.from_chat_id.as_ref().is_some_and(|id| !id.is_empty()));
        hasher.boolean(forward.sender_id.as_ref().is_some_and(|id| !id.is_empty()));
        hasher.boolean(forward.sender_tag.as_ref().is_some_and(|tag| !tag.is_empty()));
    }

    fn hash_link_preview(link_preview: Option<&MessageLinkPreview>, hasher: &mut RiseMessageStableGeometryHasher) {
        hasher.boolean(link_preview.is_some());
        let Some(link_preview) = link_preview else { return };
        hasher.string(link_preview.size.raw_value());
        hasher.boolean(link_preview.is_above_text);
        hasher.optional_string(link_preview.site_name.as_deref());
        hasher.optional_string(link_preview.title.as_deref());
        hasher.optional_string(link_preview.description.as_deref());
        hasher.boolean(link_preview.image_url.as_ref().is_some_and(|url| !url.is_empty()));
        hasher.floating(f64::from(link_preview.image_aspect));
    }

    fn hash_contact(contact: Option<&MessageContactInfo>, hasher: &mut RiseMessageStableGeometryHasher) {
        hasher.boolean(contact.is_some());
        let Some(contact) = contact else { return };
        hasher.string(&contact.name);
        hasher.string(&contact.phone);
    }

    fn hash_location(location: Option<&MessageLocationInfo>, hasher: &mut RiseMessageStableGeometryHasher) {
        hasher.boolean(location.is_some());
        let Some(location) = location else { return };
        hasher.optional_string(location.title.as_deref());
        hasher.optional_string(location.address.as_deref());
        hasher.boolean(location.live());
    }

    fn hash_poll(poll: Option<&MessagePollInfo>, hasher: &mut RiseMessageStableGeometryHasher) {
        hasher.boolean(poll.is_some());
        let Some(poll) = poll else { return };
        hasher.string(&poll.question);
        hasher.integer(poll.options.len() as i64);
        for option in &poll.options {
            hasher.string(&option.text);
            hasher.integer(option.voter_count);
        }
        hasher.integer(poll.total_voters);
        hasher.boolean(poll.is_anonymous);
        hasher.boolean(poll.allows_multiple);
        hasher.boolean(poll.closed);
    }

    fn hash_shared_post(shared_post: Option<&MessageSharedPost>, hasher: &mut RiseMessageStableGeometryHasher) {
        hasher.boolean(shared_post.is_some());
        let Some(shared_post) = shared_post else { return };
        hasher.boolean(shared_post.is_short);
        hasher.string(&shared_post.title);
        hasher.string(&shared_post.snippet);
        hasher.boolean(shared_post.display_thumbnail().is_some_and(|thumbnail| !thumbnail.is_empty()));
        hasher.integer(shared_post.likes_count);
        hasher.integer(shared_post.comments_count);
    }

    fn hash_reactions(reactions: &[MessageReaction], hasher: &mut RiseMessageStableGeometryHasher) {
        hasher.integer(reactions.len() as i64);
        for reaction in reactions {
            hasher.string(&reaction.emoji);
            hasher.integer(reaction.count);
            hasher.integer(reaction.reactors.len().min(3) as i64);
        }
    }

    fn hash_reply_markup(reply_markup: Option<&MessageBotReplyMarkup>, hasher: &mut RiseMessageStableGeometryHasher) {
        let rows = reply_markup
            .map(|markup| &markup.inline_keyboard)
            .filter(|rows| rows.iter().any(|row| !row.is_empty()));
        let Some(rows) = rows else {
            hasher.boolean(false);
            return;
        };
        hasher.boolean(true);
        hasher.integer(rows.len() as i64);
        for row in rows {
            hasher.integer(row.len() as i64);
            for button in row {
                hasher.string(&button.text);
                hasher.boolean(button.web_app_url.is_some());
                hasher.boolean(button.url.is_some());
                hasher.boolean(button.switch_inline_query.is_some());
                hasher.boolean(button.switch_inline_query_current_chat.is_some());
                hasher.boolean(button.is_pay);
                hasher.boolean(button.is_game);
            }
        }
    }

    fn kind_code(kind: ChatMessageRenderEntryKind) -> i64 {
        match kind {
            ChatMessageRenderEntryKind::DateSeparator => 0,
            ChatMessageRenderEntryKind::TopicSeparator => 1,
            ChatMessageRenderEntryKind::System => 2,
            ChatMessageRenderEntryKind::Message => 3,
            ChatMessageRenderEntryKind::UnreadSeparator => 4,
        }
    }

    fn delivery_status_code(status: MessageDeliveryStatus) -> i64 {
        match status {
            MessageDeliveryStatus::Sending => 0,
            MessageDeliveryStatus::Failed => 1,
            MessageDeliveryStatus::Sent => 2,
            MessageDeliveryStatus::Delivered => 3,
            MessageDeliveryStatus::Read => 4,
        }
    }
}

impl ChatMessageRenderEntry {
    pub fn fallback_geometry_version(
        &self,
        chat: &ChatInfo,
        calendar: &Calendar,
        time_zone: &TimeZone,
        context: Option<&mut RiseMessageFallbackGeometryContext>,
        cx: &mut App,
    ) -> RiseMessageGeometryVersion {
        RiseMessageFallbackGeometry::version(self, chat, calendar, time_zone, context, cx)
    }
}

/// no dynamic type field here, desktop has none and its only text size control is the slider
/// layout_metrics_bucket carries
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseMessageFallbackHeightCacheKey {
    pub geometry_version: RiseMessageGeometryVersion,
    pub effective_bubble_width_bucket: i64,
    pub display_scale_bucket: i64,
    pub layout_metrics_bucket: i64,
    pub locale_identifier: String,
    pub layout_direction: LayoutDirection,
}

#[derive(Clone)]
struct RiseMessageFallbackHeightCacheRecord {
    height: f32,
    previous: Option<RiseMessageFallbackHeightCacheKey>,
    next: Option<RiseMessageFallbackHeightCacheKey>,
}

pub struct RiseMessageFallbackHeightCache {
    pub capacity: i64,
    count: i64,

    records: HashMap<RiseMessageFallbackHeightCacheKey, RiseMessageFallbackHeightCacheRecord>,
    least_recent_key: Option<RiseMessageFallbackHeightCacheKey>,
    most_recent_key: Option<RiseMessageFallbackHeightCacheKey>,
}

impl RiseMessageFallbackHeightCache {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_: &mut Context<Self>| Self::new(Self::default_capacity()))
    }

    pub fn default_capacity() -> i64 {
        RiseLimits::client_value("message_fallback_height_cache_entries")
    }

    /// default is default_capacity
    pub fn new(capacity: i64) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            count: 0,
            records: HashMap::with_capacity(usize::try_from(capacity).unwrap_or(0)),
            least_recent_key: None,
            most_recent_key: None,
        }
    }

    pub fn count(&self) -> i64 {
        self.count
    }

    #[allow(clippy::too_many_arguments)]
    pub fn make_key(
        entry: &ChatMessageRenderEntry,
        chat: &ChatInfo,
        viewport_width: f32,
        locale: &Locale,
        calendar: &Calendar,
        time_zone: &TimeZone,
        layout_direction: LayoutDirection,
        display_scale: f32,
        context: Option<&mut RiseMessageFallbackGeometryContext>,
        cx: &mut App,
    ) -> RiseMessageFallbackHeightCacheKey {
        let (width, scale) =
            RiseMessageFallbackGeometry::effective_bubble_width_bucket(viewport_width, entry, chat, display_scale, cx);
        RiseMessageFallbackHeightCacheKey {
            geometry_version: entry.fallback_geometry_version(chat, calendar, time_zone, context, cx),
            effective_bubble_width_bucket: width,
            display_scale_bucket: scale,
            layout_metrics_bucket: Self::layout_metrics_bucket(viewport_width, ThemeStore::text_delta_snapshot()),
            locale_identifier: locale.identifier().to_owned(),
            layout_direction,
        }
    }

    /// wide layout bit plus the text size slider in half points
    /// a height measured at one text size must never be served at another
    pub fn layout_metrics_bucket(viewport_width: f32, text_delta: f32) -> i64 {
        let wide = i64::from(viewport_width > 680.0);
        let half_points = (text_delta * 2.0).round() as i64 + 64;
        wide | (half_points << 1)
    }

    pub fn height(&mut self, key: &RiseMessageFallbackHeightCacheKey) -> Option<f32> {
        let record = self.records.get(key)?.clone();
        let height = record.height;
        self.move_to_most_recent(key, record);
        Some(height)
    }

    pub fn insert(&mut self, height: f32, key: &RiseMessageFallbackHeightCacheKey) {
        if !(height.is_finite() && height > 0.0) {
            return;
        }
        if let Some(existing) = self.records.get_mut(key) {
            existing.height = height;
            let record = existing.clone();
            self.move_to_most_recent(key, record);
        } else {
            self.records.insert(
                key.clone(),
                RiseMessageFallbackHeightCacheRecord { height, previous: self.most_recent_key.clone(), next: None },
            );
            if let Some(most_recent_key) = &self.most_recent_key {
                if let Some(most_recent) = self.records.get_mut(most_recent_key) {
                    most_recent.next = Some(key.clone());
                }
            } else {
                self.least_recent_key = Some(key.clone());
            }
            self.most_recent_key = Some(key.clone());
            if self.records.len() as i64 > self.capacity {
                self.evict_least_recently_used();
            }
        }
        self.count = self.records.len() as i64;
    }

    pub fn remove_all(&mut self) {
        self.records.clear();
        self.least_recent_key = None;
        self.most_recent_key = None;
        self.count = 0;
    }

    fn move_to_most_recent(
        &mut self,
        key: &RiseMessageFallbackHeightCacheKey,
        record: RiseMessageFallbackHeightCacheRecord,
    ) {
        if self.most_recent_key.as_ref() == Some(key) {
            return;
        }
        if let Some(previous) = &record.previous {
            if let Some(previous_record) = self.records.get_mut(previous) {
                previous_record.next = record.next.clone();
            }
        } else {
            self.least_recent_key = record.next.clone();
        }
        if let Some(next) = &record.next
            && let Some(next_record) = self.records.get_mut(next)
        {
            next_record.previous = record.previous.clone();
        }
        if let Some(most_recent_key) = &self.most_recent_key {
            if let Some(most_recent) = self.records.get_mut(most_recent_key) {
                most_recent.next = Some(key.clone());
            }
        } else {
            self.least_recent_key = Some(key.clone());
        }
        let mut updated = record;
        updated.previous = self.most_recent_key.clone();
        updated.next = None;
        self.records.insert(key.clone(), updated);
        self.most_recent_key = Some(key.clone());
    }

    fn evict_least_recently_used(&mut self) {
        let Some(key) = self.least_recent_key.clone() else { return };
        let Some(record) = self.records.get(&key).cloned() else { return };
        self.least_recent_key = record.next.clone();
        if let Some(next) = &record.next {
            if let Some(next_record) = self.records.get_mut(next) {
                next_record.previous = None;
            }
        } else {
            self.most_recent_key = None;
        }
        self.records.remove(&key);
    }
}

struct RiseMessageStableGeometryHasher {
    state: u64,
}

impl RiseMessageStableGeometryHasher {
    fn new() -> Self {
        Self { state: 14_695_981_039_346_656_037 }
    }

    fn boolean(&mut self, value: bool) {
        self.byte(1);
        self.byte(u8::from(value));
    }

    fn integer(&mut self, value: i64) {
        self.byte(2);
        self.bytes(value as u64);
    }

    fn optional_integer(&mut self, value: Option<i64>) {
        self.boolean(value.is_some());
        if let Some(value) = value {
            self.integer(value);
        }
    }

    fn integer64(&mut self, value: i64) {
        self.byte(3);
        self.bytes(value as u64);
    }

    fn optional_integer64(&mut self, value: Option<i64>) {
        self.boolean(value.is_some());
        if let Some(value) = value {
            self.integer64(value);
        }
    }

    fn floating(&mut self, value: f64) {
        self.byte(4);
        let normalized = if value == 0.0 { 0.0 } else { value };
        self.bytes(normalized.to_bits());
    }

    fn optional_floating(&mut self, value: Option<f64>) {
        self.boolean(value.is_some());
        if let Some(value) = value {
            self.floating(value);
        }
    }

    fn string(&mut self, value: &str) {
        self.byte(5);
        self.bytes(value.len() as u64);
        for value in value.bytes() {
            self.byte(value);
        }
    }

    fn optional_string(&mut self, value: Option<&str>) {
        self.boolean(value.is_some());
        if let Some(value) = value {
            self.string(value);
        }
    }

    fn finalize(&self) -> u64 {
        let mut result = self.state;
        result ^= result >> 33;
        result = result.wrapping_mul(0xff51_afd7_ed55_8ccd);
        result ^= result >> 33;
        result = result.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        result ^= result >> 33;
        result
    }

    fn bytes(&mut self, value: u64) {
        let mut value = value;
        for _ in 0..8 {
            self.byte(value as u8);
            value >>= 8;
        }
    }

    fn byte(&mut self, value: u8) {
        self.state ^= u64::from(value);
        self.state = self.state.wrapping_mul(1_099_511_628_211);
    }
}
