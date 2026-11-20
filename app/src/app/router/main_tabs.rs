use std::collections::{HashMap, HashSet};

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyView, App, AppContext, Context, Div, ElementId, Entity, FocusHandle, IntoElement, Pixels, Render, RenderOnce,
    SharedString, Stateful, StyleRefinement, Subscription, Visibility, Window, div, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::{env, env_scope};
use rise_ui::swiftui::modifiers::ModifierExt;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};

use crate::app::aside::aside_navigator;
use crate::app::router::main_tab_chrome::{
    MainTabBarUnderlapKey, MainTabBarUnderlapSignal, MainTabChrome, MainTabPageHost, MainTabPageHostKey,
    MainTabPageSelection,
};
use crate::app::router::more_tab_bar_menu::{MoreTabBarMenuOverlay, MoreTabBarMenuState};
use crate::app::router::navigation_store::NavigationStoreKey;
use crate::app::router::navigation_types::MainTab;
use crate::core::lib::rise_feature_gate::RiseFeatureGate;
use crate::core::widgets::navigations::main_tab_bar::MainTabBar;
use crate::core::widgets::navigations::main_tab_bottom_chrome::MainTabBottomChrome;
use crate::core::widgets::navigations::main_tab_scroll_to_top::MainTabScrollToTop;
use crate::modules::chat::components::chat::voice::chat_voice_message_player::ChatVoiceMessagePlayer;
use crate::modules::chat::components::chat_search::chat_search_overlay_presenter::ChatSearchOverlayPresenter;
use crate::modules::chat::pages::chats::chats::{Chats, ChatsSelectionActionBar, ChatsSelectionChrome};
use crate::modules::chat::widgets::bottomsheets::create_chat_sheet::create_chat_sheet::CreateChatSheet;
use crate::modules::chat::widgets::bottomsheets::create_chat_sheet::create_chat_sheet_state::CreateChatSheetState;
use crate::modules::map::pages::world_map::world_map_page::WorldMapPage;
use crate::modules::music::pages::music_tab::music_tab::MusicTab;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::notify::pages::notifications::notifications::Notifications;
use crate::modules::post::engine::rise_post_engine_models::RisePostSearchKind;
use crate::modules::rtc::components::rise_rtc_rail_call_card::RiseRTCRailCallCard;
use crate::modules::post::components::repost::repost_opinion_editor::RepostOpinionEditorPresenter;
use crate::modules::post::pages::posts::posts::Posts;
use crate::modules::search::pages::global_search::global_search::GlobalSearch;
use crate::modules::shorts::pages::shorts_tab::shorts_tab::{ShortsChromeFade, ShortsTab};
use crate::modules::user::engine::rise_user_tab_bar_presentation::RiseUserTabBarPresentation;
use crate::modules::user::pages::profile::profile::Profile;
use crate::modules::vacancy::pages::vacancy_list::vacancy_list::VacancyList;

const HIDE_BAR_DURATION: f64 = 0.22;

/// tab roots under the bar
#[derive(IntoElement)]
pub struct MainTabs {
    current_tab: Binding<MainTab>,
}

impl MainTabs {
    pub fn new(current_tab: Binding<MainTab>) -> Self {
        Self { current_tab }
    }

    /// hides the tab bar
    /// holds_bar false means the bar waits off screen so defaults never flash before the account bar
    fn hide_tab_bar(holds_bar: bool, current_tab: MainTab, cx: &mut App) -> bool {
        holds_bar
            || ChatSearchOverlayPresenter::shared(cx).read(cx).is_open()
            || CreateChatSheetState::shared(cx).read(cx).is_open()
            || Self::chats_selection_shown(current_tab, cx)
    }

    /// chats selection chrome shows only while the chats tab is on screen
    fn chats_selection_shown(current_tab: MainTab, cx: &mut App) -> bool {
        current_tab == MainTab::Chats && ChatsSelectionChrome::shared(cx).read(cx).is_active()
    }

    fn holds_bar(cx: &mut App) -> bool {
        !RiseUserTabBarPresentation::shared(cx).read(cx).is_resolved()
    }

    fn tab_selection(current_tab: Binding<MainTab>) -> Binding<MainTab> {
        let read = current_tab.clone();
        Binding::new(
            move |cx| read.get(cx),
            move |tab, window, cx| {
                current_tab.set(tab, window, cx);
                if tab != MainTab::Profile && MoreTabBarMenuState::shared(cx).read(cx).owns_tab_selection() {
                    let current_tab = current_tab.clone();
                    window.defer(cx, move |window, cx| current_tab.set(MainTab::Profile, window, cx));
                }
            },
        )
    }
}

impl RenderOnce for MainTabs {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        MainTabsScrolling { current_tab: self.current_tab, holds_bar: Self::holds_bar(cx) }
    }
}

/// state of the bar slide, kept per slot
struct MainTabsBarPresence {
    shown: Presence<()>,
}

impl MainTabsBarPresence {
    /// the bar a slot mounts with is on screen at once
    /// only a later change of hide_tab_bar slides it
    fn new(shown: bool, transition: Transition) -> Self {
        Self { shown: Presence::new(shown.then_some(())).with_transition(transition) }
    }

    fn follow(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown.is_present() != shown {
            self.shown.set_in(shown.then_some(()), ease_in_out(HIDE_BAR_DURATION), RiseClock::monotonic(), cx);
        }
    }
}

/// the bar in its column on the left of the window
/// its place stays reserved while hidden so pages dont move
#[derive(IntoElement)]
pub struct MainTabsSideBar {
    current_tab: Binding<MainTab>,
    corner_radius: Pixels,
}

impl MainTabsSideBar {
    pub fn new(current_tab: Binding<MainTab>) -> Self {
        Self { current_tab, corner_radius: px(0.0) }
    }

    pub fn corner_radius(mut self, corner_radius: Pixels) -> Self {
        self.corner_radius = corner_radius;
        self
    }
}

impl RenderOnce for MainTabsSideBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let rail_width = rise_ui::theme(cx).shell.rail_width;
        let settings = RiseUserTabBarPresentation::shared(cx).read(cx).current();
        let hide_side_bar = MainTabs::holds_bar(cx);

        let presence = window.use_keyed_state(
            ElementId::Name(SharedString::new_static("main-tabs-side-bar")),
            cx,
            |_, _| MainTabsBarPresence::new(!hide_side_bar, Transition::move_edge(Edge::Leading).combined(Transition::opacity())),
        );
        presence.update(cx, |presence, cx| presence.follow(!hide_side_bar, cx));

        let call_card = window.use_keyed_state(
            ElementId::Name(SharedString::new_static("main-tabs-side-bar-call-card")),
            cx,
            |_, cx| RiseRTCRailCallCard::new(cx),
        );

        let current_tab = MainTabs::tab_selection(self.current_tab);
        let corner_radius = self.corner_radius;
        let bar = presence.read(cx).shown.render_transitioned(RiseClock::monotonic(), window, |_| {
            div().absolute().inset_0().child(
                MainTabBar::new(current_tab, settings)
                    .corner_radius(corner_radius)
                    .foot_accessory(Some(AnyView::from(call_card.clone()))),
            )
        });

        div().relative().flex_none().h_full().w(rail_width).children(bar)
    }
}

struct MainTabsPages {
    visited: HashSet<MainTab>,
    posts: Option<Entity<Posts>>,
    global_search: Option<Entity<GlobalSearch>>,
    vacancies: Option<Entity<VacancyList>>,
    maps: Option<Entity<WorldMapPage>>,
    components: HashMap<MainTab, Entity<MainTabComponentPage>>,
    last_tab: Option<MainTab>,
    focus: HashMap<MainTab, FocusHandle>,
    selection: Entity<MainTabPageSelection>,
}

impl MainTabsPages {
    fn new(cx: &mut App) -> Self {
        Self {
            visited: HashSet::new(),
            posts: None,
            global_search: None,
            vacancies: None,
            maps: None,
            components: HashMap::new(),
            last_tab: None,
            focus: MainTab::ALL_CASES.into_iter().map(|tab| (tab, cx.focus_handle())).collect(),
            selection: cx.new(|_| MainTabPageSelection::new()),
        }
    }

    fn page_focus(&self, tab: MainTab) -> Option<FocusHandle> {
        self.focus.get(&tab).cloned()
    }

    fn component(&self, tab: MainTab) -> Option<AnyView> {
        self.components.get(&tab).map(|page| AnyView::from(page.clone()))
    }

    /// the tab being opened counts as visited already
    /// written later it would mount a frame late over an empty page
    fn visit(&mut self, tab: MainTab, window: &mut Window, cx: &mut Context<Self>) {
        self.visited.insert(tab);
        match tab {
            MainTab::Posts if self.posts.is_none() => self.posts = Some(cx.new(Posts::new)),
            MainTab::GlobalSearch if self.global_search.is_none() => {
                self.global_search = Some(cx.new(|cx| GlobalSearch::new("", RisePostSearchKind::Publications, cx)));
            }
            MainTab::Vacancies if self.vacancies.is_none() => {
                self.vacancies = Some(cx.new(|cx| VacancyList::new(false, cx)));
            }
            _ => {}
        }
        if tab == MainTab::Music && self.components.contains_key(&tab) {
            cx.defer(|cx| {
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.bootstrap_music_tab(cx));
            });
        }
        if let Some(component) = MainTabComponent::of(tab)
            && !self.components.contains_key(&tab)
        {
            self.components.insert(tab, cx.new(|_| MainTabComponentPage { component }));
        }
        if tab != MainTab::Maps {
            self.maps = None;
        } else if self.maps.is_none() {
            self.maps = Some(cx.new(|cx| WorldMapPage::new(None, false, window, cx)));
        }
    }
}

struct MainTabsMiniPlayerPresence {
    is_present: bool,
    _voice: Subscription,
}

impl MainTabsMiniPlayerPresence {
    fn new(cx: &mut Context<Self>) -> Self {
        let voice = ChatVoiceMessagePlayer::shared(cx);
        Self {
            is_present: Self::reads_present(cx),
            _voice: cx.observe(&voice, |presence: &mut Self, _, cx| presence.refresh(cx)),
        }
    }

    fn reads_present(cx: &mut App) -> bool {
        ChatVoiceMessagePlayer::shared(cx).read(cx).is_active_in_chat()
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let is_present = Self::reads_present(cx);
        if is_present != self.is_present {
            self.is_present = is_present;
            cx.notify();
        }
    }
}

/// what the root hands the player layer, a change redraws the layer in the same frame
#[derive(Clone, Copy, PartialEq, Eq)]
struct MainTabsBottomChromeProps {
    shows_bar: bool,
    hides_mini_player: bool,
    is_shorts: bool,
}

struct MainTabsBottomChromeLayer {
    props: MainTabsBottomChromeProps,
}

impl MainTabsBottomChromeLayer {
    fn element(props: MainTabsBottomChromeProps, window: &mut Window, cx: &mut App) -> AnyElement {
        let view = window.with_global_id(ElementId::Name(SharedString::new_static("main-tabs-bottom-chrome-layer")), |global_id, window| {
            window.with_element_state(global_id, |existing: Option<Entity<Self>>, _| {
                let view = existing.unwrap_or_else(|| cx.new(|_| Self { props }));
                (view.clone(), view)
            })
        });
        let stale = view.update(cx, |layer, cx| {
            if layer.props == props {
                return false;
            }
            layer.props = props;
            cx.notify();
            true
        });
        let view = AnyView::from(view);
        let layer = if stale {
            view.into_any_element()
        } else {
            view.cached(StyleRefinement::default().size_full()).into_any_element()
        };
        div().absolute().top_0().left_0().right_0().bottom_0().child(layer).into_any_element()
    }
}

impl Render for MainTabsBottomChromeLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let props = self.props;
        let bottom_chrome = window.use_keyed_state(
            ElementId::Name(SharedString::new_static("main-tabs-bottom-chrome")),
            cx,
            |_, _| MainTabsBarPresence::new(props.shows_bar, Transition::move_edge(Edge::Bottom).combined(Transition::opacity())),
        );
        bottom_chrome.update(cx, |presence, cx| presence.follow(props.shows_bar, cx));
        let chrome = bottom_chrome.read(cx).shown.render_transitioned(RiseClock::monotonic(), window, |_| {
            div().w_full().child(
                MainTabBottomChrome::new()
                    .hides_mini_player(props.hides_mini_player)
                    .modifier(ShortsChromeFade::new(props.is_shorts)),
            )
        });
        div().relative().size_full().child(div().absolute().left_0().right_0().bottom_0().children(chrome))
    }
}

/// main tabs scrolling flag, minus the scrolling strip because the strip is the left bar
#[derive(IntoElement)]
struct MainTabsScrolling {
    current_tab: Binding<MainTab>,
    holds_bar: bool,
}

impl MainTabsScrolling {
    /// bottom underlap with the bar out of the column
    /// only the player slot stands over the pages
    fn underlap(mini_player_visible: bool) -> f32 {
        MainTabChrome::extra_underlap_when_mini_player_visible(mini_player_visible)
    }

    fn mini_visible(hide_tab_bar: bool, window: &mut Window, cx: &mut App) -> bool {
        let presence = window.use_keyed_state(
            ElementId::Name(SharedString::new_static("main-tabs-mini-player-presence")),
            cx,
            |_, cx| MainTabsMiniPlayerPresence::new(cx),
        );
        !hide_tab_bar && presence.read(cx).is_present
    }
}

impl RenderOnce for MainTabsScrolling {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let current_tab = self.current_tab.get(cx);
        let hide_tab_bar = MainTabs::hide_tab_bar(self.holds_bar, current_tab, cx);

        let underlap = Self::underlap(Self::mini_visible(hide_tab_bar, window, cx));
        let riffs_underlap = Self::underlap(false);
        MainTabBarUnderlapSignal::publish(underlap, window, cx);

        let pages = window.use_keyed_state(
            ElementId::Name(SharedString::new_static("main-tabs-pages")),
            cx,
            |_, cx| MainTabsPages::new(cx),
        );
        let last_tab = pages.read(cx).last_tab;
        if last_tab != Some(current_tab) {
            if let Some(leaving) = last_tab.and_then(|tab| pages.read(cx).page_focus(tab)) {
                release_leaving_tab_focus(&leaving, window, cx);
            }
            MainTabScrollToTop::forget_frames(window);
            pages.update(cx, |pages, cx| {
                pages.last_tab = Some(current_tab);
                pages.visit(current_tab, window, cx);
                pages.selection.update(cx, |selection, cx| selection.set_shown(current_tab.paged_tab(), cx));
            });
        }

        let selection = pages.read(cx).selection.clone();
        let page = |tab: MainTab, underlap: f32, focus: Option<FocusHandle>, content: AnyElement| {
            let host = MainTabPageHost { tab, selection: selection.clone() };
            (tab == current_tab.paged_tab(), main_tab_page(tab, current_tab, underlap, host, focus, content).into_any_element())
        };
        let list_width = rise_ui::theme(cx).shell.list_max_width;
        let mut drawn = Vec::with_capacity(5);
        let state = pages.read(cx);
        if let Some(posts) = state.posts.clone() {
            let content = tab_view(posts.into());
            drawn.push(page(MainTab::Posts, underlap, state.page_focus(MainTab::Posts), content));
        }
        if let Some(global_search) = state.global_search.clone() {
            let content = tab_view(global_search.into());
            drawn.push(page(MainTab::GlobalSearch, underlap, state.page_focus(MainTab::GlobalSearch), content));
        }
        if let Some(chats) = state.component(MainTab::Chats) {
            let content = list_column(list_width, tab_view(chats));
            drawn.push(page(MainTab::Chats, underlap, state.page_focus(MainTab::Chats), content));
        }
        if let Some(shorts) = state.component(MainTab::Shorts) {
            let content = tab_view(shorts);
            drawn.push(page(MainTab::Shorts, riffs_underlap, state.page_focus(MainTab::Shorts), content));
        }
        let shared = MainTabSharedPage {
            current_tab,
            visited: state.visited.clone(),
            more: state.component(MainTab::Profile),
            music: state.component(MainTab::Music),
            vacancies: state.vacancies.clone(),
            maps: state.maps.clone(),
            focus: state.focus.clone(),
        };
        if let Some(shared) = shared.when_needed() {
            drawn.push(page(MainTab::Profile, underlap, None, shared.into_any_element()));
        }
        let stack = div().relative().size_full().children(open_page_last(drawn));

        let player_beside = env::<NavigationStoreKey>(cx).is_some_and(|nav_store| {
            let nav_store = nav_store.read(cx);
            nav_store.chat_list_beside_stack() && !nav_store.path.is_empty()
        });
        let repost_editor_open = RepostOpinionEditorPresenter::shared(cx).read(cx).is_visible;
        let hides_mini_player = current_tab == MainTab::Shorts
            || player_beside
            || repost_editor_open
            || aside_navigator::player_beside(cx);
        let chrome = MainTabsBottomChromeLayer::element(
            MainTabsBottomChromeProps {
                shows_bar: !hide_tab_bar,
                hides_mini_player,
                is_shorts: current_tab == MainTab::Shorts,
            },
            window,
            cx,
        );

        let selection_active = MainTabs::chats_selection_shown(current_tab, cx);
        let selection_bar = window.use_keyed_state(
            ElementId::Name(SharedString::new_static("main-tabs-selection-bar")),
            cx,
            |_, _| MainTabsBarPresence::new(selection_active, Transition::move_edge(Edge::Bottom).combined(Transition::opacity())),
        );
        selection_bar.update(cx, |presence, cx| presence.follow(selection_active, cx));
        let selection = selection_bar.read(cx).shown.render_transitioned(RiseClock::monotonic(), window, |_| {
            div()
                .w_full()
                .px(px(MainTabChrome::FLOATING_BAR_HORIZONTAL_INSET))
                .pb(px(MainTabChrome::bar_bottom_gap(0.0)))
                .child(ChatsSelectionActionBar::new())
        });

        stack
            .child(chrome)
            .child(div().absolute().left_0().right_0().bottom_0().children(selection))
            .child(CreateChatSheet::new())
            .when(!self.holds_bar, |stack| stack.child(MoreTabBarMenuOverlay::new()))
    }
}

trait PagedTab {
    fn paged_tab(self) -> MainTab;
}

impl PagedTab for MainTab {
    fn paged_tab(self) -> MainTab {
        match self {
            MainTab::Posts | MainTab::GlobalSearch | MainTab::Chats | MainTab::Shorts => self,
            MainTab::Profile | MainTab::Music | MainTab::Vacancies | MainTab::Maps | MainTab::Notifications => {
                MainTab::Profile
            }
        }
    }
}

#[derive(IntoElement)]
struct MainTabSharedPage {
    current_tab: MainTab,
    visited: HashSet<MainTab>,
    more: Option<AnyView>,
    music: Option<AnyView>,
    vacancies: Option<Entity<VacancyList>>,
    maps: Option<Entity<WorldMapPage>>,
    focus: HashMap<MainTab, FocusHandle>,
}

impl MainTabSharedPage {
    fn when_needed(self) -> Option<Self> {
        let needed = self.current_tab.paged_tab() == MainTab::Profile
            || self.visited.contains(&MainTab::Profile)
            || self.visited.contains(&MainTab::Music)
            || self.visited.contains(&MainTab::Vacancies);
        needed.then_some(self)
    }

    fn kept(tab: MainTab, shown: bool, focus: Option<&FocusHandle>, content: AnyElement) -> impl IntoElement {
        let kept = div().id(ElementId::Name(SharedString::new_static(tab.raw_value()))).absolute().inset_0();
        offstage(kept, shown).when_some(focus, |kept, focus| kept.track_focus(focus)).child(offstage_content().child(content))
    }
}

impl RenderOnce for MainTabSharedPage {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let bg = rise_ui::theme(cx).bg._100;
        let list_width = rise_ui::theme(cx).shell.list_max_width;
        let profile_width = rise_ui::theme(cx).shell.profile_max_width;
        let current_tab = self.current_tab;
        let focus = self.focus;
        let mut kept = Vec::with_capacity(3);
        if let Some(more) = self.more {
            let shown = current_tab == MainTab::Profile;
            let content = list_column(profile_width, tab_view(more));
            kept.push((shown, Self::kept(MainTab::Profile, shown, focus.get(&MainTab::Profile), content).into_any_element()));
        }
        if let Some(music) = self.music {
            let shown = current_tab == MainTab::Music;
            kept.push((shown, Self::kept(MainTab::Music, shown, focus.get(&MainTab::Music), tab_view(music)).into_any_element()));
        }
        if RiseFeatureGate::VACANCIES
            && let Some(vacancies) = self.vacancies
        {
            let shown = current_tab == MainTab::Vacancies;
            let content = tab_view(vacancies.into());
            kept.push((shown, Self::kept(MainTab::Vacancies, shown, focus.get(&MainTab::Vacancies), content).into_any_element()));
        }
        let mut page = div().relative().size_full().bg(bg).children(open_page_last(kept));
        if current_tab == MainTab::Maps
            && let Some(maps) = self.maps
        {
            let focus = focus.get(&MainTab::Maps);
            let maps_page = div().absolute().inset_0().occlude();
            page = page.child(maps_page.when_some(focus, |maps, focus| maps.track_focus(focus)).child(tab_view(maps.into())));
        }
        if current_tab == MainTab::Notifications {
            let focus = focus.get(&MainTab::Notifications);
            let notifications = div().absolute().inset_0().occlude().when_some(focus, |page, focus| page.track_focus(focus));
            page = page.child(notifications.child(list_column(list_width, Notifications::new().into_any_element())));
        }
        page
    }
}

/// a field in a page that stopped drawing keeps window focus but has no node in the dispatch tree
/// so keys start from the root and every window shortcut dies until next click
/// this checks for it and blurs after the draw
fn release_leaving_tab_focus(leaving: &FocusHandle, window: &mut Window, cx: &mut App) {
    if !leaving.contains_focused(window, cx) {
        return;
    }
    let Some(stuck) = window.focused(cx) else {
        return;
    };
    window.defer(cx, move |window, cx| {
        if window.focused(cx).as_ref() == Some(&stuck) {
            window.blur();
        }
    });
}

/// list tabs in a wide centre keep rows to the theme list max width in the middle
/// so the dot and counter sit beside the text
fn list_column(width: Pixels, content: AnyElement) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_row()
        .justify_center()
        .wheel_into_children()
        .child(div().relative().h_full().w_full().max_w(width).child(content))
        .into_any_element()
}

fn main_tab_page(
    tab: MainTab,
    current_tab: MainTab,
    underlap: f32,
    host: MainTabPageHost,
    focus: Option<FocusHandle>,
    content: AnyElement,
) -> impl IntoElement {
    let shown = tab == current_tab.paged_tab();
    let page = div().id(ElementId::Name(SharedString::new_static(tab.sai_screen_id()))).absolute().inset_0();
    offstage(page, shown)
        .when_some(focus, |page, focus| page.track_focus(&focus))
        .child(offstage_content().child(
            env_scope::<MainTabPageHostKey>(Some(host)).child(env_scope::<MainTabBarUnderlapKey>(underlap).child(content)),
        ))
}

/// pages of a stack in drawing order, the open page goes last above the hidden ones
fn open_page_last(pages: Vec<(bool, AnyElement)>) -> impl Iterator<Item = AnyElement> {
    let (open, hidden): (Vec<_>, Vec<_>) = pages.into_iter().partition(|(shown, _)| *shown);
    hidden.into_iter().chain(open).map(|(_, page)| page)
}

/// box of a tab page, a hidden one keeps layout and prepaint but paints nothing
/// the open page occludes the mouse over it
fn offstage(page: Stateful<Div>, shown: bool) -> Stateful<Div> {
    if shown {
        return page.occlude();
    }
    let mut page = page;
    page.style().visibility = Some(Visibility::Hidden);
    page
}

/// content of a tab page, same box in both states so keyed state survives the switch
fn offstage_content() -> Div {
    div().size_full().flex().flex_col()
}

/// tab root as a cached view, open or hidden
/// window frames replay every page and only the page whose own reads changed renders again
fn tab_view(view: AnyView) -> AnyElement {
    view.cached(StyleRefinement::default().size_full()).into_any_element()
}

/// tabs whose root is a component and not a page view
#[derive(Clone, Copy)]
enum MainTabComponent {
    Chats,
    Shorts,
    More,
    Music,
}

impl MainTabComponent {
    fn of(tab: MainTab) -> Option<Self> {
        match tab {
            MainTab::Chats => Some(Self::Chats),
            MainTab::Shorts => Some(Self::Shorts),
            MainTab::Profile => Some(Self::More),
            MainTab::Music => Some(Self::Music),
            MainTab::Posts | MainTab::GlobalSearch | MainTab::Vacancies | MainTab::Maps | MainTab::Notifications => None,
        }
    }
}

/// component tab root held as a view, the boundary tab_view caches while hidden
struct MainTabComponentPage {
    component: MainTabComponent,
}

impl Render for MainTabComponentPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match self.component {
            MainTabComponent::Chats => Chats::new().into_any_element(),
            MainTabComponent::Shorts => ShortsTab::new().into_any_element(),
            MainTabComponent::More => Profile::new("main-tab-profile").shows_back_button(false).into_any_element(),
            MainTabComponent::Music => MusicTab::new("main-tab-music").into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{KeyBinding, Render, TestAppContext, VisualTestContext};

    use super::*;

    gpui::actions!(main_tabs_focus_test, [OpenFirstTab, OpenSecondTab]);

    /// the shape that loses keys, a focused root with shell context over pages hidden by display none
    /// the second one holds a field
    struct TabFocusProbe {
        root: FocusHandle,
        pages: [FocusHandle; 2],
        first_page_field: FocusHandle,
        second_page_field: FocusHandle,
        open: usize,
        last: usize,
        first_page_takes_focus: bool,
    }

    impl TabFocusProbe {
        fn new(cx: &App) -> Self {
            Self {
                root: cx.focus_handle(),
                pages: [cx.focus_handle(), cx.focus_handle()],
                first_page_field: cx.focus_handle(),
                second_page_field: cx.focus_handle(),
                open: 0,
                last: 0,
                first_page_takes_focus: false,
            }
        }
    }

    impl Render for TabFocusProbe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            if window.focused(cx).is_none() {
                window.focus(&self.root, cx);
            }
            if self.last != self.open {
                release_leaving_tab_focus(&self.pages[self.last], window, cx);
                self.last = self.open;
                if self.open == 0 && self.first_page_takes_focus {
                    window.focus(&self.first_page_field, cx);
                }
            }
            let tab_page = |index: usize, id: &'static str, field: &FocusHandle| {
                div()
                    .track_focus(&self.pages[index])
                    .when(self.open != index, |page| page.hidden())
                    .child(div().id(id).track_focus(field))
            };
            div()
                .key_context("Shell")
                .track_focus(&self.root)
                .on_action(cx.listener(|this, _: &OpenFirstTab, _, cx| {
                    this.open = 0;
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &OpenSecondTab, _, cx| {
                    this.open = 1;
                    cx.notify();
                }))
                .child(tab_page(0, "first-page-field", &self.first_page_field))
                .child(tab_page(1, "second-page-field", &self.second_page_field))
        }
    }

    fn open_probe(cx: &mut TestAppContext) -> (Entity<TabFocusProbe>, &mut VisualTestContext) {
        cx.update(|cx| {
            cx.bind_keys([
                KeyBinding::new("secondary-1", OpenFirstTab, Some("Shell")),
                KeyBinding::new("secondary-2", OpenSecondTab, Some("Shell")),
            ]);
        });
        cx.add_window_view(|_, cx| TabFocusProbe::new(cx))
    }

    fn open_tab(view: &Entity<TabFocusProbe>, cx: &mut VisualTestContext) -> usize {
        view.read_with(cx, |probe, _| probe.open)
    }

    #[gpui::test]
    fn window_keys_survive_leaving_a_tab_with_a_focused_field(cx: &mut TestAppContext) {
        let (view, cx) = open_probe(cx);
        cx.simulate_keystrokes("secondary-2");
        assert_eq!(open_tab(&view, cx), 1);

        let field = view.read_with(cx, |probe, _| probe.second_page_field.clone());
        cx.update(|window, cx| window.focus(&field, cx));
        cx.run_until_parked();

        cx.simulate_keystrokes("secondary-1");
        cx.run_until_parked();
        assert_eq!(open_tab(&view, cx), 0);
        let root = view.read_with(cx, |probe, _| probe.root.clone());
        assert!(cx.update(|window, _| root.is_focused(window)), "the root takes the focus back");

        cx.simulate_keystrokes("secondary-2");
        assert_eq!(open_tab(&view, cx), 1, "the shell context is on the key path again");
        assert!(!cx.update(|window, _| field.is_focused(window)), "coming back does not return the focus to the field");
    }

    #[gpui::test]
    fn a_field_of_the_new_tab_keeps_the_focus_it_takes_on_appear(cx: &mut TestAppContext) {
        let (view, cx) = open_probe(cx);
        cx.simulate_keystrokes("secondary-2");
        let field = view.read_with(cx, |probe, _| probe.second_page_field.clone());
        cx.update(|window, cx| window.focus(&field, cx));
        cx.run_until_parked();
        view.update(cx, |probe, _| probe.first_page_takes_focus = true);

        cx.simulate_keystrokes("secondary-1");
        cx.run_until_parked();
        let first_page_field = view.read_with(cx, |probe, _| probe.first_page_field.clone());
        assert!(cx.update(|window, _| first_page_field.is_focused(window)));
    }

    #[test]
    fn the_tabs_past_the_fourth_share_the_fifth_page() {
        for tab in [MainTab::Posts, MainTab::GlobalSearch, MainTab::Chats, MainTab::Shorts] {
            assert_eq!(tab.paged_tab(), tab);
        }
        for tab in [MainTab::Profile, MainTab::Music, MainTab::Vacancies, MainTab::Maps, MainTab::Notifications] {
            assert_eq!(tab.paged_tab(), MainTab::Profile);
        }
    }

    #[test]
    fn the_underlap_is_only_the_player_slot() {
        assert_eq!(MainTabsScrolling::underlap(false), 0.0);
        assert_eq!(
            MainTabsScrolling::underlap(true),
            MainTabChrome::MINI_PLAYER_BLOCK_HEIGHT + MainTabChrome::MINI_PLAYER_GAP_ABOVE_TAB_BAR
        );
    }
}
