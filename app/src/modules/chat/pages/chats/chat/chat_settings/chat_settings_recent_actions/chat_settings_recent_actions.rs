use std::rc::Rc;

use gpui::{App, Context, ListAlignment, SharedString, Subscription, Task, Window};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_i18n::{Locale, tr};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::MainText;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use super::super::chat_settings_shared::chat_settings_symbol_is_filled;
use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_floating_top_inset,
};
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatAction;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const LIST_TOP_PADDING: f32 = 4.0;
const ROW_DIVIDER_INSET: f32 = 52.0;
const SKELETON_ROWS: usize = 10;

fn row_divider(theme: &AppTheme) -> impl IntoElement {
    div().flex_none().w_full().pl(px(ROW_DIVIDER_INSET)).child(div().w_full().h(px(0.5)).bg(theme.border._100))
}

struct ChatSettingsRecentActionsState {
    list: ListUi,
    ids: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl ChatSettingsRecentActionsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let actions = ChatActionsStore::shared(cx);
        Self {
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            ids: Vec::new(),
            _subscriptions: vec![
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
            ],
        }
    }

    /// a next page keeps the viewport, anything else is a different list
    fn sync(&mut self, ids: Vec<String>) {
        if ids == self.ids {
            return;
        }
        if !self.ids.is_empty() && ids.starts_with(&self.ids) {
            self.list.appended(ids.len() - self.ids.len());
        } else {
            self.list.replaced(ids.len());
        }
        self.ids = ids;
    }
}

#[derive(IntoElement)]
pub struct ChatSettingsRecentActions {
    chat_id: Option<String>,
}

impl ChatSettingsRecentActions {
    pub fn new(chat_id: Option<String>) -> Self {
        Self { chat_id }
    }

    fn row(action: &ChatAction, theme: &AppTheme) -> impl IntoElement {
        let symbol = action.icon_system_name();
        hstack(12.0)
            .w_full()
            .align(VerticalAlignment::Top)
            .px(px(16.0))
            .py(px(10.0))
            .child(
                div()
                    .flex_none()
                    .size(px(30.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.0))
                    .bg(theme.bg._300)
                    .children(IconUi::sized(
                        symbol,
                        px(14.0),
                        theme.text.secondary,
                        chat_settings_symbol_is_filled(symbol),
                    )),
            )
            .child(
                vstack(2.0)
                    .flex_1()
                    .min_w_0()
                    .align(HorizontalAlignment::Leading)
                    .child(MainText::new(action.description()).px(14.0).color(theme.text.primary).number_of_lines(3))
                    .when_some(action.detail_text(), |column, detail| {
                        column.child(MainText::new(detail).px(13.0).color(theme.text.secondary))
                    })
                    .child(MainText::new(ChatAuditTime::format(action.created_at)).px(11.0).color(theme.text.secondary)),
            )
    }
}

impl RenderOnce for ChatSettingsRecentActions {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let resolved_chat_id = self.chat_id.unwrap_or_default();
        let state = window.use_keyed_state(
            SharedString::from(format!("chat-settings-recent-actions-{resolved_chat_id}")),
            cx,
            |_, cx| ChatSettingsRecentActionsState::new(cx),
        );

        let task_chat_id = resolved_chat_id.clone();
        use_task(
            SharedString::from(format!("chat-settings-recent-actions-load-{resolved_chat_id}")),
            (),
            window,
            cx,
            async move |cx| {
                let _ = cx.update(|_, cx| {
                    ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_chat_actions(&task_chat_id, cx));
                });
            },
        );

        let services = ChatServicesStore::shared(cx);
        let (actions, relative_id, has_more) = services.update(cx, |services, cx| {
            (
                services.chat_actions(&resolved_chat_id, cx),
                services.chat_actions_last_id(&resolved_chat_id, cx),
                services.chat_actions_has_more(&resolved_chat_id, cx),
            )
        });
        let ids: Vec<String> = actions.iter().map(|action| action.id.clone()).collect();
        state.update(cx, |state, _| state.sync(ids));

        let is_empty = actions.is_empty();
        let actions = Rc::new(actions);
        let slice = ChatActionsStore::shared(cx).read(cx).actions_sai.clone();
        let cache_key = SharedString::from(ChatServicesStore::chat_actions_cache_key(&resolved_chat_id));

        let list_state = state;
        let list_theme = theme.clone();
        let refresh_chat_id = resolved_chat_id.clone();
        let more_chat_id = resolved_chat_id;
        let body = SaiDataRender::slice(&*slice, move |_, cx| {
            let top_inset = list_shell_floating_top_inset(cx);
            list_state.read(cx).list.refreshable(
                "chat-settings-recent-actions-list",
                move |_, cx| {
                    ChatInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.load_chat_actions(&refresh_chat_id, cx));
                    Task::ready(())
                },
                move |index, _, _| {
                    let Some(action) = actions.get(index) else {
                        return div().into_any_element();
                    };
                    let chat_id = more_chat_id.clone();
                    let row = Self::row(action, &list_theme).sai_pagination_prefetch(
                        SharedString::from(format!("chat-settings-recent-action-{}", action.id)),
                        SaiPaginationPrefetchOptions::new(
                            index,
                            actions.len(),
                            cache_key.clone(),
                            relative_id.clone(),
                            has_more,
                        ),
                        move |_, cx| {
                            ChatInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.load_more_chat_actions(&chat_id, cx));
                        },
                    );
                    div()
                        .w_full()
                        .when(index == 0, |entry| entry.pt(px(top_inset + LIST_TOP_PADDING)))
                        .child(row)
                        .child(row_divider(&list_theme))
                        .into_any_element()
                },
            )
        })
        .id("chat-settings-recent-actions-render")
        .is_empty(is_empty)
        .no_data_text(tr("chat_settings_recent_actions_empty"))
        .need_pending(is_empty)
        .sai_data_render_loading(|_, cx| {
            let background = rise_ui::theme(cx).bg._100;
            div().size_full().bg(background).child(
                scroll_view("chat-settings-recent-actions-skeleton-scroll")
                    .size_full()
                    .scroll_disabled(true)
                    .shows_indicators(false)
                    .list_shell_scroll_under_chrome(cx)
                    .child(ChatRecentActionsSkeleton::new(SKELETON_ROWS)),
            )
        });

        SettingsWrapperUi::new(tr("chat_settings_recent_actions"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(div().size_full().bg(theme.bg._100).child(body))
    }
}

#[derive(IntoElement)]
struct ChatRecentActionRowSkeleton {
    alternation_index: usize,
}

impl ChatRecentActionRowSkeleton {
    fn new(alternation_index: usize) -> Self {
        Self { alternation_index }
    }

    fn alt(&self, values: [f32; 4]) -> f32 {
        values[self.alternation_index % values.len()]
    }
}

impl RenderOnce for ChatRecentActionRowSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        hstack(12.0)
            .w_full()
            .align(VerticalAlignment::Top)
            .px(px(16.0))
            .py(px(10.0))
            .child(SkeletonBlock::new(30.0, 30.0, 8.0))
            .child(
                vstack(8.0)
                    .align(HorizontalAlignment::Leading)
                    .child(SkeletonLine::new(self.alt([220.0, 170.0, 250.0, 190.0]), 8.0))
                    .child(SkeletonLine::new(self.alt([70.0, 90.0, 60.0, 80.0]), 6.0)),
            )
            .child(spacer_min_w(4.0))
    }
}

#[derive(IntoElement)]
struct ChatRecentActionsSkeleton {
    rows: usize,
}

impl ChatRecentActionsSkeleton {
    fn new(rows: usize) -> Self {
        Self { rows }
    }
}

impl RenderOnce for ChatRecentActionsSkeleton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let rows: Vec<AnyElement> = (0..self.rows)
            .map(|index| {
                div()
                    .w_full()
                    .child(ChatRecentActionRowSkeleton::new(index))
                    .child(row_divider(theme))
                    .into_any_element()
            })
            .collect();
        div()
            .w_full()
            .overflow_hidden()
            .child(vstack(0.0).w_full().pt(px(LIST_TOP_PADDING)).children(rows).skeleton_shimmer(None))
    }
}

pub enum ChatAuditTime {}

impl ChatAuditTime {
    pub fn format(ts: TimeInterval) -> String {
        if ts <= 0.0 {
            return String::new();
        }
        RiseFormat::date_time_label(
            &Date::from_time_interval_since_1970(ts),
            &Date::now(),
            &Calendar::current(),
            Locale::app_locale().identifier(),
        )
    }
}
