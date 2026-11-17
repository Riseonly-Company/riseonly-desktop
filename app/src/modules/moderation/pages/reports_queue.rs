use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Context, ElementId, Entity, FontWeight, KeyDownEvent, ListAlignment, SharedString, StyleRefinement, Subscription,
    Task, Window,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::grouped_btns::GroupedCardMetrics;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::MainText;
use rise_ui::page_header_toolbar_button::PageHeaderToolbarSlot;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::page_refresh::{PageFocus, is_page_refresh_keystroke};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::AnimatedTabsPagerLiveState;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, ListShellUi, list_shell_floating_top_inset,
};
use crate::modules::moderation::components::moderation_kit_ui::{
    ModerationArtwork, ModerationEmptyState, ModerationFailureState, ModerationFilterPager, ModerationListSkeleton,
    ModerationMetrics, ModerationPersonRow, ModerationRowSeparator, ModerationSegmentedFilter, ModerationTint,
    moderation_symbol,
};
use crate::modules::moderation::components::moderation_type_tints::ReportTargetTypeTintExt;
use crate::modules::moderation::engine::rise_moderation_presentation::{
    ModerationLoadState, RiseModerationPresentation,
};
use crate::modules::moderation::engine::rise_moderation_wire::ReportRowDTO;
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    ModeratorScopeType, ReportQueueSort, ReportQueueSortExt, ReportReasonExt, ReportStatus, ReportStatusExt,
    ReportTargetType, ReportTargetTypeExt,
};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;
use crate::modules::moderation::stores::moderation_services::moderation_services::ModerationServicesStore;

const SKELETON_ROWS: usize = 10;
const LOAD_MORE_THRESHOLD: usize = 6;
const TYPE_MENU_ALL_ID: i64 = -1;
const ROW_HEIGHT: f32 = ModerationMetrics::AVATAR_SIZE + 20.0;

fn interactions(cx: &mut App) -> Entity<ModerationInteractionsStore> {
    ModerationInteractionsStore::shared(cx)
}

fn statuses() -> [ReportStatus; 4] {
    ReportStatus::ALL
}

struct ReportsQueueState {
    pager_live_state: Entity<AnimatedTabsPagerLiveState>,
    menu_tap_guard: Rc<RefCell<ReportsQueueMenuTapGuard>>,
    list: ListUi,
    list_ids: Vec<i64>,
    queue: Rc<Vec<ReportRowDTO>>,
    keyboard_index: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

impl ReportsQueueState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseModerationPresentation::shared(cx);
        let roles = ModerationServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            pager_live_state: cx.new(|_| AnimatedTabsPagerLiveState::new()),
            menu_tap_guard: Rc::new(RefCell::new(ReportsQueueMenuTapGuard::new())),
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            queue: Rc::new(presentation.read(cx).queue().to_vec()),
            keyboard_index: None,
            _subscriptions: vec![
                cx.observe(&presentation, |this, presentation, cx| {
                    let queue = presentation.read(cx).queue();
                    if this.queue.as_slice() != queue {
                        this.queue = Rc::new(queue.to_vec());
                    }
                    cx.notify();
                }),
                cx.observe(&roles, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }

    /// rows to show, with the list told about any change since the last frame
    fn sync_queue(&mut self) -> Rc<Vec<ReportRowDTO>> {
        let queue = self.queue.clone();
        if !self.list_ids.iter().copied().eq(queue.iter().map(|report| report.id)) {
            self.sync_list(queue.iter().map(|report| report.id).collect());
        }
        queue
    }

    fn sync_list(&mut self, ids: Vec<i64>) {
        if ids == self.list_ids {
            return;
        }
        let old = &self.list_ids;
        if old.is_empty() {
            self.list.replaced(ids.len() + 1);
        } else if ids.len() > old.len() && ids.starts_with(old) {
            self.list.appended(ids.len() - old.len());
        } else if ids.len() > old.len() && ids.ends_with(old) {
            self.list.prepended(ids.len() - old.len());
        } else if ids.len() + 1 == old.len() {
            let removed = old.iter().zip(ids.iter()).take_while(|(left, right)| left == right).count();
            if old[removed + 1..] == ids[removed..] {
                self.list.removed(removed..removed + 1);
            } else {
                self.list.replaced(ids.len() + 1);
            }
        } else {
            self.list.replaced(ids.len() + 1);
        }
        self.keyboard_index = self.keyboard_index.filter(|index| *index < ids.len());
        self.list_ids = ids;
    }

    fn move_keyboard_index(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(last) = self.list_ids.len().checked_sub(1) else {
            return;
        };
        let next = match (self.keyboard_index, forward) {
            (None, _) => 0,
            (Some(index), true) => (index + 1).min(last),
            (Some(index), false) => index.saturating_sub(1),
        };
        self.keyboard_index = Some(next);
        self.list.state().scroll_to_reveal_item(next);
        cx.notify();
    }
}

#[derive(IntoElement, Default)]
pub struct ReportsQueue;

impl ReportsQueue {
    pub fn new() -> Self {
        Self
    }

    fn status_index(cx: &mut App) -> i64 {
        let status = RiseModerationPresentation::shared(cx).read(cx).queue_filter().status;
        statuses().iter().position(|candidate| *candidate == status).unwrap_or(0) as i64
    }

    fn subtitle(cx: &mut App) -> Option<SharedString> {
        let presentation = RiseModerationPresentation::shared(cx);
        let presentation = presentation.read(cx);
        if *presentation.queue_state() != ModerationLoadState::Loaded {
            return None;
        }
        if presentation.queue_filter().target_user_id.as_deref().is_some_and(|target| !target.is_empty()) {
            return Some(tr("moderation_queue_for_user").into());
        }
        let count = RiseFormat::compact_count(presentation.queue_total());
        Some(tr("moderation_queue_count").replace("{n}", &count).into())
    }

    fn select_status(index: i64, cx: &mut App) {
        let Some(status) = usize::try_from(index).ok().and_then(|index| statuses().get(index).copied()) else {
            return;
        };
        interactions(cx).update(cx, |store, cx| store.set_queue_status_filter(status, cx));
    }

    fn target_banner(theme: &AppTheme) -> Div {
        hstack(8.0)
            .w_full()
            .items_center()
            .px(px(14.0))
            .py(px(8.0))
            .bg(theme.bg._200)
            .capsule()
            .children(moderation_symbol("person.crop.circle.badge.exclamationmark", 13.0, theme.primary._100))
            .child(MainText::new(tr("moderation_queue_for_user")).px(13.0).font_weight(FontWeight::MEDIUM))
            .child(spacer_min_w(6.0))
            .child(
                plain("moderation-queue-target-close")
                    .on_press(|_, cx| {
                        interactions(cx).update(cx, |store, cx| store.set_queue_target(None, cx));
                    })
                    .child(
                        div()
                            .size(px(26.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme.bg._300)
                            .circle()
                            .children(moderation_symbol("xmark", 11.0, theme.text.secondary)),
                    ),
            )
    }

    fn toolbar(state: &Entity<ReportsQueueState>, theme: &AppTheme, cx: &mut App) -> Div {
        let scopes = ModerationServicesStore::moderatable_scope_types(cx);
        let shows_type = scopes.len() > 1 || ModerationServicesStore::is_admin(cx);
        let guard = state.read(cx).menu_tap_guard.clone();
        let filter = RiseModerationPresentation::shared(cx).read(cx).queue_filter().clone();
        hstack(6.0)
            .items_center()
            .when(shows_type, |row| row.child(Self::type_menu(scopes, filter.r#type, guard.clone(), theme)))
            .child(Self::sort_menu(filter.sort, guard, theme))
    }

    fn type_menu(
        scopes: Vec<ModeratorScopeType>,
        selected: Option<ReportTargetType>,
        guard: Rc<RefCell<ReportsQueueMenuTapGuard>>,
        theme: &AppTheme,
    ) -> ContextMenuUi {
        let mut items = vec![ContextMenuItem::new(TYPE_MENU_ALL_ID, tr("report_filter_all"), None, "", |_, cx| {
            interactions(cx).update(cx, |store, cx| store.set_queue_type_filter(None, cx));
        })];
        items.extend(scopes.into_iter().enumerate().map(|(index, scope)| {
            ContextMenuItem::new(
                index as i64,
                tr(scope.title_key()),
                Some(ReportTargetTypeTintExt::icon_name(scope).to_owned()),
                scope.id(),
                move |_, cx| {
                    interactions(cx).update(cx, |store, cx| store.set_queue_type_filter(Some(scope), cx));
                },
            )
        }));
        let icon = selected.map_or("line.3.horizontal.decrease", ReportTargetTypeTintExt::icon_name);
        let color = if selected.is_none() { theme.text.primary } else { theme.primary._100 };
        ContextMenuUi::new(
            "moderation-queue-type-menu",
            items,
            PageHeaderToolbarSlot::new(div().children(IconUi::sized(icon, px(16.0), color, false))),
        )
        .selected(selected.map_or(String::new(), |selected| selected.id().to_owned()))
        .on_presented_change(move |presented, _, _| guard.borrow_mut().menu_presented_changed(presented))
    }

    fn sort_menu(
        selected: ReportQueueSort,
        guard: Rc<RefCell<ReportsQueueMenuTapGuard>>,
        theme: &AppTheme,
    ) -> ContextMenuUi {
        let items = ReportQueueSort::ALL
            .into_iter()
            .enumerate()
            .map(|(index, sort)| {
                ContextMenuItem::new(index as i64, tr(sort.title_key()), None, sort.id(), move |_, cx| {
                    interactions(cx).update(cx, |store, cx| store.set_queue_sort(sort, cx));
                })
            })
            .collect();
        ContextMenuUi::new(
            "moderation-queue-sort-menu",
            items,
            PageHeaderToolbarSlot::new(div().children(IconUi::sized(
                "arrow.up.arrow.down",
                px(16.0),
                theme.text.primary,
                false,
            ))),
        )
        .selected(selected.id().to_owned())
        .on_presented_change(move |presented, _, _| guard.borrow_mut().menu_presented_changed(presented))
    }

    fn open_report(nav_store: Option<&Entity<NavigationStore>>, report_id: i64, cx: &mut App) {
        let Some(nav_store) = nav_store else {
            return;
        };
        let route = RootRoute::ReportDetail { report_id: report_id.to_string() };
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }

    fn handle_key(
        state: &Entity<ReportsQueueState>,
        nav_store: Option<&Entity<NavigationStore>>,
        is_root_focused: bool,
        event: &KeyDownEvent,
        cx: &mut App,
    ) {
        let keystroke = &event.keystroke;
        if is_page_refresh_keystroke(keystroke) {
            cx.stop_propagation();
            interactions(cx).update(cx, |store, cx| store.refresh_queue(cx));
            return;
        }
        if keystroke.modifiers.modified() || !is_root_focused {
            return;
        }
        match keystroke.key.as_str() {
            "up" | "down" => {
                cx.stop_propagation();
                let forward = keystroke.key == "down";
                state.update(cx, |state, cx| state.move_keyboard_index(forward, cx));
            }
            "enter" => {
                let selected = {
                    let state = state.read(cx);
                    state.keyboard_index.and_then(|index| state.list_ids.get(index).copied())
                };
                if let Some(report_id) = selected {
                    cx.stop_propagation();
                    Self::open_report(nav_store, report_id, cx);
                }
            }
            _ => {}
        }
    }
}

impl RenderOnce for ReportsQueue {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("reports-queue-state", cx, |_, cx| ReportsQueueState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let live_state = state.read(cx).pager_live_state.clone();
        let status_index = Self::status_index(cx);
        let subtitle = Self::subtitle(cx);
        let has_target = RiseModerationPresentation::shared(cx)
            .read(cx)
            .queue_filter()
            .target_user_id
            .as_deref()
            .is_some_and(|target| !target.is_empty());
        let toolbar = Self::toolbar(&state, &theme, cx);

        let below_header = vstack(0.0)
            .w_full()
            .child(
                ModerationSegmentedFilter::new(
                    "moderation-queue-filter",
                    statuses().iter().map(|status| SharedString::from(tr(status.title_key()))).collect(),
                    status_index,
                    |index, _, cx| Self::select_status(index, cx),
                )
                .live_drag_state(Some(live_state.clone())),
            )
            .when(has_target, |column| {
                column.child(
                    div()
                        .w_full()
                        .px(px(GroupedCardMetrics::HORIZONTAL_PADDING))
                        .pb(px(8.0))
                        .child(Self::target_banner(&theme)),
                )
            });

        let pager = ModerationFilterPager::new(
            "moderation-queue-pager",
            statuses().len() as i64,
            status_index,
            live_state,
            |index, _, cx| Self::select_status(index, cx),
            ReportsQueueBody { state: state.clone(), nav_store: nav_store.clone() },
        )
        .skeleton_rows(SKELETON_ROWS);

        let back_store = nav_store.clone();
        let key_state = state;
        let page_focus = PageFocus::keyed("reports-queue-focus", window, cx);
        let focus = page_focus.read(cx).handle.clone();
        let key_focus = focus.clone();
        let leaving_focus = page_focus.clone();
        div()
            .id("reports-queue-keys")
            .size_full()
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                Self::handle_key(&key_state, nav_store.as_ref(), key_focus.is_focused(window), event, cx);
            })
            .child(
                ListShellUi::new()
                    .title(SharedString::from(tr("moderation_queue_title")))
                    .subtitle(subtitle)
                    .shows_back_button(true)
                    .on_back(move |_, cx| {
                        if let Some(nav_store) = &back_store {
                            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                        }
                    })
                    .right_jsx(toolbar)
                    .below_header(below_header)
                    .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
                    .content(div().size_full().bg(theme.bg._100).child(pager)),
            )
            .on_appear("reports-queue-task", move |window, cx| {
                PageFocus::did_appear(&page_focus, window, cx);
                interactions(cx).update(cx, |store, cx| {
                    store.ensure_moderator_profile_loaded(cx);
                    store.open_queue(cx);
                });
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
            })
    }
}

#[derive(IntoElement)]
struct ReportsQueueBody {
    state: Entity<ReportsQueueState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl ReportsQueueBody {
    fn list(self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let top_inset = list_shell_floating_top_inset(cx);
        let (cursor, is_loading_more) = {
            let presentation = RiseModerationPresentation::shared(cx);
            let presentation = presentation.read(cx);
            (presentation.queue_cursor(), presentation.is_loading_more_queue())
        };
        let queue = self.state.update(cx, |state, _| state.sync_queue());
        let (keyboard_index, guard) = {
            let state = self.state.read(cx);
            (state.keyboard_index, state.menu_tap_guard.clone())
        };
        let count = queue.len();
        let selected_background = theme.bg._200;
        let nav_store = self.nav_store;
        self.state
            .read(cx)
            .list
            .refreshable(
                "moderation-queue-list",
                |_, cx| {
                    interactions(cx).update(cx, |store, cx| store.refresh_queue(cx));
                    Task::ready(())
                },
                move |index, window, cx| {
                    let Some(report) = queue.get(index) else {
                        let sentinel = div().w_full().h(px(1.0)).on_appear(
                            SharedString::from(format!("queue-end-{}", cursor.unwrap_or(-1))),
                            |_, cx| {
                                interactions(cx).update(cx, |store, cx| store.load_more_queue(cx));
                            },
                        );
                        return vstack(0.0)
                            .w_full()
                            .child(sentinel)
                            .when(is_loading_more, |column| column.child(div().py(px(12.0)).child(progress_view())))
                            .child(div().w_full().h(px(ModerationMetrics::BOTTOM_INSET)))
                            .into_any_element();
                    };
                    let report_id = report.id;
                    let row_view = window.use_keyed_state(
                        SharedString::from(format!("moderation-queue-row-view-{report_id}")),
                        cx,
                        |_, cx| ReportQueueRowView::new(report.clone(), cx),
                    );
                    row_view.update(cx, |view, cx| view.set_report(report, cx));
                    let tap_guard = guard.clone();
                    let row_store = nav_store.clone();
                    vstack(0.0)
                        .w_full()
                        .when(index == 0, |column| column.pt(px(top_inset)))
                        .when(keyboard_index == Some(index), |column| column.bg(selected_background))
                        .child(
                            plain(SharedString::from(format!("moderation-queue-row-{report_id}")))
                                .w_full()
                                .on_press(move |_, cx| {
                                    if !tap_guard.borrow_mut().allows_tap() {
                                        return;
                                    }
                                    ReportsQueue::open_report(row_store.as_ref(), report_id, cx);
                                })
                                .child(
                                    row_view
                                        .cached(StyleRefinement::default().w_full().h(px(ROW_HEIGHT)))
                                        .replay_when_moved(),
                                ),
                        )
                        .when(index + 1 < count, |column| column.child(ModerationRowSeparator::new()))
                        .on_appear(SharedString::from(format!("moderation-queue-appear-{report_id}")), move |_, cx| {
                            if index + LOAD_MORE_THRESHOLD >= count {
                                interactions(cx).update(cx, |store, cx| store.load_more_queue(cx));
                            }
                        })
                        .into_any_element()
                },
            )
            .into_any_element()
    }
}

impl RenderOnce for ReportsQueueBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let (queue_state, is_empty) = {
            let presentation = RiseModerationPresentation::shared(cx);
            let presentation = presentation.read(cx);
            (presentation.queue_state().clone(), presentation.queue().is_empty())
        };
        if is_empty || !matches!(queue_state, ModerationLoadState::Loaded) {
            self.state.update(cx, |state, _| state.sync_list(Vec::new()));
        }
        match queue_state {
            ModerationLoadState::Idle | ModerationLoadState::Loading => scroll_view("moderation-queue-skeleton")
                .scroll_disabled(true)
                .shows_indicators(false)
                .list_shell_scroll_under_chrome(cx)
                .child(ModerationListSkeleton::new().rows(SKELETON_ROWS))
                .into_any_element(),
            ModerationLoadState::Failed(message) => {
                div()
                    .size_full()
                    .list_shell_scroll_under_chrome(cx)
                    .child(ModerationFailureState::new("moderation-queue-failure", message, theme, |_, cx| {
                        interactions(cx).update(cx, |store, cx| store.refresh_queue(cx));
                    }))
                    .into_any_element()
            }
            ModerationLoadState::Loaded if is_empty => scroll_view("moderation-queue-empty")
                .refreshable(|_, cx| {
                    interactions(cx).update(cx, |store, cx| store.refresh_queue(cx));
                    Task::ready(())
                })
                .list_shell_scroll_under_chrome(cx)
                .child(ModerationEmptyState::new("tray", tr("moderation_queue_empty"), theme))
                .into_any_element(),
            ModerationLoadState::Loaded => self.list(&theme, cx),
        }
    }
}

/// the click that closes the type or sort menu also lands on the row under it, it only closes the menu
/// plain class, opening the menu mustnt rebuild the list under it
pub struct ReportsQueueMenuTapGuard {
    is_menu_open: bool,
    closed_at: Option<Duration>,
}

impl ReportsQueueMenuTapGuard {
    const CLOSING_WINDOW: Duration = Duration::from_millis(450);

    pub fn new() -> Self {
        Self { is_menu_open: false, closed_at: None }
    }

    pub fn menu_presented_changed(&mut self, presented: bool) {
        self.is_menu_open = presented;
        if !presented {
            self.closed_at = Some(RiseClock::monotonic());
        }
    }

    pub fn allows_tap(&mut self) -> bool {
        if self.is_menu_open {
            self.is_menu_open = false;
            self.closed_at = Some(RiseClock::monotonic());
            return false;
        }
        self.closed_at.is_none_or(|closed_at| RiseClock::monotonic().saturating_sub(closed_at) > Self::CLOSING_WINDOW)
    }
}

impl Default for ReportsQueueMenuTapGuard {
    fn default() -> Self {
        Self::new()
    }
}

struct ReportQueueRowView {
    report: ReportRowDTO,
    _theme: Subscription,
}

impl ReportQueueRowView {
    fn new(report: ReportRowDTO, cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        Self { report, _theme: cx.observe(&theme_store, |_, _, cx| cx.notify()) }
    }

    fn set_report(&mut self, report: &ReportRowDTO, cx: &mut Context<Self>) {
        if self.report != *report {
            self.report = report.clone();
            cx.notify();
        }
    }
}

impl Render for ReportQueueRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        ReportQueueRow::new(self.report.clone())
    }
}

#[derive(IntoElement)]
pub struct ReportQueueRow {
    report: ReportRowDTO,
}

impl ReportQueueRow {
    pub fn new(report: ReportRowDTO) -> Self {
        Self { report }
    }

    fn avatar(&self, r#type: ReportTargetType) -> AnyElement {
        let report = &self.report;
        let side = ModerationMetrics::AVATAR_SIZE;
        let id: ElementId = SharedString::from(format!("moderation-queue-avatar-{}", report.id)).into();
        if let Some(reported) = report.reported_user.as_ref().filter(|_| {
            r#type == ReportTargetType::User || r#type == ReportTargetType::Artist || report.snapshot_media.is_empty()
        }) {
            return UserLogoUi::new(id)
                .source(reported.logo.clone().map(SharedString::from))
                .name(reported.name.clone().map(SharedString::from))
                .user_id(report.reported_user_id().or_else(|| reported.tag.clone()).map(SharedString::from))
                .size(side)
                .into_any_element();
        }
        ModerationArtwork::new(id, ReportTargetTypeTintExt::icon_name(r#type), ModerationTint::target(r#type))
            .image_source(report.snapshot_media.first().cloned().map(SharedString::from))
            .side(side)
            .into_any_element()
    }
}

impl RenderOnce for ReportQueueRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let r#type = self.report.resolved_type();
        let avatar = self.avatar(r#type);
        let report = self.report;
        let subtitle = report.resolved_reason().map_or_else(|| tr(r#type.title_key()), |reason| tr(reason.title_key()));
        let badge_color = if report.report_count >= 3 { theme.semantic.error_100 } else { theme.text.secondary };
        ModerationPersonRow::new(SharedString::from(format!("moderation-queue-person-{}", report.id)), report.subject_line())
            .avatar(avatar)
            .subtitle(SharedString::from(subtitle))
            .trailing_text(report.created_relative_text().map(SharedString::from))
            .trailing_age_ms(report.created_at_ms)
            .trailing(div().when(report.report_count > 1, |slot| {
                slot.child(
                    div().px(px(8.0)).py(px(4.0)).bg(theme.bg._200).capsule().child(
                        MainText::new(format!("\u{00D7}{}", RiseFormat::compact_count(report.report_count)))
                            .px(12.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(badge_color)
                            .monospaced_digits(true),
                    ),
                )
            }))
    }
}
