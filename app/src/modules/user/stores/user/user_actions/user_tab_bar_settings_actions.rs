use gpui::{AsyncApp, Context};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::navigations::main_tab_bar_settings::{RiseTabBarPlatform, RiseTabBarSettingsChange};
use crate::modules::user::engine::core::rise_user_debug_trace::RiseUserDebugTrace;
use crate::modules::user::engine::rise_user_domain::RiseUserActionExecutor;
use crate::modules::user::engine::rise_user_tab_bar_presentation::RiseUserTabBarPresentation;
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;

impl UserActionsStore {
    pub fn load_tab_bar_settings_action(&mut self, force: bool, cx: &mut Context<Self>) {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| match domain.await {
            Ok(domain) => domain.load_tab_bar_settings(force).await,
            Err(error) => {
                let label = RiseUserDebugTrace::error_label(&error);
                cx.update(|cx| {
                    Sai::log("User", move || format!("tab bar settings load failed: {label}"), SaiLogKind::Network, cx);
                });
            }
        })
        .detach();
    }

    pub fn update_tab_bar_settings_action(
        &mut self,
        change: RiseTabBarSettingsChange,
        platform: RiseTabBarPlatform,
        sequence: u64,
        online: bool,
        cx: &mut Context<Self>,
    ) {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => domain.update_tab_bar_settings(change, platform, sequence, online).await,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                let label = RiseUserDebugTrace::error_label(&error);
                cx.update(|cx| {
                    RiseUserTabBarPresentation::shared(cx).update(cx, |presentation, cx| presentation.drop(sequence, cx));
                    Sai::log("User", move || format!("tab bar settings update failed: {label}"), SaiLogKind::Network, cx);
                });
            }
        })
        .detach();
    }
}
