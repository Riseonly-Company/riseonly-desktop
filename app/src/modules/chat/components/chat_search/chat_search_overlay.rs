use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, App, Bounds, Context, ElementId, Entity, FocusHandle, KeyDownEvent, Pixels, SharedString,
    StyleRefinement, Subscription, Task, Window,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};
use rise_ui::IconUi;
use rise_ui::animated_tabs_ui::animated_tabs_types::AnimatedTabsHeaderPlacement;
use rise_ui::input_ui::input_actions::{MoveDown, MoveUp};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::motion::{MotionSpec, ease_in_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::rise_engine_bridge::rise_transport_status::{RiseTransportDropWatch, RiseTransportStatus};
use crate::core::ui::animated_tabs_ui::animated_tabs::AnimatedTabs;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::AnimatedTabsPagerAction;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::chat::components::chat::chat_logo::ChatLogo;
use crate::modules::chat::components::chat_item::chat_list_preview_row::ChatListPreviewFactory;
use crate::modules::chat::components::chat_search::chat_search_overlay_store::ChatSearchOverlayStore;
use crate::modules::chat::components::chat_search::chat_search_section::ChatSearchSectionHeader;
use crate::modules::chat::components::chat_search::chat_search_tabs_catalog::{ChatSearchActiveTabKey, ChatSearchTabsCatalog};
use crate::modules::chat::components::chat_search::chats_search_context::{
    ChatSearchResultsKeyboard, ChatSearchResultsKeyboardKey, ChatsSearchContext, ChatsSearchContextKey,
};
use crate::modules::chat::engine::core::rise_chat_overlay_search_presentation::{
    RiseChatSearchRecentDestination, RiseChatSearchRecentItem,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseConversationPresentation, RiseConversationPresentationChange,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const ENTER_KEY: &str = "enter";
const UP_KEY: &str = "up";
const DOWN_KEY: &str = "down";
const CLOSE_BUTTON_SIDE: f32 = 42.0;
const SEARCH_DELAY: Duration = Duration::from_millis(300);
const RECENTS_ANIMATION_SECONDS: f64 = 0.18;
const RECENT_LOGO_SIDE: f32 = 56.0;
const RECENT_ONLINE_SIDE: f32 = 12.0;
const RECENT_ONLINE_STROKE: f32 = 2.0;
const RECENT_ONLINE_OFFSET: f32 = 20.0;

fn overlay_id(suffix: impl Into<String>) -> ElementId {
    ElementId::Name(format!("chat-search-overlay-{}", suffix.into()).into())
}

fn is_blank(query: &str) -> bool {
    trimming_whitespaces_and_newlines(query).is_empty()
}

fn normalized(value: Option<&str>) -> Option<String> {
    let trimmed = trimming_whitespaces_and_newlines(value?);
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

pub enum ChatSearchHero {}

impl ChatSearchHero {
    pub const FIELD_ID: &'static str = "chatsSearchField";

    pub fn animation() -> MotionSpec {
        spring(0.42, 0.86)
    }
}

/// hero namespace is one number here, the field frame is computed from it and the two measured frames
pub struct ChatSearchHeroNamespace {
    pub(crate) motion: Animated<f32>,
    pub collapsed_field_frame: Option<Bounds<Pixels>>,
    pub(crate) overlay_field_frame: Option<Bounds<Pixels>>,
}

impl ChatSearchHeroNamespace {
    pub fn new() -> Self {
        Self { motion: Animated::new(0.0), collapsed_field_frame: None, overlay_field_frame: None }
    }

    pub fn set_presented(&mut self, presented: bool, now: MotionTime, cx: &mut Context<Self>) {
        self.motion.set_in(if presented { 1.0 } else { 0.0 }, ChatSearchHero::animation(), now, cx);
        cx.notify();
    }
}

impl Default for ChatSearchHeroNamespace {
    fn default() -> Self {
        Self::new()
    }
}

fn live_recent_chats(items: &[RiseChatSearchRecentItem], cx: &mut App) -> HashMap<String, ChatInfo> {
    let chats = ChatServicesStore::shared(cx);
    let chats = chats.read(cx);
    let removed = chats.locally_removed_chat_ids();
    let mut result = HashMap::new();
    let mut direct_by_peer: Option<HashMap<String, ChatInfo>> = None;
    for item in items {
        if let Some(chat_id) = normalized(item.chat_id.as_deref())
            && let Some(chat) = chats.chat_info(&chat_id, cx)
            && !removed.contains(&chat.id)
        {
            result.insert(item.id.clone(), chat);
            continue;
        }
        if item.destination != RiseChatSearchRecentDestination::Channel
            && let Some(user_id) = normalized(item.user_id.as_deref())
        {
            let by_peer = direct_by_peer.get_or_insert_with(|| {
                let mut by_peer = HashMap::new();
                for chat in chats.current_chat_infos(cx) {
                    if chat.r#type == ChatType::Direct
                        && !chat.participant.id.is_empty()
                        && !removed.contains(&chat.id)
                        && !UserServicesStore::is_contact_join_virtual_chat_id(&chat.id)
                    {
                        by_peer.entry(chat.participant.id.clone()).or_insert(chat);
                    }
                }
                by_peer
            });
            if let Some(chat) = by_peer.get(&user_id) {
                result.insert(item.id.clone(), chat.clone());
            }
        }
    }
    result
}

fn distinct_recents(items: Vec<RiseChatSearchRecentItem>, live_chats: &HashMap<String, ChatInfo>) -> Vec<RiseChatSearchRecentItem> {
    let mut seen_chats = HashSet::new();
    let mut seen_peers = HashSet::new();
    items
        .into_iter()
        .filter(|item| {
            if let Some(live) = live_chats.get(&item.id)
                && !seen_chats.insert(live.id.clone())
            {
                return false;
            }
            if item.destination == RiseChatSearchRecentDestination::Channel {
                return true;
            }
            match normalized(item.user_id.as_deref()) {
                Some(peer) => seen_peers.insert(peer),
                None => true,
            }
        })
        .collect()
}

/// the view behind the search overlay, keyed by the mount of the layer
pub struct ChatSearchOverlayView {
    query: String,
    is_presented: Option<Binding<bool>>,
    query_binding: Option<Binding<String>>,
    hero_namespace: Option<Entity<ChatSearchHeroNamespace>>,
    nav_store: Option<Entity<NavigationStore>>,
    context: Option<ChatsSearchContext>,
    active_tab: i64,
    focuses_on_appear: bool,
    searched_query: Option<String>,
    task_key: Option<String>,
    search_task: Option<Task<()>>,
    connection_watch: Option<RiseTransportDropWatch>,
    recents_presence: Presence<()>,
    recents_height: Animated<f32>,
    recents_natural_height: f32,
    keyboard: Entity<ChatSearchResultsKeyboard>,
    focus: FocusHandle,
    recents: Rc<Vec<RiseChatSearchRecentItem>>,
    recent_live_chats: Rc<HashMap<String, ChatInfo>>,
    _subscriptions: Vec<Subscription>,
}

impl ChatSearchOverlayView {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseConversationPresentation::shared(cx);
        let services = ChatServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let mut view = Self {
            query: String::new(),
            is_presented: None,
            query_binding: None,
            hero_namespace: None,
            nav_store: None,
            context: None,
            active_tab: 0,
            focuses_on_appear: true,
            searched_query: None,
            task_key: None,
            search_task: None,
            connection_watch: None,
            recents_presence: Presence::new(Some(()))
                .with_transition(Transition::opacity().combined(Transition::move_edge(Edge::Top))),
            recents_height: Animated::new(0.0),
            recents_natural_height: 0.0,
            keyboard: cx.new(|_| ChatSearchResultsKeyboard::new()),
            focus: cx.focus_handle(),
            recents: Rc::new(Vec::new()),
            recent_live_chats: Rc::new(HashMap::new()),
            _subscriptions: vec![
                cx.subscribe(&presentation, |view, _, change: &RiseConversationPresentationChange, cx| {
                    if matches!(
                        change,
                        RiseConversationPresentationChange::ChatSearchRecents | RiseConversationPresentationChange::All
                    ) && view.refresh_recents(cx)
                    {
                        cx.notify();
                    }
                }),
                cx.observe(&services, |view, _, cx| {
                    if view.refresh_recents(cx) {
                        cx.notify();
                    }
                }),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        };
        view.watch_connection(cx);
        view.refresh_recents(cx);
        view
    }

    /// takes the recents and their live chats, true when either changed
    fn refresh_recents(&mut self, cx: &mut Context<Self>) -> bool {
        let stored = ChatSearchOverlayStore::shared(cx).update(cx, |store, cx| store.recent_items(cx));
        let live_chats = live_recent_chats(&stored, cx);
        if *self.recents == stored && *self.recent_live_chats == live_chats {
            return false;
        }
        self.recents = Rc::new(stored);
        self.recent_live_chats = Rc::new(live_chats);
        true
    }

    /// a search that failed while the server was unreachable is asked again once the connection is back
    /// the watch fires once so it re-arms itself
    fn watch_connection(&mut self, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        self.connection_watch = Some(RiseTransportStatus::on_next_connect(
            move |cx| {
                let _ = view.update(cx, |view, cx| {
                    view.retry_failed_searches(cx);
                    view.watch_connection(cx);
                });
            },
            cx,
        ));
    }

    fn retry_failed_searches(&self, cx: &mut Context<Self>) {
        let query = self.query.clone();
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.retry_failed_chat_overlay_searches(&query, cx));
    }

    fn sync(
        &mut self,
        is_presented: Binding<bool>,
        query: Binding<String>,
        hero_namespace: Entity<ChatSearchHeroNamespace>,
        nav_store: Option<Entity<NavigationStore>>,
        context: Option<ChatsSearchContext>,
        cx: &mut Context<Self>,
    ) {
        let query_value = query.get(cx);
        self.is_presented = Some(is_presented);
        self.query_binding = Some(query);
        self.context = context;
        if self.hero_namespace.as_ref().map(Entity::entity_id) != Some(hero_namespace.entity_id()) {
            self._subscriptions.push(cx.observe(&hero_namespace, |_, _, cx| cx.notify()));
            self.hero_namespace = Some(hero_namespace);
            cx.notify();
        }
        if nav_store.as_ref().map(Entity::entity_id) != self.nav_store.as_ref().map(Entity::entity_id) {
            self.nav_store = nav_store;
            cx.notify();
        }
        self.apply_query(query_value, cx);
    }

    fn recents_did_measure(&mut self, height: f32, cx: &mut Context<Self>) {
        if !is_blank(&self.query) || (height - self.recents_natural_height).abs() < 0.5 {
            return;
        }
        self.recents_natural_height = height;
        self.recents_height = Animated::new(height);
        cx.notify();
    }

    fn apply_query(&mut self, query: String, cx: &mut Context<Self>) {
        if query == self.query {
            return;
        }
        let was_blank = is_blank(&self.query);
        self.query = query;
        let now_blank = is_blank(&self.query);
        if was_blank != now_blank {
            let now = RiseClock::monotonic();
            self.recents_presence.set_in(now_blank.then_some(()), ease_in_out(RECENTS_ANIMATION_SECONDS), now, cx);
            let target = if now_blank { self.recents_natural_height } else { 0.0 };
            self.recents_height.set_in(target, ease_in_out(RECENTS_ANIMATION_SECONDS), now, cx);
        }
        self.run_search(cx);
        cx.notify();
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(&self.query).to_owned();
        let key = trimmed.to_lowercase();
        if self.task_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.task_key = Some(key.clone());
        self.search_task = None;
        if trimmed.is_empty() {
            self.searched_query = None;
            return;
        }
        if self.searched_query.as_deref() == Some(key.as_str()) {
            return;
        }
        self.search_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            let _ = this.update(cx, |view, cx| {
                view.searched_query = Some(key);
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.search_chat_overlay_entities(&trimmed, None, cx);
                    store.search_chat_overlay_chats(&trimmed, None, cx);
                });
            });
        }));
    }

    fn set_active_tab(&mut self, active_tab: i64, cx: &mut Context<Self>) {
        if active_tab == self.active_tab {
            return;
        }
        self.active_tab = active_tab;
        self.keyboard.update(cx, |keyboard, cx| keyboard.clear(cx));
        self.retry_failed_searches(cx);
        cx.notify();
    }

    fn close(is_presented: &Option<Binding<bool>>, window: &mut Window, cx: &mut App) {
        ScreenFocus::release_text_field(window, cx);
        if let Some(is_presented) = is_presented {
            is_presented.set(false, window, cx);
        }
    }

    fn open_recent(
        item: &RiseChatSearchRecentItem,
        live: Option<&ChatInfo>,
        is_presented: &Option<Binding<bool>>,
        nav_store: &Option<Entity<NavigationStore>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        Self::close(is_presented, window, cx);
        let interactions = MessageInteractionsStore::shared(cx);
        let route = match item.destination {
            RiseChatSearchRecentDestination::Chat | RiseChatSearchRecentDestination::Channel => {
                if let Some(live) = live {
                    if live.title.is_empty() {
                        interactions.update(cx, |store, cx| store.record_chat_search_recent(item.clone(), cx));
                    } else {
                        interactions.update(cx, |store, cx| store.record_chat_search_recent_chat(live, cx));
                    }
                    Some(RootRoute::Chat { chat_id: Some(live.id.clone()) })
                } else {
                    interactions.update(cx, |store, cx| store.record_chat_search_recent(item.clone(), cx));
                    normalized(item.chat_id.as_deref()).map(|chat_id| RootRoute::Chat { chat_id: Some(chat_id) })
                }
            }
            RiseChatSearchRecentDestination::User => {
                interactions.update(cx, |store, cx| store.record_chat_search_recent(item.clone(), cx));
                let user_id = normalized(item.user_id.as_deref());
                let tag = normalized(item.user_tag.as_deref());
                (user_id.is_some() || tag.is_some()).then_some(RootRoute::UserPage { user_id, tag })
            }
        };
        if let (Some(route), Some(nav_store)) = (route, nav_store) {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn recent_tile(&self, item: &RiseChatSearchRecentItem, live: Option<&ChatInfo>, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let title = match live {
            Some(live) => ChatListPreviewFactory::chat_title(live, cx),
            None => item.title.clone(),
        };
        let logo: AnyElement = match live {
            Some(live) => ChatLogo::new(overlay_id(format!("recent-logo-{}", item.id)), live.clone())
                .size(RECENT_LOGO_SIDE)
                .icon_size(22.0)
                .into_any_element(),
            None => {
                let source = (!item.logo_url.is_empty()).then(|| SharedString::from(item.logo_url.clone()));
                let dot_side = RECENT_ONLINE_SIDE + RECENT_ONLINE_STROKE;
                let dot_origin = RECENT_LOGO_SIDE / 2.0 + RECENT_ONLINE_OFFSET - dot_side / 2.0;
                div()
                    .relative()
                    .size(px(RECENT_LOGO_SIDE))
                    .child(
                        UserLogoUi::new(overlay_id(format!("recent-user-logo-{}", item.id)))
                            .source(source)
                            .name(SharedString::from(item.title.clone()))
                            .user_id(item.user_id.clone().map(SharedString::from))
                            .color_scope_id(item.chat_id.clone().map(SharedString::from))
                            .size(RECENT_LOGO_SIDE),
                    )
                    .children(item.is_online.then(|| {
                        div()
                            .absolute()
                            .left(px(dot_origin))
                            .top(px(dot_origin))
                            .size(px(dot_side))
                            .rounded_full()
                            .bg(theme.semantic.success_100)
                            .border_2()
                            .border_color(theme.bg._100)
                    }))
                    .into_any_element()
            }
        };
        let (open_item, open_live) = (item.clone(), live.cloned());
        let (is_presented, nav_store) = (self.is_presented.clone(), self.nav_store.clone());
        plain(overlay_id(format!("recent-{}", item.id)))
            .on_press(move |window, cx| {
                Self::open_recent(&open_item, open_live.as_ref(), &is_presented, &nav_store, window, cx);
            })
            .child(
                vstack(5.0)
                    .child(logo)
                    .child(div().max_w(px(64.0)).child(MainText::new(title).px(11.0).tac(TextAlignT::Center).number_of_lines(1))),
            )
            .into_any_element()
    }

    fn recent_chips(&self, theme: &AppTheme, cx: &mut App) -> Div {
        let live_chats = self.recent_live_chats.clone();
        let recents = distinct_recents((*self.recents).clone(), &live_chats);
        if recents.is_empty() {
            return div();
        }
        let tiles: Vec<AnyElement> =
            recents.iter().map(|item| self.recent_tile(item, live_chats.get(&item.id), theme, cx)).collect();
        vstack(0.0)
            .w_full()
            .child(
                ChatSearchSectionHeader::new(overlay_id("recent-clear"), tr("chat_search_overlay_recent").to_uppercase())
                    .right_title(Some(tr("chat_search_overlay_clear")))
                    .on_right(|_, cx| {
                        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_chat_search_recents(cx));
                    }),
            )
            .child(
                div()
                    .id(overlay_id("recent-strip"))
                    .flex()
                    .w_full()
                    .h(px(92.0))
                    .overflow_x_scroll()
                    .child(hstack(14.0).flex_none().px(px(14.0)).py(px(6.0)).children(tiles)),
            )
    }

    fn header(&self, entity: &Entity<Self>, theme: &AppTheme, now: MotionTime, window: &Window, cx: &App) -> Div {
        let change_view = entity.downgrade();
        let submit_keyboard = self.keyboard.clone();
        let cancel_presented = self.is_presented.clone();
        let field = SearchFieldUi::new(
            overlay_id(ChatSearchHero::FIELD_ID),
            self.query_binding.clone().unwrap_or_else(|| Binding::constant(String::new())),
        )
        .placeholder(SharedString::from(tr("search_placeholder")))
        .on_change(move |value, _, cx| {
            let _ = change_view.update(cx, |view, cx| view.apply_query(value.to_owned(), cx));
        })
        .on_submit(move |window, cx| Self::activate_selected(&submit_keyboard, window, cx))
        .on_cancel(move |window, cx| {
            if !HoldContextMenuPresenter::has_active_request(cx) {
                Self::close(&cancel_presented, window, cx);
            }
        })
        .keeps_focus_on_submit(true)
        .uses_inline_text_field(true)
        .auto_focus(self.focuses_on_appear);

        let mut hero = div().relative().w_full();
        if let Some(namespace) = self.hero_namespace.as_ref().map(|namespace| namespace.read(cx)) {
            let progress = namespace.motion.frame_value(now, window);
            if let (Some(collapsed), Some(slot)) = (namespace.collapsed_field_frame, namespace.overlay_field_frame)
                && (1.0 - progress).abs() > f32::EPSILON
            {
                let remaining = 1.0 - progress;
                let collapsed_width = f32::from(collapsed.size.width);
                let slot_width = f32::from(slot.size.width);
                hero = hero
                    .left(px((f32::from(collapsed.origin.x) - f32::from(slot.origin.x)) * remaining))
                    .top(px((f32::from(collapsed.origin.y) - f32::from(slot.origin.y)) * remaining))
                    .w(px(slot_width + (collapsed_width - slot_width) * remaining));
            }
        }

        let measured = self.hero_namespace.clone();
        let is_presented = self.is_presented.clone();
        hstack(10.0)
            .w_full()
            .px(px(12.0))
            .pt(px(10.0))
            .pb(px(10.0))
            .child(
                div().relative().flex_1().min_w_0().child(hero.child(field)).child(measure(
                    overlay_id("field-frame"),
                    move |bounds, _, cx| {
                        if let Some(measured) = measured {
                            measured.update(cx, |namespace, cx| {
                                if namespace.overlay_field_frame != Some(bounds) {
                                    namespace.overlay_field_frame = Some(bounds);
                                    cx.notify();
                                }
                            });
                        }
                    },
                )),
            )
            .child(
                LiquidGlassButton::new(overlay_id("close"), LiquidGlassButtonShape::Circle)
                    .size(CLOSE_BUTTON_SIDE)
                    .accessibility_label(Some(tr("cancel")))
                    .on_press(move |window, cx| Self::close(&is_presented, window, cx))
                    .children(IconUi::sized("xmark", px(15.0), theme.text.secondary, false)),
            )
    }

    fn activate_selected(keyboard: &Entity<ChatSearchResultsKeyboard>, window: &mut Window, cx: &mut App) {
        let action = keyboard.read(cx).selected_action();
        if let Some(action) = action {
            action(window, cx);
        }
    }
}

impl Render for ChatSearchOverlayView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let entity = cx.entity();
        let now = RiseClock::monotonic();
        let header = self.header(&entity, &theme, now, window, cx);
        let recents = self.recent_chips(&theme, cx);
        let recents = self.recents_presence.render_transitioned(now, window, move |_| recents);
        let natural_height = self.recents_natural_height;
        let slot_height = self.recents_height.frame_value(now, window).max(0.0);
        let measured = entity.clone();
        let recents_slot = div()
            .w_full()
            .flex_none()
            .overflow_hidden()
            .when(natural_height > 0.0, |slot| slot.h(px(slot_height)))
            .child(div().relative().w_full().flex_none().children(recents).child(measure(
                overlay_id("recents-height"),
                move |bounds, _, cx| {
                    measured.update(cx, |view, cx| view.recents_did_measure(f32::from(bounds.size.height), cx));
                },
            )));

        let tab_binding = Binding::entity(
            entity.clone(),
            |view: &ChatSearchOverlayView| view.active_tab,
            |view, value: i64, cx| view.set_active_tab(value, cx),
        );
        let swipe_presented = self.is_presented.clone();
        let on_swipe_back: AnimatedTabsPagerAction =
            Rc::new(move |window: &mut Window, cx: &mut App| Self::close(&swipe_presented, window, cx));
        let tabs = AnimatedTabs::new(overlay_id("tabs"), ChatSearchTabsCatalog::all_tabs(&self.query), tab_binding)
            .horizontal_padding(10.0)
            .top_padding(6.0)
            .bottom_padding(8.0)
            .header_placement(AnimatedTabsHeaderPlacement::Bottom)
            .on_swipe_back_beyond_first(Some(on_swipe_back));

        let action_presented = self.is_presented.clone();
        let key_entity = entity.clone();
        let up_entity = entity.clone();
        let down_entity = entity.clone();
        let disappear_view = entity.downgrade();
        let focus = self.focus.clone();
        let (refocuses_on_appear, appearing) = (!self.focuses_on_appear, self.focus.clone());
        let leaving = self.focus.clone();
        div()
            .id(overlay_id("root"))
            .size_full()
            .occlude()
            .bg(theme.bg._100)
            .track_focus(&self.focus)
            .layer_keys(LayerCancel::dismiss(move |window, cx| {
                if !HoldContextMenuPresenter::has_active_request(cx) {
                    Self::close(&action_presented, window, cx);
                }
            }))
            .capture_action(move |_: &MoveUp, _, cx| {
                if up_entity.read(cx).active_tab == 0 {
                    cx.stop_propagation();
                    up_entity.read(cx).keyboard.clone().update(cx, |keyboard, cx| keyboard.move_selection(false, cx));
                }
            })
            .capture_action(move |_: &MoveDown, _, cx| {
                if down_entity.read(cx).active_tab == 0 {
                    cx.stop_propagation();
                    down_entity.read(cx).keyboard.clone().update(cx, |keyboard, cx| keyboard.move_selection(true, cx));
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if event.keystroke.modifiers.modified() || !focus.is_focused(window) || key_entity.read(cx).active_tab != 0 {
                    return;
                }
                let keyboard = key_entity.read(cx).keyboard.clone();
                match key {
                    UP_KEY => {
                        cx.stop_propagation();
                        keyboard.update(cx, |keyboard, cx| keyboard.move_selection(false, cx));
                    }
                    DOWN_KEY => {
                        cx.stop_propagation();
                        keyboard.update(cx, |keyboard, cx| keyboard.move_selection(true, cx));
                    }
                    ENTER_KEY => {
                        cx.stop_propagation();
                        Self::activate_selected(&keyboard, window, cx);
                    }
                    _ => {}
                }
            })
            .child(
                vstack(0.0)
                    .size_full()
                    .child(header)
                    .child(recents_slot)
                    .child(
                        div().flex_1().min_h_0().w_full().child(
                            env_scope::<ChatSearchResultsKeyboardKey>(Some(self.keyboard.clone()))
                                .child(env_scope::<ChatsSearchContextKey>(self.context.clone()).child(
                                    env_scope::<ChatSearchActiveTabKey>(Some(self.active_tab)).child(tabs),
                                )),
                        ),
                    ),
            )
            .on_appear(overlay_id("lifecycle"), move |window, cx| {
                if refocuses_on_appear {
                    window.focus(&appearing, cx);
                }
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    ScreenFocus::return_to_last_screen(&leaving, window, cx);
                }
                let _ = disappear_view.update(cx, |view, cx| {
                    view.focuses_on_appear = false;
                    cx.notify();
                });
            })
    }
}

#[derive(IntoElement)]
pub struct ChatSearchOverlay {
    id: ElementId,
    is_presented: Binding<bool>,
    query: Binding<String>,
    hero_namespace: Entity<ChatSearchHeroNamespace>,
}

impl ChatSearchOverlay {
    pub fn new(
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        query: Binding<String>,
        hero_namespace: Entity<ChatSearchHeroNamespace>,
    ) -> Self {
        Self { id: id.into(), is_presented, query, hero_namespace }
    }
}

impl RenderOnce for ChatSearchOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let context = env::<ChatsSearchContextKey>(cx);
        let view = window.use_keyed_state(self.id, cx, |_, cx| ChatSearchOverlayView::new(cx));
        let (is_presented, query, hero_namespace) = (self.is_presented, self.query, self.hero_namespace);
        view.update(cx, |view, cx| view.sync(is_presented, query, hero_namespace, nav_store, context, cx));
        AnyView::from(view).cached(StyleRefinement::default().size_full())
    }
}
