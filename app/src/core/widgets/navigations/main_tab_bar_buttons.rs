use std::cell::RefCell;
use std::collections::HashSet;

use gpui::App;
use rise_canon::RiseTabBarButton;

use crate::app::router::navigation_types::MainTab;
use crate::core::preferences::tab_bar_browse_preferences::TabBarBrowsePreferences;
use crate::core::widgets::navigations::main_tab_bar::MainTabBarItem;
use crate::core::widgets::navigations::main_tab_bar_settings::RiseTabBarButtonCodable;

/// saved button order uses wire values, the bar draws MainTabBarItems
pub trait RiseTabBarButtonExt {
    fn tab(&self) -> MainTab;
    fn item(&self) -> Option<MainTabBarItem>;
}

impl RiseTabBarButtonExt for RiseTabBarButton {
    fn tab(&self) -> MainTab {
        match self {
            RiseTabBarButton::Posts => MainTab::Posts,
            RiseTabBarButton::Search => MainTab::GlobalSearch,
            RiseTabBarButton::Chats => MainTab::Chats,
            RiseTabBarButton::Riffs => MainTab::Shorts,
            RiseTabBarButton::Profile => MainTab::Profile,
            RiseTabBarButton::Music => MainTab::Music,
            RiseTabBarButton::Map => MainTab::Maps,
            RiseTabBarButton::Vacancies => MainTab::Vacancies,
        }
    }

    fn item(&self) -> Option<MainTabBarItem> {
        let tab = self.tab();
        MainTabBarItem::all().iter().find(|item| item.id == tab).copied()
    }
}

thread_local! {
    static ORDERED_CACHE: RefCell<Option<(Vec<RiseTabBarButton>, Vec<MainTabBarItem>)>> = const { RefCell::new(None) };
}

impl MainTabBarItem {
    /// bar buttons in the reader order
    /// a button missing from the order goes last so an old saved order never hides one
    pub fn ordered(order: &[RiseTabBarButton]) -> Vec<MainTabBarItem> {
        let cached = ORDERED_CACHE.with_borrow(|cache| {
            cache.as_ref().filter(|(cached_order, _)| cached_order.as_slice() == order).map(|(_, items)| items.clone())
        });
        if let Some(items) = cached {
            return items;
        }
        let named: Vec<MainTabBarItem> =
            RiseTabBarButton::normalized(order).iter().filter_map(RiseTabBarButtonExt::item).collect();
        let shown: HashSet<MainTab> = named.iter().map(|item| item.id).collect();
        let mut items = named;
        items.extend(MainTabBarItem::all().iter().filter(|item| !shown.contains(&item.id)).copied());
        ORDERED_CACHE.with_borrow_mut(|cache| *cache = Some((order.to_vec(), items.clone())));
        items
    }
}

/// one place of the left bar, a tab button or the desktop settings button
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RailEntry {
    Button(RiseTabBarButton),
    Settings,
}

impl RailEntry {
    pub const FOOT_SIZE: usize = 2;
    const SETTINGS_WIRE: &'static str = "settings";

    /// bar places top to bottom from the saved order, buttons it misses go above the foot
    /// with nothing saved yet profile and settings stay last as the bar always had them
    pub fn rail(saved: Option<&[String]>, buttons: &[RiseTabBarButton]) -> Vec<Self> {
        let mut entries: Vec<Self> = Vec::new();
        for wire in saved.unwrap_or_default() {
            let entry = if wire == Self::SETTINGS_WIRE {
                Some(Self::Settings)
            } else {
                RiseTabBarButton::from_wire(wire).filter(|button| buttons.contains(button)).map(Self::Button)
            };
            if let Some(entry) = entry.filter(|entry| !entries.contains(entry)) {
                entries.push(entry);
            }
        }
        if entries.is_empty() {
            entries.extend(
                buttons.iter().copied().filter(|button| *button != RiseTabBarButton::Profile).map(Self::Button),
            );
            if buttons.contains(&RiseTabBarButton::Profile) {
                entries.push(Self::Button(RiseTabBarButton::Profile));
            }
            entries.push(Self::Settings);
            return entries;
        }
        if !entries.contains(&Self::Settings) {
            entries.push(Self::Settings);
        }
        for button in buttons {
            if !entries.contains(&Self::Button(*button)) {
                entries.insert(Self::foot_start(&entries), Self::Button(*button));
            }
        }
        entries
    }

    pub fn saved(button_order: &[RiseTabBarButton], cx: &mut App) -> Vec<Self> {
        let buttons: Vec<RiseTabBarButton> = MainTabBarItem::ordered(button_order)
            .iter()
            .filter_map(|item| RiseTabBarButton::ALL.into_iter().find(|button| button.tab() == item.id))
            .collect();
        Self::rail(TabBarBrowsePreferences::shared(cx).read(cx).rail_order(), &buttons)
    }

    /// keeps a new order of the bar places, every bar and list showing it redraws
    pub fn save(order: &[Self], cx: &mut App) {
        let wires = Self::wires(order);
        TabBarBrowsePreferences::shared(cx).update(cx, |preferences, cx| preferences.set_rail_order(wires, cx));
        cx.refresh_windows();
    }

    pub fn wires(entries: &[Self]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                Self::Button(button) => button.wire().to_owned(),
                Self::Settings => Self::SETTINGS_WIRE.to_owned(),
            })
            .collect()
    }

    /// first entry of the bar foot, always the last two places
    pub fn foot_start(entries: &[Self]) -> usize {
        entries.len().saturating_sub(Self::FOOT_SIZE)
    }
}
