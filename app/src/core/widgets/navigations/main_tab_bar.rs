use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, App, BoxShadow, Bounds, ClickEvent, Context, Div, ElementId, Entity, FontWeight, Hsla,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, RenderOnce, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, Task, Window, deferred, div, hsla, point, prelude::FluentBuilder,
    px,
};
use rise_canon::{RiseGuideAnchor, RiseTabBarButton};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::HostOs;
use rise_theme::theme::rise_theme_preferences::RiseThemeAppearance;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::{IgnoresTextSizeDeltaKey, MainText, TextAlignT};
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::environment::{env, env_scope};
use rise_ui::swiftui::measure::measure;
use rise_ui::reorder_grip::ReorderMotion;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::environment_values::{ColorScheme, ColorSchemeKey};
use rise_ui::swiftui::motion::{ease_in_out, ease_out, spring};
use rise_widgets::navigations::tab_strip_math::TabStripLayout;

use crate::app::rise_launch_splash::RiseLaunchSplashState;
use crate::app::router::main_tab_chrome::MAIN_TAB_BAR_HEIGHT;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::lib::rise_feature_gate::RiseFeatureGate;
#[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
use crate::core::lib::sai::modules::sai_debugger::sai_debugger_overlay_state::SaiDebuggerOverlayState;
use crate::core::widgets::navigations::chat_launch_unread_bubble::ChatLaunchUnreadBubbleUi;
use crate::core::preferences::tab_bar_browse_preferences::TabBarBrowsePreferences;
use crate::core::widgets::navigations::main_tab_bar_buttons::{RailEntry, RiseTabBarButtonExt};
use crate::core::widgets::navigations::main_tab_bar_parts::MainTabBarItemLabel;
use crate::core::widgets::navigations::main_tab_bar_settings::{
    RiseTabBarButtonOrder, RiseTabBarPlatform, RiseTabBarSettings,
};
use crate::core::widgets::navigations::main_tab_scroll_to_top::MainTabScrollToTop;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::engine::rise_user_tab_bar_presentation::RiseUserTabBarPresentation;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MainTabBarItem {
    pub id: MainTab,
    pub title_key: &'static str,
    pub symbol: &'static str,
    pub anchor: Option<RiseGuideAnchor>,
}

impl MainTabBarItem {
    pub const ALL: [MainTabBarItem; 8] = [
        Self::item(MainTab::Posts, "navbtn_posts", "doc.text", Some(RiseGuideAnchor::TabPosts)),
        Self::item(MainTab::GlobalSearch, "navbtn_search", "magnifyingglass", Some(RiseGuideAnchor::TabSearch)),
        Self::item(MainTab::Chats, "navbtn_chats", "bubble.left.and.bubble.right", Some(RiseGuideAnchor::TabChats)),
        Self::item(MainTab::Shorts, "riffs", "play.rectangle.on.rectangle", Some(RiseGuideAnchor::TabRiffs)),
        Self::item(MainTab::Profile, "navbtn_profile", "person.crop.circle", Some(RiseGuideAnchor::TabMore)),
        Self::item(MainTab::Music, "navbtn_music", "music.note", None),
        Self::item(MainTab::Maps, "more_tab_map", "map", None),
        Self::item(MainTab::Vacancies, "navbtn_vacancies", "briefcase", None),
    ];

    const fn item(id: MainTab, title_key: &'static str, symbol: &'static str, anchor: Option<RiseGuideAnchor>) -> Self {
        Self { id, title_key, symbol, anchor }
    }

    /// every button the bar may show, vacancies only when the vacancies gate is on
    pub fn all() -> &'static [MainTabBarItem] {
        if RiseFeatureGate::VACANCIES { &Self::ALL } else { &Self::ALL[..Self::ALL.len() - 1] }
    }
}

/// parts of the bar the reader doesnt choose, the rest is in the tab bar settings
pub struct MainTabBarMetrics;

impl MainTabBarMetrics {
    pub const VISIBLE_COUNT: usize = 5;
    pub const HEIGHT: f32 = MAIN_TAB_BAR_HEIGHT;
    pub const INSET: f32 = 4.0;
    pub const ITEM_HEIGHT: f32 = Self::HEIGHT - Self::INSET * 2.0;
    pub const RAIL_ITEM_HEIGHT: f32 = 50.0;
    pub const REORDER_DRAG_DISTANCE: f32 = 4.0;
    pub const RAIL_ICON_SIZE: f32 = 24.0;
    pub const TRAFFIC_LIGHT_HEIGHT: f32 = 14.0;
    pub const RAIL_TOP_GAP: f32 = 8.0;
    pub const LIFTED_OUTSET: f32 = 5.0;
    pub const LIFTED_WIDEN: f32 = 12.0;
    pub const DROPLET_OUTSET: f32 = 10.0;
    pub const DROPLET_WIDEN: f32 = 20.0;
    pub const EDGE_FADE: f32 = 22.0;
    pub const RAMP_SLOTS: f32 = 0.5;
    pub const TAP_SLOP: f32 = 10.0;
    pub const LIFT_SLOP: f32 = 6.0;
    pub const HOLD_TO_LIFT: Duration = Duration::from_millis(220);
    pub const MAGNIFICATION: f32 = 1.18;
    pub const GLASS_TINT_OPACITY_DARK: f32 = 0.34;
    pub const GLASS_TINT_OPACITY_LIGHT: f32 = 0.42;
    pub const MEDIA_GLASS_WHITE: f32 = 0.17;
}

/// row of the more page, the bar draws only the settings and the command palette lists all
#[derive(Clone, Copy)]
pub struct MainTabBarMoreEntry {
    pub id: &'static str,
    pub title_key: &'static str,
    pub symbol: &'static str,
    pub icon_background: fn() -> Hsla,
    pub anchor: Option<RiseGuideAnchor>,
    pub route: fn(&mut App) -> RootRoute,
}

impl MainTabBarMoreEntry {
    pub const ALL: [MainTabBarMoreEntry; 11] = [
        Self::entry("more_achievements", "settings_achievements_title", "trophy", SettingsIconPalette::achievements, None, |_| {
            RootRoute::Achievements { user_id: None }
        }),
        Self::entry("more_friends", "profile_friends", "person.2.fill", SettingsIconPalette::privacy_friends, None, |cx| {
            let user_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone());
            RootRoute::UserFriends { user_id }
        }),
        Self::entry("more_close_friends", "story_close_friends_title", "star.fill", SettingsIconPalette::privacy_friends, None, |_| {
            RootRoute::CloseFriends
        }),
        Self::entry("more_stories_archive", "archive_title", "archivebox.fill", SettingsIconPalette::profile, None, |_| {
            RootRoute::StoriesArchive
        }),
        Self::entry("more_view_history", "view_history_page_title", "clock.arrow.circlepath", SettingsIconPalette::sessions, None, |_| {
            RootRoute::PostViewHistory
        }),
        Self::entry("more_subscription", "settings_subscription_title", "crown.fill", SettingsIconPalette::subscription, None, |_| {
            RootRoute::SubscriptionSettings
        }),
        Self::entry("more_sparks", "sparks_my_title", "", SettingsIconPalette::sparks, None, |_| RootRoute::SparksWallet),
        Self::entry("more_customization", "settings_customization_title", "paintpalette.fill", SettingsIconPalette::customization, None, |_| {
            RootRoute::CustomizationSettings
        }),
        Self::entry(
            "more_guides",
            "guide_catalogue_title",
            "book.closed.fill",
            SettingsIconPalette::customization,
            Some(RiseGuideAnchor::MoreGuides),
            |_| RootRoute::GuidesCatalogue,
        ),
        Self::entry("more_bots", "bot_my_bots_title", "command.circle.fill", SettingsIconPalette::chat_folders, None, |_| {
            RootRoute::MyBots
        }),
        Self::entry("more_bug_reports", "bug_report_my_title", "ladybug.fill", SettingsIconPalette::moderation, None, |_| {
            RootRoute::MyBugReports
        }),
    ];

    pub fn all() -> &'static [MainTabBarMoreEntry] {
        &Self::ALL
    }

    pub fn settings() -> &'static MainTabBarMoreEntry {
        &Self::SETTINGS
    }

    pub const SETTINGS: MainTabBarMoreEntry =
        Self::entry("more_settings", "settings_page_title", "gearshape.fill", SettingsIconPalette::memory, None, |_| RootRoute::Settings);

    const fn entry(
        id: &'static str,
        title_key: &'static str,
        symbol: &'static str,
        icon_background: fn() -> Hsla,
        anchor: Option<RiseGuideAnchor>,
        route: fn(&mut App) -> RootRoute,
    ) -> Self {
        Self { id, title_key, symbol, icon_background, anchor, route }
    }

    /// nav_store is read in render because the environment stack is gone by the time the click lands
    fn open(&self, nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        let route = (self.route)(cx);
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.open_rail_route(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }
}

/// the bar, current_tab is the router one
/// the settings preview copy moves only its own selection
#[derive(IntoElement)]
pub struct MainTabBar {
    current_tab: Binding<MainTab>,
    settings: RiseTabBarSettings,
    is_preview: bool,
    corner_radius: Pixels,
    foot_accessory: Option<AnyView>,
}

/// state of the tab bar, one per bar
pub struct MainTabBarState {
    pill_y: Animated<f32>,
    lift: Animated<f32>,
    stretch: Animated<f32>,
    placed: bool,
    placed_order: Option<RiseTabBarButtonOrder>,
    return_token: u64,
    handled_reveal: u64,
    geometry: Rc<MainTabBarGeometry>,
    strip_resting: bool,
    strip_rest_task: Option<Task<()>>,
    last_offset: Option<f32>,
    bubble_opacity: Animated<f32>,
    bubble_placed: bool,
    reorder: ReorderMotion<RailEntry>,
    drag_swallows_click: bool,
    pill_entry: Option<RailEntry>,
    pill_in_foot: bool,
    preview_settings: bool,
}

impl MainTabBarState {
    fn new() -> Self {
        Self {
            pill_y: Animated::new(0.0),
            lift: Animated::new(0.0),
            stretch: Animated::new(0.0),
            placed: false,
            placed_order: None,
            return_token: 0,
            handled_reveal: 0,
            geometry: Rc::new(MainTabBarGeometry::new()),
            strip_resting: true,
            strip_rest_task: None,
            last_offset: None,
            bubble_opacity: Animated::new(0.0),
            bubble_placed: false,
            reorder: ReorderMotion::new(),
            drag_swallows_click: false,
            pill_entry: None,
            pill_in_foot: false,
            preview_settings: false,
        }
    }

    /// selection changed from outside like a deep link or guide or push
    /// or the reader reordered buttons or the bar was just placed
    fn follow(&mut self, entry: RailEntry, center: f32, order: RiseTabBarButtonOrder, cx: &App) {
        let reordered = self.placed_order.is_some_and(|placed| placed != order);
        self.placed_order = Some(order);
        let same_entry = self.pill_entry.replace(entry) == Some(entry);
        if !self.placed {
            self.placed = true;
            self.pill_y.jump(center);
            return;
        }
        if (self.pill_y.target() - center).abs() <= 0.5 {
            return;
        }
        if same_entry && !reordered && !self.pill_y.is_running(RiseClock::monotonic()) {
            self.pill_y.jump(center);
            return;
        }
        let motion = if reordered { spring(0.42, 0.8) } else { spring(0.42, 0.76) };
        self.pill_y.set_in(center, motion, RiseClock::monotonic(), cx);
    }

    fn note_offset(&mut self, offset: f32, cx: &mut Context<Self>) {
        let last = self.last_offset.replace(offset);
        let moved = last.is_some_and(|last| (last - offset).abs() > 0.5);
        if moved {
            if self.pill_in_foot
                && let Some(last) = last
            {
                self.pill_y.jump(*self.pill_y.target() - (offset - last));
            }
            self.note_strip_moved(cx);
        }
    }

    fn note_strip_moved(&mut self, cx: &mut Context<Self>) {
        self.strip_resting = false;
        self.strip_rest_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(MainTabBar::CHATS_BUBBLE_REST).await;
            let _ = this.update(cx, |state, cx| {
                state.strip_resting = true;
                cx.notify();
            });
        }));
    }

    fn show_bubble(&mut self, shows: bool, cx: &App) {
        let target = if shows { 1.0 } else { 0.0 };
        if !self.bubble_placed {
            self.bubble_placed = true;
            self.bubble_opacity.jump(target);
            return;
        }
        if *self.bubble_opacity.target() != target {
            self.bubble_opacity.set_in(target, ease_in_out(0.2), RiseClock::monotonic(), cx);
        }
    }
}

impl MainTabBar {
    const CHATS_BUBBLE_REST: Duration = Duration::from_millis(550);

    pub fn new(current_tab: Binding<MainTab>, settings: RiseTabBarSettings) -> Self {
        Self { current_tab, settings, is_preview: false, corner_radius: px(0.0), foot_accessory: None }
    }

    pub fn foot_accessory(mut self, accessory: Option<AnyView>) -> Self {
        self.foot_accessory = accessory;
        self
    }

    pub fn is_preview(mut self, is_preview: bool) -> Self {
        self.is_preview = is_preview;
        self
    }

    /// bar arc runs parallel to the window corner it sits in
    pub fn corner_radius(mut self, corner_radius: Pixels) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    fn items(&self) -> Vec<MainTabBarItem> {
        MainTabBarItem::ordered(&self.settings.button_order)
    }

    /// height of the settings preview copy, every button shown without scrolling
    pub fn preview_height() -> f32 {
        MainTabBarMetrics::INSET * 5.0 + MainTabBarMetrics::RAIL_ITEM_HEIGHT * (MainTabBarItem::all().len() + 1) as f32
    }

    fn wire_button(tab: MainTab) -> Option<RiseTabBarButton> {
        RiseTabBarButton::ALL.into_iter().find(|button| button.tab() == tab)
    }

    fn rail_entries(items: &[MainTabBarItem], cx: &mut App) -> Vec<RailEntry> {
        let buttons: Vec<RiseTabBarButton> = items.iter().filter_map(|item| Self::wire_button(item.id)).collect();
        RailEntry::rail(TabBarBrowsePreferences::shared(cx).read(cx).rail_order(), &buttons)
    }

    fn rail_slots(&self, count: usize, strip_top: f32, scroll_offset: f32, window: &Window) -> Rc<Vec<f32>> {
        let foot_start = count.saturating_sub(RailEntry::FOOT_SIZE);
        let item = MainTabBarMetrics::RAIL_ITEM_HEIGHT;
        let inset = MainTabBarMetrics::INSET;
        let bottom = f32::from(window.viewport_size().height) - inset;
        Rc::new(
            (0..count)
                .map(|slot| {
                    if slot < foot_start {
                        strip_top + inset + item * slot as f32 + scroll_offset
                    } else {
                        let from_bottom = (count - slot) as f32;
                        bottom - item * from_bottom - inset * (from_bottom - 1.0)
                    }
                })
                .collect(),
        )
    }

    /// a bar place that follows the mouse when dragged, neighbours of its part make room and a drop glides home
    #[allow(clippy::too_many_arguments)]
    fn reorderable(
        &self,
        entry: RailEntry,
        element: AnyElement,
        base: &[RailEntry],
        slots: &Rc<Vec<f32>>,
        state: &Entity<MainTabBarState>,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        if self.is_preview {
            return element;
        }
        let slot_of = |slots: &[f32], index: usize| slots.get(index).copied().unwrap_or(0.0);
        let (offset, raise, leaves_strip) = {
            let motion = &state.read(cx).reorder;
            let foot_start = RailEntry::foot_start(base);
            let kept = base.iter().position(|candidate| *candidate == entry);
            let laid = motion.shown_order(base).iter().position(|candidate| *candidate == entry);
            (
                motion.offset_from_slots(&entry, base, |index| slot_of(slots, index), window),
                motion.lift(&entry, window),
                kept.is_some_and(|kept| kept < foot_start) && laid.is_some_and(|laid| laid >= foot_start),
            )
        };
        let base = base.to_vec();
        let drag_slots = slots.clone();
        let surface = rise_ui::theme(cx).bg._100;
        let change_state = state.clone();
        let end_state = state.clone();
        let down_state = state.clone();
        let key = match entry {
            RailEntry::Button(button) => SharedString::from(format!("main-tab-bar-reorder-{}", button.wire())),
            RailEntry::Settings => SharedString::new_static("main-tab-bar-reorder-settings"),
        };
        let gesture = DragGesture::new(MainTabBarMetrics::REORDER_DRAG_DISTANCE)
            .on_changed(move |value: &DragGestureValue, _: &mut Window, cx: &mut App| {
                let translation = f32::from(value.translation.height);
                let reduce_motion = cx.reduce_motion();
                change_state.update(cx, |state, cx| {
                    state.drag_swallows_click = true;
                    state.reorder.drag_slots(
                        &entry,
                        translation,
                        &base,
                        |index| slot_of(&drag_slots, index),
                        RiseClock::monotonic(),
                        reduce_motion,
                    );
                    cx.notify();
                });
            })
            .on_ended(move |_: &DragGestureValue, _: &mut Window, cx: &mut App| {
                let reduce_motion = cx.reduce_motion();
                let order = end_state.update(cx, |state, cx| {
                    let order = state.reorder.end(RiseClock::monotonic(), reduce_motion);
                    cx.notify();
                    order
                });
                if let Some(order) = order {
                    RailEntry::save(&order, cx);
                    end_state.update(cx, |state, _| state.reorder.settle());
                }
            });
        let place = div()
            .id(ElementId::Name(key.clone()))
            .relative()
            .top(px(offset))
            .flex_shrink_0()
            .w_full()
            .rounded(px(MainTabBarMetrics::RAIL_ITEM_HEIGHT / 2.0))
            .when(raise > 0.0, |place| {
                place.bg(surface).shadow(vec![BoxShadow {
                    color: hsla(0.0, 0.0, 0.0, 0.22 * raise),
                    offset: point(px(0.0), px(4.0 * raise)),
                    blur_radius: px(14.0 * raise),
                    spread_radius: px(0.0),
                    inset: false,
                }])
            })
            .capture_any_mouse_down(move |_, _, cx| {
                down_state.update(cx, |state, _| state.drag_swallows_click = false);
            })
            .child(element)
            .gesture(ElementId::Name(key), gesture);
        if raise > 0.0 {
            deferred(place).with_priority(1).into_any_element()
        } else if leaves_strip {
            deferred(place).into_any_element()
        } else {
            place.into_any_element()
        }
    }

    fn slot_center(slot: usize) -> f32 {
        MainTabBarMetrics::INSET + MainTabBarMetrics::RAIL_ITEM_HEIGHT * (slot as f32 + 0.5)
    }

    fn uses_glass(&self) -> bool {
        self.settings.surface.draws_glass()
    }

    fn lifted_outset(&self) -> f32 {
        if self.uses_glass() { MainTabBarMetrics::DROPLET_OUTSET } else { MainTabBarMetrics::LIFTED_OUTSET }
    }

    fn lifted_widen(&self) -> f32 {
        if self.uses_glass() { MainTabBarMetrics::DROPLET_WIDEN } else { MainTabBarMetrics::LIFTED_WIDEN }
    }

    #[allow(clippy::too_many_arguments)]
    fn commit(
        item: MainTabBarItem,
        pill_center: Option<f32>,
        current_tab: &Binding<MainTab>,
        state: &Entity<MainTabBarState>,
        nav_store: Option<&Entity<NavigationStore>>,
        is_preview: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let tab = item.id;
        let current = current_tab.get(cx);
        if !is_preview {
            #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
            if tab == MainTab::Profile {
                SaiDebuggerOverlayState::shared(cx).update(cx, |overlay, cx| overlay.register_profile_tab_tap(cx));
            }
            let picked =
                nav_store.is_some_and(|nav_store| nav_store.update(cx, |nav_store, cx| nav_store.pick_tab(tab, cx)));
            tracing::warn!(target: "riseonly::X1107", ?tab, ?current, picked, "rail commit");
            if tab == current && !picked {
                MainTabScrollToTop::reselect(tab, window, cx);
            }
        }
        if tab != current {
            current_tab.set(tab, window, cx);
        }

        let reduce_motion = cx.reduce_motion();
        let token = state.update(cx, |state, cx| {
            let now = RiseClock::monotonic();
            state.return_token += 1;
            state.preview_settings = false;
            let travel = match pill_center {
                Some(center) => {
                    let travel = (center - state.pill_y.value(now)).abs();
                    state.pill_y.set(center, spring(0.42, 0.76), now, reduce_motion);
                    travel
                }
                None => 0.0,
            };
            state.lift.set(0.32, ease_out(0.12), now, reduce_motion);
            let stretch = (travel / (MainTabBarMetrics::RAIL_ITEM_HEIGHT * 10.0)).min(0.22);
            state.stretch.set(stretch, ease_out(0.12), now, reduce_motion);
            cx.notify();
            state.return_token
        });

        let state = state.downgrade();
        window
            .spawn(cx, async move |cx| {
                cx.background_executor().timer(Duration::from_millis(120)).await;
                let _ = state.update(cx, |state, cx| {
                    if state.return_token != token {
                        return;
                    }
                    let now = RiseClock::monotonic();
                    let reduce_motion = cx.reduce_motion();
                    state.lift.set(0.0, spring(0.42, 0.58), now, reduce_motion);
                    state.stretch.set(0.0, spring(0.42, 0.58), now, reduce_motion);
                    cx.notify();
                });
            })
            .detach();
    }

    fn resting_fill(dark: bool) -> Hsla {
        if dark { hsla(0.0, 0.0, 1.0, 0.13) } else { hsla(0.0, 0.0, 0.0, 0.07) }
    }

    fn lens_fill(&self, dark: bool, lift: f32) -> (Hsla, Hsla) {
        let resting = Self::resting_fill(dark);
        let lifted = if self.uses_glass() {
            gpui::transparent_black()
        } else if dark {
            hsla(0.0, 0.0, 1.0, 0.07)
        } else {
            hsla(0.0, 0.0, 1.0, 0.32)
        };
        (resting.opacity(1.0 - lift), lifted.opacity(lift))
    }

    fn lens_body(dark: bool, lift: f32, radius: Pixels) -> Div {
        let fill = if dark { hsla(0.0, 0.0, 0.16, 0.55) } else { hsla(0.0, 0.0, 1.0, 0.45) };
        div().absolute().top_0().left_0().size_full().rounded(radius).bg(fill.opacity(lift)).shadow(vec![BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.22 * lift),
            offset: point(px(0.0), px(5.0)),
            blur_radius: px(12.0),
            spread_radius: px(0.0),
            inset: false,
        }])
    }

    fn lens_rim(dark: bool, lift: f32, radius: Pixels) -> Div {
        let rim = if dark { hsla(0.0, 0.0, 1.0, 0.55) } else { hsla(0.0, 0.0, 1.0, 0.95) };
        let glow = hsla(0.0, 0.0, 1.0, if dark { 0.09 } else { 0.2 });
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .rounded(radius)
            .opacity(lift)
            .border_1()
            .border_color(rim)
            .shadow(vec![BoxShadow {
                color: glow,
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(3.0),
                spread_radius: px(2.5),
                inset: true,
            }])
            .when(!dark, |lens| {
                lens.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .rounded(radius)
                        .border(px(0.5))
                        .border_color(hsla(0.0, 0.0, 0.0, 0.08)),
                )
            })
    }

    fn pill(&self, center: f32, lift: f32, stretch: f32, width: f32, dark: bool) -> AnyElement {
        let rest_length = MainTabBarMetrics::RAIL_ITEM_HEIGHT;
        let rest_thickness = width;
        let lifted_length = rest_length + self.lifted_widen();
        let lifted_thickness = width + self.lifted_outset() * 2.0;
        let length = ((rest_length + (lifted_length - rest_length) * lift) * (1.0 + stretch)).max(0.0);
        let thickness = ((rest_thickness + (lifted_thickness - rest_thickness) * lift) * (1.0 - stretch * 0.35)).max(0.0);
        let (resting, lifted) = self.lens_fill(dark, lift);
        let radius = px(MainTabBarMetrics::RAIL_ITEM_HEIGHT / 2.0);
        div()
            .absolute()
            .top(px(center - length / 2.0))
            .left(px((width - thickness) / 2.0))
            .w(px(thickness))
            .h(px(length))
            .rounded(radius)
            .when(!self.uses_glass(), |pill| pill.child(Self::lens_body(dark, lift, radius)))
            .child(div().absolute().top_0().left_0().size_full().rounded(radius).bg(resting))
            .child(div().absolute().top_0().left_0().size_full().rounded(radius).bg(lifted))
            .child(Self::lens_rim(dark, lift, radius))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn strip_button(
        &self,
        index: usize,
        item: MainTabBarItem,
        pill_center: Option<f32>,
        is_selected: bool,
        lift: f32,
        hover: Hsla,
        state: &Entity<MainTabBarState>,
        nav_store: Option<&Entity<NavigationStore>>,
    ) -> AnyElement {
        let current_tab = self.current_tab.clone();
        let state = state.clone();
        let nav_store = nav_store.cloned();
        let is_preview = self.is_preview;
        let label = MainTabBarItemLabel::new(item, is_selected);
        let label: AnyElement = if is_selected {
            use rise_ui::swiftui::transition::TransitionModifiers;
            div()
                .child(label)
                .scale_effect(1.0 + (MainTabBarMetrics::MAGNIFICATION - 1.0) * lift)
                .into_any_element()
        } else {
            label.into_any_element()
        };
        let button = div()
            .id(ElementId::NamedInteger(SharedString::new_static("main-tab-bar-item"), index as u64))
            .tab_index(index as isize)
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .w_full()
            .h(px(MainTabBarMetrics::RAIL_ITEM_HEIGHT))
            .rounded(px(MainTabBarMetrics::RAIL_ITEM_HEIGHT / 2.0))
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .capture_any_mouse_down(move |event: &MouseDownEvent, _, _| {
                tracing::warn!(target: "riseonly::X1107", tab = ?item.id, position = ?event.position, "rail capture down");
            })
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, _| {
                tracing::warn!(target: "riseonly::X1107", tab = ?item.id, position = ?event.position, "rail bubble down");
            })
            .on_click(move |_: &ClickEvent, window, cx| {
                tracing::warn!(target: "riseonly::X1107", tab = ?item.id, "rail click");
                if state.update(cx, |state, _| std::mem::take(&mut state.drag_swallows_click)) {
                    return;
                }
                Self::commit(item, pill_center, &current_tab, &state, nav_store.as_ref(), is_preview, window, cx);
            })
            .child(label);
        let anchor = if self.is_preview { None } else { item.anchor };
        button.guide_anchor(anchor).into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn settings_button(
        &self,
        nav_store: Option<Entity<NavigationStore>>,
        glyph: Hsla,
        text: Hsla,
        tint: Hsla,
        hover: Hsla,
        is_selected: bool,
        state: &Entity<MainTabBarState>,
    ) -> AnyElement {
        let entry = MainTabBarMoreEntry::settings();
        let is_preview = self.is_preview;
        let press_state = state.clone();
        let (glyph, text) = if is_selected { (tint, tint) } else { (glyph, text) };
        let button = plain(ElementId::Name(SharedString::new_static(entry.id)))
            .flex()
            .flex_col()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .gap(px(2.0))
            .w_full()
            .h(px(MainTabBarMetrics::RAIL_ITEM_HEIGHT))
            .rounded(px(MainTabBarMetrics::RAIL_ITEM_HEIGHT / 2.0))
            .hover_fill(hover)
            .on_press(move |_window, cx| {
                if press_state.update(cx, |state, _| std::mem::take(&mut state.drag_swallows_click)) {
                    return;
                }
                if is_preview {
                    press_state.update(cx, |state, cx| {
                        state.preview_settings = true;
                        cx.notify();
                    });
                } else {
                    entry.open(nav_store.as_ref(), cx);
                }
            })
            .child(div().h(px(MainTabBarMetrics::RAIL_ICON_SIZE + 4.0)).flex().items_center().justify_center().children(IconUi::sized(
                if is_selected { "gearshape.fill" } else { "gearshape" },
                px(MainTabBarMetrics::RAIL_ICON_SIZE),
                glyph,
                is_selected,
            )))
            .child(
                MainText::new(tr(entry.title_key))
                    .px(10.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(text)
                    .number_of_lines(1)
                    .minimum_scale_factor(0.7),
            );
        button.into_any_element()
    }
}

impl RenderOnce for MainTabBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (theme, theme_dark) = {
            let store = ThemeStore::shared(cx);
            let store = store.read(cx);
            (store.current_theme().clone(), store.resolved_appearance() == RiseThemeAppearance::Dark)
        };
        let shell = theme.shell;
        let state = window.use_keyed_state(
            ElementId::Name(SharedString::new_static(if self.is_preview { "main-tab-bar-preview" } else { "main-tab-bar" })),
            cx,
            |_, _| MainTabBarState::new(),
        );

        let items = self.items();
        let current = self.current_tab.get(cx);
        let nav_store = if self.is_preview { None } else { env::<NavigationStoreKey>(cx) };
        let (settings_open, page_pushed) = nav_store.as_ref().map_or((false, false), |nav_store| {
            let routes = nav_store.read(cx).routes_since_tab_switch();
            (routes.iter().any(|route| matches!(route, RootRoute::Settings)), !routes.is_empty())
        });
        let settings_open = settings_open || (self.is_preview && state.read(cx).preview_settings);
        let over_media = !theme_dark && !self.is_preview && current == MainTab::Shorts && !page_pushed;
        let dark = theme_dark || over_media;
        let selected_index = items.iter().position(|item| item.id == current).unwrap_or(0);
        let entries = Self::rail_entries(&items, cx);
        let foot_start = RailEntry::foot_start(&entries);
        let strip_base = entries[..foot_start].to_vec();
        let foot_entries = entries[foot_start..].to_vec();
        let strip_entries = strip_base.clone();
        let selected_entry =
            if settings_open { Some(RailEntry::Settings) } else { Self::wire_button(current).map(RailEntry::Button) };
        let selected_place = selected_entry
            .and_then(|selected| entries.iter().position(|entry| *entry == selected).map(|place| (selected, place)));
        let order = self.settings.button_order;

        let geometry = state.read(cx).geometry.clone();
        geometry.set_items(&items);
        geometry.set_strip(
            strip_entries
                .iter()
                .map(|entry| match entry {
                    RailEntry::Button(button) => Some(button.tab()),
                    RailEntry::Settings => None,
                })
                .collect(),
        );
        if !self.is_preview {
            let reveal = MainTabBarReveal::shared(cx);
            reveal.update(cx, |reveal, _| reveal.attach(&geometry));
            let request = reveal.read(cx).request();
            if let Some(request) = request {
                let handled = state.read(cx).handled_reveal;
                if request.id != handled {
                    state.update(cx, |state, _| state.handled_reveal = request.id);
                    geometry.bring_into_view(request.index);
                }
            }
        }

        let offset = f32::from(geometry.scroll.offset().y);
        state.update(cx, |state, cx| state.note_offset(offset, cx));
        let pill_target = selected_place.and_then(|(entry, place)| {
            if place < foot_start {
                Some((entry, Self::slot_center(place), false))
            } else {
                geometry.foot_center(place - foot_start).map(|center| (entry, center - offset, true))
            }
        });
        if let Some((entry, center, in_foot)) = pill_target {
            state.update(cx, |state, cx| {
                state.pill_in_foot = in_foot;
                state.follow(entry, center, order, cx);
            });
        }

        let now = RiseClock::monotonic();
        let (pill_y, lift, stretch) = {
            let state = state.read(cx);
            (
                state.pill_y.frame_value(now, window),
                state.lift.frame_value(now, window).clamp(0.0, 1.0),
                state.stretch.frame_value(now, window),
            )
        };

        let strip_width = f32::from(shell.rail_width) - MainTabBarMetrics::INSET * 2.0;
        let strip_top = if self.is_preview {
            MainTabBarMetrics::INSET
        } else if HostOs::current() == HostOs::MacOs {
            f32::from(shell.traffic_light_inset) + MainTabBarMetrics::TRAFFIC_LIGHT_HEIGHT + MainTabBarMetrics::RAIL_TOP_GAP
                - MainTabBarMetrics::INSET
        } else {
            f32::from(shell.window_drag_height)
        };
        let slots = self.rail_slots(entries.len(), strip_top, offset, window);
        let (settings_glyph, settings_text) =
            if over_media { (hsla(0.0, 0.0, 1.0, 0.72), gpui::white()) } else { (theme.text.secondary, theme.text.primary) };
        let hover = Self::resting_fill(dark).opacity(0.5);

        let mut strip = div()
            .id(ElementId::Name(SharedString::new_static("main-tab-bar-strip")))
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .pt(px(MainTabBarMetrics::INSET))
            .overflow_y_scroll()
            .track_scroll(&geometry.scroll);
        for (slot, entry) in strip_entries.iter().copied().enumerate() {
            let element = match entry {
                RailEntry::Button(button) => {
                    let Some(index) = items.iter().position(|item| item.id == button.tab()) else {
                        continue;
                    };
                    self.strip_button(
                        index,
                        items[index],
                        Some(Self::slot_center(slot)),
                        index == selected_index && !settings_open,
                        lift,
                        hover,
                        &state,
                        nav_store.as_ref(),
                    )
                }
                RailEntry::Settings => self.settings_button(
                    nav_store.clone(),
                    settings_glyph,
                    settings_text,
                    theme.primary._100,
                    hover,
                    settings_open,
                    &state,
                ),
            };
            strip = strip.child(self.reorderable(entry, element, &entries, &slots, &state, window, cx));
        }

        let mut foot = div().relative().flex().flex_col().gap(px(MainTabBarMetrics::INSET)).w_full();
        for (foot_index, entry) in foot_entries.into_iter().enumerate() {
            let element = match entry {
                RailEntry::Settings => self.settings_button(
                    nav_store.clone(),
                    settings_glyph,
                    settings_text,
                    theme.primary._100,
                    hover,
                    settings_open,
                    &state,
                ),
                RailEntry::Button(button) => {
                    let Some(index) = items.iter().position(|item| item.id == button.tab()) else {
                        continue;
                    };
                    let selected = !settings_open && current == button.tab();
                    self.strip_button(
                        index,
                        items[index],
                        geometry.foot_center(foot_index).map(|center| center - offset),
                        selected,
                        if selected { lift } else { 0.0 },
                        hover,
                        &state,
                        nav_store.as_ref(),
                    )
                }
            };
            foot = foot.child(self.reorderable(entry, element, &entries, &slots, &state, window, cx));
        }
        let foot_geometry = geometry.clone();
        let foot_state = state.clone();
        foot = foot.child(
            measure("main-tab-bar-foot", move |bounds, _, cx| {
                foot_geometry.foot.set(Some(bounds));
                foot_state.update(cx, |_, cx| cx.notify());
            })
            .absolute()
            .inset_0(),
        );
        let foot = div()
            .flex()
            .flex_col()
            .w_full()
            .pt(px(MainTabBarMetrics::INSET))
            .children(self.foot_accessory.clone())
            .child(foot);

        let media_backing = over_media.then(|| {
            div().absolute().top_0().left_0().size_full().rounded(self.corner_radius).bg(gpui::black())
        });

        let mut bar = div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w(shell.rail_width)
            .h_full()
            .pt(px(strip_top))
            .pb(px(MainTabBarMetrics::INSET))
            .px(px(MainTabBarMetrics::INSET))
            .children(media_backing)
            .children(pill_target.map(|_| {
                div()
                    .absolute()
                    .top(px(strip_top))
                    .bottom_0()
                    .left(px(MainTabBarMetrics::INSET))
                    .right(px(MainTabBarMetrics::INSET))
                    .overflow_y_hidden()
                    .child(self.pill(pill_y + offset, lift, stretch, strip_width, dark))
            }))
            .child(strip)
            .child(foot);

        if !self.is_preview
            && !RiseLaunchSplashState::is_visible()
            && let Some((top, is_on_screen)) = geometry.chats_bubble_top(strip_top)
        {
            let shows = is_on_screen && state.read(cx).strip_resting;
            state.update(cx, |state, cx| state.show_bubble(shows, cx));
            let opacity = state.read(cx).bubble_opacity.frame_value(now, window).clamp(0.0, 1.0);
            if opacity > 0.0 {
                let anchor = geometry.chats_center_in_window();
                let panel = f32::from(window.viewport_size().height);
                bar = bar.child(deferred(
                    div()
                        .absolute()
                        .left(shell.rail_width + px(ChatLaunchUnreadBubbleUi::LIFT_ABOVE_BAR))
                        .top(px(top))
                        .h(px(MainTabBarMetrics::RAIL_ITEM_HEIGHT))
                        .flex()
                        .items_center()
                        .opacity(opacity)
                        .child(ChatLaunchUnreadBubbleUi::new().room(anchor, (panel - anchor).max(0.0))),
                ));
            }
        }

        let bar = if over_media {
            env_scope::<ColorSchemeKey>(Some(ColorScheme::Dark)).child(bar).into_any_element()
        } else {
            bar.into_any_element()
        };
        env_scope::<IgnoresTextSizeDeltaKey>(true).child(bar)
    }
}

/// where the strip sits, outside the bar state so a guide can ask without a redraw
pub struct MainTabBarGeometry {
    pub scroll: ScrollHandle,
    pub loops: Cell<bool>,
    items: RefCell<Vec<MainTabBarItem>>,
    strip: RefCell<Vec<Option<MainTab>>>,
    foot: Cell<Option<Bounds<Pixels>>>,
}

impl Default for MainTabBarGeometry {
    fn default() -> Self {
        Self::new()
    }
}

impl MainTabBarGeometry {
    pub fn new() -> Self {
        let strip = MainTabBarItem::all()
            .iter()
            .filter(|item| item.id != MainTab::Profile)
            .map(|item| Some(item.id))
            .chain(std::iter::once(None))
            .collect();
        Self {
            scroll: ScrollHandle::new(),
            loops: Cell::new(false),
            items: RefCell::new(MainTabBarItem::all().to_vec()),
            strip: RefCell::new(strip),
            foot: Cell::new(None),
        }
    }

    /// horizontal strip layout of the phone, for bar copies that still draw one
    pub fn layout(width: f32, count: usize) -> TabStripLayout {
        TabStripLayout::new(width - MainTabBarMetrics::INSET * 2.0, count, MainTabBarMetrics::VISIBLE_COUNT, 0.0)
    }

    /// centre of a foot place under the top of the strip, the frame the pill moves in
    fn foot_center(&self, index: usize) -> Option<f32> {
        let foot = self.foot.get()?;
        let strip = self.scroll.bounds();
        if strip.size.height <= px(0.0) {
            return None;
        }
        let step = MainTabBarMetrics::RAIL_ITEM_HEIGHT + MainTabBarMetrics::INSET;
        Some(f32::from(foot.top() - strip.top()) + step * index as f32 + MainTabBarMetrics::RAIL_ITEM_HEIGHT / 2.0)
    }

    fn set_items(&self, items: &[MainTabBarItem]) {
        let mut stored = self.items.borrow_mut();
        if stored.as_slice() != items {
            *stored = items.to_vec();
        }
    }

    fn set_strip(&self, strip: Vec<Option<MainTab>>) {
        let mut stored = self.strip.borrow_mut();
        if *stored != strip {
            *stored = strip;
        }
    }

    /// strip slot of the button at index of the items, none when it sits at the foot
    fn slot_of(&self, index: usize) -> Option<usize> {
        let tab = self.items.borrow().get(index)?.id;
        self.strip.borrow().iter().position(|entry| *entry == Some(tab))
    }

    fn scroll_item(&self, index: usize) -> Option<usize> {
        self.slot_of(index)
    }

    pub fn is_fully_visible(&self, index: usize, count: usize) -> bool {
        if index >= count {
            return true;
        }
        let Some(item) = self.scroll_item(index) else {
            return true;
        };
        let viewport = self.scroll.bounds();
        let Some(bounds) = self.scroll.bounds_for_item(item) else {
            return true;
        };
        if viewport.size.height <= px(0.0) {
            return true;
        }
        bounds.top() >= viewport.top() - px(0.5) && bounds.bottom() <= viewport.bottom() + px(0.5)
    }

    fn bring_into_view(&self, index: usize) {
        if let Some(item) = self.scroll_item(index) {
            self.scroll.scroll_to_item(item);
        }
    }

    fn chats_bubble_top(&self, strip_top: f32) -> Option<(f32, bool)> {
        let items = self.items.borrow();
        let index = items.iter().position(|item| item.id == MainTab::Chats)?;
        if index >= MainTabBarMetrics::VISIBLE_COUNT {
            return None;
        }
        let slot = self.slot_of(index)?;
        let viewport = f32::from(self.scroll.bounds().size.height);
        let offset = f32::from(self.scroll.offset().y);
        let center = MainTabBar::slot_center(slot) + offset;
        let margin = MainTabBarMetrics::RAIL_ITEM_HEIGHT * 0.25;
        let is_on_screen = viewport <= 0.0 || (center >= margin && center <= viewport - margin);
        Some((strip_top + center - MainTabBarMetrics::RAIL_ITEM_HEIGHT / 2.0, is_on_screen))
    }

    fn chats_center_in_window(&self) -> f32 {
        let index = self.items.borrow().iter().position(|item| item.id == MainTab::Chats);
        let center = index.and_then(|index| self.slot_of(index)).map_or(0.0, MainTabBar::slot_center);
        f32::from(self.scroll.bounds().top()) + center + f32::from(self.scroll.offset().y)
    }
}

/// lets a guide bring a bar button into view
/// the strip isnt a kit scroll view so the guide scroller asks here first
pub struct MainTabBarReveal {
    request: Option<MainTabBarRevealRequest>,
    next_id: u64,
    geometry: Weak<MainTabBarGeometry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MainTabBarRevealRequest {
    pub index: usize,
    pub animated: bool,
    pub id: u64,
}

impl MainTabBarReveal {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { request: None, next_id: 0, geometry: Weak::new() }
    }

    pub fn request(&self) -> Option<MainTabBarRevealRequest> {
        self.request
    }

    pub fn attach(&mut self, geometry: &Rc<MainTabBarGeometry>) {
        self.geometry = Rc::downgrade(geometry);
    }

    pub fn detach(&mut self, geometry: &Rc<MainTabBarGeometry>) {
        if self.geometry.ptr_eq(&Rc::downgrade(geometry)) {
            self.geometry = Weak::new();
        }
    }

    /// none when the anchor isnt a bar button, else whether the strip has to move
    pub fn reveal(&mut self, anchor: RiseGuideAnchor, animated: bool, cx: &mut Context<Self>) -> Option<bool> {
        let order: Vec<RiseTabBarButton> =
            RiseUserTabBarPresentation::shared(cx).read(cx).current().button_order.to_vec();
        let items = MainTabBarItem::ordered(&order);
        let index = items.iter().position(|item| item.anchor == Some(anchor))?;
        match self.geometry.upgrade() {
            Some(geometry) if !geometry.is_fully_visible(index, items.len()) => {}
            _ => return Some(false),
        }
        self.next_id += 1;
        self.request = Some(MainTabBarRevealRequest { index, animated, id: self.next_id });
        cx.notify();
        Some(true)
    }
}
