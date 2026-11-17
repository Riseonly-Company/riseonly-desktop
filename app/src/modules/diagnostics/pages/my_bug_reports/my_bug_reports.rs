use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, FontWeight, Hsla, IntoElement, KeyDownEvent, Pixels, Point,
    PromptButton, PromptLevel, Render, RenderOnce, SharedString, Subscription, Task, Window, white,
};
use rise_canon::RiseBugReportStatus;
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::Date;
use rise_i18n::{Locale, tr};
use rise_theme::{AppTheme, ThemeStore, color};
use rise_ui::IconUi;
use rise_ui::animated_tabs_ui::animated_tabs_types::TabConfig;
use rise_ui::grouped_btns::GroupedCardMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::animated_tabs_ui::animated_tabs::AnimatedTabs;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_types::{
    BugReportDraftKind, BugReportListSlice, BugReportListState,
};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_wire_types::BugReportRecord;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportInteractionsStore;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_services::bug_report_services::BugReportServicesStore;

const ACTION_EDIT: &str = "edit";
const ACTION_REFILE: &str = "refile";
const ACTION_DELETE: &str = "delete";
const REFRESH_KEY: &str = "r";

const SKELETON_TITLE_WIDTHS: [f32; 4] = [220.0, 168.0, 196.0, 150.0];
const SKELETON_SUBTITLE_WIDTHS: [f32; 4] = [120.0, 96.0, 140.0, 104.0];

static REFILED_COLOR: LazyLock<Hsla> = LazyLock::new(|| color(RiseBugReportStatus::Reopened.color()));

#[derive(IntoElement, Default)]
pub struct MyBugReports;

impl MyBugReports {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for MyBugReports {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let body = window.use_keyed_state("my-bug-reports-body", cx, |window, cx| MyBugReportsBody::new(window, cx));
        SettingsWrapperUi::new(tr("bug_report_my_title"), move |_: &mut Window, cx: &mut App| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .subtitle(Some(SharedString::from(tr("bug_report_my_hint"))))
        .content(body)
    }
}

fn present_pending_deletion(pending_deletion: BugReportRecord, window: &mut Window, cx: &mut App) {
    let title = tr(if pending_deletion.is_feedback() { "feedback_delete_title" } else { "bug_report_delete_title" });
    let answer = window.prompt(
        PromptLevel::Critical,
        &title,
        None,
        &[PromptButton::cancel(tr("cancel")), PromptButton::ok(tr("bug_report_discard_confirm"))],
        cx,
    );
    let interactions = BugReportInteractionsStore::shared(cx);
    cx.spawn(async move |cx| {
        if matches!(answer.await, Ok(1)) {
            interactions.update(cx, |interactions, cx| {
                interactions.delete_report(&pending_deletion.id, pending_deletion.kind, cx);
            });
        }
    })
    .detach();
}

struct MyBugReportsBody {
    services: Entity<BugReportServicesStore>,
    interactions: Entity<BugReportInteractionsStore>,
    focus_handle: FocusHandle,
    _observers: [Subscription; 2],
}

impl MyBugReportsBody {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let services = BugReportServicesStore::shared(cx);
        let interactions = BugReportInteractionsStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let observers = [cx.observe(&services, |_, _, cx| cx.notify()), cx.observe(&theme_store, |_, _, cx| cx.notify())];
        let focus_handle = cx.focus_handle();
        let initial_focus = focus_handle.clone();
        window.on_next_frame(move |window, cx| window.focus(&initial_focus, cx));
        Self { services, interactions, focus_handle, _observers: observers }
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key != REFRESH_KEY || !event.keystroke.modifiers.secondary() {
            return;
        }
        let tab = usize::try_from(self.services.read(cx).my_reports_tab()).unwrap_or(0);
        let Some(kind) = BugReportDraftKind::ALL_CASES.get(tab).copied() else {
            return;
        };
        cx.stop_propagation();
        self.interactions.update(cx, |interactions, cx| interactions.refresh_my_reports_kind(kind, cx));
    }

    fn tab(id: i64, title: String, kind: BugReportDraftKind) -> TabConfig {
        let list_id = SharedString::from(format!("my-reports-list-{}", kind.raw_value()));
        TabConfig::new(id, title, move |window: &mut Window, cx: &mut App| {
            window
                .use_keyed_state(list_id.clone(), cx, |_, cx| MyReportsList::new(kind, cx))
                .into_any_element()
        })
    }
}

impl Render for MyBugReportsBody {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let (read_services, write_interactions) = (self.services.clone(), self.interactions.clone());
        let active_tab = Binding::new(
            move |cx: &App| read_services.read(cx).my_reports_tab(),
            move |index: i64, _: &mut Window, cx: &mut App| {
                write_interactions.update(cx, |interactions, cx| interactions.select_my_reports_tab(index, cx));
            },
        );
        div()
            .id("my-reports-body")
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .pt(px(list_shell_floating_top_inset(cx)))
            .bg(background)
            .child(
                AnimatedTabs::new(
                    "my-reports-tabs",
                    vec![
                        Self::tab(0, tr("my_reports_tab_bugs"), BugReportDraftKind::Bug),
                        Self::tab(1, tr("my_reports_tab_feedback"), BugReportDraftKind::Feedback),
                    ],
                    active_tab,
                )
                .horizontal_padding(10.0),
            )
    }
}

struct MyReportsList {
    kind: BugReportDraftKind,
    services: Entity<BugReportServicesStore>,
    interactions: Entity<BugReportInteractionsStore>,
    hold_context_menu: Entity<HoldContextMenuPresenter>,
    _observers: [Subscription; 3],
}

impl MyReportsList {
    fn new(kind: BugReportDraftKind, cx: &mut Context<Self>) -> Self {
        let services = BugReportServicesStore::shared(cx);
        let interactions = BugReportInteractionsStore::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let observers = [
            cx.observe(&services, |_, _, cx| cx.notify()),
            cx.observe(&hold_context_menu, |_, _, cx| cx.notify()),
            cx.observe(&theme_store, |_, _, cx| cx.notify()),
        ];
        let loader = interactions.clone();
        cx.defer(move |cx| {
            loader.update(cx, |interactions, cx| interactions.ensure_my_reports_loaded(kind, cx));
        });
        Self { kind, services, interactions, hold_context_menu, _observers: observers }
    }

    fn content(&self, slice: &BugReportListSlice, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        match &slice.state {
            BugReportListState::Idle | BugReportListState::Loading => {
                Self::group_skeleton(4).skeleton_shimmer(None).into_any_element()
            }
            BugReportListState::Failed { message } => self.failure(message, theme),
            BugReportListState::Loaded => {
                if slice.records.is_empty() {
                    self.empty(theme)
                } else {
                    self.list(slice, theme, cx)
                }
            }
        }
    }

    fn list(&self, slice: &BugReportListSlice, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let deleting_report_id = self.services.read(cx).deleting_report_id().map(str::to_owned);
        let last_index = slice.records.len().saturating_sub(1);
        let mut card = div()
            .flex()
            .flex_col()
            .w_full()
            .bg(theme.bg._200)
            .clip_shape(GroupedCardMetrics::SHAPE);
        for (index, record) in slice.records.iter().enumerate() {
            let is_deleting = deleting_report_id.as_deref() == Some(record.id.as_str());
            card = card.child(self.hold_row(record, is_deleting, theme));
            if index < last_index {
                card = card.child(
                    div()
                        .h(px(0.5))
                        .ml(px(GroupedCardMetrics::content_leading_inset()))
                        .bg(theme.border._200.opacity(0.55)),
                );
            }
        }

        let mut column = div()
            .flex()
            .flex_col()
            .items_center()
            .w_full()
            .gap(px(12.0))
            .child(div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(card));
        if slice.have_more {
            let (interactions, kind) = (self.interactions.clone(), self.kind);
            column = column.child(
                plain(SharedString::from(format!("my-reports-more-{}", kind.raw_value())))
                    .disabled(slice.state.is_loading())
                    .on_press(move |_: &mut Window, cx: &mut App| {
                        interactions.update(cx, |interactions, cx| interactions.load_more_my_reports(kind, cx));
                    })
                    .child(
                        MainText::new(tr("common_show_more"))
                            .px(13.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    ),
            );
        }
        column.child(div().min_h(px(80.0))).into_any_element()
    }

    fn hold_row(&self, record: &BugReportRecord, is_deleting: bool, theme: &AppTheme) -> AnyElement {
        let (actions_services, actions_record) = (self.services.clone(), record.clone());
        let (row_record, row_theme) = (record.clone(), theme.clone());
        let action_record = record.clone();
        let (tap_interactions, tap_record) = (self.interactions.clone(), record.clone());
        let action_interactions = self.interactions.clone();
        let kind = self.kind;
        let hold = HoldContextMenuContainer::new(
            SharedString::from(format!("my-report-row-{}", record.id)),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |cx: &App| {
                let slice = actions_services.read(cx).slice_for(kind);
                let current = slice.records.iter().find(|record| record.id == actions_record.id);
                Self::actions(current.unwrap_or(&actions_record))
            }),
            Rc::new(move |_: &mut Window, _: &mut App| {
                Self::row(&row_record, is_deleting, &row_theme).into_any_element()
            }),
        )
        .show_emoji_list(false)
        .disabled(is_deleting)
        .on_action(Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
            Self::perform(&id, &action_record, &action_interactions, window, cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            tap_interactions.update(cx, |interactions, cx| interactions.open_report(&tap_record, cx));
        }));
        div().w_full().child(hold).into_any_element()
    }

    fn row(record: &BugReportRecord, is_deleting: bool, theme: &AppTheme) -> impl IntoElement {
        let symbol = if record.is_feedback() { "lightbulb.fill" } else { "ladybug.fill" };
        let icon = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(GroupedCardMetrics::ICON_SIDE))
            .rounded_rect(8.0)
            .bg(color(record.status.color()))
            .children(IconUi::sized(symbol, px(15.0), white(), true));
        let texts = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(3.0))
            .child(
                MainText::new(record.message.clone())
                    .px(15.0)
                    .font_weight(FontWeight::MEDIUM)
                    .color(theme.text.primary)
                    .number_of_lines(2),
            )
            .child(MainText::new(Self::subtitle(record)).px(12.0).color(theme.text.secondary).number_of_lines(1));
        let mut chips = div().flex().flex_col().flex_none().items_end().gap(px(4.0)).child(Self::status_chip(record.status));
        if record.attempt_count > 1 {
            chips = chips.child(Self::attempt_chip(record.attempt_count));
        }
        div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .gap(px(GroupedCardMetrics::ICON_SPACING))
            .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
            .opacity(if is_deleting { 0.4 } else { 1.0 })
            .child(icon)
            .child(texts)
            .child(chips)
    }

    /// shows the edit timestamp instead of the submission one when the record was edited
    fn subtitle(record: &BugReportRecord) -> String {
        let stamp = record.updated_at_ms.unwrap_or(record.received_at_ms);
        let date = RiseFormat::date_time_label(
            &Date::from_time_interval_since_1970(stamp as f64 / 1000.0),
            &Date::now(),
            &Calendar::current(),
            Locale::app_locale().identifier(),
        );
        let mut parts = vec![date];
        if let Some(feedback_kind) = record.feedback_kind {
            parts.push(tr(feedback_kind.i18n_key()));
        }
        let areas = record.pages.iter().map(|page| tr(page.i18n_key())).collect::<Vec<_>>().join(", ");
        if !areas.is_empty() {
            parts.push(areas);
        }
        parts.join(" \u{00B7} ")
    }

    fn attempt_chip(count: i64) -> impl IntoElement {
        let chip_color = *REFILED_COLOR;
        div()
            .px(px(7.0))
            .py(px(2.0))
            .bg(chip_color.opacity(0.15))
            .capsule()
            .child(
                MainText::new(format!("\u{00D7}{count}"))
                    .px(11.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(chip_color),
            )
    }

    fn status_chip(status: RiseBugReportStatus) -> impl IntoElement {
        let chip_color = color(status.color());
        div()
            .px(px(8.0))
            .py(px(3.0))
            .bg(chip_color.opacity(0.15))
            .capsule()
            .child(
                MainText::new(tr(status.i18n_key()))
                    .px(11.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(chip_color),
            )
    }

    fn actions(record: &BugReportRecord) -> Vec<HoldContextMenuAction> {
        let mut actions = Vec::new();
        if record.is_editable() {
            actions.push(HoldContextMenuAction::new(ACTION_EDIT, tr("ctx_edit")).system_image("pencil"));
        }
        if record.can_refile() {
            actions.push(
                HoldContextMenuAction::new(ACTION_REFILE, tr("bug_report_refile"))
                    .system_image("arrow.counterclockwise"),
            );
        }
        actions.push(
            HoldContextMenuAction::new(ACTION_DELETE, tr("bug_report_discard_confirm"))
                .system_image("trash")
                .destructive(true),
        );
        actions
    }

    fn perform(
        id: &str,
        record: &BugReportRecord,
        interactions: &Entity<BugReportInteractionsStore>,
        window: &mut Window,
        cx: &mut App,
    ) {
        match id {
            ACTION_EDIT => interactions.update(cx, |interactions, cx| interactions.present_edit(record, cx)),
            ACTION_REFILE => interactions.update(cx, |interactions, cx| interactions.present_refile(record, cx)),
            ACTION_DELETE => present_pending_deletion(record.clone(), window, cx),
            _ => {}
        }
    }

    fn empty(&self, theme: &AppTheme) -> AnyElement {
        let is_feedback = self.kind == BugReportDraftKind::Feedback;
        div()
            .flex()
            .flex_col()
            .items_center()
            .w_full()
            .gap(px(8.0))
            .pt(px(64.0))
            .px(px(32.0))
            .children(IconUi::sized(
                if is_feedback { "lightbulb" } else { "ladybug" },
                px(34.0),
                theme.text.secondary.opacity(0.6),
                false,
            ))
            .child(
                MainText::new(tr(if is_feedback { "feedback_my_empty" } else { "bug_report_my_empty" }))
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary),
            )
            .into_any_element()
    }

    fn failure(&self, message: &str, theme: &AppTheme) -> AnyElement {
        let (interactions, kind) = (self.interactions.clone(), self.kind);
        div()
            .flex()
            .flex_col()
            .items_center()
            .w_full()
            .gap(px(12.0))
            .pt(px(64.0))
            .px(px(32.0))
            .child(
                MainText::new(message.to_owned())
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary),
            )
            .child(
                plain(SharedString::from(format!("my-reports-retry-{}", kind.raw_value())))
                    .on_press(move |_: &mut Window, cx: &mut App| {
                        interactions.update(cx, |interactions, cx| interactions.refresh_my_reports_kind(kind, cx));
                    })
                    .child(
                        MainText::new(tr("common_retry"))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    ),
            )
            .into_any_element()
    }

    fn group_skeleton(rows: usize) -> impl IntoElement {
        let mut column = div().flex().flex_col().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING));
        for index in 0..rows {
            column = column.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(GroupedCardMetrics::ICON_SPACING))
                    .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                    .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(GroupedCardMetrics::ICON_SPACING))
                            .child(SkeletonBlock::new(GroupedCardMetrics::ICON_SIDE, GroupedCardMetrics::ICON_SIDE, 8.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(6.0))
                                    .child(SkeletonLine::new(SKELETON_TITLE_WIDTHS[index % 4], 10.0))
                                    .child(SkeletonLine::new(SKELETON_SUBTITLE_WIDTHS[index % 4], 9.0)),
                            ),
                    )
                    .child(SkeletonBlock::new(64.0, 18.0, 9.0)),
            );
        }
        column
    }
}

impl Render for MyReportsList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let slice = self.services.read(cx).slice_for(self.kind);
        let is_scroll_disabled = self.hold_context_menu.read(cx).active_request().is_some();
        let content = self.content(&slice, &theme, cx);
        let (refresh_interactions, kind) = (self.interactions.clone(), self.kind);
        div()
            .size_full()
            .bg(theme.bg._100)
            .child(
                scroll_view(SharedString::from(format!("my-reports-scroll-{}", kind.raw_value())))
                    .size_full()
                    .scroll_disabled(is_scroll_disabled)
                    .refreshable(move |_: &mut Window, cx: &mut App| {
                        refresh_interactions
                            .update(cx, |interactions, cx| interactions.refresh_my_reports_kind(kind, cx));
                        Task::ready(())
                    })
                    .child(div().pt(px(12.0)).child(content)),
            )
    }
}
