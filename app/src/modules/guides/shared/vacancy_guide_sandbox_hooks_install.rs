use gpui::App;
use rise_canon::rise_request;

use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::{ResumeModel, VacancyModel};
use crate::modules::vacancy::stores::vacancy_guide_sandbox_hooks::{
    VacancyGuideSandboxHooks, set_vacancy_guide_sandbox_hooks,
};

pub fn install_vacancy_guide_sandbox_hooks() {
    let _ = set_vacancy_guide_sandbox_hooks(VacancyGuideSandboxHooks {
        owns_jobs_scene,
        hides_real_resumes,
        practice_vacancy,
        vacancy_list_snapshot,
        my_vacancies_snapshot,
        resumes_snapshot,
        adopt_practice_resume,
        adopt_practice_vacancy,
        delete_practice_vacancy,
        toggle_practice_vacancy,
        apply_to_practice_vacancy,
        delete_practice_resume,
    });
}

fn owns_jobs_scene(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_jobs_scene())
}

fn hides_real_resumes(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).hides_real_resumes())
}

fn practice_vacancy(id: i64, cx: &App) -> Option<VacancyModel> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).practice_vacancy(id))
}

fn vacancy_list_snapshot(real: Vec<VacancyModel>, filtered: bool, cx: &App) -> Vec<VacancyModel> {
    match GuideSandbox::existing(cx) {
        Some(sandbox) => sandbox.read(cx).vacancy_list_snapshot(real, filtered),
        None => real,
    }
}

fn my_vacancies_snapshot(real: Vec<VacancyModel>, cx: &App) -> Vec<VacancyModel> {
    match GuideSandbox::existing(cx) {
        Some(sandbox) => sandbox.read(cx).my_vacancies_snapshot(real),
        None => real,
    }
}

fn resumes_snapshot(real: Vec<ResumeModel>, cx: &mut App) -> Vec<ResumeModel> {
    let sandbox = GuideSandbox::shared(cx);
    sandbox.update(cx, |sandbox, cx| sandbox.resumes_snapshot(real, cx))
}

fn adopt_practice_resume(body: &rise_request::vacancy::CreateResume, cx: &mut App) -> Option<ResumeModel> {
    let sandbox = GuideSandbox::shared(cx);
    Some(sandbox.update(cx, |sandbox, cx| sandbox.adopt_practice_resume(body, cx)))
}

fn adopt_practice_vacancy(body: &rise_request::vacancy::CreateVacancy, cx: &mut App) -> Option<VacancyModel> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.adopt_practice_vacancy(body, cx))
}

fn delete_practice_vacancy(id: i64, cx: &mut App) -> bool {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_practice_vacancy(id, cx))
}

fn toggle_practice_vacancy(id: i64, cx: &mut App) -> bool {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.toggle_practice_vacancy(id, cx))
}

fn apply_to_practice_vacancy(id: i64, cx: &mut App) -> bool {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.apply_to_practice_vacancy(id, cx))
}

fn delete_practice_resume(id: i64, cx: &mut App) -> bool {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_practice_resume(id, cx))
}
