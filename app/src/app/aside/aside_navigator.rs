use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{AnyElement, AnyView, App, AppContext, Context, Entity, EntityId, EventEmitter, Global, IntoElement, Render, StyleRefinement, Subscription, Visibility, WeakEntity, Window, div, prelude::*};
use rise_ui::swiftui::environment::env_scope;
use rise_ui::page_header_ui::{PageBesideSplitListKey, PanelChrome, PanelChromeKey, PanelChromeSlot};
use rise_ui::page_header_toolbar_button::{PageHeaderAction, PageHeaderMetrics};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiActiveScreen;
use crate::app::router::root_route_destination::RootRouteDestination;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::comment::widgets::comments_sheet::DockedCommentsSheetHost;

use super::aside_memory::AsideMemory;
use super::aside_registry::{self, AsideSection};

pub use crate::modules::music::shared::utils::music::player_beside_hooks::{player_beside, set_player_beside};

/// asks the panel to show a page and the window shell opens the column or slide-over
/// toggles means the page is already shown so a second click closes the panel
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AsideReveal {
    pub toggles: bool,
}

struct AsidePage {
    route: RootRoute,
    view: AnyView,
}

/// the right panel as a second navigator with its own route stack
/// pushes from inside the panel stay in the panel
/// empty stack shows the section default route
pub struct AsideNavigator {
    store: Entity<NavigationStore>,
    centre: WeakEntity<NavigationStore>,
    defaults: HashMap<AsideSection, RootRoute>,
    section: Option<AsideSection>,
    pages: Vec<AsidePage>,
    parked: HashMap<Option<AsideSection>, Vec<AsidePage>>,
    recalled_stacks: HashMap<AsideSection, Vec<RootRoute>>,
    default_page: Option<AsidePage>,
    profile_owner: Option<String>,
    account: Option<String>,
    recalled: bool,
    comments_docked: bool,
    comments_thread_docked: bool,
    panel_shown: bool,
    top_drawn: Option<EntityId>,
    drawn_pages: HashSet<EntityId>,
    chrome_taken: HashMap<EntityId, Rc<Cell<bool>>>,
    section_visits: u64,
    comments_dock: Entity<DockedCommentsSheetHost>,
    dock_row_steps_back: bool,
    _store_observation: Subscription,
    _comments_observation: Subscription,
    _self_observation: Subscription,
}

impl AsideNavigator {
    pub fn new(centre: &Entity<NavigationStore>, cx: &mut Context<Self>) -> Self {
        let store = cx.new(NavigationStore::new);
        let store_observation = cx.observe(&store, |navigator, _, cx| navigator.sync_pages(cx));
        let self_observation = cx.observe_self(|navigator, cx| navigator.refresh_dock_row(cx));
        let comments = CommentInteractionsStore::shared(cx);
        let comments_observation = cx.observe(&comments, |navigator, comments, cx| {
            let docked = comments.read(cx).comments_sheet_visible && navigator.docks_comments(cx);
            if docked && navigator.comments_docked && !navigator.panel_shown {
                tracing::warn!(target: "riseonly::X1548", "docked comments in hidden panel, reveal");
                cx.emit(AsideReveal { toggles: false });
            }
            if docked != navigator.comments_docked {
                navigator.comments_docked = docked;
                if docked {
                    cx.emit(AsideReveal { toggles: false });
                }
                cx.notify();
            }
            let thread_docked = docked && comments.read(cx).thread_root().is_some();
            if thread_docked != navigator.comments_thread_docked {
                navigator.comments_thread_docked = thread_docked;
                cx.notify();
            }
        });
        Self {
            store,
            centre: centre.downgrade(),
            defaults: Self::start_defaults(),
            section: None,
            pages: Vec::new(),
            parked: HashMap::new(),
            recalled_stacks: HashMap::new(),
            default_page: None,
            profile_owner: None,
            account: None,
            recalled: false,
            comments_docked: false,
            comments_thread_docked: false,
            panel_shown: false,
            top_drawn: None,
            drawn_pages: HashSet::new(),
            chrome_taken: HashMap::new(),
            section_visits: 0,
            comments_dock: cx.new(|_| DockedCommentsSheetHost::default()),
            dock_row_steps_back: false,
            _store_observation: store_observation,
            _comments_observation: comments_observation,
            _self_observation: self_observation,
        }
    }

    fn start_defaults() -> HashMap<AsideSection, RootRoute> {
        HashMap::from([
            (AsideSection::Posts, RootRoute::MyProfile),
            (AsideSection::Chats, RootRoute::MyProfile),
            (AsideSection::Search, RootRoute::MyProfile),
            (AsideSection::Maps, RootRoute::MapFriends),
        ])
    }

    pub fn store(&self) -> &Entity<NavigationStore> {
        &self.store
    }

    /// what the section shows with an empty stack
    fn default_route(&self, section: AsideSection) -> Option<&RootRoute> {
        self.defaults.get(&section)
    }

    pub fn top_wants_centre(&self) -> bool {
        self.pages.last().is_some_and(|page| !aside_registry::lives_in_panel(&page.route))
    }

    /// route on top of the panel, stack top or else the section default
    pub fn top_route(&self) -> Option<&RootRoute> {
        self.pages.last().or(self.default_page.as_ref()).map(|page| &page.route)
    }

    /// true when post or riff comments open in the panel beside the feed
    /// also over a post page, narrow windows keep the bottom sheet
    pub fn docks_comments(&self, cx: &App) -> bool {
        let centre = self.centre.upgrade();
        let takes_details = centre.as_ref().is_some_and(|centre| centre.read(cx).aside_takes_details());
        let centre_is_post = centre.as_ref().is_some_and(|centre| {
            matches!(
                centre.read(cx).path.last(),
                Some(RootRoute::PostDetail { .. } | RootRoute::UserPage { .. } | RootRoute::MyProfile)
            )
        });
        let panel_exists = centre.as_ref().is_some_and(|centre| centre.read(cx).aside_hosts_chat_info());
        (matches!(self.section, Some(AsideSection::Posts | AsideSection::Riffs)) && takes_details)
            || (centre_is_post && (takes_details || panel_exists))
            || (matches!(self.top_route(), Some(RootRoute::PostDetail { .. })) && (takes_details || self.panel_shown))
    }

    pub fn set_panel_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.panel_shown == shown {
            return;
        }
        self.panel_shown = shown;
        if !shown {
            self.top_drawn = None;
        }
        crate::core::guides::guide_screen_resolver::GuideScreenObserver::visible_did_change(cx);
        if self.comments_docked && (!shown || !self.docks_comments(cx)) {
            self.comments_docked = false;
            CommentInteractionsStore::shared(cx).update(cx, |store, cx| store.close_comments_sheet(cx));
        }
    }

    pub fn is_panel_shown(&self) -> bool {
        self.panel_shown
    }

    pub fn can_go_back(&self) -> bool {
        !self.pages.is_empty()
    }

    /// true if the panel has any page for this section, without one the centre takes the room
    pub fn has_page_for(&self, section: Option<AsideSection>) -> bool {
        !self.pages.is_empty()
            || self.comments_docked
            || section.is_some_and(|section| self.default_route(section).is_some())
    }

    pub fn set_section(&mut self, section: Option<AsideSection>, cx: &mut Context<Self>) {
        self.recall_stack(cx);
        if self.section == section {
            return;
        }
        let leaving = std::mem::take(&mut self.pages);
        self.parked.insert(self.section, leaving);
        self.section = section;
        self.section_visits += 1;
        if section == Some(AsideSection::Riffs) {
            self.parked.remove(&section);
        }
        self.bring_section_stack(cx);
        if self.comments_docked && !self.docks_comments(cx) {
            self.comments_docked = false;
            CommentInteractionsStore::shared(cx).update(cx, |store, cx| store.close_comments_sheet(cx));
        }
        self.sync_default_page(cx);
        cx.notify();
    }

    /// counts section changes so a page sees a fresh visit even if its values are the same
    pub fn section_visits(&self) -> u64 {
        self.section_visits
    }

    pub fn open(&mut self, route: RootRoute, cx: &mut Context<Self>) {
        self.undock_comments(cx);
        if self.default_page.as_ref().is_some_and(|page| page.route == route) {
            let toggles = self.pages.is_empty();
            self.reset_to_default(cx);
            cx.emit(AsideReveal { toggles });
            return;
        }
        if self.store.read(cx).path.last() == Some(&route) {
            cx.emit(AsideReveal { toggles: false });
            return;
        }
        self.store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        cx.emit(AsideReveal { toggles: false });
    }

    pub fn open_over_default(&mut self, route: RootRoute, cx: &mut Context<Self>) {
        if self.pages.is_empty() {
            self.open(route, cx);
            return;
        }
        self.reset_to_default(cx);
        if self.default_page.as_ref().is_some_and(|page| page.route == route) {
            cx.emit(AsideReveal { toggles: false });
            return;
        }
        self.open(route, cx);
    }

    pub fn replace(&mut self, route: RootRoute, cx: &mut Context<Self>) {
        if self.pages.is_empty() {
            self.open(route, cx);
            return;
        }
        self.undock_comments(cx);
        self.store.update(cx, |store, cx| store.replace_top(route, cx));
        cx.emit(AsideReveal { toggles: false });
    }

    /// docked comments cover the whole panel so a page opened beside them would be hidden
    /// so the sheet gives way
    fn undock_comments(&mut self, cx: &mut Context<Self>) {
        let comments = CommentInteractionsStore::shared(cx);
        if !(comments.read(cx).comments_sheet_visible && self.docks_comments(cx)) {
            return;
        }
        comments.update(cx, |store, cx| store.close_comments_sheet(cx));
        self.comments_docked = false;
        cx.notify();
    }

    /// true when the comments sheet covers the panel now
    /// read live from the stores because the docked flag is only written during this render
    pub fn shows_docked_comments(&self, cx: &App) -> bool {
        rise_ui::shared::try_read::<CommentInteractionsStore>(cx).is_some_and(|store| store.comments_sheet_visible)
            && self.docks_comments(cx)
    }

    /// true when the panel back arrow should show, a page on the stack or docked comments over a page
    pub fn can_step_back(&self, cx: &App) -> bool {
        self.can_go_back()
            || (self.shows_docked_comments(cx) && (self.top_route().is_some() || Self::docked_thread_open(cx)))
    }

    fn docked_thread_open(cx: &App) -> bool {
        rise_ui::shared::try_read::<CommentInteractionsStore>(cx).is_some_and(|store| store.thread_root().is_some())
    }

    fn refresh_dock_row(&mut self, cx: &mut Context<Self>) {
        let steps_back = self.can_step_back(cx);
        if steps_back == self.dock_row_steps_back {
            return;
        }
        self.dock_row_steps_back = steps_back;
        self.comments_dock.update(cx, |_, cx| cx.notify());
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        if self.comments_docked && Self::docked_thread_open(cx) {
            CommentInteractionsStore::shared(cx).update(cx, |store, cx| store.close_thread(cx));
            return;
        }
        if self.comments_docked && self.top_route().is_some() {
            tracing::info!(target: "riseonly::X1634", pages = self.pages.len(), "panel back closes docked comments");
            self.undock_comments(cx);
            return;
        }
        if self.pages.is_empty() {
            return;
        }
        self.store.update(cx, |store, cx| store.pop(cx));
    }

    pub fn reset_to_default(&mut self, cx: &mut Context<Self>) {
        if self.pages.is_empty() {
            return;
        }
        self.store.update(cx, |store, cx| store.pop_to_root(cx));
        self.sync_pages(cx);
    }

    pub fn set_default(&mut self, section: AsideSection, route: RootRoute, cx: &mut Context<Self>) {
        if section == AsideSection::Chats {
            let open_chat = match &route {
                RootRoute::ChatProfile { chat_id: Some(chat_id) } => Some(chat_id.clone()),
                _ => None,
            };
            self.drop_forum_info_of_other_chats(section, open_chat.as_deref(), cx);
        }
        if self.defaults.get(&section) == Some(&route) {
            return;
        }
        let opens_other_chat = (section == AsideSection::Chats
            && (matches!(route, RootRoute::ChatProfile { .. })
                || matches!(self.defaults.get(&section), Some(RootRoute::ChatProfile { .. }))))
            || (section == AsideSection::Profile && self.opens_other_profile(&route));
        tracing::warn!(target: "riseonly::X980", ?section, ?route, previous = ?self.defaults.get(&section), current = ?self.section, pages = self.pages.len(), opens_other_chat, "panel default");
        self.defaults.insert(section, route);
        if opens_other_chat {
            self.parked.remove(&Some(section));
            self.recalled_stacks.remove(&section);
        }
        if self.section == Some(section) {
            if opens_other_chat {
                self.reset_to_default(cx);
                self.sync_pages(cx);
            }
            self.sync_default_page(cx);
        }
    }

    fn opens_other_profile(&mut self, route: &RootRoute) -> bool {
        let owner = match route {
            RootRoute::MyProfile => Some(String::new()),
            RootRoute::ProfileMore { user_id, tag } => user_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .or(tag.as_deref().filter(|tag| !tag.is_empty()))
                .map(str::to_owned),
            _ => None,
        };
        let Some(owner) = owner else {
            return false;
        };
        let other = self.profile_owner.as_ref().is_some_and(|previous| *previous != owner);
        self.profile_owner = Some(owner);
        other
    }

    fn drop_forum_info_of_other_chats(&mut self, section: AsideSection, open_chat: Option<&str>, cx: &mut Context<Self>) {
        let foreign = |route: &RootRoute| match route {
            RootRoute::TopicProfile { chat_id, .. } | RootRoute::GeneralProfile { chat_id } => {
                chat_id.as_deref().is_some_and(|chat_id| !chat_id.is_empty() && Some(chat_id) != open_chat)
            }
            _ => false,
        };
        self.truncate_panel_where(section, &foreign, "forum info of another chat", cx);
    }

    pub fn follow_topic(&mut self, chat_id: &str, open: Option<&RootRoute>, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }
        let stale = |route: &RootRoute| match route {
            RootRoute::TopicProfile { chat_id: id, .. } | RootRoute::GeneralProfile { chat_id: id } => {
                id.as_deref() == Some(chat_id) && Some(route) != open
            }
            _ => false,
        };
        self.truncate_panel_where(AsideSection::Chats, &stale, "info of another topic", cx);
    }

    fn truncate_panel_where(
        &mut self,
        section: AsideSection,
        foreign: &dyn Fn(&RootRoute) -> bool,
        reason: &'static str,
        cx: &mut Context<Self>,
    ) {
        if let Some(pages) = self.parked.get_mut(&Some(section))
            && let Some(index) = pages.iter().position(|page| foreign(&page.route))
        {
            pages.truncate(index);
        }
        if let Some(routes) = self.recalled_stacks.get_mut(&section)
            && let Some(index) = routes.iter().position(|route| foreign(route))
        {
            routes.truncate(index);
        }
        if self.section != Some(section) {
            return;
        }
        let Some(index) = self.pages.iter().position(|page| foreign(&page.route)) else {
            return;
        };
        tracing::warn!(target: "riseonly::X980", reason, index, pages = self.pages.len(), "panel drops info");
        let below = index.checked_sub(1).and_then(|below| self.pages.get(below)).map(|page| page.route.clone());
        self.store.update(cx, |store, cx| match below {
            Some(route) => {
                store.pop_to(route, cx);
            }
            None => store.pop_to_root(cx),
        });
        self.sync_pages(cx);
    }

    pub fn drop_centre_twin(&mut self, cx: &mut Context<Self>) {
        let Some(section) = self.section else {
            return;
        };
        if self.pages.is_empty() {
            return;
        }
        let Some(centre) = self.centre.upgrade() else {
            return;
        };
        let Some(top) = centre.read(cx).path.last() else {
            return;
        };
        if !self.pages.iter().any(|page| page.route == *top) {
            return;
        }
        let top = top.clone();
        self.truncate_panel_where(section, &|route| *route == top, "centre twin", cx);
    }

    /// drops every page of a deleted chat from the panel stack and from stacks parked by other sections
    pub fn close_chat(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }
        let in_chat = |route: &RootRoute| NavigationStore::chat_scope_id(route) == Some(chat_id);
        for pages in self.parked.values_mut() {
            if let Some(index) = pages.iter().position(|page| in_chat(&page.route)) {
                pages.truncate(index);
            }
        }
        for routes in self.recalled_stacks.values_mut() {
            if let Some(index) = routes.iter().position(|route| in_chat(route)) {
                routes.truncate(index);
            }
        }
        self.store.update(cx, |store, cx| {
            store.close_chat(chat_id, cx);
        });
    }

    pub fn clear_default(&mut self, section: AsideSection, cx: &mut Context<Self>) {
        if self.defaults.remove(&section).is_some() && self.section == Some(section) {
            self.sync_default_page(cx);
        }
    }

    pub fn open_top_in_centre(&mut self, cx: &mut Context<Self>) {
        let Some((route, view)) = self.pages.last().map(|page| (page.route.clone(), page.view.clone())) else {
            return;
        };
        let Some(centre) = self.centre.upgrade() else {
            return;
        };
        if !route.shows_music_mini_player() {
            AsideHandedPage::shared(cx).update(cx, |handed, _| handed.page = Some((route.clone(), view)));
        }
        self.reset_to_default(cx);
        centre.update(cx, |centre, cx| {
            let takes = centre.aside_takes_details();
            centre.set_aside_takes_details(false);
            centre.push(route, NavigationStoreSettlingPush::Drop, cx);
            centre.set_aside_takes_details(takes);
        });
    }

    fn account(cx: &mut Context<Self>) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()
    }

    fn recall_stack(&mut self, cx: &mut Context<Self>) {
        let Some(account) = Self::account(cx) else {
            return;
        };
        if self.recalled {
            if self.account.as_deref() == Some(account.as_str()) {
                return;
            }
            self.drop_account_pages(cx);
        }
        self.recalled = true;
        self.account = Some(account.clone());
        self.recalled_stacks = AsideMemory::recall(Some(&account))
            .into_iter()
            .map(|(section, routes)| (section, routes.into_iter().filter(|route| !Self::is_conversation(route)).collect::<Vec<_>>()))
            .collect();
        self.recalled_stacks.remove(&AsideSection::Riffs);
        AsideMemory::forget_pinned();
        if self.pages.is_empty() {
            self.bring_section_stack(cx);
        }
        self.sync_default_page(cx);
    }

    fn drop_account_pages(&mut self, cx: &mut Context<Self>) {
        tracing::info!(target: "riseonly::X1650", pages = self.pages.len(), parked = self.parked.len(), "account changed, panel pages dropped");
        self.pages.clear();
        self.parked.clear();
        self.recalled_stacks.clear();
        self.profile_owner = None;
        self.default_page = None;
        if self.comments_docked {
            self.comments_docked = false;
            CommentInteractionsStore::shared(cx).update(cx, |store, cx| store.close_comments_sheet(cx));
        }
        if !self.store.read(cx).path.is_empty() {
            self.store.update(cx, |store, cx| {
                let tab = store.current_tab;
                store.restore(Vec::new(), tab, cx);
            });
        }
        self.sync_default_page(cx);
        cx.notify();
    }

    /// puts the current section stack on the panel store
    /// parked pages first then the last launch then nothing
    fn bring_section_stack(&mut self, cx: &mut Context<Self>) {
        let pages = self.parked.remove(&self.section).unwrap_or_default();
        let routes: Vec<RootRoute> = if pages.is_empty() {
            self.section.and_then(|section| self.recalled_stacks.remove(&section)).unwrap_or_default()
        } else {
            pages.iter().map(|page| page.route.clone()).collect()
        };
        self.pages = pages;
        if routes.is_empty() && self.store.read(cx).path.is_empty() {
            return;
        }
        self.store.update(cx, |store, cx| {
            let tab = store.current_tab;
            store.restore(routes, tab, cx);
        });
        self.sync_pages(cx);
    }

    fn remember_stacks(&self) {
        let account = self.account.as_deref();
        let routes_of = |pages: &[AsidePage]| pages.iter().map(|page| page.route.clone()).collect::<Vec<_>>();
        let mut stacks: Vec<(AsideSection, Vec<RootRoute>)> = self
            .parked
            .iter()
            .filter_map(|(section, pages)| section.map(|section| (section, routes_of(pages))))
            .collect();
        stacks.extend(self.recalled_stacks.iter().map(|(section, routes)| (*section, routes.clone())));
        if let Some(section) = self.section {
            stacks.push((section, routes_of(&self.pages)));
        }
        AsideMemory::remember(account, &stacks);
    }

    fn sync_default_page(&mut self, cx: &mut Context<Self>) {
        let route = self.section.and_then(|section| self.default_route(section).cloned());
        if self.default_page.as_ref().map(|page| &page.route) == route.as_ref() {
            return;
        }
        self.default_page = route.map(|route| Self::page(route, cx));
        crate::core::guides::guide_screen_resolver::GuideScreenObserver::visible_did_change(cx);
        cx.notify();
    }

    fn hands_conversation_to_centre(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(route) = self.store.read(cx).path.last().filter(|route| Self::is_conversation(route)).cloned() else {
            return false;
        };
        if self.section == Some(AsideSection::Search) && self.store.read(cx).path.len() == 1 {
            return false;
        }
        let Some(centre) = self.centre.upgrade() else {
            return false;
        };
        tracing::info!(target: "riseonly::X839", route = ?route, "panel hands conversation to centre");
        self.store.update(cx, |store, cx| store.pop(cx));
        centre.update(cx, |centre, cx| {
            let takes = centre.aside_takes_details();
            centre.set_aside_takes_details(false);
            centre.open_from_chat_list(route, cx);
            centre.set_aside_takes_details(takes);
        });
        true
    }

    fn is_conversation(route: &RootRoute) -> bool {
        matches!(
            route,
            RootRoute::Chat { .. }
                | RootRoute::ChatTopic { .. }
                | RootRoute::ForumAll { .. }
                | RootRoute::BotStartLink { .. }
        )
    }

    fn sync_pages(&mut self, cx: &mut Context<Self>) {
        if self.hands_conversation_to_centre(cx) {
            return;
        }
        let path = self.store.read(cx).path.clone();
        let shared = self.pages.iter().zip(path.iter()).take_while(|(page, route)| page.route == **route).count();
        if shared == self.pages.len() && shared == path.len() {
            return;
        }
        self.pages.truncate(shared);
        for route in path.into_iter().skip(shared) {
            let page = Self::page(route, cx);
            self.pages.push(page);
        }
        if self.recalled {
            self.remember_stacks();
        }
        if self.comments_docked && !self.docks_comments(cx) {
            self.comments_docked = false;
            CommentInteractionsStore::shared(cx).update(cx, |store, cx| store.close_comments_sheet(cx));
        }
        crate::core::guides::guide_screen_resolver::GuideScreenObserver::visible_did_change(cx);
        cx.notify();
    }

    fn page(route: RootRoute, cx: &mut Context<Self>) -> AsidePage {
        let destination = route.clone();
        let view: AnyView = cx.new(|cx| RootRouteDestination::in_aside(destination, cx)).into();
        AsidePage { route, view }
    }
}

impl EventEmitter<AsideReveal> for AsideNavigator {}

impl Render for AsideNavigator {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let comments_visible = CommentInteractionsStore::shared(cx).read(cx).comments_sheet_visible;
        let comments_docked = comments_visible && self.docks_comments(cx);
        if comments_docked != self.comments_docked {
            tracing::warn!(target: "riseonly::X1421", docked = comments_docked, panel_shown = self.panel_shown, section = ?self.section, "panel comments dock changed");
        }
        self.comments_docked = comments_docked;
        let top = self
            .pages
            .last()
            .or(self.default_page.as_ref())
            .filter(|page| !self.comments_docked || self.top_drawn == Some(page.view.entity_id()))
            .map(|page| page.view.clone());
        self.top_drawn = top.as_ref().map(AnyView::entity_id);
        let stack_pages: Vec<AnyView> =
            self.default_page.iter().chain(self.pages.iter()).map(|page| page.view.clone()).collect();
        if let Some(drawn) = self.top_drawn {
            self.drawn_pages.insert(drawn);
        }
        self.drawn_pages.retain(|id| stack_pages.iter().any(|view| view.entity_id() == *id));
        let top_drawn = self.top_drawn;
        let chrome = self.page_chrome(cx);
        let drawn_pages = &self.drawn_pages;
        self.chrome_taken.retain(|id, _| drawn_pages.contains(id));
        let slots: Vec<AnyElement> = stack_pages
            .into_iter()
            .filter(|view| self.drawn_pages.contains(&view.entity_id()))
            .map(|view| {
                let is_top = top_drawn == Some(view.entity_id());
                let mut slot = div().absolute().inset_0();
                if is_top {
                    slot = slot.occlude();
                } else {
                    slot.style().visibility = Some(Visibility::Hidden);
                }
                let taken = self.chrome_taken.entry(view.entity_id()).or_default().clone();
                let page_chrome = is_top.then(|| chrome.clone()).flatten().map(|chrome| chrome.with_taken(taken.clone()));
                let row = page_chrome.as_ref().filter(|_| !self.comments_docked).map(PanelChrome::row);
                let page = env_scope::<PanelChromeKey>(page_chrome)
                    .child(view.cached(StyleRefinement::default().size_full()).replay_when_moved());
                slot.child(PanelChromeSlot::new(taken, page, row)).into_any_element()
            })
            .collect();
        let docked_comments = self.comments_docked.then(|| {
            div()
                .id("aside-comments-dock")
                .absolute()
                .inset_0()
                .occlude()
                .child(self.comments_dock.clone().cached(StyleRefinement::default().size_full()))
                .children(chrome.as_ref().map(|chrome| {
                    div().absolute().top_0().left_0().right_0().h(gpui::px(PageHeaderMetrics::PANEL_STRIP_HEIGHT)).child(chrome.row())
                }))
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        env_scope::<PageBesideSplitListKey>(true)
                            .child(
                                env_scope::<NavigationStoreKey>(Some(self.store.clone()))
                                    .child(div().relative().size_full().children(slots)),
                            ),
                    )
                    .children(docked_comments),
            )
    }
}

impl AsideNavigator {
    fn page_chrome(&self, cx: &mut Context<Self>) -> Option<PanelChrome> {
        let close = PanelChrome::take(cx)?.close_action();
        let this = cx.weak_entity();
        let back = self.can_step_back(cx).then(|| {
            let this = this.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |navigator, cx| navigator.back(cx));
            }) as PageHeaderAction
        });
        let open_in_centre = (self.top_wants_centre() && !self.shows_docked_comments(cx)).then(|| {
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |navigator, cx| navigator.open_top_in_centre(cx));
            }) as PageHeaderAction
        });
        Some(PanelChrome::new(Rc::new(Cell::new(false))).back(back).open_in_centre(open_in_centre).close(close))
    }
}

struct FrontmostAside(WeakEntity<AsideNavigator>);

impl Global for FrontmostAside {}

pub fn install_frontmost(navigator: &Entity<AsideNavigator>, cx: &mut App) {
    cx.set_global(FrontmostAside(navigator.downgrade()));
}

pub fn observe_frontmost<V: 'static>(cx: &mut Context<V>) -> Subscription {
    cx.observe_global::<FrontmostAside>(|_, cx| cx.notify())
}

/// true once a window panel answers the free functions, before that a default set is lost
pub fn has_frontmost(cx: &App) -> bool {
    cx.try_global::<FrontmostAside>().is_some_and(|frontmost| frontmost.0.upgrade().is_some())
}

fn with_frontmost(cx: &mut App, action: impl FnOnce(&mut AsideNavigator, &mut Context<AsideNavigator>)) {
    let Some(navigator) = cx.try_global::<FrontmostAside>().and_then(|frontmost| frontmost.0.upgrade()) else {
        return;
    };
    navigator.update(cx, action);
}

pub fn frontmost_screen(cx: &App) -> Option<SaiActiveScreen> {
    let navigator = cx.try_global::<FrontmostAside>()?.0.upgrade()?;
    let navigator = navigator.read(cx);
    if !navigator.panel_shown {
        return None;
    }
    navigator.top_route().map(RootRoute::sai_active_screen)
}

/// true when the front window panel hosts the comments sheet
pub fn docks_comments(cx: &App) -> bool {
    cx.try_global::<FrontmostAside>()
        .and_then(|frontmost| frontmost.0.upgrade())
        .is_some_and(|navigator| navigator.read(cx).docks_comments(cx))
}

/// true when the front window panel row shows its back arrow
pub fn steps_back(cx: &App) -> bool {
    cx.try_global::<FrontmostAside>()
        .and_then(|frontmost| frontmost.0.upgrade())
        .is_some_and(|navigator| navigator.read(cx).can_step_back(cx))
}

/// section change count of the front window, 0 without a panel
pub fn section_visits(cx: &App) -> u64 {
    cx.try_global::<FrontmostAside>()
        .and_then(|frontmost| frontmost.0.upgrade())
        .map_or(0, |navigator| navigator.read(cx).section_visits())
}

/// puts a page on the panel stack
/// also holds the panel page moving to the centre, taken by the next new centre page
struct AsideHandedPage {
    page: Option<(RootRoute, AnyView)>,
}

impl AsideHandedPage {
    fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { page: None })
    }
}

pub fn take_handed_page(route: &RootRoute, cx: &mut App) -> Option<AnyView> {
    let handed = AsideHandedPage::shared(cx).update(cx, |handed, _| handed.page.take())?;
    (handed.0 == *route).then_some(handed.1)
}

pub fn open(route: RootRoute, cx: &mut App) {
    with_frontmost(cx, |navigator, cx| navigator.open(route, cx));
}

pub fn open_over_default(route: RootRoute, cx: &mut App) {
    with_frontmost(cx, |navigator, cx| navigator.open_over_default(route, cx));
}

/// swaps the top page of the panel
pub fn replace(route: RootRoute, cx: &mut App) {
    with_frontmost(cx, |navigator, cx| navigator.replace(route, cx));
}

/// goes back one page in the panel
pub fn back(cx: &mut App) {
    with_frontmost(cx, |navigator, cx| navigator.back(cx));
}

/// resets the panel to the section default
pub fn reset_to_default(cx: &mut App) {
    with_frontmost(cx, |navigator, cx| navigator.reset_to_default(cx));
}

/// sets what the section shows beside its centre
pub fn set_default(section: AsideSection, route: RootRoute, cx: &mut App) {
    with_frontmost(cx, |navigator, cx| navigator.set_default(section, route, cx));
}

/// chat is gone so its pages leave the panel
pub fn close_chat(chat_id: &str, cx: &mut App) {
    with_frontmost(cx, |navigator, cx| navigator.close_chat(chat_id, cx));
}

pub fn follow_topic(chat_id: &str, open: Option<&RootRoute>, cx: &mut App) {
    with_frontmost(cx, |navigator, cx| navigator.follow_topic(chat_id, open, cx));
}

pub fn clear_default(section: AsideSection, cx: &mut App) {
    with_frontmost(cx, |navigator, cx| navigator.clear_default(section, cx));
}
