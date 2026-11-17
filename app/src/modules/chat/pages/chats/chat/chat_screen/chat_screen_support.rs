use std::collections::HashMap;
use std::rc::Rc;
use gpui::{App, Context, ElementId, Entity, Window};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::swiftui::lifecycle::on_change;
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::transition::Transition;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::modules::chat::engine::messages::rise_message_unread_marks::RiseUnreadMarkKind;
use crate::modules::chat::components::chat::composer::chat_composer_types::{ChatComposerEditTarget, ChatComposerReplyTarget};
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::modules::media_picker::editor::editor_state::EditorImageState;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChatMediaPickerMode {
    #[default]
    None,
    Attachments,
    VideoNote,
}

pub enum ChatMessagePresentationRecoveryPolicy {}

impl ChatMessagePresentationRecoveryPolicy {
    pub fn should_use_canonical_fallback(bounded_count: i64, canonical_count: i64) -> bool {
        bounded_count == 0 && canonical_count > 0
    }

    pub fn initial_data_resolved(
        is_pending_chat: bool,
        visible_count: i64,
        canonical_count: i64,
        request_in_flight: bool,
        request_status: SaiWsSliceStatus,
        allows_filtered_empty: bool,
        awaits_unread_divider: bool,
        opening_request_pending: bool,
    ) -> bool {
        if is_pending_chat {
            return true;
        }
        if awaits_unread_divider && opening_request_pending {
            return false;
        }
        if visible_count > 0 {
            return true;
        }
        if opening_request_pending {
            return false;
        }
        if request_in_flight || request_status != SaiWsSliceStatus::Fulfilled {
            return false;
        }
        if allows_filtered_empty {
            return true;
        }
        canonical_count == 0
    }
}

/// one spring for the whole navigation column
/// so a button entering or leaving and its neighbours animate together
pub enum ChatMessageNavigationButtonMotion {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatMessageNavigationButtonMotionLayout {
    pub reactions: bool,
    pub replies: bool,
    pub mentions: bool,
    pub down: bool,
}

impl ChatMessageNavigationButtonMotion {
    pub fn animation() -> MotionSpec {
        spring(0.3, 0.82)
    }

    pub fn transition() -> Transition {
        Transition::scale(0.2).combined(Transition::opacity())
    }
}

pub(super) struct UnreadMarkButtonFace {
    pub(super) system_image: &'static str,
    pub(super) icon_size: f32,
    pub(super) read_all_title: String,
    pub(super) accessibility_label: String,
}

impl UnreadMarkButtonFace {
    pub(super) fn new(kind: RiseUnreadMarkKind) -> Self {
        match kind {
            RiseUnreadMarkKind::Mention => {
                Self::with("at", 19.0, tr("ctx_read_all_mentions").to_string(), tr("chat_unread_mentions_button").to_string())
            }
            RiseUnreadMarkKind::Reply => Self::with(
                "arrowshape.turn.up.left.fill",
                17.0,
                tr("notify_mark_all_read").to_string(),
                tr("notification_settings_replies").to_string(),
            ),
            RiseUnreadMarkKind::Reaction => {
                Self::with("heart.fill", 17.0, tr("ctx_read_all_reactions").to_string(), tr("chat_unread_reactions_button").to_string())
            }
        }
    }

    pub(super) fn with(system_image: &'static str, icon_size: f32, read_all_title: String, accessibility_label: String) -> Self {
        Self { system_image, icon_size, read_all_title, accessibility_label }
    }
}

pub enum ChatJumpToLatestPresentationPolicy {}

impl ChatJumpToLatestPresentationPolicy {
    pub const FADE_DURATION: f64 = 0.16;
    pub const KEEPS_LAYOUT_SLOT_MOUNTED: bool = true;

    pub fn is_visible(requested: bool, message_selection_active: bool, bot_menu_visible: bool) -> bool {
        requested && !message_selection_active && !bot_menu_visible
    }
}

pub struct ChatViewportChromeReader {
    pub chrome: Entity<ChatMessageViewportChromeState>,
}

impl ChatViewportChromeReader {
    pub fn new(chrome: Entity<ChatMessageViewportChromeState>) -> Self {
        Self { chrome }
    }

    pub fn content<R>(&self, cx: &App, content: impl FnOnce(bool) -> R) -> R {
        let shows_jump_to_bottom = self.chrome.read(cx).shows_jump_to_bottom();
        content(shows_jump_to_bottom)
    }
}

/// one button plate replacing the composer, eg unblocking a peer or a channel sound
/// height is declared and not measured since the list computes its bottom inset from it
pub enum ChatComposerPlateMetrics {}

impl ChatComposerPlateMetrics {
    pub const ACTION_HEIGHT: f32 = 45.0;
    pub const HINT_HEIGHT: f32 = 18.0;
    pub const HINT_SPACING: f32 = 8.0;

    pub fn row_height() -> f32 {
        Self::ACTION_HEIGHT
    }
}

#[derive(Default)]
pub struct ChatMessageViewportChromeState {
    is_at_latest: bool,
    shows_jump_to_bottom: bool,
    presentation: Option<String>,
}

impl ChatMessageViewportChromeState {
    pub fn is_at_latest(&self) -> bool {
        self.is_at_latest
    }

    pub fn shows_jump_to_bottom(&self) -> bool {
        self.shows_jump_to_bottom
    }

    pub fn update(&mut self, is_at_latest: bool, shows_jump_to_bottom: bool, cx: &mut Context<Self>) {
        if self.is_at_latest != is_at_latest {
            self.is_at_latest = is_at_latest;
            cx.notify();
        }
        if self.shows_jump_to_bottom != shows_jump_to_bottom {
            self.shows_jump_to_bottom = shows_jump_to_bottom;
            cx.notify();
        }
    }

    pub fn update_presentation(
        &mut self,
        is_at_latest: bool,
        shows_jump_to_bottom: bool,
        presentation: String,
        cx: &mut Context<Self>,
    ) {
        self.presentation = Some(presentation);
        self.update(is_at_latest, shows_jump_to_bottom, cx);
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.update(false, false, cx);
    }

    pub fn reset_for_presentation(&mut self, presentation: &str, cx: &mut Context<Self>) {
        if self.presentation.as_deref() == Some(presentation) {
            return;
        }
        self.presentation = Some(presentation.to_owned());
        self.reset(cx);
    }

    pub fn finish_latest_jump(&mut self, cx: &mut Context<Self>) {
        self.update(true, false, cx);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct ChatMessageLoadTaskID {
    pub(super) scope: String,
    pub(super) scroll_position_key: String,
    pub(super) navigation_open_epoch: Option<Uuid>,
    pub(super) viewer_removed_from_community: bool,
}

/// identity of one travel to a message
/// carries the scope it was asked in and the app wide waiting state it armed
#[derive(Clone, Debug)]
pub(super) struct ChatMessageJumpToken {
    pub(super) jump_id: Uuid,
    pub(super) scope: String,
    pub(super) progress_token: i64,
}

/// an unsent own message the person tapped
/// the sheet offers retry only if the sender can still replay it, delete always
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatFailedSendTarget {
    pub message_id: String,
    pub can_retry: bool,
    pub reason: Option<String>,
}

pub struct ChatScreenState {
    pub input_text: String,
    pub media_picker_mode: ChatMediaPickerMode,
    pub voice_too_short_visible: bool,

    pub reply_target: Option<ChatComposerReplyTarget>,
    pub edit_target: Option<ChatComposerEditTarget>,
    pub saved_draft: Option<String>,

    pub is_message_selection_active: bool,
    pub selected_messages_by_server_id: HashMap<String, GetMessageMessage>,

    pub is_message_search_active: bool,
    pub message_search_query: String,
    pub message_search_index: i64,
    pub message_search_delivered_query: Option<String>,
    pub message_search_pending_step: i64,
    pub message_search_shows_list: bool,
    pub show_selected_messages_delete_dialog: bool,
    pub show_chat_delete_dialog: bool,
    pub failed_send_target: Option<ChatFailedSendTarget>,

    pub selected_topic_tab: ChatTopicTab,
    pub topic_selection_initialized: bool,
    pub show_topic_create: bool,
}

impl ChatScreenState {
    pub fn new(selected_topic_tab: ChatTopicTab, topic_selection_initialized: bool) -> Self {
        Self {
            input_text: String::new(),
            media_picker_mode: ChatMediaPickerMode::None,
            voice_too_short_visible: false,
            reply_target: None,
            edit_target: None,
            saved_draft: None,
            is_message_selection_active: false,
            selected_messages_by_server_id: HashMap::new(),
            is_message_search_active: false,
            message_search_query: String::new(),
            message_search_index: 0,
            message_search_delivered_query: None,
            message_search_pending_step: 0,
            message_search_shows_list: false,
            show_selected_messages_delete_dialog: false,
            show_chat_delete_dialog: false,
            failed_send_target: None,
            selected_topic_tab,
            topic_selection_initialized,
            show_topic_create: false,
        }
    }
}

impl Default for ChatScreenState {
    fn default() -> Self {
        Self::new(ChatTopicTab::All, false)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ChatTopicTab {
    #[default]
    All,
    General,
    Topic(String),
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct ChatRemovalObserved {
    pub(super) chat_id: String,
    pub(super) removed: bool,
}

pub(super) enum ChatRemovalObserverModifier {}

impl ChatRemovalObserverModifier {
    pub(super) fn observe(
        id: impl Into<ElementId>,
        chat_id: &str,
        on_removed: Rc<dyn Fn(&mut Window, &mut App)>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let chats = ChatServicesStore::shared(cx);
        let rise = RiseConversationPresentation::shared(cx);
        let removed = chats.read(cx).confirmed_removed_chat_ids.contains(chat_id)
            || rise.read(cx).confirmed_removed_chat_ids().contains(chat_id);
        let observed = ChatRemovalObserved { chat_id: chat_id.to_owned(), removed };
        on_change(id, observed, false, window, cx, move |old, new, window, cx| {
            if old.chat_id == new.chat_id && !old.removed && new.removed {
                on_removed(window, cx);
            }
        });
    }
}

pub use crate::modules::chat::components::tabs::chat_profile_tabs::pages::chat_header_refresh_schedule_hooks::ChatHeaderRefreshSchedule;

pub(super) struct ChatPendingReeditRenderInput {
    pub(super) image: CleverImage,
    pub(super) edit_state: EditorImageState,
}
