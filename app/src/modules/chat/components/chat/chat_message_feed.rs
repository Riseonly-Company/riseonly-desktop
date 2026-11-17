use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{App, ElementId, Pixels, SharedString, Window};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::swiftui::prelude::*;

use crate::core::widgets::wrappers::list_shell_ui::ListShellFloatingTopInsetKey;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderEntryKind;
use crate::modules::chat::components::chat::async_render::rise_native_system_row_view::RiseNativeSystemRowView;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::{
    RiseMessageListView, RiseMessageListViewBoolHandler, RiseMessageListViewLoadPageHandler,
    RiseMessageListViewMarkCandidatesHandler, RiseMessageListViewOpenSenderProfileHandler,
    RiseMessageListViewPrefetchPageHandler, RiseMessageListViewSelectDateHandler, RiseMessageListViewSelectTopicHandler,
    RiseMessageListViewStringPairHandler, RiseMessageListViewToggleSelectionHandler, RiseMessageListViewViewportStateHandler,
    RiseMessageListViewVoidHandler, RiseMessageScrollRequest,
};
use crate::modules::chat::components::chat::chat_messages_empty_state::ChatMessagesEmptyState;
use crate::modules::chat::components::chat::media_viewer::chat_media_gallery_builder::ChatViewLimitedOpenings;
use crate::modules::chat::components::chat::media_viewer::chat_media_presentation::ChatGalleryMessagesKey;
use crate::modules::chat::components::chat::message_system_pill::MessageSystemFormatContext;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseMessagePresentationContentStamp;
use crate::modules::chat::shared::chat_search_highlight::ChatMessageSearchHighlight;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageSystemKind,
};

/// picks the card that fits vertically out of the two
/// gpui has no ideal size so it renders invisible copies just to read heights
/// will do something smarter later mb
#[derive(Clone, Copy, Default)]
struct ChatMessageFeedEmptyCardFit {
    available_height: f32,
    full_height: f32,
    compact_height: f32,
}

#[derive(IntoElement)]
pub struct ChatMessageFeed {
    id: ElementId,
    pub messages: Vec<Arc<GetMessageMessage>>,
    pub chat: Arc<ChatInfo>,
    pub chat_id: String,
    pub scroll_position_key: Option<String>,
    pub bottom_inset: f32,
    pub sticky_bottom_inset: Option<f32>,
    pub keyboard_resting_bottom_inset: Option<f32>,
    pub keyboard_resting_sticky_bottom_inset: Option<f32>,
    pub keyboard_active_bottom_inset: Option<f32>,
    pub keyboard_active_sticky_bottom_inset: Option<f32>,
    pub tracks_system_keyboard: bool,
    pub settles_composer_inset_immediately: bool,
    pub source_version: i64,
    pub content_stamps_by_local_id: HashMap<String, RiseMessagePresentationContentStamp>,
    pub presentation_epoch: Option<Uuid>,
    pub interactions_enabled: bool,
    pub scroll_request: Option<RiseMessageScrollRequest>,
    pub allows_message_selection: bool,
    pub message_selection_active: bool,
    pub selected_message_ids: HashSet<String>,
    pub on_toggle_message_selection: Option<RiseMessageListViewToggleSelectionHandler>,
    pub on_reaction: Option<RiseMessageListViewStringPairHandler>,
    pub on_action: Option<RiseMessageListViewStringPairHandler>,
    pub on_open_sender_profile: Option<RiseMessageListViewOpenSenderProfileHandler>,
    pub group_by_topic: bool,
    pub topic_titles: HashMap<String, String>,
    pub on_select_topic: Option<RiseMessageListViewSelectTopicHandler>,
    pub on_select_date: Option<RiseMessageListViewSelectDateHandler>,
    pub can_load_older: bool,
    pub can_load_newer: bool,
    pub has_cached_older: bool,
    pub has_cached_newer: bool,
    pub on_load_older: Option<RiseMessageListViewLoadPageHandler>,
    pub on_load_newer: Option<RiseMessageListViewLoadPageHandler>,
    pub on_prefetch_page: Option<RiseMessageListViewPrefetchPageHandler>,
    pub initial_data_resolved: bool,
    pub on_initial_frame_ready: Option<RiseMessageListViewVoidHandler>,
    pub on_scroll_activity_changed: Option<RiseMessageListViewBoolHandler>,
    pub on_viewport_state_changed: Option<RiseMessageListViewViewportStateHandler>,
    pub on_visible_mark_candidates_changed: Option<RiseMessageListViewMarkCandidatesHandler>,
    pub search_highlight: Option<ChatMessageSearchHighlight>,
    pub unread_separator_message_id: Option<String>,
    pub view_limited_closed: HashSet<String>,
    pub gallery_paginates: bool,
}

impl PartialEq for ChatMessageFeed {
    fn eq(&self, other: &Self) -> bool {
        self.chat_id == other.chat_id
            && self.view_limited_closed == other.view_limited_closed
            && self.gallery_paginates == other.gallery_paginates
            && self.search_highlight == other.search_highlight
            && self.unread_separator_message_id == other.unread_separator_message_id
            && self.scroll_position_key == other.scroll_position_key
            && self.chat.id == other.chat.id
            && self.chat.r#type == other.chat.r#type
            && self.chat.allowed_reactions == other.chat.allowed_reactions
            && self.chat.role == other.chat.role
            && self.chat.forward_in_chat_enabled == other.chat.forward_in_chat_enabled
            && self.chat.can_pin_messages == other.chat.can_pin_messages
            && self.chat.can_manage_chat == other.chat.can_manage_chat
            && self.bottom_inset == other.bottom_inset
            && self.sticky_bottom_inset == other.sticky_bottom_inset
            && self.keyboard_resting_bottom_inset == other.keyboard_resting_bottom_inset
            && self.keyboard_resting_sticky_bottom_inset == other.keyboard_resting_sticky_bottom_inset
            && self.keyboard_active_bottom_inset == other.keyboard_active_bottom_inset
            && self.keyboard_active_sticky_bottom_inset == other.keyboard_active_sticky_bottom_inset
            && self.tracks_system_keyboard == other.tracks_system_keyboard
            && self.settles_composer_inset_immediately == other.settles_composer_inset_immediately
            && self.source_version == other.source_version
            && self.messages.len() == other.messages.len()
            && self.messages.first().map(|message| &message.id) == other.messages.first().map(|message| &message.id)
            && self.messages.last().map(|message| &message.id) == other.messages.last().map(|message| &message.id)
            && self.presentation_epoch == other.presentation_epoch
            && self.interactions_enabled == other.interactions_enabled
            && self.scroll_request == other.scroll_request
            && self.allows_message_selection == other.allows_message_selection
            && self.message_selection_active == other.message_selection_active
            && self.selected_message_ids == other.selected_message_ids
            && self.group_by_topic == other.group_by_topic
            && (!self.group_by_topic || self.topic_titles == other.topic_titles)
            && self.can_load_older == other.can_load_older
            && self.can_load_newer == other.can_load_newer
            && self.has_cached_older == other.has_cached_older
            && self.has_cached_newer == other.has_cached_newer
            && self.initial_data_resolved == other.initial_data_resolved
    }
}

impl ChatMessageFeed {
    pub fn new(
        id: impl Into<ElementId>,
        messages: Vec<Arc<GetMessageMessage>>,
        chat: Arc<ChatInfo>,
        chat_id: impl Into<String>,
        bottom_inset: f32,
        source_version: i64,
    ) -> Self {
        Self {
            id: id.into(),
            messages,
            chat,
            chat_id: chat_id.into(),
            scroll_position_key: None,
            bottom_inset,
            sticky_bottom_inset: None,
            keyboard_resting_bottom_inset: None,
            keyboard_resting_sticky_bottom_inset: None,
            keyboard_active_bottom_inset: None,
            keyboard_active_sticky_bottom_inset: None,
            tracks_system_keyboard: false,
            settles_composer_inset_immediately: false,
            source_version,
            content_stamps_by_local_id: HashMap::new(),
            presentation_epoch: None,
            interactions_enabled: true,
            scroll_request: None,
            allows_message_selection: false,
            message_selection_active: false,
            selected_message_ids: HashSet::new(),
            on_toggle_message_selection: None,
            on_reaction: None,
            on_action: None,
            on_open_sender_profile: None,
            group_by_topic: false,
            topic_titles: HashMap::new(),
            on_select_topic: None,
            on_select_date: None,
            can_load_older: false,
            can_load_newer: false,
            has_cached_older: false,
            has_cached_newer: false,
            on_load_older: None,
            on_load_newer: None,
            on_prefetch_page: None,
            initial_data_resolved: true,
            on_initial_frame_ready: None,
            on_scroll_activity_changed: None,
            on_viewport_state_changed: None,
            on_visible_mark_candidates_changed: None,
            search_highlight: None,
            unread_separator_message_id: None,
            view_limited_closed: HashSet::new(),
            gallery_paginates: true,
        }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn shows_empty_conversation_card(&self) -> bool {
        self.initial_data_resolved && self.has_no_conversation_yet() && !self.chat.is_active_bot_chat()
    }

    fn has_no_conversation_yet(&self) -> bool {
        if self.messages.is_empty() {
            return true;
        }
        if self.can_load_older || self.has_cached_older {
            return false;
        }
        self.messages.iter().all(|message| Self::is_creation_plate(message))
    }

    fn is_creation_plate(message: &GetMessageMessage) -> bool {
        match message.system.as_ref().map(|system| &system.kind) {
            Some(MessageSystemKind::ChatCreated)
            | Some(MessageSystemKind::ChannelCreated)
            | Some(MessageSystemKind::MigratedFromGroup) => true,
            Some(MessageSystemKind::Generic) | None => {
                message.content_type == MessageContentType::System
                    && MessageSystemFormatContext::creation_key(&message.text).is_some()
            }
            _ => false,
        }
    }

    fn creation_plates_height(&self) -> f32 {
        if self.messages.is_empty() {
            return 0.0;
        }
        RiseNativeSystemRowView::single_line_row_height(ChatMessageRenderEntryKind::DateSeparator)
            + self.messages.len() as f32 * RiseNativeSystemRowView::single_line_row_height(ChatMessageRenderEntryKind::System)
    }

    fn empty_conversation_card(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let floating_top_inset = env::<ListShellFloatingTopInsetKey>(cx);
        let fit = window.use_keyed_state(self.child_id("empty-card-fit"), cx, |_, _| ChatMessageFeedEmptyCardFit::default());
        let measured = *fit.read(cx);
        let record = |write: fn(&mut ChatMessageFeedEmptyCardFit, f32)| {
            let fit = fit.clone();
            move |height: Pixels, cx: &mut App| {
                fit.update(cx, |fit, cx| {
                    let mut next = *fit;
                    write(&mut next, f32::from(height));
                    if next.available_height != fit.available_height
                        || next.full_height != fit.full_height
                        || next.compact_height != fit.compact_height
                    {
                        *fit = next;
                        cx.notify();
                    }
                });
            }
        };
        let record_available = record(|fit, height| fit.available_height = height);
        let record_full = record(|fit, height| fit.full_height = height);
        let record_compact = record(|fit, height| fit.compact_height = height);

        let chosen = if measured.full_height <= measured.available_height {
            Some(ChatMessagesEmptyState::new(self.chat.clone()))
        } else if measured.compact_height <= measured.available_height {
            Some(ChatMessagesEmptyState::new(self.chat.clone()).compact(true))
        } else {
            None
        };

        div()
            .absolute()
            .inset_0()
            .pt(px(floating_top_inset + 8.0))
            .pb(px(self.bottom_inset + self.creation_plates_height()))
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(measure(self.child_id("empty-card-available"), move |bounds, _, cx| {
                        record_available(bounds.size.height, cx);
                    }))
                    .children(chosen)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .opacity(0.0)
                            .child(ChatMessagesEmptyState::new(self.chat.clone()))
                            .child(measure(self.child_id("empty-card-full"), move |bounds, _, cx| {
                                record_full(bounds.size.height, cx);
                            })),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .opacity(0.0)
                            .child(ChatMessagesEmptyState::new(self.chat.clone()).compact(true))
                            .child(measure(self.child_id("empty-card-compact"), move |bounds, _, cx| {
                                record_compact(bounds.size.height, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl RenderOnce for ChatMessageFeed {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let empty_card = self
            .shows_empty_conversation_card()
            .then(|| self.empty_conversation_card(window, cx));
        let gallery_messages = Arc::new(self.messages.clone());
        let view_limited_closed: HashSet<String> =
            self.view_limited_closed.union(ChatViewLimitedOpenings::shared(cx).read(cx).closed()).cloned().collect();

        let mut list = RiseMessageListView::new(
            self.child_id("list"),
            self.messages,
            self.chat,
            self.chat_id,
            self.bottom_inset,
            self.source_version,
        );
        list.sticky_bottom_inset = self.sticky_bottom_inset;
        list.keyboard_resting_bottom_inset = self.keyboard_resting_bottom_inset;
        list.keyboard_resting_sticky_bottom_inset = self.keyboard_resting_sticky_bottom_inset;
        list.keyboard_active_bottom_inset = self.keyboard_active_bottom_inset;
        list.keyboard_active_sticky_bottom_inset = self.keyboard_active_sticky_bottom_inset;
        list.tracks_system_keyboard = self.tracks_system_keyboard;
        list.settles_composer_inset_immediately = self.settles_composer_inset_immediately;
        list.content_stamps_by_local_id = self.content_stamps_by_local_id;
        list.presentation_epoch = self.presentation_epoch;
        list.scroll_request = self.scroll_request;
        list.scroll_position_key = self.scroll_position_key;
        list.interactions_enabled = self.interactions_enabled;
        list.allows_message_selection = self.allows_message_selection;
        list.allows_reply_jump = true;
        list.gallery_paginates = self.gallery_paginates;
        list.message_selection_active = self.message_selection_active;
        list.selected_message_ids = self.selected_message_ids;
        list.on_toggle_message_selection = self.on_toggle_message_selection;
        list.initial_data_resolved = self.initial_data_resolved;
        list.can_load_older = self.can_load_older;
        list.can_load_newer = self.can_load_newer;
        list.has_cached_older = self.has_cached_older;
        list.has_cached_newer = self.has_cached_newer;
        list.group_by_topic = self.group_by_topic;
        list.topic_titles = self.topic_titles;
        list.on_reaction = self.on_reaction;
        list.on_action = self.on_action;
        list.on_open_sender_profile = self.on_open_sender_profile;
        list.on_select_topic = self.on_select_topic;
        list.on_select_date = self.on_select_date;
        list.on_load_older = self.on_load_older;
        list.on_load_newer = self.on_load_newer;
        list.on_prefetch_page = self.on_prefetch_page;
        list.on_initial_frame_ready = self.on_initial_frame_ready;
        list.on_scroll_activity_changed = self.on_scroll_activity_changed;
        list.on_viewport_state_changed = self.on_viewport_state_changed;
        list.on_visible_mark_candidates_changed = self.on_visible_mark_candidates_changed;
        list.search_highlight = self.search_highlight;
        list.unread_separator_message_id = self.unread_separator_message_id;
        list.view_limited_closed = view_limited_closed;

        env_scope::<ChatGalleryMessagesKey>(Some(gallery_messages))
            .child(div().relative().size_full().child(list).children(empty_card))
    }
}
