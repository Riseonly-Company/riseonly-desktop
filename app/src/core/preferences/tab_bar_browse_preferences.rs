use gpui::{App, Context, Entity};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::TAB_BAR_SWIPE_BROWSE_ENABLED;

pub struct TabBarBrowsePreferences {
    swipe_browses: bool,
}

impl TabBarBrowsePreferences {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::new())
    }

    fn new() -> Self {
        Self {
            swipe_browses: UserDefaults::standard().bool(Self::BROWSE_KEY),
        }
    }

    const BROWSE_KEY: &'static str = TAB_BAR_SWIPE_BROWSE_ENABLED;

    pub fn swipe_browses(&self) -> bool {
        self.swipe_browses
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
