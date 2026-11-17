use std::cell::Cell;
use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{
    AnyView, App, Context, ElementId, Entity, FontWeight, List, ListAlignment, ListState, Pixels, Render, SharedString,
    StyleRefinement, Subscription, Window, canvas, list,
};
use rise_canon::RiseLimits;
use rise_canon::rise_key::report;
use rise_i18n::{tr, tr_plural};
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::moderation::components::moderation_case_ui::RemoteEvidenceStrip;
use crate::modules::moderation::components::moderation_kit_ui::{
    ModerationFact, ModerationHero, ModerationMetrics, ModerationPersonRow, ModerationTint, moderation_child_id,
    moderation_symbol,
};
use crate::modules::moderation::components::moderation_type_tints::ModerationAuditActionTintExt;
use crate::modules::moderation::engine::rise_moderation_presentation::RiseModerationPresentation;
use crate::modules::moderation::engine::rise_moderation_wire::{
    AppealDTO, ReportRowDTO, RiseStoredCaseCapabilities, RiseStoredCaseTimelineEntry, RiseStoredReportCase,
    RiseStoredTargetIntel, RiseStoredTargetIpDossier,
};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AdminSanctionKind, AppealStatusExt, BlockTimeFormatting, ModerationAuditActionExt, ReportReasonExt, UserBlockType,
    UserBlockTypeExt,
};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;

/// bulky parts of a case, each a row on the case screen and a screen of its own
/// stacked inline they buried the decision
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaseMaterial {
    Reporters,
    Context,
    Target,
    Notes,
    Appeals,
    Timeline,
}

impl CaseMaterial {
    pub const ALL_CASES: [CaseMaterial; 6] = [
        CaseMaterial::Reporters,
        CaseMaterial::Context,
        CaseMaterial::Target,
        CaseMaterial::Notes,
        CaseMaterial::Appeals,
        CaseMaterial::Timeline,
    ];

    pub const fn raw_value(self) -> &'static str {
        match self {
            CaseMaterial::Reporters => "reporters",
            CaseMaterial::Context => "context",
            CaseMaterial::Target => "target",
            CaseMaterial::Notes => "notes",
            CaseMaterial::Appeals => "appeals",
            CaseMaterial::Timeline => "timeline",
        }
    }

    pub const fn id(self) -> &'static str {
        self.raw_value()
    }

    pub const fn title_key(self) -> &'static str {
        match self {
            CaseMaterial::Reporters => "case_reports_section",
            CaseMaterial::Context => "case_context_section",
            CaseMaterial::Target => "case_target_section",
            CaseMaterial::Notes => "case_notes_section",
            CaseMaterial::Appeals => "case_appeals_section",
            CaseMaterial::Timeline => "case_timeline_section",
        }
    }

    pub const fn icon_name(self) -> &'static str {
        match self {
            CaseMaterial::Reporters => "flag.fill",
            CaseMaterial::Context => "text.bubble.fill",
            CaseMaterial::Target => "person.text.rectangle.fill",
            CaseMaterial::Notes => "note.text",
            CaseMaterial::Appeals => "arrow.uturn.backward",
            CaseMaterial::Timeline => "clock.arrow.circlepath",
        }
    }

    pub fn tint(self, _theme: &AppTheme) -> gpui::Hsla {
        match self {
            CaseMaterial::Reporters => ChatTileColor::red(),
            CaseMaterial::Context => ChatTileColor::blue(),
            CaseMaterial::Target => ChatTileColor::indigo(),
            CaseMaterial::Notes => ChatTileColor::orange(),
            CaseMaterial::Appeals => ChatTileColor::teal(),
            CaseMaterial::Timeline => ChatTileColor::gray(),
        }
    }

    pub fn count(self, value: &RiseStoredReportCase) -> Option<i64> {
        match self {
            CaseMaterial::Reporters => Some((value.sibling_total + 1).max(value.all_reports().len() as i64)),
            CaseMaterial::Context => Some(value.context.len() as i64),
            CaseMaterial::Target => value.target.as_ref().map(|target| target.reports_received_total),
            CaseMaterial::Notes => (!value.notes.is_empty()).then_some(value.notes.len() as i64),
            CaseMaterial::Appeals => Some(value.appeals.len() as i64),
            CaseMaterial::Timeline => Some(value.timeline.len() as i64),
        }
    }

    pub fn available(value: &RiseStoredReportCase) -> Vec<CaseMaterial> {
        let mut result = vec![CaseMaterial::Reporters];
        if !value.context.is_empty() {
            result.push(CaseMaterial::Context);
        }
        if value.target.is_some() {
            result.push(CaseMaterial::Target);
        }
        if value.capabilities.can_note {
            result.push(CaseMaterial::Notes);
        }
        if !value.appeals.is_empty() {
            result.push(CaseMaterial::Appeals);
        }
        if !value.timeline.is_empty() {
            result.push(CaseMaterial::Timeline);
        }
        result
    }
}

fn push_route(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

/// reads the live case from the presentation and not a copy
/// so a note added here shows up without closing anything
#[derive(IntoElement)]
pub struct CaseMaterialSheet {
    material: CaseMaterial,
    report_id: i64,
    fallback: Option<RiseStoredReportCase>,
}

impl CaseMaterialSheet {
    pub fn new(material: CaseMaterial, report_id: i64) -> Self {
        Self { material, report_id, fallback: None }
    }

    pub fn fallback(mut self, fallback: impl Into<Option<RiseStoredReportCase>>) -> Self {
        self.fallback = fallback.into();
        self
    }

    fn material_body(material: CaseMaterial, value: &RiseStoredReportCase, theme: &AppTheme) -> Option<AnyElement> {
        let id = ElementId::from(SharedString::from(format!("case-material-{}-{}", material.id(), value.report.id)));
        match material {
            CaseMaterial::Reporters => Some(
                CaseReportersList::new(id, value.all_reports(), value.sibling_total + 1, theme.clone())
                    .into_any_element(),
            ),
            CaseMaterial::Context | CaseMaterial::Notes => None,
            CaseMaterial::Target => value.target.as_ref().map(|target| {
                CaseTargetPanel::new(
                    id,
                    target.clone(),
                    value.capabilities.clone(),
                    Some(value.report.id),
                    None,
                    theme.clone(),
                )
                .into_any_element()
            }),
            CaseMaterial::Appeals => Some(
                CaseAppealsList::new(
                    id,
                    value.appeals.clone(),
                    value.capabilities.can_resolve_appeals,
                    theme.clone(),
                )
                .into_any_element(),
            ),
            CaseMaterial::Timeline => {
                Some(CaseTimelineList::new(id, value.timeline.clone(), theme.clone()).into_any_element())
            }
        }
    }
}

impl RenderOnce for CaseMaterialSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let close = dismiss(cx);
        let live = RiseModerationPresentation::shared(cx).read(cx).cases().get(&self.report_id).cloned();
        let value = live.or(self.fallback);
        let scroll_id = SharedString::from(format!("case-material-scroll-{}-{}", self.material.id(), self.report_id));

        let content = match (self.material, value) {
            (CaseMaterial::Reporters, Some(value)) => {
                let id = ElementId::from(SharedString::from(format!("case-material-reporters-{}", value.report.id)));
                CaseReportersList::new(id, value.all_reports(), value.sibling_total + 1, theme.clone())
                    .virtual_list(ElementId::from(scroll_id), window, cx)
                    .list_shell_scroll_under_chrome(cx)
                    .bg(theme.bg._100)
                    .into_any_element()
            }
            (CaseMaterial::Timeline, Some(value)) => {
                let id = ElementId::from(SharedString::from(format!("case-material-timeline-{}", value.report.id)));
                CaseTimelineList::new(id, value.timeline.clone(), theme.clone())
                    .virtual_list(ElementId::from(scroll_id), window, cx)
                    .list_shell_scroll_under_chrome(cx)
                    .bg(theme.bg._100)
                    .into_any_element()
            }
            (material, value) => {
                let body = value.and_then(|value| Self::material_body(material, &value, &theme));
                scroll_view(ElementId::from(scroll_id))
                    .list_shell_scroll_under_chrome(cx)
                    .bg(theme.bg._100)
                    .child(
                        vstack(ModerationMetrics::SECTION_SPACING)
                            .w_full()
                            .pt(px(12.0))
                            .pb(px(ModerationMetrics::BOTTOM_INSET))
                            .children(body),
                    )
                    .into_any_element()
            }
        };

        SettingsWrapperUi::new(tr(self.material.title_key()), move |window, cx| close.call_as_function(window, cx))
            .content(content)
    }
}

#[derive(IntoElement)]
pub struct CaseReportersList {
    id: ElementId,
    reports: Vec<ReportRowDTO>,
    total: i64,
    theme: AppTheme,
}

impl CaseReportersList {
    pub fn new(id: impl Into<ElementId>, reports: Vec<ReportRowDTO>, total: i64, theme: AppTheme) -> Self {
        Self { id: id.into(), reports, total, theme }
    }

    fn items(&self, report: &ReportRowDTO, nav_store: &Option<Entity<NavigationStore>>) -> Vec<GroupedBtn> {
        let theme = &self.theme;
        let author = report.author.as_ref();
        let route = RootRoute::UserPage {
            user_id: report.report_by_id.clone(),
            tag: author.and_then(|author| author.tag.clone()),
        };
        let press_store = nav_store.clone();
        let author_row = plain(moderation_child_id(&self.id, format!("author-{}", report.id)))
            .w_full()
            .on_press(move |_, cx| push_route(&press_store, route.clone(), cx))
            .child(
                ModerationPersonRow::new(
                    moderation_child_id(&self.id, format!("person-{}", report.id)),
                    author.map_or_else(|| tr("moderation_unknown_reporter"), |author| author.display_name()),
                )
                .logo_source(author.and_then(|author| author.logo.clone()).map(SharedString::from))
                .user_id(report.report_by_id.clone().map(SharedString::from))
                .subtitle(report.created_date_text().map(SharedString::from))
                .is_official(author.is_some_and(|author| author.is_official))
                .is_premium(author.is_some_and(|author| author.is_premium))
                .trailing(div().children(moderation_symbol("chevron.right", 12.0, theme.text.secondary.opacity(0.75)))),
            );

        let mut items = vec![
            GroupedBtn::new(format!("author-{}", report.id), "").show_chevron(false).content_slot(author_row),
        ];

        if let Some(reason) = report.resolved_reason() {
            items.push(ModerationFact::row(
                format!("reason-{}", report.id),
                reason.icon_name(),
                ChatTileColor::gray(),
                tr("case_reason_label"),
                tr(reason.title_key()),
                false,
                None,
            ));
        }

        if !report.description.is_empty() {
            items.push(ModerationFact::row(
                format!("text-{}", report.id),
                "text.quote",
                ChatTileColor::gray(),
                tr("profile_field_description"),
                report.description.clone(),
                true,
                None,
            ));
        }

        let evidence = report.evidence();
        if !evidence.is_empty() {
            items.push(
                GroupedBtn::new(format!("evidence-{}", report.id), "").show_chevron(false).content_slot(
                    vstack(8.0)
                        .align(HorizontalAlignment::Leading)
                        .w_full()
                        .px(px(16.0))
                        .py(px(12.0))
                        .child(MainText::new(tr("report_evidence_section")).px(12.0).color(theme.text.secondary))
                        .child(
                            RemoteEvidenceStrip::new(
                                moderation_child_id(&self.id, format!("evidence-strip-{}", report.id)),
                                evidence,
                            )
                            .size(92.0),
                        ),
                ),
            );
        }

        items
    }

    fn card(&self, report: &ReportRowDTO, nav_store: &Option<Entity<NavigationStore>>) -> AnyElement {
        GroupedBtns::new(moderation_child_id(&self.id, format!("report-{}", report.id)), self.items(report, nav_store))
            .separator_leading_padding(16.0)
            .into_any_element()
    }

    fn remainder(&self) -> i64 {
        self.total - self.reports.len() as i64
    }

    fn remainder_footer(remainder: i64) -> AnyElement {
        div()
            .w_full()
            .child(
                SecondaryText::new(tr("case_reports_more").replace("{n}", &RiseFormat::compact_count(remainder)))
                    .px(13.0)
                    .tac(TextAlignT::Center),
            )
            .into_any_element()
    }

    pub fn virtual_list(self, id: ElementId, window: &mut Window, cx: &mut App) -> List {
        let nav_store = env::<NavigationStoreKey>(cx);
        let remainder = self.remainder();
        let item_count = self.reports.len() + usize::from(remainder > 0);
        let report_ids: Vec<i64> = self.reports.iter().map(|report| report.id).collect();
        let state = window.use_keyed_state(id, cx, |_, _| CaseReportersListState {
            list: ListState::new(0, ListAlignment::Top, px(CaseReportersListState::OVERDRAW)),
            report_ids: Vec::new(),
            item_count: 0,
        });
        let list_state = state.update(cx, |state, _| {
            if state.report_ids != report_ids || state.item_count != item_count {
                state.list.reset(item_count);
                state.report_ids = report_ids;
                state.item_count = item_count;
            }
            state.list.clone()
        });
        let this = Rc::new(self);
        list(list_state, move |index, window, cx| {
            let body = match this.reports.get(index) {
                Some(report) => {
                    let card = window.use_keyed_state(
                        moderation_child_id(&this.id, format!("card-view-{}", report.id)),
                        cx,
                        |_, cx| CaseReporterCardView::new(this.id.clone(), report.clone(), nav_store.clone(), cx),
                    );
                    card.update(cx, |view, cx| view.set_report(report, cx));
                    CaseReporterCardView::slot(&card, cx)
                }
                None => Self::remainder_footer(remainder),
            };
            let bottom =
                if index + 1 == item_count { ModerationMetrics::BOTTOM_INSET } else { ModerationMetrics::SECTION_SPACING };
            div()
                .w_full()
                .when(index == 0, |item| item.pt(px(12.0)))
                .pb(px(bottom))
                .child(body)
                .into_any_element()
        })
        .size_full()
    }
}

/// one reporter card as its own cached view
/// uncached, every wheel frame built and laid out each visible card again
/// cached, a card renders only when its report or the theme changes
struct CaseReporterCardView {
    list_id: ElementId,
    report: ReportRowDTO,
    nav_store: Option<Entity<NavigationStore>>,
    height: Rc<Cell<Option<Pixels>>>,
    _theme: Subscription,
}

impl CaseReporterCardView {
    fn new(
        list_id: ElementId,
        report: ReportRowDTO,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme_store = ThemeStore::shared(cx);
        Self {
            list_id,
            report,
            nav_store,
            height: Rc::new(Cell::new(None)),
            _theme: cx.observe(&theme_store, |_, _, cx| cx.notify()),
        }
    }

    fn set_report(&mut self, report: &ReportRowDTO, cx: &mut Context<Self>) {
        if self.report != *report {
            self.report = report.clone();
            cx.notify();
        }
    }

    /// cached card once its height is known, first render is plain and measures
    /// list renders items again on the next wheel frame which takes the cached slot
    fn slot(card: &Entity<Self>, cx: &App) -> AnyElement {
        let view = AnyView::from(card.clone());
        match card.read(cx).height.get() {
            Some(height) => div()
                .w_full()
                .child(view.cached(StyleRefinement::default().w_full().h(height)).replay_when_moved())
                .into_any_element(),
            None => div().w_full().child(view).into_any_element(),
        }
    }
}

impl Render for CaseReporterCardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = CaseReportersList::new(self.list_id.clone(), Vec::new(), 0, rise_ui::theme(cx).clone());
        let height = self.height.clone();
        div().relative().w_full().child(list.card(&self.report, &self.nav_store)).child(
            canvas(
                move |bounds, _, _| {
                    let measured = bounds.size.height;
                    if measured > px(0.0) {
                        height.set(Some(measured));
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
    }
}

/// keyed state of the reporters list, scroll position and measured card heights
struct CaseReportersListState {
    list: ListState,
    report_ids: Vec<i64>,
    item_count: usize,
}

impl CaseReportersListState {
    const OVERDRAW: f32 = 400.0;
}

impl RenderOnce for CaseReportersList {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let cards: Vec<AnyElement> = self.reports.iter().map(|report| self.card(report, &nav_store)).collect();
        let remainder = self.remainder();

        vstack(ModerationMetrics::SECTION_SPACING)
            .w_full()
            .children(cards)
            .children((remainder > 0).then(|| Self::remainder_footer(remainder)))
    }
}

#[derive(IntoElement)]
pub struct CaseTargetPanel {
    id: ElementId,
    intel: RiseStoredTargetIntel,
    capabilities: RiseStoredCaseCapabilities,
    report_id: Option<i64>,
    appeal_id: Option<i64>,
    theme: AppTheme,
}

struct PendingIpAction {
    dossier: RiseStoredTargetIpDossier,
    kind: AdminSanctionKind,
}

impl PendingIpAction {
    fn present(self, user_id: String, report_id: Option<i64>, appeal_id: Option<i64>, cx: &mut App) {
        let title = tr(self.kind.title_key());
        let reason = title.clone();
        let kind = self.kind;
        let ip = self.dossier.ip.clone();
        NativeConfirmAlert::present_delete(
            &title,
            &self.dossier.ip,
            Some(tr("moderation_confirm_button")),
            None,
            false,
            None,
            None,
            move |cx| {
                ModerationInteractionsStore::shared(cx)
                    .update(cx, |store, cx| {
                        store.admin_sanction(
                            kind,
                            Some(&user_id),
                            None,
                            Some(0),
                            Some(&ip),
                            Some(&reason),
                            report_id,
                            appeal_id,
                            cx,
                        )
                    })
                    .detach();
            },
            cx,
        );
    }
}

impl CaseTargetPanel {
    pub fn new(
        id: impl Into<ElementId>,
        intel: RiseStoredTargetIntel,
        capabilities: RiseStoredCaseCapabilities,
        report_id: Option<i64>,
        appeal_id: Option<i64>,
        theme: AppTheme,
    ) -> Self {
        Self { id: id.into(), intel, capabilities, report_id, appeal_id, theme }
    }

    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        moderation_child_id(&self.id, name)
    }

    fn counter_items(&self) -> Vec<GroupedBtn> {
        let intel = &self.intel;
        let mut items = vec![
            ModerationFact::row(
                "received",
                "flag.fill",
                if intel.reports_received_total > 2 { ChatTileColor::red() } else { ChatTileColor::gray() },
                tr("case_stat_reports_received"),
                format!(
                    "{} \u{b7} {} {}",
                    RiseFormat::compact_count(intel.reports_received_total),
                    tr("case_stat_reports_open").to_lowercase(),
                    RiseFormat::compact_count(intel.reports_received_open)
                ),
                false,
                None,
            ),
            ModerationFact::row(
                "sanctions",
                "nosign",
                if intel.sanctions_total > 0 { ChatTileColor::red() } else { ChatTileColor::gray() },
                tr("case_stat_sanctions"),
                RiseFormat::compact_count(intel.sanctions_total),
                false,
                None,
            ),
            ModerationFact::row(
                "appeals",
                "arrow.uturn.backward",
                if intel.appeals_accepted > 0 { ChatTileColor::green() } else { ChatTileColor::gray() },
                tr("case_stat_appeals_accepted"),
                RiseFormat::compact_count(intel.appeals_accepted),
                false,
                None,
            ),
            ModerationFact::row(
                "filed",
                "paperplane.fill",
                ChatTileColor::gray(),
                tr("case_stat_reports_filed"),
                RiseFormat::compact_count(intel.reports_filed_total),
                false,
                None,
            ),
        ];
        if let Some(offenses) = intel.offense_counters.as_ref().filter(|offenses| offenses.warnings > 0) {
            items.push(ModerationFact::row(
                "warnings",
                "exclamationmark.triangle.fill",
                ChatTileColor::orange(),
                tr("case_stat_warnings"),
                RiseFormat::compact_count(offenses.warnings),
                false,
                None,
            ));
        }
        items
    }

    fn block_items(&self, nav_store: &Option<Entity<NavigationStore>>) -> Vec<GroupedBtn> {
        self.intel
            .active_blocks
            .iter()
            .map(|block| {
                let block_type = block.resolved_type();
                let value = if block.is_permanent() {
                    tr("blocks_permanent")
                } else if let Some(remaining) = BlockTimeFormatting::remaining(Some(block.block_end_timestamp)) {
                    tr("blocks_lifts").replace("{when}", &remaining)
                } else {
                    tr(block_type.title_key())
                };
                let on_tap = block.report_id.map(|linked| {
                    let nav_store = nav_store.clone();
                    Rc::new(move |_: &mut Window, cx: &mut App| {
                        push_route(&nav_store, RootRoute::ReportDetail { report_id: linked.to_string() }, cx);
                    }) as ButtonAction
                });
                ModerationFact::row(
                    block.id(),
                    block_type.icon_name(),
                    ModerationTint::block(block_type),
                    tr(block_type.title_key()),
                    value,
                    false,
                    on_tap,
                )
            })
            .collect()
    }

    fn ip_section(&self, is_sanctioning: bool) -> AnyElement {
        if self.intel.ip_dossiers.is_empty() {
            let text =
                if self.intel.ip_dossiers_available { tr("case_ip_empty") } else { tr("case_ip_unavailable") };
            return GroupedBtns::new(
                self.child_id("ip-empty"),
                vec![GroupedBtn::new("ipEmpty", text).icon("network").icon_background(ChatTileColor::gray()).show_chevron(false)],
            )
            .title(tr("case_ip_section").to_uppercase())
            .into_any_element();
        }
        let items = self
            .intel
            .ip_dossiers
            .iter()
            .map(|dossier| {
                GroupedBtn::new(dossier.id(), "").show_chevron(false).content_slot(self.ip_row(dossier, is_sanctioning))
            })
            .collect();
        let count = self.intel.ip_dossiers.len() as i64;
        let truncated = (count >= *IP_DOSSIER_CEILING).then(|| {
            div().w_full().child(
                SecondaryText::new(tr("case_timeline_truncated").replace("{n}", &RiseFormat::compact_count(count)))
                    .px(13.0)
                    .tac(TextAlignT::Center),
            )
        });
        vstack(ModerationMetrics::SECTION_SPACING)
            .w_full()
            .child(
                GroupedBtns::new(self.child_id("ip-section"), items)
                    .title(tr("case_ip_section").to_uppercase())
                    .separator_leading_padding(16.0),
            )
            .children(truncated)
            .into_any_element()
    }

    fn ip_row(&self, dossier: &RiseStoredTargetIpDossier, is_sanctioning: bool) -> impl IntoElement {
        let theme = &self.theme;
        let pending_dossier = dossier.clone();
        let kind = if dossier.is_blocked { AdminSanctionKind::UnblockIp } else { AdminSanctionKind::BlockIp };
        let user_id = self.intel.user_id.clone();
        let report_id = self.report_id;
        let appeal_id = self.appeal_id;

        hstack(12.0)
            .w_full()
            .px(px(16.0))
            .py(px(12.0))
            .child(
                vstack(3.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w(px(0.0))
                    .child(MainText::new(dossier.ip.clone()).px(15.0).font_weight(FontWeight::SEMIBOLD))
                    .child(
                        MainText::new(Self::ip_caption(dossier))
                            .px(12.0)
                            .color(theme.text.secondary)
                            .number_of_lines(1),
                    ),
            )
            .child(
                plain(self.child_id(format!("ip-action-{}", dossier.ip)))
                    .disabled(is_sanctioning)
                    .on_press(move |_, cx| {
                        PendingIpAction { dossier: pending_dossier.clone(), kind }.present(
                            user_id.clone(),
                            report_id,
                            appeal_id,
                            cx,
                        );
                    })
                    .child(
                        div().px(px(12.0)).py(px(7.0)).bg(theme.bg._100).rounded_full().child(
                            MainText::new(tr(if dossier.is_blocked {
                                "admin_sanction_unblock_ip"
                            } else {
                                "admin_sanction_block_ip"
                            }))
                            .px(13.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(if dossier.is_blocked { ChatTileColor::green() } else { theme.semantic.error_100 }),
                        ),
                    ),
            )
    }

    fn ip_caption(dossier: &RiseStoredTargetIpDossier) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(seen) = BlockTimeFormatting::since_relative(Some(dossier.last_seen_at_ms)) {
            parts.push(seen);
        }
        parts.push(format!("\u{d7}{}", RiseFormat::compact_count(dossier.observation_count)));
        if dossier.linked_account_count > 1 {
            parts.push(tr_plural("case_ip_linked", dossier.linked_account_count));
        }
        if dossier.is_blocked {
            parts.push(tr("case_ip_blocked"));
        }
        parts.join(" \u{b7} ")
    }
}

impl RenderOnce for CaseTargetPanel {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let close = dismiss(cx);
        let is_sanctioning = RiseModerationPresentation::shared(cx).read(cx).is_sanctioning();
        let theme = self.theme.clone();
        let user = self.intel.user.as_ref();
        let user_id = self.intel.user_id.clone();
        let user_tag = user.and_then(|user| user.tag.clone());

        let logo_store = nav_store.clone();
        let logo_user_id = user_id.clone();
        let hero = ModerationHero::new(
            self.child_id("hero"),
            user.map_or_else(|| tr("moderation_unknown_reporter"), |user| user.display_name()),
            UserLogoUi::new(self.child_id("logo"))
                .source(user.and_then(|user| user.logo.clone()).map(SharedString::from))
                .name(user.and_then(|user| user.name.clone()).map(SharedString::from))
                .user_id(Some(SharedString::from(user_id.clone())))
                .size(ModerationMetrics::HERO_SIDE)
                .is_button(true)
                .on_press(move |_, cx| {
                    let route = RootRoute::UserPage { user_id: Some(logo_user_id.clone()), tag: user_tag.clone() };
                    push_route(&logo_store, route, cx);
                }),
        )
        .subtitle(user.and_then(|user| user.tag.as_ref()).map(|tag| SharedString::from(format!("@{tag}"))));

        let queue_store = nav_store.clone();
        let all_reports = GroupedBtn::new("allReports", tr("case_target_all_reports"))
            .icon("tray.full.fill")
            .icon_background(theme.primary._100)
            .callback(move |window, cx| {
                ModerationInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.set_queue_target(Some(&user_id), cx));
                close.call_as_function(window, cx);
                push_route(&queue_store, RootRoute::ReportsQueue, cx);
            });

        vstack(ModerationMetrics::SECTION_SPACING)
            .w_full()
            .child(hero)
            .child(
                GroupedBtns::new(self.child_id("counters"), self.counter_items()).separator_leading_padding(16.0),
            )
            .children((!self.intel.active_blocks.is_empty()).then(|| {
                GroupedBtns::new(self.child_id("blocks"), self.block_items(&nav_store)).separator_leading_padding(16.0)
            }))
            .child(GroupedBtns::new(self.child_id("all-reports"), vec![all_reports]))
            .children(self.capabilities.is_admin.then(|| self.ip_section(is_sanctioning)))
    }
}

#[derive(IntoElement)]
pub struct CaseAppealsList {
    id: ElementId,
    appeals: Vec<AppealDTO>,
    can_open: bool,
    theme: AppTheme,
}

impl CaseAppealsList {
    pub fn new(id: impl Into<ElementId>, appeals: Vec<AppealDTO>, can_open: bool, theme: AppTheme) -> Self {
        Self { id: id.into(), appeals, can_open, theme }
    }

    fn items(
        &self,
        appeal: &AppealDTO,
        nav_store: &Option<Entity<NavigationStore>>,
        close: &DismissAction,
    ) -> Vec<GroupedBtn> {
        let status = appeal.resolved_status();
        let on_tap = self.can_open.then(|| {
            let nav_store = nav_store.clone();
            let close = close.clone();
            let appeal_id = appeal.id;
            Rc::new(move |window: &mut Window, cx: &mut App| {
                close.call_as_function(window, cx);
                push_route(&nav_store, RootRoute::AppealReview { appeal_id: appeal_id.to_string() }, cx);
            }) as ButtonAction
        });
        let mut items = vec![ModerationFact::row(
            format!("status-{}", appeal.id),
            status.icon_name(),
            ModerationTint::appeal(status, &self.theme),
            appeal.created_date_text().unwrap_or_else(|| tr("appeal_review_title")),
            tr(status.title_key()),
            false,
            on_tap,
        )];

        if !appeal.text.is_empty() {
            items.push(ModerationFact::row(
                format!("text-{}", appeal.id),
                "text.quote",
                ChatTileColor::gray(),
                tr("appeal_review_appeal_text"),
                appeal.text.clone(),
                true,
                None,
            ));
        }

        if let Some(decision) = appeal.decision_text.as_ref().filter(|decision| !decision.is_empty()) {
            items.push(ModerationFact::row(
                format!("verdict-{}", appeal.id),
                "checkmark.seal.fill",
                ModerationTint::appeal(status, &self.theme),
                tr("appeals_verdict"),
                decision.clone(),
                true,
                None,
            ));
        }

        items
    }
}

impl RenderOnce for CaseAppealsList {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let close = dismiss(cx);
        let cards: Vec<AnyElement> = self
            .appeals
            .iter()
            .map(|appeal| {
                GroupedBtns::new(
                    moderation_child_id(&self.id, format!("appeal-{}", appeal.id)),
                    self.items(appeal, &nav_store, &close),
                )
                .separator_leading_padding(16.0)
                .into_any_element()
            })
            .collect();
        vstack(ModerationMetrics::SECTION_SPACING).w_full().children(cards)
    }
}

static IP_DOSSIER_CEILING: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("report_case_ip_dossiers"));

static SERVER_CEILING: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("report_case_timeline"));

#[derive(IntoElement)]
pub struct CaseTimelineList {
    id: ElementId,
    entries: Vec<RiseStoredCaseTimelineEntry>,
    theme: AppTheme,
}

impl CaseTimelineList {
    pub fn new(id: impl Into<ElementId>, entries: Vec<RiseStoredCaseTimelineEntry>, theme: AppTheme) -> Self {
        Self { id: id.into(), entries, theme }
    }

    fn entry_row(entry: &RiseStoredCaseTimelineEntry, theme: &AppTheme) -> GroupedBtn {
        let action = entry.resolved_action();
        let mut row =
            GroupedBtn::new(entry.id.to_string(), action.map_or_else(|| entry.action.clone(), |action| tr(action.title_key())))
                .icon(action.map_or("circle", |action| action.icon_name()))
                .icon_background(action.map_or_else(ChatTileColor::gray, |action| action.tint(theme)))
                .subtitle(Self::subtitle(entry))
                .subtitle_line_limit(2)
                .show_chevron(false);
        if let Some(since) = BlockTimeFormatting::since_relative(Some(entry.created_at_ms)) {
            row = row.pretitle(since);
        }
        row
    }

    fn entry_list(&self) -> GroupedBtns {
        let items = self.entries.iter().map(|entry| Self::entry_row(entry, &self.theme)).collect();
        GroupedBtns::new(moderation_child_id(&self.id, "entries"), items)
    }

    fn truncated_footer(count: i64) -> AnyElement {
        div()
            .w_full()
            .child(
                SecondaryText::new(tr("case_timeline_truncated").replace("{n}", &RiseFormat::compact_count(count)))
                    .px(13.0)
                    .tac(TextAlignT::Center),
            )
            .into_any_element()
    }

    pub fn virtual_list(self, id: ElementId, window: &mut Window, cx: &mut App) -> List {
        let count = self.entries.len() as i64;
        let truncated = count >= *SERVER_CEILING;
        let entry_count = self.entries.len();
        let item_count = entry_count + usize::from(truncated);
        let entry_ids: Vec<i64> = self.entries.iter().map(|entry| entry.id).collect();
        let state = window.use_keyed_state(id, cx, |_, _| CaseTimelineListState {
            list: ListState::new(0, ListAlignment::Top, px(CaseTimelineListState::OVERDRAW)),
            entry_ids: Vec::new(),
            item_count: 0,
        });
        let list_state = state.update(cx, |state, _| {
            if state.entry_ids != entry_ids || state.item_count != item_count {
                state.list.reset(item_count);
                state.entry_ids = entry_ids;
                state.item_count = item_count;
            }
            state.list.clone()
        });
        let this = Rc::new(self);
        list(list_state, move |index, window, cx| {
            let body = match this.entries.get(index) {
                Some(entry) => {
                    let place = CaseTimelinePlace { first: index == 0, last: index + 1 == entry_count };
                    let row = window.use_keyed_state(
                        moderation_child_id(&this.id, format!("row-view-{}", entry.id)),
                        cx,
                        |_, cx| CaseTimelineRowView::new(this.id.clone(), entry.clone(), place, cx),
                    );
                    row.update(cx, |view, cx| view.set_entry(entry, place, cx));
                    CaseTimelineRowView::slot(&row, cx)
                }
                None => Self::truncated_footer(count),
            };
            let is_last = index + 1 == item_count;
            div()
                .w_full()
                .when(index == 0, |item| item.pt(px(12.0)))
                .when(truncated && is_last, |item| item.pt(px(ModerationMetrics::SECTION_SPACING)))
                .when(is_last, |item| item.pb(px(ModerationMetrics::BOTTOM_INSET)))
                .child(body)
                .into_any_element()
        })
        .size_full()
    }
}

/// where a row sits in the grouped card, ends get rounded corners and the rest a separator
#[derive(Clone, Copy, PartialEq, Eq)]
struct CaseTimelinePlace {
    first: bool,
    last: bool,
}

/// one timeline row as its own cached view
/// renders only when its entry, place or the theme changes
struct CaseTimelineRowView {
    list_id: ElementId,
    entry: RiseStoredCaseTimelineEntry,
    place: CaseTimelinePlace,
    height: Rc<Cell<Option<Pixels>>>,
    _theme: Subscription,
}

impl CaseTimelineRowView {
    fn new(
        list_id: ElementId,
        entry: RiseStoredCaseTimelineEntry,
        place: CaseTimelinePlace,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme_store = ThemeStore::shared(cx);
        Self {
            list_id,
            entry,
            place,
            height: Rc::new(Cell::new(None)),
            _theme: cx.observe(&theme_store, |_, _, cx| cx.notify()),
        }
    }

    fn set_entry(&mut self, entry: &RiseStoredCaseTimelineEntry, place: CaseTimelinePlace, cx: &mut Context<Self>) {
        if self.entry != *entry || self.place != place {
            self.entry = entry.clone();
            self.place = place;
            cx.notify();
        }
    }

    /// cached row once its height is known, first render is plain and measures
    fn slot(row: &Entity<Self>, cx: &App) -> AnyElement {
        let view = AnyView::from(row.clone());
        match row.read(cx).height.get() {
            Some(height) => div()
                .w_full()
                .child(view.cached(StyleRefinement::default().w_full().h(height)).replay_when_moved())
                .into_any_element(),
            None => div().w_full().child(view).into_any_element(),
        }
    }
}

impl Render for CaseTimelineRowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let row = CaseTimelineList::entry_row(&self.entry, &theme);
        let place = self.place;
        let radius = px(GroupedCardMetrics::CORNER_RADIUS);
        let group = GroupedBtns::new(moderation_child_id(&self.list_id, format!("entry-{}", self.entry.id)), vec![row])
            .horizontal_padding(0.0)
            .corner_radius(0.0)
            .surface_color(gpui::transparent_black());
        let separator = (!place.last).then(|| {
            div()
                .w_full()
                .padding_edges(EdgeSet::LEADING, GroupedCardMetrics::content_leading_inset())
                .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)))
        });
        let height = self.height.clone();
        div()
            .relative()
            .w_full()
            .padding_edges(EdgeSet::HORIZONTAL, GroupedCardMetrics::HORIZONTAL_PADDING)
            .child(
                div()
                    .w_full()
                    .bg(theme.bg._200)
                    .when(place.first, |slice| slice.rounded_t(radius))
                    .when(place.last, |slice| slice.rounded_b(radius))
                    .child(group)
                    .children(separator),
            )
            .child(
                canvas(
                    move |bounds, _, _| {
                        let measured = bounds.size.height;
                        if measured > px(0.0) {
                            height.set(Some(measured));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}

/// keyed state of the timeline list, scroll position and measured row heights
struct CaseTimelineListState {
    list: ListState,
    entry_ids: Vec<i64>,
    item_count: usize,
}

impl CaseTimelineListState {
    const OVERDRAW: f32 = 400.0;
}

impl CaseTimelineList {

    fn subtitle(entry: &RiseStoredCaseTimelineEntry) -> String {
        let mut parts = vec![entry.actor.as_ref().map_or_else(|| entry.actor_id.clone(), |actor| actor.display_name())];
        let facts = entry.details_facts();
        if let Some(block_type) = facts
            .string(report::BLOCK_TYPE)
            .and_then(|block_type| UserBlockType::from_wire(&block_type.to_uppercase()))
        {
            parts.push(tr(block_type.title_key()));
        }
        if let Some(days) = facts.int(report::DURATION_DAYS).or_else(|| facts.int(report::BLOCK_DURATION_DAYS)) {
            parts.push(if days == 0 {
                tr("blocks_permanent")
            } else {
                tr("case_days").replace("{n}", &days.to_string())
            });
        }
        if let Some(reason) = facts.string(report::REASON).or_else(|| facts.string(report::DECISION)) {
            parts.push(reason);
        }
        if let Some(ip) = facts.string(report::IP) {
            parts.push(ip);
        }
        parts.join(" \u{b7} ")
    }
}

impl RenderOnce for CaseTimelineList {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let count = self.entries.len() as i64;
        vstack(ModerationMetrics::SECTION_SPACING)
            .w_full()
            .child(self.entry_list())
            .children((count >= *SERVER_CEILING).then(|| Self::truncated_footer(count)))
    }
}
