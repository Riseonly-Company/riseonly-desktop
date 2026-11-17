use std::rc::Rc;

use gpui::{
    App, Context, ElementId, Empty, Entity, FontWeight, Hsla, ListAlignment, ListState, SharedString, Subscription, WeakEntity,
    Window, list, rgb,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellUi, list_shell_floating_top_inset};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::post::components::moderation_hidden_banner::ModerationHiddenBanner;
use crate::modules::vacancy::pages::vacancy_list::vacancy_list::{VacancyListSkeleton, sync_rows};
use crate::modules::vacancy::stores::vacancy_actions::vacancy_actions::VacancyActionsStore;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::{ResumeModel, VacancyModel};
use crate::modules::vacancy::stores::vacancy_interactions::vacancy_interactions::VacancyInteractionsStore;
use crate::modules::vacancy::stores::vacancy_services::vacancy_services::VacancyServicesStore;

const TAB_VACANCIES: usize = 0;
const TAB_RESUMES: usize = 1;
const TAB_ANIMATION_SECONDS: f64 = 0.2;
const PREFETCH_THRESHOLD: usize = 8;
const ACTIVE_GREEN: u32 = 0x00C781;

fn push(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
    if let Some(nav_store) = nav_store.as_ref() {
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

fn named(prefix: &str, id: i64) -> ElementId {
    ElementId::Name(format!("{prefix}-{id}").into())
}

/// one row of the flattened lazy list
enum MyVacanciesRow {
    Skeleton,
    Empty { icon: &'static str, title_key: &'static str, subtitle_key: &'static str },
    Vacancy { vacancy: VacancyModel, index: usize },
    Resume { resume: ResumeModel, index: usize },
    Progress,
    BottomSpacer,
}

impl MyVacanciesRow {
    fn key(&self) -> String {
        match self {
            Self::Skeleton => "skeleton".to_owned(),
            Self::Empty { title_key, .. } => format!("empty-{title_key}"),
            Self::Vacancy { vacancy, .. } => format!("vacancy-{}", vacancy.id),
            Self::Resume { resume, .. } => format!("resume-{}", resume.id),
            Self::Progress => "progress".to_owned(),
            Self::BottomSpacer => "bottom".to_owned(),
        }
    }
}

pub struct MyVacancies {
    selected_tab: usize,
    deletion_request_id: Option<Uuid>,
    list: ListUi,
    row_keys: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl MyVacancies {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let services = VacancyServicesStore::shared(cx);
        let actions = VacancyActionsStore::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        Self {
            selected_tab: TAB_VACANCIES,
            deletion_request_id: None,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            row_keys: Vec::new(),
            _subscriptions: vec![
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
                cx.observe(&cache_revisions, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn sync_list(&mut self, rows: &[MyVacanciesRow]) {
        let keys: Vec<String> = rows.iter().map(MyVacanciesRow::key).collect();
        if keys == self.row_keys {
            return;
        }
        sync_rows(&self.list, &self.row_keys, &keys);
        self.row_keys = keys;
    }

    fn vacancies_rows(cx: &mut App) -> (Vec<MyVacanciesRow>, bool) {
        let my_vacancies = VacancyServicesStore::my_vacancies(None, cx);
        let has_more = VacancyServicesStore::my_vacancies_has_more(None, cx);
        let is_pending = VacancyActionsStore::shared(cx).read(cx).my_vacancies_sai.is_pending();
        let mut rows = Vec::new();
        if is_pending && my_vacancies.is_empty() {
            rows.push(MyVacanciesRow::Skeleton);
        } else if my_vacancies.is_empty() {
            rows.push(MyVacanciesRow::Empty {
                icon: "briefcase",
                title_key: "vacancy_my_empty_title",
                subtitle_key: "vacancy_my_empty_subtitle",
            });
        } else {
            rows.extend(
                my_vacancies
                    .into_iter()
                    .enumerate()
                    .map(|(index, vacancy)| MyVacanciesRow::Vacancy { vacancy, index }),
            );
            if has_more {
                rows.push(MyVacanciesRow::Progress);
            }
        }
        (rows, has_more)
    }

    fn resumes_rows(cx: &mut App) -> (Vec<MyVacanciesRow>, bool) {
        let my_resumes = VacancyServicesStore::my_resumes(cx);
        let has_more = VacancyServicesStore::my_resumes_has_more(cx);
        let is_pending = VacancyActionsStore::shared(cx).read(cx).my_resumes_sai.is_pending();
        let mut rows = Vec::new();
        if is_pending && my_resumes.is_empty() {
            rows.push(MyVacanciesRow::Skeleton);
        } else if my_resumes.is_empty() {
            rows.push(MyVacanciesRow::Empty {
                icon: "doc.text",
                title_key: "vacancy_resume_empty_title",
                subtitle_key: "vacancy_resume_empty_subtitle",
            });
        } else {
            rows.extend(
                my_resumes.into_iter().enumerate().map(|(index, resume)| MyVacanciesRow::Resume { resume, index }),
            );
            if has_more {
                rows.push(MyVacanciesRow::Progress);
            }
        }
        (rows, has_more)
    }

    fn request_delete_vacancy(page: &WeakEntity<Self>, vacancy: &VacancyModel, cx: &mut App) {
        let request_id = Uuid::new_v4();
        let _ = page.update(cx, |page, cx| {
            page.deletion_request_id = Some(request_id);
            cx.notify();
        });
        let proceed_page = page.clone();
        let can_proceed: Rc<dyn Fn(&mut App) -> bool> = Rc::new(move |cx: &mut App| {
            proceed_page.read_with(cx, |page, _| page.deletion_request_id == Some(request_id)).unwrap_or(false)
        });
        let finish_page = page.clone();
        let finish: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            let _ = finish_page.update(cx, |page, cx| {
                if page.deletion_request_id == Some(request_id) {
                    page.deletion_request_id = None;
                    cx.notify();
                }
            });
        });
        let (id, title) = (vacancy.id, vacancy.title.clone());
        VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.delete_vacancy(id, title, can_proceed, finish.clone(), Some(finish), cx);
        });
    }

    fn tab_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_tab = self.selected_tab;
        let page = cx.weak_entity();
        let tab = move |index: usize, title_key: &str| MyVacanciesTabItem {
            id: named("my-vacancies-tab", index as i64),
            title: tr(title_key),
            is_selected: selected_tab == index,
            index,
            page: page.clone(),
        };
        hstack(0.0)
            .w_full()
            .padding_edges(EdgeSet::HORIZONTAL, 16.0)
            .padding_edges(EdgeSet::VERTICAL, 8.0)
            .child(tab(TAB_VACANCIES, "vacancy_my_tab_vacancies"))
            .child(tab(TAB_RESUMES, "vacancy_my_tab_resumes"))
    }
}

impl Render for MyVacancies {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let selected_tab = self.selected_tab;
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        let (mut rows, has_more, cache_key, relative_id) = if selected_tab == TAB_VACANCIES {
            let (rows, has_more) = Self::vacancies_rows(cx);
            let relative_id = VacancyServicesStore::my_vacancies_relative_id(None, cx);
            (rows, has_more, VacancyServicesStore::my_vacancies_cache_key(&user_id, None), relative_id)
        } else {
            let (rows, has_more) = Self::resumes_rows(cx);
            let relative_id = VacancyServicesStore::my_resumes_relative_id(cx);
            (rows, has_more, VacancyServicesStore::my_resumes_cache_key(&user_id), relative_id)
        };
        let item_count = rows
            .iter()
            .filter(|row| matches!(row, MyVacanciesRow::Vacancy { .. } | MyVacanciesRow::Resume { .. }))
            .count();
        rows.push(MyVacanciesRow::BottomSpacer);
        self.sync_list(&rows);

        let practice_scene_is_active = GuideSandbox::shared(cx).read(cx).owns_jobs_scene();
        let is_delete_pending = VacancyActionsStore::shared(cx).read(cx).delete_vacancy_sai.is_pending();
        let context = MyVacanciesRowContext {
            page: cx.weak_entity(),
            nav_store: nav_store.clone(),
            practice_scene_is_active,
            is_delete_pending,
            cache_key: SharedString::from(cache_key),
            relative_id: relative_id.map(|id| id.to_string()),
            has_more,
            item_count,
        };

        let create_nav_store = nav_store.clone();
        let back_nav_store = nav_store;
        let leaving_page = cx.weak_entity();
        ListShellUi::new()
            .title(SharedString::from(tr("vacancy_my_title")))
            .on_back(move |_, cx| {
                if let Some(nav_store) = back_nav_store.as_ref() {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .right_jsx(
                PageHeaderIconButton::new("my-vacancies-create", "plus", move |_, cx| {
                    let route = if selected_tab == TAB_VACANCIES { RootRoute::CreateVacancy } else { RootRoute::CreateResume };
                    push(&create_nav_store, route, cx);
                })
                .guide_anchor(RiseGuideAnchor::MyVacanciesCreate),
            )
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
            .below_header(self.tab_selector(cx))
            .content(MyVacanciesBody {
                list_state: self.list.state().clone(),
                rows: Rc::new(rows),
                context: Rc::new(context),
            })
            .on_appear("my-vacancies-page", |_, cx| {
                VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.ensure_my_vacancies_loaded(cx);
                    interactions.ensure_my_resumes_loaded(cx);
                });
            })
            .and_on_disappear(move |_, cx| {
                let _ = leaving_page.update(cx, |page, cx| {
                    page.deletion_request_id = None;
                    cx.notify();
                });
            })
    }
}

struct MyVacanciesRowContext {
    page: WeakEntity<MyVacancies>,
    nav_store: Option<Entity<NavigationStore>>,
    practice_scene_is_active: bool,
    is_delete_pending: bool,
    cache_key: SharedString,
    relative_id: Option<String>,
    has_more: bool,
    item_count: usize,
}

impl MyVacanciesRowContext {
    fn prefetch(&self, index: usize) -> SaiPaginationPrefetchOptions {
        SaiPaginationPrefetchOptions::new(
            index,
            self.item_count,
            self.cache_key.clone(),
            self.relative_id.clone(),
            self.has_more,
        )
        .threshold(PREFETCH_THRESHOLD)
    }
}

#[derive(IntoElement)]
struct MyVacanciesTabItem {
    id: ElementId,
    title: String,
    is_selected: bool,
    index: usize,
    page: WeakEntity<MyVacancies>,
}

impl RenderOnce for MyVacanciesTabItem {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let target = if self.is_selected { 1_f32 } else { 0_f32 };
        let selection = window.use_keyed_state(self.id.clone(), cx, |_, _| Animated::new(target));
        let now = RiseClock::monotonic();
        let progress = selection.update(cx, |animated, cx| {
            animated.set_in(target, ease_in_out(TAB_ANIMATION_SECONDS), now, cx);
            animated.frame_value(now, window)
        });
        let index = self.index;
        let page = self.page;
        plain(self.id)
            .flex_1()
            .on_press(move |_, cx| {
                let _ = page.update(cx, |page, cx| {
                    page.selected_tab = index;
                    cx.notify();
                });
            })
            .child(
                vstack(6.0)
                    .w_full()
                    .child(
                        MainText::new(self.title)
                            .px(14.0)
                            .tac(TextAlignT::Center)
                            .font_weight(if self.is_selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                            .color(theme.text.secondary.blend(theme.primary._100.opacity(progress))),
                    )
                    .child(div().w_full().h(px(2.0)).capsule().bg(theme.primary._100.opacity(progress))),
            )
    }
}

#[derive(IntoElement)]
struct MyVacanciesBody {
    list_state: ListState,
    rows: Rc<Vec<MyVacanciesRow>>,
    context: Rc<MyVacanciesRowContext>,
}

impl RenderOnce for MyVacanciesBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let top_inset = list_shell_floating_top_inset(cx);
        let rows = self.rows;
        let context = self.context;
        let row_theme = theme.clone();
        div().size_full().bg(theme.bg._100).child(
            list(self.list_state, move |index, _, _| {
                let Some(row) = rows.get(index) else {
                    return Empty.into_any_element();
                };
                let top = if index == 0 { top_inset + 12.0 } else { 12.0 };
                my_vacancies_row(row, &context, top, &row_theme)
            })
            .size_full(),
        )
    }
}

fn my_vacancies_row(row: &MyVacanciesRow, context: &Rc<MyVacanciesRowContext>, top: f32, theme: &AppTheme) -> AnyElement {
    let cell = div().w_full().px(px(16.0)).pt(px(top));
    match row {
        MyVacanciesRow::Skeleton => cell.child(VacancyListSkeleton::new().cards(3)).into_any_element(),
        MyVacanciesRow::Empty { icon, title_key, subtitle_key } => {
            cell.child(empty_state(icon, title_key, subtitle_key, theme)).into_any_element()
        }
        MyVacanciesRow::Vacancy { vacancy, index } => {
            let is_guide_row = *index == 0
                && (!context.practice_scene_is_active || GuideSandbox::is_practice_job_id(vacancy.id));
            cell.child(
                my_vacancy_row(vacancy, context, theme)
                    .guide_anchor(is_guide_row.then_some(RiseGuideAnchor::MyVacanciesFirst))
                    .sai_pagination_prefetch(named("my-vacancy-prefetch", vacancy.id), context.prefetch(*index), |_, cx| {
                        VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            interactions.load_more_my_vacancies(cx);
                        });
                    }),
            )
            .into_any_element()
        }
        MyVacanciesRow::Resume { resume, index } => cell
            .child(resume_row(resume, context, theme).sai_pagination_prefetch(
                named("my-resume-prefetch", resume.id),
                context.prefetch(*index),
                |_, cx| {
                    VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.load_more_my_resumes(cx);
                    });
                },
            ))
            .into_any_element(),
        MyVacanciesRow::Progress => {
            div().w_full().flex().justify_center().pt(px(top + 8.0)).pb(px(8.0)).child(progress_view()).into_any_element()
        }
        MyVacanciesRow::BottomSpacer => div().w_full().h(px(40.0)).into_any_element(),
    }
}

fn card_surface(theme: &AppTheme) -> Div {
    vstack(10.0)
        .w_full()
        .items_start()
        .p(px(14.0))
        .bg(theme.bg._200)
        .rounded(theme.radius._300)
        .stroke_border(theme.border._100, 1.0)
}

fn city_and_date(city: &str, date: String) -> Div {
    let separated = !city.is_empty();
    hstack(6.0)
        .children(separated.then(|| SecondaryText::new(city.to_owned()).px(12.0)))
        .children(separated.then(|| SecondaryText::new("\u{2022}").px(12.0)))
        .child(SecondaryText::new(date).px(12.0))
}

fn my_vacancy_row(vacancy: &VacancyModel, context: &Rc<MyVacanciesRowContext>, theme: &AppTheme) -> Div {
    let vacancy_id = vacancy.id;
    let edit_nav_store = context.nav_store.clone();
    let delete_page = context.page.clone();
    let delete_model = vacancy.clone();
    let (status_icon, status_label) = if vacancy.active {
        ("pause.circle", tr("vacancy_status_inactive"))
    } else {
        ("play.circle", tr("vacancy_status_active"))
    };
    card_surface(theme)
        .child(
            hstack(8.0)
                .w_full()
                .child(
                    MainText::new(vacancy.title.clone())
                        .px(15.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.primary)
                        .line_limit(2),
                )
                .child(spacer())
                .child(status_badge(vacancy.active, vacancy.is_awaiting_review(), theme)),
        )
        .children(
            vacancy
                .is_moderation_disabled()
                .then(|| ModerationHiddenBanner::new(tr("vacancy_disabled_by_moderation"))),
        )
        .child(
            MainText::new(vacancy.formatted_salary())
                .px(14.0)
                .font_weight(FontWeight::BOLD)
                .color(theme.primary._100),
        )
        .child(city_and_date(&vacancy.city, vacancy.published_at()))
        .child(action_row(
            named("my-vacancy-actions", vacancy_id),
            vec![
                action_btn(named("my-vacancy-edit", vacancy_id), "pencil", tr("vacancy_edit"), false, false, theme, move |_, cx| {
                    push(&edit_nav_store, RootRoute::EditVacancy { vacancy_id }, cx);
                }),
                action_btn(named("my-vacancy-status", vacancy_id), status_icon, status_label, false, false, theme, move |_, cx| {
                    VacancyActionsStore::shared(cx).update(cx, |actions, cx| {
                        actions.toggle_vacancy_status_action(vacancy_id, cx);
                    });
                }),
                action_btn(
                    named("my-vacancy-delete", vacancy_id),
                    "trash",
                    tr("vacancy_delete"),
                    true,
                    context.is_delete_pending,
                    theme,
                    move |_, cx| MyVacancies::request_delete_vacancy(&delete_page, &delete_model, cx),
                ),
            ],
        ))
}

fn resume_row(resume: &ResumeModel, context: &Rc<MyVacanciesRowContext>, theme: &AppTheme) -> Div {
    let resume_id = resume.id;
    let view_nav_store = context.nav_store.clone();
    let edit_nav_store = context.nav_store.clone();
    let share_model = resume.clone();
    card_surface(theme)
        .child(
            hstack(8.0)
                .w_full()
                .child(
                    vstack(2.0)
                        .items_start()
                        .min_w_0()
                        .child(
                            MainText::new(resume.profession.clone())
                                .px(15.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.text.primary),
                        )
                        .child(SecondaryText::new(resume.full_name()).px(13.0)),
                )
                .child(spacer()),
        )
        .child(
            MainText::new(resume.formatted_salary())
                .px(14.0)
                .font_weight(FontWeight::BOLD)
                .color(theme.primary._100),
        )
        .child(city_and_date(&resume.city, resume.updated_at()))
        .child(action_row(
            named("my-resume-actions", resume_id),
            vec![
                action_btn(named("my-resume-view", resume_id), "eye", tr("vacancy_resume_view"), false, false, theme, move |_, cx| {
                    push(&view_nav_store, RootRoute::MyResume, cx);
                }),
                action_btn(named("my-resume-edit", resume_id), "pencil", tr("vacancy_edit"), false, false, theme, move |_, cx| {
                    push(&edit_nav_store, RootRoute::CreateResume, cx);
                }),
                action_btn(
                    named("my-resume-download", resume_id),
                    "arrow.down.circle",
                    tr("vacancy_resume_download"),
                    false,
                    false,
                    theme,
                    move |_, cx| {
                        let model = share_model.clone();
                        VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            interactions.share_resume(model, cx);
                        });
                    },
                ),
            ],
        ))
}

fn status_badge(active: bool, awaiting_review: bool, theme: &AppTheme) -> Div {
    let color: Hsla = if active && !awaiting_review { rgb(ACTIVE_GREEN).into() } else { theme.text.secondary };
    let label = if awaiting_review {
        tr("music_status_pending")
    } else if active {
        tr("vacancy_status_active")
    } else {
        tr("vacancy_status_inactive")
    };
    div()
        .flex_none()
        .padding_edges(EdgeSet::HORIZONTAL, 8.0)
        .padding_edges(EdgeSet::VERTICAL, 4.0)
        .bg(color.opacity(0.12))
        .capsule()
        .swiftui_font(Font::system_weight(11.0, FontWeight::MEDIUM))
        .text_color(color)
        .whitespace_nowrap()
        .child(SharedString::from(label))
}

/// action row,
/// picks the row or its scroller and one element is both since a fitting scroller doesnt move
fn action_row(id: ElementId, buttons: Vec<PlainButton>) -> impl IntoElement {
    div()
        .id(id)
        .w_full()
        .flex()
        .overflow_x_scroll()
        .restrict_scroll_to_axis()
        .child(hstack(8.0).flex_none().children(buttons))
}

/// action button with icon, label and destructive flag
fn action_btn(
    id: ElementId,
    icon: &'static str,
    label: String,
    destructive: bool,
    disabled: bool,
    theme: &AppTheme,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> PlainButton {
    let ink = if destructive { theme.semantic.error_100 } else { theme.text.secondary };
    plain(id).flex_none().disabled(disabled).on_press(action).child(
        hstack(4.0)
            .padding_edges(EdgeSet::HORIZONTAL, 12.0)
            .padding_edges(EdgeSet::VERTICAL, 6.0)
            .bg(ink.opacity(0.08))
            .capsule()
            .children(IconUi::sized(icon, px(12.0), ink, false))
            .child(
                div()
                    .swiftui_font(Font::system_weight(12.0, FontWeight::MEDIUM))
                    .text_color(ink)
                    .whitespace_nowrap()
                    .child(SharedString::from(label)),
            ),
    )
}

fn empty_state(icon: &str, title_key: &str, subtitle_key: &str, theme: &AppTheme) -> impl IntoElement {
    vstack(16.0)
        .w_full()
        .pt(px(80.0))
        .px(px(32.0))
        .children(IconUi::sized(icon, px(48.0), theme.text.secondary.opacity(0.5), false))
        .child(
            MainText::new(tr(title_key))
                .px(17.0)
                .tac(TextAlignT::Center)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.text.primary),
        )
        .child(SecondaryText::new(tr(subtitle_key)).px(14.0).tac(TextAlignT::Center))
}
