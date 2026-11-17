use std::rc::Rc;

use gpui::{App, Context, Entity, Task};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::UrlComponents;
use rise_i18n::tr;
use rise_platform::open_url::open_url;
use rise_platform::system_share::ShareItem;
use rise_ui::native_confirm_alert::NativeConfirmAlert;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::widgets::sheets::share_sheet::SystemShare;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_actions::VacancyActionsStore;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::{ResumeModel, VacancyModel};
use crate::modules::vacancy::stores::vacancy_actions::work_experience_entry::WorkExperienceEntry;
use crate::modules::vacancy::stores::vacancy_services::vacancy_services::{
    VacancyOnSuccess, VacancyResumeOnSuccess, VacancyServicesStore,
};

const EMPLOYER_SITE_PATH: &str = "/employer";

pub struct VacancyInteractionsStore {}

impl VacancyInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self {})
    }

    fn actions(cx: &mut App) -> Entity<VacancyActionsStore> {
        VacancyActionsStore::shared(cx)
    }

    fn wire_user_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .filter(|user_id| !user_id.is_empty())
    }

    fn can_request_more(cache_key: &str, cx: &mut App) -> bool {
        SaiWs::pagination_state(cache_key, cx).read(cx).trailing().can_request_automatically()
    }

    pub fn delete_vacancy(
        &mut self,
        id: i64,
        title: String,
        can_proceed: Rc<dyn Fn(&mut App) -> bool>,
        on_cancel: Rc<dyn Fn(&mut App)>,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if id <= 0 || Self::actions(cx).read(cx).delete_vacancy_sai.is_pending() {
            return;
        }
        let (Some(user_id), Some(account_epoch)) =
            (SaiAccountCacheScope::active_user_id(), SaiAccountCacheScope::active_epoch())
        else {
            return;
        };
        if !can_proceed(cx) {
            return;
        }

        NativeConfirmAlert::present_delete(
            &tr("vacancy_delete"),
            &title,
            Some(tr("vacancy_delete")),
            None,
            false,
            None,
            Some(Box::new(move |cx: &mut App| on_cancel(cx))),
            move |cx: &mut App| {
                if !SaiAccountCacheScope::is_current_account_epoch(Some(&user_id), Some(&account_epoch)) {
                    return;
                }
                let actions = Self::actions(cx);
                if actions.read(cx).delete_vacancy_sai.is_pending() {
                    return;
                }
                actions.update(cx, |actions, cx| actions.delete_vacancy_action(id, on_success, cx));
            },
            cx,
        );
    }

    pub fn toggle_vacancy_status(&mut self, id: i64, cx: &mut Context<Self>) {
        if id <= 0 {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.toggle_vacancy_status_action(id, cx));
    }

    pub fn ensure_vacancy_list_loaded(&mut self, cx: &mut Context<Self>) {
        let has_list = !VacancyServicesStore::vacancy_list(None, None, None, None, None, None, None, cx)
            .is_empty();
        if has_list {
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).list_vacancies_sai.is_pending() {
            return;
        }

        actions.update(cx, |actions, cx| {
            actions.list_vacancies_action(None, None, None, None, None, None, None, None, cx);
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn refresh_vacancy_list(
        &mut self,
        q: Option<String>,
        schedule: Option<String>,
        job_format: Option<String>,
        min_salary: Option<i64>,
        max_salary: Option<i64>,
        city: Option<String>,
        is_relocation: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.list_vacancies_action(
                q,
                schedule,
                job_format,
                min_salary,
                max_salary,
                city,
                is_relocation,
                None,
                cx,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn load_more_vacancies(
        &mut self,
        q: Option<String>,
        schedule: Option<String>,
        job_format: Option<String>,
        min_salary: Option<i64>,
        max_salary: Option<i64>,
        city: Option<String>,
        is_relocation: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let cache_key = VacancyServicesStore::vacancy_list_cache_key(
            q.as_deref(),
            schedule.as_deref(),
            job_format.as_deref(),
            min_salary,
            max_salary,
            city.as_deref(),
            is_relocation,
            user_id.as_deref(),
        );
        let can_request = Self::can_request_more(&cache_key, cx);

        let has_more = VacancyServicesStore::vacancy_list_has_more(
            q.as_deref(),
            schedule.as_deref(),
            job_format.as_deref(),
            min_salary,
            max_salary,
            city.as_deref(),
            is_relocation,
            cx,
        );
        let relative_id = VacancyServicesStore::vacancy_list_relative_id(
            q.as_deref(),
            schedule.as_deref(),
            job_format.as_deref(),
            min_salary,
            max_salary,
            city.as_deref(),
            is_relocation,
            cx,
        );
        let Some(relative_id) = relative_id.filter(|_| has_more && can_request) else {
            return;
        };

        Self::actions(cx).update(cx, |actions, cx| {
            actions.list_vacancies_action(
                q,
                schedule,
                job_format,
                min_salary,
                max_salary,
                city,
                is_relocation,
                Some(relative_id),
                cx,
            );
        });
    }

    pub fn ensure_vacancy_loaded(&mut self, id: i64, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        if actions.read(cx).get_vacancy_sai.is_pending() {
            return;
        }

        actions.update(cx, |actions, cx| actions.get_vacancy_action(id, cx));
    }

    pub fn ensure_my_vacancies_loaded(&mut self, cx: &mut Context<Self>) {
        if !VacancyServicesStore::my_vacancies(None, cx).is_empty() {
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).my_vacancies_sai.is_pending() {
            return;
        }

        actions.update(cx, |actions, cx| actions.get_my_vacancies_action(None, None, cx));
    }

    pub fn refresh_my_vacancies(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_my_vacancies_action(None, None, cx));
    }

    pub fn load_more_my_vacancies(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        let cache_key = VacancyServicesStore::my_vacancies_cache_key(&user_id, None);
        let can_request = Self::can_request_more(&cache_key, cx);

        let has_more = VacancyServicesStore::my_vacancies_has_more(None, cx);
        let relative_id = VacancyServicesStore::my_vacancies_relative_id(None, cx);
        let Some(relative_id) = relative_id.filter(|_| has_more && can_request) else {
            return;
        };

        Self::actions(cx).update(cx, |actions, cx| actions.get_my_vacancies_action(None, Some(relative_id), cx));
    }

    pub fn ensure_my_resumes_loaded(&mut self, cx: &mut Context<Self>) {
        if !VacancyServicesStore::my_resumes(cx).is_empty() {
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).my_resumes_sai.is_pending() {
            return;
        }

        actions.update(cx, |actions, cx| actions.get_my_resumes_action(None, cx));
    }

    pub fn refresh_my_resumes(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_my_resumes_action(None, cx));
    }

    pub fn delete_resume(&mut self, id: i64, on_success: Option<Rc<dyn Fn(&mut App)>>, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        if actions.read(cx).delete_resume_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.delete_resume_action(id, on_success, cx));
    }

    pub fn load_more_my_resumes(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        let cache_key = VacancyServicesStore::my_resumes_cache_key(&user_id);
        let can_request = Self::can_request_more(&cache_key, cx);

        let has_more = VacancyServicesStore::my_resumes_has_more(cx);
        let relative_id = VacancyServicesStore::my_resumes_relative_id(cx);
        let Some(relative_id) = relative_id.filter(|_| has_more && can_request) else {
            return;
        };

        Self::actions(cx).update(cx, |actions, cx| actions.get_my_resumes_action(Some(relative_id), cx));
    }

    pub fn ensure_my_applications_loaded(&mut self, cx: &mut Context<Self>) {
        if !VacancyServicesStore::my_applications(cx).is_empty() {
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).my_applications_sai.is_pending() {
            return;
        }

        actions.update(cx, |actions, cx| actions.get_my_applications_action(None, cx));
    }

    pub fn load_more_my_applications(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return;
        };

        let cache_key = VacancyServicesStore::my_applications_cache_key(&user_id);
        let can_request = Self::can_request_more(&cache_key, cx);

        let has_more = VacancyServicesStore::my_applications_has_more(cx);
        let relative_id = VacancyServicesStore::my_applications_relative_id(cx);
        let Some(relative_id) = relative_id.filter(|_| has_more && can_request) else {
            return;
        };

        Self::actions(cx).update(cx, |actions, cx| actions.get_my_applications_action(Some(relative_id), cx));
    }

    pub fn ensure_employer_status_loaded(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = &actions.read(cx).employer_status_sai;
        if slice.data().is_some() && slice.status() == SaiWsSliceStatus::Fulfilled {
            return;
        }
        if slice.is_pending() {
            return;
        }

        actions.update(cx, |actions, cx| actions.get_employer_status_action(cx));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit_create_resume(
        &mut self,
        resume_id: Option<i64>,
        first_name: String,
        second_name: String,
        phone: String,
        city: String,
        country: String,
        profession: String,
        gender: String,
        birth_date: String,
        education: String,
        experience_level: String,
        skills: Vec<String>,
        experiences: Vec<WorkExperienceEntry>,
        summary: String,
        salary_from: String,
        salary_to: String,
        salary_currency: String,
        ready_to_relocation: bool,
        ready_to_army: bool,
        on_success: VacancyResumeOnSuccess,
        cx: &mut Context<Self>,
    ) {
        VacancyServicesStore::submit_create_resume(
            resume_id,
            &first_name,
            &second_name,
            &phone,
            &city,
            &country,
            &profession,
            &gender,
            &birth_date,
            &education,
            &experience_level,
            &skills,
            &experiences,
            &summary,
            &salary_from,
            &salary_to,
            &salary_currency,
            ready_to_relocation,
            ready_to_army,
            on_success,
            cx,
        );
    }

    pub fn share_resume(&mut self, model: ResumeModel, cx: &mut Context<Self>) {
        VacancyServicesStore::share_resume(model, None, cx);
    }

    pub fn share_vacancy(&mut self, model: VacancyModel, cx: &mut Context<Self>) {
        if model.id <= 0 {
            return;
        }
        let parts = [
            model.title.clone(),
            model.company.clone(),
            model.city.clone(),
            model.formatted_salary(),
            model.schedule_display(),
            model.job_format_display(),
            model.experience_display(),
            model.skills.join(", "),
            model.description.clone(),
        ];
        let text = parts
            .iter()
            .map(|part| trimming_whitespaces_and_newlines(part))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        if text.is_empty() {
            return;
        }
        SystemShare::present(vec![ShareItem::Text(text)], None, cx);
    }

    pub fn upload_resume_logo(
        &mut self,
        file: SaiWsOptimisticLocalFile,
        on_progress: Option<Rc<dyn Fn(f64, &mut App)>>,
        cx: &mut Context<Self>,
    ) -> Task<Option<String>> {
        VacancyServicesStore::upload_resume_logo(file, on_progress, cx)
    }

    pub fn my_resume(&mut self, cx: &mut Context<Self>) -> Option<ResumeModel> {
        VacancyServicesStore::my_resume(cx)
    }

    /// salary input kept as a whole number, a hardware keyboard can type past the number pad
    pub fn salary_input(&self, raw: &str) -> String {
        raw.chars().filter(char::is_ascii_digit).collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit_create_vacancy(
        &mut self,
        title: String,
        company: String,
        city: String,
        salary_from: String,
        salary_to: String,
        salary_currency: String,
        schedule: String,
        job_format: String,
        description: String,
        skills: Vec<String>,
        is_relocation: bool,
        editing_vacancy_id: Option<i64>,
        on_success: VacancyOnSuccess,
        cx: &mut Context<Self>,
    ) {
        VacancyServicesStore::submit_create_vacancy(
            &title,
            &company,
            &city,
            &salary_from,
            &salary_to,
            &salary_currency,
            &schedule,
            &job_format,
            &description,
            &skills,
            is_relocation,
            editing_vacancy_id,
            on_success,
            cx,
        );
    }

    pub fn open_employer_application_site(&mut self, cx: &mut Context<Self>) {
        let site = AppEnvironment::current().public_site_url();
        let Some(mut components) = UrlComponents::from_url(&site, false) else {
            return;
        };
        components.set_path(EMPLOYER_SITE_PATH);
        let Some(url) = components.url() else {
            return;
        };

        cx.defer(move |cx| {
            open_url(&url, cx);
        });
    }
}
