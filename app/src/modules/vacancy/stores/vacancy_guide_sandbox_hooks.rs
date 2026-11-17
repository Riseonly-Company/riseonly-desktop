use gpui::App;
use rise_canon::rise_request;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::{ResumeModel, VacancyModel};

pub const PRACTICE_VACANCY_ID: i64 = 9_100_000_000_000_000_010;

pub const PRACTICE_JOB_ID_SPAN: i64 = 1_000;

pub fn is_practice_job_id(id: i64) -> bool {
    id >= PRACTICE_VACANCY_ID && id < PRACTICE_VACANCY_ID + PRACTICE_JOB_ID_SPAN
}

pub struct VacancyGuideSandboxHooks {
    pub owns_jobs_scene: fn(cx: &App) -> bool,
    pub hides_real_resumes: fn(cx: &App) -> bool,
    pub practice_vacancy: fn(id: i64, cx: &App) -> Option<VacancyModel>,
    pub vacancy_list_snapshot: fn(real: Vec<VacancyModel>, filtered: bool, cx: &App) -> Vec<VacancyModel>,
    pub my_vacancies_snapshot: fn(real: Vec<VacancyModel>, cx: &App) -> Vec<VacancyModel>,
    pub resumes_snapshot: fn(real: Vec<ResumeModel>, cx: &mut App) -> Vec<ResumeModel>,
    pub adopt_practice_resume: fn(body: &rise_request::vacancy::CreateResume, cx: &mut App) -> Option<ResumeModel>,
    pub adopt_practice_vacancy: fn(body: &rise_request::vacancy::CreateVacancy, cx: &mut App) -> Option<VacancyModel>,
    pub delete_practice_vacancy: fn(id: i64, cx: &mut App) -> bool,
    pub toggle_practice_vacancy: fn(id: i64, cx: &mut App) -> bool,
    pub apply_to_practice_vacancy: fn(id: i64, cx: &mut App) -> bool,
    pub delete_practice_resume: fn(id: i64, cx: &mut App) -> bool,
}

pub(crate) static HOOKS: FeatureHookSlot<VacancyGuideSandboxHooks> = FeatureHookSlot::new("HOOKS");

static UNINSTALLED: VacancyGuideSandboxHooks = VacancyGuideSandboxHooks {
    owns_jobs_scene: |_| false,
    hides_real_resumes: |_| false,
    practice_vacancy: |_, _| None,
    vacancy_list_snapshot: |real, _, _| real,
    my_vacancies_snapshot: |real, _| real,
    resumes_snapshot: |real, _| real,
    adopt_practice_resume: |_, _| None,
    adopt_practice_vacancy: |_, _| None,
    delete_practice_vacancy: |_, _| false,
    toggle_practice_vacancy: |_, _| false,
    apply_to_practice_vacancy: |_, _| false,
    delete_practice_resume: |_, _| false,
};

pub fn set_vacancy_guide_sandbox_hooks(hooks: VacancyGuideSandboxHooks) -> Result<(), VacancyGuideSandboxHooks> {
    HOOKS.set(hooks)
}

pub fn vacancy_guide_sandbox_hooks() -> &'static VacancyGuideSandboxHooks {
    HOOKS.get().unwrap_or(&UNINSTALLED)
}
