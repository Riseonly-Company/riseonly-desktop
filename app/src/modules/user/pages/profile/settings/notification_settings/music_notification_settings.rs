use gpui::{App, Entity, SharedString, Subscription, Window};
use rise_canon::rise_key::notify;
use rise_i18n::tr;
use rise_theme::{ThemeStore, color};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::TextAlignT;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::switch_ui::SwitchUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use super::notification_settings::{
    notification_settings_error_state, notification_settings_group, notification_settings_loading_state,
};
use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::notify::stores::notify_actions::notify_actions::NotifyActionsStore;
use crate::modules::notify::stores::notify_actions::notify_settings_types::{
    MusicNotificationSetting, NotificationSettingsModel, NotificationSettingsModelValues, canon_notification_flag,
};
use crate::modules::notify::stores::notify_interactions::notify_interactions::NotifyInteractionsStore;
use crate::modules::notify::stores::notify_services::notify_services::NotifyServicesStore;

fn interactions(cx: &mut App) -> Entity<NotifyInteractionsStore> {
    NotifyInteractionsStore::shared(cx)
}

struct MusicNotificationSettingsState {
    _subscriptions: Vec<Subscription>,
}

impl MusicNotificationSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = NotifyServicesStore::shared(cx);
        let actions = NotifyActionsStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            _subscriptions: vec![
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement, Default)]
pub struct MusicNotificationSettings;

impl MusicNotificationSettings {
    pub fn new() -> Self {
        Self
    }

    fn music_master_binding(services: Entity<NotifyServicesStore>) -> Binding<bool> {
        Binding::new(
            move |cx: &App| services.read(cx).notification_settings(cx).music_enabled,
            |enabled, _, cx| interactions(cx).update(cx, |store, cx| store.set_music_notifications_enabled(enabled, cx)),
        )
    }

    fn binding(services: Entity<NotifyServicesStore>, setting: MusicNotificationSetting) -> Binding<bool> {
        Binding::new(
            move |cx: &App| services.read(cx).notification_settings(cx).value(setting),
            move |enabled, _, cx| {
                interactions(cx).update(cx, |store, cx| store.set_music_notification_setting(setting, enabled, cx));
            },
        )
    }

    fn rows(
        settings: &[MusicNotificationSetting],
        enabled: bool,
        services: &Entity<NotifyServicesStore>,
    ) -> Vec<GroupedBtn> {
        settings
            .iter()
            .copied()
            .map(|setting| {
                let reader = services.clone();
                GroupedBtn::new(setting.wire_key(), tr(setting.title_key()))
                    .icon(setting.icon())
                    .icon_background(color(setting.icon_hex()))
                    .trailing_slot(SwitchUi::new(
                        SharedString::from(format!("music-notification-switch-{}", setting.wire_key())),
                        Self::binding(services.clone(), setting),
                    ))
                    .callback(move |_, cx| {
                        if !enabled {
                            return;
                        }
                        let current = reader.read(cx).notification_settings(cx).value(setting);
                        interactions(cx)
                            .update(cx, |store, cx| store.set_music_notification_setting(setting, !current, cx));
                    })
            })
            .collect()
    }

    fn content(settings: NotificationSettingsModel, services: Entity<NotifyServicesStore>) -> Div {
        let music_enabled = settings.all_enabled && settings.music_enabled;
        let music_flag = canon_notification_flag(notify::MUSIC_ENABLED);
        vstack(16.0)
            .w_full()
            .pt(px(12.0))
            .child(
                div()
                    .w_full()
                    .px(px(16.0))
                    .child(SecondaryText::new(tr("notification_settings_music_hint")).px(13.0).tac(TextAlignT::Center)),
            )
            .child(notification_settings_group(
                settings.all_enabled,
                GroupedBtns::new(
                    "music-notification-master-group",
                    vec![
                        GroupedBtn::new("music_master", tr(music_flag.i18n_key))
                            .icon("music.note")
                            .icon_background(color(music_flag.icon_hex))
                            .trailing_slot(SwitchUi::new(
                                "music-notification-master-switch",
                                Self::music_master_binding(services.clone()),
                            ))
                            .callback(move |_, cx| {
                                if !settings.all_enabled {
                                    return;
                                }
                                interactions(cx).update(cx, |store, cx| {
                                    store.set_music_notifications_enabled(!settings.music_enabled, cx);
                                });
                            }),
                    ],
                ),
            ))
            .child(notification_settings_group(
                music_enabled,
                GroupedBtns::new(
                    "music-notification-sessions-group",
                    Self::rows(MusicNotificationSetting::session_cases(), music_enabled, &services),
                )
                .title(tr("notification_settings_music_sessions").to_uppercase()),
            ))
            .child(notification_settings_group(
                music_enabled,
                GroupedBtns::new(
                    "music-notification-general-group",
                    Self::rows(MusicNotificationSetting::general_cases(), music_enabled, &services),
                )
                .title(tr("notification_settings_music_general").to_uppercase()),
            ))
            .child(spacer_min_h(80.0))
    }
}

impl RenderOnce for MusicNotificationSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        window.use_keyed_state("music-notification-settings-state", cx, |_, cx| {
            MusicNotificationSettingsState::new(cx)
        });
        let nav_store = env::<NavigationStoreKey>(cx);
        let services = NotifyServicesStore::shared(cx);
        let (failed, loading, settings) = {
            let services = services.read(cx);
            (
                services.notification_settings_failed(cx),
                services.notification_settings_loading(cx),
                services.notification_settings(cx),
            )
        };
        let body = if failed {
            notification_settings_error_state("music-notification-settings-retry", &theme)
        } else if loading {
            notification_settings_loading_state()
        } else {
            Self::content(settings, services)
        };
        SettingsWrapperUi::new(tr("notification_settings_music"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            scroll_view("music-notification-settings-scroll")
                .list_shell_scroll_under_chrome(cx)
                .bg(theme.bg._100)
                .child(body),
        )
        .on_appear("music-notification-settings-task", |_, cx| {
            interactions(cx).update(cx, |store, cx| store.ensure_notification_settings_loaded(cx));
        })
        .on_disappear("music-notification-settings-flush", |_, cx| {
            interactions(cx).update(cx, |store, cx| store.flush_notification_settings(cx));
        })
    }
}
