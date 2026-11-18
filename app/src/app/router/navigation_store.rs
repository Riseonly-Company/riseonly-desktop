use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use rise_canon::rise_key::guide_screen::camel as guide_screen_keys;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::swiftui::environment::EnvKey;

use crate::app::aside::aside_registry;
use crate::app::router::navigation_aside_hooks::navigation_aside_hooks;
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiActiveScreen;
use crate::core::lib::sai::sai::Sai;
use crate::core::shared_element_transition::navigation::shared_element_navigation_transitioning::SharedElementNavigationController;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::pages::chats::embedded_forum_transition::EmbeddedForumPresentationStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

const LOG_NAME: &str = "Navigation";
const CHAT_OPEN_FIRST_FRAME_READY_EVENT: &str = "chat.open.first_frame_ready";
const CHAT_OPEN_PUSH_EVENT: &str = "chat.open.push";
const EPOCH_FIELD: &str = "epoch";
const SETTLE_POLL: Duration = Duration::from_millis(50);
const SETTLE_LIMIT: Duration = Duration::from_millis(2_000);
const PUSH_TOUCH_HOLD_MINIMUM: Duration = Duration::from_millis(400);
const PUSH_TOUCH_HOLD_TAIL: Duration = Duration::from_millis(250);
const REPEATED_CHAT_PUSH_EXPIRY: Duration = Duration::from_nanos(600_000_000);

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BotMiniAppPresentation {
    pub id: Uuid,
    pub url: Url,
    pub title: String,
    pub allows_development_http: bool,
    pub chat_id: Option<String>,
    pub bot_id: Option<String>,
    pub is_web_link: bool,
}

impl BotMiniAppPresentation {
    /// bot id and is_web_link default to none and false
    pub fn new(url: Url, title: String, allows_development_http: bool, chat_id: Option<String>) -> Self {
        Self::new_with_id(Uuid::new_v4(), url, title, allows_development_http, chat_id, None, false)
    }

    pub fn new_with_id(
        id: Uuid,
        url: Url,
        title: String,
        allows_development_http: bool,
        chat_id: Option<String>,
        bot_id: Option<String>,
        is_web_link: bool,
    ) -> Self {
        Self { id, url, title, allows_development_http, chat_id, bot_id, is_web_link }
    }

    pub fn from_route(route: &RootRoute) -> Option<Self> {
        let RootRoute::BotMiniApp { url, title, allows_development_http, chat_id, bot_id, is_web_link } = route else {
            return None;
        };
        Some(Self::new_with_id(
            Uuid::new_v4(),
            url.clone(),
            title.clone(),
            *allows_development_http,
            chat_id.clone(),
            bot_id.clone(),
            *is_web_link,
        ))
    }

    pub fn route(&self) -> RootRoute {
        RootRoute::BotMiniApp {
            url: self.url.clone(),
            title: self.title.clone(),
            allows_development_http: self.allows_development_http,
            chat_id: self.chat_id.clone(),
            bot_id: self.bot_id.clone(),
            is_web_link: self.is_web_link,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
struct NavigationStorePendingRepeatedChatPush {
    id: Uuid,
    route: RootRoute,
    expected_source_path: Vec<RootRoute>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum NavigationStoreSettlingPush {
    #[default]
    Drop,
    Wait,
}

pub struct NavigationStore {
    pub path: Vec<RootRoute>,
    pub current_tab: MainTab,
    bot_mini_app_presentation: Option<BotMiniAppPresentation>,
    sai_active_screen_overlay_params: HashMap<String, String>,
    sai_active_screen_overlay: Option<SaiActiveScreenOverlay>,
    embedded_forum_request: Option<EmbeddedForumRequest>,
    chat_presentation_epoch: Option<Uuid>,
    chat_first_frame_ready_epoch: Option<Uuid>,
    chat_presentation_scope: Option<String>,
    chat_presentation_route: Option<RootRoute>,
    chat_first_frame_eligible_epoch: Option<Uuid>,
    pending_repeated_chat_push: Option<NavigationStorePendingRepeatedChatPush>,
    pending_repeated_chat_push_expiry: Option<Task<()>>,
    deferred_repeated_chat_push_id: Option<Uuid>,
    push_touch_hold_generation: Option<Uuid>,
    deferred_push_generation: Option<Uuid>,
    navigation_window: Option<Weak<dyn SharedElementNavigationController>>,
    chat_list_beside_stack: bool,
    settings_list_beside_stack: bool,
    settings_opened_from_rail: bool,
    aside_takes_details: bool,
    aside_hosts_chat_info: bool,
    tab_paths: HashMap<MainTab, Vec<RootRoute>>,
    tab_switch_floor: Option<usize>,
    tab_switch_origin: Option<(MainTab, Vec<RootRoute>)>,
}

impl NavigationStore {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            path: Vec::new(),
            current_tab: MainTab::Posts,
            bot_mini_app_presentation: None,
            sai_active_screen_overlay_params: HashMap::new(),
            sai_active_screen_overlay: None,
            embedded_forum_request: None,
            chat_presentation_epoch: None,
            chat_first_frame_ready_epoch: None,
            chat_presentation_scope: None,
            chat_presentation_route: None,
            chat_first_frame_eligible_epoch: None,
            pending_repeated_chat_push: None,
            pending_repeated_chat_push_expiry: None,
            deferred_repeated_chat_push_id: None,
            push_touch_hold_generation: None,
            deferred_push_generation: None,
            navigation_window: None,
            chat_list_beside_stack: false,
            settings_list_beside_stack: false,
            settings_opened_from_rail: false,
            aside_takes_details: false,
            aside_hosts_chat_info: false,
            tab_paths: HashMap::new(),
            tab_switch_floor: None,
            tab_switch_origin: None,
        }
    }

    pub fn set_chat_list_beside_stack(&mut self, is_beside: bool) {
        self.chat_list_beside_stack = is_beside;
    }

    pub fn chat_list_beside_stack(&self) -> bool {
        self.chat_list_beside_stack
    }

    /// window shell tells whether the settings root is drawn as the list column beside the open section
    /// layout input only so no notify
    pub fn set_settings_list_beside_stack(&mut self, is_beside: bool) {
        self.settings_list_beside_stack = is_beside;
    }

    pub fn settings_list_beside_stack(&self) -> bool {
        self.settings_list_beside_stack
    }

    /// where the open settings start in the stack
    /// the shell draws that root as the list column and the routes above it in the centre
    pub fn settings_split_index(&self) -> Option<usize> {
        self.marked_settings_index()
    }

    /// section open beside the settings list so the list can mark its row, none in the phone stack
    pub fn settings_open_section(&self) -> Option<&RootRoute> {
        if !self.settings_list_beside_stack {
            return None;
        }
        self.marked_settings_index().and_then(|index| self.path.get(index + 1))
    }

    /// a settings row opens its section
    /// beside the list it acts like a split sidebar, the picked section replaces the open one
    pub fn open_from_settings_list(&mut self, route: RootRoute, cx: &mut Context<Self>) {
        let index = self.marked_settings_index().filter(|_| self.settings_list_beside_stack);
        tracing::warn!(
            target: "riseonly::X1418",
            ?route,
            ?index,
            beside = self.settings_list_beside_stack,
            path = self.path.len(),
            holding = self.push_touch_hold_generation.is_some(),
            "settings list opens"
        );
        let Some(index) = index else {
            self.push(route, NavigationStoreSettlingPush::Drop, cx);
            return;
        };
        if self.path.len() == index + 2 && self.path[index + 1] == route {
            return;
        }
        if self.path.len() <= index + 1 {
            self.push_settings_section(route, cx);
            return;
        }
        self.deferred_push_generation = None;
        self.path.truncate(index + 2);
        self.replace_top(route, cx);
    }

    pub fn open_first_settings_section(&mut self, route: RootRoute, cx: &mut Context<Self>) {
        if self.deferred_push_generation.is_some() {
            return;
        }
        self.push_settings_section(route, cx);
    }

    /// a section beside the settings list opens at once, it never waits out the touch hold
    fn push_settings_section(&mut self, route: RootRoute, cx: &mut Context<Self>) {
        if self.opens_in_aside(&route) || BotMiniAppPresentation::from_route(&route).is_some() {
            self.push(route, NavigationStoreSettlingPush::Wait, cx);
            return;
        }
        if self.path.last() == Some(&route) {
            return;
        }
        self.drop_embedded_forum_request();
        self.deferred_push_generation = None;
        self.cancel_pending_repeated_chat_push();
        self.perform_push(route, cx);
    }

    /// window shell tells whether the right panel is a column beside the centre
    /// layout input only so no notify
    pub fn set_aside_takes_details(&mut self, takes: bool) {
        self.aside_takes_details = takes;
    }

    pub fn aside_takes_details(&self) -> bool {
        self.aside_takes_details
    }

    pub fn set_aside_hosts_chat_info(&mut self, hosts: bool) {
        self.aside_hosts_chat_info = hosts;
    }

    pub fn aside_hosts_chat_info(&self) -> bool {
        self.aside_hosts_chat_info
    }

    fn opens_in_aside(&self, route: &RootRoute) -> bool {
        let chat_info = self.aside_hosts_chat_info && aside_registry::is_chat_info(route);
        let chat_tool = self.aside_hosts_chat_info
            && aside_registry::is_chat_tool(route)
            && (self.path.last().is_some_and(aside_registry::is_conversation) || self.is_embedded_forum_settings(route));
        let activity = self.aside_hosts_chat_info && matches!(route, RootRoute::Notifications);
        let opens = chat_tool
            || activity
            || ((self.aside_takes_details || chat_info) && aside_registry::opens_beside_centre(route));
        if matches!(route, RootRoute::Notifications) {
            tracing::warn!(target: "riseonly::X1324", takes = self.aside_takes_details, hosts = self.aside_hosts_chat_info, opens, tab = ?self.current_tab, path = self.path.len(), "notifications push");
        }
        opens
    }

    fn is_embedded_forum_settings(&self, route: &RootRoute) -> bool {
        let RootRoute::ChatSettings { chat_id: Some(id) } = route else {
            return false;
        };
        (self.path.is_empty() || self.path.last().is_some_and(aside_registry::is_conversation))
            && self.embedded_forum_chat_id() == Some(id.as_str())
    }

    /// a chat list row opens its conversation and replaces the open one
    /// so back closes the chat and doesnt return to the previous pick
    pub fn open_from_chat_list(&mut self, route: RootRoute, cx: &mut Context<Self>) {
        if self.current_tab != MainTab::Chats || self.path.is_empty() {
            self.push(route, NavigationStoreSettlingPush::default(), cx);
            return;
        }
        if self.path.len() == 1 && self.path[0] == route {
            return;
        }
        self.deferred_push_generation = None;
        self.path.truncate(1);
        self.settle_tab_switch_floor();
        self.replace_top(route, cx);
    }

    pub fn bot_mini_app_presentation(&self) -> Option<&BotMiniAppPresentation> {
        self.bot_mini_app_presentation.as_ref()
    }

    pub fn sai_active_screen_overlay_params(&self) -> &HashMap<String, String> {
        &self.sai_active_screen_overlay_params
    }

    pub fn sai_active_screen_overlay(&self) -> Option<&SaiActiveScreenOverlay> {
        self.sai_active_screen_overlay.as_ref()
    }

    /// the group whose forum is open in the current tab, under everything in the path
    pub fn embedded_forum_chat_id(&self) -> Option<&str> {
        let overlay = self.sai_active_screen_overlay.as_ref()?;
        let forum_home_id = (RootRoute::ForumHome { chat_id: None }).sai_active_screen().id;
        if overlay.tab != self.current_tab || overlay.screen.id != forum_home_id {
            return None;
        }
        let chat_id = overlay.screen.params.get(guide_screen_keys::CHAT_ID)?;
        if chat_id.is_empty() {
            return None;
        }
        Some(chat_id.as_str())
    }

    pub fn embedded_forum_request(&self) -> Option<&EmbeddedForumRequest> {
        self.embedded_forum_request.as_ref()
    }

    pub fn chat_presentation_epoch(&self) -> Option<Uuid> {
        self.chat_presentation_epoch
    }

    pub fn chat_first_frame_ready_epoch(&self) -> Option<Uuid> {
        self.chat_first_frame_ready_epoch
    }

    pub fn is_window_user_interaction_enabled(&self) -> bool {
        self.push_touch_hold_generation.is_none()
    }

    pub fn attach_navigation_window(&mut self, stack: &Rc<dyn SharedElementNavigationController>) {
        self.navigation_window = Some(Rc::downgrade(stack));
    }

    /// chat destinations are prepared before the path changes
    /// the message list hides its first viewport until it has a full prepared commit
    pub fn push(&mut self, route: RootRoute, settling: NavigationStoreSettlingPush, cx: &mut Context<Self>) {
        if matches!(route, RootRoute::Settings) {
            self.settings_opened_from_rail = false;
        }
        if self.opens_in_aside(&route) {
            cx.defer(move |cx| {
                if let Some(aside) = navigation_aside_hooks() {
                    (aside.open)(route, cx);
                }
            });
            return;
        }
        self.push_in_centre(route, settling, cx);
    }

    /// push without the panel, a page reached with nothing behind it is the centres own screen
    pub fn push_in_centre(&mut self, route: RootRoute, settling: NavigationStoreSettlingPush, cx: &mut Context<Self>) {
        self.drop_embedded_forum_request();
        if let Some(presentation) = BotMiniAppPresentation::from_route(&route) {
            self.present_bot_mini_app(presentation, cx);
            return;
        }
        if self.open_forum_beside_chats(&route, cx) {
            return;
        }
        if settling == NavigationStoreSettlingPush::Wait && self.push_touch_hold_generation.is_some() {
            tracing::warn!(target: "riseonly::X1458", ?route, "centre push deferred until the hold ends");
            let generation = Uuid::new_v4();
            let tab = self.current_tab;
            self.deferred_push_generation = Some(generation);
            cx.spawn(async move |this, cx| {
                let started = RiseClock::monotonic();
                loop {
                    cx.background_executor().timer(SETTLE_POLL).await;
                    let waiting = this
                        .read_with(cx, |store, _| {
                            store.push_touch_hold_generation.is_some()
                                && store.deferred_push_generation == Some(generation)
                        })
                        .unwrap_or(false);
                    if !waiting || RiseClock::monotonic().saturating_sub(started) >= SETTLE_LIMIT {
                        break;
                    }
                }
                let _ = this.update(cx, |store, cx| {
                    if store.deferred_push_generation != Some(generation) {
                        return;
                    }
                    store.deferred_push_generation = None;
                    tracing::warn!(target: "riseonly::X1458", same_tab = store.current_tab == tab, on_top = store.path.last() == Some(&route), "deferred centre push resumes");
                    if store.current_tab != tab {
                        Sai::log(LOG_NAME, || "deferred push dropped after tab switch", SaiLogKind::Ui, cx);
                        return;
                    }
                    if store.path.last() == Some(&route) {
                        return;
                    }
                    store.cancel_pending_repeated_chat_push();
                    store.perform_push(route, cx);
                });
            })
            .detach();
            return;
        }
        if self.path.last() == Some(&route) {
            self.enqueue_repeated_chat_push(route, cx);
            return;
        }
        if self.push_touch_hold_generation.is_some() {
            tracing::warn!(target: "riseonly::X1458", ?route, "centre push dropped while a push settles");
            Sai::log(LOG_NAME, || "push dropped while previous push settles", SaiLogKind::Ui, cx);
            return;
        }
        self.deferred_push_generation = None;
        self.cancel_pending_repeated_chat_push();
        self.perform_push(route, cx);
    }

    fn perform_push(&mut self, route: RootRoute, cx: &mut Context<Self>) {
        self.bot_mini_app_presentation = None;
        self.prepare_first_frame_transition(&route);
        self.hold_touches_during_push(cx);
        self.path.push(route);
        cx.notify();
        self.notify_sai_active_screen_changed(cx);
    }

    fn hold_touches_during_push(&mut self, cx: &mut Context<Self>) {
        let Some(window) = Self::navigation_window(&self.navigation_window) else {
            return;
        };
        let generation = Uuid::new_v4();
        self.push_touch_hold_generation = Some(generation);
        let source_top = window.top_view_controller(cx);
        let stack = Rc::downgrade(&window);
        cx.spawn(async move |this, cx| {
            let started = RiseClock::monotonic();
            cx.background_executor().timer(PUSH_TOUCH_HOLD_MINIMUM).await;
            loop {
                let settling = cx.update(|cx| {
                    let Some(stack) = stack.upgrade() else {
                        return false;
                    };
                    Self::has_running_transition(&stack, cx)
                        || (source_top.is_some() && stack.top_view_controller(cx) == source_top)
                });
                if !settling || RiseClock::monotonic().saturating_sub(started) >= SETTLE_LIMIT {
                    break;
                }
                cx.background_executor().timer(SETTLE_POLL).await;
            }
            cx.background_executor().timer(PUSH_TOUCH_HOLD_TAIL).await;
            let _ = this.update(cx, |store, _| {
                if store.push_touch_hold_generation != Some(generation) {
                    return;
                }
                store.push_touch_hold_generation = None;
            });
        })
        .detach();
    }

    fn navigation_window(
        navigation_window: &Option<Weak<dyn SharedElementNavigationController>>,
    ) -> Option<Rc<dyn SharedElementNavigationController>> {
        navigation_window.as_ref().and_then(Weak::upgrade)
    }

    fn has_running_transition(stack: &Rc<dyn SharedElementNavigationController>, cx: &App) -> bool {
        stack.transition_coordinator(cx).is_some()
    }

    /// route stack of the window, for the back pan installed on it
    pub fn navigation_window_stack(&self) -> Option<Rc<dyn SharedElementNavigationController>> {
        Self::navigation_window(&self.navigation_window)
    }

    /// true if the stack that owns the path is mid transition, used by back refusals
    pub fn navigation_window_has_running_transition(&self, cx: &App) -> bool {
        Self::navigation_window(&self.navigation_window).is_some_and(|stack| Self::has_running_transition(&stack, cx))
    }

    pub fn pop(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.bot_mini_app_presentation.as_ref().map(|presentation| presentation.id) {
            self.dismiss_bot_mini_app(id, cx);
            return;
        }
        self.cancel_pending_repeated_chat_push();
        self.cancel_chat_first_frame_transition(None);
        self.path.pop();
        self.settle_tab_switch_floor();
        cx.notify();
        self.clear_sai_active_screen_overlay_params(false, cx);
        self.notify_sai_active_screen_changed(cx);
    }

    pub fn replace_top(&mut self, route: RootRoute, cx: &mut Context<Self>) {
        if let Some(presentation) = BotMiniAppPresentation::from_route(&route) {
            self.cancel_pending_repeated_chat_push();
            self.cancel_chat_first_frame_transition(None);
            self.path.pop();
            self.settle_tab_switch_floor();
            cx.notify();
            self.clear_sai_active_screen_overlay_params(false, cx);
            self.present_bot_mini_app(presentation, cx);
            return;
        }
        if self.open_forum_beside_chats(&route, cx) {
            return;
        }
        self.cancel_pending_repeated_chat_push();
        self.bot_mini_app_presentation = None;
        self.prepare_first_frame_transition(&route);
        self.replace_top_path_element(route);
        cx.notify();
        self.clear_sai_active_screen_overlay_params(false, cx);
        self.notify_sai_active_screen_changed(cx);
    }

    /// a group with topics opens beside the chat list from every entry point
    /// false while the group row isnt loaded yet
    fn open_forum_beside_chats(&mut self, route: &RootRoute, cx: &mut Context<Self>) -> bool {
        let RootRoute::ForumHome { chat_id: Some(chat_id) } = route else {
            return false;
        };
        let Some(chat) = ChatServicesStore::shared(cx).read(cx).chat_info(chat_id, cx) else {
            return false;
        };
        if !chat.topics_enabled {
            return false;
        }
        EmbeddedForumPresentationStore::shared(cx).update(cx, |store, cx| store.present(chat, cx));
        self.pop_to_root(cx);
        self.switch_tab(MainTab::Chats, cx);
        true
    }

    pub fn pop_to_root(&mut self, cx: &mut Context<Self>) {
        self.drop_embedded_forum_request();
        self.cancel_pending_repeated_chat_push();
        self.cancel_chat_first_frame_transition(None);
        self.bot_mini_app_presentation = None;
        self.path.clear();
        self.clear_tab_switch_floor();
        cx.notify();
        self.clear_sai_active_screen_overlay_params(false, cx);
        self.notify_sai_active_screen_changed(cx);
    }

    /// unwinds to the route if its already in the stack, like telegram does for an open chat
    /// false when the route isnt there
    pub fn pop_to(&mut self, route: RootRoute, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.path.iter().rposition(|candidate| *candidate == route) else {
            return false;
        };
        if index + 1 >= self.path.len() {
            return true;
        }
        if !self.is_push_settling(cx) {
            self.unwind(index, cx);
            return true;
        }
        cx.spawn(async move |this, cx| {
            let started = RiseClock::monotonic();
            loop {
                cx.background_executor().timer(SETTLE_POLL).await;
                let settling = this.update(cx, |store, cx| store.is_push_settling(cx)).unwrap_or(false);
                if !settling || RiseClock::monotonic().saturating_sub(started) >= SETTLE_LIMIT {
                    break;
                }
            }
            let _ = this.update(cx, |store, cx| {
                let Some(index) = store.path.iter().rposition(|candidate| *candidate == route) else {
                    return;
                };
                if index + 1 >= store.path.len() {
                    return;
                }
                store.unwind(index, cx);
            });
        })
        .detach();
        true
    }

    fn unwind(&mut self, index: usize, cx: &mut Context<Self>) {
        self.cancel_pending_repeated_chat_push();
        self.cancel_chat_first_frame_transition(None);
        self.bot_mini_app_presentation = None;
        self.path.truncate(index + 1);
        self.settle_tab_switch_floor();
        cx.notify();
        self.clear_sai_active_screen_overlay_params(false, cx);
        self.notify_sai_active_screen_changed(cx);
    }

    pub fn close_topic(&mut self, chat_id: String, topic_id: String, cx: &mut Context<Self>) -> bool {
        let matches = move |route: &RootRoute| match route {
            RootRoute::ChatTopic { chat_id: route_chat_id, topic_id: route_topic_id }
            | RootRoute::TopicProfile { chat_id: route_chat_id, topic_id: route_topic_id } => {
                route_chat_id.as_deref() == Some(chat_id.as_str())
                    && route_topic_id.as_deref().unwrap_or_default() == topic_id.as_str()
            }
            RootRoute::GeneralProfile { chat_id: route_chat_id } => {
                topic_id.is_empty() && route_chat_id.as_deref() == Some(chat_id.as_str())
            }
            _ => false,
        };
        if !self.path.iter().any(&matches) {
            return false;
        }
        if !self.is_push_settling(cx) {
            self.unwind_topic(&matches, cx);
            return true;
        }
        cx.spawn(async move |this, cx| {
            let started = RiseClock::monotonic();
            loop {
                cx.background_executor().timer(SETTLE_POLL).await;
                let settling = this.update(cx, |store, cx| store.is_push_settling(cx)).unwrap_or(false);
                if !settling || RiseClock::monotonic().saturating_sub(started) >= SETTLE_LIMIT {
                    break;
                }
            }
            let _ = this.update(cx, |store, cx| store.unwind_topic(&matches, cx));
        })
        .detach();
        true
    }

    /// a pop during the push animation of the same screen gets lost
    /// screen stays and back does nothing so unwinds wait for it
    fn is_push_settling(&self, cx: &App) -> bool {
        if self.push_touch_hold_generation.is_some() {
            return true;
        }
        let Some(stack) = Self::navigation_window(&self.navigation_window) else {
            return false;
        };
        Self::has_running_transition(&stack, cx)
    }

    fn unwind_topic(&mut self, matches: &dyn Fn(&RootRoute) -> bool, cx: &mut Context<Self>) {
        let Some(index) = self.path.iter().position(|route| matches(route)) else {
            return;
        };
        self.cancel_pending_repeated_chat_push();
        self.cancel_chat_first_frame_transition(None);
        self.bot_mini_app_presentation = None;
        self.path.truncate(index);
        self.settle_tab_switch_floor();
        cx.notify();
        self.clear_sai_active_screen_overlay_params(false, cx);
        self.notify_sai_active_screen_changed(cx);
    }

    /// unwinds every screen of a chat the account was removed from plus anything opened over it
    /// false when the chat isnt in the stack
    pub fn close_chat(&mut self, chat_id: &str, cx: &mut Context<Self>) -> bool {
        if chat_id.is_empty() {
            return false;
        }
        let Some(index) = self.path.iter().position(|route| Self::chat_scope_id(route) == Some(chat_id)) else {
            return false;
        };
        self.cancel_pending_repeated_chat_push();
        self.cancel_chat_first_frame_transition(None);
        self.bot_mini_app_presentation = None;
        self.path.truncate(index);
        self.settle_tab_switch_floor();
        cx.notify();
        self.clear_sai_active_screen_overlay_params(false, cx);
        self.notify_sai_active_screen_changed(cx);
        true
    }

    pub fn close_playlist(&mut self, playlist_id: &str, cx: &mut Context<Self>) -> bool {
        if playlist_id.is_empty() {
            return false;
        }
        let Some(index) = self
            .path
            .iter()
            .position(|route| Self::playlist_scope_id(route) == Some(playlist_id))
        else {
            return false;
        };
        self.cancel_pending_repeated_chat_push();
        self.cancel_chat_first_frame_transition(None);
        self.bot_mini_app_presentation = None;
        self.path.truncate(index);
        self.settle_tab_switch_floor();
        cx.notify();
        self.clear_sai_active_screen_overlay_params(false, cx);
        self.notify_sai_active_screen_changed(cx);
        true
    }

    fn playlist_scope_id(route: &RootRoute) -> Option<&str> {
        match route {
            RootRoute::MusicPlaylistDetail { collection_id: id }
            | RootRoute::MusicPlaylistLink { playlist_id: id }
            | RootRoute::MusicPlaylistSettings { playlist_id: id }
            | RootRoute::MusicPlaylistInviteLink { playlist_id: id }
            | RootRoute::MusicPlaylistJoinRequests { playlist_id: id }
            | RootRoute::MusicPlaylistMembers { playlist_id: id } => Some(id.as_str()),
            RootRoute::MusicPlaylistTrackLink { playlist_id: id, .. }
            | RootRoute::MusicPlaylistMemberRights { playlist_id: id, .. } => Some(id.as_str()),
            _ => None,
        }
    }

    pub fn chat_scope_id(route: &RootRoute) -> Option<&str> {
        match route {
            RootRoute::Chat { chat_id: id }
            | RootRoute::ChatProfile { chat_id: id }
            | RootRoute::ChatSettings { chat_id: id }
            | RootRoute::ChatSettingsType { chat_id: id }
            | RootRoute::ChatSettingsLinks { chat_id: id }
            | RootRoute::ChatSettingsLinkedChat { chat_id: id }
            | RootRoute::ChatStatistics { chat_id: id }
            | RootRoute::ChatSettingsAllowedReactions { chat_id: id }
            | RootRoute::ChatSettingsMembers { chat_id: id }
            | RootRoute::ChatSettingsPermissions { chat_id: id }
            | RootRoute::ChatSettingsAdmins { chat_id: id }
            | RootRoute::ChatSettingsBlacklist { chat_id: id }
            | RootRoute::ChatSettingsRecentActions { chat_id: id }
            | RootRoute::ChatTopics { chat_id: id }
            | RootRoute::ForumHome { chat_id: id }
            | RootRoute::ForumAll { chat_id: id }
            | RootRoute::GeneralProfile { chat_id: id }
            | RootRoute::ChatRoles { chat_id: id }
            | RootRoute::PinnedMessages { chat_id: id, .. } => id.as_deref(),
            RootRoute::ChatTopic { chat_id: id, .. }
            | RootRoute::ChatThread { chat_id: id, .. }
            | RootRoute::ChatWallpaperPicker { chat_id: id, .. }
            | RootRoute::TopicProfile { chat_id: id, .. }
            | RootRoute::ChatRoleEdit { chat_id: id, .. } => id.as_deref(),
            RootRoute::GroupVoiceSettings { chat_id: id } => Some(id.as_str()),
            _ => None,
        }
    }

    pub fn parked_tab_routes(&self) -> Vec<RootRoute> {
        self.tab_paths.values().flatten().cloned().collect()
    }

    pub fn restore(&mut self, snapshot: Vec<RootRoute>, tab: MainTab, cx: &mut Context<Self>) {
        self.cancel_pending_repeated_chat_push();
        self.cancel_chat_first_frame_transition(None);
        self.bot_mini_app_presentation = None;
        self.tab_paths.remove(&tab);
        self.current_tab = tab;
        self.path = snapshot;
        self.clear_tab_switch_floor();
        cx.notify();
        self.clear_sai_active_screen_overlay_params(false, cx);
        self.notify_sai_active_screen_changed(cx);
    }

    pub fn set_sai_active_screen_overlay_params(&mut self, params: HashMap<String, String>, cx: &mut Context<Self>) {
        self.sai_active_screen_overlay_params = params;
        cx.notify();
    }

    pub fn request_embedded_forum(&mut self, chat_id: Option<String>, cx: &mut Context<Self>) {
        self.embedded_forum_request = Some(EmbeddedForumRequest::new(chat_id));
        cx.notify();
    }

    fn drop_embedded_forum_request(&mut self) {
        if self.embedded_forum_request.take().is_some() {
            tracing::warn!(target: "riseonly::X2078", "stale embedded forum request dropped by navigation");
        }
    }

    pub fn set_sai_active_screen_overlay(&mut self, overlay: Option<SaiActiveScreenOverlay>, cx: &mut Context<Self>) {
        if self.sai_active_screen_overlay == overlay {
            return;
        }
        self.sai_active_screen_overlay = overlay;
        cx.notify();
        self.notify_sai_active_screen_changed(cx);
    }

    pub fn clear_sai_active_screen_overlay_params(&mut self, notify: bool, cx: &mut Context<Self>) {
        if self.sai_active_screen_overlay_params.is_empty() {
            return;
        }
        self.sai_active_screen_overlay_params = HashMap::new();
        cx.notify();
        if notify {
            self.notify_sai_active_screen_changed(cx);
        }
    }

    pub fn switch_tab(&mut self, tab: MainTab, cx: &mut Context<Self>) {
        self.cancel_pending_repeated_chat_push();
        self.cancel_chat_first_frame_transition(None);
        self.bot_mini_app_presentation = None;
        let dropped = self.tab_paths.remove(&tab).unwrap_or_default();
        if tab != self.current_tab {
            self.tab_switch_origin = (!self.path.is_empty()).then_some((self.current_tab, dropped));
            self.current_tab = tab;
            self.tab_switch_floor = Some(self.path.len());
        } else if self.tab_switch_origin.is_none() {
            self.tab_switch_floor = Some(self.path.len());
        }
        cx.notify();
        self.notify_sai_active_screen_changed(cx);
    }

    /// the section a link opens its page in
    /// a link tapped in an open chat keeps the chats section so back returns to the chat
    pub fn switch_tab_for_link(&mut self, tab: MainTab, cx: &mut Context<Self>) {
        if self.current_tab == MainTab::Chats && !self.path.is_empty() {
            return;
        }
        self.switch_tab(tab, cx);
    }

    /// route opened from the rail foot or the palette
    /// picking settings while open pops to their root instead of stacking a second one
    pub fn open_rail_route(&mut self, route: RootRoute, settling: NavigationStoreSettlingPush, cx: &mut Context<Self>) {
        if matches!(route, RootRoute::Settings)
            && let Some(index) = self.path.iter().rposition(|candidate| matches!(candidate, RootRoute::Settings))
        {
            if index + 1 < self.path.len() {
                self.unwind(index, cx);
            }
            self.clear_tab_switch_floor();
            cx.notify();
            return;
        }
        let opens_settings = matches!(route, RootRoute::Settings);
        self.push(route, settling, cx);
        if opens_settings {
            self.settings_opened_from_rail = true;
        }
    }

    /// true when settings draws its back
    /// from a page it keeps the phone back, from the rail its a section root
    pub fn shows_settings_back(&self) -> bool {
        if self.marked_settings_index().is_none() {
            return true;
        }
        if self.settings_list_beside_stack || self.shows_settings_section_root() {
            return !self.settings_opened_from_rail;
        }
        true
    }

    /// back of the settings page
    /// beside the list it closes settings with every section over them and returns to the page under
    pub fn pop_settings(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.marked_settings_index().filter(|_| self.settings_list_beside_stack) else {
            self.pop(cx);
            return;
        };
        self.cancel_pending_repeated_chat_push();
        self.cancel_chat_first_frame_transition(None);
        self.bot_mini_app_presentation = None;
        self.deferred_push_generation = None;
        self.path.truncate(index);
        self.settle_tab_switch_floor();
        cx.notify();
        self.clear_sai_active_screen_overlay_params(false, cx);
        self.notify_sai_active_screen_changed(cx);
    }

    /// routes pushed since the last programmatic tab switch or the whole stack
    /// the rail always shows so a link switching section over open settings marks the section
    pub fn routes_since_tab_switch(&self) -> &[RootRoute] {
        let floor = self.tab_switch_floor.unwrap_or(0).min(self.path.len());
        &self.path[floor..]
    }

    /// where the settings the rail marks start in the stack
    fn marked_settings_index(&self) -> Option<usize> {
        let floor = self.tab_switch_floor.unwrap_or(0).min(self.path.len());
        self.path[floor..].iter().position(|route| matches!(route, RootRoute::Settings)).map(|index| floor + index)
    }

    /// true when the stack top is the root of the settings the rail marks, so no back
    pub fn shows_settings_section_root(&self) -> bool {
        matches!(self.path.last(), Some(RootRoute::Settings))
            && self.marked_settings_index().is_some_and(|index| index + 1 == self.path.len())
    }

    fn settle_tab_switch_floor(&mut self) {
        if !self.tab_switch_floor.is_some_and(|floor| self.path.len() <= floor) {
            return;
        }
        self.tab_switch_floor = None;
        let Some((origin, dropped)) = self.tab_switch_origin.take() else {
            return;
        };
        if origin == self.current_tab || self.path.is_empty() {
            return;
        }
        if !dropped.is_empty() {
            self.tab_paths.insert(self.current_tab, dropped);
        }
        self.tab_paths.remove(&origin);
        self.current_tab = origin;
    }

    fn clear_tab_switch_floor(&mut self) {
        self.tab_switch_floor = None;
        self.tab_switch_origin = None;
    }

    /// a section picked on the rail, keyboard or palette
    /// every section keeps its own stack
    /// false if nothing was pushed so caller scrolls to top
    pub fn pick_tab(&mut self, tab: MainTab, cx: &mut Context<Self>) -> bool {
        let settings_index = self.marked_settings_index();
        if tab == self.current_tab {
            if self.path.is_empty() {
                return false;
            }
            match settings_index {
                Some(index) if index > 0 => {
                    let below_settings = self.path[..index].to_vec();
                    self.restore(below_settings, tab, cx);
                }
                _ => self.pop_to_root(cx),
            }
            return true;
        }
        if let Some(floor) = self.tab_switch_floor.filter(|floor| *floor > 0 && *floor <= self.path.len())
            && let Some((origin, dropped)) = self.tab_switch_origin.take_if(|(origin, _)| *origin == tab)
        {
            let mut opened = dropped;
            opened.extend(self.path.split_off(floor));
            if opened.is_empty() {
                self.tab_paths.remove(&self.current_tab);
            } else {
                self.tab_paths.insert(self.current_tab, opened);
            }
            let below = std::mem::take(&mut self.path);
            self.restore(below, origin, cx);
            return true;
        }
        let mut leaving = std::mem::take(&mut self.path);
        if let Some(index) = settings_index {
            leaving.truncate(index);
        }
        let arriving = self.tab_paths.remove(&tab).unwrap_or_default();
        let keeps_root = leaving.is_empty() && arriving.is_empty();
        if leaving.is_empty() {
            self.tab_paths.remove(&self.current_tab);
        } else {
            self.tab_paths.insert(self.current_tab, leaving);
        }
        if keeps_root {
            self.switch_tab(tab, cx);
        } else {
            self.restore(arriving, tab, cx);
        }
        true
    }

    pub fn go_to_main_tabs_posts(&mut self, cx: &mut Context<Self>) {
        self.cancel_pending_repeated_chat_push();
        self.cancel_chat_first_frame_transition(None);
        self.bot_mini_app_presentation = None;
        self.path.clear();
        self.tab_paths.clear();
        self.clear_tab_switch_floor();
        self.current_tab = MainTab::Posts;
        cx.notify();
        self.notify_sai_active_screen_changed(cx);
    }

    pub fn complete_chat_first_frame_transition(
        &mut self,
        route: &RootRoute,
        epoch: Option<Uuid>,
        cx: &mut Context<Self>,
    ) {
        let Some(epoch) = epoch else {
            return;
        };
        if self.chat_presentation_epoch != Some(epoch)
            || self.chat_first_frame_eligible_epoch != Some(epoch)
            || self.chat_presentation_route.as_ref() != Some(route)
            || self.path.last() != Some(route)
        {
            return;
        }
        let Some(scope) = self.chat_presentation_scope.clone() else {
            return;
        };
        self.chat_first_frame_eligible_epoch = None;
        self.chat_first_frame_ready_epoch = Some(epoch);
        cx.notify();
        ChatPerformanceTrace::shared().record_scope_event(CHAT_OPEN_FIRST_FRAME_READY_EVENT, &scope, HashMap::new());
    }

    pub fn cancel_chat_first_frame_transition(&mut self, scope: Option<&str>) {
        let Some(active_scope) = self.chat_presentation_scope.as_deref() else {
            return;
        };
        if scope.is_some_and(|scope| scope != active_scope) {
            return;
        }
        self.chat_first_frame_eligible_epoch = None;
    }

    pub fn validate_chat_first_frame_transition_path(&mut self, cx: &mut Context<Self>) {
        self.resolve_pending_repeated_chat_push_if_possible(cx);
        if self.chat_first_frame_eligible_epoch.is_none() {
            return;
        }
        let Some(route) = self.chat_presentation_route.as_ref() else {
            return;
        };
        if self.path.last() != Some(route) {
            self.cancel_chat_first_frame_transition(None);
        }
    }

    /// ownership token a new destination captures and echoes with its first frame callback
    /// independent of the route being on top because a pop removes it while the screen still settles
    pub fn chat_presentation_epoch_for(&self, route: &RootRoute) -> Option<Uuid> {
        if self.chat_presentation_route.as_ref() != Some(route) {
            return None;
        }
        self.chat_presentation_epoch
    }

    fn prepare_first_frame_transition(&mut self, route: &RootRoute) {
        let Some(scope) = route.chat_first_frame_transition_scope() else {
            self.cancel_chat_first_frame_transition(None);
            return;
        };
        let epoch = Uuid::new_v4();
        self.chat_presentation_epoch = Some(epoch);
        self.chat_first_frame_ready_epoch = None;
        self.chat_presentation_route = Some(route.clone());
        self.chat_presentation_scope = Some(scope.clone());
        self.chat_first_frame_eligible_epoch = Some(epoch);
        let fields = HashMap::from([(EPOCH_FIELD.to_owned(), epoch.uuid_string())]);
        ChatPerformanceTrace::shared().record_scope_event(CHAT_OPEN_PUSH_EVENT, &scope, fields);
    }

    fn enqueue_repeated_chat_push(&mut self, route: RootRoute, cx: &mut Context<Self>) {
        self.pending_repeated_chat_push_expiry = None;
        self.deferred_repeated_chat_push_id = None;

        let source_len = self.path.len().saturating_sub(1);
        let request = NavigationStorePendingRepeatedChatPush {
            id: Uuid::new_v4(),
            route,
            expected_source_path: self.path[..source_len].to_vec(),
        };
        let request_id = request.id;
        self.pending_repeated_chat_push = Some(request);
        self.pending_repeated_chat_push_expiry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REPEATED_CHAT_PUSH_EXPIRY).await;
            let _ = this.update(cx, |store, _| {
                if store.pending_repeated_chat_push.as_ref().map(|pending| pending.id) != Some(request_id) {
                    return;
                }
                store.pending_repeated_chat_push = None;
                store.deferred_repeated_chat_push_id = None;
                if let Some(expiry) = store.pending_repeated_chat_push_expiry.take() {
                    expiry.detach();
                }
            });
        }));
    }

    fn resolve_pending_repeated_chat_push_if_possible(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.pending_repeated_chat_push.clone() else {
            return;
        };
        if self.path == request.expected_source_path {
            self.pending_repeated_chat_push_expiry = None;
            self.pending_repeated_chat_push = None;
            self.deferred_repeated_chat_push_id = Some(request.id);
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |store, cx| {
                    if store.deferred_repeated_chat_push_id != Some(request.id)
                        || store.path != request.expected_source_path
                    {
                        return;
                    }
                    store.deferred_repeated_chat_push_id = None;
                    store.perform_push(request.route, cx);
                });
            })
            .detach();
            return;
        }

        if self.path.last() != Some(&request.route) {
            self.cancel_pending_repeated_chat_push();
        }
    }

    fn cancel_pending_repeated_chat_push(&mut self) {
        self.pending_repeated_chat_push_expiry = None;
        self.pending_repeated_chat_push = None;
        self.deferred_repeated_chat_push_id = None;
    }

    fn replace_top_path_element(&mut self, route: RootRoute) {
        match self.path.last_mut() {
            Some(top) => *top = route,
            None => self.path.push(route),
        }
    }

    fn notify_sai_active_screen_changed(&self, cx: &mut Context<Self>) {
        cx.defer(|cx| SaiWs::notify_active_screen_changed(cx));
    }

    pub fn dismiss_bot_mini_app(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.bot_mini_app_presentation.as_ref().map(|presentation| presentation.id) != Some(id) {
            return;
        }
        self.bot_mini_app_presentation = None;
        cx.notify();
        self.clear_sai_active_screen_overlay_params(false, cx);
        self.notify_sai_active_screen_changed(cx);
    }

    pub fn is_bot_mini_app_presentation_active(&self, id: Uuid) -> bool {
        self.bot_mini_app_presentation.as_ref().map(|presentation| presentation.id) == Some(id)
    }

    fn present_bot_mini_app(&mut self, presentation: BotMiniAppPresentation, cx: &mut Context<Self>) {
        self.bot_mini_app_presentation = Some(presentation);
        cx.notify();
        self.clear_sai_active_screen_overlay_params(false, cx);
        self.notify_sai_active_screen_changed(cx);
    }

    pub fn sai_active_screen(&self) -> SaiActiveScreen {
        if let Some(presentation) = self.bot_mini_app_presentation.as_ref() {
            return presentation.route().sai_active_screen();
        }
        if let Some(top) = self.path.last() {
            let base = top.sai_active_screen();
            if self.sai_active_screen_overlay_params.is_empty() {
                return base;
            }
            let mut merged = base.params;
            for (key, value) in &self.sai_active_screen_overlay_params {
                merged.insert(key.clone(), value.clone());
            }
            return SaiActiveScreen::new(base.id, merged);
        }
        if let Some(overlay) = self.sai_active_screen_overlay.as_ref()
            && overlay.tab == self.current_tab
        {
            return overlay.screen.clone();
        }
        SaiActiveScreen::new(self.current_tab.sai_screen_id().to_owned(), HashMap::new())
    }
}

impl RootRoute {
    pub fn chat_first_frame_transition_scope(&self) -> Option<String> {
        match self {
            RootRoute::Chat { chat_id } => chat_id.as_ref().filter(|chat_id| !chat_id.is_empty()).cloned(),
            RootRoute::ChatTopic { chat_id, .. } => chat_id.as_ref().filter(|chat_id| !chat_id.is_empty()).cloned(),
            RootRoute::ChatThread { chat_id, root_id } => {
                let chat_id = chat_id.as_deref().filter(|chat_id| !chat_id.is_empty())?;
                Some(MessageServicesStore::message_scope(chat_id, None, root_id.as_deref()))
            }
            RootRoute::ForumAll { chat_id } => chat_id.as_ref().filter(|chat_id| !chat_id.is_empty()).cloned(),
            _ => None,
        }
    }
}

pub struct NavigationStoreKey;

impl EnvKey for NavigationStoreKey {
    type Value = Option<Entity<NavigationStore>>;

    fn default_value() -> Self::Value {
        None
    }
}

/// routeless screen over a tab root, which tab it covers and what it reports as the active screen
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SaiActiveScreenOverlay {
    pub tab: MainTab,
    pub screen: SaiActiveScreen,
}

/// one request to the chats tab about its embedded forum
/// stamped so two identical requests in a row both get delivered
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EmbeddedForumRequest {
    pub id: Uuid,
    pub chat_id: Option<String>,
}

impl EmbeddedForumRequest {
    pub fn new(chat_id: Option<String>) -> Self {
        Self { id: Uuid::new_v4(), chat_id }
    }
}
