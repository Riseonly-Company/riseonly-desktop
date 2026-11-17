use rise_canon::{rise_proto, rise_request};

use super::vacancy_types::{
    ApplicationModel, EmployerStatusModel, ResumeCurrency, ResumeModel, VacancyModel, WorkExperienceModel,
};

pub type WireVacancyAuthorDTO = rise_proto::vacancy::ProtoVacancyAuthor;

pub type WireVacancyDTO = rise_proto::vacancy::ProtoVacancyResponse;
pub type WireWorkExperienceDTO = rise_proto::vacancy::ProtoWorkExperienceItem;

pub type WireResumeDTO = rise_proto::vacancy::ProtoResumeResponse;

pub type WireApplicationDTO = rise_proto::vacancy::ProtoApplicationResponse;

pub type WireVacancyListDTO = rise_proto::vacancy::GatewayListVacanciesResponse;
pub type WireResumeListDTO = rise_proto::vacancy::GatewayListResumesResponse;
pub type WireApplicationsListDTO = rise_proto::vacancy::GatewayGetMyApplicationsResponse;

pub type WireSingleVacancyDTO = rise_proto::vacancy::GatewayVacancyResponse;
pub type WireSingleResumeDTO = rise_proto::vacancy::GatewayResumeResponse;
pub type WireSingleApplicationDTO = rise_proto::vacancy::GatewayApplyVacancyResponse;
pub type WireEmployerStatusDTO = rise_proto::vacancy::GatewayGetEmployerStatusResponse;

pub trait WireVacancyDTOExt {
    fn to_model(&self) -> VacancyModel;
    fn with(&self, active: Option<bool>, is_applied: Option<bool>) -> WireVacancyDTO;
}

impl WireVacancyDTOExt for WireVacancyDTO {
    fn to_model(&self) -> VacancyModel {
        VacancyModel {
            id: self.id,
            title: self.title.clone(),
            company: self.company.clone(),
            city: self.city.clone(),
            salary_range: self.salary_range.clone(),
            experience: self.experience.clone(),
            schedule: self.schedule.clone(),
            job_format: self.job_format.clone(),
            is_relocation_available: self.is_relocation_available,
            skills: self.skills.clone(),
            description: self.description.clone(),
            company_logo: self.company_logo.clone(),
            active: self.active,
            author_id: self.author_id.clone(),
            created_at_ms: self.created_at,
            is_applied: self.is_applied,
            moderation_status: self.moderation_status.clone(),
            review_status: self.status.clone(),
            salary_currency: ResumeCurrency::option(&self.salary_currency).code.to_owned(),
        }
    }

    fn with(&self, active: Option<bool>, is_applied: Option<bool>) -> WireVacancyDTO {
        let mut copy = self.clone();
        if let Some(active) = active {
            copy.active = active;
        }
        if let Some(is_applied) = is_applied {
            copy.is_applied = Some(is_applied);
        }
        copy
    }
}

pub trait WireResumeDTOExt {
    fn to_model(&self) -> ResumeModel;
}

impl WireResumeDTOExt for WireResumeDTO {
    fn to_model(&self) -> ResumeModel {
        ResumeModel {
            id: self.id,
            profession: self.profession.clone().unwrap_or_default(),
            first_name: self.first_name.clone().unwrap_or_default(),
            second_name: self.second_name.clone().unwrap_or_default(),
            last_name: self.last_name.clone().unwrap_or_default(),
            city: self.city.clone().unwrap_or_default(),
            experience_level: self.experience_level.clone().unwrap_or_default(),
            skills: self.skills.clone(),
            expected_salary_range: self.expected_salary_range.clone(),
            updated_at_ms: self.updated_at,
            work_experience: self.work_experience.iter().map(WireWorkExperienceDTOExt::to_model).collect(),
            professional_summary: self.professional_summary.clone().unwrap_or_default(),
            ready_to_relocation: self.ready_to_relocation,
            ready_to_army: self.ready_to_army,
            gender: self.gender.clone().unwrap_or_default(),
            birth_date: self.birth_date.clone().unwrap_or_default(),
            education: self.education.clone().unwrap_or_default(),
            phone: self.phone.clone().unwrap_or_default(),
            country: self.country.clone().unwrap_or_default(),
            salary_currency: self.salary_currency.clone().unwrap_or_default(),
            avatar: self.avatar.clone().unwrap_or_default(),
        }
    }
}

pub trait WireWorkExperienceDTOExt {
    fn to_model(&self) -> WorkExperienceModel;
}

impl WireWorkExperienceDTOExt for WireWorkExperienceDTO {
    fn to_model(&self) -> WorkExperienceModel {
        WorkExperienceModel::new(
            self.company.clone(),
            self.position.clone(),
            self.from_date.clone(),
            self.to_date.clone(),
            self.is_current,
            self.description.clone(),
            self.company_logo_url.clone(),
        )
    }
}

pub trait WireApplicationDTOExt {
    fn to_model(&self) -> ApplicationModel;
}

impl WireApplicationDTOExt for WireApplicationDTO {
    fn to_model(&self) -> ApplicationModel {
        ApplicationModel {
            id: self.id,
            vacancy_id: self.vacancy_id,
            vacancy_title: self.vacancy_title.clone(),
            company: self.company.clone(),
            status: self.status.clone(),
            created_at_ms: self.created_at,
        }
    }
}

pub trait WireEmployerStatusDTOExt {
    fn to_model(&self) -> EmployerStatusModel;
}

impl WireEmployerStatusDTOExt for WireEmployerStatusDTO {
    fn to_model(&self) -> EmployerStatusModel {
        EmployerStatusModel {
            is_employer: self.is_employer,
            company_name: self.company_name.clone(),
            status: self.status.clone(),
        }
    }
}

pub trait UpdateResumeExt {
    fn with_form(resume_id: i64, form: &rise_request::vacancy::CreateResume) -> Self;
}

impl UpdateResumeExt for rise_request::vacancy::UpdateResume {
    fn with_form(resume_id: i64, form: &rise_request::vacancy::CreateResume) -> Self {
        Self {
            avatar: form.avatar.clone(),
            birth_date: form.birth_date.clone(),
            city: form.city.clone(),
            country: form.country.clone(),
            education: form.education.clone(),
            expected_salary_range: form.expected_salary_range.clone(),
            experience_level: form.experience_level.clone(),
            first_name: form.first_name.clone(),
            gender: form.gender.clone(),
            last_name: form.last_name.clone(),
            phone: form.phone.clone(),
            profession: form.profession.clone(),
            professional_summary: form.professional_summary.clone(),
            ready_to_army: form.ready_to_army,
            ready_to_relocation: form.ready_to_relocation,
            resume_id: Some(resume_id),
            salary_currency: form.salary_currency.clone(),
            second_name: form.second_name.clone(),
            skills: form.skills.clone(),
            work_experience: form.work_experience.as_ref().map(|items| {
                items
                    .iter()
                    .map(|item| rise_request::vacancy::update_resume::WorkExperienceElement {
                        company: item.company.clone(),
                        company_logo_url: item.company_logo_url.clone(),
                        description: item.description.clone(),
                        from_date: item.from_date.clone(),
                        is_current: item.is_current,
                        position: item.position.clone(),
                        to_date: item.to_date.clone(),
                    })
                    .collect()
            }),
        }
    }
}

pub trait UpdateVacancyExt {
    fn with_form(vacancy_id: i64, form: &rise_request::vacancy::CreateVacancy) -> Self;
}

impl UpdateVacancyExt for rise_request::vacancy::UpdateVacancy {
    fn with_form(vacancy_id: i64, form: &rise_request::vacancy::CreateVacancy) -> Self {
        Self {
            vacancy_id,
            city: form.city.clone(),
            company: form.company.clone(),
            company_logo: form.company_logo.clone(),
            description: form.description.clone(),
            employment_type: form.employment_type,
            experience: form.experience.clone(),
            is_relocation_available: form.is_relocation_available,
            job_format: form.job_format.clone(),
            location: form.location.clone(),
            original_description: form.original_description.clone(),
            salary_currency: form.salary_currency.clone(),
            salary_range: form.salary_range.clone(),
            schedule: form.schedule.clone(),
            skills: form.skills.clone(),
            title: form.title.clone(),
        }
    }
}
