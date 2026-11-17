use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyView, App, Context, ElementId, Empty, Entity, FontWeight, Hsla, Render, SharedString, StyleRefinement, Subscription,
    TextRun, Window, px,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_theme::avatar_palette::AvatarPalette;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::expandable_text_ui::{EdgeInsets, ExpandableTextUi};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::{NativeConfirmAlert, NativeConfirmAlertAction};
use rise_ui::swiftui::prelude::*;
use rise_core::foundation::foundation_url::Url;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{MainTab, RiseRTCRoomTarget, RootRoute};
use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::qr_code::rise_qr_code_card::RiseQRCodeSubject;
use crate::core::ui::qr_code::rise_qr_code_modal::RiseQRCodePresentation;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::{ShareSheetStore, ShareTagPolicy};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::shared::bot_ui_validation::BotChatManagement;
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotTagRules;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_sheet::ChatWallpaperSheet;
use crate::modules::chat::engine::forum::rise_chat_voice_presence::RiseChatVoicePresentation;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_contact_profile_policy::ChatContactProfilePolicy;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_profile_more_sheets::{
    AutoDeleteSheet, TransferOwnershipSheet,
};
#[allow(unused_imports)]
pub(crate) use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::{
    ChatCountFormatter, ChatTileColor,
};
use crate::modules::chat::shared::chat_message_search_presenter::ChatMessageSearchPresenter;
use crate::modules::chat::shared::chat_presentation_policy::{ChatProfileAction, ChatProfileActionRowPolicy};
use crate::modules::chat::stores::chats::chat_actions::chat_avatar_palette::AvatarPaletteChat;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatPerm, ChatType};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
pub(crate) use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::gifts::engine::rise_gifts_engine_models::RiseGiftItemKind;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::{
    GiftRecipient, GiftsInteractionsStore,
};
use crate::modules::moderation::components::admin_ban_entry::{AdminBanEntry, AdminBanTarget, admin_ban_sheet};
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::notify::components::notifications_hub_sheet_store::NotificationsHubSheetStore;
use crate::modules::rtc::shared::rise_rtc_navigation_gate::RiseRTCNavigationGate;
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;
use crate::modules::rtc::system::rise_rtc_call_system_coordinator::RiseRTCCallStartRefusal;
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::components::user_block_scope_dialog::UserBlockAndReport;
use crate::modules::user::shared::profile_missing_tag_state::ProfileMissingTagState;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

/// keys hang off the instance id and not the chat id
/// one chat can be open in the side panel and the stack at once and they must not share state
fn profile_id(name: &str, scope: &ElementId) -> ElementId {
    ElementId::NamedChild(Arc::new(scope.clone()), SharedString::from(format!("chat-profile-{name}")))
}

/// what a report from the chat profile is about
/// the person or the whole group or channel or the bot
/// lives here since the overflow menu offers it now
pub enum ChatProfileReportTarget {}

impl ChatProfileReportTarget {
    pub fn resolve(chat: &ChatInfo, cx: &mut App) -> Option<ReportTarget> {
        match chat.r#type {
            ChatType::Group | ChatType::Channel => {
                if ChatProfilePermissions::is_owner(chat) || chat.id.is_empty() {
                    return None;
                }
                Some(ReportTarget::Chat(chat.id.clone()))
            }
            ChatType::Direct => {
                let wire_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
                if chat.is_bot_chat() {
                    let tag = BotChatManagement::normalized_tag(chat);
                    let bot = if BotTagRules::is_valid(&tag) { BotServicesStore::bot_by_tag(&tag, cx) } else { None };
                    if BotChatManagement::can_manage(chat, wire_user_id.as_deref().unwrap_or(""), bot.as_ref()) {
                        return None;
                    }
                    if let Some(bot_id) = bot.map(|bot| bot.id).filter(|id| !id.is_empty()) {
                        return Some(ReportTarget::Bot(bot_id));
                    }
                    return (!chat.id.is_empty()).then(|| ReportTarget::Chat(chat.id.clone()));
                }
                let peer = &chat.participant.id;
                if peer.is_empty()
                    || wire_user_id.as_deref() == Some(peer.as_str())
                    || UserServicesStore::is_contact_join_virtual_chat_id(&chat.id)
                {
                    return None;
                }
                Some(ReportTarget::User(peer.clone()))
            }
            ChatType::Favourites => None,
        }
    }
}

const ROW_INSET: f32 = 12.0;
const BUTTON_SPACING: f32 = 10.0;
const LABEL_INSET: f32 = 4.0;
const LABEL_FONT_SIZE: f32 = 11.0;
const LABEL_MIN_READABLE_SCALE: f32 = 0.75;
const ICON_BOX_HEIGHT: f32 = 26.0;
const ACTION_FACE_HEIGHT: f32 = 66.0;

/// what the call button draws, a change of it is the only thing that redraws besides the start guard
#[derive(Clone, PartialEq)]
struct ChatProfileCallButtonFace {
    id: ElementId,
    label: String,
    fill: Hsla,
    icon: Hsla,
    text: Hsla,
    text_delta: f32,
}

/// call button dims and ignores taps while a start is in flight
/// own view so the dimming doesnt redraw the side panel
#[derive(Default)]
struct ChatProfileCallButton {
    is_starting: bool,
    face: Option<ChatProfileCallButtonFace>,
    theme: Option<AppTheme>,
    on_tap: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl ChatProfileCallButton {
    fn configure(
        &mut self,
        face: ChatProfileCallButtonFace,
        theme: AppTheme,
        on_tap: Rc<dyn Fn(&mut Window, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        self.on_tap = Some(on_tap);
        self.theme = Some(theme);
        if self.face.as_ref() != Some(&face) {
            self.face = Some(face);
            cx.notify();
        }
    }

    fn set_starting(&mut self, is_starting: bool, cx: &mut Context<Self>) {
        if self.is_starting != is_starting {
            self.is_starting = is_starting;
            cx.notify();
        }
    }
}

impl Render for ChatProfileCallButton {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (Some(face), Some(theme)) = (self.face.clone(), self.theme.as_ref()) else {
            return div().into_any_element();
        };
        let this = cx.entity().downgrade();
        plain(face.id)
            .w_full()
            .disabled(self.is_starting)
            .on_press(move |window, cx| {
                let tap = this.upgrade().and_then(|button| button.read(cx).on_tap.clone());
                if let Some(tap) = tap {
                    tap(window, cx);
                }
            })
            .child(ChatProfileActionRowParts::action_face("phone.fill", face.label, theme))
            .into_any_element()
    }
}

struct ChatProfileActionRowState {
    show_wallpaper: bool,
    show_transfer: bool,
    show_auto_delete: bool,
    call_button: Entity<ChatProfileCallButton>,
    admin_ban_target: Option<AdminBanTarget>,
    row_width: Option<f32>,
    _observers: [Subscription; 3],
}

impl ChatProfileActionRowState {
    fn new(cx: &mut Context<Self>) -> Self {
        let chat_services = ChatServicesStore::shared(cx);
        let user_services = UserServicesStore::shared(cx);
        let voice = RiseChatVoicePresentation::shared(cx);
        Self {
            show_wallpaper: false,
            show_transfer: false,
            show_auto_delete: false,
            call_button: cx.new(|_| ChatProfileCallButton::default()),
            admin_ban_target: None,
            row_width: None,
            _observers: [
                cx.observe(&chat_services, |_, _, cx| cx.notify()),
                cx.observe(&user_services, |_, _, cx| cx.notify()),
                cx.observe(&voice, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement)]
pub struct ChatProfileActionRow {
    id: ElementId,
    chat: ChatInfo,
    is_topic_profile: bool,
    shows_open_chat: bool,
    on_open_chat: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ChatProfileActionRow {
    pub fn new(id: impl Into<ElementId>, chat: ChatInfo) -> Self {
        Self {
            id: id.into(),
            chat,
            is_topic_profile: false,
            shows_open_chat: true,
            on_open_chat: Rc::new(|_: &mut Window, _: &mut App| {}),
        }
    }

    pub fn on_open_chat(mut self, on_open_chat: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_open_chat = Rc::new(on_open_chat);
        self
    }

    pub fn is_topic_profile(mut self, is_topic_profile: bool) -> Self {
        self.is_topic_profile = is_topic_profile;
        self
    }

    pub fn shows_open_chat(mut self, shows_open_chat: bool) -> Self {
        self.shows_open_chat = shows_open_chat;
        self
    }
}

/// what the buttons and menu entries need, each holds its own copy
#[derive(Clone)]
struct ChatProfileActionRowParts {
    id: ElementId,
    chat: Rc<ChatInfo>,
    is_topic_profile: bool,
    shows_open_chat: bool,
    on_open_chat: Rc<dyn Fn(&mut Window, &mut App)>,
    nav_store: Option<Entity<NavigationStore>>,
    state: Entity<ChatProfileActionRowState>,
}

impl ChatProfileActionRowParts {
    /// private chat is made by its first message
    /// so mute, auto delete and delete have nothing on the server yet
    fn is_draft_chat(&self) -> bool {
        ChatServicesStore::is_draft_direct_chat_id(&self.chat.id)
    }

    fn action_candidates(&self, cx: &mut App) -> Vec<ChatProfileAction> {
        let chat = &self.chat;
        let mut candidates = Vec::new();
        if self.shows_open_chat {
            candidates.push(ChatProfileAction::OpenChat);
        }
        if chat.r#type == ChatType::Direct && !chat.participant.id.is_empty() {
            candidates.push(ChatProfileAction::Profile);
        }
        if self.shows_group_voice(cx) {
            candidates.push(ChatProfileAction::GroupVoice);
        }
        if chat.r#type == ChatType::Direct
            && !chat.is_bot_chat()
            && !chat.participant.is_deleted
            && !chat.participant.id.is_empty()
        {
            candidates.push(ChatProfileAction::Call);
        }
        if !self.is_draft_chat() {
            candidates.push(ChatProfileAction::Notifications);
        }
        if self.can_send_gift(cx) {
            candidates.push(ChatProfileAction::Gift);
        }
        candidates.push(ChatProfileAction::Search);
        candidates.push(ChatProfileAction::More);
        candidates
    }

    fn can_send_gift(&self, cx: &mut App) -> bool {
        let chat = &self.chat;
        chat.r#type == ChatType::Direct
            && !chat.is_bot_chat()
            && !chat.participant.is_deleted
            && !chat.participant.id.is_empty()
            && !AuthServicesStore::shared(cx).read(cx).is_current_user(&chat.participant.id)
    }

    /// only a plain group has a voice chat
    /// in a forum the voice channels cover it, private chats and channels have none
    /// starting one needs the right to manage video chats
    fn shows_group_voice(&self, cx: &mut App) -> bool {
        let chat = &self.chat;
        if chat.r#type != ChatType::Group || chat.topics_enabled || self.is_topic_profile || chat.is_community_preview() {
            return false;
        }
        let services = ChatServicesStore::shared(cx);
        services.update(cx, |services, cx| services.can(&chat.id, ChatPerm::CONNECT_VOICE, cx))
            && (services.update(cx, |services, cx| services.can(&chat.id, ChatPerm::MANAGE_VOICE_CHANNEL, cx))
                || self.is_group_voice_live(cx))
    }

    fn is_group_voice_live(&self, cx: &mut App) -> bool {
        RiseChatVoicePresentation::shared(cx).read(cx).is_live(&self.chat.id).unwrap_or(self.chat.has_live_voice)
    }

    fn title(action: ChatProfileAction) -> String {
        match action {
            ChatProfileAction::OpenChat => tr("chat_profile_text"),
            ChatProfileAction::Profile => tr("ctx_open_profile"),
            ChatProfileAction::GroupVoice => tr("rtc_voice_chat"),
            ChatProfileAction::Call => tr("chat_profile_phone"),
            ChatProfileAction::Notifications => tr("chat_mute_advanced_title"),
            ChatProfileAction::Gift => tr("gift_action_send"),
            ChatProfileAction::Search => tr("chat_profile_search"),
            ChatProfileAction::More => tr("chat_profile_more"),
        }
    }

    fn fitting_button_count(
        candidates: &[ChatProfileAction],
        outer_width: Option<f32>,
        text_delta: f32,
        window: &Window,
    ) -> i64 {
        let mut count = ChatProfileActionRowPolicy::max_visible_buttons();
        let Some(outer_width) = outer_width else {
            return count;
        };
        let row_width = outer_width - ROW_INSET * 2.0;
        let mut font = window.text_style().font();
        font.weight = FontWeight::MEDIUM;
        let font_size = px((LABEL_FONT_SIZE + text_delta).max(1.0));
        while count > 3 {
            let visible = ChatProfileActionRowPolicy::layout(candidates, Some(count)).visible;
            let buttons = visible.len() as f32;
            let button_width = (row_width - BUTTON_SPACING * (buttons - 1.0)) / buttons;
            let room = (button_width - LABEL_INSET * 2.0) / LABEL_MIN_READABLE_SCALE;
            let fits = visible.iter().all(|action| {
                let title = Self::title(*action);
                let run = TextRun {
                    len: title.len(),
                    font: font.clone(),
                    color: gpui::Hsla::default(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                f32::from(window.text_system().shape_line(SharedString::from(title), font_size, &[run], None).width())
                    <= room
            });
            if fits {
                return count;
            }
            count -= 1;
        }
        count
    }

    fn action_face(icon: &'static str, label: String, theme: &AppTheme) -> gpui::Div {
        let is_filled = icon.ends_with(".fill");
        vstack(7.0)
            .w_full()
            .min_w_0()
            .min_h(px(ACTION_FACE_HEIGHT))
            .items_center()
            .justify_center()
            .bg(theme.bg._200)
            .rounded_rect(16.0)
            .child(
                div()
                    .h(px(ICON_BOX_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized(icon, px(19.0), theme.primary._100, is_filled)),
            )
            .child(
                div().w_full().min_w_0().px(px(LABEL_INSET)).flex().justify_center().child(
                    MainText::new(label)
                        .px(LABEL_FONT_SIZE)
                        .font_weight(FontWeight::MEDIUM)
                        .color(theme.text.primary.opacity(0.85))
                        .number_of_lines(1)
                        .minimum_scale_factor(0.7),
                ),
            )
    }

    fn action_button(
        &self,
        name: &str,
        icon: &'static str,
        label: String,
        theme: &AppTheme,
        tap: impl Fn(&mut Window, &mut App) + 'static,
    ) -> PlainButton {
        plain(profile_id(name, &self.id))
            .w_full()
            .on_press(tap)
            .child(Self::action_face(icon, label, theme))
    }

    fn row_button(
        &self,
        action: ChatProfileAction,
        overflow: &[ChatProfileAction],
        theme: &AppTheme,
        cx: &mut App,
    ) -> gpui::AnyElement {
        let label = Self::title(action);
        let parts = self.clone();
        match action {
            ChatProfileAction::OpenChat => self
                .action_button("open-chat", "bubble.left.fill", label, theme, move |window, cx| {
                    (parts.on_open_chat)(window, cx);
                })
                .into_any_element(),
            ChatProfileAction::Profile => self
                .action_button("peer-profile", "person.crop.circle.fill", label, theme, move |_, cx| {
                    parts.open_peer_profile(cx);
                })
                .into_any_element(),
            ChatProfileAction::GroupVoice => self
                .action_button("group-voice", "waveform", label, theme, move |_, cx| parts.open_group_voice(cx))
                .into_any_element(),
            ChatProfileAction::Call => {
                let button = self.state.read(cx).call_button.clone();
                let face = ChatProfileCallButtonFace {
                    id: profile_id("call", &self.id),
                    label,
                    fill: theme.bg._200,
                    icon: theme.primary._100,
                    text: theme.text.primary,
                    text_delta: ThemeStore::shared(cx).read(cx).text_delta(),
                };
                let theme = theme.clone();
                button.update(cx, |button, cx| {
                    button.configure(face, theme, Rc::new(move |_: &mut Window, cx: &mut App| parts.start_audio_call(cx)), cx);
                });
                div()
                    .w_full()
                    .child(
                        AnyView::from(button)
                            .cached(StyleRefinement::default().w_full().h(px(ACTION_FACE_HEIGHT)))
                            .replay_when_moved(),
                    )
                    .guide_anchor(RiseGuideAnchor::ChatProfileCall)
                    .into_any_element()
            }
            ChatProfileAction::Notifications => {
                let services = ChatServicesStore::shared(cx);
                let is_muted = services.read(cx).effective_is_muted(&self.chat, cx);
                let icon = if is_muted { "bell.slash.fill" } else { "bell.fill" };
                self.action_button("notifications", icon, label, theme, move |_, cx| {
                    parts.present_notifications_hub(cx);
                })
                .guide_anchor(RiseGuideAnchor::ChatProfileNotifications)
                .into_any_element()
            }
            ChatProfileAction::Gift => self
                .action_button("gift", "gift.fill", label, theme, move |_, cx| parts.open_gift_shop(cx))
                .into_any_element(),
            ChatProfileAction::Search => self
                .action_button("search", "magnifyingglass", label, theme, move |window, cx| {
                    parts.open_message_search(window, cx);
                })
                .into_any_element(),
            ChatProfileAction::More => {
                let items = self.more_menu_items(overflow, cx);
                ContextMenuUi::new(
                    profile_id("more", &self.id),
                    items,
                    div().w_full().child(Self::action_face("ellipsis", label, theme)),
                )
                .guide_anchor(RiseGuideAnchor::ChatProfileMore)
                .into_any_element()
            }
        }
    }

    fn overflow_menu_entry(&self, action: ChatProfileAction, next_id: &mut i64) -> Option<ContextMenuItem> {
        let parts = self.clone();
        let key = match action {
            ChatProfileAction::OpenChat => "overflow-open-chat",
            ChatProfileAction::GroupVoice => "overflow-group-voice",
            ChatProfileAction::Call => "overflow-call",
            ChatProfileAction::Profile => "overflow-profile",
            ChatProfileAction::Gift => "overflow-gift",
            ChatProfileAction::Notifications | ChatProfileAction::Search | ChatProfileAction::More => return None,
        };
        let id = *next_id;
        *next_id += 1;
        Some(ContextMenuItem::new(id, Self::title(action), None, key, move |window, cx| match action {
            ChatProfileAction::OpenChat => (parts.on_open_chat)(window, cx),
            ChatProfileAction::GroupVoice => parts.open_group_voice(cx),
            ChatProfileAction::Call => parts.start_audio_call(cx),
            ChatProfileAction::Profile => parts.open_peer_profile(cx),
            ChatProfileAction::Gift => parts.open_gift_shop(cx),
            ChatProfileAction::Notifications | ChatProfileAction::Search | ChatProfileAction::More => {}
        }))
    }

    fn more_menu_items(&self, overflow: &[ChatProfileAction], cx: &mut App) -> Vec<ContextMenuItem> {
        let mut next_id = 0_i64;
        let mut items: Vec<ContextMenuItem> =
            overflow.iter().filter_map(|action| self.overflow_menu_entry(*action, &mut next_id)).collect();
        self.more_menu(&mut items, &mut next_id, cx);
        items
    }

    fn push_item(
        items: &mut Vec<ContextMenuItem>,
        next_id: &mut i64,
        label: String,
        key: &str,
        destructive: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) {
        items.push(ContextMenuItem::new(*next_id, label, None, key, action).destructive(destructive));
        *next_id += 1;
    }

    fn more_menu(&self, items: &mut Vec<ContextMenuItem>, next_id: &mut i64, cx: &mut App) {
        let chat = self.chat.clone();
        let is_draft = self.is_draft_chat();
        let is_community = matches!(chat.r#type, ChatType::Group | ChatType::Channel);
        let is_owner = ChatProfilePermissions::is_owner(&chat);

        if chat.is_official_bot_chat() {
            let parts = self.clone();
            Self::push_item(items, next_id, tr("chat_mute_advanced_title"), "notifications", false, move |_, cx| {
                parts.present_notifications_hub(cx);
            });
            self.report_menu_entry(items, next_id, cx);
            if !is_draft {
                let parts = self.clone();
                Self::push_item(items, next_id, tr("chat_clear_history_action"), "clear-official", true, move |_, cx| {
                    let chat = parts.chat.clone();
                    ChatInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.confirm_delete_chat(&chat, None, None, cx));
                });
            }
            return;
        }

        if !is_draft {
            let parts = self.clone();
            Self::push_item(items, next_id, tr("chat_mute_advanced_title"), "notifications", false, move |_, cx| {
                parts.present_notifications_hub(cx);
            });
            let parts = self.clone();
            Self::push_item(items, next_id, tr("wallpaper_chat_sheet_title"), "wallpaper", false, move |_, cx| {
                parts.state.update(cx, |state, cx| {
                    state.show_wallpaper = true;
                    cx.notify();
                });
            });
        }
        if !is_draft && ChatProfilePermissions::sets_auto_delete(&chat, cx) {
            let parts = self.clone();
            Self::push_item(items, next_id, tr("chat_autodelete_title"), "autodelete", false, move |_, cx| {
                parts.state.update(cx, |state, cx| {
                    state.show_auto_delete = true;
                    cx.notify();
                });
            });
        }
        if is_owner && is_community {
            let parts = self.clone();
            Self::push_item(items, next_id, tr("chat_transfer_title"), "transfer", false, move |_, cx| {
                parts.show_transfer(cx);
            });
        }
        if is_community {
            let parts = self.clone();
            let title = if chat.r#type == ChatType::Channel { tr("chat_leave_channel") } else { tr("chat_leave_group") };
            Self::push_item(items, next_id, title, "leave", true, move |_, cx| parts.leave_chat(cx));
        }
        self.report_menu_entry(items, next_id, cx);
        self.block_menu_entries(items, next_id, cx);
        self.admin_ban_menu_entry(items, next_id, cx);
        if !is_draft && chat.r#type == ChatType::Direct {
            let parts = self.clone();
            Self::push_item(items, next_id, tr("chat_clear_history_action"), "clear-history", true, move |_, cx| {
                let chat = parts.chat.clone();
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.confirm_clear_history(&chat, cx));
            });
        }
        if !is_draft && (is_owner || chat.r#type == ChatType::Direct) {
            let parts = self.clone();
            Self::push_item(items, next_id, tr("chat_delete_text"), "delete", true, move |_, cx| {
                let chat = parts.chat.clone();
                let back = parts.clone();
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.confirm_delete_chat(&chat, None, Some(Box::new(move |cx: &mut App| back.back_to_chats_list(cx))), cx);
                });
            });
        }
    }

    /// destructive and above the block entries
    /// reporting is the heavier action and under block it read as softer
    fn report_menu_entry(&self, items: &mut Vec<ContextMenuItem>, next_id: &mut i64, cx: &mut App) {
        let Some(target) = ChatProfileReportTarget::resolve(&self.chat, cx) else {
            return;
        };
        Self::push_item(items, next_id, tr("ctx_report"), "report", true, move |_, cx| {
            let target = target.clone();
            ReportSheetStore::shared(cx).update(cx, |store, cx| store.present(target, None, None, cx));
        });
    }

    fn block_menu_entries(&self, items: &mut Vec<ContextMenuItem>, next_id: &mut i64, cx: &mut App) {
        let current_user_id =
            AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default();
        if !ChatContactProfilePolicy::is_blockable(&self.chat, &current_user_id, self.is_topic_profile) {
            return;
        }
        let target_id = self.chat.participant.id.clone();
        let user_services = UserServicesStore::shared(cx);
        let block_scope = user_services.update(cx, |services, cx| services.viewer_block_scope(&target_id, cx));
        if block_scope.is_some() {
            Self::push_item(items, next_id, tr("contact_unblock_action"), "unblock", false, move |_, cx| {
                UserInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.set_user_block_scope(&target_id, None, |_, _| {}, cx));
            });
            let parts = self.clone();
            Self::push_item(items, next_id, tr("contact_block_scope_change_action"), "block-change", false, move |_, cx| {
                parts.present_block_scope_dialog(cx);
            });
        } else {
            let parts = self.clone();
            Self::push_item(items, next_id, tr("contact_block_action"), "block", true, move |_, cx| {
                parts.present_block_scope_dialog(cx);
            });
        }
    }

    fn admin_ban_menu_entry(&self, items: &mut Vec<ContextMenuItem>, next_id: &mut i64, cx: &mut App) {
        let chat = &self.chat;
        if chat.r#type != ChatType::Direct
            || self.is_topic_profile
            || chat.is_bot_chat()
            || !AdminBanEntry::is_available(&chat.participant.id, None, cx)
        {
            return;
        }
        let parts = self.clone();
        Self::push_item(items, next_id, AdminBanEntry::title(), AdminBanEntry::ACTION_ID, true, move |_, cx| {
            let target =
                AdminBanTarget { user_id: parts.chat.participant.id.clone(), name: Some(parts.chat.participant.name.clone()) };
            parts.state.update(cx, |state, cx| {
                state.admin_ban_target = Some(target);
                cx.notify();
            });
        });
    }

    fn present_block_scope_dialog(&self, cx: &mut App) {
        let message = format!(
            "{}. {}\n{}. {}",
            tr("contact_block_scope_standard"),
            tr("contact_block_scope_standard_desc"),
            tr("contact_block_scope_total"),
            tr("contact_block_scope_total_desc"),
        );
        let standard = self.clone();
        let total = self.clone();
        let report_target = self.chat.participant.id.clone();
        let on_standard: NativeConfirmAlertAction =
            Box::new(move |cx: &mut App| standard.set_block_scope(RiseBlockScope::Standard, cx));
        let on_total: NativeConfirmAlertAction =
            Box::new(move |cx: &mut App| total.set_block_scope(RiseBlockScope::Total, cx));
        let on_block_and_report: NativeConfirmAlertAction = Box::new(move |cx: &mut App| {
            if report_target.is_empty() {
                return;
            }
            UserBlockAndReport::run(&report_target, None, None, cx);
        });
        NativeConfirmAlert::present_destructive_choices(
            &tr("contact_block_scope_title"),
            &message,
            vec![
                (tr("contact_block_scope_standard"), on_standard),
                (tr("contact_block_scope_total"), on_total),
                (tr("user_block_and_report"), on_block_and_report),
            ],
            None,
            cx,
        );
    }

    fn set_block_scope(&self, scope: RiseBlockScope, cx: &mut App) {
        let target_id = &self.chat.participant.id;
        if target_id.is_empty() {
            return;
        }
        UserInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.set_user_block_scope(target_id, Some(scope), |_, _| {}, cx));
    }

    fn show_transfer(&self, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.show_transfer = true;
            cx.notify();
        });
    }

    fn leave_chat(&self, cx: &mut App) {
        let chat = self.chat.clone();
        let back = self.clone();
        let completion: Box<dyn FnOnce(&mut App)> = Box::new(move |cx: &mut App| back.back_to_chats_list(cx));
        let store = ChatInteractionsStore::shared(cx);
        if ChatProfilePermissions::is_owner(&chat) {
            let transfer = self.clone();
            let on_transfer: Box<dyn FnOnce(&mut App)> = Box::new(move |cx: &mut App| transfer.show_transfer(cx));
            store.update(cx, |store, cx| store.confirm_delete_chat(&chat, Some(on_transfer), Some(completion), cx));
        } else {
            store.update(cx, |store, cx| store.confirm_leave_chat(&chat, None, Some(completion), cx));
        }
    }

    fn open_gift_shop(&self, cx: &mut App) {
        let peer = &self.chat.participant;
        let recipient = GiftRecipient {
            id: peer.id.clone(),
            name: peer.name.clone(),
            tag: peer.tag.clone(),
            avatar: peer.more.logo.clone(),
            cover: None,
        };
        GiftsInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.open_shop(recipient, RiseGiftItemKind::Item, cx));
    }

    fn open_peer_profile(&self, cx: &mut App) {
        let Some(nav_store) = &self.nav_store else {
            return;
        };
        let peer = &self.chat.participant;
        if peer.is_deleted {
            ProfileMissingTagState::mark_deleted_user(&peer.id, cx);
        }
        let route = RootRoute::UserPage {
            user_id: Some(peer.id.clone()),
            tag: (!peer.is_deleted && !peer.tag.is_empty()).then(|| peer.tag.clone()),
        };
        nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }

    fn open_group_voice(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            RiseRTCNavigationGate::open(RiseRTCRoomTarget::Group { chat_id: self.chat.id.clone() }, nav_store, cx);
        }
    }

    fn open_message_search(&self, window: &mut Window, cx: &mut App) {
        let chat_id = &self.chat.id;
        ChatMessageSearchPresenter::shared(cx).update(cx, |presenter, cx| presenter.enqueue(chat_id, cx));
        (self.on_open_chat)(window, cx);
    }

    fn present_notifications_hub(&self, cx: &mut App) {
        let chat = &self.chat;
        let target_user_id =
            (chat.r#type == ChatType::Direct && !chat.is_bot_chat()).then_some(chat.participant.id.as_str());
        NotificationsHubSheetStore::shared(cx).update(cx, |store, cx| {
            store.present(Some(chat.id.as_str()), Some(chat.notification_scopes()), target_user_id, cx);
        });
    }

    fn back_to_chats_list(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |store, cx| {
                store.switch_tab(MainTab::Chats, cx);
                store.pop_to_root(cx);
            });
        }
    }

    fn start_audio_call(&self, cx: &mut App) {
        let call_button = self.state.read(cx).call_button.clone();
        if call_button.read(cx).is_starting {
            return;
        }
        call_button.update(cx, |button, cx| button.set_starting(true, cx));
        let chat = self.chat.clone();
        cx.spawn(async move |cx| {
            let peer = &chat.participant;
            let display_name = trimming_whitespaces_and_newlines(&peer.name);
            let display_name =
                if display_name.is_empty() { chat.direct_peer_profile_name() } else { display_name.to_owned() };
            let avatar_url = if peer.more.logo.is_empty() { chat.logo_url.clone() } else { Some(peer.more.logo.clone()) };
            let server_chat_id = if peer.is_deleted {
                None
            } else {
                ChatInteractionsStore::server_chat_id(chat.id.clone(), cx).await
            };
            let failure = match server_chat_id {
                Some(chat_id) => {
                    let call = cx.update(|cx| {
                        RTCInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.request_system_outgoing_call(&peer.id, &chat_id, display_name, avatar_url, false, cx)
                        })
                    });
                    call.await.err().map(|error| RiseRTCCallStartRefusal::text_or_failed(&error))
                }
                None => Some(tr("rtc_call_failed")),
            };
            cx.update(|cx| {
                call_button.update(cx, |button, cx| button.set_starting(false, cx));
                if let Some(text) = failure {
                    Self::notify_call_failed(text, cx);
                }
            });
        })
        .detach();
    }

    fn notify_call_failed(text: String, cx: &mut App) {
        Notifier::send(
            text,
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }
}

impl RenderOnce for ChatProfileActionRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let text_delta = ThemeStore::shared(cx).read(cx).text_delta();
        let chat_id = self.chat.id.clone();
        let id = self.id.clone();
        let state = window.use_keyed_state(profile_id("action-row-state", &id), cx, |_, cx| {
            ChatProfileActionRowState::new(cx)
        });
        let parts = ChatProfileActionRowParts {
            id: self.id,
            chat: Rc::new(self.chat),
            is_topic_profile: self.is_topic_profile,
            shows_open_chat: self.shows_open_chat,
            on_open_chat: self.on_open_chat,
            nav_store: env::<NavigationStoreKey>(cx),
            state: state.clone(),
        };

        let candidates = parts.action_candidates(cx);
        let row_width = state.read(cx).row_width;
        let max_visible =
            ChatProfileActionRowParts::fitting_button_count(&candidates, row_width, text_delta, window);
        let layout = ChatProfileActionRowPolicy::layout(&candidates, Some(max_visible));
        let buttons = layout
            .visible
            .iter()
            .map(|action| {
                div().flex_1().min_w_0().child(parts.row_button(*action, &layout.overflow, &theme, cx))
            })
            .collect::<Vec<_>>();

        let measure_state = state.clone();
        let wallpaper_chat = parts.chat.clone();
        let transfer_chat_id = chat_id.clone();
        let auto_delete_chat_id = chat_id.clone();
        let auto_delete_current = parts.chat.auto_delete_messages_after.unwrap_or(0);

        div()
            .relative()
            .w_full()
            .child(hstack(BUTTON_SPACING).w_full().px(px(ROW_INSET)).children(buttons))
            .child(
                measure(profile_id("action-row-measure", &id), move |bounds, _, cx| {
                    let width = f32::from(bounds.size.width);
                    measure_state.update(cx, |state, cx| {
                        if state.row_width != Some(width) {
                            state.row_width = Some(width);
                            cx.notify();
                        }
                    });
                })
                .absolute()
                .size_full(),
            )
            .child(
                NativeBottomSheetPresentationHost::new(
                    profile_id("wallpaper-sheet", &id),
                    Binding::entity(
                        state.clone(),
                        |state: &ChatProfileActionRowState| state.show_wallpaper,
                        |state, value, _| state.show_wallpaper = value,
                    ),
                    move |_, _| ChatWallpaperSheet::new(wallpaper_chat.id.clone(), (*wallpaper_chat).clone()),
                )
                .presentation_detents([])
                .presentation_drag_indicator(true),
            )
            .child(
                NativeBottomSheetPresentationHost::new(
                    profile_id("transfer-sheet", &id),
                    Binding::entity(
                        state.clone(),
                        |state: &ChatProfileActionRowState| state.show_transfer,
                        |state, value, _| state.show_transfer = value,
                    ),
                    move |_, _| TransferOwnershipSheet::new(transfer_chat_id.clone()),
                )
                .presentation_detents([PresentationDetent::Large])
                .presentation_drag_indicator(true),
            )
            .child(admin_ban_sheet(
                profile_id("admin-ban-sheet", &id),
                Binding::entity(
                    state.clone(),
                    |state: &ChatProfileActionRowState| state.admin_ban_target.clone(),
                    |state, value, _| state.admin_ban_target = value,
                ),
            ))
            .child(
                NativeBottomSheetPresentationHost::new(
                    profile_id("autodelete-sheet", &id),
                    Binding::entity(
                        state,
                        |state: &ChatProfileActionRowState| state.show_auto_delete,
                        |state, value, _| state.show_auto_delete = value,
                    ),
                    move |_, _| AutoDeleteSheet::new(auto_delete_chat_id.clone(), auto_delete_current),
                )
                .presentation_detents([PresentationDetent::Medium])
                .presentation_drag_indicator(true),
            )
    }
}

/// trailing icon is its own button, the row opens the share sheet and the icon opens the code card
#[derive(Clone)]
struct Trailing {
    system_image: &'static str,
    action: Rc<dyn Fn(&mut App)>,
}

/// the only way to get the tag itself and not a link
/// share sheet and row copy always give a link since only this app opens a tag
#[derive(Clone)]
struct Hold {
    title: String,
    value: String,
}

#[derive(Clone)]
struct Row {
    id: &'static str,
    label: String,
    value: String,
    tinted: bool,
    trailing: Option<Trailing>,
    multiline: bool,
    expandable: bool,
    hold: Option<Hold>,
    context_copy_value: Option<String>,
    on_tap: Option<Rc<dyn Fn(&mut App)>>,
}

const EXPANDABLE_LINE_LIMIT: usize = 5;

struct ChatProfileInfoCardObservers {
    expanded_row_ids: HashSet<&'static str>,
    _chat_services: Subscription,
}

#[derive(IntoElement)]
pub struct ChatProfileInfoCard {
    id: ElementId,
    chat: ChatInfo,
    additional_items: Vec<GroupedBtn>,
}

impl ChatProfileInfoCard {
    pub fn new(id: impl Into<ElementId>, chat: ChatInfo) -> Self {
        Self { id: id.into(), chat, additional_items: Vec::new() }
    }

    pub fn additional_items(mut self, additional_items: Vec<GroupedBtn>) -> Self {
        self.additional_items = additional_items;
        self
    }

    fn row_view(scope: &ElementId, row: Row, expanded: Binding<bool>, theme: &AppTheme) -> gpui::Div {
        let trailing = row.trailing.clone();
        let row_id = row.id;
        hstack(0.0).w_full().child(Self::value_block(scope, row, expanded, theme)).when_some(trailing, |content, trailing| {
            let system_image = trailing.system_image;
            let action = trailing.action;
            content.child(
                div().mr(px(8.0)).child(
                    plain(profile_id(&format!("info-trailing-{row_id}"), scope))
                        .size(px(44.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_press(move |_, cx| action(cx))
                        .children(IconUi::sized(system_image, px(20.0), theme.primary._100, false)),
                ),
            )
        })
    }

    fn value_label(scope: &ElementId, row: &Row, expanded: &Binding<bool>, theme: &AppTheme) -> gpui::Div {
        let value = if row.expandable {
            ExpandableTextUi::new(
                profile_id(&format!("info-expandable-{}", row.id), scope),
                row.value.clone(),
                theme.text.primary,
                theme.primary._100,
            )
            .font_size(15.0)
            .line_limit(EXPANDABLE_LINE_LIMIT)
            .is_expanded(expanded.clone())
            .collapse_control_insets(EdgeInsets::new(4.0, 0.0, 0.0, 48.0))
            .parent_handles_taps(true)
            .into_any_element()
        } else {
            let value = MainText::new(row.value.clone())
                .px(15.0)
                .color(if row.tinted { theme.primary._100 } else { theme.text.primary });
            if row.multiline { value.into_any_element() } else { value.number_of_lines(1).into_any_element() }
        };
        vstack(3.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .pl(px(16.0))
            .pr(px(if row.trailing.is_none() { 16.0 } else { 6.0 }))
            .py(px(12.0))
            .child(MainText::new(row.label.clone()).px(12.0).color(theme.text.secondary))
            .child(value)
    }

    fn value_block(scope: &ElementId, row: Row, expanded: Binding<bool>, theme: &AppTheme) -> gpui::Div {
        let hold = row.hold.clone().or_else(|| {
            row.context_copy_value.clone().map(|value| Hold { title: tr("ctx_copy"), value })
        });
        let on_tap = row.on_tap.clone();
        let expandable = row.expandable;
        let tap_expanded = expanded.clone();
        let tap: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |window: &mut Window, cx: &mut App| {
            if !expandable {
                if let Some(on_tap) = &on_tap {
                    on_tap(cx);
                }
                return;
            }
            ExpandableTextUi::toggle_animated(&tap_expanded, window, cx);
        });
        let block = div().flex_1().min_w_0();
        let Some(hold) = hold else {
            let is_disabled = row.on_tap.is_none() && !row.expandable;
            return block.child(
                plain(profile_id(&format!("info-row-{}", row.id), scope))
                    .w_full()
                    .disabled(is_disabled)
                    .on_press(move |window, cx| tap(window, cx))
                    .child(Self::value_label(scope, &row, &expanded, theme)),
            );
        };

        let actions = vec![HoldContextMenuAction::new("copy", hold.title).system_image("doc.on.doc")];
        let copy_value = hold.value;
        let content_row = row.clone();
        let content_theme = theme.clone();
        let content_scope = scope.clone();
        block.child(
            HoldContextMenuContainer::new(
                profile_id(&format!("info-hold-{}", row.id), scope),
                HoldContextMenuAlignment::Trailing,
                Rc::new(move |_: &App| actions.clone()),
                Rc::new(move |_: &mut Window, _: &mut App| {
                    Self::value_label(&content_scope, &content_row, &expanded, &content_theme).into_any_element()
                }),
            )
            .show_emoji_list(false)
            .on_action(Rc::new(move |_: String, _: &mut Window, cx: &mut App| Self::copy_to_pasteboard(&copy_value, cx)))
            .on_tap(Rc::new(move |_: gpui::Point<gpui::Pixels>, window: &mut Window, cx: &mut App| tap(window, cx))),
        )
    }

    fn copy_to_pasteboard(value: &str, cx: &mut App) {
        UIPasteboard::general().set_string(value, cx);
        let _ = notification(NotificationKind::Success);
    }

    fn make_rows(chat: &Rc<ChatInfo>, nav_store: Option<Entity<NavigationStore>>, cx: &mut App) -> Vec<Row> {
        let mut rows = Vec::new();

        if let Some(link) = Self::share_link(chat, cx) {
            let tag = Self::public_tag(chat);
            let qr_chat = chat.clone();
            let qr_link = link.clone();
            let qr_tag = tag.clone();
            let share_link = link.clone();
            rows.push(Row {
                id: "public-link",
                label: Self::row_label(chat, tag.is_some()),
                value: tag.clone().unwrap_or_else(|| link.to_string()),
                tinted: true,
                trailing: Some(Trailing {
                    system_image: "qrcode",
                    action: Rc::new(move |cx: &mut App| {
                        Self::open_qr_code(&qr_chat, qr_link.clone(), qr_tag.clone(), nav_store.as_ref(), cx);
                    }),
                }),
                multiline: false,
                expandable: false,
                hold: tag.map(|value| Hold { title: tr("qr_code_copy_tag"), value }),
                context_copy_value: Some(link.to_string()),
                on_tap: Some(Rc::new(move |cx: &mut App| {
                    let link = share_link.clone();
                    ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_link(link, cx));
                })),
            });
        }

        if chat.r#type == ChatType::Direct && !chat.participant.is_deleted {
            let aliases = chat
                .participant
                .alias_tags
                .iter()
                .map(|alias| trimming_whitespaces_and_newlines(alias))
                .filter(|alias| !alias.is_empty())
                .map(|alias| format!("@{alias}"))
                .collect::<Vec<_>>();
            if !aliases.is_empty() {
                let joined = aliases.join(", ");
                let copied = joined.clone();
                rows.push(Row {
                    id: "alias-tags",
                    label: tr("chat_profile_info_alias_tags_title"),
                    value: joined,
                    tinted: true,
                    trailing: None,
                    multiline: true,
                    expandable: false,
                    hold: None,
                    context_copy_value: None,
                    on_tap: Some(Rc::new(move |cx: &mut App| Self::copy_to_pasteboard(&copied, cx))),
                });
            }
        }

        let description = chat.description.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or("");
        if !description.is_empty() {
            let desc = description.to_owned();
            let copied = desc.clone();
            rows.push(Row {
                id: "desc",
                label: if chat.r#type == ChatType::Direct {
                    tr("chat_profile_info_description_title")
                } else {
                    tr("chat_profile_info_channel_description_title")
                },
                value: desc,
                tinted: false,
                trailing: None,
                multiline: true,
                expandable: true,
                hold: None,
                context_copy_value: Some(copied),
                on_tap: None,
            });
        }

        rows
    }

    /// public address of a peer or community is always a link
    /// tag is only displayed, anything leaving the app uses the link
    /// the tag is reachable via hold menu only
    fn share_link(chat: &ChatInfo, cx: &mut App) -> Option<Url> {
        if chat.r#type == ChatType::Direct { Self::direct_profile_link(chat) } else { Self::community_share_link(chat, cx) }
    }

    fn row_label(chat: &ChatInfo, has_tag: bool) -> String {
        if !has_tag {
            return tr("chat_profile_info_link_title");
        }
        if chat.r#type == ChatType::Direct {
            tr("chat_profile_info_username_title")
        } else {
            tr("chat_profile_info_tag_title")
        }
    }

    fn public_tag(chat: &ChatInfo) -> Option<String> {
        let raw = if chat.r#type == ChatType::Direct {
            chat.participant.tag.as_str()
        } else if chat.is_public {
            chat.tag.as_deref().unwrap_or("")
        } else {
            ""
        };
        ShareTagPolicy::normalized(raw)
    }

    fn direct_profile_link(chat: &ChatInfo) -> Option<Url> {
        if chat.participant.is_deleted {
            return None;
        }
        let tag = trimming_whitespaces_and_newlines(&chat.participant.tag).trim_start_matches('@');
        if tag.is_empty() {
            return None;
        }
        RiseonlyPublicLink::Profile { tag: tag.to_owned() }.web_url(AppEnvironment::current())
    }

    fn open_qr_code(
        chat: &ChatInfo,
        link: Url,
        tag: Option<String>,
        nav_store: Option<&Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        let is_direct = chat.r#type == ChatType::Direct;
        let title = if is_direct { chat.direct_peer_profile_name() } else { chat.title.clone() };
        let seed = if is_direct { chat.participant.id.clone() } else { chat.id.clone() };
        let subject =
            RiseQRCodeSubject::new(link, tag, title, AvatarPalette::resolved_logo_url_for_chat(chat), Some(seed));
        RiseQRCodePresentation::present(subject, nav_store.cloned(), cx);
    }

    fn community_share_link(chat: &ChatInfo, cx: &mut App) -> Option<Url> {
        if !matches!(chat.r#type, ChatType::Group | ChatType::Channel) {
            return None;
        }
        let can_share_private =
            chat.is_public || (chat.can_share_invite && ChatProfilePermissions::distributes_invites(chat, cx));
        if !can_share_private {
            return None;
        }

        let environment = AppEnvironment::current();
        if chat.is_public {
            let tag = chat.tag.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or("");
            if !tag.is_empty() {
                return RiseonlyPublicLink::PublicTag { tag: tag.to_owned() }.web_url(environment);
            }
        }
        if let Some(url) =
            chat.invite_link.as_deref().and_then(|invite| RiseonlyPublicLink::join_url(invite, environment))
        {
            return Some(url);
        }
        if chat.is_public && chat.r#type == ChatType::Group {
            RiseonlyPublicLink::Chat { id: chat.id.clone() }.web_url(environment)
        } else {
            None
        }
    }
}

impl RenderOnce for ChatProfileInfoCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let id = self.id.clone();
        let card_state =
            window.use_keyed_state(profile_id("info-observers", &id), cx, |_, cx: &mut Context<ChatProfileInfoCardObservers>| {
                let services = ChatServicesStore::shared(cx);
                ChatProfileInfoCardObservers {
                    expanded_row_ids: HashSet::new(),
                    _chat_services: cx.observe(&services, |_, _, cx| cx.notify()),
                }
            });
        let nav_store = env::<NavigationStoreKey>(cx);
        let chat = Rc::new(self.chat);
        let mut items = Self::make_rows(&chat, nav_store, cx)
            .into_iter()
            .map(|row| {
                let row_id = row.id;
                let expanded = Binding::entity(
                    card_state.clone(),
                    move |state: &ChatProfileInfoCardObservers| state.expanded_row_ids.contains(row_id),
                    move |state, open, cx| {
                        if open {
                            state.expanded_row_ids.insert(row_id);
                        } else {
                            state.expanded_row_ids.remove(row_id);
                        }
                        cx.notify();
                    },
                );
                GroupedBtn::new(format!("chat-profile-info-{}", row.id), row.value.clone())
                    .show_chevron(false)
                    .content_slot(Self::row_view(&id, row, expanded, &theme))
            })
            .collect::<Vec<_>>();
        items.extend(self.additional_items);

        if items.is_empty() {
            return Empty.into_any_element();
        }
        GroupedBtns::new(profile_id("info-card", &id), items).separator_leading_padding(16.0).into_any_element()
    }
}
