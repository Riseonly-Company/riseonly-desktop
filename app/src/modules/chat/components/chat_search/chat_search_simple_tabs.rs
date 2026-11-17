use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, App, Context, ElementId, Empty, Entity, FontWeight, Hsla, ListAlignment, Pixels, Point, SharedString,
    StyleRefinement, Subscription, Task, Window, rgb,
};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::IconUi;
use rise_ui::bot_icon::BotIcon;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::prelude::*;

use crate::app::router::in_app_link_browser::InAppLinkBrowser;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::{ChatScrollPositionStore, ChatScrollPositionStorePosition};
use crate::modules::chat::components::chat_item::chat_item::ChatItem;
use crate::modules::chat::components::chat_search::chat_search_chats_tab::ChatSearchRowFreshness;
use crate::modules::chat::components::chat_search::chat_search_global_search_hooks::ChatSearchGlobalSearchHooks;
use crate::modules::chat::components::chat_search::chat_search_overlay_presenter::ChatSearchOverlayPresenter;
use crate::modules::chat::components::chat_search::chat_search_overlay_store::{ChatSearchMessageKind, ChatSearchOverlayStore};
use crate::modules::chat::components::chat_search::chat_search_section::{
    ChatSearchErrorState, ChatSearchSectionHeader, chat_search_sync_list,
};
use crate::modules::chat::components::chat_search::chat_search_tabs_catalog::{ChatSearchActiveTabKey, ChatSearchTabsCatalog};
use crate::modules::chat::components::chat_search::chats_search_context::{ChatsSearchContext, ChatsSearchContextKey};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileFormatter;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::RiseConversationAuxiliaryLoadState;
use crate::modules::chat::engine::core::rise_chat_overlay_search_presentation::RiseChatOverlaySearchPage;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseConversationPresentation, RiseConversationPresentationChange,
};
use crate::modules::chat::engine::core::rise_global_message_search_presentation::{
    RiseGlobalMessageSearchPresentationState, RiseGlobalMessageSearchResult,
};
use crate::modules::chat::shared::chat_message_jump_presenter::ChatMessageJumpPresenter;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageMediaKind};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::post::engine::rise_post_engine_models::RisePostPageStatus;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::search::engine::rise_search_engine_models::{RiseGlobalSearchCategory, RiseGlobalSearchRow};
use crate::modules::search::engine::rise_search_presentation::RiseSearchPresentation;
use crate::modules::search::shared::global_search_route_resolver::GlobalSearchRouteResolver;
use crate::modules::search::stores::search_actions::search_types::SearchEntityRow;
use crate::modules::search::stores::search_interactions::search_interactions::SearchInteractionsStore;
use crate::modules::search::stores::search_services::search_services::SearchServicesStore;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

/// gpui list cant disable scroll so a layer eats the wheel while a hold menu is up
/// open flag is a flip only observer, presenter reads rerender every tab
fn scroll_disabled_under_hold_menu(list: impl IntoElement, hold_is_active: bool) -> Div {
    div().relative().size_full().child(list).children(hold_is_active.then(|| div().absolute().inset_0().occlude()))
}

const SEARCH_DELAY: Duration = Duration::from_millis(300);
const SEPARATOR_HEIGHT: f32 = 0.5;
const BOTTOM_PADDING: f32 = 80.0;
const PROMPT_ICON_SIDE: f32 = 30.0;
const PROMPT_SPACING: f32 = 6.0;
const PROMPT_TOP_PADDING: f32 = 60.0;
const LOAD_MORE_THRESHOLD: usize = 8;
const TYPED_TAIL_STATE_HEIGHT: f32 = 52.0;
const GRID_COLUMNS: usize = 3;
const GRID_SPACING: f32 = 2.0;
const ANCHOR_VIEWPORT_OFFSET: f32 = 120.0;
const ANCHOR_DISTANCE_FROM_TOP: f32 = 1.0;
const FALLBACK_FILE_EXT: &str = "FILE";
const GIF_BADGE: &str = "GIF";

fn tab_id(suffix: impl Into<String>) -> ElementId {
    ElementId::Name(format!("chat-search-simple-{}", suffix.into()).into())
}

fn trimmed_query(query: &str) -> String {
    trimming_whitespaces_and_newlines(query).to_owned()
}

/// presentation changes that carry the overlay search pages
fn overlay_search_changed(change: &RiseConversationPresentationChange) -> bool {
    matches!(change, RiseConversationPresentationChange::ChatOverlaySearch | RiseConversationPresentationChange::All)
}

fn is_failed(state: RiseConversationAuxiliaryLoadState) -> bool {
    matches!(state, RiseConversationAuxiliaryLoadState::Failed)
}

fn separator(theme: &AppTheme) -> Div {
    div().w_full().h(px(SEPARATOR_HEIGHT)).bg(theme.border._100)
}

fn dot(theme: &AppTheme) -> Div {
    div().size(px(2.0)).rounded_full().bg(theme.text.secondary.opacity(0.4))
}

fn prompt(icon: &str, text: String, theme: &AppTheme) -> AnyElement {
    vstack(PROMPT_SPACING)
        .w_full()
        .pt(px(PROMPT_TOP_PADDING))
        .children(IconUi::sized(icon, px(PROMPT_ICON_SIDE), theme.text.secondary, icon.ends_with(".fill")))
        .child(MainText::new(text).px(13.0).tac(TextAlignT::Center).color(theme.text.secondary))
        .into_any_element()
}

fn close_overlay(cx: &mut App) {
    ChatSearchOverlayPresenter::shared(cx).update(cx, |presenter, cx| presenter.set_is_open(false, cx));
}

fn push_route(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

fn chat_route(chat_id: String) -> RootRoute {
    RootRoute::Chat { chat_id: Some(chat_id) }
}

#[derive(Clone)]
enum ChatSearchChannelsRow {
    ChannelsHeader,
    Channel(ChatInfo),
    GlobalHeader,
    Entity(usize, SearchEntityRow),
    LoadFailed,
    Tail(String),
    Footer,
}

impl ChatSearchChannelsRow {
    fn id(&self) -> String {
        match self {
            Self::ChannelsHeader => "channels-header".to_owned(),
            Self::Channel(chat) => format!("channel-{}", chat.id),
            Self::GlobalHeader => "global-header".to_owned(),
            Self::Entity(_, row) => format!("entity-{}", row.id),
            Self::LoadFailed => "load-failed".to_owned(),
            Self::Tail(_) => "tail".to_owned(),
            Self::Footer => "footer".to_owned(),
        }
    }
}

#[derive(Clone)]
struct ChatSearchChannelsParts {
    q: String,
    theme: AppTheme,
    nav_store: Option<Entity<NavigationStore>>,
    context: Option<ChatsSearchContext>,
    live_source: Rc<Vec<ChatInfo>>,
    entity_count: usize,
    has_more: bool,
}

/// people and channels, the person own channels come first
/// found among their chats by title. a private channel has no handle so global search cant reach it
#[derive(IntoElement)]
pub struct ChatSearchChannelsTab {
    query: String,
}

impl ChatSearchChannelsTab {
    pub fn new(query: impl Into<String>) -> Self {
        Self { query: query.into() }
    }

    fn row(parts: &ChatSearchChannelsParts, row: &ChatSearchChannelsRow) -> AnyElement {
        let content = match row {
            ChatSearchChannelsRow::ChannelsHeader => {
                ChatSearchSectionHeader::new(tab_id("channels-header"), tr("chat_search_tab_channels").to_uppercase()).into_any_element()
            }
            ChatSearchChannelsRow::Channel(chat) => Self::channel_row(parts, chat),
            ChatSearchChannelsRow::GlobalHeader => {
                ChatSearchSectionHeader::new(tab_id("global-header"), tr("global_search_section_global").to_uppercase()).into_any_element()
            }
            ChatSearchChannelsRow::Entity(index, entity) => {
                let index = *index;
                let appear_parts = parts.clone();
                vstack(0.0)
                    .w_full()
                    .child(Self::entity_row(parts, entity).on_appear(tab_id(format!("entity-appear-{}", entity.id)), move |_, cx| {
                        if index >= appear_parts.entity_count.saturating_sub(LOAD_MORE_THRESHOLD) {
                            Self::load_more_entities(&appear_parts.q, appear_parts.has_more, cx);
                        }
                    }))
                    .child(separator(&parts.theme))
                    .into_any_element()
            }
            ChatSearchChannelsRow::LoadFailed => {
                let retry_parts = parts.clone();
                ChatSearchErrorState::new(tab_id("load-failed-retry"), move |_, cx| {
                    if retry_parts.has_more {
                        Self::load_more_entities(&retry_parts.q, retry_parts.has_more, cx);
                    } else {
                        Self::retry(&retry_parts.q, cx);
                    }
                })
                .into_any_element()
            }
            ChatSearchChannelsRow::Tail(revision) => {
                let appear_parts = parts.clone();
                UserListSkeleton::new()
                    .rows(1)
                    .on_appear(tab_id(format!("tail-{revision}")), move |_, cx| {
                        Self::load_more_entities(&appear_parts.q, appear_parts.has_more, cx);
                    })
                    .into_any_element()
            }
            ChatSearchChannelsRow::Footer => div().w_full().h(px(BOTTOM_PADDING)).into_any_element(),
        };
        env_scope::<NavigationStoreKey>(parts.nav_store.clone()).child(content).into_any_element()
    }

    fn channel_row(parts: &ChatSearchChannelsParts, chat: &ChatInfo) -> AnyElement {
        let press_chat = chat.clone();
        let nav_store = parts.nav_store.clone();
        let context = parts.context.clone();
        let source = parts.live_source.clone();
        let chat_id = chat.id.clone();
        vstack(0.0)
            .w_full()
            .child(
                ChatItem::new(tab_id(format!("channel-{}", chat.id)), chat.clone())
                    .on_press(move |window, cx| Self::open_channel(&press_chat, &nav_store, window, cx))
                    .on_hold_action(move |action, window, cx| {
                        if let Some(context) = &context {
                            (context.apply_hold_action)(chat_id.as_str(), action, source.as_slice(), window, cx);
                        }
                    }),
            )
            .child(separator(&parts.theme))
            .into_any_element()
    }

    fn entity_row(parts: &ChatSearchChannelsParts, row: &SearchEntityRow) -> AnyElement {
        let theme = &parts.theme;
        let open_row = row.clone();
        let nav_store = parts.nav_store.clone();
        let logo = if row.logo_url.is_empty() { None } else { Some(SharedString::from(row.logo_url.clone())) };
        let tag_line = (!row.tag_line.is_empty()).then(|| MainText::new(row.tag_line.clone()).px(12.0).color(theme.text.secondary));
        plain(tab_id(format!("entity-{}", row.id)))
            .w_full()
            .on_press(move |_, cx| Self::open_entity(&open_row, &nav_store, cx))
            .child(
                hstack(10.0)
                    .w_full()
                    .px(px(12.0))
                    .py(px(8.0))
                    .bg(theme.bg._100)
                    .child(
                        UserLogoUi::new(tab_id(format!("entity-logo-{}", row.id)))
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
                                    .children(row.is_bot_user().then(|| BotIcon::new(theme.text.primary).size(12.0))),
                            )
                            .children(tag_line),
                    ),
            )
            .into_any_element()
    }

    fn own_channels(query: &str, cached: &[ChatInfo], server: &[ChatInfo], locally_removed: HashSet<String>) -> Vec<ChatInfo> {
        let needle = query.to_lowercase();
        let mut seen = locally_removed;
        let mut result = Vec::new();
        let server_by_id: HashMap<String, ChatInfo> = ChatSearchRowFreshness::index(server);
        for chat in cached {
            if matches!(chat.r#type, ChatType::Channel) && chat.title.to_lowercase().contains(&needle) && seen.insert(chat.id.clone()) {
                result.push(ChatSearchRowFreshness::fresher(chat.clone(), server_by_id.get(&chat.id)));
            }
        }
        for chat in server {
            if matches!(chat.r#type, ChatType::Channel) && seen.insert(chat.id.clone()) {
                result.push(chat.clone());
            }
        }
        result
    }

    fn open_channel(chat: &ChatInfo, nav_store: &Option<Entity<NavigationStore>>, window: &mut Window, cx: &mut App) {
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.record_chat_search_recent_chat(chat, cx));
        ScreenFocus::release_text_field(window, cx);
        close_overlay(cx);
        push_route(nav_store, chat_route(chat.id.clone()), cx);
    }

    /// a channel opens itself like in the chats tab, someone not subscribed gets the join button there
    /// a person opens the profile, their chat id is the user chat handle and not a chat
    fn open_entity(row: &SearchEntityRow, nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        let Some(route) = GlobalSearchRouteResolver::route(row) else {
            return;
        };
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.record_chat_search_recent_entity(row, cx));
        close_overlay(cx);
        if let RootRoute::Chat { chat_id: Some(chat_id) } = &route {
            ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.note_community_opening_row(row, chat_id, cx));
        }
        push_route(nav_store, route, cx);
    }

    fn load_more_entities(query: &str, has_more: bool, cx: &mut App) {
        if !has_more {
            return;
        }
        SearchInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.load_more_global_search(query, RiseGlobalSearchCategory::Channels, cx));
    }

    fn activate_channel_search(query: &str, cx: &mut App) {
        SearchInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.activate_global_search(query, RiseGlobalSearchCategory::Channels, cx));
    }

    fn retry(query: &str, cx: &mut App) {
        Self::activate_channel_search(query, cx);
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.search_chat_overlay_chats(query, None, cx));
    }

    fn empty_search_prompt(theme: &AppTheme) -> AnyElement {
        prompt("person.2.fill", tr("chat_search_type_to_find_people_channels"), theme)
    }

    fn empty_state(theme: &AppTheme) -> AnyElement {
        prompt("magnifyingglass", tr("chat_search_no_results"), theme)
    }
}

pub struct ChatSearchChannelsTabView {
    query: String,
    nav_store: Option<Entity<NavigationStore>>,
    context: Option<ChatsSearchContext>,
    list: ListUi,
    list_ids: Vec<String>,
    task_q: Option<String>,
    search_task: Option<Task<()>>,
    server_chats: Vec<ChatInfo>,
    hold_menu_active: bool,
    _subscriptions: Vec<Subscription>,
}

impl ChatSearchChannelsTabView {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseConversationPresentation::shared(cx);
        let services = ChatServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let search_presentation = RiseSearchPresentation::shared(cx);
        Self {
            query: String::new(),
            nav_store: None,
            context: None,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            task_q: None,
            search_task: None,
            server_chats: Vec::new(),
            hold_menu_active: hold_context_menu.read(cx).active_request().is_some(),
            _subscriptions: vec![
                cx.observe(&hold_context_menu, |view, menu, cx| {
                    let active = menu.read(cx).active_request().is_some();
                    if active != view.hold_menu_active {
                        view.hold_menu_active = active;
                        cx.notify();
                    }
                }),
                cx.subscribe(&presentation, |view, _, change: &RiseConversationPresentationChange, cx| {
                    if overlay_search_changed(change) {
                        view.refresh_server_chats(cx);
                        cx.notify();
                    }
                }),
                cx.observe(&search_presentation, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn sync(
        &mut self,
        query: String,
        nav_store: Option<Entity<NavigationStore>>,
        context: Option<ChatsSearchContext>,
        cx: &mut Context<Self>,
    ) {
        self.nav_store = nav_store;
        self.context = context;
        if trimmed_query(&query) != trimmed_query(&self.query) {
            self.list.state().reset(0);
            self.list_ids.clear();
        }
        if query != self.query {
            self.query = query;
            self.refresh_server_chats(cx);
            cx.notify();
        }
        self.run_search_task(cx);
    }

    fn refresh_server_chats(&mut self, cx: &mut Context<Self>) {
        let q = trimmed_query(&self.query);
        self.server_chats = if q.is_empty() {
            Vec::new()
        } else {
            ChatSearchOverlayStore::shared(cx).update(cx, |store, cx| store.chats_page(&q, cx)).items
        };
    }

    fn run_search_task(&mut self, cx: &mut Context<Self>) {
        let q = trimmed_query(&self.query);
        if self.task_q.as_deref() == Some(q.as_str()) {
            return;
        }
        self.task_q = Some(q.clone());
        self.search_task = None;
        if q.is_empty() {
            return;
        }
        self.search_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            let _ = this.update(cx, |_, cx| ChatSearchChannelsTab::activate_channel_search(&q, cx));
        }));
    }
}

impl Render for ChatSearchChannelsTabView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let q = trimmed_query(&self.query);
        let page = SearchServicesStore::shared(cx)
            .update(cx, |services, cx| services.global_search_page(&q, RiseGlobalSearchCategory::Channels, cx));
        let services = ChatServicesStore::shared(cx);
        let live_source = Rc::new(if q.is_empty() { Vec::new() } else { services.read(cx).current_chat_infos(cx) });
        let channels = if q.is_empty() {
            Vec::new()
        } else {
            let locally_removed = services.read(cx).locally_removed_chat_ids();
            ChatSearchChannelsTab::own_channels(&q, &live_source, &self.server_chats, locally_removed)
        };
        let channel_ids: HashSet<&str> = channels.iter().map(|chat| chat.id.as_str()).collect();
        let entities: Vec<SearchEntityRow> = page
            .items
            .iter()
            .filter_map(|row| match row {
                RiseGlobalSearchRow::Channel(row) => Some(row),
                RiseGlobalSearchRow::Post(_) | RiseGlobalSearchRow::Profile(_) | RiseGlobalSearchRow::Group(_) => None,
            })
            .filter(|row| !row.chat_id.as_deref().is_some_and(|chat_id| channel_ids.contains(chat_id)))
            .cloned()
            .collect();
        let load_failed = page.error_message.is_some() && !page.items.is_empty();
        let is_loading = page.is_refreshing || page.is_loading_more;
        let parts = ChatSearchChannelsParts {
            q: q.clone(),
            theme: theme.clone(),
            nav_store: self.nav_store.clone(),
            context: self.context.clone(),
            live_source,
            entity_count: entities.len(),
            has_more: page.has_more,
        };

        let content = if q.is_empty() {
            ChatSearchChannelsTab::empty_search_prompt(&theme)
        } else if channels.is_empty() && entities.is_empty() {
            let skeleton = || div().size_full().overflow_hidden().child(UserListSkeleton::new());
            match page.status {
                RisePostPageStatus::Idle | RisePostPageStatus::Loading => skeleton().into_any_element(),
                RisePostPageStatus::Failed => {
                    let retry_q = q.clone();
                    ChatSearchErrorState::new(tab_id("rejected-retry"), move |_, cx| ChatSearchChannelsTab::retry(&retry_q, cx))
                        .into_any_element()
                }
                RisePostPageStatus::Loaded => {
                    if is_loading {
                        skeleton().into_any_element()
                    } else if page.has_more && page.error_message.is_none() {
                        let empty_q = q.clone();
                        skeleton()
                            .on_appear(tab_id(format!("empty-tail-{}", page.revision)), move |_, cx| {
                                ChatSearchChannelsTab::load_more_entities(&empty_q, true, cx);
                            })
                            .into_any_element()
                    } else {
                        ChatSearchChannelsTab::empty_state(&theme)
                    }
                }
            }
        } else {
            let mut rows = Vec::with_capacity(channels.len() + entities.len() + 5);
            if !channels.is_empty() {
                rows.push(ChatSearchChannelsRow::ChannelsHeader);
                rows.extend(channels.iter().cloned().map(ChatSearchChannelsRow::Channel));
            }
            if !entities.is_empty() {
                rows.push(ChatSearchChannelsRow::GlobalHeader);
                rows.extend(entities.iter().cloned().enumerate().map(|(index, row)| ChatSearchChannelsRow::Entity(index, row)));
            }
            if load_failed {
                rows.push(ChatSearchChannelsRow::LoadFailed);
            } else if page.has_more && !page.items.is_empty() {
                rows.push(ChatSearchChannelsRow::Tail(page.items.len().to_string()));
            }
            rows.push(ChatSearchChannelsRow::Footer);
            chat_search_sync_list(&self.list, &mut self.list_ids, rows.iter().map(ChatSearchChannelsRow::id).collect());
            let list = self.list.element(move |index, _, _| {
                let Some(row) = rows.get(index) else {
                    return Empty.into_any_element();
                };
                ChatSearchChannelsTab::row(&parts, row)
            });
            scroll_disabled_under_hold_menu(list, self.hold_menu_active).into_any_element()
        };
        div().size_full().bg(theme.bg._100).child(content)
    }
}

impl RenderOnce for ChatSearchChannelsTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let context = env::<ChatsSearchContextKey>(cx);
        let view = window.use_keyed_state(tab_id("channels-view"), cx, |_, cx| ChatSearchChannelsTabView::new(cx));
        let query = self.query;
        view.update(cx, |view, cx| view.sync(query, nav_store, context, cx));
        AnyView::from(view).cached(StyleRefinement::default().size_full())
    }
}

#[derive(Clone)]
struct ChatSearchPostsParts {
    q: String,
    me: String,
    nav_store: Option<Entity<NavigationStore>>,
    items: Rc<Vec<GetPostFeedResponse>>,
    has_more: bool,
    cursor: Option<String>,
    cell_side: f32,
}

#[derive(IntoElement)]
pub struct ChatSearchPostsTab {
    query: String,
}

impl ChatSearchPostsTab {
    pub fn new(query: impl Into<String>) -> Self {
        Self { query: query.into() }
    }

    fn cell(parts: &ChatSearchPostsParts, index: usize) -> AnyElement {
        let Some(post) = parts.items.get(index) else {
            return Empty.into_any_element();
        };
        let post = Rc::new(post.clone());
        let actions = Self::overlay_post_hold_actions(&post, &parts.me);
        let content_post = post.clone();
        let action_post = post.clone();
        let action_nav = parts.nav_store.clone();
        let tap_post = post.clone();
        let tap_nav = parts.nav_store.clone();
        let appear_parts = parts.clone();
        let cell_side = parts.cell_side;
        HoldContextMenuContainer::new(
            tab_id(format!("post-hold-{}", post.id)),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| {
                div()
                    .size(px(cell_side))
                    .child(ChatSearchGlobalSearchHooks::grid_post_cell_or_default(&content_post))
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            Self::handle_post_action(&action_post, &id, &action_nav, cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| Self::open_post(&tap_post, &tap_nav, cx)))
        .on_appear(tab_id(format!("post-appear-{}", post.id)), move |_, cx| {
            let count = appear_parts.items.len();
            let Some(cursor) = appear_parts.cursor.as_deref().filter(|_| appear_parts.has_more) else {
                return;
            };
            if index >= count.saturating_sub(LOAD_MORE_THRESHOLD) {
                MessageInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.search_chat_overlay_posts(&appear_parts.q, Some(cursor), cx));
            }
        })
        .into_any_element()
    }

    fn row(parts: &ChatSearchPostsParts, cells: &Range<usize>) -> AnyElement {
        let spacing = if cells.end < parts.items.len() { GRID_SPACING } else { 0.0 };
        let fillers = GRID_COLUMNS - cells.len();
        hstack(GRID_SPACING)
            .w_full()
            .pb(px(spacing))
            .children(cells.clone().map(|index| div().flex_1().min_w_0().h(px(parts.cell_side)).child(Self::cell(parts, index))))
            .children((0..fillers).map(|_| div().flex_1().min_w_0().h(px(parts.cell_side))))
            .into_any_element()
    }

    fn open_post(post: &GetPostFeedResponse, nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        close_overlay(cx);
        push_route(nav_store, RootRoute::PostDetail { post_id: Some(post.id.clone()), origin: None }, cx);
    }

    fn overlay_post_hold_actions(post: &GetPostFeedResponse, me: &str) -> Vec<HoldContextMenuAction> {
        let mine = post.author.id.as_deref() == Some(me) || post.author_id == me;
        let mut items = vec![
            HoldContextMenuAction::new("open", tr("ctx_open")).system_image("arrow.up.right.square"),
            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
            HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"),
        ];
        if mine {
            items.push(HoldContextMenuAction::new("delete", tr("ctx_delete")).system_image("trash").destructive(true));
        } else {
            items.push(HoldContextMenuAction::new("report", tr("ctx_report")).system_image("exclamationmark.bubble").destructive(true));
        }
        items
    }

    fn handle_post_action(post: &GetPostFeedResponse, id: &str, nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        match id {
            "open" => Self::open_post(post, nav_store, cx),
            "copy" => {
                let link = RiseonlyPublicLink::Post { id: post.id.clone() };
                if let Some(url) = link.web_url(AppEnvironment::current()) {
                    UIPasteboard::general().set_string(url.absolute_string(), cx);
                }
                Notifier::copied(cx);
            }
            "share" => {
                let post = post.clone();
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_post(post, cx));
            }
            "delete" => {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.delete_own_post(post, None, cx));
            }
            "report" => {
                if let Some(post_id) = int_from_string(&post.id) {
                    ReportSheetStore::shared(cx).update(cx, |sheet, cx| sheet.present(ReportTarget::Post(post_id), None, None, cx));
                }
            }
            _ => {}
        }
    }

    fn empty_search_prompt(theme: &AppTheme) -> AnyElement {
        prompt("doc.text.magnifyingglass", tr("chat_search_type_to_find_posts"), theme)
    }
}

pub struct ChatSearchPostsTabView {
    query: String,
    task_q: Option<String>,
    search_task: Option<Task<()>>,
    nav_store: Option<Entity<NavigationStore>>,
    list: ListUi,
    list_ids: Vec<String>,
    container_width: f32,
    posts_page: RiseChatOverlaySearchPage<GetPostFeedResponse>,
    hold_menu_active: bool,
    _subscriptions: Vec<Subscription>,
}

impl ChatSearchPostsTabView {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseConversationPresentation::shared(cx);
        let post_services = PostServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        Self {
            query: String::new(),
            task_q: None,
            search_task: None,
            nav_store: None,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            container_width: 0.0,
            posts_page: RiseChatOverlaySearchPage::empty(),
            hold_menu_active: hold_context_menu.read(cx).active_request().is_some(),
            _subscriptions: vec![
                cx.observe(&hold_context_menu, |view, menu, cx| {
                    let active = menu.read(cx).active_request().is_some();
                    if active != view.hold_menu_active {
                        view.hold_menu_active = active;
                        cx.notify();
                    }
                }),
                cx.subscribe(&presentation, |view, _, change: &RiseConversationPresentationChange, cx| {
                    if overlay_search_changed(change) {
                        view.refresh_posts_page(cx);
                        cx.notify();
                    }
                }),
                cx.observe(&post_services, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn sync(&mut self, query: String, nav_store: Option<Entity<NavigationStore>>, cx: &mut Context<Self>) {
        self.nav_store = nav_store;
        if trimmed_query(&query) != trimmed_query(&self.query) {
            self.list.state().reset(0);
            self.list_ids.clear();
        }
        if query != self.query {
            self.query = query;
            self.refresh_posts_page(cx);
            cx.notify();
        }
        self.run_search_task(cx);
    }

    fn refresh_posts_page(&mut self, cx: &mut Context<Self>) {
        let q = trimmed_query(&self.query);
        self.posts_page = if q.is_empty() {
            RiseChatOverlaySearchPage::empty()
        } else {
            ChatSearchOverlayStore::shared(cx).update(cx, |store, cx| store.posts_page(&q, cx))
        };
    }

    fn run_search_task(&mut self, cx: &mut Context<Self>) {
        let q = trimmed_query(&self.query);
        if self.task_q.as_deref() == Some(q.as_str()) {
            return;
        }
        self.task_q = Some(q.clone());
        self.search_task = None;
        if q.is_empty() {
            return;
        }
        self.search_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            let _ = this.update(cx, |_, cx| {
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.search_chat_overlay_posts(&q, None, cx));
            });
        }));
    }
}

impl Render for ChatSearchPostsTabView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let q = trimmed_query(&self.query);
        let weak = cx.entity().downgrade();
        let on_disappear = move |_: Option<&mut Window>, cx: &mut App| {
            let _ = weak.update(cx, |view, _| {
                view.task_q = None;
                view.search_task = None;
            });
        };
        if q.is_empty() {
            return div()
                .size_full()
                .bg(theme.bg._100)
                .child(ChatSearchPostsTab::empty_search_prompt(&theme))
                .on_disappear(tab_id("posts-disappear"), on_disappear);
        }
        let page = self.posts_page.clone();
        let items: Vec<GetPostFeedResponse> = {
            let post_services = PostServicesStore::shared(cx);
            let post_services = post_services.read(cx);
            page.items.iter().filter(|post| !post_services.is_post_hidden(&post.id)).cloned().collect()
        };
        let me = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default();
        let is_empty = items.is_empty() && page.did_load();
        let cell_side = ((self.container_width - GRID_SPACING * (GRID_COLUMNS - 1) as f32) / GRID_COLUMNS as f32).max(0.0);
        let skeleton_cell_height = (cell_side > 0.0).then_some(cell_side);
        let measured = cx.entity().downgrade();

        let mut rows: Vec<Option<Range<usize>>> =
            (0..items.len()).step_by(GRID_COLUMNS).map(|start| Some(start..(start + GRID_COLUMNS).min(items.len()))).collect();
        rows.push(None);
        let ids = rows
            .iter()
            .map(|row| match row {
                Some(cells) => format!("post-{}", items[cells.start].id),
                None => "footer".to_owned(),
            })
            .collect();
        chat_search_sync_list(&self.list, &mut self.list_ids, ids);

        let parts = ChatSearchPostsParts {
            q: q.clone(),
            me,
            nav_store: self.nav_store.clone(),
            items: Rc::new(items),
            has_more: page.has_more,
            cursor: page.cursor.clone(),
            cell_side,
        };
        let list = self.list.element(move |index, _, _| match rows.get(index) {
            Some(Some(cells)) => ChatSearchPostsTab::row(&parts, cells),
            Some(None) => div().w_full().h(px(BOTTOM_PADDING)).into_any_element(),
            None => Empty.into_any_element(),
        });
        let list = scroll_disabled_under_hold_menu(list, self.hold_menu_active);
        let loading_bg = theme.bg._100;
        let retry_q = q.clone();
        div()
            .relative()
            .size_full()
            .bg(theme.bg._100)
            .child(
                SaiDataRender::new(page.status(), move |_, _| list)
                    .id(tab_id("posts-data"))
                    .is_empty(is_empty)
                    .no_data_text(tr("no_posts_found"))
                    .message_height_percent(40.0)
                    .no_data_height_percent(25.0)
                    .sai_data_render_loading(move |_, _| {
                        div()
                            .size_full()
                            .overflow_hidden()
                            .bg(loading_bg)
                            .child(PostGridSkeleton::default().cell_height(skeleton_cell_height).rows(4))
                    })
                    .sai_data_render_error(move |_, _| {
                        let q = retry_q.clone();
                        ChatSearchErrorState::new(tab_id("posts-retry"), move |_, cx| {
                            MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.search_chat_overlay_posts(&q, None, cx));
                        })
                    }),
            )
            .child(measure(tab_id("posts-geometry"), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                let _ = measured.update(cx, |view, cx| {
                    if view.container_width != width {
                        view.container_width = width;
                        view.list.state().remeasure();
                        cx.notify();
                    }
                });
            }))
            .on_disappear(tab_id("posts-disappear"), on_disappear)
    }
}

impl RenderOnce for ChatSearchPostsTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let view = window.use_keyed_state(tab_id("posts-view"), cx, |_, cx| ChatSearchPostsTabView::new(cx));
        let query = self.query;
        view.update(cx, |view, cx| view.sync(query, nav_store, cx));
        AnyView::from(view).cached(StyleRefinement::default().size_full())
    }
}

type ChatSearchTypedCell = fn(&Rc<ChatSearchTypedParts>, &RiseGlobalMessageSearchResult) -> AnyElement;
type ChatSearchTypedContent = fn(&RiseGlobalMessageSearchResult, &AppTheme) -> AnyElement;
type ChatSearchTypedAction = fn(&ChatSearchTypedParts, &RiseGlobalMessageSearchResult, &str, &mut App);
type ChatSearchTypedTap = fn(&ChatSearchTypedParts, &RiseGlobalMessageSearchResult, &mut App);

/// what a typed tab hands the scaffold, like the content builder of a generic
#[derive(Clone, Copy)]
struct ChatSearchTypedTabConfig {
    kind: ChatSearchMessageKind,
    prompt_icon: &'static str,
    columns: usize,
    cell: ChatSearchTypedCell,
}

struct ChatSearchTypedParts {
    kind: ChatSearchMessageKind,
    q: String,
    theme: AppTheme,
    nav_store: Option<Entity<NavigationStore>>,
    results: Arc<Vec<RiseGlobalMessageSearchResult>>,
    source_message_count: usize,
    cursor: Option<String>,
    has_more: bool,
    columns: usize,
    cell: ChatSearchTypedCell,
}

enum ChatSearchTypedRow {
    Cells(Range<usize>),
    PaginationSentinel,
    LoadingMore,
    LoadFailed,
    Footer,
}

impl ChatSearchTypedParts {
    fn row(self: &Rc<Self>, row: &ChatSearchTypedRow) -> AnyElement {
        match row {
            ChatSearchTypedRow::Cells(cells) => {
                if self.columns == 1 {
                    return self.paginated_cell(cells.start);
                }
                let spacing = if cells.end < self.results.len() { GRID_SPACING } else { 0.0 };
                let fillers = self.columns - cells.len();
                hstack(GRID_SPACING)
                    .w_full()
                    .pb(px(spacing))
                    .children(cells.clone().map(|index| div().flex_1().min_w_0().child(self.paginated_cell(index))))
                    .children((0..fillers).map(|_| div().flex_1().min_w_0()))
                    .into_any_element()
            }
            ChatSearchTypedRow::PaginationSentinel => {
                let count = self.source_message_count;
                self.chat_search_typed_pagination(div().w_full().h(px(1.0)).into_any_element(), count.saturating_sub(1), count)
            }
            ChatSearchTypedRow::LoadingMore => hstack(0.0)
                .w_full()
                .h(px(TYPED_TAIL_STATE_HEIGHT))
                .justify_center()
                .items_center()
                .child(progress_view().control_size(ControlSize::Small))
                .into_any_element(),
            ChatSearchTypedRow::LoadFailed => {
                let (kind, q, cursor) = (self.kind, self.q.clone(), self.cursor.clone());
                hstack(10.0)
                    .w_full()
                    .h(px(TYPED_TAIL_STATE_HEIGHT))
                    .px(px(16.0))
                    .justify_center()
                    .items_center()
                    .child(
                        MainText::new(tr("error_fetching_data"))
                            .px(13.0)
                            .color(self.theme.text.secondary)
                            .number_of_lines(1),
                    )
                    .child(
                        system_button(tab_id(format!("typed-page-retry-{}", kind.raw_value())), BorderedStyle::Bordered)
                            .on_press(move |_, cx| chat_search_typed_retry_next_page(kind, &q, cursor.as_deref(), cx))
                            .child(SharedString::from(tr("common_retry"))),
                    )
                    .into_any_element()
            }
            ChatSearchTypedRow::Footer => div().w_full().h(px(BOTTOM_PADDING)).into_any_element(),
        }
    }

    fn paginated_cell(self: &Rc<Self>, index: usize) -> AnyElement {
        let Some(entry) = self.results.get(index) else {
            return Empty.into_any_element();
        };
        self.chat_search_typed_pagination((self.cell)(self, entry), entry.position, self.results.len())
    }

    /// the id carries what the change watcher looks at
    /// so a row on screen evaluates again when count, cursor or end flag changes
    fn chat_search_typed_pagination(&self, element: AnyElement, index: usize, item_count: usize) -> AnyElement {
        let modifier = ChatSearchTypedPaginationModifier {
            index,
            item_count,
            scope: ChatSearchTypedPaginationGate::scope(self.kind, &self.q),
            cursor: self.cursor.clone(),
            has_more: self.has_more,
            kind: self.kind,
            q: self.q.clone(),
        };
        if !modifier.is_near_tail() {
            return element;
        }
        let id = format!(
            "typed-page-{}-{index}-{item_count}-{}-{}",
            self.kind.raw_value(),
            self.cursor.as_deref().unwrap_or_default(),
            self.has_more
        );
        element.on_appear(tab_id(id), move |_, cx| modifier.evaluate(cx)).into_any_element()
    }
}

#[derive(IntoElement)]
struct ChatSearchTypedTabScaffold {
    config: ChatSearchTypedTabConfig,
    query: String,
    prompt_text: String,
    no_data_text: String,
}

impl RenderOnce for ChatSearchTypedTabScaffold {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let config = self.config;
        let view = window.use_keyed_state(tab_id(format!("typed-view-{}", config.kind.raw_value())), cx, move |_, cx| {
            ChatSearchTypedTabView::new(config, cx)
        });
        let shown = env::<ChatSearchActiveTabKey>(cx)
            .is_none_or(|active| (active - ChatSearchTabsCatalog::typed_tab_id(config.kind)).abs() <= 1);
        let (query, prompt_text, no_data_text) = (self.query, self.prompt_text, self.no_data_text);
        view.update(cx, |view, cx| view.sync(query, prompt_text, no_data_text, nav_store, shown, cx));
        AnyView::from(view).cached(StyleRefinement::default().size_full())
    }
}

pub struct ChatSearchTypedTabView {
    config: ChatSearchTypedTabConfig,
    query: String,
    prompt_text: String,
    no_data_text: String,
    task_q: Option<String>,
    search_task: Option<Task<()>>,
    was_failed: bool,
    is_connected: bool,
    resume_after_failure: bool,
    nav_store: Option<Entity<NavigationStore>>,
    list: ListUi,
    list_ids: Vec<String>,
    presentation_key: (bool, u64, u64),
    search_state: RiseGlobalMessageSearchPresentationState,
    hold_menu_active: bool,
    _subscriptions: Vec<Subscription>,
}

impl ChatSearchTypedTabView {
    /// what the state reads from the presentation, the account and global search boxes
    fn presentation_key(presentation: &RiseConversationPresentation) -> (bool, u64, u64) {
        (
            presentation.is_active(),
            presentation.generation(),
            presentation.global_message_search_presentation_revision(),
        )
    }

    fn new(config: ChatSearchTypedTabConfig, cx: &mut Context<Self>) -> Self {
        let presentation = RiseConversationPresentation::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let socket = SaiWebSocketManager::shared(cx);
        let is_connected = socket.read(cx).is_connected();
        Self {
            config,
            query: String::new(),
            prompt_text: String::new(),
            no_data_text: String::new(),
            task_q: None,
            search_task: None,
            was_failed: false,
            is_connected,
            resume_after_failure: false,
            nav_store: None,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            presentation_key: Self::presentation_key(presentation.read(cx)),
            search_state: RiseGlobalMessageSearchPresentationState::empty(),
            hold_menu_active: hold_context_menu.read(cx).active_request().is_some(),
            _subscriptions: vec![
                cx.observe(&hold_context_menu, |view, menu, cx| {
                    let active = menu.read(cx).active_request().is_some();
                    if active != view.hold_menu_active {
                        view.hold_menu_active = active;
                        cx.notify();
                    }
                }),
                cx.observe(&presentation, |view, presentation, cx| {
                    let key = Self::presentation_key(presentation.read(cx));
                    if key == view.presentation_key {
                        return;
                    }
                    view.presentation_key = key;
                    let previous = view.search_state.clone();
                    view.refresh_search_state(cx);
                    view.resume_after_late_failure(&previous, cx);
                    if !Self::same_search_state(&previous, &view.search_state) {
                        cx.notify();
                    }
                }),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
                cx.observe(&socket, |view, socket, cx| {
                    let is_connected = socket.read(cx).is_connected();
                    if is_connected == view.is_connected {
                        return;
                    }
                    view.is_connected = is_connected;
                    view.resume_after_failure = is_connected;
                    if is_connected {
                        view.resume_next_page_after_reconnect(cx);
                    }
                }),
            ],
        }
    }

    fn sync(
        &mut self,
        query: String,
        prompt_text: String,
        no_data_text: String,
        nav_store: Option<Entity<NavigationStore>>,
        shown: bool,
        cx: &mut Context<Self>,
    ) {
        self.nav_store = nav_store;
        if !shown {
            self.run_search_task_for(trimmed_query(&query), cx);
            return;
        }
        let query_changed = trimmed_query(&query) != trimmed_query(&self.query);
        if query_changed {
            self.list.state().reset(0);
            self.list_ids.clear();
        }
        if query != self.query || prompt_text != self.prompt_text || no_data_text != self.no_data_text {
            self.query = query;
            self.prompt_text = prompt_text;
            self.no_data_text = no_data_text;
            cx.notify();
        }
        if query_changed {
            self.refresh_search_state(cx);
        }
        self.run_search_task(cx);
    }

    fn state(&self, q: &str, cx: &mut Context<Self>) -> RiseGlobalMessageSearchPresentationState {
        let kind = self.config.kind;
        MessageServicesStore::shared(cx).update(cx, |store, cx| store.global_message_search_state(kind, q, cx))
    }

    fn same_search_state(
        lhs: &RiseGlobalMessageSearchPresentationState,
        rhs: &RiseGlobalMessageSearchPresentationState,
    ) -> bool {
        let same_results =
            Arc::ptr_eq(&lhs.results, &rhs.results) || (lhs.results.is_empty() && rhs.results.is_empty());
        same_results
            && lhs.revision == rhs.revision
            && lhs.source_message_count == rhs.source_message_count
            && lhs.cursor == rhs.cursor
            && lhs.has_more == rhs.has_more
            && lhs.load_state == rhs.load_state
            && lhs.error == rhs.error
    }

    fn refresh_search_state(&mut self, cx: &mut Context<Self>) {
        let q = trimmed_query(&self.query);
        self.search_state = self.state(&q, cx);
    }

    fn run_search_task(&mut self, cx: &mut Context<Self>) {
        self.run_search_task_for(trimmed_query(&self.query), cx);
    }

    fn run_search_task_for(&mut self, q: String, cx: &mut Context<Self>) {
        if self.task_q.as_deref() == Some(q.as_str()) {
            return;
        }
        self.task_q = Some(q.clone());
        self.search_task = None;
        if q.is_empty() {
            return;
        }
        let kind = self.config.kind;
        self.search_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            let _ = this.update(cx, |_, cx| {
                ChatSearchTypedPaginationGate::reset(&ChatSearchTypedPaginationGate::scope(kind, &q));
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.search_global_messages(kind, &q, None, cx));
            });
        }));
    }

    fn resume_next_page_after_reconnect(&mut self, cx: &mut Context<Self>) {
        let q = trimmed_query(&self.query);
        if q.is_empty() {
            return;
        }
        let kind = self.config.kind;
        let current = self.state(&q, cx);
        let cursor = current.cursor.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or_default().to_owned();
        if !current.has_more || (current.results.is_empty() && current.source_message_count == 0) || cursor.is_empty() {
            return;
        }
        let scope = ChatSearchTypedPaginationGate::scope(kind, &q);
        if !is_failed(current.load_state) && !ChatSearchTypedPaginationGate::is_claimed(&scope, &cursor) {
            return;
        }
        ChatSearchTypedPaginationGate::reset(&scope);
        if !ChatSearchTypedPaginationGate::claim(&scope, &cursor) {
            return;
        }
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.search_global_messages(kind, &q, Some(cursor.as_str()), cx));
    }

    fn resume_after_late_failure(&mut self, previous: &RiseGlobalMessageSearchPresentationState, cx: &mut Context<Self>) {
        if !self.resume_after_failure {
            return;
        }
        if previous.cursor != self.search_state.cursor {
            self.resume_after_failure = false;
            return;
        }
        if !self.is_connected || !is_failed(self.search_state.load_state) || previous.revision == self.search_state.revision {
            return;
        }
        self.resume_after_failure = false;
        self.resume_next_page_after_reconnect(cx);
    }
}

impl Render for ChatSearchTypedTabView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let q = trimmed_query(&self.query);
        let kind = self.config.kind;
        let state = self.search_state.clone();
        let failed = is_failed(state.load_state);
        if failed && !self.was_failed && !q.is_empty() {
            ChatSearchTypedPaginationGate::reset(&ChatSearchTypedPaginationGate::scope(kind, &q));
        }
        self.was_failed = failed;

        let weak = cx.entity().downgrade();
        let on_disappear = move |_: Option<&mut Window>, cx: &mut App| {
            let _ = weak.update(cx, |view, _| {
                view.task_q = None;
                view.search_task = None;
            });
        };
        let disappear_id = tab_id(format!("typed-disappear-{}", kind.raw_value()));
        if q.is_empty() {
            return div()
                .size_full()
                .bg(theme.bg._100)
                .child(prompt(self.config.prompt_icon, self.prompt_text.clone(), &theme))
                .on_disappear(disappear_id, on_disappear);
        }

        let columns = self.config.columns;
        let results = state.results.clone();
        let mut rows: Vec<ChatSearchTypedRow> = (0..results.len())
            .step_by(columns)
            .map(|start| ChatSearchTypedRow::Cells(start..(start + columns).min(results.len())))
            .collect();
        if results.is_empty() && state.source_message_count > 0 {
            rows.push(ChatSearchTypedRow::PaginationSentinel);
        }
        if !results.is_empty() && state.has_more {
            rows.push(if failed { ChatSearchTypedRow::LoadFailed } else { ChatSearchTypedRow::LoadingMore });
        }
        rows.push(ChatSearchTypedRow::Footer);
        let ids = rows
            .iter()
            .map(|row| match row {
                ChatSearchTypedRow::Cells(cells) => format!("cell-{}", results[cells.start].id),
                ChatSearchTypedRow::PaginationSentinel => "pagination-sentinel".to_owned(),
                ChatSearchTypedRow::LoadingMore => "footer-loading".to_owned(),
                ChatSearchTypedRow::LoadFailed => "footer-failed".to_owned(),
                ChatSearchTypedRow::Footer => "footer".to_owned(),
            })
            .collect();
        chat_search_sync_list(&self.list, &mut self.list_ids, ids);

        let is_empty = state.results.is_empty() && state.did_load();
        let status = state.status();
        let parts = Rc::new(ChatSearchTypedParts {
            kind,
            q: q.clone(),
            theme: theme.clone(),
            nav_store: self.nav_store.clone(),
            results,
            source_message_count: state.source_message_count,
            cursor: state.cursor.clone(),
            has_more: state.has_more,
            columns,
            cell: self.config.cell,
        });
        let list = self.list.element(move |index, _, _| {
            let Some(row) = rows.get(index) else {
                return Empty.into_any_element();
            };
            parts.row(row)
        });
        let list = scroll_disabled_under_hold_menu(list, self.hold_menu_active);
        div()
            .size_full()
            .bg(theme.bg._100)
            .child(
                SaiDataRender::new(status, move |_, _| list)
                    .id(tab_id(format!("typed-data-{}", kind.raw_value())))
                    .is_empty(is_empty)
                    .no_data_text(self.no_data_text.clone())
                    .message_height_percent(40.0)
                    .is_skeleton(true)
                    .sai_data_render_error(move |_, _| {
                        let q = q.clone();
                        ChatSearchErrorState::new(tab_id(format!("typed-retry-{}", kind.raw_value())), move |_, cx| {
                            ChatSearchTypedPaginationGate::reset(&ChatSearchTypedPaginationGate::scope(kind, &q));
                            MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.search_global_messages(kind, &q, None, cx));
                        })
                    }),
            )
            .on_disappear(disappear_id, on_disappear)
    }
}

#[derive(Default)]
struct ChatSearchTypedPaginationGateState {
    cursor_by_scope: HashMap<String, String>,
    order: Vec<String>,
}

thread_local! {
    static CHAT_SEARCH_TYPED_PAGINATION_GATE: RefCell<ChatSearchTypedPaginationGateState> = RefCell::default();
}

enum ChatSearchTypedPaginationGate {}

impl ChatSearchTypedPaginationGate {
    fn is_claimed(scope: &str, cursor: &str) -> bool {
        let key = SaiAccountCacheScope::scoped_key(scope);
        CHAT_SEARCH_TYPED_PAGINATION_GATE.with_borrow(|gate| gate.cursor_by_scope.get(&key).is_some_and(|claimed| claimed == cursor))
    }

    fn claim(scope: &str, cursor: &str) -> bool {
        let key = SaiAccountCacheScope::scoped_key(scope);
        let limit = RiseLimits::client_value("chat_search_typed_pagination_gate_limit").max(0) as usize;
        CHAT_SEARCH_TYPED_PAGINATION_GATE.with_borrow_mut(|gate| {
            if gate.cursor_by_scope.get(&key).is_some_and(|claimed| claimed == cursor) {
                return false;
            }
            gate.cursor_by_scope.insert(key.clone(), cursor.to_owned());
            gate.order.retain(|existing| *existing != key);
            gate.order.push(key);
            while gate.order.len() > limit {
                let oldest = gate.order.remove(0);
                gate.cursor_by_scope.remove(&oldest);
            }
            true
        })
    }

    fn reset(scope: &str) {
        let key = SaiAccountCacheScope::scoped_key(scope);
        CHAT_SEARCH_TYPED_PAGINATION_GATE.with_borrow_mut(|gate| {
            gate.cursor_by_scope.remove(&key);
            gate.order.retain(|existing| *existing != key);
        });
    }

    fn scope(kind: ChatSearchMessageKind, query: &str) -> String {
        format!("{}:{}", kind.raw_value(), query.to_lowercase())
    }
}

struct ChatSearchTypedPaginationModifier {
    index: usize,
    item_count: usize,
    scope: String,
    cursor: Option<String>,
    has_more: bool,
    kind: ChatSearchMessageKind,
    q: String,
}

impl ChatSearchTypedPaginationModifier {
    fn is_near_tail(&self) -> bool {
        self.item_count > 0 && self.index < self.item_count && self.item_count - self.index <= LOAD_MORE_THRESHOLD
    }

    fn evaluate(&self, cx: &mut App) {
        if !self.is_near_tail() || !self.has_more {
            return;
        }
        let cursor = self.cursor.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if cursor.is_empty() || !ChatSearchTypedPaginationGate::claim(&self.scope, cursor) {
            return;
        }
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.search_global_messages(self.kind, &self.q, Some(cursor), cx));
    }
}

fn chat_search_typed_retry_next_page(kind: ChatSearchMessageKind, q: &str, cursor: Option<&str>, cx: &mut App) {
    let cursor = cursor.map(trimming_whitespaces_and_newlines).unwrap_or_default();
    if q.is_empty() || cursor.is_empty() {
        return;
    }
    let scope = ChatSearchTypedPaginationGate::scope(kind, q);
    ChatSearchTypedPaginationGate::reset(&scope);
    if !ChatSearchTypedPaginationGate::claim(&scope, cursor) {
        return;
    }
    MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.search_global_messages(kind, q, Some(cursor), cx));
}

fn chat_search_typed_hold_actions() -> Vec<HoldContextMenuAction> {
    vec![
        HoldContextMenuAction::new("goto", tr("ctx_go_to_message")).system_image("arrow.turn.up.left"),
        HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
    ]
}

fn chat_search_typed_hold(
    parts: &Rc<ChatSearchTypedParts>,
    entry: &RiseGlobalMessageSearchResult,
    content: ChatSearchTypedContent,
    on_action: ChatSearchTypedAction,
    on_tap: ChatSearchTypedTap,
) -> AnyElement {
    let entry = Rc::new(entry.clone());
    let theme = parts.theme.clone();
    let content_entry = entry.clone();
    let action_entry = entry.clone();
    let action_parts = parts.clone();
    let tap_parts = parts.clone();
    HoldContextMenuContainer::new(
        tab_id(format!("typed-hold-{}-{}", parts.kind.raw_value(), entry.id)),
        HoldContextMenuAlignment::Leading,
        Rc::new(|_: &App| chat_search_typed_hold_actions()),
        Rc::new(move |_: &mut Window, _: &mut App| content(&content_entry, &theme)),
    )
    .show_emoji_list(false)
    .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| on_action(&action_parts, &action_entry, &id, cx)))
    .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| on_tap(&tap_parts, &entry, cx)))
    .into_any_element()
}

/// a typed list row, the hold container with the separator below it
fn chat_search_typed_list_row(
    parts: &Rc<ChatSearchTypedParts>,
    entry: &RiseGlobalMessageSearchResult,
    content: ChatSearchTypedContent,
    on_action: ChatSearchTypedAction,
    on_tap: ChatSearchTypedTap,
) -> AnyElement {
    vstack(0.0)
        .w_full()
        .items_stretch()
        .child(chat_search_typed_hold(parts, entry, content, on_action, on_tap))
        .child(separator(&parts.theme))
        .into_any_element()
}

fn copy_media_url(entry: &RiseGlobalMessageSearchResult, cx: &mut App) {
    let url = entry.media_item.as_ref().map(|item| trimming_whitespaces_and_newlines(&item.url)).unwrap_or_default();
    if !url.is_empty() {
        UIPasteboard::general().set_string(url, cx);
        Notifier::copied(cx);
    }
}

fn prepare_chat_search_anchor(message: &GetMessageMessage, cx: &mut App) {
    let server_id = message.sai_server_id();
    let anchor_id = trimming_whitespaces_and_newlines(&server_id);
    if message.chat_id.is_empty() || anchor_id.is_empty() {
        return;
    }
    ChatScrollPositionStore::save_position(
        ChatScrollPositionStorePosition::new(Some(anchor_id.to_owned()), ANCHOR_VIEWPORT_OFFSET, ANCHOR_DISTANCE_FROM_TOP),
        &message.chat_id,
        false,
        cx,
    );
    ChatMessageJumpPresenter::shared(cx)
        .update(cx, |presenter, cx| presenter.enqueue(&message.chat_id, None, message.topic_id.as_deref(), anchor_id, cx));
}

fn open_chat_at_message(parts: &ChatSearchTypedParts, message: &GetMessageMessage, cx: &mut App) {
    if message.chat_id.is_empty() {
        return;
    }
    prepare_chat_search_anchor(message, cx);
    close_overlay(cx);
    push_route(&parts.nav_store, chat_route(message.chat_id.clone()), cx);
}

fn handle_media_action(parts: &ChatSearchTypedParts, entry: &RiseGlobalMessageSearchResult, id: &str, cx: &mut App) {
    match id {
        "goto" => open_chat_at_message(parts, &entry.message, cx),
        "copy" => copy_media_url(entry, cx),
        _ => {}
    }
}

fn typed_config(kind: ChatSearchMessageKind, prompt_icon: &'static str, columns: usize, cell: ChatSearchTypedCell) -> ChatSearchTypedTabConfig {
    ChatSearchTypedTabConfig { kind, prompt_icon, columns, cell }
}

#[derive(IntoElement)]
pub struct ChatSearchMediaTab {
    query: String,
}

impl ChatSearchMediaTab {
    pub fn new(query: impl Into<String>) -> Self {
        Self { query: query.into() }
    }

    fn cell(parts: &Rc<ChatSearchTypedParts>, entry: &RiseGlobalMessageSearchResult) -> AnyElement {
        if entry.media_item.is_none() {
            return Empty.into_any_element();
        }
        chat_search_typed_hold(parts, entry, Self::cell_content, Self::handle_action, Self::open_entry)
    }

    fn cell_content(entry: &RiseGlobalMessageSearchResult, _theme: &AppTheme) -> AnyElement {
        let Some(item) = &entry.media_item else {
            return Empty.into_any_element();
        };
        let badge = match item.kind {
            MessageMediaKind::Video => Some(Self::play_badge(item.duration)),
            MessageMediaKind::Animation | MessageMediaKind::Gif => Some(Self::gif_badge()),
            _ => None,
        };
        div()
            .relative()
            .w_full()
            .aspect_ratio(1.0)
            .overflow_hidden()
            .child(
                div().absolute().inset_0().child(
                    CleverImageUi::new(item.display_source())
                        .id(tab_id(format!("media-image-{}", entry.id)))
                        .content_mode(ContentMode::Fill)
                        .corner_radius(0.0)
                        .persist_to_disk(true)
                        .maximum_decode_pixel_dimension(512_i64),
                ),
            )
            .children(badge.map(|badge| div().absolute().left(px(6.0)).bottom(px(6.0)).child(badge)))
            .into_any_element()
    }

    fn handle_action(parts: &ChatSearchTypedParts, entry: &RiseGlobalMessageSearchResult, id: &str, cx: &mut App) {
        handle_media_action(parts, entry, id, cx);
    }

    fn open_entry(parts: &ChatSearchTypedParts, entry: &RiseGlobalMessageSearchResult, cx: &mut App) {
        Self::open_chat(parts, &entry.message, cx);
    }

    fn open_chat(parts: &ChatSearchTypedParts, message: &GetMessageMessage, cx: &mut App) {
        open_chat_at_message(parts, message, cx);
    }

    fn play_badge(duration: Option<f64>) -> AnyElement {
        hstack(3.0)
            .px(px(5.0))
            .py(px(2.0))
            .rounded_full()
            .bg(gpui::black().opacity(0.55))
            .children(IconUi::sized("play.fill", px(9.0), gpui::white(), true))
            .children(
                duration.map(|duration| {
                    MainText::new(ChatProfileFormatter::duration(duration)).px(11.0).font_weight(FontWeight::SEMIBOLD).color(gpui::white())
                }),
            )
            .into_any_element()
    }

    fn gif_badge() -> AnyElement {
        div()
            .px(px(5.0))
            .py(px(2.0))
            .rounded_full()
            .bg(gpui::black().opacity(0.55))
            .child(MainText::new(GIF_BADGE).px(9.0).font_weight(FontWeight::BOLD).color(gpui::white()))
            .into_any_element()
    }
}

impl RenderOnce for ChatSearchMediaTab {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        ChatSearchTypedTabScaffold {
            config: typed_config(ChatSearchMessageKind::Media, "photo.on.rectangle.angled", GRID_COLUMNS, Self::cell),
            query: self.query,
            prompt_text: tr("chat_search_type_to_find_media"),
            no_data_text: tr("no_media_found"),
        }
    }
}

#[derive(IntoElement, Default)]
pub struct ChatSearchDownloadsTab;

impl ChatSearchDownloadsTab {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for ChatSearchDownloadsTab {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        div().size_full().bg(theme.bg._100).child(prompt("arrow.down.circle", tr("chat_search_downloads_empty"), &theme))
    }
}

#[derive(IntoElement)]
pub struct ChatSearchLinksTab {
    query: String,
}

impl ChatSearchLinksTab {
    pub fn new(query: impl Into<String>) -> Self {
        Self { query: query.into() }
    }

    fn cell(parts: &Rc<ChatSearchTypedParts>, entry: &RiseGlobalMessageSearchResult) -> AnyElement {
        chat_search_typed_list_row(parts, entry, Self::row, Self::handle_action, Self::open_link)
    }

    fn row(entry: &RiseGlobalMessageSearchResult, theme: &AppTheme) -> AnyElement {
        let image = entry.link_image_url.as_deref().filter(|image| !image.is_empty());
        let leading = match image {
            Some(image) => div().size(px(56.0)).child(
                CleverImageUi::new(image.to_owned())
                    .id(tab_id(format!("link-image-{}", entry.id)))
                    .content_mode(ContentMode::Fill)
                    .corner_radius(12.0)
                    .persist_to_disk(true)
                    .maximum_decode_pixel_dimension(168_i64),
            ),
            None => div()
                .size(px(56.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.primary._100.opacity(0.16))
                .children(IconUi::sized("link", px(20.0), theme.primary._100, false)),
        };
        let title = entry.link_title.clone().or_else(|| entry.link_url.clone()).unwrap_or_default();
        let description = entry
            .link_description
            .as_deref()
            .filter(|description| !description.is_empty())
            .map(|description| MainText::new(description.to_owned()).px(12.0).color(theme.text.secondary).number_of_lines(2));
        hstack(12.0)
            .items_start()
            .w_full()
            .px(px(14.0))
            .py(px(10.0))
            .child(leading.flex_none().rounded(px(12.0)).overflow_hidden())
            .child(
                vstack(3.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(MainText::new(title).px(14.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(2))
                    .children(description)
                    .child(
                        MainText::new(Self::host(entry.link_url.as_deref().unwrap_or_default()))
                            .px(12.0)
                            .color(theme.primary._100)
                            .number_of_lines(1),
                    )
                    .child(
                        hstack(6.0)
                            .pt(px(2.0))
                            .child(MainText::new(entry.message.sender_name.clone()).px(11.0).color(theme.text.secondary))
                            .child(dot(theme))
                            .child(
                                MainText::new(ChatProfileFormatter::relative_date(entry.message.created_at))
                                    .px(11.0)
                                    .color(theme.text.secondary),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn host(url: &str) -> String {
        match Url::from_string(url).and_then(|url| url.host()) {
            Some(host) => host.replace("www.", ""),
            None => url.to_owned(),
        }
    }

    fn open_link(_parts: &ChatSearchTypedParts, entry: &RiseGlobalMessageSearchResult, cx: &mut App) {
        let Some(url) = entry.link_url.as_deref().and_then(Url::from_string) else {
            return;
        };
        InAppLinkBrowser::route(&url, cx);
    }

    fn handle_action(parts: &ChatSearchTypedParts, entry: &RiseGlobalMessageSearchResult, id: &str, cx: &mut App) {
        match id {
            "goto" => open_chat_at_message(parts, &entry.message, cx),
            "copy" => {
                if let Some(url) = &entry.link_url {
                    UIPasteboard::general().set_string(url, cx);
                }
                Notifier::copied(cx);
            }
            _ => {}
        }
    }
}

impl RenderOnce for ChatSearchLinksTab {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        ChatSearchTypedTabScaffold {
            config: typed_config(ChatSearchMessageKind::Links, "link", 1, Self::cell),
            query: self.query,
            prompt_text: tr("chat_search_type_to_find_links"),
            no_data_text: tr("no_links_found"),
        }
    }
}

#[derive(IntoElement)]
pub struct ChatSearchFilesTab {
    query: String,
}

impl ChatSearchFilesTab {
    pub fn new(query: impl Into<String>) -> Self {
        Self { query: query.into() }
    }

    fn cell(parts: &Rc<ChatSearchTypedParts>, entry: &RiseGlobalMessageSearchResult) -> AnyElement {
        chat_search_typed_list_row(parts, entry, Self::row, Self::handle_action, Self::open_entry)
    }

    fn row(entry: &RiseGlobalMessageSearchResult, theme: &AppTheme) -> AnyElement {
        let Some(item) = &entry.media_item else {
            return Empty.into_any_element();
        };
        let ext = Self::file_ext(item.file_name.as_deref().unwrap_or(FALLBACK_FILE_EXT)).to_uppercase();
        let badge_text: String = ext.chars().take(4).collect();
        let size = item.file_size.map(|size| {
            hstack(6.0)
                .child(MainText::new(ChatProfileFormatter::bytes(size)).px(11.0).color(theme.text.secondary))
                .child(dot(theme))
        });
        hstack(12.0)
            .w_full()
            .px(px(14.0))
            .py(px(10.0))
            .child(
                div()
                    .size(px(44.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(12.0))
                    .bg(Self::ext_color(&ext, theme))
                    .child(
                        MainText::new(badge_text)
                            .px(11.0)
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .color(theme.content_on(Self::ext_color(&ext, theme)))
                            .number_of_lines(1)
                            .minimum_scale_factor(0.5),
                    ),
            )
            .child(
                vstack(2.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(
                        MainText::new(item.file_name.clone().unwrap_or_else(|| tr("msg_preview_document_fallback")))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .number_of_lines(1),
                    )
                    .child(
                        hstack(6.0)
                            .w_full()
                            .children(size)
                            .child(MainText::new(entry.message.sender_name.clone()).px(11.0).color(theme.text.secondary))
                            .child(div().flex_1())
                            .child(
                                MainText::new(ChatProfileFormatter::relative_date(entry.message.created_at))
                                    .px(11.0)
                                    .color(theme.text.secondary),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn file_ext(name: &str) -> &str {
        match name.rfind('.') {
            Some(dot) => &name[dot + 1..],
            None => FALLBACK_FILE_EXT,
        }
    }

    fn ext_color(ext: &str, theme: &AppTheme) -> Hsla {
        match ext {
            "PDF" => rgb(0xE03B3B).into(),
            "ZIP" | "RAR" | "7Z" => rgb(0x7E57C2).into(),
            "DOC" | "DOCX" | "TXT" | "RTF" => rgb(0x3D7BD9).into(),
            "XLS" | "XLSX" | "CSV" | "NUMBERS" => rgb(0x34A853).into(),
            "PPT" | "KEY" => rgb(0xF59E2B).into(),
            "FIG" | "FIGMA" => rgb(0xA259FF).into(),
            "PSD" | "AI" | "SKETCH" => rgb(0xFF7A00).into(),
            "MP4" | "MOV" | "AVI" => rgb(0x5E81F4).into(),
            "MP3" | "WAV" | "FLAC" | "M4A" => rgb(0x1DB954).into(),
            _ => theme.primary._100,
        }
    }

    fn handle_action(parts: &ChatSearchTypedParts, entry: &RiseGlobalMessageSearchResult, id: &str, cx: &mut App) {
        handle_media_action(parts, entry, id, cx);
    }

    fn open_entry(parts: &ChatSearchTypedParts, entry: &RiseGlobalMessageSearchResult, cx: &mut App) {
        Self::open_chat(parts, &entry.message, cx);
    }

    fn open_chat(parts: &ChatSearchTypedParts, message: &GetMessageMessage, cx: &mut App) {
        open_chat_at_message(parts, message, cx);
    }
}

impl RenderOnce for ChatSearchFilesTab {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        ChatSearchTypedTabScaffold {
            config: typed_config(ChatSearchMessageKind::Files, "doc.text", 1, Self::cell),
            query: self.query,
            prompt_text: tr("chat_search_type_to_find_files"),
            no_data_text: tr("no_files_found"),
        }
    }
}

#[derive(IntoElement)]
pub struct ChatSearchMusicTab {
    query: String,
}

impl ChatSearchMusicTab {
    pub fn new(query: impl Into<String>) -> Self {
        Self { query: query.into() }
    }

    fn cell(parts: &Rc<ChatSearchTypedParts>, entry: &RiseGlobalMessageSearchResult) -> AnyElement {
        chat_search_typed_list_row(parts, entry, Self::row, Self::handle_action, Self::open_entry)
    }

    fn row(entry: &RiseGlobalMessageSearchResult, theme: &AppTheme) -> AnyElement {
        let Some(item) = &entry.media_item else {
            return Empty.into_any_element();
        };
        let duration = item.duration.map(|duration| {
            hstack(6.0)
                .child(MainText::new(ChatProfileFormatter::duration(duration)).px(12.0).color(theme.text.secondary))
                .child(dot(theme))
        });
        hstack(12.0)
            .w_full()
            .px(px(14.0))
            .py(px(10.0))
            .child(
                div()
                    .size(px(44.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.0))
                    .bg(theme.primary._100)
                    .children(IconUi::sized("music.note", px(18.0), theme.content_on(theme.primary._100), false)),
            )
            .child(
                vstack(3.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(MainText::new(Self::track_title(entry)).px(14.0).font_weight(FontWeight::MEDIUM).number_of_lines(1))
                    .child(
                        hstack(6.0)
                            .w_full()
                            .children(duration)
                            .child(MainText::new(entry.message.sender_name.clone()).px(12.0).color(theme.text.secondary))
                            .child(div().flex_1())
                            .child(
                                MainText::new(ChatProfileFormatter::relative_date(entry.message.created_at))
                                    .px(11.0)
                                    .color(theme.text.secondary),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn track_title(entry: &RiseGlobalMessageSearchResult) -> String {
        if let Some(name) = entry.media_item.as_ref().and_then(|item| item.file_name.as_ref()).filter(|name| !name.is_empty()) {
            return name.clone();
        }
        let text = trimming_whitespaces_and_newlines(&entry.message.text);
        if text.is_empty() { tr("chat_search_tab_music") } else { text.to_owned() }
    }

    fn handle_action(parts: &ChatSearchTypedParts, entry: &RiseGlobalMessageSearchResult, id: &str, cx: &mut App) {
        handle_media_action(parts, entry, id, cx);
    }

    fn open_entry(parts: &ChatSearchTypedParts, entry: &RiseGlobalMessageSearchResult, cx: &mut App) {
        Self::open_chat(parts, &entry.message, cx);
    }

    fn open_chat(parts: &ChatSearchTypedParts, message: &GetMessageMessage, cx: &mut App) {
        open_chat_at_message(parts, message, cx);
    }
}

impl RenderOnce for ChatSearchMusicTab {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        ChatSearchTypedTabScaffold {
            config: typed_config(ChatSearchMessageKind::Music, "music.note", 1, Self::cell),
            query: self.query,
            prompt_text: tr("chat_search_type_to_find_music"),
            no_data_text: tr("no_music_found"),
        }
    }
}

#[derive(IntoElement)]
pub struct ChatSearchVoiceTab {
    query: String,
}

impl ChatSearchVoiceTab {
    pub fn new(query: impl Into<String>) -> Self {
        Self { query: query.into() }
    }

    fn cell(parts: &Rc<ChatSearchTypedParts>, entry: &RiseGlobalMessageSearchResult) -> AnyElement {
        chat_search_typed_list_row(parts, entry, Self::row, Self::handle_action, Self::open_entry)
    }

    fn row(entry: &RiseGlobalMessageSearchResult, theme: &AppTheme) -> AnyElement {
        let Some(item) = &entry.media_item else {
            return Empty.into_any_element();
        };
        let duration = item.duration.map(|duration| {
            hstack(6.0)
                .child(MainText::new(ChatProfileFormatter::duration(duration)).px(12.0).color(theme.text.secondary))
                .child(dot(theme))
        });
        hstack(12.0)
            .w_full()
            .px(px(14.0))
            .py(px(10.0))
            .child(
                div()
                    .size(px(40.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(theme.primary._100)
                    .children(IconUi::sized("waveform", px(18.0), theme.content_on(theme.primary._100), false)),
            )
            .child(
                vstack(3.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(
                        MainText::new(entry.message.sender_name.clone()).px(14.0).font_weight(FontWeight::MEDIUM).number_of_lines(1),
                    )
                    .child(
                        hstack(6.0).children(duration).child(
                            MainText::new(ChatProfileFormatter::relative_date(entry.message.created_at))
                                .px(12.0)
                                .color(theme.text.secondary),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn handle_action(parts: &ChatSearchTypedParts, entry: &RiseGlobalMessageSearchResult, id: &str, cx: &mut App) {
        handle_media_action(parts, entry, id, cx);
    }

    fn open_entry(parts: &ChatSearchTypedParts, entry: &RiseGlobalMessageSearchResult, cx: &mut App) {
        Self::open_chat(parts, &entry.message, cx);
    }

    fn open_chat(parts: &ChatSearchTypedParts, message: &GetMessageMessage, cx: &mut App) {
        open_chat_at_message(parts, message, cx);
    }
}

impl RenderOnce for ChatSearchVoiceTab {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        ChatSearchTypedTabScaffold {
            config: typed_config(ChatSearchMessageKind::Voices, "waveform", 1, Self::cell),
            query: self.query,
            prompt_text: tr("chat_search_type_to_find_voice"),
            no_data_text: tr("no_voice_found"),
        }
    }
}
