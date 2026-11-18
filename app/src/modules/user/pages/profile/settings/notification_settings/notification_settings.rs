use gpui::{App, Entity, FontWeight, SharedString, Subscription, Window};
use rise_canon::RiseGuideAnchor;
use rise_canon::rise_key::{notify, user};
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore, color};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::switch_ui::SwitchUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::widgets::notifier::notifier_store::NotifierBannerPosition;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::notify::stores::notify_actions::notify_actions::NotifyActionsStore;
use crate::modules::notify::stores::notify_actions::notify_settings_types::{
    NotificationCategory, NotificationSettingsModel, NotificationSettingsModelValues, canon_notification_flag,
};
use crate::modules::notify::stores::notify_interactions::notify_interactions::NotifyInteractionsStore;
use crate::modules::notify::stores::notify_services::notify_services::NotifyServicesStore;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const MUTE_ALL_ICON_BACKGROUND: &str = "#8E8E93";
const DISABLED_GROUP_OPACITY: f32 = 0.4;

fn interactions(cx: &mut App) -> Entity<NotifyInteractionsStore> {
    NotifyInteractionsStore::shared(cx)
}

/// dim and dead hit area of a whole group live on the wrapper, a disabled row would dim itself twice
pub(crate) fn notification_settings_group(enabled: bool, group: GroupedBtns) -> Div {
    div()
        .relative()
        .w_full()
        .opacity(if enabled { 1.0 } else { DISABLED_GROUP_OPACITY })
        .child(group)
        .when(!enabled, |wrapper| {
            wrapper.child(div().absolute().top_0().left_0().size_full().block_mouse_except_scroll())
        })
}

pub(crate) fn notification_settings_loading_state() -> Div {
    div().w_full().flex().justify_center().pt(px(80.0)).child(progress_view())
}

pub(crate) fn notification_settings_error_state(id: &'static str, theme: &AppTheme) -> Div {
    vstack(12.0)
        .w_full()
        .pt(px(60.0))
        .child(MainText::new(tr("retry_later_error")).px(14.0).color(theme.text.secondary))
        .child(
            plain(id)
                .on_press(|_, cx| interactions(cx).update(cx, |store, cx| store.reload_notification_settings(cx)))
                .child(
                    MainText::new(tr("privacy_settings_retry"))
                        .px(14.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.primary._100),
                ),
        )
}

struct NotificationSettingsState {
    _subscriptions: Vec<Subscription>,
}

impl NotificationSettingsState {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let services = NotifyServicesStore::shared(cx);
        let actions = NotifyActionsStore::shared(cx);
        let user_services = UserServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            _subscriptions: vec![
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&user_services, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
                cx.observe_window_activation(window, |_, window, cx| {
                    if window.is_window_active() {
                        NotifyInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.refresh_system_notification_permission(cx));
                    }
                }),
            ],
        }
    }
}

#[derive(IntoElement, Default)]
pub struct NotificationSettings;

impl NotificationSettings {
    pub fn new() -> Self {
        Self
    }

    fn master_binding(services: Entity<NotifyServicesStore>) -> Binding<bool> {
        Binding::new(
            move |cx: &App| services.read(cx).notification_settings(cx).all_enabled,
            |enabled, _, cx| interactions(cx).update(cx, |store, cx| store.set_all_notifications_enabled(enabled, cx)),
        )
    }

    fn category_binding(services: Entity<NotifyServicesStore>, category: NotificationCategory) -> Binding<bool> {
        Binding::new(
            move |cx: &App| services.read(cx).notification_settings(cx).value(category),
            move |enabled, _, cx| {
                interactions(cx).update(cx, |store, cx| store.set_notification_category(category, enabled, cx));
            },
        )
    }

    fn mute_all_binding(user_services: Entity<UserServicesStore>) -> Binding<bool> {
        Binding::new(
            move |cx: &App| user_services.read(cx).mute_all_chats_enabled(),
            |enabled, _, cx| {
                UserInteractionsStore::shared(cx).update(cx, |store, cx| store.set_mute_all_chats(enabled, cx));
            },
        )
    }

    fn mute_all_profiles_binding(services: Entity<NotifyServicesStore>) -> Binding<bool> {
        Binding::new(
            move |cx: &App| services.read(cx).notification_settings(cx).mute_all_profiles,
            |enabled, _, cx| interactions(cx).update(cx, |store, cx| store.set_mute_all_profiles(enabled, cx)),
        )
    }

    fn push_row(
        id: impl Into<SharedString>,
        icon: &'static str,
        background: gpui::Hsla,
        text: String,
        route: RootRoute,
        nav_store: Option<Entity<NavigationStore>>,
    ) -> GroupedBtn {
        GroupedBtn::new(id, text).icon(icon).icon_background(background).callback(move |_, cx| {
            if let Some(nav_store) = &nav_store {
                let route = route.clone();
                nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
            }
        })
    }

    fn content(
        settings: NotificationSettingsModel,
        system_denied: bool,
        mute_all_chats_enabled: bool,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) -> Div {
        let services = NotifyServicesStore::shared(cx);
        let user_services = UserServicesStore::shared(cx);
        let categories = NotificationCategory::ALL_CASES
            .iter()
            .copied()
            .map(|category| {
                GroupedBtn::new(category.wire_key(), tr(category.title_key()))
                    .icon(category.icon())
                    .icon_background(color(category.icon_hex()))
                    .trailing_slot(SwitchUi::new(
                        SharedString::from(format!("notification-category-switch-{}", category.wire_key())),
                        Self::category_binding(services.clone(), category),
                    ))
                    .callback(move |_, cx| {
                        if !settings.all_enabled {
                            return;
                        }
                        let enabled = !settings.value(category);
                        interactions(cx).update(cx, |store, cx| store.set_notification_category(category, enabled, cx));
                    })
            })
            .collect();
        let banner_position = NotifierBannerPosition::current(cx);
        let banner_positions = NotifierBannerPosition::ALL
            .into_iter()
            .map(|position| {
                GroupedBtn::new(position.raw_value(), tr(position.title_key()))
                    .selected(position == banner_position)
                    .callback(move |window, cx| {
                        NotifierBannerPosition::set(position, cx);
                        window.refresh();
                    })
            })
            .collect();
        let master_flag = canon_notification_flag(notify::ALL_ENABLED);
        let music_flag = canon_notification_flag(notify::MUSIC_ENABLED);
        let mute_all_profiles_flag = canon_notification_flag(notify::MUTE_ALL_PROFILES);

        vstack(16.0)
            .w_full()
            .pt(px(12.0))
            .when(system_denied, |column| {
                if rise_platform::notifications::can_open_notification_settings() {
                    column.child(
                        GroupedBtns::new(
                            "notification-system-group",
                            vec![
                                GroupedBtn::new("system_notifications_open_settings", tr("notification_settings_open_system"))
                                    .icon("gear")
                                    .icon_background(SettingsIconPalette::system_notifications())
                                    .callback(|_, cx| {
                                        interactions(cx).update(cx, |store, cx| store.open_system_notification_settings(cx));
                                    }),
                            ],
                        )
                        .end_group_title(tr("desktop_notification_settings_system_denied")),
                    )
                } else {
                    column.child(
                        div().w_full().px(px(16.0)).child(
                            SecondaryText::new(tr("desktop_notification_settings_system_denied")).px(13.0).tac(TextAlignT::Center),
                        ),
                    )
                }
            })
            .child(
                div()
                    .w_full()
                    .px(px(16.0))
                    .child(SecondaryText::new(tr("notification_settings_hint")).px(13.0).tac(TextAlignT::Center)),
            )
            .child(GroupedBtns::new(
                "notification-master-group",
                vec![
                    GroupedBtn::new("master", tr(master_flag.i18n_key))
                        .icon("bell.badge")
                        .icon_background(SettingsIconPalette::notifications())
                        .trailing_slot(SwitchUi::new("notification-master-switch", Self::master_binding(services.clone())))
                        .callback(move |_, cx| {
                            interactions(cx)
                                .update(cx, |store, cx| store.set_all_notifications_enabled(!settings.all_enabled, cx));
                        }),
                ],
            ))
            .child(notification_settings_group(
                settings.all_enabled,
                GroupedBtns::new("notification-categories-group", categories)
                    .title(tr("notification_settings_categories").to_uppercase()),
            ))
            .child(
                GroupedBtns::new("notification-banner-position-group", banner_positions)
                    .title(tr("desktop_notification_banner_section").to_uppercase())
                    .end_group_title(tr("desktop_notification_banner_footer")),
            )
            .child(GroupedBtns::new(
                "notification-language-group",
                vec![Self::push_row(
                    user::NOTIFICATION_LANGUAGE,
                    "globe",
                    SettingsIconPalette::language(),
                    tr("settings_notification_language_title"),
                    RootRoute::NotificationLanguageSettings,
                    nav_store.clone(),
                )],
            ))
            .child(GroupedBtns::new(
                "notification-music-group",
                vec![Self::push_row(
                    "music",
                    "music.note",
                    color(music_flag.icon_hex),
                    tr("notification_settings_music"),
                    RootRoute::MusicNotificationSettings,
                    nav_store,
                )],
            ))
            .child(
                GroupedBtns::new(
                    "notification-chats-group",
                    vec![
                        GroupedBtn::new(user::MUTE_ALL, tr("notification_settings_mute_all"))
                            .icon("bell.slash")
                            .icon_background(color(MUTE_ALL_ICON_BACKGROUND))
                            .guide_anchor(RiseGuideAnchor::NotificationSettingsMuteAll)
                            .trailing_slot(SwitchUi::new(
                                "notification-mute-all-switch",
                                Self::mute_all_binding(user_services),
                            ))
                            .callback(move |_, cx| {
                                UserInteractionsStore::shared(cx)
                                    .update(cx, |store, cx| store.set_mute_all_chats(!mute_all_chats_enabled, cx));
                            }),
                    ],
                )
                .title(tr("notification_section_chats").to_uppercase())
                .end_group_title(tr("notification_settings_mute_all_hint")),
            )
            .child(
                GroupedBtns::new(
                    "notification-profiles-group",
                    vec![
                        GroupedBtn::new(notify::MUTE_ALL_PROFILES, tr(mute_all_profiles_flag.i18n_key))
                            .icon("person.crop.circle.badge.xmark")
                            .icon_background(color(mute_all_profiles_flag.icon_hex))
                            .trailing_slot(SwitchUi::new(
                                "notification-mute-all-profiles-switch",
                                Self::mute_all_profiles_binding(services),
                            ))
                            .callback(move |_, cx| {
                                interactions(cx)
                                    .update(cx, |store, cx| store.set_mute_all_profiles(!settings.mute_all_profiles, cx));
                            }),
                    ],
                )
                .title(tr("notification_section_profiles").to_uppercase())
                .end_group_title(tr("notification_settings_mute_all_profiles_hint")),
            )
            .child(spacer_min_h(80.0))
    }
}

impl RenderOnce for NotificationSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        window.use_keyed_state("notification-settings-state", cx, NotificationSettingsState::new);
        let nav_store = env::<NavigationStoreKey>(cx);
        let back_store = nav_store.clone();
        let (failed, loading, settings, system_denied) = {
            let services = NotifyServicesStore::shared(cx);
            let services = services.read(cx);
            (
                services.notification_settings_failed(cx),
                services.notification_settings_loading(cx),
                services.notification_settings(cx),
                services.system_notifications_denied(cx),
            )
        };
        let mute_all_chats_enabled = UserServicesStore::shared(cx).read(cx).mute_all_chats_enabled();
        let body = if failed {
            notification_settings_error_state("notification-settings-retry", &theme)
        } else if loading {
            notification_settings_loading_state()
        } else {
            Self::content(settings, system_denied, mute_all_chats_enabled, nav_store, cx)
        };
        SettingsWrapperUi::new(tr("settings_notifications_title"), move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            scroll_view("notification-settings-scroll")
                .list_shell_scroll_under_chrome(cx)
                .bg(theme.bg._100)
                .child(body),
        )
        .on_appear("notification-settings-task", |_, cx| {
            interactions(cx).update(cx, |store, cx| {
                store.ensure_notification_settings_loaded(cx);
                store.refresh_system_notification_permission(cx);
            });
            UserInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_chat_mute_overrides_loaded(cx));
        })
        .on_disappear("notification-settings-flush", |_, cx| {
            interactions(cx).update(cx, |store, cx| store.flush_notification_settings(cx));
        })
    }
}
