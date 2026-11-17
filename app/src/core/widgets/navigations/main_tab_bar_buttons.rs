use std::cell::RefCell;
use std::collections::HashSet;

use rise_canon::RiseTabBarButton;

use crate::app::router::navigation_types::MainTab;
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
