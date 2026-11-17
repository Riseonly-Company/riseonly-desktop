use gpui::{App, Entity, FontWeight, Hsla, ListAlignment, SharedString, Subscription, Task, Window};
use rise_canon::rise_key::report_snapshot;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore, color};
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::filter_pill_ui::FilterPillUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::bottom_sheet_ui::sheet_floating_header::SheetFloatingHeader;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, ListShellUi, list_shell_floating_top_inset,
};
use crate::modules::moderation::components::moderation_case_ui::{ModerationQuoteBlock, RemoteEvidenceStrip};
use crate::modules::moderation::components::moderation_kit_ui::{
    ModerationArtwork, ModerationBadge, ModerationEmptyState, ModerationFailureState, ModerationHero,
    ModerationListSkeleton, ModerationMetrics, ModerationPersonRow, ModerationRowSeparator, ModerationTint,
    moderation_symbol,
};
use crate::modules::moderation::components::moderation_type_tints::{
    ReportStatusStyle, ReportTargetTypeTintExt, ResolveActionTintExt,
};
use crate::modules::moderation::engine::rise_moderation_presentation::{
    ModerationLoadState, RiseModerationPresentation,
};
use crate::modules::moderation::engine::rise_moderation_wire::ReportRowDTO;
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    ReportReasonExt, ReportStatus, ReportStatusExt, ReportTargetType, ReportTargetTypeExt, ResolveActionExt,
};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;

const REOPEN_TINT: &str = "#F2A53C";
const SKELETON_ROWS: usize = 10;
const LOAD_MORE_THRESHOLD: usize = 4;
const LIST_BOTTOM_SPACER: f32 = 80.0;
const DETAIL_ARTWORK_SIDE: f32 = 88.0;

fn interactions(cx: &mut App) -> Entity<ModerationInteractionsStore> {
    ModerationInteractionsStore::shared(cx)
}

fn refresh(cx: &mut App) {
    interactions(cx).update(cx, |store, cx| store.refresh_my_reports(cx));
}

fn load_more(cx: &mut App) {
    interactions(cx).update(cx, |store, cx| store.load_more_my_reports(cx));
}

fn badge_color(status: ReportStatus, theme: &AppTheme) -> Option<gpui::Hsla> {
    if status == ReportStatus::Active { None } else { Some(ReportStatusStyle::color(status, theme)) }
}

fn label_row(symbol: Option<gpui::Svg>, label: impl IntoElement) -> Div {
    hstack(8.0).children(symbol).child(label)
}

/// selected report and the presentation
struct MyReportsState {
    selected_report: Option<ReportRowDTO>,
    list: ListUi,
    list_ids: Vec<i64>,
    _subscriptions: Vec<Subscription>,
}

impl MyReportsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseModerationPresentation::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            selected_report: None,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            _subscriptions: vec![
                cx.observe(&presentation, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn sync_list(&mut self, ids: Vec<i64>) {
        if ids == self.list_ids {
            return;
        }
        let old = &self.list_ids;
        if !old.is_empty() && ids.len() > old.len() && ids.starts_with(old) {
            self.list.appended(ids.len() - old.len());
        } else {
            self.list.replaced(ids.len() + 1);
        }
        self.list_ids = ids;
    }
}

#[derive(IntoElement, Default)]
pub struct MyReports;

impl MyReports {
    pub fn new() -> Self {
        Self
    }

    fn status_filter_pill(cx: &mut App) -> Div {
        let (current, total) = {
            let presentation = RiseModerationPresentation::shared(cx);
            let presentation = presentation.read(cx);
            (presentation.my_reports_filter(), presentation.my_reports_total())
        };
        let mut items = vec![ContextMenuItem::new(-1, tr("report_filter_all"), None, "", |_, cx| {
            interactions(cx).update(cx, |store, cx| store.set_my_reports_filter(None, cx));
        })];
        items.extend(ReportStatus::ALL.iter().copied().enumerate().map(|(index, status)| {
            ContextMenuItem::new(index as i64, tr(status.title_key()), None, status.wire(), move |_, cx| {
                interactions(cx).update(cx, |store, cx| store.set_my_reports_filter(Some(status), cx));
            })
        }));
        let current_label = current.map_or_else(|| tr("report_filter_all"), |status| tr(status.title_key()));
        hstack(8.0)
            .w_full()
            .px(px(16.0))
            .py(px(8.0))
            .child(FilterPillUi::new(
                "my-reports-filter",
                items,
                current.map_or("", |status| status.wire()),
                current_label,
            ))
            .child(spacer_min_w(0.0))
            .when(total > 0, |row| row.child(SecondaryText::new(RiseFormat::compact_count(total)).px(12.0)))
    }

    fn sheet(state: Entity<MyReportsState>, background: Hsla) -> NativeBottomSheetPresentationHost {
        let reader = state.clone();
        let content = state.clone();
        NativeBottomSheetPresentationHost::new(
            "my-reports-detail-sheet",
            Binding::new(
                move |cx: &App| reader.read(cx).selected_report.is_some(),
                move |presented, _window, cx| {
                    if !presented {
                        state.update(cx, |state, cx| {
                            state.selected_report = None;
                            cx.notify();
                        });
                    }
                },
            ),
            move |_, cx| match content.read(cx).selected_report.clone() {
                Some(opened) => MyReportDetailSheet { opened, state: content.clone() }.into_any_element(),
                None => div().into_any_element(),
            },
        )
        .presentation_detents([PresentationDetent::Medium, PresentationDetent::Large])
        .presentation_drag_indicator(true)
        .presentation_background(background)
    }
}

impl RenderOnce for MyReports {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("my-reports-state", cx, |_, cx| MyReportsState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        div()
            .size_full()
            .child(
                ListShellUi::new()
                    .title(SharedString::from(tr("report_my_reports_title")))
                    .shows_back_button(true)
                    .on_back(move |_, cx| {
                        if let Some(nav_store) = &nav_store {
                            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                        }
                    })
                    .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
                    .below_header(Self::status_filter_pill(cx))
                    .content(div().size_full().bg(theme.bg._100).child(MyReportsBody { state: state.clone() })),
            )
            .child(Self::sheet(state, theme.bg._100))
            .on_appear("my-reports-task", |_, cx| {
                interactions(cx).update(cx, |store, cx| store.ensure_my_reports_loaded(cx));
            })
    }
}

#[derive(IntoElement)]
struct MyReportsBody {
    state: Entity<MyReportsState>,
}

impl MyReportsBody {
    fn load_more_retry(message: String, theme: &AppTheme) -> Div {
        vstack(10.0)
            .w_full()
            .px(px(28.0))
            .py(px(14.0))
            .child(MainText::new(message).px(12.0).tac(TextAlignT::Center).color(theme.text.secondary))
            .child(
                div().id("my-reports-retry-label").aria_label(tr("common_retry")).child(
                    plain("my-reports-retry").on_press(|_, cx| load_more(cx)).child(
                        hstack(6.0)
                            .h(px(34.0))
                            .px(px(13.0))
                            .rounded_full()
                            .bg(theme.bg._200)
                            .children(IconUi::sized("arrow.clockwise", px(12.0), theme.text.primary, false))
                            .child(
                                MainText::new(tr("common_retry"))
                                    .px(13.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.text.primary),
                            ),
                    ),
                ),
            )
    }

    fn footer(cx: &App, presentation: &RiseModerationPresentation) -> Div {
        let theme = rise_ui::theme(cx);
        let footer = vstack(0.0).w_full();
        let footer = if presentation.is_loading_more_my_reports() {
            footer.child(div().py(px(10.0)).child(progress_view()))
        } else if let Some(message) = presentation.my_reports_load_more_error() {
            footer.child(Self::load_more_retry(message.to_owned(), theme))
        } else if presentation.my_reports_has_more() {
            let cursor = presentation.my_reports_cursor();
            footer.child(
                div()
                    .w_full()
                    .h(px(1.0))
                    .on_appear(SharedString::from(format!("my-reports-next-page-{cursor:?}")), |_, cx| load_more(cx)),
            )
        } else {
            footer
        };
        footer.child(div().w_full().h(px(LIST_BOTTOM_SPACER)))
    }

    fn list(self, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let top_inset = list_shell_floating_top_inset(cx);
        let presentation = RiseModerationPresentation::shared(cx);
        let reports = presentation.read(cx).my_reports().to_vec();
        self.state.update(cx, |state, _| state.sync_list(reports.iter().map(|report| report.id).collect()));
        let count = reports.len();
        let state = self.state.clone();
        self.state
            .read(cx)
            .list
            .refreshable(
                "my-reports-list",
                |_, cx| {
                    refresh(cx);
                    Task::ready(())
                },
                move |index, _, cx| {
                    let Some(report) = reports.get(index) else {
                        return Self::footer(cx, presentation.read(cx)).into_any_element();
                    };
                    let report_id = report.id;
                    let tapped = report.clone();
                    let row_state = state.clone();
                    vstack(0.0)
                        .w_full()
                        .when(index == 0, |column| column.pt(px(top_inset)))
                        .child(
                            plain(SharedString::from(format!("my-report-row-{report_id}")))
                                .w_full()
                                .on_press(move |_, cx| {
                                    let tapped = tapped.clone();
                                    row_state.update(cx, |state, cx| {
                                        state.selected_report = Some(tapped);
                                        cx.notify();
                                    });
                                })
                                .child(report_row(report, &theme)),
                        )
                        .when(index + 1 < count, |column| column.child(ModerationRowSeparator::new()))
                        .on_appear(SharedString::from(format!("my-report-appear-{report_id}")), move |_, cx| {
                            if index + LOAD_MORE_THRESHOLD >= count {
                                load_more(cx);
                            }
                        })
                        .into_any_element()
                },
            )
            .into_any_element()
    }
}

impl RenderOnce for MyReportsBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let (load_state, is_empty, has_filter) = {
            let presentation = RiseModerationPresentation::shared(cx);
            let presentation = presentation.read(cx);
            (
                presentation.my_reports_state().clone(),
                presentation.my_reports().is_empty(),
                presentation.my_reports_filter().is_some(),
            )
        };
        match load_state {
            ModerationLoadState::Idle | ModerationLoadState::Loading => scroll_view("my-reports-skeleton")
                .scroll_disabled(true)
                .shows_indicators(false)
                .list_shell_scroll_under_chrome(cx)
                .child(ModerationListSkeleton::new().rows(SKELETON_ROWS))
                .into_any_element(),
            ModerationLoadState::Failed(message) => div()
                .size_full()
                .list_shell_scroll_under_chrome(cx)
                .child(ModerationFailureState::new("my-reports-failure", message, theme, |_, cx| refresh(cx)))
                .into_any_element(),
            ModerationLoadState::Loaded if is_empty => scroll_view("my-reports-empty")
                .refreshable(|_, cx| {
                    refresh(cx);
                    Task::ready(())
                })
                .list_shell_scroll_under_chrome(cx)
                .child(ModerationEmptyState::new(
                    "flag",
                    tr(if has_filter { "report_my_reports_filter_empty" } else { "report_my_reports_empty" }),
                    theme,
                ))
                .into_any_element(),
            ModerationLoadState::Loaded => self.list(cx),
        }
    }
}

fn report_row(report: &ReportRowDTO, theme: &AppTheme) -> ModerationPersonRow {
    let r#type = report.resolved_type();
    let status = report.resolved_status();
    let is_chat_or_bot = r#type == ReportTargetType::Chat || r#type == ReportTargetType::Bot;
    let person = if is_chat_or_bot { None } else { report.reported_user.as_ref() };
    let subject_name = if is_chat_or_bot {
        report
            .snapshot_title
            .as_deref()
            .map(|title| trimming_whitespaces_and_newlines(title).to_string())
            .filter(|title| !title.is_empty())
            .or_else(|| {
                report.snapshot_facts.string(if r#type == ReportTargetType::Bot {
                    report_snapshot::NAME
                } else {
                    report_snapshot::TITLE
                })
            })
            .unwrap_or_else(|| tr(r#type.title_key()))
    } else {
        person.and_then(|person| person.name.clone()).unwrap_or_else(|| report.subject_line())
    };
    let detail = report
        .decision
        .clone()
        .filter(|decision| !decision.is_empty())
        .or_else(|| report.resolved_reason().map(|reason| tr(reason.title_key())))
        .unwrap_or_else(|| tr(r#type.title_key()));
    let user_id = if is_chat_or_bot {
        None
    } else {
        report.reported_user_id().or_else(|| person.and_then(|person| person.tag.clone()))
    };
    let row = ModerationPersonRow::new(SharedString::from(format!("my-report-person-{}", report.id)), subject_name)
        .logo_source(person.and_then(|person| person.logo.clone()).map(SharedString::from))
        .user_id(user_id.map(SharedString::from))
        .subtitle(SharedString::from(detail))
        .is_official(person.is_some_and(|person| person.is_official))
        .is_premium(person.is_some_and(|person| person.is_premium))
        .trailing_text(report.created_relative_text().map(SharedString::from))
        .trailing_age_ms(report.created_at_ms)
        .trailing(ModerationBadge::new(tr(status.title_key())).color(badge_color(status, theme)));
    if person.is_some() {
        return row;
    }
    row.avatar(
        ModerationArtwork::new(
            SharedString::from(format!("my-report-artwork-{}", report.id)),
            r#type.icon_name(),
            ModerationTint::target(r#type),
        )
        .image_source(report.snapshot_media.first().cloned().map(SharedString::from))
        .side(ModerationMetrics::AVATAR_SIZE),
    )
}

#[derive(Default)]
struct MyReportDetailSheetState {
    last_seen: Option<ReportRowDTO>,
}

#[derive(IntoElement)]
struct MyReportDetailSheet {
    opened: ReportRowDTO,
    state: Entity<MyReportsState>,
}

impl RenderOnce for MyReportDetailSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let opened_id = self.opened.id;
        let state = window.use_keyed_state(
            SharedString::from(format!("my-report-detail-{opened_id}")),
            cx,
            |_, _| MyReportDetailSheetState::default(),
        );
        let presentation = RiseModerationPresentation::shared(cx);
        let live_row = presentation.read(cx).my_reports().iter().find(|row| row.id == opened_id).cloned();
        if let Some(row) = &live_row {
            state.update(cx, |state, _| state.last_seen = Some(row.clone()));
        }
        let report = live_row.or_else(|| state.read(cx).last_seen.clone()).unwrap_or(self.opened);
        let r#type = report.resolved_type();
        let status = report.resolved_status();
        let evidence = report.evidence();
        let non_empty = |text: &Option<String>| text.clone().filter(|text| !text.is_empty());

        let closer = self.state.clone();
        let header = div().flex_none().w_full().bg(theme.bg._100).child(
            SheetFloatingHeader::new("my-report-detail-header", "", move |_, cx| {
                closer.update(cx, |state, cx| {
                    state.selected_report = None;
                    cx.notify();
                });
            })
            .shows_title(false)
            .shows_chrome(false),
        );

        div().flex().flex_col().size_full().bg(theme.bg._100).child(header).child(scroll_view("my-report-detail-scroll").flex_1().min_h_0().w_full().bg(theme.bg._100).child(
            vstack(16.0)
                .w_full()
                .items_start()
                .p(px(20.0))
                .pt(px(4.0))
                .child(
                    div().w_full().pb(px(4.0)).child(
                        ModerationHero::new(
                            "my-report-detail-hero",
                            tr(r#type.title_key()),
                            ModerationArtwork::new(
                                "my-report-detail-artwork",
                                r#type.icon_name(),
                                ModerationTint::target(r#type),
                            )
                            .image_source(report.snapshot_media.first().cloned().map(SharedString::from))
                            .side(DETAIL_ARTWORK_SIDE),
                        )
                        .subtitle(report.resolved_reason().map(|reason| SharedString::from(tr(reason.title_key()))))
                        .badge_text(SharedString::from(tr(status.title_key())))
                        .badge_color(badge_color(status, &theme)),
                    ),
                )
                .when_some(report.reported_user.as_ref(), |column, reported| {
                    let has_name = reported.name.as_deref().is_some_and(|name| !name.is_empty());
                    let subtitle = if has_name {
                        reported.tag.as_ref().map(|tag| SharedString::from(format!("@{tag}")))
                    } else {
                        None
                    };
                    column.child(
                        Shape::continuous(14.0).round(div().w_full().overflow_hidden()).bg(theme.bg._200).child(
                            ModerationPersonRow::new("my-report-detail-person", reported.display_name())
                                .logo_source(reported.logo.clone().map(SharedString::from))
                                .user_id(report.reported_user_id().map(SharedString::from))
                                .subtitle(subtitle)
                                .is_official(reported.is_official)
                                .is_premium(reported.is_premium),
                        ),
                    )
                })
                .when_some(non_empty(&report.snapshot_title), |column, title| {
                    column.child(ModerationQuoteBlock::new(
                        tr("report_detail_target"),
                        title,
                        theme.bg._200,
                        theme.clone(),
                    ))
                })
                .when_some(non_empty(&report.snapshot_text), |column, text| {
                    column.child(ModerationQuoteBlock::new(
                        tr("report_detail_snapshot"),
                        text,
                        theme.bg._200,
                        theme.clone(),
                    ))
                })
                .when(!report.description.is_empty(), |column| {
                    column.child(ModerationQuoteBlock::new(
                        tr("report_detail_your_reason"),
                        report.description.clone(),
                        theme.bg._200,
                        theme.clone(),
                    ))
                })
                .when(!evidence.is_empty(), |column| {
                    column.child(
                        vstack(6.0)
                            .w_full()
                            .items_start()
                            .child(SecondaryText::new(tr("report_evidence_section")).px(12.0))
                            .child(RemoteEvidenceStrip::new("my-report-detail-evidence", evidence).size(84.0)),
                    )
                })
                .when(report.has_context, |column| {
                    column.child(label_row(
                        moderation_symbol("text.bubble.fill", 13.0, theme.primary._100),
                        MainText::new(tr("report_context_attached")).px(13.0).color(theme.text.secondary),
                    ))
                })
                .when_some(non_empty(&report.decision), |column, decision| {
                    column.child(
                        ModerationQuoteBlock::new(
                            tr("report_detail_verdict"),
                            decision,
                            theme.primary._100.opacity(0.1),
                            theme.clone(),
                        )
                        .title_color(theme.primary._100),
                    )
                })
                .when_some(report.resolved_action(), |column, action| {
                    column.child(label_row(
                        moderation_symbol(action.icon_name(), 13.0, action.tile_color()),
                        MainText::new(tr(action.title_key())).px(13.0).font_weight(FontWeight::SEMIBOLD),
                    ))
                })
                .when(report.content_deleted, |column| {
                    column.child(label_row(
                        moderation_symbol("eye.slash", 13.0, theme.semantic.error_100),
                        MainText::new(tr("report_detail_content_deleted")).px(13.0).color(theme.semantic.error_100),
                    ))
                })
                .when(report.reopened_at_ms.is_some(), |column| {
                    column.child(label_row(
                        moderation_symbol("arrow.counterclockwise.circle", 13.0, color(REOPEN_TINT)),
                        MainText::new(tr("report_reopened_content")).px(13.0).color(theme.text.secondary),
                    ))
                })
                .when_some(report.created_date_text(), |column, created_at| {
                    column.child(SecondaryText::new(created_at).px(12.0))
                })
                .child(spacer_min_h(16.0)),
        ))
    }
}
