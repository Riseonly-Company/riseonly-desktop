use std::collections::HashMap;
use std::sync::LazyLock;

use rise_catalog::{
    VacancyEducation, VacancyExperienceLevel, VacancyGender, VacancyJobFormat, VacancySchedule, WireEnum,
};
use rise_i18n::tr;

use crate::modules::vacancy::shared::picker_option::PickerOption;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::ResumeCurrency;

/// rows of one canon table as code and key, wire spelling and translation key in canon order
fn canon_rows<T: WireEnum>() -> Vec<(&'static str, &'static str)> {
    T::all()
        .iter()
        .filter_map(|value| value.i18n_key().map(|key| (value.wire(), key)))
        .collect()
}

fn canon_keys<T: WireEnum>() -> HashMap<&'static str, &'static str> {
    canon_rows::<T>().into_iter().collect()
}

static GENDERS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(canon_rows::<VacancyGender>);
static EDUCATIONS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(canon_rows::<VacancyEducation>);
static EXPERIENCE_LEVELS: LazyLock<Vec<(&'static str, &'static str)>> =
    LazyLock::new(canon_rows::<VacancyExperienceLevel>);
static SCHEDULE_KEYS: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(canon_keys::<VacancySchedule>);
static JOB_FORMAT_KEYS: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(canon_keys::<VacancyJobFormat>);

/// option lists of the resume and vacancy forms, built from the canon tables
pub enum ResumeOptionCatalog {}

impl ResumeOptionCatalog {
    pub fn genders() -> &'static [(&'static str, &'static str)] {
        &GENDERS
    }

    pub fn educations() -> &'static [(&'static str, &'static str)] {
        &EDUCATIONS
    }

    pub fn experience_levels() -> &'static [(&'static str, &'static str)] {
        &EXPERIENCE_LEVELS
    }

    pub fn schedule_keys() -> &'static HashMap<&'static str, &'static str> {
        &SCHEDULE_KEYS
    }

    pub fn job_format_keys() -> &'static HashMap<&'static str, &'static str> {
        &JOB_FORMAT_KEYS
    }

    pub fn schedule_title(wire: &str) -> String {
        let lookup = VacancySchedule::from_wire(wire).map_or(wire, |schedule| schedule.wire());
        Self::schedule_keys()
            .get(lookup)
            .map_or_else(|| wire.to_owned(), |key| tr(key))
    }

    pub fn job_format_title(wire: &str) -> String {
        Self::job_format_keys()
            .get(wire)
            .map_or_else(|| wire.to_owned(), |key| tr(key))
    }

    pub fn gender_key(code: &str) -> &'static str {
        Self::genders()
            .iter()
            .find(|(row_code, _)| *row_code == code)
            .map_or(VacancyGender::Male.i18n_key(), |(_, key)| *key)
    }

    pub fn education_key(code: &str) -> &'static str {
        Self::educations()
            .iter()
            .find(|(row_code, _)| *row_code == code)
            .map_or(VacancyEducation::High.i18n_key(), |(_, key)| *key)
    }

    pub fn experience_key(code: &str) -> &'static str {
        Self::experience_levels()
            .iter()
            .find(|(row_code, _)| *row_code == code)
            .map_or(VacancyExperienceLevel::From1To3.i18n_key(), |(_, key)| *key)
    }

    pub fn gender_options() -> Vec<PickerOption> {
        Self::genders()
            .iter()
            .map(|(code, key)| PickerOption::new(*code, tr(key)))
            .collect()
    }

    pub fn education_options() -> Vec<PickerOption> {
        Self::educations()
            .iter()
            .map(|(code, key)| PickerOption::new(*code, tr(key)))
            .collect()
    }

    pub fn experience_options() -> Vec<PickerOption> {
        Self::experience_levels()
            .iter()
            .map(|(code, key)| PickerOption::new(*code, tr(key)))
            .collect()
    }

    pub fn currency_options() -> Vec<PickerOption> {
        ResumeCurrency::all()
            .iter()
            .map(|option| {
                PickerOption::with_leading(
                    option.code,
                    option.symbol,
                    format!("{} ({})", tr(option.name_key), option.code),
                )
            })
            .collect()
    }
}
