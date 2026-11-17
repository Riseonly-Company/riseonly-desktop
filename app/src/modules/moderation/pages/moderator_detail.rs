use std::collections::HashMap;
use std::rc::Rc;

use gpui::{AnyView, App, Entity, Hsla, ListAlignment, SharedString, StyleRefinement, Subscription, Window};
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::MainText;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonShimmerExt};
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::moderation::components::moderation_kit_ui::{
    ModerationFact, ModerationHero, ModerationMetrics, ModerationTint,
};
use crate::modules::moderation::components::moderation_type_tints::ReportTargetTypeTintExt;
use crate::modules::moderation::pages::all_moderators::ModeratorRightsSheet;
use crate::modules::moderation::stores::moderation_actions::moderation_actions::ModerationActionsStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    ModeratorKind, ModeratorKindExt, ModeratorPermissions, ModeratorStatFormatting, ReportDateFormatting,
    ReportReasonExt, ReportStatusExt, ReportTargetTypeExt,
};
use crate::modules::moderation::stores::moderation_actions::moderation_wire_types::{
    ModeratorStatsWireDTO, ModeratorWithUserWireDTO, ProtoModeratorProfileWithUserExt,
    ProtoReportWithAuthorExt, ReportQueueWireDTO,
};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;
use crate::modules::moderation::stores::moderation_services::moderation_services::ModerationServicesStore;

const SEPARATOR_LEADING_PADDING: f32 = 16.0;
const END_GROUP_TITLE_PX: f32 = 12.0;
const ANALYTICS_SKELETON_HEIGHT: f32 = 220.0;
const HISTORY_SKELETON_HEIGHT: f32 = 160.0;
const EMPTY_HISTORY_MIN_HEIGHT: f32 = 80.0;
const PROMOTION_MAX_OVERTURN_RATE: f64 = 0.1;
const OVERTURN_WARNING_RATE: f64 = 0.25;
const NO_VALUE: &str = "\u{2014}";
const HISTORY_ROW_ESTIMATED_HEIGHT: f32 = 74.0;
const HISTORY_ROW_HEIGHT_TOLERANCE: f32 = 0.5;
const HISTORY_HEADER_GAP: f32 = 8.0;
const HERO_ESTIMATED_HEIGHT: f32 = 170.0;
const HISTORY_SEPARATOR_OPACITY: f32 = 0.55;

fn interactions(cx: &mut App) -> Entity<ModerationInteractionsStore> {
    ModerationInteractionsStore::shared(cx)
}

/// analytics card as drawn, request state and figures
#[derive(Clone, PartialEq)]
struct ModeratorDetailAnalytics {
    status: SaiWsSliceStatus,
    did_load: bool,
    stats: Option<ModeratorStatsWireDTO>,
}

impl Default for ModeratorDetailAnalytics {
    fn default() -> Self {
        Self { status: SaiWsSliceStatus::Idle, did_load: false, stats: None }
    }
}

/// store values the body and its cached sections draw
/// they read this copy and not the stores so a store notify that changes none of it renders nothing
#[derive(Clone, PartialEq)]
struct ModeratorDetailDrawn {
    is_caller: bool,
    is_updating: bool,
    history_status: SaiWsSliceStatus,
    did_load_history: bool,
    history_error: Option<String>,
    analytics: ModeratorDetailAnalytics,
}

impl Default for ModeratorDetailDrawn {
    fn default() -> Self {
        Self {
            is_caller: false,
            is_updating: false,
            history_status: SaiWsSliceStatus::Idle,
            did_load_history: false,
            history_error: None,
            analytics: ModeratorDetailAnalytics::default(),
        }
    }
}

impl ModeratorDetailDrawn {
    fn read(user_id: &str, history_error: Option<String>, cx: &mut App) -> Self {
        let is_updating = interactions(cx).update(cx, |store, cx| store.update_pending(cx));
        let is_caller = ModerationServicesStore::is_caller(Some(user_id), cx);
        let did_load_history = ModerationServicesStore::did_load_history(user_id, cx);
        let actions = ModerationActionsStore::shared(cx);
        let actions = actions.read(cx);
        let history_status = actions.moderator_history_sai.status();
        let stats_status = actions.moderator_stats_sai.status();
        let services = ModerationServicesStore::shared(cx);
        let services = services.read(cx);
        Self {
            is_caller,
            is_updating,
            history_status,
            did_load_history,
            history_error,
            analytics: ModeratorDetailAnalytics {
                status: stats_status,
                did_load: services.did_load_stats(user_id),
                stats: services.stats(user_id),
            },
        }
    }
}

struct ModeratorDetailPageState {
    user_id: String,
    show_edit_rights: bool,
    edit_permissions: ModeratorPermissions,
    history: Rc<Vec<ReportQueueWireDTO>>,
    history_has_more: bool,
    history_total: i64,
    moderator: Option<ModeratorWithUserWireDTO>,
    drawn: ModeratorDetailDrawn,
    list: ListUi,
    list_ids: Vec<i64>,
    history_row_heights: HashMap<i64, f32>,
    section_heights: HashMap<ModeratorDetailSection, f32>,
    _subscriptions: Vec<Subscription>,
}

impl ModeratorDetailPageState {
    fn new(user_id: String, cx: &mut Context<Self>) -> Self {
        let services = ModerationServicesStore::shared(cx);
        let actions = ModerationActionsStore::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        let pending_requests = SaiWsPendingRegistry::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let mut state = Self {
            user_id,
            show_edit_rights: false,
            edit_permissions: ModeratorPermissions::default(),
            history: Rc::new(Vec::new()),
            history_has_more: false,
            history_total: 0,
            moderator: None,
            drawn: ModeratorDetailDrawn::default(),
            list: ListUi::new(rise_ui::theme(cx), 2, ListAlignment::Top),
            list_ids: Vec::new(),
            history_row_heights: HashMap::new(),
            section_heights: HashMap::new(),
            _subscriptions: vec![
                cx.observe(&services, |this, _, cx| this.refresh_history(cx)),
                cx.observe(&actions, |this, _, cx| this.refresh_history(cx)),
                cx.observe(&cache_revisions, |this, _, cx| this.refresh_history(cx)),
                cx.observe(&pending_requests, |this, _, cx| this.refresh_history(cx)),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        };
        state.load_history(cx);
        state
    }

    /// copies what the page draws from the stores, true when any of it moved
    fn load_history(&mut self, cx: &mut App) -> bool {
        let detail = ModerationServicesStore::detail(&self.user_id, cx);
        let mut changed = false;
        if self.history.as_slice() != detail.history.as_slice() {
            self.history = Rc::new(detail.history);
            changed = true;
        }
        if self.history_has_more != detail.history_has_more || self.history_total != detail.history_total {
            self.history_has_more = detail.history_has_more;
            self.history_total = detail.history_total;
            changed = true;
        }
        let moderator = ModerationServicesStore::moderator(&self.user_id, cx);
        if self.moderator != moderator {
            self.moderator = moderator;
            changed = true;
        }
        let drawn = ModeratorDetailDrawn::read(&self.user_id, detail.history_error, cx);
        if self.drawn != drawn {
            self.drawn = drawn;
            changed = true;
        }
        changed
    }

    fn refresh_history(&mut self, cx: &mut Context<Self>) {
        if self.load_history(cx) {
            cx.notify();
        }
    }

    fn sync_history(&mut self) -> (Rc<Vec<ReportQueueWireDTO>>, bool, i64) {
        let rows = self.history.clone();
        if !self.list_ids.iter().copied().eq(rows.iter().map(|report| report.id)) {
            let ids: Vec<i64> = rows.iter().map(|report| report.id).collect();
            let old = self.list_ids.len();
            if old > 0 && ids.len() > old && ids.starts_with(&self.list_ids) {
                self.list.state().splice(old + 1..old + 1, ids.len() - old);
            } else {
                self.list.state().splice(1..old + 1, ids.len());
            }
            self.list_ids = ids;
        }
        (rows, self.history_has_more, self.history_total)
    }

    fn section_height(&self, section: ModeratorDetailSection) -> f32 {
        self.section_heights.get(&section).copied().unwrap_or_else(|| section.estimated_height())
    }

    fn history_row_height(&self, report_id: i64) -> f32 {
        self.history_row_heights.get(&report_id).copied().unwrap_or(HISTORY_ROW_ESTIMATED_HEIGHT)
    }
}

#[derive(IntoElement)]
pub struct ModeratorDetail {
    user_id: String,
}

impl ModeratorDetail {
    pub fn new(user_id: impl Into<String>) -> Self {
        Self { user_id: user_id.into() }
    }

    fn has_resolved_work(stats: &ModeratorStatsWireDTO) -> bool {
        stats.reports_resolved > 0
    }

    fn is_promotion_candidate(stats: &ModeratorStatsWireDTO) -> bool {
        Self::has_resolved_work(stats) && stats.overturn_rate <= PROMOTION_MAX_OVERTURN_RATE
    }

    fn work_items(stats: &ModeratorStatsWireDTO) -> Vec<GroupedBtn> {
        let row = |id: &'static str, icon: &'static str, tint: Hsla, label_key: &str, value: i64| {
            ModerationFact::row(id, icon, tint, tr(label_key), RiseFormat::compact_count(value), false, None)
        };
        vec![
            row(
                "resolved",
                "checkmark.seal.fill",
                ChatTileColor::green(),
                "moderator_stat_reports_resolved",
                stats.reports_resolved,
            ),
            row("deleted", "trash.fill", ChatTileColor::red(), "moderator_stat_content_deleted", stats.content_deleted),
            row("blocked", "nosign", ChatTileColor::red(), "moderator_stat_users_blocked", stats.users_blocked),
            row("muted", "speaker.slash.fill", ChatTileColor::orange(), "moderator_stat_users_muted", stats.users_muted),
            row(
                "warnings",
                "exclamationmark.triangle.fill",
                ChatTileColor::orange(),
                "moderator_stat_warnings",
                stats.warnings_issued,
            ),
            row("dismissed", "xmark.circle.fill", ChatTileColor::gray(), "moderator_stat_dismissed", stats.dismissed),
        ]
    }

    fn appeal_items(stats: &ModeratorStatsWireDTO) -> Vec<GroupedBtn> {
        let rate = stats.overturn_rate;
        let has_work = Self::has_resolved_work(stats);
        let tint = if !has_work {
            ChatTileColor::gray()
        } else if rate <= PROMOTION_MAX_OVERTURN_RATE {
            ChatTileColor::green()
        } else if rate <= OVERTURN_WARNING_RATE {
            ChatTileColor::orange()
        } else {
            ChatTileColor::red()
        };
        let overturn = if has_work {
            format!(
                "{} \u{00B7} {}",
                ModeratorStatFormatting::percent(Some(rate)),
                RiseFormat::compact_count(stats.appeals_overturned)
            )
        } else {
            NO_VALUE.to_owned()
        };
        vec![
            ModerationFact::row(
                "overturn",
                "arrow.uturn.backward",
                tint,
                tr("moderator_overturn_headline"),
                overturn,
                false,
                None,
            ),
            ModerationFact::row(
                "against",
                "exclamationmark.bubble.fill",
                ChatTileColor::gray(),
                tr("moderator_stat_appeals_against"),
                format!(
                    "{} \u{00B7} {}",
                    RiseFormat::compact_count(stats.appeals_against),
                    ModeratorStatFormatting::percent(Some(stats.appeal_rate))
                ),
                false,
                None,
            ),
        ]
    }

    fn empty_stats(user_id: &str) -> ModeratorStatsWireDTO {
        ModeratorStatsWireDTO {
            appeal_rate: 0.0,
            appeals_against: 0,
            appeals_overturned: 0,
            content_deleted: 0,
            dismissed: 0,
            overturn_rate: 0.0,
            reports_resolved: 0,
            user_id: user_id.to_owned(),
            users_blocked: 0,
            users_muted: 0,
            warnings_issued: 0,
        }
    }

    fn loading_skeleton(height: f32) -> impl IntoElement + use<> {
        pass_through()
            .w_full()
            .px(px(GroupedCardMetrics::HORIZONTAL_PADDING))
            .child(SkeletonBlock::new(None::<f32>, height, GroupedCardMetrics::CORNER_RADIUS).skeleton_shimmer(None))
    }
}

#[derive(Clone)]
struct ModeratorDetailParts {
    user_id: String,
    state: Entity<ModeratorDetailPageState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl ModeratorDetailParts {
    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn hero(&self, moderator: Option<&ModeratorWithUserWireDTO>, theme: &AppTheme) -> ModerationHero {
        let user = moderator.and_then(|moderator| moderator.user.as_ref());
        let kind = moderator.and_then(ProtoModeratorProfileWithUserExt::resolved_kind);
        let target_user_id = moderator
            .and_then(ProtoModeratorProfileWithUserExt::target_user_id)
            .unwrap_or_else(|| self.user_id.clone());
        let route = RootRoute::UserPage {
            user_id: Some(target_user_id.clone()),
            tag: user.map(|user| user.tag.clone()),
        };
        let parts = self.clone();
        let hero = ModerationHero::new(
            "moderator-detail-hero",
            user.map_or_else(|| NO_VALUE.to_owned(), |user| user.name.clone()),
            UserLogoUi::new("moderator-detail-logo")
                .source(user.and_then(|user| user.logo.clone()).map(SharedString::from))
                .name(user.map(|user| SharedString::from(user.name.clone())))
                .user_id(SharedString::from(target_user_id))
                .size(ModerationMetrics::HERO_SIDE)
                .is_button(true)
                .on_press(move |_, cx| parts.push(route.clone(), cx)),
        )
        .subtitle(user.map(|user| SharedString::from(format!("@{}", user.tag))))
        .badge_text(kind.map(|kind| SharedString::from(tr(kind.title_key()))));
        if kind == Some(ModeratorKind::SuperModer) { hero.badge_color(theme.primary._100) } else { hero }
    }

    fn controls(
        &self,
        moderator: Option<&ModeratorWithUserWireDTO>,
        is_updating: bool,
        theme: &AppTheme,
    ) -> GroupedBtns {
        let is_active = moderator.is_some_and(ProtoModeratorProfileWithUserExt::is_active);
        let has_moderator = moderator.is_some();
        let edit = self.clone();
        let switch_user = self.user_id.clone();
        let tap_user = self.user_id.clone();
        GroupedBtns::new(
            "moderator-detail-controls",
            vec![
                GroupedBtn::new("rights", tr("moderator_edit_rights_button"))
                    .icon("slider.horizontal.3")
                    .icon_background(theme.primary._100)
                    .callback(move |_, cx| {
                        let permissions = ModerationServicesStore::moderator(&edit.user_id, cx)
                            .map(|moderator| moderator.permissions())
                            .unwrap_or_default();
                        edit.state.update(cx, |state, cx| {
                            state.edit_permissions = permissions;
                            state.show_edit_rights = true;
                            cx.notify();
                        });
                    }),
                GroupedBtn::new("active", tr("moderator_active_toggle"))
                    .icon("power")
                    .icon_background(if is_active { ChatTileColor::green() } else { ChatTileColor::gray() })
                    .subtitle(tr("moderator_active_toggle_caption"))
                    .subtitle_line_limit(2)
                    .disabled(is_updating || !has_moderator)
                    .show_chevron(false)
                    .trailing_slot(SwitchUi::new(
                        "moderator-detail-active-switch",
                        Binding::new(
                            move |_: &App| is_active,
                            move |value, _window, cx| {
                                interactions(cx).update(cx, |store, cx| store.set_active(&switch_user, value, cx));
                            },
                        ),
                    ))
                    .callback(move |_, cx| {
                        if !has_moderator {
                            return;
                        }
                        interactions(cx).update(cx, |store, cx| store.set_active(&tap_user, !is_active, cx));
                    }),
            ],
        )
    }

    fn analytics_section(&self, analytics: &ModeratorDetailAnalytics) -> SaiDataRender {
        let status = analytics.status;
        let did_load = analytics.did_load;
        let stats = analytics.stats.clone().unwrap_or_else(|| ModeratorDetail::empty_stats(&self.user_id));
        SaiDataRender::new(status, move |_, _| {
            let is_candidate = ModeratorDetail::is_promotion_candidate(&stats);
            let appeals = GroupedBtns::new("moderator-detail-appeals", ModeratorDetail::appeal_items(&stats))
                .title(tr("case_appeals_section").to_uppercase())
                .end_group_title_px(END_GROUP_TITLE_PX)
                .separator_leading_padding(SEPARATOR_LEADING_PADDING);
            vstack(ModerationMetrics::SECTION_SPACING)
                .w_full()
                .child(
                    GroupedBtns::new("moderator-detail-work", ModeratorDetail::work_items(&stats))
                        .title(tr("moderator_analytics_label").to_uppercase())
                        .separator_leading_padding(SEPARATOR_LEADING_PADDING),
                )
                .child(if is_candidate { appeals.end_group_title(tr("moderator_overturn_caption")) } else { appeals })
        })
        .id("moderator-detail-analytics")
        .is_empty(false)
        .need_pending(!did_load)
        .sai_data_render_loading(|_, _| ModeratorDetail::loading_skeleton(ANALYTICS_SKELETON_HEIGHT))
    }

    fn empty_history_section(&self, total: i64, drawn: &ModeratorDetailDrawn) -> Div {
        let status = drawn.history_status;
        let did_load = drawn.did_load_history;
        let error_text = drawn.history_error.clone().unwrap_or_else(|| tr("error_fetching_data"));
        div().w_full().min_h(px(EMPTY_HISTORY_MIN_HEIGHT)).child(
            SaiDataRender::new(status, move |_, _| {
                let card = GroupedBtns::new("moderator-detail-history", Vec::new())
                    .title(tr("moderator_history_label").to_uppercase());
                if total > 0 { card.right_top_text(RiseFormat::compact_count(total)) } else { card }
            })
            .id("moderator-detail-history-render")
            .is_empty(true)
            .no_data_text(tr("moderator_history_empty"))
            .error_text(error_text)
            .need_pending(!did_load)
            .sai_data_render_loading(|_, _| ModeratorDetail::loading_skeleton(HISTORY_SKELETON_HEIGHT)),
        )
    }

    fn history_header(total: i64, theme: &AppTheme) -> Div {
        let secondary = theme.text.secondary;
        hstack(None)
            .w_full()
            .child(MainText::new(tr("moderator_history_label").to_uppercase()).px(12.0).color(secondary))
            .child(spacer())
            .when(total > 0, |row| row.child(MainText::new(RiseFormat::compact_count(total)).px(12.0).color(secondary)))
            .padding_edges(EdgeSet::HORIZONTAL, GroupedCardMetrics::TITLE_HORIZONTAL_PADDING)
    }

    fn history_more_item(&self) -> GroupedBtn {
        let user_id = self.user_id.clone();
        GroupedBtn::new("more", tr("common_show_more"))
            .icon("ellipsis")
            .icon_background(ChatTileColor::gray())
            .show_chevron(false)
            .callback(move |_, cx| {
                interactions(cx).update(cx, |store, cx| store.load_more_history(&user_id, cx));
            })
    }

    fn history_item(&self, report: &ReportQueueWireDTO) -> GroupedBtn {
        let r#type = report.resolved_type();
        let status = report.resolved_status();
        let mut subtitle_parts = vec![tr(status.title_key())];
        if let Some(decision) = report.decision.as_ref().filter(|decision| !decision.is_empty()) {
            subtitle_parts.push(decision.clone());
        }
        let text = match report.resolved_reason() {
            Some(reason) => format!("{} \u{00B7} {}", tr(r#type.title_key()), tr(reason.title_key())),
            None => tr(r#type.title_key()),
        };
        let parts = self.clone();
        let route = RootRoute::ReportDetail { report_id: report.id.to_string() };
        let row = GroupedBtn::new(report.id.to_string(), text)
            .icon(ReportTargetTypeTintExt::icon_name(r#type))
            .icon_background(ModerationTint::target(r#type))
            .subtitle(subtitle_parts.join(" \u{00B7} "))
            .text_line_limit(1)
            .subtitle_line_limit(1)
            .callback(move |_, cx| parts.push(route.clone(), cx));
        match ReportDateFormatting::short(Some(report.created_at.as_str())) {
            Some(pretitle) => row.pretitle(pretitle),
            None => row,
        }
    }

    fn rights_sheet(&self) -> NativeBottomSheetPresentationHost {
        let reader = self.state.clone();
        let writer = self.state.clone();
        let content = self.clone();
        NativeBottomSheetPresentationHost::new(
            "moderator-detail-rights-sheet",
            Binding::new(
                move |cx: &App| reader.read(cx).show_edit_rights,
                move |presented, _window, cx| {
                    writer.update(cx, |state, cx| {
                        state.show_edit_rights = presented;
                        cx.notify();
                    });
                },
            ),
            move |_, cx| {
                let show_appeals_toggle = ModerationServicesStore::moderator(&content.user_id, cx)
                    .and_then(|moderator| moderator.resolved_kind())
                    == Some(ModeratorKind::SuperModer);
                let permissions_reader = content.state.clone();
                let permissions_writer = content.state.clone();
                let saved = content.state.clone();
                ModeratorRightsSheet::new(
                    content.user_id.clone(),
                    Binding::new(
                        move |cx: &App| permissions_reader.read(cx).edit_permissions.clone(),
                        move |permissions, _window, cx| {
                            permissions_writer.update(cx, |state, cx| {
                                state.edit_permissions = permissions;
                                cx.notify();
                            });
                        },
                    ),
                    show_appeals_toggle,
                    move |cx| {
                        saved.update(cx, |state, cx| {
                            state.show_edit_rights = false;
                            cx.notify();
                        });
                    },
                )
            },
        )
        .presentation_detents([PresentationDetent::Large])
        .presentation_drag_indicator(true)
    }
}

/// one row of the history card without its surface, the list item around it paints the card slice
fn single_row_card(id: impl Into<SharedString>, item: GroupedBtn) -> GroupedBtns {
    GroupedBtns::new(gpui::ElementId::Name(id.into()), vec![item])
        .horizontal_padding(0.0)
        .corner_radius(0.0)
        .surface_color(gpui::transparent_black())
}

/// slice of the history card one list item paints, inset like the card and rounded at its ends
fn card_slice(theme: &AppTheme, is_first: bool, is_last: bool, content: impl IntoElement) -> Div {
    let radius = px(GroupedCardMetrics::CORNER_RADIUS);
    div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(
        div()
            .w_full()
            .bg(theme.bg._200)
            .when(is_first, |slice| slice.rounded_t(radius))
            .when(is_last, |slice| slice.rounded_b(radius))
            .child(content),
    )
}

fn card_separator(theme: &AppTheme) -> Div {
    div()
        .w_full()
        .pl(px(GroupedCardMetrics::content_leading_inset()))
        .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(HISTORY_SEPARATOR_OPACITY)))
}

#[derive(IntoElement)]
struct ModeratorDetailBody {
    parts: ModeratorDetailParts,
}

impl ModeratorDetailBody {
    fn top(
        parts: &ModeratorDetailParts,
        rows_empty: bool,
        total: i64,
        top_inset: f32,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let state = parts.state.read(cx);
        let moderator = state.moderator.clone();
        let drawn = state.drawn.clone();
        let is_caller = drawn.is_caller;
        let is_updating = drawn.is_updating;
        let hero = ModeratorDetailSection::Hero.view(parts, window, cx);
        let analytics = ModeratorDetailSection::Analytics.view(parts, window, cx);
        let history = if rows_empty {
            parts.empty_history_section(total, &drawn)
        } else {
            ModeratorDetailParts::history_header(total, theme)
        };
        vstack(ModerationMetrics::SECTION_SPACING)
            .w_full()
            .pt(px(top_inset + 12.0))
            .when(!rows_empty, |column| column.pb(px(HISTORY_HEADER_GAP)))
            .child(hero)
            .when(!is_caller, |column| column.child(parts.controls(moderator.as_ref(), is_updating, theme)))
            .child(analytics)
            .child(history)
    }
}

impl RenderOnce for ModeratorDetailBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let parts = self.parts;
        let top_inset = list_shell_floating_top_inset(cx);
        let (rows, has_more, total) = parts.state.update(cx, |state, _| state.sync_history());
        let count = rows.len();
        let item_theme = theme.clone();
        let item_parts = parts.clone();
        let list = parts.state.read(cx).list.hosted(
            SharedString::from(format!("moderator-detail-list-{}", parts.user_id)),
            move |index, window, cx| {
                let theme = &item_theme;
                if index == 0 {
                    return ModeratorDetailBody::top(&item_parts, count == 0, total, top_inset, theme, window, cx)
                        .into_any_element();
                }
                let Some(report) = rows.get(index - 1) else {
                    let more = (has_more && count > 0).then(|| {
                        card_slice(
                            theme,
                            false,
                            true,
                            vstack(0.0)
                                .w_full()
                                .child(card_separator(theme))
                                .child(single_row_card("moderator-detail-history-more", item_parts.history_more_item())),
                        )
                    });
                    return vstack(0.0)
                        .w_full()
                        .children(more)
                        .child(div().w_full().h(px(ModerationMetrics::BOTTOM_INSET)))
                        .into_any_element();
                };
                let report_id = report.id;
                let row_view = window.use_keyed_state(
                    SharedString::from(format!("moderator-detail-history-row-view-{report_id}")),
                    cx,
                    |_, cx| ModeratorHistoryRowView::new(report.clone(), item_parts.clone(), cx),
                );
                row_view.update(cx, |view, cx| view.set_report(report, cx));
                let height = item_parts.state.read(cx).history_row_height(report_id);
                let is_last = index == count && !has_more;
                card_slice(
                    theme,
                    index == 1,
                    is_last,
                    vstack(0.0)
                        .w_full()
                        .child(row_view.cached(StyleRefinement::default().w_full().h(px(height))).replay_when_moved())
                        .when(!is_last, |column| column.child(card_separator(theme))),
                )
                .into_any_element()
            },
        );
        div().size_full().bg(theme.bg._100).child(list)
    }
}

struct ModeratorDetailBodyView {
    parts: ModeratorDetailParts,
}

impl ModeratorDetailBodyView {
    fn sync(&mut self, nav_store: Option<Entity<NavigationStore>>, cx: &mut Context<Self>) {
        if self.parts.nav_store != nav_store {
            self.parts.nav_store = nav_store;
            cx.notify();
        }
    }
}

impl Render for ModeratorDetailBodyView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        ModeratorDetailBody { parts: self.parts.clone() }
    }
}

/// hero and analytics as their own cached views
/// switching the controls between them animates and each frame rendered the body item holding them
/// now they render only when a store they read or the theme changes
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ModeratorDetailSection {
    Hero,
    Analytics,
}

impl ModeratorDetailSection {
    fn key(self) -> &'static str {
        match self {
            Self::Hero => "hero",
            Self::Analytics => "analytics",
        }
    }

    fn estimated_height(self) -> f32 {
        match self {
            Self::Hero => HERO_ESTIMATED_HEIGHT,
            Self::Analytics => ANALYTICS_SKELETON_HEIGHT,
        }
    }

    fn view(self, parts: &ModeratorDetailParts, window: &mut Window, cx: &mut App) -> AnyElement {
        let view = window.use_keyed_state(
            SharedString::from(format!("moderator-detail-{}-view-{}", self.key(), parts.user_id)),
            cx,
            |_, cx| ModeratorDetailSectionView::new(self, parts.clone(), cx),
        );
        view.update(cx, |view, cx| view.set_parts(parts, cx));
        let height = parts.state.read(cx).section_height(self);
        view.cached(StyleRefinement::default().w_full().h(px(height))).replay_when_moved().into_any_element()
    }
}

#[derive(Clone, PartialEq)]
enum ModeratorDetailSectionDrawn {
    Hero(Option<ModeratorWithUserWireDTO>),
    Analytics(ModeratorDetailAnalytics),
}

impl ModeratorDetailSectionDrawn {
    fn of(section: ModeratorDetailSection, state: &ModeratorDetailPageState) -> Self {
        match section {
            ModeratorDetailSection::Hero => Self::Hero(state.moderator.clone()),
            ModeratorDetailSection::Analytics => Self::Analytics(state.drawn.analytics.clone()),
        }
    }
}

struct ModeratorDetailSectionView {
    section: ModeratorDetailSection,
    parts: ModeratorDetailParts,
    drawn: ModeratorDetailSectionDrawn,
    _subscriptions: Vec<Subscription>,
}

impl ModeratorDetailSectionView {
    fn new(section: ModeratorDetailSection, parts: ModeratorDetailParts, cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        let drawn = ModeratorDetailSectionDrawn::of(section, parts.state.read(cx));
        let state = parts.state.clone();
        Self {
            section,
            parts,
            drawn,
            _subscriptions: vec![
                cx.observe(&state, |this, state, cx| this.sync_drawn(&state, cx)),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn sync_drawn(&mut self, state: &Entity<ModeratorDetailPageState>, cx: &mut Context<Self>) {
        let drawn = ModeratorDetailSectionDrawn::of(self.section, state.read(cx));
        if drawn != self.drawn {
            self.drawn = drawn;
            cx.notify();
        }
    }

    fn set_parts(&mut self, parts: &ModeratorDetailParts, cx: &mut Context<Self>) {
        if self.parts.nav_store != parts.nav_store || self.parts.user_id != parts.user_id {
            self.parts = parts.clone();
            let drawn = ModeratorDetailSectionDrawn::of(self.section, parts.state.read(cx));
            self.drawn = drawn;
            cx.notify();
        }
    }
}

impl Render for ModeratorDetailSectionView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let section = self.section;
        let content = match &self.drawn {
            ModeratorDetailSectionDrawn::Hero(moderator) => {
                let theme = rise_ui::theme(cx).clone();
                self.parts.hero(moderator.as_ref(), &theme).into_any_element()
            }
            ModeratorDetailSectionDrawn::Analytics(analytics) => {
                self.parts.analytics_section(analytics).into_any_element()
            }
        };
        let state = self.parts.state.clone();
        div()
            .w_full()
            .relative()
            .child(content)
            .child(measure(
                SharedString::from(format!("moderator-detail-{}-height", section.key())),
                move |bounds, _, cx| {
                    let height = f32::from(bounds.size.height);
                    state.update(cx, |state, cx| {
                        let known = state.section_heights.get(&section).copied();
                        if known.is_none_or(|known| (known - height).abs() > HISTORY_ROW_HEIGHT_TOLERANCE) {
                            state.section_heights.insert(section, height);
                            cx.notify();
                        }
                    });
                },
            ))
    }
}

struct ModeratorHistoryRowView {
    report: ReportQueueWireDTO,
    parts: ModeratorDetailParts,
    _theme: Subscription,
}

impl ModeratorHistoryRowView {
    fn new(report: ReportQueueWireDTO, parts: ModeratorDetailParts, cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        Self { report, parts, _theme: cx.observe(&theme_store, |_, _, cx| cx.notify()) }
    }

    fn set_report(&mut self, report: &ReportQueueWireDTO, cx: &mut Context<Self>) {
        if self.report != *report {
            self.report = report.clone();
            cx.notify();
        }
    }
}

impl Render for ModeratorHistoryRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let report_id = self.report.id;
        let state = self.parts.state.clone();
        div()
            .w_full()
            .relative()
            .child(single_row_card(
                format!("moderator-detail-history-{report_id}"),
                self.parts.history_item(&self.report),
            ))
            .child(measure(SharedString::from(format!("moderator-detail-history-height-{report_id}")), move |bounds, _, cx| {
                let height = f32::from(bounds.size.height);
                state.update(cx, |state, cx| {
                    let known = state.history_row_heights.get(&report_id).copied();
                    if known.is_none_or(|known| (known - height).abs() > HISTORY_ROW_HEIGHT_TOLERANCE) {
                        state.history_row_heights.insert(report_id, height);
                        cx.notify();
                    }
                });
            }))
    }
}

impl RenderOnce for ModeratorDetail {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let user_id = self.user_id;
        let state = window.use_keyed_state(
            SharedString::from(format!("moderator-detail-state-{user_id}")),
            cx,
            {
                let user_id = user_id.clone();
                move |_, cx| ModeratorDetailPageState::new(user_id, cx)
            },
        );
        let nav_store = env::<NavigationStoreKey>(cx);
        let parts = ModeratorDetailParts { user_id: user_id.clone(), state, nav_store: nav_store.clone() };
        let body = window.use_keyed_state(SharedString::from(format!("moderator-detail-body-{user_id}")), cx, {
            let parts = parts.clone();
            move |_, _| ModeratorDetailBodyView { parts }
        });
        body.update(cx, |body, cx| body.sync(nav_store, cx));
        let back = parts.clone();
        let task_user = user_id.clone();
        div()
            .size_full()
            .child(
                SettingsWrapperUi::new(tr("moderator_detail_title"), move |_, cx| {
                    if let Some(nav_store) = &back.nav_store {
                        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                    }
                })
                .content(AnyView::from(body).cached(StyleRefinement::default().size_full()).replay_when_moved()),
            )
            .child(parts.rights_sheet())
            .on_appear(SharedString::from(format!("moderator-detail-task-{user_id}")), move |_, cx| {
                interactions(cx).update(cx, |store, cx| store.load_detail(&task_user, cx));
            })
    }
}
