use gpui::{App, Context, Subscription, Window, px};
use rise_i18n::tr;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::swiftui::prelude::*;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::preferences::rise_launch_preferences::RiseLaunchPreferences;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

fn set_launch_animation_enabled(enabled: bool, cx: &mut App) {
    UserInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.set_launch_animation_enabled(enabled, cx));
}

struct LaunchAnimationSettingsState {
    _subscription: Subscription,
}

impl LaunchAnimationSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let preferences = RiseLaunchPreferences::shared(cx);
        Self { _subscription: cx.observe(&preferences, |_, _, cx| cx.notify()) }
    }
}

#[derive(IntoElement)]
pub struct LaunchAnimationSettings;

impl LaunchAnimationSettings {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for LaunchAnimationSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let _state = window.use_keyed_state("launch-animation-settings-state", cx, |_, cx| LaunchAnimationSettingsState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let preferences = RiseLaunchPreferences::shared(cx);
        let read_preferences = preferences.clone();
        let value = Binding::new(
            move |cx| read_preferences.read(cx).animation_enabled(),
            |enabled: bool, _, cx| set_launch_animation_enabled(enabled, cx),
        );

        let row = GroupedBtn::new("launch_animation", tr("settings_launch_animation"))
            .show_chevron(false)
            .trailing_slot(SwitchUi::new("launch-animation-switch", value))
            .callback(move |_, cx| {
                let enabled = preferences.read(cx).animation_enabled();
                set_launch_animation_enabled(!enabled, cx);
            });

        SettingsWrapperUi::new(tr("settings_launch_animation"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            scroll_view("launch-animation-settings-scroll")
                .size_full()
                .bg(theme.bg._100)
                .list_shell_scroll_under_chrome(cx)
                .child(div().w_full().pt(px(12.0)).child(GroupedBtns::new("launch-animation-group", vec![row]))),
        )
    }
}
