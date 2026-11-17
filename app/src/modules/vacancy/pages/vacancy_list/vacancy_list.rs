use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{
    App, Context, ElementId, Empty, Entity, FontWeight, KeyDownEvent, ListAlignment, ListState, SharedString, Subscription,
    WeakEntity, Window, list,
};
use rise_canon::RiseGuideAnchor;
use rise_catalog::{VacancyJobFormat, VacancySchedule, WireEnum};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::button_ui::ButtonUi;
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::filter_pill_ui::FilterPillUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;

use crate::app::router::main_tab_chrome::main_tab_bar_underlap;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellUi, list_shell_floating_top_inset};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::vacancy::components::vacancy_card::VacancyCard;
use crate::modules::vacancy::shared::vacancy_mocks::VacancyLocalState;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_actions::VacancyActionsStore;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::VacancyModel;
use crate::modules::vacancy::stores::vacancy_interactions::vacancy_interactions::VacancyInteractionsStore;
use crate::modules::vacancy::stores::vacancy_services::vacancy_services::VacancyServicesStore;

const FILTER_ALL_KEY: &str = "vacancy_filter_all";
const FILTER_SCHEDULE_KEY: &str = "vacancy_filter_schedule";
const FILTER_FORMAT_KEY: &str = "vacancy_filter_format";
const SEARCH_FOCUS_KEY: &str = "f";
const PREFETCH_THRESHOLD: usize = 8;

fn canon_option<T: WireEnum>(value: T, fallback_key: &'static str) -> (&'static str, &'static str) {
    (value.wire(), value.i18n_key().unwrap_or(fallback_key))
}

static SCHEDULE_OPTIONS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
    vec![
        ("", FILTER_ALL_KEY),
        canon_option(VacancySchedule::FullTime, FILTER_SCHEDULE_KEY),
        canon_option(VacancySchedule::Flexible, FILTER_SCHEDULE_KEY),
        canon_option(VacancySchedule::Remote, FILTER_SCHEDULE_KEY),
    ]
});

static FORMAT_OPTIONS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
    vec![
        ("", FILTER_ALL_KEY),
        (VacancyJobFormat::Office.wire(), "vacancy_format_office"),
        (VacancyJobFormat::Remote.wire(), "vacancy_format_remote"),
        (VacancyJobFormat::Hybrid.wire(), "vacancy_format_hybrid"),
    ]
});

fn label_key(options: &[(&'static str, &'static str)], value: &str, empty_key: &'static str) -> &'static str {
    if value.is_empty() {
        return empty_key;
    }
    options.iter().find(|option| option.0 == value).map_or(empty_key, |option| option.1)
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

pub(crate) fn sync_rows(list: &ListUi, old_keys: &[String], new_keys: &[String]) {
    let prefix = old_keys.iter().zip(new_keys).take_while(|(old, new)| old == new).count();
    if prefix == 0 {
        list.replaced(new_keys.len());
        return;
    }
    let suffix = old_keys[prefix..]
        .iter()
        .rev()
        .zip(new_keys[prefix..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    list.state().splice(prefix..old_keys.len() - suffix, new_keys.len() - prefix - suffix);
}

/// the three filters of the body, none when empty
#[derive(Clone)]
struct VacancyListQuery {
    q: Option<String>,
    schedule: Option<String>,
    job_format: Option<String>,
}

impl VacancyListQuery {
    fn load_more(&self, cx: &mut App) {
        let (q, schedule, job_format) = (self.q.clone(), self.schedule.clone(), self.job_format.clone());
        VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_more_vacancies(q, schedule, job_format, None, None, None, None, cx);
        });
    }
}

/// one row of the flattened scroll list, gpui lists rows not nested stacks
enum VacancyListRow {
    Filters,
    Skeleton,
    LoadError,
    Empty,
    Card { vacancy: VacancyModel, index: usize, is_viewed: bool },
    TrailingError,
    TrailingProgress,
    BottomSpacer,
}

impl VacancyListRow {
    /// row identity for the list, viewed and applied marks change a card in place
    fn key(&self) -> String {
        match self {
            Self::Filters => "filters".to_owned(),
            Self::Skeleton => "skeleton".to_owned(),
            Self::LoadError => "load-error".to_owned(),
            Self::Empty => "empty".to_owned(),
            Self::Card { vacancy, .. } => format!("card-{}", vacancy.id),
            Self::TrailingError => "trailing-error".to_owned(),
            Self::TrailingProgress => "trailing-progress".to_owned(),
            Self::BottomSpacer => "bottom".to_owned(),
        }
    }
}

pub struct VacancyList {
    shows_back_button: bool,
    search_text: String,
    selected_schedule: String,
    selected_format: String,
    search_focus_requests: u64,
    list: ListUi,
    row_keys: Vec<String>,
    pagination_observation: Option<(String, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

impl VacancyList {
    pub fn new(shows_back_button: bool, cx: &mut Context<Self>) -> Self {
        let services = VacancyServicesStore::shared(cx);
        let actions = VacancyActionsStore::shared(cx);
        let local_state = VacancyLocalState::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        Self {
            shows_back_button,
            search_text: String::new(),
            selected_schedule: String::new(),
            selected_format: String::new(),
            search_focus_requests: 0,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            row_keys: Vec::new(),
            pagination_observation: None,
            _subscriptions: vec![
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&local_state, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
                cx.observe(&cache_revisions, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn query(&self) -> VacancyListQuery {
        VacancyListQuery {
            q: non_empty(&self.search_text),
            schedule: non_empty(&self.selected_schedule),
            job_format: non_empty(&self.selected_format),
        }
    }

    /// starts a search with the current query
    fn trigger_search(&self, cx: &mut Context<Self>) {
        let query = self.query();
        VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.refresh_vacancy_list(query.q, query.schedule, query.job_format, None, None, None, None, cx);
        });
    }

    fn set_search_text(&mut self, search_text: String, cx: &mut Context<Self>) {
        if self.search_text == search_text {
            return;
        }
        self.search_text = search_text;
        self.trigger_search(cx);
        cx.notify();
    }

    fn set_selected_schedule(&mut self, selected_schedule: String, cx: &mut Context<Self>) {
        if self.selected_schedule == selected_schedule {
            return;
        }
        self.selected_schedule = selected_schedule;
        self.trigger_search(cx);
        cx.notify();
    }

    fn set_selected_format(&mut self, selected_format: String, cx: &mut Context<Self>) {
        if self.selected_format == selected_format {
            return;
        }
        self.selected_format = selected_format;
        self.trigger_search(cx);
        cx.notify();
    }

    fn observe_pagination(&mut self, cache_key: &str, cx: &mut Context<Self>) {
        if self.pagination_observation.as_ref().is_some_and(|(key, _)| key == cache_key) {
            return;
        }
        let state = SaiWs::pagination_state(cache_key, cx);
        let observation = cx.observe(&state, |_, _, cx| cx.notify());
        self.pagination_observation = Some((cache_key.to_owned(), observation));
    }

    fn sync_list(&mut self, rows: &[VacancyListRow]) {
        let keys: Vec<String> = rows.iter().map(VacancyListRow::key).collect();
        if keys == self.row_keys {
            return;
        }
        sync_rows(&self.list, &self.row_keys, &keys);
        self.row_keys = keys;
    }

    fn rows(
        list: Vec<VacancyModel>,
        viewed: impl Fn(i64) -> bool,
        is_loading: bool,
        has_error: bool,
        is_fulfilled: bool,
        has_more: bool,
    ) -> Vec<VacancyListRow> {
        let mut rows = vec![VacancyListRow::Filters];
        if is_loading {
            rows.push(VacancyListRow::Skeleton);
            rows.push(VacancyListRow::BottomSpacer);
        } else if list.is_empty() && has_error {
            rows.push(VacancyListRow::LoadError);
        } else if list.is_empty() && is_fulfilled {
            rows.push(VacancyListRow::Empty);
        } else {
            rows.extend(list.into_iter().enumerate().map(|(index, vacancy)| {
                let is_viewed = viewed(vacancy.id);
                VacancyListRow::Card { vacancy, index, is_viewed }
            }));
            if has_error {
                rows.push(VacancyListRow::TrailingError);
            } else if has_more {
                rows.push(VacancyListRow::TrailingProgress);
            }
            rows.push(VacancyListRow::BottomSpacer);
        }
        rows
    }

    fn pop(nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        if let Some(nav_store) = nav_store.as_ref() {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let binding = Binding::entity(
            cx.entity(),
            |page: &VacancyList| page.search_text.clone(),
            |page, value, cx| page.set_search_text(value, cx),
        );
        div().px(px(16.0)).pb(px(8.0)).child(
            SearchFieldUi::new("vacancy-list-search", binding)
                .placeholder(SharedString::from(tr("vacancy_search_placeholder")))
                .uses_inline_text_field(true)
                .focus_requests(self.search_focus_requests),
        )
    }
}

impl Render for VacancyList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let query = self.query();
        let (q, schedule, job_format) = (query.q.as_deref(), query.schedule.as_deref(), query.job_format.as_deref());
        let list = VacancyServicesStore::vacancy_list(q, schedule, job_format, None, None, None, None, cx);
        let has_more = VacancyServicesStore::vacancy_list_has_more(q, schedule, job_format, None, None, None, None, cx);
        let relative_id =
            VacancyServicesStore::vacancy_list_relative_id(q, schedule, job_format, None, None, None, None, cx);
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let cache_key = VacancyServicesStore::vacancy_list_cache_key(
            q,
            schedule,
            job_format,
            None,
            None,
            None,
            None,
            user_id.as_deref(),
        );
        self.observe_pagination(&cache_key, cx);
        let pagination = SaiWs::pagination_state(&cache_key, cx);
        let (is_initial_page_loading, is_trailing_loading, has_trailing_error) = {
            let pagination = pagination.read(cx);
            (
                pagination.is_initial_page_loading(),
                pagination.trailing().is_loading,
                pagination.trailing().error.is_some(),
            )
        };
        let (is_slice_pending, slice_status) = {
            let actions = VacancyActionsStore::shared(cx);
            let slice = actions.read(cx).list_vacancies_sai.clone();
            (slice.is_pending(), slice.status())
        };
        let is_fulfilled = slice_status == SaiWsSliceStatus::Fulfilled;
        let is_pending = is_slice_pending || is_initial_page_loading || is_trailing_loading;
        let has_error = !is_pending
            && (slice_status == SaiWsSliceStatus::Rejected
                || has_trailing_error
                || (is_fulfilled && has_more && relative_id.is_none()));
        let is_loading = list.is_empty() && !has_error && (is_pending || has_more);

        let item_count = list.len();
        let local_state = VacancyLocalState::shared(cx);
        let rows = {
            let viewed_ids = local_state.read(cx).viewed_ids();
            Self::rows(list, |id| viewed_ids.contains(&id), is_loading, has_error, is_fulfilled, has_more)
        };
        self.sync_list(&rows);

        let page = cx.weak_entity();
        let refresh_page = page.clone();
        let focus_page = page.clone();
        let body_context = VacancyListRowContext {
            page,
            selected_schedule: self.selected_schedule.clone(),
            selected_format: self.selected_format.clone(),
            query,
            cache_key: SharedString::from(cache_key),
            relative_id: relative_id.map(|id| id.to_string()),
            has_more,
            item_count,
            nav_store: nav_store.clone(),
        };

        div()
            .size_full()
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if event.keystroke.key != SEARCH_FOCUS_KEY || !event.keystroke.modifiers.secondary() {
                    return;
                }
                cx.stop_propagation();
                let _ = focus_page.update(cx, |page, cx| {
                    page.search_focus_requests += 1;
                    cx.notify();
                });
            })
            .child(PageRefreshKeys::new(
                "vacancy-list-keys",
                move |_, cx| {
                    let _ = refresh_page.update(cx, |page, cx| page.trigger_search(cx));
                },
                ListShellUi::new()
                    .title(SharedString::from(tr("navbtn_vacancies")))
                    .shows_back_button(self.shows_back_button)
                    .on_back(move |_, cx| Self::pop(&nav_store, cx))
                    .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_blurred_header()))
                    .below_header(self.search_bar(cx))
                    .content(VacancyListBody {
                        list_state: self.list.state().clone(),
                        rows: Rc::new(rows),
                        context: Rc::new(body_context),
                    }),
            ))
            .on_appear("vacancy-list-page", |_, cx| {
                VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.ensure_vacancy_list_loaded(cx);
                });
            })
    }
}

struct VacancyListRowContext {
    page: WeakEntity<VacancyList>,
    selected_schedule: String,
    selected_format: String,
    query: VacancyListQuery,
    cache_key: SharedString,
    relative_id: Option<String>,
    has_more: bool,
    item_count: usize,
    nav_store: Option<Entity<NavigationStore>>,
}

#[derive(IntoElement)]
struct VacancyListBody {
    list_state: ListState,
    rows: Rc<Vec<VacancyListRow>>,
    context: Rc<VacancyListRowContext>,
}

impl RenderOnce for VacancyListBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let top_inset = list_shell_floating_top_inset(cx);
        let bottom_inset = (main_tab_bar_underlap(cx) + 12.0).max(24.0);
        let rows = self.rows;
        let context = self.context;
        let row_theme = theme.clone();
        div().size_full().bg(theme.bg._100).child(
            list(self.list_state, move |index, _, _| {
                let Some(row) = rows.get(index) else {
                    return Empty.into_any_element();
                };
                vacancy_list_row(row, &context, top_inset, bottom_inset, &row_theme)
            })
            .size_full(),
        )
    }
}

fn vacancy_list_row(
    row: &VacancyListRow,
    context: &Rc<VacancyListRowContext>,
    top_inset: f32,
    bottom_inset: f32,
    theme: &AppTheme,
) -> AnyElement {
    match row {
        VacancyListRow::Filters => {
            let query = context.query.clone();
            div()
                .w_full()
                .px(px(16.0))
                .pt(px(top_inset + 8.0))
                .child(filter_pills(context))
                .sai_pagination_drain_filtered_empty(
                    "vacancy-list-drain",
                    SaiPaginationPrefetchOptions::drain_filtered_empty(
                        context.item_count,
                        context.cache_key.clone(),
                        context.relative_id.clone(),
                        context.has_more,
                    ),
                    move |_, cx| query.load_more(cx),
                )
                .into_any_element()
        }
        VacancyListRow::Skeleton => {
            div().w_full().px(px(16.0)).pt(px(12.0)).child(VacancyListSkeleton::new()).into_any_element()
        }
        VacancyListRow::LoadError => {
            div().w_full().px(px(16.0)).pt(px(40.0)).child(load_error_state(context, theme)).into_any_element()
        }
        VacancyListRow::Empty => empty_state(theme).into_any_element(),
        VacancyListRow::Card { vacancy, index, is_viewed } => {
            let vacancy_id = vacancy.id;
            let nav_store = context.nav_store.clone();
            let query = context.query.clone();
            div()
                .w_full()
                .px(px(16.0))
                .pt(px(12.0))
                .child(
                    VacancyCard::new(ElementId::Name(format!("vacancy-card-{vacancy_id}").into()), vacancy.clone())
                        .is_viewed(*is_viewed)
                        .is_applied(vacancy.is_applied.unwrap_or(false))
                        .on_tap(move |_, cx| {
                            if let Some(nav_store) = nav_store.as_ref() {
                                nav_store.update(cx, |nav_store, cx| {
                                    nav_store.push(
                                        RootRoute::VacancyDetail { vacancy_id },
                                        NavigationStoreSettlingPush::Drop,
                                        cx,
                                    );
                                });
                            }
                        })
                        .guide_anchor((*index == 0).then_some(RiseGuideAnchor::VacancyListFirst))
                        .sai_pagination_prefetch(
                            ElementId::Name(format!("vacancy-card-prefetch-{vacancy_id}").into()),
                            SaiPaginationPrefetchOptions::new(
                                *index,
                                context.item_count,
                                context.cache_key.clone(),
                                context.relative_id.clone(),
                                context.has_more,
                            )
                            .threshold(PREFETCH_THRESHOLD),
                            move |_, cx| query.load_more(cx),
                        ),
                )
                .into_any_element()
        }
        VacancyListRow::TrailingError => {
            div().w_full().px(px(16.0)).pt(px(12.0)).child(load_error_state(context, theme)).into_any_element()
        }
        VacancyListRow::TrailingProgress => div()
            .w_full()
            .pt(px(28.0))
            .pb(px(16.0))
            .flex()
            .justify_center()
            .child(progress_view())
            .into_any_element(),
        VacancyListRow::BottomSpacer => div().w_full().h(px(bottom_inset)).into_any_element(),
    }
}

fn load_error_state(context: &Rc<VacancyListRowContext>, theme: &AppTheme) -> impl IntoElement {
    let page = context.page.clone();
    vstack(16.0)
        .w_full()
        .py(px(16.0))
        .child(SecondaryText::new(tr("error_fetching_data")).px(14.0).tac(TextAlignT::Center))
        .child(
            ButtonUi::new("vacancy-list-retry")
                .on_press(move |_, cx| {
                    let _ = page.update(cx, |page, cx| page.trigger_search(cx));
                })
                .child(MainText::new(tr("retry")).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary)),
        )
}

fn filter_pills(context: &Rc<VacancyListRowContext>) -> impl IntoElement {
    let schedule_items = SCHEDULE_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, (value, key))| {
            let page = context.page.clone();
            let value = *value;
            ContextMenuItem::new(index as i64, tr(key), None, value, move |_, cx| {
                let _ = page.update(cx, |page, cx| page.set_selected_schedule(value.to_owned(), cx));
            })
        })
        .collect();
    let format_items = FORMAT_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, (value, key))| {
            let page = context.page.clone();
            let value = *value;
            ContextMenuItem::new(index as i64, tr(key), None, value, move |_, cx| {
                let _ = page.update(cx, |page, cx| page.set_selected_format(value.to_owned(), cx));
            })
        })
        .collect();
    hstack(8.0)
        .w_full()
        .child(FilterPillUi::new(
            "vacancy-list-schedule-filter",
            schedule_items,
            context.selected_schedule.clone(),
            tr(label_key(&SCHEDULE_OPTIONS, &context.selected_schedule, FILTER_SCHEDULE_KEY)),
        ))
        .child(FilterPillUi::new(
            "vacancy-list-format-filter",
            format_items,
            context.selected_format.clone(),
            tr(label_key(&FORMAT_OPTIONS, &context.selected_format, FILTER_FORMAT_KEY)),
        ))
        .child(spacer_min_w(0.0))
}

fn empty_state(theme: &AppTheme) -> impl IntoElement {
    vstack(16.0)
        .w_full()
        .pt(px(80.0))
        .px(px(32.0))
        .children(IconUi::sized("briefcase", px(48.0), theme.text.secondary.opacity(0.5), false))
        .child(
            MainText::new(tr("vacancy_empty_title"))
                .px(17.0)
                .tac(TextAlignT::Center)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.text.primary),
        )
        .child(SecondaryText::new(tr("vacancy_empty_subtitle")).px(14.0).tac(TextAlignT::Center))
}

#[derive(IntoElement, Default)]
pub struct VacancyCardSkeleton {
    alternation_index: usize,
}

impl VacancyCardSkeleton {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alternation_index(mut self, alternation_index: usize) -> Self {
        self.alternation_index = alternation_index;
        self
    }

    fn alt(&self, values: [f32; 4]) -> f32 {
        values[self.alternation_index % values.len()]
    }
}

impl RenderOnce for VacancyCardSkeleton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        vstack(12.0)
            .w_full()
            .items_start()
            .p(px(16.0))
            .bg(theme.bg._200)
            .rounded(theme.radius._300)
            .stroke_border(theme.border._100, 1.0)
            .child(
                hstack(12.0)
                    .w_full()
                    .child(SkeletonBlock::new(48.0_f32, 48.0_f32, 12.0))
                    .child(
                        vstack(7.0)
                            .items_start()
                            .child(SkeletonLine::new(self.alt([110.0, 90.0, 130.0, 100.0]), 8.0))
                            .child(SkeletonLine::new(self.alt([70.0, 90.0, 60.0, 80.0]), 7.0)),
                    )
                    .child(spacer_min_w(0.0))
                    .child(SkeletonLine::new(40.0_f32, 6.0)),
            )
            .child(SkeletonLine::new(self.alt([210.0, 240.0, 180.0, 225.0]), 10.0))
            .child(SkeletonLine::new(self.alt([110.0, 90.0, 125.0, 100.0]), 9.0))
            .child(
                hstack(6.0)
                    .child(SkeletonLine::new(64.0_f32, 20.0))
                    .child(SkeletonLine::new(56.0_f32, 20.0))
                    .child(SkeletonLine::new(74.0_f32, 20.0)),
            )
    }
}

#[derive(IntoElement)]
pub struct VacancyListSkeleton {
    cards: usize,
}

impl Default for VacancyListSkeleton {
    fn default() -> Self {
        Self { cards: 6 }
    }
}

impl VacancyListSkeleton {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cards(mut self, cards: usize) -> Self {
        self.cards = cards;
        self
    }
}

impl RenderOnce for VacancyListSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        vstack(12.0)
            .w_full()
            .children((0..self.cards).map(|index| VacancyCardSkeleton::new().alternation_index(index)))
            .skeleton_shimmer(None)
    }
}
