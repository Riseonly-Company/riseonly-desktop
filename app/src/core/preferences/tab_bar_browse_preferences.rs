use gpui::{App, Context, Entity};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{TAB_BAR_RAIL_ORDER, TAB_BAR_SETTINGS_SLOT, TAB_BAR_SWIPE_BROWSE_ENABLED};

pub struct TabBarBrowsePreferences {
    swipe_browses: bool,
    rail_order: Option<Vec<String>>,
}

impl TabBarBrowsePreferences {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::new())
    }

    fn new() -> Self {
        let defaults = UserDefaults::standard();
        defaults.remove_object(TAB_BAR_SETTINGS_SLOT);
        let rail_order = defaults.string_array(TAB_BAR_RAIL_ORDER).filter(|order| !order.is_empty());
        Self { swipe_browses: defaults.bool(Self::BROWSE_KEY), rail_order }
    }

    const BROWSE_KEY: &'static str = TAB_BAR_SWIPE_BROWSE_ENABLED;

    pub fn swipe_browses(&self) -> bool {
        self.swipe_browses
    }

    /// saved places of the left bar top to bottom, button wires and settings
    pub fn rail_order(&self) -> Option<&[String]> {
        self.rail_order.as_deref()
    }

    pub fn set_rail_order(&mut self, order: Vec<String>, cx: &mut Context<Self>) {
        if self.rail_order.as_ref() == Some(&order) {
            return;
        }
        UserDefaults::standard().set(TAB_BAR_RAIL_ORDER, order.clone());
        self.rail_order = Some(order);
        cx.notify();
    }

    pub fn set_swipe_browses(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.swipe_browses == enabled {
            return;
        }
        self.swipe_browses = enabled;
        UserDefaults::standard().set(Self::BROWSE_KEY, enabled);
        cx.notify();
    }
}
