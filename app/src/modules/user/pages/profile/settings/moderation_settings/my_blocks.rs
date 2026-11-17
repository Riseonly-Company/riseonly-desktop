use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, Entity, FontWeight, Hsla, ListAlignment, SharedString, StyleRefinement, Subscription, Task,
    Window,
};
use rise_catalog::RestrictionScope;
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::Date;
use rise_i18n::{Locale, tr};
use rise_theme::{AppTheme, ThemeStore, color};
use rise_ui::animated_tabs_ui::animated_tabs_types::TabConfig;
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::filter_pill_ui::FilterPillUi;
use rise_ui::grouped_btns::GroupedCardMetrics;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::skeleton_ui::SkeletonListUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::restriction_until_format::RestrictionUntilFormat;
use crate::core::ui::animated_tabs_ui::animated_tabs::AnimatedTabs;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::moderation::components::moderation_case_ui::RemoteEvidenceStrip;
use crate::modules::moderation::components::moderation_kit_ui::{
    ModerationArtwork, ModerationBadge, ModerationEmptyState, ModerationFailureState, moderation_symbol,
};
use crate::modules::moderation::components::moderation_type_tints::{
    AppealStatusTintExt, ReportTargetTypeTintExt, UserBlockTypeTintExt,
};
use crate::modules::moderation::engine::rise_moderation_presentation::{
    ModerationLoadState, RiseModerationPresentation,
};
use crate::modules::moderation::engine::rise_moderation_wire::{
    AppealItemDTO, RiseStoredAutoStanding, SanctionHistoryItemDTO, UserBlockDTO, WarningStateDTO,
};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AppealStatus, AppealStatusExt, AppealStatusFilter, BlockTimeFormatting, ModerationAuditAction,
    ModerationAuditActionExt, ReportReasonExt, ReportTargetTypeExt, UserBlockType, UserBlockTypeExt,
};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;
use crate::modules::user::pages::profile::settings::moderation_settings::appeal_sheet::AppealSheetStore;

const CAUTION_TINT: &str = "#F2A53C";
const DOT_SEPARATOR: &str = " \u{00B7} ";
const SKELETON_ROWS: usize = 5;
const CONTENT_BOTTOM_SPACER: f32 = 80.0;
const ARTWORK_SIDE: f32 = GroupedCardMetrics::ICON_SIDE + 11.0;

fn interactions(cx: &mut App) -> Entity<ModerationInteractionsStore> {
    ModerationInteractionsStore::shared(cx)
}

fn refresh_blocks(cx: &mut App) {
    interactions(cx).update(cx, |store, cx| store.refresh_blocks(cx));
}

fn refresh_my_appeals(cx: &mut App) {
    interactions(cx).update(cx, |store, cx| store.refresh_my_appeals(cx));
}

fn fact_icon_frame(icon: &str, theme: &AppTheme) -> Div {
    div()
        .flex_none()
        .w(px(GroupedCardMetrics::ICON_SIDE))
        .h(px(20.0))
        .flex()
        .items_center()
        .justify_center()
        .children(moderation_symbol(icon, 13.0, theme.text.secondary))
}

fn notice_row(icon: &str, text: String, theme: &AppTheme) -> Div {
    hstack(8.0)
        .w_full()
        .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
        .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
        .children(moderation_symbol(icon, 13.0, theme.text.secondary))
        .child(MainText::new(text).px(13.0).color(theme.text.secondary))
        .child(spacer_min_w(0.0))
}

#[derive(IntoElement, Default)]
pub struct MyBlocks;

impl MyBlocks {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for MyBlocks {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        SettingsWrapperUi::new(tr("blocks_page_title"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .shows_back_button(true)
        .content(MyBlocksBody)
        .on_appear("my-blocks-task", |_, cx| {
            interactions(cx).update(cx, |store, cx| {
                store.ensure_blocks_loaded(cx);
                store.ensure_my_appeals_loaded(cx);
            });
        })
    }
}

/// active tab of the body
struct MyBlocksBodyState {
    active_tab: i64,
    pages: [AnyView; 3],
    _subscriptions: Vec<Subscription>,
}

impl MyBlocksBodyState {
    fn new(cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        let opens_on_appeals =
            ModerationInteractionsStore::shared(cx).update(cx, |store, _| store.take_appeals_tab_request());
        let pages = [
            MyBlocksTabScreen::view(|| BlocksTab.into_any_element(), BlocksTab::drawn_inputs, cx),
            MyBlocksTabScreen::view(|| WarningsTab.into_any_element(), WarningsTab::drawn_inputs, cx),
            MyBlocksTabScreen::view(|| MyAppealsTab.into_any_element(), MyAppealsTab::drawn_inputs, cx),
        ];
        Self {
            active_tab: if opens_on_appeals { 2 } else { 0 },
            pages,
            _subscriptions: vec![cx.observe(&theme_store, |_, _, cx| cx.notify())],
        }
    }
}

/// one pager tab as its own cached view
/// header measures labels after first layout and notifies
/// else all three tabs relayout every frame
struct MyBlocksTabScreen {
    build: fn() -> AnyElement,
    drawn: u64,
    _subscriptions: Vec<Subscription>,
}

impl MyBlocksTabScreen {
    fn view(
        build: fn() -> AnyElement,
        inputs: fn(&RiseModerationPresentation) -> u64,
        cx: &mut Context<MyBlocksBodyState>,
    ) -> AnyView {
        cx.new(|cx| {
            let presentation = RiseModerationPresentation::shared(cx);
            let theme_store = ThemeStore::shared(cx);
            let drawn = inputs(presentation.read(cx));
            Self {
                build,
                drawn,
                _subscriptions: vec![
                    cx.observe(&presentation, move |screen: &mut Self, presentation, cx| {
                        let next = inputs(presentation.read(cx));
                        if next != screen.drawn {
                            screen.drawn = next;
                            cx.notify();
                        }
                    }),
                    cx.observe(&theme_store, |_, _, cx| cx.notify()),
                ],
            }
        })
        .into()
    }

    /// idle and loading draw the same skeleton so they hash alike
    fn hash_load_state(state: &ModerationLoadState, hasher: &mut DefaultHasher) {
        match state {
            ModerationLoadState::Idle | ModerationLoadState::Loading => 0u8.hash(hasher),
            ModerationLoadState::Loaded => 1u8.hash(hasher),
            ModerationLoadState::Failed(message) => {
                2u8.hash(hasher);
                message.hash(hasher);
            }
        }
    }

    fn cached(view: &AnyView) -> AnyElement {
        view.clone().cached(StyleRefinement::default().size_full()).replay_when_moved().into_any_element()
    }
}

impl Render for MyBlocksTabScreen {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        (self.build)()
    }
}

#[derive(IntoElement)]
struct MyBlocksBody;

impl RenderOnce for MyBlocksBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let top_inset = list_shell_floating_top_inset(cx);
        let state = window.use_keyed_state("my-blocks-body-state", cx, |_, cx| MyBlocksBodyState::new(cx));
        let [blocks, warnings, appeals] = state.read(cx).pages.clone();
        let active_tab = Binding::entity(state, |state| state.active_tab, |state, tab, _| state.active_tab = tab);
        let tabs = vec![
            TabConfig::new(0, tr("blocks_tab_blocks"), move |_, _| MyBlocksTabScreen::cached(&blocks)),
            TabConfig::new(1, tr("blocks_tab_warnings"), move |_, _| MyBlocksTabScreen::cached(&warnings)),
            TabConfig::new(2, tr("blocks_tab_appeals"), move |_, _| MyBlocksTabScreen::cached(&appeals)),
        ];
        vstack(0.0)
            .size_full()
            .pt(px(top_inset))
            .bg(theme.bg._100)
            .child(AnimatedTabs::new("my-blocks-tabs", tabs, active_tab).horizontal_padding(10.0))
    }
}

#[derive(IntoElement)]
struct BlocksCard {
    content: Div,
}

impl BlocksCard {
    fn new(content: Div) -> Self {
        Self { content }
    }
}

impl RenderOnce for BlocksCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(
            GroupedCardMetrics::SHAPE
                .round(div().w_full().overflow_hidden())
                .bg(theme.bg._200)
                .child(self.content.w_full().items_start()),
        )
    }
}

#[derive(IntoElement)]
struct BlocksSectionTitle {
    text: String,
}

impl RenderOnce for BlocksSectionTitle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .w_full()
            .px(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING + GroupedCardMetrics::HORIZONTAL_PADDING))
            .child(MainText::new(self.text.to_uppercase()).px(12.0).color(rise_ui::theme(cx).text.secondary))
    }
}

#[derive(IntoElement)]
struct BlocksRowSeparator;

impl RenderOnce for BlocksRowSeparator {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .w_full()
            .pl(px(GroupedCardMetrics::content_leading_inset()))
            .child(div().w_full().h(px(0.5)).bg(rise_ui::theme(cx).border._200.opacity(0.55)))
    }
}

#[derive(IntoElement)]
struct BlocksSkeleton {
    id: SharedString,
}

impl BlocksSkeleton {
    fn new(id: &'static str) -> Self {
        Self { id: SharedString::new_static(id) }
    }
}

impl RenderOnce for BlocksSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        scroll_view(self.id)
            .size_full()
            .scroll_disabled(true)
            .shows_indicators(false)
            .child(div().w_full().pt(px(12.0)).child(SkeletonListUi { rows: SKELETON_ROWS, ..SkeletonListUi::default() }))
    }
}

enum BlocksText {}

impl BlocksText {
    fn days(days: Option<i64>) -> String {
        match days {
            Some(days) if days > 0 => tr("blocks_history_duration").replace("{days}", &days.to_string()),
            _ => tr("blocks_permanent"),
        }
    }

    fn stamp(ms: i64) -> Option<String> {
        if ms <= 0 {
            return None;
        }
        Some(RiseFormat::date_time_label(
            &BlockTimeFormatting::date(ms),
            &Date::now(),
            &Calendar::current(),
            Locale::app_locale().identifier(),
        ))
    }

    fn scope(standing: &RiseStoredAutoStanding) -> String {
        tr(RestrictionScope::from_wire(&standing.scope).map_or("restriction_scope_generic", RestrictionScope::i18n_key))
    }
}

#[derive(IntoElement)]
struct BlocksTab;

impl BlocksTab {
    fn drawn_inputs(presentation: &RiseModerationPresentation) -> u64 {
        let mut hasher = DefaultHasher::new();
        MyBlocksTabScreen::hash_load_state(presentation.my_blocks_state(), &mut hasher);
        if *presentation.my_blocks_state() == ModerationLoadState::Loaded {
            for block in presentation.active_blocks() {
                presentation.has_pending_appeal(&block).hash(&mut hasher);
                block.hash(&mut hasher);
            }
            presentation.active_auto_mutes().hash(&mut hasher);
        }
        hasher.finish()
    }

    fn empty_state(theme: &AppTheme) -> Div {
        vstack(14.0)
            .w_full()
            .pt(px(72.0))
            .child(
                div()
                    .size(px(76.0))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.semantic.success_100.opacity(0.14))
                    .children(moderation_symbol("checkmark.shield.fill", 32.0, theme.semantic.success_100)),
            )
            .child(MainText::new(tr("blocks_empty_title")).px(17.0).tac(TextAlignT::Center).font_weight(FontWeight::BOLD))
            .child(
                div().w_full().px(px(32.0)).child(
                    MainText::new(tr("blocks_empty")).px(14.0).tac(TextAlignT::Center).color(theme.text.secondary),
                ),
            )
    }
}

impl RenderOnce for BlocksTab {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let presentation = RiseModerationPresentation::shared(cx);
        let presentation = presentation.read(cx);
        let body = match presentation.my_blocks_state().clone() {
            ModerationLoadState::Idle | ModerationLoadState::Loading => {
                BlocksSkeleton::new("my-blocks-skeleton").into_any_element()
            }
            ModerationLoadState::Failed(message) => {
                ModerationFailureState::new("my-blocks-failure", message, theme.clone(), |_, cx| refresh_blocks(cx))
                    .into_any_element()
            }
            ModerationLoadState::Loaded => {
                let blocks = presentation.active_blocks();
                let auto_mutes = presentation.active_auto_mutes();
                let content = if blocks.is_empty() && auto_mutes.is_empty() {
                    Self::empty_state(&theme)
                } else {
                    let cards = blocks.into_iter().map(|block| {
                        let appeal_pending = presentation.has_pending_appeal(&block);
                        let target = block.clone();
                        BlockCard {
                            block,
                            appeal_pending,
                            on_appeal: Rc::new(move |_: &mut Window, cx: &mut App| {
                                if !target.allows_appeal() || appeal_pending {
                                    return;
                                }
                                let target = target.clone();
                                AppealSheetStore::shared(cx).update(cx, |store, cx| store.present_block(target, cx));
                            }),
                        }
                    });
                    vstack(12.0)
                        .w_full()
                        .pt(px(12.0))
                        .children(cards)
                        .children(auto_mutes.into_iter().map(|standing| AutoMuteCard { standing }))
                        .child(spacer_min_h(CONTENT_BOTTOM_SPACER))
                };
                scroll_view("my-blocks-scroll")
                    .size_full()
                    .shows_indicators(false)
                    .refreshable(|_, cx| {
                        refresh_blocks(cx);
                        Task::ready(())
                    })
                    .child(content)
                    .into_any_element()
            }
        };
        div().size_full().bg(theme.bg._100).child(body)
    }
}

#[derive(IntoElement)]
struct BlockCard {
    block: UserBlockDTO,
    appeal_pending: bool,
    on_appeal: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl BlockCard {
    fn lifts_text(&self) -> String {
        if self.block.is_permanent() {
            return tr("blocks_permanent");
        }
        let mut parts: Vec<String> = Vec::new();
        if let Some(remaining) = BlockTimeFormatting::remaining(Some(self.block.block_end_timestamp)) {
            parts.push(tr("blocks_lifts").replace("{when}", &remaining));
        }
        if let Some(absolute) = BlockTimeFormatting::absolute(Some(self.block.block_end_timestamp)) {
            parts.push(absolute);
        }
        parts.join(DOT_SEPARATOR)
    }

    fn why_text(&self) -> Option<String> {
        if let Some(report_reason) = self.block.resolved_report_reason() {
            return Some(tr(report_reason.title_key()));
        }
        self.block.reason.clone().filter(|reason| !reason.is_empty())
    }

    fn fact(icon: &str, title: String, value: String, value_color: Option<Hsla>, theme: &AppTheme) -> Div {
        hstack(GroupedCardMetrics::ICON_SPACING)
            .w_full()
            .items_start()
            .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .py(px(10.0))
            .child(fact_icon_frame(icon, theme))
            .child(
                vstack(2.0)
                    .items_start()
                    .flex_1()
                    .min_w_0()
                    .child(MainText::new(title).px(12.0).color(theme.text.secondary))
                    .child(
                        MainText::new(value)
                            .px(14.0)
                            .font_weight(if value_color.is_some() { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                            .color(value_color.unwrap_or(theme.text.primary)),
                    ),
            )
    }

    fn appeal_section(&self, theme: &AppTheme) -> AnyElement {
        if self.appeal_pending {
            return notice_row("hourglass", tr("blocked_account_appeal_pending"), theme).into_any_element();
        }
        if !self.block.allows_appeal() {
            return notice_row("info.circle", tr("blocks_appeal_unavailable"), theme).into_any_element();
        }
        let on_appeal = self.on_appeal.clone();
        plain(SharedString::from(format!("my-block-appeal-{}", self.block.id())))
            .w_full()
            .on_press(move |window, cx| on_appeal(window, cx))
            .child(
                hstack(8.0)
                    .w_full()
                    .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                    .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
                    .children(moderation_symbol("arrow.uturn.backward.circle.fill", 15.0, theme.primary._100))
                    .child(
                        MainText::new(tr("blocks_appeal_button"))
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    )
                    .child(spacer_min_w(0.0))
                    .children(moderation_symbol(rise_ui::IconUi::trailing_chevron(), 12.0, theme.text.secondary.opacity(0.7))),
            )
            .into_any_element()
    }
}

impl RenderOnce for BlockCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let r#type = self.block.resolved_type();
        let accent = r#type.accent_color(&theme);
        let is_permanent = self.block.is_permanent();
        let block_id = self.block.id();
        let snapshot = self.block.snapshot_title.clone().filter(|snapshot| !snapshot.is_empty());
        let target_icon = self.block.resolved_report_type().map_or("doc.text", |target| target.icon_name());

        let content = vstack(0.0)
            .child(
                hstack(GroupedCardMetrics::ICON_SPACING)
                    .w_full()
                    .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                    .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
                    .child(
                        ModerationArtwork::new(
                            SharedString::from(format!("my-block-artwork-{block_id}")),
                            r#type.icon_name(),
                            accent,
                        )
                        .side(ARTWORK_SIDE),
                    )
                    .child(
                        vstack(3.0)
                            .items_start()
                            .flex_shrink(1.0)
                            .min_w_0()
                            .child(MainText::new(tr(r#type.title_key())).px(16.0).font_weight(FontWeight::SEMIBOLD))
                            .child(
                                MainText::new(tr(r#type.description_key()))
                                    .px(12.0)
                                    .color(theme.text.secondary)
                                    .line_limit(2_usize),
                            ),
                    )
                    .child(spacer_min_w(6.0)),
            )
            .child(BlocksRowSeparator)
            .child(Self::fact(
                if is_permanent { "infinity" } else { "clock" },
                tr("blocks_lifts_label"),
                self.lifts_text(),
                Some(accent),
                &theme,
            ))
            .when_some(self.why_text(), |column, reason| {
                column
                    .child(BlocksRowSeparator)
                    .child(Self::fact("text.alignleft", tr("blocks_why_label"), reason, None, &theme))
            })
            .when_some(snapshot, |column, snapshot| {
                column
                    .child(BlocksRowSeparator)
                    .child(Self::fact(target_icon, tr("blocks_target_label"), snapshot, None, &theme))
            })
            .child(BlocksRowSeparator)
            .child(self.appeal_section(&theme));

        BlocksCard::new(content)
    }
}

#[derive(IntoElement)]
struct AutoMuteCard {
    standing: RiseStoredAutoStanding,
}

impl RenderOnce for AutoMuteCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let accent = color(CAUTION_TINT);
        let content = vstack(0.0)
            .child(
                hstack(GroupedCardMetrics::ICON_SPACING)
                    .w_full()
                    .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                    .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
                    .child(
                        ModerationArtwork::new(
                            SharedString::from(format!("my-auto-mute-artwork-{}", self.standing.id())),
                            "exclamationmark.shield.fill",
                            accent,
                        )
                        .side(ARTWORK_SIDE),
                    )
                    .child(
                        vstack(3.0)
                            .items_start()
                            .flex_shrink(1.0)
                            .min_w_0()
                            .child(MainText::new(tr("blocks_auto_muted")).px(16.0).font_weight(FontWeight::SEMIBOLD))
                            .child(
                                MainText::new(BlocksText::scope(&self.standing))
                                    .px(12.0)
                                    .color(theme.text.secondary)
                                    .line_limit(2_usize),
                            ),
                    )
                    .child(spacer_min_w(6.0)),
            )
            .child(BlocksRowSeparator)
            .child(
                hstack(GroupedCardMetrics::ICON_SPACING)
                    .w_full()
                    .items_start()
                    .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                    .py(px(10.0))
                    .child(fact_icon_frame("clock", &theme))
                    .child(
                        div().flex_1().min_w_0().child(
                            MainText::new(RestrictionUntilFormat::text(self.standing.muted_until_ms, &Date::now()))
                                .px(14.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(accent),
                        ),
                    ),
            );
        BlocksCard::new(content)
    }
}

#[derive(IntoElement)]
struct WarningsTab;

impl WarningsTab {
    fn drawn_inputs(presentation: &RiseModerationPresentation) -> u64 {
        let mut hasher = DefaultHasher::new();
        MyBlocksTabScreen::hash_load_state(presentation.my_blocks_state(), &mut hasher);
        if *presentation.my_blocks_state() == ModerationLoadState::Loaded {
            presentation.my_warnings().hash(&mut hasher);
            presentation.visible_auto_standings().hash(&mut hasher);
            presentation.my_history().hash(&mut hasher);
        }
        hasher.finish()
    }
}

impl RenderOnce for WarningsTab {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let presentation = RiseModerationPresentation::shared(cx);
        let presentation = presentation.read(cx);
        let body = match presentation.my_blocks_state().clone() {
            ModerationLoadState::Idle | ModerationLoadState::Loading => {
                BlocksSkeleton::new("my-warnings-skeleton").into_any_element()
            }
            ModerationLoadState::Failed(message) => {
                ModerationFailureState::new("my-warnings-failure", message, theme.clone(), |_, cx| refresh_blocks(cx))
                    .into_any_element()
            }
            ModerationLoadState::Loaded => {
                let standings = presentation.visible_auto_standings();
                let history = presentation.my_history();
                let last_index = history.len().saturating_sub(1);
                let column = vstack(12.0)
                    .w_full()
                    .pt(px(12.0))
                    .child(WarningsHero { state: presentation.my_warnings().clone() })
                    .when(!standings.is_empty(), |column| {
                        column
                            .child(BlocksSectionTitle { text: tr("blocks_auto_title") })
                            .child(AutoStandingsCard { standings })
                    });
                let column = if history.is_empty() {
                    column.child(BlocksCard::new(
                        vstack(0.0).child(
                            hstack(10.0)
                                .w_full()
                                .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                                .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
                                .children(moderation_symbol("checkmark.circle", 15.0, theme.text.secondary))
                                .child(MainText::new(tr("blocks_history_empty")).px(14.0).color(theme.text.secondary))
                                .child(spacer_min_w(0.0)),
                        ),
                    ))
                } else {
                    let mut rows = vstack(0.0);
                    for (index, item) in history.iter().enumerate() {
                        rows = rows.child(HistoryRow { item: item.clone() });
                        if index < last_index {
                            rows = rows.child(BlocksRowSeparator);
                        }
                    }
                    column
                        .child(BlocksSectionTitle { text: tr("blocks_history_title") })
                        .child(BlocksCard::new(rows))
                };
                scroll_view("my-warnings-scroll")
                    .size_full()
                    .shows_indicators(false)
                    .refreshable(|_, cx| {
                        refresh_blocks(cx);
                        Task::ready(())
                    })
                    .child(column.child(spacer_min_h(CONTENT_BOTTOM_SPACER)))
                    .into_any_element()
            }
        };
        div().size_full().bg(theme.bg._100).child(body)
    }
}

#[derive(IntoElement)]
struct WarningsHero {
    state: WarningStateDTO,
}

impl WarningsHero {
    fn title(&self) -> String {
        if self.state.count == 0 {
            return tr("blocks_warnings_none_title");
        }
        tr("blocks_warnings_count")
            .replace("{count}", &self.state.count.to_string())
            .replace("{free}", &self.state.free_count.to_string())
    }

    fn next_step(&self) -> String {
        if self.state.next_is_block {
            return match self.state.next_block_days {
                Some(days) => tr("blocks_warnings_next_block").replace("{days}", &days.to_string()),
                None => tr("blocks_warnings_next_permanent"),
            };
        }
        if self.state.count == 0 {
            return tr("blocks_warnings_none");
        }
        tr("blocks_warnings_left").replace("{left}", &self.state.left_before_block().to_string())
    }

    fn explainer(&self) -> String {
        let ladder: Vec<String> = UserBlockType::Full.ladder().steps().iter().map(u32::to_string).collect();
        tr("blocks_warnings_explainer")
            .replace("{free}", &self.state.free_count.to_string())
            .replace("{ladder}", &ladder.join(", "))
    }
}

impl RenderOnce for WarningsHero {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let has_warnings = self.state.count > 0;
        let accent = if self.state.next_is_block {
            theme.semantic.error_100
        } else if has_warnings {
            color(CAUTION_TINT)
        } else {
            theme.semantic.success_100
        };
        let badge = div()
            .flex_none()
            .size(px(60.0))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(accent.opacity(0.14));
        let badge = if has_warnings {
            badge.child(
                MainText::new(RiseFormat::compact_count(self.state.count))
                    .px(24.0)
                    .font_weight(FontWeight::BOLD)
                    .color(accent),
            )
        } else {
            badge.children(moderation_symbol("checkmark.shield.fill", 26.0, accent))
        };
        let count = self.state.count;
        let free_count = self.state.free_count;
        let content = vstack(0.0)
            .child(
                hstack(14.0)
                    .w_full()
                    .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                    .py(px(14.0))
                    .child(badge)
                    .child(
                        vstack(4.0)
                            .items_start()
                            .flex_1()
                            .min_w_0()
                            .child(MainText::new(self.title()).px(16.0).font_weight(FontWeight::SEMIBOLD))
                            .child(MainText::new(self.next_step()).px(13.0).color(theme.text.secondary)),
                    ),
            )
            .when(free_count > 0, |column| {
                column.child(
                    hstack(6.0)
                        .w_full()
                        .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                        .pb(px(12.0))
                        .children((0..free_count).map(|index| {
                            div().flex_1().h(px(5.0)).rounded_full().bg(if index < count {
                                accent
                            } else {
                                theme.border._200
                            })
                        })),
                )
            })
            .child(BlocksRowSeparator)
            .child(
                div()
                    .w_full()
                    .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                    .py(px(12.0))
                    .child(MainText::new(self.explainer()).px(13.0).color(theme.text.secondary)),
            );
        BlocksCard::new(content)
    }
}

#[derive(IntoElement)]
struct AutoStandingsCard {
    standings: Vec<RiseStoredAutoStanding>,
}

impl RenderOnce for AutoStandingsCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let limit = self.standings.first().map_or(0, |standing| standing.strikes_to_mute);
        let mut content = vstack(0.0);
        for standing in &self.standings {
            content = content
                .child(
                    vstack(3.0)
                        .w_full()
                        .items_start()
                        .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                        .py(px(10.0))
                        .child(MainText::new(BlocksText::scope(standing)).px(14.0))
                        .child(
                            MainText::new(
                                tr("blocks_auto_strikes")
                                    .replace("{count}", &standing.strikes.to_string())
                                    .replace("{limit}", &standing.strikes_to_mute.to_string()),
                            )
                            .px(12.0)
                            .color(theme.text.secondary),
                        ),
                )
                .child(BlocksRowSeparator);
        }
        let content = content.child(
            div()
                .w_full()
                .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                .py(px(12.0))
                .child(
                    MainText::new(tr("blocks_auto_explainer").replace("{limit}", &limit.to_string()))
                        .px(13.0)
                        .color(theme.text.secondary),
                ),
        );
        BlocksCard::new(content)
    }
}

#[derive(IntoElement)]
struct HistoryRow {
    item: SanctionHistoryItemDTO,
}

impl HistoryRow {
    fn object_line(&self) -> Option<String> {
        let item = &self.item;
        let mut parts: Vec<String> = Vec::new();
        if let Some(r#type) = item.resolved_report_type() {
            parts.push(tr(r#type.title_key()));
        }
        if let Some(reason) = item.resolved_report_reason() {
            parts.push(tr(reason.title_key()));
        }
        if let Some(snapshot) = item.snapshot_title.as_ref().filter(|snapshot| !snapshot.is_empty()) {
            parts.push(snapshot.clone());
        }
        if parts.is_empty() { None } else { Some(parts.join(DOT_SEPARATOR)) }
    }

    fn chip_text(&self) -> Option<String> {
        let item = &self.item;
        match item.resolved_action() {
            Some(ModerationAuditAction::Warn) => {
                if item.block_type.is_none() {
                    return None;
                }
                if item.is_appealed() {
                    return Some(tr("appeals_sanction_lifted"));
                }
                Some(BlocksText::days(item.block_duration_days))
            }
            Some(
                ModerationAuditAction::BlockUser
                | ModerationAuditAction::Mute
                | ModerationAuditAction::Both
                | ModerationAuditAction::AdminBlock
                | ModerationAuditAction::Block,
            ) => {
                if item.is_appealed() {
                    return Some(tr("appeals_sanction_lifted"));
                }
                Some(BlocksText::days(item.block_duration_days))
            }
            Some(ModerationAuditAction::DeleteContent) => item.is_appealed().then(|| tr("appeals_sanction_lifted")),
            _ => None,
        }
    }

    fn tint_color(&self, theme: &AppTheme) -> Hsla {
        match self.item.resolved_action() {
            Some(ModerationAuditAction::Warn) => color(CAUTION_TINT),
            Some(
                ModerationAuditAction::BlockUser
                | ModerationAuditAction::Both
                | ModerationAuditAction::AdminBlock
                | ModerationAuditAction::Block
                | ModerationAuditAction::DeleteContent,
            ) => theme.semantic.error_100,
            Some(ModerationAuditAction::Mute) => theme.primary._100,
            Some(
                ModerationAuditAction::AdminUnblock
                | ModerationAuditAction::RestoreContent
                | ModerationAuditAction::AppealAccept,
            ) => theme.semantic.success_100,
            _ => theme.text.secondary,
        }
    }
}

impl RenderOnce for HistoryRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let action = self.item.resolved_action();
        let tint = self.tint_color(&theme);
        let chip = self.chip_text();
        let object_line = self.object_line();
        let item = self.item;
        let title = action.map_or_else(|| item.action.clone(), |action| tr(action.title_key()));
        let decision = item.decision.clone().filter(|decision| !decision.is_empty());
        let stamp = BlocksText::stamp(item.created_at_ms);
        let allows_appeal = item.allows_appeal();
        let item_id = item.id;

        hstack(GroupedCardMetrics::ICON_SPACING)
            .w_full()
            .items_start()
            .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
            .child(
                Shape::continuous(8.0)
                    .round(div())
                    .flex_none()
                    .size(px(GroupedCardMetrics::ICON_SIDE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(tint.opacity(0.14))
                    .children(moderation_symbol(action.map_or("shield", |action| action.icon_name()), 13.0, tint)),
            )
            .child(
                vstack(3.0)
                    .items_start()
                    .flex_1()
                    .min_w_0()
                    .child(
                        hstack(8.0)
                            .w_full()
                            .items_baseline()
                            .child(
                                MainText::new(title).px(15.0).font_weight(FontWeight::MEDIUM).number_of_lines(1),
                            )
                            .child(spacer_min_w(4.0))
                            .when_some(chip, |row, chip| row.child(ModerationBadge::new(chip).color(tint))),
                    )
                    .when_some(object_line, |column, object| {
                        column.child(MainText::new(object).px(13.0).color(theme.text.secondary).number_of_lines(2))
                    })
                    .when_some(decision, |column, decision| {
                        column.child(MainText::new(decision).px(13.0).color(theme.text.primary).number_of_lines(3))
                    })
                    .when_some(stamp, |column, stamp| {
                        column.child(MainText::new(stamp).px(11.0).color(theme.text.secondary))
                    })
                    .when(allows_appeal, |column| {
                        column.child(
                            plain(SharedString::from(format!("my-history-appeal-{item_id}")))
                                .on_press(move |_, cx| {
                                    let item = item.clone();
                                    AppealSheetStore::shared(cx)
                                        .update(cx, |store, cx| store.present_sanction(item, cx));
                                })
                                .child(
                                    hstack(6.0)
                                        .pt(px(4.0))
                                        .children(moderation_symbol("arrow.uturn.backward.circle.fill", 14.0, theme.primary._100))
                                        .child(
                                            MainText::new(tr("blocks_appeal_button"))
                                                .px(14.0)
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .color(theme.primary._100),
                                        ),
                                ),
                        )
                    }),
            )
    }
}

struct MyAppealsTabState {
    list: ListUi,
    list_ids: Vec<i64>,
    list_rows: Vec<u64>,
    list_filter: Option<AppealStatusFilter>,
}

impl MyAppealsTabState {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            list_rows: Vec::new(),
            list_filter: None,
        }
    }

    fn row_hash(item: &AppealItemDTO) -> u64 {
        let mut hasher = DefaultHasher::new();
        item.hash(&mut hasher);
        hasher.finish()
    }

    fn sync_list(&mut self, filter: AppealStatusFilter, appeals: &[AppealItemDTO]) {
        let ids: Vec<i64> = appeals.iter().map(AppealItemDTO::id).collect();
        let rows: Vec<u64> = appeals.iter().map(Self::row_hash).collect();
        if ids == self.list_ids && rows == self.list_rows && self.list_filter == Some(filter) {
            return;
        }
        let old = &self.list_ids;
        if self.list_filter != Some(filter) || old.is_empty() {
            self.list.replaced(ids.len() + 1);
        } else if ids.len() > old.len() && ids.starts_with(old) && rows.starts_with(&self.list_rows) {
            self.list.appended(ids.len() - old.len());
        } else {
            let old_len = old.len();
            let new_len = ids.len();
            self.list.rebuilt(new_len + 1, |old_index| {
                if old_index >= old_len {
                    return Some(new_len);
                }
                let id = old[old_index];
                ids.iter().position(|candidate| *candidate == id)
            });
        }
        self.list_ids = ids;
        self.list_rows = rows;
        self.list_filter = Some(filter);
    }
}

#[derive(IntoElement)]
struct MyAppealsTab;

impl MyAppealsTab {
    fn drawn_inputs(presentation: &RiseModerationPresentation) -> u64 {
        let mut hasher = DefaultHasher::new();
        MyBlocksTabScreen::hash_load_state(presentation.my_appeals_state(), &mut hasher);
        presentation.my_appeals_filter().hash(&mut hasher);
        presentation.my_appeals().hash(&mut hasher);
        presentation.is_loading_more_my_appeals().hash(&mut hasher);
        hasher.finish()
    }

    fn filter_pill(cx: &mut App) -> Div {
        let current = RiseModerationPresentation::shared(cx).read(cx).my_appeals_filter();
        let items = AppealStatusFilter::ALL
            .iter()
            .copied()
            .enumerate()
            .map(|(index, filter)| {
                let key = if matches!(filter, AppealStatusFilter::All) { "" } else { filter.raw_value() };
                ContextMenuItem::new(index as i64, tr(filter.title_key()), None, key, move |_, cx| {
                    interactions(cx).update(cx, |store, cx| store.set_my_appeals_filter(filter, cx));
                })
            })
            .collect();
        let selected_key = if matches!(current, AppealStatusFilter::All) { "" } else { current.raw_value() };
        hstack(8.0)
            .w_full()
            .px(px(16.0))
            .pt(px(10.0))
            .pb(px(4.0))
            .child(FilterPillUi::new("my-appeals-filter", items, selected_key, tr(current.title_key())))
            .child(spacer_min_w(0.0))
    }

    fn list(state: Entity<MyAppealsTabState>, cx: &mut App) -> AnyElement {
        let (appeals, is_loading_more, filter) = {
            let presentation = RiseModerationPresentation::shared(cx);
            let presentation = presentation.read(cx);
            (
                presentation.my_appeals().to_vec(),
                presentation.is_loading_more_my_appeals(),
                presentation.my_appeals_filter(),
            )
        };
        state.update(cx, |state, _| state.sync_list(filter, &appeals));
        let last_id = appeals.last().map(AppealItemDTO::id);
        state
            .read(cx)
            .list
            .refreshable(
                "my-appeals-list",
                |_, cx| {
                    refresh_my_appeals(cx);
                    Task::ready(())
                },
                move |index, _, _| {
                    let Some(item) = appeals.get(index) else {
                        return vstack(0.0)
                            .w_full()
                            .when(is_loading_more, |column| column.child(div().py(px(8.0)).child(progress_view())))
                            .child(div().w_full().h(px(CONTENT_BOTTOM_SPACER)))
                            .into_any_element();
                    };
                    let appeal_id = item.id();
                    vstack(0.0)
                        .w_full()
                        .pt(px(if index == 0 { 8.0 } else { 0.0 }))
                        .pb(px(12.0))
                        .child(MyAppealCard { item: item.clone() })
                        .on_appear(SharedString::from(format!("my-appeal-appear-{appeal_id}")), move |_, cx| {
                            if Some(appeal_id) == last_id {
                                interactions(cx).update(cx, |store, cx| store.load_more_my_appeals(cx));
                            }
                        })
                        .into_any_element()
                },
            )
            .into_any_element()
    }
}

impl RenderOnce for MyAppealsTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("my-appeals-tab-state", cx, |_, cx| MyAppealsTabState::new(cx));
        let (load_state, is_empty) = {
            let presentation = RiseModerationPresentation::shared(cx);
            let presentation = presentation.read(cx);
            (presentation.my_appeals_state().clone(), presentation.my_appeals().is_empty())
        };
        let body = match load_state {
            ModerationLoadState::Idle | ModerationLoadState::Loading => {
                BlocksSkeleton::new("my-appeals-skeleton").into_any_element()
            }
            ModerationLoadState::Failed(message) => {
                ModerationFailureState::new("my-appeals-failure", message, theme.clone(), |_, cx| {
                    refresh_my_appeals(cx);
                })
                .into_any_element()
            }
            ModerationLoadState::Loaded if is_empty => scroll_view("my-appeals-empty")
                .size_full()
                .shows_indicators(false)
                .refreshable(|_, cx| {
                    refresh_my_appeals(cx);
                    Task::ready(())
                })
                .child(ModerationEmptyState::new("arrow.uturn.backward.circle", tr("appeals_empty"), theme.clone()))
                .into_any_element(),
            ModerationLoadState::Loaded => Self::list(state, cx),
        };
        vstack(0.0)
            .size_full()
            .bg(theme.bg._100)
            .child(Self::filter_pill(cx))
            .child(div().w_full().flex_1().min_h_0().child(body))
    }
}

#[derive(IntoElement)]
struct MyAppealCard {
    item: AppealItemDTO,
}

impl RenderOnce for MyAppealCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let item = self.item;
        let status = item.resolved_status();
        let status_color = status.color(&theme);
        let appeal = &item.appeal;
        let appeal_id = appeal.id;
        let block_type = item.resolved_block_type();
        let report_type = item.resolved_report_type();
        let evidence = appeal.evidence();
        let has_details = !appeal.text.is_empty() || !evidence.is_empty();

        let artwork_icon = block_type
            .map(|block_type| block_type.icon_name())
            .or_else(|| report_type.map(|report_type| report_type.icon_name()))
            .unwrap_or("arrow.uturn.backward.circle");
        let artwork_tint = block_type.map_or(theme.primary._100, |block_type| block_type.accent_color(&theme));
        let title = block_type
            .map(|block_type| tr(block_type.title_key()))
            .or_else(|| report_type.map(|report_type| tr(report_type.title_key())))
            .unwrap_or_else(|| tr("blocks_tab_appeals"));
        let snapshot = item.snapshot_title.clone().filter(|snapshot| !snapshot.is_empty());
        let reason = item.resolved_report_reason();
        let subtitle = snapshot.map(|snapshot| (snapshot, true)).or_else(|| {
            reason.map(|reason| (tr(reason.title_key()), false))
        });
        let decision = appeal.decision_text.clone().filter(|decision| !decision.is_empty());

        let footer = if status != AppealStatus::Pending {
            vstack(6.0)
                .w_full()
                .items_start()
                .when_some(decision, |column, decision| {
                    column
                        .child(MainText::new(tr("appeals_verdict")).px(12.0).color(status_color))
                        .child(MainText::new(decision).px(14.0).number_of_lines(6))
                })
                .when(status == AppealStatus::Accepted, |column| {
                    column.child(
                        hstack(6.0)
                            .children(moderation_symbol("checkmark.seal.fill", 13.0, theme.semantic.success_100))
                            .child(
                                MainText::new(tr("appeals_sanction_lifted"))
                                    .px(13.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.semantic.success_100),
                            ),
                    )
                })
                .when_some(appeal.reviewed_date_text(), |column, reviewed_at| {
                    column.child(
                        MainText::new(tr("appeals_reviewed_at").replace("{when}", &reviewed_at))
                            .px(11.0)
                            .color(theme.text.secondary),
                    )
                })
        } else {
            vstack(6.0).w_full().items_start().when_some(appeal.created_date_text(), |column, created_at| {
                column.child(
                    MainText::new(tr("appeals_submitted_at").replace("{when}", &created_at))
                        .px(11.0)
                        .color(theme.text.secondary),
                )
            })
        };

        let content = vstack(0.0)
            .child(
                hstack(GroupedCardMetrics::ICON_SPACING)
                    .w_full()
                    .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                    .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
                    .child(
                        ModerationArtwork::new(
                            SharedString::from(format!("my-appeal-artwork-{appeal_id}")),
                            artwork_icon,
                            artwork_tint,
                        )
                        .side(ARTWORK_SIDE),
                    )
                    .child(
                        vstack(3.0)
                            .items_start()
                            .flex_shrink(1.0)
                            .min_w_0()
                            .child(MainText::new(title).px(16.0).font_weight(FontWeight::SEMIBOLD))
                            .when_some(subtitle, |column, (text, is_snapshot)| {
                                let line = MainText::new(text).px(12.0).color(theme.text.secondary);
                                column.child(if is_snapshot { line.line_limit(2_usize) } else { line })
                            }),
                    )
                    .child(spacer_min_w(6.0))
                    .child(
                        ModerationBadge::new(tr(status.title_key()))
                            .color(status_color)
                            .icon(SharedString::from(status.icon_name())),
                    ),
            )
            .when(has_details, |column| {
                column.child(BlocksRowSeparator).child(
                    vstack(8.0)
                        .w_full()
                        .items_start()
                        .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                        .py(px(10.0))
                        .when(!appeal.text.is_empty(), |details| {
                            details
                                .child(MainText::new(tr("appeals_your_text")).px(12.0).color(theme.text.secondary))
                                .child(MainText::new(appeal.text.clone()).px(14.0).number_of_lines(6))
                        })
                        .when(!evidence.is_empty(), |details| {
                            details.child(
                                RemoteEvidenceStrip::new(
                                    SharedString::from(format!("my-appeal-evidence-{appeal_id}")),
                                    evidence,
                                )
                                .size(64.0),
                            )
                        }),
                )
            })
            .child(BlocksRowSeparator)
            .child(
                div()
                    .w_full()
                    .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                    .py(px(10.0))
                    .child(footer),
            );
        BlocksCard::new(content)
    }
}
