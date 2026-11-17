use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{App, Context, Entity, Task};
use rise_canon::rise_key::vacancy as key;
use rise_canon::rise_request::{self, vacancy::{CreateResume, CreateVacancy, create_resume}};
use rise_canon::{RiseLimits, rise_wire_methods};
use rise_catalog::{VacancyExperienceLevel, VacancyJobFormat, VacancySchedule};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_file_name::host_file_name;
use rise_core::foundation::foundation_file_url::PathFlavor;
use rise_core::foundation::foundation_text::{
    character_count, character_prefix, trimming_whitespaces, trimming_whitespaces_and_newlines,
};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_errors::ErrorCategory;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::date_format::iso8601_date_formatter::{ISO8601DateFormatter, ISO8601DateFormatterOptions};
use rise_i18n::tr;
use rise_platform::file_dialog::{self, FileFilter, SaveOutcome, SaveRequest};
use rise_platform::file_manager::FileManager;
use rise_platform::paths::{downloads_directory, temporary_directory};
use rise_platform::system_share::ShareItem;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_chunked_file_upload::{SaiWsChunkedFileUpload, SaiWsUploadOnProgress};
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::share_sheet::SystemShare;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::music::shared::utils::music::music_cover_upload::MusicCoverUpload;
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::vacancy::shared::resume_pdf_builder::ResumePdfBuilder;
use crate::modules::vacancy::shared::vacancy_mocks::VacancyLocalState;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_actions::VacancyActionsStore;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::{ApplicationModel, ResumeCurrency, ResumeModel, VacancyModel};
use crate::modules::vacancy::stores::vacancy_actions::vacancy_wire_types::{
    UpdateResumeExt, UpdateVacancyExt, WireApplicationDTOExt, WireApplicationsListDTO, WireResumeDTO, WireResumeDTOExt,
    WireResumeListDTO, WireSingleVacancyDTO, WireVacancyDTO, WireVacancyDTOExt, WireVacancyListDTO,
};
use crate::modules::vacancy::stores::vacancy_actions::work_experience_entry::WorkExperienceEntry;
use crate::modules::vacancy::stores::vacancy_guide_sandbox_hooks::vacancy_guide_sandbox_hooks;

const VACANCY_LOG_NAME: &str = "Vacancy";
const RESUME_LOGO_FILE_CONTEXT: &str = "post";
const WIRE_DATE_FORMAT: &str = "yyyy-MM-dd";
const POSIX_LOCALE: &str = "en_US_POSIX";
const PDF_EXTENSION: &str = "pdf";
const VACANCY_EMPLOYMENT_TYPE: i64 = 1;
pub const DEFAULT_VACANCY_EXPERIENCE: [i64; 2] = [0, 3];

/// called once with the resume the server or the lesson settled on
pub type VacancyResumeOnSuccess = Box<dyn FnOnce(ResumeModel, &mut App)>;
/// called once with the vacancy the server or the lesson settled on
pub type VacancyOnSuccess = Box<dyn FnOnce(VacancyModel, &mut App)>;

pub struct VacancyServicesStore {
    known_list_cache_keys: HashSet<String>,
    gone_vacancy_ids: HashSet<i64>,
    is_submitting_vacancy: bool,
    is_submitting_resume: bool,
    resume_update_id: Option<Uuid>,
    is_building_resume_pdf: bool,
}

impl VacancyServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    pub fn try_read(cx: &App) -> Option<&Self> {
        rise_ui::shared::try_read::<Self>(cx)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            known_list_cache_keys: HashSet::new(),
            gone_vacancy_ids: HashSet::new(),
            is_submitting_vacancy: false,
            is_submitting_resume: false,
            resume_update_id: None,
            is_building_resume_pdf: false,
        }
    }

    pub fn list_limit() -> i64 {
        RiseLimits::client_value("vacancy_services_store_list_limit")
    }

    pub fn gone_vacancy_ids(&self) -> &HashSet<i64> {
        &self.gone_vacancy_ids
    }

    pub fn is_submitting_vacancy(&self) -> bool {
        self.is_submitting_vacancy
    }

    pub fn is_submitting_resume(&self) -> bool {
        self.is_submitting_resume
    }

    pub fn is_building_resume_pdf(&self) -> bool {
        self.is_building_resume_pdf
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.known_list_cache_keys.clear();
        self.gone_vacancy_ids.clear();
        self.is_building_resume_pdf = false;
        self.is_submitting_vacancy = false;
        self.is_submitting_resume = false;
        self.resume_update_id = None;
        cx.notify();
    }

    pub fn note_list_cache_key(key: String, cx: &mut App) {
        Self::shared(cx).update(cx, |this, _cx| {
            this.known_list_cache_keys.insert(key);
        });
    }

    pub fn normalized_list_filter(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        if value.is_empty() {
            return None;
        }
        Some(value.to_owned())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn vacancy_list_cache_key(
        q: Option<&str>,
        schedule: Option<&str>,
        job_format: Option<&str>,
        min_salary: Option<i64>,
        max_salary: Option<i64>,
        city: Option<&str>,
        is_relocation: Option<bool>,
        user_id: Option<&str>,
    ) -> String {
        SaiCache::new([
            "vacancyList".to_owned(),
            user_id.unwrap_or_default().to_owned(),
            Self::normalized_list_filter(q).unwrap_or_default(),
            Self::normalized_list_filter(schedule).unwrap_or_default(),
            Self::normalized_list_filter(job_format).unwrap_or_default(),
            min_salary.map(|value| value.to_string()).unwrap_or_default(),
            max_salary.map(|value| value.to_string()).unwrap_or_default(),
            Self::normalized_list_filter(city).unwrap_or_default(),
            is_relocation.map(|value| if value { "true" } else { "false" }).unwrap_or_default().to_owned(),
        ])
        .key
    }

    pub fn vacancy_cache_key(vacancy_id: i64) -> String {
        SaiCache::new(["vacancy".to_owned(), vacancy_id.to_string()]).key
    }

    pub fn my_vacancies_cache_key(user_id: &str, status: Option<&str>) -> String {
        SaiCache::new(["myVacancies", user_id, status.unwrap_or_default()]).key
    }

    pub fn my_applications_cache_key(user_id: &str) -> String {
        SaiCache::new(["myApplications", user_id]).key
    }

    pub fn my_resumes_cache_key(user_id: &str) -> String {
        SaiCache::new(["myResumes", user_id]).key
    }

    pub fn employer_status_cache_key(user_id: &str) -> String {
        SaiCache::new(["employerStatus", user_id]).key
    }

    fn effective_user_id_for_wire(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()
    }

    fn non_empty_user_id(cx: &mut App) -> Option<String> {
        Self::effective_user_id_for_wire(cx).filter(|user_id| !user_id.is_empty())
    }

    fn default_vacancy_list_cache_key(cx: &mut App) -> String {
        let user_id = Self::effective_user_id_for_wire(cx);
        Self::vacancy_list_cache_key(None, None, None, None, None, None, None, user_id.as_deref())
    }

    fn cached_dto<T: DeserializeOwned>(cache_key: &str, cx: &mut App) -> Option<T> {
        let payload = SaiWsResponseCache::observed_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )?;
        SaiJSONCodec::decode::<T>(Some(&payload))
    }

    #[allow(clippy::too_many_arguments)]
    fn cached_vacancy_list_dto(
        q: Option<&str>,
        schedule: Option<&str>,
        job_format: Option<&str>,
        min_salary: Option<i64>,
        max_salary: Option<i64>,
        city: Option<&str>,
        is_relocation: Option<bool>,
        cx: &mut App,
    ) -> Option<WireVacancyListDTO> {
        let user_id = Self::effective_user_id_for_wire(cx);
        let key = Self::vacancy_list_cache_key(
            q,
            schedule,
            job_format,
            min_salary,
            max_salary,
            city,
            is_relocation,
            user_id.as_deref(),
        );
        Self::cached_dto::<WireVacancyListDTO>(&key, cx)
    }

    fn cached_my_vacancies_dto(status: Option<&str>, cx: &mut App) -> Option<WireVacancyListDTO> {
        let user_id = Self::non_empty_user_id(cx)?;
        Self::cached_dto::<WireVacancyListDTO>(&Self::my_vacancies_cache_key(&user_id, status), cx)
    }

    fn cached_my_applications_dto(cx: &mut App) -> Option<WireApplicationsListDTO> {
        let user_id = Self::non_empty_user_id(cx)?;
        Self::cached_dto::<WireApplicationsListDTO>(&Self::my_applications_cache_key(&user_id), cx)
    }

    fn cached_my_resumes_dto(cx: &mut App) -> Option<WireResumeListDTO> {
        let user_id = Self::non_empty_user_id(cx)?;
        Self::cached_dto::<WireResumeListDTO>(&Self::my_resumes_cache_key(&user_id), cx)
    }

    fn cached_single_vacancy_dto(vacancy_id: i64, observed: bool, cx: &mut App) -> Option<WireSingleVacancyDTO> {
        let key = Self::vacancy_cache_key(vacancy_id);
        let payload = if observed {
            SaiWsResponseCache::observed_payload(
                &key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
        } else {
            SaiWsResponseCache::current_payload(
                &key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
        }?;
        SaiJSONCodec::decode::<WireSingleVacancyDTO>(Some(&payload))
    }

    fn cached_vacancy_dto(vacancy_id: i64, cx: &mut App) -> Option<WireVacancyDTO> {
        Self::cached_single_vacancy_dto(vacancy_id, false, cx)?.vacancy
    }

    #[allow(clippy::too_many_arguments)]
    pub fn vacancy_list(
        q: Option<&str>,
        schedule: Option<&str>,
        job_format: Option<&str>,
        min_salary: Option<i64>,
        max_salary: Option<i64>,
        city: Option<&str>,
        is_relocation: Option<bool>,
        cx: &mut App,
    ) -> Vec<VacancyModel> {
        let real: Vec<VacancyModel> =
            Self::cached_vacancy_list_dto(q, schedule, job_format, min_salary, max_salary, city, is_relocation, cx)
                .map(|dto| dto.list.iter().map(WireVacancyDTOExt::to_model).collect())
                .unwrap_or_default();
        let filtered = [q, schedule, job_format, city]
            .into_iter()
            .any(|value| Self::normalized_list_filter(value).is_some())
            || min_salary.is_some()
            || max_salary.is_some()
            || is_relocation.is_some();
        (vacancy_guide_sandbox_hooks().vacancy_list_snapshot)(real, filtered, cx)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn vacancy_list_has_more(
        q: Option<&str>,
        schedule: Option<&str>,
        job_format: Option<&str>,
        min_salary: Option<i64>,
        max_salary: Option<i64>,
        city: Option<&str>,
        is_relocation: Option<bool>,
        cx: &mut App,
    ) -> bool {
        Self::cached_vacancy_list_dto(q, schedule, job_format, min_salary, max_salary, city, is_relocation, cx)
            .is_some_and(|dto| dto.is_have_more)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn vacancy_list_relative_id(
        q: Option<&str>,
        schedule: Option<&str>,
        job_format: Option<&str>,
        min_salary: Option<i64>,
        max_salary: Option<i64>,
        city: Option<&str>,
        is_relocation: Option<bool>,
        cx: &mut App,
    ) -> Option<i64> {
        Self::cached_vacancy_list_dto(q, schedule, job_format, min_salary, max_salary, city, is_relocation, cx)?
            .relative_id
    }

    pub fn my_vacancies(status: Option<&str>, cx: &mut App) -> Vec<VacancyModel> {
        let real: Vec<VacancyModel> = Self::cached_my_vacancies_dto(status, cx)
            .map(|dto| dto.list.iter().map(WireVacancyDTOExt::to_model).collect())
            .unwrap_or_default();
        if status.is_none() {
            (vacancy_guide_sandbox_hooks().my_vacancies_snapshot)(real, cx)
        } else {
            real
        }
    }

    pub fn my_vacancies_has_more(status: Option<&str>, cx: &mut App) -> bool {
        Self::cached_my_vacancies_dto(status, cx).is_some_and(|dto| dto.is_have_more)
    }

    pub fn my_vacancies_relative_id(status: Option<&str>, cx: &mut App) -> Option<i64> {
        Self::cached_my_vacancies_dto(status, cx)?.relative_id
    }

    pub fn my_applications(cx: &mut App) -> Vec<ApplicationModel> {
        Self::cached_my_applications_dto(cx)
            .map(|dto| dto.list.iter().map(WireApplicationDTOExt::to_model).collect())
            .unwrap_or_default()
    }

    pub fn my_applications_has_more(cx: &mut App) -> bool {
        Self::cached_my_applications_dto(cx).is_some_and(|dto| dto.is_have_more)
    }

    pub fn my_applications_relative_id(cx: &mut App) -> Option<i64> {
        Self::cached_my_applications_dto(cx)?.relative_id
    }

    pub fn my_resumes(cx: &mut App) -> Vec<ResumeModel> {
        let real: Vec<ResumeModel> = Self::cached_my_resumes_dto(cx)
            .map(|dto| dto.list.iter().map(WireResumeDTOExt::to_model).collect())
            .unwrap_or_default();
        (vacancy_guide_sandbox_hooks().resumes_snapshot)(real, cx)
    }

    pub fn my_resumes_has_more(cx: &mut App) -> bool {
        if (vacancy_guide_sandbox_hooks().hides_real_resumes)(cx) {
            return false;
        }
        Self::cached_my_resumes_dto(cx).is_some_and(|dto| dto.is_have_more)
    }

    pub fn my_resumes_relative_id(cx: &mut App) -> Option<i64> {
        Self::cached_my_resumes_dto(cx)?.relative_id
    }

    pub fn vacancy(id: i64, cx: &mut App) -> Option<VacancyModel> {
        if let Some(practice) = (vacancy_guide_sandbox_hooks().practice_vacancy)(id, cx) {
            return Some(practice);
        }
        Self::cached_single_vacancy_dto(id, true, cx)?.vacancy.map(|vacancy| vacancy.to_model())
    }

    pub fn is_vacancy_gone(&self, vacancy_id: i64) -> bool {
        self.gone_vacancy_ids.contains(&vacancy_id)
    }

    pub fn vacancy_loaded_handler(vacancy_id: i64, cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| {
            if this.gone_vacancy_ids.remove(&vacancy_id) {
                cx.notify();
            }
        });
    }

    pub fn vacancy_load_failure_handler(error: &anyhow::Error, vacancy_id: i64, cx: &mut App) {
        let code = error.downcast_ref::<SaiWsResponseError>().and_then(|response| response.code.as_deref());
        if CanonErrors::category(code) != ErrorCategory::NotFound {
            return;
        }
        Self::vacancy_gone_handler(vacancy_id, cx);
    }

    pub fn vacancy_gone_handler(vacancy_id: i64, cx: &mut App) {
        let mut keys = Self::shared(cx).update(cx, |this, cx| {
            this.gone_vacancy_ids.insert(vacancy_id);
            cx.notify();
            this.known_list_cache_keys.clone()
        });
        keys.insert(Self::default_vacancy_list_cache_key(cx));

        let match_id = vacancy_id.to_string();
        for cache_key in keys {
            let _ = SaiUpdater::remove_row(&cache_key, key::LIST, key::ID, &match_id, SaiUpdaterWriteScope::All, cx);
        }
    }

    pub fn apply_vacancy_failure_handler(error: &anyhow::Error, vacancy_id: i64, cx: &mut App) {
        let description = error.to_string();
        Sai::log(
            VACANCY_LOG_NAME,
            move || format!("{} error: {description}", rise_wire_methods::vacancy::APPLY_VACANCY),
            SaiLogKind::Network,
            cx,
        );

        let response = error.downcast_ref::<SaiWsResponseError>();
        let code = response.and_then(|response| response.code.clone());
        let details = response.map(|response| response.details.clone()).unwrap_or_default();
        if CanonErrors::category(code.as_deref()) == ErrorCategory::NotFound {
            Self::vacancy_gone_handler(vacancy_id, cx);
        }

        Notifier::send(
            CanonErrors::text(code.as_deref(), &details).unwrap_or_else(|| tr("save_failed")),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn apply_vacancy_success_handler(vacancy_id: i64, cx: &mut App) {
        let mut keys = Self::shared(cx).read(cx).known_list_cache_keys.clone();
        keys.insert(Self::default_vacancy_list_cache_key(cx));

        let match_id = vacancy_id.to_string();
        let mut patch = Map::new();
        patch.insert(key::IS_APPLIED.to_owned(), Value::Bool(true));
        for cache_key in keys {
            let _ = SaiUpdater::patch_row(
                &cache_key,
                key::LIST,
                key::ID,
                &match_id,
                &patch,
                SaiUpdaterWriteScope::All,
                cx,
            );
        }

        let Some(detail) = Self::cached_vacancy_dto(vacancy_id, cx) else { return };
        if detail.id != vacancy_id {
            return;
        }
        let Some(row) = SaiJSONCodec::encode(&detail.with(None, Some(true))) else { return };

        let mut fields = Map::new();
        fields.insert(key::VACANCY.to_owned(), Value::Object(row));
        let _ = SaiUpdater::patch_root(&Self::vacancy_cache_key(vacancy_id), &fields, SaiUpdaterWriteScope::All, cx);
    }

    pub fn revert_toggle_vacancy_optimistic(
        vacancy_id: i64,
        snapshot: Option<WireVacancyDTO>,
        list_cache_key: &str,
        cx: &mut App,
    ) {
        Sai::log(
            VACANCY_LOG_NAME,
            || format!("{} failed", rise_wire_methods::vacancy::TOGGLE_VACANCY_STATUS),
            SaiLogKind::Network,
            cx,
        );

        let Some(row) = snapshot.as_ref().and_then(SaiJSONCodec::encode) else { return };

        let _ = SaiUpdater::patch_row(
            list_cache_key,
            key::LIST,
            key::ID,
            &vacancy_id.to_string(),
            &row,
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    pub fn apply_toggled_vacancy_status(
        vacancy_id: i64,
        vacancy: Option<&WireVacancyDTO>,
        list_cache_key: &str,
        cx: &mut App,
    ) {
        let Some(active) = vacancy.map(|vacancy| vacancy.active) else { return };
        let mut patch = Map::new();
        patch.insert(key::ACTIVE.to_owned(), Value::Bool(active));
        let _ = SaiUpdater::patch_row(
            list_cache_key,
            key::LIST,
            key::ID,
            &vacancy_id.to_string(),
            &patch,
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    pub fn revert_delete_vacancy_optimistic(revert: (usize, WireVacancyDTO), list_cache_key: &str, cx: &mut App) {
        let Some(row) = SaiJSONCodec::encode(&revert.1) else { return };

        let _ = SaiUpdater::insert_row_if_missing(
            list_cache_key,
            key::LIST,
            key::ID,
            row,
            false,
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    pub fn revert_delete_resume_optimistic(revert: (usize, WireResumeDTO), list_cache_key: &str, cx: &mut App) {
        let Some(row) = SaiJSONCodec::encode(&revert.1) else { return };

        let _ = SaiUpdater::insert_row_if_missing(
            list_cache_key,
            key::LIST,
            key::ID,
            row,
            false,
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    pub fn create_resume_success_handler(on_success: VacancyResumeOnSuccess, cx: &mut App) {
        let dto = VacancyActionsStore::shared(cx).read(cx).create_resume_sai.data().and_then(|data| data.resume);
        let Some(dto) = dto else {
            Self::set_submitting_resume(false, cx);
            return;
        };
        Self::resume_write_success_handler(dto, on_success, cx);
    }

    pub fn complete_resume_update(
        dto: WireResumeDTO,
        request_id: Uuid,
        on_success: VacancyResumeOnSuccess,
        cx: &mut App,
    ) {
        let is_current = Self::shared(cx).update(cx, |this, _cx| {
            if this.resume_update_id != Some(request_id) {
                return false;
            }
            this.resume_update_id = None;
            true
        });
        if !is_current {
            return;
        }
        Self::resume_write_success_handler(dto, on_success, cx);
    }

    pub fn fail_resume_update(request_id: Uuid, notify: bool, cx: &mut App) {
        let is_current = Self::shared(cx).update(cx, |this, cx| {
            if this.resume_update_id != Some(request_id) {
                return false;
            }
            this.resume_update_id = None;
            this.is_submitting_resume = false;
            cx.notify();
            true
        });
        if is_current && notify {
            Notifier::send(
                tr("save_failed"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Top,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        }
    }

    fn set_submitting_resume(value: bool, cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| {
            this.is_submitting_resume = value;
            cx.notify();
        });
    }

    fn set_submitting_vacancy(value: bool, cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| {
            this.is_submitting_vacancy = value;
            cx.notify();
        });
    }

    fn resume_write_success_handler(dto: WireResumeDTO, on_success: VacancyResumeOnSuccess, cx: &mut App) {
        Self::set_submitting_resume(false, cx);

        let model = dto.to_model();

        let row = SaiJSONCodec::encode(&dto);
        let user_id = Self::effective_user_id_for_wire(cx);
        let (Some(row), Some(user_id)) = (row, user_id) else {
            on_success(model, cx);
            return;
        };

        let mut root = Map::new();
        root.insert(key::LIST.to_owned(), Value::Array(vec![Value::Object(row)]));
        root.insert(key::TOTAL.to_owned(), Value::from(1));
        root.insert(key::LIMIT.to_owned(), Value::from(Self::list_limit()));
        root.insert(key::IS_HAVE_MORE.to_owned(), Value::Bool(false));
        let _ = SaiUpdater::replace_root(&Self::my_resumes_cache_key(&user_id), root, SaiUpdaterWriteScope::All, cx);

        on_success(model, cx);
    }

    pub fn my_resume(cx: &mut App) -> Option<ResumeModel> {
        if (vacancy_guide_sandbox_hooks().owns_jobs_scene)(cx) {
            return Self::my_resumes(cx).into_iter().next();
        }
        if let Some(resume) = Self::my_resumes(cx).into_iter().next() {
            return Some(resume);
        }
        VacancyActionsStore::shared(cx)
            .read(cx)
            .my_resumes_sai
            .data()
            .and_then(|data| data.list.first().map(WireResumeDTOExt::to_model))
    }

    pub fn submit_failure_handler(error: &anyhow::Error, method: &str, cx: &mut App) {
        if method == rise_wire_methods::vacancy::CREATE_VACANCY || method == rise_wire_methods::vacancy::UPDATE_VACANCY {
            Self::set_submitting_vacancy(false, cx);
        } else if method == rise_wire_methods::vacancy::CREATE_RESUME {
            Self::set_submitting_resume(false, cx);
        }

        let response = error.downcast_ref::<SaiWsResponseError>();
        let code = response.and_then(|response| response.code.clone());
        let details: HashMap<String, String> = response.map(|response| response.details.clone()).unwrap_or_default();
        let logged_code = code.clone().unwrap_or_else(|| "local".to_owned());
        let logged_method = method.to_owned();
        Sai::log(
            VACANCY_LOG_NAME,
            move || format!("{logged_method} error code: {logged_code}"),
            SaiLogKind::Network,
            cx,
        );

        Notifier::send(
            CanonErrors::text(code.as_deref(), &details).unwrap_or_else(|| tr("save_failed")),
            NotifierSendOptions {
                content: if response.is_none() { Some(CanonErrors::refusal_text(error)) } else { None },
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn create_vacancy_success_handler(on_success: VacancyOnSuccess, cx: &mut App) {
        Self::set_submitting_vacancy(false, cx);

        let dto = VacancyActionsStore::shared(cx).read(cx).create_vacancy_sai.data().and_then(|data| data.vacancy);
        let Some(dto) = dto else { return };

        let model = dto.to_model();

        let row = SaiJSONCodec::encode(&dto);
        let user_id = Self::effective_user_id_for_wire(cx);
        let (Some(row), Some(user_id)) = (row, user_id) else {
            on_success(model, cx);
            return;
        };

        let _ = SaiUpdater::insert_row_if_missing(
            &Self::my_vacancies_cache_key(&user_id, None),
            key::LIST,
            key::ID,
            row,
            true,
            SaiUpdaterWriteScope::All,
            cx,
        );

        on_success(model, cx);
    }

    pub fn update_vacancy_success_handler(user_id: &str, on_success: VacancyOnSuccess, cx: &mut App) {
        Self::set_submitting_vacancy(false, cx);

        let dto = VacancyActionsStore::shared(cx).read(cx).update_vacancy_sai.data().and_then(|data| data.vacancy);
        let Some(dto) = dto else { return };
        let vacancy_id = dto.id;
        let match_id = vacancy_id.to_string();

        let model = dto.to_model();
        let row = SaiJSONCodec::encode(&dto);

        if let Some(row) = row.as_ref() {
            let _ = SaiUpdater::patch_row(
                &Self::my_vacancies_cache_key(user_id, None),
                key::LIST,
                key::ID,
                &match_id,
                row,
                SaiUpdaterWriteScope::All,
                cx,
            );
            let mut fields = Map::new();
            fields.insert(key::VACANCY.to_owned(), Value::Object(row.clone()));
            let _ =
                SaiUpdater::patch_root(&Self::vacancy_cache_key(vacancy_id), &fields, SaiUpdaterWriteScope::All, cx);
        }

        let keys = Self::shared(cx).read(cx).known_list_cache_keys.clone();
        if model.is_awaiting_review() {
            for cache_key in keys {
                let _ = SaiUpdater::remove_row(&cache_key, key::LIST, key::ID, &match_id, SaiUpdaterWriteScope::All, cx);
            }
        } else if let Some(row) = row.as_ref() {
            for cache_key in keys {
                let _ = SaiUpdater::patch_row(
                    &cache_key,
                    key::LIST,
                    key::ID,
                    &match_id,
                    row,
                    SaiUpdaterWriteScope::All,
                    cx,
                );
            }
        }

        on_success(model, cx);
    }

    fn session_wire_context(cx: &mut App) -> Option<(String, Option<String>)> {
        let user_id = Self::non_empty_user_id(cx)?;
        let user_avatar_url = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .and_then(|profile| profile.avatar_url.clone());
        Some((user_id, user_avatar_url))
    }

    pub fn birth_date_wire_string(&self, iso: Option<&str>) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(iso.unwrap_or_default());
        if trimmed.is_empty() {
            return None;
        }
        if character_count(trimmed) >= 10 && trimmed.contains('-') {
            return Some(character_prefix(trimmed, 10).to_owned());
        }

        let mut iso_fmt = ISO8601DateFormatter::new();
        iso_fmt.set_format_options(
            ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME | ISO8601DateFormatterOptions::WITH_FRACTIONAL_SECONDS,
        );
        let date = iso_fmt.date_from(trimmed).or_else(|| ISO8601DateFormatter::new().date_from(trimmed))?;
        Some(self.wire_date_string(&date))
    }

    pub fn birth_date_from_profile_hb(cx: &mut App) -> Option<Date> {
        let hb = UserActionsStore::shared(cx)
            .read(cx)
            .my_profile_sai
            .data()
            .and_then(|profile| profile.more)
            .and_then(|more| more.hb);
        let this = Self::shared(cx);
        let wire = this.read(cx).birth_date_wire_string(hb.as_deref())?;
        Self::wire_date_formatter().date_from(&wire)
    }

    fn wire_date_formatter() -> DateFormatter {
        let mut formatter = DateFormatter::new();
        formatter.set_locale(POSIX_LOCALE);
        formatter.set_date_format(WIRE_DATE_FORMAT);
        formatter
    }

    pub fn wire_date_string(&self, date: &Date) -> String {
        Self::wire_date_formatter().string_from(date)
    }

    pub fn parse_salary_int(&self, raw: &str) -> i64 {
        raw.chars().filter(|c| c.is_numeric()).collect::<String>().parse::<i64>().unwrap_or(0)
    }

    pub fn experience_range(&self, level: &str) -> Vec<i64> {
        match VacancyExperienceLevel::from_wire(level) {
            Some(VacancyExperienceLevel::NoExperience) | None => vec![0, 0],
            Some(VacancyExperienceLevel::From1To3) => vec![1, 3],
            Some(VacancyExperienceLevel::From3To6) => vec![3, 6],
            Some(VacancyExperienceLevel::From6To12) => vec![6, 12],
            Some(VacancyExperienceLevel::Over12) => vec![12, 99],
        }
    }

    pub fn split_name(&self, full: &str) -> (String, String) {
        let mut parts = trimming_whitespaces_and_newlines(full)
            .splitn(2, ' ')
            .filter(|part| !part.is_empty())
            .map(str::to_owned);
        match (parts.next(), parts.next()) {
            (None, _) => (String::new(), String::new()),
            (Some(first), None) => (first.clone(), first),
            (Some(first), Some(second)) => (first, second),
        }
    }

    pub fn work_experience_bodies(&self, entries: &[WorkExperienceEntry]) -> Vec<create_resume::WorkExperienceElement> {
        entries
            .iter()
            .filter(|entry| !trimming_whitespaces(&entry.company).is_empty())
            .map(|entry| create_resume::WorkExperienceElement {
                company: Some(entry.company.clone()),
                company_logo_url: entry.company_logo_url.clone().filter(|url| !url.is_empty()),
                description: Some(entry.job_description.clone()),
                from_date: Some(entry.from_date.clone()),
                is_current: Some(entry.is_current),
                position: Some(entry.position.clone()),
                to_date: if entry.is_current || entry.to_date.is_empty() { None } else { Some(entry.to_date.clone()) },
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_create_resume_body(
        first_name: &str,
        second_name: &str,
        phone: &str,
        city: &str,
        country: &str,
        profession: &str,
        gender: &str,
        birth_date: &str,
        education: &str,
        experience_level: &str,
        skills: &[String],
        work_experience: Vec<create_resume::WorkExperienceElement>,
        professional_summary: &str,
        salary_from: i64,
        salary_to: i64,
        salary_currency: &str,
        ready_to_relocation: bool,
        ready_to_army: bool,
        cx: &mut App,
    ) -> Option<CreateResume> {
        let (_, user_avatar_url) = Self::session_wire_context(cx)?;

        Some(CreateResume {
            avatar: user_avatar_url,
            birth_date: Some(birth_date.to_owned()),
            city: Some(city.to_owned()),
            country: if country.is_empty() { None } else { Some(country.to_owned()) },
            education: Some(education.to_owned()),
            expected_salary_range: Some(vec![salary_from, salary_to]),
            experience_level: Some(experience_level.to_owned()),
            first_name: Some(first_name.to_owned()),
            gender: Some(gender.to_owned()),
            last_name: None,
            phone: Some(phone.to_owned()),
            profession: Some(profession.to_owned()),
            professional_summary: Some(professional_summary.to_owned()),
            ready_to_army: Some(ready_to_army),
            ready_to_relocation: Some(ready_to_relocation),
            salary_currency: if salary_currency.is_empty() { None } else { Some(salary_currency.to_owned()) },
            second_name: Some(second_name.to_owned()),
            skills: Some(skills.to_vec()),
            work_experience: Some(work_experience),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_create_vacancy_body(
        title: &str,
        description: &str,
        company: &str,
        city: &str,
        salary_from: i64,
        salary_to: i64,
        salary_currency: &str,
        schedule: &str,
        job_format: &str,
        is_relocation: bool,
        skills: &[String],
        experience: Vec<i64>,
        cx: &mut App,
    ) -> Option<CreateVacancy> {
        Self::session_wire_context(cx)?;

        Some(CreateVacancy {
            city: Some(city.to_owned()),
            company: Some(company.to_owned()),
            company_logo: Some(String::new()),
            description: Some(description.to_owned()),
            employment_type: Some(VACANCY_EMPLOYMENT_TYPE),
            experience: Some(experience),
            is_relocation_available: Some(is_relocation),
            job_format: Some(job_format.to_owned()),
            location: None,
            original_description: Some(description.to_owned()),
            salary_currency: Some(ResumeCurrency::option(salary_currency).code.to_owned()),
            salary_range: Some(vec![salary_from, salary_to]),
            schedule: Some(schedule.to_owned()),
            skills: Some(skills.to_vec()),
            title: Some(title.to_owned()),
        })
    }

    pub fn normalize_schedule(&self, raw: &str) -> String {
        VacancySchedule::from_wire(raw).unwrap_or(VacancySchedule::FullTime).wire().to_owned()
    }

    pub fn normalize_job_format(&self, raw: &str) -> String {
        VacancyJobFormat::from_wire(raw).unwrap_or(VacancyJobFormat::Office).wire().to_owned()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit_create_resume(
        resume_id: Option<i64>,
        first_name: &str,
        second_name: &str,
        phone: &str,
        city: &str,
        country: &str,
        profession: &str,
        gender: &str,
        birth_date: &str,
        education: &str,
        experience_level: &str,
        skills: &[String],
        experiences: &[WorkExperienceEntry],
        summary: &str,
        salary_from: &str,
        salary_to: &str,
        salary_currency: &str,
        ready_to_relocation: bool,
        ready_to_army: bool,
        on_success: VacancyResumeOnSuccess,
        cx: &mut App,
    ) {
        let (work_experience, salary_from, salary_to) = {
            let this = Self::shared(cx);
            let this = this.read(cx);
            (
                this.work_experience_bodies(experiences),
                this.parse_salary_int(salary_from),
                this.parse_salary_int(salary_to),
            )
        };
        let Some(body) = Self::build_create_resume_body(
            first_name,
            second_name,
            phone,
            city,
            country,
            profession,
            gender,
            birth_date,
            education,
            experience_level,
            skills,
            work_experience,
            summary,
            salary_from,
            salary_to,
            salary_currency,
            ready_to_relocation,
            ready_to_army,
            cx,
        ) else {
            return;
        };

        if (vacancy_guide_sandbox_hooks().owns_jobs_scene)(cx) {
            if let Some(practice) = (vacancy_guide_sandbox_hooks().adopt_practice_resume)(&body, cx) {
                on_success(practice, cx);
            }
            return;
        }

        if Self::shared(cx).read(cx).is_submitting_resume {
            return;
        }

        Self::set_submitting_resume(true, cx);

        let previous_resume = Self::my_resume(cx);
        let clears_repeated_field = (body.skills.as_ref().is_none_or(Vec::is_empty)
            && previous_resume.as_ref().map(|resume| resume.skills.is_empty()) != Some(true))
            || (body.work_experience.as_ref().is_none_or(Vec::is_empty)
                && previous_resume.as_ref().map(|resume| resume.work_experience.is_empty()) != Some(true))
            || (body.expected_salary_range.as_ref().is_none_or(Vec::is_empty)
                && previous_resume.as_ref().map(|resume| resume.expected_salary_range.is_empty()) != Some(true));

        let adopt_default: VacancyResumeOnSuccess = Box::new(move |model: ResumeModel, cx: &mut App| {
            let resume_id = model.id;
            VacancyLocalState::shared(cx).update(cx, |state, cx| state.set_default_resume_id(resume_id, cx));
            on_success(model, cx);
        });

        if let Some(resume_id) = resume_id
            && !clears_repeated_field
        {
            let request_id = Uuid::new_v4();
            Self::shared(cx).update(cx, |this, _cx| {
                this.resume_update_id = Some(request_id);
            });
            VacancyActionsStore::shared(cx).update(cx, |actions, cx| {
                actions.update_resume_action(
                    rise_request::vacancy::UpdateResume::with_form(resume_id, &body),
                    request_id,
                    adopt_default,
                    cx,
                );
            });
            return;
        }

        VacancyActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.create_resume_action(body, adopt_default, cx);
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit_create_vacancy(
        title: &str,
        company: &str,
        city: &str,
        salary_from: &str,
        salary_to: &str,
        salary_currency: &str,
        schedule: &str,
        job_format: &str,
        description: &str,
        skills: &[String],
        is_relocation: bool,
        editing_vacancy_id: Option<i64>,
        on_success: VacancyOnSuccess,
        cx: &mut App,
    ) {
        let (salary_from, salary_to) = {
            let this = Self::shared(cx);
            let this = this.read(cx);
            (this.parse_salary_int(salary_from), this.parse_salary_int(salary_to))
        };
        let Some(body) = Self::build_create_vacancy_body(
            title,
            description,
            company,
            city,
            salary_from,
            salary_to,
            salary_currency,
            schedule,
            job_format,
            is_relocation,
            skills,
            DEFAULT_VACANCY_EXPERIENCE.to_vec(),
            cx,
        ) else {
            return;
        };

        if let Some(practice) = (vacancy_guide_sandbox_hooks().adopt_practice_vacancy)(&body, cx) {
            on_success(practice, cx);
            return;
        }

        if Self::shared(cx).read(cx).is_submitting_vacancy {
            return;
        }

        Self::set_submitting_vacancy(true, cx);

        if let Some(editing_vacancy_id) = editing_vacancy_id {
            VacancyActionsStore::shared(cx).update(cx, |actions, cx| {
                actions.update_vacancy_action(
                    rise_request::vacancy::UpdateVacancy::with_form(editing_vacancy_id, &body),
                    on_success,
                    cx,
                );
            });
        } else {
            VacancyActionsStore::shared(cx).update(cx, |actions, cx| {
                actions.create_vacancy_action(body, on_success, cx);
            });
        }
    }

    pub fn clear_default_resume_if_matches(resume_id: i64, cx: &mut App) {
        VacancyLocalState::shared(cx).update(cx, |state, cx| {
            if state.default_resume_id(cx) != Some(resume_id) {
                return;
            }
            state.clear_default_resume_id(cx);
        });
    }

    pub fn upload_resume_logo(
        local_file: SaiWsOptimisticLocalFile,
        on_progress: Option<SaiWsUploadOnProgress>,
        cx: &mut App,
    ) -> Task<Option<String>> {
        let remover = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            let original_path = local_file.url.to_file_path()?;
            let original = RiseOutgoingMediaFile::new(
                original_path,
                local_file.filename.clone(),
                local_file.content_type.clone(),
            );
            let source_url = local_file.url.standardized();
            let compressed = OutgoingMediaCompressor::compress_if_needed_optimistic(
                local_file,
                None,
                MediaCompressionConfiguration::avatar(),
                None,
                None,
            )
            .await
            .ok()?;
            let compressed_path = compressed.url.to_file_path()?;
            let generated_temporary_path =
                (compressed.url.standardized() != source_url).then(|| compressed_path.clone());

            let prepared = RiseConversationUploadSupport::prepared_local_file(&original, &compressed);
            let upload = cx.update(|cx| {
                SaiWsChunkedFileUpload::upload(
                    compressed_path,
                    prepared.filename.clone(),
                    compressed.content_type.clone(),
                    RESUME_LOGO_FILE_CONTEXT.to_owned(),
                    None,
                    SaiAccountCacheScope::active_user_id(),
                    SaiAccountCacheScope::active_epoch(),
                    on_progress,
                    None,
                    cx,
                )
            });
            let done = upload.await;

            if let Some(path) = generated_temporary_path {
                remover
                    .spawn(async move {
                        let _ = FileManager.remove_item(&path);
                    })
                    .await;
            }

            MusicCoverUpload::extract_remote_url(&done.ok()?)
        })
    }

    pub fn share_resume(model: ResumeModel, on_finish: Option<Box<dyn FnOnce(&mut App)>>, cx: &mut App) {
        let started = Self::shared(cx).update(cx, |this, cx| {
            if this.is_building_resume_pdf {
                return false;
            }
            this.is_building_resume_pdf = true;
            cx.notify();
            true
        });
        if !started {
            return;
        }

        let full_name = model.full_name();
        let weak = Self::shared(cx).downgrade();
        ResumePdfBuilder::build(
            model,
            move |data, cx| {
                let Ok(()) = weak.update(cx, |this, cx| {
                    this.is_building_resume_pdf = false;
                    cx.notify();
                }) else {
                    return;
                };
                let Some(data) = data else {
                    if let Some(on_finish) = on_finish {
                        on_finish(cx);
                    }
                    return;
                };
                let title = tr("vacancy_resume_my_title");
                let file_name = if full_name.is_empty() {
                    format!("{title}.{PDF_EXTENSION}")
                } else {
                    format!("{full_name} - {title}.{PDF_EXTENSION}")
                };
                Self::present_resume_pdf(data, Some(file_name), cx);
                if let Some(on_finish) = on_finish {
                    on_finish(cx);
                }
            },
            cx,
        );
    }

    pub fn present_resume_pdf(data: Vec<u8>, file_name: Option<String>, cx: &mut App) {
        let requested_file_name =
            file_name.unwrap_or_else(|| format!("{}.{PDF_EXTENSION}", tr("vacancy_resume_my_title")));
        let resolved_file_name = host_file_name(&requested_file_name, PathFlavor::HOST);
        let directory = temporary_directory();
        let url = directory.join(&resolved_file_name);
        let written_url = url.clone();
        let writer = cx.background_executor().clone();

        cx.spawn(async move |cx| {
            let written = writer
                .spawn(async move {
                    Self::write_atomically(&written_url, &data)?;
                    Ok::<Vec<u8>, std::io::Error>(data)
                })
                .await;
            let Ok(data) = written else {
                cx.update(Self::notify_resume_pdf_save_failed);
                return;
            };

            cx.update(|cx| Self::present_resume_pdf_sheet(url, directory, resolved_file_name, data, cx));
        })
        .detach();
    }

    fn present_resume_pdf_sheet(
        url: PathBuf,
        directory: PathBuf,
        file_name: String,
        data: Vec<u8>,
        cx: &mut App,
    ) {
        let shared = Rc::new(Cell::new(false));
        let on_success_shared = shared.clone();
        SystemShare::present_reporting(
            vec![ShareItem::File(url)],
            Some(Box::new(move |_cx: &mut App| on_success_shared.set(true))),
            Some(Box::new(move |cx: &mut App| {
                if !shared.get() {
                    Self::save_resume_pdf_as(directory, file_name, data, cx);
                }
            })),
            cx,
        );
    }

    fn save_resume_pdf_as(directory: PathBuf, file_name: String, data: Vec<u8>, cx: &mut App) {
        let writer = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            let start_directory = downloads_directory().unwrap_or(directory);
            let request = SaveRequest::new(start_directory, FileFilter::any()).with_suggested_name(file_name);
            let outcome = cx.update(|cx| file_dialog::save(cx, &request)).await;
            let destination = match outcome {
                SaveOutcome::Chosen(destination) => destination,
                SaveOutcome::Cancelled => return,
                SaveOutcome::Failed(_) => {
                    cx.update(Self::notify_resume_pdf_save_failed);
                    return;
                }
            };

            let saved = writer.spawn(async move { Self::write_atomically(&destination, &data) }).await;
            if saved.is_err() {
                cx.update(Self::notify_resume_pdf_save_failed);
            }
        })
        .detach();
    }

    fn notify_resume_pdf_save_failed(cx: &mut App) {
        Notifier::send(
            tr("save_failed"),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut temporary = url.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, data)?;
        std::fs::rename(&temporary, url).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    }
}
