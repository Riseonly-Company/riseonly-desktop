use std::time::Duration;

use gpui::{App, ElementId, Entity, FontWeight, RenderOnce, Window};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_i18n::tr;
use rise_theme::{AppTheme, color};
use rise_ui::bottom_sheet_ui::bottom_sheet_screen::{
    BottomSheetNavigationState, BottomSheetScreenItem, bottom_sheet_navigation,
};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::MainText;
use rise_ui::skeleton_ui::SkeletonBlock;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::switch_ui::SwitchRowUi;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_pages_ui::BottomSheetPagesUi;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_profile_more_sheets::ChatNotificationScopesContent;
use crate::modules::chat::shared::chat_notifications_debounce::ChatNotificationsDebounce;
pub(crate) use crate::modules::notify::components::notifications_hub_sheet_store::NotificationsHubSheetStore;
use crate::modules::notify::stores::notify_actions::notify_profile_settings_wire_types::{
    ProfileNotificationScope, ProfileNotificationSettingsModel,
};
use crate::modules::notify::stores::notify_interactions::notify_interactions::NotifyInteractionsStore;
use crate::modules::notify::stores::notify_services::notify_services::NotifyServicesStore;

#[derive(IntoElement, Default)]
pub struct NotificationsHubSheetHost;

impl NotificationsHubSheetHost {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for NotificationsHubSheetHost {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = NotificationsHubSheetStore::shared(cx);
        div().size_0().when(store.read(cx).mounts(), |host| host.child(NotificationsHubSheet::new()))
    }
}

/// last laid out heights of a fitted page
/// the page area under the header and its content
#[derive(Clone, Copy, Default)]
struct FittedHeights {
    room: f32,
    content: f32,
    room_generation: u64,
}

#[derive(Clone)]
struct FittedPage {
    fraction: Entity<f32>,
    heights: Entity<FittedHeights>,
}

struct FittedIds {
    content: &'static str,
    room: &'static str,
    scroll: Option<&'static str>,
}

#[derive(IntoElement, Default)]
pub struct NotificationsHubSheet;

impl NotificationsHubSheet {
    const MENU_PAGE_FRACTION: f32 = 0.34;
    const CHAT_PAGE_FRACTION: f32 = 0.62;
    const PAGE_HEADER_HEIGHT: f32 = 64.0;
    const MINIMUM_FITTED_FRACTION: f32 = 0.12;
    const MAXIMUM_FITTED_FRACTION: f32 = 0.86;
    const REFIT_GATE: f32 = 48.0;
    const REFIT_TOLERANCE: f32 = 0.5;
    const REFIT_SETTLE: Duration = Duration::from_millis(120);

    pub fn new() -> Self {
        Self
    }

    fn fitted_fraction(content_height: f32, window: &Window) -> f32 {
        let viewport = f32::from(window.viewport_size().height).max(1.0);
        ((content_height + Self::PAGE_HEADER_HEIGHT) / viewport)
            .clamp(Self::MINIMUM_FITTED_FRACTION, Self::MAXIMUM_FITTED_FRACTION)
    }

    /// a small room correction waits until the page height stops springing
    fn refit(
        fraction: &Entity<f32>,
        heights: &Entity<FittedHeights>,
        room_settled: bool,
        window: &Window,
        cx: &mut App,
    ) {
        let FittedHeights { room, content, .. } = *heights.read(cx);
        if content <= 0.0 {
            return;
        }
        let viewport = f32::from(window.viewport_size().height).max(1.0);
        let current = *fraction.read(cx);
        let difference = content - room;
        let target = if room > 0.0 && difference.abs() < Self::REFIT_GATE {
            if difference.abs() <= Self::REFIT_TOLERANCE || !room_settled {
                return;
            }
            (current + difference / viewport).clamp(Self::MINIMUM_FITTED_FRACTION, Self::MAXIMUM_FITTED_FRACTION)
        } else {
            Self::fitted_fraction(content, window)
        };
        fraction.update(cx, |fraction, cx| {
            if (target - *fraction).abs() > f32::EPSILON {
                *fraction = target;
                cx.notify();
            }
        });
    }

    /// reports the laid out height of content and its page area into the page fraction
    /// scroll_id puts the content in a scroll view filling that area
    fn fitted(
        ids: FittedIds,
        fraction: &Entity<f32>,
        heights: &Entity<FittedHeights>,
        content: impl IntoElement,
    ) -> Div {
        let content_fraction = fraction.clone();
        let content_heights = heights.clone();
        let measured = div().relative().w_full().child(content).child(measure(ids.content, move |bounds, window, cx| {
            content_heights.update(cx, |heights, _| heights.content = f32::from(bounds.size.height));
            Self::refit(&content_fraction, &content_heights, false, window, cx);
        }));
        let room_fraction = fraction.clone();
        let room_heights = heights.clone();
        let room = measure(ids.room, move |bounds, window, cx| {
            let generation = room_heights.update(cx, |heights, _| {
                heights.room = f32::from(bounds.size.height);
                heights.room_generation = heights.room_generation.wrapping_add(1);
                heights.room_generation
            });
            Self::refit(&room_fraction, &room_heights, false, window, cx);
            let (settle_fraction, settle_heights) = (room_fraction.clone(), room_heights.clone());
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor().timer(Self::REFIT_SETTLE).await;
                    let _ = cx.update(|window, cx| {
                        if settle_heights.read(cx).room_generation == generation {
                            Self::refit(&settle_fraction, &settle_heights, true, window, cx);
                        }
                    });
                })
                .detach();
        });
        let body = match ids.scroll {
            Some(scroll_id) => scroll_view(scroll_id).size_full().child(measured).into_any_element(),
            None => measured.into_any_element(),
        };
        div().relative().size_full().child(body).child(room)
    }

    fn pages(
        store: &NotificationsHubSheetStore,
        theme: &AppTheme,
        menu: &FittedPage,
        chat: &FittedPage,
        cx: &App,
    ) -> Vec<BottomSheetScreenItem> {
        let mut pages = Vec::new();
        if store.chat_id().is_some() && store.target_user_id().is_some() {
            let menu_theme = theme.clone();
            let menu = menu.clone();
            pages.push(BottomSheetScreenItem::new(
                "menu",
                tr("ctx_notifications"),
                *menu.fraction.read(cx),
                move |_, _| {
                    Self::fitted(
                        FittedIds {
                            content: "notifications-hub-menu-height",
                            room: "notifications-hub-menu-room",
                            scroll: None,
                        },
                        &menu.fraction,
                        &menu.heights,
                        NotificationsHubMenuView::new(menu_theme.clone()),
                    )
                },
            ));
        }
        if let Some(chat_id) = store.chat_id() {
            let chat_id = chat_id.to_owned();
            let current = store.chat_scopes().cloned();
            let chat = chat.clone();
            pages.push(BottomSheetScreenItem::new(
                "chat",
                tr("notifications_hub_chat"),
                *chat.fraction.read(cx),
                move |_, _| {
                    Self::fitted(
                        FittedIds {
                            content: "notifications-hub-chat-height",
                            room: "notifications-hub-chat-room",
                            scroll: Some("notifications-hub-chat-scroll"),
                        },
                        &chat.fraction,
                        &chat.heights,
                        div().w_full().p(px(16.0)).child(ChatNotificationScopesContent::new(chat_id.clone(), current)),
                    )
                },
            ));
        }
        if let Some(target_user_id) = store.target_user_id() {
            let target_user_id = target_user_id.to_owned();
            pages.push(BottomSheetScreenItem::new("profile", tr("notifications_hub_profile"), 0.9, move |_, _| {
                scroll_view("notifications-hub-profile-scroll").size_full().child(
                    div().w_full().p(px(16.0)).child(ProfileNotificationScopesContent::new(target_user_id.clone())),
                )
            }));
        }
        pages
    }

    fn flush_pending(cx: &mut App) {
        let store = NotificationsHubSheetStore::shared(cx);
        let chat_id = store.read(cx).chat_id().map(str::to_owned);
        let target_user_id = store.read(cx).target_user_id().map(str::to_owned);
        if let Some(chat_id) = chat_id {
            SaiDebouncer::flush(chat_id, ChatNotificationsDebounce::GROUP_KEY, cx);
        }
        if let Some(target_user_id) = target_user_id {
            NotifyInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.flush_profile_notification_settings(&target_user_id, cx));
        }
    }
}

impl RenderOnce for NotificationsHubSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let store = NotificationsHubSheetStore::shared(cx);
        let menu = FittedPage {
            fraction: window.use_keyed_state("notifications-hub-menu-fraction", cx, |_, _| Self::MENU_PAGE_FRACTION),
            heights: window.use_keyed_state("notifications-hub-menu-heights", cx, |_, _| FittedHeights::default()),
        };
        let chat = FittedPage {
            fraction: window.use_keyed_state("notifications-hub-chat-fraction", cx, |_, _| Self::CHAT_PAGE_FRACTION),
            heights: window.use_keyed_state("notifications-hub-chat-heights", cx, |_, _| FittedHeights::default()),
        };
        let pages = Self::pages(store.read(cx), &theme, &menu, &chat, cx);
        let root_page_id = store.read(cx).root_page_id();
        let dismissed_store = store.clone();
        BottomSheetPagesUi::new(
            "notifications-hub-sheet",
            Binding::entity(
                store,
                |store: &NotificationsHubSheetStore| store.is_presented(),
                |store: &mut NotificationsHubSheetStore, value, cx| store.set_is_presented(value, cx),
            ),
            pages,
            root_page_id,
        )
        .sheet_background(Some(theme.bg._100))
        .modal_sized_by_detent(true)
        .on_dismiss(|_, cx| Self::flush_pending(cx))
        .on_did_dismiss(move |_, cx| dismissed_store.update(cx, |store, cx| store.did_dismiss(cx)))
    }
}

#[derive(IntoElement)]
struct NotificationsHubMenuView {
    theme: AppTheme,
}

impl NotificationsHubMenuView {
    fn new(theme: AppTheme) -> Self {
        Self { theme }
    }
}

impl RenderOnce for NotificationsHubMenuView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let chat_navigation = bottom_sheet_navigation(cx);
        let profile_navigation = chat_navigation.clone();
        div().w_full().px(px(15.0)).pt(px(4.0)).pb(px(8.0)).child(GroupedBtns::new(
            "notifications-hub-menu",
            vec![
                GroupedBtn::new("chat", tr("notifications_hub_chat"))
                    .icon("bubble.left.and.bubble.right.fill")
                    .icon_background(self.theme.primary._100)
                    .subtitle(tr("notifications_hub_chat_subtitle"))
                    .callback(move |window, cx| {
                        if let Some(navigation) = &chat_navigation {
                            BottomSheetNavigationState::navigate(navigation, "chat", window, cx);
                        }
                    }),
                GroupedBtn::new("profile", tr("notifications_hub_profile"))
                    .icon("person.crop.circle.fill")
                    .icon_background(color("#E056FD"))
                    .subtitle(tr("notifications_hub_profile_subtitle"))
                    .guide_anchor(RiseGuideAnchor::NotificationsHubProfile)
                    .callback(move |window, cx| {
                        if let Some(navigation) = &profile_navigation {
                            BottomSheetNavigationState::navigate(navigation, "profile", window, cx);
                        }
                    }),
            ],
        ))
    }
}

#[derive(IntoElement)]
pub struct ProfileNotificationScopesContent {
    target_user_id: String,
}

impl ProfileNotificationScopesContent {
    pub fn new(target_user_id: impl Into<String>) -> Self {
        Self { target_user_id: target_user_id.into() }
    }

    fn master_binding(target_user_id: &str, model: &ProfileNotificationSettingsModel) -> Binding<bool> {
        let value = model.is_all_enabled();
        let target_user_id = target_user_id.to_owned();
        Binding::new(move |_| value, move |enabled, _, cx| {
            NotifyInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.set_all_profile_notifications(&target_user_id, enabled, cx));
        })
    }

    fn scope_binding(
        target_user_id: &str,
        scope: ProfileNotificationScope,
        model: &ProfileNotificationSettingsModel,
    ) -> Binding<bool> {
        let value = model.value(scope);
        let target_user_id = target_user_id.to_owned();
        Binding::new(move |_| value, move |enabled, _, cx| {
            NotifyInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.set_profile_notification_scope(&target_user_id, scope, enabled, cx));
        })
    }

    fn row(id: ElementId, title: String, is_on: Binding<bool>) -> SwitchRowUi {
        SwitchRowUi::new(id, title, is_on)
    }

    fn divider(theme: &AppTheme) -> Div {
        div().w_full().pl(px(16.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)))
    }

    fn scopes_card(
        target_user_id: &str,
        scopes: &[ProfileNotificationScope],
        model: &ProfileNotificationSettingsModel,
        theme: &AppTheme,
    ) -> Div {
        let mut card = vstack(0.0).w_full().bg(theme.bg._200).continuous_corners(16.0).overflow_hidden();
        for (index, scope) in scopes.iter().enumerate() {
            if index > 0 {
                card = card.child(Self::divider(theme));
            }
            card = card.child(Self::row(
                ElementId::Name(format!("profile-notify-scope-{}", scope.id()).into()),
                scope.title(),
                Self::scope_binding(target_user_id, *scope, model),
            ));
        }
        card
    }

    fn skeleton() -> Div {
        vstack(16.0)
            .w_full()
            .child(SkeletonBlock::new(None::<f32>, 46.0, 16.0))
            .child(SkeletonBlock::new(None::<f32>, 7.0 * 46.0, 16.0))
            .child(SkeletonBlock::new(None::<f32>, 2.0 * 46.0, 16.0))
    }

    fn error_state(target_user_id: &str, theme: &AppTheme) -> Div {
        let target_user_id = target_user_id.to_owned();
        vstack(12.0)
            .w_full()
            .items_center()
            .pt(px(24.0))
            .child(MainText::new(tr("retry_later_error")).px(14.0).color(theme.text.secondary))
            .child(
                plain("profile-notify-retry")
                    .on_press(move |_, cx| {
                        NotifyInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.reload_profile_notification_settings(&target_user_id, cx));
                    })
                    .child(
                        MainText::new(tr("privacy_settings_retry"))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    ),
            )
    }
}

impl RenderOnce for ProfileNotificationScopesContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let target_user_id = self.target_user_id;
        let notify_service = NotifyServicesStore::shared(cx);
        let model = notify_service.update(cx, |service, cx| service.profile_notification_settings(&target_user_id, cx));
        let failed = notify_service.read(cx).profile_notification_settings_failed(cx);

        on_change(
            ElementId::Name(format!("profile-notify-task-{target_user_id}").into()),
            target_user_id.clone(),
            true,
            window,
            cx,
            |_, target_user_id, _, cx| {
                NotifyInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.ensure_profile_notification_settings_loaded(target_user_id, cx));
            },
        );

        let body = match &model {
            Some(model) => vstack(16.0)
                .w_full()
                .child(
                    vstack(0.0).w_full().bg(theme.bg._200).continuous_corners(16.0).overflow_hidden().child(Self::row(
                        "profile-notify-master".into(),
                        tr("chat_mute_master_all"),
                        Self::master_binding(&target_user_id, model),
                    )),
                )
                .child(
                    Self::scopes_card(&target_user_id, &ProfileNotificationScope::REGULAR, model, &theme)
                        .guide_anchor(RiseGuideAnchor::PersonNotificationScopes),
                )
                .child(Self::scopes_card(&target_user_id, &ProfileNotificationScope::WATCH, model, &theme))
                .child(
                    div()
                        .px(px(4.0))
                        .child(MainText::new(tr("profile_notify_scopes_hint")).px(13.0).color(theme.text.secondary)),
                ),
            None if failed => Self::error_state(&target_user_id, &theme),
            None => Self::skeleton(),
        };

        let disappear_user_id = target_user_id;
        body.on_appear("profile-notify-appear", |_, cx| {
            GuideFacts::set(RiseGuideSignal::PersonNotificationsOpen, true, cx);
        })
        .on_disappear("profile-notify-disappear", move |_, cx| {
            GuideFacts::set(RiseGuideSignal::PersonNotificationsOpen, false, cx);
            NotifyInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.flush_profile_notification_settings(&disappear_user_id, cx));
        })
    }
}
