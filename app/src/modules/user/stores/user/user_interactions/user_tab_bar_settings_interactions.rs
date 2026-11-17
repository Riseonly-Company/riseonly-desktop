use gpui::Context;

use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::widgets::navigations::main_tab_bar_settings::{
    RiseTabBarPlatform, RiseTabBarSettings, RiseTabBarSettingsChange,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::engine::rise_user_tab_bar_presentation::RiseUserTabBarPresentation;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

impl UserInteractionsStore {
    pub fn load_tab_bar_settings(&mut self, force: bool, cx: &mut Context<Self>) {
        if !AuthServicesStore::shared(cx).read(cx).is_authenticated() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.load_tab_bar_settings_action(force, cx));
    }

    pub fn set_tab_bar_settings(
        &mut self,
        settings: RiseTabBarSettings,
        platform: RiseTabBarPlatform,
        cx: &mut Context<Self>,
    ) {
        self.change_tab_bar_settings(RiseTabBarSettingsChange::Set(settings.clamped()), platform, cx);
    }

    pub fn reset_tab_bar_settings(&mut self, platform: RiseTabBarPlatform, cx: &mut Context<Self>) {
        self.change_tab_bar_settings(RiseTabBarSettingsChange::Reset, platform, cx);
    }

    fn change_tab_bar_settings(
        &mut self,
        change: RiseTabBarSettingsChange,
        platform: RiseTabBarPlatform,
        cx: &mut Context<Self>,
    ) {
        if !AuthServicesStore::shared(cx).read(cx).is_authenticated() {
            return;
        }
        let presentation = RiseUserTabBarPresentation::shared(cx);
        if presentation.read(cx).settings()[platform] == change.settings() && change != RiseTabBarSettingsChange::Reset {
            return;
        }
        let sequence = presentation.update(cx, |presentation, cx| presentation.take(change, platform, cx));
        let online = RiseTransportStatus::is_connected(cx);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_tab_bar_settings_action(change, platform, sequence, online, cx);
        });
    }
}
