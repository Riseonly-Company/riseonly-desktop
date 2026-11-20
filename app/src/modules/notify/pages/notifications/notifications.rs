use gpui::{App, Entity, SharedString, Subscription, Window};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::animated_tabs_ui::animated_tabs_types::{TabConfig, TabHeaderConfig};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::ui::animated_tabs_ui::animated_tabs::AnimatedTabs;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::AnimatedTabsPagerLiveState;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellUi, list_shell_floating_top_inset};
use crate::modules::notify::components::all_notifications::{AllNotifications, CommentNotifications};
use crate::modules::notify::engine::rise_notifications_presentation::RiseNotificationsPresentation;
use crate::modules::notify::stores::notify_interactions::notify_interactions::NotifyInteractionsStore;
use crate::modules::notify::stores::notify_services::notify_services::NotifyServicesStore;

/// state of the notifications page, active tab and store
struct NotificationsState {
    active_tab: i64,
    neighbours_mounted: bool,
    tabs_live_state: Entity<AnimatedTabsPagerLiveState>,
    _subscriptions: Vec<Subscription>,
}

impl NotificationsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let store = NotifyServicesStore::shared(cx);
        let presentation = RiseNotificationsPresentation::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            active_tab: 0,
            neighbours_mounted: false,
            tabs_live_state: cx.new(|_| AnimatedTabsPagerLiveState::new()),
            _subscriptions: vec![
                cx.observe(&store, |_, _, cx| cx.notify()),
                cx.observe(&presentation, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement, Default)]
pub struct Notifications {
    shows_back_button: bool,
}

impl Notifications {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shows_back_button(mut self, shows_back_button: bool) -> Self {
        self.shows_back_button = shows_back_button;
        self
    }
}

impl RenderOnce for Notifications {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("notifications-state", cx, |_, cx| NotificationsState::new(cx));
        let neighbours_mounted = state.read(cx).neighbours_mounted;
        let active_tab = Binding::entity(state.clone(), |state| state.active_tab, |state, tab, _| {
            state.active_tab = tab;
            state.neighbours_mounted = true;
        });
        let store = NotifyServicesStore::shared(cx);
        let has_unread = store.read(cx).has_unread(cx);
        let tabs_live_state = state.read(cx).tabs_live_state.clone();
        let list_max_width = rise_ui::theme(cx).shell.list_max_width;
        let tabs_strip = div().w_full().flex().flex_row().justify_center().child(
            div().w_full().max_w(list_max_width).child(AnimatedTabs::header_strip(
                "notifications-tabs-header",
                vec![
                    TabHeaderConfig::new(0, tr("all_notify_title")),
                    TabHeaderConfig::new(1, tr("comment_notify_title")),
                ],
                active_tab.clone(),
                tabs_live_state.clone(),
                10.0,
            )),
        );

        let shell = ListShellUi::new()
            .title(SharedString::from(tr("settings_notify_title")))
            .shows_back_button(self.shows_back_button)
            .on_back(move |_, cx| {
                if let Some(nav_store) = &nav_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .below_header(tabs_strip);
        let shell = if has_unread {
            shell.right_jsx(
                PageHeaderIconButton::new("notifications-mark-all-read", "checkmark.circle", |_, cx| {
                    NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.mark_all_read(cx));
                })
                .font_size(16.0),
            )
        } else {
            shell
        };
        shell.content(NotificationsBody { state, active_tab, neighbours_mounted, tabs_live_state })
    }
}

#[derive(IntoElement)]
struct NotificationsBody {
    state: Entity<NotificationsState>,
    active_tab: Binding<i64>,
    neighbours_mounted: bool,
    tabs_live_state: Entity<AnimatedTabsPagerLiveState>,
}

impl RenderOnce for NotificationsBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let list_shell_top_inset = list_shell_floating_top_inset(cx);
        let active_tab = self.active_tab.get(cx);
        let neighbours_mounted = self.neighbours_mounted;
        let tabs = vec![
            TabConfig::new(0, tr("all_notify_title"), move |_, _| {
                if active_tab != 0 && !neighbours_mounted {
                    return div().into_any_element();
                }
                AllNotifications::new(active_tab == 0).into_any_element()
            }),
            TabConfig::new(1, tr("comment_notify_title"), move |_, _| {
                if active_tab != 1 && !neighbours_mounted {
                    return div().into_any_element();
                }
                CommentNotifications::new(active_tab == 1).into_any_element()
            }),
        ];

        let generation = RiseNotificationsPresentation::shared(cx).read(cx).generation();
        use_task("notifications-observe", generation, window, cx, async move |cx| {
            let observation = cx.update(|_, cx| {
                NotifyInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.observe_notifications(Uuid::new_v4(), cx))
            });
            if let Ok(observation) = observation {
                observation.await;
            }
        });

        let state = self.state.clone();
        div()
            .id("notifications-body")
            .size_full()
            .flex()
            .flex_row()
            .justify_center()
            .wheel_into_children()
            .bg(theme.bg._100)
            .when(!neighbours_mounted, |body| {
                body.on_hover(move |hovered, _, cx| {
                    if *hovered {
                        state.update(cx, |state, cx| {
                            if !state.neighbours_mounted {
                                state.neighbours_mounted = true;
                                cx.notify();
                            }
                        });
                    }
                })
            })
            .child(
                vstack(0.0)
                    .h_full()
                    .w_full()
                    .max_w(theme.shell.list_max_width)
                    .pt(px(list_shell_top_inset))
                    .child(
                        AnimatedTabs::new("notifications-tabs", tabs, self.active_tab)
                            .horizontal_padding(10.0)
                            .detached_header(self.tabs_live_state),
                    ),
            )
    }
}
