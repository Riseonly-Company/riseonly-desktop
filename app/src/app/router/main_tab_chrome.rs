use gpui::{App, AppContext, Context, Entity, Global, Window};
use rise_ui::swiftui::environment::{EnvKey, env};

use crate::app::router::navigation_types::MainTab;

pub const MAIN_TAB_BAR_HEIGHT: f32 = 62.0;

pub struct MainTabChrome;

pub enum MainTabChromeZLayer {}

impl MainTabChromeZLayer {
    pub const TAB_BAR: f64 = 1.0;
    pub const SELECTION_BAR: f64 = 2.0;
    pub const SHEET: f64 = 3.0;
    pub const MENU: f64 = 4.0;
}

impl MainTabChrome {
    pub const FLOATING_BAR_HORIZONTAL_INSET: f32 = 15.0;
    pub const FLOATING_BAR_MIN_BOTTOM_GAP: f32 = Self::FLOATING_BAR_HORIZONTAL_INSET;
    pub const MINI_PLAYER_GAP_ABOVE_TAB_BAR: f32 = 10.0;
    pub const MINI_PLAYER_BLOCK_HEIGHT: f32 = 54.0;

    pub fn extra_underlap_when_mini_player_visible(visible: bool) -> f32 {
        if !visible {
            return 0.0;
        }
        Self::MINI_PLAYER_BLOCK_HEIGHT + Self::MINI_PLAYER_GAP_ABOVE_TAB_BAR
    }

    /// gap from the window bottom edge to the bar
    pub fn bar_bottom_gap(safe_bottom: f32) -> f32 {
        Self::FLOATING_BAR_MIN_BOTTOM_GAP.max(safe_bottom - 5.0)
    }

    pub fn content_bottom_underlap(safe_bottom: f32, mini_player_visible: bool) -> f32 {
        Self::bar_bottom_gap(safe_bottom)
            + MAIN_TAB_BAR_HEIGHT
            + Self::extra_underlap_when_mini_player_visible(mini_player_visible)
    }

    pub const BOTTOM_UNDERLAP: f32 = 78.0;
    pub const NATIVE_TAB_BAR_DEVICE_FUDGE: f32 = 12.0;
    pub const NATIVE_TAB_BAR_BLOCK_HEIGHT: f32 = 49.0;

    pub fn native_tab_bar_reserved_height(safe_bottom: f32) -> f32 {
        safe_bottom + Self::NATIVE_TAB_BAR_BLOCK_HEIGHT + Self::NATIVE_TAB_BAR_DEVICE_FUDGE
    }

    pub fn native_content_bottom_underlap(safe_bottom: f32, mini_player_visible: bool) -> f32 {
        Self::native_tab_bar_reserved_height(safe_bottom)
            + Self::extra_underlap_when_mini_player_visible(mini_player_visible)
    }
}

pub struct MainTabBarUnderlapKey;

impl EnvKey for MainTabBarUnderlapKey {
    type Value = f32;

    fn default_value() -> Self::Value {
        0.0
    }

    fn fingerprint(_value: &Self::Value) -> Option<u64> {
        None
    }
}

/// change signal of the tab pages underlap, only views that read the value get rendered on change
pub struct MainTabBarUnderlapSignal;

struct MainTabBarUnderlapSlot {
    signal: Entity<MainTabBarUnderlapSignal>,
    published: f32,
}

impl Global for MainTabBarUnderlapSlot {}

impl MainTabBarUnderlapSignal {
    /// called by the tab host in render with its underlap, a change notifies readers after the frame
    pub fn publish(underlap: f32, window: &mut Window, cx: &mut App) {
        if !cx.has_global::<MainTabBarUnderlapSlot>() {
            let signal = cx.new(|_| MainTabBarUnderlapSignal);
            cx.set_global(MainTabBarUnderlapSlot { signal, published: underlap });
            return;
        }
        let slot = cx.global_mut::<MainTabBarUnderlapSlot>();
        if slot.published == underlap {
            return;
        }
        slot.published = underlap;
        let signal = slot.signal.clone();
        window.defer(cx, move |_, cx| signal.update(cx, |_, cx| cx.notify()));
    }
}

/// the paged tab on screen
/// a hidden tab keeps its cached frame and draws nothing so its pages learn the switch from here
pub struct MainTabPageSelection {
    shown: Option<MainTab>,
}

impl MainTabPageSelection {
    pub fn new() -> Self {
        Self { shown: None }
    }

    pub fn shown(&self) -> Option<MainTab> {
        self.shown
    }

    pub fn set_shown(&mut self, tab: MainTab, cx: &mut Context<Self>) {
        if self.shown != Some(tab) {
            self.shown = Some(tab);
            cx.notify();
        }
    }
}

impl Default for MainTabPageSelection {
    fn default() -> Self {
        Self::new()
    }
}

/// the tab page a view is drawn in and the selection that shows or hides it
#[derive(Clone)]
pub struct MainTabPageHost {
    pub tab: MainTab,
    pub selection: Entity<MainTabPageSelection>,
}

impl MainTabPageHost {
    pub fn is_shown(&self, cx: &App) -> bool {
        self.selection.read(cx).shown() == Some(self.tab)
    }
}

/// set by each main tab page, none outside the tabs (pushed window or right panel host)
pub struct MainTabPageHostKey;

impl EnvKey for MainTabPageHostKey {
    type Value = Option<MainTabPageHost>;

    fn default_value() -> Self::Value {
        None
    }
}

/// reads the tab bar underlap, a host sets it with env_scope
pub fn main_tab_bar_underlap(cx: &App) -> f32 {
    if let Some(slot) = cx.try_global::<MainTabBarUnderlapSlot>() {
        let _ = slot.signal.read(cx);
    }
    env::<MainTabBarUnderlapKey>(cx)
}
