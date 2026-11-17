use gpui::App;

use crate::core::widgets::navigations::main_tab_bar_settings::RiseTabBarSettingsSet;
use crate::modules::user::engine::rise_user_tab_bar_presentation::RiseUserTabBarPresentation;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

impl UserServicesStore {
    pub fn tab_bar_settings(&self, cx: &mut App) -> RiseTabBarSettingsSet {
        RiseUserTabBarPresentation::shared(cx).read(cx).settings()
    }

    pub fn tab_bar_settings_synced(&self, cx: &mut App) -> bool {
        RiseUserTabBarPresentation::shared(cx).read(cx).is_synced()
    }
}
