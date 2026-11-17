use std::collections::HashSet;
use std::sync::LazyLock;

use gpui::{App, Context};
use rise_canon::rise_wire_methods::{report, vacancy};
use rise_canon::{RiseGuideSignal, rise_request};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::tr;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::{
    ResumeCurrency, ResumeModel, VacancyModel, WorkExperienceModel,
};
use crate::modules::vacancy::stores::vacancy_guide_sandbox_hooks::{PRACTICE_JOB_ID_SPAN, PRACTICE_VACANCY_ID};
use crate::modules::vacancy::stores::vacancy_services::vacancy_services::VacancyServicesStore;

/// the jobs lessons on the real jobs screens
/// resume lesson sets real resumes aside so everyone starts at create resume
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideSandboxJobsScene {
    pub hides_real_resumes: bool,
    pub resume: Option<ResumeModel>,
    pub applied: bool,
    pub own_vacancies: Vec<VacancyModel>,
}

impl GuideSandboxJobsScene {
    pub fn new(hides_real_resumes: bool) -> Self {
        Self {
            hides_real_resumes,
            resume: None,
            applied: false,
            own_vacancies: Vec::new(),
        }
    }
}

static JOBS_REFUSED_METHODS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    [
        vacancy::CREATE_RESUME,
        vacancy::DELETE_RESUME,
        vacancy::APPLY_VACANCY,
        vacancy::CREATE_VACANCY,
        vacancy::UPDATE_VACANCY,
        vacancy::DELETE_VACANCY,
        vacancy::TOGGLE_VACANCY_STATUS,
    ]
    .into_iter()
    .map(|method| format!("{}.{method}", vacancy::SERVICE))
    .chain(std::iter::once(format!("{}.{}", report::SERVICE, report::CREATE_REPORT)))
    .collect()
});

impl GuideSandbox {
    pub const PRACTICE_VACANCY_ID: i64 = PRACTICE_VACANCY_ID;
    pub const PRACTICE_RESUME_ID: i64 = 9_100_000_000_000_000_011;
    pub const PRACTICE_OWN_VACANCY_FIRST_ID: i64 = 9_100_000_000_000_000_020;

    pub fn is_practice_job_id(id: i64) -> bool {
        id >= Self::PRACTICE_VACANCY_ID && id < Self::PRACTICE_VACANCY_ID + PRACTICE_JOB_ID_SPAN
    }

    pub fn arm_jobs(&mut self, cx: &mut Context<Self>) {
        let resume_lesson = self.scenes.contains(&Self::resume_scene_id());
        if !resume_lesson && !self.scenes.contains(&Self::jobs_scene_id()) {
            return;
        }
        self.jobs_scene = Some(GuideSandboxJobsScene::new(resume_lesson));
        Self::refuse_methods(&JOBS_REFUSED_METHODS);
        cx.notify();
    }

    pub fn withdraw_jobs(&mut self, cx: &mut Context<Self>) {
        if self.jobs_scene.is_none() {
            return;
        }
        self.jobs_scene = None;
        cx.notify();
        GuideFacts::set(RiseGuideSignal::ResumeFormReady, false, cx);
        GuideFacts::set(RiseGuideSignal::VacancyFormReady, false, cx);
        GuideFacts::set(RiseGuideSignal::VacancyApplyOpen, false, cx);
        GuideFacts::set(RiseGuideSignal::VacancyApplyResumeChosen, false, cx);
        GuideFacts::set(RiseGuideSignal::MyResumeOpen, false, cx);
    }

    pub fn owns_jobs_scene(&self) -> bool {
        self.is_armed && self.jobs_scene.is_some()
    }

    /// what the resume screens list
    /// resume lesson shows only the one filled in
    /// vacancies lesson shows the reader's own or one from the profile
    pub fn resumes_snapshot(&self, real: Vec<ResumeModel>, cx: &mut App) -> Vec<ResumeModel> {
        let Some(scene) = self.jobs_scene.as_ref().filter(|_| self.owns_jobs_scene()) else {
            return real;
        };
        if scene.hides_real_resumes {
            return scene.resume.iter().cloned().collect();
        }
        if let Some(resume) = &scene.resume {
            return vec![resume.clone()];
        }
        if real.is_empty() { vec![self.profile_resume(cx)] } else { real }
    }

    pub fn hides_real_resumes(&self) -> bool {
        self.owns_jobs_scene() && self.jobs_scene.as_ref().is_some_and(|scene| scene.hides_real_resumes)
    }

    pub fn adopt_practice_resume(
        &mut self,
        body: &rise_request::vacancy::CreateResume,
        cx: &mut Context<Self>,
    ) -> ResumeModel {
        let resume = ResumeModel {
            id: Self::PRACTICE_RESUME_ID,
            profession: body.profession.clone().unwrap_or_default(),
            first_name: body.first_name.clone().unwrap_or_default(),
            second_name: body.second_name.clone().unwrap_or_default(),
            last_name: String::new(),
            city: body.city.clone().unwrap_or_default(),
            experience_level: body.experience_level.clone().unwrap_or_default(),
            skills: body.skills.clone().unwrap_or_default(),
            expected_salary_range: body.expected_salary_range.clone().unwrap_or_default(),
            updated_at_ms: now_epoch_ms(),
            work_experience: body
                .work_experience
                .iter()
                .flatten()
                .map(|entry| {
                    WorkExperienceModel::new(
                        entry.company.clone().unwrap_or_default(),
                        entry.position.clone().unwrap_or_default(),
                        entry.from_date.clone().unwrap_or_default(),
                        entry.to_date.clone(),
                        entry.is_current.unwrap_or(false),
                        entry.description.clone().unwrap_or_default(),
                        entry.company_logo_url.clone(),
                    )
                })
                .collect(),
            professional_summary: body.professional_summary.clone().unwrap_or_default(),
            ready_to_relocation: body.ready_to_relocation.unwrap_or(false),
            ready_to_army: body.ready_to_army.unwrap_or(false),
            gender: body.gender.clone().unwrap_or_default(),
            birth_date: body.birth_date.clone().unwrap_or_default(),
            education: body.education.clone().unwrap_or_default(),
            phone: body.phone.clone().unwrap_or_default(),
            country: body.country.clone().unwrap_or_default(),
            salary_currency: body.salary_currency.clone().unwrap_or_default(),
            avatar: body.avatar.clone().unwrap_or_default(),
        };
        if let Some(scene) = self.jobs_scene.as_mut() {
            scene.resume = Some(resume.clone());
            cx.notify();
        }
        GuideSignalBus::emit(RiseGuideSignal::ResumeSaved);
        resume
    }

    /// deleting the practice resume only removes it from the computer
    pub fn delete_practice_resume(&mut self, id: i64, cx: &mut Context<Self>) -> bool {
        if !self.owns_jobs_scene() || id != Self::PRACTICE_RESUME_ID {
            return false;
        }
        if let Some(scene) = self.jobs_scene.as_mut() {
            scene.resume = None;
            cx.notify();
        }
        true
    }

    fn profile_resume(&self, cx: &mut App) -> ResumeModel {
        let (name, avatar) = {
            let auth = AuthServicesStore::shared(cx);
            let profile = auth.read(cx).profile();
            (
                profile.map(|profile| profile.name.clone()).unwrap_or_default(),
                profile.and_then(|profile| profile.avatar_url.clone()).unwrap_or_default(),
            )
        };
        let (first, second) = VacancyServicesStore::shared(cx).read(cx).split_name(&name);
        ResumeModel {
            id: Self::PRACTICE_RESUME_ID,
            profession: tr("guide_scene_resume_profession"),
            second_name: if second == first { String::new() } else { second },
            first_name: first,
            last_name: String::new(),
            city: String::new(),
            experience_level: "1-3".to_owned(),
            skills: Vec::new(),
            expected_salary_range: Vec::new(),
            updated_at_ms: (Self::scene_stamp() * 1000.0) as i64,
            work_experience: Vec::new(),
            professional_summary: String::new(),
            ready_to_relocation: false,
            ready_to_army: false,
            gender: String::new(),
            birth_date: String::new(),
            education: String::new(),
            phone: String::new(),
            country: String::new(),
            salary_currency: String::new(),
            avatar,
        }
    }

    pub fn practice_vacancy(&self, id: i64) -> Option<VacancyModel> {
        let scene = self.jobs_scene.as_ref().filter(|_| self.owns_jobs_scene())?;
        if id == Self::PRACTICE_VACANCY_ID && !scene.hides_real_resumes {
            return Some(self.panda_vacancy(scene.applied));
        }
        scene.own_vacancies.iter().find(|vacancy| vacancy.id == id).cloned()
    }

    /// panda's vacancy heads the unfiltered list, search or filter shows real results only
    pub fn vacancy_list_snapshot(&self, real: Vec<VacancyModel>, filtered: bool) -> Vec<VacancyModel> {
        let Some(scene) = self.jobs_scene.as_ref().filter(|_| self.owns_jobs_scene()) else {
            return real;
        };
        if scene.hides_real_resumes || filtered {
            return real;
        }
        let mut list = vec![self.panda_vacancy(scene.applied)];
        list.extend(real);
        list
    }

    pub fn apply_to_practice_vacancy(&mut self, id: i64, cx: &mut Context<Self>) -> bool {
        if !self.owns_jobs_scene() || id != Self::PRACTICE_VACANCY_ID {
            return false;
        }
        if let Some(scene) = self.jobs_scene.as_mut() {
            scene.applied = true;
            cx.notify();
        }
        GuideFacts::set(RiseGuideSignal::VacancyApplyOpen, false, cx);
        GuideSignalBus::emit(RiseGuideSignal::VacancyApplied);
        true
    }

    fn panda_vacancy(&self, applied: bool) -> VacancyModel {
        VacancyModel {
            id: Self::PRACTICE_VACANCY_ID,
            title: tr("guide_scene_vacancy_title"),
            company: tr("guide_scene_vacancy_company"),
            city: tr("vacancy_format_remote"),
            salary_range: Vec::new(),
            experience: Vec::new(),
            schedule: tr("vacancy_schedule_flexible"),
            job_format: tr("vacancy_format_remote"),
            is_relocation_available: false,
            skills: vec![tr("guide_scene_vacancy_skill")],
            description: tr("guide_scene_vacancy_text"),
            company_logo: String::new(),
            active: true,
            author_id: Self::panda_user_id(),
            created_at_ms: (Self::scene_stamp() * 1000.0) as i64,
            is_applied: Some(applied),
            salary_currency: ResumeCurrency::DEFAULT_CODE.to_owned(),
            moderation_status: VacancyModel::default_moderation_status(),
            review_status: VacancyModel::default_review_status(),
        }
    }

    pub fn my_vacancies_snapshot(&self, real: Vec<VacancyModel>) -> Vec<VacancyModel> {
        let Some(scene) = self.jobs_scene.as_ref().filter(|_| self.owns_jobs_scene()) else {
            return real;
        };
        let mut list = scene.own_vacancies.clone();
        list.extend(real);
        list
    }

    pub fn adopt_practice_vacancy(
        &mut self,
        body: &rise_request::vacancy::CreateVacancy,
        cx: &mut Context<Self>,
    ) -> Option<VacancyModel> {
        if !self.owns_jobs_scene() {
            return None;
        }
        let author_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        let scene = self.jobs_scene.as_mut()?;
        let vacancy = VacancyModel {
            id: Self::PRACTICE_OWN_VACANCY_FIRST_ID + scene.own_vacancies.len() as i64,
            title: body.title.clone().unwrap_or_default(),
            company: body.company.clone().unwrap_or_default(),
            city: body.city.clone().unwrap_or_default(),
            salary_range: body.salary_range.clone().unwrap_or_default(),
            experience: body.experience.clone().unwrap_or_default(),
            schedule: body.schedule.clone().unwrap_or_default(),
            job_format: body.job_format.clone().unwrap_or_default(),
            is_relocation_available: body.is_relocation_available.unwrap_or(false),
            skills: body.skills.clone().unwrap_or_default(),
            description: body.description.clone().unwrap_or_default(),
            company_logo: String::new(),
            active: true,
            author_id,
            created_at_ms: now_epoch_ms(),
            is_applied: None,
            salary_currency: ResumeCurrency::option(body.salary_currency.as_deref().unwrap_or_default()).code.to_owned(),
            moderation_status: VacancyModel::default_moderation_status(),
            review_status: VacancyModel::default_review_status(),
        };
        scene.own_vacancies.insert(0, vacancy.clone());
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::VacancyPublished);
        Some(vacancy)
    }

    /// pausing and deleting a practice vacancy happen on the computer
    pub fn toggle_practice_vacancy(&mut self, id: i64, cx: &mut Context<Self>) -> bool {
        if !self.owns_jobs_scene() {
            return false;
        }
        let Some(vacancy) = self
            .jobs_scene
            .as_mut()
            .and_then(|scene| scene.own_vacancies.iter_mut().find(|vacancy| vacancy.id == id))
        else {
            return false;
        };
        vacancy.active = !vacancy.active;
        cx.notify();
        true
    }

    pub fn delete_practice_vacancy(&mut self, id: i64, cx: &mut Context<Self>) -> bool {
        if !self.owns_jobs_scene() {
            return false;
        }
        let Some(scene) = self.jobs_scene.as_mut() else {
            return false;
        };
        if !scene.own_vacancies.iter().any(|vacancy| vacancy.id == id) {
            return false;
        }
        scene.own_vacancies.retain(|vacancy| vacancy.id != id);
        cx.notify();
        true
    }
}

fn now_epoch_ms() -> i64 {
    (Date::now().time_interval_since_1970() * 1000.0) as i64
}
