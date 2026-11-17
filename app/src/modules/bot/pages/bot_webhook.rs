use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context, Entity, Hsla, SharedString, Subscription, Task, Window};
use rise_bot_api::UpdateType as RiseBotUpdateType;
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::app_date_formatter::AppDateFormatter;
use rise_i18n::date_format::date_formatter::Style as DateFormatterStyle;
use rise_i18n::tr;
use rise_limits::service_limit::BOT_MAX_CONNECTIONS;
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, color};
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::switch_ui::SwitchUi;
use rise_ui::swiftui::lifecycle::on_change;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::stepper::Stepper;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::bot::components::bot_ui_components::{BotSettingsFieldRow, BotStateView, BotToggleRow, bot_symbol_is_solid};
use crate::modules::bot::shared::bot_ui_validation::{BotFieldRules, BotFormValidation};
use crate::modules::bot::stores::bot::bot_actions::bot_actions::BotActionsStore;
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotWebhookInfoModel;
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::bot::stores::bot::bot_services::bot_guide_sandbox_hooks::bot_guide_sandbox_hooks;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;

const DEFAULT_MAX_CONNECTIONS: i64 = 40;
const MIN_MAX_CONNECTIONS: i64 = 1;
const SECTION_RADIUS: f32 = 22.0;
const ORANGE: &str = "#FF9500";
const BLUE: &str = "#007AFF";
const INDIGO: &str = "#5856D6";
const DROP_RED: &str = "#FF3B30";
const HTTP_ERROR_PREFIX: &str = "Webhook returned HTTP ";
const TIMEOUT_ERROR: &str = "Webhook request timed out";
const CONNECT_ERROR: &str = "Webhook connection failed";
const CODE_PLACEHOLDER: &str = "{code}";

fn max_connections_limit() -> i64 {
    BOT_MAX_CONNECTIONS as i64
}

/// page state and its stores
struct BotWebhookState {
    bot_id: String,
    url: String,
    secret_token: String,
    max_connections: i64,
    allowed_updates: HashSet<String>,
    drop_pending_updates: bool,
    did_hydrate: bool,
    _subscriptions: Vec<Subscription>,
}

impl BotWebhookState {
    fn new(bot_id: String, cx: &mut Context<Self>) -> Self {
        let services = BotServicesStore::shared(cx);
        let actions = BotActionsStore::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        Self {
            bot_id,
            url: String::new(),
            secret_token: String::new(),
            max_connections: DEFAULT_MAX_CONNECTIONS,
            allowed_updates: HashSet::new(),
            drop_pending_updates: false,
            did_hydrate: false,
            _subscriptions: vec![
                cx.observe(&services, |state, _, cx| {
                    state.hydrate_if_possible(cx);
                    cx.notify();
                }),
                cx.observe(&actions, |state, _, cx| {
                    state.hydrate_if_possible(cx);
                    cx.notify();
                }),
                cx.observe(&cache_revisions, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn can_save(&self) -> bool {
        BotFormValidation::is_valid_webhook_url(&self.url) && BotFormValidation::is_valid_webhook_secret(&self.secret_token)
    }

    fn hydrate_if_possible(&mut self, cx: &mut Context<Self>) {
        if self.did_hydrate {
            return;
        }
        let fulfilled = matches!(
            BotActionsStore::shared(cx).read(cx).get_bot_webhook_info_sai.status(),
            SaiWsSliceStatus::Fulfilled
        );
        if !fulfilled && !(bot_guide_sandbox_hooks().owns_practice_bot)(&self.bot_id, cx) {
            return;
        }
        self.did_hydrate = true;
        cx.notify();
        let Some(webhook) = BotWebhook::webhook(&self.bot_id, cx) else {
            return;
        };
        self.url = webhook.url;
        let max_connections =
            if webhook.max_connections == 0 { DEFAULT_MAX_CONNECTIONS } else { webhook.max_connections };
        self.max_connections = max_connections.clamp(MIN_MAX_CONNECTIONS, max_connections_limit());
        self.allowed_updates = webhook.allowed_updates.into_iter().collect();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.did_hydrate = false;
        cx.notify();
        let bot_id = self.bot_id.clone();
        BotInteractionsStore::shared(cx).update(cx, |store, cx| store.load_webhook(&bot_id, cx));
    }

    fn save(state: &Entity<Self>, cx: &mut App) {
        let (bot_id, url, secret_token, max_connections, allowed_updates, drop_pending_updates) = {
            let state = state.read(cx);
            if !state.can_save() {
                return;
            }
            let mut allowed_updates: Vec<String> = state.allowed_updates.iter().cloned().collect();
            allowed_updates.sort();
            (
                state.bot_id.clone(),
                trimming_whitespaces_and_newlines(&state.url).to_owned(),
                state.secret_token.clone(),
                state.max_connections,
                allowed_updates,
                state.drop_pending_updates,
            )
        };
        BotInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.set_webhook(
                &bot_id,
                &url,
                &secret_token,
                max_connections,
                allowed_updates,
                drop_pending_updates,
                Some(Rc::new(|_: Option<BotWebhookInfoModel>, cx: &mut App| {
                    Notifier::send(
                        tr("bot_changes_saved"),
                        NotifierSendOptions {
                            r#type: NotifierType::Success,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                })),
                cx,
            );
        });
    }
}

#[derive(IntoElement)]
pub struct BotWebhook {
    bot_id: String,
}

impl BotWebhook {
    pub fn new(bot_id: impl Into<String>) -> Self {
        Self { bot_id: bot_id.into() }
    }

    fn webhook(bot_id: &str, cx: &mut App) -> Option<BotWebhookInfoModel> {
        BotServicesStore::webhook(bot_id, cx)
            .filter(|webhook| !trimming_whitespaces_and_newlines(&webhook.url).is_empty())
    }

    fn status_section(webhook: &BotWebhookInfoModel, theme: &AppTheme) -> GroupedBtns {
        let has_no_error = webhook.last_error_message.is_empty();
        let mut error = GroupedBtn::new(
            "error",
            if has_no_error { tr("bot_webhook_no_errors") } else { Self::delivery_error_text(&webhook.last_error_message) },
        )
        .icon(if has_no_error { "checkmark.circle.fill" } else { "exclamationmark.triangle.fill" })
        .icon_background(if has_no_error { UIColor::system_green() } else { UIColor::system_red() }.resolved_color(theme.appearance))
        .show_chevron(false);
        if webhook.last_error_date > 0 {
            error = error.subtitle(Self::formatted_date(webhook.last_error_date));
        }
        GroupedBtns::new(
            "bot-webhook-status",
            vec![
                GroupedBtn::new("status", tr(if webhook.enabled { "bot_webhook_active" } else { "bot_webhook_inactive" }))
                    .icon(if webhook.enabled { "checkmark.circle.fill" } else { "pause.circle.fill" })
                    .icon_background(if webhook.enabled { UIColor::system_green().resolved_color(theme.appearance) } else { theme.text.secondary })
                    .show_chevron(false),
                GroupedBtn::new("pending", tr("bot_webhook_pending_updates"))
                    .icon("tray.full.fill")
                    .icon_background(color(ORANGE))
                    .trailing(webhook.pending_update_count.to_string())
                    .show_chevron(false),
                error,
            ],
        )
        .title(tr("bot_webhook_status"))
        .corner_radius(SECTION_RADIUS)
    }

    fn endpoint_section(state: &Entity<BotWebhookState>, can_save: bool, theme: &AppTheme) -> GroupedBtns {
        let trailing_tint: Hsla = if can_save { UIColor::system_green().resolved_color(theme.appearance) } else { theme.semantic.error_100 };
        let trailing_icon = if can_save { "checkmark.circle.fill" } else { "lock.trianglebadge.exclamationmark.fill" };
        GroupedBtns::new(
            "bot-webhook-endpoint",
            vec![
                GroupedBtn::new("url", "").content_slot(
                    BotSettingsFieldRow::new(
                        "bot-webhook-url",
                        "link",
                        color(BLUE),
                        tr("bot_webhook_url"),
                        Binding::entity(
                            state.clone(),
                            |state: &BotWebhookState| state.url.clone(),
                            |state: &mut BotWebhookState, url, cx| {
                                state.url = url;
                                cx.notify();
                            },
                        ),
                    )
                    .trailing(div().flex_none().children(IconUi::sized(trailing_icon, px(17.0), trailing_tint, false))),
                ),
                GroupedBtn::new("secret", "").content_slot(
                    BotSettingsFieldRow::new(
                        "bot-webhook-secret",
                        "key.fill",
                        color(INDIGO),
                        tr("bot_webhook_secret"),
                        Binding::entity(
                            state.clone(),
                            |state: &BotWebhookState| state.secret_token.clone(),
                            |state: &mut BotWebhookState, secret_token: String, cx| {
                                state.secret_token = BotFieldRules::webhook_secret().clamped(&secret_token);
                                cx.notify();
                            },
                        ),
                    )
                    .is_secure(true),
                ),
            ],
        )
        .title(tr("bot_webhook_endpoint"))
        .end_group_title(tr("bot_webhook_https_hint"))
        .corner_radius(SECTION_RADIUS)
    }

    fn allowed_updates_section(state: &Entity<BotWebhookState>, theme: &AppTheme) -> GroupedBtns {
        let items = RiseBotUpdateType::ALL
            .into_iter()
            .map(|update| {
                let wire = update.wire();
                let icon = Self::update_icon(wire);
                GroupedBtn::new(wire, "").content_slot(
                    hstack(12.0)
                        .w_full()
                        .items_center()
                        .px(px(16.0))
                        .py(px(10.0))
                        .child(
                            Shape::continuous(8.0)
                                .round(div())
                                .flex_none()
                                .size(px(29.0))
                                .bg(theme.primary._100.opacity(0.12))
                                .flex()
                                .items_center()
                                .justify_center()
                                .children(IconUi::sized(icon, px(15.0), theme.primary._100, bot_symbol_is_solid(icon))),
                        )
                        .child(MainText::new(tr(&format!("bot_update_{wire}"))).px(14.0))
                        .child(spacer_min_w(0.0))
                        .child(SwitchUi::new(
                            SharedString::from(format!("bot-webhook-update-{wire}")),
                            Binding::entity(
                                state.clone(),
                                move |state: &BotWebhookState| state.allowed_updates.contains(wire),
                                move |state: &mut BotWebhookState, enabled: bool, cx| {
                                    if enabled {
                                        state.allowed_updates.insert(wire.to_owned());
                                    } else {
                                        state.allowed_updates.remove(wire);
                                    }
                                    cx.notify();
                                },
                            ),
                        )),
                )
            })
            .collect();
        GroupedBtns::new("bot-webhook-updates", items)
            .title(tr("bot_webhook_allowed_updates"))
            .corner_radius(SECTION_RADIUS)
    }

    fn delivery_section(state: &Entity<BotWebhookState>, theme: &AppTheme, cx: &App) -> GroupedBtns {
        let max_connections = state.read(cx).max_connections;
        GroupedBtns::new(
            "bot-webhook-delivery",
            vec![
                GroupedBtn::new("connections", "").content_slot(
                    hstack(12.0)
                        .w_full()
                        .items_center()
                        .px(px(16.0))
                        .py(px(10.0))
                        .children(IconUi::sized("arrow.triangle.branch", px(17.0), theme.primary._100, false))
                        .child(MainText::new(tr("bot_webhook_max_connections")).px(14.0))
                        .child(spacer_min_w(0.0))
                        .child(MainText::new(max_connections.to_string()))
                        .child(Stepper::new(
                            "bot-webhook-max-connections",
                            Binding::entity(
                                state.clone(),
                                |state: &BotWebhookState| state.max_connections,
                                |state: &mut BotWebhookState, max_connections, cx| {
                                    state.max_connections = max_connections;
                                    cx.notify();
                                },
                            ),
                            MIN_MAX_CONNECTIONS..=max_connections_limit(),
                        )),
                ),
                GroupedBtn::new("drop", "").content_slot(BotToggleRow::new(
                    "bot-webhook-drop",
                    "trash.slash.fill",
                    color(DROP_RED),
                    tr("bot_webhook_drop_pending"),
                    tr("bot_webhook_drop_pending_hint"),
                    Binding::entity(
                        state.clone(),
                        |state: &BotWebhookState| state.drop_pending_updates,
                        |state: &mut BotWebhookState, drop_pending_updates, cx| {
                            state.drop_pending_updates = drop_pending_updates;
                            cx.notify();
                        },
                    ),
                )),
            ],
        )
        .title(tr("bot_webhook_delivery"))
        .corner_radius(SECTION_RADIUS)
    }

    fn present_delete_confirm(state: &Entity<BotWebhookState>, cx: &mut App) {
        let state = state.clone();
        NativeConfirmAlert::present_delete(
            &tr("bot_webhook_delete"),
            &tr("bot_webhook_delete_message"),
            Some(tr("bot_webhook_delete")),
            None,
            false,
            None,
            None,
            move |cx| {
                let (bot_id, drop_pending_updates) = {
                    let state = state.read(cx);
                    (state.bot_id.clone(), state.drop_pending_updates)
                };
                BotInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.delete_webhook(
                        &bot_id,
                        drop_pending_updates,
                        Some(Rc::new(move |cx: &mut App| {
                            state.update(cx, |state, cx| {
                                state.url.clear();
                                state.secret_token.clear();
                                cx.notify();
                            });
                        })),
                        cx,
                    );
                });
            },
            cx,
        );
    }

    fn update_icon(update: &str) -> &'static str {
        match RiseBotUpdateType::parse(update) {
            Some(RiseBotUpdateType::Message | RiseBotUpdateType::EditedMessage) => "bubble.left.fill",
            Some(RiseBotUpdateType::InlineQuery) => "text.magnifyingglass",
            Some(RiseBotUpdateType::CallbackQuery) => "hand.tap.fill",
            Some(RiseBotUpdateType::ChatMember | RiseBotUpdateType::MyChatMember) => "person.2.fill",
            Some(RiseBotUpdateType::DeletedMessage) | None => "bell.fill",
        }
    }

    fn delivery_error_text(message: &str) -> String {
        if let Some(code) = message.strip_prefix(HTTP_ERROR_PREFIX).and_then(|code| code.parse::<i64>().ok()) {
            return tr("bot_webhook_error_http").replace(CODE_PLACEHOLDER, &code.to_string());
        }
        match message {
            TIMEOUT_ERROR => tr("bot_webhook_error_timeout"),
            CONNECT_ERROR => tr("bot_webhook_error_connect"),
            _ => tr("bot_webhook_error_failed"),
        }
    }

    fn formatted_date(milliseconds: i64) -> String {
        let mut formatter = AppDateFormatter::make(None, None, None);
        formatter.set_date_style(DateFormatterStyle::Medium);
        formatter.set_time_style(DateFormatterStyle::Short);
        formatter.string_from(&Date::from_ms(milliseconds))
    }

    fn content(state: &Entity<BotWebhookState>, theme: &AppTheme, cx: &mut App) -> impl IntoElement + use<> {
        let (bot_id, did_hydrate, can_save) = {
            let state = state.read(cx);
            (state.bot_id.clone(), state.did_hydrate, state.can_save())
        };
        let webhook = Self::webhook(&bot_id, cx);
        let is_enabled = webhook.as_ref().is_some_and(|webhook| webhook.enabled);
        let error = {
            let actions = BotActionsStore::shared(cx);
            let actions = actions.read(cx);
            actions.set_bot_webhook_sai.error().or_else(|| actions.delete_bot_webhook_sai.error())
        };
        let refresh_state = state.clone();
        let delete_state = state.clone();

        scroll_view("bot-webhook-scroll")
            .refreshable(move |_, cx| {
                refresh_state.update(cx, |state, cx| state.reload(cx));
                Task::ready(())
            })
            .size_full()
            .bg(theme.bg._100)
            .list_shell_scroll_under_chrome(cx)
            .child(
                vstack(20.0)
                    .w_full()
                    .pt(px(12.0))
                    .when_some(webhook.as_ref(), |column, webhook| {
                        column.child(
                            div()
                                .w_full()
                                .child(Self::status_section(webhook, theme))
                                .guide_anchor(RiseGuideAnchor::BotWebhookStatus),
                        )
                    })
                    .when(webhook.is_none() && did_hydrate, |column| {
                        column.child(
                            div().w_full().h(px(180.0)).child(
                                BotStateView::new(
                                    "bot-webhook-not-configured",
                                    "network.slash",
                                    tr("bot_webhook_not_configured"),
                                )
                                .message(tr("bot_webhook_not_configured_hint")),
                            ),
                        )
                    })
                    .child(
                        div()
                            .w_full()
                            .child(Self::endpoint_section(state, can_save, theme))
                            .guide_anchor(RiseGuideAnchor::BotWebhookEndpoint),
                    )
                    .child(
                        div()
                            .w_full()
                            .child(Self::allowed_updates_section(state, theme))
                            .guide_anchor(RiseGuideAnchor::BotWebhookUpdates),
                    )
                    .child(Self::delivery_section(state, theme, cx))
                    .when(is_enabled, |column| {
                        column.child(
                            GroupedBtns::new(
                                "bot-webhook-delete",
                                vec![
                                    GroupedBtn::new("delete", tr("bot_webhook_delete"))
                                        .icon("trash.fill")
                                        .icon_background(UIColor::system_red().resolved_color(theme.appearance))
                                        .destructive(true)
                                        .show_chevron(false)
                                        .callback(move |_, cx| Self::present_delete_confirm(&delete_state, cx)),
                                ],
                            )
                            .corner_radius(SECTION_RADIUS),
                        )
                    })
                    .when_some(error, |column, error| {
                        column.child(
                            div()
                                .w_full()
                                .px(px(18.0))
                                .child(MainText::new(error).px(13.0).color(theme.semantic.error_100)),
                        )
                    })
                    .child(div().flex_none().h(px(80.0))),
            )
    }
}

impl RenderOnce for BotWebhook {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let bot_id = self.bot_id.clone();
        let state = window.use_keyed_state(SharedString::from(format!("bot-webhook-state-{bot_id}")), cx, move |_, cx| {
            BotWebhookState::new(bot_id, cx)
        });

        let (did_hydrate, can_save) = {
            let state = state.read(cx);
            (state.did_hydrate, state.can_save())
        };
        let (slice_status, slice_error, is_saving, save_error) = {
            let actions = BotActionsStore::shared(cx);
            let actions = actions.read(cx);
            (
                actions.get_bot_webhook_info_sai.status(),
                actions.get_bot_webhook_info_sai.error(),
                actions.set_bot_webhook_sai.is_pending(),
                actions.set_bot_webhook_sai.error(),
            )
        };
        on_change("bot-webhook-save-error", save_error, false, window, cx, |_, error, _, cx| {
            if let Some(error) = error {
                Notifier::send(
                    error.clone(),
                    NotifierSendOptions {
                        r#type: NotifierType::Error,
                        position: NotifierPosition::Bottom,
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            }
        });
        let status = if did_hydrate { SaiWsSliceStatus::Fulfilled } else { slice_status };

        on_change("bot-webhook-can-save", can_save, true, window, cx, |_, ready, _, cx| {
            GuideFacts::set(RiseGuideSignal::BotWebhookReady, *ready, cx);
        });

        let save_state = state.clone();
        let content_state = state.clone();
        let content_theme = theme.clone();
        let retry_state = state.clone();
        let appear_state = state;
        let appear_bot_id = self.bot_id;
        let retry_error = slice_error.clone();

        SettingsWrapperUi::new(tr("bot_webhook_title"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .right_jsx(
            div()
                .opacity(if can_save && !is_saving { 1.0 } else { 0.42 })
                .child(
                    PageHeaderIconButton::new("bot-webhook-save", "checkmark", move |_, cx| {
                        BotWebhookState::save(&save_state, cx);
                    })
                    .disabled(!can_save || is_saving),
                )
                .guide_anchor(RiseGuideAnchor::BotWebhookSave),
        )
        .content(
            SaiDataRender::new(status, move |_, cx| Self::content(&content_state, &content_theme, cx))
                .id("bot-webhook-data")
                .is_empty(false)
                .error_text(slice_error.unwrap_or_else(|| tr("bot_webhook_load_error")))
                .sai_data_render_error(move |_, _| {
                    let retry_state = retry_state.clone();
                    BotStateView::new(
                        "bot-webhook-error",
                        "exclamationmark.arrow.triangle.2.circlepath",
                        tr("bot_webhook_load_error"),
                    )
                    .message(retry_error.clone())
                    .action_title(tr("bot_retry"))
                    .action(move |_, cx| retry_state.update(cx, |state, cx| state.reload(cx)))
                }),
        )
        .on_appear("bot-webhook-appear", move |_, cx| {
            BotInteractionsStore::shared(cx).update(cx, |store, cx| store.load_webhook(&appear_bot_id, cx));
            appear_state.update(cx, |state, cx| state.hydrate_if_possible(cx));
        })
        .and_on_disappear(|_, cx| GuideFacts::set(RiseGuideSignal::BotWebhookReady, false, cx))
    }
}
