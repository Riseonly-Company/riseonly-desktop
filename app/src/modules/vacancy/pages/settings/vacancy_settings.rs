use gpui::{App, Context, Entity, FontWeight, Hsla, Subscription, Window, rgb};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::vacancy::stores::vacancy_actions::vacancy_actions::VacancyActionsStore;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::ResumeModel;
use crate::modules::vacancy::stores::vacancy_interactions::vacancy_interactions::VacancyInteractionsStore;
use crate::modules::vacancy::stores::vacancy_services::vacancy_services::VacancyServicesStore;

const BLUE: u32 = 0x008DE5;
const GREEN: u32 = 0x00C781;
const ORANGE: u32 = 0xF19A1A;

fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

fn push_callback(
    nav_store: &Option<Entity<NavigationStore>>,
    route: RootRoute,
) -> impl Fn(&mut Window, &mut App) + 'static + use<> {
    let nav_store = nav_store.clone();
    move |_, cx| {
        if let Some(nav_store) = nav_store.as_ref() {
            let route = route.clone();
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }
}

pub struct VacancySettings {
    _subscriptions: Vec<Subscription>,
}

impl VacancySettings {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let services = VacancyServicesStore::shared(cx);
        let actions = VacancyActionsStore::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        Self {
            _subscriptions: vec![
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&cache_revisions, |_, _, cx| cx.notify()),
            ],
        }
    }
}

impl Render for VacancySettings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let resume = VacancyServicesStore::my_resume(cx);
        let back_nav_store = nav_store.clone();
        SettingsWrapperUi::new(tr("vacancy_settings_title"), move |_, cx| {
            if let Some(nav_store) = back_nav_store.as_ref() {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(VacancySettingsBody { resume, nav_store })
        .on_appear("vacancy-settings-page", |_, cx| {
            VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.ensure_my_resumes_loaded(cx);
            });
        })
    }
}

#[derive(IntoElement)]
struct VacancySettingsBody {
    resume: Option<ResumeModel>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl RenderOnce for VacancySettingsBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        div().size_full().bg(theme.bg._100).child(
            scroll_view("vacancy-settings-scroll").size_full().list_shell_scroll_under_chrome(cx).child(
                vstack(20.0)
                    .w_full()
                    .pt(px(12.0))
                    .child(employer_banner(&theme))
                    .child(GroupedBtns::new(
                        "vacancy-settings-my-vacancies",
                        vec![
                            GroupedBtn::new("my_vacancies", tr("vacancy_my_vacancies_btn"))
                                .icon("briefcase.fill")
                                .icon_background(hex(BLUE))
                                .guide_anchor(RiseGuideAnchor::VacancyMyVacancies)
                                .callback(push_callback(&self.nav_store, RootRoute::MyVacancies)),
                        ],
                    ))
                    .child(resume_group(self.resume.as_ref(), &self.nav_store))
                    .child(GroupedBtns::new(
                        "vacancy-settings-employer",
                        vec![
                            GroupedBtn::new("be_employer", tr("vacancy_be_employer_btn"))
                                .icon("building.2.fill")
                                .icon_background(hex(ORANGE))
                                .subtitle(tr("vacancy_be_employer_site_hint"))
                                .callback(|_, cx| {
                                    VacancyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                                        interactions.open_employer_application_site(cx);
                                    });
                                }),
                        ],
                    ))
                    .child(spacer_min_h(40.0)),
            ),
        )
    }
}

fn resume_group(resume: Option<&ResumeModel>, nav_store: &Option<Entity<NavigationStore>>) -> GroupedBtns {
    let Some(resume) = resume else {
        return GroupedBtns::new(
            "vacancy-settings-resume",
            vec![
                GroupedBtn::new("create_resume", tr("vacancy_create_resume_btn"))
                    .icon("doc.text.fill")
                    .icon_background(hex(GREEN))
                    .guide_anchor(RiseGuideAnchor::VacancyCreateResume)
                    .callback(push_callback(nav_store, RootRoute::CreateResume)),
            ],
        );
    };
    let subtitle = if resume.profession.is_empty() { resume.full_name() } else { resume.profession.clone() };
    GroupedBtns::new(
        "vacancy-settings-resume",
        vec![
            GroupedBtn::new("view_resume", tr("vacancy_resume_view"))
                .icon("doc.text.fill")
                .icon_background(hex(GREEN))
                .subtitle(subtitle)
                .guide_anchor(RiseGuideAnchor::VacancyViewResume)
                .callback(push_callback(nav_store, RootRoute::MyResume)),
            GroupedBtn::new("edit_resume", tr("vacancy_resume_edit_title"))
                .icon("pencil")
                .icon_background(hex(BLUE))
                .callback(push_callback(nav_store, RootRoute::CreateResume)),
        ],
    )
    .title(tr("vacancy_resume_my_title"))
}

fn employer_banner(theme: &AppTheme) -> impl IntoElement {
    div().w_full().px(px(16.0)).child(
        hstack(16.0)
            .w_full()
            .p(px(16.0))
            .bg(theme.bg._200)
            .rounded(theme.radius._300)
            .stroke_border(theme.primary._100.opacity(0.2), 1.0)
            .child(
                div()
                    .flex_none()
                    .size(px(52.0))
                    .bg(theme.primary._100.opacity(0.15))
                    .continuous_corners(14.0)
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("briefcase.fill", px(22.0), theme.primary._100, true)),
            )
            .child(
                vstack(3.0)
                    .items_start()
                    .min_w_0()
                    .child(
                        MainText::new(tr("vacancy_settings_banner_title"))
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary),
                    )
                    .child(SecondaryText::new(tr("vacancy_settings_banner_subtitle")).px(12.0).line_limit(2)),
            )
            .child(spacer_min_w(0.0)),
    )
}
