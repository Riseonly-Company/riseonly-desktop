use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, Context, ElementId, Empty, Entity, FontWeight, ListAlignment, Pixels, SharedString,
    StyleRefinement, Subscription, WeakEntity, Window,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::IconUi;
use rise_ui::bot_icon::BotIcon;
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::list_ui::list_row_transitions::{ListRowTransitions, ListRowsFrame, ListShownRow};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::official_badge::OfficialBadge;
use rise_ui::skeleton_ui::{SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::motion::{MotionSpec, ease_in_out};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_content_bottom_margin;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::components::chat_item::chat_item::ChatItem;
use crate::modules::chat::components::chat_item::chat_item_skeleton::ChatItemSkeleton;
use crate::modules::chat::components::chat_search::chat_message_search_results_list::{
    ChatMessageSearchResultRow, ChatMessageSearchRowMetrics,
};
use crate::modules::chat::components::chat_search::chat_search_overlay_presenter::ChatSearchOverlayPresenter;
use crate::modules::chat::components::chat_search::chat_search_overlay_store::ChatSearchOverlayStore;
use crate::modules::chat::components::chat_search::chat_search_section::{
    ChatSearchErrorState, ChatSearchSectionHeader,
};
use crate::modules::chat::components::chat_search::chats_search_context::{
    ChatSearchKeyboardTarget, ChatSearchResultsKeyboard, ChatSearchResultsKeyboardKey, ChatsSearchContext, ChatsSearchContextKey,
};
use crate::modules::chat::engine::chats::rise_chat_repository_realtime::RiseChatRetractedPreviews;
use crate::modules::chat::engine::core::rise_chat_overlay_search_presentation::{
    RiseChatOverlaySearchPage, RiseChatSearchResultsPolicy,
};
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::RiseConversationAuxiliaryLoadState;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseConversationPresentation, RiseConversationPresentationChange,
};
use crate::modules::chat::engine::messages::rise_message_search_session::{
    ChatMessageSearchPolicy, ChatSearchChatKindFilter, RiseMessageSearchHit, RiseMessageSearchPresentationState,
};
use crate::modules::chat::pages::chats::chat_list_history_warmup_controller::ChatListHistoryWarmupController;
use crate::modules::chat::pages::chats::embedded_forum_transition::EmbeddedForumPresentationStore;
use crate::modules::chat::shared::chat_message_jump_presenter::ChatMessageJumpPresenter;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::MessageDeliveryStatus;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::search::shared::global_search_route_resolver::GlobalSearchRouteResolver;
use crate::modules::search::stores::search_actions::search_types::SearchEntityRow;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const WARMUP_PAGE_ID: &str = "chat-search-overlay";
const SEPARATOR_HEIGHT: f32 = 0.5;
const BOTTOM_PADDING: f32 = 20.0;
const SELECTED_ROW_TINT: f32 = 0.10;
const GLOBAL_TOGGLE_DURATION: f64 = 0.18;

fn tab_id(suffix: impl Into<String>) -> ElementId {
    ElementId::Name(format!("chat-search-chats-{}", suffix.into()).into())
}

fn normalized_query(query: &str) -> String {
    trimming_whitespaces_and_newlines(query).to_lowercase()
}

fn separator(theme: &AppTheme) -> Div {
    div().w_full().h(px(SEPARATOR_HEIGHT)).bg(theme.border._100)
}

fn is_failed(state: RiseConversationAuxiliaryLoadState) -> bool {
    matches!(state, RiseConversationAuxiliaryLoadState::Failed)
}

/// stale cached preview, sender the server couldnt name then
/// server row wins if its last message is newer, a call keeps outcome
/// no preview in cache means last message deleted
pub enum ChatSearchRowFreshness {}

impl ChatSearchRowFreshness {
    pub fn index(server: &[ChatInfo]) -> HashMap<String, ChatInfo> {
        let mut index = HashMap::with_capacity(server.len());
        for chat in server {
            index.entry(chat.id.clone()).or_insert_with(|| chat.clone());
        }
        index
    }

    pub fn fresher(cached: ChatInfo, server: Option<&ChatInfo>) -> ChatInfo {
        let Some(server) = server else {
            return cached;
        };
        if cached.last_message.is_none() {
            return cached;
        }
        if server
            .last_message
            .as_ref()
            .is_some_and(|message| RiseChatRetractedPreviews::contains(&server.id, &message.id))
        {
            return cached;
        }
        if server.last_message.as_ref().is_some_and(|message| {
            matches!(message.delivery_status, MessageDeliveryStatus::Sending | MessageDeliveryStatus::Failed)
        }) {
            return cached;
        }
        let cached_at = cached.last_message.as_ref().map_or(0.0, |message| message.created_at);
        let server_at = server.last_message.as_ref().map_or(0.0, |message| message.created_at);
        if server_at <= cached_at {
            return cached;
        }
        let mut server = server.clone();
        if let (Some(known), Some(message)) = (cached.last_message.as_ref().and_then(|message| message.call_info.as_ref()), server.last_message.as_mut())
            && message.call_info.is_none()
            && cached.last_message.as_ref().map(|cached| cached.id.as_str()) == Some(message.id.as_str())
        {
            message.call_info = Some(known.clone());
        }
        server
    }
}

fn merged_chats(cached: Vec<ChatInfo>, server: &[ChatInfo], locally_removed: HashSet<String>) -> Vec<ChatInfo> {
    let mut seen = locally_removed;
    let mut result = Vec::with_capacity(cached.len() + server.len());
    let server_by_id = ChatSearchRowFreshness::index(server);
    for chat in cached.into_iter().chain(server.iter().cloned()) {
        if seen.insert(chat.id.clone()) {
            let fresher_row = server_by_id.get(&chat.id);
            result.push(ChatSearchRowFreshness::fresher(chat, fresher_row));
        }
    }
    result
}

fn global_rows(rows: Vec<SearchEntityRow>, excluding: &[ChatInfo]) -> Vec<SearchEntityRow> {
    let chat_ids: HashSet<&str> = excluding.iter().map(|chat| chat.id.as_str()).collect();
    let user_ids: HashSet<&str> =
        excluding.iter().filter(|chat| !chat.participant.id.is_empty()).map(|chat| chat.participant.id.as_str()).collect();
    rows.into_iter()
        .filter(|row| {
            if row.chat_id.as_deref().is_some_and(|chat_id| chat_ids.contains(chat_id)) {
                return false;
            }
            !row.user_id.as_deref().is_some_and(|user_id| user_ids.contains(user_id))
        })
        .collect()
}

#[derive(Clone, PartialEq)]
enum ChatSearchChatsRow {
    SearchPrompt,
    ResultsSkeleton,
    LocalHeader,
    Local(usize, ChatInfo),
    ChatsPager(String),
    GlobalHeader(Option<String>),
    Global(SearchEntityRow),
    MessagesHeader,
    Hit(usize, RiseMessageSearchHit, Option<ChatInfo>),
    MessagesSkeleton,
    MessagesRetry,
    MessagesPager,
    MessagesNoResults,
    Failure,
    NoResults,
    Footer,
}

impl ChatSearchChatsRow {
    fn id(&self) -> String {
        match self {
            Self::SearchPrompt => "prompt".to_owned(),
            Self::ResultsSkeleton => "results-skeleton".to_owned(),
            Self::LocalHeader => "local-header".to_owned(),
            Self::Local(_, chat) => format!("chat-{}", chat.id),
            Self::ChatsPager(_) => "chats-pager".to_owned(),
            Self::GlobalHeader(_) => "global-header".to_owned(),
            Self::Global(row) => format!("global-{}", row.id),
            Self::MessagesHeader => "messages-header".to_owned(),
            Self::Hit(_, hit, _) => format!("hit-{}", hit.id),
            Self::MessagesSkeleton => "messages-skeleton".to_owned(),
            Self::MessagesRetry => "messages-retry".to_owned(),
            Self::MessagesPager => "messages-pager".to_owned(),
            Self::MessagesNoResults => "messages-no-results".to_owned(),
            Self::Failure => "failure".to_owned(),
            Self::NoResults => "no-results".to_owned(),
            Self::Footer => "footer".to_owned(),
        }
    }
}

#[derive(Clone)]
struct ChatSearchChatsParts {
    q: String,
    theme: AppTheme,
    view: WeakEntity<ChatSearchChatsTabView>,
    nav_store: Option<Entity<NavigationStore>>,
    context: Option<ChatsSearchContext>,
    live_source: Rc<Vec<ChatInfo>>,
    local_rows: Rc<Vec<ChatInfo>>,
    history_warmup: Entity<ChatListHistoryWarmupController>,
    message_chat_kind: ChatSearchChatKindFilter,
    messages: Rc<RiseMessageSearchPresentationState>,
    selected_row_id: Option<String>,
}

impl ChatSearchChatsParts {
    fn push(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    /// a pushed destination leaves the overlay mounted under it
    /// coming back shows the same query, tab and scroll position
    /// only focus and tab bar flag are released
    fn leave_overlay_for_push(&self, window: &mut Window, cx: &mut App) {
        ScreenFocus::release_text_field(window, cx);
        ChatSearchOverlayPresenter::shared(cx).update(cx, |presenter, cx| presenter.set_is_open(false, cx));
    }

    fn dismiss_overlay(&self, window: &mut Window, cx: &mut App) {
        if let Some(context) = &self.context {
            (context.set_search_overlay)(false, window, cx);
            (context.set_query)(String::new(), window, cx);
        }
        ChatSearchOverlayPresenter::shared(cx).update(cx, |presenter, cx| presenter.set_is_open(false, cx));
    }

    fn open_chat(&self, chat: &ChatInfo, window: &mut Window, cx: &mut App) {
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.record_chat_search_recent_chat(chat, cx));
        let is_virtual = UserServicesStore::is_contact_join_virtual_chat_id(&chat.id);
        if chat.topics_enabled && !is_virtual {
            self.dismiss_overlay(window, cx);
        } else {
            self.leave_overlay_for_push(window, cx);
        }
        if is_virtual {
            let nav_store = self.nav_store.clone();
            UserInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.open_contact_join_virtual_chat(
                    chat,
                    move |chat_id, cx| Self::push(&nav_store, RootRoute::Chat { chat_id: Some(chat_id) }, cx),
                    |message, cx| {
                        let _ = Notifier::send(
                            tr("contact_join_chat_open_failed"),
                            NotifierSendOptions {
                                content: Some(message),
                                r#type: NotifierType::Error,
                                position: NotifierPosition::Bottom,
                                ..NotifierSendOptions::default()
                            },
                            cx,
                        );
                    },
                    cx,
                );
            });
            return;
        }
        if chat.topics_enabled {
            EmbeddedForumPresentationStore::shared(cx).update(cx, |store, cx| store.present(chat.clone(), cx));
        } else {
            Self::push(&self.nav_store, RootRoute::Chat { chat_id: Some(chat.id.clone()) }, cx);
        }
    }

    fn open_entity(&self, row: &SearchEntityRow, window: &mut Window, cx: &mut App) {
        let Some(route) = GlobalSearchRouteResolver::route(row) else {
            return;
        };
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.record_chat_search_recent_entity(row, cx));
        if let RootRoute::Chat { chat_id: Some(chat_id) } = &route {
            ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.note_community_opening_row(row, chat_id, cx));
        }
        self.leave_overlay_for_push(window, cx);
        Self::push(&self.nav_store, route, cx);
    }

    /// opens the hit chat with the message as travel target, same hand off the profile tabs use
    fn open_message_hit(&self, hit: &RiseMessageSearchHit, chat: Option<&ChatInfo>, window: &mut Window, cx: &mut App) {
        let chat_id = trimming_whitespaces_and_newlines(&hit.message.chat_id).to_owned();
        if chat_id.is_empty() {
            return;
        }
        if let Some(chat) = chat {
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.record_chat_search_recent_chat(chat, cx));
        }
        ChatMessageJumpPresenter::shared(cx)
            .update(cx, |presenter, cx| presenter.enqueue(&chat_id, None, hit.message.topic_id.as_deref(), &hit.id, cx));
        self.leave_overlay_for_push(window, cx);
        Self::push(&self.nav_store, RootRoute::Chat { chat_id: Some(chat_id) }, cx);
    }

    fn retry(&self, cx: &mut App) {
        let q = self.q.clone();
        let kind = self.message_chat_kind;
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.search_chat_overlay_entities(&q, None, cx);
            store.search_chat_overlay_chats(&q, None, cx);
            store.search_chat_overlay_messages(&q, kind, cx);
        });
    }

    fn load_more_messages(&self, cx: &mut App) {
        let (q, kind) = (self.q.clone(), self.message_chat_kind);
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_chat_overlay_messages(&q, kind, cx));
    }

    fn load_newer_messages(&self, cx: &mut App) {
        let (q, kind) = (self.q.clone(), self.message_chat_kind);
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.load_newer_chat_overlay_messages(&q, kind, cx));
    }

    fn search_state(&self, text_key: &'static str) -> AnyElement {
        vstack(6.0)
            .w_full()
            .pt(px(60.0))
            .children(IconUi::sized("magnifyingglass", px(30.0), self.theme.text.secondary, false))
            .child(MainText::new(tr(text_key)).px(13.0).tac(TextAlignT::Center).color(self.theme.text.secondary))
            .into_any_element()
    }

    fn results_skeleton(&self) -> AnyElement {
        let entity_rows = (0..3).map(|index: i64| {
            vstack(0.0)
                .w_full()
                .child(
                    hstack(10.0)
                        .w_full()
                        .px(px(12.0))
                        .py(px(8.0))
                        .child(SkeletonCircle::new(40.0))
                        .child(
                            vstack(7.0)
                                .align(HorizontalAlignment::Leading)
                                .flex_1()
                                .min_w_0()
                                .child(SkeletonLine::new(if index % 2 == 0 { 132.0_f32 } else { 98.0_f32 }, 9.0))
                                .child(SkeletonLine::new(76.0_f32, 7.0)),
                        ),
                )
                .child(separator(&self.theme))
        });
        let chat_rows = (0..3).map(|index: i64| {
            vstack(0.0).w_full().child(ChatItemSkeleton::new(index)).child(separator(&self.theme))
        });
        vstack(0.0).w_full().children(entity_rows).children(chat_rows).skeleton_shimmer(None).into_any_element()
    }

    fn global_header(&self, right_title: Option<String>) -> AnyElement {
        let view = self.view.clone();
        ChatSearchSectionHeader::new(tab_id("global-header"), tr("global_search_section_global").to_uppercase())
            .right_title(right_title)
            .on_right(move |_, cx| {
                let _ = view.update(cx, |view, cx| {
                    view.is_global_expanded = !view.is_global_expanded;
                    view.row_animation = Some(ease_in_out(GLOBAL_TOGGLE_DURATION));
                    view.rows_dirty = true;
                    cx.notify();
                });
            })
            .into_any_element()
    }

    fn local_row(&self, index: usize, chat: &ChatInfo) -> AnyElement {
        let is_virtual = UserServicesStore::is_contact_join_virtual_chat_id(&chat.id);
        let press_parts = self.clone();
        let pressed = chat.clone();
        let context = self.context.clone();
        let source = self.live_source.clone();
        let chat_id = chat.id.clone();
        let warmup = self.history_warmup.clone();
        let local_rows = self.local_rows.clone();
        vstack(0.0)
            .w_full()
            .child(
                ChatItem::new(tab_id(format!("chat-{}", chat.id)), chat.clone())
                    .on_press(move |window, cx| press_parts.open_chat(&pressed, window, cx))
                    .on_hold_action(move |action, window, cx| {
                        if let Some(context) = &context {
                            (context.apply_hold_action)(chat_id.as_str(), action, source.as_slice(), window, cx);
                        }
                    })
                    .custom_hold_actions(is_virtual.then(Vec::new))
                    .on_appear(tab_id(format!("chat-appear-{}", chat.id)), move |_, cx| {
                        warmup.update(cx, |warmup, cx| warmup.update(WARMUP_PAGE_ID, index as i64, &local_rows, cx));
                    }),
            )
            .child(separator(&self.theme))
            .into_any_element()
    }

    fn global_entity_row(&self, row: &SearchEntityRow) -> AnyElement {
        let action_parts = self.clone();
        let action_row = row.clone();
        let tap_parts = self.clone();
        let tap_row = row.clone();
        let content_row = row.clone();
        let theme = self.theme.clone();
        vstack(0.0)
            .w_full()
            .child(div().w_full().flex().flex_col().child(
                HoldContextMenuContainer::new(
                    tab_id(format!("global-hold-{}", row.id)),
                    HoldContextMenuAlignment::Leading,
                    Rc::new(|_: &App| {
                        vec![
                            HoldContextMenuAction::new("open", tr("ctx_open")).system_image("arrow.up.right.square"),
                            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
                        ]
                    }),
                    Rc::new(move |_: &mut Window, _: &mut App| Self::global_entity_content(&content_row, &theme)),
                )
                .show_emoji_list(false)
                .on_action(Rc::new(move |action: String, window: &mut Window, cx: &mut App| {
                    if action == "open" {
                        action_parts.open_entity(&action_row, window, cx);
                    } else if action == "copy" {
                        let text = if action_row.tag_line.is_empty() { &action_row.display_name } else { &action_row.tag_line };
                        UIPasteboard::general().set_string(text.as_str(), cx);
                    }
                }))
                .on_tap(Rc::new(move |_: gpui::Point<gpui::Pixels>, window: &mut Window, cx: &mut App| {
                    tap_parts.open_entity(&tap_row, window, cx);
                })),
            ))
            .child(separator(&self.theme))
            .into_any_element()
    }

    fn global_entity_content(row: &SearchEntityRow, theme: &AppTheme) -> AnyElement {
        let logo = if row.logo_url.is_empty() { None } else { Some(SharedString::from(row.logo_url.clone())) };
        let tag_line = (!row.tag_line.is_empty()).then(|| MainText::new(row.tag_line.clone()).px(12.0).color(theme.text.secondary));
        hstack(10.0)
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .bg(theme.bg._100)
            .child(
                UserLogoUi::new(tab_id(format!("global-logo-{}", row.id)))
                    .source(logo)
                    .name(SharedString::from(row.display_name.clone()))
                    .user_id(row.user_id.clone().map(SharedString::from))
                    .color_scope_id(row.chat_id.clone().map(SharedString::from))
                    .size(40.0),
            )
            .child(
                vstack(2.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(
                        hstack(6.0)
                            .child(MainText::new(row.display_name.clone()).px(15.0).font_weight(FontWeight::SEMIBOLD))
                            .children(row.is_bot_user().then(|| BotIcon::new(theme.text.primary).size(12.0)))
                            .children(row.is_official.then(|| OfficialBadge::new().size(px(15.0))))
                            .children(row.is_premium.then(|| {
                                PremiumBadge::new()
                                    .user_id(row.user_id.clone().map(SharedString::from))
                                    .emoji(row.premium_badge_emoji.clone().map(SharedString::from))
                                    .size(15.0)
                            })),
                    )
                    .children(tag_line),
            )
            .into_any_element()
    }

    fn messages_header(&self) -> AnyElement {
        hstack(4.0)
            .w_full()
            .px(px(16.0))
            .py(px(8.0))
            .child(
                MainText::new(tr("chat_search_section_messages").to_uppercase())
                    .px(12.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(self.theme.text.secondary),
            )
            .child(div().flex_1())
            .child(self.message_chat_kind_menu())
            .into_any_element()
    }

    fn message_chat_kind_menu(&self) -> ContextMenuUi {
        let items = ChatSearchChatKindFilter::ALL_CASES
            .into_iter()
            .enumerate()
            .map(|(index, kind)| {
                let view = self.view.clone();
                ContextMenuItem::new(index as i64, tr(kind.title_key()), None, kind.raw_value(), move |_, cx| {
                    let _ = view.update(cx, |view, cx| view.set_message_chat_kind(kind, cx));
                })
            })
            .collect();
        let title_key =
            if self.message_chat_kind == ChatSearchChatKindFilter::All { "chat_search_from_all_chats" } else { self.message_chat_kind.title_key() };
        ContextMenuUi::new(
            tab_id("kind-menu"),
            items,
            hstack(4.0)
                .child(MainText::new(tr(title_key)).px(12.0).font_weight(FontWeight::SEMIBOLD).color(self.theme.primary._100))
                .children(IconUi::sized("chevron.down", px(11.0), self.theme.primary._100, false)),
        )
        .selected(self.message_chat_kind.raw_value().to_owned())
    }

    fn hit_row(&self, index: usize, hit: &RiseMessageSearchHit, chat: Option<&ChatInfo>) -> AnyElement {
        let tap_parts = self.clone();
        let tap_hit = hit.clone();
        let tap_chat = chat.cloned();
        let appear_parts = self.clone();
        let row_chat = chat.cloned().or_else(|| hit.chat.clone());
        vstack(0.0)
            .w_full()
            .child(
                ChatMessageSearchResultRow::new(
                    tab_id(format!("hit-{}", hit.id)),
                    hit.clone(),
                    row_chat,
                    false,
                    self.theme.clone(),
                    move |window, cx| tap_parts.open_message_hit(&tap_hit, tap_chat.as_ref(), window, cx),
                )
                .on_appear(tab_id(format!("hit-appear-{}", hit.id)), move |_, cx| {
                    let state = appear_parts.messages.clone();
                    let visible = index as i64;
                    if ChatMessageSearchPolicy::should_load_more(visible, state.hits.len() as i64, state.has_more, state.is_loading()) {
                        appear_parts.load_more_messages(cx);
                    }
                    if ChatMessageSearchPolicy::should_load_newer(visible, state.has_newer, state.is_loading()) {
                        appear_parts.load_newer_messages(cx);
                    }
                }),
            )
            .child(div().w_full().pl(px(ChatMessageSearchRowMetrics::SEPARATOR_INSET)).child(separator(&self.theme)))
            .into_any_element()
    }

    fn messages_retry_row(&self) -> AnyElement {
        let parts = self.clone();
        hstack(8.0)
            .w_full()
            .px(px(16.0))
            .py(px(10.0))
            .child(div().flex_1().min_w_0().child(MainText::new(tr("error_fetching_data")).px(13.0).color(self.theme.text.secondary)))
            .child(spacer_min_w(8.0).flex_none())
            .child(div().flex_none().child(system_button(tab_id("messages-retry"), BorderedStyle::Bordered).child(SharedString::from(tr("common_retry"))).on_press(
                move |_, cx| {
                    let state = parts.messages.clone();
                    if state.has_more {
                        parts.load_more_messages(cx);
                    } else if state.has_newer {
                        parts.load_newer_messages(cx);
                    } else {
                        let (q, kind) = (parts.q.clone(), parts.message_chat_kind);
                        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.search_chat_overlay_messages(&q, kind, cx));
                    }
                },
            )))
            .into_any_element()
    }

    fn row(&self, row: &ChatSearchChatsRow) -> AnyElement {
        let content = match row {
            ChatSearchChatsRow::SearchPrompt => self.search_state("chat_search_type_to_find_chats"),
            ChatSearchChatsRow::ResultsSkeleton | ChatSearchChatsRow::MessagesSkeleton => self.results_skeleton(),
            ChatSearchChatsRow::LocalHeader => ChatSearchSectionHeader::new(
                tab_id("local-header"),
                tr("chat_search_section_chats_contacts").to_uppercase(),
            )
            .into_any_element(),
            ChatSearchChatsRow::Local(index, chat) => self.local_row(*index, chat),
            ChatSearchChatsRow::ChatsPager(cursor) => {
                let (q, cursor) = (self.q.clone(), cursor.clone());
                div()
                    .w_full()
                    .h(px(1.0))
                    .on_appear(tab_id(format!("chats-pager-{cursor}")), move |_, cx| {
                        MessageInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.search_chat_overlay_chats(&q, Some(cursor.as_str()), cx));
                    })
                    .into_any_element()
            }
            ChatSearchChatsRow::GlobalHeader(right_title) => self.global_header(right_title.clone()),
            ChatSearchChatsRow::Global(row) => self.global_entity_row(row),
            ChatSearchChatsRow::MessagesHeader => self.messages_header(),
            ChatSearchChatsRow::Hit(index, hit, chat) => self.hit_row(*index, hit, chat.as_ref()),
            ChatSearchChatsRow::MessagesRetry => self.messages_retry_row(),
            ChatSearchChatsRow::MessagesPager => {
                let parts = self.clone();
                ChatItemSkeleton::new(0)
                    .skeleton_shimmer(None)
                    .on_appear(tab_id("messages-pager"), move |_, cx| parts.load_more_messages(cx))
                    .into_any_element()
            }
            ChatSearchChatsRow::MessagesNoResults => div()
                .w_full()
                .px(px(16.0))
                .py(px(10.0))
                .child(MainText::new(tr("chat_search_no_results")).px(13.0).color(self.theme.text.secondary))
                .into_any_element(),
            ChatSearchChatsRow::Failure => {
                let parts = self.clone();
                ChatSearchErrorState::new(tab_id("failure-retry"), move |_, cx| parts.retry(cx)).into_any_element()
            }
            ChatSearchChatsRow::NoResults => self.search_state("chat_search_no_results"),
            ChatSearchChatsRow::Footer => div().w_full().h(px(BOTTOM_PADDING)).into_any_element(),
        };
        let is_selected = self.selected_row_id.as_deref() == Some(row.id().as_str());
        let selected_tint = self.theme.primary._100.opacity(SELECTED_ROW_TINT);
        env_scope::<NavigationStoreKey>(self.nav_store.clone())
            .child(
                div()
                    .relative()
                    .w_full()
                    .child(content)
                    .children(is_selected.then(|| div().absolute().inset_0().bg(selected_tint))),
            )
            .into_any_element()
    }

    fn keyboard_targets(&self, rows: &[ListShownRow<ChatSearchChatsRow>]) -> Vec<ChatSearchKeyboardTarget> {
        rows.iter()
            .enumerate()
            .filter(|(_, shown)| !shown.is_leaving)
            .filter_map(|(list_index, shown)| {
                let row = &shown.row;
                let parts = self.clone();
                let activate: Rc<dyn Fn(&mut Window, &mut App)> = match row {
                    ChatSearchChatsRow::Local(_, chat) => {
                        let chat = chat.clone();
                        Rc::new(move |window: &mut Window, cx: &mut App| parts.open_chat(&chat, window, cx))
                    }
                    ChatSearchChatsRow::Global(entity) => {
                        let entity = entity.clone();
                        Rc::new(move |window: &mut Window, cx: &mut App| parts.open_entity(&entity, window, cx))
                    }
                    ChatSearchChatsRow::Hit(_, hit, chat) => {
                        let (hit, chat) = (hit.clone(), chat.clone());
                        Rc::new(move |window: &mut Window, cx: &mut App| parts.open_message_hit(&hit, chat.as_ref(), window, cx))
                    }
                    _ => return None,
                };
                Some(ChatSearchKeyboardTarget { row_id: row.id(), list_index, activate })
            })
            .collect()
    }
}

/// one chat, entity or message hit row as its own cached view
/// a wheel step notifies the tab and rows only moved
/// so they replay instead of laying out every visible item
struct ChatSearchRowView {
    row: ChatSearchChatsRow,
    parts: Rc<ChatSearchChatsParts>,
    is_selected: bool,
    height: Option<Pixels>,
    list: WeakEntity<ChatSearchChatsTabView>,
}

impl ChatSearchRowView {
    fn is_cached_kind(row: &ChatSearchChatsRow) -> bool {
        matches!(row, ChatSearchChatsRow::Local(..) | ChatSearchChatsRow::Global(_) | ChatSearchChatsRow::Hit(..))
    }

    fn element(view: &Entity<Self>, cx: &App) -> AnyElement {
        let height = view.read(cx).height;
        let view = AnyView::from(view.clone());
        match height {
            Some(height) => {
                view.cached(StyleRefinement::default().w_full().h(height)).replay_when_moved().into_any_element()
            }
            None => view.into_any_element(),
        }
    }

    fn set_row(&mut self, row: ChatSearchChatsRow, parts: Rc<ChatSearchChatsParts>, cx: &mut Context<Self>) {
        let is_selected = parts.selected_row_id.as_deref() == Some(row.id().as_str());
        let redraws = self.row != row || self.is_selected != is_selected;
        self.row = row;
        self.parts = parts;
        self.is_selected = is_selected;
        if redraws {
            self.height = None;
            cx.notify();
        }
    }

    fn set_height(&mut self, height: Pixels, cx: &mut Context<Self>) {
        let prior = self.height.replace(height);
        if prior.is_some_and(|prior| prior != height) {
            self.list.update(cx, |_, cx| cx.notify()).ok();
        }
    }
}

impl Render for ChatSearchRowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        div().relative().w_full().child(self.parts.row(&self.row)).child(measure(
            tab_id(format!("row-size-{}", self.row.id())),
            move |bounds, _, cx| {
                this.update(cx, |view, cx| view.set_height(bounds.size.height, cx)).ok();
            },
        ))
    }
}

fn push_messages_section(
    rows: &mut Vec<ChatSearchChatsRow>,
    state: &RiseMessageSearchPresentationState,
    q: &str,
    is_filtered: bool,
    chat_index: &HashMap<&str, &ChatInfo>,
    shows_empty_note: bool,
) {
    let failed = is_failed(state.load_state);
    let shows_section = !state.hits.is_empty() || state.is_loading() || is_filtered || (failed && shows_empty_note);
    if q.is_empty() || !shows_section {
        return;
    }
    rows.push(ChatSearchChatsRow::MessagesHeader);
    for (index, hit) in state.hits.iter().enumerate() {
        let chat = chat_index.get(hit.message.chat_id.as_str()).map(|chat| (*chat).clone());
        rows.push(ChatSearchChatsRow::Hit(index, hit.clone(), chat));
    }
    if state.is_loading() && state.hits.is_empty() {
        rows.push(ChatSearchChatsRow::MessagesSkeleton);
    } else if failed {
        if !state.hits.is_empty() || shows_empty_note {
            rows.push(ChatSearchChatsRow::MessagesRetry);
        }
    } else if state.has_more {
        rows.push(ChatSearchChatsRow::MessagesPager);
    } else if state.hits.is_empty() && state.did_load() && shows_empty_note {
        rows.push(ChatSearchChatsRow::MessagesNoResults);
    }
}

/// the view behind the chats tab, state of the tab and its lazy list
pub struct ChatSearchChatsTabView {
    query: String,
    last_q: String,
    is_global_expanded: bool,
    message_chat_kind: ChatSearchChatKindFilter,
    searched_messages_key: Option<String>,
    task_key: Option<String>,
    history_warmup: Entity<ChatListHistoryWarmupController>,
    nav_store: Option<Entity<NavigationStore>>,
    context: Option<ChatsSearchContext>,
    keyboard: Option<Entity<ChatSearchResultsKeyboard>>,
    list: ListUi,
    row_transitions: ListRowTransitions<ChatSearchChatsRow>,
    row_animation: Option<MotionSpec>,
    chats_page: RiseChatOverlaySearchPage<ChatInfo>,
    global_page: RiseChatOverlaySearchPage<SearchEntityRow>,
    messages: Rc<RiseMessageSearchPresentationState>,
    shown_local_rows: Rc<Vec<ChatInfo>>,
    rows_dirty: bool,
    frame: Option<Rc<ListRowsFrame<ChatSearchChatsRow>>>,
    parts: Option<Rc<ChatSearchChatsParts>>,
    row_views: Rc<HashMap<String, Entity<ChatSearchRowView>>>,
    _subscriptions: Vec<Subscription>,
}

impl ChatSearchChatsTabView {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseConversationPresentation::shared(cx);
        let services = ChatServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            query: String::new(),
            last_q: String::new(),
            is_global_expanded: false,
            message_chat_kind: ChatSearchChatKindFilter::All,
            searched_messages_key: None,
            task_key: None,
            history_warmup: cx.new(|_| ChatListHistoryWarmupController::new()),
            nav_store: None,
            context: None,
            keyboard: None,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            row_transitions: ListRowTransitions::new(),
            row_animation: None,
            chats_page: RiseChatOverlaySearchPage::empty(),
            global_page: RiseChatOverlaySearchPage::empty(),
            messages: Rc::new(RiseMessageSearchPresentationState::empty()),
            shown_local_rows: Rc::new(Vec::new()),
            rows_dirty: true,
            frame: None,
            parts: None,
            row_views: Rc::new(HashMap::new()),
            _subscriptions: vec![
                cx.subscribe(&presentation, |view, _, change: &RiseConversationPresentationChange, cx| {
                    if matches!(
                        change,
                        RiseConversationPresentationChange::ChatOverlaySearch
                            | RiseConversationPresentationChange::MessageSearch
                            | RiseConversationPresentationChange::All
                    ) {
                        view.refresh_search_pages(cx);
                        view.rows_dirty = true;
                        cx.notify();
                    }
                    if matches!(change, RiseConversationPresentationChange::ChatList(RiseChatListKind::Inbox))
                        && !normalized_query(&view.query).is_empty()
                    {
                        let (_, local_rows) = view.local_sources(false, cx);
                        if local_rows != *view.shown_local_rows {
                            view.rows_dirty = true;
                            cx.notify();
                        }
                    }
                }),
                cx.observe(&services, |view, _, cx| {
                    view.rows_dirty = true;
                    cx.notify();
                }),
                cx.observe(&theme_store, |view, _, cx| {
                    view.row_views = Rc::new(HashMap::new());
                    view.rows_dirty = true;
                    cx.notify();
                }),
            ],
        }
    }

    fn sync(
        &mut self,
        query: String,
        nav_store: Option<Entity<NavigationStore>>,
        context: Option<ChatsSearchContext>,
        keyboard: Option<Entity<ChatSearchResultsKeyboard>>,
        cx: &mut Context<Self>,
    ) {
        self.nav_store = nav_store;
        self.context = context;
        if keyboard.as_ref().map(Entity::entity_id) != self.keyboard.as_ref().map(Entity::entity_id) {
            if let Some(keyboard) = &keyboard {
                self._subscriptions.push(cx.observe(keyboard, |view, keyboard, cx| {
                    if let Some(index) = keyboard.read(cx).selected_list_index() {
                        view.list.state().scroll_to_reveal_item(index);
                    }
                    view.rows_dirty = true;
                    cx.notify();
                }));
            }
            self.keyboard = keyboard;
        }
        if query != self.query {
            self.query = query;
            let q = normalized_query(&self.query);
            if q != self.last_q {
                self.last_q = q;
                self.is_global_expanded = false;
            }
            self.refresh_search_pages(cx);
            self.rows_dirty = true;
            cx.notify();
        }
        self.run_messages_search(cx);
    }

    /// live inbox and local hits merged with the chats page, as render shows them
    fn local_sources(&self, q_is_empty: bool, cx: &mut Context<Self>) -> (Rc<Vec<ChatInfo>>, Vec<ChatInfo>) {
        let services = ChatServicesStore::shared(cx);
        let live_source = Rc::new(services.read(cx).current_chat_infos(cx));
        let mut locally_removed = services.read(cx).locally_removed_chat_ids();
        locally_removed.extend(RiseConversationPresentation::shared(cx).read(cx).confirmed_removed_chat_ids().iter().cloned());
        let cached_rows = match (&self.context, q_is_empty) {
            (Some(context), false) => (context.visible_chats)(self.query.as_str(), live_source.as_slice(), cx),
            _ => Vec::new(),
        };
        let local_rows = merged_chats(cached_rows, &self.chats_page.items, locally_removed);
        (live_source, local_rows)
    }

    fn refresh_search_pages(&mut self, cx: &mut Context<Self>) {
        let q = normalized_query(&self.query);
        let kind = self.message_chat_kind;
        let store = ChatSearchOverlayStore::shared(cx);
        self.chats_page = store.update(cx, |store, cx| store.chats_page(&q, cx));
        self.global_page = store.update(cx, |store, cx| store.entities_page(&q, cx));
        self.messages = Rc::new(store.update(cx, |store, cx| store.messages_state(&q, kind, cx)));
    }

    /// runs the messages search for the query and chat kind
    fn run_messages_search(&mut self, cx: &mut Context<Self>) {
        let q = normalized_query(&self.query);
        let kind = self.message_chat_kind;
        let key = format!("{q}|{}", kind.raw_value());
        if self.task_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.task_key = Some(key.clone());
        if q.is_empty() {
            self.searched_messages_key = None;
            return;
        }
        if self.searched_messages_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.searched_messages_key = Some(key);
        cx.defer(move |cx| {
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.search_chat_overlay_messages(&q, kind, cx));
        });
    }

    fn set_message_chat_kind(&mut self, kind: ChatSearchChatKindFilter, cx: &mut Context<Self>) {
        self.message_chat_kind = kind;
        self.refresh_search_pages(cx);
        self.run_messages_search(cx);
        self.rows_dirty = true;
        cx.notify();
    }
}

impl ChatSearchChatsTabView {
    fn rebuild_rows(
        &mut self,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Rc<ListRowsFrame<ChatSearchChatsRow>>, Rc<ChatSearchChatsParts>) {
        let q = normalized_query(&self.query);
        let (live_source, local_rows) = self.local_sources(q.is_empty(), cx);
        let local_rows = Rc::new(local_rows);
        self.shown_local_rows = local_rows.clone();
        let chats_page = self.chats_page.clone();
        let global_page = self.global_page.clone();
        let kind = self.message_chat_kind;
        let messages = self.messages.clone();

        let global_did_load = global_page.did_load();
        let has_error = global_page.error.is_some() || chats_page.error.is_some();
        let filtered_global = global_rows(global_page.items, &local_rows);
        let global_limit = RiseChatSearchResultsPolicy::global_limit(local_rows.len() as i64, self.is_global_expanded);
        let visible_global: Vec<SearchEntityRow> =
            filtered_global.iter().take(global_limit.max(0) as usize).cloned().collect();

        let is_loading =
            !q.is_empty() && local_rows.is_empty() && (!chats_page.did_load() || (filtered_global.is_empty() && !global_did_load));

        let mut rows = Vec::new();
        if q.is_empty() {
            rows.push(ChatSearchChatsRow::SearchPrompt);
        } else if is_loading {
            rows.push(ChatSearchChatsRow::ResultsSkeleton);
        } else {
            if !local_rows.is_empty() {
                rows.push(ChatSearchChatsRow::LocalHeader);
                rows.extend(local_rows.iter().cloned().enumerate().map(|(index, chat)| ChatSearchChatsRow::Local(index, chat)));
                if chats_page.has_more
                    && let Some(cursor) = chats_page.cursor.clone()
                {
                    rows.push(ChatSearchChatsRow::ChatsPager(cursor));
                }
            }
            if !visible_global.is_empty() {
                let shows_toggle =
                    filtered_global.len() as i64 > RiseChatSearchResultsPolicy::collapsed_global_limit() && !local_rows.is_empty();
                let right_title =
                    shows_toggle.then(|| tr(if self.is_global_expanded { "common_show_less" } else { "common_show_more" }));
                rows.push(ChatSearchChatsRow::GlobalHeader(right_title));
                rows.extend(visible_global.iter().cloned().map(ChatSearchChatsRow::Global));
            }
            let mut chat_index: HashMap<&str, &ChatInfo> = HashMap::with_capacity(live_source.len());
            for chat in live_source.iter() {
                chat_index.entry(chat.id.as_str()).or_insert(chat);
            }
            push_messages_section(
                &mut rows,
                &messages,
                &q,
                kind != ChatSearchChatKindFilter::All,
                &chat_index,
                !visible_global.is_empty() || !local_rows.is_empty(),
            );
            if visible_global.is_empty() && local_rows.is_empty() && messages.hits.is_empty() && !messages.is_loading() {
                rows.push(if has_error || is_failed(messages.load_state) {
                    ChatSearchChatsRow::Failure
                } else {
                    ChatSearchChatsRow::NoResults
                });
            }
        }
        rows.push(ChatSearchChatsRow::Footer);
        let row_animation = self.row_animation.take();
        let frame = self.row_transitions.sync(
            &self.list,
            rows.into_iter().map(|row| (row.id(), row)).collect(),
            row_animation,
            RiseClock::monotonic(),
            window,
            cx,
        );

        let selected_row_id = self.keyboard.as_ref().and_then(|keyboard| keyboard.read(cx).selected_row_id().map(str::to_owned));
        let parts = ChatSearchChatsParts {
            q,
            theme: theme.clone(),
            view: cx.entity().downgrade(),
            nav_store: self.nav_store.clone(),
            context: self.context.clone(),
            live_source,
            local_rows,
            history_warmup: self.history_warmup.clone(),
            message_chat_kind: kind,
            messages,
            selected_row_id,
        };
        if let Some(keyboard) = &self.keyboard {
            let targets = parts.keyboard_targets(frame.rows());
            keyboard.update(cx, |keyboard, cx| keyboard.publish(targets, cx));
        }
        let parts = Rc::new(parts);
        let mut old_views = Rc::try_unwrap(std::mem::take(&mut self.row_views)).unwrap_or_else(|shared| (*shared).clone());
        let mut views = HashMap::new();
        let list_view = cx.entity().downgrade();
        for shown in frame.rows().iter().filter(|shown| ChatSearchRowView::is_cached_kind(&shown.row)) {
            let view = match old_views.remove(&shown.id) {
                Some(view) => {
                    view.update(cx, |view, cx| view.set_row(shown.row.clone(), parts.clone(), cx));
                    view
                }
                None => {
                    let (row, row_parts, list) = (shown.row.clone(), parts.clone(), list_view.clone());
                    let is_selected = parts.selected_row_id.as_deref() == Some(shown.id.as_str());
                    cx.new(move |_| ChatSearchRowView { row, parts: row_parts, is_selected, height: None, list })
                }
            };
            views.insert(shown.id.clone(), view);
        }
        self.row_views = Rc::new(views);
        let frame = Rc::new(frame);
        self.frame = Some(frame.clone());
        self.parts = Some(parts.clone());
        (frame, parts)
    }
}

impl Render for ChatSearchChatsTabView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let reused = match (&self.frame, &self.parts) {
            (Some(frame), Some(parts))
                if !self.rows_dirty && self.row_animation.is_none() && !self.row_transitions.is_animating() =>
            {
                Some((frame.clone(), parts.clone()))
            }
            _ => None,
        };
        let (frame, parts) = match reused {
            Some(reused) => reused,
            None => {
                self.rows_dirty = false;
                self.rebuild_rows(&theme, window, cx)
            }
        };
        let row_views = self.row_views.clone();

        let warmup_on = self.history_warmup.clone();
        let warmup_off = self.history_warmup.clone();
        let bottom_margin = scroll_content_bottom_margin(cx);
        let list = self
            .list
            .element(move |index, _, cx| {
                let Some(shown) = frame.rows().get(index) else {
                    return Empty.into_any_element();
                };
                let content = match row_views.get(&shown.id) {
                    Some(view) => ChatSearchRowView::element(view, cx),
                    None => parts.row(&shown.row),
                };
                frame.element(&shown.id, content)
            })
            .pb(px(bottom_margin));
        div()
            .size_full()
            .bg(theme.bg._100)
            .child(list)
            .on_appear(tab_id("warmup-activate"), move |_, cx| {
                warmup_on.update(cx, |warmup, cx| warmup.activate(WARMUP_PAGE_ID, cx));
            })
            .on_disappear(tab_id("warmup-cancel"), move |_, cx| {
                warmup_off.update(cx, |warmup, cx| warmup.cancel_all(cx));
            })
    }
}

#[derive(IntoElement)]
pub struct ChatSearchChatsTab {
    query: String,
}

impl ChatSearchChatsTab {
    pub fn new(query: impl Into<String>) -> Self {
        Self { query: query.into() }
    }
}

impl RenderOnce for ChatSearchChatsTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let context = env::<ChatsSearchContextKey>(cx);
        let keyboard = env::<ChatSearchResultsKeyboardKey>(cx);
        let view = window.use_keyed_state(tab_id("view"), cx, |_, cx| ChatSearchChatsTabView::new(cx));
        let query = self.query;
        view.update(cx, |view, cx| view.sync(query, nav_store, context, keyboard, cx));
        AnyView::from(view).cached(StyleRefinement::default().size_full())
    }
}
