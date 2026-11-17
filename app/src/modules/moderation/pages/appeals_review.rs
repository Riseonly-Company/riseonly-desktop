use std::rc::Rc;

use gpui::{App, Context, Entity, ListAlignment, SharedString, StyleRefinement, Subscription, Task, Window};
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::AnimatedTabsPagerLiveState;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, ListShellUi, list_shell_floating_top_inset,
};
use crate::modules::moderation::components::moderation_kit_ui::{
    ModerationEmptyState, ModerationFailureState, ModerationFilterPager, ModerationListSkeleton, ModerationMetrics,
    ModerationPersonRow, ModerationRowSeparator, ModerationSegmentedFilter, ModerationTint, moderation_symbol,
};
use crate::modules::moderation::engine::rise_moderation_presentation::{
    ModerationLoadState, RiseModerationPresentation,
};
use crate::modules::moderation::engine::rise_moderation_wire::AppealItemDTO;
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AppealReviewFilter, AppealStatus, AppealStatusExt, ReportReasonExt, ReportTargetTypeExt, UserBlockType,
    UserBlockTypeExt,
};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;

const SKELETON_ROWS: usize = 8;
const LOAD_MORE_THRESHOLD: usize = 4;
const ROW_HEIGHT: f32 = ModerationMetrics::AVATAR_SIZE + 20.0;

fn interactions(cx: &mut App) -> Entity<ModerationInteractionsStore> {
    ModerationInteractionsStore::shared(cx)
}

fn filters() -> [AppealReviewFilter; 3] {
    AppealReviewFilter::ALL
}

struct AppealsReviewState {
    pager_live_state: Entity<AnimatedTabsPagerLiveState>,
    list: ListUi,
    list_ids: Vec<i64>,
    review: Rc<Vec<AppealItemDTO>>,
    _subscriptions: Vec<Subscription>,
}

impl AppealsReviewState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseModerationPresentation::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            pager_live_state: cx.new(|_| AnimatedTabsPagerLiveState::new()),
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            review: Rc::new(presentation.read(cx).review().to_vec()),
            _subscriptions: vec![
                cx.observe(&presentation, |this, presentation, cx| {
                    let review = presentation.read(cx).review();
                    if this.review.as_slice() != review {
                        this.review = Rc::new(review.to_vec());
                    }
                    cx.notify();
                }),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn sync_review(&mut self) -> Rc<Vec<AppealItemDTO>> {
        let review = self.review.clone();
        if !self.list_ids.iter().copied().eq(review.iter().map(AppealItemDTO::id)) {
            self.sync_list(review.iter().map(AppealItemDTO::id).collect());
        }
        review
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
pub struct AppealsReview;

impl AppealsReview {
    pub fn new() -> Self {
        Self
    }

    fn filter_index(cx: &mut App) -> i64 {
        let filter = RiseModerationPresentation::shared(cx).read(cx).review_filter();
        filters().iter().position(|candidate| *candidate == filter).unwrap_or(0) as i64
    }

    fn subtitle(cx: &mut App) -> Option<SharedString> {
        let presentation = RiseModerationPresentation::shared(cx);
        let presentation = presentation.read(cx);
        if *presentation.review_state() != ModerationLoadState::Loaded {
            return None;
        }
        let count = RiseFormat::compact_count(presentation.review_total());
        Some(tr("moderation_queue_count").replace("{n}", &count).into())
    }

    fn select_filter(index: i64, cx: &mut App) {
        let Some(filter) = usize::try_from(index).ok().and_then(|index| filters().get(index).copied()) else {
            return;
        };
        interactions(cx).update(cx, |store, cx| store.set_review_filter(filter, cx));
    }

    fn refresh(cx: &mut App) {
        interactions(cx).update(cx, |store, cx| store.refresh_review(cx));
    }
}

impl RenderOnce for AppealsReview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("appeals-review-state", cx, |_, cx| AppealsReviewState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let live_state = state.read(cx).pager_live_state.clone();
        let filter_index = Self::filter_index(cx);
        let subtitle = Self::subtitle(cx);

        let filter = ModerationSegmentedFilter::new(
            "appeals-review-filter",
            filters().iter().map(|filter| SharedString::from(tr(filter.title_key()))).collect(),
            filter_index,
            |index, _, cx| Self::select_filter(index, cx),
        )
        .live_drag_state(Some(live_state.clone()));

        let pager = ModerationFilterPager::new(
            "appeals-review-pager",
            filters().len() as i64,
            filter_index,
            live_state,
            |index, _, cx| Self::select_filter(index, cx),
            AppealsReviewBody { state, nav_store: nav_store.clone() },
        )
        .skeleton_rows(SKELETON_ROWS);

        div()
            .size_full()
            .child(PageRefreshKeys::new(
                "appeals-review-keys",
                |_, cx| Self::refresh(cx),
                ListShellUi::new()
                    .title(SharedString::from(tr("appeals_review_title")))
                    .subtitle(subtitle)
                    .shows_back_button(true)
                    .on_back(move |_, cx| {
                        if let Some(nav_store) = &nav_store {
                            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                        }
                    })
                    .below_header(filter)
                    .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
                    .content(div().size_full().bg(theme.bg._100).child(pager)),
            ))
            .on_appear("appeals-review-task", |_, cx| {
                interactions(cx).update(cx, |store, cx| {
                    store.ensure_moderator_profile_loaded(cx);
                    store.ensure_review_loaded(cx);
                });
            })
    }
}

#[derive(IntoElement)]
struct AppealsReviewBody {
    state: Entity<AppealsReviewState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl AppealsReviewBody {
    fn list(self, cx: &mut App) -> AnyElement {
        let top_inset = list_shell_floating_top_inset(cx);
        let is_loading_more = RiseModerationPresentation::shared(cx).read(cx).is_loading_more_review();
        let review = self.state.update(cx, |state, _| state.sync_review());
        let count = review.len();
        let nav_store = self.nav_store;
        self.state
            .read(cx)
            .list
            .refreshable(
                "appeals-review-list",
                |_, cx| {
                    AppealsReview::refresh(cx);
                    Task::ready(())
                },
                move |index, window, cx| {
                    let Some(item) = review.get(index) else {
                        return vstack(0.0)
                            .w_full()
                            .when(is_loading_more, |column| column.child(div().py(px(12.0)).child(progress_view())))
                            .child(div().w_full().h(px(ModerationMetrics::BOTTOM_INSET)))
                            .into_any_element();
                    };
                    let appeal_id = item.id();
                    let row_view = window.use_keyed_state(
                        SharedString::from(format!("appeals-review-row-view-{appeal_id}")),
                        cx,
                        |_, cx| AppealReviewRowView::new(item.clone(), cx),
                    );
                    row_view.update(cx, |view, cx| view.set_item(item, cx));
                    let row_store = nav_store.clone();
                    vstack(0.0)
                        .w_full()
                        .when(index == 0, |column| column.pt(px(top_inset)))
                        .child(
                            plain(SharedString::from(format!("appeals-review-row-{appeal_id}")))
                                .w_full()
                                .on_press(move |_, cx| {
                                    let Some(nav_store) = &row_store else {
                                        return;
                                    };
                                    let route = RootRoute::AppealReview { appeal_id: appeal_id.to_string() };
                                    nav_store.update(cx, |nav_store, cx| {
                                        nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                                    });
                                })
                                .child(
                                    row_view
                                        .cached(StyleRefinement::default().w_full().h(px(ROW_HEIGHT)))
                                        .replay_when_moved(),
                                ),
                        )
                        .when(index + 1 < count, |column| column.child(ModerationRowSeparator::new()))
                        .on_appear(SharedString::from(format!("appeals-review-appear-{appeal_id}")), move |_, cx| {
                            if index + LOAD_MORE_THRESHOLD >= count {
                                interactions(cx).update(cx, |store, cx| store.load_more_review(cx));
                            }
                        })
                        .into_any_element()
                },
            )
            .into_any_element()
    }
}

impl RenderOnce for AppealsReviewBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let (review_state, is_empty) = {
            let presentation = RiseModerationPresentation::shared(cx);
            let presentation = presentation.read(cx);
            (presentation.review_state().clone(), presentation.review().is_empty())
        };
        match review_state {
            ModerationLoadState::Idle | ModerationLoadState::Loading => scroll_view("appeals-review-skeleton")
                .scroll_disabled(true)
                .shows_indicators(false)
                .list_shell_scroll_under_chrome(cx)
                .child(ModerationListSkeleton::new().rows(SKELETON_ROWS))
                .into_any_element(),
            ModerationLoadState::Failed(message) => div()
                .size_full()
                .list_shell_scroll_under_chrome(cx)
                .child(ModerationFailureState::new("appeals-review-failure", message, theme, |_, cx| {
                    AppealsReview::refresh(cx);
                }))
                .into_any_element(),
            ModerationLoadState::Loaded if is_empty => scroll_view("appeals-review-empty")
                .refreshable(|_, cx| {
                    AppealsReview::refresh(cx);
                    Task::ready(())
                })
                .list_shell_scroll_under_chrome(cx)
                .child(ModerationEmptyState::new("checkmark.circle", tr("appeals_review_empty"), theme))
                .into_any_element(),
            ModerationLoadState::Loaded => self.list(cx),
        }
    }
}

struct AppealReviewRowView {
    item: AppealItemDTO,
    _theme: Subscription,
}

impl AppealReviewRowView {
    fn new(item: AppealItemDTO, cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        Self { item, _theme: cx.observe(&theme_store, |_, _, cx| cx.notify()) }
    }

    fn set_item(&mut self, item: &AppealItemDTO, cx: &mut Context<Self>) {
        if self.item != *item {
            self.item = item.clone();
            cx.notify();
        }
    }
}

impl Render for AppealReviewRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        AppealReviewRow::new(self.item.clone())
    }
}

#[derive(IntoElement)]
pub struct AppealReviewRow {
    item: AppealItemDTO,
}

impl AppealReviewRow {
    pub fn new(item: AppealItemDTO) -> Self {
        Self { item }
    }

    fn sanction_line(&self) -> Option<String> {
        let item = &self.item;
        let measure = item
            .resolved_block_type()
            .or_else(|| item.appeal.block_type.as_deref().map(|raw| UserBlockType::parse_or_default(Some(raw))));
        let cause = item
            .resolved_report_reason()
            .map(|reason| tr(reason.title_key()))
            .or_else(|| item.resolved_report_type().map(|r#type| tr(r#type.title_key())));
        let parts: Vec<String> = [measure.map(|measure| tr(measure.title_key())), cause].into_iter().flatten().collect();
        if parts.is_empty() { None } else { Some(parts.join(" \u{00B7} ")) }
    }

    fn trailing(status: AppealStatus, theme: &AppTheme) -> Div {
        div().when(status != AppealStatus::Pending, |slot| {
            slot.children(moderation_symbol(status.icon_name(), 15.0, ModerationTint::appeal(status, theme)))
        })
    }
}

impl RenderOnce for AppealReviewRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let status = self.item.resolved_status();
        let sanction_line = self.sanction_line();
        let item = self.item;
        let appeal = &item.appeal;
        let appellant = item.appellant.as_ref();
        let subtitle = sanction_line.unwrap_or_else(|| {
            if appeal.text.is_empty() { tr("appeal_review_title") } else { appeal.text.clone() }
        });
        ModerationPersonRow::new(
            SharedString::from(format!("appeals-review-person-{}", appeal.id)),
            appellant.map_or_else(|| tr("moderation_unknown_reporter"), |appellant| appellant.display_name()),
        )
        .logo_source(appellant.and_then(|appellant| appellant.logo.clone()).map(SharedString::from))
        .user_id(SharedString::from(appeal.appellant_id.clone()))
        .subtitle(SharedString::from(subtitle))
        .is_official(appellant.is_some_and(|appellant| appellant.is_official))
        .is_premium(appellant.is_some_and(|appellant| appellant.is_premium))
        .trailing_text(appeal.created_date_text().map(SharedString::from))
        .trailing(Self::trailing(status, &theme))
    }
}
