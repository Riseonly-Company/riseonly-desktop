use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FontWeight, Hsla, IntoElement, RenderOnce, SharedString, Subscription,
    Svg, Window,
};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_range::{NSRange, length_utf16, utf16_to_byte_range};
use rise_i18n::language_store::Locale;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_theme::theme::theme_environment::theme_store;
use rise_ui::IconUi;
use rise_ui::bot_icon::BotIcon;
use rise_ui::main_text::MainText;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::environment::LayoutDirection;
use rise_ui::swiftui::transition::{Presence, Transition};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::formatted_preview_text::FormattedPreviewText;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::widgets::sheets::share_sheet::ShareTagPolicy;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::chat_typing_indicator::ChatTypingIndicator;
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatDraftPreview;
use crate::modules::chat::components::chat_item::chat_item::ChatListTimestampFormatter;
use crate::modules::chat::components::chat_item::chat_list_row_activity::{ChatListRowActivity, ChatListRowActivityKey};
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::messages::rise_message_search_session::RiseMessageSearchHit;
use crate::modules::chat::pages::chats::embedded_forum_transition::{
    EmbeddedForumRailContentMotion, EmbeddedForumRailMetrics,
};
use crate::modules::chat::shared::utils::message_preview::MessagePreview;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatTopic, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageDeliveryStatus, MessageMediaKind, RemoteMediaURL,
};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::user::stores::user::user_actions::user_contact_wire_types::SavedContactDtoExt;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const BADGE_SPRING_RESPONSE: f64 = 0.3;
const BADGE_SPRING_DAMPING_FRACTION: f64 = 0.85;
const MARK_BADGE_TRANSITION_SCALE: f32 = 0.4;
const ROW_MIN_HEIGHT: f32 = 70.0;
const SNIPPET_THUMBNAIL_SIDE: f32 = 17.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatListPreviewPresentationTitleAccessory {
    Bot,
    Official,
    Premium { user_id: String, emoji: Option<String> },
    Tag(String),
    SystemImage(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChatListPreviewPresentationDeliveryState {
    #[default]
    None,
    Sending,
    Failed,
    Sent,
    Read,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatListPreviewPresentation {
    pub title: String,
    pub sender: Option<String>,
    pub snippet: Option<String>,
    pub snippet_markup: Option<String>,
    pub draft: Option<String>,
    pub media_thumbnail_source: Option<String>,
    pub timestamp: TimeInterval,
    pub unread_count: i64,
    pub unread_mention_count: i64,
    pub unread_reply_count: i64,
    pub unread_reaction_count: i64,
    pub is_muted: bool,
    pub is_pinned: bool,
    pub is_verified: bool,
    pub is_closed: bool,
    pub is_private: bool,
    pub delivery_state: ChatListPreviewPresentationDeliveryState,
    pub title_accessories: Vec<ChatListPreviewPresentationTitleAccessory>,
    pub shows_metadata: bool,
    pub snippet_highlight_ranges: Vec<NSRange>,
}

impl Default for ChatListPreviewPresentation {
    fn default() -> Self {
        Self {
            title: String::new(),
            sender: None,
            snippet: None,
            snippet_markup: None,
            draft: None,
            media_thumbnail_source: None,
            timestamp: 0.0,
            unread_count: 0,
            unread_mention_count: 0,
            unread_reply_count: 0,
            unread_reaction_count: 0,
            is_muted: false,
            is_pinned: false,
            is_verified: false,
            is_closed: false,
            is_private: false,
            delivery_state: ChatListPreviewPresentationDeliveryState::None,
            title_accessories: Vec::new(),
            shows_metadata: true,
            snippet_highlight_ranges: Vec::new(),
        }
    }
}

pub enum ChatListPreviewFactory {}

impl ChatListPreviewFactory {
    pub fn chat(
        chat: &ChatInfo,
        draft: Option<&ChatDraftPreview>,
        shows_metadata: bool,
        cx: &mut App,
    ) -> ChatListPreviewPresentation {
        let message = chat.last_message.as_ref();
        let draft_text = Self::draft_text(draft, &format!("chat:{}", chat.id), cx);
        let message_timestamp = message.map_or(0.0, |message| message.created_at);
        let shows_sender = Self::shows_sender(chat);
        let has_draft = draft_text.is_some();
        ChatListPreviewPresentation {
            title: Self::row_title(chat, cx),
            sender: if has_draft { None } else { Self::sender_label(message, shows_sender, cx) },
            snippet: Some(match &draft_text {
                Some(text) => text.clone(),
                None => Self::preview_snippet(message, cx),
            }),
            snippet_markup: if has_draft { None } else { message.and_then(|message| MessagePreview::list_markup(message, cx)) },
            draft: draft_text,
            media_thumbnail_source: if has_draft { None } else { Self::media_thumbnail_source(message) },
            timestamp: Self::timestamp(has_draft, draft, message_timestamp),
            unread_count: chat.unread_count,
            unread_mention_count: chat.unread_mentions_count,
            unread_reply_count: chat.unread_replies_count,
            unread_reaction_count: chat.unread_reactions_count,
            is_muted: chat.is_muted,
            is_pinned: chat.is_pinned,
            is_verified: chat.is_verified,
            delivery_state: if has_draft {
                ChatListPreviewPresentationDeliveryState::None
            } else {
                Self::delivery_state(message, cx)
            },
            title_accessories: Self::title_accessories(chat),
            shows_metadata,
            ..ChatListPreviewPresentation::default()
        }
    }

    pub fn search_hit(hit: &RiseMessageSearchHit, chat: Option<&ChatInfo>, cx: &mut App) -> ChatListPreviewPresentation {
        let message = &hit.message;
        let author = Self::sender_label(Some(message), true, cx).unwrap_or_else(|| message.sender_id.clone());
        let uses_server_snippet = !hit.snippet.is_empty() && !message.is_system();
        ChatListPreviewPresentation {
            title: chat.map_or(author, |chat| Self::chat_title(chat, cx)),
            sender: chat.and_then(|chat| Self::sender_label(Some(message), Self::shows_sender(chat), cx)),
            snippet: Some(if uses_server_snippet {
                hit.snippet.clone()
            } else {
                Self::preview_snippet(Some(message), cx)
            }),
            media_thumbnail_source: Self::media_thumbnail_source(Some(message)),
            timestamp: message.created_at,
            is_verified: chat.is_some_and(|chat| chat.is_verified),
            delivery_state: Self::delivery_state(Some(message), cx),
            title_accessories: chat.map(Self::title_accessories).unwrap_or_default(),
            snippet_highlight_ranges: if uses_server_snippet { hit.snippet_ranges.clone() } else { Vec::new() },
            ..ChatListPreviewPresentation::default()
        }
    }

    pub fn topic(
        topic: &ChatTopic,
        last_message: Option<&GetMessageMessage>,
        unread_count: i64,
        is_muted: bool,
        updated_at: TimeInterval,
        draft: Option<&ChatDraftPreview>,
        cx: &mut App,
    ) -> ChatListPreviewPresentation {
        let draft_text = Self::draft_text(draft, &format!("topic:{}", topic.id), cx);
        let has_draft = draft_text.is_some();
        let message_timestamp = last_message.map_or(updated_at, |message| message.created_at);
        ChatListPreviewPresentation {
            title: topic.title.clone(),
            sender: if has_draft { None } else { Self::sender_label(last_message, true, cx) },
            snippet: Some(match &draft_text {
                Some(text) => text.clone(),
                None => Self::preview_snippet(last_message, cx),
            }),
            snippet_markup: if has_draft {
                None
            } else {
                last_message.and_then(|message| MessagePreview::list_markup(message, cx))
            },
            draft: draft_text,
            media_thumbnail_source: if has_draft { None } else { Self::media_thumbnail_source(last_message) },
            timestamp: Self::timestamp(has_draft, draft, message_timestamp),
            unread_count,
            unread_mention_count: topic.unread_mentions_count,
            unread_reply_count: topic.unread_replies_count,
            unread_reaction_count: topic.unread_reactions_count,
            is_muted,
            is_closed: topic.is_closed,
            is_private: topic.is_private,
            delivery_state: if has_draft {
                ChatListPreviewPresentationDeliveryState::None
            } else {
                Self::delivery_state(last_message, cx)
            },
            ..ChatListPreviewPresentation::default()
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn synthetic_topic(
        title: &str,
        last_message: Option<&GetMessageMessage>,
        unread_count: i64,
        unread_mention_count: i64,
        unread_reply_count: i64,
        unread_reaction_count: i64,
        is_muted: bool,
        updated_at: TimeInterval,
        draft: Option<&ChatDraftPreview>,
        cx: &mut App,
    ) -> ChatListPreviewPresentation {
        let draft_text = Self::draft_text(draft, &format!("synthetic:{title}"), cx);
        let has_draft = draft_text.is_some();
        let message_timestamp = last_message.map_or(updated_at, |message| message.created_at);
        ChatListPreviewPresentation {
            title: title.to_owned(),
            sender: if has_draft { None } else { Self::sender_label(last_message, true, cx) },
            snippet: Some(match &draft_text {
                Some(text) => text.clone(),
                None => Self::preview_snippet(last_message, cx),
            }),
            snippet_markup: if has_draft {
                None
            } else {
                last_message.and_then(|message| MessagePreview::list_markup(message, cx))
            },
            draft: draft_text,
            media_thumbnail_source: if has_draft { None } else { Self::media_thumbnail_source(last_message) },
            timestamp: Self::timestamp(has_draft, draft, message_timestamp),
            unread_count,
            unread_mention_count,
            unread_reply_count,
            unread_reaction_count,
            is_muted,
            delivery_state: if has_draft {
                ChatListPreviewPresentationDeliveryState::None
            } else {
                Self::delivery_state(last_message, cx)
            },
            ..ChatListPreviewPresentation::default()
        }
    }

    /// draft text for the row, none also for a draft that compacts to nothing
    fn draft_text(draft: Option<&ChatDraftPreview>, scope: &str, cx: &mut App) -> Option<String> {
        let draft = draft?;
        let text = MessagePreview::list_draft(&draft.text, scope, cx);
        if text.is_empty() { None } else { Some(text) }
    }

    fn timestamp(has_draft: bool, draft: Option<&ChatDraftPreview>, message_timestamp: TimeInterval) -> TimeInterval {
        if has_draft {
            draft.map_or(0.0, |draft| draft.updated_at).max(message_timestamp)
        } else {
            message_timestamp
        }
    }

    /// only a group has several authors
    /// saved messages are all the viewers own so a You prefix is noise
    /// and pushes the snippet to a second line
    fn shows_sender(chat: &ChatInfo) -> bool {
        chat.r#type == ChatType::Group
    }

    fn sender_label(message: Option<&GetMessageMessage>, shows_sender: bool, cx: &mut App) -> Option<String> {
        if !shows_sender {
            return None;
        }
        let message = message?;
        if message.content_type == MessageContentType::System {
            return None;
        }
        if AuthServicesStore::shared(cx).read(cx).is_current_user(&message.sender_id) {
            return Some(tr("chat_preview_you"));
        }
        if Self::hides_anonymous_author(message) {
            return None;
        }
        let name = Self::contact_name(
            Some(message.sender_id.as_str()),
            trimming_whitespaces_and_newlines(&message.sender_name),
            cx,
        );
        if name.is_empty() { None } else { Some(name) }
    }

    fn hides_anonymous_author(message: &GetMessageMessage) -> bool {
        let signed = message
            .author_signature
            .as_deref()
            .is_some_and(|signature| !trimming_whitespaces_and_newlines(signature).is_empty());
        signed && message.sender_id != message.chat_id
    }

    /// name the viewer saved this person under, as telegram prints it everywhere in the list
    pub fn contact_name(user_id: Option<&str>, fallback: &str, cx: &mut App) -> String {
        let Some(user_id) = user_id.filter(|user_id| !user_id.is_empty()) else {
            return fallback.to_owned();
        };
        let alias = Self::saved_contact_alias(user_id, cx);
        if alias.is_empty() { fallback.to_owned() } else { alias }
    }

    fn saved_contact_alias(user_id: &str, cx: &mut App) -> String {
        UserServicesStore::observed_saved_contact(user_id, cx)
            .map(|contact| trimming_whitespaces_and_newlines(&contact.resolved_display_name()).to_owned())
            .unwrap_or_default()
    }

    fn preview_snippet(message: Option<&GetMessageMessage>, cx: &mut App) -> String {
        let Some(message) = message else {
            return tr("chat_preview_empty");
        };
        let value = MessagePreview::list_snippet(message, cx);
        if value.is_empty() { tr("msg_reply_generic") } else { value }
    }

    /// internal not private, the chat list prewarm must request exactly the url the row will draw
    /// else it warms the wrong cache entry
    pub fn media_thumbnail_source(message: Option<&GetMessageMessage>) -> Option<String> {
        let item = message?.first_grid_media_item()?;
        if item.kind == MessageMediaKind::Video
            && RemoteMediaURL::optional_normalized(item.thumbnail_url.as_deref()).is_none()
            && item.resolved_local_preview_url().is_none()
        {
            return None;
        }
        let display_source = item.display_source();
        let source = trimming_whitespaces_and_newlines(&display_source);
        if source.is_empty() { None } else { Some(source.to_owned()) }
    }

    /// a list row always has a title, a chat whose card came without a name reads as untitled
    /// like the archive summary row
    pub fn row_title(chat: &ChatInfo, cx: &mut App) -> String {
        let title = Self::chat_title(chat, cx);
        if trimming_whitespaces_and_newlines(&title).is_empty() { tr("chat_untitled") } else { title }
    }

    pub fn chat_title(chat: &ChatInfo, cx: &mut App) -> String {
        match chat.r#type {
            ChatType::Favourites => tr("chats_favourites"),
            ChatType::Direct => {
                if chat.participant.is_deleted {
                    return tr("chat_deleted_account");
                }
                let alias = Self::saved_contact_alias(&chat.participant.id, cx);
                if !alias.is_empty() {
                    return alias;
                }
                chat.direct_peer_profile_name()
            }
            ChatType::Group | ChatType::Channel => chat.title.clone(),
        }
    }

    fn title_accessories(chat: &ChatInfo) -> Vec<ChatListPreviewPresentationTitleAccessory> {
        let mut accessories = Vec::new();
        if chat.r#type == ChatType::Direct {
            if chat.participant.is_bot {
                accessories.push(ChatListPreviewPresentationTitleAccessory::Bot);
            }
            if chat.participant.more.is_official {
                accessories.push(ChatListPreviewPresentationTitleAccessory::Official);
            }
            if chat.is_official_bot_chat() {
                if let Some(tag) = ShareTagPolicy::normalized(&chat.participant.tag) {
                    accessories.push(ChatListPreviewPresentationTitleAccessory::Tag(tag));
                }
            }
            if chat.participant.more.is_premium {
                accessories.push(ChatListPreviewPresentationTitleAccessory::Premium {
                    user_id: chat.participant.id.clone(),
                    emoji: chat.participant.more.premium_badge_emoji.clone(),
                });
            }
        }
        match chat.r#type {
            ChatType::Group => {
                let symbol = if chat.topics_enabled { "person.3.fill" } else { "person.2.fill" };
                accessories.push(ChatListPreviewPresentationTitleAccessory::SystemImage(symbol.to_owned()));
            }
            ChatType::Channel => {
                accessories.push(ChatListPreviewPresentationTitleAccessory::SystemImage("megaphone.fill".to_owned()));
            }
            ChatType::Favourites => {
                accessories.push(ChatListPreviewPresentationTitleAccessory::SystemImage("bookmark.fill".to_owned()));
            }
            ChatType::Direct => {}
        }
        accessories
    }

    fn delivery_state(message: Option<&GetMessageMessage>, cx: &mut App) -> ChatListPreviewPresentationDeliveryState {
        let Some(message) = message else {
            return ChatListPreviewPresentationDeliveryState::None;
        };
        if message.content_type == MessageContentType::System
            || !AuthServicesStore::shared(cx).read(cx).is_current_user(&message.sender_id)
        {
            return ChatListPreviewPresentationDeliveryState::None;
        }
        if message.is_read || message.delivery_status == MessageDeliveryStatus::Read {
            return ChatListPreviewPresentationDeliveryState::Read;
        }
        match message.delivery_status {
            MessageDeliveryStatus::Sending => ChatListPreviewPresentationDeliveryState::Sending,
            MessageDeliveryStatus::Failed => ChatListPreviewPresentationDeliveryState::Failed,
            MessageDeliveryStatus::Delivered => ChatListPreviewPresentationDeliveryState::Sent,
            MessageDeliveryStatus::Sent => ChatListPreviewPresentationDeliveryState::Sent,
            MessageDeliveryStatus::Read => ChatListPreviewPresentationDeliveryState::Read,
        }
    }
}

pub enum ChatListRowMetrics {}

impl ChatListRowMetrics {
    pub const HORIZONTAL_PADDING: f32 = 12.0;
    pub const LEADING_SPACING: f32 = 12.0;
    pub const AVATAR_SIZE: f32 = 55.0;
    pub const THUMBNAIL_DECODE_PIXEL_DIMENSION: i64 = 48;
    pub const SEPARATOR_HEIGHT: f32 = 0.5;
}

/// conditional reply, mention and heart discs of one row with their shared transition
struct ChatListPreviewRowMarkBadges {
    reaction: Presence<()>,
    mention: Presence<()>,
    reply: Presence<()>,
}

impl ChatListPreviewRowMarkBadges {
    fn new(has_reaction: bool, has_mention: bool, has_reply: bool) -> Self {
        Self {
            reaction: Self::presence(has_reaction),
            mention: Self::presence(has_mention),
            reply: Self::presence(has_reply),
        }
    }

    fn presence(present: bool) -> Presence<()> {
        Presence::new(present.then_some(())).with_transition(
            Transition::scale(MARK_BADGE_TRANSITION_SCALE).combined(Transition::opacity()),
        )
    }

    fn sync(&mut self, has_reaction: bool, has_mention: bool, has_reply: bool, cx: &mut Context<Self>) {
        let spec: MotionSpec = spring(BADGE_SPRING_RESPONSE, BADGE_SPRING_DAMPING_FRACTION);
        let now = RiseClock::monotonic();
        if self.reaction.is_present() != has_reaction {
            self.reaction.set_in(has_reaction.then_some(()), spec, now, cx);
        }
        if self.mention.is_present() != has_mention {
            self.mention.set_in(has_mention.then_some(()), spec, now, cx);
        }
        if self.reply.is_present() != has_reply {
            self.reply.set_in(has_reply.then_some(()), spec, now, cx);
        }
    }
}

#[derive(IntoElement)]
pub struct ChatListPreviewRow {
    id: ElementId,
    presentation: ChatListPreviewPresentation,
    horizontal_padding: f32,
    vertical_padding: f32,
    leading_spacing: f32,
    rail_avatar_size: Option<f32>,
    measurement_only: bool,
    typing_chat_id: Option<String>,
    typing_topic_id: Option<String>,
    typing_shows_names: bool,
    is_active: bool,
    leading: Option<AnyElement>,
    trailing: Option<AnyElement>,
}

impl ChatListPreviewRow {
    pub fn new(id: impl Into<ElementId>, presentation: ChatListPreviewPresentation) -> Self {
        Self {
            id: id.into(),
            presentation,
            horizontal_padding: ChatListRowMetrics::HORIZONTAL_PADDING,
            vertical_padding: 10.0,
            leading_spacing: ChatListRowMetrics::LEADING_SPACING,
            rail_avatar_size: None,
            measurement_only: false,
            typing_chat_id: None,
            typing_topic_id: None,
            typing_shows_names: false,
            is_active: true,
            leading: None,
            trailing: None,
        }
    }

    pub fn horizontal_padding(mut self, horizontal_padding: f32) -> Self {
        self.horizontal_padding = horizontal_padding;
        self
    }

    pub fn vertical_padding(mut self, vertical_padding: f32) -> Self {
        self.vertical_padding = vertical_padding;
        self
    }

    pub fn leading_spacing(mut self, leading_spacing: f32) -> Self {
        self.leading_spacing = leading_spacing;
        self
    }

    pub fn rail_avatar_size(mut self, rail_avatar_size: impl Into<Option<f32>>) -> Self {
        self.rail_avatar_size = rail_avatar_size.into();
        self
    }

    pub fn measurement_only(mut self, measurement_only: bool) -> Self {
        self.measurement_only = measurement_only;
        self
    }

    pub fn typing_chat_id(mut self, typing_chat_id: impl Into<Option<String>>) -> Self {
        self.typing_chat_id = typing_chat_id.into();
        self
    }

    pub fn typing_topic_id(mut self, typing_topic_id: impl Into<Option<String>>) -> Self {
        self.typing_topic_id = typing_topic_id.into();
        self
    }

    pub fn typing_shows_names(mut self, typing_shows_names: bool) -> Self {
        self.typing_shows_names = typing_shows_names;
        self
    }

    pub fn is_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }

    pub fn leading(mut self, leading: impl IntoElement) -> Self {
        self.leading = Some(leading.into_any_element());
        self
    }

    pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
        self.trailing = Some(trailing.into_any_element());
        self
    }

    fn child_id(&self, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(suffix))
    }

    fn highlighted_snippet(&self, snippet: &str, theme: &AppTheme) -> Vec<(Range<usize>, Hsla)> {
        let length = length_utf16(snippet);
        self.presentation
            .snippet_highlight_ranges
            .iter()
            .filter(|range| range.location.checked_add(range.length).is_some_and(|end| end <= length))
            .filter_map(|range| utf16_to_byte_range(snippet, *range))
            .map(|range| (range, theme.text.primary))
            .collect()
    }

    fn snippet_text(&self, snippet: &str, theme: &AppTheme) -> AnyElement {
        let presentation = &self.presentation;
        if presentation.snippet_highlight_ranges.is_empty()
            && let Some(markup) = presentation.snippet_markup.as_ref()
        {
            return FormattedPreviewText::new(
                self.child_id("snippet-markup"),
                markup.clone(),
                13.0,
                theme.text.secondary,
                if presentation.sender.is_none() && presentation.draft.is_none() { 2 } else { 1 },
                None,
            )
            .into_any_element();
        }
        let text = MainText::new(snippet.to_owned()).px(13.0).color(theme.text.secondary);
        if presentation.snippet_highlight_ranges.is_empty() {
            text.number_of_lines(if presentation.sender.is_none() && presentation.draft.is_none() { 2 } else { 1 })
                .into_any_element()
        } else {
            text.number_of_lines(if presentation.sender.is_none() { 2 } else { 1 })
                .color_runs(self.highlighted_snippet(snippet, theme))
                .into_any_element()
        }
    }

    fn preview_content(&self, theme: &AppTheme) -> Div {
        let presentation = &self.presentation;
        let mut content = vstack(2.0).align(HorizontalAlignment::Leading).w_full();
        if presentation.draft.is_some() {
            content = content.child(
                div().w_full().child(
                    MainText::new(tr("chat_preview_draft"))
                        .px(13.0)
                        .font_weight(FontWeight::MEDIUM)
                        .color(theme.primary._100)
                        .number_of_lines(1),
                ),
            );
        } else if let Some(sender) = presentation.sender.as_ref().filter(|sender| !sender.is_empty()) {
            content = content.child(
                div().w_full().child(
                    MainText::new(sender.clone())
                        .px(13.0)
                        .font_weight(FontWeight::MEDIUM)
                        .color(theme.text.primary.opacity(0.92))
                        .number_of_lines(1),
                ),
            );
        }
        if let Some(snippet) = presentation.snippet.as_ref().filter(|snippet| !snippet.is_empty()) {
            let thumbnail = presentation
                .media_thumbnail_source
                .as_ref()
                .filter(|source| !source.is_empty())
                .map(|source| self.snippet_thumbnail(source));
            content = content.child(
                hstack(5.0)
                    .w_full()
                    .children(thumbnail)
                    .child(div().flex_1().min_w_0().child(self.snippet_text(snippet, theme))),
            );
        }
        content
    }

    fn snippet_thumbnail(&self, source: &str) -> Div {
        let frame = div().flex_none().frame_size(SNIPPET_THUMBNAIL_SIDE, SNIPPET_THUMBNAIL_SIDE);
        if self.measurement_only {
            return frame;
        }
        frame.overflow_hidden().child(
            CleverImageUi::new(source.to_owned())
                .id(self.child_id("thumbnail"))
                .content_mode(ContentMode::Fill)
                .corner_radius(3.0)
                .persist_to_disk(true)
                .shows_progress_indicator(false)
                .maximum_decode_pixel_dimension(ChatListRowMetrics::THUMBNAIL_DECODE_PIXEL_DIMENSION),
        )
    }

    fn title_line(&self, theme: &AppTheme) -> Div {
        let presentation = &self.presentation;
        let secondary = theme.text.secondary;
        let accessories: Vec<AnyElement> =
            presentation.title_accessories.iter().map(|accessory| Self::title_accessory(accessory, theme)).collect();
        hstack(4.0)
            .w_full()
            .child(
                div()
                    .min_w_0()
                    .child(MainText::new(presentation.title.clone()).px(16.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1)),
            )
            .children(accessories)
            .when(presentation.is_verified, |line| {
                line.children(symbol("checkmark.seal.fill", 12.0, theme.primary._100, true))
            })
            .when(presentation.is_pinned, |line| line.children(symbol("pin.fill", 11.0, secondary, true)))
            .when(presentation.is_muted, |line| line.children(symbol("bell.slash", 11.0, secondary, false)))
            .when(presentation.is_closed, |line| line.children(symbol("lock.fill", 11.0, secondary, true)))
            .when(!presentation.is_closed && presentation.is_private, |line| {
                line.children(symbol("lock.fill", 10.0, secondary.opacity(0.75), true))
            })
            .child(spacer_min_w(0.0))
    }

    fn title_accessory(accessory: &ChatListPreviewPresentationTitleAccessory, theme: &AppTheme) -> AnyElement {
        match accessory {
            ChatListPreviewPresentationTitleAccessory::Bot => {
                div().flex_none().child(BotIcon::new(theme.text.primary).size(12.0)).into_any_element()
            }
            ChatListPreviewPresentationTitleAccessory::Official => {
                div().flex_none().child(OfficialBadge::new().size(px(16.0))).into_any_element()
            }
            ChatListPreviewPresentationTitleAccessory::Premium { user_id, emoji } => div()
                .flex_none()
                .child(
                    PremiumBadge::new()
                        .user_id(Some(SharedString::from(user_id.clone())))
                        .emoji(emoji.clone().map(SharedString::from))
                        .size(16.0)
                        .animated(false),
                )
                .into_any_element(),
            ChatListPreviewPresentationTitleAccessory::Tag(tag) => div()
                .min_w_0()
                .child(MainText::new(tag.clone()).px(12.0).color(theme.text.secondary).number_of_lines(1))
                .into_any_element(),
            ChatListPreviewPresentationTitleAccessory::SystemImage(name) => div()
                .flex_none()
                .children(symbol(name, 11.0, theme.text.secondary, name.ends_with(".fill")))
                .into_any_element(),
        }
    }

    fn delivery_indicator(&self, theme: &AppTheme) -> Option<AnyElement> {
        match self.presentation.delivery_state {
            ChatListPreviewPresentationDeliveryState::None => None,
            ChatListPreviewPresentationDeliveryState::Sending => {
                symbol("clock", 11.0, theme.text.secondary, false).map(IntoElement::into_any_element)
            }
            ChatListPreviewPresentationDeliveryState::Failed => {
                symbol("exclamationmark.circle.fill", 12.0, theme.semantic.error_100, true)
                    .map(IntoElement::into_any_element)
            }
            ChatListPreviewPresentationDeliveryState::Sent => {
                symbol("checkmark", 13.0, theme.text.secondary, false).map(IntoElement::into_any_element)
            }
            ChatListPreviewPresentationDeliveryState::Read => {
                let check = |offset: f32| {
                    div().relative().left(px(offset)).children(symbol("checkmark", 13.0, theme.primary._100, false))
                };
                Some(
                    zstack(Alignment::Center)
                        .flex_none()
                        .frame_size(18.0, None::<f32>)
                        .child(check(-3.0))
                        .child(check(2.0))
                        .into_any_element(),
                )
            }
        }
    }

    fn badge_color(&self, theme: &AppTheme) -> Hsla {
        if self.presentation.is_muted { theme.text.secondary.opacity(0.46) } else { theme.primary._100 }
    }

    /// the mention and heart circles, same 20pt disc as the count pill with an icon
    /// greyed with it when the chat is muted
    fn mark_badge(system_image: &str, color: Hsla, ink: Hsla) -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .frame_size(20.0, 20.0)
            .circle()
            .bg(color)
            .children(symbol(system_image, 11.0, ink, system_image.ends_with(".fill")))
    }

    fn unread_badge(&self, theme: &AppTheme) -> Div {
        let unread_count = self.presentation.unread_count;
        let text = if unread_count > 999 { "999+".to_owned() } else { unread_count.to_string() };
        let color = self.badge_color(theme);
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .px(px(6.0))
            .frame_min(20.0, 20.0)
            .capsule()
            .bg(color)
            .child(
                MainText::new(text)
                    .px(11.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.content_on(color))
                    .number_of_lines(1),
            )
    }

    fn trailing_column(
        &self,
        trailing: Option<AnyElement>,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let presentation = &self.presentation;
        let mut column = vstack(4.0).align(HorizontalAlignment::Trailing).flex_none().min_h(px(50.0));
        if presentation.shows_metadata {
            let timestamp = (presentation.timestamp > 0.0).then(|| {
                let text = ChatListTimestampFormatter::string(
                    presentation.timestamp,
                    &Locale::app_locale(),
                    &Calendar::current(),
                );
                MainText::new(text).px(12.0).color(theme.text.secondary).number_of_lines(1)
            });
            column = column.child(hstack(4.0).children(self.delivery_indicator(theme)).children(timestamp));
        }
        column = column.child(spacer_min_h(if presentation.shows_metadata { 2.0 } else { 0.0 }));

        let has_reaction = presentation.unread_reaction_count > 0;
        let has_mention = presentation.unread_mention_count > 0;
        let has_reply = presentation.unread_reply_count > 0;
        let mark_badges: Entity<ChatListPreviewRowMarkBadges> =
            window.use_keyed_state(self.child_id("mark-badges"), cx, |_, _| {
                ChatListPreviewRowMarkBadges::new(has_reaction, has_mention, has_reply)
            });
        mark_badges.update(cx, |badges, cx| badges.sync(has_reaction, has_mention, has_reply, cx));

        let mut badges = hstack(6.0);
        if presentation.shows_metadata {
            let color = self.badge_color(theme);
            let ink = theme.content_on(color);
            let now = RiseClock::monotonic();
            let marks = mark_badges.read(cx);
            badges = badges
                .children(marks.reaction.render_transitioned(now, window, |_| Self::mark_badge("heart.fill", color, ink)))
                .children(marks.mention.render_transitioned(now, window, |_| Self::mark_badge("at", color, ink)))
                .children(marks.reply.render_transitioned(now, window, |_| {
                    Self::mark_badge("arrowshape.turn.up.left.fill", color, ink)
                }));
            if presentation.unread_count > 0 {
                badges = badges.child(self.unread_badge(theme));
            }
        }
        column.child(badges.children(trailing))
    }
}

impl RenderOnce for ChatListPreviewRow {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = theme_store(cx);
        let theme = store.read(cx).current_theme().clone();
        let leading = self.leading.take();
        let trailing = self.trailing.take();

        let preview = vstack(2.0)
            .align(HorizontalAlignment::Leading)
            .flex_1()
            .min_w_0()
            .child(self.title_line(&theme))
            .child(ChatListActivityPreview {
                id: self.child_id("activity"),
                chat_id: self.typing_chat_id.clone(),
                topic_id: self.typing_topic_id.clone(),
                shows_names: self.typing_shows_names,
                is_active: self.is_active && !self.measurement_only,
                content: self.preview_content(&theme).into_any_element(),
            });
        let body = hstack(self.leading_spacing)
            .align(VerticalAlignment::Top)
            .w_full()
            .pl(px(self.leading_spacing))
            .child(preview)
            .child(self.trailing_column(trailing, &theme, window, cx));
        let clip_extension = self
            .rail_avatar_size
            .map(|row_avatar_size| {
                EmbeddedForumRailMetrics::content_clip_extension(row_avatar_size, self.horizontal_padding)
            })
            .unwrap_or(0.0);
        let body = EmbeddedForumRailContentMotion { clip_extension }.body(body.into_any_element(), cx);

        let row = div().flex().items_start();
        let row = if LayoutDirection::current().is_right_to_left() { row.flex_row() } else { row.flex_row_reverse() };
        let hover_color = row_hover_fill(&theme);
        div()
            .flex()
            .items_center()
            .w_full()
            .min_h(px(ROW_MIN_HEIGHT))
            .bg(theme.bg._100)
            .hover(move |style| style.bg(hover_color))
            .child(
                row.w_full()
                    .px(px(self.horizontal_padding))
                    .py(px(self.vertical_padding))
                    .child(div().flex_1().min_w_0().child(body))
                    .children(leading.map(|leading| div().flex_none().child(leading))),
            )
    }
}

fn symbol(name: &str, side: f32, tint: Hsla, is_filled: bool) -> Option<Svg> {
    IconUi::sized(name, px(side), tint, is_filled).map(|icon| icon.flex_none())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ChatListActivityPreviewScope {
    chat_id: Option<String>,
    topic_id: Option<String>,
    account_epoch: Option<String>,
    engine_active: bool,
    is_active: bool,
    shows_names: bool,
}

struct ChatListActivityPreviewState {
    text: Option<String>,
    text_scope: Option<ChatListActivityPreviewScope>,
    engine_active: bool,
    activity_active: bool,
    rendered_active: bool,
    missed: bool,
    _presentation_observer: Subscription,
    _row_activity_observer: Option<Subscription>,
}

impl ChatListActivityPreviewState {
    fn new(
        presentation: &Entity<RiseConversationPresentation>,
        row_activity: Option<&Entity<ChatListRowActivity>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let presentation_observer = cx.observe(presentation, |this, presentation, cx| {
            let engine_active = presentation.read(cx).is_active();
            if this.engine_active != engine_active {
                this.engine_active = engine_active;
                cx.notify();
            }
        });
        Self {
            text: None,
            text_scope: None,
            engine_active: presentation.read(cx).is_active(),
            activity_active: true,
            rendered_active: true,
            missed: false,
            _presentation_observer: presentation_observer,
            _row_activity_observer: row_activity.map(|activity| {
                let (initial, this) = (activity.clone(), cx.weak_entity());
                cx.defer(move |cx| {
                    let active = initial.read(cx).is_active();
                    let _ = this.update(cx, |this, _| this.activity_active = active);
                });
                cx.observe(activity, |this, activity, cx| {
                    let active = activity.read(cx).is_active();
                    if this.activity_active == active {
                        return;
                    }
                    this.activity_active = active;
                    if active && (!this.rendered_active || this.missed) {
                        this.missed = false;
                        cx.notify();
                    }
                })
            }),
        }
    }
}

#[derive(IntoElement)]
struct ChatListActivityPreview {
    id: ElementId,
    chat_id: Option<String>,
    topic_id: Option<String>,
    shows_names: bool,
    is_active: bool,
    content: AnyElement,
}

impl ChatListActivityPreview {
    fn child_id(&self, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(suffix))
    }
}

impl RenderOnce for ChatListActivityPreview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let presentation = RiseConversationPresentation::shared(cx);
        let row_activity = env::<ChatListRowActivityKey>(cx);
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| {
            ChatListActivityPreviewState::new(&presentation, row_activity.as_ref(), cx)
        });
        let scope = ChatListActivityPreviewScope {
            chat_id: self.chat_id.clone(),
            topic_id: self.topic_id.clone(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
            engine_active: state.read(cx).engine_active,
            is_active: self.is_active && state.read(cx).activity_active,
            shows_names: self.shows_names,
        };
        let visible_text = state.update(cx, |state, _| {
            state.rendered_active = state.activity_active;
            state.missed = false;
            if state.text_scope.as_ref() == Some(&scope) { state.text.clone() } else { None }
        });

        let task_scope = scope.clone();
        let task_state = state.clone();
        use_task(self.child_id("typing"), scope, window, cx, async move |cx| {
            let reset = cx.update(|_, cx| {
                task_state.update(cx, |state, cx| {
                    let shown = state.text.take().is_some();
                    state.text_scope = Some(task_scope.clone());
                    if shown {
                        cx.notify();
                    }
                });
            });
            if reset.is_err() || !task_scope.is_active || !task_scope.engine_active {
                return;
            }
            let Some(chat_id) = task_scope.chat_id.clone() else {
                return;
            };
            let shows_names = task_scope.shows_names;
            let topic_id = task_scope.topic_id.clone();
            let update_state = task_state.downgrade();
            let on_update: Rc<dyn Fn(Option<String>, &mut App)> = Rc::new(move |value: Option<String>, cx: &mut App| {
                let _ = update_state.update(cx, |state, cx| {
                    if state.text_scope.as_ref() == Some(&task_scope) && state.text != value {
                        state.text = value;
                        if state.activity_active {
                            cx.notify();
                        } else {
                            state.missed = true;
                        }
                    }
                });
            });
            let observation = cx.update(|_, cx| {
                MessageInteractionsStore::shared(cx)
                    .update(cx, |store, cx| {
                    store.observe_chat_list_typing(&chat_id, topic_id.as_deref(), shows_names, on_update, cx)
                })
            });
            if let Ok(observation) = observation {
                observation.await;
            }
        });

        let store = theme_store(cx);
        let success = store.read(cx).current_theme().semantic.success_100;
        let indicator_id = self.child_id("indicator");
        let content_opacity = if visible_text.is_none() { 1.0 } else { 0.0 };
        div()
            .relative()
            .w_full()
            .child(div().w_full().opacity(content_opacity).child(self.content))
            .children(visible_text.map(|text| {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .child(ChatTypingIndicator::new(indicator_id, text, success).font_size(13.0))
            }))
    }
}
