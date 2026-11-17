use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AsyncApp, Context, Entity};
use rise_canon::rise_key::vacancy as key;
use rise_canon::{rise_request, rise_wire_methods};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_errors::ErrorCategory;
use rise_i18n::tr;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiTypedSlice, SaiWireEmptyDTO};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticUpdate;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsRequestOptions, SaiWsRequestOptionsInit};
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::vacancy::engine::rise_vacancy_resume_rpc::RiseVacancyResumeRPC;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_wire_types::{
    WireApplicationsListDTO, WireEmployerStatusDTO, WireResumeDTO, WireResumeListDTO, WireSingleApplicationDTO,
    WireSingleResumeDTO, WireSingleVacancyDTO, WireVacancyListDTO,
};
use crate::modules::vacancy::stores::vacancy_guide_sandbox_hooks::vacancy_guide_sandbox_hooks;
use crate::modules::vacancy::stores::vacancy_services::vacancy_services::{
    VacancyOnSuccess, VacancyResumeOnSuccess, VacancyServicesStore,
};

pub struct VacancyActionsStore {
    pub list_vacancies_sai: Rc<SaiTypedSlice<WireVacancyListDTO>>,
    pub get_vacancy_sai: Rc<SaiTypedSlice<WireSingleVacancyDTO>>,
    pub my_vacancies_sai: Rc<SaiTypedSlice<WireVacancyListDTO>>,
    pub delete_vacancy_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub toggle_vacancy_status_sai: Rc<SaiTypedSlice<WireSingleVacancyDTO>>,
    pub apply_vacancy_sai: Rc<SaiTypedSlice<WireSingleApplicationDTO>>,
    pub my_applications_sai: Rc<SaiTypedSlice<WireApplicationsListDTO>>,
    pub employer_status_sai: Rc<SaiTypedSlice<WireEmployerStatusDTO>>,
    pub my_resumes_sai: Rc<SaiTypedSlice<WireResumeListDTO>>,
    pub create_resume_sai: Rc<SaiTypedSlice<WireSingleResumeDTO>>,
    pub delete_resume_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub create_vacancy_sai: Rc<SaiTypedSlice<WireSingleVacancyDTO>>,
    pub update_vacancy_sai: Rc<SaiTypedSlice<WireSingleVacancyDTO>>,
}

impl VacancyActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            list_vacancies_sai: SaiTypedSlice::new(cx),
            get_vacancy_sai: SaiTypedSlice::new(cx),
            my_vacancies_sai: SaiTypedSlice::new(cx),
            delete_vacancy_sai: SaiTypedSlice::new(cx),
            toggle_vacancy_status_sai: SaiTypedSlice::new(cx),
            apply_vacancy_sai: SaiTypedSlice::new(cx),
            my_applications_sai: SaiTypedSlice::new(cx),
            employer_status_sai: SaiTypedSlice::new(cx),
            my_resumes_sai: SaiTypedSlice::new(cx),
            create_resume_sai: SaiTypedSlice::new(cx),
            delete_resume_sai: SaiTypedSlice::new(cx),
            create_vacancy_sai: SaiTypedSlice::new(cx),
            update_vacancy_sai: SaiTypedSlice::new(cx),
        }
    }

    fn wire_user_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()
    }

    fn log_network(text: String, cx: &mut App) {
        Sai::log("Vacancy", move || text, SaiLogKind::Network, cx);
    }

    fn notify_error(cx: &mut App) {
        Notifier::send(
            tr("error"),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn owned_error(error: &anyhow::Error) -> anyhow::Error {
        if let Some(response) = error.downcast_ref::<SaiWsResponseError>() {
            return anyhow::Error::new(response.clone());
        }
        if error.is::<CancellationError>() {
            return CancellationError.into();
        }
        anyhow::Error::msg(format!("{error:#}"))
    }

    fn relative_id_pagination(relative_id: Option<i64>) -> SaiWsPaginationOptions {
        SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
            relative_id: relative_id.map(|id| id.to_string()),
            has_more_path: Some(key::IS_HAVE_MORE.to_owned()),
            duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
            ..SaiWsPaginationRelativeIdArgs::default()
        })
    }

    /// true if the account the request was sent for is still the active one
    fn is_current_account(user_id: &str, account_epoch: &str) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(Some(user_id), Some(account_epoch))
    }

    /// user id and active epoch for a write that must land on the account it started on
    fn current_account_scope(cx: &mut App) -> Option<(String, String)> {
        let user_id = Self::wire_user_id(cx).filter(|user_id| !user_id.is_empty())?;
        let account_epoch = SaiAccountCacheScope::active_epoch()?;
        Self::is_current_account(&user_id, &account_epoch).then_some((user_id, account_epoch))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn list_vacancies_action(
        &mut self,
        q: Option<String>,
        schedule: Option<String>,
        job_format: Option<String>,
        min_salary: Option<i64>,
        max_salary: Option<i64>,
        city: Option<String>,
        is_relocation: Option<bool>,
        relative_id: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let user_id = Self::wire_user_id(cx);
        let first_page = relative_id.is_none();
        let normalized_query = VacancyServicesStore::normalized_list_filter(q.as_deref());
        let normalized_schedule = VacancyServicesStore::normalized_list_filter(schedule.as_deref());
        let normalized_job_format = VacancyServicesStore::normalized_list_filter(job_format.as_deref());
        let normalized_city = VacancyServicesStore::normalized_list_filter(city.as_deref());

        let list_key = VacancyServicesStore::vacancy_list_cache_key(
            normalized_query.as_deref(),
            normalized_schedule.as_deref(),
            normalized_job_format.as_deref(),
            min_salary,
            max_salary,
            normalized_city.as_deref(),
            is_relocation,
            user_id.as_deref(),
        );

        let body = rise_request::vacancy::ListVacancies {
            city: normalized_city,
            is_relocation_available: is_relocation,
            job_format: normalized_job_format,
            limit: Some(VacancyServicesStore::list_limit()),
            max_salary,
            min_salary,
            q: normalized_query,
            relative_id,
            schedule: normalized_schedule,
            up: Some(!first_page),
            ..rise_request::vacancy::ListVacancies::default()
        };

        VacancyServicesStore::note_list_cache_key(list_key.clone(), cx);

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::vacancy::SERVICE.to_owned(),
            method: rise_wire_methods::vacancy::LIST_VACANCIES.to_owned(),
            cache_key: Some(list_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: first_page,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            reload_when_only_disk_cached: true,
            path_to_array: Some(key::LIST.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            pagination: Some(Self::relative_id_pagination(relative_id)),
            ..SaiWsRequestOptionsInit::default()
        });

        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("list_vacancies error: {error}"), cx);
        });
        SaiWs::request_slice(&self.list_vacancies_sai, body, options, None, Some(failure), cx);
    }

    pub fn get_vacancy_action(&mut self, vacancy_id: i64, cx: &mut Context<Self>) {
        if (vacancy_guide_sandbox_hooks().practice_vacancy)(vacancy_id, cx).is_some() {
            return;
        }

        let body = rise_request::vacancy::GetVacancy { vacancy_id: Some(vacancy_id) };

        let cache_key = VacancyServicesStore::vacancy_cache_key(vacancy_id);

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::vacancy::SERVICE.to_owned(),
            method: rise_wire_methods::vacancy::GET_VACANCY.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            cx.defer(move |cx| {
                VacancyServicesStore::vacancy_loaded_handler(vacancy_id, cx);
            });
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let error = Self::owned_error(error);
            cx.defer(move |cx| {
                VacancyServicesStore::vacancy_load_failure_handler(&error, vacancy_id, cx);
            });
        });
        SaiWs::request_slice(&self.get_vacancy_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_my_vacancies_action(&mut self, status: Option<String>, relative_id: Option<i64>, cx: &mut Context<Self>) {
        let Some(user_id) = Self::wire_user_id(cx).filter(|user_id| !user_id.is_empty()) else {
            return;
        };

        let first_page = relative_id.is_none();
        let cache_key = VacancyServicesStore::my_vacancies_cache_key(&user_id, status.as_deref());
        let body = rise_request::vacancy::GetMyVacancies {
            limit: Some(VacancyServicesStore::list_limit()),
            relative_id,
            status,
            up: Some(!first_page),
        };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::vacancy::SERVICE.to_owned(),
            method: rise_wire_methods::vacancy::GET_MY_VACANCIES.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: first_page,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            path_to_array: Some(key::LIST.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            pagination: Some(Self::relative_id_pagination(relative_id)),
            ..SaiWsRequestOptionsInit::default()
        });

        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("get_my_vacancies error: {error}"), cx);
        });
        SaiWs::request_slice(&self.my_vacancies_sai, body, options, None, Some(failure), cx);
    }

    pub fn delete_vacancy_action(
        &mut self,
        vacancy_id: i64,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if self.delete_vacancy_sai.is_pending() {
            return;
        }
        if (vacancy_guide_sandbox_hooks().delete_practice_vacancy)(vacancy_id, cx) {
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
            return;
        }
        if vacancy_id <= 0 {
            return;
        }
        let Some((user_id, account_epoch)) = Self::current_account_scope(cx) else {
            return;
        };

        let list_cache_key = VacancyServicesStore::my_vacancies_cache_key(&user_id, None);
        let list = self.my_vacancies_sai.data().map(|data| data.list).unwrap_or_default();
        let removed_index = list.iter().position(|item| item.id == vacancy_id);
        let removed_item = removed_index.and_then(|index| list.get(index).cloned());

        let optimistic = SaiWsOptimisticUpdate {
            enabled: true,
            delete_mode: true,
            delete_id: Some(vacancy_id.to_string()),
            target_cache_key: Some(list_cache_key.clone()),
            ..SaiWsOptimisticUpdate::default()
        };

        let body = rise_request::vacancy::DeleteVacancy { vacancy_id: Some(vacancy_id) };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::vacancy::SERVICE.to_owned(),
            method: rise_wire_methods::vacancy::DELETE_VACANCY.to_owned(),
            reflect_pending_in_ui: true,
            allows_deferred_send: false,
            store_response_in_cache: true,
            path_to_array: Some(key::LIST.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            optimistic_update: Some(optimistic),
            ..SaiWsRequestOptionsInit::default()
        });

        let success_user_id = user_id.clone();
        let success_epoch = account_epoch.clone();
        let success_callback = on_success.clone();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if !Self::is_current_account(&success_user_id, &success_epoch) {
                return;
            }
            let success_callback = success_callback.clone();
            cx.defer(move |cx| {
                VacancyServicesStore::vacancy_gone_handler(vacancy_id, cx);
                if let Some(on_success) = &success_callback {
                    on_success(cx);
                }
            });
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if !Self::is_current_account(&user_id, &account_epoch) {
                return;
            }
            let code = error.downcast_ref::<SaiWsResponseError>().and_then(|error| error.code.clone());
            let not_found = CanonErrors::category(code.as_deref()) == ErrorCategory::NotFound;
            let revert = removed_index.zip(removed_item.clone());
            let list_cache_key = list_cache_key.clone();
            let on_success = on_success.clone();
            cx.defer(move |cx| {
                if not_found {
                    VacancyServicesStore::vacancy_gone_handler(vacancy_id, cx);
                    if let Some(on_success) = &on_success {
                        on_success(cx);
                    }
                    return;
                }
                if let Some(revert) = revert {
                    VacancyServicesStore::revert_delete_vacancy_optimistic(revert, &list_cache_key, cx);
                }
                Self::log_network("delete_vacancy failed".to_owned(), cx);
                Self::notify_error(cx);
                Self::shared(cx).update(cx, |actions, cx| actions.get_my_vacancies_action(None, None, cx));
            });
        });
        SaiWs::request_slice(&self.delete_vacancy_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn toggle_vacancy_status_action(&mut self, vacancy_id: i64, cx: &mut Context<Self>) {
        if (vacancy_guide_sandbox_hooks().toggle_practice_vacancy)(vacancy_id, cx) {
            return;
        }
        let Some((user_id, account_epoch)) = Self::current_account_scope(cx) else {
            return;
        };

        let list_cache_key = VacancyServicesStore::my_vacancies_cache_key(&user_id, None);
        let snapshot = self
            .my_vacancies_sai
            .data()
            .and_then(|data| data.list.into_iter().find(|item| item.id == vacancy_id));

        let optimistic = SaiWsOptimisticUpdate {
            enabled: true,
            patch_in_place: true,
            patch_match_id: Some(vacancy_id.to_string()),
            target_cache_key: Some(list_cache_key.clone()),
            patch_paths: vec![key::LIST.to_owned()],
            create_patch: Some(Arc::new(
                |_: &Value, row: &Value, _: &mut App| -> anyhow::Result<Map<String, Value>> {
                    let Some(dict) = AnyDictionaryBridge::dictionary(Some(row)) else {
                        return Ok(Map::new());
                    };
                    let active = dict.get(key::ACTIVE).and_then(Value::as_bool).unwrap_or(true);
                    let mut patch = Map::new();
                    patch.insert(key::ACTIVE.to_owned(), Value::Bool(!active));
                    Ok(patch)
                },
            )),
            create_rollback_snapshot: Some(Arc::new(
                |_: &Value, row: &Value, _: &mut App| -> anyhow::Result<Map<String, Value>> {
                    let Some(dict) = AnyDictionaryBridge::dictionary(Some(row)) else {
                        return Ok(Map::new());
                    };
                    let mut snapshot = Map::new();
                    snapshot.insert(
                        key::ACTIVE.to_owned(),
                        dict.get(key::ACTIVE).cloned().unwrap_or(Value::Bool(true)),
                    );
                    Ok(snapshot)
                },
            )),
            ..SaiWsOptimisticUpdate::default()
        };

        let body = rise_request::vacancy::ToggleVacancyStatus { vacancy_id: Some(vacancy_id) };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::vacancy::SERVICE.to_owned(),
            method: rise_wire_methods::vacancy::TOGGLE_VACANCY_STATUS.to_owned(),
            reflect_pending_in_ui: false,
            allows_deferred_send: false,
            store_response_in_cache: true,
            path_to_array: Some(key::LIST.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            optimistic_update: Some(optimistic),
            ..SaiWsRequestOptionsInit::default()
        });

        let toggled = Rc::downgrade(&self.toggle_vacancy_status_sai);
        let success_user_id = user_id.clone();
        let success_epoch = account_epoch.clone();
        let success_key = list_cache_key.clone();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if !Self::is_current_account(&success_user_id, &success_epoch) {
                return;
            }
            let vacancy = toggled.upgrade().and_then(|slice| slice.data()).and_then(|data| data.vacancy);
            let list_cache_key = success_key.clone();
            cx.defer(move |cx| {
                VacancyServicesStore::apply_toggled_vacancy_status(vacancy_id, vacancy.as_ref(), &list_cache_key, cx);
            });
        });
        let failure: SaiWsOnError = Rc::new(move |_: &anyhow::Error, cx: &mut App| {
            if !Self::is_current_account(&user_id, &account_epoch) {
                return;
            }
            let snapshot = snapshot.clone();
            let list_cache_key = list_cache_key.clone();
            cx.defer(move |cx| {
                VacancyServicesStore::revert_toggle_vacancy_optimistic(vacancy_id, snapshot, &list_cache_key, cx);
                Self::notify_error(cx);
            });
        });
        SaiWs::request_slice(&self.toggle_vacancy_status_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn apply_vacancy_action(
        &mut self,
        vacancy_id: i64,
        resume_id: Option<i64>,
        cover_letter: Option<String>,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_failure: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if (vacancy_guide_sandbox_hooks().apply_to_practice_vacancy)(vacancy_id, cx) {
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
            return;
        }
        if Self::wire_user_id(cx).filter(|user_id| !user_id.is_empty()).is_none() {
            return;
        }
        if self.apply_vacancy_sai.is_pending() {
            return;
        }

        let body = rise_request::vacancy::ApplyVacancy {
            cover_letter: cover_letter.filter(|letter| !letter.is_empty()),
            resume_id,
            vacancy_id: Some(vacancy_id),
        };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::vacancy::SERVICE.to_owned(),
            method: rise_wire_methods::vacancy::APPLY_VACANCY.to_owned(),
            reflect_pending_in_ui: true,
            allows_deferred_send: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        self.apply_vacancy_sai.mark_pending();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let on_success = on_success.clone();
            cx.defer(move |cx| {
                VacancyServicesStore::apply_vacancy_success_handler(vacancy_id, cx);
                if let Some(on_success) = &on_success {
                    on_success(cx);
                }
            });
        });
        let applied = Rc::downgrade(&self.apply_vacancy_sai);
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(slice) = applied.upgrade() {
                slice.mark_rejected(error);
            }
            let error = Self::owned_error(error);
            let on_failure = on_failure.clone();
            cx.defer(move |cx| {
                VacancyServicesStore::apply_vacancy_failure_handler(&error, vacancy_id, cx);
                if let Some(on_failure) = &on_failure {
                    on_failure(cx);
                }
            });
        });
        SaiWs::request_slice(&self.apply_vacancy_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_my_applications_action(&mut self, relative_id: Option<i64>, cx: &mut Context<Self>) {
        let Some(user_id) = Self::wire_user_id(cx).filter(|user_id| !user_id.is_empty()) else {
            return;
        };

        let first_page = relative_id.is_none();
        let body = rise_request::vacancy::GetMyApplications {
            limit: Some(VacancyServicesStore::list_limit()),
            relative_id,
            up: Some(!first_page),
        };

        let cache_key = VacancyServicesStore::my_applications_cache_key(&user_id);

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::vacancy::SERVICE.to_owned(),
            method: rise_wire_methods::vacancy::GET_MY_APPLICATIONS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: first_page,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            path_to_array: Some(key::LIST.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            pagination: Some(Self::relative_id_pagination(relative_id)),
            ..SaiWsRequestOptionsInit::default()
        });

        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("get_my_applications error: {error}"), cx);
        });
        SaiWs::request_slice(&self.my_applications_sai, body, options, None, Some(failure), cx);
    }

    pub fn get_employer_status_action(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = Self::wire_user_id(cx).filter(|user_id| !user_id.is_empty()) else {
            return;
        };

        let body = rise_request::vacancy::GetEmployerStatus {};

        let cache_key = VacancyServicesStore::employer_status_cache_key(&user_id);

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::vacancy::SERVICE.to_owned(),
            method: rise_wire_methods::vacancy::GET_EMPLOYER_STATUS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        SaiWs::request_slice(&self.employer_status_sai, body, options, None, None, cx);
    }

    pub fn get_my_resumes_action(&mut self, relative_id: Option<i64>, cx: &mut Context<Self>) {
        let Some(user_id) = Self::wire_user_id(cx).filter(|user_id| !user_id.is_empty()) else {
            return;
        };

        let first_page = relative_id.is_none();
        let body = rise_request::vacancy::GetMyResumes {
            limit: Some(VacancyServicesStore::list_limit()),
            relative_id,
            up: Some(!first_page),
        };

        let cache_key = VacancyServicesStore::my_resumes_cache_key(&user_id);

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::vacancy::SERVICE.to_owned(),
            method: rise_wire_methods::vacancy::GET_MY_RESUMES.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: first_page,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            path_to_array: Some(key::LIST.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            pagination: Some(Self::relative_id_pagination(relative_id)),
            ..SaiWsRequestOptionsInit::default()
        });

        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("get_my_resumes error: {error}"), cx);
        });
        SaiWs::request_slice(&self.my_resumes_sai, body, options, None, Some(failure), cx);
    }

    pub fn create_resume_action(
        &mut self,
        body: rise_request::vacancy::CreateResume,
        on_success: VacancyResumeOnSuccess,
        cx: &mut Context<Self>,
    ) {
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::vacancy::SERVICE.to_owned(),
            method: rise_wire_methods::vacancy::CREATE_RESUME.to_owned(),
            reflect_pending_in_ui: false,
            allows_deferred_send: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success = RefCell::new(Some(on_success));
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(on_success) = on_success.borrow_mut().take() else {
                return;
            };
            cx.defer(move |cx| {
                VacancyServicesStore::create_resume_success_handler(on_success, cx);
            });
        });
        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            let error = Self::owned_error(error);
            cx.defer(move |cx| {
                VacancyServicesStore::submit_failure_handler(&error, rise_wire_methods::vacancy::CREATE_RESUME, cx);
            });
        });
        SaiWs::request_slice(&self.create_resume_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn update_resume_action(
        &mut self,
        body: rise_request::vacancy::UpdateResume,
        request_id: Uuid,
        on_success: VacancyResumeOnSuccess,
        cx: &mut Context<Self>,
    ) {
        let account_id = Self::wire_user_id(cx).unwrap_or_default();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        if body.resume_id.unwrap_or(0) <= 0
            || account_id.is_empty()
            || !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), account_epoch.as_deref())
        {
            VacancyServicesStore::fail_resume_update(request_id, true, cx);
            return;
        }

        let prepare = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let is_current =
                || SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), account_epoch.as_deref());
            let outcome: anyhow::Result<WireResumeDTO> = async {
                let engine = prepare.await?;
                if engine.account.account_id != account_id
                    || Some(engine.account.epoch.as_str()) != account_epoch.as_deref()
                    || !is_current()
                {
                    return Err(anyhow::Error::from(CancellationError));
                }

                let dto = RiseVacancyResumeRPC { engine: Arc::clone(&engine) }.update(&body).await?;
                let same_engine = cx.update(|cx| {
                    RiseEngineAppRuntime::shared(cx)
                        .read(cx)
                        .engine()
                        .is_some_and(|active| Arc::ptr_eq(&active, &engine))
                });
                if !same_engine || !is_current() {
                    return Err(anyhow::Error::from(CancellationError));
                }
                anyhow::Ok(dto)
            }
            .await;

            match outcome {
                Ok(dto) => cx.update(|cx| {
                    VacancyServicesStore::complete_resume_update(dto, request_id, on_success, cx);
                }),
                Err(error) => {
                    if !is_current() {
                        return;
                    }
                    let notify = !error.is::<CancellationError>();
                    cx.update(|cx| {
                        VacancyServicesStore::fail_resume_update(request_id, notify, cx);
                    });
                }
            }
        })
        .detach();
    }

    pub fn delete_resume_action(
        &mut self,
        resume_id: i64,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if self.delete_resume_sai.is_pending() {
            return;
        }
        if (vacancy_guide_sandbox_hooks().delete_practice_resume)(resume_id, cx) {
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
            return;
        }
        if resume_id <= 0 {
            return;
        }
        let Some((user_id, account_epoch)) = Self::current_account_scope(cx) else {
            return;
        };

        let list_cache_key = VacancyServicesStore::my_resumes_cache_key(&user_id);
        let list = self.my_resumes_sai.data().map(|data| data.list).unwrap_or_default();
        let removed_index = list.iter().position(|item| item.id == resume_id);
        let removed_item: Option<WireResumeDTO> = removed_index.and_then(|index| list.get(index).cloned());

        let optimistic = SaiWsOptimisticUpdate {
            enabled: true,
            delete_mode: true,
            delete_id: Some(resume_id.to_string()),
            target_cache_key: Some(list_cache_key.clone()),
            ..SaiWsOptimisticUpdate::default()
        };

        let body = rise_request::vacancy::DeleteResume { resume_id: Some(resume_id) };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::vacancy::SERVICE.to_owned(),
            method: rise_wire_methods::vacancy::DELETE_RESUME.to_owned(),
            reflect_pending_in_ui: true,
            allows_deferred_send: false,
            store_response_in_cache: true,
            path_to_array: Some(key::LIST.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            optimistic_update: Some(optimistic),
            ..SaiWsRequestOptionsInit::default()
        });

        let success_user_id = user_id.clone();
        let success_epoch = account_epoch.clone();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if !Self::is_current_account(&success_user_id, &success_epoch) {
                return;
            }
            let on_success = on_success.clone();
            cx.defer(move |cx| {
                VacancyServicesStore::clear_default_resume_if_matches(resume_id, cx);
                if let Some(on_success) = &on_success {
                    on_success(cx);
                }
            });
        });
        let failure: SaiWsOnError = Rc::new(move |_: &anyhow::Error, cx: &mut App| {
            if !Self::is_current_account(&user_id, &account_epoch) {
                return;
            }
            let revert = removed_index.zip(removed_item.clone());
            let list_cache_key = list_cache_key.clone();
            cx.defer(move |cx| {
                if let Some(revert) = revert {
                    VacancyServicesStore::revert_delete_resume_optimistic(revert, &list_cache_key, cx);
                }
                Self::log_network("delete_resume failed".to_owned(), cx);
                Self::notify_error(cx);
                Self::shared(cx).update(cx, |actions, cx| actions.get_my_resumes_action(None, cx));
            });
        });
        SaiWs::request_slice(&self.delete_resume_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn create_vacancy_action(
        &mut self,
        body: rise_request::vacancy::CreateVacancy,
        on_success: VacancyOnSuccess,
        cx: &mut Context<Self>,
    ) {
        let Some((user_id, account_epoch)) = Self::current_account_scope(cx) else {
            let error = anyhow::Error::from(CancellationError);
            VacancyServicesStore::submit_failure_handler(&error, rise_wire_methods::vacancy::CREATE_VACANCY, cx);
            return;
        };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::vacancy::SERVICE.to_owned(),
            method: rise_wire_methods::vacancy::CREATE_VACANCY.to_owned(),
            reflect_pending_in_ui: false,
            allows_deferred_send: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success = RefCell::new(Some(on_success));
        let success_user_id = user_id.clone();
        let success_epoch = account_epoch.clone();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if !Self::is_current_account(&success_user_id, &success_epoch) {
                return;
            }
            let Some(on_success) = on_success.borrow_mut().take() else {
                return;
            };
            cx.defer(move |cx| {
                VacancyServicesStore::create_vacancy_success_handler(on_success, cx);
            });
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if !Self::is_current_account(&user_id, &account_epoch) {
                return;
            }
            let error = Self::owned_error(error);
            cx.defer(move |cx| {
                VacancyServicesStore::submit_failure_handler(&error, rise_wire_methods::vacancy::CREATE_VACANCY, cx);
            });
        });
        SaiWs::request_slice(&self.create_vacancy_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn update_vacancy_action(
        &mut self,
        body: rise_request::vacancy::UpdateVacancy,
        on_success: VacancyOnSuccess,
        cx: &mut Context<Self>,
    ) {
        let Some((user_id, account_epoch)) = Self::current_account_scope(cx) else {
            let error = anyhow::Error::from(CancellationError);
            VacancyServicesStore::submit_failure_handler(&error, rise_wire_methods::vacancy::UPDATE_VACANCY, cx);
            return;
        };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::vacancy::SERVICE.to_owned(),
            method: rise_wire_methods::vacancy::UPDATE_VACANCY.to_owned(),
            reflect_pending_in_ui: false,
            allows_deferred_send: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success = RefCell::new(Some(on_success));
        let success_user_id = user_id.clone();
        let success_epoch = account_epoch.clone();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if !Self::is_current_account(&success_user_id, &success_epoch) {
                return;
            }
            let Some(on_success) = on_success.borrow_mut().take() else {
                return;
            };
            let user_id = success_user_id.clone();
            cx.defer(move |cx| {
                VacancyServicesStore::update_vacancy_success_handler(&user_id, on_success, cx);
            });
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if !Self::is_current_account(&user_id, &account_epoch) {
                return;
            }
            let error = Self::owned_error(error);
            cx.defer(move |cx| {
                VacancyServicesStore::submit_failure_handler(&error, rise_wire_methods::vacancy::UPDATE_VACANCY, cx);
            });
        });
        SaiWs::request_slice(&self.update_vacancy_sai, body, options, Some(success), Some(failure), cx);
    }
}
