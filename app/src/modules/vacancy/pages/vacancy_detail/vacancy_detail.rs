use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Context, ElementId, Empty, Entity, FontWeight, Hsla, SharedString, Subscription, WeakEntity, Window, rgb};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_text::characters;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::button_ui::ButtonUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::{PageHeaderIconButton, PageHeaderToolbarSlot};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::text_area_ui::TextAreaUi;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::vacancy::shared::vacancy_mocks::VacancyLocalState;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_actions::VacancyActionsStore;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::VacancyModel;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_wire_types::WireVacancyDTOExt;
use crate::modules::vacancy::stores::vacancy_interactions::vacancy_interactions::VacancyInteractionsStore;
use crate::modules::vacancy::stores::vacancy_services::vacancy_services::VacancyServicesStore;

const APPLIED_GREEN: u32 = 0x00C781;
const COVER_LETTER_FIELD: &str = "cover_letter";
const MEDIUM_DETENT: f32 = 0.5;
const PREFETCH_THRESHOLD: usize = 8;
const DESCRIPTION_LINE_HEIGHT: f32 = 21.0;

fn applied_green() -> Hsla {
    rgb(APPLIED_GREEN).into()
}

pub struct VacancyDetail {
    vacancy_id: i64,
    show_apply_sheet: bool,
    selected_resume_id: Option<i64>,
    cover_letter: String,
    is_vacancy_menu_presented: bool,
    _subscriptions: Vec<Subscription>,
}

impl VacancyDetail {
    pub fn new(vacancy_id: i64, cx: &mut Context<Self>) -> Self {
        let services = VacancyServicesStore::shared(cx);
        let actions = VacancyActionsStore::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        Self {
            vacancy_id,
            show_apply_sheet: false,
            selected_resume_id: None,
            cover_letter: String::new(),
            is_vacancy_menu_presented: false,
            _subscriptions: vec![
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
                cx.observe(&cache_revisions, |_, _, cx| cx.notify()),
            ],
        }
    }

    /// the vacancy model, if loaded
    fn vacancy(&self, cx: &mut App) -> Option<VacancyModel> {
        let vacancy_id = self.vacancy_id;
        let sandbox = GuideSandbox::shared(cx);
        if let Some(practice) = sandbox.read(cx).practice_vacancy(vacancy_id) {
            return Some(practice);
        }
        if VacancyServicesStore::shared(cx).read(cx).is_vacancy_gone(vacancy_id) {
            return None;
        }
        let actions = VacancyActionsStore::shared(cx);
        let (single, list) = {
            let actions = actions.read(cx);
            (actions.get_vacancy_sai.clone(), actions.list_vacancies_sai.clone())
        };
        if let Some(vacancy) = single.data().and_then(|dto| dto.vacancy).filter(|vacancy| vacancy.id == vacancy_id) {
            return Some(vacancy.to_model());
        }
        list.data()?.list.iter().find(|vacancy| vacancy.id == vacancy_id).map(WireVacancyDTOExt::to_model)
    }

    /// reacts to the apply sheet flag changing
    fn set_show_apply_sheet(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.show_apply_sheet == open {
            return;
        }
        self.show_apply_sheet = open;
        GuideFacts::set(RiseGuideSignal::VacancyApplyOpen, open, cx);
        cx.notify();
    }

    fn set_selected_resume_id(&mut self, id: Option<i64>, cx: &mut Context<Self>) {
        if self.selected_resume_id == id {
            return;
        }
        self.selected_resume_id = id;
        GuideFacts::set(RiseGuideSignal::VacancyApplyResumeChosen, id.is_some(), cx);
        cx.notify();
    }

    fn pop(nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        if let Some(nav_store) = nav_store.as_ref() {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn request_delete_vacancy(
        vacancy: &VacancyModel,
        vacancy_id: i64,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        let route = RootRoute::VacancyDetail { vacancy_id };
        let can_proceed: Rc<dyn Fn(&mut App) -> bool> = Rc::new(|_: &mut App| true);
        let on_cancel: Rc<dyn Fn(&mut App)> = Rc::new(|_: &mut App| {});
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            let Some(nav_store) = nav_store.as_ref() else {
                return;
            };
            if nav_store.read(cx).path.last() != Some(&route) {
                return;
            }
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        });
        let (id, title) = (vacancy.id, vacancy.title.clone());
        VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.delete_vacancy(id, title, can_proceed, on_cancel, Some(on_success), cx);
        });
    }

    fn header_actions(
        this: Entity<VacancyDetail>,
        vacancy: Option<&VacancyModel>,
        vacancy_id: i64,
        viewer_id: Option<&str>,
        nav_store: &Option<Entity<NavigationStore>>,
        theme: &AppTheme,
        cx: &mut App,
    ) -> AnyElement {
        let share_model = vacancy.cloned();
        let share_disabled = vacancy.is_none_or(|vacancy| vacancy.id <= 0);
        let menu = vacancy
            .filter(|vacancy| viewer_id != Some(vacancy.author_id.as_str()))
            .map(|vacancy| Self::header_menu(this, vacancy, vacancy_id, nav_store, theme, cx));
        hstack(6.0)
            .child(
                PageHeaderIconButton::new("vacancy-detail-share", "square.and.arrow.up", move |_, cx| {
                    let Some(model) = share_model.clone() else {
                        return;
                    };
                    VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.share_vacancy(model, cx);
                    });
                })
                .font_size(17.0)
                .disabled(share_disabled),
            )
            .children(menu)
            .into_any_element()
    }

    fn vacancy_menu_glyph(theme: &AppTheme) -> PageHeaderToolbarSlot {
        PageHeaderToolbarSlot::new(div().children(IconUi::sized("ellipsis", px(17.0), theme.text.primary, false)))
    }

    fn vacancy_menu_actions(vacancy: &VacancyModel, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let mut actions = Vec::new();
        if AuthServicesStore::shared(cx).read(cx).viewer_acts_as_author() {
            let (title, image) = if vacancy.active {
                (tr("vacancy_status_inactive"), "pause.circle")
            } else {
                (tr("vacancy_status_active"), "play.circle")
            };
            actions.push(HoldContextMenuAction::new("toggle_status", title).system_image(image));
            actions.push(
                HoldContextMenuAction::new("delete", tr("vacancy_delete"))
                    .system_image("trash")
                    .destructive(true)
                    .disabled(VacancyActionsStore::shared(cx).read(cx).delete_vacancy_sai.is_pending()),
            );
        }
        let section_break = !actions.is_empty();
        actions.push(
            HoldContextMenuAction::new("report", tr("ctx_report"))
                .system_image("exclamationmark.bubble")
                .destructive(true)
                .section_break(section_break),
        );
        actions
    }

    fn header_menu(
        this: Entity<VacancyDetail>,
        vacancy: &VacancyModel,
        vacancy_id: i64,
        nav_store: &Option<Entity<NavigationStore>>,
        theme: &AppTheme,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        let is_presented = Binding::entity(
            this.clone(),
            |page: &VacancyDetail| page.is_vacancy_menu_presented,
            |page: &mut VacancyDetail, value: bool, cx| {
                page.is_vacancy_menu_presented = value;
                cx.notify();
            },
        );
        let menu_actions = Self::vacancy_menu_actions(vacancy, cx);
        let actions: HoldContextMenuPresentationActions = Rc::new(move |_: &App| menu_actions.clone());
        let action_model = vacancy.clone();
        let action_nav_store = nav_store.clone();
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            match id.as_str() {
                "toggle_status" => {
                    let id = action_model.id;
                    VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.toggle_vacancy_status(id, cx);
                    });
                }
                "delete" => Self::request_delete_vacancy(&action_model, vacancy_id, action_nav_store.clone(), cx),
                "report" => {
                    let report_id = action_model.id;
                    ReportSheetStore::shared(cx).update(cx, |store, cx| {
                        store.present(ReportTarget::Vacancy(report_id), None, None, cx);
                    });
                }
                _ => {}
            }
        });
        let source_theme = theme.clone();
        let source: HoldContextMenuTapSource =
            Rc::new(move |_: &mut Window, _: &mut App| Self::vacancy_menu_glyph(&source_theme).into_any_element());
        plain("vacancy-detail-menu")
            .on_press(move |_, cx| {
                this.update(cx, |page, cx| {
                    page.is_vacancy_menu_presented = true;
                    cx.notify();
                });
            })
            .child(Self::vacancy_menu_glyph(theme))
            .tap_context_menu(
                "vacancy-detail-menu-anchor",
                is_presented,
                HoldContextMenuAlignment::Trailing,
                false,
                false,
                false,
                None,
                actions,
                on_action,
                source,
            )
    }
}

impl Render for VacancyDetail {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let vacancy_id = self.vacancy_id;
        let vacancy = self.vacancy(cx);
        let viewer_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let (is_loading_vacancy, is_apply_pending) = {
            let actions = VacancyActionsStore::shared(cx);
            let actions = actions.read(cx);
            (actions.get_vacancy_sai.is_pending(), actions.apply_vacancy_sai.is_pending())
        };
        let title = vacancy.as_ref().map_or_else(|| tr("vacancy_detail_title"), |vacancy| vacancy.company.clone());
        let header_actions =
            Self::header_actions(cx.entity(), vacancy.as_ref(), vacancy_id, viewer_id.as_deref(), &nav_store, &theme, cx);

        let page = cx.weak_entity();
        let sheet_page = page.clone();
        let is_presented = Binding::entity(
            cx.entity(),
            |page: &VacancyDetail| page.show_apply_sheet,
            |page, open, cx| page.set_show_apply_sheet(open, cx),
        );
        let back_nav_store = nav_store;

        div()
            .relative()
            .size_full()
            .child(
                SettingsWrapperUi::new(title, move |_, cx| Self::pop(&back_nav_store, cx))
                    .right_jsx(header_actions)
                    .content(VacancyDetailBody { vacancy, viewer_id, is_loading_vacancy, is_apply_pending, page }),
            )
            .child(
                BottomSheetUi::new(
                    "vacancy-detail-apply-sheet",
                    is_presented,
                    move |_, cx| apply_sheet(&sheet_page, cx),
                    |_, _| Empty,
                )
                .snap_fractions(vec![MEDIUM_DETENT])
                .content_scrollable(true)
                .sheet_background(Some(theme.bg._100)),
            )
            .on_appear(ElementId::Name(format!("vacancy-detail-{vacancy_id}").into()), move |_, cx| {
                VacancyLocalState::shared(cx).update(cx, |state, cx| state.mark_viewed(vacancy_id, cx));
                VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.ensure_vacancy_loaded(vacancy_id, cx);
                    interactions.ensure_my_resumes_loaded(cx);
                });
            })
            .and_on_disappear(move |_, cx| {
                GuideFacts::set(RiseGuideSignal::VacancyApplyOpen, false, cx);
                GuideFacts::set(RiseGuideSignal::VacancyApplyResumeChosen, false, cx);
            })
    }
}

#[derive(IntoElement)]
struct VacancyDetailBody {
    vacancy: Option<VacancyModel>,
    viewer_id: Option<String>,
    is_loading_vacancy: bool,
    is_apply_pending: bool,
    page: WeakEntity<VacancyDetail>,
}

impl RenderOnce for VacancyDetailBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let Some(vacancy) = self.vacancy else {
            let center = div().size_full().flex().items_center().justify_center();
            return if self.is_loading_vacancy {
                center.child(progress_view()).into_any_element()
            } else {
                center
                    .child(
                        MainText::new(tr("vacancy_not_found"))
                            .px(16.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    )
                    .into_any_element()
            };
        };
        let is_foreign = self.viewer_id.as_deref() != Some(vacancy.author_id.as_str());
        let is_applied = vacancy.is_applied.unwrap_or(false);
        let apply = is_foreign.then(|| apply_button(is_applied, self.is_apply_pending, self.page.clone(), &theme));
        div()
            .size_full()
            .bg(theme.bg._100)
            .child(
                scroll_view("vacancy-detail-scroll")
                    .size_full()
                    .shows_indicators(false)
                    .list_shell_scroll_under_chrome(cx)
                    .child(
                        vstack(0.0)
                            .w_full()
                            .items_start()
                            .child(header(&vacancy, &theme))
                            .child(
                                vstack(20.0)
                                    .w_full()
                                    .items_start()
                                    .p(px(20.0))
                                    .child(salary_block(&vacancy, &theme))
                                    .child(tags_block(&vacancy, &theme))
                                    .child(skills_block(&vacancy, &theme))
                                    .child(div().w_full().h(px(1.0)).bg(theme.border._100))
                                    .child(description_block(&vacancy, &theme))
                                    .children(apply),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn header(vacancy: &VacancyModel, theme: &AppTheme) -> impl IntoElement {
    let initial = SharedString::from(characters(&vacancy.company).next().unwrap_or("").to_owned());
    vstack(14.0)
        .w_full()
        .items_start()
        .p(px(20.0))
        .bg(theme.bg._200)
        .child(
            hstack(14.0)
                .w_full()
                .child(
                    div()
                        .flex_none()
                        .size(px(64.0))
                        .bg(theme.primary._100.opacity(0.15))
                        .continuous_corners(16.0)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .swiftui_font(Font::system_weight(26.0, FontWeight::BOLD))
                                .text_color(theme.primary._100)
                                .child(initial),
                        ),
                )
                .child(
                    vstack(4.0)
                        .items_start()
                        .min_w_0()
                        .child(
                            MainText::new(vacancy.company.clone())
                                .px(15.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.text.primary),
                        )
                        .child(
                            hstack(4.0)
                                .children(IconUi::sized("mappin.circle.fill", px(13.0), theme.text.secondary, true))
                                .child(SecondaryText::new(vacancy.city.clone()).px(13.0)),
                        ),
                )
                .child(spacer()),
        )
        .child(
            MainText::new(vacancy.title.clone())
                .px(22.0)
                .font_weight(FontWeight::BOLD)
                .color(theme.text.primary)
                .line_limit(3),
        )
        .child(SecondaryText::new(format!("{} {}", tr("vacancy_published"), vacancy.published_at())).px(12.0))
}

fn section_title(key: &str, theme: &AppTheme) -> MainText {
    MainText::new(tr(key)).px(14.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary)
}

fn salary_block(vacancy: &VacancyModel, theme: &AppTheme) -> impl IntoElement {
    vstack(4.0)
        .items_start()
        .child(SecondaryText::new(tr("vacancy_salary")).px(12.0))
        .child(
            MainText::new(vacancy.formatted_salary())
                .px(20.0)
                .font_weight(FontWeight::BOLD)
                .color(theme.primary._100),
        )
}

fn tags_block(vacancy: &VacancyModel, theme: &AppTheme) -> impl IntoElement {
    vstack(10.0)
        .w_full()
        .items_start()
        .child(section_title("vacancy_conditions", theme))
        .child(
            div()
                .w_full()
                .grid()
                .grid_cols(2)
                .gap(px(8.0))
                .child(condition_row("clock.fill", vacancy.schedule_display(), theme))
                .child(condition_row("building.2.fill", vacancy.job_format_display(), theme))
                .child(condition_row("briefcase.fill", vacancy.experience_display(), theme))
                .children(
                    vacancy
                        .is_relocation_available
                        .then(|| condition_row("airplane", tr("vacancy_relocation"), theme)),
                ),
        )
}

fn condition_row(icon: &'static str, label: String, theme: &AppTheme) -> Div {
    hstack(8.0)
        .min_w_0()
        .padding_edges(EdgeSet::HORIZONTAL, 12.0)
        .padding_edges(EdgeSet::VERTICAL, 10.0)
        .bg(theme.bg._200)
        .continuous_corners(10.0)
        .child(
            div()
                .flex_none()
                .w(px(20.0))
                .flex()
                .justify_center()
                .children(IconUi::sized(icon, px(13.0), theme.primary._100, false)),
        )
        .child(MainText::new(label).px(13.0).color(theme.text.primary).line_limit(1))
        .child(spacer_min_w(0.0))
}

fn skills_block(vacancy: &VacancyModel, theme: &AppTheme) -> impl IntoElement {
    vstack(10.0).w_full().items_start().child(section_title("vacancy_skills", theme)).child(
        div().w_full().flex().flex_wrap().gap(px(8.0)).children(vacancy.skills.iter().map(|skill| {
            div()
                .flex_none()
                .padding_edges(EdgeSet::HORIZONTAL, 12.0)
                .padding_edges(EdgeSet::VERTICAL, 6.0)
                .bg(theme.primary._100.opacity(0.12))
                .capsule()
                .swiftui_font(Font::system_weight(13.0, FontWeight::MEDIUM))
                .text_color(theme.primary._100)
                .whitespace_nowrap()
                .child(SharedString::from(skill.clone()))
        })),
    )
}

fn description_block(vacancy: &VacancyModel, theme: &AppTheme) -> impl IntoElement {
    vstack(10.0)
        .w_full()
        .items_start()
        .child(section_title("vacancy_description", theme))
        .child(
            MainText::new(vacancy.description.clone())
                .px(14.0)
                .line_height(DESCRIPTION_LINE_HEIGHT)
                .color(theme.text.primary),
        )
}

fn apply_button(is_applied: bool, is_pending: bool, page: WeakEntity<VacancyDetail>, theme: &AppTheme) -> Div {
    let green = applied_green();
    let content = if is_applied {
        hstack(8.0)
            .w_full()
            .justify_center()
            .py(px(16.0))
            .bg(green.opacity(0.12))
            .rounded(theme.radius._200)
            .children(IconUi::sized("checkmark.circle.fill", px(17.0), green, true))
            .child(MainText::new(tr("vacancy_applied")).px(15.0).font_weight(FontWeight::SEMIBOLD).color(green))
            .into_any_element()
    } else {
        ButtonUi::new("vacancy-detail-apply")
            .disabled(is_pending)
            .on_press(move |_, cx| {
                let _ = page.update(cx, |page, cx| page.set_show_apply_sheet(true, cx));
            })
            .child(MainText::new(tr("vacancy_apply_btn")))
            .guide_anchor(RiseGuideAnchor::VacancyApply)
            .into_any_element()
    };
    vstack(12.0).w_full().pt(px(8.0)).pb(px(20.0)).child(content)
}

fn apply_sheet(page: &WeakEntity<VacancyDetail>, cx: &mut App) -> AnyElement {
    let Some(entity) = page.upgrade() else {
        return Empty.into_any_element();
    };
    let theme = rise_ui::theme(cx).clone();
    let (vacancy_id, selected_resume_id, cover_letter) = {
        let page = entity.read(cx);
        (page.vacancy_id, page.selected_resume_id, page.cover_letter.clone())
    };
    let my_resumes = VacancyServicesStore::my_resumes(cx);
    let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
    let cache_key = SharedString::from(VacancyServicesStore::my_resumes_cache_key(&user_id));
    let relative_id = VacancyServicesStore::my_resumes_relative_id(cx).map(|id| id.to_string());
    let has_more = VacancyServicesStore::my_resumes_has_more(cx);
    let is_pending = VacancyActionsStore::shared(cx).read(cx).apply_vacancy_sai.is_pending();

    let item_count = my_resumes.len();
    let resume_rows = my_resumes.into_iter().enumerate().map(|(index, resume)| {
        let resume_id = resume.id;
        let is_selected = selected_resume_id == Some(resume_id);
        let select_page = page.clone();
        plain(ElementId::Name(format!("vacancy-apply-resume-{resume_id}").into()))
            .w_full()
            .on_press(move |_, cx| {
                let _ = select_page.update(cx, |page, cx| {
                    let next = if page.selected_resume_id == Some(resume_id) { None } else { Some(resume_id) };
                    page.set_selected_resume_id(next, cx);
                });
            })
            .child(
                hstack(8.0)
                    .w_full()
                    .p(px(12.0))
                    .bg(if is_selected { theme.primary._100.opacity(0.1) } else { theme.bg._200 })
                    .rounded(theme.radius._200)
                    .when(is_selected, |row| row.stroke_border(theme.primary._100, 1.0))
                    .child(
                        vstack(2.0)
                            .items_start()
                            .min_w_0()
                            .child(
                                MainText::new(resume.profession.clone())
                                    .px(14.0)
                                    .font_weight(FontWeight::MEDIUM)
                                    .color(theme.text.primary),
                            )
                            .child(SecondaryText::new(resume.full_name()).px(12.0)),
                    )
                    .child(spacer())
                    .children(is_selected.then(|| IconUi::sized("checkmark.circle.fill", px(17.0), theme.primary._100, true)).flatten()),
            )
            .guide_anchor((index == 0).then_some(RiseGuideAnchor::VacancyApplyResume))
            .sai_pagination_prefetch(
                ElementId::Name(format!("vacancy-apply-resume-prefetch-{resume_id}").into()),
                SaiPaginationPrefetchOptions::new(index, item_count, cache_key.clone(), relative_id.clone(), has_more)
                    .threshold(PREFETCH_THRESHOLD),
                |_, cx| {
                    VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.load_more_my_resumes(cx);
                    });
                },
            )
    });
    let resumes = (item_count > 0).then(|| {
        vstack(8.0)
            .w_full()
            .items_start()
            .child(SecondaryText::new(tr("vacancy_apply_resume_label")).px(13.0))
            .children(resume_rows)
    });

    let cover_page = page.clone();
    let cover_field = vstack(8.0)
        .w_full()
        .items_start()
        .child(SecondaryText::new(tr("vacancy_apply_cover_label")).px(13.0))
        .child(
            TextAreaUi::new("vacancy-apply-cover")
                .values(HashMap::from([(COVER_LETTER_FIELD.to_owned(), cover_letter.clone())]))
                .name(COVER_LETTER_FIELD)
                .min_lines(3)
                .max_lines(8)
                .set_value(move |_, value, _, cx| {
                    let _ = cover_page.update(cx, |page, cx| {
                        page.cover_letter = value.to_owned();
                        cx.notify();
                    });
                }),
        )
        .guide_anchor(RiseGuideAnchor::VacancyApplyCover);

    let close_page = page.clone();
    let close: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
        let _ = close_page.update(cx, |page, cx| page.set_show_apply_sheet(false, cx));
    });
    let confirm_label = if is_pending { tr("vacancy_apply_sending") } else { tr("vacancy_apply_confirm_btn") };
    let confirm = ButtonUi::new("vacancy-apply-confirm")
        .disabled(is_pending)
        .on_press(move |_, cx| {
            let cover_letter = (!cover_letter.is_empty()).then(|| cover_letter.clone());
            let (on_success, on_failure) = (close.clone(), close.clone());
            VacancyActionsStore::shared(cx).update(cx, |actions, cx| {
                actions.apply_vacancy_action(
                    vacancy_id,
                    selected_resume_id,
                    cover_letter,
                    Some(on_success),
                    Some(on_failure),
                    cx,
                );
            });
        })
        .child(MainText::new(confirm_label))
        .guide_anchor(RiseGuideAnchor::VacancyApplyConfirm);

    vstack(16.0)
        .w_full()
        .items_start()
        .pt(px(16.0))
        .px(px(20.0))
        .pb(px(32.0))
        .bg(theme.bg._100)
        .child(
            MainText::new(tr("vacancy_apply_confirm_title"))
                .px(18.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.text.primary),
        )
        .children(resumes)
        .child(cover_field)
        .child(confirm)
        .into_any_element()
}
