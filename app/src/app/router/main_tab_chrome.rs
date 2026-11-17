use gpui::{App, AppContext, Context, Entity, Global, Window};
use rise_ui::swiftui::environment::{EnvKey, env};

use crate::app::router::navigation_types::MainTab;
pub use crate::core::widgets::navigations::main_tab_chrome_metrics_hooks::{MAIN_TAB_BAR_HEIGHT, MainTabChrome};

pub enum MainTabChromeZLayer {}

impl MainTabChromeZLayer {
    pub const TAB_BAR: f64 = 1.0;
    pub const SELECTION_BAR: f64 = 2.0;
    pub const SHEET: f64 = 3.0;
    pub const MENU: f64 = 4.0;
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
