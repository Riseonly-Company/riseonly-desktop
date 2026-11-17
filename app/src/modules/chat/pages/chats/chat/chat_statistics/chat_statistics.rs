use gpui::{
    App, Context, Entity, FontWeight, Render, SharedString, Subscription, Task, Window,
};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::{AppDateFormatter, tr};
use rise_theme::AppTheme;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::on_change;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::swiftui::stacks::{hstack, spacer, spacer_min_h, vstack};

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatCountFormatter;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatAnalyticsModel;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const DAY_LABEL_FORMAT: &str = "d MMM";
const CARD_CORNER_RADIUS: f32 = 16.0;
const GROWTH_CHART_HEIGHT: f32 = 100.0;
const GROWTH_BAR_MAXIMUM_HEIGHT: f32 = 96.0;
const GROWTH_BAR_MINIMUM_HEIGHT: f32 = 4.0;
const GROWTH_EMPTY_MINIMUM_HEIGHT: f32 = 80.0;

pub struct ChatStatistics {
    chat_id: Option<String>,
    settings_store: Entity<ChatServicesStore>,
    actions_store: Entity<ChatActionsStore>,
    _subscriptions: Vec<Subscription>,
}

impl ChatStatistics {
    pub fn new(chat_id: Option<String>, cx: &mut Context<Self>) -> Self {
        let settings_store = ChatServicesStore::shared(cx);
        let actions_store = ChatActionsStore::shared(cx);
        let subscriptions = vec![
            cx.observe(&settings_store, |_, _, cx| cx.notify()),
            cx.observe(&actions_store, |_, _, cx| cx.notify()),
        ];
        Self { chat_id, settings_store, actions_store, _subscriptions: subscriptions }
    }

    fn resolved_chat_id(&self) -> String {
        self.chat_id.clone().unwrap_or_default()
    }

    fn load_analytics(chat_id: &str, cx: &mut App) {
        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_analytics(chat_id, cx));
    }

    fn counter_grid(model: &ChatAnalyticsModel, theme: &AppTheme) -> Div {
        hstack(10.0)
            .w_full()
            .px(px(12.0))
            .child(Self::counter_tile(
                ChatCountFormatter::compact(model.member_count),
                tr("chat_stats_subscribers"),
                (model.new_members > 0).then(|| format!("+{}", model.new_members)),
                theme,
            ))
            .child(Self::counter_tile(
                ChatCountFormatter::compact(model.posts_count),
                tr("chat_stats_posts"),
                None,
                theme,
            ))
    }

    fn counter_tile(value: String, label: String, delta: Option<String>, theme: &AppTheme) -> Div {
        vstack(4.0)
            .flex_1()
            .items_start()
            .p(px(14.0))
            .bg(theme.bg._200)
            .continuous_corners(CARD_CORNER_RADIUS)
            .child(
                hstack(6.0)
                    .items_baseline()
                    .child(MainText::new(value).px(24.0).font_weight(FontWeight::BOLD))
                    .children(delta.map(|delta| {
                        MainText::new(delta)
                            .px(13.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.semantic.success_100)
                    })),
            )
            .child(MainText::new(label).px(13.0).color(theme.text.secondary))
    }

    fn growth_card(model: &ChatAnalyticsModel, theme: &AppTheme) -> Div {
        let max_count = model.buckets.iter().map(|point| point.count).max().unwrap_or(0).max(1);
        let title = MainText::new(tr("chat_stats_growth_title"))
            .px(13.0)
            .font_weight(FontWeight::SEMIBOLD)
            .color(theme.text.secondary);
        let card = vstack(10.0).items_stretch().p(px(14.0)).child(title);
        let card = if model.buckets.iter().all(|point| point.count == 0) {
            card.child(
                div()
                    .w_full()
                    .min_h(px(GROWTH_EMPTY_MINIMUM_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(MainText::new(tr("chat_stats_growth_empty")).px(13.0).color(theme.text.secondary)),
            )
        } else {
            card.child(
                hstack(2.0).w_full().items_end().h(px(GROWTH_CHART_HEIGHT)).children(model.buckets.iter().map(
                    |point| {
                        div()
                            .flex_1()
                            .h(px((point.count as f32 / max_count as f32 * GROWTH_BAR_MAXIMUM_HEIGHT)
                                .max(GROWTH_BAR_MINIMUM_HEIGHT)))
                            .capsule()
                            .bg(if point.count > 0 { theme.primary._100 } else { theme.bg._300 })
                    },
                )),
            )
            .child(
                hstack(None::<f32>)
                    .w_full()
                    .child(
                        MainText::new(Self::day_label(model.buckets.first().map(|point| point.day_start_ms)))
                            .px(11.0)
                            .color(theme.text.secondary),
                    )
                    .child(spacer())
                    .child(
                        MainText::new(Self::day_label(model.buckets.last().map(|point| point.day_start_ms)))
                            .px(11.0)
                            .color(theme.text.secondary),
                    ),
            )
        };

        div().w_full().px(px(12.0)).child(card.w_full().bg(theme.bg._200).continuous_corners(CARD_CORNER_RADIUS))
    }

    fn day_label(ms: Option<i64>) -> String {
        let Some(ms) = ms else {
            return String::new();
        };
        let date = Date::from_time_interval_since_1970(ms as f64 / 1000.0);
        let mut formatter = AppDateFormatter::make(None, None, None);
        formatter.set_date_format(DAY_LABEL_FORMAT);
        formatter.string_from(&date)
    }

    fn posts_card(model: &ChatAnalyticsModel, theme: &AppTheme) -> Div {
        div().w_full().px(px(12.0)).child(
            vstack(0.0)
                .w_full()
                .items_stretch()
                .bg(theme.bg._200)
                .continuous_corners(CARD_CORNER_RADIUS)
                .child(Self::stat_row(tr("chat_stats_views"), ChatCountFormatter::compact(model.views_total), theme))
                .child(Self::divider(theme))
                .child(Self::stat_row(
                    tr("chat_stats_avg_views"),
                    ChatCountFormatter::compact(model.avg_views_per_post),
                    theme,
                ))
                .child(Self::divider(theme))
                .child(Self::stat_row(
                    tr("chat_stats_reactions"),
                    ChatCountFormatter::compact(model.reactions_total),
                    theme,
                )),
        )
    }

    fn stat_row(label: String, value: String, theme: &AppTheme) -> Div {
        hstack(None::<f32>)
            .w_full()
            .px(px(16.0))
            .py(px(12.0))
            .child(MainText::new(label).px(15.0))
            .child(spacer())
            .child(MainText::new(value).px(15.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.secondary))
    }

    fn divider(theme: &AppTheme) -> Div {
        div().ml(px(16.0)).h(px(0.5)).bg(theme.border._200.opacity(0.55))
    }
}

impl Render for ChatStatistics {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let resolved_chat_id = self.resolved_chat_id();
        let analytics = self.settings_store.read(cx).analytics(&resolved_chat_id).cloned();
        let slice = self.actions_store.read(cx).chat_analytics_sai.clone();
        let navigation_store = env::<NavigationStoreKey>(cx);

        on_change("chat-statistics-task", resolved_chat_id.clone(), true, window, cx, |_, chat_id, _, cx| {
            if !chat_id.is_empty() {
                Self::load_analytics(chat_id, cx);
            }
        });

        let refresh_chat_id = resolved_chat_id.clone();
        let reload_chat_id = resolved_chat_id.clone();
        let content_theme = theme.clone();
        let need_pending = analytics.is_none();
        let body = SaiDataRender::slice(&*slice, move |_, cx| {
            let theme = content_theme;
            let cards = analytics.map(|model| {
                vstack(16.0)
                    .w_full()
                    .child(Self::counter_grid(&model, &theme))
                    .child(Self::growth_card(&model, &theme))
                    .when(model.stats_available, |column| column.child(Self::posts_card(&model, &theme)))
            });
            scroll_view(SharedString::from(format!("chat-statistics-{resolved_chat_id}")))
                .list_shell_scroll_under_chrome(cx)
                .size_full()
                .bg(theme.bg._100)
                .refreshable(move |_, cx| {
                    Self::load_analytics(&refresh_chat_id, cx);
                    Task::ready(())
                })
                .child(vstack(16.0).w_full().pt(px(12.0)).children(cards).child(spacer_min_h(80.0)))
        })
        .is_empty(false)
        .need_pending(need_pending);

        PageRefreshKeys::new(
            "chat-statistics",
            move |_, cx| {
                if !reload_chat_id.is_empty() {
                    Self::load_analytics(&reload_chat_id, cx);
                }
            },
            SettingsWrapperUi::new(tr("chat_settings_statistics"), move |_: &mut Window, cx: &mut App| {
                if let Some(navigation_store) = &navigation_store {
                    navigation_store.update(cx, |store, cx| store.pop(cx));
                }
            })
            .content(body),
        )
    }
}
