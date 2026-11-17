use std::rc::Rc;

use gpui::{App, Context, ElementId, Empty, Entity, FontWeight, ListAlignment, Pixels, Point, SharedString, Subscription, Task, Window, WeakEntity};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::icon_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_floating_top_inset,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::components::bot_ui_components::BotStateView;
use crate::modules::bot::shared::bot_ui_validation::BotMiniAppURLPolicy;
use crate::modules::bot::stores::bot::bot_actions::bot_actions::BotActionsStore;
use crate::modules::bot::stores::bot::bot_actions::bot_types::{BotMiniAppLaunchData, BotMiniAppModel};
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

const PREFETCH_THRESHOLD: usize = 8;
const CARD_RADIUS: f32 = 22.0;
const ICON_SIZE: f32 = 48.0;
const ICON_RADIUS: f32 = 13.0;
const CONTENT_BOTTOM_INSET: f32 = 90.0;

fn page_id(bot_id: &str, name: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("bot-mini-apps-{bot_id}-{name}")))
}

fn load_mini_apps(bot_id: &str, cx: &mut App) {
    BotInteractionsStore::shared(cx).update(cx, |store, cx| store.load_mini_apps(bot_id, cx));
}

fn refresh_mini_apps(bot_id: &str, cx: &mut App) {
    BotInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_mini_apps(bot_id, cx));
}

fn owns_practice_bot(bot_id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_practice_bot(bot_id))
}

/// bot mini apps view, the stack keeps it under an open mini app editor
/// launch_error and list position have to outlive that push
pub struct BotMiniApps {
    bot_id: String,
    launch_error: Option<String>,
    list: ListUi,
    row_keys: Vec<String>,
    _services_subscription: Subscription,
}

impl BotMiniApps {
    pub fn new(bot_id: String, cx: &mut Context<Self>) -> Self {
        let services = BotServicesStore::shared(cx);
        Self {
            bot_id,
            launch_error: None,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            row_keys: Vec::new(),
            _services_subscription: cx.observe(&services, |_, _, cx| cx.notify()),
        }
    }

    fn sync_list(&mut self, keys: Vec<String>) {
        if keys == self.row_keys {
            return;
        }
        let start = keys.iter().zip(&self.row_keys).take_while(|(new, old)| new == old).count();
        let tail_limit = keys.len().min(self.row_keys.len()) - start;
        let end = keys
            .iter()
            .rev()
            .zip(self.row_keys.iter().rev())
            .take(tail_limit)
            .take_while(|(new, old)| new == old)
            .count();
        self.list.state().splice(start..self.row_keys.len() - end, keys.len() - start - end);
        self.row_keys = keys;
    }

    fn resolved_launch_error(&self, cx: &mut App) -> Option<String> {
        self.launch_error.clone().or_else(|| BotActionsStore::shared(cx).read(cx).create_mini_app_launch_data_sai.error())
    }
}

impl Render for BotMiniApps {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let bot_id = self.bot_id.clone();
        let nav_store = env::<NavigationStoreKey>(cx);

        let task_bot_id = bot_id.clone();
        use_task(page_id(&bot_id, "task"), (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| load_mini_apps(&task_bot_id, cx));
        });

        let resolved_launch_error = self.resolved_launch_error(cx);
        let alert_page = this.downgrade();
        on_change(
            page_id(&bot_id, "launch-error"),
            resolved_launch_error,
            true,
            window,
            cx,
            move |_: &Option<String>, error: &Option<String>, _: &mut Window, cx: &mut App| {
                let Some(error) = error else {
                    return;
                };
                NativeConfirmAlert::present_notice(
                    &tr("bot_mini_app_launch_error"),
                    error,
                    None,
                    None,
                    move |cx: &mut App| {
                        let _ = alert_page.update(cx, |page, cx| {
                            page.launch_error = None;
                            cx.notify();
                        });
                        BotInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_sensitive_results(cx));
                    },
                    cx,
                );
            },
        );

        let parts = BotMiniAppsParts { bot_id: bot_id.clone(), page: this.clone(), nav_store: nav_store.clone() };
        let back_store = nav_store;
        let create_parts = parts.clone();
        let keys_bot_id = bot_id.clone();
        let shell = SettingsWrapperUi::new(tr("bot_mini_apps_title"), move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .right_jsx(
            PageHeaderIconButton::new(page_id(&bot_id, "create"), "plus", move |_: &mut Window, cx: &mut App| {
                create_parts.push_editor(None, cx);
            })
            .guide_anchor(RiseGuideAnchor::BotMiniAppsCreate),
        )
        .content(BotMiniAppsBody { parts });

        PageRefreshKeys::new(
            page_id(&bot_id, "keys"),
            move |_: &mut Window, cx: &mut App| refresh_mini_apps(&keys_bot_id, cx),
            shell,
        )
    }
}

/// what the rows and menus of the screen need, each holds its own copy
#[derive(Clone)]
struct BotMiniAppsParts {
    bot_id: String,
    page: Entity<BotMiniApps>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl BotMiniAppsParts {
    fn push_editor(&self, mini_app_id: Option<String>, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            let route = RootRoute::BotMiniAppEditor { bot_id: self.bot_id.clone(), mini_app_id };
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn mini_app_actions(mini_app: &BotMiniAppModel) -> Vec<HoldContextMenuAction> {
        let mut actions = vec![
            HoldContextMenuAction::new("open", tr("bot_mini_app_open"))
                .system_image("play.fill")
                .disabled(!mini_app.is_active),
            HoldContextMenuAction::new("edit", tr("bot_mini_app_edit")).system_image("pencil"),
        ];
        if !mini_app.development_url.is_empty() {
            actions.push(
                HoldContextMenuAction::new("development", tr("bot_mini_app_dev_preview")).system_image("hammer.fill"),
            );
        }
        actions.push(HoldContextMenuAction::new("delete", tr("bot_mini_app_delete")).system_image("trash").destructive(true));
        actions
    }

    fn handle_action(&self, action: &str, mini_app: &BotMiniAppModel, cx: &mut App) {
        match action {
            "open" => self.launch(mini_app, false, cx),
            "edit" => self.push_editor(Some(mini_app.id.clone()), cx),
            "development" => self.launch(mini_app, true, cx),
            "delete" => self.confirm_delete(mini_app, cx),
            _ => {}
        }
    }

    fn confirm_delete(&self, mini_app: &BotMiniAppModel, cx: &mut App) {
        let bot_id = self.bot_id.clone();
        let mini_app_id = mini_app.id.clone();
        NativeConfirmAlert::present_delete(
            &tr("bot_mini_app_delete"),
            &tr("bot_mini_app_delete_message"),
            Some(tr("bot_mini_app_delete")),
            None,
            false,
            None,
            None,
            move |cx: &mut App| {
                BotInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.delete_mini_app(&bot_id, &mini_app_id, None, cx));
            },
            cx,
        );
    }

    fn launch(&self, mini_app: &BotMiniAppModel, development: bool, cx: &mut App) {
        self.page.update(cx, |page, cx| {
            page.launch_error = None;
            cx.notify();
        });
        let page: WeakEntity<BotMiniApps> = self.page.downgrade();
        let nav_store = self.nav_store.clone();
        let title = mini_app.title.clone();
        let bot_id = self.bot_id.clone();
        let route_bot_id = bot_id.clone();
        let mini_app_id = mini_app.id.clone();
        BotInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.create_mini_app_launch_data(
                &bot_id,
                &mini_app_id,
                None,
                None,
                development,
                None,
                Some(Rc::new(move |launch: BotMiniAppLaunchData, cx: &mut App| {
                    let url = Url::from_string(&launch.launch_url)
                        .filter(|url| BotMiniAppURLPolicy::allows(url, development));
                    let Some(url) = url else {
                        let _ = page.update(cx, |page, cx| {
                            page.launch_error = Some(tr("bot_mini_app_url_blocked"));
                            cx.notify();
                        });
                        return;
                    };
                    if let Some(nav_store) = &nav_store {
                        let route = RootRoute::BotMiniApp {
                            url,
                            title: title.clone(),
                            allows_development_http: development,
                            chat_id: None,
                            bot_id: Some(route_bot_id.clone()),
                            is_web_link: false,
                        };
                        nav_store.update(cx, |nav_store, cx| {
                            nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                        });
                    }
                })),
                None,
                cx,
            );
        });
    }

    fn mini_app_icon(mini_app: &BotMiniAppModel, theme: &AppTheme) -> impl IntoElement + use<> {
        let placeholder = div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.primary._100.opacity(0.14))
            .children(IconUi::sized("square.grid.2x2.fill", px(19.0), theme.primary._100, true));
        let has_image = !mini_app.icon_url.is_empty() && Url::from_string(&mini_app.icon_url).is_some();
        div()
            .relative()
            .flex_shrink_0()
            .size(px(ICON_SIZE))
            .rounded(px(ICON_RADIUS))
            .overflow_hidden()
            .child(placeholder)
            .when(has_image, |icon| {
                icon.child(
                    div().absolute().inset_0().child(
                        CleverImageUi::new(SharedString::from(mini_app.icon_url.clone()))
                            .id(SharedString::from(format!("bot-mini-app-icon-{}", mini_app.id)))
                            .content_mode(ContentMode::Fill)
                            .shows_progress_indicator(false)
                            .shows_failure_icon(false)
                            .placeholder_color(gpui::transparent_black()),
                    ),
                )
            })
    }

    fn mini_app_row_content(mini_app: &BotMiniAppModel, theme: &AppTheme) -> AnyElement {
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(13.0))
            .px(px(16.0))
            .py(px(12.0))
            .bg(theme.bg._200)
            .child(Self::mini_app_icon(mini_app, theme))
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(4.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                MainText::new(mini_app.title.clone())
                                    .px(16.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .number_of_lines(1),
                            )
                            .when(!mini_app.is_active, |title| {
                                title.child(
                                    div().px(px(6.0)).py(px(2.0)).bg(theme.bg._300).rounded_full().child(
                                        MainText::new(tr("bot_mini_app_inactive")).px(10.0).color(theme.text.secondary),
                                    ),
                                )
                            }),
                    )
                    .child(SecondaryText::new(mini_app.short_name.clone()).px(13.0).line_limit(1))
                    .when(!mini_app.description.is_empty(), |texts| {
                        texts.child(SecondaryText::new(mini_app.description.clone()).px(12.0).line_limit(1))
                    }),
            )
            .child(div().flex_1().min_w(px(4.0)))
            .children(IconUi::sized("chevron.right", px(12.0), theme.text.secondary.opacity(0.7), false))
            .into_any_element()
    }

    fn mini_app_row_container(&self, mini_app: &BotMiniAppModel, theme: &AppTheme) -> AnyElement {
        let actions = Self::mini_app_actions(mini_app);
        let content_mini_app = mini_app.clone();
        let content_theme = theme.clone();
        let action_parts = self.clone();
        let action_mini_app = mini_app.clone();
        let tap_parts = self.clone();
        let tap_mini_app = mini_app.clone();
        HoldContextMenuContainer::new(
            SharedString::from(format!("bot-mini-app-hold-{}", mini_app.id)),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| {
                Self::mini_app_row_content(&content_mini_app, &content_theme)
            }),
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            action_parts.handle_action(&id, &action_mini_app, cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            if tap_mini_app.is_active {
                tap_parts.launch(&tap_mini_app, false, cx);
            }
        }))
        .into_any_element()
    }

    fn list_card_row(content: AnyElement, is_first: bool, is_last: bool, top_inset: f32, theme: &AppTheme) -> AnyElement {
        let card = div().w_full().bg(theme.bg._200).overflow_hidden().child(content);
        let card = if is_first { card.rounded_t(px(CARD_RADIUS)) } else { card };
        let card = if is_last { card.rounded_b(px(CARD_RADIUS)) } else { card };
        div()
            .w_full()
            .px(px(12.0))
            .when(is_first, |row| row.pt(px(top_inset + 12.0)))
            .when(is_last, |row| row.pb(px(CONTENT_BOTTOM_INSET)))
            .child(card)
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct BotMiniAppsBody {
    parts: BotMiniAppsParts,
}

impl RenderOnce for BotMiniAppsBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let parts = self.parts;
        let bot_id = parts.bot_id.clone();
        let mini_apps = BotServicesStore::mini_apps(&bot_id, cx);
        let status = if mini_apps.is_empty() && !owns_practice_bot(&bot_id, cx) {
            BotActionsStore::shared(cx).read(cx).list_mini_apps_sai.status()
        } else {
            SaiWsSliceStatus::Fulfilled
        };
        let slice_error = BotActionsStore::shared(cx).read(cx).list_mini_apps_sai.error();
        let is_empty = mini_apps.is_empty();
        let loading_id = page_id(&bot_id, "loading");
        let retry_bot_id = bot_id.clone();
        let error_id = page_id(&bot_id, "error");
        let error_message = slice_error.clone();
        let empty_parts = parts.clone();
        let empty_id = page_id(&bot_id, "empty");
        let render_id = page_id(&bot_id, "render");

        SaiDataRender::new(status, move |_, cx| {
            let has_more = BotServicesStore::mini_apps_has_more(&bot_id, cx);
            let relative_id = BotServicesStore::mini_apps_relative_id(&bot_id, cx);
            let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
            let cache_key = BotServicesStore::mini_apps_cache_key(&user_id, &bot_id);
            let item_count = mini_apps.len();
            let mut keys: Vec<String> = mini_apps.iter().map(|mini_app| format!("mini-app-{}", mini_app.id)).collect();
            if has_more {
                keys.push("loading-more".to_owned());
            }
            parts.page.update(cx, |page, _| page.sync_list(keys));

            let theme = rise_ui::theme(cx).clone();
            let top_inset = list_shell_floating_top_inset(cx);
            let refresh_bot_id = bot_id.clone();
            let screen = parts.page.clone();
            let mini_apps = Rc::new(mini_apps);
            let row_theme = theme.clone();
            let list = screen.read(cx).list.refreshable(
                page_id(&bot_id, "list"),
                move |_, cx| {
                    refresh_mini_apps(&refresh_bot_id, cx);
                    Task::ready(())
                },
                move |index, _, _| {
                    let theme = &row_theme;
                    let Some(mini_app) = mini_apps.get(index) else {
                        if has_more && index == item_count {
                            return BotMiniAppsParts::list_card_row(
                                div()
                                    .w_full()
                                    .flex()
                                    .justify_center()
                                    .py(px(20.0))
                                    .child(progress_view())
                                    .into_any_element(),
                                item_count == 0,
                                true,
                                top_inset,
                                theme,
                            );
                        }
                        return Empty.into_any_element();
                    };
                    let load_bot_id = parts.bot_id.clone();
                    let row = parts
                        .mini_app_row_container(mini_app, theme)
                        .guide_anchor((index == 0).then_some(RiseGuideAnchor::BotMiniAppsFirst))
                        .sai_pagination_prefetch(
                            SharedString::from(format!("bot-mini-app-prefetch-{}", mini_app.id)),
                            SaiPaginationPrefetchOptions::new(
                                index,
                                item_count,
                                cache_key.clone(),
                                relative_id.clone(),
                                has_more,
                            )
                            .threshold(PREFETCH_THRESHOLD),
                            move |_, cx| {
                                BotInteractionsStore::shared(cx)
                                    .update(cx, |store, cx| store.load_more_mini_apps(&load_bot_id, cx));
                            },
                        );
                    let is_last_mini_app = index + 1 == item_count;
                    let content = div()
                        .w_full()
                        .child(row)
                        .when(!is_last_mini_app, |content| {
                            content.child(div().h(px(0.5)).ml(px(76.0)).bg(theme.border._200.opacity(0.55)))
                        })
                        .into_any_element();
                    BotMiniAppsParts::list_card_row(content, index == 0, is_last_mini_app && !has_more, top_inset, theme)
                },
            );
            div().size_full().bg(theme.bg._100).child(list)
        })
        .id(render_id)
        .is_empty(is_empty)
        .no_data_text(tr("bot_mini_apps_empty"))
        .error_text(slice_error.or_else(|| Some(tr("bot_mini_apps_load_error"))))
        .sai_data_render_loading(move |_, cx| {
            let theme = rise_ui::theme(cx).clone();
            scroll_view(loading_id.clone())
                .size_full()
                .scroll_disabled(true)
                .shows_indicators(false)
                .list_shell_scroll_under_chrome(cx)
                .bg(theme.bg._100)
                .child(
                    div().px(px(12.0)).pt(px(12.0)).pb(px(CONTENT_BOTTOM_INSET)).child(
                        div()
                            .bg(theme.bg._200)
                            .rounded(px(CARD_RADIUS))
                            .overflow_hidden()
                            .child(UserListSkeleton::new().rows(12)),
                    ),
                )
        })
        .sai_data_render_error(move |_, _| {
            let retry_bot_id = retry_bot_id.clone();
            BotStateView::new(
                error_id.clone(),
                "exclamationmark.arrow.triangle.2.circlepath",
                tr("bot_mini_apps_load_error"),
            )
            .message(error_message.clone())
            .action_title(tr("bot_retry"))
            .action(move |_: &mut Window, cx: &mut App| load_mini_apps(&retry_bot_id, cx))
        })
        .sai_data_render_empty(move |_, _| {
            let empty_parts = empty_parts.clone();
            BotStateView::new(empty_id.clone(), "square.grid.2x2.fill", tr("bot_mini_apps_empty"))
                .message(tr("bot_mini_apps_empty_hint"))
                .action_title(tr("bot_mini_app_create"))
                .action(move |_: &mut Window, cx: &mut App| empty_parts.push_editor(None, cx))
        })
    }
}
