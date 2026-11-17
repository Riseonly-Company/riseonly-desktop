use gpui::{App, Entity, Hsla, SharedString, Subscription, Task, Window};
use rise_core::foundation::foundation_character::int_from_string;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::aside::{aside_navigator, aside_registry};
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::moderation::components::moderation_kit_ui::{
    ModerationArtwork, ModerationCaseSkeleton, ModerationEmptyState, ModerationFact, ModerationFailureState,
    ModerationHero, ModerationMetrics, ModerationPersonRow, ModerationTint, moderation_symbol,
};
use crate::modules::moderation::components::moderation_type_tints::ReportTargetTypeTintExt;
use crate::modules::moderation::engine::rise_moderation_presentation::{
    ModerationLoadState, RiseModerationPresentation,
};
use crate::modules::moderation::engine::rise_moderation_wire::{
    ModerationUserDTO, ReportRowDTO, RiseStoredCaseCapabilities, RiseStoredReportCase,
};
use crate::modules::moderation::pages::moderation_case_sections::{CaseMaterial, CaseMaterialSheet};
use crate::modules::moderation::pages::moderation_case_sheets::{AdminSanctionSheet, ReopenSheet, ResolveConfirmSheet};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AdminSanctionKind, ModerationDateFormatting, ReportReasonExt, ReportStatusExt, ReportTargetType,
    ReportTargetTypeExt, ResolveAction, ResolveActionExt, UserBlockType, UserBlockTypeExt,
};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;

const SEPARATOR_LEADING_PADDING: f32 = 16.0;
const END_GROUP_TITLE_PX: f32 = 12.0;
const MEDIA_SIDE: f32 = 104.0;
const MEDIA_CORNER_RADIUS: f32 = 12.0;
const MEDIA_DECODE_DIMENSION: i64 = 320;
const CHEVRON_OPACITY: f32 = 0.75;

fn interactions(cx: &mut App) -> Entity<ModerationInteractionsStore> {
    ModerationInteractionsStore::shared(cx)
}

struct ReportDetailState {
    pending_action: Option<ResolveAction>,
    pending_admin: Option<AdminSanctionKind>,
    open_sheet: Option<CaseMaterial>,
    is_reopening: bool,
    _subscriptions: Vec<Subscription>,
}

impl ReportDetailState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseModerationPresentation::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            pending_action: None,
            pending_admin: None,
            open_sheet: None,
            is_reopening: false,
            _subscriptions: vec![
                cx.observe(&presentation, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement)]
pub struct ReportDetail {
    report_id: String,
}

impl ReportDetail {
    pub fn new(report_id: impl Into<String>) -> Self {
        Self { report_id: report_id.into() }
    }

    fn applied_block_type(value: &RiseStoredReportCase) -> Option<UserBlockType> {
        let active = value.target.as_ref().map(|target| target.active_blocks.as_slice()).unwrap_or_default();
        let applied = value.report.block_type.as_deref().map(|raw| UserBlockType::parse_or_default(Some(raw)));
        if let Some(applied) = applied
            && active.iter().any(|block| UserBlockType::parse_or_default(Some(block.block_type.as_str())) == applied)
        {
            return Some(applied);
        }
        active.first().map(|block| UserBlockType::parse_or_default(Some(block.block_type.as_str())))
    }

    fn threshold_hint(capabilities: &RiseStoredCaseCapabilities, has_content: bool) -> Option<String> {
        if !capabilities.can_delete_content
            || !has_content
            || capabilities.delete_threshold_met
            || capabilities.is_admin
        {
            return None;
        }
        Some(tr("case_delete_threshold_hint").replace("{n}", &capabilities.delete_threshold.to_string()))
    }
}

#[derive(Clone)]
struct ReportDetailParts {
    report_id: String,
    resolved_id: Option<i64>,
    state: Entity<ReportDetailState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl ReportDetailParts {
    fn set_state(&self, cx: &mut App, change: impl FnOnce(&mut ReportDetailState)) {
        self.state.update(cx, |state, cx| {
            change(state);
            cx.notify();
        });
    }

    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn refresh_case(&self, cx: &mut App) {
        if let Some(id) = self.resolved_id {
            interactions(cx).update(cx, |store, cx| store.refresh_case(id, cx));
        }
    }

    fn open_case(&self, cx: &mut App) {
        if let Some(id) = self.resolved_id {
            interactions(cx).update(cx, |store, cx| store.open_case(id, cx));
        }
    }

    fn sheets(&self, cx: &mut App) -> Div {
        let value = self.resolved_id.and_then(|id| RiseModerationPresentation::shared(cx).read(cx).cases().get(&id).cloned());
        div()
            .child(self.material_sheet())
            .child(self.resolve_sheet(value.clone()))
            .child(self.admin_sheet(value))
            .child(self.reopen_sheet())
    }

    fn material_sheet(&self) -> NativeBottomSheetPresentationHost {
        let reader = self.state.clone();
        let writer = self.clone();
        let content = self.clone();
        NativeBottomSheetPresentationHost::new(
            "report-detail-material-sheet",
            Binding::new(
                move |cx: &App| reader.read(cx).open_sheet.is_some(),
                move |presented, _window, cx| {
                    if !presented {
                        writer.set_state(cx, |state| state.open_sheet = None);
                    }
                },
            ),
            move |_, cx| match (content.state.read(cx).open_sheet, content.resolved_id) {
                (Some(material), Some(id)) => CaseMaterialSheet::new(material, id).into_any_element(),
                _ => div().into_any_element(),
            },
        )
        .presentation_detents([PresentationDetent::Large])
        .presentation_drag_indicator(true)
    }

    fn resolve_sheet(&self, value: Option<RiseStoredReportCase>) -> NativeBottomSheetPresentationHost {
        let reader = self.state.clone();
        let writer = self.clone();
        let content = self.clone();
        NativeBottomSheetPresentationHost::new(
            "report-detail-resolve-sheet",
            Binding::new(
                move |cx: &App| reader.read(cx).pending_action.is_some(),
                move |presented, _window, cx| {
                    if !presented {
                        writer.set_state(cx, |state| state.pending_action = None);
                    }
                },
            ),
            move |_, cx| match (content.state.read(cx).pending_action, value.as_ref()) {
                (Some(action), Some(value)) => {
                    let done = content.clone();
                    ResolveConfirmSheet::new(value.report.clone(), value.capabilities.clone(), action, move |_, cx| {
                        done.set_state(cx, |state| state.pending_action = None);
                    })
                    .into_any_element()
                }
                _ => div().into_any_element(),
            },
        )
        .presentation_detents([PresentationDetent::Large])
        .presentation_drag_indicator(true)
    }

    fn admin_sheet(&self, value: Option<RiseStoredReportCase>) -> NativeBottomSheetPresentationHost {
        let reader = self.state.clone();
        let writer = self.clone();
        let content = self.clone();
        NativeBottomSheetPresentationHost::new(
            "report-detail-admin-sheet",
            Binding::new(
                move |cx: &App| reader.read(cx).pending_admin.is_some(),
                move |presented, _window, cx| {
                    if !presented {
                        writer.set_state(cx, |state| state.pending_admin = None);
                    }
                },
            ),
            move |_, cx| match (content.state.read(cx).pending_admin, value.as_ref()) {
                (Some(kind), Some(value)) => {
                    let target = value.target.as_ref();
                    let done = content.clone();
                    AdminSanctionSheet::new(
                        kind,
                        target.map(|target| target.user_id.clone()).or_else(|| value.report.reported_user_id()),
                        target
                            .and_then(|target| target.user.as_ref())
                            .or(value.report.reported_user.as_ref())
                            .map(ModerationUserDTO::display_name),
                        Some(value.report.id),
                        None,
                        move |_, cx| {
                            done.set_state(cx, |state| state.pending_admin = None);
                        },
                    )
                    .initial_block_type(if kind == AdminSanctionKind::Unblock {
                        ReportDetail::applied_block_type(value)
                    } else {
                        None
                    })
                    .into_any_element()
                }
                _ => div().into_any_element(),
            },
        )
        .presentation_detents([PresentationDetent::Large])
        .presentation_drag_indicator(true)
    }

    fn reopen_sheet(&self) -> NativeBottomSheetPresentationHost {
        let reader = self.state.clone();
        let writer = self.clone();
        let content = self.clone();
        NativeBottomSheetPresentationHost::new(
            "report-detail-reopen-sheet",
            Binding::new(
                move |cx: &App| reader.read(cx).is_reopening,
                move |presented, _window, cx| {
                    writer.set_state(cx, |state| state.is_reopening = presented);
                },
            ),
            move |_, _| match content.resolved_id {
                Some(id) => {
                    let done = content.clone();
                    ReopenSheet::new(id, move |_, cx| {
                        done.set_state(cx, |state| state.is_reopening = false);
                    })
                    .into_any_element()
                }
                None => div().into_any_element(),
            },
        )
        .presentation_detents([PresentationDetent::Medium])
        .presentation_drag_indicator(true)
    }

    fn content(&self, value: &RiseStoredReportCase, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let refresh = self.clone();
        let is_resolving = RiseModerationPresentation::shared(cx).read(cx).resolving_reports().contains(&value.report.id);
        let is_reopening = RiseModerationPresentation::shared(cx).read(cx).reopening_reports().contains(&value.report.id);
        let is_sanctioning = RiseModerationPresentation::shared(cx).read(cx).is_sanctioning();
        scroll_view(SharedString::from(format!("report-detail-scroll-{}", self.report_id)))
            .refreshable(move |_, cx| {
                refresh.refresh_case(cx);
                Task::ready(())
            })
            .list_shell_scroll_under_chrome(cx)
            .child(
                vstack(ModerationMetrics::SECTION_SPACING)
                    .w_full()
                    .pt(px(12.0))
                    .pb(px(ModerationMetrics::BOTTOM_INSET))
                    .child(self.hero(value, theme))
                    .child(self.subject_card(value, theme))
                    .child(self.decision_card(value, is_resolving, is_reopening, theme))
                    .child(self.materials_card(value, theme))
                    .when(value.capabilities.can_admin_sanction, |column| {
                        column.child(self.admin_card(value, is_sanctioning))
                    }),
            )
            .into_any_element()
    }

    fn hero(&self, value: &RiseStoredReportCase, theme: &AppTheme) -> ModerationHero {
        let report = &value.report;
        let r#type = report.resolved_type();
        let status = report.resolved_status();
        ModerationHero::new(
            "report-detail-hero",
            report.subject_line(),
            ModerationArtwork::new(
                "report-detail-artwork",
                ReportTargetTypeTintExt::icon_name(r#type),
                ModerationTint::target(r#type),
            )
            .image_source(report.snapshot_media.first().cloned().map(SharedString::from)),
        )
        .subtitle(report.resolved_reason().map(|reason| SharedString::from(tr(reason.title_key()))))
        .badge_text(SharedString::from(tr(status.title_key())))
        .badge_color(ModerationTint::status(status, theme))
    }

    fn person_row(
        &self,
        id: &'static str,
        user: &ModerationUserDTO,
        user_id: Option<String>,
        subtitle: String,
        theme: &AppTheme,
    ) -> GroupedBtn {
        let parts = self.clone();
        let route = RootRoute::UserPage { user_id: user_id.clone(), tag: user.tag.clone() };
        let chevron = moderation_symbol("chevron.right", 12.0, theme.text.secondary.opacity(CHEVRON_OPACITY));
        GroupedBtn::new(id, "").show_chevron(false).content_slot(
            plain(SharedString::from(format!("report-detail-{id}-button")))
                .w_full()
                .on_press(move |_, cx| parts.push(route.clone(), cx))
                .child(
                    ModerationPersonRow::new(SharedString::from(format!("report-detail-{id}-row")), user.display_name())
                        .logo_source(user.logo.clone().map(SharedString::from))
                        .user_id(user_id.map(SharedString::from))
                        .subtitle(SharedString::from(subtitle))
                        .is_official(user.is_official)
                        .is_premium(user.is_premium)
                        .trailing(div().children(chevron)),
                ),
        )
    }

    fn media_row(report: &ReportRowDTO) -> GroupedBtn {
        let images = report.snapshot_media.iter().enumerate().map(|(offset, url)| {
            div().size(px(MEDIA_SIDE)).flex_none().overflow_hidden().rounded_rect(MEDIA_CORNER_RADIUS).child(
                CleverImageUi::new(SharedString::from(url.clone()))
                    .id(SharedString::from(format!("report-detail-media-{offset}")))
                    .content_mode(ContentMode::Fill)
                    .corner_radius(MEDIA_CORNER_RADIUS)
                    .persist_to_disk(false)
                    .maximum_decode_pixel_dimension(MEDIA_DECODE_DIMENSION)
                    .cache_policy(CleverImageCachePolicy::MemoryOnly),
            )
        });
        GroupedBtn::new("media", "").show_chevron(false).content_slot(
            div()
                .id("report-detail-media")
                .w_full()
                .flex()
                .overflow_x_scroll()
                .py(px(12.0))
                .child(hstack(8.0).flex_none().px(px(16.0)).children(images)),
        )
    }

    fn subject_card(&self, value: &RiseStoredReportCase, theme: &AppTheme) -> GroupedBtns {
        let report = &value.report;
        let r#type = report.resolved_type();
        let mut items = vec![ModerationFact::row(
            "type",
            ReportTargetTypeTintExt::icon_name(r#type),
            ModerationTint::target(r#type),
            tr("report_detail_target"),
            tr(r#type.title_key()),
            false,
            None,
        )];

        if let Some(text) = report.snapshot_text.as_ref().filter(|text| !text.is_empty()) {
            items.push(ModerationFact::row(
                "content",
                "text.alignleft",
                ChatTileColor::gray(),
                tr("report_detail_snapshot"),
                text.clone(),
                true,
                None,
            ));
        }

        if report.snapshot_media.len() > 1 {
            items.push(Self::media_row(report));
        }

        if let Some(reported) = &report.reported_user {
            items.push(self.person_row(
                "author",
                reported,
                report.reported_user_id(),
                tr("case_author_caption"),
                theme,
            ));
        }

        if report.content_deleted {
            items.push(
                GroupedBtn::new("deleted", tr("report_detail_content_deleted"))
                    .icon("eye.slash.fill")
                    .icon_background(ChatTileColor::red())
                    .show_chevron(false),
            );
        } else if report.open_route().is_some() {
            let parts = self.clone();
            let open_report = report.clone();
            items.push(
                GroupedBtn::new("open", tr("moderation_open_original"))
                    .icon("arrow.up.forward.app.fill")
                    .icon_background(theme.primary._100)
                    .callback(move |_, cx| parts.open_original(&open_report, cx)),
            );
        }

        GroupedBtns::new("report-detail-subject", items)
            .title(tr("moderation_reported_content_label").to_uppercase())
            .separator_leading_padding(SEPARATOR_LEADING_PADDING)
    }

    fn open_original(&self, report: &ReportRowDTO, cx: &mut App) {
        let Some(route) = report.open_route() else {
            return;
        };
        if report.resolved_type() == ReportTargetType::Comment
            && let Some(post_id) = report.resolved_post_id()
        {
            let post_id = post_id.to_string();
            let comment_id = report.comment_id.map(|comment_id| comment_id.to_string());
            CommentInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.arm_comment_focus(&post_id, comment_id.as_deref(), false, cx);
            });
        }
        if aside_navigator::has_frontmost(cx) && aside_registry::opens_beside_centre(&route) {
            aside_navigator::open(route, cx);
            return;
        }
        self.push(route, cx);
    }

    fn decision_card(
        &self,
        value: &RiseStoredReportCase,
        is_resolving: bool,
        is_reopening: bool,
        theme: &AppTheme,
    ) -> GroupedBtns {
        let report = &value.report;
        let capabilities = &value.capabilities;
        if report.is_closed() {
            return GroupedBtns::new("report-detail-verdict", self.verdict_items(value, is_reopening, theme))
                .title(tr("report_detail_verdict").to_uppercase())
                .separator_leading_padding(SEPARATOR_LEADING_PADDING);
        }
        let r#type = report.resolved_type();
        let has_content =
            r#type != ReportTargetType::User && r#type != ReportTargetType::Artist && !report.content_deleted;
        let actions = ResolveAction::available(capabilities, has_content);
        if actions.is_empty() {
            return GroupedBtns::new(
                "report-detail-locked",
                vec![
                    GroupedBtn::new("locked", tr("moderation_no_permissions"))
                        .icon("lock.fill")
                        .icon_background(ChatTileColor::gray())
                        .show_chevron(false),
                ],
            );
        }
        let items = actions
            .into_iter()
            .map(|action| {
                let parts = self.clone();
                GroupedBtn::new(action.id(), tr(action.title_key()))
                    .icon(action.icon_name())
                    .icon_background(ModerationTint::action(action))
                    .subtitle(tr(action.hint_key()))
                    .subtitle_line_limit(2)
                    .disabled(is_resolving)
                    .callback(move |_, cx| {
                        parts.set_state(cx, |state| state.pending_action = Some(action));
                    })
            })
            .collect();
        let card = GroupedBtns::new("report-detail-actions", items)
            .title(tr("moderation_actions_label").to_uppercase())
            .end_group_title_px(END_GROUP_TITLE_PX);
        match ReportDetail::threshold_hint(capabilities, has_content) {
            Some(hint) => card.end_group_title(hint),
            None => card,
        }
    }

    fn verdict_items(&self, value: &RiseStoredReportCase, is_reopening: bool, theme: &AppTheme) -> Vec<GroupedBtn> {
        let report = &value.report;
        let mut items = Vec::new();

        if let Some(action) = report.resolved_action() {
            items.push(ModerationFact::row(
                "verdict",
                action.icon_name(),
                ModerationTint::action(action),
                tr(report.resolved_status().title_key()),
                tr(action.title_key()),
                false,
                None,
            ));
        }

        if let Some(block_type) = report.resolved_block_type() {
            let duration = report.block_duration_days.map(|days| {
                if days == 0 { tr("blocks_permanent") } else { tr("case_days").replace("{n}", &days.to_string()) }
            });
            let sanction: Vec<String> = [Some(tr(block_type.title_key())), duration].into_iter().flatten().collect();
            items.push(ModerationFact::row(
                "sanction",
                block_type.icon_name(),
                ModerationTint::block(block_type),
                tr("case_scope_label"),
                sanction.join(" \u{00B7} "),
                false,
                None,
            ));
        }

        if let Some(decision) = report.decision.as_ref().filter(|decision| !decision.is_empty()) {
            items.push(ModerationFact::row(
                "decisionText",
                "text.quote",
                ChatTileColor::gray(),
                tr("moderation_verdict_label"),
                decision.clone(),
                true,
                None,
            ));
        }

        if let Some(resolver) = &report.resolver {
            items.push(self.person_row(
                "resolver",
                resolver,
                report.resolved_by_id.clone(),
                ModerationDateFormatting::short(report.resolved_at_ms, None).unwrap_or_else(|| tr("case_resolved_by")),
                theme,
            ));
        }

        if value.capabilities.can_reopen {
            let parts = self.clone();
            items.push(
                GroupedBtn::new("reopen", tr("case_reopen_title"))
                    .icon("arrow.counterclockwise")
                    .icon_background(ChatTileColor::orange())
                    .subtitle(tr("case_reopen_subtitle"))
                    .disabled(is_reopening)
                    .callback(move |_, cx| {
                        parts.set_state(cx, |state| state.is_reopening = true);
                    }),
            );
        }

        items
    }

    fn materials_card(&self, value: &RiseStoredReportCase, theme: &AppTheme) -> GroupedBtns {
        let items = CaseMaterial::available(value)
            .into_iter()
            .map(|material| {
                let parts = self.clone();
                let tint: Hsla = material.tint(theme);
                let row = GroupedBtn::new(material.raw_value(), tr(material.title_key()))
                    .icon(material.icon_name())
                    .icon_background(tint)
                    .callback(move |_, cx| parts.open(material, cx));
                match material.count(value) {
                    Some(count) => row.trailing(RiseFormat::compact_count(count)),
                    None => row,
                }
            })
            .collect();
        GroupedBtns::new("report-detail-materials", items).separator_leading_padding(SEPARATOR_LEADING_PADDING)
    }

    /// transcript and note thread need full width and their own keyboard
    /// a sheet detent fights that so both open a screen
    fn open(&self, material: CaseMaterial, cx: &mut App) {
        match material {
            CaseMaterial::Context => {
                self.push(RootRoute::CaseContext { report_id: self.report_id.clone() }, cx);
            }
            CaseMaterial::Notes => {
                self.push(RootRoute::CaseNotes { report_id: self.report_id.clone() }, cx);
            }
            CaseMaterial::Reporters | CaseMaterial::Target | CaseMaterial::Appeals | CaseMaterial::Timeline => {
                self.set_state(cx, |state| state.open_sheet = Some(material));
            }
        }
    }

    fn admin_card(&self, value: &RiseStoredReportCase, is_sanctioning: bool) -> GroupedBtns {
        let has_active_blocks = value.target.as_ref().is_some_and(|target| !target.active_blocks.is_empty());
        let mut kinds = vec![AdminSanctionKind::Block];
        if has_active_blocks {
            kinds.push(AdminSanctionKind::Unblock);
        }
        if value.report.content_deleted {
            kinds.push(AdminSanctionKind::RestoreContent);
        }
        let items = kinds
            .into_iter()
            .map(|kind| {
                let parts = self.clone();
                GroupedBtn::new(kind.raw_value(), tr(kind.title_key()))
                    .icon(kind.icon_name())
                    .icon_background(ModerationTint::sanction(kind))
                    .subtitle(tr(kind.hint_key()))
                    .subtitle_line_limit(2)
                    .disabled(is_sanctioning)
                    .callback(move |_, cx| {
                        parts.set_state(cx, |state| state.pending_admin = Some(kind));
                    })
            })
            .collect();
        GroupedBtns::new("report-detail-admin", items)
            .title(tr("case_admin_section").to_uppercase())
            .end_group_title(tr("case_admin_hint"))
            .end_group_title_px(END_GROUP_TITLE_PX)
    }
}

#[derive(IntoElement)]
struct ReportDetailBody {
    parts: ReportDetailParts,
}

impl RenderOnce for ReportDetailBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let parts = self.parts;
        let (value, load_state) = {
            let presentation = RiseModerationPresentation::shared(cx);
            let presentation = presentation.read(cx);
            match parts.resolved_id {
                Some(id) => (presentation.cases().get(&id).cloned(), presentation.case_state(id)),
                None => (None, ModerationLoadState::Failed(tr("moderation_report_not_found"))),
            }
        };
        let body = if let Some(value) = &value {
            parts.content(value, &theme, cx)
        } else {
            match load_state {
                ModerationLoadState::Idle | ModerationLoadState::Loading => scroll_view("report-detail-skeleton")
                    .scroll_disabled(true)
                    .list_shell_scroll_under_chrome(cx)
                    .child(ModerationCaseSkeleton::new())
                    .into_any_element(),
                ModerationLoadState::Failed(message) => {
                    let retry = parts.clone();
                    div()
                        .size_full()
                        .list_shell_scroll_under_chrome(cx)
                        .child(ModerationFailureState::new("report-detail-failure", message, theme.clone(), move |_, cx| {
                            retry.open_case(cx);
                        }))
                        .into_any_element()
                }
                ModerationLoadState::Loaded => scroll_view("report-detail-empty")
                    .list_shell_scroll_under_chrome(cx)
                    .child(ModerationEmptyState::new(
                        "doc.text.magnifyingglass",
                        tr("moderation_report_not_found"),
                        theme.clone(),
                    ))
                    .into_any_element(),
            }
        };
        div().size_full().bg(theme.bg._100).child(body)
    }
}

impl RenderOnce for ReportDetail {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let report_id = self.report_id;
        let state = window.use_keyed_state(
            SharedString::from(format!("report-detail-state-{report_id}")),
            cx,
            |_, cx| ReportDetailState::new(cx),
        );
        let parts = ReportDetailParts {
            resolved_id: int_from_string(&report_id),
            report_id: report_id.clone(),
            state: state.clone(),
            nav_store: env::<NavigationStoreKey>(cx),
        };
        let sheets = parts.sheets(cx);

        let back = parts.clone();
        let keys = parts.clone();
        let task = parts.clone();
        let closing = parts.resolved_id;
        let page = SettingsWrapperUi::new(tr("moderation_report_detail_title"), move |_, cx| {
            if let Some(nav_store) = &back.nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .subtitle(SharedString::from(format!("#{report_id}")))
        .content(ReportDetailBody { parts });
        div()
            .size_full()
            .child(PageRefreshKeys::new(
                SharedString::from(format!("report-detail-keys-{report_id}")),
                move |_, cx| keys.refresh_case(cx),
                page,
            ))
            .child(sheets)
            .on_appear(SharedString::from(format!("report-detail-task-{report_id}")), move |_, cx| {
                let store = interactions(cx);
                store.update(cx, |store, cx| store.ensure_moderator_profile_loaded(cx));
                task.open_case(cx);
                if let Some(id) = task.resolved_id {
                    store.update(cx, |store, _| store.case_screen_opened(id));
                }
            })
            .and_on_disappear(move |_, cx| {
                if let Some(id) = closing {
                    interactions(cx).update(cx, |store, _| store.case_screen_closed(id));
                }
            })
    }
}
