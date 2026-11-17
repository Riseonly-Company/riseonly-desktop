use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use futures::channel::oneshot;
use gpui::{App, AsyncApp, Context, Task};
use rise_canon::rise_key::user as key;
use rise_canon::rise_service_text::user_service;
use rise_canon::{rise_request, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsRequestOptions, SaiWsRequestOptionsInit, SaiWsSendError,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::engine::rise_user_domain::RiseUserActionExecutor;
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_actions::user_contact_types::{
    UserContactDraft, UserContactLimits, UserContactMutationError, UserContactPeer, UserContactPhotoUpload,
};
use crate::modules::user::stores::user::user_actions::user_contact_wire_types::{
    SyncUserContactsGatewayDTO, UserContactPhotoSuggestionWireDTO, UserContactSyncSettingsGatewayDTO,
    UserSavedContactGatewayDTO, UserSavedContactWireDTO,
};
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

/// one-shot result handler of a contact action, called on the main thread at most once
pub type UserContactCallback<A> = Box<dyn FnOnce(A, &mut App)>;

struct UserContactCallbackSlot<A>(Rc<RefCell<Option<UserContactCallback<A>>>>);

impl<A> Clone for UserContactCallbackSlot<A> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<A> UserContactCallbackSlot<A> {
    fn new(callback: Option<UserContactCallback<A>>) -> Self {
        Self(Rc::new(RefCell::new(callback)))
    }

    fn resolve(&self, value: A, cx: &mut App) {
        let callback = self.0.borrow_mut().take();
        if let Some(callback) = callback {
            callback(value, cx);
        }
    }
}

struct UserContactAsyncResultBox<Value> {
    continuation: RefCell<Option<oneshot::Sender<anyhow::Result<Value>>>>,
}

impl<Value> UserContactAsyncResultBox<Value> {
    fn install(continuation: oneshot::Sender<anyhow::Result<Value>>) -> Rc<Self> {
        Rc::new(Self { continuation: RefCell::new(Some(continuation)) })
    }

    fn resolve(&self, result: anyhow::Result<Value>) {
        let continuation = self.continuation.borrow_mut().take();
        if let Some(continuation) = continuation {
            let _ = continuation.send(result);
        }
    }
}

impl UserActionsStore {
    fn contact_wire_user_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()
    }

    fn contact_non_empty_wire_user_id(cx: &mut App) -> Option<String> {
        Self::contact_wire_user_id(cx).filter(|user_id| !user_id.is_empty())
    }

    fn contact_is_same_wire_user(user_id: &str, cx: &mut App) -> bool {
        Self::contact_wire_user_id(cx).as_deref() == Some(user_id)
    }

    fn contact_client_request_id(client_request_id: Option<&str>) -> String {
        client_request_id.map(str::to_owned).unwrap_or_else(|| Uuid::new_v4().uuid_string())
    }

    fn contact_decoded_payload<T: DeserializeOwned + 'static>(payload: Option<Box<dyn Any>>) -> Option<T> {
        match payload?.downcast::<T>() {
            Ok(decoded) => Some(*decoded),
            Err(payload) => SaiJSONCodec::decode::<T>(payload.downcast_ref::<Value>()),
        }
    }

    fn contact_owned_error(error: &anyhow::Error) -> anyhow::Error {
        if let Some(send_error) = error.downcast_ref::<SaiWsSendError>() {
            return anyhow::Error::new(send_error.clone());
        }
        if let Some(response_error) = error.downcast_ref::<SaiWsResponseError>() {
            return anyhow::Error::new(response_error.clone());
        }
        if let Some(mutation_error) = error.downcast_ref::<UserContactMutationError>() {
            return anyhow::Error::new(*mutation_error);
        }
        if error.downcast_ref::<CancellationError>().is_some() {
            return anyhow::Error::new(CancellationError);
        }
        anyhow::anyhow!("{error:#}")
    }

    fn contact_invalid_response() -> anyhow::Error {
        anyhow::Error::new(UserContactMutationError::InvalidResponse)
    }

    fn contact_log_network(text: String, cx: &mut App) {
        Sai::log("User", move || text, SaiLogKind::Network, cx);
    }

    fn contact_mutation_options(method: &str) -> SaiWsRequestOptionsInit {
        SaiWsRequestOptionsInit {
            service: rise_wire_methods::user::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            invoke_success_for_cached_response: false,
            track_until_acknowledged: true,
            exclude_from_debugger: true,
            ..SaiWsRequestOptionsInit::default()
        }
    }

    fn contact_page_options(
        method: &str,
        cache_key: &str,
        relative_id: Option<&str>,
        path_to_array: &str,
        path_to_id: &str,
        reflect_pending_in_ui: bool,
    ) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::user::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: Some(cache_key.to_owned()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(path_to_array.to_owned()),
            path_to_id: Some(path_to_id.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id: relative_id.map(str::to_owned),
                response_relative_id_path: key::RELATIVE_ID.to_owned(),
                has_more_path: Some(key::HAS_MORE.to_owned()),
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            exclude_from_debugger: true,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    fn contact_can_request_next_page(cache_key: &str, cx: &mut App) -> bool {
        SaiWs::pagination_state(cache_key, cx).read(cx).trailing().can_request_automatically()
    }

    pub fn get_contact_sync_settings_action(
        &mut self,
        on_result: Option<UserContactCallback<anyhow::Result<UserContactSyncSettingsGatewayDTO>>>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::contact_non_empty_wire_user_id(cx) else {
            return;
        };

        let cache_key = UserServicesStore::contact_sync_settings_cache_key(&user_id);
        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Disk, cx);
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::user::SERVICE.to_owned(),
            method: rise_wire_methods::user::GET_CONTACT_SYNC_SETTINGS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            exclude_from_debugger: true,
            ..SaiWsRequestOptionsInit::default()
        });

        let success_slot = UserContactCallbackSlot::new(on_result);
        let failure_slot = success_slot.clone();
        let slice = Rc::clone(&self.get_contact_sync_settings_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let services = UserServicesStore::shared(cx);
            let Some(gateway) = slice.data().filter(|gateway| gateway.settings.is_some()) else {
                let error = Self::contact_invalid_response();
                services.update(cx, |services, cx| services.contact_sync_settings_failed(&error, cx));
                success_slot.resolve(Err(error), cx);
                return;
            };
            services.update(cx, |services, cx| services.reconcile_contact_sync_settings(&gateway, &user_id, cx));
            success_slot.resolve(Ok(gateway), cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| services.contact_sync_settings_failed(error, cx));
            failure_slot.resolve(Err(Self::contact_owned_error(error)), cx);
        });

        SaiWs::request_slice(
            &self.get_contact_sync_settings_sai,
            rise_request::user::GetContactSyncSettings {},
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn update_contact_sync_settings_action(
        &mut self,
        enabled: bool,
        client_request_id: Option<&str>,
        on_result: Option<UserContactCallback<anyhow::Result<UserContactSyncSettingsGatewayDTO>>>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::contact_non_empty_wire_user_id(cx) else {
            return;
        };

        let services = UserServicesStore::shared(cx);
        let snapshot = services.update(cx, |services, cx| services.contact_sync_settings_snapshot(&user_id, cx));
        services.update(cx, |services, cx| services.apply_contact_sync_setting_optimistic(&user_id, enabled, cx));

        let body = rise_request::user::UpdateContactSyncSettings {
            client_request_id: Self::contact_client_request_id(client_request_id),
            sync_enabled: enabled,
        };
        let options = SaiWsRequestOptions::new(Self::contact_mutation_options(
            rise_wire_methods::user::UPDATE_CONTACT_SYNC_SETTINGS,
        ));

        let success_slot = UserContactCallbackSlot::new(on_result);
        let failure_slot = success_slot.clone();
        let failure_snapshot = snapshot.clone();
        let failure_user_id = user_id.clone();
        let slice = Rc::clone(&self.update_contact_sync_settings_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let services = UserServicesStore::shared(cx);
            let Some(gateway) = slice.data().filter(|gateway| gateway.settings.is_some()) else {
                services.update(cx, |services, cx| {
                    services.restore_contact_sync_settings(snapshot.clone(), &user_id, cx);
                });
                success_slot.resolve(Err(Self::contact_invalid_response()), cx);
                return;
            };
            services.update(cx, |services, cx| services.reconcile_contact_sync_settings(&gateway, &user_id, cx));
            success_slot.resolve(Ok(gateway), cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.restore_contact_sync_settings(failure_snapshot.clone(), &failure_user_id, cx);
            });
            failure_slot.resolve(Err(Self::contact_owned_error(error)), cx);
        });

        SaiWs::request_slice(&self.update_contact_sync_settings_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn sync_contacts_action(
        &mut self,
        body: rise_request::user::SyncContacts,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<SyncUserContactsGatewayDTO>> {
        if !SaiWs::manager(cx).read(cx).is_connected() {
            return Task::ready(Err(anyhow::Error::new(SaiWsSendError::NotConnected)));
        }

        let (continuation, waiter) = oneshot::channel();
        let result_box = UserContactAsyncResultBox::install(continuation);
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            track_until_acknowledged: false,
            ..Self::contact_mutation_options(rise_wire_methods::user::SYNC_CONTACTS)
        });

        let success_box = Rc::clone(&result_box);
        let slice = Rc::clone(&self.sync_contacts_sai);
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, _: &mut App| {
            let gateway =
                Self::contact_decoded_payload::<SyncUserContactsGatewayDTO>(payload).or_else(|| slice.data());
            match gateway {
                Some(gateway) => success_box.resolve(Ok(gateway)),
                None => success_box.resolve(Err(Self::contact_invalid_response())),
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, _: &mut App| {
            result_box.resolve(Err(Self::contact_owned_error(error)));
        });

        SaiWs::request_slice(&self.sync_contacts_sai, body, options, Some(success), Some(failure), cx);

        cx.foreground_executor().spawn(async move {
            match waiter.await {
                Ok(result) => result,
                Err(_) => Err(anyhow::Error::new(CancellationError)),
            }
        })
    }

    pub fn get_saved_contact_action(
        &mut self,
        target_user_id: &str,
        on_result: Option<UserContactCallback<anyhow::Result<UserSavedContactGatewayDTO>>>,
        cx: &mut Context<Self>,
    ) {
        let user_id = Self::contact_non_empty_wire_user_id(cx);
        let (Some(user_id), false) = (user_id, target_user_id.is_empty()) else {
            UserContactCallbackSlot::new(on_result)
                .resolve(Err(anyhow::Error::new(UserContactMutationError::InvalidContact)), cx);
            return;
        };

        let body = rise_request::user::GetSavedContact { target_user_id: target_user_id.to_owned() };
        let cache_key = UserServicesStore::saved_contact_cache_key(&user_id, target_user_id);
        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Disk, cx);
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::user::SERVICE.to_owned(),
            method: rise_wire_methods::user::GET_SAVED_CONTACT.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            exclude_from_debugger: true,
            ..SaiWsRequestOptionsInit::default()
        });

        let success_slot = UserContactCallbackSlot::new(on_result);
        let failure_slot = success_slot.clone();
        let success_target = target_user_id.to_owned();
        let failure_target = target_user_id.to_owned();
        let failure_user_id = user_id.clone();
        let slice = Rc::clone(&self.get_saved_contact_sai);
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            if !Self::contact_is_same_wire_user(&user_id, cx) {
                success_slot.resolve(Err(anyhow::Error::new(CancellationError)), cx);
                return;
            }
            let gateway =
                Self::contact_decoded_payload::<UserSavedContactGatewayDTO>(payload).or_else(|| slice.data());
            let services = UserServicesStore::shared(cx);
            let Some(gateway) = gateway else {
                let error = Self::contact_invalid_response();
                services.update(cx, |services, cx| services.saved_contact_fetch_failed(&success_target, &error, cx));
                success_slot.resolve(Err(error), cx);
                return;
            };
            services.update(cx, |services, cx| services.reconcile_saved_contact(&gateway, &success_target, cx));
            success_slot.resolve(Ok(gateway), cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if !Self::contact_is_same_wire_user(&failure_user_id, cx) {
                failure_slot.resolve(Err(anyhow::Error::new(CancellationError)), cx);
                return;
            }
            UserServicesStore::shared(cx)
                .update(cx, |services, cx| services.saved_contact_fetch_failed(&failure_target, error, cx));
            failure_slot.resolve(Err(Self::contact_owned_error(error)), cx);
        });

        SaiWs::request_slice(&self.get_saved_contact_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn list_saved_contacts_action(
        &mut self,
        relative_id: Option<&str>,
        on_result: Option<UserContactCallback<bool>>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::contact_non_empty_wire_user_id(cx) else {
            return;
        };

        let cache_key = UserServicesStore::saved_contacts_list_cache_key(&user_id);
        if relative_id.is_none() {
            SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Disk, cx);
        }
        let body = rise_request::user::ListSavedContacts {
            limit: Some(UserContactLimits::list_page()),
            relative_id: relative_id.map(str::to_owned),
        };
        let reflect_pending_in_ui = relative_id.is_none()
            && UserServicesStore::shared(cx).update(cx, |services, cx| services.saved_contacts(cx).is_empty());
        let options = Self::contact_page_options(
            rise_wire_methods::user::LIST_SAVED_CONTACTS,
            &cache_key,
            relative_id,
            key::CONTACTS,
            key::TARGET_USER_ID,
            reflect_pending_in_ui,
        );

        let success_slot = UserContactCallbackSlot::new(on_result);
        let failure_slot = success_slot.clone();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| services.hydrate_saved_contact_details_from_list(cx));
            success_slot.resolve(true, cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::contact_log_network(format!("list_saved_contacts failed: {error}"), cx);
            failure_slot.resolve(false, cx);
        });

        SaiWs::request_slice(&self.list_saved_contacts_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn upsert_saved_contact_action(
        &mut self,
        draft: UserContactDraft,
        peer: UserContactPeer,
        client_request_id: Option<&str>,
        on_result: UserContactCallback<anyhow::Result<UserSavedContactWireDTO>>,
        cx: &mut Context<Self>,
    ) {
        if Self::contact_non_empty_wire_user_id(cx).is_none() || !draft.is_valid() {
            on_result(Err(anyhow::Error::new(UserContactMutationError::InvalidContact)), cx);
            return;
        }

        let services = UserServicesStore::shared(cx);
        let snapshot = services.update(cx, |services, cx| services.saved_contact_snapshot(&draft.target_user_id, cx));
        services.update(cx, |services, cx| services.apply_saved_contact_optimistic(&draft, &peer, cx));

        let body = rise_request::user::UpsertSavedContact {
            client_request_id: Self::contact_client_request_id(client_request_id),
            share_phone_number: draft.share_phone_number,
            sync_to_phone: draft.sync_to_phone,
            target_user_id: draft.target_user_id.clone(),
            first_name: Some(draft.normalized_first_name()),
            last_name: Some(draft.normalized_last_name()),
            note: Some(draft.normalized_note()),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            allows_deferred_send: false,
            ..Self::contact_mutation_options(rise_wire_methods::user::UPSERT_SAVED_CONTACT)
        });

        let success_slot = UserContactCallbackSlot::new(Some(on_result));
        let failure_slot = success_slot.clone();
        let success_target = draft.target_user_id.clone();
        let failure_target = draft.target_user_id;
        let failure_snapshot = snapshot.clone();
        let slice = Rc::clone(&self.upsert_saved_contact_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let services = UserServicesStore::shared(cx);
            let gateway = slice.data();
            let Some((gateway, contact)) =
                gateway.and_then(|gateway| gateway.contact.clone().map(|contact| (gateway, contact)))
            else {
                let error = Self::contact_invalid_response();
                services.update(cx, |services, cx| {
                    services.restore_saved_contact(snapshot.clone(), &success_target, &error, cx);
                });
                success_slot.resolve(Err(error), cx);
                return;
            };
            services.update(cx, |services, cx| services.reconcile_saved_contact(&gateway, &success_target, cx));
            success_slot.resolve(Ok(contact), cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.restore_saved_contact(failure_snapshot.clone(), &failure_target, error, cx);
            });
            failure_slot.resolve(Err(Self::contact_owned_error(error)), cx);
        });

        SaiWs::request_slice(&self.upsert_saved_contact_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn delete_saved_contact_action(
        &mut self,
        target_user_id: &str,
        client_request_id: Option<&str>,
        on_result: UserContactCallback<anyhow::Result<()>>,
        cx: &mut Context<Self>,
    ) {
        if Self::contact_non_empty_wire_user_id(cx).is_none() || target_user_id.is_empty() {
            on_result(Err(anyhow::Error::new(UserContactMutationError::InvalidContact)), cx);
            return;
        }

        let services = UserServicesStore::shared(cx);
        let snapshot = services.update(cx, |services, cx| services.saved_contact_snapshot(target_user_id, cx));
        services.update(cx, |services, cx| services.apply_delete_saved_contact_optimistic(target_user_id, cx));

        let body = rise_request::user::DeleteSavedContact {
            client_request_id: Self::contact_client_request_id(client_request_id),
            target_user_id: target_user_id.to_owned(),
        };
        let options = SaiWsRequestOptions::new(Self::contact_mutation_options(
            rise_wire_methods::user::DELETE_SAVED_CONTACT,
        ));

        let success_slot = UserContactCallbackSlot::new(Some(on_result));
        let failure_slot = success_slot.clone();
        let success_target = target_user_id.to_owned();
        let failure_target = target_user_id.to_owned();
        let failure_snapshot = snapshot.clone();
        let slice = Rc::clone(&self.delete_saved_contact_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let services = UserServicesStore::shared(cx);
            let Some(gateway) = slice.data() else {
                let error = Self::contact_invalid_response();
                services.update(cx, |services, cx| {
                    services.restore_saved_contact(snapshot.clone(), &success_target, &error, cx);
                });
                success_slot.resolve(Err(error), cx);
                return;
            };
            services.update(cx, |services, cx| services.reconcile_saved_contact(&gateway, &success_target, cx));
            success_slot.resolve(Ok(()), cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.restore_saved_contact(failure_snapshot.clone(), &failure_target, error, cx);
            });
            failure_slot.resolve(Err(Self::contact_owned_error(error)), cx);
        });

        SaiWs::request_slice(&self.delete_saved_contact_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn set_saved_contact_photo_action(
        &mut self,
        target_user_id: &str,
        upload: Option<UserContactPhotoUpload>,
        client_request_id: Option<&str>,
        on_result: UserContactCallback<anyhow::Result<UserSavedContactWireDTO>>,
        cx: &mut Context<Self>,
    ) {
        if Self::contact_wire_user_id(cx).is_none() || target_user_id.is_empty() {
            on_result(Err(anyhow::Error::new(UserContactMutationError::InvalidContact)), cx);
            return;
        }

        let snapshot = UserServicesStore::shared(cx).update(cx, |services, cx| {
            services.apply_saved_contact_photo_optimistic(target_user_id, upload.as_ref(), cx)
        });
        let body = rise_request::user::SetSavedContactPhoto {
            client_request_id: Self::contact_client_request_id(client_request_id),
            target_user_id: target_user_id.to_owned(),
            asset_id: upload.as_ref().map(|upload| upload.asset_id.clone()),
            stable_file_url: upload.as_ref().map(|upload| upload.stable_file_url.clone()),
        };
        let options = SaiWsRequestOptions::new(Self::contact_mutation_options(
            rise_wire_methods::user::SET_SAVED_CONTACT_PHOTO,
        ));

        let success_slot = UserContactCallbackSlot::new(Some(on_result));
        let failure_slot = success_slot.clone();
        let success_target = target_user_id.to_owned();
        let failure_target = target_user_id.to_owned();
        let failure_snapshot = snapshot.clone();
        let slice = Rc::clone(&self.set_saved_contact_photo_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let services = UserServicesStore::shared(cx);
            let settled = slice.data().and_then(|gateway| {
                let contact = gateway.contact.clone()?;
                let matches_request = match &upload {
                    Some(upload) => {
                        let returned_asset = contact
                            .avatar_override_asset_id
                            .as_deref()
                            .map(|asset_id| trimming_whitespaces_and_newlines(asset_id).to_lowercase());
                        let returned_url =
                            contact.avatar_override_url.as_deref().map(trimming_whitespaces_and_newlines);
                        returned_asset == Some(upload.asset_id.to_lowercase())
                            && returned_url.is_some_and(|url| !url.is_empty())
                    }
                    None => {
                        contact.avatar_override_asset_id.as_deref().is_none_or(str::is_empty)
                            && contact.avatar_override_url.as_deref().is_none_or(str::is_empty)
                    }
                };
                matches_request.then_some((gateway, contact))
            });
            let Some((gateway, contact)) = settled else {
                let error = Self::contact_invalid_response();
                services.update(cx, |services, cx| {
                    services.restore_saved_contact_photo(snapshot.clone(), &success_target, &error, cx);
                });
                success_slot.resolve(Err(error), cx);
                return;
            };
            services.update(cx, |services, cx| services.reconcile_saved_contact_photo(&gateway, &success_target, cx));
            success_slot.resolve(Ok(contact), cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.restore_saved_contact_photo(failure_snapshot.clone(), &failure_target, error, cx);
            });
            failure_slot.resolve(Err(Self::contact_owned_error(error)), cx);
        });

        SaiWs::request_slice(&self.set_saved_contact_photo_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn suggest_contact_photo_action(
        &mut self,
        target_user_id: &str,
        upload: UserContactPhotoUpload,
        client_request_id: Option<&str>,
        on_result: UserContactCallback<anyhow::Result<UserContactPhotoSuggestionWireDTO>>,
        cx: &mut Context<Self>,
    ) {
        if Self::contact_wire_user_id(cx).is_none() || target_user_id.is_empty() {
            on_result(Err(anyhow::Error::new(UserContactMutationError::InvalidContact)), cx);
            return;
        }

        UserServicesStore::shared(cx).update(cx, |services, cx| {
            services.contact_photo_suggestion_pending_targets.insert(target_user_id.to_owned());
            cx.notify();
        });

        let body = rise_request::user::SuggestContactPhoto {
            asset_id: upload.asset_id,
            client_request_id: Self::contact_client_request_id(client_request_id),
            stable_file_url: upload.stable_file_url,
            target_user_id: target_user_id.to_owned(),
        };
        let options = SaiWsRequestOptions::new(Self::contact_mutation_options(
            rise_wire_methods::user::SUGGEST_CONTACT_PHOTO,
        ));

        let success_slot = UserContactCallbackSlot::new(Some(on_result));
        let failure_slot = success_slot.clone();
        let success_target = target_user_id.to_owned();
        let failure_target = target_user_id.to_owned();
        let slice = Rc::clone(&self.suggest_contact_photo_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.contact_photo_suggestion_pending_targets.remove(&success_target);
                cx.notify();
            });
            let suggestion = slice
                .data()
                .and_then(|gateway| gateway.suggestion)
                .filter(|suggestion| {
                    suggestion.status.to_uppercase() == user_service::CONTACT_PHOTO_SUGGESTION_STATUS_PENDING
                });
            match suggestion {
                Some(suggestion) => success_slot.resolve(Ok(suggestion), cx),
                None => success_slot.resolve(Err(Self::contact_invalid_response()), cx),
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.contact_photo_suggestion_pending_targets.remove(&failure_target);
                cx.notify();
            });
            failure_slot.resolve(Err(Self::contact_owned_error(error)), cx);
        });

        SaiWs::request_slice(&self.suggest_contact_photo_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn list_contact_photo_suggestions_action(
        &mut self,
        relative_id: Option<&str>,
        automatic_page: i64,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return;
        };

        let body = rise_request::user::ListContactPhotoSuggestions {
            limit: Some(UserContactLimits::list_page()),
            relative_id: relative_id.map(str::to_owned),
        };
        let cache_key = UserServicesStore::contact_photo_suggestions_cache_key(&user_id);
        if relative_id.is_none() {
            SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Disk, cx);
        }
        let options = Self::contact_page_options(
            rise_wire_methods::user::LIST_CONTACT_PHOTO_SUGGESTIONS,
            &cache_key,
            relative_id,
            key::SUGGESTIONS,
            key::ID,
            false,
        );

        let requested_relative_id = relative_id.map(str::to_owned);
        let slice = Rc::clone(&self.list_contact_photo_suggestions_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let page = slice.data();
            let next = page
                .filter(|page| page.has_more)
                .and_then(|page| page.relative_id)
                .filter(|_| automatic_page + 1 < UserContactLimits::maximum_automatic_inbox_pages())
                .filter(|next| requested_relative_id.as_deref() != Some(next.as_str()));
            if let Some(next) = next
                && Self::contact_can_request_next_page(&cache_key, cx)
            {
                cx.defer(move |cx| {
                    UserActionsStore::shared(cx).update(cx, |actions, cx| {
                        actions.list_contact_photo_suggestions_action(Some(next.as_str()), automatic_page + 1, cx);
                    });
                });
                return;
            }
            UserServicesStore::shared(cx)
                .update(cx, |services, cx| services.present_next_contact_photo_suggestion_if_needed(cx));
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::contact_log_network(format!("list_contact_photo_suggestions failed: {error}"), cx);
        });

        SaiWs::request_slice(&self.list_contact_photo_suggestions_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn resolve_contact_photo_suggestion_action(
        &mut self,
        suggestion_id: &str,
        accept: bool,
        client_request_id: Option<&str>,
        on_result: UserContactCallback<anyhow::Result<UserContactPhotoSuggestionWireDTO>>,
        cx: &mut Context<Self>,
    ) {
        if Self::contact_wire_user_id(cx).is_none() {
            return;
        }

        let snapshot = UserServicesStore::shared(cx).update(cx, |services, cx| {
            services.apply_contact_photo_suggestion_resolved_optimistic(suggestion_id, cx)
        });
        let body = rise_request::user::ResolveContactPhotoSuggestion {
            accept,
            client_request_id: Self::contact_client_request_id(client_request_id),
            suggestion_id: suggestion_id.to_owned(),
        };
        let options = SaiWsRequestOptions::new(Self::contact_mutation_options(
            rise_wire_methods::user::RESOLVE_CONTACT_PHOTO_SUGGESTION,
        ));

        let success_slot = UserContactCallbackSlot::new(Some(on_result));
        let failure_slot = success_slot.clone();
        let failure_snapshot = snapshot.clone();
        let slice = Rc::clone(&self.resolve_contact_photo_suggestion_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(suggestion) = slice.data().and_then(|gateway| gateway.suggestion) else {
                UserServicesStore::shared(cx)
                    .update(cx, |services, cx| services.restore_contact_photo_suggestions(snapshot.clone(), cx));
                success_slot.resolve(Err(Self::contact_invalid_response()), cx);
                return;
            };
            success_slot.resolve(Ok(suggestion), cx);
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx)
                .update(cx, |services, cx| services.restore_contact_photo_suggestions(failure_snapshot.clone(), cx));
            failure_slot.resolve(Err(Self::contact_owned_error(error)), cx);
        });

        SaiWs::request_slice(
            &self.resolve_contact_photo_suggestion_sai,
            body,
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn block_user_action(
        &mut self,
        target_user_id: &str,
        blocked: bool,
        on_result: UserContactCallback<anyhow::Result<bool>>,
        cx: &mut Context<Self>,
    ) {
        self.block_user_action_scope(target_user_id, blocked.then_some(RiseBlockScope::Standard), on_result, cx);
    }

    pub fn block_user_action_scope(
        &mut self,
        target_user_id: &str,
        scope: Option<RiseBlockScope>,
        on_result: UserContactCallback<anyhow::Result<bool>>,
        cx: &mut Context<Self>,
    ) {
        let user_id = Self::contact_wire_user_id(cx);
        let Some(user_id) =
            user_id.filter(|user_id| !target_user_id.is_empty() && target_user_id != user_id.as_str())
        else {
            on_result(Err(anyhow::Error::new(UserContactMutationError::InvalidContact)), cx);
            return;
        };

        let target_user_id = target_user_id.to_owned();
        cx.defer(move |cx: &mut App| Self::block_user_apply_scope(user_id, target_user_id, scope, on_result, cx));
    }

    fn block_user_apply_scope(
        user_id: String,
        target_user_id: String,
        scope: Option<RiseBlockScope>,
        on_result: UserContactCallback<anyhow::Result<bool>>,
        cx: &mut App,
    ) {
        let services = UserServicesStore::shared(cx);
        let snapshot = services.update(cx, |services, cx| services.block_projection_snapshot(&target_user_id, cx));
        services.update(cx, |services, cx| services.apply_blocked_optimistic(&target_user_id, scope, cx));

        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => domain.set_block(&target_user_id, scope).await,
                Err(error) => Err(error),
            };
            cx.update(|cx| {
                if !Self::contact_is_same_wire_user(&user_id, cx) {
                    return;
                }
                let services = UserServicesStore::shared(cx);
                match result {
                    Ok(()) => {
                        services.update(cx, |services, cx| services.reconcile_blocked(&target_user_id, scope, cx));
                        on_result(Ok(scope.is_some()), cx);
                    }
                    Err(error) => {
                        services.update(cx, |services, cx| {
                            services.restore_blocked(snapshot, &target_user_id, &error, cx);
                        });
                        on_result(Err(error), cx);
                    }
                }
            });
        })
        .detach();
    }

    pub fn note_block_scope_action(&mut self, target_user_id: &str, scope: Option<RiseBlockScope>, cx: &mut Context<Self>) {
        let target_user_id = target_user_id.to_owned();
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            if let Ok(domain) = domain.await {
                domain.note_block_scope(&target_user_id, scope).await;
            }
        })
        .detach();
    }

    pub fn load_blocked_users_action(&mut self, force: bool, cx: &mut Context<Self>) {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            if let Ok(domain) = domain.await {
                domain.load_blocked_users(force).await;
            }
        })
        .detach();
    }

    pub fn load_more_blocked_users_action(&mut self, cx: &mut Context<Self>) {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            if let Ok(domain) = domain.await {
                domain.load_more_blocked_users().await;
            }
        })
        .detach();
    }

    pub fn trim_blocked_users_action(&mut self, cx: &mut Context<Self>) {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            if let Ok(domain) = domain.await {
                domain.trim_blocked_users().await;
            }
        })
        .detach();
    }

    pub fn list_contact_join_events_action(
        &mut self,
        relative_id: Option<&str>,
        automatic_page: i64,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return;
        };

        let body = rise_request::user::ListContactJoinEvents {
            limit: Some(UserContactLimits::list_page()),
            relative_id: relative_id.map(str::to_owned),
        };
        let cache_key = UserServicesStore::contact_join_events_cache_key(&user_id);
        if relative_id.is_none() {
            SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Disk, cx);
        }
        let options = Self::contact_page_options(
            rise_wire_methods::user::LIST_CONTACT_JOIN_EVENTS,
            &cache_key,
            relative_id,
            key::EVENTS,
            key::ID,
            false,
        );

        let requested_relative_id = relative_id.map(str::to_owned);
        let slice = Rc::clone(&self.list_contact_join_events_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let page = slice.data();
            let next = page
                .filter(|page| page.has_more)
                .and_then(|page| page.relative_id)
                .filter(|_| automatic_page + 1 < UserContactLimits::maximum_automatic_inbox_pages())
                .filter(|next| requested_relative_id.as_deref() != Some(next.as_str()));
            if let Some(next) = next
                && Self::contact_can_request_next_page(&UserServicesStore::contact_join_events_cache_key(&user_id), cx)
            {
                cx.defer(move |cx| {
                    UserActionsStore::shared(cx).update(cx, |actions, cx| {
                        actions.list_contact_join_events_action(Some(next.as_str()), automatic_page + 1, cx);
                    });
                });
                return;
            }
            let services = UserServicesStore::shared(cx);
            services.update(cx, |services, cx| services.restore_contact_join_dialogs_from_inbox(cx));
            let ids = services.update(cx, |services, cx| services.recent_unread_contact_join_event_ids(cx));
            if !ids.is_empty() {
                cx.defer(move |cx| {
                    UserActionsStore::shared(cx)
                        .update(cx, |actions, cx| actions.mark_contact_join_events_read_action(ids, cx));
                });
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::contact_log_network(format!("list_contact_join_events failed: {error}"), cx);
        });

        SaiWs::request_slice(&self.list_contact_join_events_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn mark_contact_join_events_read_action(&mut self, event_ids: Vec<String>, cx: &mut Context<Self>) {
        let batch_limit = usize::try_from(UserContactLimits::maximum_join_ack_batch()).unwrap_or(0);
        let exact_ids = event_ids
            .into_iter()
            .filter(|event_id| !event_id.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(batch_limit)
            .collect::<Vec<_>>();
        if Self::contact_wire_user_id(cx).is_none() || exact_ids.is_empty() {
            return;
        }

        UserServicesStore::shared(cx)
            .update(cx, |services, cx| services.apply_contact_join_events_read_optimistic(&exact_ids, cx));

        let body = rise_request::user::MarkContactJoinEventsRead { event_ids: Some(exact_ids) };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::user::SERVICE.to_owned(),
            method: rise_wire_methods::user::MARK_CONTACT_JOIN_EVENTS_READ.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            track_until_acknowledged: true,
            exclude_from_debugger: true,
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Rc::clone(&self.mark_contact_join_events_read_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if slice.data().is_none() {
                Self::contact_log_network(
                    "mark_contact_join_events_read returned an unsuccessful response".to_owned(),
                    cx,
                );
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::contact_log_network(format!("mark_contact_join_events_read failed: {error}"), cx);
        });

        SaiWs::request_slice(&self.mark_contact_join_events_read_sai, body, options, Some(success), Some(failure), cx);
    }
}
