use std::rc::Rc;

use gpui::{App, Entity, FontWeight, Hsla, SharedString, Subscription, Window};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_text::character_prefix;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::button_style::PlainButton;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::vacancy::components::resume_form_ui::ResumeFlowLayout;
use crate::modules::vacancy::components::vacancy_form_support::pop_screen;
use crate::modules::vacancy::shared::resume_option_catalog::ResumeOptionCatalog;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_actions::VacancyActionsStore;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::{ResumeModel, WorkExperienceModel};
use crate::modules::vacancy::stores::vacancy_interactions::vacancy_interactions::VacancyInteractionsStore;
use crate::modules::vacancy::stores::vacancy_services::vacancy_services::VacancyServicesStore;

const POSIX_LOCALE: &str = "en_US_POSIX";
const WIRE_DATE_FORMAT: &str = "yyyy-MM-dd";
const DISPLAY_DATE_FORMAT: &str = "dd.MM.yyyy";
const WIRE_DATE_LENGTH: usize = 10;
const PERIOD_SEPARATOR: &str = " \u{2014} ";
const COMPANY_SEPARATOR: &str = " \u{b7} ";
const LOCATION_PIN: &str = "\u{1F4CD}";

struct MyResumeState {
    resume_pending_deletion: Option<i64>,
    deletion_account_id: Option<String>,
    deletion_account_epoch: Option<String>,
    deletion_request_id: Option<Uuid>,
    _subscriptions: [Subscription; 3],
}

impl MyResumeState {
    fn new(cx: &mut Context<Self>) -> Self {
        let store = VacancyActionsStore::shared(cx);
        let service = VacancyServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            resume_pending_deletion: None,
            deletion_account_id: None,
            deletion_account_epoch: None,
            deletion_request_id: None,
            _subscriptions: [
                cx.observe(&store, |_, _, cx| cx.notify()),
                cx.observe(&service, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement, Default)]
pub struct MyResume;

impl MyResume {
    pub fn new() -> Self {
        Self
    }

    fn push_create_resume(nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| {
                nav_store.push(RootRoute::CreateResume, NavigationStoreSettlingPush::Drop, cx);
            });
        }
    }

    fn avatar(r: &ResumeModel, theme: &AppTheme) -> Div {
        const SIDE: f32 = 96.0;
        let placeholder = div()
            .absolute()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized("person.fill", px(36.0), theme.text.secondary, true));
        let circle =
            div().relative().w(px(SIDE)).h(px(SIDE)).rounded_full().overflow_hidden().bg(theme.bg._300).child(placeholder);
        if r.avatar.is_empty() {
            return circle;
        }
        circle.child(
            div().absolute().size_full().child(
                CleverImageUi::new(r.avatar.clone())
                    .corner_radius(SIDE / 2.0)
                    .shows_progress_indicator(false)
                    .shows_failure_icon(false),
            ),
        )
    }

    fn header(r: &ResumeModel, theme: &AppTheme) -> Div {
        let location = r.location_display();
        let salary = r.formatted_salary();
        vstack(10.0)
            .w_full()
            .items_center()
            .child(Self::avatar(r, theme))
            .child(MainText::new(r.full_name()).px(20.0).font_weight(FontWeight::BOLD).color(theme.text.primary))
            .when(!r.profession.is_empty(), |column| {
                column.child(
                    MainText::new(r.profession.clone()).px(15.0).font_weight(FontWeight::SEMIBOLD).color(theme.primary._100),
                )
            })
            .when(!location.is_empty(), |column| {
                column.child(SecondaryText::new(format!("{LOCATION_PIN} {location}")).px(13.0))
            })
            .when(salary != tr("vacancy_salary_negotiable"), |column| {
                column.child(MainText::new(salary).px(15.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary))
            })
    }

    fn action_button(
        id: &'static str,
        icon: &str,
        label: String,
        theme: &AppTheme,
        primary: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> PlainButton {
        let background = if primary { theme.primary._100 } else { theme.bg._200 };
        let foreground: Hsla = if primary { theme.content_on(background) } else { theme.text.primary };
        plain(id).w_full().on_press(action).child(
            div()
                .relative()
                .w_full()
                .h(px(46.0))
                .child(
                    hstack(7.0)
                        .size_full()
                        .justify_center()
                        .bg(background)
                        .capsule()
                        .children(IconUi::sized(icon, px(15.0), foreground, false))
                        .child(MainText::new(label).px(14.0).font_weight(FontWeight::SEMIBOLD).color(foreground)),
                )
                .when(!primary, |button| {
                    button.child(div().absolute().size_full().continuous_corners(13.0).stroke_border(theme.border._100, 1.0))
                }),
        )
    }

    fn actions(r: &ResumeModel, is_building_pdf: bool, nav_store: Option<Entity<NavigationStore>>, theme: &AppTheme) -> Div {
        let shared_resume = r.clone();
        hstack(10.0)
            .w_full()
            .px(px(16.0))
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    Self::action_button(
                        "my-resume-download",
                        "arrow.down.doc.fill",
                        if is_building_pdf { tr("vacancy_resume_generating") } else { tr("vacancy_resume_download") },
                        theme,
                        true,
                        move |_, cx| {
                            let model = shared_resume.clone();
                            VacancyInteractionsStore::shared(cx).update(cx, |store, cx| store.share_resume(model, cx));
                        },
                    )
                    .guide_anchor(RiseGuideAnchor::ResumeDownload),
                ),
            )
            .child(div().flex_1().min_w(px(0.0)).child(Self::action_button(
                "my-resume-edit",
                "pencil",
                tr("vacancy_edit"),
                theme,
                false,
                move |_, cx| Self::push_create_resume(nav_store.as_ref(), cx),
            )))
    }

    fn info_card(title: String, content: Vec<AnyElement>, theme: &AppTheme) -> Div {
        div().w_full().px(px(16.0)).child(
            vstack(12.0)
                .align(HorizontalAlignment::Leading)
                .w_full()
                .p(px(16.0))
                .bg(theme.bg._200)
                .continuous_corners(18.0)
                .child(MainText::new(title).px(14.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary))
                .children(content),
        )
    }

    fn info_row(label: String, value: String, theme: &AppTheme, last: bool) -> AnyElement {
        vstack(8.0)
            .w_full()
            .child(
                hstack(12.0)
                    .w_full()
                    .items_start()
                    .child(SecondaryText::new(label).px(13.0))
                    .child(spacer())
                    .child(
                        div()
                            .min_w(px(0.0))
                            .child(MainText::new(value).px(13.0).tac(TextAlignT::Right).color(theme.text.primary)),
                    ),
            )
            .when(!last, |column| column.child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.4))))
            .into_any_element()
    }

    fn contacts_card(r: &ResumeModel, theme: &AppTheme) -> Div {
        let yes_no = |value: bool| if value { tr("yes") } else { tr("no") };
        Self::info_card(
            tr("vacancy_resume_section_personal"),
            vec![
                Self::info_row(tr("vacancy_resume_phone"), r.phone.clone(), theme, false),
                Self::info_row(tr("vacancy_resume_gender"), tr(ResumeOptionCatalog::gender_key(&r.gender)), theme, false),
                Self::info_row(tr("vacancy_resume_birth_date"), Self::format_birth_date(&r.birth_date), theme, false),
                Self::info_row(tr("vacancy_resume_education"), tr(ResumeOptionCatalog::education_key(&r.education)), theme, false),
                Self::info_row(
                    tr("vacancy_resume_exp_level"),
                    tr(ResumeOptionCatalog::experience_key(&r.experience_level)),
                    theme,
                    false,
                ),
                Self::info_row(tr("vacancy_relocation_available"), yes_no(r.ready_to_relocation), theme, false),
                Self::info_row(tr("vacancy_resume_army"), yes_no(r.ready_to_army), theme, true),
            ],
            theme,
        )
    }

    fn company_logo(index: usize, exp: &WorkExperienceModel, theme: &AppTheme) -> Div {
        const SIDE: f32 = 42.0;
        let initial = div()
            .absolute()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .swiftui_font(Font::system_weight(16.0, FontWeight::BOLD))
            .text_color(theme.text.secondary)
            .child(character_prefix(&exp.company, 1).to_owned());
        let logo = exp.company_logo_url.clone().filter(|logo| !logo.is_empty());
        div()
            .relative()
            .flex_none()
            .w(px(SIDE))
            .h(px(SIDE))
            .bg(theme.bg._300)
            .continuous_corners(10.0)
            .overflow_hidden()
            .child(initial)
            .when_some(logo, |frame, logo| {
                frame.child(
                    div().absolute().size_full().child(
                        CleverImageUi::new(logo)
                            .id(SharedString::from(format!("my-resume-logo-{index}")))
                            .corner_radius(10.0)
                            .shows_progress_indicator(false)
                            .shows_failure_icon(false),
                    ),
                )
            })
    }

    fn experience_card(r: &ResumeModel, theme: &AppTheme) -> Div {
        let count = r.work_experience.len();
        let mut rows: Vec<AnyElement> = Vec::with_capacity(count * 2);
        for (idx, exp) in r.work_experience.iter().enumerate() {
            let subtitle = [exp.company.clone(), Self::period_text(exp)]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(COMPANY_SEPARATOR);
            rows.push(
                hstack(12.0)
                    .w_full()
                    .items_start()
                    .child(Self::company_logo(idx, exp, theme))
                    .child(
                        vstack(3.0)
                            .align(HorizontalAlignment::Leading)
                            .flex_1()
                            .min_w(px(0.0))
                            .child(
                                MainText::new(if exp.position.is_empty() { exp.company.clone() } else { exp.position.clone() })
                                    .px(14.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.text.primary),
                            )
                            .child(SecondaryText::new(subtitle).px(12.0))
                            .when(!exp.description.is_empty(), |column| {
                                column.child(div().pt(px(2.0)).child(SecondaryText::new(exp.description.clone()).px(12.0)))
                            }),
                    )
                    .into_any_element(),
            );
            if idx + 1 < count {
                rows.push(div().w_full().h(px(0.5)).my(px(4.0)).bg(theme.border._200.opacity(0.5)).into_any_element());
            }
        }
        Self::info_card(tr("vacancy_experience_section"), rows, theme)
    }

    fn skills_card(r: &ResumeModel, theme: &AppTheme) -> Div {
        let accent = theme.primary._100;
        let mut flow = ResumeFlowLayout::new().spacing(8.0).line_spacing(8.0);
        for skill in &r.skills {
            flow = flow.child(
                div()
                    .px(px(11.0))
                    .py(px(6.0))
                    .bg(accent.opacity(0.13))
                    .capsule()
                    .swiftui_font(Font::system_weight(13.0, FontWeight::MEDIUM))
                    .text_color(accent)
                    .child(skill.clone()),
            );
        }
        Self::info_card(tr("vacancy_field_skills"), vec![flow.into_any_element()], theme)
    }

    fn summary_card(r: &ResumeModel, theme: &AppTheme) -> Div {
        Self::info_card(
            tr("vacancy_resume_section_summary"),
            vec![
                div()
                    .w_full()
                    .child(MainText::new(r.professional_summary.clone()).px(14.0).color(theme.text.primary))
                    .into_any_element(),
            ],
            theme,
        )
    }

    fn delete_button(
        r: &ResumeModel,
        is_deleting: bool,
        state: &Entity<MyResumeState>,
        nav_store: Option<Entity<NavigationStore>>,
        theme: &AppTheme,
    ) -> Div {
        let error = theme.semantic.error_100;
        let resume_id = r.id;
        let state = state.clone();
        div().w_full().px(px(16.0)).pt(px(4.0)).child(
            plain("my-resume-delete")
                .w_full()
                .disabled(is_deleting)
                .on_press(move |_, cx| Self::request_delete(resume_id, &state, nav_store.clone(), cx))
                .child(
                    hstack(8.0)
                        .w_full()
                        .justify_center()
                        .py(px(14.0))
                        .bg(error.opacity(0.1))
                        .capsule()
                        .children(IconUi::sized("trash", px(15.0), error, false))
                        .child(MainText::new(tr("vacancy_resume_delete")).px(15.0).font_weight(FontWeight::SEMIBOLD).color(error)),
                ),
        )
    }

    fn request_delete(
        resume_id: i64,
        state: &Entity<MyResumeState>,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        state.update(cx, |form, cx| {
            form.deletion_account_id = SaiAccountCacheScope::active_user_id();
            form.deletion_account_epoch = SaiAccountCacheScope::active_epoch();
            form.resume_pending_deletion = Some(resume_id);
            cx.notify();
        });
        let cancel_state = state.clone();
        let confirm_state = state.clone();
        let title = tr("vacancy_resume_delete");
        NativeConfirmAlert::present_delete(
            &title,
            "",
            Some(title.clone()),
            None,
            false,
            None,
            Some(Box::new(move |cx: &mut App| {
                cancel_state.update(cx, |form, cx| {
                    form.resume_pending_deletion = None;
                    cx.notify();
                });
            })),
            move |cx: &mut App| Self::confirm_delete_resume(resume_id, &confirm_state, nav_store, cx),
            cx,
        );
    }

    fn confirm_delete_resume(
        id: i64,
        state: &Entity<MyResumeState>,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        state.update(cx, |form, cx| {
            form.resume_pending_deletion = None;
            cx.notify();
        });
        let is_current_resume = VacancyServicesStore::my_resume(cx).is_some_and(|resume| resume.id == id);
        let is_current_account = {
            let form = state.read(cx);
            SaiAccountCacheScope::is_current_account_epoch(
                form.deletion_account_id.as_deref(),
                form.deletion_account_epoch.as_deref(),
            )
        };
        if !is_current_resume || !is_current_account {
            return;
        }

        let request_id = Uuid::new_v4();
        state.update(cx, |form, cx| {
            form.deletion_request_id = Some(request_id);
            cx.notify();
        });
        let finished_state = state.downgrade();
        VacancyInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.delete_resume(
                id,
                Some(Rc::new(move |cx: &mut App| {
                    let Some(state) = finished_state.upgrade() else {
                        return;
                    };
                    if state.read(cx).deletion_request_id != Some(request_id) {
                        return;
                    }
                    state.update(cx, |form, cx| {
                        form.deletion_request_id = None;
                        cx.notify();
                    });
                    pop_screen(nav_store.as_ref(), cx);
                })),
                cx,
            );
        });
    }

    fn empty_state(nav_store: Option<Entity<NavigationStore>>, theme: &AppTheme) -> Div {
        vstack(16.0)
            .w_full()
            .items_center()
            .pt(px(80.0))
            .px(px(32.0))
            .children(IconUi::sized("doc.text", px(48.0), theme.text.secondary.opacity(0.5), false))
            .child(
                MainText::new(tr("vacancy_resume_empty_title"))
                    .px(17.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.primary),
            )
            .child(SecondaryText::new(tr("vacancy_resume_empty_subtitle")).px(14.0).tac(TextAlignT::Center))
            .child(
                div().pt(px(6.0)).child(
                    plain("my-resume-create")
                        .on_press(move |_, cx| Self::push_create_resume(nav_store.as_ref(), cx))
                        .child(
                            div().px(px(28.0)).py(px(13.0)).bg(theme.primary._100).capsule().child(
                                MainText::new(tr("vacancy_create_resume_btn")).px(15.0).font_weight(FontWeight::SEMIBOLD).color(theme.content_on(theme.primary._100)),
                            ),
                        ),
                ),
            )
    }

    fn period_text(exp: &WorkExperienceModel) -> String {
        let end = if exp.is_current { tr("vacancy_experience_current") } else { exp.to_date.clone().unwrap_or_default() };
        [exp.from_date.clone(), end].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(PERIOD_SEPARATOR)
    }

    fn format_birth_date(raw: &str) -> String {
        let mut in_fmt = DateFormatter::new();
        in_fmt.set_locale(POSIX_LOCALE);
        in_fmt.set_date_format(WIRE_DATE_FORMAT);
        let Some(d) = in_fmt.date_from(character_prefix(raw, WIRE_DATE_LENGTH)) else {
            return raw.to_owned();
        };
        let mut out_fmt = DateFormatter::new();
        out_fmt.set_date_format(DISPLAY_DATE_FORMAT);
        out_fmt.string_from(&d)
    }
}

impl RenderOnce for MyResume {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("my-resume-state", cx, |_, cx| MyResumeState::new(cx));
        let resume = VacancyServicesStore::my_resume(cx);
        let (is_loading_resumes, is_deleting) = {
            let store = VacancyActionsStore::shared(cx);
            let store = store.read(cx);
            (store.my_resumes_sai.is_pending(), store.delete_resume_sai.is_pending())
        };
        let is_building_pdf = VacancyServicesStore::shared(cx).read(cx).is_building_resume_pdf();

        let content = if let Some(resume) = resume {
            vstack(20.0)
                .w_full()
                .py(px(16.0))
                .child(Self::header(&resume, &theme))
                .child(Self::actions(&resume, is_building_pdf, nav_store.clone(), &theme))
                .child(Self::contacts_card(&resume, &theme))
                .when(!resume.work_experience.is_empty(), |column| column.child(Self::experience_card(&resume, &theme)))
                .when(!resume.skills.is_empty(), |column| column.child(Self::skills_card(&resume, &theme)))
                .when(!resume.professional_summary.is_empty(), |column| column.child(Self::summary_card(&resume, &theme)))
                .child(Self::delete_button(&resume, is_deleting, &state, nav_store.clone(), &theme))
                .into_any_element()
        } else if is_loading_resumes || is_deleting {
            div().w_full().pt(px(80.0)).flex().justify_center().child(progress_view()).into_any_element()
        } else {
            Self::empty_state(nav_store.clone(), &theme).into_any_element()
        };

        let disappear_state = state.clone();
        SettingsWrapperUi::new(tr("vacancy_resume_my_title"), move |_, cx| pop_screen(nav_store.as_ref(), cx))
            .content(scroll_view("my-resume-scroll").list_shell_scroll_under_chrome(cx).bg(theme.bg._100).child(content))
            .on_appear("my-resume-appear", |_, cx| {
                VacancyInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_my_resumes_loaded(cx));
                GuideFacts::set(RiseGuideSignal::MyResumeOpen, true, cx);
            })
            .on_disappear("my-resume-disappear", move |_, cx| {
                disappear_state.update(cx, |form, cx| {
                    form.resume_pending_deletion = None;
                    form.deletion_request_id = None;
                    cx.notify();
                });
                GuideFacts::set(RiseGuideSignal::MyResumeOpen, false, cx);
            })
    }
}
