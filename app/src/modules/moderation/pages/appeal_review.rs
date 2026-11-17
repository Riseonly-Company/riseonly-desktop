use std::rc::Rc;

use gpui::{AnyElement, App, Context, ElementId, Entity, Hsla, SharedString, Subscription, Task, Window};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::moderation::components::moderation_case_ui::RemoteEvidenceStrip;
use crate::modules::moderation::components::moderation_kit_ui::{
    ModerationCaseSkeleton, ModerationEmptyState, ModerationFact, ModerationFailureState, ModerationHero,
    ModerationMetrics, ModerationPersonRow, ModerationTint, moderation_child_id, moderation_symbol,
};
use crate::modules::moderation::components::moderation_type_tints::ReportTargetTypeTintExt;
use crate::modules::moderation::engine::rise_moderation_presentation::{
    ModerationLoadState, RiseModerationPresentation,
};
use crate::modules::moderation::engine::rise_moderation_wire::{RiseStoredAppealCase, RiseStoredReportCase};
use crate::modules::moderation::pages::moderation_case_sections::{CaseMaterial, CaseMaterialSheet};
use crate::modules::moderation::pages::moderation_case_sheets::{AdminSanctionSheet, AppealVerdictSheet};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AdminSanctionKind, AppealDecision, AppealDecisionExt, AppealStatus, AppealStatusExt, ReportReasonExt,
    UserBlockType, UserBlockTypeExt,
};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;

const SEPARATOR_LEADING_PADDING: f32 = 16.0;
const EVIDENCE_TILE: f32 = 96.0;
const LIST_SEPARATOR: &str = " \u{b7} ";

struct AppealReviewState {
    pending_decision: Option<AppealDecision>,
    pending_admin: Option<AdminSanctionKind>,
    open_sheet: Option<CaseMaterial>,
    _observation: Subscription,
}

impl AppealReviewState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseModerationPresentation::shared(cx);
        Self {
            pending_decision: None,
            pending_admin: None,
            open_sheet: None,
            _observation: cx.observe(&presentation, |_, _, cx| cx.notify()),
        }
    }
}

#[derive(IntoElement)]
pub struct AppealReview {
    appeal_id: String,
}

impl AppealReview {
    pub fn new(appeal_id: impl Into<String>) -> Self {
        Self { appeal_id: appeal_id.into() }
    }

    fn element_id(&self, name: impl Into<SharedString>) -> ElementId {
        appeal_element_id(&self.appeal_id, name)
    }

    fn open_case(resolved_id: Option<i64>, cx: &mut App) {
        if let Some(id) = resolved_id {
            ModerationInteractionsStore::shared(cx).update(cx, |store, cx| store.open_appeal_case(id, cx));
        }
    }

    fn appeal_case(resolved_id: Option<i64>, cx: &mut App) -> Option<RiseStoredAppealCase> {
        let id = resolved_id?;
        RiseModerationPresentation::shared(cx).read(cx).appeal_cases().get(&id).cloned()
    }

    /// sheet is shown while the state holds an item, dismissing it clears the item
    fn item_binding(
        state: &Entity<AppealReviewState>,
        is_shown: impl Fn(&AppealReviewState) -> bool + 'static,
        clear: impl Fn(&mut AppealReviewState) + 'static,
    ) -> Binding<bool> {
        let reader = state.clone();
        let writer = state.clone();
        Binding::new(
            move |cx| is_shown(reader.read(cx)),
            move |shown, _, cx| {
                if !shown {
                    writer.update(cx, |state, cx| {
                        clear(state);
                        cx.notify();
                    });
                }
            },
        )
    }

    fn material_sheet(&self, state: &Entity<AppealReviewState>, resolved_id: Option<i64>) -> AnyElement {
        let content_state = state.clone();
        NativeBottomSheetPresentationHost::new(
            self.element_id("material-sheet"),
            Self::item_binding(state, |state| state.open_sheet.is_some(), |state| state.open_sheet = None),
            move |_, cx| {
                let material = content_state.read(cx).open_sheet;
                let report_case = Self::appeal_case(resolved_id, cx).and_then(|value| value.report_case);
                match (material, report_case) {
                    (Some(material), Some(report_case)) => {
                        CaseMaterialSheet::new(material, report_case.report.id)
                            .fallback(Some(report_case))
                            .into_any_element()
                    }
                    _ => div().into_any_element(),
                }
            },
        )
        .presentation_detents([PresentationDetent::Large])
        .presentation_drag_indicator(true)
        .into_any_element()
    }

    fn verdict_sheet(&self, state: &Entity<AppealReviewState>, resolved_id: Option<i64>) -> AnyElement {
        let content_state = state.clone();
        NativeBottomSheetPresentationHost::new(
            self.element_id("verdict-sheet"),
            Self::item_binding(
                state,
                |state| state.pending_decision.is_some(),
                |state| state.pending_decision = None,
            ),
            move |_, cx| {
                let decision = content_state.read(cx).pending_decision;
                let (Some(decision), Some(id)) = (decision, resolved_id) else {
                    return div().into_any_element();
                };
                let resolved = content_state.clone();
                AppealVerdictSheet::new(id, decision, move |_, cx| {
                    resolved.update(cx, |state, cx| {
                        state.pending_decision = None;
                        cx.notify();
                    });
                })
                .into_any_element()
            },
        )
        .presentation_detents([PresentationDetent::Medium, PresentationDetent::Large])
        .presentation_drag_indicator(true)
        .into_any_element()
    }

    fn admin_sheet(&self, state: &Entity<AppealReviewState>, resolved_id: Option<i64>) -> AnyElement {
        let content_state = state.clone();
        NativeBottomSheetPresentationHost::new(
            self.element_id("admin-sheet"),
            Self::item_binding(state, |state| state.pending_admin.is_some(), |state| state.pending_admin = None),
            move |_, cx| {
                let kind = content_state.read(cx).pending_admin;
                let (Some(kind), Some(value)) = (kind, Self::appeal_case(resolved_id, cx)) else {
                    return div().into_any_element();
                };
                let report = value.report_case.as_ref().map(|report_case| &report_case.report);
                let initial_block_type = (kind == AdminSanctionKind::Unblock)
                    .then(|| {
                        value
                            .appeal
                            .block_type
                            .as_deref()
                            .or(report.and_then(|report| report.block_type.as_deref()))
                            .map(|raw| UserBlockType::parse_or_default(Some(raw)))
                    })
                    .flatten();
                let done = content_state.clone();
                AdminSanctionSheet::new(
                    kind,
                    Some(value.appeal.appellant_id.clone()),
                    value.appellant.as_ref().map(|appellant| appellant.display_name()),
                    report.map(|report| report.id),
                    Some(value.appeal.id),
                    move |_, cx| {
                        done.update(cx, |state, cx| {
                            state.pending_admin = None;
                            cx.notify();
                        });
                    },
                )
                .initial_block_type(initial_block_type)
                .into_any_element()
            },
        )
        .presentation_detents([PresentationDetent::Large])
        .presentation_drag_indicator(true)
        .into_any_element()
    }
}

fn appeal_element_id(appeal_id: &str, name: impl Into<SharedString>) -> ElementId {
    moderation_child_id(&ElementId::Name(SharedString::from(format!("appeal-review-{appeal_id}"))), name)
}

impl RenderOnce for AppealReview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(self.element_id("state"), cx, |_, cx| AppealReviewState::new(cx));
        let resolved_id = self.appeal_id.parse::<i64>().ok();
        let back_store = nav_store.clone();

        let wrapper = SettingsWrapperUi::new(tr("appeal_review_title"), move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .subtitle(SharedString::from(format!("#{}", self.appeal_id)))
        .content(AppealReviewBody {
            appeal_id: self.appeal_id.clone(),
            resolved_id,
            state: state.clone(),
            nav_store,
        });

        div()
            .size_full()
            .relative()
            .bg(background)
            .child(PageRefreshKeys::new(
                self.element_id("keys"),
                move |_, cx| Self::open_case(resolved_id, cx),
                wrapper,
            ))
            .child(self.material_sheet(&state, resolved_id))
            .child(self.verdict_sheet(&state, resolved_id))
            .child(self.admin_sheet(&state, resolved_id))
            .on_appear(self.element_id("task"), move |_, cx| {
                ModerationInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_moderator_profile_loaded(cx));
                Self::open_case(resolved_id, cx);
            })
    }
}

#[derive(IntoElement)]
struct AppealReviewBody {
    appeal_id: String,
    resolved_id: Option<i64>,
    state: Entity<AppealReviewState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl AppealReviewBody {
    fn element_id(&self, name: impl Into<SharedString>) -> ElementId {
        appeal_element_id(&self.appeal_id, name)
    }

    fn push(&self, route: RootRoute, cx: &mut App) {
        push_route(&self.nav_store, route, cx);
    }

    fn content(&self, value: &RiseStoredAppealCase, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let resolved_id = self.resolved_id;
        scroll_view(self.element_id("scroll"))
            .list_shell_scroll_under_chrome(cx)
            .refreshable(move |_, cx| {
                AppealReview::open_case(resolved_id, cx);
                Task::ready(())
            })
            .child(
                vstack(ModerationMetrics::SECTION_SPACING)
                    .w_full()
                    .pt(px(12.0))
                    .pb(px(ModerationMetrics::BOTTOM_INSET))
                    .child(self.hero(value, theme))
                    .child(self.appeal_card(value, theme))
                    .child(self.sanction_card(value))
                    .child(self.decision_card(value, theme, cx))
                    .when_some(value.report_case.as_ref(), |column, report_case| {
                        column.child(self.materials_card(report_case, theme))
                    })
                    .when(value.capabilities.can_admin_sanction, |column| {
                        column.child(self.admin_card(value, cx))
                    }),
            )
            .into_any_element()
    }

    fn hero(&self, value: &RiseStoredAppealCase, theme: &AppTheme) -> AnyElement {
        let appeal = &value.appeal;
        let status = appeal.resolved_status();
        let subtitle = [
            value.appellant.as_ref().and_then(|appellant| appellant.tag.as_ref()).map(|tag| format!("@{tag}")),
            appeal.created_date_text(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(LIST_SEPARATOR);
        let title = value
            .appellant
            .as_ref()
            .map(|appellant| appellant.display_name())
            .unwrap_or_else(|| tr("moderation_unknown_reporter"));
        let open_appellant = self.open_appellant(value);
        let press_appellant = open_appellant.clone();
        let appellant = value.appellant.as_ref();
        let artwork = UserLogoUi::new(self.element_id("hero-logo"))
            .source(appellant.and_then(|appellant| appellant.logo.clone()).map(SharedString::from))
            .name(appellant.and_then(|appellant| appellant.name.clone()).map(SharedString::from))
            .user_id(SharedString::from(appeal.appellant_id.clone()))
            .size(ModerationMetrics::HERO_SIDE)
            .is_button(true)
            .open_story_on_press(false)
            .on_press(move |window, cx| press_appellant(window, cx));
        ModerationHero::new(self.element_id("hero"), title, artwork)
            .subtitle(SharedString::from(subtitle))
            .badge_text(SharedString::from(tr(status.title_key())))
            .badge_color((status != AppealStatus::Pending).then(|| ModerationTint::appeal(status, theme)))
            .on_title_press(Some(open_appellant))
            .into_any_element()
    }

    fn open_appellant(&self, value: &RiseStoredAppealCase) -> ButtonAction {
        let nav_store = self.nav_store.clone();
        let user_id = value.appeal.appellant_id.clone();
        let tag = value.appellant.as_ref().and_then(|appellant| appellant.tag.clone());
        Rc::new(move |_: &mut Window, cx: &mut App| {
            let route = RootRoute::UserPage { user_id: Some(user_id.clone()), tag: tag.clone() };
            push_route(&nav_store, route, cx);
        })
    }

    fn appeal_card(&self, value: &RiseStoredAppealCase, theme: &AppTheme) -> AnyElement {
        let appeal = &value.appeal;
        let mut items: Vec<GroupedBtn> = Vec::new();

        if !appeal.text.is_empty() {
            items.push(ModerationFact::row(
                "text",
                "text.quote",
                theme.primary._100,
                tr("appeal_review_appeal_text"),
                appeal.text.clone(),
                true,
                None,
            ));
        }

        let evidence = appeal.evidence();
        if !evidence.is_empty() {
            items.push(
                GroupedBtn::new("evidence", "").show_chevron(false).content_slot(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .px(px(16.0))
                        .py(px(12.0))
                        .child(MainText::new(tr("appeal_evidence_section")).px(12.0).color(theme.text.secondary))
                        .child(RemoteEvidenceStrip::new(self.element_id("evidence"), evidence).size(EVIDENCE_TILE)),
                ),
            );
        }

        GroupedBtns::new(self.element_id("appeal-card"), items)
            .separator_leading_padding(SEPARATOR_LEADING_PADDING)
            .into_any_element()
    }

    fn sanction_card(&self, value: &RiseStoredAppealCase) -> AnyElement {
        let mut items: Vec<GroupedBtn> = Vec::new();

        let Some(report) = value.report_case.as_ref().map(|report_case| &report_case.report) else {
            if let Some(block_type) = value.appeal.block_type.as_deref().map(|raw| UserBlockType::parse_or_default(Some(raw)))
            {
                items.push(ModerationFact::row(
                    "sanction",
                    block_type.icon_name(),
                    ModerationTint::block(block_type),
                    tr("case_scope_label"),
                    tr(block_type.title_key()),
                    false,
                    None,
                ));
            }
            return self.sanction_group(items);
        };
        let target_type = report.resolved_type();

        if let Some(block_type) = report.resolved_block_type() {
            let duration = report.block_duration_days.map(|days| {
                if days == 0 {
                    tr("blocks_permanent")
                } else {
                    tr("case_days").replace("{n}", &days.to_string())
                }
            });
            let parts: Vec<String> = [Some(tr(block_type.title_key())), duration].into_iter().flatten().collect();
            items.push(ModerationFact::row(
                "sanction",
                block_type.icon_name(),
                ModerationTint::block(block_type),
                tr("case_scope_label"),
                parts.join(LIST_SEPARATOR),
                false,
                None,
            ));
        }

        if let Some(reason) = report.resolved_reason() {
            items.push(ModerationFact::row(
                "reason",
                reason.icon_name(),
                ChatTileColor::gray(),
                tr("case_reason_label"),
                tr(reason.title_key()),
                false,
                None,
            ));
        }

        if let Some(decision) = report.decision.as_ref().filter(|decision| !decision.is_empty()) {
            items.push(ModerationFact::row(
                "verdict",
                "text.quote",
                ChatTileColor::gray(),
                tr("appeal_review_original_verdict"),
                decision.clone(),
                true,
                None,
            ));
        }

        let nav_store = self.nav_store.clone();
        let report_id = report.id.to_string();
        items.push(
            GroupedBtn::new("openReport", report.subject_line())
                .icon(target_type.icon_name())
                .icon_background(ModerationTint::target(target_type))
                .pretitle(tr("appeal_review_open_report"))
                .text_line_limit(1)
                .callback(move |_, cx| {
                    push_route(&nav_store, RootRoute::ReportDetail { report_id: report_id.clone() }, cx);
                }),
        );

        self.sanction_group(items)
    }

    fn sanction_group(&self, items: Vec<GroupedBtn>) -> AnyElement {
        GroupedBtns::new(self.element_id("sanction-card"), items)
            .title(tr("appeal_review_sanction_label").to_uppercase())
            .separator_leading_padding(SEPARATOR_LEADING_PADDING)
            .into_any_element()
    }

    fn decision_card(&self, value: &RiseStoredAppealCase, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let status = value.appeal.resolved_status();
        let id = self.element_id("decision-card");
        if status != AppealStatus::Pending {
            return GroupedBtns::new(id, self.resolved_items(value, theme))
                .title(tr("appeal_review_actions_label").to_uppercase())
                .separator_leading_padding(SEPARATOR_LEADING_PADDING)
                .into_any_element();
        }
        if !value.capabilities.can_resolve_appeals {
            return GroupedBtns::new(
                id,
                vec![
                    GroupedBtn::new("locked", tr("moderation_no_permissions"))
                        .icon("lock.fill")
                        .icon_background(ChatTileColor::gray())
                        .show_chevron(false),
                ],
            )
            .into_any_element();
        }
        let presentation = RiseModerationPresentation::shared(cx);
        let is_pending = self.resolved_id.is_some_and(|id| presentation.read(cx).resolving_appeals().contains(&id));
        let items = [AppealDecision::Accept, AppealDecision::Reject]
            .into_iter()
            .map(|decision| {
                let state = self.state.clone();
                let (tint, hint): (Hsla, &str) = if decision == AppealDecision::Accept {
                    (ChatTileColor::green(), "appeal_review_accept_hint")
                } else {
                    (ChatTileColor::red(), "appeal_review_reject_hint")
                };
                GroupedBtn::new(decision.id(), tr(decision.title_key()))
                    .icon(decision.icon_name())
                    .icon_background(tint)
                    .subtitle(tr(hint))
                    .subtitle_line_limit(2)
                    .disabled(is_pending)
                    .callback(move |_, cx| {
                        state.update(cx, |state, cx| {
                            state.pending_decision = Some(decision);
                            cx.notify();
                        });
                    })
            })
            .collect();
        GroupedBtns::new(id, items).title(tr("appeal_review_actions_label").to_uppercase()).into_any_element()
    }

    fn resolved_items(&self, value: &RiseStoredAppealCase, theme: &AppTheme) -> Vec<GroupedBtn> {
        let appeal = &value.appeal;
        let status = appeal.resolved_status();
        let mut items = vec![ModerationFact::row(
            "status",
            status.icon_name(),
            ModerationTint::appeal(status, theme),
            appeal.reviewed_date_text().unwrap_or_else(|| tr("appeal_review_actions_label")),
            if status == AppealStatus::Accepted {
                tr("appeal_review_accepted_banner")
            } else {
                tr("appeal_review_rejected_banner")
            },
            false,
            None,
        )];

        if let Some(verdict) = appeal.decision_text.as_ref().filter(|verdict| !verdict.is_empty()) {
            items.push(ModerationFact::row(
                "verdictText",
                "text.quote",
                ChatTileColor::gray(),
                tr("appeal_review_verdict_label"),
                verdict.clone(),
                true,
                None,
            ));
        }

        if let Some(reviewer) = &value.reviewer {
            let nav_store = self.nav_store.clone();
            let reviewed_by_id = appeal.reviewed_by_id.clone();
            let tag = reviewer.tag.clone();
            let mut row = ModerationPersonRow::new(self.element_id("reviewer-row"), reviewer.display_name())
                .logo_source(reviewer.logo.clone().map(SharedString::from))
                .user_id(appeal.reviewed_by_id.clone().map(SharedString::from))
                .subtitle(SharedString::from(tr("case_resolved_by")))
                .is_official(reviewer.is_official)
                .is_premium(reviewer.is_premium);
            if let Some(chevron) = moderation_symbol("chevron.right", 12.0, theme.text.secondary.opacity(0.75)) {
                row = row.trailing(chevron);
            }
            items.push(
                GroupedBtn::new("reviewer", "").show_chevron(false).content_slot(
                    plain(self.element_id("reviewer"))
                        .w_full()
                        .on_press(move |_, cx| {
                            let route = RootRoute::UserPage { user_id: reviewed_by_id.clone(), tag: tag.clone() };
                            push_route(&nav_store, route, cx);
                        })
                        .child(row),
                ),
            );
        }

        items
    }

    fn materials_card(&self, report_case: &RiseStoredReportCase, theme: &AppTheme) -> AnyElement {
        let mut materials: Vec<CaseMaterial> = Vec::new();
        if report_case.target.is_some() {
            materials.push(CaseMaterial::Target);
        }
        if !report_case.timeline.is_empty() {
            materials.push(CaseMaterial::Timeline);
        }
        let items = materials
            .into_iter()
            .map(|material| {
                let state = self.state.clone();
                let mut row = GroupedBtn::new(material.id(), tr(material.title_key()))
                    .icon(material.icon_name())
                    .icon_background(material.tint(theme))
                    .callback(move |_, cx| {
                        state.update(cx, |state, cx| {
                            state.open_sheet = Some(material);
                            cx.notify();
                        });
                    });
                if let Some(count) = material.count(report_case) {
                    row = row.trailing(RiseFormat::compact_count(count));
                }
                row
            })
            .collect();
        GroupedBtns::new(self.element_id("materials-card"), items).into_any_element()
    }

    fn admin_card(&self, value: &RiseStoredAppealCase, cx: &mut App) -> AnyElement {
        let has_active_blocks = value.report_case.as_ref().map_or(
            value.appeal.resolved_status() == AppealStatus::Pending,
            |report_case| report_case.target.as_ref().is_some_and(|target| !target.active_blocks.is_empty()),
        );
        let mut kinds: Vec<AdminSanctionKind> = Vec::new();
        if has_active_blocks {
            kinds.push(AdminSanctionKind::Unblock);
        }
        kinds.push(AdminSanctionKind::Block);
        if value.report_case.as_ref().is_some_and(|report_case| report_case.report.content_deleted) {
            kinds.push(AdminSanctionKind::RestoreContent);
        }
        let presentation = RiseModerationPresentation::shared(cx);
        let is_sanctioning = presentation.read(cx).is_sanctioning();
        let items = kinds
            .into_iter()
            .map(|kind| {
                let state = self.state.clone();
                GroupedBtn::new(kind.raw_value(), tr(kind.title_key()))
                    .icon(kind.icon_name())
                    .icon_background(ModerationTint::sanction(kind))
                    .subtitle(tr(kind.hint_key()))
                    .subtitle_line_limit(2)
                    .disabled(is_sanctioning)
                    .callback(move |_, cx| {
                        state.update(cx, |state, cx| {
                            state.pending_admin = Some(kind);
                            cx.notify();
                        });
                    })
            })
            .collect();
        GroupedBtns::new(self.element_id("admin-card"), items)
            .title(tr("case_admin_section").to_uppercase())
            .end_group_title(tr("case_admin_hint"))
            .end_group_title_px(12.0)
            .into_any_element()
    }
}

fn push_route(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

impl RenderOnce for AppealReviewBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let resolved_id = self.resolved_id;
        let value = AppealReview::appeal_case(resolved_id, cx);
        let state = match resolved_id {
            Some(id) => RiseModerationPresentation::shared(cx).read(cx).appeal_case_state(id),
            None => ModerationLoadState::Failed(tr("appeal_review_not_found")),
        };
        let background = theme.bg._100;
        let element = if let Some(value) = value {
            self.content(&value, &theme, cx)
        } else {
            match state {
                ModerationLoadState::Idle | ModerationLoadState::Loading => scroll_view(self.element_id("skeleton"))
                    .scroll_disabled(true)
                    .list_shell_scroll_under_chrome(cx)
                    .child(ModerationCaseSkeleton::new())
                    .into_any_element(),
                ModerationLoadState::Failed(message) => {
                    ModerationFailureState::new(self.element_id("failure"), message, theme, move |_, cx| {
                        AppealReview::open_case(resolved_id, cx);
                    })
                    .into_any_element()
                }
                ModerationLoadState::Loaded => scroll_view(self.element_id("empty"))
                    .list_shell_scroll_under_chrome(cx)
                    .child(ModerationEmptyState::new(
                        "arrow.uturn.backward.circle",
                        tr("appeal_review_not_found"),
                        theme,
                    ))
                    .into_any_element(),
            }
        };
        div().size_full().bg(background).child(element)
    }
}
